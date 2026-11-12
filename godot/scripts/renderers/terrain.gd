# SPDX-License-Identifier: GPL-2.0-only

## Terrain patch renderer — uploads chunk-local visual terrain patches and world-edge terrain skirts.
##
## Rust methods called: get_terrain_patch_layout(), get_dirty_terrain_patches(),
##   get_dirty_terrain_patch_payload_states(),
##   request_terrain_patch_payloads(), poll_ready_terrain_patch_payloads(),
##   get_terrain_border_loop(), get_heightmap_size(), get_terrain_world_size(),
##   is_terrain_dirty(), acknowledge_terrain_patches(), sculpt_terrain(), intersect_terrain(),
##   get_pollution_image_data(), get_noise_image_data(), get_current_day(),
##   get_desirability_image_data(), get_coal_pit_overlay_data(),
##   get_coal_pit_overlay_size(), get_coal_pit_overlay_world_bounds(),
##   get_coal_pit_overlay_revision(), get_agriculture_field_overlay_data(),
##   get_agriculture_field_overlay_size(),
##   get_agriculture_field_overlay_world_bounds(),
##   get_agriculture_field_overlay_revision()
extends Node3D

# Temporary render clients must restore their saved resources before a patch is changed/recycled.
signal patch_render_will_change(key: Vector2i)
signal patches_will_reset

const TERRAIN_SHADER := preload("res://assets/materials/terrain.gdshader")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const TreeSpecies := preload("res://scripts/renderers/tree_species.gd")
const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const TerrainWorldNoise := preload("res://scripts/renderers/terrain_world_noise.gd")
const RenderDebug := preload("res://scripts/renderers/render_debug.gd")
const WorldMaterials := preload("res://scripts/renderers/world_materials.gd")
const TerrainGradedTiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const TERRAIN_COAL_ALBEDO_PATH := "res://assets/textures/general/coal/dark_rock_diff_2k.jpg"
const TERRAIN_GRAIN_ALBEDO_PATH := "res://assets/textures/general/grain/withered_grass_diff_2k.jpg"
const HEIGHT_SCALE := 20.0
const TERRAIN_ROCK_SLOPE_START := 0.15
const TERRAIN_ROCK_SLOPE_END := 0.34
const TERRAIN_RELIEF_START_M := 2.0
const TERRAIN_RELIEF_END_M := 16.0
const TERRAIN_SHORE_BLEND_STRENGTH := 0.15
const TERRAIN_SHORE_LOOKUP_RADIUS_TEXELS := 0.55
# The cliff test's thresholds and radii are mirrored in patch_masks.rs, whose baked cliff reach
# decides where the shader runs the test; change both together.
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
const TERRAIN_BORDER_TOPSOIL_COLOR := Color(0.17, 0.15, 0.11)
const TERRAIN_BORDER_BOTTOM_COLOR := Color(0.18, 0.17, 0.15)
const TERRAIN_BORDER_BAND_INTERVAL_M := 12.0
const TERRAIN_BORDER_BAND_STRENGTH := 0.08
const TERRAIN_BORDER_STRATA_WARP_M := 3.5
const TERRAIN_BORDER_TOPSOIL_DEPTH_M := 3.0
const TERRAIN_BORDER_TOPSOIL_STRENGTH := 0.74
const TERRAIN_BORDER_NORMAL_STRENGTH := 0.09
const TERRAIN_BORDER_CONTOUR_MINOR_COLOR := Color(0.13, 0.19, 0.16)
const TERRAIN_BORDER_CONTOUR_MAJOR_COLOR := Color(0.10, 0.16, 0.14)
const TERRAIN_BORDER_CONTOUR_MINOR_STRENGTH := 0.14
const TERRAIN_BORDER_CONTOUR_MAJOR_STRENGTH := 0.28
const TERRAIN_DEBUG_LOG_INTERVAL_S := 0.5
const PATCH_EXTRA_CULL_MARGIN_M := 4096.0
const PATCH_CULL_PAD_M := 64.0
# The terrain's least resident half-side (the streaming core's `CULL_FAR_M`): the scene's distance
# fade reads it through get_visibility_cull_far_m.
const PATCH_RESIDENCY_CULL_FAR_M := 8000.0
const PATCH_RESOURCE_POOL_PREWARM_COUNT := 64
const PATCH_RESOURCE_POOL_MAX := 96
const PATCH_PAYLOAD_POLL_BUDGET_PER_FRAME := 64
const PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME := 128
const PATCH_WATER_TEXTURE_SYNC_BUDGET_MS := 1.0
const FIELD_OVERLAY_TEXTURE_TILE_M := 12.0
const FIELD_OVERLAY_STRENGTH := 0.84
const FIELD_OVERLAY_GRAIN_TINT := Color(0.98, 1.02, 0.90, 1.0)
const FIELD_OVERLAY_GRAIN_BRIGHTNESS := 1.06
const FIELD_OVERLAY_GRAIN_MIX_STRENGTH := 0.38
const FIELD_OVERLAY_GRAIN_MACRO_STRENGTH := 0.12
const FIELD_OVERLAY_FALLBACK_COLOR := Color(0.86, 0.70, 0.34, 1.0)

@onready var simulation_node = $"../SimulationNode"
@onready var water_node = $"../Water"
# The streaming core (`RENDER-32-F`): the vegetation's residency in Rust is told of every resident
# patch, residency revision and surface commit as it happens, instead of asking each frame.
@onready var _stream: Node = get_node_or_null("../StreamNode")

var overlay_mode: int = 0
## Graded terrain tiles drawn as cells over the plain base-terrain patches.
var graded_tiles: TerrainGradedTiles = null
# The resident patches the graded tiles read (key -> {node, material, rect, step, generation,
# aabb}), kept as patches change rather than rebuilt per frame, with the keys changed since the
# last frame (the graded tiles follow those patches only), and the keys whose material
# parameters the graded cells copy may have changed since. An idle frame hands them over in O(1).
var _graded_resident: Dictionary = {}
var _graded_changed: Dictionary = {}
var _graded_materials_changed: Dictionary = {}
var terrain_world_size: Vector2 = Vector2.ZERO
var terrain_cell_m: float = 1.0
var patch_cols: int = 0
var patch_rows: int = 0
var patch_interval_cells: int = 1
# Baked relief and cliff-reach masks (patch_masks.rs), one array layer per patch slot. Every patch
# material shares a page: one texture per patch material instead added irregular 1-5 ms frames on
# Metal, although the shading work was the same.
const MASK_LAYERS_PER_PAGE := 1024
var _mask_pages: Array[Texture2DArray] = []
var _mask_layer_size := Vector2i.ZERO
var patch_span_m: float = 1.0
var overlay_texture: ImageTexture
var overlay_image: Image
var coal_pit_texture: ImageTexture
var coal_pit_image: Image
var coal_pit_overlay_world_bounds: Vector4 = Vector4.ZERO
var field_overlay_texture: ImageTexture
var field_overlay_image: Image
var field_overlay_world_bounds: Vector4 = Vector4.ZERO
var empty_water_texture: ImageTexture
var grass_albedo_texture: Texture2D
var grass_height_texture: Texture2D
var coal_albedo_texture: Texture2D
var grain_albedo_texture: Texture2D
var patches: Dictionary = {}
var resident_patch_lookup: Dictionary = {}
var patch_payload_requested: Dictionary = {}
var patch_payload_requested_generation: Dictionary = {}
# Patch key -> [render step mm, reason] of a payload request Rust refused by contract: a bug,
# logged once and never asked again (a refusal retried every frame hid TERRAIN-07-D's render
# defect). Tests assert there is none (get_pending_render_work_counts()["refused"]).
var patch_payload_refused: Dictionary = {}
var patch_payload_ready: Dictionary = {}
var patch_payload_road_generation: int = -1
# Keys whose payload was requested for a prewarm build (a subset of patch_payload_requested): a
# settled view does not wait for them.
var _prewarm_requests: Dictionary = {}
# The perf channel's split of this frame's patch builds (_create_patch), microseconds: builds,
# total, resources, height texture, mesh and node, masks, material parameters, land cover.
var _build_us := PackedFloat64Array([0, 0, 0, 0, 0, 0, 0, 0, 0, 0])
var _build_perf := false
# The dirty-patch commit's parts while the perf channel is on (`RENDER-40`): its prepare (stage)
# and commit milliseconds and the patches it committed, since the last perf record.
var _commit_us := PackedFloat64Array([0, 0, 0])
var pending_terrain_ack_states: PackedInt64Array = PackedInt64Array()
var dirty_patch_payload_generations: Dictionary = {}
var patch_mesh_cache: Dictionary = {}
var patch_resource_pool: Array[Dictionary] = []
# Count of payload commits over all patches. See get_surface_commit_revision.
var _surface_commit_revision := 0
# Land cover epoch at which every built patch was last found current, or -1 for none.
var _land_cover_clean_epoch := -1
var water_texture_sync_queue: Array[Vector2i] = []
var water_texture_sync_lookup: Dictionary = {}
var cached_overlay_mode: int = -1
var cached_overlay_day: int = -1
var cached_coal_pit_overlay_revision: int = -1
var cached_field_overlay_revision: int = -1
var coal_pit_overlay_dirty: bool = true
var field_overlay_dirty: bool = true
var border_loop_positions: PackedVector3Array = PackedVector3Array()
var border_revision: int = 0
var border_skirt_instance: MeshInstance3D
var border_bottom_cap_instance: MeshInstance3D
var border_skirt_material: ShaderMaterial
var border_bottom_cap_material: StandardMaterial3D
# Channel switches are read live so the debug console takes effect on the next frame.
var _terrain_debug_enabled: bool:
	get:
		return MetrumDebug.is_enabled("terrain")
var _road_debug_enabled: bool:
	get:
		return MetrumDebug.is_enabled("road")
var _terrain_debug_verbose: bool = false
# Visual debug residency modes, cached because residency and LOD selection read them per patch.
# The debug console changes them through `_on_visual_mode_changed`.
var _terrain_force_full_world: bool = false
var _terrain_force_lod1: bool = false
var _terrain_resource_pool_hit_count: int = 0
var _terrain_resource_pool_miss_count: int = 0
var _terrain_resource_pool_release_count: int = 0
var _terrain_resource_pool_prewarm_count: int = 0
var _terrain_debug_elapsed_s: float = 0.0
var _terrain_debug_frames: int = 0
var _terrain_debug_frame_ms_total: float = 0.0
var _terrain_debug_frame_ms_max: float = 0.0
var _terrain_debug_upload_ms_total: float = 0.0
var _terrain_debug_border_ms_total: float = 0.0
var _terrain_debug_water_sync_ms_total: float = 0.0
var _terrain_debug_patch_creates: int = 0
var _terrain_debug_patch_removes: int = 0
var _terrain_debug_patch_uploads: int = 0
var _terrain_debug_dirty_batches: int = 0
var _terrain_debug_dirty_patch_total: int = 0

# Per-world bake of the position-only ground noise that terrain.gdshader reads.
var _world_noise := TerrainWorldNoise.new()

# The graded cells are the stream backend's `RenderingServer` instances, not nodes under this
# one: they follow its visibility here (`RENDER-32-K`).
func _notification(what: int) -> void:
	if what == NOTIFICATION_VISIBILITY_CHANGED and graded_tiles != null:
		graded_tiles.set_drawn(is_visible_in_tree())

func _ready() -> void:
	add_child(_world_noise)
	var console := get_node_or_null("/root/DebugConsole")
	if console != null:
		console.visual_mode_changed.connect(_on_visual_mode_changed)
	rebuild_from_simulation_state()

func _exit_tree() -> void:
	if graded_tiles != null:
		graded_tiles.dispose()
		graded_tiles = null

