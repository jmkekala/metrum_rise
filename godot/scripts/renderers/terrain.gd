## Terrain patch renderer — uploads chunk-local visual terrain patches and world-edge terrain skirts.
##
## Rust methods called: get_terrain_patch_layout(), get_terrain_patch(), get_dirty_terrain_patches(),
##   request_terrain_patch_payloads(), poll_ready_terrain_patch_payloads(),
##   get_refined_terrain_patch(), get_refined_terrain_patch_debug(),
##   get_terrain_border_loop(), get_heightmap_size(), get_terrain_world_size(),
##   is_terrain_dirty(), clear_terrain_dirty(), sculpt_terrain(), intersect_terrain(),
##   get_pollution_image_data(), get_noise_image_data(),
##   get_desirability_image_data()
extends Node3D

const TERRAIN_SHADER := preload("res://assets/materials/terrain.gdshader")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const TERRAIN_GRASS_ALBEDO_PATH := "res://assets/textures/general/grass/Grass002_2K_Runtime/grass002_2k_albedo.jpg"
const TERRAIN_GRASS_HEIGHT_PATH := "res://assets/textures/general/grass/Grass002_2K_Runtime/grass002_2k_height.jpg"
const HEIGHT_SCALE := 20.0
const HILLSHADE_AZIMUTH_DEG := 315.0
const HILLSHADE_ALTITUDE_DEG := 38.0
const HILLSHADE_STRENGTH := 0.22
const HILLSHADE_AMBIENT := 0.62
const HILLSHADE_CONTRAST := 1.10
const HILLSHADE_SHADOW_TINT := Color(0.82, 0.88, 0.90)
const HILLSHADE_LIGHT_TINT := Color(1.00, 0.99, 0.94)
const TERRAIN_MACRO_VARIATION_STRENGTH := 0.10
const TERRAIN_GRASS_TINT := Color(0.22, 0.42, 0.16)
const TERRAIN_GRASS_TINT_STRENGTH := 0.0
const TERRAIN_GRASS_ALBEDO_STRENGTH := 0.90
const TERRAIN_GRASS_MACRO_SCALE := 0.018
const TERRAIN_GRASS_MID_SCALE := 0.065
const TERRAIN_GRASS_MACRO_STRENGTH := 0.58
const TERRAIN_GRASS_MID_STRENGTH := 0.80
const TERRAIN_GRASS_MICRO_STRENGTH := 0.50
const TERRAIN_NATURAL_VARIATION_STRENGTH := 0.18
const TERRAIN_MEADOW_MOTTLE_STRENGTH := 0.08
const TERRAIN_BAKED_NORMAL_BLEND := 0.0
const TERRAIN_BAKED_READABILITY_STRENGTH := 0.12
const TERRAIN_GRASS_DETAIL_SCALE := 0.34
const TERRAIN_GRASS_DETAIL_STRENGTH := 0.58
const TERRAIN_GRASS_HEIGHT_DETAIL_STRENGTH := 0.24
const TERRAIN_GRASS_DETAIL_FADE_START := 0.08
const TERRAIN_GRASS_DETAIL_FADE_END := 0.90
const TERRAIN_ROCK_SLOPE_START := 0.15
const TERRAIN_ROCK_SLOPE_END := 0.34
const TERRAIN_RELIEF_SAMPLE_RADIUS_TEXELS := 3.0
const TERRAIN_RELIEF_START_M := 2.0
const TERRAIN_RELIEF_END_M := 16.0
const TERRAIN_SHORE_BLEND_STRENGTH := 0.15
const TERRAIN_SHORE_LOOKUP_RADIUS_TEXELS := 0.55
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
const CONTOUR_RELIEF_MINOR_BOOST_STRENGTH := 0.10
const CONTOUR_ZERO_ELEVATION_FADE_M := 0.75
const CONTOUR_FLAT_RELIEF_START_M := 0.10
const CONTOUR_FLAT_RELIEF_END_M := 1.25
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
const RETAINING_WALL_COLOR := Color(0.54, 0.54, 0.50)
const RETAINING_WALL_ROUGHNESS := 0.88
const PATCH_RESIDENCY_CULL_FAR_M := 8000.0
const PATCH_EXTRA_CULL_MARGIN_M := 4096.0
const TERRAIN_DEBUG_LOG_INTERVAL_S := 0.5
const PATCH_RESIDENCY_HYSTERESIS_PATCHES := 2
const PATCH_RESIDENCY_MUTATION_MAX_PER_FRAME := 256
const PATCH_RESIDENCY_ADD_ATTEMPT_MAX_PER_FRAME := 64
const PATCH_RESIDENCY_ADD_APPLY_MAX_PER_FRAME := 2
const PATCH_RESIDENCY_MUTATION_BUDGET_MS := 1.5
const PATCH_RESOURCE_POOL_PREWARM_COUNT := 64
const PATCH_RESOURCE_POOL_MAX := 96
const PATCH_PAYLOAD_REQUEST_BUDGET_PER_FRAME := 16
const REFINED_PATCH_PAYLOAD_REQUEST_BUDGET_PER_FRAME := 2
const PATCH_PAYLOAD_POLL_BUDGET_PER_FRAME := 64
const PATCH_PREWARM_MAX_PER_FRAME := 4
const PATCH_PREWARM_BUDGET_MS := 0.75
const PATCH_PREWARM_HALO_PATCHES := 1
const PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME := 128
const PATCH_WATER_TEXTURE_SYNC_BUDGET_MS := 1.0
const PATCH_LOD_START_HEADROOM_MS := 1.25
const PATCH_PREWARM_START_HEADROOM_MS := 1.75
const PATCH_MESH_LOD_REFRESH_INTERVAL_S := 0.20
const PATCH_MESH_LOD_REFRESH_BUDGET_MS := 1.0
const PATCH_MESH_LOD_REFRESH_CAMERA_MOVE_M := 96.0
const PATCH_MESH_LOD_REFRESH_MAX_CHECKS_PER_FRAME := 32
const PATCH_MESH_LOD_REFRESH_MAX_CHANGES_PER_FRAME := 1
const PATCH_MESH_LOD_NEAR_DISTANCE_M := 2000.0
const PATCH_MESH_LOD_MID_DISTANCE_M := 5000.0
const PATCH_MESH_LOD_FAR_DISTANCE_M := 12000.0
const ROAD_LOCKED_PATCH_TARGET_RENDER_STEP_M := 2.0
const TERRAIN_CDT_CONTRACT_REVISION := 2
const ROAD_GEOMETRY_TERRAIN_SEAM_SAMPLE_LOG_LIMIT := 4
const ROAD_CLIP_LOOP_ROLE_OUTER := 0
const ROAD_CLIP_LOOP_ROLE_HOLE := 1

@onready var simulation_node = $"../SimulationNode"
@onready var water_node = $"../Water"

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
var grass_albedo_texture: Texture2D
var grass_height_texture: Texture2D
var patches: Dictionary = {}
var resident_patch_lookup: Dictionary = {}
var patch_payload_requested: Dictionary = {}
var patch_payload_requested_generation: Dictionary = {}
var patch_payload_ready: Dictionary = {}
var patch_payload_surface_generation: int = -1
var patch_mesh_cache: Dictionary = {}
var patch_resource_pool: Array[Dictionary] = []
var patch_prewarm_queue: Array[Vector2i] = []
var patch_lod_refresh_queue: Array[Vector2i] = []
var patch_lod_refresh_lookup: Dictionary = {}
var water_texture_sync_queue: Array[Vector2i] = []
var water_texture_sync_lookup: Dictionary = {}
var road_locked_patch_lookup: Dictionary = {}
var cached_overlay_mode: int = -1
var border_loop_positions: PackedVector3Array = PackedVector3Array()
var border_revision: int = 0
var border_skirt_instance: MeshInstance3D
var border_bottom_cap_instance: MeshInstance3D
var border_skirt_material: ShaderMaterial
var border_bottom_cap_material: StandardMaterial3D
var retaining_wall_material: StandardMaterial3D
var _resident_patch_bounds_valid: bool = false
var _resident_min_patch_x: int = 0
var _resident_max_patch_x: int = -1
var _resident_min_patch_z: int = 0
var _resident_max_patch_z: int = -1
var _terrain_debug_enabled: bool = false
var _terrain_debug_verbose: bool = false
var _terrain_force_full_world: bool = false
var _terrain_force_lod1: bool = false
var _road_debug_enabled: bool = false
var _road_geometry_debug_enabled: bool = false
var _terrain_mesh_lod_refresh_elapsed_s: float = 0.0
var _terrain_lod_refresh_camera_valid: bool = false
var _terrain_lod_refresh_last_camera_position: Vector3 = Vector3.ZERO
var _terrain_lod_last_processed_count: int = 0
var _terrain_lod_last_changed_count: int = 0
var _terrain_lod_last_queued_count: int = 0
var _terrain_lod_last_queue_count: int = 0
var _terrain_lod_last_replaced_count: int = 0
var _terrain_lod_last_skipped_count: int = 0
var _terrain_lod_last_deferred_count: int = 0
var _terrain_prewarm_last_deferred_count: int = 0
var _terrain_residency_pending_mutations: bool = false
var _terrain_residency_target_bounds_valid: bool = false
var _terrain_residency_target_bounds: Dictionary = {}
var _terrain_resident_patch_revision: int = 0
var _terrain_residency_last_add_count: int = 0
var _terrain_residency_last_remove_count: int = 0
var _terrain_residency_last_add_pending_count: int = 0
var _terrain_residency_last_remove_pending_count: int = 0
var _terrain_resource_pool_hit_count: int = 0
var _terrain_resource_pool_miss_count: int = 0
var _terrain_resource_pool_release_count: int = 0
var _terrain_resource_pool_prewarm_count: int = 0
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
var _terrain_visual_debug_mode: int = 0
var _terrain_grass_visual_debug_mode: int = 0

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
	_terrain_force_lod1 = _terrain_debug_force_lod1()
	_road_debug_enabled = _road_debug_is_enabled()
	_road_geometry_debug_enabled = _road_geometry_debug_is_enabled()
	_terrain_visual_debug_mode = _terrain_visual_debug_mode_from_env()
	_terrain_grass_visual_debug_mode = _terrain_grass_visual_debug_mode_from_env()
	_terrain_mesh_lod_refresh_elapsed_s = 0.0
	_terrain_lod_refresh_camera_valid = false
	_record_lod_perf_counters(0, 0, 0, 0)
	_terrain_lod_last_deferred_count = 0
	_terrain_prewarm_last_deferred_count = 0
	_reset_terrain_debug_counters()
	_clear_patches()
	_prewarm_regular_terrain_mesh_variants()
	_resident_patch_bounds_valid = false
	_ensure_overlay_texture()
	_ensure_empty_water_texture()
	_ensure_grass_textures()
	_ensure_border_visuals()
	_prewarm_terrain_patch_resource_pool()
	_refresh_road_locked_patch_lookup()
	_sync_patch_residency(true)
	_update_overlay_texture()
	_apply_overlay_mode()
	_rebuild_border_skirt()
	_queue_all_water_patch_texture_syncs()
	_process_water_patch_texture_sync_queue(PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME)
	cached_overlay_mode = overlay_mode
	_rebuild_patch_prewarm_queue()
	if _terrain_debug_enabled:
		_terrain_debug_log(
			"renderer ready patch_grid=%dx%d patch_span=%.1fm chunk_span=%.1fm force_full_world=%s force_lod1=%s visual=%d"
			% [
				patch_cols,
				patch_rows,
				patch_span_m,
				float(patch_layout.get("chunk_span_m", 0.0)),
				str(_terrain_force_full_world),
				str(_terrain_force_lod1),
				_terrain_visual_debug_mode,
			]
		)
	if _terrain_visual_debug_mode != 0:
		print(
			"[DEBUG:terrain] terrain_visual_debug_mode=%d source=%s"
			% [_terrain_visual_debug_mode, OS.get_environment("METRUM_DEBUG_TERRAIN_VISUAL")]
		)
	if _terrain_grass_visual_debug_mode != 0:
		print(
			"[DEBUG:terrain] grass_visual_debug_mode=%d source=%s"
			% [_terrain_grass_visual_debug_mode, OS.get_environment("METRUM_DEBUG_TERRAIN_GRASS")]
		)

func _process(delta: float) -> void:
	var frame_start_us := Time.get_ticks_usec()
	var perf_enabled := PerfDebug.is_enabled()
	var payload_poll_elapsed_ms := 0.0
	var payload_poll_count := 0
	var residency_start_us := frame_start_us
	var sim_core_busy: bool = simulation_node.is_sim_core_busy()
	var network_refresh_pending: bool = simulation_node.is_network_dirty()
	var residency_changed := false
	if not sim_core_busy:
		var payload_poll_start_us := Time.get_ticks_usec()
		payload_poll_count = _poll_ready_terrain_patch_payloads(PATCH_PAYLOAD_POLL_BUDGET_PER_FRAME)
		payload_poll_elapsed_ms = float(Time.get_ticks_usec() - payload_poll_start_us) / 1000.0
		residency_changed = _sync_patch_residency()
	var residency_elapsed_ms := float(Time.get_ticks_usec() - residency_start_us) / 1000.0
	var upload_elapsed_ms := 0.0
	var border_elapsed_ms := 0.0
	var water_sync_elapsed_ms := 0.0
	var water_sync_perf_stats := {}
	var lod_elapsed_ms := 0.0
	var prewarm_elapsed_ms := 0.0
	_terrain_lod_last_deferred_count = 0
	_terrain_prewarm_last_deferred_count = 0
	if not sim_core_busy and not network_refresh_pending and simulation_node.is_terrain_dirty():
		_refresh_road_locked_patch_lookup()
		var dirty_start_us := Time.get_ticks_usec()
		var dirty_keys := _dirty_patch_keys(simulation_node.get_dirty_terrain_patches())
		_terrain_debug_dirty_batches += 1
		_terrain_debug_dirty_patch_total += dirty_keys.size()
		var dirty_upload_pending := false
		if dirty_keys.is_empty():
			pass
		else:
			for key in dirty_keys:
				if patches.has(key):
					if _upload_patch(key, true):
						_refresh_one_patch_mesh_lod(key)
						_queue_water_patch_texture_sync(key)
					else:
						dirty_upload_pending = true
		upload_elapsed_ms = float(Time.get_ticks_usec() - dirty_start_us) / 1000.0
		if not dirty_upload_pending:
			var border_start_us := Time.get_ticks_usec()
			if not dirty_keys.is_empty() and _dirty_patch_keys_touch_border(dirty_keys):
				_rebuild_border_skirt()
			border_elapsed_ms = float(Time.get_ticks_usec() - border_start_us) / 1000.0
			simulation_node.clear_terrain_dirty()
	if not simulation_node.is_terrain_dirty():
		_prune_patch_payload_cache()

	var water_sync_start_us := Time.get_ticks_usec()
	water_sync_perf_stats = _process_water_patch_texture_sync_queue(
		PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME,
		perf_enabled
	)
	water_sync_elapsed_ms = float(Time.get_ticks_usec() - water_sync_start_us) / 1000.0

	if overlay_mode != cached_overlay_mode:
		_update_overlay_texture()
		_apply_overlay_mode()
		cached_overlay_mode = overlay_mode

	if _terrain_frame_headroom_available(frame_start_us, PATCH_LOD_START_HEADROOM_MS):
		if perf_enabled:
			var lod_start_us := Time.get_ticks_usec()
			_refresh_patch_mesh_lods(delta)
			lod_elapsed_ms = float(Time.get_ticks_usec() - lod_start_us) / 1000.0
		else:
			_refresh_patch_mesh_lods(delta)
	else:
		_defer_patch_mesh_lods(delta)

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

	if (
		not sim_core_busy
		and not network_refresh_pending
		and not simulation_node.is_terrain_dirty()
		and not residency_changed
		and not _terrain_residency_pending_mutations
	):
		if (
			patch_lod_refresh_queue.is_empty()
			and _terrain_frame_headroom_available(frame_start_us, PATCH_PREWARM_START_HEADROOM_MS)
		):
			if perf_enabled:
				var prewarm_start_us := Time.get_ticks_usec()
				_prewarm_patch_cache()
				prewarm_elapsed_ms = float(Time.get_ticks_usec() - prewarm_start_us) / 1000.0
			else:
				_prewarm_patch_cache()
		else:
			_terrain_prewarm_last_deferred_count = 1 if not patch_prewarm_queue.is_empty() else 0

	if perf_enabled:
		var perf_details := {
			"residency": residency_elapsed_ms,
			"residency_add_count": float(_terrain_residency_last_add_count),
			"residency_remove_count": float(_terrain_residency_last_remove_count),
			"residency_add_pending_count": float(_terrain_residency_last_add_pending_count),
			"residency_remove_pending_count": float(_terrain_residency_last_remove_pending_count),
			"resource_pool_hit_count": float(_terrain_resource_pool_hit_count),
			"resource_pool_miss_count": float(_terrain_resource_pool_miss_count),
			"resource_pool_release_count": float(_terrain_resource_pool_release_count),
			"resource_pool_prewarm_count": float(_terrain_resource_pool_prewarm_count),
			"resource_pool_size": float(patch_resource_pool.size()),
			"payload_poll": payload_poll_elapsed_ms,
			"payload_poll_count": float(payload_poll_count),
			"payload_requested_count": float(patch_payload_requested.size()),
			"payload_ready_count": float(patch_payload_ready.size()),
			"upload": upload_elapsed_ms,
			"border": border_elapsed_ms,
			"water_sync": water_sync_elapsed_ms,
			"lod": lod_elapsed_ms,
			"lod_processed_count": float(_terrain_lod_last_processed_count),
			"lod_changed_count": float(_terrain_lod_last_changed_count),
			"lod_queued_count": float(_terrain_lod_last_queued_count),
			"lod_queue_count": float(_terrain_lod_last_queue_count),
			"lod_replaced_count": float(_terrain_lod_last_replaced_count),
			"lod_skipped_count": float(_terrain_lod_last_skipped_count),
			"lod_deferred_count": float(_terrain_lod_last_deferred_count),
			"prewarm": prewarm_elapsed_ms,
			"prewarm_deferred_count": float(_terrain_prewarm_last_deferred_count),
		}
		for key_variant in water_sync_perf_stats.keys():
			var perf_key := str(key_variant)
			perf_details[perf_key] = water_sync_perf_stats[key_variant]
		PerfDebug.record(
			"terrain",
			float(Time.get_ticks_usec() - frame_start_us) / 1000.0,
			perf_details
		)
		_reset_terrain_resource_pool_perf_counters()

