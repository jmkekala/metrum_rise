## Zone overlay -- builds a mesh from Rust-authored road-aligned parcel geometry.
##
## Rust methods called: get_zoning_overlay_revision(), get_zoning_overlay_occupancy_revision(),
##   try_get_zoning_parcels_overlay_packed(), try_get_no_building_spawn_lines()
extends MeshInstance3D

const PerfDebug := preload("res://scripts/core/perf_debug.gd")

@onready var simulation_node = $"../SimulationNode"

var _tool_active: float = 0.0
var _tool_active_target: float = 0.0
const FADE_SPEED: float = 6.0

var _zone_dirty: bool = true
var _no_build_dirty: bool = true
var _parcel_debug_count: int = 0
var _zone_revision_seen: int = -1
var _zone_occupancy_revision_seen: int = -1

var _no_build_mesh_instance: MeshInstance3D = null

func _ready():
	cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var mat := StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.vertex_color_use_as_albedo = true
	mat.no_depth_test = true
	material_override = mat
	position = Vector3.ZERO
	visible = false

func _process(delta):
	var overlay_requested := _overlay_requested()
	if not PerfDebug.is_enabled():
		if abs(_tool_active - _tool_active_target) > 0.001:
			_tool_active = move_toward(_tool_active, _tool_active_target, FADE_SPEED * delta)
		visible = _tool_active > 0.001 or _tool_active_target > 0.0
		if overlay_requested:
			_refresh_zone_dirty_from_revision()

		if overlay_requested and _zone_dirty:
			_zone_dirty = not _rebuild_parcel_overlay()
		if overlay_requested and _no_build_dirty:
			_no_build_dirty = not _rebuild_no_build_overlay()
		elif _no_build_mesh_instance and not overlay_requested:
			_no_build_mesh_instance.visible = false
		return

	var frame_start_us := Time.get_ticks_usec()
	var fade_start_us := frame_start_us
	if abs(_tool_active - _tool_active_target) > 0.001:
		_tool_active = move_toward(_tool_active, _tool_active_target, FADE_SPEED * delta)
	visible = _tool_active > 0.001 or _tool_active_target > 0.0
	var fade_elapsed_ms := float(Time.get_ticks_usec() - fade_start_us) / 1000.0
	var revision_elapsed_ms := 0.0
	if overlay_requested:
		var revision_start_us := Time.get_ticks_usec()
		_refresh_zone_dirty_from_revision()
		revision_elapsed_ms = float(Time.get_ticks_usec() - revision_start_us) / 1000.0

	var parcel_elapsed_ms := 0.0
	if overlay_requested and _zone_dirty:
		var parcel_start_us := Time.get_ticks_usec()
		var parcel_rebuilt := _rebuild_parcel_overlay()
		parcel_elapsed_ms = float(Time.get_ticks_usec() - parcel_start_us) / 1000.0
		_zone_dirty = not parcel_rebuilt
	var no_build_elapsed_ms := 0.0
	if overlay_requested and _no_build_dirty:
		var no_build_start_us := Time.get_ticks_usec()
		var no_build_rebuilt := _rebuild_no_build_overlay()
		no_build_elapsed_ms = float(Time.get_ticks_usec() - no_build_start_us) / 1000.0
		_no_build_dirty = not no_build_rebuilt
	elif _no_build_mesh_instance and not overlay_requested:
		_no_build_mesh_instance.visible = false
	PerfDebug.record(
		"zoning",
		float(Time.get_ticks_usec() - frame_start_us) / 1000.0,
		{
			"fade": fade_elapsed_ms,
			"revision": revision_elapsed_ms,
			"parcel": parcel_elapsed_ms,
			"no_build": no_build_elapsed_ms,
		}
	)

func mark_zone_dirty():
	_zone_dirty = true

func mark_occupied_dirty():
	if _overlay_requested():
		_refresh_zone_dirty_from_revision()

func mark_no_build_dirty():
	_no_build_dirty = true

