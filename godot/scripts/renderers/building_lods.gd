# SPDX-License-Identifier: GPL-2.0-only

## Resource half of spatial building LOD rendering. Rust owns candidate gathering, projection,
## hysteresis, grouping and the batches themselves: it draws them through the stream backend's
## pooled `RenderingServer` objects, in the world and in the water reflection's
## (`RENDER-32-L`). This script loads the tier meshes, builds each group's mesh and material
## override when Rust first names the group, and hands them over by RID.
extends Node3D

const BuildingMeshes := preload("res://scripts/renderers/building_meshes.gd")
const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")
const SchemeMaterials := preload("res://scripts/renderers/scheme_materials.gd")
const GameSettings := preload("res://scripts/core/game_settings.gd")

var simulation_node: Node
var catalog: Array = []
var meshes: Array = []
var resources: RefCounted
var generation := -1
var quality := 1
var evaluated_parts := 0
var queried_chunks := 0
var uploaded_bytes := 0
## The last frame's Rust microseconds (the perf channel): before the update, the update, the
## publication.
var timing: Dictionary = {}
var _source_parts: Dictionary = {}
# Authored surface materials of each loaded mesh, by mesh. The mesh itself draws converted
# copies (see _converted), and scheme bindings still match and vary the originals.
var _source_materials: Dictionary = {}
# The window brightness each converted mesh carries, and per [shared mesh, brightness] the
# converted copy made when another asset reuses that mesh at a different brightness.
var _mesh_brightness: Dictionary = {}
var _brightness_variants: Dictionary = {}
var _scheme_variants: Dictionary = {}
# Batch keys pack tier/deserted/scheme into one int; the stride is the widest authored
# scheme table in this catalog, so a key stays unique across every part.
var _scheme_stride := 1
var _placeholder: Mesh
var _deserted_material: Material
var _sun: DirectionalLight3D
# The look registered per group, Vector4i(part, tier, deserted, scheme) -> [mesh, override]:
# keeps the resources alive while Rust draws with their RIDs.
var _looks: Dictionary = {}
var _reflection_scenario := RID()
var _fresh := false

func configure(simulation: Node, placeholder: Mesh) -> void:
	simulation_node = simulation
	_placeholder = placeholder
	var deserted := StandardMaterial3D.new()
	deserted.albedo_color = Color(0.45, 0.42, 0.38, 1.0)
	_deserted_material = WindowMaterials.create(deserted)
	quality = GameSettings.get_building_lod_quality()
	add_to_group("building_lod_renderers")
	if get_parent() and get_parent().get_parent():
		_sun = get_parent().get_parent().get_node_or_null("DirectionalLight3D") as DirectionalLight3D
	simulation_node.set_building_lod_scenario(get_world_3d().scenario)
	var reflection := get_tree().get_first_node_in_group("water_reflections")
	if reflection != null:
		_reflection_scenario = reflection.reflection_scenario()
	simulation_node.set_building_lod_reflection_scenario(_reflection_scenario)
	reload_resources()

## The water reflection's world (`water_reflection.gd`); an invalid RID when it goes away.
func set_reflection_scenario(scenario: RID) -> void:
	_reflection_scenario = scenario
	if simulation_node != null:
		simulation_node.set_building_lod_reflection_scenario(scenario)

func set_quality(value: int) -> void:
	quality = clampi(value, 0, 2)

func reload_resources() -> void:
	_looks.clear()
	meshes.clear()
	_source_parts.clear()
	_source_materials.clear()
	_mesh_brightness.clear()
	_brightness_variants.clear()
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
			if mesh != null:
				mesh = _converted(mesh, float(part.get("window_brightness", WindowMaterials.DEFAULT_BRIGHTNESS)))
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

# Every building surface draws through the window shader, whose light() lets the clouds dim
# the sun; Godot's own materials offer no hook for that. The meshes are this renderer's own
# bakes, not shared imports, so their surfaces are converted in place, once per mesh, and no
# geometry is duplicated. The shader carries the asset's window brightness, and BuildingMeshes
# shares a mesh between assets that use the same model: another asset reusing it at a different
# brightness gets its own converted copy, the only case that duplicates geometry.
func _converted(mesh: Mesh, window_brightness: float) -> Mesh:
	if not _source_materials.has(mesh):
		var originals: Array[BaseMaterial3D] = []
		for surface in mesh.get_surface_count():
			var source := mesh.surface_get_material(surface) as BaseMaterial3D
			originals.append(source)
			if source != null:
				mesh.surface_set_material(surface, WindowMaterials.create(source, false, window_brightness))
		_source_materials[mesh] = originals
		_mesh_brightness[mesh] = window_brightness
		return mesh
	if is_equal_approx(_mesh_brightness[mesh], window_brightness):
		return mesh
	var key := [mesh, window_brightness]
	if not _brightness_variants.has(key):
		var variant := mesh.duplicate() as Mesh
		var originals: Array = _source_materials[mesh]
		for surface in variant.get_surface_count():
			if originals[surface] != null:
				variant.surface_set_material(surface,
					WindowMaterials.create(originals[surface], false, window_brightness))
		_source_materials[variant] = originals
		_mesh_brightness[variant] = window_brightness
		_brightness_variants[key] = variant
	return _brightness_variants[key]