func get_resident_patch_keys() -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		keys.append(key)
	return keys

func get_resident_patch_revision() -> int:
	return _terrain_resident_patch_revision

func get_patch_height_texture(key: Vector2i) -> Texture2D:
	if not patches.has(key):
		return null
	return patches[key]["height_texture"]

func get_border_loop_positions() -> PackedVector3Array:
	return border_loop_positions

func get_border_revision() -> int:
	return border_revision

func refresh_water_patch_bindings() -> void:
	_queue_all_water_patch_texture_syncs()

func refresh_water_patch_binding(key: Vector2i) -> void:
	_queue_water_patch_texture_sync(key)

func update_terrain_visuals() -> bool:
	_refresh_road_locked_patch_lookup()
	var dirty_keys := _dirty_patch_keys(simulation_node.get_dirty_terrain_patches())
	if dirty_keys.is_empty():
		_process_water_patch_texture_sync_queue(PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME)
		return true

	_poll_ready_terrain_patch_payloads(PATCH_PAYLOAD_POLL_BUDGET_PER_FRAME)
	if not _dirty_patch_payloads_ready_for_atomic_upload(dirty_keys):
		_process_water_patch_texture_sync_queue(PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME)
		return false

	var upload_pending := false
	for key in dirty_keys:
		if patches.has(key):
			if _upload_patch(key, true):
				_refresh_one_patch_mesh_lod(key)
				_queue_water_patch_texture_sync(key)
			else:
				upload_pending = true
	if not upload_pending and (dirty_keys.is_empty() or _dirty_patch_keys_touch_border(dirty_keys)):
		_rebuild_border_skirt()
	_process_water_patch_texture_sync_queue(PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME)
	return not upload_pending

func _sync_patch_residency(force_full_sync: bool = false) -> bool:
	if patch_cols <= 0 or patch_rows <= 0:
		_record_residency_perf_counters(0, 0, 0, 0)
		return false

	var desired_bounds: Dictionary = _desired_patch_bounds()
	_terrain_debug_last_desired_bounds = desired_bounds
	var resident_target_bounds := _expanded_patch_bounds(
		desired_bounds,
		PATCH_RESIDENCY_HYSTERESIS_PATCHES
	)
	if (
		not force_full_sync
		and not _terrain_residency_pending_mutations
		and _terrain_residency_target_bounds_valid
		and _patch_bounds_equal(resident_target_bounds, _terrain_residency_target_bounds)
	):
		_record_residency_perf_counters(0, 0, 0, 0)
		return false

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
		_terrain_residency_pending_mutations = false
		_terrain_residency_target_bounds = resident_target_bounds.duplicate()
		_terrain_residency_target_bounds_valid = true
		_refresh_resident_patch_bounds()
		_record_residency_perf_counters(0, 0, 0, 0)
		return false

	_sort_patch_keys_by_camera_priority(keys_to_add)
	_sort_patch_keys_by_camera_priority(keys_to_remove)
	keys_to_remove.reverse()
	_request_terrain_patch_payloads(keys_to_add, PATCH_PAYLOAD_REQUEST_BUDGET_PER_FRAME)

	var changed := false
	var mutation_limit := PATCH_RESIDENCY_MUTATION_MAX_PER_FRAME
	if force_full_sync:
		mutation_limit = keys_to_add.size() + keys_to_remove.size()
	var budget_start_us: int = Time.get_ticks_usec()
	var processed_mutations := 0
	var processed_adds := 0
	var processed_removes := 0
	for key in keys_to_remove:
		if processed_mutations >= mutation_limit:
			break
		if _time_budget_exhausted(budget_start_us, PATCH_RESIDENCY_MUTATION_BUDGET_MS, processed_mutations):
			break
		_deactivate_patch(key)
		processed_mutations += 1
		processed_removes += 1
		changed = true

	var attempted_adds := 0
	var add_attempt_limit: int = mini(
		PATCH_RESIDENCY_ADD_ATTEMPT_MAX_PER_FRAME,
		max(1, mutation_limit - processed_mutations)
	)
	for key in keys_to_add:
		if processed_mutations >= mutation_limit:
			break
		if processed_adds >= PATCH_RESIDENCY_ADD_APPLY_MAX_PER_FRAME:
			break
		if attempted_adds >= add_attempt_limit:
			break
		if _time_budget_exhausted(budget_start_us, PATCH_RESIDENCY_MUTATION_BUDGET_MS, processed_mutations):
			break
		if not patches.has(key) and not patch_payload_ready.has(key):
			attempted_adds += 1
			continue
		attempted_adds += 1
		if _activate_patch(key):
			processed_mutations += 1
			processed_adds += 1
			changed = true

	_terrain_residency_pending_mutations = (
		processed_adds < keys_to_add.size()
		or processed_removes < keys_to_remove.size()
	)
	_record_residency_perf_counters(
		processed_adds,
		processed_removes,
		max(0, keys_to_add.size() - processed_adds),
		max(0, keys_to_remove.size() - processed_removes)
	)
	_refresh_resident_patch_bounds()
	_terrain_residency_target_bounds = resident_target_bounds.duplicate()
	_terrain_residency_target_bounds_valid = true
	if changed:
		_terrain_resident_patch_revision += 1
		_terrain_debug_residency_changes += 1
		_rebuild_patch_prewarm_queue()
		if _terrain_debug_verbose:
			_terrain_debug_log(
				"residency changed desired=%s resident=%s resident_count=%d add_pending=%d remove_pending=%d"
				% [
					_terrain_debug_bounds_label(desired_bounds),
					_terrain_debug_current_resident_bounds_label(),
					resident_patch_lookup.size(),
					max(0, keys_to_add.size() - processed_adds),
					max(0, keys_to_remove.size() - processed_removes),
				]
			)
	return changed

func _record_residency_perf_counters(
	add_count: int,
	remove_count: int,
	add_pending_count: int,
	remove_pending_count: int
) -> void:
	_terrain_residency_last_add_count = add_count
	_terrain_residency_last_remove_count = remove_count
	_terrain_residency_last_add_pending_count = add_pending_count
	_terrain_residency_last_remove_pending_count = remove_pending_count

func _patch_bounds_equal(a: Dictionary, b: Dictionary) -> bool:
	return (
		int(a.get("min_x", 0)) == int(b.get("min_x", 0))
		and int(a.get("max_x", -1)) == int(b.get("max_x", -1))
		and int(a.get("min_z", 0)) == int(b.get("min_z", 0))
		and int(a.get("max_z", -1)) == int(b.get("max_z", -1))
	)

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

func _create_patch(key: Vector2i, allow_async: bool = true) -> void:
	if patches.has(key):
		return
	var patch_data: Dictionary = _terrain_patch_data_for_key(key, false, allow_async)
	if patch_data.is_empty():
		return
	if road_locked_patch_lookup.has(key) and not _road_locked_patch_data_is_renderable(patch_data):
		if _road_debug_enabled:
			print(
				"[DEBUG:road] terrain_create key=(%d,%d) deferred_bad_cdt_no_heightmap_fallback=true cdt_status=%s cdt_error=%s"
				% [
					key.x,
					key.y,
					str(patch_data.get("terrain_cdt_status", "none")),
					str(patch_data.get("terrain_cdt_error", "none")),
				]
			)
		_request_terrain_patch_payload(key, true)
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
	var patch_resources: Dictionary = _acquire_terrain_patch_resources()
	var height_image: Image = patch_resources["height_image"] as Image
	var height_texture: ImageTexture = _upload_terrain_patch_height_texture(
		patch_resources,
		texture_width,
		texture_height,
		_terrain_patch_height_bytes(patch_data)
	)
	var patch_mesh: Mesh
	var patch_center_x := world_origin_x + world_size_x * 0.5
	var patch_center_z := world_origin_z + world_size_z * 0.5
	var initial_lod_step := _mesh_lod_step_for_patch(key, patch_center_x, patch_center_z)
	var sample_step_m := world_size_x / float(max(1, sample_width - 1))
	var initial_subdivision_factor := _mesh_subdivision_factor_for_patch(key, sample_step_m)
	var height_is_baked: bool = _patch_has_baked_terrain_mesh(patch_data)
	patch_mesh = _terrain_patch_mesh_from_data(patch_data, initial_lod_step, initial_subdivision_factor)

	var patch_node: MeshInstance3D = patch_resources["node"] as MeshInstance3D
	patch_node.name = "TerrainPatch_%d_%d" % [key.x, key.y]
	patch_node.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
	patch_node.mesh = patch_mesh
	patch_node.visible = false
	patch_node.position = Vector3(
		world_origin_x + world_size_x * 0.5,
		0.0,
		world_origin_z + world_size_z * 0.5
	)
	var retaining_wall_node: MeshInstance3D = patch_resources["retaining_wall_node"] as MeshInstance3D
	retaining_wall_node.name = "RetainingWalls"
	retaining_wall_node.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
	retaining_wall_node.mesh = _retaining_wall_patch_mesh(patch_data)
	retaining_wall_node.visible = (
		not _patch_has_unusable_refined_cdt(patch_data)
		and _patch_has_retaining_wall_mesh(patch_data)
	)
	retaining_wall_node.material_override = _retaining_wall_material()

	var material: ShaderMaterial = patch_resources["material"] as ShaderMaterial
	material.shader = TERRAIN_SHADER
	material.set_shader_parameter("heightmap", height_texture)
	material.set_shader_parameter("overlay_texture", overlay_texture)
	material.set_shader_parameter("watermap", empty_water_texture)
	material.set_shader_parameter("terrain_grass_albedo", grass_albedo_texture)
	material.set_shader_parameter("terrain_grass_height", grass_height_texture)
	material.set_shader_parameter("overlay_mode", overlay_mode)
	material.set_shader_parameter("height_scale", HEIGHT_SCALE)
	material.set_shader_parameter("height_is_baked", height_is_baked)
	material.set_shader_parameter("world_size", terrain_world_size)
	material.set_shader_parameter("terrain_visual_debug_mode", _terrain_visual_debug_mode)
	material.set_shader_parameter("terrain_debug_patch_key", Vector2(key.x, key.y))
	material.set_shader_parameter("terrain_debug_lod_step", float(initial_lod_step))
	material.set_shader_parameter("terrain_grass_visual_debug_mode", _terrain_grass_visual_debug_mode)
	material.set_shader_parameter("scene_sun_direction", SceneLightingConfig.sun_direction())
	material.set_shader_parameter("scene_sun_color", SceneLightingConfig.sun_color())
	material.set_shader_parameter("scene_sky_color", SceneLightingConfig.sky_color())
	material.set_shader_parameter("scene_ambient_strength", SceneLightingConfig.ambient_strength())
	material.set_shader_parameter("scene_shadow_max_distance_m", SceneLightingConfig.SHADOW_MAX_DISTANCE_M)
	material.set_shader_parameter(
		"scene_shadow_split_distances_m",
		SceneLightingConfig.shadow_split_distances()
	)
	SceneLightingConfig.apply_ground_shadow_parameters(material)
	material.set_shader_parameter("heightmap_texture_size", Vector2(texture_width, texture_height))
	material.set_shader_parameter("inner_sample_offset_texels", Vector2(inner_offset_x, inner_offset_z))
	material.set_shader_parameter("inner_sample_size_texels", Vector2(sample_width, sample_height))
	material.set_shader_parameter("watermap_texture_size", Vector2(2, 2))
	material.set_shader_parameter("watermap_inner_sample_offset_texels", Vector2.ZERO)
	material.set_shader_parameter("watermap_inner_sample_size_texels", Vector2(2, 2))
	material.set_shader_parameter("patch_world_size_m", Vector2(world_size_x, world_size_z))
	material.set_shader_parameter("terrain_cell_m", terrain_cell_m)
	material.set_shader_parameter("hillshade_azimuth_deg", HILLSHADE_AZIMUTH_DEG)
	material.set_shader_parameter("hillshade_altitude_deg", HILLSHADE_ALTITUDE_DEG)
	material.set_shader_parameter("hillshade_strength", HILLSHADE_STRENGTH)
	material.set_shader_parameter("hillshade_ambient", HILLSHADE_AMBIENT)
	material.set_shader_parameter("hillshade_contrast", HILLSHADE_CONTRAST)
	material.set_shader_parameter("hillshade_shadow_tint", HILLSHADE_SHADOW_TINT)
	material.set_shader_parameter("hillshade_light_tint", HILLSHADE_LIGHT_TINT)
	material.set_shader_parameter("terrain_macro_variation_strength", TERRAIN_MACRO_VARIATION_STRENGTH)
	material.set_shader_parameter("terrain_grass_tint", TERRAIN_GRASS_TINT)
	material.set_shader_parameter("terrain_grass_tint_strength", TERRAIN_GRASS_TINT_STRENGTH)
	material.set_shader_parameter("terrain_grass_albedo_strength", TERRAIN_GRASS_ALBEDO_STRENGTH)
	material.set_shader_parameter("terrain_grass_macro_scale", TERRAIN_GRASS_MACRO_SCALE)
	material.set_shader_parameter("terrain_grass_mid_scale", TERRAIN_GRASS_MID_SCALE)
	material.set_shader_parameter("terrain_grass_macro_strength", TERRAIN_GRASS_MACRO_STRENGTH)
	material.set_shader_parameter("terrain_grass_mid_strength", TERRAIN_GRASS_MID_STRENGTH)
	material.set_shader_parameter("terrain_grass_micro_strength", TERRAIN_GRASS_MICRO_STRENGTH)
	material.set_shader_parameter("terrain_natural_variation_strength", TERRAIN_NATURAL_VARIATION_STRENGTH)
	material.set_shader_parameter("terrain_meadow_mottle_strength", TERRAIN_MEADOW_MOTTLE_STRENGTH)
	material.set_shader_parameter("terrain_baked_normal_blend", TERRAIN_BAKED_NORMAL_BLEND)
	material.set_shader_parameter(
		"terrain_baked_readability_strength",
		TERRAIN_BAKED_READABILITY_STRENGTH
	)
	material.set_shader_parameter("terrain_grass_detail_scale", TERRAIN_GRASS_DETAIL_SCALE)
	material.set_shader_parameter("terrain_grass_detail_strength", TERRAIN_GRASS_DETAIL_STRENGTH)
	material.set_shader_parameter(
		"terrain_grass_height_detail_strength",
		TERRAIN_GRASS_HEIGHT_DETAIL_STRENGTH
	)
	material.set_shader_parameter("terrain_grass_detail_fade_start", TERRAIN_GRASS_DETAIL_FADE_START)
	material.set_shader_parameter("terrain_grass_detail_fade_end", TERRAIN_GRASS_DETAIL_FADE_END)
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
	material.set_shader_parameter(
		"contour_relief_minor_boost_strength",
		CONTOUR_RELIEF_MINOR_BOOST_STRENGTH
	)
	material.set_shader_parameter("contour_zero_elevation_fade_m", CONTOUR_ZERO_ELEVATION_FADE_M)
	material.set_shader_parameter("contour_flat_relief_start_m", CONTOUR_FLAT_RELIEF_START_M)
	material.set_shader_parameter("contour_flat_relief_end_m", CONTOUR_FLAT_RELIEF_END_M)
	patch_node.material_override = material
	_ensure_patch_node_parent(patch_node)
	_terrain_debug_patch_creates += 1

	patches[key] = {
		"node": patch_node,
		"retaining_wall_node": retaining_wall_node,
		"material": material,
		"height_image": height_image,
		"height_texture": height_texture,
		"water_texture": empty_water_texture,
		"water_texture_width": 2,
		"water_texture_height": 2,
		"water_inner_offset_x": 0,
		"water_inner_offset_z": 0,
		"water_sample_width": 2,
		"water_sample_height": 2,
		"water_depth_nonzero_count": 0,
		"water_world_origin_x": world_origin_x,
		"water_world_origin_z": world_origin_z,
		"water_world_size_x": world_size_x,
		"water_world_size_z": world_size_z,
		"sample_width": sample_width,
		"sample_height": sample_height,
		"texture_width": texture_width,
		"texture_height": texture_height,
		"world_size_x": world_size_x,
		"world_size_z": world_size_z,
		"sample_step_m": sample_step_m,
		"lod_step": initial_lod_step,
		"subdivision_factor": initial_subdivision_factor,
		"height_is_baked": height_is_baked,
		"road_locked_bad_cdt_blocked": false,
		"last_patch_data": patch_data,
	}

