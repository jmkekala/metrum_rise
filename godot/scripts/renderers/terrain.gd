## Terrain patch renderer — uploads chunk-local visual terrain patches and world-edge terrain skirts.
##
## Rust methods called: get_terrain_patch_layout(), get_terrain_patch(), get_dirty_terrain_patches(),
##   get_terrain_border_loop(), get_heightmap_size(), get_terrain_world_size(),
##   is_terrain_dirty(), clear_terrain_dirty(), sculpt_terrain(), intersect_terrain(),
##   get_pollution_image_data(), get_noise_image_data(), get_desirability_image_data()
extends Node3D

const TERRAIN_SHADER := preload("res://assets/materials/terrain.gdshader")
const HEIGHT_SCALE := 20.0
const HILLSHADE_AZIMUTH_DEG := 315.0
const HILLSHADE_ALTITUDE_DEG := 38.0
const HILLSHADE_STRENGTH := 0.58
const HILLSHADE_AMBIENT := 0.24
const HILLSHADE_CONTRAST := 1.35
const HILLSHADE_SHADOW_TINT := Color(0.62, 0.71, 0.77)
const HILLSHADE_LIGHT_TINT := Color(0.97, 0.99, 0.95)
const TERRAIN_MACRO_VARIATION_STRENGTH := 0.10
const TERRAIN_ROCK_SLOPE_START := 0.15
const TERRAIN_ROCK_SLOPE_END := 0.34
const TERRAIN_RELIEF_SAMPLE_RADIUS_TEXELS := 3.0
const TERRAIN_RELIEF_START_M := 2.0
const TERRAIN_RELIEF_END_M := 16.0
const TERRAIN_SHORE_BLEND_STRENGTH := 0.28
const TERRAIN_SHORE_LOOKUP_RADIUS_TEXELS := 1.0
const CLIFF_SLOPE_START := 0.26
const CLIFF_SLOPE_END := 0.44
const CLIFF_RELIEF_START_M := 4.0
const CLIFF_RELIEF_END_M := 14.0
const CLIFF_SAMPLE_RADIUS_TEXELS := 2.25
const CLIFF_LATERAL_SMOOTHING_TEXELS := 1.2
const CLIFF_FACE_STRENGTH := 0.28
const CLIFF_EDGE_STRENGTH := 0.46
const CLIFF_CONTOUR_FADE := 0.78
const CLIFF_FACE_COLOR := Color(0.35, 0.35, 0.32)
const CLIFF_TOP_EDGE_COLOR := Color(0.27, 0.28, 0.22)
const CLIFF_TOE_EDGE_COLOR := Color(0.19, 0.20, 0.18)
const CONTOUR_MINOR_INTERVAL_M := 5.0
const CONTOUR_MAJOR_INTERVAL_M := 25.0
const CONTOUR_MINOR_THICKNESS := 0.95
const CONTOUR_MAJOR_THICKNESS := 1.25
const CONTOUR_MINOR_STRENGTH := 0.14
const CONTOUR_MAJOR_STRENGTH := 0.34
const TERRAIN_BORDER_DEPTH_M := 120.0
const TERRAIN_BORDER_TOP_COLOR := Color(0.42, 0.40, 0.34)
const TERRAIN_BORDER_MID_COLOR := Color(0.33, 0.31, 0.27)
const TERRAIN_BORDER_DEEP_COLOR := Color(0.24, 0.22, 0.20)
const TERRAIN_BORDER_RIM_COLOR := Color(0.65, 0.63, 0.54)
const TERRAIN_BORDER_BOTTOM_COLOR := Color(0.18, 0.17, 0.15)
const TERRAIN_BORDER_BAND_INTERVAL_M := 12.0
const TERRAIN_BORDER_BAND_STRENGTH := 0.08
const TERRAIN_BORDER_CONTOUR_MINOR_COLOR := Color(0.13, 0.19, 0.16)
const TERRAIN_BORDER_CONTOUR_MAJOR_COLOR := Color(0.10, 0.16, 0.14)
const TERRAIN_BORDER_CONTOUR_MINOR_STRENGTH := 0.14
const TERRAIN_BORDER_CONTOUR_MAJOR_STRENGTH := 0.28
const PATCH_RESIDENCY_CULL_FAR_M := 8000.0
const PATCH_EXTRA_CULL_MARGIN_M := 4096.0
const TERRAIN_DEBUG_LOG_INTERVAL_S := 0.5
const PATCH_RESIDENCY_HYSTERESIS_PATCHES := 2
const PATCH_RESIDENCY_MUTATION_BUDGET_PER_FRAME := 256
const PATCH_PREWARM_BUDGET_PER_FRAME := 16
const PATCH_MESH_LOD_REFRESH_INTERVAL_S := 0.20
const PATCH_MESH_LOD_NEAR_DISTANCE_M := 2000.0
const PATCH_MESH_LOD_MID_DISTANCE_M := 5000.0
const PATCH_MESH_LOD_FAR_DISTANCE_M := 12000.0

@onready var simulation_node = $"../SimulationNode"