func get_source_mesh(asset_id: String, part_index: int) -> Mesh:
	return _source_parts.get("%s|part:%d" % [asset_id, part_index])

func update(camera: Camera3D) -> bool:
	uploaded_bytes = 0
	evaluated_parts = 0
	queried_chunks = 0
	timing = {}
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
	# Motion vectors (TAA, FSR 2) keep a buffer's previous upload: reused slots take fresh storage.
	var fresh := viewport.use_taa or viewport.scaling_3d_mode == Viewport.SCALING_3D_MODE_FSR2
	if fresh != _fresh:
		_fresh = fresh
		simulation_node.set_building_lod_fresh_buffers(fresh)
	var projection := camera.get_camera_projection()
	var frame: Dictionary = simulation_node.update_building_lods(
		generation, projection * Projection(camera_transform.affine_inverse()), projection,
		camera_transform, render_size, quality, shadow_direction, shadow_distance
	)
	if frame.get("reload", false):
		reload_resources()
		return false
	if frame.get("busy", true):
		return false
	evaluated_parts = int(frame.get("evaluated_parts", 0))
	queried_chunks = int(frame.get("queried_chunks", 0))
	timing = {"prepare_us": frame.get("prepare_us", 0.0), "update_us": frame.get("update_us", 0.0),
		"publish_us": frame.get("publish_us", 0.0), "placing_us": frame.get("placing_us", 0.0),
		"evaluating_us": frame.get("evaluating_us", 0.0)}
	var missing: PackedInt32Array = frame.get("missing", PackedInt32Array())
	if missing.is_empty():
		uploaded_bytes = int(frame.get("uploaded_bytes", 0))
		return true
	# Groups seen for the first time: their looks, then the waiting frame, whole.
	for at in range(0, missing.size(), 4):
		_register_look(missing[at], missing[at + 1], missing[at + 2] == 1, missing[at + 3])
	uploaded_bytes = maxi(0, simulation_node.publish_building_lods())
	return true

# The mesh and whole-instance override one group draws with, handed to Rust by RID.
func _register_look(part: int, lod: int, deserted: bool, scheme: int) -> void:
	var mesh: Mesh = meshes[part][lod]
	var override: Material = null
	if deserted:
		# One flat material replaces every surface, so a deserted group never needs a
		# scheme variant and keeps sharing the source mesh.
		override = _deserted_material
	else:
		var group := _scheme_group(part, lod, scheme)
		mesh = group["mesh"]
		override = group["override"]
	_looks[Vector4i(part, lod, int(deserted), scheme)] = [mesh, override]
	simulation_node.register_building_lod_look(part, lod, deserted, scheme, mesh.get_rid(),
		override.get_rid() if override != null else RID())

## The mesh and override registered for a group (tests), or an empty array.
func look(part: int, lod: int, deserted: bool, scheme: int) -> Array:
	return _looks.get(Vector4i(part, lod, int(deserted), scheme), [])

## The drawn batches (`SimulationNode.get_building_lod_batches`): chunk, part, lod, deserted,
## scheme, count and the `MultiMesh` RID of each.
func drawn_batches() -> Array:
	return simulation_node.get_building_lod_batches()

## Mesh and whole-instance override that draw one catalog part, tier and scheme.
## Built once per triple and shared by every chunk that draws it.
##
## Godot exposes per-surface overrides on `MeshInstance3D` only, never on a MultiMesh, so a
## tier with one surface takes an instance override and keeps sharing the source mesh, while
## a multi-material tier needs its own mesh resource to leave untargeted surfaces authored.
## Only parts with scheme overrides pay for that duplicated geometry; window and cloud-shadow
## conversion is already on the shared mesh.
func _scheme_group(part: int, lod: int, scheme: int) -> Dictionary:
	var cache_key := Vector3i(part, lod, scheme)
	if _scheme_variants.has(cache_key):
		return _scheme_variants[cache_key]
	var mesh: Mesh = meshes[part][lod]
	var group := {"mesh": mesh, "override": null}
	var bindings := _scheme_bindings(part, lod, scheme)
	if mesh != null:
		var replacements: Array[Material] = []
		var replaced := 0
		var originals: Array = _source_materials.get(mesh, [])
		for surface in mesh.get_surface_count():
			var source: BaseMaterial3D = originals[surface] if surface < originals.size() else null
			var replacement: Material = null
			if source != null and bindings.has(source.resource_name):
				replacement = WindowMaterials.create(
					SchemeMaterials.variant(source, bindings[source.resource_name]), false,
					float(catalog[part]["window_brightness"]))
			if replacement != null:
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
