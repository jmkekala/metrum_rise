## Water patch renderer — uploads chunk-local visible water patches and world-edge water curtains.
##
## Rust methods called: get_water_patch(), get_water_patch_debug(),
##   get_water_patch_authored_fill_debug(), get_dirty_water_patches(), get_water_border_depths(),
##   get_water_patch_meshes(), clear_water_patch_mesh_cache(), is_water_dirty(),
##   clear_water_dirty()
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
const WATER_PATCH_MUTATION_BUDGET_PER_FRAME := 256
const WATER_PATCH_PREWARM_BUDGET_PER_FRAME := 12
const WATER_PATCH_HEIGHT_REBIND_BUDGET_PER_FRAME := 128
const WATER_TERRAIN_BINDING_BUDGET_PER_FRAME := 128
const WATER_PATCH_MESH_REFRESH_BUDGET_PER_FRAME := 4
const WATER_PATCH_MESH_LOD_REFRESH_INTERVAL_S := 0.20
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
var mesh_refresh_queue: Array[Vector3i] = []
var mesh_refresh_lookup: Dictionary = {}
var fallback_height_texture: ImageTexture
var water_border_instance: MeshInstance3D
var water_border_material: ShaderMaterial
var terrain_border_revision: int = -1
var _terrain_debug_enabled: bool = false
var _terrain_debug_verbose: bool = false
var _terrain_force_lod1: bool = false
var _water_visual_debug_mode: int = 0
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
	_terrain_force_lod1 = _terrain_debug_force_lod1()
	_water_visual_debug_mode = _terrain_visual_debug_mode_from_env()
	_water_mesh_lod_refresh_elapsed_s = 0.0
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
	_sync_patch_residency()
	var patch_sync_elapsed_ms := float(Time.get_ticks_usec() - patch_sync_start_us) / 1000.0
	var height_rebind_elapsed_ms := 0.0
	var upload_elapsed_ms := 0.0
	var border_elapsed_ms := 0.0
	var terrain_binding_elapsed_ms := 0.0
	var lod_elapsed_ms := 0.0
	var mesh_refresh_elapsed_ms := 0.0
	var prewarm_elapsed_ms := 0.0
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

	if perf_enabled:
		var lod_start_us := Time.get_ticks_usec()
		_refresh_patch_mesh_lods(delta)
		lod_elapsed_ms = float(Time.get_ticks_usec() - lod_start_us) / 1000.0
	else:
		_refresh_patch_mesh_lods(delta)

	var mesh_refresh_start_us := Time.get_ticks_usec()
	_process_mesh_refresh_queue(WATER_PATCH_MESH_REFRESH_BUDGET_PER_FRAME)
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
	if not simulation_node.is_water_dirty():
		if perf_enabled:
			var prewarm_start_us := Time.get_ticks_usec()
			_prewarm_patch_cache()
			prewarm_elapsed_ms = float(Time.get_ticks_usec() - prewarm_start_us) / 1000.0
		else:
			_prewarm_patch_cache()

	if perf_enabled:
		PerfDebug.record(
			"water",
			float(Time.get_ticks_usec() - frame_start_us) / 1000.0,
			{
				"residency": patch_sync_elapsed_ms,
				"upload": upload_elapsed_ms,
				"border": border_elapsed_ms,
				"height_rebind": height_rebind_elapsed_ms,
				"terrain_binding": terrain_binding_elapsed_ms,
				"lod": lod_elapsed_ms,
				"mesh_refresh": mesh_refresh_elapsed_ms,
				"prewarm": prewarm_elapsed_ms,
			}
		)

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

	var patch_center_x := world_origin_x + world_size_x * 0.5
	var patch_center_z := world_origin_z + world_size_z * 0.5
	var initial_lod_step := _mesh_lod_step_for_patch_center(patch_center_x, patch_center_z)
	var mesh_stats: Dictionary = _empty_water_mesh_stats(initial_lod_step)

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
		"world_size_x": world_size_x,
		"world_size_z": world_size_z,
		"world_origin_x": world_origin_x,
		"world_origin_z": world_origin_z,
		"lod_step": initial_lod_step,
		"depth_nonzero_count": _patch_visible_depth_count(patch_data),
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
	var signature_changed := int(patch.get("road_clip_signature", road_clip_signature - 1)) != road_clip_signature
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
	var should_refresh_mesh := not road_clip_only or signature_changed or depth_visibility_changed
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
		mesh_elapsed_ms = float(Time.get_ticks_usec() - mesh_start_us) / 1000.0
		patch["mesh_stats"] = mesh_stats
	patch["sample_width"] = int(patch_data["sample_width"])
	patch["sample_height"] = int(patch_data["sample_height"])
	patch["texture_width"] = texture_width
	patch["texture_height"] = texture_height
	patch["world_size_x"] = world_size_x
	patch["world_size_z"] = world_size_z
	patch["world_origin_x"] = float(patch_data["world_origin_x"])
	patch["world_origin_z"] = float(patch_data["world_origin_z"])
	patch["depth_nonzero_count"] = depth_nonzero_count
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
		return
	var patch: Dictionary = patches[key]
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.visible = false
	resident_patch_lookup.erase(key)
	_queue_terrain_patch_binding(key)