func rebuild_from_simulation_state() -> void:
	terrain_world_size = simulation_node.get_terrain_world_size()
	_world_noise.bake(terrain_world_size)
	var patch_layout: Dictionary = simulation_node.get_terrain_patch_layout()
	patch_cols = int(patch_layout.get("patch_cols", 0))
	patch_rows = int(patch_layout.get("patch_rows", 0))
	patch_interval_cells = max(1, int(patch_layout.get("patch_interval_cells", 1)))
	_rebuild_mask_pages(int(patch_layout.get("patch_border_texels", 0)))
	terrain_cell_m = float(patch_layout.get("terrain_cell_m", 1.0))
	patch_span_m = terrain_cell_m * float(patch_interval_cells)
	_stream.set_terrain_patch_span(patch_span_m)
	_terrain_debug_verbose = MetrumDebug.startup_mode("terrain_verbose")
	_reset_terrain_debug_counters()
	_clear_patches()
	# The streaming core decides residency, levels of detail and prewarm from here on and calls
	# the stream_* units below (`RENDER-32-M`). The water renderer registers itself as it rebuilds
	# (`reset_water_patches`), after this.
	_stream.reset_terrain_patches(
		self, null, patch_cols, patch_rows, patch_interval_cells, terrain_cell_m, terrain_world_size
	)
	_push_debug_modes()
	if graded_tiles != null:
		graded_tiles.dispose()
		graded_tiles = null
	_prewarm_regular_terrain_mesh_variants()
	_ensure_overlay_texture()
	_ensure_coal_pit_texture()
	_ensure_field_overlay_texture()
	_ensure_empty_water_texture()
	_ensure_grass_textures()
	_ensure_border_visuals()
	_prewarm_terrain_patch_resource_pool()
	graded_tiles = TerrainGradedTiles.new()
	graded_tiles.setup(simulation_node, terrain_cell_m, terrain_world_size, patch_span_m)
	mark_overlay_dirty()
	_refresh_overlay_texture(int(simulation_node.get_current_day()))
	var coal_pit_updated := _update_coal_pit_texture()
	if coal_pit_updated:
		_apply_coal_pit_texture()
	var field_updated := _update_field_overlay_texture()
	if field_updated:
		_apply_field_overlay_texture()
	_rebuild_border_skirt()
	_queue_all_water_patch_texture_syncs()
	_process_water_patch_texture_sync_queue(PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME)
	cached_coal_pit_overlay_revision = (
		int(simulation_node.get_coal_pit_overlay_revision()) if coal_pit_updated else -1
	)
	cached_field_overlay_revision = (
		int(simulation_node.get_agriculture_field_overlay_revision()) if field_updated else -1
	)
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
				MetrumDebug.visual_mode_index("terrain_visual"),
			]
		)

# The residency debug modes go to the streaming core: the square becomes the world, or every
# step 1; the next frame loads or drops patches through the frame budget and checks every
# resident patch's step.
func _push_debug_modes() -> void:
	_terrain_force_full_world = MetrumDebug.visual_mode_index("terrain_force_full_world") != 0
	_terrain_force_lod1 = MetrumDebug.visual_mode_index("terrain_force_lod1") != 0
	_stream.set_terrain_modes(_terrain_force_full_world, _terrain_force_lod1)

func _on_visual_mode_changed(mode: String) -> void:
	if mode != "terrain_force_full_world" and mode != "terrain_force_lod1":
		return
	_push_debug_modes()
	if _terrain_debug_enabled:
		_terrain_debug_log(
			"residency debug force_full_world=%s force_lod1=%s"
			% [str(_terrain_force_full_world), str(_terrain_force_lod1)]
		)

func _process(delta: float) -> void:
	var frame_start_us := Time.get_ticks_usec()
	var perf_enabled := PerfDebug.is_enabled()
	_build_perf = perf_enabled
	var network_refresh_pending: bool = simulation_node.is_network_dirty()
	var payload_poll_start_us := Time.get_ticks_usec()
	var payload_poll_count := _poll_ready_terrain_patch_payloads(PATCH_PAYLOAD_POLL_BUDGET_PER_FRAME)
	var payload_poll_elapsed_ms := float(Time.get_ticks_usec() - payload_poll_start_us) / 1000.0
	network_refresh_pending = network_refresh_pending or simulation_node.is_network_dirty()
	var upload_elapsed_ms := 0.0
	var border_elapsed_ms := 0.0
	var water_sync_elapsed_ms := 0.0
	var water_sync_perf_stats := {}
	var dirty_patch_count := 0
	if not network_refresh_pending and simulation_node.is_terrain_dirty():
		var dirty_start_us := Time.get_ticks_usec()
		var dirty_pairs: PackedInt32Array = simulation_node.get_dirty_terrain_patches()
		dirty_patch_count = int(dirty_pairs.size() / 2)
		_terrain_debug_dirty_batches += 1
		_terrain_debug_dirty_patch_total += int(dirty_pairs.size() / 2)
		_try_commit_standalone_terrain_visual_update()
		upload_elapsed_ms = float(Time.get_ticks_usec() - dirty_start_us) / 1000.0
	if not simulation_node.is_terrain_dirty():
		_prune_patch_payload_cache()

	if not network_refresh_pending and not simulation_node.is_terrain_dirty():
		_sync_land_cover()

	var water_sync_start_us := Time.get_ticks_usec()
	water_sync_perf_stats = _process_water_patch_texture_sync_queue(
		PATCH_WATER_TEXTURE_SYNC_BUDGET_PER_FRAME,
		perf_enabled
	)
	water_sync_elapsed_ms = float(Time.get_ticks_usec() - water_sync_start_us) / 1000.0

	_refresh_overlay_texture(int(simulation_node.get_current_day()))
	var coal_pit_revision := int(simulation_node.get_coal_pit_overlay_revision())
	if (
		coal_pit_texture == null
		or coal_pit_overlay_dirty
		or coal_pit_revision != cached_coal_pit_overlay_revision
	):
		if _update_coal_pit_texture():
			_apply_coal_pit_texture()
			cached_coal_pit_overlay_revision = coal_pit_revision
	var field_revision := int(simulation_node.get_agriculture_field_overlay_revision())
	if (
		field_overlay_texture == null
		or field_overlay_dirty
		or field_revision != cached_field_overlay_revision
	):
		if _update_field_overlay_texture():
			_apply_field_overlay_texture()
			cached_field_overlay_revision = field_revision

	if graded_tiles != null:
		_process_graded_tiles()

	var input_manager = get_node_or_null("../InputManager")
	if input_manager and input_manager.current_tool == input_manager.Tool.SCULPT:
		if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) or Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT):
			_sculpt_at_mouse(delta)

	if _terrain_debug_enabled:
		var frame_elapsed_ms := float(Time.get_ticks_usec() - frame_start_us) / 1000.0
		_record_terrain_debug_frame(
			delta,
			frame_elapsed_ms,
			upload_elapsed_ms,
			border_elapsed_ms,
			water_sync_elapsed_ms
		)

	if perf_enabled:
		var perf_details := {
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
			# The upload's parts (`RENDER-40`): dirty patches asked, the commit's prepare and
			# commit, and the patches committed.
			"upload_dirty_patches": dirty_patch_count,
			"upload_prepare_ms": _commit_us[0] / 1000.0, "upload_commit_ms": _commit_us[1] / 1000.0,
			"upload_committed_patches": int(_commit_us[2]),
			"border": border_elapsed_ms,
			"water_sync": water_sync_elapsed_ms,
		}
		# The patch builds the streaming core ran since the last record (its units run after
		# this script's frame).
		var build_names := ["build_count", "build_ms", "build_resources_ms", "build_height_ms",
			"build_node_ms", "build_masks_ms", "build_params_ms", "build_land_cover_ms",
			"build_land_cover_query_ms", "build_land_cover_prepared"]
		for part in build_names.size():
			var counted := part == 0 or part == 9
			perf_details[build_names[part]] = _build_us[part] if counted else _build_us[part] / 1000.0
			_build_us[part] = 0.0
		for key_variant in water_sync_perf_stats.keys():
			var perf_key := str(key_variant)
			perf_details[perf_key] = water_sync_perf_stats[key_variant]
		# The frame's own time ends here: the diagnostic count below walks every cell set.
		var elapsed_ms := float(Time.get_ticks_usec() - frame_start_us) / 1000.0
		if graded_tiles != null:
			var graded_parts: Dictionary = graded_tiles.counters["frame_parts"]
			graded_tiles.count_submitted_cells(graded_parts)
			for part: String in graded_parts.keys():
				perf_details["graded_" + part] = float(graded_parts[part])
		PerfDebug.record("terrain", elapsed_ms, perf_details)
		_commit_us.fill(0.0)
		_reset_terrain_resource_pool_perf_counters()

# Hands the resident patches to the graded tile renderer with the keys changed since last frame,
# and the camera with its height above the ground under it, from which the tiles' levels take
# their distance.
func _process_graded_tiles() -> void:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	var eye := camera.global_position
	# The view's frustum: an unready patch in it counts against the residency margin (T53 (a)).
	graded_tiles.view_planes = camera.get_frustum()
	eye.y -= float(simulation_node.get_world_surface_height(Vector2(eye.x, eye.z)))
	graded_tiles.residency_settling = has_pending_render_work(false)
	graded_tiles.process(eye, _graded_resident, _graded_changed, _graded_materials_changed)
	_graded_changed.clear()
	_graded_materials_changed.clear()

# A material parameter of patch `key` the graded cells copy may have changed (`key` null for a
# patch dictionary without one: nothing to sync).
func _graded_material_changed(key: Variant) -> void:
	if graded_tiles != null and key is Vector2i:
		_graded_materials_changed[key] = true

# Rewrites the graded tiles' entry of patch `key` (its node, material, rectangle, LOD step and
# payload generation) after its residency, payload or LOD step changed; a patch that is not
# resident or has no payload has none.
func _graded_patch_changed(key: Vector2i) -> void:
	if graded_tiles == null:
		return
	_graded_changed[key] = true
	var patch: Dictionary = patches.get(key, {})
	var data: Dictionary = patch.get("last_patch_data", {}) as Dictionary
	if not resident_patch_lookup.has(key) or patch.is_empty() or data.is_empty():
		if _graded_resident.erase(key):
			graded_tiles.patch_mutated(key, {})
		return
	var size := Vector2(float(data["world_size_x"]), float(data["world_size_z"]))
	_graded_resident[key] = {
		"node": patch["node"],
		"material": patch["material"],
		"rect": Rect2(Vector2(float(data["world_origin_x"]), float(data["world_origin_z"])), size),
		"step": int(patch.get("lod_step", 1)),
		"generation": get_patch_surface_generation(key),
		"aabb": patch_cull_aabb(size),
	}
	graded_tiles.patch_mutated(key, _graded_resident[key])

func get_resident_patch_keys() -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	for key_variant in resident_patch_lookup.keys():
		var key: Vector2i = key_variant
		keys.append(key)
	return keys

# The generation the patch's payload now carries (get_patch_surface_generation), -1 once gone.
func _push_surface_generation(key: Vector2i) -> void:
	_stream.set_terrain_surface_generation(key, get_patch_surface_generation(key))

## The script's queues and the streaming core's patch counts (`terrain_missing`,
## `terrain_leaving`, `terrain_lod_queue`, `terrain_resident` and the water's).
func get_pending_render_work_counts() -> Dictionary:
	var counts := {
		"payload_requested": patch_payload_requested.size(),
		"prewarm_requested": _prewarm_requests.size(),
		"payload_ready": patch_payload_ready.size(),
		"pending_ack": int(pending_terrain_ack_states.size() / 3),
		"water_texture_sync": water_texture_sync_queue.size(),
		"refused": patch_payload_refused.size(),
	}
	counts.merge(_stream.get_terrain_patch_counts())
	return counts

func get_blocked_dirty_patch_failures() -> Array[Dictionary]:
	return []

func has_blocked_dirty_patch_failure() -> bool:
	return false

## Payloads requested (prewarm's only with the background cache), an acknowledgement or water
## texture sync waiting, or the streaming core's patches missing, leaving or queued for a level
## check; with the background cache also payloads held for a later build.
func has_pending_render_work(include_background_cache: bool = true) -> bool:
	return (
		patch_payload_requested.size() > _prewarm_requests.size()
		or not pending_terrain_ack_states.is_empty()
		or not water_texture_sync_queue.is_empty()
		or _stream.has_terrain_pending_work()
		or (
			include_background_cache
			and (not patch_payload_ready.is_empty() or not _prewarm_requests.is_empty())
		)
	)