func _upload_patch(key: Vector2i, allow_async: bool = false) -> bool:
	if not patches.has(key):
		return false
	var total_start_us := Time.get_ticks_usec()
	var fetch_start_us := Time.get_ticks_usec()
	var patch_data: Dictionary = _terrain_patch_data_for_key(key, false, allow_async)
	var fetch_ms := float(Time.get_ticks_usec() - fetch_start_us) / 1000.0
	if patch_data.is_empty():
		if allow_async:
			return false
		_remove_patch(key)
		if _road_debug_enabled:
			print(
				"[DEBUG:road] terrain_upload key=(%d,%d) missing_patch_data=true fetch_ms=%.3f total_ms=%.3f"
				% [
					key.x,
					key.y,
					fetch_ms,
					float(Time.get_ticks_usec() - total_start_us) / 1000.0,
				]
			)
		return false
	var patch: Dictionary = patches[key]
	var road_locked_patch := road_locked_patch_lookup.has(key)
	if road_locked_patch and not _road_locked_patch_data_is_renderable(patch_data):
		var previous_patch_data := _last_renderable_road_locked_patch_data(patch)
		if not previous_patch_data.is_empty():
			patch["road_locked_bad_cdt_blocked"] = false
			_apply_patch_visibility_for_residency(key, patch, previous_patch_data)
			if _road_debug_enabled:
				print(
					"[DEBUG:road] terrain_upload key=(%d,%d) preserved_last_good_cdt=true cdt_status=%s cdt_error=%s fetch_ms=%.3f total_ms=%.3f"
					% [
						key.x,
						key.y,
						str(patch_data.get("terrain_cdt_status", "none")),
						str(patch_data.get("terrain_cdt_error", "none")),
						fetch_ms,
						float(Time.get_ticks_usec() - total_start_us) / 1000.0,
					]
				)
			return true
		_block_road_locked_patch_until_valid_cdt(patch)
		if _road_debug_enabled:
			print(
				"[DEBUG:road] terrain_upload key=(%d,%d) hidden_bad_cdt_no_heightmap_fallback=true cdt_status=%s cdt_error=%s fetch_ms=%.3f total_ms=%.3f"
				% [
					key.x,
					key.y,
					str(patch_data.get("terrain_cdt_status", "none")),
					str(patch_data.get("terrain_cdt_error", "none")),
					fetch_ms,
					float(Time.get_ticks_usec() - total_start_us) / 1000.0,
				]
			)
		_request_terrain_patch_payload(key, true)
		return false
	elif _patch_has_unusable_refined_cdt(patch_data):
		var fallback_patch_data: Dictionary = simulation_node.get_terrain_patch(key.x, key.y)
		if not fallback_patch_data.is_empty():
			patch_data = fallback_patch_data
	patch["last_patch_data"] = patch_data
	patch["road_locked_bad_cdt_blocked"] = false
	var metadata_start_us := Time.get_ticks_usec()
	var texture_width := int(patch_data["texture_width"])
	var texture_height := int(patch_data["texture_height"])
	var old_texture_width := int(patch.get("texture_width", texture_width))
	var old_texture_height := int(patch.get("texture_height", texture_height))
	var material: ShaderMaterial = patch["material"]
	var height_image: Image = patch["height_image"]
	var height_texture: ImageTexture = patch["height_texture"]
	var metadata_ms := float(Time.get_ticks_usec() - metadata_start_us) / 1000.0

	var texture_start_us := Time.get_ticks_usec()
	height_image.set_data(
		texture_width,
		texture_height,
		false,
		Image.FORMAT_RF,
		_terrain_patch_height_bytes(patch_data)
	)
	if old_texture_width == texture_width and old_texture_height == texture_height:
		height_texture.update(height_image)
	else:
		height_texture = ImageTexture.create_from_image(height_image)
		patch["height_texture"] = height_texture
		material.set_shader_parameter("heightmap", height_texture)
	material.set_shader_parameter("heightmap_texture_size", Vector2(texture_width, texture_height))
	material.set_shader_parameter(
		"inner_sample_offset_texels",
		Vector2(float(patch_data["inner_offset_x"]), float(patch_data["inner_offset_z"]))
	)
	material.set_shader_parameter(
		"inner_sample_size_texels",
		Vector2(int(patch_data["sample_width"]), int(patch_data["sample_height"]))
	)
	material.set_shader_parameter("patch_world_size_m", Vector2(
		float(patch_data["world_size_x"]),
		float(patch_data["world_size_z"])
	))
	var height_is_baked: bool = _patch_has_baked_terrain_mesh(patch_data)
	material.set_shader_parameter("height_is_baked", height_is_baked)
	var texture_ms := float(Time.get_ticks_usec() - texture_start_us) / 1000.0

	_terrain_debug_patch_uploads += 1

	var patch_update_start_us := Time.get_ticks_usec()
	var patch_node: MeshInstance3D = patch["node"]
	var world_size_x := float(patch_data["world_size_x"])
	var world_size_z := float(patch_data["world_size_z"])
	patch["sample_width"] = int(patch_data["sample_width"])
	patch["sample_height"] = int(patch_data["sample_height"])
	patch["texture_width"] = texture_width
	patch["texture_height"] = texture_height
	patch["world_size_x"] = world_size_x
	patch["world_size_z"] = world_size_z
	patch["sample_step_m"] = world_size_x / float(max(1, int(patch_data["sample_width"]) - 1))
	patch["height_is_baked"] = height_is_baked
	var patch_update_ms := float(Time.get_ticks_usec() - patch_update_start_us) / 1000.0

	var mesh_start_us := Time.get_ticks_usec()
	var terrain_mesh := _terrain_patch_mesh_from_data(
		patch_data,
		int(patch.get("lod_step", 1)),
		int(patch.get("subdivision_factor", 1))
	)
	patch_node.mesh = terrain_mesh
	var mesh_ms := float(Time.get_ticks_usec() - mesh_start_us) / 1000.0

	var retaining_start_us := Time.get_ticks_usec()
	var retaining_wall_node: MeshInstance3D = patch.get("retaining_wall_node", null) as MeshInstance3D
	if retaining_wall_node != null:
		retaining_wall_node.mesh = _retaining_wall_patch_mesh(patch_data)
		retaining_wall_node.visible = _patch_has_retaining_wall_mesh(patch_data)
	var retaining_ms := float(Time.get_ticks_usec() - retaining_start_us) / 1000.0

	var position_start_us := Time.get_ticks_usec()
	patch_node.position = Vector3(
		float(patch_data["world_origin_x"]) + world_size_x * 0.5,
		0.0,
		float(patch_data["world_origin_z"]) + world_size_z * 0.5
	)
	_apply_patch_visibility_for_residency(key, patch, patch_data)
	var position_ms := float(Time.get_ticks_usec() - position_start_us) / 1000.0
	if _road_debug_enabled:
		var terrain_vertices := 0
		var terrain_indices := 0
		var retaining_vertices := 0
		var retaining_indices := 0
		if patch_data.has("terrain_mesh_vertices"):
			terrain_vertices = (patch_data["terrain_mesh_vertices"] as PackedVector3Array).size()
		if patch_data.has("terrain_mesh_indices"):
			terrain_indices = (patch_data["terrain_mesh_indices"] as PackedInt32Array).size()
		if patch_data.has("terrain_retaining_wall_mesh_vertices"):
			retaining_vertices = (
				patch_data["terrain_retaining_wall_mesh_vertices"] as PackedVector3Array
			).size()
		if patch_data.has("terrain_retaining_wall_mesh_indices"):
			retaining_indices = (
				patch_data["terrain_retaining_wall_mesh_indices"] as PackedInt32Array
			).size()
		print(
			"[DEBUG:road] terrain_upload key=(%d,%d) road_locked=%s include_debug=%s fetch_ms=%.3f metadata_ms=%.3f texture_ms=%.3f patch_update_ms=%.3f mesh_ms=%.3f retaining_ms=%.3f position_ms=%.3f total_ms=%.3f terrain_vertices=%d terrain_indices=%d retaining_vertices=%d retaining_indices=%d"
			% [
				key.x,
				key.y,
				str(road_locked_patch_lookup.has(key)),
				"false",
				fetch_ms,
				metadata_ms,
				texture_ms,
				patch_update_ms,
				mesh_ms,
				retaining_ms,
				position_ms,
				float(Time.get_ticks_usec() - total_start_us) / 1000.0,
				terrain_vertices,
				terrain_indices,
				retaining_vertices,
				retaining_indices,
			]
		)
	return true

func _block_road_locked_patch_until_valid_cdt(patch: Dictionary) -> void:
	patch["road_locked_bad_cdt_blocked"] = true
	patch["last_patch_data"] = {}
	patch["height_is_baked"] = true
	var patch_node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
	if patch_node != null:
		patch_node.visible = false
		patch_node.mesh = null
	var retaining_wall_node: MeshInstance3D = patch.get("retaining_wall_node", null) as MeshInstance3D
	if retaining_wall_node != null:
		retaining_wall_node.visible = false
		retaining_wall_node.mesh = null

func _patch_is_blocked_by_bad_cdt(patch: Dictionary) -> bool:
	return bool(patch.get("road_locked_bad_cdt_blocked", false))

func _apply_patch_visibility_for_residency(
	key: Vector2i,
	patch: Dictionary,
	patch_data: Dictionary
) -> void:
	var visible_for_residency := resident_patch_lookup.has(key)
	var patch_node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
	if patch_node != null:
		patch_node.visible = visible_for_residency
	var retaining_wall_node: MeshInstance3D = patch.get("retaining_wall_node", null) as MeshInstance3D
	if retaining_wall_node != null:
		retaining_wall_node.visible = (
			visible_for_residency
			and _patch_has_retaining_wall_mesh(patch_data)
		)

func _activate_patch(key: Vector2i) -> bool:
	if resident_patch_lookup.has(key):
		return false
	if not patches.has(key):
		_create_patch(key)
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return false
	var last_patch_data: Dictionary = patch.get("last_patch_data", {}) as Dictionary
	if (
		_patch_is_blocked_by_bad_cdt(patch)
		or (
			road_locked_patch_lookup.has(key)
			and not _road_locked_patch_data_is_renderable(last_patch_data)
		)
	):
		if not _upload_patch(key, true):
			return false
		patch = patches.get(key, {})
		last_patch_data = patch.get("last_patch_data", {}) as Dictionary
	if patch.is_empty() or _patch_is_blocked_by_bad_cdt(patch):
		return false
	_refresh_one_patch_mesh_lod(key)
	resident_patch_lookup[key] = true
	_apply_patch_visibility_for_residency(key, patch, last_patch_data)
	_queue_water_patch_texture_sync(key)
	return true

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
	var was_resident: bool = resident_patch_lookup.has(key)
	var patch: Dictionary = patches[key]
	_release_terrain_patch_resources(patch)
	patches.erase(key)
	resident_patch_lookup.erase(key)
	patch_lod_refresh_lookup.erase(key)
	if was_resident:
		_terrain_residency_target_bounds_valid = false
		_terrain_resident_patch_revision += 1
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
		_release_terrain_patch_resources(patch)
	patches.clear()
	resident_patch_lookup.clear()
	patch_payload_requested.clear()
	patch_payload_requested_generation.clear()
	patch_payload_ready.clear()
	patch_payload_surface_generation = -1
	patch_prewarm_queue.clear()
	patch_lod_refresh_queue.clear()
	patch_lod_refresh_lookup.clear()
	_terrain_lod_refresh_camera_valid = false
	_record_lod_perf_counters(0, 0, 0, 0)
	_terrain_lod_last_deferred_count = 0
	_terrain_prewarm_last_deferred_count = 0
	water_texture_sync_queue.clear()
	water_texture_sync_lookup.clear()
	_resident_patch_bounds_valid = false
	_terrain_residency_pending_mutations = false
	_terrain_residency_target_bounds_valid = false
	_terrain_residency_target_bounds.clear()
	_terrain_resident_patch_revision += 1

func _prewarm_terrain_patch_resource_pool() -> void:
	while patch_resource_pool.size() < PATCH_RESOURCE_POOL_PREWARM_COUNT:
		patch_resource_pool.append(_new_terrain_patch_resources())
		_terrain_resource_pool_prewarm_count += 1

func _acquire_terrain_patch_resources() -> Dictionary:
	var resources: Dictionary
	if patch_resource_pool.is_empty():
		resources = _new_terrain_patch_resources()
		_terrain_resource_pool_miss_count += 1
	else:
		resources = patch_resource_pool.pop_back() as Dictionary
		_terrain_resource_pool_hit_count += 1
	var patch_node: MeshInstance3D = resources["node"] as MeshInstance3D
	patch_node.visible = false
	patch_node.mesh = null
	patch_node.position = Vector3.ZERO
	var retaining_wall_node: MeshInstance3D = resources["retaining_wall_node"] as MeshInstance3D
	retaining_wall_node.visible = false
	retaining_wall_node.mesh = null
	_ensure_patch_node_parent(patch_node)
	return resources

func _release_terrain_patch_resources(patch: Dictionary) -> void:
	var patch_node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
	if patch_node == null:
		return
	var retaining_wall_node: MeshInstance3D = (
		patch.get("retaining_wall_node", null) as MeshInstance3D
	)
	var material: ShaderMaterial = patch.get("material", null) as ShaderMaterial
	var height_image: Image = patch.get("height_image", null) as Image
	var height_texture: ImageTexture = patch.get("height_texture", null) as ImageTexture
	patch_node.visible = false
	patch_node.mesh = null
	patch_node.position = Vector3.ZERO
	patch_node.name = "TerrainPatchPool"
	if material != null:
		material.shader = TERRAIN_SHADER
		patch_node.material_override = material
	if retaining_wall_node != null:
		retaining_wall_node.visible = false
		retaining_wall_node.mesh = null
	if patch_resource_pool.size() >= PATCH_RESOURCE_POOL_MAX:
		patch_node.queue_free()
		return
	patch_resource_pool.append({
		"node": patch_node,
		"retaining_wall_node": retaining_wall_node,
		"material": material,
		"height_image": height_image,
		"height_texture": height_texture,
		"height_texture_width": int(patch.get("texture_width", 0)),
		"height_texture_height": int(patch.get("texture_height", 0)),
	})
	_terrain_resource_pool_release_count += 1

func _new_terrain_patch_resources() -> Dictionary:
	var patch_node := MeshInstance3D.new()
	patch_node.name = "TerrainPatchPool"
	SceneLightingConfig.apply_shadow_policy(
		patch_node,
		SceneLightingConfig.SHADOW_RECEIVER_ONLY,
		"terrain"
	)
	patch_node.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
	patch_node.visible = false

	var retaining_wall_node := MeshInstance3D.new()
	retaining_wall_node.name = "RetainingWalls"
	SceneLightingConfig.apply_shadow_policy(
		retaining_wall_node,
		SceneLightingConfig.SHADOW_RECEIVER_ONLY,
		"terrain"
	)
	retaining_wall_node.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
	retaining_wall_node.visible = false
	patch_node.add_child(retaining_wall_node)

	var material := ShaderMaterial.new()
	material.shader = TERRAIN_SHADER
	patch_node.material_override = material

	var texture_size: Vector2i = _terrain_default_patch_texture_size()
	var height_image := Image.create(texture_size.x, texture_size.y, false, Image.FORMAT_RF)
	height_image.fill(Color.BLACK)
	var height_texture := ImageTexture.create_from_image(height_image)

	add_child(patch_node)
	return {
		"node": patch_node,
		"retaining_wall_node": retaining_wall_node,
		"material": material,
		"height_image": height_image,
		"height_texture": height_texture,
		"height_texture_width": texture_size.x,
		"height_texture_height": texture_size.y,
	}

func _terrain_default_patch_texture_size() -> Vector2i:
	var sample_count: int = max(2, patch_interval_cells + 1)
	return Vector2i(sample_count + 2, sample_count + 2)

func _upload_terrain_patch_height_texture(
	resources: Dictionary,
	texture_width: int,
	texture_height: int,
	height_bytes: PackedByteArray
) -> ImageTexture:
	var height_image: Image = resources["height_image"] as Image
	height_image.set_data(texture_width, texture_height, false, Image.FORMAT_RF, height_bytes)
	var height_texture: ImageTexture = resources.get("height_texture", null) as ImageTexture
	var old_width: int = int(resources.get("height_texture_width", 0))
	var old_height: int = int(resources.get("height_texture_height", 0))
	if height_texture != null and old_width == texture_width and old_height == texture_height:
		height_texture.update(height_image)
	else:
		height_texture = ImageTexture.create_from_image(height_image)
		resources["height_texture"] = height_texture
	resources["height_texture_width"] = texture_width
	resources["height_texture_height"] = texture_height
	return height_texture

func _ensure_patch_node_parent(patch_node: MeshInstance3D) -> void:
	if patch_node.get_parent() == null:
		add_child(patch_node)

func _reset_terrain_resource_pool_perf_counters() -> void:
	_terrain_resource_pool_hit_count = 0
	_terrain_resource_pool_miss_count = 0
	_terrain_resource_pool_release_count = 0
	_terrain_resource_pool_prewarm_count = 0

func _rebuild_patch_prewarm_queue() -> void:
	patch_prewarm_queue.clear()
	if patch_cols <= 0 or patch_rows <= 0:
		return
	var prewarm_bounds: Dictionary = _prewarm_patch_bounds()
	if int(prewarm_bounds["max_x"]) < int(prewarm_bounds["min_x"]):
		return
	if int(prewarm_bounds["max_z"]) < int(prewarm_bounds["min_z"]):
		return
	for patch_z in range(int(prewarm_bounds["min_z"]), int(prewarm_bounds["max_z"]) + 1):
		for patch_x in range(int(prewarm_bounds["min_x"]), int(prewarm_bounds["max_x"]) + 1):
			var key := Vector2i(patch_x, patch_z)
			if patches.has(key):
				continue
			patch_prewarm_queue.append(key)
	_sort_patch_keys_by_camera_priority(patch_prewarm_queue)

func _prewarm_patch_bounds() -> Dictionary:
	if patch_cols <= 0 or patch_rows <= 0:
		return {"min_x": 0, "max_x": -1, "min_z": 0, "max_z": -1}
	if _terrain_residency_target_bounds_valid:
		return _expanded_patch_bounds(_terrain_residency_target_bounds, PATCH_PREWARM_HALO_PATCHES)
	if _resident_patch_bounds_valid:
		return _expanded_patch_bounds(
			{
				"min_x": _resident_min_patch_x,
				"max_x": _resident_max_patch_x,
				"min_z": _resident_min_patch_z,
				"max_z": _resident_max_patch_z,
			},
			PATCH_PREWARM_HALO_PATCHES
		)
	return _expanded_patch_bounds(
		_desired_patch_bounds(),
		PATCH_RESIDENCY_HYSTERESIS_PATCHES + PATCH_PREWARM_HALO_PATCHES
	)

func _prewarm_patch_cache() -> void:
	if patch_prewarm_queue.is_empty():
		return
	var budget_start_us: int = Time.get_ticks_usec()
	var attempted_patches := 0
	var created_patches := 0
	while attempted_patches < PATCH_PREWARM_MAX_PER_FRAME and not patch_prewarm_queue.is_empty():
		if _time_budget_exhausted(budget_start_us, PATCH_PREWARM_BUDGET_MS, created_patches):
			break
		var key: Vector2i = patch_prewarm_queue.pop_front()
		attempted_patches += 1
		if patches.has(key):
			continue
		_create_patch(key)
		var patch: Dictionary = patches.get(key, {})
		if patch.is_empty():
			continue
		created_patches += 1
		var patch_node: MeshInstance3D = patch["node"]
		patch_node.visible = false