func _remove_patch(key: Vector2i) -> void:
	if not patches.has(key):
		return
	var patch: Dictionary = patches[key]
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.queue_free()
	patches.erase(key)
	resident_patch_lookup.erase(key)
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
	mesh_refresh_lookup.clear()

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
		var key: Vector2i = patch_prewarm_queue.pop_back()
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
	if _terrain_force_lod1:
		return 1
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
	var material: ShaderMaterial = patch["material"]
	material.set_shader_parameter("water_debug_lod_step", float(target_lod_step))
	var mesh_stats := _empty_water_mesh_stats(target_lod_step)
	if int(patch.get("depth_nonzero_count", 0)) > 0:
		_queue_patch_mesh_refresh(key, target_lod_step)
	else:
		patch_node.mesh = ArrayMesh.new()
	patch["mesh_stats"] = mesh_stats

func _queue_patch_mesh_refresh(key: Vector2i, lod_step: int) -> void:
	var request := Vector3i(key.x, key.y, max(1, lod_step))
	if mesh_refresh_lookup.has(request):
		return
	mesh_refresh_lookup[request] = true
	mesh_refresh_queue.append(request)

func _process_mesh_refresh_queue(budget: int) -> void:
	if simulation_node == null or not simulation_node.has_method("get_water_patch_meshes"):
		return
	var requests := PackedInt32Array()
	var remaining_budget := budget
	while remaining_budget > 0 and not mesh_refresh_queue.is_empty():
		var request: Vector3i = mesh_refresh_queue.pop_back()
		mesh_refresh_lookup.erase(request)
		remaining_budget -= 1
		var key := Vector2i(request.x, request.y)
		var patch: Dictionary = patches.get(key, {})
		if patch.is_empty() or not resident_patch_lookup.has(key):
			continue
		if int(patch.get("depth_nonzero_count", 0)) <= 0:
			var patch_node: MeshInstance3D = patch["node"]
			patch_node.mesh = ArrayMesh.new()
			patch["mesh_stats"] = _empty_water_mesh_stats(int(patch.get("lod_step", 1)))
			continue
		if int(patch.get("lod_step", 1)) != request.z:
			continue
		requests.push_back(request.x)
		requests.push_back(request.y)
		requests.push_back(request.z)

	if requests.is_empty():
		return

	var meshes: Array = simulation_node.get_water_patch_meshes(requests)
	for mesh_variant in meshes:
		var mesh_data: Dictionary = mesh_variant as Dictionary
		_apply_water_patch_mesh_data(mesh_data)

func _apply_water_patch_mesh_data(mesh_data: Dictionary) -> void:
	var key := Vector2i(int(mesh_data.get("patch_x", -1)), int(mesh_data.get("patch_z", -1)))
	if not patches.has(key):
		return
	var patch: Dictionary = patches[key]
	var lod_step := int(mesh_data.get("lod_step", 1))
	if int(patch.get("lod_step", 1)) != lod_step:
		return
	var patch_node: MeshInstance3D = patch["node"]
	patch_node.mesh = _baked_water_patch_mesh(mesh_data)
	patch["mesh_stats"] = _water_mesh_stats_from_baked_data(mesh_data)

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
		var key: Vector2i = height_texture_rebind_queue.pop_back()
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
		var key: Vector2i = terrain_patch_binding_queue.pop_back()
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