## Whether the renderer has settled: no pending patch work (has_pending_render_work without the
## background cache) and no graded tile work (requests, lists, counts, deferred results, an edit
## batch, cell set builds, a resident patch drawing nothing). A capture of what the player sees
## waits for it, and a world load hands the game over on it (input_manager.gd): until then a
## patch whose graded cells are not ready draws nothing (T53 (a)). O(1).
func is_render_settled() -> bool:
	return not has_pending_render_work(false) and (graded_tiles == null or not graded_tiles.has_pending_work())

## The world's patches within `radius_m` of the camera's ground point that do not yet draw what
## they draw once settled (graded_tiles.unready_near; none before a world is set up, as
## is_render_settled). O(patches within the radius).
func unready_patches_near_camera(radius_m: float) -> int:
	var camera := get_viewport().get_camera_3d()
	if camera == null or graded_tiles == null:
		return 0
	return graded_tiles.unready_near(Vector2(camera.global_position.x, camera.global_position.z), radius_m)

## While a world load waits for is_render_settled() (the load flow holds the game), the graded
## renderer's work and the streaming core's patch units take the whole frame instead of the
## streaming budget's spare time; the frame time does not matter then. The load flow turns it off
## as it hands the game over.
func set_load_settling(settling: bool) -> void:
	_stream.set_terrain_load_settling(settling)
	if graded_tiles != null:
		graded_tiles.unbudgeted = settling

func get_visibility_cull_far_m(camera: Camera3D) -> float:
	if camera == null:
		return 0.0
	return minf(camera.far, _terrain_patch_cull_far_m(camera))

func get_render_patch_span_m() -> float:
	return patch_span_m

## Advances whenever any patch commits a payload, so a consumer of get_patch_surface_generation
## can skip its per-patch comparison while this is unchanged. Never reset: an old value can
## only cause one redundant comparison, never a missed one.
func get_surface_commit_revision() -> int:
	return _surface_commit_revision

func get_patch_surface_generation(key: Vector2i) -> int:
	# The generation stamped on the payload this patch last committed. It advances only
	# when a road/terrain edit dirties this patch, so consumers can detect a stale
	# derived product per patch instead of rebuilding every patch on any edit.
	if not patches.has(key):
		return -1
	var last_patch_data: Variant = patches[key].get("last_patch_data", null)
	if not (last_patch_data is Dictionary):
		return -1
	return int((last_patch_data as Dictionary).get("surface_generation", -1))

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
	var prepared := prepare_terrain_visual_update()
	if prepared.is_empty():
		return false
	return commit_prepared_terrain_visual_update(prepared)

func _try_commit_standalone_terrain_visual_update() -> bool:
	if simulation_node.is_network_dirty():
		return false
	var network_generation := _terrain_network_render_generation()
	var prepare_start_us := Time.get_ticks_usec() if _build_perf else 0
	var prepared := prepare_terrain_visual_update(false)
	if _build_perf:
		_commit_us[0] += Time.get_ticks_usec() - prepare_start_us
	if prepared.is_empty():
		return false
	# A network edit may arrive while detached resources are being built. Leave those resources
	# inactive so NetworkRenderer can publish the new terrain/road generation as one pair.
	if (
		simulation_node.is_network_dirty()
		or _terrain_network_render_generation() != network_generation
	):
		return false
	if not _build_perf:
		return commit_prepared_terrain_visual_update(prepared)
	var commit_start_us := Time.get_ticks_usec()
	var committed := commit_prepared_terrain_visual_update(prepared)
	_commit_us[1] += Time.get_ticks_usec() - commit_start_us
	_commit_us[2] += (prepared["dirty_keys"] as Array).size() if committed else 0
	return committed

func _terrain_network_render_generation() -> int:
	if simulation_node.has_method("get_network_render_generation"):
		return int(simulation_node.get_network_render_generation())
	return int(simulation_node.get_road_tool_surface_generation())

func prepare_terrain_visual_update(poll_ready: bool = true) -> Dictionary:
	if not _retry_pending_terrain_ack():
		return {}
	var dirty_states: PackedInt64Array = simulation_node.get_dirty_terrain_patch_payload_states()
	var dirty_keys := _dirty_patch_payload_keys_from_states(dirty_states)
	if dirty_keys.is_empty():
		if simulation_node.is_terrain_dirty():
			return {}
		return {
			"dirty_states": dirty_states,
			"dirty_keys": dirty_keys,
			"patch_stages": {},
			"border_stage": _stage_terrain_border_update(),
		}

	if poll_ready:
		_poll_ready_terrain_patch_payloads(PATCH_PAYLOAD_POLL_BUDGET_PER_FRAME)
	if not _dirty_patch_payloads_ready_for_atomic_upload(dirty_keys):
		return {}
	if not _dirty_patch_payloads_renderable_for_atomic_upload(dirty_keys):
		return {}
	var staged_result := _stage_dirty_patch_payloads_for_atomic_commit(dirty_keys)
	if not bool(staged_result.get("valid", false)):
		return {}
	var border_stage: Dictionary = {}
	if _dirty_patch_keys_touch_border(dirty_keys):
		border_stage = _stage_terrain_border_update()
		if not bool(border_stage.get("valid", false)):
			return {}
	return {
		"dirty_states": dirty_states,
		"dirty_keys": dirty_keys,
		"patch_stages": staged_result["stages"],
		"border_stage": border_stage,
	}

func commit_prepared_terrain_visual_update(prepared: Dictionary) -> bool:
	if (
		not prepared.has("dirty_states")
		or not prepared.has("dirty_keys")
		or not prepared.has("patch_stages")
		or not prepared.has("border_stage")
	):
		return false
	var dirty_states: PackedInt64Array = prepared["dirty_states"] as PackedInt64Array
	var dirty_keys: Array = prepared["dirty_keys"] as Array
	var patch_stages: Dictionary = prepared["patch_stages"] as Dictionary
	var border_stage: Dictionary = prepared["border_stage"] as Dictionary
	# Verify every detached stage still targets the same resident node before the first scene swap.
	for key in dirty_keys:
		if patches.has(key) and (
			not patch_stages.has(key)
			or typeof(patch_stages[key]) != TYPE_DICTIONARY
			or not _terrain_patch_stage_matches_target(key, patch_stages[key] as Dictionary)
		):
			return false
	if not border_stage.is_empty() and not bool(border_stage.get("valid", false)):
		return false
	for key in dirty_keys:
		if not patches.has(key):
			continue
		patch_payload_ready.erase(key)
		patch_payload_requested.erase(key)
		patch_payload_requested_generation.erase(key)
		_commit_staged_patch_data(key, patch_stages[key] as Dictionary)
		_queue_water_patch_texture_sync(key)
	if not border_stage.is_empty():
		_commit_terrain_border_stage(border_stage)
	# Scene resources now represent this complete terrain batch. A failed exact acknowledgement
	# means a newer patch generation arrived; it remains dirty without invalidating this visual pair.
	_acknowledge_terrain_batch(dirty_states)
	return true

func _terrain_patch_cull_far_m(camera: Camera3D) -> float:
	var camera_height := absf(camera.global_position.y)
	var height_scaled_far := camera_height * 4.0
	return maxf(PATCH_RESIDENCY_CULL_FAR_M, height_scaled_far)

func _create_patch(key: Vector2i, initial_lod_step: int, allow_async: bool = true) -> void:
	_graded_material_changed(key)
	if patches.has(key):
		return
	var patch_data: Dictionary = _terrain_patch_data_for_key(key, false, allow_async)
	if patch_data.is_empty():
		return
	var build_start_us := Time.get_ticks_usec() if _build_perf else 0
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
	var resources_us := Time.get_ticks_usec() if _build_perf else 0
	var height_image: Image = patch_resources["height_image"] as Image
	var height_texture: ImageTexture = _upload_terrain_patch_height_texture(
		patch_resources,
		texture_width,
		texture_height,
		_terrain_patch_height_bytes(patch_data)
	)
	var height_us := Time.get_ticks_usec() if _build_perf else 0
	var patch_mesh: Mesh
	var sample_step_m := world_size_x / float(max(1, sample_width - 1))
	var initial_subdivision_factor := _mesh_subdivision_factor_for_patch(key, sample_step_m)
	patch_mesh = _terrain_patch_mesh_from_data(patch_data, initial_lod_step, initial_subdivision_factor)

	var patch_node: MeshInstance3D = patch_resources["node"] as MeshInstance3D
	patch_node.name = "TerrainPatch_%d_%d" % [key.x, key.y]
	# The heightmap moves vertices in the shader, so the mesh bounds do not hold the surface.
	# A margin on every axis kept patches 4 km behind the camera drawn; only height is unknown.
	patch_node.extra_cull_margin = 0.0
	patch_node.custom_aabb = patch_cull_aabb(Vector2(world_size_x, world_size_z))
	patch_node.mesh = patch_mesh
	patch_node.visible = false
	patch_node.position = Vector3(
		world_origin_x + world_size_x * 0.5,
		0.0,
		world_origin_z + world_size_z * 0.5
	)
	var material: ShaderMaterial = patch_resources["material"] as ShaderMaterial
	material.shader = TERRAIN_SHADER
	material.set_shader_parameter("heightmap", height_texture)
	var node_us := Time.get_ticks_usec() if _build_perf else 0
	_apply_patch_masks(key, material, _terrain_patch_mask_image(patch_data), texture_width, texture_height)
	var masks_us := Time.get_ticks_usec() if _build_perf else 0
	material.set_shader_parameter("overlay_texture", overlay_texture)
	material.set_shader_parameter("coal_pit_texture", coal_pit_texture)
	material.set_shader_parameter("coal_pit_overlay_world_bounds", coal_pit_overlay_world_bounds)
	material.set_shader_parameter("field_overlay_texture", field_overlay_texture)
	material.set_shader_parameter("field_overlay_world_bounds", field_overlay_world_bounds)
	material.set_shader_parameter("watermap", empty_water_texture)
	material.set_shader_parameter("terrain_grass_albedo", grass_albedo_texture)
	material.set_shader_parameter("terrain_grass_height", grass_height_texture)
	material.set_shader_parameter("terrain_coal_albedo", coal_albedo_texture)
	material.set_shader_parameter("terrain_grain_albedo", grain_albedo_texture)
	material.set_shader_parameter("overlay_mode", overlay_mode)
	material.set_shader_parameter("world_size", terrain_world_size)
	material.set_shader_parameter("terrain_debug_patch_key", Vector2(key.x, key.y))
	material.set_shader_parameter("terrain_debug_lod_step", float(initial_lod_step))
	material.set_shader_parameter(
		"scene_shadow_max_distance_m", SceneLightingConfig.shadow_max_distance_m()
	)
	material.set_shader_parameter(
		"scene_shadow_split_distances_m",
		SceneLightingConfig.shadow_split_distances()
	)
	SceneLightingConfig.apply_canopy_floor_shading(material)
	material.set_shader_parameter("heightmap_texture_size", Vector2(texture_width, texture_height))
	material.set_shader_parameter("inner_sample_offset_texels", Vector2(inner_offset_x, inner_offset_z))
	material.set_shader_parameter("inner_sample_size_texels", Vector2(sample_width, sample_height))
	material.set_shader_parameter("watermap_texture_size", Vector2(2, 2))
	material.set_shader_parameter("watermap_inner_sample_offset_texels", Vector2.ZERO)
	material.set_shader_parameter("watermap_inner_sample_size_texels", Vector2(2, 2))
	material.set_shader_parameter("patch_world_size_m", Vector2(world_size_x, world_size_z))
	material.set_shader_parameter("terrain_cell_m", terrain_cell_m)
	patch_node.material_override = material
	_ensure_patch_node_parent(patch_node)
	_terrain_debug_patch_creates += 1

	patches[key] = {
		"key": key,
		"node": patch_node,
		"material": material,
		"height_image": height_image,
		"height_texture": height_texture,
		"spare_height_image": patch_resources.get("spare_height_image", null),
		"spare_height_texture": patch_resources.get("spare_height_texture", null),
		"spare_height_texture_width": int(
			patch_resources.get("spare_height_texture_width", 0)
		),
		"spare_height_texture_height": int(
			patch_resources.get("spare_height_texture_height", 0)
		),
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
		"last_patch_data": patch_data,
	}
	_surface_commit_revision += 1

	var params_us := Time.get_ticks_usec() if _build_perf else 0
	patches[key]["land_cover"] = patch_resources.get("land_cover", {})
	patches[key]["spare_land_cover"] = patch_resources.get("spare_land_cover", {})
	_commit_patch_land_cover(
		patches[key],
		_stage_patch_land_cover(key, patches[key], true, patch_data.get("land_cover", {}) as Dictionary)
	)
	if _build_perf:
		var end_us := Time.get_ticks_usec()
		var marks := [build_start_us, resources_us, height_us, node_us, masks_us, params_us, end_us]
		_build_us[0] += 1.0
		_build_us[1] += float(end_us - build_start_us)
		for part in 6:
			_build_us[part + 2] += float(marks[part + 1] - marks[part])

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
			MetrumDebug.log(
				"road",
				"terrain_upload key=(%d,%d) missing_patch_data=true fetch_ms=%.3f total_ms=%.3f"
				% [
					key.x,
					key.y,
					fetch_ms,
					float(Time.get_ticks_usec() - total_start_us) / 1000.0,
				]
			)
		return false
	var stage := _stage_terrain_patch_update(
		key,
		patch_data,
		int(patch_data.get("surface_generation", -1)),
		_terrain_patch_payload_render_step_mm(key)
	)
	if stage.is_empty():
		return false
	_commit_staged_patch_data(key, stage, total_start_us, fetch_ms)
	return true