func _time_budget_exhausted(start_us: int, budget_ms: float, completed_count: int) -> bool:
	if completed_count <= 0:
		return false
	return float(Time.get_ticks_usec() - start_us) / 1000.0 >= budget_ms

func _terrain_frame_headroom_available(frame_start_us: int, start_budget_ms: float) -> bool:
	return float(Time.get_ticks_usec() - frame_start_us) / 1000.0 < start_budget_ms

func _sort_patch_keys_by_camera_priority(keys: Array[Vector2i]) -> void:
	if keys.size() <= 1:
		return
	var origin: Vector2i = _current_camera_patch_key()
	keys.sort_custom(func(a: Vector2i, b: Vector2i):
		var distance_a: int = absi(a.x - origin.x) + absi(a.y - origin.y)
		var distance_b: int = absi(b.x - origin.x) + absi(b.y - origin.y)
		if distance_a == distance_b:
			if a.y == b.y:
				return a.x < b.x
			return a.y < b.y
		return distance_a < distance_b
	)

func _current_camera_patch_key() -> Vector2i:
	if patch_cols <= 0 or patch_rows <= 0 or patch_span_m <= 0.0:
		return Vector2i.ZERO
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return Vector2i(int(patch_cols / 2), int(patch_rows / 2))
	var half_world := terrain_world_size * 0.5
	return Vector2i(
		clampi(int(floor((camera.global_position.x + half_world.x) / patch_span_m)), 0, patch_cols - 1),
		clampi(int(floor((camera.global_position.z + half_world.y) / patch_span_m)), 0, patch_rows - 1)
	)

func _dirty_patch_keys(flat_pairs: PackedInt32Array) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	var pair_count := flat_pairs.size() / 2
	for index in range(pair_count):
		keys.append(Vector2i(flat_pairs[index * 2], flat_pairs[index * 2 + 1]))
	return keys

func _dirty_patch_keys_touch_border(keys: Array[Vector2i]) -> bool:
	for key in keys:
		if key.x <= 0 or key.y <= 0 or key.x >= patch_cols - 1 or key.y >= patch_rows - 1:
			return true
	return false

func road_geometry_debug_patch_lines(flat_pairs: PackedInt32Array) -> Array[String]:
	var lines: Array[String] = []
	var keys: Array[Vector2i] = _dirty_patch_keys(flat_pairs)
	if keys.is_empty():
		lines.append("terrain_patch none")
		return lines
	for key in keys:
		var patch: Dictionary = patches.get(key, {})
		var patch_data: Dictionary = patch.get("last_patch_data", {})
		if patch_data.is_empty():
			lines.append("terrain_patch key=(%d,%d) missing_cached_patch_data=true" % [key.x, key.y])
			continue
		var patch_node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
		var mesh: Mesh = null
		if patch_node != null:
			mesh = patch_node.mesh
		var height_stats: Dictionary = _terrain_patch_height_stats(patch_data)
		var water_texture_width := int(patch.get("water_texture_width", 0))
		var water_texture_height := int(patch.get("water_texture_height", 0))
		var water_depth_nonzero_count := int(patch.get("water_depth_nonzero_count", 0))
		var water_world_origin_x := float(patch.get("water_world_origin_x", 0.0))
		var water_world_origin_z := float(patch.get("water_world_origin_z", 0.0))
		var water_world_size_x := float(patch.get("water_world_size_x", 0.0))
		var water_world_size_z := float(patch.get("water_world_size_z", 0.0))
		var clip_stats: Dictionary = _road_geometry_clip_stats(patch_data)
		var baked_vertex_count: int = _road_geometry_baked_vertex_count(patch_data)
		var retaining_wall_baked_vertex_count: int = _road_geometry_retaining_wall_baked_vertex_count(patch_data)
		var baked_mesh_stats: String = _road_geometry_baked_mesh_stats_label(patch_data)
		var cdt_status: String = str(patch_data.get("terrain_cdt_status", "none"))
		var cdt_error: String = str(patch_data.get("terrain_cdt_error", "none"))
		var cdt_stage: String = str(patch_data.get("terrain_cdt_diagnostic_stage", "none"))
		var cdt_backend: String = str(patch_data.get("terrain_cdt_diagnostic_backend", "none"))
		var cdt_input_vertices: int = int(patch_data.get("terrain_cdt_input_vertices", 0))
		var cdt_constraint_edges: int = int(patch_data.get("terrain_cdt_constraint_edges", 0))
		var cdt_road_constraint_edges: int = int(patch_data.get("terrain_cdt_road_constraint_edges", 0))
		var cdt_preserved_road_constraint_edges: int = int(patch_data.get("terrain_cdt_preserved_road_constraint_edges", 0))
		var cdt_spade_missing_road_constraint_edges: int = int(patch_data.get("terrain_cdt_spade_missing_road_constraint_edges", 0))
		var cdt_rejected_road_constraint_edges: int = int(patch_data.get("terrain_cdt_rejected_road_constraint_edges", 0))
		var cdt_internal_road_constraint_edges: int = int(patch_data.get("terrain_cdt_internal_road_constraint_edges", 0))
		var cdt_invalid_constraints: int = int(patch_data.get("terrain_cdt_invalid_constraints", 0))
		var cdt_accepted_faces: int = int(patch_data.get("terrain_cdt_accepted_faces", 0))
		var cdt_rejected_road_faces: int = int(patch_data.get("terrain_cdt_rejected_road_faces", 0))
		var cdt_emitted_faces: int = int(patch_data.get("terrain_cdt_emitted_faces", 0))
		var cdt_terrain_face_sources: String = _road_geometry_cdt_face_sources_summary_label(
			patch_data,
			"terrain_mesh"
		)
		var cdt_max_face_y_delta_m: float = float(patch_data.get("terrain_cdt_max_face_y_delta_m", 0.0))
		var cdt_max_face_slope_ratio: float = float(patch_data.get("terrain_cdt_max_face_slope_ratio", 0.0))
		var cdt_longest_triangle_edge_m: float = float(patch_data.get("terrain_cdt_longest_triangle_edge_m", 0.0))
		var cdt_road_seam_faces: int = int(patch_data.get("terrain_cdt_road_seam_faces", 0))
		var cdt_road_seam_max_y_delta_m: float = float(patch_data.get("terrain_cdt_road_seam_max_y_delta_m", 0.0))
		var cdt_road_seam_max_slope_ratio: float = float(patch_data.get("terrain_cdt_road_seam_max_slope_ratio", 0.0))
		var cdt_retaining_wall_faces: int = int(patch_data.get("terrain_cdt_retaining_wall_faces", 0))
		var cdt_retaining_wall_emitted_faces: int = int(patch_data.get("terrain_cdt_retaining_wall_emitted_faces", 0))
		var cdt_retaining_wall_face_sources: String = _road_geometry_cdt_face_sources_summary_label(
			patch_data,
			"terrain_retaining_wall_mesh"
		)
		var cdt_retaining_wall_max_y_delta_m: float = float(patch_data.get("terrain_cdt_retaining_wall_max_y_delta_m", 0.0))
		var cdt_retaining_wall_max_slope_ratio: float = float(patch_data.get("terrain_cdt_retaining_wall_max_slope_ratio", 0.0))
		var cdt_accepted_seam_edges: int = int(patch_data.get("terrain_cdt_accepted_seam_edges", 0))
		var cdt_merged_subbudget_seam_edges: int = int(patch_data.get("terrain_cdt_merged_subbudget_seam_edges", 0))
		var cdt_omitted_near_seam_source_samples: int = int(patch_data.get("terrain_cdt_omitted_near_seam_source_samples", 0))
		var cdt_retaining_wall_required_seam_edges: int = int(patch_data.get("terrain_cdt_retaining_wall_required_seam_edges", 0))
		var cdt_retaining_wall_required_seam_faces: int = int(patch_data.get("terrain_cdt_retaining_wall_required_seam_faces", 0))
		var cdt_blocking_degenerate_seam_edges: int = int(patch_data.get("terrain_cdt_blocking_degenerate_seam_edges", 0))
		var cdt_tie_in_widened_source_samples: int = int(patch_data.get("terrain_cdt_tie_in_widened_source_samples", 0))
		var cdt_tie_in_widened_max_y_delta_m: float = float(patch_data.get("terrain_cdt_tie_in_widened_max_y_delta_m", 0.0))
		var cdt_tie_in_widened_max_slope_ratio: float = float(patch_data.get("terrain_cdt_tie_in_widened_max_slope_ratio", 0.0))
		var cdt_invalid_constraint_samples: String = _road_geometry_terrain_invalid_constraint_samples_label(patch_data)
		var cdt_road_seam_samples: String = _road_geometry_terrain_seam_samples_label(patch_data)
		var cdt_retaining_wall_samples: String = _road_geometry_terrain_retaining_wall_samples_label(patch_data)
		var cdt_seam_quality_samples: String = _road_geometry_terrain_seam_quality_samples_label(patch_data)
		var cdt_tie_in_widened_samples: String = _road_geometry_terrain_tie_in_widened_samples_label(patch_data)
		lines.append(
			"terrain_patch key=(%d,%d) resident=%s road_locked=%s mesh=\"%s\" sample=%dx%d texture=%dx%d world_origin=(%.3f,%.3f) world_size=(%.3f,%.3f) watermap=terrain_aligned:%dx%d water_nonzero=%d water_world_origin=(%.3f,%.3f) water_world_size=(%.3f,%.3f) height_min=%.3f height_max=%.3f clip_groups=%d clip_loops=%d clip_points=%d clip_area=%.3f clip_bounds=%s max_clip_bbox=(%.3f,%.3f) baked_vertices=%d retaining_vertices=%d baked_mesh=%s cdt_status=%s cdt_error=%s cdt_stage=%s cdt_backend=%s cdt_input_vertices=%d cdt_constraints=%d cdt_road_constraints=%d cdt_preserved_road_constraints=%d cdt_spade_missing_road_constraints=%d cdt_rejected_road_constraints=%d cdt_internal_road_constraints=%d cdt_invalid_constraints=%d cdt_accepted_faces=%d cdt_rejected_road_faces=%d cdt_emitted_faces=%d cdt_retaining_wall_emitted_faces=%d cdt_terrain_face_sources=%s cdt_retaining_wall_face_sources=%s cdt_face_max_y_delta=%.3f cdt_face_max_slope=%.3f cdt_longest_triangle_edge=%.3f cdt_road_seam_faces=%d cdt_road_seam_max_y_delta=%.3f cdt_road_seam_max_slope=%.3f cdt_retaining_wall_faces=%d cdt_retaining_wall_max_y_delta=%.3f cdt_retaining_wall_max_slope=%.3f cdt_seam_quality={accepted=%d,merged_subbudget=%d,omitted_near_samples=%d,retaining_wall_required_edges=%d,retaining_wall_required_faces=%d,blocking_degenerate=%d,samples=%s} cdt_tie_in_widened_samples=%d cdt_tie_in_widened_max_y_delta=%.3f cdt_tie_in_widened_max_slope=%.3f cdt_invalid_samples=%s cdt_road_seam_samples=%s cdt_retaining_wall_samples=%s cdt_tie_in_widened_sample_points=%s"
			% [
				key.x,
				key.y,
				str(resident_patch_lookup.has(key)),
				str(road_locked_patch_lookup.has(key)),
				_road_geometry_mesh_label(mesh),
				int(patch_data["sample_width"]),
				int(patch_data["sample_height"]),
				int(patch_data["texture_width"]),
				int(patch_data["texture_height"]),
				float(patch_data["world_origin_x"]),
				float(patch_data["world_origin_z"]),
				float(patch_data["world_size_x"]),
				float(patch_data["world_size_z"]),
				water_texture_width,
				water_texture_height,
				water_depth_nonzero_count,
				water_world_origin_x,
				water_world_origin_z,
				water_world_size_x,
				water_world_size_z,
				float(height_stats.get("min", 0.0)),
				float(height_stats.get("max", 0.0)),
				int(clip_stats.get("group_count", 0)),
				int(clip_stats.get("loop_count", 0)),
				int(clip_stats.get("point_count", 0)),
				float(clip_stats.get("area", 0.0)),
				_road_geometry_bounds_label(clip_stats),
				float(clip_stats.get("max_bbox_x", 0.0)),
				float(clip_stats.get("max_bbox_z", 0.0)),
				baked_vertex_count,
				retaining_wall_baked_vertex_count,
				baked_mesh_stats,
				cdt_status,
				cdt_error,
				cdt_stage,
				cdt_backend,
				cdt_input_vertices,
				cdt_constraint_edges,
				cdt_road_constraint_edges,
				cdt_preserved_road_constraint_edges,
				cdt_spade_missing_road_constraint_edges,
				cdt_rejected_road_constraint_edges,
				cdt_internal_road_constraint_edges,
				cdt_invalid_constraints,
				cdt_accepted_faces,
				cdt_rejected_road_faces,
				cdt_emitted_faces,
				cdt_retaining_wall_emitted_faces,
				cdt_terrain_face_sources,
				cdt_retaining_wall_face_sources,
				cdt_max_face_y_delta_m,
				cdt_max_face_slope_ratio,
				cdt_longest_triangle_edge_m,
				cdt_road_seam_faces,
				cdt_road_seam_max_y_delta_m,
				cdt_road_seam_max_slope_ratio,
				cdt_retaining_wall_faces,
				cdt_retaining_wall_max_y_delta_m,
				cdt_retaining_wall_max_slope_ratio,
				cdt_accepted_seam_edges,
				cdt_merged_subbudget_seam_edges,
				cdt_omitted_near_seam_source_samples,
				cdt_retaining_wall_required_seam_edges,
				cdt_retaining_wall_required_seam_faces,
				cdt_blocking_degenerate_seam_edges,
				cdt_seam_quality_samples,
				cdt_tie_in_widened_source_samples,
				cdt_tie_in_widened_max_y_delta_m,
				cdt_tie_in_widened_max_slope_ratio,
				cdt_invalid_constraint_samples,
				cdt_road_seam_samples,
				cdt_retaining_wall_samples,
				cdt_tie_in_widened_samples,
			]
		)
	return lines

func _terrain_patch_payload_render_step_mm(key: Vector2i) -> int:
	if road_locked_patch_lookup.has(key):
		return int(round(ROAD_LOCKED_PATCH_TARGET_RENDER_STEP_M * 1000.0))
	return 0

func _terrain_patch_payload_surface_generation() -> int:
	if simulation_node.has_method("get_road_tool_surface_generation"):
		return int(simulation_node.get_road_tool_surface_generation())
	return 0

func _sync_patch_payload_surface_generation() -> int:
	var surface_generation := _terrain_patch_payload_surface_generation()
	if surface_generation != patch_payload_surface_generation:
		patch_payload_requested.clear()
		patch_payload_requested_generation.clear()
		patch_payload_ready.clear()
		patch_payload_surface_generation = surface_generation
	return surface_generation

func _request_terrain_patch_payload(key: Vector2i, include_existing: bool = false) -> bool:
	var keys: Array[Vector2i] = []
	keys.append(key)
	return _request_terrain_patch_payloads(keys, 1, include_existing) > 0

func _request_terrain_patch_payloads(
	keys: Array[Vector2i],
	budget: int,
	include_existing: bool = false
) -> int:
	if budget <= 0 or not simulation_node.has_method("request_terrain_patch_payloads"):
		return 0
	var flat_requests := PackedInt32Array()
	var requested_count := 0
	var refined_requested_count := 0
	var surface_generation := _sync_patch_payload_surface_generation()
	for key in keys:
		if requested_count >= budget:
			break
		if not include_existing and patches.has(key):
			continue
		if patch_payload_ready.has(key):
			var ready_patch_data: Dictionary = patch_payload_ready[key] as Dictionary
			if int(ready_patch_data.get("surface_generation", -1)) == surface_generation:
				continue
			patch_payload_ready.erase(key)
		var render_step_mm := _terrain_patch_payload_render_step_mm(key)
		if render_step_mm > 0 and refined_requested_count >= REFINED_PATCH_PAYLOAD_REQUEST_BUDGET_PER_FRAME:
			continue
		if (
			int(patch_payload_requested.get(key, -1)) == render_step_mm
			and int(patch_payload_requested_generation.get(key, -1)) == surface_generation
		):
			continue
		flat_requests.push_back(key.x)
		flat_requests.push_back(key.y)
		flat_requests.push_back(render_step_mm)
		patch_payload_requested[key] = render_step_mm
		patch_payload_requested_generation[key] = surface_generation
		requested_count += 1
		if render_step_mm > 0:
			refined_requested_count += 1
	if not flat_requests.is_empty():
		simulation_node.request_terrain_patch_payloads(flat_requests)
	return requested_count

func _poll_ready_terrain_patch_payloads(budget: int) -> int:
	if budget <= 0 or not simulation_node.has_method("poll_ready_terrain_patch_payloads"):
		return 0
	var result: Dictionary = simulation_node.poll_ready_terrain_patch_payloads(budget) as Dictionary
	var payloads: Array = result.get("patches", []) as Array
	var accepted_count := 0
	var surface_generation := _sync_patch_payload_surface_generation()
	for payload_variant in payloads:
		var patch_data: Dictionary = payload_variant as Dictionary
		var key := Vector2i(
			int(patch_data.get("patch_x", -1)),
			int(patch_data.get("patch_z", -1))
		)
		if key.x < 0 or key.y < 0:
			continue
		if not patch_payload_requested.has(key):
			continue
		var payload_generation := int(patch_data.get("surface_generation", -1))
		var requested_generation := int(patch_payload_requested_generation.get(key, -1))
		if payload_generation != requested_generation or payload_generation != surface_generation:
			continue
		var render_step_mm := int(patch_data.get("render_step_mm", _terrain_patch_payload_render_step_mm(key)))
		if int(patch_payload_requested.get(key, -1)) != render_step_mm:
			continue
		if bool(patch_data.get("retry", false)):
			patch_payload_requested.erase(key)
			patch_payload_requested_generation.erase(key)
			accepted_count += 1
			continue
		patch_payload_ready[key] = patch_data
		patch_payload_requested.erase(key)
		patch_payload_requested_generation.erase(key)
		accepted_count += 1
	return accepted_count

