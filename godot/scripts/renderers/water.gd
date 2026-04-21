## Water patch renderer — uploads chunk-local visible water patches and world-edge water curtains.
##
## Rust methods called: get_water_patch(), get_dirty_water_patches(), get_water_border_depths(),
##   add_water_source(), is_water_dirty(), clear_water_dirty()
extends Node3D

const WATER_SHADER := preload("res://assets/materials/water.gdshader")
const HEIGHT_SCALE := 20.0
const SHORE_SOFTNESS_M := 0.5
const SHORE_FOAM_BAND_M := 0.5
const SHALLOW_WATER_COLOR := Color(0.20, 0.37, 0.40, 0.58)
const DEEP_WATER_COLOR := Color(0.05, 0.16, 0.29, 0.86)
const WATER_FRESNEL_STRENGTH := 0.24
const WATER_FRESNEL_POWER := 4.0
const WATER_WAVE_COLOR_STRENGTH := 0.025
const WATER_WAVE_ROUGHNESS_STRENGTH := 0.010
const WATER_DISPLAY_SURFACE_SMOOTHING := 0.94
const WATER_DISPLAY_SURFACE_BLEND_RADIUS_TEXELS := 1.0
const WATER_BORDER_MIN_DEPTH_M := 0.02
const WATER_PATCH_EXTRA_CULL_MARGIN_M := 4096.0
const WATER_DEBUG_LOG_INTERVAL_S := 0.5
const WATER_PATCH_MUTATION_BUDGET_PER_FRAME := 256
const WATER_PATCH_PREWARM_BUDGET_PER_FRAME := 12
const WATER_PATCH_MESH_LOD_REFRESH_INTERVAL_S := 0.20
const WATER_PATCH_MESH_LOD_NEAR_DISTANCE_M := 2000.0
const WATER_PATCH_MESH_LOD_MID_DISTANCE_M := 5000.0
const WATER_PATCH_MESH_LOD_FAR_DISTANCE_M := 12000.0

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var patches: Dictionary = {}
var resident_patch_lookup: Dictionary = {}
var patch_mesh_cache: Dictionary = {}
var patch_prewarm_queue: Array[Vector2i] = []
var fallback_height_texture: ImageTexture
var water_border_instance: MeshInstance3D
var water_border_material: ShaderMaterial
var terrain_border_revision: int = -1
var _terrain_debug_enabled: bool = false
var _terrain_debug_verbose: bool = false
var _water_mesh_lod_refresh_elapsed_s: float = 0.0
var _water_debug_elapsed_s: float = 0.0
var _water_debug_frames: int = 0
var _water_debug_frame_ms_total: float = 0.0
var _water_debug_frame_ms_max: float = 0.0
var _water_debug_patch_sync_ms_total: float = 0.0
var _water_debug_upload_ms_total: float = 0.0
var _water_debug_border_ms_total: float = 0.0
var _water_debug_height_rebind_ms_total: float = 0.0
var _water_debug_patch_creates: int = 0
var _water_debug_patch_removes: int = 0
var _water_debug_patch_uploads: int = 0
var _water_debug_height_rebinds: int = 0
var _water_debug_border_rebuilds: int = 0
var _water_debug_residency_changes: int = 0

func _ready() -> void:
	rebuild_from_simulation_state()

func rebuild_from_simulation_state() -> void:
	_terrain_debug_enabled = _terrain_debug_is_enabled()
	_terrain_debug_verbose = _terrain_debug_is_verbose()
	_water_mesh_lod_refresh_elapsed_s = 0.0
	_reset_water_debug_counters()
	_clear_patches()
	_ensure_fallback_height_texture()
	_ensure_water_border_visual()
	_sync_patch_residency(true)
	_sync_patch_height_textures()
	_rebuild_water_border()
	_refresh_terrain_patch_bindings()
	_rebuild_patch_prewarm_queue()
	if _terrain_debug_enabled:
		_water_debug_log("renderer ready")

