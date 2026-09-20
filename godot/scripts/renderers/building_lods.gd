# SPDX-License-Identifier: GPL-2.0-only

## Resource/upload half of spatial building LOD rendering. Rust owns candidate gathering,
## projection, hysteresis and grouping; Godot applies complete changed batches before drawing.
extends Node3D

const BuildingMeshes := preload("res://scripts/renderers/building_meshes.gd")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const GameSettings := preload("res://scripts/core/game_settings.gd")

var simulation_node: Node
var batches: Dictionary = {}
var catalog: Array = []
var meshes: Array = []
var resources: RefCounted
var generation := -1
var quality := 1
var evaluated_parts := 0
var queried_chunks := 0
var uploaded_bytes := 0
var _source_parts: Dictionary = {}
var _placeholder: Mesh
var _deserted_material: StandardMaterial3D
var _sun: DirectionalLight3D

func configure(simulation: Node, placeholder: Mesh) -> void:
	simulation_node = simulation
	_placeholder = placeholder
	_deserted_material = StandardMaterial3D.new()
	_deserted_material.albedo_color = Color(0.45, 0.42, 0.38, 1.0)
	quality = GameSettings.get_building_lod_quality()
	add_to_group("building_lod_renderers")
	if get_parent() and get_parent().get_parent():
		_sun = get_parent().get_parent().get_node_or_null("DirectionalLight3D") as DirectionalLight3D
	reload_resources()

func set_quality(value: int) -> void:
	quality = clampi(value, 0, 2)

func reload_resources() -> void:
	for instance: MultiMeshInstance3D in batches.values():
		instance.free()
	batches.clear()
	meshes.clear()
	_source_parts.clear()
	resources = BuildingMeshes.new()
	var description: Dictionary = simulation_node.get_building_lod_catalog()
	generation = int(description.generation)
	catalog = description.parts
	var availability: Array[PackedByteArray] = []
	var resource_bounds: Array[AABB] = []
	for part: Dictionary in catalog:
		var chain: Array[Mesh] = []
		var loaded := PackedByteArray()
		for path: String in part.paths:
			var mesh: Mesh = _placeholder if path.is_empty() else resources.load_mesh(path)
			chain.append(mesh)
			loaded.append(1 if mesh != null else 0)
			if mesh == null:
				push_warning("Building LOD import failed; a valid tier will be retained: " + path)
		if not loaded.has(1):
			chain[0] = _placeholder
			loaded[0] = 1
		var first := loaded.find(1)
		_source_parts["%s|part:%d" % [part.asset_id, part.part_index]] = chain[first]
		var bounds := chain[first].get_aabb()
		for mesh: Mesh in chain:
			if mesh != null:
				bounds = bounds.merge(mesh.get_aabb())
		resource_bounds.append(bounds)
		meshes.append(chain)
		availability.append(loaded)
	if not simulation_node.configure_building_lod_resources(generation, availability, resource_bounds):
		push_error("Building LOD catalog changed during resource loading")

func get_source_mesh(asset_id: String, part_index: int) -> Mesh:
	return _source_parts.get("%s|part:%d" % [asset_id, part_index])

func update(camera: Camera3D) -> bool:
	uploaded_bytes = 0
	evaluated_parts = 0
	queried_chunks = 0
	if camera == null:
		return false
	var viewport := camera.get_viewport()
	var render_size := Vector2(viewport.get_texture().get_size()) * viewport.scaling_3d_scale
	var camera_transform := camera.get_camera_transform()
	var shadow_direction := Vector3.UP
	var shadow_distance := 0.0
	if is_instance_valid(_sun) and _sun.shadow_enabled and _sun.visible:
		shadow_direction = _sun.global_basis.z
		shadow_distance = _sun.directional_shadow_max_distance
	var frame: Dictionary = simulation_node.try_get_building_lod_frame(
		generation, camera.get_camera_projection() * Projection(camera_transform.affine_inverse()),
		render_size, camera_transform.origin, -camera_transform.basis.z, quality,
		shadow_direction, shadow_distance
	)
	if frame.get("reload", false):
		reload_resources()
		return false
	if frame.get("busy", true):
		return false
	evaluated_parts = int(frame.get("evaluated_parts", 0))
	queried_chunks = int(frame.get("queried_chunks", 0))
	for change: Dictionary in frame.get("updates", []):
		_apply_batch(change)
	return true

func _apply_batch(change: Dictionary) -> void:
	var chunk: Vector2i = change.chunk
	var key := Vector4i(chunk.x, chunk.y, int(change.part), int(change.lod) * 2 + int(change.deserted))
	var buffer: PackedFloat32Array = change.transforms
	var count := buffer.size() / 12
	var instance: MultiMeshInstance3D = batches.get(key)
	if change.get("retired", false):
		if instance != null:
			instance.free()
			batches.erase(key)
		return
	if instance == null:
		if count == 0:
			return
		instance = MultiMeshInstance3D.new()
		instance.multimesh = MultiMesh.new()
		instance.multimesh.transform_format = MultiMesh.TRANSFORM_3D
		instance.multimesh.mesh = meshes[int(change.part)][int(change.lod)]
		instance.gi_mode = GeometryInstance3D.GI_MODE_DYNAMIC
		if change.deserted:
			instance.material_override = _deserted_material
		SceneLightingConfig.apply_shadow_policy(instance, SceneLightingConfig.SHADOW_STATIC_CASTER, "buildings")
		add_child(instance)
		batches[key] = instance
	var multimesh := instance.multimesh
	# Keep peak power-of-two capacity while this chunk is resident; a tier switch does
	# not shrink/reallocate buffers. Only visible_instance_count determines what is drawn.
	if count > multimesh.instance_count:
		var capacity := maxi(1, multimesh.instance_count)
		while capacity < count:
			capacity *= 2
		multimesh.instance_count = capacity
	if count > 0:
		multimesh.custom_aabb = change.bounds
		buffer.resize(multimesh.instance_count * 12)
		multimesh.buffer = buffer
		uploaded_bytes += buffer.size() * 4
	multimesh.visible_instance_count = count
	instance.visible = count > 0