func _stage_terrain_patch_update(
	key: Vector2i,
	patch_data: Dictionary,
	expected_generation: int,
	expected_render_step_mm: int
) -> Dictionary:
	if not _terrain_patch_payload_is_stageable(
		key,
		patch_data,
		expected_generation,
		expected_render_step_mm
	):
		return {}
	var patch: Dictionary = patches[key]
	var texture_width := int(patch_data["texture_width"])
	var texture_height := int(patch_data["texture_height"])
	var texture_start_us := Time.get_ticks_usec()
	var height_image: Image = patch.get("spare_height_image", null) as Image
	if height_image == null:
		height_image = Image.new()
	height_image.set_data(
		texture_width,
		texture_height,
		false,
		Image.FORMAT_RF,
		_terrain_patch_height_bytes(patch_data)
	)
	if height_image.is_empty():
		return {}
	var height_texture: ImageTexture = patch.get("spare_height_texture", null) as ImageTexture
	var spare_texture_width := int(patch.get("spare_height_texture_width", 0))
	var spare_texture_height := int(patch.get("spare_height_texture_height", 0))
	if (
		height_texture != null
		and spare_texture_width == texture_width
		and spare_texture_height == texture_height
	):
		height_texture.update(height_image)
	else:
		height_texture = ImageTexture.create_from_image(height_image)
	if height_texture == null:
		return {}
	patch["spare_height_image"] = height_image
	patch["spare_height_texture"] = height_texture
	patch["spare_height_texture_width"] = texture_width
	patch["spare_height_texture_height"] = texture_height
	var mask_image := _terrain_patch_mask_image(patch_data)
	var texture_ms := float(Time.get_ticks_usec() - texture_start_us) / 1000.0

	var sample_width := int(patch_data["sample_width"])
	var world_size_x := float(patch_data["world_size_x"])
	var world_size_z := float(patch_data["world_size_z"])
	var sample_step_m := world_size_x / float(sample_width - 1)
	var center_x := float(patch_data["world_origin_x"]) + world_size_x * 0.5
	var center_z := float(patch_data["world_origin_z"]) + world_size_z * 0.5
	# The streaming core owns the step: an edit keeps the installed one.
	var lod_step := int(patch.get("lod_step", 1))
	var subdivision_factor := _mesh_subdivision_factor_for_patch(key, sample_step_m)
	var mesh_start_us := Time.get_ticks_usec()
	var terrain_mesh := _terrain_patch_mesh_from_data(
		patch_data,
		lod_step,
		subdivision_factor
	)
	if terrain_mesh == null:
		return {}
	var mesh_ms := float(Time.get_ticks_usec() - mesh_start_us) / 1000.0
	return {
		"valid": true,
		"patch_node_id": (patch["node"] as MeshInstance3D).get_instance_id(),
		"patch_data": patch_data,
		"land_cover": _stage_patch_land_cover(key, patch),
		"height_image": height_image,
		"height_texture": height_texture,
		"mask_image": mask_image,
		"terrain_mesh": terrain_mesh,
		"position": Vector3(center_x, 0.0, center_z),
		"sample_width": sample_width,
		"sample_height": int(patch_data["sample_height"]),
		"texture_width": texture_width,
		"texture_height": texture_height,
		"world_size_x": world_size_x,
		"world_size_z": world_size_z,
		"sample_step_m": sample_step_m,
		"lod_step": lod_step,
		"subdivision_factor": subdivision_factor,
		"texture_ms": texture_ms,
		"mesh_ms": mesh_ms,
	}

func _commit_staged_patch_data(
	key: Vector2i,
	stage: Dictionary,
	total_start_us: int = 0,
	fetch_ms: float = 0.0
) -> void:
	_graded_material_changed(key)
	patch_render_will_change.emit(key)
	if total_start_us <= 0:
		total_start_us = Time.get_ticks_usec()
	var patch: Dictionary = patches[key]
	var patch_data: Dictionary = stage["patch_data"] as Dictionary
	var old_height_image: Image = patch["height_image"] as Image
	var old_height_texture: ImageTexture = patch["height_texture"] as ImageTexture
	var old_texture_width := int(patch.get("texture_width", 0))
	var old_texture_height := int(patch.get("texture_height", 0))
	patch["height_image"] = stage["height_image"]
	patch["height_texture"] = stage["height_texture"]
	patch["spare_height_image"] = old_height_image
	patch["spare_height_texture"] = old_height_texture
	patch["spare_height_texture_width"] = old_texture_width
	patch["spare_height_texture_height"] = old_texture_height
	patch["last_patch_data"] = patch_data
	_surface_commit_revision += 1
	_push_surface_generation(key)
	_commit_patch_land_cover(patch, stage["land_cover"])

	var material: ShaderMaterial = patch["material"] as ShaderMaterial
	material.set_shader_parameter("heightmap", stage["height_texture"])
	# The layer is written here, with the heightmap swap, so the masks never lead the heights.
	_apply_patch_masks(key, material, stage["mask_image"],
		int(stage["texture_width"]), int(stage["texture_height"]))
	material.set_shader_parameter(
		"heightmap_texture_size",
		Vector2(int(stage["texture_width"]), int(stage["texture_height"]))
	)
	material.set_shader_parameter(
		"inner_sample_offset_texels",
		Vector2(float(patch_data["inner_offset_x"]), float(patch_data["inner_offset_z"]))
	)
	material.set_shader_parameter(
		"inner_sample_size_texels",
		Vector2(int(stage["sample_width"]), int(stage["sample_height"]))
	)
	material.set_shader_parameter(
		"patch_world_size_m",
		Vector2(float(stage["world_size_x"]), float(stage["world_size_z"]))
	)
	material.set_shader_parameter("terrain_debug_lod_step", float(stage["lod_step"]))

	patch["sample_width"] = int(stage["sample_width"])
	patch["sample_height"] = int(stage["sample_height"])
	patch["texture_width"] = int(stage["texture_width"])
	patch["texture_height"] = int(stage["texture_height"])
	patch["world_size_x"] = float(stage["world_size_x"])
	patch["world_size_z"] = float(stage["world_size_z"])
	patch["sample_step_m"] = float(stage["sample_step_m"])
	patch["lod_step"] = int(stage["lod_step"])
	_graded_patch_changed(key)
	patch["subdivision_factor"] = int(stage["subdivision_factor"])

	var patch_node: MeshInstance3D = patch["node"] as MeshInstance3D
	patch_node.mesh = stage["terrain_mesh"] as Mesh
	patch_node.position = stage["position"] as Vector3
	_apply_patch_visibility_for_residency(key, patch, patch_data)
	_terrain_debug_patch_uploads += 1

	if _road_debug_enabled:
		MetrumDebug.log(
			"road",
			"terrain_upload key=(%d,%d) fetch_ms=%.3f texture_ms=%.3f mesh_ms=%.3f total_ms=%.3f"
			% [
				key.x,
				key.y,
				fetch_ms,
				float(stage["texture_ms"]),
				float(stage["mesh_ms"]),
				float(Time.get_ticks_usec() - total_start_us) / 1000.0,
			]
		)

func _apply_patch_visibility_for_residency(
	key: Vector2i,
	patch: Dictionary,
	patch_data: Dictionary
) -> void:
	var visible_for_residency := resident_patch_lookup.has(key)
	var patch_node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
	if patch_node != null:
		patch_node.visible = visible_for_residency

# The streaming core's units (`RENDER-32-M`, `stream/layers/terrain.rs`): the StreamNode decides
# residency, levels of detail and prewarm after the camera moved and calls these, one unit at a
# time under its frame budget. None of them calls the StreamNode back (it is mid-frame): what it
# must know is returned.

## Requests the payloads of `keys` (flat x, z pairs); `prewarm` marks a prewarm build's requests,
## which a settled view does not wait for. Answers, as flat x, z, news triples, the keys whose
## payload is already held or built (news 1) or was refused (2).
func stream_request(keys: PackedInt32Array, prewarm: bool) -> PackedInt32Array:
	var answers := PackedInt32Array()
	var wanted: Array[Vector2i] = []
	for index in range(0, keys.size(), 2):
		var key := Vector2i(keys[index], keys[index + 1])
		if patch_payload_ready.has(key) or patches.has(key):
			answers.append_array(PackedInt32Array([key.x, key.y, 1]))
		elif patch_payload_refused.has(key):
			answers.append_array(PackedInt32Array([key.x, key.y, 2]))
		else:
			wanted.append(key)
			if prewarm:
				_prewarm_requests[key] = true
	_request_terrain_patch_payloads(wanted, wanted.size())
	return answers

## Makes `key` resident at mesh step `step`, building its node from its payload if needed.
## Answers Vector2i(status, surface generation): status 0 resident, 1 no payload (requested
## again), 2 a network publication is pending (no activation this frame).
func stream_activate(key: Vector2i, step: int) -> Vector2i:
	if simulation_node.is_network_dirty():
		return Vector2i(2, -1)
	if resident_patch_lookup.has(key):
		return Vector2i(0, get_patch_surface_generation(key))
	var network_generation := _terrain_network_render_generation()
	if not patches.has(key):
		_create_patch(key, step)
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty():
		return Vector2i(1, -1)
	_set_patch_mesh_lod(key, step)
	# Creation can build hidden resources, but a concurrent network generation owns their first
	# visible publication together with its road chunks.
	if (
		simulation_node.is_network_dirty()
		or _terrain_network_render_generation() != network_generation
	):
		return Vector2i(2, -1)
	resident_patch_lookup[key] = true
	_graded_patch_changed(key)
	_apply_patch_visibility_for_residency(key, patch, patch.get("last_patch_data", {}) as Dictionary)
	_queue_water_patch_texture_sync(key)
	return Vector2i(0, get_patch_surface_generation(key))

## Hides resident `key`, which left the wanted square.
func stream_deactivate(key: Vector2i) -> void:
	if not resident_patch_lookup.has(key):
		return
	resident_patch_lookup.erase(key)
	if patches.has(key):
		var patch_node: MeshInstance3D = patches[key]["node"]
		patch_node.visible = false
		if graded_tiles != null:
			graded_tiles.release_patch(key, patch_node)
	_graded_patch_changed(key)

## Builds `key`'s node hidden at mesh step `step` (prewarm). Answers its surface generation, -2
## when nothing was built (its payload went).
func stream_build(key: Vector2i, step: int) -> int:
	_prewarm_requests.erase(key)
	if not patches.has(key):
		_create_patch(key, step)
	if not patches.has(key):
		return -2
	(patches[key]["node"] as MeshInstance3D).visible = false
	return get_patch_surface_generation(key)

## Gives resident `key` the mesh of step `step`.
func stream_set_lod(key: Vector2i, step: int) -> void:
	_set_patch_mesh_lod(key, step)