var overlay_mode: int = 0
var terrain_world_size: Vector2 = Vector2.ZERO
var terrain_cell_m: float = 1.0
var patch_cols: int = 0
var patch_rows: int = 0
var patch_interval_cells: int = 1
var patch_span_m: float = 1.0
var overlay_texture: ImageTexture
var overlay_image: Image
var empty_water_texture: ImageTexture
var patches: Dictionary = {}
var resident_patch_lookup: Dictionary = {}
var patch_mesh_cache: Dictionary = {}
var patch_prewarm_queue: Array[Vector2i] = []
var cached_overlay_mode: int = -1
var border_loop_positions: PackedVector3Array = PackedVector3Array()
var border_revision: int = 0
var border_skirt_instance: MeshInstance3D
var border_bottom_cap_instance: MeshInstance3D
var border_skirt_material: ShaderMaterial
var border_bottom_cap_material: StandardMaterial3D
var _resident_patch_bounds_valid: bool = false
var _resident_min_patch_x: int = 0
var _resident_max_patch_x: int = -1
var _resident_min_patch_z: int = 0
var _resident_max_patch_z: int = -1
var _terrain_debug_enabled: bool = false
var _terrain_debug_verbose: bool = false
var _terrain_force_full_world: bool = false
var _terrain_mesh_lod_refresh_elapsed_s: float = 0.0
var _terrain_debug_elapsed_s: float = 0.0
var _terrain_debug_frames: int = 0
var _terrain_debug_frame_ms_total: float = 0.0
var _terrain_debug_frame_ms_max: float = 0.0
var _terrain_debug_residency_ms_total: float = 0.0
var _terrain_debug_upload_ms_total: float = 0.0
var _terrain_debug_border_ms_total: float = 0.0
var _terrain_debug_water_sync_ms_total: float = 0.0
var _terrain_debug_patch_creates: int = 0
var _terrain_debug_patch_removes: int = 0
var _terrain_debug_patch_uploads: int = 0
var _terrain_debug_residency_changes: int = 0
var _terrain_debug_dirty_batches: int = 0
var _terrain_debug_dirty_patch_total: int = 0
var _terrain_debug_last_cull_far_m: float = 0.0
var _terrain_debug_last_desired_bounds: Dictionary = {}

func _ready() -> void:
	rebuild_from_simulation_state()

func rebuild_from_simulation_state() -> void:
	terrain_world_size = simulation_node.get_terrain_world_size()
	var patch_layout: Dictionary = simulation_node.get_terrain_patch_layout()
	patch_cols = int(patch_layout.get("patch_cols", 0))
	patch_rows = int(patch_layout.get("patch_rows", 0))
	patch_interval_cells = max(1, int(patch_layout.get("patch_interval_cells", 1)))
	terrain_cell_m = float(patch_layout.get("terrain_cell_m", 1.0))
	patch_span_m = terrain_cell_m * float(patch_interval_cells)
	_terrain_debug_enabled = _terrain_debug_is_enabled()
	_terrain_debug_verbose = _terrain_debug_is_verbose()
	_terrain_force_full_world = _terrain_debug_force_full_world()
	_terrain_mesh_lod_refresh_elapsed_s = 0.0
	_reset_terrain_debug_counters()
	_clear_patches()
	_resident_patch_bounds_valid = false
	_ensure_overlay_texture()
	_ensure_empty_water_texture()
	_ensure_border_visuals()
	_sync_patch_residency(true)
	_update_overlay_texture()
	_apply_overlay_mode()
	_rebuild_border_skirt()
	_sync_water_patch_textures()
	cached_overlay_mode = overlay_mode
	_rebuild_patch_prewarm_queue()
	if _terrain_debug_enabled:
		_terrain_debug_log(
			"renderer ready patch_grid=%dx%d patch_span=%.1fm chunk_span=%.1fm force_full_world=%s"
			% [patch_cols, patch_rows, patch_span_m, float(patch_layout.get("chunk_span_m", 0.0)), str(_terrain_force_full_world)]
		)

func _process(delta: float) -> void:
	var frame_start_us := Time.get_ticks_usec()
	var residency_start_us := frame_start_us
	var residency_changed := _sync_patch_residency()
	var residency_elapsed_ms := float(Time.get_ticks_usec() - residency_start_us) / 1000.0
	var upload_elapsed_ms := 0.0
	var border_elapsed_ms := 0.0
	var water_sync_elapsed_ms := 0.0
	if simulation_node.is_terrain_dirty():
		var dirty_start_us := Time.get_ticks_usec()
		var dirty_keys := _dirty_patch_keys(simulation_node.get_dirty_terrain_patches())
		_terrain_debug_dirty_batches += 1
		_terrain_debug_dirty_patch_total += dirty_keys.size()
		for key in dirty_keys:
			if patches.has(key):
				_upload_patch(key)
		upload_elapsed_ms = float(Time.get_ticks_usec() - dirty_start_us) / 1000.0
		var border_start_us := Time.get_ticks_usec()
		_rebuild_border_skirt()
		border_elapsed_ms = float(Time.get_ticks_usec() - border_start_us) / 1000.0
		var water_sync_start_us := Time.get_ticks_usec()
		_sync_water_patch_textures()
		water_sync_elapsed_ms = float(Time.get_ticks_usec() - water_sync_start_us) / 1000.0
		simulation_node.clear_terrain_dirty()
	elif residency_changed:
		var water_sync_start_us := Time.get_ticks_usec()
		_sync_water_patch_textures()
		water_sync_elapsed_ms = float(Time.get_ticks_usec() - water_sync_start_us) / 1000.0

	if overlay_mode != cached_overlay_mode:
		_update_overlay_texture()
		_apply_overlay_mode()
		cached_overlay_mode = overlay_mode

	_refresh_patch_mesh_lods(delta)

	var input_manager = get_node_or_null("../InputManager")
	if input_manager and input_manager.current_tool == input_manager.Tool.SCULPT:
		if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) or Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT):
			_sculpt_at_mouse(delta)

	if _terrain_debug_enabled:
		var frame_elapsed_ms := float(Time.get_ticks_usec() - frame_start_us) / 1000.0
		_record_terrain_debug_frame(
			delta,
			frame_elapsed_ms,
			residency_elapsed_ms,
			upload_elapsed_ms,
			border_elapsed_ms,
			water_sync_elapsed_ms
		)

	if not simulation_node.is_terrain_dirty():
		_prewarm_patch_cache()