func _process(delta: float) -> void:
	var frame_start_us := Time.get_ticks_usec()
	var patch_sync_start_us := frame_start_us
	var residency_changed := _sync_patch_residency()
	var patch_sync_elapsed_ms := float(Time.get_ticks_usec() - patch_sync_start_us) / 1000.0
	var height_rebind_elapsed_ms := 0.0
	var upload_elapsed_ms := 0.0
	var border_elapsed_ms := 0.0
	if residency_changed:
		var height_rebind_start_us := Time.get_ticks_usec()
		_sync_patch_height_textures()
		height_rebind_elapsed_ms = float(Time.get_ticks_usec() - height_rebind_start_us) / 1000.0

	var border_changed := _current_terrain_border_revision() != terrain_border_revision
	if simulation_node.is_water_dirty():
		var upload_start_us := Time.get_ticks_usec()
		update_water_visuals()
		upload_elapsed_ms = float(Time.get_ticks_usec() - upload_start_us) / 1000.0
		simulation_node.clear_water_dirty()
	elif border_changed:
		var border_start_us := Time.get_ticks_usec()
		_rebuild_water_border()
		border_elapsed_ms = float(Time.get_ticks_usec() - border_start_us) / 1000.0

	if residency_changed:
		_refresh_terrain_patch_bindings()

	_refresh_patch_mesh_lods(delta)

	handle_water_input(delta)
	if _terrain_debug_enabled:
		var frame_elapsed_ms := float(Time.get_ticks_usec() - frame_start_us) / 1000.0
		_record_water_debug_frame(
			delta,
			frame_elapsed_ms,
			patch_sync_elapsed_ms,
			upload_elapsed_ms,
			border_elapsed_ms,
			height_rebind_elapsed_ms
		)
	if not simulation_node.is_water_dirty():
		_prewarm_patch_cache()

func update_water_visuals() -> void:
	var dirty_keys := _dirty_patch_keys(simulation_node.get_dirty_water_patches())
	if dirty_keys.is_empty():
		for key in get_resident_patch_keys():
			_upload_patch(key)
	else:
		for key in dirty_keys:
			if patches.has(key):
				_upload_patch(key)
	_rebuild_water_border()

func get_resident_patch_keys() -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		keys.append(key)
	return keys

func get_patch_water_texture(key: Vector2i) -> Texture2D:
	if not patches.has(key):
		return null
	return patches[key]["depth_texture"]

func _sync_patch_residency(force_full_sync: bool = false) -> bool:
	if terrain_node == null or not terrain_node.has_method("get_resident_patch_keys"):
		return false

	var desired_keys: Array[Vector2i] = terrain_node.get_resident_patch_keys()
	var desired_lookup: Dictionary = {}
	for key in desired_keys:
		desired_lookup[key] = true

	var keys_to_remove: Array[Vector2i] = []
	for key in get_resident_patch_keys():
		if not desired_lookup.has(key):
			keys_to_remove.append(key)

	var keys_to_add: Array[Vector2i] = []
	for key in desired_keys:
		if not resident_patch_lookup.has(key):
			keys_to_add.append(key)

	if keys_to_add.is_empty() and keys_to_remove.is_empty():
		return false

	var changed := false
	var remaining_budget := WATER_PATCH_MUTATION_BUDGET_PER_FRAME
	if force_full_sync:
		remaining_budget = keys_to_add.size() + keys_to_remove.size()
	var initial_budget := remaining_budget
	for key in keys_to_add:
		if remaining_budget <= 0:
			break
		_activate_patch(key)
		remaining_budget -= 1
		changed = true

	for key in keys_to_remove:
		if remaining_budget <= 0:
			break
		_deactivate_patch(key)
		remaining_budget -= 1
		changed = true

	if changed:
		_water_debug_residency_changes += 1
		if _terrain_debug_verbose:
			var executed_adds: int = min(keys_to_add.size(), initial_budget)
			var executed_removes: int = min(keys_to_remove.size(), max(0, initial_budget - executed_adds))
			_water_debug_log(
				"water residency changed resident=%d desired=%d add_pending=%d remove_pending=%d"
				% [
					resident_patch_lookup.size(),
					desired_keys.size(),
					max(0, keys_to_add.size() - executed_adds),
					max(0, keys_to_remove.size() - executed_removes),
				]
			)
	return changed