func _prune_patch_payload_cache() -> void:
	var surface_generation := _sync_patch_payload_surface_generation()
	for key_variant in patch_payload_ready.keys():
		var key: Vector2i = key_variant
		var patch_data: Dictionary = patch_payload_ready[key] as Dictionary
		if int(patch_data.get("surface_generation", -1)) != surface_generation:
			patch_payload_ready.erase(key)
			continue
		if patches.has(key):
			patch_payload_ready.erase(key)
			continue
		if _terrain_residency_target_bounds_valid and not _patch_key_in_bounds(
			key,
			_terrain_residency_target_bounds
		):
			patch_payload_ready.erase(key)
	for key_variant in patch_payload_requested.keys():
		var key: Vector2i = key_variant
		if (
			int(patch_payload_requested_generation.get(key, -1)) != surface_generation
			or (
				_terrain_residency_target_bounds_valid
				and not patches.has(key)
				and not resident_patch_lookup.has(key)
				and not _patch_key_in_bounds(key, _terrain_residency_target_bounds)
			)
		):
			patch_payload_requested.erase(key)
			patch_payload_requested_generation.erase(key)

func _dirty_patch_payloads_ready_for_atomic_upload(keys: Array[Vector2i]) -> bool:
	var all_ready := true
	for key in keys:
		if not patches.has(key):
			continue
		if not _patch_payload_ready_for_key(key):
			all_ready = false
	return all_ready

func _patch_payload_ready_for_key(key: Vector2i) -> bool:
	var expected_render_step_mm := _terrain_patch_payload_render_step_mm(key)
	var expected_generation := _sync_patch_payload_surface_generation()
	if patch_payload_ready.has(key):
		var ready_patch_data: Dictionary = patch_payload_ready[key] as Dictionary
		if (
			int(ready_patch_data.get("render_step_mm", expected_render_step_mm)) == expected_render_step_mm
			and int(ready_patch_data.get("surface_generation", -1)) == expected_generation
		):
			return true
		patch_payload_ready.erase(key)
	_request_terrain_patch_payload(key, true)
	return false

func _terrain_patch_data_for_key(
	key: Vector2i,
	include_debug: bool = false,
	allow_async: bool = false
) -> Dictionary:
	if allow_async and not include_debug and simulation_node.has_method("request_terrain_patch_payloads"):
		var expected_render_step_mm := _terrain_patch_payload_render_step_mm(key)
		var expected_generation := _sync_patch_payload_surface_generation()
		if patch_payload_ready.has(key):
			var ready_patch_data: Dictionary = patch_payload_ready[key] as Dictionary
			if (
				int(ready_patch_data.get("render_step_mm", expected_render_step_mm)) == expected_render_step_mm
				and int(ready_patch_data.get("surface_generation", -1)) == expected_generation
			):
				patch_payload_ready.erase(key)
				patch_payload_requested.erase(key)
				patch_payload_requested_generation.erase(key)
				return _terrain_patch_data_or_empty_refined_fallback(key, ready_patch_data)
			patch_payload_ready.erase(key)
		_request_terrain_patch_payload(key, patches.has(key))
		return {}
	if road_locked_patch_lookup.has(key):
		if include_debug:
			return simulation_node.get_refined_terrain_patch_debug(
				key.x,
				key.y,
				ROAD_LOCKED_PATCH_TARGET_RENDER_STEP_M
			)
		var refined_patch_data: Dictionary = simulation_node.get_refined_terrain_patch(
			key.x,
			key.y,
			ROAD_LOCKED_PATCH_TARGET_RENDER_STEP_M
		)
		return _terrain_patch_data_or_empty_refined_fallback(key, refined_patch_data)
	return simulation_node.get_terrain_patch(key.x, key.y)

func _terrain_patch_data_or_empty_refined_fallback(
	key: Vector2i,
	patch_data: Dictionary
) -> Dictionary:
	if road_locked_patch_lookup.has(key):
		return patch_data
	if _patch_has_empty_refined_cdt(patch_data):
		return simulation_node.get_terrain_patch(key.x, key.y)
	if _patch_has_unusable_refined_cdt(patch_data) and not patches.has(key):
		return simulation_node.get_terrain_patch(key.x, key.y)
	return patch_data

func _terrain_patch_height_bytes(patch_data: Dictionary) -> PackedByteArray:
	var height_bytes: PackedByteArray = (
		patch_data.get("height_bytes", PackedByteArray())
		as PackedByteArray
	)
	if not height_bytes.is_empty():
		return height_bytes
	return (patch_data["height_data"] as PackedFloat32Array).to_byte_array()

func _terrain_patch_height_stats(patch_data: Dictionary) -> Dictionary:
	if patch_data.has("height_data"):
		return _road_geometry_float_stats(patch_data["height_data"] as PackedFloat32Array)
	return _road_geometry_float_stats(PackedFloat32Array())

func _patch_has_road_clip_loops(patch_data: Dictionary) -> bool:
	if (
		not patch_data.has("road_clip_loop_counts")
		or not patch_data.has("road_clip_loop_groups")
		or not patch_data.has("road_clip_loop_roles")
		or not patch_data.has("road_clip_loop_points")
	):
		return false
	var counts := patch_data["road_clip_loop_counts"] as PackedInt32Array
	var groups := patch_data["road_clip_loop_groups"] as PackedInt32Array
	var roles := patch_data["road_clip_loop_roles"] as PackedInt32Array
	var points := patch_data["road_clip_loop_points"] as PackedVector3Array
	if counts.size() == 0:
		return false
	if groups.size() != counts.size() or roles.size() != counts.size():
		return false
	var expected_points := 0
	for index in range(counts.size()):
		var count: int = counts[index]
		if count < 3:
			return false
		if groups[index] < 0:
			return false
		if roles[index] != ROAD_CLIP_LOOP_ROLE_OUTER and roles[index] != ROAD_CLIP_LOOP_ROLE_HOLE:
			return false
		expected_points += count
	return expected_points == points.size()

func _terrain_patch_mesh_from_data(
	patch_data: Dictionary,
	lod_step: int,
	subdivision_factor: int
) -> Mesh:
	if _patch_uses_cdt_terrain_mesh(patch_data):
		return _baked_terrain_patch_mesh(patch_data)
	if not patch_data.has("terrain_cdt_status") and _patch_has_baked_terrain_mesh(patch_data):
		return _baked_terrain_patch_mesh(patch_data)
	return _patch_mesh(
		int(patch_data["sample_width"]),
		int(patch_data["sample_height"]),
		float(patch_data["world_size_x"]),
		float(patch_data["world_size_z"]),
		lod_step,
		subdivision_factor
	)

func _patch_uses_cdt_terrain_mesh(patch_data: Dictionary) -> bool:
	# Failed CDT keeps diagnostic fields but must not replace the heightmap mesh with an empty bake.
	if bool(patch_data.get("terrain_cdt_mesh_suppressed", false)):
		return false
	return (
		patch_data.has("terrain_cdt_status")
		and _patch_has_current_cdt_contract(patch_data)
		and not _patch_has_bad_refined_cdt_status(patch_data)
		and _patch_has_baked_terrain_mesh(patch_data)
	)

func _patch_has_current_cdt_contract(patch_data: Dictionary) -> bool:
	return int(patch_data.get("terrain_cdt_contract_revision", -1)) == TERRAIN_CDT_CONTRACT_REVISION

func _patch_has_bad_refined_cdt_status(patch_data: Dictionary) -> bool:
	var cdt_status := str(patch_data.get("terrain_cdt_status", ""))
	return cdt_status == "failed" or cdt_status == "conflicted" or cdt_status == "pathological"

func _patch_has_empty_refined_cdt(patch_data: Dictionary) -> bool:
	return (
		bool(patch_data.get("terrain_cdt_empty_refined", false))
		or str(patch_data.get("terrain_cdt_status", "")) == "empty"
	)

func _patch_has_unusable_refined_cdt(patch_data: Dictionary) -> bool:
	if patch_data.has("terrain_cdt_status") and not _patch_has_current_cdt_contract(patch_data):
		return true
	return _patch_has_empty_refined_cdt(patch_data) or _patch_has_bad_refined_cdt_status(patch_data)

func _last_renderable_road_locked_patch_data(patch: Dictionary) -> Dictionary:
	var previous_patch_data: Dictionary = patch.get("last_patch_data", {}) as Dictionary
	if _road_locked_patch_data_is_renderable(previous_patch_data):
		return previous_patch_data
	return {}

func _road_locked_patch_data_is_renderable(patch_data: Dictionary) -> bool:
	if patch_data.is_empty():
		return false
	if _patch_has_unusable_refined_cdt(patch_data):
		return false
	return _patch_uses_cdt_terrain_mesh(patch_data)

func _patch_has_baked_terrain_mesh(patch_data: Dictionary) -> bool:
	if not patch_data.has("terrain_mesh_vertices"):
		return false
	var vertices: PackedVector3Array = patch_data["terrain_mesh_vertices"] as PackedVector3Array
	return vertices.size() >= 3

func _patch_has_retaining_wall_mesh(patch_data: Dictionary) -> bool:
	if bool(patch_data.get("terrain_cdt_mesh_suppressed", false)):
		return false
	if not patch_data.has("terrain_retaining_wall_mesh_vertices"):
		return false
	var vertices: PackedVector3Array = patch_data["terrain_retaining_wall_mesh_vertices"] as PackedVector3Array
	return vertices.size() >= 3

func _baked_terrain_patch_mesh(patch_data: Dictionary) -> ArrayMesh:
	var vertices: PackedVector3Array = patch_data["terrain_mesh_vertices"] as PackedVector3Array
	var normals: PackedVector3Array = patch_data["terrain_mesh_normals"] as PackedVector3Array
	var uvs: PackedVector2Array = patch_data["terrain_mesh_uvs"] as PackedVector2Array
	var indices: PackedInt32Array = patch_data.get("terrain_mesh_indices", PackedInt32Array()) as PackedInt32Array
	var mesh: ArrayMesh = ArrayMesh.new()
	if vertices.size() < 3:
		return mesh
	var arrays: Array = []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	if normals.size() == vertices.size():
		arrays[Mesh.ARRAY_NORMAL] = normals
	if uvs.size() == vertices.size():
		arrays[Mesh.ARRAY_TEX_UV] = uvs
	if indices.size() >= 3:
		arrays[Mesh.ARRAY_INDEX] = indices
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh

func _retaining_wall_patch_mesh(patch_data: Dictionary) -> ArrayMesh:
	var mesh: ArrayMesh = ArrayMesh.new()
	if not _patch_has_retaining_wall_mesh(patch_data):
		return mesh
	var vertices: PackedVector3Array = patch_data["terrain_retaining_wall_mesh_vertices"] as PackedVector3Array
	var normals: PackedVector3Array = (
		patch_data.get("terrain_retaining_wall_mesh_normals", PackedVector3Array())
		as PackedVector3Array
	)
	var uvs: PackedVector2Array = (
		patch_data.get("terrain_retaining_wall_mesh_uvs", PackedVector2Array())
		as PackedVector2Array
	)
	var indices: PackedInt32Array = (
		patch_data.get("terrain_retaining_wall_mesh_indices", PackedInt32Array())
		as PackedInt32Array
	)
	var arrays: Array = []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	if normals.size() == vertices.size():
		arrays[Mesh.ARRAY_NORMAL] = normals
	if uvs.size() == vertices.size():
		arrays[Mesh.ARRAY_TEX_UV] = uvs
	if indices.size() >= 3:
		arrays[Mesh.ARRAY_INDEX] = indices
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh

func _retaining_wall_material() -> StandardMaterial3D:
	if retaining_wall_material == null:
		retaining_wall_material = StandardMaterial3D.new()
		retaining_wall_material.albedo_color = RETAINING_WALL_COLOR
		retaining_wall_material.roughness = RETAINING_WALL_ROUGHNESS
		retaining_wall_material.metallic = 0.0
		retaining_wall_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	return retaining_wall_material

func _road_clip_loop_groups_from_patch_data(patch_data: Dictionary) -> Array:
	if not _patch_has_road_clip_loops(patch_data):
		return []
	var counts := patch_data["road_clip_loop_counts"] as PackedInt32Array
	var group_indices := patch_data["road_clip_loop_groups"] as PackedInt32Array
	var roles := patch_data["road_clip_loop_roles"] as PackedInt32Array
	var points := patch_data["road_clip_loop_points"] as PackedVector3Array
	var groups_by_id: Dictionary = {}
	var group_ids: Array = []
	var cursor := 0
	for loop_index in range(counts.size()):
		var count: int = counts[loop_index]
		var group_id: int = group_indices[loop_index]
		var loop_points := PackedVector2Array()
		for offset in range(count):
			var point := points[cursor + offset]
			loop_points.append(Vector2(point.x, point.z))
		var loop_bounds := _polygon_bounds(loop_points)
		var loop_entry := {
			"points": loop_points,
			"bounds": loop_bounds,
			"role": roles[loop_index],
		}
		if not groups_by_id.has(group_id):
			groups_by_id[group_id] = {
				"group_id": group_id,
				"outer_loops": [],
				"hole_loops": [],
				"bounds": loop_bounds,
				"has_bounds": false,
			}
			group_ids.append(group_id)
		var group_entry: Dictionary = groups_by_id[group_id]
		if bool(group_entry["has_bounds"]):
			group_entry["bounds"] = _merge_bounds(group_entry["bounds"], loop_bounds)
		else:
			group_entry["bounds"] = loop_bounds
			group_entry["has_bounds"] = true
		if roles[loop_index] == ROAD_CLIP_LOOP_ROLE_HOLE:
			var hole_loops: Array = group_entry["hole_loops"]
			hole_loops.append(loop_entry)
		else:
			var outer_loops: Array = group_entry["outer_loops"]
			outer_loops.append(loop_entry)
		cursor += count
	group_ids.sort()
	var loop_groups: Array = []
	for group_id_variant in group_ids:
		var group_id: int = int(group_id_variant)
		var group_entry: Dictionary = groups_by_id[group_id]
		var outer_loops: Array = group_entry["outer_loops"]
		if outer_loops.is_empty():
			continue
		loop_groups.append({
			"group_id": group_id,
			"outer_loops": outer_loops,
			"hole_loops": group_entry["hole_loops"],
			"bounds": group_entry["bounds"],
		})
	return loop_groups

func _polygon_bounds(polygon: PackedVector2Array) -> Rect2:
	if polygon.size() == 0:
		return Rect2()
	var min_x := polygon[0].x
	var max_x := polygon[0].x
	var min_y := polygon[0].y
	var max_y := polygon[0].y
	for point in polygon:
		min_x = minf(min_x, point.x)
		max_x = maxf(max_x, point.x)
		min_y = minf(min_y, point.y)
		max_y = maxf(max_y, point.y)
	return Rect2(Vector2(min_x, min_y), Vector2(max_x - min_x, max_y - min_y))

func _merge_bounds(a: Rect2, b: Rect2) -> Rect2:
	var min_x := minf(a.position.x, b.position.x)
	var min_y := minf(a.position.y, b.position.y)
	var max_x := maxf(a.position.x + a.size.x, b.position.x + b.size.x)
	var max_y := maxf(a.position.y + a.size.y, b.position.y + b.size.y)
	return Rect2(Vector2(min_x, min_y), Vector2(max_x - min_x, max_y - min_y))

func _prewarm_regular_terrain_mesh_variants() -> void:
	if patch_interval_cells <= 0 or patch_span_m <= 0.0:
		return
	var sample_count: int = patch_interval_cells + 1
	var lod_steps: Array[int] = [1, 2, 4, 8]
	var subdivision_factors: Array[int] = [1]
	var road_subdivision_factor: int = max(
		1,
		int(ceili(terrain_cell_m / ROAD_LOCKED_PATCH_TARGET_RENDER_STEP_M))
	)
	if road_subdivision_factor != 1:
		subdivision_factors.append(road_subdivision_factor)
	for lod_step: int in lod_steps:
		for subdivision_factor: int in subdivision_factors:
			_patch_mesh(
				sample_count,
				sample_count,
				patch_span_m,
				patch_span_m,
				lod_step,
				subdivision_factor
			)

func _patch_mesh(
	sample_width: int,
	sample_height: int,
	world_size_x: float,
	world_size_z: float,
	lod_step: int,
	subdivision_factor: int
) -> PlaneMesh:
	var mesh_cache_key := _patch_mesh_cache_key(
		sample_width,
		sample_height,
		world_size_x,
		world_size_z,
		lod_step,
		subdivision_factor
	)
	var patch_mesh: PlaneMesh
	if patch_mesh_cache.has(mesh_cache_key):
		patch_mesh = patch_mesh_cache[mesh_cache_key]
	else:
		patch_mesh = PlaneMesh.new()
		patch_mesh.size = Vector2(world_size_x, world_size_z)
		patch_mesh.subdivide_width = _mesh_subdivisions_for_sample_count(
			sample_width,
			lod_step,
			subdivision_factor
		)
		patch_mesh.subdivide_depth = _mesh_subdivisions_for_sample_count(
			sample_height,
			lod_step,
			subdivision_factor
		)
		patch_mesh_cache[mesh_cache_key] = patch_mesh
	return patch_mesh

func _patch_mesh_cache_key(
	sample_width: int,
	sample_height: int,
	world_size_x: float,
	world_size_z: float,
	lod_step: int,
	subdivision_factor: int
) -> String:
	return "%d:%d:%.3f:%.3f:%d:%d" % [
		sample_width,
		sample_height,
		world_size_x,
		world_size_z,
		lod_step,
		subdivision_factor,
	]