func get_resident_patch_keys() -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		keys.append(key)
	return keys

func get_patch_height_texture(key: Vector2i) -> Texture2D:
	if not patches.has(key):
		return null
	return patches[key]["height_texture"]

func get_border_loop_positions() -> PackedVector3Array:
	return border_loop_positions

func get_border_revision() -> int:
	return border_revision

func refresh_water_patch_bindings() -> void:
	_sync_water_patch_textures()

func update_terrain_visuals() -> void:
	var dirty_keys := _dirty_patch_keys(simulation_node.get_dirty_terrain_patches())
	if dirty_keys.is_empty():
		for key in get_resident_patch_keys():
			_upload_patch(key)
	else:
		for key in dirty_keys:
			if patches.has(key):
				_upload_patch(key)
	_rebuild_border_skirt()
	_sync_water_patch_textures()

func _sync_patch_residency(force_full_sync: bool = false) -> bool:
	if patch_cols <= 0 or patch_rows <= 0:
		return false

	var desired_bounds: Dictionary = _desired_patch_bounds()
	_terrain_debug_last_desired_bounds = desired_bounds
	var resident_target_bounds := _expanded_patch_bounds(
		desired_bounds,
		PATCH_RESIDENCY_HYSTERESIS_PATCHES
	)
	var keys_to_add: Array[Vector2i] = []
	for patch_z in range(
		int(resident_target_bounds["min_z"]),
		int(resident_target_bounds["max_z"]) + 1
	):
		for patch_x in range(
			int(resident_target_bounds["min_x"]),
			int(resident_target_bounds["max_x"]) + 1
		):
			var key := Vector2i(patch_x, patch_z)
			if not resident_patch_lookup.has(key):
				keys_to_add.append(key)

	var keys_to_remove: Array[Vector2i] = []
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		if not _patch_key_in_bounds(key, resident_target_bounds):
			keys_to_remove.append(key)

	if keys_to_add.is_empty() and keys_to_remove.is_empty():
		_refresh_resident_patch_bounds()
		return false

	var changed := false
	var remaining_budget := PATCH_RESIDENCY_MUTATION_BUDGET_PER_FRAME
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

	_refresh_resident_patch_bounds()
	if changed:
		_terrain_debug_residency_changes += 1
		if _terrain_debug_verbose:
			var executed_adds: int = min(keys_to_add.size(), initial_budget)
			var executed_removes: int = min(keys_to_remove.size(), max(0, initial_budget - executed_adds))
			_terrain_debug_log(
				"residency changed desired=%s resident=%s resident_count=%d add_pending=%d remove_pending=%d"
				% [
					_terrain_debug_bounds_label(desired_bounds),
					_terrain_debug_current_resident_bounds_label(),
					resident_patch_lookup.size(),
					max(0, keys_to_add.size() - executed_adds),
					max(0, keys_to_remove.size() - executed_removes),
				]
			)
	return changed

func _desired_patch_bounds() -> Dictionary:
	if patch_cols <= 0 or patch_rows <= 0:
		return {"min_x": 0, "max_x": -1, "min_z": 0, "max_z": -1}

	if _terrain_force_full_world:
		_terrain_debug_last_cull_far_m = terrain_world_size.length()
		return {
			"min_x": 0,
			"max_x": patch_cols - 1,
			"min_z": 0,
			"max_z": patch_rows - 1,
		}

	var camera := get_viewport().get_camera_3d()
	if camera == null:
		_terrain_debug_last_cull_far_m = 0.0
		return {
			"min_x": 0,
			"max_x": patch_cols - 1,
			"min_z": 0,
			"max_z": patch_rows - 1,
		}

	return _camera_patch_bounds(camera)