func _create_patch(key: Vector2i) -> void:
	if patches.has(key):
		return
	var patch_data: Dictionary = simulation_node.get_water_patch(key.x, key.y)
	if patch_data.is_empty():
		return

	var sample_width := int(patch_data["sample_width"])
	var sample_height := int(patch_data["sample_height"])
	var texture_width := int(patch_data["texture_width"])
	var texture_height := int(patch_data["texture_height"])
	var world_size_x := float(patch_data["world_size_x"])
	var world_size_z := float(patch_data["world_size_z"])
	var world_origin_x := float(patch_data["world_origin_x"])
	var world_origin_z := float(patch_data["world_origin_z"])
	var inner_offset_x := float(patch_data["inner_offset_x"])
	var inner_offset_z := float(patch_data["inner_offset_z"])

	var depth_image: Image = Image.create(texture_width, texture_height, false, Image.FORMAT_RF)
	depth_image.set_data(
		texture_width,
		texture_height,
		false,
		Image.FORMAT_RF,
		(patch_data["depth_data"] as PackedFloat32Array).to_byte_array()
	)
	var depth_texture: ImageTexture = ImageTexture.create_from_image(depth_image)

	var velocity_image: Image = Image.create(texture_width, texture_height, false, Image.FORMAT_RF)
	velocity_image.set_data(
		texture_width,
		texture_height,
		false,
		Image.FORMAT_RF,
		(patch_data["velocity_data"] as PackedFloat32Array).to_byte_array()
	)
	var velocity_texture: ImageTexture = ImageTexture.create_from_image(velocity_image)

	var patch_center_x := world_origin_x + world_size_x * 0.5
	var patch_center_z := world_origin_z + world_size_z * 0.5
	var initial_lod_step := _mesh_lod_step_for_patch_center(patch_center_x, patch_center_z)
	var patch_mesh := _patch_mesh(
		sample_width,
		sample_height,
		world_size_x,
		world_size_z,
		initial_lod_step
	)

	var patch_node: MeshInstance3D = MeshInstance3D.new()
	patch_node.name = "WaterPatch_%d_%d" % [key.x, key.y]
	patch_node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	patch_node.extra_cull_margin = WATER_PATCH_EXTRA_CULL_MARGIN_M
	patch_node.mesh = patch_mesh
	patch_node.visible = false
	patch_node.position = Vector3(
		world_origin_x + world_size_x * 0.5,
		0.0,
		world_origin_z + world_size_z * 0.5
	)

	var material: ShaderMaterial = ShaderMaterial.new()
	material.shader = WATER_SHADER
	material.set_shader_parameter("heightmap", _terrain_height_texture(key))
	material.set_shader_parameter("watermap", depth_texture)
	material.set_shader_parameter("velocity_map", velocity_texture)
	material.set_shader_parameter("height_scale", HEIGHT_SCALE)
	material.set_shader_parameter("shore_softness_m", SHORE_SOFTNESS_M)
	material.set_shader_parameter("shore_foam_band_m", SHORE_FOAM_BAND_M)
	material.set_shader_parameter("shallow_water_color", SHALLOW_WATER_COLOR)
	material.set_shader_parameter("deep_water_color", DEEP_WATER_COLOR)
	material.set_shader_parameter("water_fresnel_strength", WATER_FRESNEL_STRENGTH)
	material.set_shader_parameter("water_fresnel_power", WATER_FRESNEL_POWER)
	material.set_shader_parameter("water_wave_color_strength", WATER_WAVE_COLOR_STRENGTH)
	material.set_shader_parameter("water_wave_roughness_strength", WATER_WAVE_ROUGHNESS_STRENGTH)
	material.set_shader_parameter("water_surface_smoothing", WATER_DISPLAY_SURFACE_SMOOTHING)
	material.set_shader_parameter("water_surface_blend_radius_texels", WATER_DISPLAY_SURFACE_BLEND_RADIUS_TEXELS)
	material.set_shader_parameter("watermap_texture_size", Vector2(texture_width, texture_height))
	material.set_shader_parameter("inner_sample_offset_texels", Vector2(inner_offset_x, inner_offset_z))
	material.set_shader_parameter("inner_sample_size_texels", Vector2(sample_width, sample_height))
	patch_node.material_override = material
	add_child(patch_node)
	_water_debug_patch_creates += 1

	patches[key] = {
		"node": patch_node,
		"material": material,
		"depth_image": depth_image,
		"depth_texture": depth_texture,
		"velocity_image": velocity_image,
		"velocity_texture": velocity_texture,
		"height_texture": material.get_shader_parameter("heightmap"),
		"sample_width": sample_width,
		"sample_height": sample_height,
		"world_size_x": world_size_x,
		"world_size_z": world_size_z,
		"lod_step": initial_lod_step,
	}