func _mesh_subdivisions_for_sample_count(
	sample_count: int,
	lod_step: int,
	subdivision_factor: int
) -> int:
	var interval_count: int = max(0, sample_count - 1)
	var effective_interval_count: int = interval_count * max(1, subdivision_factor)
	var lod_vertex_count: int = max(
		2,
		int(ceili(float(effective_interval_count) / float(max(1, lod_step)))) + 1
	)
	return max(0, lod_vertex_count - 2)

func _mesh_subdivision_factor_for_patch(key: Vector2i, sample_step_m: float) -> int:
	if not road_locked_patch_lookup.has(key):
		return 1
	return max(1, int(ceili(sample_step_m / ROAD_LOCKED_PATCH_TARGET_RENDER_STEP_M)))

func _mesh_lod_step_for_patch(key: Vector2i, center_x: float, center_z: float) -> int:
	var camera: Camera3D = get_viewport().get_camera_3d()
	if camera == null:
		return _mesh_lod_step_for_patch_with_camera(key, center_x, center_z, Vector3.ZERO, false)
	return _mesh_lod_step_for_patch_with_camera(key, center_x, center_z, camera.global_position, true)

func _mesh_lod_step_for_patch_with_camera(
	key: Vector2i,
	center_x: float,
	center_z: float,
	camera_position: Vector3,
	camera_valid: bool
) -> int:
	if _terrain_force_lod1:
		return 1
	if road_locked_patch_lookup.has(key):
		return 1
	if not camera_valid:
		return 1
	var distance_m := camera_position.distance_to(Vector3(center_x, 0.0, center_z))
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
	_terrain_lod_last_deferred_count = 0
	if resident_patch_lookup.is_empty():
		_terrain_mesh_lod_refresh_elapsed_s = 0.0
		patch_lod_refresh_queue.clear()
		patch_lod_refresh_lookup.clear()
		_record_lod_perf_counters(0, 0, 0, 0)
		return
	var queued_count := 0
	var replaced_count: int = 0
	_terrain_lod_last_skipped_count = 0
	_terrain_mesh_lod_refresh_elapsed_s += delta
	if (
		_terrain_mesh_lod_refresh_elapsed_s >= PATCH_MESH_LOD_REFRESH_INTERVAL_S
		and _lod_refresh_camera_moved(PATCH_MESH_LOD_REFRESH_CAMERA_MOVE_M)
	):
		_terrain_mesh_lod_refresh_elapsed_s = 0.0
		replaced_count = patch_lod_refresh_queue.size()
		queued_count = _replace_resident_patch_lod_refreshes()
	_process_patch_lod_refresh_queue(
		PATCH_MESH_LOD_REFRESH_BUDGET_MS,
		PATCH_MESH_LOD_REFRESH_MAX_CHECKS_PER_FRAME,
		PATCH_MESH_LOD_REFRESH_MAX_CHANGES_PER_FRAME
	)
	_terrain_lod_last_queued_count = queued_count
	_terrain_lod_last_queue_count = patch_lod_refresh_queue.size()
	_terrain_lod_last_replaced_count = replaced_count

func _defer_patch_mesh_lods(delta: float) -> void:
	_terrain_mesh_lod_refresh_elapsed_s += delta
	_terrain_lod_last_deferred_count = 1
	_record_lod_perf_counters(
		0,
		0,
		0,
		patch_lod_refresh_queue.size(),
		0,
		0
	)

func _lod_refresh_camera_moved(min_distance_m: float) -> bool:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return true
	var position: Vector3 = camera.global_position
	if not _terrain_lod_refresh_camera_valid:
		_terrain_lod_refresh_camera_valid = true
		_terrain_lod_refresh_last_camera_position = position
		return true
	if position.distance_squared_to(_terrain_lod_refresh_last_camera_position) < min_distance_m * min_distance_m:
		return false
	_terrain_lod_refresh_last_camera_position = position
	return true

func _replace_resident_patch_lod_refreshes() -> int:
	var keys: Array[Vector2i] = get_resident_patch_keys()
	var candidates: Array[Vector2i] = []
	var camera: Camera3D = get_viewport().get_camera_3d()
	var camera_position: Vector3 = Vector3.ZERO
	var camera_valid: bool = camera != null
	if camera_valid:
		camera_position = camera.global_position
	for key in keys:
		if _terrain_patch_lod_refresh_needed(key, camera_position, camera_valid):
			candidates.append(key)
		else:
			_terrain_lod_last_skipped_count += 1
	_sort_patch_keys_by_camera_priority(candidates)
	patch_lod_refresh_lookup.clear()
	for key in candidates:
		patch_lod_refresh_lookup[key] = true
	patch_lod_refresh_queue = candidates
	return candidates.size()

func _terrain_patch_lod_refresh_needed(
	key: Vector2i,
	camera_position: Vector3,
	camera_valid: bool
) -> bool:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return false
	if bool(patch.get("height_is_baked", false)):
		return false
	var patch_node: MeshInstance3D = patch["node"]
	var target_lod_step: int = _mesh_lod_step_for_patch_with_camera(
		key,
		patch_node.position.x,
		patch_node.position.z,
		camera_position,
		camera_valid
	)
	var target_subdivision_factor: int = _mesh_subdivision_factor_for_patch(
		key,
		float(patch.get("sample_step_m", terrain_cell_m))
	)
	return (
		int(patch.get("lod_step", 1)) != target_lod_step
		or int(patch.get("subdivision_factor", 1)) != target_subdivision_factor
	)

func _process_patch_lod_refresh_queue(
	refresh_budget_ms: float,
	max_checks_per_frame: int,
	max_changes_per_frame: int
) -> void:
	_terrain_lod_last_processed_count = 0
	_terrain_lod_last_changed_count = 0
	if patch_lod_refresh_queue.is_empty():
		return
	var refresh_start_us: int = Time.get_ticks_usec()
	var processed_count := 0
	var changed_count := 0
	while not patch_lod_refresh_queue.is_empty():
		if processed_count >= max_checks_per_frame or changed_count >= max_changes_per_frame:
			break
		if _time_budget_exhausted(refresh_start_us, refresh_budget_ms, processed_count):
			break
		var key: Vector2i = patch_lod_refresh_queue.pop_front()
		patch_lod_refresh_lookup.erase(key)
		if resident_patch_lookup.has(key):
			if _refresh_one_patch_mesh_lod(key):
				changed_count += 1
		processed_count += 1
	_terrain_lod_last_processed_count = processed_count
	_terrain_lod_last_changed_count = changed_count

func _refresh_one_patch_mesh_lod(key: Vector2i) -> bool:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return false
	if _patch_is_blocked_by_bad_cdt(patch):
		return false
	if bool(patch.get("height_is_baked", false)):
		return false
	var patch_node: MeshInstance3D = patch["node"]
	var target_lod_step := _mesh_lod_step_for_patch(key, patch_node.position.x, patch_node.position.z)
	var target_subdivision_factor := _mesh_subdivision_factor_for_patch(
		key,
		float(patch.get("sample_step_m", terrain_cell_m))
	)
	var current_lod_step := int(patch.get("lod_step", 1))
	var current_subdivision_factor := int(patch.get("subdivision_factor", 1))
	if current_lod_step == target_lod_step and current_subdivision_factor == target_subdivision_factor:
		return false
	var patch_data: Dictionary = patch.get("last_patch_data", {})
	if patch_data.is_empty():
		patch_data = _terrain_patch_data_for_key(key, false)
	if patch_data.is_empty():
		return false
	if road_locked_patch_lookup.has(key) and not _road_locked_patch_data_is_renderable(patch_data):
		return false
	patch["lod_step"] = target_lod_step
	patch["subdivision_factor"] = target_subdivision_factor
	var material: ShaderMaterial = patch["material"]
	material.set_shader_parameter("terrain_debug_lod_step", float(target_lod_step))
	patch_node.mesh = _terrain_patch_mesh_from_data(
		patch_data,
		target_lod_step,
		target_subdivision_factor
	)
	return true

func _record_lod_perf_counters(
	processed_count: int,
	changed_count: int,
	queued_count: int,
	queue_count: int,
	replaced_count: int = 0,
	skipped_count: int = 0
) -> void:
	_terrain_lod_last_processed_count = processed_count
	_terrain_lod_last_changed_count = changed_count
	_terrain_lod_last_queued_count = queued_count
	_terrain_lod_last_queue_count = queue_count
	_terrain_lod_last_replaced_count = replaced_count
	_terrain_lod_last_skipped_count = skipped_count

func _refresh_road_locked_patch_lookup() -> void:
	road_locked_patch_lookup.clear()
	var flat_pairs: PackedInt32Array = simulation_node.get_road_locked_terrain_patches()
	var pair_count: int = flat_pairs.size() / 2
	for index in range(pair_count):
		road_locked_patch_lookup[Vector2i(flat_pairs[index * 2], flat_pairs[index * 2 + 1])] = true

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

func _ensure_grass_textures() -> void:
	if grass_albedo_texture == null:
		grass_albedo_texture = _load_texture_or_solid(TERRAIN_GRASS_ALBEDO_PATH, Color(0.5, 0.5, 0.5, 1.0))
	if grass_height_texture == null:
		grass_height_texture = _load_texture_or_solid(TERRAIN_GRASS_HEIGHT_PATH, Color(0.5, 0.5, 0.5, 1.0))

func _load_texture_or_solid(path: String, fallback_color: Color) -> Texture2D:
	var texture: Texture2D = null
	if ResourceLoader.exists(path):
		texture = load(path) as Texture2D
	if texture != null:
		return texture

	var image := Image.load_from_file(ProjectSettings.globalize_path(path))
	if image:
		image.generate_mipmaps()
		return ImageTexture.create_from_image(image)

	var fallback_image := Image.create(1, 1, false, Image.FORMAT_RGBA8)
	fallback_image.fill(fallback_color)
	return ImageTexture.create_from_image(fallback_image)

func _bind_empty_water_texture(patch: Dictionary, material: ShaderMaterial) -> void:
	if patch.get("water_texture", null) != empty_water_texture:
		material.set_shader_parameter("watermap", empty_water_texture)
	material.set_shader_parameter("watermap_texture_size", Vector2(2, 2))
	material.set_shader_parameter("watermap_inner_sample_offset_texels", Vector2.ZERO)
	material.set_shader_parameter("watermap_inner_sample_size_texels", Vector2(2, 2))
	patch["water_texture"] = empty_water_texture
	patch["water_texture_width"] = 2
	patch["water_texture_height"] = 2
	patch["water_inner_offset_x"] = 0
	patch["water_inner_offset_z"] = 0
	patch["water_sample_width"] = 2
	patch["water_sample_height"] = 2
	patch["water_depth_nonzero_count"] = 0

func _sync_water_patch_textures() -> void:
	_queue_all_water_patch_texture_syncs()
	_process_water_patch_texture_sync_queue(PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME)

func _queue_all_water_patch_texture_syncs() -> void:
	var keys: Array[Vector2i] = get_resident_patch_keys()
	_sort_patch_keys_by_camera_priority(keys)
	for key in keys:
		_queue_water_patch_texture_sync(key)

func _queue_water_patch_texture_sync(key: Vector2i) -> void:
	if water_texture_sync_lookup.has(key):
		return
	water_texture_sync_lookup[key] = true
	water_texture_sync_queue.append(key)

func _process_water_patch_texture_sync_queue(
	budget: int,
	collect_perf_stats: bool = false
) -> Dictionary:
	var perf_stats := _new_water_sync_perf_stats() if collect_perf_stats else {}
	var sync_start_us := Time.get_ticks_usec()
	var processed_count := 0
	var updated_count := 0
	var missing_count := 0
	var depth_nonzero_total := 0
	var remaining_budget := budget
	while remaining_budget > 0 and not water_texture_sync_queue.is_empty():
		if _time_budget_exhausted(sync_start_us, PATCH_WATER_TEXTURE_SYNC_BUDGET_MS, processed_count):
			break
		var key: Vector2i = water_texture_sync_queue.pop_front()
		water_texture_sync_lookup.erase(key)
		var depth_nonzero_count := _sync_one_water_patch_texture(key, perf_stats)
		processed_count += 1
		if depth_nonzero_count >= 0:
			updated_count += 1
			depth_nonzero_total += depth_nonzero_count
		else:
			missing_count += 1
		remaining_budget -= 1

	if processed_count > 0 and _terrain_debug_enabled and (_terrain_debug_verbose or _terrain_visual_debug_mode >= 6):
		_terrain_debug_log(
			"watermap_sync source=water_renderer_texture processed=%d updated=%d missing=%d queued=%d depth_nonzero=%d elapsed_ms=%.3f"
			% [
				processed_count,
				updated_count,
				missing_count,
				water_texture_sync_queue.size(),
				depth_nonzero_total,
				float(Time.get_ticks_usec() - sync_start_us) / 1000.0,
			]
		)
	if collect_perf_stats:
		perf_stats["water_sync_processed_count"] = float(processed_count)
		perf_stats["water_sync_updated_count"] = float(updated_count)
		perf_stats["water_sync_missing_count"] = float(missing_count)
		perf_stats["water_sync_queued_count"] = float(water_texture_sync_queue.size())
	return perf_stats

func _new_water_sync_perf_stats() -> Dictionary:
	return {
		"water_sync_fetch": 0.0,
		"water_sync_bytes": 0.0,
		"water_sync_image": 0.0,
		"water_sync_texture": 0.0,
		"water_sync_bind": 0.0,
		"water_sync_metadata": 0.0,
		"water_sync_processed_count": 0.0,
		"water_sync_updated_count": 0.0,
		"water_sync_missing_count": 0.0,
		"water_sync_queued_count": 0.0,
	}

func _sync_one_water_patch_texture(key: Vector2i, perf_stats: Dictionary = {}) -> int:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return -1
	var material: ShaderMaterial = patch["material"]
	var collect_perf_stats := perf_stats.has("water_sync_fetch")
	if water_node == null or not water_node.has_method("get_water_patch_texture_binding"):
		if collect_perf_stats:
			var bind_missing_node_start_us := Time.get_ticks_usec()
			_bind_empty_water_texture(patch, material)
			perf_stats["water_sync_bind"] = (
				float(perf_stats["water_sync_bind"])
				+ float(Time.get_ticks_usec() - bind_missing_node_start_us) / 1000.0
			)
		else:
			_bind_empty_water_texture(patch, material)
		return -1

	var water_binding: Dictionary
	if collect_perf_stats:
		var fetch_start_us := Time.get_ticks_usec()
		water_binding = water_node.get_water_patch_texture_binding(key)
		perf_stats["water_sync_fetch"] = (
			float(perf_stats["water_sync_fetch"])
			+ float(Time.get_ticks_usec() - fetch_start_us) / 1000.0
		)
	else:
		water_binding = water_node.get_water_patch_texture_binding(key)
	if water_binding.is_empty():
		if collect_perf_stats:
			var bind_empty_start_us := Time.get_ticks_usec()
			_bind_empty_water_texture(patch, material)
			perf_stats["water_sync_bind"] = (
				float(perf_stats["water_sync_bind"])
				+ float(Time.get_ticks_usec() - bind_empty_start_us) / 1000.0
			)
		else:
			_bind_empty_water_texture(patch, material)
		return -1

	var water_texture: Texture2D = water_binding.get("texture", null) as Texture2D
	var texture_width := int(water_binding.get("texture_width", 0))
	var texture_height := int(water_binding.get("texture_height", 0))
	var inner_offset_x := int(water_binding.get("inner_offset_x", 0))
	var inner_offset_z := int(water_binding.get("inner_offset_z", 0))
	var sample_width := int(water_binding.get("sample_width", 0))
	var sample_height := int(water_binding.get("sample_height", 0))
	if (
		water_texture == null
		or texture_width <= 0
		or texture_height <= 0
		or sample_width <= 0
		or sample_height <= 0
	):
		if collect_perf_stats:
			var bind_invalid_start_us := Time.get_ticks_usec()
			_bind_empty_water_texture(patch, material)
			perf_stats["water_sync_bind"] = (
				float(perf_stats["water_sync_bind"])
				+ float(Time.get_ticks_usec() - bind_invalid_start_us) / 1000.0
			)
		else:
			_bind_empty_water_texture(patch, material)
		return -1

	if collect_perf_stats:
		var bind_start_us := Time.get_ticks_usec()
		_bind_water_texture_from_binding(
			patch,
			material,
			water_texture,
			texture_width,
			texture_height,
			inner_offset_x,
			inner_offset_z,
			sample_width,
			sample_height
		)
		perf_stats["water_sync_bind"] = (
			float(perf_stats["water_sync_bind"])
			+ float(Time.get_ticks_usec() - bind_start_us) / 1000.0
		)
	else:
		_bind_water_texture_from_binding(
			patch,
			material,
			water_texture,
			texture_width,
			texture_height,
			inner_offset_x,
			inner_offset_z,
			sample_width,
			sample_height
		)

	var depth_nonzero_count := int(water_binding.get("depth_nonzero_count", 0))
	if collect_perf_stats:
		var metadata_start_us := Time.get_ticks_usec()
		patch["water_depth_nonzero_count"] = depth_nonzero_count
		patch["water_world_origin_x"] = float(water_binding.get("world_origin_x", 0.0))
		patch["water_world_origin_z"] = float(water_binding.get("world_origin_z", 0.0))
		patch["water_world_size_x"] = float(water_binding.get("world_size_x", 0.0))
		patch["water_world_size_z"] = float(water_binding.get("world_size_z", 0.0))
		perf_stats["water_sync_metadata"] = (
			float(perf_stats["water_sync_metadata"])
			+ float(Time.get_ticks_usec() - metadata_start_us) / 1000.0
		)
	else:
		patch["water_depth_nonzero_count"] = depth_nonzero_count
		patch["water_world_origin_x"] = float(water_binding.get("world_origin_x", 0.0))
		patch["water_world_origin_z"] = float(water_binding.get("world_origin_z", 0.0))
		patch["water_world_size_x"] = float(water_binding.get("world_size_x", 0.0))
		patch["water_world_size_z"] = float(water_binding.get("world_size_z", 0.0))
	return depth_nonzero_count

