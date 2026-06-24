## Water patch renderer — uploads chunk-local visible water patches and world-edge water curtains.
##
## Rust methods called: get_water_patch(), get_water_patch_debug(),
##   get_water_patch_authored_fill_debug(), get_dirty_water_patches(), get_water_border_depths(),
##   request_water_patch_meshes(), poll_ready_water_patch_meshes(),
##   clear_water_patch_mesh_cache(), is_water_dirty(), clear_water_dirty()
extends Node3D

const WATER_SHADER := preload("res://assets/materials/water.gdshader")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const HEIGHT_SCALE := 20.0
const SHORE_SOFTNESS_M := 0.26
const SHORE_FOAM_BAND_M := 0.18
const SHALLOW_WATER_COLOR := Color(0.31, 0.58, 0.64, 0.46)
const DEEP_WATER_COLOR := Color(0.03, 0.16, 0.31, 0.84)
const FOAM_COLOR := Color(0.76, 0.91, 0.96, 0.82)
const SKY_REFLECTION_COLOR := Color(0.62, 0.82, 0.95, 1.0)
const WATER_FRESNEL_STRENGTH := 0.42
const WATER_FRESNEL_POWER := 3.2
const WATER_WAVE_COLOR_STRENGTH := 0.052
const WATER_WAVE_ROUGHNESS_STRENGTH := 0.024
const WATER_WAVE_NORMAL_STRENGTH := 0.42
const WATER_REFRACTION_STRENGTH := 0.010
const WATER_REFRACTION_MIX := 0.13
const WATER_DISPLAY_SURFACE_SMOOTHING := 0.94
const WATER_DISPLAY_SURFACE_BLEND_RADIUS_TEXELS := 1.0
const WATER_MIN_VISIBLE_DEPTH_M := 0.001
const WATER_BORDER_MIN_DEPTH_M := 0.02
const WATER_PATCH_EXTRA_CULL_MARGIN_M := 4096.0
const WATER_DEBUG_LOG_INTERVAL_S := 0.5
const WATER_PATCH_MUTATION_MAX_PER_FRAME := 256
const WATER_PATCH_MUTATION_BUDGET_MS := 1.5
const WATER_PATCH_PREWARM_MAX_PER_FRAME := 4
const WATER_PATCH_PREWARM_BUDGET_MS := 0.75
const WATER_PATCH_PREWARM_HALO_PATCHES := 1
const WATER_PATCH_HEIGHT_REBIND_BUDGET_PER_FRAME := 128
const WATER_TERRAIN_BINDING_BUDGET_PER_FRAME := 128
const WATER_PATCH_MESH_REQUEST_BUDGET_PER_FRAME := 8
const WATER_PATCH_MESH_BUSY_REQUEST_BUDGET_PER_FRAME := 4
const WATER_PATCH_MESH_BACKLOG_REQUEST_THRESHOLD := 96
const WATER_PATCH_MESH_BACKLOG_REQUEST_BUDGET_PER_FRAME := 16
const WATER_PATCH_MESH_SUBMIT_BATCH_SIZE := 8
const WATER_PATCH_MESH_SUBMIT_BUDGET_MS := 1.0
const WATER_PATCH_MESH_REFRESH_SOFT_BUDGET_MS := 2.25
const WATER_PATCH_MESH_PENDING_SOFT_LIMIT := 48
const WATER_PATCH_MESH_PENDING_HARD_LIMIT := 96
const WATER_PATCH_MESH_POLL_BUDGET_PER_FRAME := 2
const WATER_PATCH_MESH_POLL_BACKLOG_BUDGET_PER_FRAME := 8
const WATER_PATCH_MESH_POLL_HEADROOM_BUDGET_PER_FRAME := 12
const WATER_PATCH_MESH_POLL_BACKLOG_THRESHOLD := 16
const WATER_PATCH_MESH_READY_BACKLOG_BOOST_THRESHOLD := 32
const WATER_PATCH_MESH_APPLY_QUEUE_SOFT_LIMIT := 8
const WATER_PATCH_MESH_APPLY_QUEUE_HARD_LIMIT := 16
const WATER_PATCH_MESH_APPLY_MAX_PER_FRAME := 2
const WATER_PATCH_MESH_APPLY_HEADROOM_MAX_PER_FRAME := 3
const WATER_PATCH_MESH_APPLY_BUDGET_MS := 1.5
const WATER_PATCH_MESH_APPLY_HEADROOM_BUDGET_MS := 2.0
const WATER_PATCH_MESH_APPLY_HEADROOM_FRAME_MS := 5.0
const WATER_PATCH_MESH_APPLY_HEADROOM_PREVIOUS_APPLY_MS := 1.4
const WATER_PATCH_LOD_START_HEADROOM_MS := 1.25
const WATER_PATCH_PREWARM_START_HEADROOM_MS := 1.75
const WATER_PATCH_MESH_LOD_REFRESH_INTERVAL_S := 0.20
const WATER_PATCH_MESH_LOD_REFRESH_BUDGET_MS := 0.75
const WATER_PATCH_MESH_LOD_REFRESH_CAMERA_MOVE_M := 96.0
const WATER_PATCH_MESH_LOD_REFRESH_MAX_CHECKS_PER_FRAME := 48
const WATER_PATCH_MESH_LOD_REFRESH_MAX_CHANGES_PER_FRAME := 8
const WATER_PATCH_MESH_LOD_NEAR_DISTANCE_M := 2000.0
const WATER_PATCH_MESH_LOD_MID_DISTANCE_M := 5000.0
const WATER_PATCH_MESH_LOD_FAR_DISTANCE_M := 12000.0
const ROAD_CLIP_LOOP_ROLE_OUTER := 0
const ROAD_CLIP_LOOP_ROLE_HOLE := 1

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var patches: Dictionary = {}
var resident_patch_lookup: Dictionary = {}
var patch_prewarm_queue: Array[Vector2i] = []
var height_texture_rebind_queue: Array[Vector2i] = []
var height_texture_rebind_lookup: Dictionary = {}
var terrain_patch_binding_queue: Array[Vector2i] = []
var terrain_patch_binding_lookup: Dictionary = {}
var mesh_refresh_queue: Array[Vector2i] = []
var mesh_refresh_requested_lod: Dictionary = {}
var mesh_pending_lod: Dictionary = {}
var mesh_apply_queue: Array[Dictionary] = []
var patch_lod_refresh_queue: Array[Vector2i] = []
var patch_lod_refresh_lookup: Dictionary = {}
var terrain_world_size: Vector2 = Vector2.ZERO
var terrain_patch_cols: int = 0
var terrain_patch_rows: int = 0
var terrain_patch_span_m: float = 1.0
var fallback_height_texture: ImageTexture
var water_border_instance: MeshInstance3D
var water_border_material: ShaderMaterial
var terrain_border_revision: int = -1
var _terrain_debug_enabled: bool = false
var _terrain_debug_verbose: bool = false
var _terrain_force_lod1: bool = false
var _water_visual_debug_mode: int = 0
var _water_mesh_lod_refresh_elapsed_s: float = 0.0
var _water_lod_refresh_camera_valid: bool = false
var _water_lod_refresh_last_camera_position: Vector3 = Vector3.ZERO
var _water_lod_last_processed_count: int = 0
var _water_lod_last_changed_count: int = 0
var _water_lod_last_queued_count: int = 0
var _water_lod_last_queue_count: int = 0
var _water_lod_last_replaced_count: int = 0
var _water_lod_last_skipped_count: int = 0
var _water_lod_last_deferred_count: int = 0
var _water_prewarm_last_deferred_count: int = 0
var _water_residency_pending_mutations: bool = false
var _terrain_resident_patch_revision_seen: int = -1
var _water_mesh_ready_backlog_estimate: int = 0
var _water_mesh_last_frame_elapsed_ms: float = 0.0
var _water_mesh_last_apply_elapsed_ms: float = 0.0
var _water_residency_last_add_count: int = 0
var _water_residency_last_remove_count: int = 0
var _water_residency_last_add_pending_count: int = 0
var _water_residency_last_remove_pending_count: int = 0
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
	var patch_layout: Dictionary = simulation_node.get_terrain_patch_layout()
	terrain_world_size = simulation_node.get_terrain_world_size()
	terrain_patch_cols = int(patch_layout.get("patch_cols", 0))
	terrain_patch_rows = int(patch_layout.get("patch_rows", 0))
	var patch_interval_cells: int = max(1, int(patch_layout.get("patch_interval_cells", 1)))
	var terrain_cell_m: float = float(patch_layout.get("terrain_cell_m", 1.0))
	terrain_patch_span_m = terrain_cell_m * float(patch_interval_cells)
	_terrain_debug_enabled = _terrain_debug_is_enabled()
	_terrain_debug_verbose = _terrain_debug_is_verbose()
	_terrain_force_lod1 = _terrain_debug_force_lod1()
	_water_visual_debug_mode = _terrain_visual_debug_mode_from_env()
	_water_mesh_lod_refresh_elapsed_s = 0.0
	_water_lod_refresh_camera_valid = false
	_record_lod_perf_counters(0, 0, 0, 0)
	_water_lod_last_deferred_count = 0
	_water_prewarm_last_deferred_count = 0
	_water_mesh_ready_backlog_estimate = 0
	_water_mesh_last_frame_elapsed_ms = 0.0
	_water_mesh_last_apply_elapsed_ms = 0.0
	_reset_water_debug_counters()
	_clear_patches()
	_ensure_fallback_height_texture()
	_ensure_water_border_visual()
	_sync_patch_residency(true)
	_process_height_texture_rebinds(WATER_PATCH_HEIGHT_REBIND_BUDGET_PER_FRAME)
	_rebuild_water_border()
	_process_terrain_patch_binding_queue(WATER_TERRAIN_BINDING_BUDGET_PER_FRAME)
	_rebuild_patch_prewarm_queue()
	if _terrain_debug_enabled:
		_water_debug_log(
			"renderer ready force_lod1=%s visual=%d"
			% [str(_terrain_force_lod1), _water_visual_debug_mode]
		)

