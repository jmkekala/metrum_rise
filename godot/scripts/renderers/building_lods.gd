# SPDX-License-Identifier: GPL-2.0-only

## Resource/upload half of spatial building LOD rendering. Rust owns candidate gathering,
## projection, hysteresis and grouping; Godot applies complete changed batches before drawing.
extends Node3D

const BuildingMeshes := preload("res://scripts/renderers/building_meshes.gd")
const SchemeMaterials := preload("res://scripts/renderers/scheme_materials.gd")
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
var _scheme_variants: Dictionary = {}
# Batch keys pack tier/deserted/scheme into one int; the stride is the widest authored
# scheme table in this catalog, so a key stays unique across every part.
var _scheme_stride := 1
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
	_scheme_variants.clear()
	_scheme_stride = 1
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
		_scheme_stride = maxi(_scheme_stride, part.get("schemes", []).size())
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
	var scheme := int(change.get("scheme", 0))
	var key := Vector4i(chunk.x, chunk.y, int(change.part),
		(int(change.lod) * 2 + int(change.deserted)) * _scheme_stride + scheme)
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
		instance.gi_mode = GeometryInstance3D.GI_MODE_DYNAMIC
		if change.deserted:
			# One flat material replaces every surface, so a deserted group never needs a
			# scheme variant and keeps sharing the source mesh.
			instance.multimesh.mesh = meshes[int(change.part)][int(change.lod)]
			instance.material_override = _deserted_material
		else:
			var group := _scheme_group(int(change.part), int(change.lod), scheme)
			instance.multimesh.mesh = group["mesh"]
			if group["override"] != null:
				instance.material_override = group["override"]
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

## Decode a batch dictionary key back into its group identity. `_apply_batch` is the only
## writer of this packing, so diagnostics and regressions cannot drift from it.
func batch_identity(key: Vector4i) -> Dictionary:
	var group := key.w / _scheme_stride
	return {"chunk": Vector2i(key.x, key.y), "part": key.z, "lod": group / 2,
		"deserted": group % 2 == 1, "scheme": key.w % _scheme_stride}

## Mesh and whole-instance override that draw one catalog part, tier and scheme.
## Built once per triple and shared by every chunk that draws it.
##
## Godot exposes per-surface overrides on `MeshInstance3D` only, never on a MultiMesh, so a
## tier with one surface takes an instance override and keeps sharing the source mesh, while
## a multi-material tier needs its own mesh resource to leave untargeted surfaces authored.
## Only parts that actually author schemes pay for that duplicated geometry.
func _scheme_group(part: int, lod: int, scheme: int) -> Dictionary:
	var cache_key := Vector3i(part, lod, scheme)
	if _scheme_variants.has(cache_key):
		return _scheme_variants[cache_key]
	var mesh: Mesh = meshes[part][lod]
	var group := {"mesh": mesh, "override": null}
	var bindings := _scheme_bindings(part, lod, scheme)
	if not bindings.is_empty() and mesh != null:
		var replacements: Array[Material] = []
		var replaced := 0
		for surface in mesh.get_surface_count():
			var source := mesh.surface_get_material(surface) as BaseMaterial3D
			var replacement: Material = null
			if source != null and bindings.has(source.resource_name):
				replacement = SchemeMaterials.variant(source, bindings[source.resource_name])
				replaced += 1
			replacements.append(replacement)
		if replaced == 1 and replacements.size() == 1:
			group["override"] = replacements[0]
		elif replaced > 0:
			var variant := mesh.duplicate() as ArrayMesh
			for surface in replacements.size():
				if replacements[surface] != null:
					variant.surface_set_material(surface, replacements[surface])
			group["mesh"] = variant
	_scheme_variants[cache_key] = group
	return group

## Resolve Rust's published scheme table into source material name to loaded textures.
## An unreadable texture drops its channel, leaving that channel's authored value in place.
func _scheme_bindings(part: int, lod: int, scheme: int) -> Dictionary:
	var schemes: Array = catalog[part].get("schemes", [])
	if scheme < 0 or scheme >= schemes.size():
		return {}
	var lods: Array = schemes[scheme].get("lods", [])
	if lod < 0 or lod >= lods.size():
		return {}
	var result: Dictionary = {}
	for binding: Dictionary in lods[lod]:
		var textures: Dictionary = {}
		for channel in ["albedo", "orm", "normal", "emission"]:
			if not binding.has(channel):
				continue
			var texture: Texture2D = resources.load_texture(str(binding[channel]))
			if texture != null:
				textures[channel] = texture
		if not textures.is_empty():
			result[str(binding.material)] = textures
	return result