func _upload_patch(key: Vector2i) -> void:
	if not patches.has(key):
		return
	var patch_data: Dictionary = simulation_node.get_water_patch(key.x, key.y)
	if patch_data.is_empty():
		_remove_patch(key)
		return

	var patch: Dictionary = patches[key]
	var texture_width := int(patch_data["texture_width"])
	var texture_height := int(patch_data["texture_height"])

	var depth_image: Image = patch["depth_image"]
	var depth_texture: ImageTexture = patch["depth_texture"]
	depth_image.set_data(
		texture_width,
		texture_height,
		false,
		Image.FORMAT_RF,
		(patch_data["depth_data"] as PackedFloat32Array).to_byte_array()
	)
	depth_texture.update(depth_image)

	var velocity_image: Image = patch["velocity_image"]
	var velocity_texture: ImageTexture = patch["velocity_texture"]
	velocity_image.set_data(
		texture_width,
		texture_height,
		false,
		Image.FORMAT_RF,
		(patch_data["velocity_data"] as PackedFloat32Array).to_byte_array()
	)
	velocity_texture.update(velocity_image)
	_water_debug_patch_uploads += 1

	var patch_node: MeshInstance3D = patch["node"]
	var world_size_x := float(patch_data["world_size_x"])
	var world_size_z := float(patch_data["world_size_z"])
	patch_node.position = Vector3(
		float(patch_data["world_origin_x"]) + world_size_x * 0.5,
		0.0,
		float(patch_data["world_origin_z"]) + world_size_z * 0.5
	)

func _activate_patch(key: Vector2i) -> void:
	if resident_patch_lookup.has(key):
		return
	if not patches.has(key):
		_create_patch(key)
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return
	_refresh_one_patch_mesh_lod(key)
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.visible = true
	resident_patch_lookup[key] = true

func _deactivate_patch(key: Vector2i) -> void:
	if not resident_patch_lookup.has(key):
		return
	if not patches.has(key):
		resident_patch_lookup.erase(key)
		return
	var patch: Dictionary = patches[key]
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.visible = false
	resident_patch_lookup.erase(key)

func _remove_patch(key: Vector2i) -> void:
	if not patches.has(key):
		return
	var patch: Dictionary = patches[key]
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.queue_free()
	patches.erase(key)
	resident_patch_lookup.erase(key)
	_water_debug_patch_removes += 1

func _clear_patches() -> void:
	for key in patches.keys():
		var patch: Dictionary = patches[key]
		var patch_node: MeshInstance3D = patch["node"]
		patch_node.queue_free()
	patches.clear()
	resident_patch_lookup.clear()
	patch_prewarm_queue.clear()