func set_tool_active(active: bool):
	_tool_active_target = 1.0 if active else 0.0
	if active:
		visible = true
		if mesh == null:
			_zone_dirty = true
		_no_build_dirty = true
	elif _no_build_mesh_instance:
		_no_build_mesh_instance.visible = false

func is_overlay_requested() -> bool:
	return _overlay_requested()

func full_refresh():
	_zone_dirty = true
	_no_build_dirty = true

func _overlay_requested() -> bool:
	return _tool_active > 0.001 or _tool_active_target > 0.0

func _refresh_zone_dirty_from_revision() -> void:
	var revision := int(simulation_node.get_zoning_overlay_revision())
	var occupancy_revision := int(simulation_node.get_zoning_overlay_occupancy_revision())
	if revision == _zone_revision_seen and occupancy_revision == _zone_occupancy_revision_seen:
		return
	_zone_revision_seen = revision
	_zone_occupancy_revision_seen = occupancy_revision
	_zone_dirty = true

func road_geometry_debug_patch_lines(_flat_pairs: PackedInt32Array) -> Array[String]:
	return [
		"zoning_overlay visible=%s tool_active=%.3f target=%.3f parcels=%d"
		% [str(visible), _tool_active, _tool_active_target, _parcel_debug_count]
	]

func _rebuild_parcel_overlay() -> bool:
	var payload: Dictionary = simulation_node.try_get_zoning_parcels_overlay_packed()
	if bool(payload.get("busy", true)):
		return false
	_zone_revision_seen = int(payload.get("revision", _zone_revision_seen))
	_parcel_debug_count = int(payload.get("parcel_count", 0))
	var triangle_vertices := payload.get("triangle_vertices", PackedVector3Array()) as PackedVector3Array
	var triangle_colors := payload.get("triangle_colors", PackedColorArray()) as PackedColorArray
	var line_vertices := payload.get("line_vertices", PackedVector3Array()) as PackedVector3Array
	var line_colors := payload.get("line_colors", PackedColorArray()) as PackedColorArray
	_zone_occupancy_revision_seen = int(payload.get("occupancy_revision", _zone_occupancy_revision_seen))
	if triangle_vertices.is_empty() and line_vertices.is_empty():
		mesh = null
		return true

	var overlay_mesh := ArrayMesh.new()
	if triangle_vertices.size() >= 3 and triangle_vertices.size() == triangle_colors.size():
		var triangle_arrays := []
		triangle_arrays.resize(Mesh.ARRAY_MAX)
		triangle_arrays[Mesh.ARRAY_VERTEX] = triangle_vertices
		triangle_arrays[Mesh.ARRAY_COLOR] = triangle_colors
		overlay_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, triangle_arrays)
	if line_vertices.size() >= 2 and line_vertices.size() == line_colors.size():
		var line_arrays := []
		line_arrays.resize(Mesh.ARRAY_MAX)
		line_arrays[Mesh.ARRAY_VERTEX] = line_vertices
		line_arrays[Mesh.ARRAY_COLOR] = line_colors
		overlay_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_LINES, line_arrays)

	mesh = overlay_mesh if overlay_mesh.get_surface_count() > 0 else null
	return true

func _rebuild_no_build_overlay() -> bool:
	if not _no_build_mesh_instance:
		_no_build_mesh_instance = MeshInstance3D.new()
		_no_build_mesh_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		var mat := StandardMaterial3D.new()
		mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		mat.albedo_color = Color(1.0, 0.5, 0.0, 0.9)
		mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		mat.no_depth_test = true
		_no_build_mesh_instance.material_override = mat
		add_sibling(_no_build_mesh_instance)

	_no_build_mesh_instance.visible = _tool_active_target > 0.5

	var payload: Dictionary = simulation_node.try_get_no_building_spawn_lines()
	if bool(payload.get("busy", true)):
		return false
	var line_vertices := payload.get("line_vertices", PackedVector3Array()) as PackedVector3Array
	if line_vertices.is_empty():
		_no_build_mesh_instance.mesh = null
		return true

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	for point in line_vertices:
		im.surface_add_vertex(point)
	im.surface_end()
	_no_build_mesh_instance.mesh = im
	return true