func _bind_water_texture_from_binding(
	patch: Dictionary,
	material: ShaderMaterial,
	water_texture: Texture2D,
	texture_width: int,
	texture_height: int,
	inner_offset_x: int,
	inner_offset_z: int,
	sample_width: int,
	sample_height: int
) -> void:
	var texture_changed: bool = patch.get("water_texture", null) != water_texture
	var texture_layout_changed: bool = (
		int(patch.get("water_texture_width", 0)) != texture_width
		or int(patch.get("water_texture_height", 0)) != texture_height
		or int(patch.get("water_inner_offset_x", 0)) != inner_offset_x
		or int(patch.get("water_inner_offset_z", 0)) != inner_offset_z
		or int(patch.get("water_sample_width", 0)) != sample_width
		or int(patch.get("water_sample_height", 0)) != sample_height
	)
	if texture_changed:
		material.set_shader_parameter("watermap", water_texture)
	if texture_layout_changed:
		material.set_shader_parameter("watermap_texture_size", Vector2(texture_width, texture_height))
		material.set_shader_parameter(
			"watermap_inner_sample_offset_texels",
			Vector2(inner_offset_x, inner_offset_z)
		)
		material.set_shader_parameter(
			"watermap_inner_sample_size_texels",
			Vector2(sample_width, sample_height)
		)
	patch["water_texture"] = water_texture
	patch["water_texture_width"] = texture_width
	patch["water_texture_height"] = texture_height
	patch["water_inner_offset_x"] = inner_offset_x
	patch["water_inner_offset_z"] = inner_offset_z
	patch["water_sample_width"] = sample_width
	patch["water_sample_height"] = sample_height

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
		SceneLightingConfig.apply_shadow_policy(
			border_skirt_instance,
			SceneLightingConfig.SHADOW_RECEIVER_ONLY,
			"terrain"
		)
		border_skirt_instance.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
		add_child(border_skirt_instance)
	if border_bottom_cap_instance == null:
		border_bottom_cap_instance = MeshInstance3D.new()
		border_bottom_cap_instance.name = "TerrainBorderBottomCap"
		SceneLightingConfig.apply_shadow_policy(
			border_bottom_cap_instance,
			SceneLightingConfig.SHADOW_RECEIVER_ONLY,
			"terrain"
		)
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

func _road_geometry_float_stats(values: PackedFloat32Array) -> Dictionary:
	if values.is_empty():
		return {
			"min": 0.0,
			"max": 0.0,
			"nonzero": 0,
			"sum": 0.0,
		}
	var min_value: float = values[0]
	var max_value: float = values[0]
	var nonzero_count: int = 0
	var sum_value: float = 0.0
	for value_variant in values:
		var value: float = float(value_variant)
		min_value = minf(min_value, value)
		max_value = maxf(max_value, value)
		sum_value += value
		if absf(value) > 0.001:
			nonzero_count += 1
	return {
		"min": min_value,
		"max": max_value,
		"nonzero": nonzero_count,
		"sum": sum_value,
	}

func _road_geometry_clip_stats(patch_data: Dictionary) -> Dictionary:
	var stats: Dictionary = {
		"group_count": 0,
		"loop_count": 0,
		"point_count": 0,
		"area": 0.0,
		"has_bounds": false,
		"min_x": 0.0,
		"max_x": 0.0,
		"min_z": 0.0,
		"max_z": 0.0,
		"max_bbox_x": 0.0,
		"max_bbox_z": 0.0,
	}
	if not _patch_has_road_clip_loops(patch_data):
		return stats
	var loop_groups: Array = _road_clip_loop_groups_from_patch_data(patch_data)
	var has_bounds: bool = false
	var min_x: float = 0.0
	var max_x: float = 0.0
	var min_z: float = 0.0
	var max_z: float = 0.0
	var point_count: int = 0
	var total_area: float = 0.0
	var max_bbox_x: float = 0.0
	var max_bbox_z: float = 0.0
	var loop_count: int = 0
	for group_variant in loop_groups:
		var clip_group: Dictionary = group_variant
		var bounds: Rect2 = clip_group["bounds"]
		max_bbox_x = maxf(max_bbox_x, bounds.size.x)
		max_bbox_z = maxf(max_bbox_z, bounds.size.y)
		if not has_bounds:
			min_x = bounds.position.x
			max_x = bounds.position.x + bounds.size.x
			min_z = bounds.position.y
			max_z = bounds.position.y + bounds.size.y
			has_bounds = true
		else:
			min_x = minf(min_x, bounds.position.x)
			max_x = maxf(max_x, bounds.position.x + bounds.size.x)
			min_z = minf(min_z, bounds.position.y)
			max_z = maxf(max_z, bounds.position.y + bounds.size.y)
		var group_area: float = 0.0
		var outer_loops: Array = clip_group["outer_loops"]
		for outer_variant in outer_loops:
			var outer: Dictionary = outer_variant
			var outer_points: PackedVector2Array = outer["points"]
			point_count += outer_points.size()
			loop_count += 1
			group_area += absf(_road_geometry_polygon_area(outer_points))
		var hole_loops: Array = clip_group["hole_loops"]
		for hole_variant in hole_loops:
			var hole: Dictionary = hole_variant
			var hole_points: PackedVector2Array = hole["points"]
			point_count += hole_points.size()
			loop_count += 1
			group_area -= absf(_road_geometry_polygon_area(hole_points))
		total_area += maxf(0.0, group_area)
	stats["group_count"] = loop_groups.size()
	stats["loop_count"] = loop_count
	stats["point_count"] = point_count
	stats["area"] = total_area
	stats["has_bounds"] = has_bounds
	stats["min_x"] = min_x
	stats["max_x"] = max_x
	stats["min_z"] = min_z
	stats["max_z"] = max_z
	stats["max_bbox_x"] = max_bbox_x
	stats["max_bbox_z"] = max_bbox_z
	return stats

func _road_geometry_terrain_seam_samples_label(patch_data: Dictionary) -> String:
	if not patch_data.has("terrain_cdt_road_seam_sample_centroids"):
		return "[]"
	var centroids: PackedVector3Array = (
		patch_data["terrain_cdt_road_seam_sample_centroids"] as PackedVector3Array
	)
	var bounds: PackedVector3Array = (
		patch_data.get("terrain_cdt_road_seam_sample_bounds", PackedVector3Array())
		as PackedVector3Array
	)
	var metrics: PackedFloat32Array = (
		patch_data.get("terrain_cdt_road_seam_sample_metrics", PackedFloat32Array())
		as PackedFloat32Array
	)
	var vertices: PackedVector3Array = (
		patch_data.get("terrain_cdt_road_seam_sample_vertices", PackedVector3Array())
		as PackedVector3Array
	)
	var kinds: PackedInt32Array = (
		patch_data.get("terrain_cdt_road_seam_sample_kinds", PackedInt32Array())
		as PackedInt32Array
	)
	var sample_count: int = mini(
		centroids.size(),
		mini(int(bounds.size() / 2), int(metrics.size() / 2))
	)
	sample_count = mini(sample_count, ROAD_GEOMETRY_TERRAIN_SEAM_SAMPLE_LOG_LIMIT)
	if sample_count <= 0:
		return "[]"
	var parts: Array[String] = []
	for index in range(sample_count):
		var centroid: Vector3 = centroids[index]
		var bounds_min: Vector3 = bounds[index * 2]
		var bounds_max: Vector3 = bounds[index * 2 + 1]
		var y_delta_m: float = metrics[index * 2]
		var slope_ratio: float = metrics[index * 2 + 1]
		var kind_label := "terrain"
		if kinds.size() > index:
			kind_label = _road_geometry_terrain_tie_in_kind_label(kinds[index])
		var vertices_label := ""
		if vertices.size() >= (index + 1) * 3:
			var v0: Vector3 = vertices[index * 3]
			var v1: Vector3 = vertices[index * 3 + 1]
			var v2: Vector3 = vertices[index * 3 + 2]
			vertices_label = ",verts=[(%.3f,%.3f,%.3f),(%.3f,%.3f,%.3f),(%.3f,%.3f,%.3f)]" % [
				v0.x,
				v0.y,
				v0.z,
				v1.x,
				v1.y,
				v1.z,
				v2.x,
				v2.y,
				v2.z,
			]
		parts.append(
			"{kind=%s,centroid=(%.3f,%.3f,%.3f),bounds=[(%.3f,%.3f,%.3f)..(%.3f,%.3f,%.3f)],y_delta=%.3f,slope=%.3f%s,sources=%s}"
			% [
				kind_label,
				centroid.x,
				centroid.y,
				centroid.z,
				bounds_min.x,
				bounds_min.y,
				bounds_min.z,
				bounds_max.x,
				bounds_max.y,
				bounds_max.z,
				y_delta_m,
				slope_ratio,
				vertices_label,
				_road_geometry_cdt_sample_sources_label(patch_data, "terrain_cdt_road_seam", index),
			]
		)
	return "[" + ", ".join(parts) + "]"

func _road_geometry_terrain_retaining_wall_samples_label(patch_data: Dictionary) -> String:
	if not patch_data.has("terrain_cdt_retaining_wall_sample_centroids"):
		return "[]"
	var centroids: PackedVector3Array = (
		patch_data["terrain_cdt_retaining_wall_sample_centroids"] as PackedVector3Array
	)
	var bounds: PackedVector3Array = (
		patch_data.get("terrain_cdt_retaining_wall_sample_bounds", PackedVector3Array())
		as PackedVector3Array
	)
	var metrics: PackedFloat32Array = (
		patch_data.get("terrain_cdt_retaining_wall_sample_metrics", PackedFloat32Array())
		as PackedFloat32Array
	)
	var vertices: PackedVector3Array = (
		patch_data.get("terrain_cdt_retaining_wall_sample_vertices", PackedVector3Array())
		as PackedVector3Array
	)
	var sample_count: int = mini(
		centroids.size(),
		mini(int(bounds.size() / 2), int(metrics.size() / 2))
	)
	sample_count = mini(sample_count, ROAD_GEOMETRY_TERRAIN_SEAM_SAMPLE_LOG_LIMIT)
	if sample_count <= 0:
		return "[]"
	var parts: Array[String] = []
	for index in range(sample_count):
		var centroid: Vector3 = centroids[index]
		var bounds_min: Vector3 = bounds[index * 2]
		var bounds_max: Vector3 = bounds[index * 2 + 1]
		var y_delta_m: float = metrics[index * 2]
		var slope_ratio: float = metrics[index * 2 + 1]
		var vertices_label := ""
		if vertices.size() >= (index + 1) * 3:
			var v0: Vector3 = vertices[index * 3]
			var v1: Vector3 = vertices[index * 3 + 1]
			var v2: Vector3 = vertices[index * 3 + 2]
			vertices_label = ",verts=[(%.3f,%.3f,%.3f),(%.3f,%.3f,%.3f),(%.3f,%.3f,%.3f)]" % [
				v0.x,
				v0.y,
				v0.z,
				v1.x,
				v1.y,
				v1.z,
				v2.x,
				v2.y,
				v2.z,
			]
		parts.append(
			"{centroid=(%.3f,%.3f,%.3f),bounds=[(%.3f,%.3f,%.3f)..(%.3f,%.3f,%.3f)],y_delta=%.3f,slope=%.3f%s,sources=%s}"
			% [
				centroid.x,
				centroid.y,
				centroid.z,
				bounds_min.x,
				bounds_min.y,
				bounds_min.z,
				bounds_max.x,
				bounds_max.y,
				bounds_max.z,
				y_delta_m,
				slope_ratio,
				vertices_label,
				_road_geometry_cdt_sample_sources_label(
					patch_data,
					"terrain_cdt_retaining_wall",
					index
				),
			]
		)
	return "[" + ", ".join(parts) + "]"

func _road_geometry_terrain_tie_in_kind_label(kind: int) -> String:
	if kind == 1:
		return "retaining_wall"
	return "terrain"

func _road_geometry_cdt_sample_sources_label(
	patch_data: Dictionary,
	prefix: String,
	sample_index: int
) -> String:
	var counts: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_counts", PackedInt32Array())
		as PackedInt32Array
	)
	if sample_index < 0 or sample_index >= counts.size():
		return "[]"
	var source_count: int = maxi(0, counts[sample_index])
	if source_count <= 0:
		return "[]"
	var row_start := 0
	for index in range(sample_index):
		row_start += maxi(0, counts[index])
	var kind_codes: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_kind_codes", PackedInt32Array())
		as PackedInt32Array
	)
	var primary_ids: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_primary_ids", PackedInt32Array())
		as PackedInt32Array
	)
	var node_kind_codes: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_node_kind_codes", PackedInt32Array())
		as PackedInt32Array
	)
	var edge_class_codes: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_edge_class_codes", PackedInt32Array())
		as PackedInt32Array
	)
	var owner_kinds: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_owner_kinds", PackedInt32Array())
		as PackedInt32Array
	)
	var owner_indices: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_owner_indices", PackedInt32Array())
		as PackedInt32Array
	)
	var support_policies: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_support_policies", PackedInt32Array())
		as PackedInt32Array
	)
	var roles: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_roles", PackedInt32Array())
		as PackedInt32Array
	)
	var section_ranges: PackedInt32Array = (
		patch_data.get(prefix + "_sample_source_section_ranges", PackedInt32Array())
		as PackedInt32Array
	)
	var s_ranges: PackedFloat32Array = (
		patch_data.get(prefix + "_sample_source_s_ranges", PackedFloat32Array())
		as PackedFloat32Array
	)
	var parts: Array[String] = []
	for local_index in range(source_count):
		var row: int = row_start + local_index
		parts.append(
			_road_geometry_cdt_source_row_label(
				_road_geometry_int_at(kind_codes, row, -1),
				_road_geometry_int_at(primary_ids, row, -1),
				_road_geometry_int_at(node_kind_codes, row, -1),
				_road_geometry_int_at(edge_class_codes, row, -1),
				_road_geometry_int_at(owner_kinds, row, -1),
				_road_geometry_int_at(owner_indices, row, -1),
				_road_geometry_int_at(support_policies, row, -1),
				_road_geometry_int_at(roles, row, -1),
				_road_geometry_int_pair_at(section_ranges, row, 0, -1),
				_road_geometry_int_pair_at(section_ranges, row, 1, -1),
				_road_geometry_float_pair_at(s_ranges, row, 0, -1.0),
				_road_geometry_float_pair_at(s_ranges, row, 1, -1.0)
			)
		)
	return "[" + ", ".join(parts) + "]"

func _road_geometry_cdt_face_sources_summary_label(patch_data: Dictionary, prefix: String) -> String:
	var counts: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_counts", PackedInt32Array())
		as PackedInt32Array
	)
	if counts.is_empty():
		return "{faces=0,unsourced=0,source_rows=0,span=0,node=0,synthetic=0,samples=[]}"
	var kind_codes: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_kind_codes", PackedInt32Array())
		as PackedInt32Array
	)
	var primary_ids: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_primary_ids", PackedInt32Array())
		as PackedInt32Array
	)
	var node_kind_codes: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_node_kind_codes", PackedInt32Array())
		as PackedInt32Array
	)
	var edge_class_codes: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_edge_class_codes", PackedInt32Array())
		as PackedInt32Array
	)
	var owner_kinds: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_owner_kinds", PackedInt32Array())
		as PackedInt32Array
	)
	var owner_indices: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_owner_indices", PackedInt32Array())
		as PackedInt32Array
	)
	var support_policies: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_support_policies", PackedInt32Array())
		as PackedInt32Array
	)
	var roles: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_roles", PackedInt32Array())
		as PackedInt32Array
	)
	var section_ranges: PackedInt32Array = (
		patch_data.get(prefix + "_face_source_section_ranges", PackedInt32Array())
		as PackedInt32Array
	)
	var s_ranges: PackedFloat32Array = (
		patch_data.get(prefix + "_face_source_s_ranges", PackedFloat32Array())
		as PackedFloat32Array
	)
	var unsourced_count := 0
	var source_rows := 0
	var span_source_rows := 0
	var node_source_rows := 0
	var synthetic_source_rows := 0
	var row_cursor := 0
	var first_unsourced_face := -1
	var samples: Array[String] = []
	for face_index in range(counts.size()):
		var source_count: int = maxi(0, counts[face_index])
		if source_count <= 0:
			unsourced_count += 1
			if first_unsourced_face < 0:
				first_unsourced_face = face_index
			continue
		source_rows += source_count
		var source_parts: Array[String] = []
		for local_index in range(source_count):
			var row: int = row_cursor + local_index
			var source_kind_code: int = _road_geometry_int_at(kind_codes, row, -1)
			if source_kind_code == 0:
				span_source_rows += 1
			elif source_kind_code == 1:
				node_source_rows += 1
			elif source_kind_code == 2:
				synthetic_source_rows += 1
			if samples.size() < ROAD_GEOMETRY_TERRAIN_SEAM_SAMPLE_LOG_LIMIT:
				source_parts.append(
					_road_geometry_cdt_source_row_label(
						source_kind_code,
						_road_geometry_int_at(primary_ids, row, -1),
						_road_geometry_int_at(node_kind_codes, row, -1),
						_road_geometry_int_at(edge_class_codes, row, -1),
						_road_geometry_int_at(owner_kinds, row, -1),
						_road_geometry_int_at(owner_indices, row, -1),
						_road_geometry_int_at(support_policies, row, -1),
						_road_geometry_int_at(roles, row, -1),
						_road_geometry_int_pair_at(section_ranges, row, 0, -1),
						_road_geometry_int_pair_at(section_ranges, row, 1, -1),
						_road_geometry_float_pair_at(s_ranges, row, 0, -1.0),
						_road_geometry_float_pair_at(s_ranges, row, 1, -1.0)
					)
				)
		if not source_parts.is_empty() and samples.size() < ROAD_GEOMETRY_TERRAIN_SEAM_SAMPLE_LOG_LIMIT:
			samples.append(
				"{face=%d,sources=[%s]}" % [face_index, ", ".join(source_parts)]
			)
		row_cursor += source_count
	if samples.is_empty() and first_unsourced_face >= 0:
		samples.append("{face=%d,sources=[]}" % [first_unsourced_face])
	return (
		"{faces=%d,unsourced=%d,source_rows=%d,span=%d,node=%d,synthetic=%d,samples=%s}"
		% [
			counts.size(),
			unsourced_count,
			source_rows,
			span_source_rows,
			node_source_rows,
			synthetic_source_rows,
			"[" + ", ".join(samples) + "]",
		]
	)