func _camera_patch_bounds(camera: Camera3D) -> Dictionary:
	var cull_far := minf(camera.far, _terrain_patch_cull_far_m(camera))
	_terrain_debug_last_cull_far_m = cull_far
	var viewport_size := get_viewport().get_visible_rect().size
	var corners := [
		Vector2.ZERO,
		Vector2(viewport_size.x, 0.0),
		Vector2(viewport_size.x, viewport_size.y),
		Vector2(0.0, viewport_size.y),
	]
	var min_x := INF
	var max_x := -INF
	var min_z := INF
	var max_z := -INF
	for corner in corners:
		var origin := camera.project_ray_origin(corner)
		var direction := camera.project_ray_normal(corner)
		var distance := cull_far
		if direction.y < -1e-3:
			distance = min(-origin.y / direction.y, cull_far)
		var point := origin + direction * distance
		min_x = min(min_x, point.x)
		max_x = max(max_x, point.x)
		min_z = min(min_z, point.z)
		max_z = max(max_z, point.z)

	var pad := patch_span_m
	var half_world_w := terrain_world_size.x * 0.5
	var half_world_h := terrain_world_size.y * 0.5
	var min_patch_x := clampi(int(floor((min_x - pad + half_world_w) / patch_span_m)), 0, patch_cols - 1)
	var max_patch_x := clampi(int(floor((max_x + pad + half_world_w) / patch_span_m)), 0, patch_cols - 1)
	var min_patch_z := clampi(int(floor((min_z - pad + half_world_h) / patch_span_m)), 0, patch_rows - 1)
	var max_patch_z := clampi(int(floor((max_z + pad + half_world_h) / patch_span_m)), 0, patch_rows - 1)
	return {
		"min_x": min_patch_x,
		"max_x": max_patch_x,
		"min_z": min_patch_z,
		"max_z": max_patch_z,
	}

func _terrain_patch_cull_far_m(camera: Camera3D) -> float:
	var camera_height := absf(camera.global_position.y)
	var height_scaled_far := camera_height * 4.0
	return maxf(PATCH_RESIDENCY_CULL_FAR_M, height_scaled_far)

func _create_patch(key: Vector2i) -> void:
	if patches.has(key):
		return
	var patch_data: Dictionary = simulation_node.get_terrain_patch(key.x, key.y)
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
	var height_image: Image = Image.create(texture_width, texture_height, false, Image.FORMAT_RF)
	height_image.set_data(
		texture_width,
		texture_height,
		false,
		Image.FORMAT_RF,
		(patch_data["height_data"] as PackedFloat32Array).to_byte_array()
	)
	var height_texture: ImageTexture = ImageTexture.create_from_image(height_image)

	var mesh_cache_key := "%d:%d:%.3f:%.3f" % [sample_width, sample_height, world_size_x, world_size_z]
	var patch_mesh: PlaneMesh
	var patch_center_x := world_origin_x + world_size_x * 0.5
	var patch_center_z := world_origin_z + world_size_z * 0.5
	var initial_lod_step := _mesh_lod_step_for_patch_center(patch_center_x, patch_center_z)
	mesh_cache_key = _patch_mesh_cache_key(
		sample_width,
		sample_height,
		world_size_x,
		world_size_z,
		initial_lod_step
	)
	patch_mesh = _patch_mesh(
		sample_width,
		sample_height,
		world_size_x,
		world_size_z,
		initial_lod_step
	)

	var patch_node: MeshInstance3D = MeshInstance3D.new()
	patch_node.name = "TerrainPatch_%d_%d" % [key.x, key.y]
	patch_node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	patch_node.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
	patch_node.mesh = patch_mesh
	patch_node.visible = false
	patch_node.position = Vector3(
		world_origin_x + world_size_x * 0.5,
		0.0,
		world_origin_z + world_size_z * 0.5
	)

	var material: ShaderMaterial = ShaderMaterial.new()
	material.shader = TERRAIN_SHADER
	material.set_shader_parameter("heightmap", height_texture)
	material.set_shader_parameter("overlay_texture", overlay_texture)
	material.set_shader_parameter("watermap", empty_water_texture)
	material.set_shader_parameter("overlay_mode", overlay_mode)
	material.set_shader_parameter("height_scale", HEIGHT_SCALE)
	material.set_shader_parameter("world_size", terrain_world_size)
	material.set_shader_parameter("heightmap_texture_size", Vector2(texture_width, texture_height))
	material.set_shader_parameter("inner_sample_offset_texels", Vector2(inner_offset_x, inner_offset_z))
	material.set_shader_parameter("inner_sample_size_texels", Vector2(sample_width, sample_height))
	material.set_shader_parameter("terrain_cell_m", terrain_cell_m)
	material.set_shader_parameter("hillshade_azimuth_deg", HILLSHADE_AZIMUTH_DEG)
	material.set_shader_parameter("hillshade_altitude_deg", HILLSHADE_ALTITUDE_DEG)
	material.set_shader_parameter("hillshade_strength", HILLSHADE_STRENGTH)
	material.set_shader_parameter("hillshade_ambient", HILLSHADE_AMBIENT)
	material.set_shader_parameter("hillshade_contrast", HILLSHADE_CONTRAST)
	material.set_shader_parameter("hillshade_shadow_tint", HILLSHADE_SHADOW_TINT)
	material.set_shader_parameter("hillshade_light_tint", HILLSHADE_LIGHT_TINT)
	material.set_shader_parameter("terrain_macro_variation_strength", TERRAIN_MACRO_VARIATION_STRENGTH)
	material.set_shader_parameter("terrain_rock_slope_start", TERRAIN_ROCK_SLOPE_START)
	material.set_shader_parameter("terrain_rock_slope_end", TERRAIN_ROCK_SLOPE_END)
	material.set_shader_parameter("terrain_relief_sample_radius_texels", TERRAIN_RELIEF_SAMPLE_RADIUS_TEXELS)
	material.set_shader_parameter("terrain_relief_start_m", TERRAIN_RELIEF_START_M)
	material.set_shader_parameter("terrain_relief_end_m", TERRAIN_RELIEF_END_M)
	material.set_shader_parameter("terrain_shore_blend_strength", TERRAIN_SHORE_BLEND_STRENGTH)
	material.set_shader_parameter("terrain_shore_lookup_radius_texels", TERRAIN_SHORE_LOOKUP_RADIUS_TEXELS)
	material.set_shader_parameter("cliff_slope_start", CLIFF_SLOPE_START)
	material.set_shader_parameter("cliff_slope_end", CLIFF_SLOPE_END)
	material.set_shader_parameter("cliff_relief_start_m", CLIFF_RELIEF_START_M)
	material.set_shader_parameter("cliff_relief_end_m", CLIFF_RELIEF_END_M)
	material.set_shader_parameter("cliff_sample_radius_texels", CLIFF_SAMPLE_RADIUS_TEXELS)
	material.set_shader_parameter("cliff_lateral_smoothing_texels", CLIFF_LATERAL_SMOOTHING_TEXELS)
	material.set_shader_parameter("cliff_face_strength", CLIFF_FACE_STRENGTH)
	material.set_shader_parameter("cliff_edge_strength", CLIFF_EDGE_STRENGTH)
	material.set_shader_parameter("cliff_contour_fade", CLIFF_CONTOUR_FADE)
	material.set_shader_parameter("cliff_face_color", CLIFF_FACE_COLOR)
	material.set_shader_parameter("cliff_top_edge_color", CLIFF_TOP_EDGE_COLOR)
	material.set_shader_parameter("cliff_toe_edge_color", CLIFF_TOE_EDGE_COLOR)
	material.set_shader_parameter("contour_minor_interval_m", CONTOUR_MINOR_INTERVAL_M)
	material.set_shader_parameter("contour_major_interval_m", CONTOUR_MAJOR_INTERVAL_M)
	material.set_shader_parameter("contour_minor_thickness", CONTOUR_MINOR_THICKNESS)
	material.set_shader_parameter("contour_major_thickness", CONTOUR_MAJOR_THICKNESS)
	material.set_shader_parameter("contour_minor_strength", CONTOUR_MINOR_STRENGTH)
	material.set_shader_parameter("contour_major_strength", CONTOUR_MAJOR_STRENGTH)
	patch_node.material_override = material
	add_child(patch_node)
	_terrain_debug_patch_creates += 1

	patches[key] = {
		"node": patch_node,
		"material": material,
		"height_image": height_image,
		"height_texture": height_texture,
		"water_texture": empty_water_texture,
		"sample_width": sample_width,
		"sample_height": sample_height,
		"world_size_x": world_size_x,
		"world_size_z": world_size_z,
		"lod_step": initial_lod_step,
	}