func _rebuild_patch_prewarm_queue() -> void:
	patch_prewarm_queue.clear()
	if simulation_node == null:
		return
	var patch_layout: Dictionary = simulation_node.get_terrain_patch_layout()
	var patch_cols := int(patch_layout.get("patch_cols", 0))
	var patch_rows := int(patch_layout.get("patch_rows", 0))
	for patch_z in range(patch_rows):
		for patch_x in range(patch_cols):
			var key := Vector2i(patch_x, patch_z)
			if patches.has(key):
				continue
			patch_prewarm_queue.append(key)

func _prewarm_patch_cache() -> void:
	if patch_prewarm_queue.is_empty():
		return
	var remaining_budget := WATER_PATCH_PREWARM_BUDGET_PER_FRAME
	while remaining_budget > 0 and not patch_prewarm_queue.is_empty():
		var key: Vector2i = patch_prewarm_queue.pop_front()
		if patches.has(key):
			continue
		_create_patch(key)
		var patch: Dictionary = patches.get(key, {})
		if not patch.is_empty():
			var patch_node: MeshInstance3D = patch["node"]
			patch_node.visible = false
		remaining_budget -= 1

func _dirty_patch_keys(flat_pairs: PackedInt32Array) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	var pair_count := flat_pairs.size() / 2
	for index in range(pair_count):
		keys.append(Vector2i(flat_pairs[index * 2], flat_pairs[index * 2 + 1]))
	return keys

func _patch_mesh(
	sample_width: int,
	sample_height: int,
	world_size_x: float,
	world_size_z: float,
	lod_step: int
) -> PlaneMesh:
	var mesh_cache_key := _patch_mesh_cache_key(
		sample_width,
		sample_height,
		world_size_x,
		world_size_z,
		lod_step
	)
	var patch_mesh: PlaneMesh
	if patch_mesh_cache.has(mesh_cache_key):
		patch_mesh = patch_mesh_cache[mesh_cache_key]
	else:
		patch_mesh = PlaneMesh.new()
		patch_mesh.size = Vector2(world_size_x, world_size_z)
		patch_mesh.subdivide_width = _mesh_subdivisions_for_sample_count(sample_width, lod_step)
		patch_mesh.subdivide_depth = _mesh_subdivisions_for_sample_count(sample_height, lod_step)
		patch_mesh_cache[mesh_cache_key] = patch_mesh
	return patch_mesh

func _patch_mesh_cache_key(
	sample_width: int,
	sample_height: int,
	world_size_x: float,
	world_size_z: float,
	lod_step: int
) -> String:
	return "%d:%d:%.3f:%.3f:%d" % [
		sample_width,
		sample_height,
		world_size_x,
		world_size_z,
		lod_step,
	]

func _mesh_subdivisions_for_sample_count(sample_count: int, lod_step: int) -> int:
	var interval_count: int = max(0, sample_count - 1)
	var lod_vertex_count: int = max(
		2,
		int(ceili(float(interval_count) / float(max(1, lod_step)))) + 1
	)
	return max(0, lod_vertex_count - 2)

func _mesh_lod_step_for_patch_center(center_x: float, center_z: float) -> int:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return 1
	var distance_m := camera.global_position.distance_to(Vector3(center_x, 0.0, center_z))
	return _mesh_lod_step_for_distance(distance_m)

func _mesh_lod_step_for_distance(distance_m: float) -> int:
	if distance_m <= WATER_PATCH_MESH_LOD_NEAR_DISTANCE_M:
		return 1
	if distance_m <= WATER_PATCH_MESH_LOD_MID_DISTANCE_M:
		return 2
	if distance_m <= WATER_PATCH_MESH_LOD_FAR_DISTANCE_M:
		return 4
	return 8

func _refresh_patch_mesh_lods(delta: float) -> void:
	if resident_patch_lookup.is_empty():
		_water_mesh_lod_refresh_elapsed_s = 0.0
		return
	_water_mesh_lod_refresh_elapsed_s += delta
	if _water_mesh_lod_refresh_elapsed_s < WATER_PATCH_MESH_LOD_REFRESH_INTERVAL_S:
		return
	_water_mesh_lod_refresh_elapsed_s = 0.0
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		_refresh_one_patch_mesh_lod(key)