func _road_geometry_cdt_source_row_label(
	source_kind_code: int,
	primary_id: int,
	node_kind_code: int,
	edge_class_code: int,
	owner_kind_code: int,
	owner_index: int,
	support_policy_code: int,
	role_code: int,
	section_start: int,
	section_end: int,
	s_start_m: float,
	s_end_m: float
) -> String:
	if source_kind_code == 0:
		return (
			"{kind=span_support,edge=%d,edge_class=%s,support_policy=%s,owner_kind=%s,owner=%d,role=%s,sections=%d..%d,s=%.3f..%.3f}"
			% [
				primary_id,
				_road_geometry_cdt_edge_class_label(edge_class_code),
				_road_geometry_cdt_support_policy_label(support_policy_code),
				_road_geometry_cdt_owner_kind_label(owner_kind_code),
				owner_index,
				_road_geometry_cdt_role_label(role_code),
				section_start,
				section_end,
				s_start_m,
				s_end_m,
			]
		)
	if source_kind_code == 1:
		return (
			"{kind=node_footprint,node=%d,node_kind=%s,owner_kind=%s,owner=%d}"
			% [
				primary_id,
				_road_geometry_cdt_node_kind_label(node_kind_code),
				_road_geometry_cdt_owner_kind_label(owner_kind_code),
				owner_index,
			]
		)
	if source_kind_code == 2:
		return "{kind=synthetic_test,piece=%d}" % [primary_id]
	return (
		"{kind=unknown,primary=%d,owner_kind=%s,owner=%d}"
		% [
			primary_id,
			_road_geometry_cdt_owner_kind_label(owner_kind_code),
			owner_index,
		]
	)

func _road_geometry_cdt_node_kind_label(code: int) -> String:
	if code == 0:
		return "terminal"
	if code == 1:
		return "bend"
	if code == 2:
		return "junction_n"
	return "unknown"

func _road_geometry_cdt_edge_class_label(code: int) -> String:
	if code == 0:
		return "standard"
	if code == 1:
		return "bridge"
	if code == 2:
		return "tunnel"
	return "unknown"

func _road_geometry_cdt_support_policy_label(code: int) -> String:
	if code == 0:
		return "standard_full_grounded_span"
	if code == 1:
		return "bridge_endpoint_abutments"
	if code == 2:
		return "tunnel_visible_portals"
	return "unknown"

func _road_geometry_cdt_owner_kind_label(code: int) -> String:
	if code == 0:
		return "carriageway"
	if code == 1:
		return "curb_or_shoulder"
	if code == 2:
		return "sidewalk"
	if code == 3:
		return "footpath"
	if code == 4:
		return "median"
	if code == 5:
		return "parking"
	if code == 6:
		return "cycle_track"
	if code == 7:
		return "tram_reservation"
	return "unknown"

func _road_geometry_cdt_role_label(code: int) -> String:
	if code == 0:
		return "asphalt"
	if code == 1:
		return "curb_or_shoulder"
	if code == 2:
		return "non_road"
	return "unknown"

func _road_geometry_int_at(values: PackedInt32Array, index: int, fallback: int) -> int:
	if index >= 0 and index < values.size():
		return values[index]
	return fallback

func _road_geometry_int_pair_at(
	values: PackedInt32Array,
	pair_index: int,
	component_index: int,
	fallback: int
) -> int:
	var index: int = pair_index * 2 + component_index
	if index >= 0 and index < values.size():
		return values[index]
	return fallback

func _road_geometry_float_pair_at(
	values: PackedFloat32Array,
	pair_index: int,
	component_index: int,
	fallback: float
) -> float:
	var index: int = pair_index * 2 + component_index
	if index >= 0 and index < values.size():
		return values[index]
	return fallback

func _road_geometry_terrain_seam_quality_samples_label(patch_data: Dictionary) -> String:
	if not patch_data.has("terrain_cdt_seam_quality_sample_edges"):
		return "[]"
	var edges: PackedVector3Array = (
		patch_data["terrain_cdt_seam_quality_sample_edges"] as PackedVector3Array
	)
	var metrics: PackedFloat32Array = (
		patch_data.get("terrain_cdt_seam_quality_sample_metrics", PackedFloat32Array())
		as PackedFloat32Array
	)
	var kinds: PackedInt32Array = (
		patch_data.get("terrain_cdt_seam_quality_sample_kinds", PackedInt32Array())
		as PackedInt32Array
	)
	var sample_count: int = mini(int(edges.size() / 2), int(metrics.size() / 2))
	sample_count = mini(sample_count, kinds.size())
	sample_count = mini(sample_count, ROAD_GEOMETRY_TERRAIN_SEAM_SAMPLE_LOG_LIMIT)
	if sample_count <= 0:
		return "[]"
	var parts: Array[String] = []
	for index in range(sample_count):
		var start: Vector3 = edges[index * 2]
		var end: Vector3 = edges[index * 2 + 1]
		var length_m: float = metrics[index * 2]
		var y_delta_m: float = metrics[index * 2 + 1]
		parts.append(
			"{kind=%s,start=(%.3f,%.3f,%.3f),end=(%.3f,%.3f,%.3f),length=%.3f,y_delta=%.3f,sources=%s}"
			% [
				_road_geometry_cdt_seam_quality_kind_label(kinds[index]),
				start.x,
				start.y,
				start.z,
				end.x,
				end.y,
				end.z,
				length_m,
				y_delta_m,
				_road_geometry_cdt_sample_sources_label(
					patch_data,
					"terrain_cdt_seam_quality",
					index
				),
			]
		)
	return "[" + ", ".join(parts) + "]"

func _road_geometry_cdt_seam_quality_kind_label(kind: int) -> String:
	match kind:
		0:
			return "merged_subbudget"
		1:
			return "retaining_wall_required"
		2:
			return "blocking_degenerate"
	return "unknown"

func _road_geometry_terrain_tie_in_widened_samples_label(patch_data: Dictionary) -> String:
	if not patch_data.has("terrain_cdt_tie_in_widened_sample_points"):
		return "[]"
	var points: PackedVector3Array = (
		patch_data["terrain_cdt_tie_in_widened_sample_points"] as PackedVector3Array
	)
	var metrics: PackedFloat32Array = (
		patch_data.get("terrain_cdt_tie_in_widened_sample_metrics", PackedFloat32Array())
		as PackedFloat32Array
	)
	var sample_count: int = mini(int(points.size() / 2), int(metrics.size() / 4))
	sample_count = mini(sample_count, ROAD_GEOMETRY_TERRAIN_SEAM_SAMPLE_LOG_LIMIT)
	if sample_count <= 0:
		return "[]"
	var parts: Array[String] = []
	for index in range(sample_count):
		var source: Vector3 = points[index * 2]
		var seam: Vector3 = points[index * 2 + 1]
		var distance_m: float = metrics[index * 4]
		var required_distance_m: float = metrics[index * 4 + 1]
		var y_delta_m: float = metrics[index * 4 + 2]
		var slope_ratio: float = metrics[index * 4 + 3]
		parts.append(
			"{source=(%.3f,%.3f,%.3f),seam=(%.3f,%.3f,%.3f),distance=%.3f,required=%.3f,y_delta=%.3f,slope=%.3f,sources=%s}"
			% [
				source.x,
				source.y,
				source.z,
				seam.x,
				seam.y,
				seam.z,
				distance_m,
				required_distance_m,
				y_delta_m,
				slope_ratio,
				_road_geometry_cdt_sample_sources_label(
					patch_data,
					"terrain_cdt_tie_in_widened",
					index
				),
			]
		)
	return "[" + ", ".join(parts) + "]"

func _road_geometry_terrain_invalid_constraint_samples_label(patch_data: Dictionary) -> String:
	if not patch_data.has("terrain_cdt_invalid_constraint_sample_edges"):
		return "[]"
	var edges: PackedVector3Array = (
		patch_data["terrain_cdt_invalid_constraint_sample_edges"] as PackedVector3Array
	)
	var metadata: PackedInt32Array = (
		patch_data.get("terrain_cdt_invalid_constraint_sample_metadata", PackedInt32Array())
		as PackedInt32Array
	)
	var sample_count: int = int(edges.size() / 2)
	sample_count = mini(sample_count, ROAD_GEOMETRY_TERRAIN_SEAM_SAMPLE_LOG_LIMIT)
	if sample_count <= 0:
		return "[]"
	var parts: Array[String] = []
	for index in range(sample_count):
		var start: Vector3 = edges[index * 2]
		var end: Vector3 = edges[index * 2 + 1]
		var road_owned := false
		var stable_piece_id := -1
		var local_loop_index := -1
		var local_edge_index := -1
		if metadata.size() >= (index + 1) * 4:
			road_owned = metadata[index * 4] != 0
			stable_piece_id = metadata[index * 4 + 1]
			local_loop_index = metadata[index * 4 + 2]
			local_edge_index = metadata[index * 4 + 3]
		var source_label := _road_geometry_cdt_sample_sources_label(
			patch_data,
			"terrain_cdt_invalid_constraint",
			index
		)
		if source_label == "[]":
			source_label = "none"
		parts.append(
			"{road=%s,piece=%d,loop=%d,edge=%d,start=(%.3f,%.3f,%.3f),end=(%.3f,%.3f,%.3f),source=%s}"
			% [
				str(road_owned),
				stable_piece_id,
				local_loop_index,
				local_edge_index,
				start.x,
				start.y,
				start.z,
				end.x,
				end.y,
				end.z,
				source_label,
			]
		)
	return "[" + ", ".join(parts) + "]"

func _road_geometry_baked_vertex_count(patch_data: Dictionary) -> int:
	if not _patch_has_baked_terrain_mesh(patch_data):
		return 0
	var vertices: PackedVector3Array = patch_data["terrain_mesh_vertices"] as PackedVector3Array
	return vertices.size()

func _road_geometry_retaining_wall_baked_vertex_count(patch_data: Dictionary) -> int:
	if not _patch_has_retaining_wall_mesh(patch_data):
		return 0
	var vertices: PackedVector3Array = patch_data["terrain_retaining_wall_mesh_vertices"] as PackedVector3Array
	return vertices.size()

func _road_geometry_baked_mesh_stats_label(patch_data: Dictionary) -> String:
	if not _patch_has_baked_terrain_mesh(patch_data):
		return "none"
	var vertices: PackedVector3Array = patch_data["terrain_mesh_vertices"] as PackedVector3Array
	var min_vertex: Vector3 = vertices[0]
	var max_vertex: Vector3 = vertices[0]
	for vertex_variant in vertices:
		var vertex: Vector3 = vertex_variant
		min_vertex.x = minf(min_vertex.x, vertex.x)
		min_vertex.y = minf(min_vertex.y, vertex.y)
		min_vertex.z = minf(min_vertex.z, vertex.z)
		max_vertex.x = maxf(max_vertex.x, vertex.x)
		max_vertex.y = maxf(max_vertex.y, vertex.y)
		max_vertex.z = maxf(max_vertex.z, vertex.z)

	var uv_label := "none"
	var uvs: PackedVector2Array = (
		patch_data.get("terrain_mesh_uvs", PackedVector2Array()) as PackedVector2Array
	)
	if uvs.size() == vertices.size():
		var min_uv: Vector2 = uvs[0]
		var max_uv: Vector2 = uvs[0]
		for uv_variant in uvs:
			var uv: Vector2 = uv_variant
			min_uv.x = minf(min_uv.x, uv.x)
			min_uv.y = minf(min_uv.y, uv.y)
			max_uv.x = maxf(max_uv.x, uv.x)
			max_uv.y = maxf(max_uv.y, uv.y)
		uv_label = "[(%.3f,%.3f)..(%.3f,%.3f)]" % [
			min_uv.x,
			min_uv.y,
			max_uv.x,
			max_uv.y,
		]

	var normal_label := "none"
	var normals: PackedVector3Array = (
		patch_data.get("terrain_mesh_normals", PackedVector3Array()) as PackedVector3Array
	)
	if normals.size() == vertices.size():
		var min_normal_y: float = normals[0].y
		var max_normal_y: float = normals[0].y
		var min_normal_length := normals[0].length()
		var max_normal_length := min_normal_length
		var normal_y_sum := 0.0
		for normal_variant in normals:
			var normal: Vector3 = normal_variant
			var normal_length := normal.length()
			min_normal_y = minf(min_normal_y, normal.y)
			max_normal_y = maxf(max_normal_y, normal.y)
			min_normal_length = minf(min_normal_length, normal_length)
			max_normal_length = maxf(max_normal_length, normal_length)
			normal_y_sum += normal.y
		normal_label = "y=[%.3f..%.3f],avg_y=%.3f,len=[%.3f..%.3f]" % [
			min_normal_y,
			max_normal_y,
			normal_y_sum / float(maxi(1, normals.size())),
			min_normal_length,
			max_normal_length,
		]

	return "{local_bounds=[(%.3f,%.3f,%.3f)..(%.3f,%.3f,%.3f)],uv=%s,normal=%s}" % [
		min_vertex.x,
		min_vertex.y,
		min_vertex.z,
		max_vertex.x,
		max_vertex.y,
		max_vertex.z,
		uv_label,
		normal_label,
	]

func _road_geometry_polygon_area(points: PackedVector2Array) -> float:
	if points.size() < 3:
		return 0.0
	var area: float = 0.0
	for index in range(points.size()):
		var a: Vector2 = points[index]
		var b: Vector2 = points[(index + 1) % points.size()]
		area += a.x * b.y - b.x * a.y
	return area * 0.5

func _road_geometry_bounds_label(stats: Dictionary) -> String:
	if not bool(stats.get("has_bounds", false)):
		return "none"
	return "[(%.3f,%.3f)..(%.3f,%.3f)]" % [
		float(stats.get("min_x", 0.0)),
		float(stats.get("min_z", 0.0)),
		float(stats.get("max_x", 0.0)),
		float(stats.get("max_z", 0.0)),
	]

func _road_geometry_mesh_label(mesh: Mesh) -> String:
	if mesh == null:
		return "null"
	if mesh is ArrayMesh:
		var array_mesh: ArrayMesh = mesh as ArrayMesh
		var vertex_count: int = 0
		for surface_index in range(array_mesh.get_surface_count()):
			var arrays: Array = array_mesh.surface_get_arrays(surface_index)
			if arrays.size() > Mesh.ARRAY_VERTEX:
				var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX] as PackedVector3Array
				vertex_count += vertices.size()
		return "ArrayMesh surfaces=%d vertices=%d" % [
			array_mesh.get_surface_count(),
			vertex_count,
		]
	return mesh.get_class()

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
		if (
			entry == "terrain"
			or entry == "terrain-verbose"
			or entry == "terrain-full"
			or entry == "terrain-lod1"
			or entry == "terrain-full-lod1"
		):
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
		if entry == "terrain-full" or entry == "terrain-full-lod1":
			return true
	return false

func _terrain_debug_force_lod1() -> bool:
	var explicit_value := OS.get_environment("METRUM_DEBUG_TERRAIN_FORCE_LOD1").strip_edges()
	if explicit_value == "1":
		return true
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	for entry_variant in filter.split(","):
		var entry := String(entry_variant).strip_edges()
		if entry == "terrain-lod1" or entry == "terrain-full-lod1":
			return true
	return false

func _terrain_visual_debug_mode_from_env() -> int:
	var value := OS.get_environment("METRUM_DEBUG_TERRAIN_VISUAL").strip_edges().to_lower()
	if value.is_empty() or value == "0" or value == "off" or value == "false":
		return 0
	if value.is_valid_int():
		return clampi(value.to_int(), 0, 10)
	match value:
		"patch", "patches":
			return 1
		"lod", "lods":
			return 2
		"height":
			return 3
		"relief":
			return 4
		"shore", "shoreline":
			return 5
		"water", "depth", "water-depth":
			return 6
		"water-lod":
			return 7
		"water-patch":
			return 8
		"water-material", "water-mat", "material-water":
			return 9
		"lighting", "light", "sun":
			return 10
		_:
			return 0

func _terrain_grass_visual_debug_mode_from_env() -> int:
	var value := OS.get_environment("METRUM_DEBUG_TERRAIN_GRASS").strip_edges().to_lower()
	if value.is_empty() or value == "0" or value == "off" or value == "false":
		return 0
	if value.is_valid_int():
		return clampi(value.to_int(), 0, 10)
	match value:
		"raw", "albedo":
			return 1
		"macro":
			return 2
		"mid":
			return 3
		"micro":
			return 4
		"fade", "fades", "visibility":
			return 5
		"material", "composite":
			return 6
		"height":
			return 7
		"mask", "grass-mask":
			return 8
		"luminance", "luma", "brightness":
			return 9
		"footprint", "footprints":
			return 10
		_:
			return 0

func _road_debug_is_enabled() -> bool:
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value != "1":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	if filter.is_empty():
		return true
	for entry_variant in filter.split(","):
		var entry := String(entry_variant).strip_edges()
		if entry == "road":
			return true
	return false

func _road_geometry_debug_is_enabled() -> bool:
	if OS.get_environment("METRUM_DEBUG_ROAD_GEOMETRY_DUMP").strip_edges() == "1":
		return true
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value != "1":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	for entry_variant in filter.split(","):
		var entry := String(entry_variant).strip_edges()
		if entry == "road":
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
		"fps=%d cam=%s resident=%d/%d desired=%d desired_bounds=%s resident_bounds=%s cull_far=%.1f residency_changes=%d creates=%d removes=%d uploads=%d dirty_batches=%d dirty_patches=%d lods=%s avg_ms=%.3f max_ms=%.3f residency_ms=%.3f upload_ms=%.3f border_ms=%.3f water_sync_ms=%.3f force_full_world=%s force_lod1=%s visual=%d"
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
			str(_terrain_force_lod1),
			_terrain_visual_debug_mode,
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
