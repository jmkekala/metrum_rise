## Dedicated bulldoze tool for deleting one building or road target per click.
##
## Rust methods called: get_bulldoze_target_at(), bulldoze_at(), intersect_world_surface()
extends Node3D

@onready var simulation_node = $"../SimulationNode"

var active: bool = false

var hover_mesh: MeshInstance3D
var _target_valid := false
var _target_world_pos := Vector2.ZERO

func _ready() -> void:
	hover_mesh = MeshInstance3D.new()
	hover_mesh.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	hover_mesh.top_level = true
	hover_mesh.visible = false
	hover_mesh.material_override = _make_hover_material()
	add_child(hover_mesh)

func _process(_delta: float) -> void:
	if not active:
		_clear_hover()
		return
	_update_hover()

func _unhandled_input(event) -> void:
	if not active:
		return
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		_delete_hovered_target()
		get_viewport().set_input_as_handled()

func _update_hover() -> void:
	var wp = _mouse_world_pos()
	if wp == null:
		_clear_hover()
		return
	_target_world_pos = wp
	var target: Dictionary = simulation_node.get_bulldoze_target_at(wp.x, wp.y)
	if not bool(target.get("valid", false)):
		_clear_hover()
		return
	var mesh := _build_hover_mesh(target)
	if mesh == null:
		_clear_hover()
		return
	_target_valid = true
	hover_mesh.mesh = mesh
	hover_mesh.visible = true

func _delete_hovered_target() -> void:
	if not _target_valid:
		return
	var result: Dictionary = simulation_node.bulldoze_at(_target_world_pos.x, _target_world_pos.y)
	if not bool(result.get("queued", false)):
		_clear_hover()
		return
	_clear_hover()

func _clear_hover() -> void:
	_target_valid = false
	if hover_mesh:
		hover_mesh.mesh = null
		hover_mesh.visible = false

func _build_hover_mesh(target: Dictionary) -> Mesh:
	var kind := str(target.get("kind", ""))
	var points: PackedVector3Array = target.get("points", PackedVector3Array())
	if points.size() < 2:
		return null
	if kind == "building":
		return _build_polygon_mesh(points)
	if kind == "road":
		var width_m := float(target.get("width_m", 0.0))
		return _build_road_strip_mesh(points, width_m)
	return null

func _build_polygon_mesh(points: PackedVector3Array) -> Mesh:
	if points.size() < 3:
		return null
	var im := ImmediateMesh.new()
	var fill := Color(1.0, 0.08, 0.03, 0.34)
	var line := Color(1.0, 0.16, 0.08, 0.90)
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	im.surface_set_color(fill)
	for i in range(1, points.size() - 1):
		im.surface_add_vertex(points[0])
		im.surface_add_vertex(points[i])
		im.surface_add_vertex(points[i + 1])
	im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(line)
	for i in range(points.size()):
		im.surface_add_vertex(points[i])
		im.surface_add_vertex(points[(i + 1) % points.size()])
	im.surface_end()
	return im

func _build_road_strip_mesh(points: PackedVector3Array, width_m: float) -> Mesh:
	if points.size() < 2 or width_m <= 0.0:
		return null
	var im := ImmediateMesh.new()
	var half_width := width_m * 0.5
	var fill := Color(1.0, 0.08, 0.03, 0.30)
	var line := Color(1.0, 0.16, 0.08, 0.86)
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	im.surface_set_color(fill)
	for i in range(points.size() - 1):
		var quad := _segment_quad(points[i], points[i + 1], half_width)
		if quad.is_empty():
			continue
		im.surface_add_vertex(quad[0])
		im.surface_add_vertex(quad[1])
		im.surface_add_vertex(quad[2])
		im.surface_add_vertex(quad[0])
		im.surface_add_vertex(quad[2])
		im.surface_add_vertex(quad[3])
	im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(line)
	for i in range(points.size() - 1):
		var quad := _segment_quad(points[i], points[i + 1], half_width)
		if quad.is_empty():
			continue
		for j in range(4):
			im.surface_add_vertex(quad[j])
			im.surface_add_vertex(quad[(j + 1) % 4])
	im.surface_end()
	return im

func _segment_quad(a: Vector3, b: Vector3, half_width: float) -> Array[Vector3]:
	var dir := Vector2(b.x - a.x, b.z - a.z)
	if dir.length_squared() <= 0.0001:
		return []
	dir = dir.normalized()
	var normal := Vector2(-dir.y, dir.x) * half_width
	return [
		Vector3(a.x + normal.x, a.y, a.z + normal.y),
		Vector3(b.x + normal.x, b.y, b.z + normal.y),
		Vector3(b.x - normal.x, b.y, b.z - normal.y),
		Vector3(a.x - normal.x, a.y, a.z - normal.y),
	]

func _make_hover_material() -> StandardMaterial3D:
	var mat := StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.vertex_color_use_as_albedo = true
	mat.no_depth_test = false
	mat.render_priority = 8
	return mat

func _mouse_world_pos() -> Variant:
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if camera == null:
		return null
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var hit = simulation_node.intersect_world_surface(ray_origin, ray_dir)
	if hit == null:
		return null
	return Vector2(hit.x, hit.z)