func _refresh_one_patch_mesh_lod(key: Vector2i) -> void:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return
	var patch_node: MeshInstance3D = patch["node"]
	var target_lod_step := _mesh_lod_step_for_patch_center(patch_node.position.x, patch_node.position.z)
	var current_lod_step := int(patch.get("lod_step", 1))
	if current_lod_step == target_lod_step:
		return
	patch["lod_step"] = target_lod_step
	patch_node.mesh = _patch_mesh(
		int(patch["sample_width"]),
		int(patch["sample_height"]),
		float(patch["world_size_x"]),
		float(patch["world_size_z"]),
		target_lod_step
	)

func _ensure_fallback_height_texture() -> void:
	if fallback_height_texture != null:
		return
	var image := Image.create(2, 2, false, Image.FORMAT_RF)
	image.fill(Color.BLACK)
	fallback_height_texture = ImageTexture.create_from_image(image)

func _terrain_height_texture(key: Vector2i) -> Texture2D:
	if terrain_node != null and terrain_node.has_method("get_patch_height_texture"):
		var texture: Texture2D = terrain_node.get_patch_height_texture(key)
		if texture != null:
			return texture
	return fallback_height_texture

func _sync_patch_height_textures() -> void:
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		var patch: Dictionary = patches.get(key, {})
		if patch.is_empty():
			continue
		var material: ShaderMaterial = patch["material"]
		var next_texture := _terrain_height_texture(key)
		if patch["height_texture"] == next_texture:
			continue
		patch["height_texture"] = next_texture
		material.set_shader_parameter("heightmap", next_texture)
		_water_debug_height_rebinds += 1

func _refresh_terrain_patch_bindings() -> void:
	if terrain_node != null and terrain_node.has_method("refresh_water_patch_bindings"):
		terrain_node.refresh_water_patch_bindings()

func handle_water_input(delta: float) -> void:
	var input_manager = get_node_or_null("../InputManager")
	if input_manager and input_manager.current_tool == input_manager.Tool.WATER:
		if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
			var mouse_pos = get_viewport().get_mouse_position()
			var camera = get_viewport().get_camera_3d()
			if camera == null:
				return

			var ray_origin = camera.project_ray_origin(mouse_pos)
			var ray_dir = camera.project_ray_normal(mouse_pos)

			var plane := Plane(Vector3.UP, 0.0)
			var intersection = plane.intersects_ray(ray_origin, ray_dir)
			if intersection != null:
				simulation_node.add_water_source(Vector2(intersection.x, intersection.z), 0.5 * delta)

func _ensure_water_border_visual() -> void:
	if water_border_instance == null:
		water_border_instance = MeshInstance3D.new()
		water_border_instance.name = "WaterBorderCurtain"
		water_border_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		water_border_instance.extra_cull_margin = WATER_PATCH_EXTRA_CULL_MARGIN_M
		add_child(water_border_instance)
	if water_border_material == null:
		water_border_material = ShaderMaterial.new()
		water_border_material.shader = load("res://scripts/renderers/water_border.gdshader")
		water_border_material.set_shader_parameter("shallow_water_color", SHALLOW_WATER_COLOR)
		water_border_material.set_shader_parameter("deep_water_color", DEEP_WATER_COLOR)
	water_border_instance.material_override = water_border_material