func _upload_patch(key: Vector2i) -> void:
	if not patches.has(key):
		return
	var patch_data: Dictionary = simulation_node.get_terrain_patch(key.x, key.y)
	if patch_data.is_empty():
		_remove_patch(key)
		return
	var patch: Dictionary = patches[key]
	var texture_width := int(patch_data["texture_width"])
	var texture_height := int(patch_data["texture_height"])
	var height_image: Image = patch["height_image"]
	var height_texture: ImageTexture = patch["height_texture"]
	height_image.set_data(
		texture_width,
		texture_height,
		false,
		Image.FORMAT_RF,
		(patch_data["height_data"] as PackedFloat32Array).to_byte_array()
	)
	height_texture.update(height_image)
	_terrain_debug_patch_uploads += 1

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
	_terrain_debug_patch_removes += 1

func _patch_key_in_bounds(key: Vector2i, bounds: Dictionary) -> bool:
	return (
		key.x >= int(bounds["min_x"])
		and key.x <= int(bounds["max_x"])
		and key.y >= int(bounds["min_z"])
		and key.y <= int(bounds["max_z"])
	)

func _expanded_patch_bounds(bounds: Dictionary, margin_patches: int) -> Dictionary:
	return {
		"min_x": max(0, int(bounds["min_x"]) - margin_patches),
		"max_x": min(patch_cols - 1, int(bounds["max_x"]) + margin_patches),
		"min_z": max(0, int(bounds["min_z"]) - margin_patches),
		"max_z": min(patch_rows - 1, int(bounds["max_z"]) + margin_patches),
	}

func _refresh_resident_patch_bounds() -> void:
	if resident_patch_lookup.is_empty():
		_resident_patch_bounds_valid = false
		_resident_min_patch_x = 0
		_resident_max_patch_x = -1
		_resident_min_patch_z = 0
		_resident_max_patch_z = -1
		return

	var min_patch_x := patch_cols - 1
	var max_patch_x := 0
	var min_patch_z := patch_rows - 1
	var max_patch_z := 0
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		min_patch_x = min(min_patch_x, key.x)
		max_patch_x = max(max_patch_x, key.x)
		min_patch_z = min(min_patch_z, key.y)
		max_patch_z = max(max_patch_z, key.y)

	_resident_patch_bounds_valid = true
	_resident_min_patch_x = min_patch_x
	_resident_max_patch_x = max_patch_x
	_resident_min_patch_z = min_patch_z
	_resident_max_patch_z = max_patch_z

func _clear_patches() -> void:
	for key in patches.keys():
		var patch: Dictionary = patches[key]
		var patch_node: MeshInstance3D = patch["node"]
		patch_node.queue_free()
	patches.clear()
	resident_patch_lookup.clear()
	patch_prewarm_queue.clear()
	_resident_patch_bounds_valid = false

