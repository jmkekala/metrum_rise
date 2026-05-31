## Zone overlay -- builds a mesh from Rust-authored road-aligned parcel geometry.
##
## Rust methods called: get_zoning_parcels_overlay(), get_no_building_spawn_edge_indices(),
##   get_edge_geometry_3d()
extends MeshInstance3D

@onready var simulation_node = $"../SimulationNode"

var _tool_active: float = 0.0
var _tool_active_target: float = 0.0
const FADE_SPEED: float = 6.0

var _zone_dirty: bool = true
var _no_build_dirty: bool = true
var _parcel_debug_count: int = 0

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
	if abs(_tool_active - _tool_active_target) > 0.001:
		_tool_active = move_toward(_tool_active, _tool_active_target, FADE_SPEED * delta)
	visible = _tool_active > 0.001 or _tool_active_target > 0.0

	if _zone_dirty:
		_rebuild_parcel_overlay()
		_zone_dirty = false
	if _no_build_dirty:
		_rebuild_no_build_overlay()
		_no_build_dirty = false

func mark_zone_dirty():
	_zone_dirty = true

func mark_occupied_dirty():
	_zone_dirty = true

func mark_distance_dirty():
	_no_build_dirty = true

func mark_no_build_dirty():
	_no_build_dirty = true

func set_tool_active(active: bool):
	_tool_active_target = 1.0 if active else 0.0
	if active:
		visible = true
	_no_build_dirty = true

func full_refresh():
	_zone_dirty = true
	_no_build_dirty = true

func road_geometry_debug_patch_lines(_flat_pairs: PackedInt32Array) -> Array[String]:
	return [
		"zoning_overlay visible=%s tool_active=%.3f target=%.3f parcels=%d"
		% [str(visible), _tool_active, _tool_active_target, _parcel_debug_count]
	]

func _rebuild_parcel_overlay():
	var payload: Array = simulation_node.get_zoning_parcels_overlay()
	_parcel_debug_count = payload.size()
	if payload.is_empty():
		mesh = null
		return

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	for entry in payload:
		if not (entry is Dictionary):
			continue
		var parcel: Dictionary = entry
		var corners: PackedVector3Array = parcel.get("corners", PackedVector3Array())
		if corners.size() != 4:
			continue
		var color: Color = parcel.get("color", Color(0.7, 0.9, 0.7, 0.34))
		im.surface_set_color(color)
		im.surface_add_vertex(corners[0])
		im.surface_add_vertex(corners[1])
		im.surface_add_vertex(corners[2])
		im.surface_add_vertex(corners[0])
		im.surface_add_vertex(corners[2])
		im.surface_add_vertex(corners[3])
	im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	for entry in payload:
		if not (entry is Dictionary):
			continue
		var parcel: Dictionary = entry
		var corners: PackedVector3Array = parcel.get("corners", PackedVector3Array())
		if corners.size() != 4:
			continue
		var color: Color = parcel.get("color", Color(0.7, 0.9, 0.7, 0.34))
		im.surface_set_color(Color(color.r, color.g, color.b, 0.9))
		for i in range(4):
			im.surface_add_vertex(corners[i])
			im.surface_add_vertex(corners[(i + 1) % 4])
	im.surface_end()

	mesh = im

func _rebuild_no_build_overlay():
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

	var indices: PackedInt32Array = simulation_node.get_no_building_spawn_edge_indices()
	if indices.is_empty():
		_no_build_mesh_instance.mesh = null
		return

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	for edge_idx in indices:
		var pts: PackedVector3Array = simulation_node.get_edge_geometry_3d(edge_idx)
		for i in range(pts.size() - 1):
			im.surface_add_vertex(pts[i])
			im.surface_add_vertex(pts[i + 1])
	im.surface_end()
	_no_build_mesh_instance.mesh = im