func _set_patch_mesh_lod(key: Vector2i, step: int) -> void:
	var patch: Dictionary = patches.get(key, {})
	if patch.is_empty() or int(patch.get("lod_step", 1)) == step:
		return
	var patch_data: Dictionary = patch.get("last_patch_data", {})
	if patch_data.is_empty():
		return
	patch_render_will_change.emit(key)
	patch["lod_step"] = step
	_graded_patch_changed(key)
	var material: ShaderMaterial = patch["material"]
	material.set_shader_parameter("terrain_debug_lod_step", float(step))
	(patch["node"] as MeshInstance3D).mesh = _terrain_patch_mesh_from_data(
		patch_data,
		step,
		int(patch.get("subdivision_factor", 1))
	)

func _remove_patch(key: Vector2i) -> void:
	if not patches.has(key):
		return
	patch_render_will_change.emit(key)
	var patch: Dictionary = patches[key]
	if graded_tiles != null:
		graded_tiles.release_patch(key, patch["node"])
	_release_terrain_patch_resources(patch)
	patches.erase(key)
	resident_patch_lookup.erase(key)
	_push_surface_generation(key)
	_graded_patch_changed(key)
	# The streaming core wants it again as a hole when it was resident.
	_stream.terrain_patch_removed(key)
	_terrain_debug_patch_removes += 1

func _clear_patches() -> void:
	patches_will_reset.emit()
	if graded_tiles != null:
		graded_tiles.clear()
	for key in patches.keys():
		var patch: Dictionary = patches[key]
		_release_terrain_patch_resources(patch)
	patches.clear()
	resident_patch_lookup.clear()
	_stream.clear_terrain_patches()
	if graded_tiles != null and not _graded_resident.is_empty():
		graded_tiles.patch_mutated(Vector2i.ZERO, {}, true)
	_graded_resident.clear()
	_graded_changed.clear()
	patch_payload_requested.clear()
	patch_payload_requested_generation.clear()
	patch_payload_ready.clear()
	dirty_patch_payload_generations.clear()
	patch_payload_refused.clear()
	patch_payload_road_generation = -1
	pending_terrain_ack_states = PackedInt64Array()
	_prewarm_requests.clear()
	# A load restarts both epoch counters, so an old value could match a new world.
	_land_cover_clean_epoch = -1
	water_texture_sync_queue.clear()
	water_texture_sync_lookup.clear()
	_stream.revise_terrain_residency()

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
	_ensure_patch_node_parent(patch_node)
	return resources

func _release_terrain_patch_resources(patch: Dictionary) -> void:
	var patch_node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
	if patch_node == null:
		return
	var material: ShaderMaterial = patch.get("material", null) as ShaderMaterial
	var height_image: Image = patch.get("height_image", null) as Image
	var height_texture: ImageTexture = patch.get("height_texture", null) as ImageTexture
	var spare_height_image: Image = patch.get("spare_height_image", null) as Image
	var spare_height_texture: ImageTexture = patch.get("spare_height_texture", null) as ImageTexture
	patch_node.visible = false
	patch_node.mesh = null
	patch_node.position = Vector3.ZERO
	patch_node.name = "TerrainPatchPool"
	if material != null:
		material.shader = TERRAIN_SHADER
		patch_node.material_override = material
	if patch_resource_pool.size() >= PATCH_RESOURCE_POOL_MAX:
		patch_node.queue_free()
		return
	patch_resource_pool.append({
		"node": patch_node,
		"material": material,
		"height_image": height_image,
		"height_texture": height_texture,
		"land_cover": patch.get("land_cover", {}),
		"spare_land_cover": patch.get("spare_land_cover", {}),
		"height_texture_width": int(patch.get("texture_width", 0)),
		"height_texture_height": int(patch.get("texture_height", 0)),
		"spare_height_image": spare_height_image,
		"spare_height_texture": spare_height_texture,
		"spare_height_texture_width": int(patch.get("spare_height_texture_width", 0)),
		"spare_height_texture_height": int(patch.get("spare_height_texture_height", 0)),
	})
	_terrain_resource_pool_release_count += 1

## Culling bounds for a patch centred on its node: its own footprint with a small pad for
## skirts, and the full height margin, because a sculpted height has no cached range.
static func patch_cull_aabb(size_xz: Vector2) -> AABB:
	var half := size_xz * 0.5 + Vector2.ONE * PATCH_CULL_PAD_M
	return AABB(
		Vector3(-half.x, -PATCH_EXTRA_CULL_MARGIN_M, -half.y),
		Vector3(half.x * 2.0, PATCH_EXTRA_CULL_MARGIN_M * 2.0, half.y * 2.0)
	)

# The patch material's parameters that no patch, world or setting changes (`RENDER-32-M`): set
# once when the pooled material is made, so a build sets only its own (a material keeps them
# through the pool). The shadow distances and the canopy floor shading follow a setting and stay
# per build, as before. (A template duplicated per material cost 0.31 ms a material.)
func _apply_constant_patch_parameters(material: ShaderMaterial) -> void:
	WorldMaterials.apply_terrain_paving_parameters(material)
	material.set_shader_parameter("height_scale", HEIGHT_SCALE)
	SceneLightingConfig.apply_ground_shadow_parameters(material)
	TreeSpecies.apply_far_canopy(material)
	material.set_shader_parameter("terrain_grain_detail_scale", 1.0 / FIELD_OVERLAY_TEXTURE_TILE_M)
	material.set_shader_parameter("terrain_field_strength", FIELD_OVERLAY_STRENGTH)
	material.set_shader_parameter("terrain_grain_tint", FIELD_OVERLAY_GRAIN_TINT)
	material.set_shader_parameter("terrain_grain_brightness", FIELD_OVERLAY_GRAIN_BRIGHTNESS)
	material.set_shader_parameter("terrain_grain_mix_strength", FIELD_OVERLAY_GRAIN_MIX_STRENGTH)
	material.set_shader_parameter("terrain_grain_macro_strength", FIELD_OVERLAY_GRAIN_MACRO_STRENGTH)
	material.set_shader_parameter("terrain_rock_slope_start", TERRAIN_ROCK_SLOPE_START)
	material.set_shader_parameter("terrain_rock_slope_end", TERRAIN_ROCK_SLOPE_END)
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

	var material := ShaderMaterial.new()
	material.shader = TERRAIN_SHADER
	_apply_constant_patch_parameters(material)
	patch_node.material_override = material

	var texture_size: Vector2i = _terrain_default_patch_texture_size()
	var height_image := Image.create(texture_size.x, texture_size.y, false, Image.FORMAT_RF)
	height_image.fill(Color.BLACK)
	var height_texture := ImageTexture.create_from_image(height_image)

	add_child(patch_node)
	return {
		"node": patch_node,
		"material": material,
		"height_image": height_image,
		"height_texture": height_texture,
		"height_texture_width": texture_size.x,
		"height_texture_height": texture_size.y,
		"spare_height_image": null,
		"spare_height_texture": null,
		"spare_height_texture_width": 0,
		"spare_height_texture_height": 0,
	}

func _terrain_default_patch_texture_size() -> Vector2i:
	var sample_count: int = max(2, patch_interval_cells + 1)
	return Vector2i(sample_count + 2, sample_count + 2)

# A coverage upload uses the same active/spare ownership as the height texture. Building a
# terrain stage cannot mutate the currently displayed texture; commit swaps both products.
## `prepared` is the land cover the patch's payload carried (`RENDER-32-M`: built on the payload
## worker, the same bytes and revisions as the query); empty, the core is queried here.
func _stage_patch_land_cover(
	key: Vector2i, patch: Dictionary, force: bool = false, prepared: Dictionary = {}
) -> Dictionary:
	var current: Dictionary = patch.get("land_cover", {})
	if not force and not current.is_empty() and simulation_node.is_vegetation_land_cover_current(key, current["generations"]):
		return current
	var query_us := Time.get_ticks_usec() if _build_perf else 0
	var data: Dictionary = prepared if not prepared.is_empty() else simulation_node.get_vegetation_land_cover(key)
	if _build_perf:
		_build_us[8] += float(Time.get_ticks_usec() - query_us)
		_build_us[9] += 0.0 if prepared.is_empty() else 1.0
	var spare: Dictionary = patch.get("spare_land_cover", {})
	var image: Image = spare.get("image", null) as Image
	if image == null:
		image = Image.new()
	image.set_data(int(data["width"]), int(data["height"]), false, Image.FORMAT_R8, data["bytes"])
	var texture: ImageTexture = spare.get("texture", null) as ImageTexture
	if texture != null and texture.get_width() == image.get_width() and texture.get_height() == image.get_height():
		texture.update(image)
	else:
		texture = ImageTexture.create_from_image(image)
	spare = {"image": image, "texture": texture,
		"world_bounds": data["world_bounds"], "generations": data["generations"]}
	patch["spare_land_cover"] = spare
	return spare

func _commit_patch_land_cover(patch: Dictionary, stage: Dictionary) -> void:
	_graded_material_changed(patch.get("key"))
	if stage == patch.get("land_cover", {}):
		return
	patch["spare_land_cover"] = patch.get("land_cover", {})
	patch["land_cover"] = stage
	var material: ShaderMaterial = patch["material"] as ShaderMaterial
	material.set_shader_parameter("land_cover_texture", stage["texture"])
	material.set_shader_parameter("land_cover_world_bounds", stage["world_bounds"])

func _sync_land_cover() -> void:
	# O(1) while no revision moved: the epoch advances with either stream, and a patch built
	# since the last walk stamped the generations current at its build. Otherwise O(built
	# patches), nine pairs of existing revisions per patch, one upload per frame.
	var epoch: int = simulation_node.get_land_cover_epoch()
	if epoch == _land_cover_clean_epoch:
		return
	for key: Vector2i in patches:
		var patch: Dictionary = patches[key]
		var current: Dictionary = patch["land_cover"]
		if not simulation_node.is_vegetation_land_cover_current(key, current["generations"]):
			_commit_patch_land_cover(patch, _stage_patch_land_cover(key, patch))
			return
	# Read before the walk, so a revision that moved during it is walked again next frame.
	_land_cover_clean_epoch = epoch

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

func _time_budget_exhausted(start_us: int, budget_ms: float, completed_count: int) -> bool:
	if completed_count <= 0:
		return false
	return float(Time.get_ticks_usec() - start_us) / 1000.0 >= budget_ms

func _dirty_patch_keys(flat_pairs: PackedInt32Array) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	var pair_count := flat_pairs.size() / 2
	for index in range(pair_count):
		keys.append(Vector2i(flat_pairs[index * 2], flat_pairs[index * 2 + 1]))
	return keys

func _dirty_patch_payload_keys_from_states(
	flat_states: PackedInt64Array
) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	dirty_patch_payload_generations.clear()
	var state_count := flat_states.size() / 3
	for index in range(state_count):
		var key := Vector2i(
			int(flat_states[index * 3]),
			int(flat_states[index * 3 + 1])
		)
		var generation := int(flat_states[index * 3 + 2])
		var render_step_mm := _terrain_patch_payload_render_step_mm(key)
		keys.append(key)
		dirty_patch_payload_generations[key] = generation
		if patch_payload_requested.has(key) and (
			int(patch_payload_requested.get(key, -1)) != render_step_mm
			or int(patch_payload_requested_generation.get(key, -1)) != generation
		):
			patch_payload_requested.erase(key)
			patch_payload_requested_generation.erase(key)
		if patch_payload_ready.has(key):
			var ready_patch_data: Dictionary = patch_payload_ready[key] as Dictionary
			if (
				int(ready_patch_data.get("render_step_mm", -1)) != render_step_mm
				or int(ready_patch_data.get("surface_generation", -1)) != generation
			):
				patch_payload_ready.erase(key)
	return keys

func _acknowledge_terrain_batch(flat_states: PackedInt64Array) -> bool:
	if flat_states.is_empty():
		return true
	if simulation_node.acknowledge_terrain_patches(flat_states):
		pending_terrain_ack_states = PackedInt64Array()
		return true
	# Failed acks may be stale generations; refetch authoritative dirty states next frame.
	pending_terrain_ack_states = PackedInt64Array()
	return false

func _retry_pending_terrain_ack() -> bool:
	if pending_terrain_ack_states.is_empty():
		return true
	if not simulation_node.acknowledge_terrain_patches(pending_terrain_ack_states):
		pending_terrain_ack_states = PackedInt64Array()
		return false
	pending_terrain_ack_states = PackedInt64Array()
	return true

