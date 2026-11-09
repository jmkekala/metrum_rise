# SPDX-License-Identifier: GPL-2.0-only

## Dedicated bulldoze tool for deleting one building, road or vegetation target per click. A
## roundabout goes whole (`ROAD-62`, D15): Rust's target is its whole ring and names it, and the
## name shows beside the pointer. The hover keeps its last target while the simulation thread
## holds the core; the click finds what is under the pointer itself and waits for the core, so
## it is never dropped (`CODE-25`).
##
## Rust methods called: get_bulldoze_target_at(), bulldoze_at(), hover_world_surface(),
## intersect_world_surface()
extends Node3D

@onready var simulation_node = $"../SimulationNode"

var active: bool = false

var hover_mesh: MeshInstance3D
var _hover_label: Label

func _ready() -> void:
	var canvas := CanvasLayer.new()
	add_child(canvas)
	_hover_label = Label.new()
	_hover_label.visible = false
	_hover_label.add_theme_color_override("font_shadow_color", Color.BLACK)
	_hover_label.add_theme_constant_override("shadow_outline_size", 4)
	canvas.add_child(_hover_label)
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
		_delete_at_pointer()
		get_viewport().set_input_as_handled()

func _update_hover() -> void:
	var wp = _mouse_world_pos()
	# The simulation thread holds the core: keep the last hover.
	if wp is bool:
		return
	if wp == null:
		_clear_hover()
		return
	var target: Dictionary = simulation_node.get_bulldoze_target_at(wp.x, wp.y)
	if bool(target.get("busy", false)):
		return
	if not bool(target.get("valid", false)):
		_clear_hover()
		return
	var mesh := _build_hover_mesh(target)
	if mesh == null:
		_clear_hover()
		return
	hover_mesh.mesh = mesh
	hover_mesh.visible = true
	_hover_label.text = str(target.get("label", ""))
	_hover_label.visible = not _hover_label.text.is_empty()
	_hover_label.position = get_viewport().get_mouse_position() + Vector2(16, 16)

# Deletes what is under the pointer at the click, not the last hover's target, which may be
# older while the simulation thread holds the core; both queries wait for the core.
func _delete_at_pointer() -> void:
	var wp = _mouse_world_pos(true)
	if wp != null:
		simulation_node.bulldoze_at(wp.x, wp.y, true)
	_clear_hover()

func _clear_hover() -> void:
	if _hover_label:
		_hover_label.visible = false
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
	if kind == "vegetation":
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

# The ground point under the pointer, null for none. The click's query waits for the core; the
# hover's answers false while the simulation thread holds it.
func _mouse_world_pos(click := false) -> Variant:
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if camera == null:
		return null
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var hit = (simulation_node.intersect_world_surface(ray_origin, ray_dir, true) if click
		else simulation_node.hover_world_surface(ray_origin, ray_dir))
	if hit == null or hit is bool:
		return hit
	return Vector2(hit.x, hit.z)