func _process(delta: float) -> void:
	var frame_start_us := Time.get_ticks_usec()
	var perf_enabled := PerfDebug.is_enabled()
	var patch_sync_start_us := frame_start_us
	var residency_changed := _sync_patch_residency()
	var patch_sync_elapsed_ms := float(Time.get_ticks_usec() - patch_sync_start_us) / 1000.0
	var height_rebind_elapsed_ms := 0.0
	var upload_elapsed_ms := 0.0
	var border_elapsed_ms := 0.0
	var terrain_binding_elapsed_ms := 0.0
	var lod_elapsed_ms := 0.0
	var mesh_refresh_elapsed_ms := 0.0
	var mesh_refresh_perf_stats: Dictionary = {}
	var prewarm_elapsed_ms := 0.0
	_water_lod_last_deferred_count = 0
	_water_prewarm_last_deferred_count = 0
	var height_rebind_start_us := Time.get_ticks_usec()
	_process_height_texture_rebinds(WATER_PATCH_HEIGHT_REBIND_BUDGET_PER_FRAME)
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

	var terrain_binding_start_us := Time.get_ticks_usec()
	_process_terrain_patch_binding_queue(WATER_TERRAIN_BINDING_BUDGET_PER_FRAME)
	terrain_binding_elapsed_ms = float(Time.get_ticks_usec() - terrain_binding_start_us) / 1000.0

	if _water_frame_headroom_available(frame_start_us, WATER_PATCH_LOD_START_HEADROOM_MS):
		if perf_enabled:
			var lod_start_us := Time.get_ticks_usec()
			_refresh_patch_mesh_lods(delta)
			lod_elapsed_ms = float(Time.get_ticks_usec() - lod_start_us) / 1000.0
		else:
			_refresh_patch_mesh_lods(delta)
	else:
		_defer_patch_mesh_lods(delta)

	var mesh_refresh_start_us := Time.get_ticks_usec()
	mesh_refresh_perf_stats = _process_mesh_refresh_queue(
		WATER_PATCH_MESH_REQUEST_BUDGET_PER_FRAME,
		perf_enabled
	)
	mesh_refresh_elapsed_ms = float(Time.get_ticks_usec() - mesh_refresh_start_us) / 1000.0

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
	if (
		not simulation_node.is_water_dirty()
		and not residency_changed
		and not _water_residency_pending_mutations
	):
		if (
			patch_lod_refresh_queue.is_empty()
			and _water_frame_headroom_available(frame_start_us, WATER_PATCH_PREWARM_START_HEADROOM_MS)
		):
			if perf_enabled:
				var prewarm_start_us := Time.get_ticks_usec()
				_prewarm_patch_cache()
				prewarm_elapsed_ms = float(Time.get_ticks_usec() - prewarm_start_us) / 1000.0
			else:
				_prewarm_patch_cache()
		else:
			_water_prewarm_last_deferred_count = 1 if not patch_prewarm_queue.is_empty() else 0

	if perf_enabled:
		var water_frame_elapsed_ms := float(Time.get_ticks_usec() - frame_start_us) / 1000.0
		var perf_details := {
			"residency": patch_sync_elapsed_ms,
			"residency_add_count": float(_water_residency_last_add_count),
			"residency_remove_count": float(_water_residency_last_remove_count),
			"residency_add_pending_count": float(_water_residency_last_add_pending_count),
			"residency_remove_pending_count": float(_water_residency_last_remove_pending_count),
			"upload": upload_elapsed_ms,
			"border": border_elapsed_ms,
			"height_rebind": height_rebind_elapsed_ms,
			"terrain_binding": terrain_binding_elapsed_ms,
			"lod": lod_elapsed_ms,
			"lod_processed_count": float(_water_lod_last_processed_count),
			"lod_changed_count": float(_water_lod_last_changed_count),
			"lod_queued_count": float(_water_lod_last_queued_count),
			"lod_queue_count": float(_water_lod_last_queue_count),
			"lod_replaced_count": float(_water_lod_last_replaced_count),
			"lod_skipped_count": float(_water_lod_last_skipped_count),
			"lod_deferred_count": float(_water_lod_last_deferred_count),
			"mesh_refresh": mesh_refresh_elapsed_ms,
			"prewarm": prewarm_elapsed_ms,
			"prewarm_deferred_count": float(_water_prewarm_last_deferred_count),
		}
		for key_variant in mesh_refresh_perf_stats.keys():
			var perf_key := str(key_variant)
			perf_details[perf_key] = mesh_refresh_perf_stats[key_variant]
		PerfDebug.record(
			"water",
			water_frame_elapsed_ms,
			perf_details
		)
		_water_mesh_last_frame_elapsed_ms = water_frame_elapsed_ms
	else:
		_water_mesh_last_frame_elapsed_ms = float(Time.get_ticks_usec() - frame_start_us) / 1000.0

func update_water_visuals() -> void:
	var dirty_keys := _dirty_patch_keys(simulation_node.get_dirty_water_patches())
	if dirty_keys.is_empty():
		for key in get_resident_patch_keys():
			_upload_patch(key)
	else:
		for key in dirty_keys:
			if patches.has(key):
				_upload_patch(key)
				_queue_terrain_patch_binding(key)
	_rebuild_water_border()

func get_resident_patch_keys() -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		keys.append(key)
	return keys

func get_water_patch_texture_binding(key: Vector2i) -> Dictionary:
	if not patches.has(key) or not resident_patch_lookup.has(key):
		return {}
	var patch: Dictionary = patches[key]
	var texture: Texture2D = patch.get("depth_texture", null) as Texture2D
	if texture == null:
		return {}
	return {
		"texture": texture,
		"texture_width": int(patch.get("texture_width", 0)),
		"texture_height": int(patch.get("texture_height", 0)),
		"inner_offset_x": int(patch.get("inner_offset_x", 0)),
		"inner_offset_z": int(patch.get("inner_offset_z", 0)),
		"sample_width": int(patch.get("sample_width", 0)),
		"sample_height": int(patch.get("sample_height", 0)),
		"depth_nonzero_count": int(patch.get("depth_nonzero_count", 0)),
		"world_origin_x": float(patch.get("world_origin_x", 0.0)),
		"world_origin_z": float(patch.get("world_origin_z", 0.0)),
		"world_size_x": float(patch.get("world_size_x", 0.0)),
		"world_size_z": float(patch.get("world_size_z", 0.0)),
	}