func _dirty_patch_keys_touch_border(keys: Array[Vector2i]) -> bool:
	for key in keys:
		if key.x <= 0 or key.y <= 0 or key.x >= patch_cols - 1 or key.y >= patch_rows - 1:
			return true
	return false

func road_geometry_debug_patch_lines(flat_pairs: PackedInt32Array) -> Array[String]:
	var lines: Array[String] = []
	for index in range(flat_pairs.size() / 2):
		var key := Vector2i(flat_pairs[index * 2], flat_pairs[index * 2 + 1])
		# The patch's cached payload: a diagnostic never requests one (as on road before the deletion).
		var patch: Dictionary = patches.get(key, {})
		var patch_data: Dictionary = patch.get("last_patch_data", {})
		if patch_data.is_empty():
			lines.append("terrain_patch key=(%d,%d) missing_cached_patch_data=true" % [key.x, key.y])
			continue
		var node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
		var mesh: Mesh = node.mesh if node != null else null
		var height_stats := _terrain_patch_height_stats(patch_data)
		lines.append(
			"terrain_patch key=(%d,%d) resident=%s mesh=\"%s\" sample=%dx%d texture=%dx%d height_min=%.3f height_max=%.3f"
			% [
				key.x, key.y, str(resident_patch_lookup.has(key)), RenderDebug.mesh_label(mesh),
				int(patch_data["sample_width"]), int(patch_data["sample_height"]),
				int(patch_data["texture_width"]), int(patch_data["texture_height"]),
				float(height_stats.get("min", 0.0)), float(height_stats.get("max", 0.0)),
			]
		)
	return lines

func _terrain_patch_payload_render_step_mm(_key: Vector2i) -> int:
	return 0

func _road_surface_payload_generation() -> int:
	if simulation_node.has_method("get_road_tool_surface_generation"):
		return int(simulation_node.get_road_tool_surface_generation())
	return 0

func _sync_patch_payload_road_generation() -> void:
	# Terrain payload validity is patch-local. Dirty-state generations below
	# retire only affected requests; the road-tool revision is diagnostic here.
	patch_payload_road_generation = _road_surface_payload_generation()

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
	_sync_patch_payload_road_generation()
	for key in keys:
		if requested_count >= budget:
			break
		if not include_existing and patches.has(key):
			continue
		var render_step_mm := _terrain_patch_payload_render_step_mm(key)
		if patch_payload_ready.has(key):
			var ready_patch_data: Dictionary = patch_payload_ready[key] as Dictionary
			if int(ready_patch_data.get("render_step_mm", -1)) == render_step_mm:
				continue
			patch_payload_ready.erase(key)
		if int(patch_payload_requested.get(key, -1)) == render_step_mm:
			continue
		if patch_payload_refused.has(key) and int(patch_payload_refused[key][0]) == render_step_mm:
			continue
		flat_requests.push_back(key.x)
		flat_requests.push_back(key.y)
		flat_requests.push_back(render_step_mm)
		requested_count += 1
	if not flat_requests.is_empty():
		var result: Dictionary = simulation_node.request_terrain_patch_payloads(flat_requests)
		var tracked_requests: PackedInt64Array = result.get(
			"tracked_requests",
			PackedInt64Array()
		) as PackedInt64Array
		var tracked_count := tracked_requests.size() / 4
		for index in range(tracked_count):
			var key := Vector2i(
				int(tracked_requests[index * 4]),
				int(tracked_requests[index * 4 + 1])
			)
			patch_payload_requested[key] = int(tracked_requests[index * 4 + 2])
			patch_payload_requested_generation[key] = int(tracked_requests[index * 4 + 3])
	return requested_count

func _poll_ready_terrain_patch_payloads(budget: int) -> int:
	if budget <= 0 or not simulation_node.has_method("poll_ready_terrain_patch_payloads"):
		return 0
	var result: Dictionary = simulation_node.poll_ready_terrain_patch_payloads(budget) as Dictionary
	var payloads: Array = result.get("patches", []) as Array
	var accepted_count := 0
	_sync_patch_payload_road_generation()
	var retry_requests: PackedInt64Array = result.get(
		"retry_requests",
		PackedInt64Array()
	) as PackedInt64Array
	var retry_count := retry_requests.size() / 4
	for index in range(retry_count):
		var retry_key := Vector2i(
			int(retry_requests[index * 4]),
			int(retry_requests[index * 4 + 1])
		)
		var retry_render_step_mm := int(retry_requests[index * 4 + 2])
		var retry_generation := int(retry_requests[index * 4 + 3])
		if (
			int(patch_payload_requested.get(retry_key, -1)) == retry_render_step_mm
			and int(patch_payload_requested_generation.get(retry_key, -1)) == retry_generation
		):
			patch_payload_requested.erase(retry_key)
			patch_payload_requested_generation.erase(retry_key)
			_prewarm_requests.erase(retry_key)
			_stream.terrain_payload(retry_key, 0)
	var refused_requests: PackedInt64Array = result.get(
		"refused_requests",
		PackedInt64Array()
	) as PackedInt64Array
	var refused_reasons: PackedStringArray = result.get("refused_reasons", PackedStringArray()) as PackedStringArray
	for index in range(refused_requests.size() / 4):
		var refused_key := Vector2i(int(refused_requests[index * 4]), int(refused_requests[index * 4 + 1]))
		var refused_step := int(refused_requests[index * 4 + 2])
		var reason := refused_reasons[index] if index < refused_reasons.size() else "unknown"
		if int(patch_payload_requested.get(refused_key, -1)) == refused_step:
			patch_payload_requested.erase(refused_key)
			patch_payload_requested_generation.erase(refused_key)
			_prewarm_requests.erase(refused_key)
			_stream.terrain_payload(refused_key, 2)
		if not patch_payload_refused.has(refused_key):
			MetrumDebug.log(
				"terrain",
				"terrain payload refused key=(%d,%d) step_mm=%d reason=%s"
				% [refused_key.x, refused_key.y, refused_step, reason]
			)
		patch_payload_refused[refused_key] = [refused_step, reason]
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
		if payload_generation != requested_generation:
			continue
		var render_step_mm := int(patch_data.get("render_step_mm", _terrain_patch_payload_render_step_mm(key)))
		if int(patch_payload_requested.get(key, -1)) != render_step_mm:
			continue
		_prewarm_requests.erase(key)
		if bool(patch_data.get("retry", false)):
			patch_payload_requested.erase(key)
			patch_payload_requested_generation.erase(key)
			_stream.terrain_payload(key, 0)
			accepted_count += 1
			continue
		patch_payload_ready[key] = patch_data
		patch_payload_requested.erase(key)
		patch_payload_requested_generation.erase(key)
		_stream.terrain_payload(key, 1)
		accepted_count += 1
	return accepted_count

# Drops held and requested payloads the streaming core no longer wants: outside its prewarm
# square (the wanted square), and held payloads of built patches. O(held +
# requested payloads), none on a settled frame.
func _prune_patch_payload_cache() -> void:
	_sync_patch_payload_road_generation()
	if patch_payload_ready.is_empty() and patch_payload_requested.is_empty():
		return
	var halo: Vector4i = _stream.get_terrain_halo()
	var outside := func(key: Vector2i) -> bool:
		return key.x < halo.x or key.y < halo.y or key.x > halo.z or key.y > halo.w
	for key_variant in patch_payload_ready.keys():
		var key: Vector2i = key_variant
		if patches.has(key):
			patch_payload_ready.erase(key)
			continue
		if outside.call(key):
			patch_payload_ready.erase(key)
			_stream.terrain_payload(key, 0)
	for key_variant in patch_payload_requested.keys():
		var key: Vector2i = key_variant
		if not patches.has(key) and not resident_patch_lookup.has(key) and outside.call(key):
			patch_payload_requested.erase(key)
			patch_payload_requested_generation.erase(key)
			_prewarm_requests.erase(key)
			_stream.terrain_payload(key, 0)

func _dirty_patch_payloads_ready_for_atomic_upload(keys: Array[Vector2i]) -> bool:
	var all_ready := true
	for key in keys:
		if not patches.has(key):
			continue
		if not _patch_payload_ready_for_key(key):
			all_ready = false
	return all_ready

func _dirty_patch_payloads_renderable_for_atomic_upload(keys: Array[Vector2i]) -> bool:
	for key in keys:
		if not patches.has(key):
			continue
		var patch_data: Dictionary = patch_payload_ready.get(key, {}) as Dictionary
		if patch_data.is_empty():
			return false
	return true

func _stage_dirty_patch_payloads_for_atomic_commit(keys: Array[Vector2i]) -> Dictionary:
	var stages: Dictionary = {}
	for key in keys:
		if not patches.has(key):
			continue
		var patch_data: Dictionary = patch_payload_ready[key] as Dictionary
		var expected_generation := int(dirty_patch_payload_generations.get(key, -1))
		var stage := _stage_terrain_patch_update(
			key,
			patch_data,
			expected_generation,
			_terrain_patch_payload_render_step_mm(key)
		)
		if stage.is_empty():
			return {"valid": false, "stages": {}}
		stages[key] = stage
	return {"valid": true, "stages": stages}

func _terrain_patch_stage_matches_target(key: Vector2i, stage: Dictionary) -> bool:
	if not bool(stage.get("valid", false)) or not patches.has(key):
		return false
	var patch: Dictionary = patches[key]
	var patch_node: MeshInstance3D = patch.get("node", null) as MeshInstance3D
	return (
		patch_node != null
		and is_instance_valid(patch_node)
		and int(stage.get("patch_node_id", 0)) == patch_node.get_instance_id()
		and typeof(stage.get("patch_data", null)) == TYPE_DICTIONARY
		and stage.get("height_image", null) is Image
		and stage.get("height_texture", null) is ImageTexture
		and stage.has("mask_image")
		and typeof(stage.get("land_cover", null)) == TYPE_DICTIONARY
		and simulation_node.is_vegetation_land_cover_current(key, stage["land_cover"]["generations"])
		and stage.get("terrain_mesh", null) is Mesh
	)

func _terrain_patch_payload_is_stageable(
	key: Vector2i,
	patch_data: Dictionary,
	expected_generation: int,
	expected_render_step_mm: int
) -> bool:
	if not patches.has(key):
		return false
	var patch: Dictionary = patches[key]
	if (
		not (patch.get("node", null) is MeshInstance3D)
		or not (patch.get("material", null) is ShaderMaterial)
		or not (patch.get("height_image", null) is Image)
		or not (patch.get("height_texture", null) is ImageTexture)
	):
		return false
	if (
		typeof(patch_data.get("patch_x", null)) != TYPE_INT
		or typeof(patch_data.get("patch_z", null)) != TYPE_INT
		or int(patch_data["patch_x"]) != key.x
		or int(patch_data["patch_z"]) != key.y
		or typeof(patch_data.get("surface_generation", null)) != TYPE_INT
		or int(patch_data["surface_generation"]) != expected_generation
		or typeof(patch_data.get("render_step_mm", null)) != TYPE_INT
		or int(patch_data["render_step_mm"]) != expected_render_step_mm
	):
		return false
	for field in [
		"sample_width",
		"sample_height",
		"texture_width",
		"texture_height",
		"inner_offset_x",
		"inner_offset_z",
	]:
		if typeof(patch_data.get(field, null)) != TYPE_INT:
			return false
	var sample_width := int(patch_data["sample_width"])
	var sample_height := int(patch_data["sample_height"])
	var texture_width := int(patch_data["texture_width"])
	var texture_height := int(patch_data["texture_height"])
	var inner_offset_x := int(patch_data["inner_offset_x"])
	var inner_offset_z := int(patch_data["inner_offset_z"])
	if (
		sample_width < 2
		or sample_height < 2
		or texture_width <= 0
		or texture_height <= 0
		or inner_offset_x < 0
		or inner_offset_z < 0
		or inner_offset_x + sample_width > texture_width
		or inner_offset_z + sample_height > texture_height
	):
		return false
	for field in ["world_origin_x", "world_origin_z", "world_size_x", "world_size_z"]:
		if not _terrain_numeric_field_is_finite(patch_data, field):
			return false
	if float(patch_data["world_size_x"]) <= 0.0 or float(patch_data["world_size_z"]) <= 0.0:
		return false
	var expected_height_bytes := texture_width * texture_height * 4
	if expected_height_bytes <= 0:
		return false
	if (
		typeof(patch_data.get("height_bytes", null)) != TYPE_PACKED_BYTE_ARRAY
		or (patch_data["height_bytes"] as PackedByteArray).size() != expected_height_bytes
	):
		return false
	return true