func _rebuild_patch_prewarm_queue() -> void:
	patch_prewarm_queue.clear()
	for patch_z in range(patch_rows):
		for patch_x in range(patch_cols):
			var key := Vector2i(patch_x, patch_z)
			if patches.has(key):
				continue
			patch_prewarm_queue.append(key)

func _prewarm_patch_cache() -> void:
	if patch_prewarm_queue.is_empty():
		return
	var remaining_budget := PATCH_PREWARM_BUDGET_PER_FRAME
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
	if distance_m <= PATCH_MESH_LOD_NEAR_DISTANCE_M:
		return 1
	if distance_m <= PATCH_MESH_LOD_MID_DISTANCE_M:
		return 2
	if distance_m <= PATCH_MESH_LOD_FAR_DISTANCE_M:
		return 4
	return 8

func _refresh_patch_mesh_lods(delta: float) -> void:
	if resident_patch_lookup.is_empty():
		_terrain_mesh_lod_refresh_elapsed_s = 0.0
		return
	_terrain_mesh_lod_refresh_elapsed_s += delta
	if _terrain_mesh_lod_refresh_elapsed_s < PATCH_MESH_LOD_REFRESH_INTERVAL_S:
		return
	_terrain_mesh_lod_refresh_elapsed_s = 0.0
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

func _ensure_overlay_texture() -> void:
	var dims: Vector2 = simulation_node.get_heightmap_size()
	overlay_image = Image.create(int(dims.x), int(dims.y), false, Image.FORMAT_RGBA8)
	overlay_texture = ImageTexture.create_from_image(overlay_image)

func _update_overlay_texture() -> void:
	if overlay_image == null:
		return
	var dims: Vector2 = simulation_node.get_heightmap_size()
	var overlay_bytes := PackedByteArray()
	if overlay_mode == 1:
		overlay_bytes = simulation_node.get_pollution_image_data()
	elif overlay_mode == 2:
		overlay_bytes = simulation_node.get_noise_image_data()
	elif overlay_mode == 3:
		overlay_bytes = simulation_node.get_desirability_image_data()

	if overlay_bytes.is_empty():
		overlay_image.fill(Color(0.0, 0.0, 0.0, 0.0))
	else:
		overlay_image.set_data(int(dims.x), int(dims.y), false, Image.FORMAT_RGBA8, overlay_bytes)
	overlay_texture.update(overlay_image)

func _apply_overlay_mode() -> void:
	for key in patches.keys():
		var material: ShaderMaterial = patches[key]["material"]
		material.set_shader_parameter("overlay_mode", overlay_mode)
		material.set_shader_parameter("overlay_texture", overlay_texture)

func _ensure_empty_water_texture() -> void:
	if empty_water_texture != null:
		return
	var image := Image.create(2, 2, false, Image.FORMAT_RF)
	image.fill(Color.BLACK)
	empty_water_texture = ImageTexture.create_from_image(image)

func _sync_water_patch_textures() -> void:
	var water_node: Node = get_node_or_null("../Water")
	if water_node == null or not water_node.has_method("get_patch_water_texture"):
		return
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		var patch: Dictionary = patches.get(key, {})
		if patch.is_empty():
			continue
		var material: ShaderMaterial = patch["material"]
		var next_texture: Texture2D = water_node.get_patch_water_texture(key)
		if next_texture == null:
			next_texture = empty_water_texture
		if patch["water_texture"] == next_texture:
			continue
		patch["water_texture"] = next_texture
		material.set_shader_parameter("watermap", next_texture)

func _sculpt_at_mouse(delta: float) -> void:
	var mouse_pos := get_viewport().get_mouse_position()
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return

	var ray_origin := camera.project_ray_origin(mouse_pos)
	var ray_dir := camera.project_ray_normal(mouse_pos)
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if intersection == null:
		return

	var strength := 2.0 * delta
	if Input.is_key_pressed(KEY_CTRL) or Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT):
		strength = -2.0 * delta
	simulation_node.sculpt_terrain(Vector2(intersection.x, intersection.z), 15.0, strength)

	var road_tool = get_node_or_null("../RoadTool")
	if road_tool:
		road_tool.update_main_mesh()

