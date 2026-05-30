## Water patch renderer — uploads chunk-local visible water patches and world-edge water curtains.
##
## Rust methods called: get_water_patch(), get_water_patch_debug(),
##   get_water_patch_authored_fill_debug(), get_dirty_water_patches(), get_water_border_depths(),
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
const ROAD_CLIP_LOOP_ROLE_OUTER := 0
const ROAD_CLIP_LOOP_ROLE_HOLE := 1

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var patches: Dictionary = {}
var resident_patch_lookup: Dictionary = {}
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
	var patch_mesh: Mesh = ArrayMesh.new()
	if _patch_visible_depth_count(patch_data) > 0:
		patch_mesh = _water_patch_mesh_from_data(patch_data, initial_lod_step)

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
		"texture_width": texture_width,
		"texture_height": texture_height,
		"world_size_x": world_size_x,
		"world_size_z": world_size_z,
		"world_origin_x": world_origin_x,
		"world_origin_z": world_origin_z,
		"lod_step": initial_lod_step,
		"depth_nonzero_count": _patch_visible_depth_count(patch_data),
		"road_clip_signature": _patch_road_clip_signature(patch_data),
		"last_patch_data": patch_data,
	}

func refresh_road_clipped_patches(flat_pairs: PackedInt32Array) -> void:
	var dirty_keys: Array[Vector2i] = _dirty_patch_keys(flat_pairs)
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
		texture_elapsed_ms = float(Time.get_ticks_usec() - texture_start_us) / 1000.0
		_water_debug_patch_uploads += 1

	var patch_node: MeshInstance3D = patch["node"]
	var world_size_x := float(patch_data["world_size_x"])
	var world_size_z := float(patch_data["world_size_z"])
	var lod_step := int(patch.get("lod_step", 1))
	var should_refresh_mesh := not road_clip_only or signature_changed or depth_visibility_changed
	var mesh_elapsed_ms := 0.0
	if should_refresh_mesh:
		var mesh_start_us := Time.get_ticks_usec()
		if depth_nonzero_count > 0:
			patch_node.mesh = _water_patch_mesh_from_data(patch_data, lod_step)
		else:
			patch_node.mesh = ArrayMesh.new()
		mesh_elapsed_ms = float(Time.get_ticks_usec() - mesh_start_us) / 1000.0
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
				"water_upload key=(%d,%d) road_clip_only=%s depth_nonzero=%d clip_loops=%d clip_points=%d signature_changed=%s texture_ms=%.3f mesh_ms=%.3f fetch_ms=%.3f total_ms=%.3f"
				% [
					key.x,
					key.y,
					str(road_clip_only),
					depth_nonzero_count,
					int(patch_data.get("road_clip_loop_count", 0)),
					int(patch_data.get("road_clip_point_count", 0)),
					str(signature_changed),
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
		lines.append(
			"water_patch key=(%d,%d) resident=%s mesh=\"%s\" sample=%dx%d texture=%dx%d world_origin=(%.3f,%.3f) world_size=(%.3f,%.3f) depth_nonzero=%d/%d depth_min=%.3f depth_max=%.3f depth_sum=%.3f baseline_nonzero=%d/%d baseline_max=%.3f baseline_sum=%.3f dynamic_nonzero=%d/%d dynamic_max=%.3f dynamic_sum=%.3f combined_nonzero=%d/%d combined_max=%.3f combined_sum=%.3f velocity_nonzero=%d/%d velocity_max=%.3f velocity_sum=%.3f source_points=%d/%d source_rate_sum=%.3f source_rate_abs_sum=%.3f clip_status=%s clip_error=%s clip_sources=%d clip_groups=%d clip_loops=%d clip_points=%d clip_area=%.3f clip_bounds=%s max_clip_bbox=(%.3f,%.3f)"
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
				int(layer_stats.get("dynamic_nonzero", -1)),
				layer_sample_count,
				float(layer_stats.get("dynamic_max", -1.0)),
				float(layer_stats.get("dynamic_sum", -1.0)),
				int(layer_stats.get("combined_nonzero", -1)),
				layer_sample_count,
				float(layer_stats.get("combined_max", -1.0)),
				float(layer_stats.get("combined_sum", -1.0)),
				int(layer_stats.get("velocity_nonzero", -1)),
				layer_sample_count,
				float(layer_stats.get("velocity_max", -1.0)),
				float(layer_stats.get("velocity_sum", -1.0)),
				int(layer_stats.get("source_count_in_patch", -1)),
				int(layer_stats.get("source_count_total", -1)),
				float(layer_stats.get("source_rate_sum", -1.0)),
				float(layer_stats.get("source_rate_abs_sum", -1.0)),
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

func _water_patch_mesh_from_data(patch_data: Dictionary, lod_step: int) -> Mesh:
	if _patch_has_road_clip_failure(patch_data):
		return ArrayMesh.new()
	return _clipped_water_patch_mesh(patch_data, lod_step)

func _patch_has_road_clip_failure(patch_data: Dictionary) -> bool:
	return str(patch_data.get("road_clip_status", "ok")) == "failed"

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

func _road_clip_group_bins(
	clip_groups: Array,
	world_origin_x: float,
	world_origin_z: float,
	world_size_x: float,
	world_size_z: float,
	x_interval_count: int,
	z_interval_count: int
) -> Dictionary:
	var bins: Dictionary = {}
	var safe_world_size_x: float = maxf(world_size_x, 0.001)
	var safe_world_size_z: float = maxf(world_size_z, 0.001)
	for clip_group in clip_groups:
		var clip_bounds: Rect2 = clip_group["bounds"]
		var min_x_index: int = clampi(
			int(floor((clip_bounds.position.x - world_origin_x) / safe_world_size_x * float(x_interval_count))),
			0,
			x_interval_count - 1
		)
		var max_x_index: int = clampi(
			int(floor((clip_bounds.position.x + clip_bounds.size.x - world_origin_x) / safe_world_size_x * float(x_interval_count))),
			0,
			x_interval_count - 1
		)
		var min_z_index: int = clampi(
			int(floor((clip_bounds.position.y - world_origin_z) / safe_world_size_z * float(z_interval_count))),
			0,
			z_interval_count - 1
		)
		var max_z_index: int = clampi(
			int(floor((clip_bounds.position.y + clip_bounds.size.y - world_origin_z) / safe_world_size_z * float(z_interval_count))),
			0,
			z_interval_count - 1
		)
		for z_index in range(min_z_index, max_z_index + 1):
			for x_index in range(min_x_index, max_x_index + 1):
				var bin_index: int = z_index * x_interval_count + x_index
				if not bins.has(bin_index):
					bins[bin_index] = []
				var bin: Array = bins[bin_index]
				bin.append(clip_group)
	return bins

func _clipped_water_patch_mesh(patch_data: Dictionary, lod_step: int) -> ArrayMesh:
	var sample_width := int(patch_data["sample_width"])
	var sample_height := int(patch_data["sample_height"])
	var world_size_x := float(patch_data["world_size_x"])
	var world_size_z := float(patch_data["world_size_z"])
	var world_origin_x := float(patch_data["world_origin_x"])
	var world_origin_z := float(patch_data["world_origin_z"])
	var texture_width := int(patch_data["texture_width"])
	var texture_height := int(patch_data["texture_height"])
	var inner_offset_x := int(patch_data["inner_offset_x"])
	var inner_offset_z := int(patch_data["inner_offset_z"])
	var depth_data: PackedFloat32Array = patch_data["depth_data"] as PackedFloat32Array
	var center_x := world_origin_x + world_size_x * 0.5
	var center_z := world_origin_z + world_size_z * 0.5
	var x_vertex_count: int = _mesh_subdivisions_for_sample_count(sample_width, lod_step) + 2
	var z_vertex_count: int = _mesh_subdivisions_for_sample_count(sample_height, lod_step) + 2
	var x_interval_count: int = maxi(1, x_vertex_count - 1)
	var z_interval_count: int = maxi(1, z_vertex_count - 1)
	var clip_groups: Array = _road_clip_loop_groups_from_patch_data(patch_data)
	var clip_bins: Dictionary = _road_clip_group_bins(
		clip_groups,
		world_origin_x,
		world_origin_z,
		world_size_x,
		world_size_z,
		x_interval_count,
		z_interval_count
	)
	var surface_tool := SurfaceTool.new()
	surface_tool.begin(Mesh.PRIMITIVE_TRIANGLES)
	var emitted_vertices: int = 0

	for z_index in range(z_interval_count):
		var z0 := float(z_index) / float(z_interval_count)
		var z1 := float(z_index + 1) / float(z_interval_count)
		var world_z0 := world_origin_z + z0 * world_size_z
		var world_z1 := world_origin_z + z1 * world_size_z
		for x_index in range(x_interval_count):
			var x0 := float(x_index) / float(x_interval_count)
			var x1 := float(x_index + 1) / float(x_interval_count)
			var world_x0 := world_origin_x + x0 * world_size_x
			var world_x1 := world_origin_x + x1 * world_size_x
			if not _water_cell_has_visible_depth(
				depth_data,
				sample_width,
				sample_height,
				texture_width,
				texture_height,
				inner_offset_x,
				inner_offset_z,
				x_index,
				z_index,
				x_interval_count,
				z_interval_count
			):
				continue
			var cell := PackedVector2Array([
				Vector2(world_x0, world_z0),
				Vector2(world_x1, world_z0),
				Vector2(world_x1, world_z1),
				Vector2(world_x0, world_z1),
			])
			var bin_index: int = z_index * x_interval_count + x_index
			var cell_clip_groups: Array = clip_bins.get(bin_index, [])
			emitted_vertices += _emit_clipped_water_cell(
				surface_tool,
				cell,
				cell_clip_groups,
				center_x,
				center_z,
				world_origin_x,
				world_origin_z,
				world_size_x,
				world_size_z
			)

	if emitted_vertices == 0:
		return ArrayMesh.new()
	return surface_tool.commit()

func _water_cell_has_visible_depth(
	depth_data: PackedFloat32Array,
	sample_width: int,
	sample_height: int,
	texture_width: int,
	texture_height: int,
	inner_offset_x: int,
	inner_offset_z: int,
	x_index: int,
	z_index: int,
	x_interval_count: int,
	z_interval_count: int
) -> bool:
	const MIN_VISIBLE_DEPTH_M := 0.001
	if (
		depth_data.is_empty()
		or sample_width <= 0
		or sample_height <= 0
		or texture_width <= 0
		or texture_height <= 0
	):
		return false
	var max_sample_x: int = max(0, sample_width - 1)
	var max_sample_z: int = max(0, sample_height - 1)
	var start_sample_x: int = clampi(
		int(floor(float(x_index) / float(maxi(1, x_interval_count)) * float(max_sample_x))),
		0,
		max_sample_x
	)
	var end_sample_x: int = clampi(
		int(ceil(float(x_index + 1) / float(maxi(1, x_interval_count)) * float(max_sample_x))),
		0,
		max_sample_x
	)
	var start_sample_z: int = clampi(
		int(floor(float(z_index) / float(maxi(1, z_interval_count)) * float(max_sample_z))),
		0,
		max_sample_z
	)
	var end_sample_z: int = clampi(
		int(ceil(float(z_index + 1) / float(maxi(1, z_interval_count)) * float(max_sample_z))),
		0,
		max_sample_z
	)
	for sample_z in range(start_sample_z, end_sample_z + 1):
		var texture_z: int = clampi(inner_offset_z + sample_z, 0, texture_height - 1)
		var row_offset: int = texture_z * texture_width
		for sample_x in range(start_sample_x, end_sample_x + 1):
			var texture_x: int = clampi(inner_offset_x + sample_x, 0, texture_width - 1)
			var sample_index: int = row_offset + texture_x
			if sample_index >= 0 and sample_index < depth_data.size() and depth_data[sample_index] > MIN_VISIBLE_DEPTH_M:
				return true
	return false

func _emit_clipped_water_cell(
	surface_tool: SurfaceTool,
	cell: PackedVector2Array,
	clip_groups: Array,
	center_x: float,
	center_z: float,
	world_origin_x: float,
	world_origin_z: float,
	world_size_x: float,
	world_size_z: float
) -> int:
	if clip_groups.is_empty():
		return _emit_unclipped_water_cell(
			surface_tool,
			cell,
			center_x,
			center_z,
			world_origin_x,
			world_origin_z,
			world_size_x,
			world_size_z
		)

	var cell_bounds := _polygon_bounds(cell)
	for clip_group in clip_groups:
		if _cell_touches_road_clip_group(cell, cell_bounds, clip_group):
			return 0

	return _emit_unclipped_water_cell(
		surface_tool,
		cell,
		center_x,
		center_z,
		world_origin_x,
		world_origin_z,
		world_size_x,
		world_size_z
	)

func _cell_touches_road_clip_group(
	cell: PackedVector2Array,
	cell_bounds: Rect2,
	clip_group: Dictionary
) -> bool:
	var group_bounds: Rect2 = clip_group["bounds"]
	if not _bounds_intersect(cell_bounds, group_bounds):
		return false
	if _cell_fully_inside_any_road_clip_hole(cell, clip_group):
		return false
	for sample_variant in _cell_road_clip_samples(cell):
		var sample: Vector2 = sample_variant
		if _point_in_road_clip_group(sample, clip_group):
			return true
	var outer_loops: Array = clip_group["outer_loops"]
	for outer_variant in outer_loops:
		var outer: Dictionary = outer_variant
		var outer_points: PackedVector2Array = outer["points"]
		var outer_bounds: Rect2 = outer["bounds"]
		if _cell_fully_inside_polygon(cell, outer_points):
			return true
		if _polygon_intersects_cell(outer_points, outer_bounds, cell, cell_bounds):
			return true
	return false

func _cell_road_clip_samples(cell: PackedVector2Array) -> Array:
	return [
		cell[0],
		cell[1],
		cell[2],
		cell[3],
		(cell[0] + cell[2]) * 0.5,
	]

func _cell_fully_inside_any_road_clip_hole(
	cell: PackedVector2Array,
	clip_group: Dictionary
) -> bool:
	var hole_loops: Array = clip_group["hole_loops"]
	for hole_variant in hole_loops:
		var hole: Dictionary = hole_variant
		if _cell_fully_inside_polygon(cell, hole["points"]):
			return true
	return false

func _point_in_road_clip_group(point: Vector2, clip_group: Dictionary) -> bool:
	var inside_outer := false
	var outer_loops: Array = clip_group["outer_loops"]
	for outer_variant in outer_loops:
		var outer: Dictionary = outer_variant
		var outer_points: PackedVector2Array = outer["points"]
		if Geometry2D.is_point_in_polygon(point, outer_points) or _point_on_polygon_boundary(point, outer_points):
			inside_outer = true
			break
	if not inside_outer:
		return false
	var hole_loops: Array = clip_group["hole_loops"]
	for hole_variant in hole_loops:
		var hole: Dictionary = hole_variant
		var hole_points: PackedVector2Array = hole["points"]
		if _point_on_polygon_boundary(point, hole_points):
			return true
		if Geometry2D.is_point_in_polygon(point, hole_points):
			return false
	return true

func _emit_unclipped_water_cell(
	surface_tool: SurfaceTool,
	cell: PackedVector2Array,
	center_x: float,
	center_z: float,
	world_origin_x: float,
	world_origin_z: float,
	world_size_x: float,
	world_size_z: float
) -> int:
	_add_clipped_water_vertex(surface_tool, cell[0], center_x, center_z, world_origin_x, world_origin_z, world_size_x, world_size_z)
	_add_clipped_water_vertex(surface_tool, cell[2], center_x, center_z, world_origin_x, world_origin_z, world_size_x, world_size_z)
	_add_clipped_water_vertex(surface_tool, cell[1], center_x, center_z, world_origin_x, world_origin_z, world_size_x, world_size_z)
	_add_clipped_water_vertex(surface_tool, cell[0], center_x, center_z, world_origin_x, world_origin_z, world_size_x, world_size_z)
	_add_clipped_water_vertex(surface_tool, cell[3], center_x, center_z, world_origin_x, world_origin_z, world_size_x, world_size_z)
	_add_clipped_water_vertex(surface_tool, cell[2], center_x, center_z, world_origin_x, world_origin_z, world_size_x, world_size_z)
	return 6

func _add_clipped_water_vertex(
	surface_tool: SurfaceTool,
	world_xz: Vector2,
	center_x: float,
	center_z: float,
	world_origin_x: float,
	world_origin_z: float,
	world_size_x: float,
	world_size_z: float
) -> void:
	var uv := Vector2(
		clampf((world_xz.x - world_origin_x) / maxf(world_size_x, 0.001), 0.0, 1.0),
		clampf((world_xz.y - world_origin_z) / maxf(world_size_z, 0.001), 0.0, 1.0)
	)
	surface_tool.set_normal(Vector3.UP)
	surface_tool.set_uv(uv)
	surface_tool.add_vertex(Vector3(world_xz.x - center_x, 0.0, world_xz.y - center_z))

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

func _bounds_intersect(a: Rect2, b: Rect2) -> bool:
	return (
		a.position.x <= b.position.x + b.size.x
		and a.position.x + a.size.x >= b.position.x
		and a.position.y <= b.position.y + b.size.y
		and a.position.y + a.size.y >= b.position.y
	)

func _polygon_intersects_cell(
	polygon: PackedVector2Array,
	polygon_bounds: Rect2,
	cell: PackedVector2Array,
	cell_bounds: Rect2
) -> bool:
	if not _bounds_intersect(cell_bounds, polygon_bounds):
		return false
	for cell_point in cell:
		if Geometry2D.is_point_in_polygon(cell_point, polygon):
			return true
	for polygon_point in polygon:
		if _point_in_bounds(polygon_point, cell_bounds):
			return true
	for polygon_index in range(polygon.size()):
		var polygon_a := polygon[polygon_index]
		var polygon_b := polygon[(polygon_index + 1) % polygon.size()]
		for cell_index in range(cell.size()):
			var cell_a := cell[cell_index]
			var cell_b := cell[(cell_index + 1) % cell.size()]
			if _segments_intersect(polygon_a, polygon_b, cell_a, cell_b):
				return true
	return false

func _cell_fully_inside_polygon(cell: PackedVector2Array, polygon: PackedVector2Array) -> bool:
	for point in cell:
		if Geometry2D.is_point_in_polygon(point, polygon):
			continue
		if _point_on_polygon_boundary(point, polygon):
			continue
		return false
	return true

func _point_on_polygon_boundary(point: Vector2, polygon: PackedVector2Array) -> bool:
	for index in range(polygon.size()):
		if _point_on_segment(point, polygon[index], polygon[(index + 1) % polygon.size()]):
			return true
	return false

func _point_in_bounds(point: Vector2, bounds: Rect2) -> bool:
	const EPSILON := 0.01
	return (
		point.x >= bounds.position.x - EPSILON
		and point.x <= bounds.position.x + bounds.size.x + EPSILON
		and point.y >= bounds.position.y - EPSILON
		and point.y <= bounds.position.y + bounds.size.y + EPSILON
	)

func _segments_intersect(a: Vector2, b: Vector2, c: Vector2, d: Vector2) -> bool:
	var ab := b - a
	var cd := d - c
	var denom := _cross_2d(ab, cd)
	var ca := c - a
	if absf(denom) <= 0.00001:
		return _point_on_segment(c, a, b) or _point_on_segment(d, a, b) or _point_on_segment(a, c, d) or _point_on_segment(b, c, d)
	var t := _cross_2d(ca, cd) / denom
	var u := _cross_2d(ca, ab) / denom
	return t >= -0.00001 and t <= 1.00001 and u >= -0.00001 and u <= 1.00001

func _point_on_segment(point: Vector2, a: Vector2, b: Vector2) -> bool:
	var ab := b - a
	var ap := point - a
	if absf(_cross_2d(ab, ap)) > 0.00001:
		return false
	var dot := ap.dot(ab)
	if dot < -0.00001:
		return false
	return dot <= ab.length_squared() + 0.00001

func _cross_2d(a: Vector2, b: Vector2) -> float:
	return a.x * b.y - a.y * b.x

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
	var patch_data: Dictionary = simulation_node.get_water_patch(key.x, key.y)
	if patch_data.is_empty():
		return
	patch["lod_step"] = target_lod_step
	patch["last_patch_data"] = patch_data
	patch["depth_nonzero_count"] = _patch_visible_depth_count(patch_data)
	patch["road_clip_signature"] = _patch_road_clip_signature(patch_data)
	if _patch_visible_depth_count(patch_data) > 0:
		patch_node.mesh = _water_patch_mesh_from_data(patch_data, target_lod_step)
	else:
		patch_node.mesh = ArrayMesh.new()

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