func _terrain_numeric_field_is_finite(patch_data: Dictionary, field: String) -> bool:
	var value = patch_data.get(field, null)
	return (typeof(value) == TYPE_FLOAT or typeof(value) == TYPE_INT) and is_finite(float(value))

func _patch_payload_ready_for_key(key: Vector2i) -> bool:
	var expected_render_step_mm := _terrain_patch_payload_render_step_mm(key)
	_sync_patch_payload_road_generation()
	if patch_payload_ready.has(key):
		var ready_patch_data: Dictionary = patch_payload_ready[key] as Dictionary
		if int(ready_patch_data.get("render_step_mm", expected_render_step_mm)) == expected_render_step_mm:
			return true
		patch_payload_ready.erase(key)
	_request_terrain_patch_payload(key, true)
	return false

func _terrain_patch_data_for_key(
	key: Vector2i,
	_include_debug: bool = false,
	_allow_async: bool = false
) -> Dictionary:
	var expected_render_step_mm := _terrain_patch_payload_render_step_mm(key)
	_sync_patch_payload_road_generation()
	if patch_payload_ready.has(key):
		var ready_patch_data: Dictionary = patch_payload_ready[key] as Dictionary
		if int(ready_patch_data.get("render_step_mm", expected_render_step_mm)) == expected_render_step_mm:
			patch_payload_ready.erase(key)
			patch_payload_requested.erase(key)
			patch_payload_requested_generation.erase(key)
			_prewarm_requests.erase(key)
			return ready_patch_data
		patch_payload_ready.erase(key)
	_request_terrain_patch_payload(key, patches.has(key))
	return {}

func _terrain_patch_height_bytes(patch_data: Dictionary) -> PackedByteArray:
	return patch_data["height_bytes"] as PackedByteArray

func _rebuild_mask_pages(border_texels: int) -> void:
	_mask_pages.clear()
	_mask_layer_size = Vector2i.ONE * (patch_interval_cells + 1 + 2 * border_texels)
	var blank := Image.create(_mask_layer_size.x, _mask_layer_size.y, false, Image.FORMAT_RG8)
	var slots := patch_cols * patch_rows
	for first in range(0, slots, MASK_LAYERS_PER_PAGE):
		var layers: Array[Image] = []
		layers.resize(mini(MASK_LAYERS_PER_PAGE, slots - first))
		layers.fill(blank)
		var page := Texture2DArray.new()
		page.create_from_images(layers)
		_mask_pages.append(page)

## The relief and cliff-reach masks Rust baked from this payload's height samples, one texel per
## heightmap texel. Null for a payload without them (a test fixture built outside the simulation).
func _terrain_patch_mask_image(patch_data: Dictionary) -> Image:
	var width := int(patch_data["texture_width"])
	var height := int(patch_data["texture_height"])
	var bytes: PackedByteArray = patch_data.get("mask_bytes", PackedByteArray())
	if width <= 0 or height <= 0 or bytes.size() != width * height * 2:
		return null
	return Image.create_from_data(width, height, false, Image.FORMAT_RG8, bytes)

# Writes the patch's mask layer and points its material at it; without masks the shader treats the
# patch as flat ground with no cliff.
func _apply_patch_masks(
	key: Vector2i, material: ShaderMaterial, image: Image, texture_width: int, texture_height: int
) -> void:
	_graded_material_changed(key)
	var slot := key.y * patch_cols + key.x
	if (
		image == null or key.x < 0 or key.x >= patch_cols or slot < 0 or slot >= patch_cols * patch_rows
		or image.get_width() > _mask_layer_size.x or image.get_height() > _mask_layer_size.y
	):
		material.set_shader_parameter("terrain_mask_layer", -1.0)
		return
	if image.get_size() != _mask_layer_size:
		# A world-edge patch has fewer samples. Its UVs never reach the padding: the shader samples
		# only the inner samples, inside the border ring.
		var padded := Image.create(_mask_layer_size.x, _mask_layer_size.y, false, Image.FORMAT_RG8)
		padded.blit_rect(image, Rect2i(Vector2i.ZERO, image.get_size()), Vector2i.ZERO)
		image = padded
	var page: Texture2DArray = _mask_pages[slot / MASK_LAYERS_PER_PAGE]
	page.update_layer(image, slot % MASK_LAYERS_PER_PAGE)
	material.set_shader_parameter("terrain_masks", page)
	material.set_shader_parameter("terrain_mask_layer", float(slot % MASK_LAYERS_PER_PAGE))
	material.set_shader_parameter(
		"terrain_mask_uv_scale", Vector2(texture_width, texture_height) / Vector2(_mask_layer_size))

func _terrain_patch_height_stats(patch_data: Dictionary) -> Dictionary:
	var bytes: PackedByteArray = patch_data.get("height_bytes", PackedByteArray())
	return RenderDebug.float_stats(bytes.to_float32_array())

func _terrain_patch_mesh_from_data(
	patch_data: Dictionary,
	lod_step: int,
	subdivision_factor: int
) -> Mesh:
	return _patch_mesh(
		int(patch_data["sample_width"]),
		int(patch_data["sample_height"]),
		float(patch_data["world_size_x"]),
		float(patch_data["world_size_z"]),
		lod_step,
		subdivision_factor
	)

func _prewarm_regular_terrain_mesh_variants() -> void:
	if patch_interval_cells <= 0 or patch_span_m <= 0.0:
		return
	var sample_count: int = patch_interval_cells + 1
	var lod_steps: Array[int] = [1, 2, 4, 8]
	for lod_step: int in lod_steps:
		_patch_mesh(sample_count, sample_count, patch_span_m, patch_span_m, lod_step, 1)

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

func _mesh_subdivision_factor_for_patch(_key: Vector2i, _sample_step_m: float) -> int:
	# Graded terrain draws a road's patch with its cells, and its PlaneMesh samples the plain base:
	# a finer grid adds only vertices (the water reflection mirrored them, 5.8 M extra primitives on
	# the saved Kuopio paths, TERRAIN-07-D round 8).
	return 1

func _ensure_overlay_texture() -> bool:
	var dims: Vector2 = simulation_node.get_heightmap_size()
	var width := int(dims.x)
	var height := int(dims.y)
	if width <= 0 or height <= 0:
		overlay_image = null
		overlay_texture = null
		return false
	if (
		overlay_image != null
		and overlay_texture != null
		and overlay_image.get_width() == width
		and overlay_image.get_height() == height
	):
		return true

	var image := Image.create(width, height, false, Image.FORMAT_RGBA8)
	if image == null or image.is_empty():
		overlay_image = null
		overlay_texture = null
		return false
	var texture := ImageTexture.create_from_image(image)
	if texture == null:
		overlay_image = null
		overlay_texture = null
		return false
	overlay_image = image
	overlay_texture = texture
	return true

func _ensure_coal_pit_texture() -> bool:
	var dims: Vector2 = simulation_node.get_coal_pit_overlay_size()
	var width := int(dims.x)
	var height := int(dims.y)
	if width <= 0 or height <= 0:
		coal_pit_image = null
		coal_pit_texture = null
		return false
	if (
		coal_pit_image != null
		and coal_pit_texture != null
		and coal_pit_image.get_width() == width
		and coal_pit_image.get_height() == height
	):
		return true

	var image := Image.create(width, height, false, Image.FORMAT_L8)
	if image == null or image.is_empty():
		coal_pit_image = null
		coal_pit_texture = null
		return false
	image.fill(Color(0.0, 0.0, 0.0, 0.0))
	var texture := ImageTexture.create_from_image(image)
	if texture == null:
		coal_pit_image = null
		coal_pit_texture = null
		return false
	coal_pit_image = image
	coal_pit_texture = texture
	coal_pit_overlay_dirty = true
	return true

func _update_overlay_texture() -> bool:
	if not _ensure_overlay_texture():
		return false
	var width := overlay_image.get_width()
	var height := overlay_image.get_height()
	var overlay_bytes := PackedByteArray()
	if overlay_mode == 1:
		overlay_bytes = simulation_node.get_pollution_image_data()
	elif overlay_mode == 2:
		overlay_bytes = simulation_node.get_noise_image_data()
	elif overlay_mode == 3:
		overlay_bytes = simulation_node.get_desirability_image_data()
	elif overlay_mode == 4:
		overlay_bytes = simulation_node.get_world_coal_deposit_overlay_data()

	if overlay_bytes.is_empty():
		overlay_image.fill(Color(0.0, 0.0, 0.0, 0.0))
	else:
		if overlay_bytes.size() != width * height * 4:
			return false
		overlay_image.set_data(width, height, false, Image.FORMAT_RGBA8, overlay_bytes)
	overlay_texture.update(overlay_image)
	return true

func _update_coal_pit_texture() -> bool:
	if not _ensure_coal_pit_texture():
		return false
	var width := coal_pit_image.get_width()
	var height := coal_pit_image.get_height()
	coal_pit_overlay_world_bounds = (
		simulation_node.get_coal_pit_overlay_world_bounds()
		if simulation_node.has_method("get_coal_pit_overlay_world_bounds")
		else Vector4.ZERO
	)
	var overlay_bytes: PackedByteArray = simulation_node.get_coal_pit_overlay_data()
	if overlay_bytes.is_empty():
		coal_pit_image.fill(Color(0.0, 0.0, 0.0, 0.0))
	else:
		if overlay_bytes.size() != width * height:
			return false
		coal_pit_image.set_data(width, height, false, Image.FORMAT_L8, overlay_bytes)
	coal_pit_texture.update(coal_pit_image)
	coal_pit_overlay_dirty = false
	return true

func _ensure_field_overlay_texture() -> bool:
	var dims: Vector2 = simulation_node.get_agriculture_field_overlay_size()
	var width := int(dims.x)
	var height := int(dims.y)
	if width <= 0 or height <= 0:
		field_overlay_image = null
		field_overlay_texture = null
		return false
	if (
		field_overlay_image != null
		and field_overlay_texture != null
		and field_overlay_image.get_width() == width
		and field_overlay_image.get_height() == height
	):
		return true

	var image := Image.create(width, height, false, Image.FORMAT_L8)
	if image == null or image.is_empty():
		field_overlay_image = null
		field_overlay_texture = null
		return false
	image.fill(Color(0.0, 0.0, 0.0, 0.0))
	var texture := ImageTexture.create_from_image(image)
	if texture == null:
		field_overlay_image = null
		field_overlay_texture = null
		return false
	field_overlay_image = image
	field_overlay_texture = texture
	field_overlay_dirty = true
	return true

func _update_field_overlay_texture() -> bool:
	if not _ensure_field_overlay_texture():
		return false
	var width := field_overlay_image.get_width()
	var height := field_overlay_image.get_height()
	field_overlay_world_bounds = (
		simulation_node.get_agriculture_field_overlay_world_bounds()
		if simulation_node.has_method("get_agriculture_field_overlay_world_bounds")
		else Vector4.ZERO
	)
	var overlay_bytes: PackedByteArray = simulation_node.get_agriculture_field_overlay_data()
	if overlay_bytes.is_empty():
		field_overlay_image.fill(Color(0.0, 0.0, 0.0, 0.0))
	else:
		if overlay_bytes.size() != width * height:
			return false
		field_overlay_image.set_data(width, height, false, Image.FORMAT_L8, overlay_bytes)
	field_overlay_texture.update(field_overlay_image)
	field_overlay_dirty = false
	return true

func _refresh_overlay_texture(current_day: int) -> void:
	# The three environmental fields change at daily settlement; resource deposits use explicit invalidation.
	var environment_day_changed := (
		overlay_mode >= 1 and overlay_mode <= 3 and current_day != cached_overlay_day
	)
	if overlay_texture != null and overlay_mode == cached_overlay_mode and not environment_day_changed:
		return
	var previous_texture := overlay_texture
	if not _update_overlay_texture():
		cached_overlay_mode = -1
		return
	# Updating an existing ImageTexture already updates every material that references it.
	if overlay_mode != cached_overlay_mode or overlay_texture != previous_texture:
		_apply_overlay_mode()
	cached_overlay_mode = overlay_mode
	cached_overlay_day = current_day