func _ensure_border_visuals() -> void:
	if border_skirt_instance == null:
		border_skirt_instance = MeshInstance3D.new()
		border_skirt_instance.name = "TerrainBorderSkirt"
		border_skirt_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		border_skirt_instance.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
		add_child(border_skirt_instance)
	if border_bottom_cap_instance == null:
		border_bottom_cap_instance = MeshInstance3D.new()
		border_bottom_cap_instance.name = "TerrainBorderBottomCap"
		border_bottom_cap_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		border_bottom_cap_instance.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
		add_child(border_bottom_cap_instance)
	if border_skirt_material == null:
		border_skirt_material = ShaderMaterial.new()
		border_skirt_material.shader = load("res://scripts/renderers/terrain_border.gdshader")
		border_skirt_material.set_shader_parameter("skirt_depth_m", TERRAIN_BORDER_DEPTH_M)
		border_skirt_material.set_shader_parameter("top_color", TERRAIN_BORDER_TOP_COLOR)
		border_skirt_material.set_shader_parameter("mid_color", TERRAIN_BORDER_MID_COLOR)
		border_skirt_material.set_shader_parameter("deep_color", TERRAIN_BORDER_DEEP_COLOR)
		border_skirt_material.set_shader_parameter("rim_color", TERRAIN_BORDER_RIM_COLOR)
		border_skirt_material.set_shader_parameter("band_interval_m", TERRAIN_BORDER_BAND_INTERVAL_M)
		border_skirt_material.set_shader_parameter("band_strength", TERRAIN_BORDER_BAND_STRENGTH)
		border_skirt_material.set_shader_parameter("contour_minor_interval_m", CONTOUR_MINOR_INTERVAL_M)
		border_skirt_material.set_shader_parameter("contour_major_interval_m", CONTOUR_MAJOR_INTERVAL_M)
		border_skirt_material.set_shader_parameter("contour_minor_color", TERRAIN_BORDER_CONTOUR_MINOR_COLOR)
		border_skirt_material.set_shader_parameter("contour_major_color", TERRAIN_BORDER_CONTOUR_MAJOR_COLOR)
		border_skirt_material.set_shader_parameter("contour_minor_strength", TERRAIN_BORDER_CONTOUR_MINOR_STRENGTH)
		border_skirt_material.set_shader_parameter("contour_major_strength", TERRAIN_BORDER_CONTOUR_MAJOR_STRENGTH)
	if border_bottom_cap_material == null:
		border_bottom_cap_material = StandardMaterial3D.new()
		border_bottom_cap_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		border_bottom_cap_material.albedo_color = TERRAIN_BORDER_BOTTOM_COLOR
		border_bottom_cap_material.cull_mode = BaseMaterial3D.CULL_FRONT
	border_skirt_instance.material_override = border_skirt_material
	border_bottom_cap_instance.material_override = border_bottom_cap_material

func _rebuild_border_skirt() -> void:
	_ensure_border_visuals()
	border_loop_positions = simulation_node.get_terrain_border_loop()
	if border_loop_positions.size() < 4:
		border_skirt_instance.mesh = null
		border_bottom_cap_instance.mesh = null
		return

	var min_edge_y := INF
	for position in border_loop_positions:
		min_edge_y = min(min_edge_y, position.y)

	var bottom_y := min_edge_y - TERRAIN_BORDER_DEPTH_M
	var surface_tool := SurfaceTool.new()
	surface_tool.begin(Mesh.PRIMITIVE_TRIANGLES)
	var perimeter_u := 0.0
	for index in range(border_loop_positions.size()):
		var next_index := (index + 1) % border_loop_positions.size()
		var top_a: Vector3 = border_loop_positions[index]
		var top_b: Vector3 = border_loop_positions[next_index]
		var bottom_a := Vector3(top_a.x, bottom_y, top_a.z)
		var bottom_b := Vector3(top_b.x, bottom_y, top_b.z)
		var segment_length := top_a.distance_to(top_b)
		_add_skirt_quad(surface_tool, top_a, top_b, bottom_b, bottom_a, perimeter_u, perimeter_u + segment_length)
		perimeter_u += segment_length

	border_skirt_instance.mesh = surface_tool.commit()
	border_skirt_instance.material_override = border_skirt_material
	var bottom_cap := PlaneMesh.new()
	bottom_cap.size = terrain_world_size
	border_bottom_cap_instance.mesh = bottom_cap
	border_bottom_cap_instance.position = Vector3(0.0, bottom_y, 0.0)
	border_bottom_cap_instance.material_override = border_bottom_cap_material
	border_revision += 1

func _add_skirt_quad(
	surface_tool: SurfaceTool,
	top_a: Vector3,
	top_b: Vector3,
	bottom_b: Vector3,
	bottom_a: Vector3,
	u0: float,
	u1: float
) -> void:
	var normal := (top_b - top_a).cross(bottom_a - top_a).normalized()
	_add_skirt_vertex(surface_tool, top_a, normal, Vector2(u0, 0.0))
	_add_skirt_vertex(surface_tool, top_b, normal, Vector2(u1, 0.0))
	_add_skirt_vertex(surface_tool, bottom_b, normal, Vector2(u1, 1.0))
	_add_skirt_vertex(surface_tool, top_a, normal, Vector2(u0, 0.0))
	_add_skirt_vertex(surface_tool, bottom_b, normal, Vector2(u1, 1.0))
	_add_skirt_vertex(surface_tool, bottom_a, normal, Vector2(u0, 1.0))

func _add_skirt_vertex(surface_tool: SurfaceTool, position: Vector3, normal: Vector3, uv: Vector2) -> void:
	surface_tool.set_normal(normal)
	surface_tool.set_uv(uv)
	surface_tool.add_vertex(position)

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

func _terrain_debug_force_full_world() -> bool:
	var explicit_value := OS.get_environment("METRUM_DEBUG_TERRAIN_FORCE_FULL_WORLD").strip_edges()
	if explicit_value == "1":
		return true
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	for entry_variant in filter.split(","):
		var entry := String(entry_variant).strip_edges()
		if entry == "terrain-full":
			return true
	return false