func _rebuild_water_border() -> void:
	_ensure_water_border_visual()
	if terrain_node == null or not terrain_node.has_method("get_border_loop_positions"):
		water_border_instance.mesh = null
		return

	var border_loop_positions: PackedVector3Array = terrain_node.get_border_loop_positions()
	var border_depths: PackedFloat32Array = simulation_node.get_water_border_depths()
	if border_loop_positions.size() < 4 or border_loop_positions.size() != border_depths.size():
		water_border_instance.mesh = null
		terrain_border_revision = _current_terrain_border_revision()
		return

	var surface_tool := SurfaceTool.new()
	surface_tool.begin(Mesh.PRIMITIVE_TRIANGLES)
	var segment_count := 0
	for index in range(border_loop_positions.size()):
		var next_index := (index + 1) % border_loop_positions.size()
		segment_count += _add_water_border_segment(
			surface_tool,
			border_loop_positions[index],
			border_loop_positions[next_index],
			float(border_depths[index]),
			float(border_depths[next_index])
		)

	if segment_count == 0:
		water_border_instance.mesh = null
	else:
		water_border_instance.mesh = surface_tool.commit()
		water_border_instance.material_override = water_border_material
	_water_debug_border_rebuilds += 1

	terrain_border_revision = _current_terrain_border_revision()

func _add_water_border_segment(
	surface_tool: SurfaceTool,
	terrain_top_a: Vector3,
	terrain_top_b: Vector3,
	depth_a: float,
	depth_b: float
) -> int:
	if depth_a <= WATER_BORDER_MIN_DEPTH_M and depth_b <= WATER_BORDER_MIN_DEPTH_M:
		return 0

	var top_a: Vector3 = terrain_top_a + Vector3(0.0, depth_a + 0.02, 0.0)
	var top_b: Vector3 = terrain_top_b + Vector3(0.0, depth_b + 0.02, 0.0)
	var max_depth: float = max(depth_a, depth_b)
	_add_water_border_quad(surface_tool, top_a, top_b, terrain_top_b, terrain_top_a, depth_a, depth_b, max_depth)
	return 1

func _add_water_border_quad(
	surface_tool: SurfaceTool,
	top0: Vector3,
	top1: Vector3,
	bottom1: Vector3,
	bottom0: Vector3,
	depth0: float,
	depth1: float,
	max_depth: float
) -> void:
	var normal := (top1 - top0).cross(bottom0 - top0).normalized()
	_add_water_border_vertex(surface_tool, top0, normal, Vector2(0.0, 0.0), depth0, max_depth)
	_add_water_border_vertex(surface_tool, top1, normal, Vector2(1.0, 0.0), depth1, max_depth)
	_add_water_border_vertex(surface_tool, bottom1, normal, Vector2(1.0, 1.0), depth1, max_depth)
	_add_water_border_vertex(surface_tool, top0, normal, Vector2(0.0, 0.0), depth0, max_depth)
	_add_water_border_vertex(surface_tool, bottom1, normal, Vector2(1.0, 1.0), depth1, max_depth)
	_add_water_border_vertex(surface_tool, bottom0, normal, Vector2(0.0, 1.0), depth0, max_depth)

func _add_water_border_vertex(
	surface_tool: SurfaceTool,
	position: Vector3,
	normal: Vector3,
	uv: Vector2,
	local_depth_m: float,
	max_depth_m: float
) -> void:
	var encoded_depth := 0.0
	if max_depth_m > 0.001:
		encoded_depth = clamp(local_depth_m / max_depth_m, 0.0, 1.0)
	surface_tool.set_normal(normal)
	surface_tool.set_uv(uv)
	surface_tool.set_color(Color(encoded_depth, 0.0, 0.0, clamp(local_depth_m / 10.0, 0.0, 1.0)))
	surface_tool.add_vertex(position)

func _current_terrain_border_revision() -> int:
	if terrain_node != null and terrain_node.has_method("get_border_revision"):
		return int(terrain_node.get_border_revision())
	return -1

func _terrain_debug_is_enabled() -> bool:
	var explicit_value := OS.get_environment("METRUM_DEBUG_TERRAIN").strip_edges()
	if explicit_value == "1":
		return true
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value.is_empty() or debug_value == "0":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	if filter.is_empty():
		return false
	for entry_variant in filter.split(","):
		var entry := String(entry_variant).strip_edges()
		if entry == "terrain" or entry == "terrain-verbose" or entry == "terrain-full":
			return true
	return false