func _sync_patch_residency(force_full_sync: bool = false) -> bool:
	if terrain_node == null or not terrain_node.has_method("get_resident_patch_keys"):
		_record_residency_perf_counters(0, 0, 0, 0)
		return false

	var terrain_revision := _current_terrain_resident_patch_revision()
	if (
		not force_full_sync
		and not _water_residency_pending_mutations
		and terrain_revision >= 0
		and terrain_revision == _terrain_resident_patch_revision_seen
	):
		_record_residency_perf_counters(0, 0, 0, 0)
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
		_water_residency_pending_mutations = false
		_terrain_resident_patch_revision_seen = terrain_revision
		_record_residency_perf_counters(0, 0, 0, 0)
		return false

	_sort_patch_keys_by_camera_priority(keys_to_add)
	_sort_patch_keys_by_camera_priority(keys_to_remove)
	keys_to_remove.reverse()

	var changed := false
	var mutation_limit := WATER_PATCH_MUTATION_MAX_PER_FRAME
	if force_full_sync:
		mutation_limit = keys_to_add.size() + keys_to_remove.size()
	var budget_start_us: int = Time.get_ticks_usec()
	var processed_mutations := 0
	var processed_adds := 0
	var processed_removes := 0
	for key in keys_to_remove:
		if processed_mutations >= mutation_limit:
			break
		if _time_budget_exhausted(budget_start_us, WATER_PATCH_MUTATION_BUDGET_MS, processed_mutations):
			break
		_deactivate_patch(key)
		processed_mutations += 1
		processed_removes += 1
		changed = true

	for key in keys_to_add:
		if processed_mutations >= mutation_limit:
			break
		if _time_budget_exhausted(budget_start_us, WATER_PATCH_MUTATION_BUDGET_MS, processed_mutations):
			break
		_activate_patch(key)
		processed_mutations += 1
		processed_adds += 1
		changed = true

	_water_residency_pending_mutations = (
		processed_adds < keys_to_add.size()
		or processed_removes < keys_to_remove.size()
	)
	_terrain_resident_patch_revision_seen = terrain_revision
	_record_residency_perf_counters(
		processed_adds,
		processed_removes,
		max(0, keys_to_add.size() - processed_adds),
		max(0, keys_to_remove.size() - processed_removes)
	)
	if changed:
		_water_debug_residency_changes += 1
		_rebuild_patch_prewarm_queue()
		if _terrain_debug_verbose:
			_water_debug_log(
				"water residency changed resident=%d desired=%d add_pending=%d remove_pending=%d"
				% [
					resident_patch_lookup.size(),
					desired_keys.size(),
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
	_water_residency_last_add_count = add_count
	_water_residency_last_remove_count = remove_count
	_water_residency_last_add_pending_count = add_pending_count
	_water_residency_last_remove_pending_count = remove_pending_count

func _current_terrain_resident_patch_revision() -> int:
	if terrain_node != null and terrain_node.has_method("get_resident_patch_revision"):
		return int(terrain_node.get_resident_patch_revision())
	return -1

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

	var patch_center_x := world_origin_x + world_size_x * 0.5
	var patch_center_z := world_origin_z + world_size_z * 0.5
	var initial_lod_step := _mesh_lod_step_for_patch_center(patch_center_x, patch_center_z)
	var mesh_stats: Dictionary = _empty_water_mesh_stats(initial_lod_step)
	var depth_signature: int = int(patch_data.get("depth_signature", 0))

	var patch_node: MeshInstance3D = MeshInstance3D.new()
	patch_node.name = "WaterPatch_%d_%d" % [key.x, key.y]
	SceneLightingConfig.apply_shadow_policy(
		patch_node,
		SceneLightingConfig.SHADOW_RECEIVER_ONLY,
		"water"
	)
	patch_node.extra_cull_margin = WATER_PATCH_EXTRA_CULL_MARGIN_M
	patch_node.mesh = ArrayMesh.new()
	patch_node.visible = false
	patch_node.position = Vector3(
		world_origin_x + world_size_x * 0.5,
		0.0,
		world_origin_z + world_size_z * 0.5
	)

	var material: ShaderMaterial = ShaderMaterial.new()
	material.shader = WATER_SHADER
	var height_texture := _terrain_height_texture(key)
	material.set_shader_parameter("heightmap", height_texture)
	material.set_shader_parameter("watermap", depth_texture)
	material.set_shader_parameter("height_scale", HEIGHT_SCALE)
	material.set_shader_parameter("shore_softness_m", SHORE_SOFTNESS_M)
	material.set_shader_parameter("shore_foam_band_m", SHORE_FOAM_BAND_M)
	material.set_shader_parameter("shallow_water_color", SHALLOW_WATER_COLOR)
	material.set_shader_parameter("deep_water_color", DEEP_WATER_COLOR)
	material.set_shader_parameter("foam_color", FOAM_COLOR)
	material.set_shader_parameter("sky_reflection_color", SKY_REFLECTION_COLOR)
	material.set_shader_parameter("water_fresnel_strength", WATER_FRESNEL_STRENGTH)
	material.set_shader_parameter("water_fresnel_power", WATER_FRESNEL_POWER)
	material.set_shader_parameter("water_wave_color_strength", WATER_WAVE_COLOR_STRENGTH)
	material.set_shader_parameter("water_wave_roughness_strength", WATER_WAVE_ROUGHNESS_STRENGTH)
	material.set_shader_parameter("water_wave_normal_strength", WATER_WAVE_NORMAL_STRENGTH)
	material.set_shader_parameter("water_refraction_strength", WATER_REFRACTION_STRENGTH)
	material.set_shader_parameter("water_refraction_mix", WATER_REFRACTION_MIX)
	material.set_shader_parameter("scene_sun_direction", SceneLightingConfig.sun_direction())
	material.set_shader_parameter("scene_sun_color", SceneLightingConfig.sun_color())
	material.set_shader_parameter("scene_sky_color", SceneLightingConfig.sky_color())
	material.set_shader_parameter("scene_ambient_strength", SceneLightingConfig.ambient_strength())
	material.set_shader_parameter("scene_shadow_max_distance_m", SceneLightingConfig.SHADOW_MAX_DISTANCE_M)
	material.set_shader_parameter(
		"scene_shadow_split_distances_m",
		SceneLightingConfig.shadow_split_distances()
	)
	material.set_shader_parameter("water_surface_smoothing", WATER_DISPLAY_SURFACE_SMOOTHING)
	material.set_shader_parameter("water_surface_blend_radius_texels", WATER_DISPLAY_SURFACE_BLEND_RADIUS_TEXELS)
	material.set_shader_parameter("water_visual_debug_mode", _water_visual_debug_mode)
	material.set_shader_parameter("water_debug_patch_key", Vector2(key.x, key.y))
	material.set_shader_parameter("water_debug_lod_step", float(initial_lod_step))
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
		"height_texture": height_texture,
		"sample_width": sample_width,
		"sample_height": sample_height,
		"texture_width": texture_width,
		"texture_height": texture_height,
		"inner_offset_x": int(inner_offset_x),
		"inner_offset_z": int(inner_offset_z),
		"world_size_x": world_size_x,
		"world_size_z": world_size_z,
		"world_origin_x": world_origin_x,
		"world_origin_z": world_origin_z,
		"lod_step": initial_lod_step,
		"depth_nonzero_count": _patch_visible_depth_count(patch_data),
		"depth_signature": depth_signature,
		"mesh_stats": mesh_stats,
		"road_clip_signature": _patch_road_clip_signature(patch_data),
		"last_patch_data": patch_data,
	}

func refresh_road_clipped_patches(flat_pairs: PackedInt32Array) -> void:
	var dirty_keys: Array[Vector2i] = _dirty_patch_keys(flat_pairs)
	if not dirty_keys.is_empty() and simulation_node.has_method("clear_water_patch_mesh_cache"):
		simulation_node.clear_water_patch_mesh_cache(flat_pairs)
	for key in dirty_keys:
		if patches.has(key):
			_upload_patch(key, true)

func _upload_patch(key: Vector2i, road_clip_only: bool = false) -> void:
	if not patches.has(key):
		return
	var total_start_us := Time.get_ticks_usec()
	var patch: Dictionary = patches[key]
	var fetch_start_us := total_start_us
	var fetch_elapsed_ms := 0.0
	if road_clip_only and simulation_node.has_method("get_water_patch_road_clip"):
		var clip_data: Dictionary = simulation_node.get_water_patch_road_clip(
			key.x,
			key.y,
			float(patch["world_origin_x"]),
			float(patch["world_origin_z"]),
			float(patch["world_origin_x"]) + float(patch["world_size_x"]),
			float(patch["world_origin_z"]) + float(patch["world_size_z"])
		)
		fetch_elapsed_ms = float(Time.get_ticks_usec() - fetch_start_us) / 1000.0
		if clip_data.is_empty():
			return
		var clip_signature := _patch_road_clip_signature(clip_data)
		var clip_signature_changed := int(patch.get("road_clip_signature", clip_signature - 1)) != clip_signature
		var cached_depth_nonzero_count := int(patch.get("depth_nonzero_count", 0))
		if not clip_signature_changed or cached_depth_nonzero_count <= 0:
			patch["road_clip_signature"] = clip_signature
			if _terrain_debug_enabled and fetch_elapsed_ms >= 2.0:
				_water_debug_log(
					"water_upload key=(%d,%d) road_clip_only=true depth_nonzero=%d clip_loops=%d clip_points=%d signature_changed=%s texture_ms=0.000 mesh_ms=0.000 fetch_ms=%.3f total_ms=%.3f"
					% [
						key.x,
						key.y,
						cached_depth_nonzero_count,
						int(clip_data.get("road_clip_loop_count", 0)),
						int(clip_data.get("road_clip_point_count", 0)),
						str(clip_signature_changed),
						fetch_elapsed_ms,
						float(Time.get_ticks_usec() - total_start_us) / 1000.0,
					]
				)
			return
		fetch_start_us = Time.get_ticks_usec()
	var patch_data: Dictionary = simulation_node.get_water_patch(key.x, key.y)
	fetch_elapsed_ms += float(Time.get_ticks_usec() - fetch_start_us) / 1000.0
	if patch_data.is_empty():
		_remove_patch(key)
		return

	patch["last_patch_data"] = patch_data
	var texture_width := int(patch_data["texture_width"])
	var texture_height := int(patch_data["texture_height"])
	var depth_nonzero_count := _patch_visible_depth_count(patch_data)
	var road_clip_signature := _patch_road_clip_signature(patch_data)
	var depth_signature: int = int(patch_data.get("depth_signature", 0))
	var signature_changed := int(patch.get("road_clip_signature", road_clip_signature - 1)) != road_clip_signature
	var depth_signature_changed: bool = int(patch.get("depth_signature", depth_signature - 1)) != depth_signature
	var depth_visibility_changed := int(patch.get("depth_nonzero_count", depth_nonzero_count - 1)) != depth_nonzero_count
	var texture_shape_changed := int(patch.get("texture_width", texture_width)) != texture_width or int(patch.get("texture_height", texture_height)) != texture_height
	var should_upload_textures := not road_clip_only or texture_shape_changed
	var texture_elapsed_ms := 0.0
	if should_upload_textures:
		var texture_start_us := Time.get_ticks_usec()
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
		texture_elapsed_ms = float(Time.get_ticks_usec() - texture_start_us) / 1000.0
		_water_debug_patch_uploads += 1

	var material: ShaderMaterial = patch["material"]
	material.set_shader_parameter("watermap_texture_size", Vector2(texture_width, texture_height))
	material.set_shader_parameter(
		"inner_sample_offset_texels",
		Vector2(float(patch_data["inner_offset_x"]), float(patch_data["inner_offset_z"]))
	)
	material.set_shader_parameter(
		"inner_sample_size_texels",
		Vector2(int(patch_data["sample_width"]), int(patch_data["sample_height"]))
	)
	var patch_node: MeshInstance3D = patch["node"]
	var world_size_x := float(patch_data["world_size_x"])
	var world_size_z := float(patch_data["world_size_z"])
	var lod_step := int(patch.get("lod_step", 1))
	var should_refresh_mesh := not road_clip_only or signature_changed or depth_signature_changed or depth_visibility_changed
	var mesh_elapsed_ms := 0.0
	var mesh_stats: Dictionary = patch.get("mesh_stats", _empty_water_mesh_stats(lod_step))
	if should_refresh_mesh:
		var mesh_start_us := Time.get_ticks_usec()
		mesh_stats = _empty_water_mesh_stats(lod_step)
		if depth_nonzero_count > 0:
			if resident_patch_lookup.has(key):
				_queue_patch_mesh_refresh(key, lod_step)
		else:
			patch_node.mesh = ArrayMesh.new()
			mesh_refresh_requested_lod.erase(key)
			mesh_pending_lod.erase(key)
		mesh_elapsed_ms = float(Time.get_ticks_usec() - mesh_start_us) / 1000.0
		patch["mesh_stats"] = mesh_stats
	patch["sample_width"] = int(patch_data["sample_width"])
	patch["sample_height"] = int(patch_data["sample_height"])
	patch["texture_width"] = texture_width
	patch["texture_height"] = texture_height
	patch["inner_offset_x"] = int(patch_data["inner_offset_x"])
	patch["inner_offset_z"] = int(patch_data["inner_offset_z"])
	patch["world_size_x"] = world_size_x
	patch["world_size_z"] = world_size_z
	patch["world_origin_x"] = float(patch_data["world_origin_x"])
	patch["world_origin_z"] = float(patch_data["world_origin_z"])
	patch["depth_nonzero_count"] = depth_nonzero_count
	patch["depth_signature"] = depth_signature
	patch["road_clip_signature"] = road_clip_signature
	patch_node.position = Vector3(
		float(patch_data["world_origin_x"]) + world_size_x * 0.5,
		0.0,
		float(patch_data["world_origin_z"]) + world_size_z * 0.5
	)
	if _terrain_debug_enabled:
		var total_elapsed_ms := float(Time.get_ticks_usec() - total_start_us) / 1000.0
		if total_elapsed_ms >= 2.0:
			_water_debug_log(
				"water_upload key=(%d,%d) road_clip_only=%s depth_nonzero=%d clip_loops=%d clip_points=%d signature_changed=%s mesh_cells=%d full=%d partial=%d conservative=%d dry=%d road_clipped=%d mesh_tris=%d texture_ms=%.3f mesh_ms=%.3f fetch_ms=%.3f total_ms=%.3f"
				% [
					key.x,
					key.y,
					str(road_clip_only),
					depth_nonzero_count,
					int(patch_data.get("road_clip_loop_count", 0)),
					int(patch_data.get("road_clip_point_count", 0)),
					str(signature_changed),
					int(mesh_stats.get("cells_total", 0)),
					int(mesh_stats.get("full_cells", 0)),
					int(mesh_stats.get("partial_cells", 0)),
					int(mesh_stats.get("conservative_cells", 0)),
					int(mesh_stats.get("dry_cells", 0)),
					int(mesh_stats.get("road_clipped_cells", 0)),
					int(mesh_stats.get("emitted_triangles", 0)),
					texture_elapsed_ms,
					mesh_elapsed_ms,
					fetch_elapsed_ms,
					total_elapsed_ms,
				]
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
	if int(patch.get("depth_nonzero_count", 0)) > 0:
		_queue_patch_mesh_refresh(key, int(patch.get("lod_step", 1)))
	_queue_height_texture_rebind(key)
	_queue_terrain_patch_binding(key)

func _deactivate_patch(key: Vector2i) -> void:
	if not resident_patch_lookup.has(key):
		return
	if not patches.has(key):
		resident_patch_lookup.erase(key)
		mesh_refresh_requested_lod.erase(key)
		mesh_pending_lod.erase(key)
		patch_lod_refresh_lookup.erase(key)
		return
	var patch: Dictionary = patches[key]
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.visible = false
	resident_patch_lookup.erase(key)
	mesh_refresh_requested_lod.erase(key)
	mesh_pending_lod.erase(key)
	patch_lod_refresh_lookup.erase(key)
	_queue_terrain_patch_binding(key)

func _remove_patch(key: Vector2i) -> void:
	if not patches.has(key):
		return
	var was_resident: bool = resident_patch_lookup.has(key)
	var patch: Dictionary = patches[key]
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.queue_free()
	patches.erase(key)
	resident_patch_lookup.erase(key)
	mesh_refresh_requested_lod.erase(key)
	mesh_pending_lod.erase(key)
	patch_lod_refresh_lookup.erase(key)
	if was_resident:
		_terrain_resident_patch_revision_seen = -1
	_queue_terrain_patch_binding(key)
	_water_debug_patch_removes += 1

func _clear_patches() -> void:
	for key in patches.keys():
		var patch: Dictionary = patches[key]
		var patch_node: MeshInstance3D = patch["node"]
		patch_node.queue_free()
	patches.clear()
	resident_patch_lookup.clear()
	patch_prewarm_queue.clear()
	height_texture_rebind_queue.clear()
	height_texture_rebind_lookup.clear()
	terrain_patch_binding_queue.clear()
	terrain_patch_binding_lookup.clear()
	mesh_refresh_queue.clear()
	mesh_refresh_requested_lod.clear()
	mesh_pending_lod.clear()
	mesh_apply_queue.clear()
	patch_lod_refresh_queue.clear()
	patch_lod_refresh_lookup.clear()
	_water_lod_refresh_camera_valid = false
	_record_lod_perf_counters(0, 0, 0, 0)
	_water_lod_last_deferred_count = 0
	_water_prewarm_last_deferred_count = 0
	_water_mesh_ready_backlog_estimate = 0
	_water_mesh_last_frame_elapsed_ms = 0.0
	_water_mesh_last_apply_elapsed_ms = 0.0
	_water_residency_pending_mutations = false
	_terrain_resident_patch_revision_seen = -1

func _rebuild_patch_prewarm_queue() -> void:
	patch_prewarm_queue.clear()
	if terrain_patch_cols <= 0 or terrain_patch_rows <= 0:
		return
	var prewarm_bounds: Dictionary = _water_prewarm_patch_bounds()
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

func _water_prewarm_patch_bounds() -> Dictionary:
	if terrain_patch_cols <= 0 or terrain_patch_rows <= 0:
		return {"min_x": 0, "max_x": -1, "min_z": 0, "max_z": -1}
	var keys: Array[Vector2i] = []
	if terrain_node != null and terrain_node.has_method("get_resident_patch_keys"):
		keys = terrain_node.get_resident_patch_keys()
	if keys.is_empty():
		keys = get_resident_patch_keys()
	if keys.is_empty():
		var camera_key: Vector2i = _current_camera_patch_key()
		return _water_expanded_patch_bounds(
			{
				"min_x": camera_key.x,
				"max_x": camera_key.x,
				"min_z": camera_key.y,
				"max_z": camera_key.y,
			},
			WATER_PATCH_PREWARM_HALO_PATCHES
		)
	var min_patch_x: int = terrain_patch_cols - 1
	var max_patch_x: int = 0
	var min_patch_z: int = terrain_patch_rows - 1
	var max_patch_z: int = 0
	for key in keys:
		min_patch_x = min(min_patch_x, key.x)
		max_patch_x = max(max_patch_x, key.x)
		min_patch_z = min(min_patch_z, key.y)
		max_patch_z = max(max_patch_z, key.y)
	return _water_expanded_patch_bounds(
		{
			"min_x": min_patch_x,
			"max_x": max_patch_x,
			"min_z": min_patch_z,
			"max_z": max_patch_z,
		},
		WATER_PATCH_PREWARM_HALO_PATCHES
	)

func _water_expanded_patch_bounds(bounds: Dictionary, margin_patches: int) -> Dictionary:
	return {
		"min_x": max(0, int(bounds["min_x"]) - margin_patches),
		"max_x": min(terrain_patch_cols - 1, int(bounds["max_x"]) + margin_patches),
		"min_z": max(0, int(bounds["min_z"]) - margin_patches),
		"max_z": min(terrain_patch_rows - 1, int(bounds["max_z"]) + margin_patches),
	}

func _prewarm_patch_cache() -> void:
	if patch_prewarm_queue.is_empty():
		return
	var budget_start_us: int = Time.get_ticks_usec()
	var attempted_patches := 0
	var created_patches := 0
	while attempted_patches < WATER_PATCH_PREWARM_MAX_PER_FRAME and not patch_prewarm_queue.is_empty():
		if _time_budget_exhausted(budget_start_us, WATER_PATCH_PREWARM_BUDGET_MS, created_patches):
			break
		var key: Vector2i = patch_prewarm_queue.pop_front()
		attempted_patches += 1
		if patches.has(key):
			continue
		_create_patch(key)
		created_patches += 1
		var patch: Dictionary = patches.get(key, {})
		if not patch.is_empty():
			var patch_node: MeshInstance3D = patch["node"]
			patch_node.visible = false

func _time_budget_exhausted(start_us: int, budget_ms: float, completed_count: int) -> bool:
	if completed_count <= 0:
		return false
	return float(Time.get_ticks_usec() - start_us) / 1000.0 >= budget_ms

func _water_frame_headroom_available(frame_start_us: int, start_budget_ms: float) -> bool:
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
	if terrain_patch_cols <= 0 or terrain_patch_rows <= 0 or terrain_patch_span_m <= 0.0:
		return Vector2i.ZERO
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return Vector2i(int(terrain_patch_cols / 2), int(terrain_patch_rows / 2))
	var half_world := terrain_world_size * 0.5
	return Vector2i(
		clampi(
			int(floor((camera.global_position.x + half_world.x) / terrain_patch_span_m)),
			0,
			terrain_patch_cols - 1
		),
		clampi(
			int(floor((camera.global_position.z + half_world.y) / terrain_patch_span_m)),
			0,
			terrain_patch_rows - 1
		)
	)

func _dirty_patch_keys(flat_pairs: PackedInt32Array) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	var pair_count := flat_pairs.size() / 2
	for index in range(pair_count):
		keys.append(Vector2i(flat_pairs[index * 2], flat_pairs[index * 2 + 1]))
	return keys

func road_geometry_debug_patch_lines(flat_pairs: PackedInt32Array) -> Array[String]:
	var lines: Array[String] = []
	lines.append(_road_geometry_water_border_line())
	var keys: Array[Vector2i] = _dirty_patch_keys(flat_pairs)
	if keys.is_empty():
		lines.append("water_patch none")
		return lines
	for key in keys:
		var patch: Dictionary = patches.get(key, {})
		var patch_data: Dictionary = patch.get("last_patch_data", {})
		if patch_data.is_empty():
			lines.append("water_patch key=(%d,%d) missing_cached_patch_data=true" % [key.x, key.y])
			continue
		var patch_node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
		var mesh: Mesh = null
		if patch_node != null:
			mesh = patch_node.mesh
		var depth_stats: Dictionary = _road_geometry_float_stats(
			patch_data["depth_data"] as PackedFloat32Array
		)
		var layer_stats: Dictionary = {}
		if simulation_node.has_method("get_water_patch_debug"):
			layer_stats = simulation_node.get_water_patch_debug(key.x, key.y) as Dictionary
		var depth_sample_count: int = (patch_data["depth_data"] as PackedFloat32Array).size()
		var layer_sample_count: int = int(layer_stats.get("total_samples", depth_sample_count))
		var baseline_nonzero_count: int = int(layer_stats.get("baseline_nonzero", -1))
		var clip_stats: Dictionary = _road_geometry_clip_stats(patch_data)
		var road_clip_status: String = str(patch_data.get("road_clip_status", "ok"))
		var road_clip_error: String = str(patch_data.get("road_clip_error", "none"))
		var road_clip_source_count: int = int(patch_data.get("road_clip_source_count", 0))
		var mesh_stats: Dictionary = patch.get(
			"mesh_stats",
			_empty_water_mesh_stats(int(patch.get("lod_step", 1)))
		)
		lines.append(
			"water_patch key=(%d,%d) resident=%s mesh=\"%s\" sample=%dx%d texture=%dx%d world_origin=(%.3f,%.3f) world_size=(%.3f,%.3f) depth_nonzero=%d/%d depth_min=%.3f depth_max=%.3f depth_sum=%.3f baseline_nonzero=%d/%d baseline_max=%.3f baseline_sum=%.3f visible_nonzero=%d/%d visible_max=%.3f visible_sum=%.3f clip_status=%s clip_error=%s clip_sources=%d clip_groups=%d clip_loops=%d clip_points=%d clip_area=%.3f clip_bounds=%s max_clip_bbox=(%.3f,%.3f) mesh_lod=%d mesh_cells=%d mesh_full=%d mesh_partial=%d mesh_conservative=%d mesh_dry=%d mesh_road_clipped=%d mesh_tris=%d"
			% [
				key.x,
				key.y,
				str(resident_patch_lookup.has(key)),
				_road_geometry_mesh_label(mesh),
				int(patch_data["sample_width"]),
				int(patch_data["sample_height"]),
				int(patch_data["texture_width"]),
				int(patch_data["texture_height"]),
				float(patch_data["world_origin_x"]),
				float(patch_data["world_origin_z"]),
				float(patch_data["world_size_x"]),
				float(patch_data["world_size_z"]),
				int(depth_stats.get("nonzero", 0)),
				depth_sample_count,
				float(depth_stats.get("min", 0.0)),
				float(depth_stats.get("max", 0.0)),
				float(depth_stats.get("sum", 0.0)),
				baseline_nonzero_count,
				layer_sample_count,
				float(layer_stats.get("baseline_max", -1.0)),
				float(layer_stats.get("baseline_sum", -1.0)),
				int(layer_stats.get("visible_nonzero", -1)),
				layer_sample_count,
				float(layer_stats.get("visible_max", -1.0)),
				float(layer_stats.get("visible_sum", -1.0)),
				road_clip_status,
				road_clip_error,
				road_clip_source_count,
				int(clip_stats.get("group_count", 0)),
				int(clip_stats.get("loop_count", 0)),
				int(clip_stats.get("point_count", 0)),
				float(clip_stats.get("area", 0.0)),
				_road_geometry_bounds_label(clip_stats),
				float(clip_stats.get("max_bbox_x", 0.0)),
				float(clip_stats.get("max_bbox_z", 0.0)),
				int(mesh_stats.get("lod_step", patch.get("lod_step", 1))),
				int(mesh_stats.get("cells_total", 0)),
				int(mesh_stats.get("full_cells", 0)),
				int(mesh_stats.get("partial_cells", 0)),
				int(mesh_stats.get("conservative_cells", 0)),
				int(mesh_stats.get("dry_cells", 0)),
				int(mesh_stats.get("road_clipped_cells", 0)),
				int(mesh_stats.get("emitted_triangles", 0)),
			]
		)
		if baseline_nonzero_count > 0 and simulation_node.has_method("get_water_patch_authored_fill_debug"):
			var fill_debug: Array = simulation_node.get_water_patch_authored_fill_debug(key.x, key.y) as Array
			if fill_debug.is_empty():
				lines.append(
					"water_patch_baseline_fill key=(%d,%d) contributors=0 warning=baseline_without_authored_contributor"
					% [key.x, key.y]
				)
			for fill_variant in fill_debug:
				var fill: Dictionary = fill_variant as Dictionary
				lines.append(
					"water_patch_baseline_fill key=(%d,%d) kind=%s index=%d preview=%s seed=(%.1f,%.1f) surface=%.3f filled_cells=%d touches_edge=%s patch_nonzero=%d patch_max=%.3f patch_sum=%.3f"
					% [
						key.x,
						key.y,
						str(fill.get("kind", "unknown")),
						int(fill.get("fill_index", -999)),
						str(bool(fill.get("preview", false))),
						float(fill.get("world_x", 0.0)),
						float(fill.get("world_z", 0.0)),
						float(fill.get("surface_elevation_m", 0.0)),
						int(fill.get("filled_cells", 0)),
						str(bool(fill.get("touches_world_edge", false))),
						int(fill.get("patch_nonzero_samples", 0)),
						float(fill.get("patch_max_depth_m", 0.0)),
						float(fill.get("patch_sum_depth_m", 0.0)),
					]
				)
	return lines

func _road_geometry_water_border_line() -> String:
	var border_depths: PackedFloat32Array = simulation_node.get_water_border_depths()
	var depth_stats: Dictionary = _road_geometry_float_stats(border_depths)
	var border_mesh: Mesh = null
	if water_border_instance != null:
		border_mesh = water_border_instance.mesh
	var terrain_revision: int = _current_terrain_border_revision()
	return (
		"water_border mesh=\"%s\" terrain_border_revision=%d depth_nonzero=%d/%d depth_min=%.3f depth_max=%.3f depth_sum=%.3f"
		% [
			_road_geometry_mesh_label(border_mesh),
			terrain_revision,
			int(depth_stats.get("nonzero", 0)),
			border_depths.size(),
			float(depth_stats.get("min", 0.0)),
			float(depth_stats.get("max", 0.0)),
			float(depth_stats.get("sum", 0.0)),
		]
	)

func _patch_has_road_clip_loops(patch_data: Dictionary) -> bool:
	if (
		not patch_data.has("road_clip_loop_counts")
		or not patch_data.has("road_clip_loop_groups")
		or not patch_data.has("road_clip_loop_roles")
		or not patch_data.has("road_clip_loop_points")
	):
		return false
	var counts: PackedInt32Array = patch_data["road_clip_loop_counts"] as PackedInt32Array
	var groups: PackedInt32Array = patch_data["road_clip_loop_groups"] as PackedInt32Array
	var roles: PackedInt32Array = patch_data["road_clip_loop_roles"] as PackedInt32Array
	var points: PackedVector3Array = patch_data["road_clip_loop_points"] as PackedVector3Array
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

func _patch_visible_depth_count(patch_data: Dictionary) -> int:
	return int(patch_data.get("depth_nonzero_count", 1))

func _patch_road_clip_signature(patch_data: Dictionary) -> int:
	return int(patch_data.get("road_clip_signature", 0))

func _road_clip_loop_groups_from_patch_data(patch_data: Dictionary) -> Array:
	if not _patch_has_road_clip_loops(patch_data):
		return []
	var counts: PackedInt32Array = patch_data["road_clip_loop_counts"] as PackedInt32Array
	var group_indices: PackedInt32Array = patch_data["road_clip_loop_groups"] as PackedInt32Array
	var roles: PackedInt32Array = patch_data["road_clip_loop_roles"] as PackedInt32Array
	var points: PackedVector3Array = patch_data["road_clip_loop_points"] as PackedVector3Array
	var groups_by_id: Dictionary = {}
	var group_ids: Array = []
	var cursor := 0
	for loop_index in range(counts.size()):
		var count: int = counts[loop_index]
		var group_id: int = group_indices[loop_index]
		var loop_points := PackedVector2Array()
		for offset in range(count):
			var point: Vector3 = points[cursor + offset]
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

func _empty_water_mesh_stats(lod_step: int) -> Dictionary:
	return {
		"lod_step": lod_step,
		"cells_total": 0,
		"full_cells": 0,
		"partial_cells": 0,
		"conservative_cells": 0,
		"dry_cells": 0,
		"road_clipped_cells": 0,
		"emitted_vertices": 0,
		"emitted_triangles": 0,
	}

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

func _mesh_lod_step_for_patch_center(center_x: float, center_z: float) -> int:
	var camera: Camera3D = get_viewport().get_camera_3d()
	if camera == null:
		return 1
	return _mesh_lod_step_for_patch_center_with_camera(
		center_x,
		center_z,
		camera.global_position,
		true
	)

func _mesh_lod_step_for_patch_center_with_camera(
	center_x: float,
	center_z: float,
	camera_position: Vector3,
	camera_valid: bool
) -> int:
	if _terrain_force_lod1:
		return 1
	if not camera_valid:
		return 1
	var distance_m := camera_position.distance_to(Vector3(center_x, 0.0, center_z))
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
	_water_lod_last_deferred_count = 0
	if resident_patch_lookup.is_empty():
		_water_mesh_lod_refresh_elapsed_s = 0.0
		patch_lod_refresh_queue.clear()
		patch_lod_refresh_lookup.clear()
		_record_lod_perf_counters(0, 0, 0, 0)
		return
	var queued_count := 0
	var replaced_count: int = 0
	_water_lod_last_skipped_count = 0
	_water_mesh_lod_refresh_elapsed_s += delta
	if (
		_water_mesh_lod_refresh_elapsed_s >= WATER_PATCH_MESH_LOD_REFRESH_INTERVAL_S
		and _lod_refresh_camera_moved(WATER_PATCH_MESH_LOD_REFRESH_CAMERA_MOVE_M)
	):
		_water_mesh_lod_refresh_elapsed_s = 0.0
		replaced_count = patch_lod_refresh_queue.size()
		queued_count = _replace_resident_patch_lod_refreshes()
	_process_patch_lod_refresh_queue(
		WATER_PATCH_MESH_LOD_REFRESH_BUDGET_MS,
		WATER_PATCH_MESH_LOD_REFRESH_MAX_CHECKS_PER_FRAME,
		WATER_PATCH_MESH_LOD_REFRESH_MAX_CHANGES_PER_FRAME
	)
	_water_lod_last_queued_count = queued_count
	_water_lod_last_queue_count = patch_lod_refresh_queue.size()
	_water_lod_last_replaced_count = replaced_count

func _defer_patch_mesh_lods(delta: float) -> void:
	_water_mesh_lod_refresh_elapsed_s += delta
	_water_lod_last_deferred_count = 1
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
	if not _water_lod_refresh_camera_valid:
		_water_lod_refresh_camera_valid = true
		_water_lod_refresh_last_camera_position = position
		return true
	if position.distance_squared_to(_water_lod_refresh_last_camera_position) < min_distance_m * min_distance_m:
		return false
	_water_lod_refresh_last_camera_position = position
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
		if _water_patch_lod_refresh_needed(key, camera_position, camera_valid):
			candidates.append(key)
		else:
			_water_lod_last_skipped_count += 1
	_sort_patch_keys_by_camera_priority(candidates)
	patch_lod_refresh_lookup.clear()
	for key in candidates:
		patch_lod_refresh_lookup[key] = true
	patch_lod_refresh_queue = candidates
	return candidates.size()

func _water_patch_lod_refresh_needed(
	key: Vector2i,
	camera_position: Vector3,
	camera_valid: bool
) -> bool:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return false
	var patch_node: MeshInstance3D = patch["node"]
	var target_lod_step: int = _mesh_lod_step_for_patch_center_with_camera(
		patch_node.position.x,
		patch_node.position.z,
		camera_position,
		camera_valid
	)
	return int(patch.get("lod_step", 1)) != target_lod_step

func _process_patch_lod_refresh_queue(
	refresh_budget_ms: float,
	max_checks_per_frame: int,
	max_changes_per_frame: int
) -> void:
	_water_lod_last_processed_count = 0
	_water_lod_last_changed_count = 0
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
	_water_lod_last_processed_count = processed_count
	_water_lod_last_changed_count = changed_count

func _refresh_one_patch_mesh_lod(key: Vector2i) -> bool:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return false
	var patch_node: MeshInstance3D = patch["node"]
	var target_lod_step := _mesh_lod_step_for_patch_center(patch_node.position.x, patch_node.position.z)
	var current_lod_step := int(patch.get("lod_step", 1))
	if current_lod_step == target_lod_step:
		return false
	patch["lod_step"] = target_lod_step
	var material: ShaderMaterial = patch["material"]
	material.set_shader_parameter("water_debug_lod_step", float(target_lod_step))
	var mesh_stats := _empty_water_mesh_stats(target_lod_step)
	if int(patch.get("depth_nonzero_count", 0)) > 0:
		_queue_patch_mesh_refresh(key, target_lod_step)
	else:
		patch_node.mesh = ArrayMesh.new()
	patch["mesh_stats"] = mesh_stats
	return true

func _record_lod_perf_counters(
	processed_count: int,
	changed_count: int,
	queued_count: int,
	queue_count: int,
	replaced_count: int = 0,
	skipped_count: int = 0
) -> void:
	_water_lod_last_processed_count = processed_count
	_water_lod_last_changed_count = changed_count
	_water_lod_last_queued_count = queued_count
	_water_lod_last_queue_count = queue_count
	_water_lod_last_replaced_count = replaced_count
	_water_lod_last_skipped_count = skipped_count

func _queue_patch_mesh_refresh(key: Vector2i, lod_step: int) -> void:
	var safe_lod_step: int = max(1, lod_step)
	if (
		int(mesh_pending_lod.get(key, 0)) == safe_lod_step
		and not mesh_refresh_requested_lod.has(key)
	):
		return
	if not mesh_refresh_requested_lod.has(key):
		mesh_refresh_queue.append(key)
	mesh_refresh_requested_lod[key] = safe_lod_step

func _process_mesh_refresh_queue(budget: int, collect_perf_stats: bool = false) -> Dictionary:
	var perf_stats: Dictionary = {}
	if collect_perf_stats:
		perf_stats = {
			"mesh_queue_count": float(mesh_refresh_queue.size()),
			"mesh_requested_count": float(mesh_refresh_requested_lod.size()),
			"mesh_pending_count": float(mesh_pending_lod.size()),
			"mesh_apply_queue_count": float(mesh_apply_queue.size()),
			"mesh_submit": 0.0,
			"mesh_poll": 0.0,
			"mesh_apply": 0.0,
			"mesh_refresh_stage_deferred_count": 0.0,
			"mesh_queue_compact": 0.0,
			"mesh_queue_compact_removed_count": 0.0,
			"mesh_submit_budget_count": float(budget),
			"mesh_submit_backlog_active_count": 0.0,
			"mesh_submit_inspected_count": 0.0,
			"mesh_submit_stale_count": 0.0,
			"mesh_submit_dry_count": 0.0,
			"mesh_submit_request_sent_count": 0.0,
			"mesh_submitted_count": 0.0,
			"mesh_request_raw_count": 0.0,
			"mesh_request_accepted_count": 0.0,
			"mesh_request_invalid_count": 0.0,
			"mesh_request_duplicate_count": 0.0,
			"mesh_request_cache_hit_count": 0.0,
			"mesh_request_in_flight_count": 0.0,
			"mesh_request_build_queued_count": 0.0,
			"mesh_poll_return_budget_count": 0.0,
			"mesh_poll_backlog_active_count": 0.0,
			"mesh_poll_headroom_active_count": 0.0,
			"mesh_poll_apply_queue_room_count": 0.0,
			"mesh_ready_backlog_estimate_count": float(_water_mesh_ready_backlog_estimate),
			"mesh_poll_requested_count": 0.0,
			"mesh_poll_ready_returned_count": 0.0,
			"mesh_poll_local_stale_count": 0.0,
			"mesh_ready_completed_count": 0.0,
			"mesh_ready_before_count": 0.0,
			"mesh_ready_emitted_count": 0.0,
			"mesh_ready_stale_count": 0.0,
			"mesh_ready_missing_count": 0.0,
			"mesh_ready_requested_before_count": 0.0,
			"mesh_ready_requested_after_count": 0.0,
			"mesh_polled_count": 0.0,
			"mesh_submit_batch_count": 0.0,
			"mesh_apply_processed_count": 0.0,
			"mesh_apply_limit_count": 0.0,
			"mesh_apply_budget_ms": 0.0,
			"mesh_apply_boost_active_count": 0.0,
			"mesh_apply_headroom_frame_ms": _water_mesh_last_frame_elapsed_ms,
			"mesh_apply_previous_ms": _water_mesh_last_apply_elapsed_ms,
			"mesh_apply_stale_count": 0.0,
			"mesh_apply_queue_sort_count": 0.0,
			"mesh_applied_count": 0.0,
			"mesh_queue_after_count": 0.0,
			"mesh_pending_after_count": 0.0,
			"mesh_apply_queue_after_count": 0.0,
		}
	if simulation_node == null:
		return perf_stats

	var compact_start_us := Time.get_ticks_usec()
	var compact_removed_count: int = 0
	if _water_patch_mesh_queue_compaction_needed():
		compact_removed_count = _compact_water_patch_mesh_refresh_queue()
	if collect_perf_stats:
		perf_stats["mesh_queue_compact"] = float(Time.get_ticks_usec() - compact_start_us) / 1000.0
		perf_stats["mesh_queue_compact_removed_count"] = float(compact_removed_count)

	var refresh_work_start_us: int = Time.get_ticks_usec()
	var deferred_stage_count := 0
	var completed_stage_count := 0
	var apply_start_us := Time.get_ticks_usec()
	var previous_apply_elapsed_ms: float = _water_mesh_last_apply_elapsed_ms
	var apply_boost_active: bool = _water_patch_mesh_apply_boost_active()
	var apply_limit: int = _water_patch_mesh_apply_limit(apply_boost_active)
	var apply_budget_ms: float = _water_patch_mesh_apply_budget_ms(apply_boost_active)
	var apply_queue_before: int = mesh_apply_queue.size()
	var applied_count: int = _apply_ready_water_patch_meshes(
		apply_budget_ms,
		apply_limit,
		perf_stats
	)
	var apply_elapsed_ms := float(Time.get_ticks_usec() - apply_start_us) / 1000.0
	_water_mesh_last_apply_elapsed_ms = apply_elapsed_ms
	if apply_queue_before != mesh_apply_queue.size():
		completed_stage_count += 1
	if collect_perf_stats:
		perf_stats["mesh_apply"] = apply_elapsed_ms
		perf_stats["mesh_applied_count"] = float(applied_count)
		perf_stats["mesh_apply_limit_count"] = float(apply_limit)
		perf_stats["mesh_apply_budget_ms"] = apply_budget_ms
		perf_stats["mesh_apply_boost_active_count"] = 1.0 if apply_boost_active else 0.0
		perf_stats["mesh_apply_headroom_frame_ms"] = _water_mesh_last_frame_elapsed_ms
		perf_stats["mesh_apply_previous_ms"] = previous_apply_elapsed_ms

	var polled_count := 0
	if _water_mesh_refresh_stage_budget_exhausted(refresh_work_start_us, completed_stage_count):
		if not mesh_pending_lod.is_empty():
			deferred_stage_count += 1
	else:
		var poll_start_us := Time.get_ticks_usec()
		polled_count = _poll_water_patch_mesh_results(perf_stats)
		if polled_count > 0:
			completed_stage_count += 1
		if collect_perf_stats:
			perf_stats["mesh_poll"] = float(Time.get_ticks_usec() - poll_start_us) / 1000.0
			perf_stats["mesh_polled_count"] = float(polled_count)

	var submit_backlog_active := false
	var submit_budget := 0
	var submitted_count := 0
	if _water_mesh_refresh_stage_budget_exhausted(refresh_work_start_us, completed_stage_count):
		if not mesh_refresh_queue.is_empty():
			deferred_stage_count += 1
	else:
		var submit_start_us := Time.get_ticks_usec()
		submit_backlog_active = _water_patch_mesh_backlog_active()
		submit_budget = _water_patch_mesh_submit_budget(budget, submit_backlog_active)
		submitted_count = _submit_water_patch_mesh_requests(
			submit_budget,
			WATER_PATCH_MESH_SUBMIT_BUDGET_MS,
			perf_stats,
			submit_backlog_active
		)
		if collect_perf_stats:
			perf_stats["mesh_submit"] = float(Time.get_ticks_usec() - submit_start_us) / 1000.0
			perf_stats["mesh_submit_budget_count"] = float(submit_budget)
			perf_stats["mesh_submitted_count"] = float(submitted_count)

	if collect_perf_stats:
		perf_stats["mesh_refresh_stage_deferred_count"] = float(deferred_stage_count)
		perf_stats["mesh_submit_budget_count"] = float(submit_budget)
		perf_stats["mesh_submitted_count"] = float(submitted_count)
		perf_stats["mesh_queue_after_count"] = float(mesh_refresh_queue.size())
		perf_stats["mesh_pending_after_count"] = float(mesh_pending_lod.size())
		perf_stats["mesh_apply_queue_after_count"] = float(mesh_apply_queue.size())
	return perf_stats

func _water_patch_mesh_queue_compaction_needed() -> bool:
	var queue_count: int = mesh_refresh_queue.size()
	if queue_count == 0:
		return false
	var requested_count: int = mesh_refresh_requested_lod.size()
	if requested_count == 0:
		return true
	return queue_count >= 256 or queue_count >= requested_count * 2 + 32

func _compact_water_patch_mesh_refresh_queue() -> int:
	if mesh_refresh_queue.is_empty():
		return 0
	var removed_count: int = 0
	if mesh_refresh_requested_lod.is_empty():
		removed_count = mesh_refresh_queue.size()
		mesh_refresh_queue.clear()
		return removed_count

	var compacted_queue: Array[Vector2i] = []
	var retained_lookup: Dictionary = {}
	for key in mesh_refresh_queue:
		if not mesh_refresh_requested_lod.has(key) or retained_lookup.has(key):
			removed_count += 1
			continue
		retained_lookup[key] = true
		compacted_queue.append(key)
	_sort_patch_keys_by_camera_priority(compacted_queue)
	mesh_refresh_queue = compacted_queue
	return removed_count

func _water_patch_mesh_submit_budget(base_budget: int, backlog_active: bool) -> int:
	var pending_count: int = mesh_pending_lod.size()
	if pending_count >= WATER_PATCH_MESH_PENDING_HARD_LIMIT:
		return 0
	if pending_count >= WATER_PATCH_MESH_PENDING_SOFT_LIMIT:
		return mini(base_budget, 2)
	if _water_patch_mesh_ready_backlog_active():
		return mini(base_budget, WATER_PATCH_MESH_BUSY_REQUEST_BUDGET_PER_FRAME)
	if backlog_active:
		return maxi(base_budget, WATER_PATCH_MESH_BACKLOG_REQUEST_BUDGET_PER_FRAME)
	return base_budget

func _water_mesh_refresh_stage_budget_exhausted(start_us: int, completed_stage_count: int) -> bool:
	if completed_stage_count <= 0:
		return false
	return float(Time.get_ticks_usec() - start_us) / 1000.0 >= WATER_PATCH_MESH_REFRESH_SOFT_BUDGET_MS

func _water_patch_mesh_backlog_active() -> bool:
	return (
		mesh_pending_lod.size() < WATER_PATCH_MESH_PENDING_SOFT_LIMIT
		and mesh_refresh_requested_lod.size() >= WATER_PATCH_MESH_BACKLOG_REQUEST_THRESHOLD
	)

func _submit_water_patch_mesh_requests(
	budget: int,
	submit_budget_ms: float,
	perf_stats: Dictionary = {},
	backlog_active: bool = false
) -> int:
	if not simulation_node.has_method("request_water_patch_meshes"):
		return 0
	if budget <= 0:
		return 0
	var submit_start_us: int = Time.get_ticks_usec()
	var submitted_count := 0
	var inspected_count := 0
	var stale_count := 0
	var dry_count := 0
	var request_sent_count := 0
	var request_raw_count := 0
	var request_accepted_count := 0
	var request_invalid_count := 0
	var request_duplicate_count := 0
	var request_cache_hit_count := 0
	var request_in_flight_count := 0
	var request_build_queued_count := 0
	var batch_count := 0
	while submitted_count < budget and not mesh_refresh_queue.is_empty():
		if _time_budget_exhausted(submit_start_us, submit_budget_ms, inspected_count):
			break
		var requests := PackedInt32Array()
		var batch_remaining: int = mini(WATER_PATCH_MESH_SUBMIT_BATCH_SIZE, budget - submitted_count)
		while batch_remaining > 0 and not mesh_refresh_queue.is_empty():
			if _time_budget_exhausted(submit_start_us, submit_budget_ms, inspected_count):
				break
			var key: Vector2i = mesh_refresh_queue.pop_front()
			inspected_count += 1
			if not mesh_refresh_requested_lod.has(key):
				stale_count += 1
				continue
			var lod_step := int(mesh_refresh_requested_lod[key])
			mesh_refresh_requested_lod.erase(key)
			var patch: Dictionary = patches.get(key, {})
			if patch.is_empty() or not resident_patch_lookup.has(key):
				mesh_pending_lod.erase(key)
				stale_count += 1
				continue
			if int(patch.get("depth_nonzero_count", 0)) <= 0:
				var patch_node: MeshInstance3D = patch["node"]
				patch_node.mesh = ArrayMesh.new()
				patch["mesh_stats"] = _empty_water_mesh_stats(int(patch.get("lod_step", 1)))
				mesh_pending_lod.erase(key)
				dry_count += 1
				continue
			if int(patch.get("lod_step", 1)) != lod_step:
				stale_count += 1
				continue
			requests.push_back(key.x)
			requests.push_back(key.y)
			requests.push_back(lod_step)
			mesh_pending_lod[key] = lod_step
			batch_remaining -= 1
		if requests.is_empty():
			continue
		var request_result: Dictionary = simulation_node.request_water_patch_meshes(requests) as Dictionary
		var batch_submitted_count: int = int(request_result.get("accepted_count", requests.size() / 3))
		request_sent_count += requests.size() / 3
		request_raw_count += int(request_result.get("raw_count", requests.size() / 3))
		request_accepted_count += batch_submitted_count
		request_invalid_count += int(request_result.get("invalid_count", 0))
		request_duplicate_count += int(request_result.get("duplicate_count", 0))
		request_cache_hit_count += int(request_result.get("cache_hit_count", 0))
		request_in_flight_count += int(request_result.get("in_flight_count", 0))
		request_build_queued_count += int(request_result.get("build_queued_count", 0))
		submitted_count += batch_submitted_count
		batch_count += 1
	if not perf_stats.is_empty():
		perf_stats["mesh_submit_backlog_active_count"] = 1.0 if backlog_active else 0.0
		perf_stats["mesh_submit_inspected_count"] = float(inspected_count)
		perf_stats["mesh_submit_stale_count"] = float(stale_count)
		perf_stats["mesh_submit_dry_count"] = float(dry_count)
		perf_stats["mesh_submit_request_sent_count"] = float(request_sent_count)
		perf_stats["mesh_request_raw_count"] = float(request_raw_count)
		perf_stats["mesh_request_accepted_count"] = float(request_accepted_count)
		perf_stats["mesh_request_invalid_count"] = float(request_invalid_count)
		perf_stats["mesh_request_duplicate_count"] = float(request_duplicate_count)
		perf_stats["mesh_request_cache_hit_count"] = float(request_cache_hit_count)
		perf_stats["mesh_request_in_flight_count"] = float(request_in_flight_count)
		perf_stats["mesh_request_build_queued_count"] = float(request_build_queued_count)
		perf_stats["mesh_submit_batch_count"] = float(batch_count)
	return submitted_count

func _water_patch_mesh_poll_budget() -> int:
	var apply_queue_count: int = mesh_apply_queue.size()
	if apply_queue_count >= WATER_PATCH_MESH_APPLY_QUEUE_HARD_LIMIT:
		return 0
	var apply_queue_room: int = WATER_PATCH_MESH_APPLY_QUEUE_HARD_LIMIT - apply_queue_count
	var pending_count: int = mesh_pending_lod.size()
	if pending_count <= 0:
		return 0
	var backlog_active: bool = (
		pending_count >= WATER_PATCH_MESH_POLL_BACKLOG_THRESHOLD
		or _water_mesh_ready_backlog_estimate >= WATER_PATCH_MESH_POLL_BACKLOG_THRESHOLD
	)
	var target_budget := WATER_PATCH_MESH_POLL_BUDGET_PER_FRAME
	if backlog_active and _water_patch_mesh_apply_boost_active():
		target_budget = WATER_PATCH_MESH_POLL_HEADROOM_BUDGET_PER_FRAME
	elif backlog_active and apply_queue_count < WATER_PATCH_MESH_APPLY_QUEUE_SOFT_LIMIT:
		target_budget = WATER_PATCH_MESH_POLL_BACKLOG_BUDGET_PER_FRAME
	return mini(pending_count, mini(apply_queue_room, target_budget))

func _water_patch_mesh_ready_backlog_active() -> bool:
	return (
		_water_mesh_ready_backlog_estimate >= WATER_PATCH_MESH_READY_BACKLOG_BOOST_THRESHOLD
		or mesh_apply_queue.size() >= WATER_PATCH_MESH_APPLY_QUEUE_SOFT_LIMIT
	)

func _water_patch_mesh_apply_boost_active() -> bool:
	return (
		_water_patch_mesh_ready_backlog_active()
		and _water_mesh_last_frame_elapsed_ms > 0.0
		and _water_mesh_last_frame_elapsed_ms <= WATER_PATCH_MESH_APPLY_HEADROOM_FRAME_MS
		and _water_mesh_last_apply_elapsed_ms <= WATER_PATCH_MESH_APPLY_HEADROOM_PREVIOUS_APPLY_MS
	)

func _water_patch_mesh_apply_limit(boost_active: bool) -> int:
	if boost_active:
		return WATER_PATCH_MESH_APPLY_HEADROOM_MAX_PER_FRAME
	return WATER_PATCH_MESH_APPLY_MAX_PER_FRAME

func _water_patch_mesh_apply_budget_ms(boost_active: bool) -> float:
	if boost_active:
		return WATER_PATCH_MESH_APPLY_HEADROOM_BUDGET_MS
	return WATER_PATCH_MESH_APPLY_BUDGET_MS

func _poll_water_patch_mesh_results(perf_stats: Dictionary) -> int:
	if not simulation_node.has_method("poll_ready_water_patch_meshes") or mesh_pending_lod.is_empty():
		_water_mesh_ready_backlog_estimate = 0
		return 0

	var poll_budget: int = _water_patch_mesh_poll_budget()
	var apply_queue_room: int = max(0, WATER_PATCH_MESH_APPLY_QUEUE_HARD_LIMIT - mesh_apply_queue.size())
	if not perf_stats.is_empty():
		var headroom_active: bool = _water_patch_mesh_apply_boost_active()
		perf_stats["mesh_poll_requested_count"] = float(mesh_pending_lod.size())
		perf_stats["mesh_poll_return_budget_count"] = float(poll_budget)
		perf_stats["mesh_poll_backlog_active_count"] = (
			1.0
			if (
				mesh_pending_lod.size() >= WATER_PATCH_MESH_POLL_BACKLOG_THRESHOLD
				or _water_mesh_ready_backlog_estimate >= WATER_PATCH_MESH_POLL_BACKLOG_THRESHOLD
			)
			else 0.0
		)
		perf_stats["mesh_poll_headroom_active_count"] = 1.0 if headroom_active else 0.0
		perf_stats["mesh_poll_apply_queue_room_count"] = float(apply_queue_room)
		perf_stats["mesh_ready_backlog_estimate_count"] = float(_water_mesh_ready_backlog_estimate)
	if poll_budget <= 0:
		return 0

	var poll_result: Dictionary = simulation_node.poll_ready_water_patch_meshes(
		poll_budget
	) as Dictionary
	var meshes: Array = poll_result.get("meshes", []) as Array
	var polled_count: int = 0
	var local_stale_count: int = 0
	for mesh_variant in meshes:
		var mesh_data: Dictionary = mesh_variant as Dictionary
		var key: Vector2i = Vector2i(
			int(mesh_data.get("patch_x", -1)),
			int(mesh_data.get("patch_z", -1))
		)
		var lod_step: int = int(mesh_data.get("lod_step", 1))
		if int(mesh_pending_lod.get(key, 0)) != lod_step:
			local_stale_count += 1
			continue
		if not _water_patch_mesh_data_matches_patch(key, lod_step, mesh_data):
			local_stale_count += 1
			continue
		mesh_pending_lod.erase(key)
		mesh_apply_queue.append(mesh_data)
		polled_count += 1
	var sorted_apply_queue_count := 0
	if polled_count > 0 and mesh_apply_queue.size() > 1:
		_sort_mesh_apply_queue_by_camera_priority()
		sorted_apply_queue_count = mesh_apply_queue.size()
	var ready_before_count: int = int(poll_result.get("ready_before_count", meshes.size()))
	var emitted_count: int = int(poll_result.get("emitted_count", meshes.size()))
	var requested_after_count: int = int(poll_result.get("requested_after_count", mesh_pending_lod.size()))
	_water_mesh_ready_backlog_estimate = max(0, max(ready_before_count - emitted_count, requested_after_count))
	if not perf_stats.is_empty():
		perf_stats["mesh_poll_ready_returned_count"] = float(meshes.size())
		perf_stats["mesh_poll_local_stale_count"] = float(local_stale_count)
		perf_stats["mesh_apply_queue_sort_count"] = float(sorted_apply_queue_count)
		perf_stats["mesh_ready_completed_count"] = float(poll_result.get("completed_count", 0))
		perf_stats["mesh_ready_before_count"] = float(ready_before_count)
		perf_stats["mesh_ready_emitted_count"] = float(emitted_count)
		perf_stats["mesh_ready_stale_count"] = float(poll_result.get("stale_ready_count", 0))
		perf_stats["mesh_ready_missing_count"] = float(poll_result.get("missing_ready_count", 0))
		perf_stats["mesh_ready_requested_before_count"] = float(
			poll_result.get("requested_before_count", 0)
		)
		perf_stats["mesh_ready_requested_after_count"] = float(requested_after_count)
		perf_stats["mesh_ready_backlog_estimate_count"] = float(_water_mesh_ready_backlog_estimate)
	return polled_count

func _sort_mesh_apply_queue_by_camera_priority() -> void:
	var origin: Vector2i = _current_camera_patch_key()
	mesh_apply_queue.sort_custom(func(a: Dictionary, b: Dictionary):
		var ax: int = int(a.get("patch_x", -1))
		var ay: int = int(a.get("patch_z", -1))
		var bx: int = int(b.get("patch_x", -1))
		var by: int = int(b.get("patch_z", -1))
		var distance_a: int = absi(ax - origin.x) + absi(ay - origin.y)
		var distance_b: int = absi(bx - origin.x) + absi(by - origin.y)
		if distance_a == distance_b:
			if ay == by:
				return ax < bx
			return ay < by
		return distance_a < distance_b
	)

func _apply_ready_water_patch_meshes(
	apply_budget_ms: float,
	max_patches: int,
	perf_stats: Dictionary
) -> int:
	if mesh_apply_queue.is_empty():
		return 0
	var apply_start_us := Time.get_ticks_usec()
	var processed_count: int = 0
	var applied_count: int = 0
	while processed_count < max_patches and not mesh_apply_queue.is_empty():
		var mesh_data: Dictionary = mesh_apply_queue.pop_front()
		if _apply_water_patch_mesh_data(mesh_data):
			applied_count += 1
		processed_count += 1
		var elapsed_ms := float(Time.get_ticks_usec() - apply_start_us) / 1000.0
		if processed_count > 0 and elapsed_ms >= apply_budget_ms:
			break
	if not perf_stats.is_empty():
		perf_stats["mesh_apply_processed_count"] = float(processed_count)
		perf_stats["mesh_apply_stale_count"] = float(processed_count - applied_count)
	return applied_count

func _apply_water_patch_mesh_data(mesh_data: Dictionary) -> bool:
	var key: Vector2i = Vector2i(
		int(mesh_data.get("patch_x", -1)),
		int(mesh_data.get("patch_z", -1))
	)
	var lod_step: int = int(mesh_data.get("lod_step", 1))
	if not _water_patch_mesh_data_matches_patch(key, lod_step, mesh_data):
		return false
	var patch: Dictionary = patches[key]
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.mesh = _baked_water_patch_mesh(mesh_data)
	patch["mesh_stats"] = _water_mesh_stats_from_baked_data(mesh_data)
	return true

func _water_patch_mesh_data_matches_patch(
	key: Vector2i,
	lod_step: int,
	mesh_data: Dictionary
) -> bool:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty() or not resident_patch_lookup.has(key):
		return false
	if lod_step <= 0 or int(patch.get("lod_step", 1)) != lod_step:
		return false
	if int(patch.get("depth_nonzero_count", 0)) <= 0:
		return false
	var expected_road_clip_signature: int = int(patch.get("road_clip_signature", 0))
	var mesh_road_clip_signature: int = int(
		mesh_data.get("road_clip_signature", expected_road_clip_signature - 1)
	)
	if mesh_road_clip_signature != expected_road_clip_signature:
		return false
	var expected_depth_signature: int = int(patch.get("depth_signature", 0))
	var mesh_depth_signature: int = int(mesh_data.get("depth_signature", expected_depth_signature - 1))
	return mesh_depth_signature == expected_depth_signature

func _baked_water_patch_mesh(mesh_data: Dictionary) -> ArrayMesh:
	var mesh := ArrayMesh.new()
	var vertices: PackedVector3Array = mesh_data.get("vertices", PackedVector3Array()) as PackedVector3Array
	if vertices.size() < 3:
		return mesh
	var normals: PackedVector3Array = mesh_data.get("normals", PackedVector3Array()) as PackedVector3Array
	var uvs: PackedVector2Array = mesh_data.get("uvs", PackedVector2Array()) as PackedVector2Array
	var indices: PackedInt32Array = mesh_data.get("indices", PackedInt32Array()) as PackedInt32Array
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

func _water_mesh_stats_from_baked_data(mesh_data: Dictionary) -> Dictionary:
	return {
		"lod_step": int(mesh_data.get("lod_step", 1)),
		"cells_total": int(mesh_data.get("mesh_cells", 0)),
		"full_cells": int(mesh_data.get("mesh_full_cells", 0)),
		"partial_cells": int(mesh_data.get("mesh_partial_cells", 0)),
		"conservative_cells": int(mesh_data.get("mesh_conservative_cells", 0)),
		"dry_cells": int(mesh_data.get("mesh_dry_cells", 0)),
		"road_clipped_cells": int(mesh_data.get("mesh_road_clipped_cells", 0)),
		"emitted_vertices": int(mesh_data.get("mesh_emitted_vertices", 0)),
		"emitted_triangles": int(mesh_data.get("mesh_emitted_triangles", 0)),
	}

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

func _queue_height_texture_rebind(key: Vector2i) -> void:
	if height_texture_rebind_lookup.has(key):
		return
	height_texture_rebind_lookup[key] = true
	height_texture_rebind_queue.append(key)

func _process_height_texture_rebinds(budget: int) -> void:
	var remaining_budget := budget
	while remaining_budget > 0 and not height_texture_rebind_queue.is_empty():
		var key: Vector2i = height_texture_rebind_queue.pop_front()
		height_texture_rebind_lookup.erase(key)
		_bind_patch_height_texture(key)
		remaining_budget -= 1

func _bind_patch_height_texture(key: Vector2i) -> void:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return
	var material: ShaderMaterial = patch["material"]
	var next_texture := _terrain_height_texture(key)
	if patch["height_texture"] == next_texture:
		return
	patch["height_texture"] = next_texture
	material.set_shader_parameter("heightmap", next_texture)
	_water_debug_height_rebinds += 1

func _queue_terrain_patch_binding(key: Vector2i) -> void:
	if terrain_patch_binding_lookup.has(key):
		return
	terrain_patch_binding_lookup[key] = true
	terrain_patch_binding_queue.append(key)

func _process_terrain_patch_binding_queue(budget: int) -> void:
	var remaining_budget := budget
	while remaining_budget > 0 and not terrain_patch_binding_queue.is_empty():
		var key: Vector2i = terrain_patch_binding_queue.pop_front()
		terrain_patch_binding_lookup.erase(key)
		_refresh_terrain_patch_binding(key)
		remaining_budget -= 1

func _refresh_terrain_patch_binding(key: Vector2i) -> void:
	if terrain_node == null:
		return
	if terrain_node.has_method("refresh_water_patch_binding"):
		terrain_node.refresh_water_patch_binding(key)
	elif terrain_node.has_method("refresh_water_patch_bindings"):
		terrain_node.refresh_water_patch_bindings()

func _ensure_water_border_visual() -> void:
	if water_border_instance == null:
		water_border_instance = MeshInstance3D.new()
		water_border_instance.name = "WaterBorderCurtain"
		SceneLightingConfig.apply_shadow_policy(
			water_border_instance,
			SceneLightingConfig.SHADOW_RECEIVER_ONLY,
			"water"
		)
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
		"water resident=%d creates=%d removes=%d uploads=%d rebinds=%d border_rebuilds=%d residency_changes=%d lods=%s avg_ms=%.3f max_ms=%.3f patch_sync_ms=%.3f upload_ms=%.3f border_ms=%.3f rebind_ms=%.3f force_lod1=%s visual=%d"
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
			str(_terrain_force_lod1),
			_water_visual_debug_mode,
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