func _record_terrain_debug_frame(
	delta: float,
	frame_elapsed_ms: float,
	residency_elapsed_ms: float,
	upload_elapsed_ms: float,
	border_elapsed_ms: float,
	water_sync_elapsed_ms: float
) -> void:
	_terrain_debug_elapsed_s += delta
	_terrain_debug_frames += 1
	_terrain_debug_frame_ms_total += frame_elapsed_ms
	_terrain_debug_frame_ms_max = maxf(_terrain_debug_frame_ms_max, frame_elapsed_ms)
	_terrain_debug_residency_ms_total += residency_elapsed_ms
	_terrain_debug_upload_ms_total += upload_elapsed_ms
	_terrain_debug_border_ms_total += border_elapsed_ms
	_terrain_debug_water_sync_ms_total += water_sync_elapsed_ms
	if _terrain_debug_elapsed_s < TERRAIN_DEBUG_LOG_INTERVAL_S:
		return

	var desired_patch_count := _terrain_debug_patch_count_for_bounds(_terrain_debug_last_desired_bounds)
	var desired_bounds_label := _terrain_debug_bounds_label(_terrain_debug_last_desired_bounds)
	var resident_bounds_label := _terrain_debug_current_resident_bounds_label()
	var resident_count := resident_patch_lookup.size()
	var patch_capacity := patch_cols * patch_rows
	var average_frame_ms := _terrain_debug_frame_ms_total / maxf(1.0, float(_terrain_debug_frames))
	var average_residency_ms := _terrain_debug_residency_ms_total / maxf(1.0, float(_terrain_debug_frames))
	var average_upload_ms := _terrain_debug_upload_ms_total / maxf(1.0, float(_terrain_debug_frames))
	var average_border_ms := _terrain_debug_border_ms_total / maxf(1.0, float(_terrain_debug_frames))
	var average_water_sync_ms := _terrain_debug_water_sync_ms_total / maxf(1.0, float(_terrain_debug_frames))
	var lod_summary := _terrain_debug_lod_summary()
	var camera := get_viewport().get_camera_3d()
	var camera_label := "none"
	if camera != null:
		camera_label = "(%.1f, %.1f, %.1f)" % [
			camera.global_position.x,
			camera.global_position.y,
			camera.global_position.z,
		]

	_terrain_debug_log(
		"fps=%d cam=%s resident=%d/%d desired=%d desired_bounds=%s resident_bounds=%s cull_far=%.1f residency_changes=%d creates=%d removes=%d uploads=%d dirty_batches=%d dirty_patches=%d lods=%s avg_ms=%.3f max_ms=%.3f residency_ms=%.3f upload_ms=%.3f border_ms=%.3f water_sync_ms=%.3f force_full_world=%s"
		% [
			Engine.get_frames_per_second(),
			camera_label,
			resident_count,
			patch_capacity,
			desired_patch_count,
			desired_bounds_label,
			resident_bounds_label,
			_terrain_debug_last_cull_far_m,
			_terrain_debug_residency_changes,
			_terrain_debug_patch_creates,
			_terrain_debug_patch_removes,
			_terrain_debug_patch_uploads,
			_terrain_debug_dirty_batches,
			_terrain_debug_dirty_patch_total,
			lod_summary,
			average_frame_ms,
			_terrain_debug_frame_ms_max,
			average_residency_ms,
			average_upload_ms,
			average_border_ms,
			average_water_sync_ms,
			str(_terrain_force_full_world),
		]
	)
	_reset_terrain_debug_counters()

func _terrain_debug_lod_summary() -> String:
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

func _terrain_debug_patch_count_for_bounds(bounds: Dictionary) -> int:
	if bounds.is_empty():
		return 0
	var min_x: int = int(bounds.get("min_x", 0))
	var max_x: int = int(bounds.get("max_x", -1))
	var min_z: int = int(bounds.get("min_z", 0))
	var max_z: int = int(bounds.get("max_z", -1))
	if max_x < min_x or max_z < min_z:
		return 0
	return (max_x - min_x + 1) * (max_z - min_z + 1)

func _terrain_debug_bounds_label(bounds: Dictionary) -> String:
	if bounds.is_empty():
		return "none"
	return "[%d..%d,%d..%d]" % [
		int(bounds.get("min_x", 0)),
		int(bounds.get("max_x", -1)),
		int(bounds.get("min_z", 0)),
		int(bounds.get("max_z", -1)),
	]

func _terrain_debug_current_resident_bounds_label() -> String:
	if not _resident_patch_bounds_valid:
		return "none"
	return "[%d..%d,%d..%d]" % [
		_resident_min_patch_x,
		_resident_max_patch_x,
		_resident_min_patch_z,
		_resident_max_patch_z,
	]

func _reset_terrain_debug_counters() -> void:
	_terrain_debug_elapsed_s = 0.0
	_terrain_debug_frames = 0
	_terrain_debug_frame_ms_total = 0.0
	_terrain_debug_frame_ms_max = 0.0
	_terrain_debug_residency_ms_total = 0.0
	_terrain_debug_upload_ms_total = 0.0
	_terrain_debug_border_ms_total = 0.0
	_terrain_debug_water_sync_ms_total = 0.0
	_terrain_debug_patch_creates = 0
	_terrain_debug_patch_removes = 0
	_terrain_debug_patch_uploads = 0
	_terrain_debug_residency_changes = 0
	_terrain_debug_dirty_batches = 0
	_terrain_debug_dirty_patch_total = 0

func _terrain_debug_log(message: String) -> void:
	if _terrain_debug_enabled:
		print("[DEBUG:terrain] %s" % message)