func _terrain_debug_is_verbose() -> bool:
	var explicit_value := OS.get_environment("METRUM_DEBUG_TERRAIN_VERBOSE").strip_edges()
	if explicit_value == "1":
		return true
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	for entry_variant in filter.split(","):
		var entry := String(entry_variant).strip_edges()
		if entry == "terrain-verbose":
			return true
	return false

func _record_water_debug_frame(
	delta: float,
	frame_elapsed_ms: float,
	patch_sync_elapsed_ms: float,
	upload_elapsed_ms: float,
	border_elapsed_ms: float,
	height_rebind_elapsed_ms: float
) -> void:
	_water_debug_elapsed_s += delta
	_water_debug_frames += 1
	_water_debug_frame_ms_total += frame_elapsed_ms
	_water_debug_frame_ms_max = maxf(_water_debug_frame_ms_max, frame_elapsed_ms)
	_water_debug_patch_sync_ms_total += patch_sync_elapsed_ms
	_water_debug_upload_ms_total += upload_elapsed_ms
	_water_debug_border_ms_total += border_elapsed_ms
	_water_debug_height_rebind_ms_total += height_rebind_elapsed_ms
	if _water_debug_elapsed_s < WATER_DEBUG_LOG_INTERVAL_S:
		return

	var average_frame_ms := _water_debug_frame_ms_total / maxf(1.0, float(_water_debug_frames))
	var average_patch_sync_ms := _water_debug_patch_sync_ms_total / maxf(1.0, float(_water_debug_frames))
	var average_upload_ms := _water_debug_upload_ms_total / maxf(1.0, float(_water_debug_frames))
	var average_border_ms := _water_debug_border_ms_total / maxf(1.0, float(_water_debug_frames))
	var average_height_rebind_ms := _water_debug_height_rebind_ms_total / maxf(1.0, float(_water_debug_frames))
	var lod_summary := _water_debug_lod_summary()

	_water_debug_log(
		"water resident=%d creates=%d removes=%d uploads=%d rebinds=%d border_rebuilds=%d residency_changes=%d lods=%s avg_ms=%.3f max_ms=%.3f patch_sync_ms=%.3f upload_ms=%.3f border_ms=%.3f rebind_ms=%.3f"
		% [
			resident_patch_lookup.size(),
			_water_debug_patch_creates,
			_water_debug_patch_removes,
			_water_debug_patch_uploads,
			_water_debug_height_rebinds,
			_water_debug_border_rebuilds,
			_water_debug_residency_changes,
			lod_summary,
			average_frame_ms,
			_water_debug_frame_ms_max,
			average_patch_sync_ms,
			average_upload_ms,
			average_border_ms,
			average_height_rebind_ms,
		]
	)
	_reset_water_debug_counters()

func _water_debug_lod_summary() -> String:
	var lod1 := 0
	var lod2 := 0
	var lod4 := 0
	var lod8 := 0
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		var patch: Dictionary = patches.get(key, {})
		var lod_step := int(patch.get("lod_step", 1))
		match lod_step:
			1:
				lod1 += 1
			2:
				lod2 += 1
			4:
				lod4 += 1
			_:
				lod8 += 1
	return "1x:%d,2x:%d,4x:%d,8x:%d" % [lod1, lod2, lod4, lod8]

func _reset_water_debug_counters() -> void:
	_water_debug_elapsed_s = 0.0
	_water_debug_frames = 0
	_water_debug_frame_ms_total = 0.0
	_water_debug_frame_ms_max = 0.0
	_water_debug_patch_sync_ms_total = 0.0
	_water_debug_upload_ms_total = 0.0
	_water_debug_border_ms_total = 0.0
	_water_debug_height_rebind_ms_total = 0.0
	_water_debug_patch_creates = 0
	_water_debug_patch_removes = 0
	_water_debug_patch_uploads = 0
	_water_debug_height_rebinds = 0
	_water_debug_border_rebuilds = 0
	_water_debug_residency_changes = 0

func _water_debug_log(message: String) -> void:
	if _terrain_debug_enabled:
		print("[DEBUG:terrain] %s" % message)