func mark_overlay_dirty() -> void:
	cached_overlay_mode = -1

func mark_coal_pit_overlay_dirty() -> void:
	coal_pit_overlay_dirty = true

func mark_field_overlay_dirty() -> void:
	field_overlay_dirty = true

func _apply_overlay_mode() -> void:
	for key in _graded_resident.keys():
		_graded_material_changed(key)
	for key in patches.keys():
		var material: ShaderMaterial = patches[key]["material"]
		material.set_shader_parameter("overlay_mode", overlay_mode)
		material.set_shader_parameter("overlay_texture", overlay_texture)

func _apply_coal_pit_texture() -> void:
	for key in patches.keys():
		var material: ShaderMaterial = patches[key]["material"]
		material.set_shader_parameter("coal_pit_texture", coal_pit_texture)
		material.set_shader_parameter("coal_pit_overlay_world_bounds", coal_pit_overlay_world_bounds)

func _apply_field_overlay_texture() -> void:
	for key in patches.keys():
		var material: ShaderMaterial = patches[key]["material"]
		material.set_shader_parameter("field_overlay_texture", field_overlay_texture)
		material.set_shader_parameter("field_overlay_world_bounds", field_overlay_world_bounds)

func _ensure_empty_water_texture() -> void:
	if empty_water_texture != null:
		return
	var image := Image.create(2, 2, false, Image.FORMAT_RF)
	image.fill(Color.BLACK)
	empty_water_texture = ImageTexture.create_from_image(image)

func _ensure_grass_textures() -> void:
	if grass_albedo_texture == null:
		grass_albedo_texture = WorldMaterials.load_texture_or_solid(WorldMaterials.GRASS_ALBEDO, Color(0.5, 0.5, 0.5, 1.0))
	if grass_height_texture == null:
		grass_height_texture = WorldMaterials.load_texture_or_solid(WorldMaterials.GRASS_HEIGHT, Color(0.5, 0.5, 0.5, 1.0))
	if coal_albedo_texture == null:
		coal_albedo_texture = WorldMaterials.load_texture_or_solid(TERRAIN_COAL_ALBEDO_PATH, Color(0.04, 0.04, 0.04, 1.0))
	if grain_albedo_texture == null:
		grain_albedo_texture = WorldMaterials.load_texture_or_solid(TERRAIN_GRAIN_ALBEDO_PATH, FIELD_OVERLAY_FALLBACK_COLOR)

func _bind_empty_water_texture(patch: Dictionary, material: ShaderMaterial) -> void:
	_graded_material_changed(patch.get("key"))
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

func _queue_all_water_patch_texture_syncs() -> void:
	for key_variant in resident_patch_lookup.keys():
		_queue_water_patch_texture_sync(key_variant as Vector2i)

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

	if processed_count > 0 and _terrain_debug_enabled and (_terrain_debug_verbose or MetrumDebug.visual_mode_index("terrain_visual") >= 6):
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
	_graded_material_changed(patch.get("key"))
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
	simulation_node.sculpt_terrain(
		Vector2(intersection.x, intersection.z),
		15.0,
		strength / HEIGHT_SCALE
	)

	var road_tool = get_node_or_null("../RoadTool")
	if road_tool:
		road_tool.update_main_mesh()

func _ensure_border_visuals() -> void:
	if border_skirt_instance == null:
		border_skirt_instance = MeshInstance3D.new()
		border_skirt_instance.name = "TerrainBorderSkirt"
		SceneLightingConfig.apply_shadow_policy(
			border_skirt_instance,
			SceneLightingConfig.SHADOW_NON_RECEIVER,
			"terrain"
		)
		border_skirt_instance.extra_cull_margin = PATCH_EXTRA_CULL_MARGIN_M
		add_child(border_skirt_instance)
	if border_bottom_cap_instance == null:
		border_bottom_cap_instance = MeshInstance3D.new()
		border_bottom_cap_instance.name = "TerrainBorderBottomCap"
		SceneLightingConfig.apply_shadow_policy(
			border_bottom_cap_instance,
			SceneLightingConfig.SHADOW_NON_RECEIVER,
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
		border_skirt_material.set_shader_parameter("topsoil_color", TERRAIN_BORDER_TOPSOIL_COLOR)
		border_skirt_material.set_shader_parameter("band_interval_m", TERRAIN_BORDER_BAND_INTERVAL_M)
		border_skirt_material.set_shader_parameter("band_strength", TERRAIN_BORDER_BAND_STRENGTH)
		border_skirt_material.set_shader_parameter("strata_warp_m", TERRAIN_BORDER_STRATA_WARP_M)
		border_skirt_material.set_shader_parameter("topsoil_depth_m", TERRAIN_BORDER_TOPSOIL_DEPTH_M)
		border_skirt_material.set_shader_parameter("topsoil_strength", TERRAIN_BORDER_TOPSOIL_STRENGTH)
		border_skirt_material.set_shader_parameter("normal_strength", TERRAIN_BORDER_NORMAL_STRENGTH)
		border_skirt_material.set_shader_parameter("contour_minor_interval_m", CONTOUR_MINOR_INTERVAL_M)
		border_skirt_material.set_shader_parameter("contour_major_interval_m", CONTOUR_MAJOR_INTERVAL_M)
		border_skirt_material.set_shader_parameter("contour_minor_color", TERRAIN_BORDER_CONTOUR_MINOR_COLOR)
		border_skirt_material.set_shader_parameter("contour_major_color", TERRAIN_BORDER_CONTOUR_MAJOR_COLOR)
		border_skirt_material.set_shader_parameter("contour_minor_strength", TERRAIN_BORDER_CONTOUR_MINOR_STRENGTH)
		border_skirt_material.set_shader_parameter("contour_major_strength", TERRAIN_BORDER_CONTOUR_MAJOR_STRENGTH)
	if border_bottom_cap_material == null:
		border_bottom_cap_material = StandardMaterial3D.new()
		border_bottom_cap_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		border_bottom_cap_material.disable_receive_shadows = true
		border_bottom_cap_material.albedo_color = TERRAIN_BORDER_BOTTOM_COLOR
		border_bottom_cap_material.cull_mode = BaseMaterial3D.CULL_FRONT
	border_skirt_instance.material_override = border_skirt_material
	border_bottom_cap_instance.material_override = border_bottom_cap_material

func _rebuild_border_skirt() -> void:
	var stage := _stage_terrain_border_update()
	if bool(stage.get("valid", false)):
		_commit_terrain_border_stage(stage)

func _stage_terrain_border_update() -> Dictionary:
	var positions: PackedVector3Array = simulation_node.get_terrain_border_loop()
	for position in positions:
		if not is_finite(position.x) or not is_finite(position.y) or not is_finite(position.z):
			return {"valid": false}
	if positions.size() < 4:
		return {
			"valid": true,
			"positions": positions,
			"skirt_mesh": null,
			"bottom_mesh": null,
			"bottom_position": Vector3.ZERO,
			"increment_revision": false,
		}

	var min_edge_y := INF
	for position in positions:
		min_edge_y = min(min_edge_y, position.y)

	var bottom_y := min_edge_y - TERRAIN_BORDER_DEPTH_M
	var surface_tool := SurfaceTool.new()
	surface_tool.begin(Mesh.PRIMITIVE_TRIANGLES)
	var perimeter_u := 0.0
	for index in range(positions.size()):
		var next_index := (index + 1) % positions.size()
		var top_a: Vector3 = positions[index]
		var top_b: Vector3 = positions[next_index]
		var bottom_a := Vector3(top_a.x, bottom_y, top_a.z)
		var bottom_b := Vector3(top_b.x, bottom_y, top_b.z)
		var segment_length := top_a.distance_to(top_b)
		_add_skirt_quad(surface_tool, top_a, top_b, bottom_b, bottom_a, perimeter_u, perimeter_u + segment_length)
		perimeter_u += segment_length

	surface_tool.generate_tangents()
	var skirt_mesh := surface_tool.commit()
	if skirt_mesh == null:
		return {"valid": false}
	var bottom_cap := PlaneMesh.new()
	bottom_cap.size = terrain_world_size
	return {
		"valid": true,
		"positions": positions,
		"skirt_mesh": skirt_mesh,
		"bottom_mesh": bottom_cap,
		"bottom_position": Vector3(0.0, bottom_y, 0.0),
		"increment_revision": true,
	}

func _commit_terrain_border_stage(stage: Dictionary) -> void:
	_ensure_border_visuals()
	border_loop_positions = stage["positions"] as PackedVector3Array
	border_skirt_instance.mesh = stage.get("skirt_mesh", null) as Mesh
	border_skirt_instance.material_override = border_skirt_material
	border_bottom_cap_instance.mesh = stage.get("bottom_mesh", null) as Mesh
	border_bottom_cap_instance.position = stage["bottom_position"] as Vector3
	border_bottom_cap_instance.material_override = border_bottom_cap_material
	if bool(stage.get("increment_revision", false)):
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

func _record_terrain_debug_frame(
	delta: float,
	frame_elapsed_ms: float,
	upload_elapsed_ms: float,
	border_elapsed_ms: float,
	water_sync_elapsed_ms: float
) -> void:
	_terrain_debug_elapsed_s += delta
	_terrain_debug_frames += 1
	_terrain_debug_frame_ms_total += frame_elapsed_ms
	_terrain_debug_frame_ms_max = maxf(_terrain_debug_frame_ms_max, frame_elapsed_ms)
	_terrain_debug_upload_ms_total += upload_elapsed_ms
	_terrain_debug_border_ms_total += border_elapsed_ms
	_terrain_debug_water_sync_ms_total += water_sync_elapsed_ms
	if _terrain_debug_elapsed_s < TERRAIN_DEBUG_LOG_INTERVAL_S:
		return

	var frames := maxf(1.0, float(_terrain_debug_frames))
	var camera := get_viewport().get_camera_3d()
	var camera_label := "none"
	if camera != null:
		camera_label = "(%.1f, %.1f, %.1f)" % [
			camera.global_position.x,
			camera.global_position.y,
			camera.global_position.z,
		]
	# Residency and levels are the streaming core's: its counts stand for the old bounds.
	_terrain_debug_log(
		"fps=%d cam=%s resident=%d/%d stream=%s creates=%d removes=%d uploads=%d dirty_batches=%d dirty_patches=%d avg_ms=%.3f max_ms=%.3f upload_ms=%.3f border_ms=%.3f water_sync_ms=%.3f force_full_world=%s force_lod1=%s visual=%d"
		% [
			Engine.get_frames_per_second(),
			camera_label,
			resident_patch_lookup.size(),
			patch_cols * patch_rows,
			str(_stream.get_terrain_patch_counts()),
			_terrain_debug_patch_creates,
			_terrain_debug_patch_removes,
			_terrain_debug_patch_uploads,
			_terrain_debug_dirty_batches,
			_terrain_debug_dirty_patch_total,
			_terrain_debug_frame_ms_total / frames,
			_terrain_debug_frame_ms_max,
			_terrain_debug_upload_ms_total / frames,
			_terrain_debug_border_ms_total / frames,
			_terrain_debug_water_sync_ms_total / frames,
			str(_terrain_force_full_world),
			str(_terrain_force_lod1),
			MetrumDebug.visual_mode_index("terrain_visual"),
		]
	)
	_reset_terrain_debug_counters()

func _reset_terrain_debug_counters() -> void:
	_terrain_debug_elapsed_s = 0.0
	_terrain_debug_frames = 0
	_terrain_debug_frame_ms_total = 0.0
	_terrain_debug_frame_ms_max = 0.0
	_terrain_debug_upload_ms_total = 0.0
	_terrain_debug_border_ms_total = 0.0
	_terrain_debug_water_sync_ms_total = 0.0
	_terrain_debug_patch_creates = 0
	_terrain_debug_patch_removes = 0
	_terrain_debug_patch_uploads = 0
	_terrain_debug_dirty_batches = 0
	_terrain_debug_dirty_patch_total = 0

func _terrain_debug_log(message: String) -> void:
	if _terrain_debug_enabled:
		MetrumDebug.log("terrain", message)
