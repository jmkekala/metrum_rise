# SPDX-License-Identifier: GPL-2.0-only

## Farm vertex editor. Rust validates and commits each released drag atomically.
## Draws the active field and farm boundaries; production and land reservations stay in Rust.
extends Node3D

signal field_changed(building_id: int, details: Dictionary)

const PICK_RADIUS_PX := 14.0
const HELP := "Drag a field vertex. Release to apply. Esc / right-click to finish."
const ProductionPlotOverlay := preload("res://scripts/tools/production_plot_overlay.gd")

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var active := false
var _building_id := -1
var _center := Vector2.ZERO
var _committed := PackedVector2Array()
var _preview := PackedVector2Array()
var _dragged := -1
var _mouse_dirty := false
var _valid := true
var _outline := ImmediateMesh.new()
var _material := StandardMaterial3D.new()
var _handles: Array[MeshInstance3D] = []
var _hint := Label.new()
var _plot_overlay := ProductionPlotOverlay.new()

func _ready() -> void:
	add_child(_plot_overlay)
	_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_material.no_depth_test = true
	var mesh_instance := MeshInstance3D.new()
	mesh_instance.mesh = _outline
	mesh_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(mesh_instance)
	var layer := CanvasLayer.new()
	layer.layer = 90
	add_child(layer)
	_hint.position = Vector2(24, 100)
	_hint.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_hint.add_theme_color_override("font_shadow_color", Color.BLACK)
	_hint.add_theme_constant_override("shadow_offset_x", 1)
	_hint.add_theme_constant_override("shadow_offset_y", 1)
	layer.add_child(_hint)
	cancel_edit()

func begin_edit(info: Dictionary) -> void:
	cancel_edit()
	_building_id = int(info.get("building_id", -1))
	_center = Vector2(float(info.get("center_x", 0.0)), float(info.get("center_z", 0.0)))
	_committed = info.get("field_polygon", PackedVector2Array()).duplicate()
	if _building_id < 0 or _committed.size() < 3:
		return
	_preview = _committed.duplicate()
	active = true
	_valid = true
	_hint.text = HELP + "\n" + ProductionPlotOverlay.LEGEND
	_plot_overlay.show_boundaries(info)
	_hint.show()
	var sphere := SphereMesh.new()
	sphere.radius = 1.0
	sphere.height = 2.0
	sphere.radial_segments = 12
	sphere.rings = 6
	for _point in _preview:
		var handle := MeshInstance3D.new()
		handle.mesh = sphere
		handle.material_override = _material
		handle.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		add_child(handle)
		_handles.append(handle)
	_redraw()

func cancel_edit() -> void:
	_plot_overlay.clear_boundaries()
	active = false
	_dragged = -1
	_mouse_dirty = false
	_outline.clear_surfaces()
	for handle in _handles:
		handle.hide()
		handle.queue_free()
	_handles.clear()
	_hint.hide()

func _unhandled_input(event: InputEvent) -> void:
	if not active or not event is InputEventMouseButton:
		return
	if event.button_index != MOUSE_BUTTON_LEFT or not event.pressed:
		return
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	var best_distance := PICK_RADIUS_PX
	for index in _handles.size():
		var point := _handles[index].global_position
		if camera.is_position_behind(point):
			continue
		var distance := camera.unproject_position(point).distance_to(event.position)
		if distance < best_distance:
			best_distance = distance
			_dragged = index
	if _dragged >= 0:
		get_viewport().set_input_as_handled()

func _input(event: InputEvent) -> void:
	if not active or _dragged < 0:
		return
	if event is InputEventMouseMotion:
		_mouse_dirty = true
	elif event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and not event.pressed:
		# Receive release even over UI, so a drag cannot get stuck behind a window.
		_mouse_dirty = true
		_update_drag()
		_release_drag()
		get_viewport().set_input_as_handled()

func _process(_delta: float) -> void:
	if not active:
		return
	if _mouse_dirty:
		_update_drag()
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	# Keep handles pickable when zooming; this loops over only the edited vertices.
	var viewport_height := maxf(get_viewport().get_visible_rect().size.y, 1.0)
	for handle in _handles:
		var world_height := camera.size
		if camera.projection == Camera3D.PROJECTION_PERSPECTIVE:
			world_height = 2.0 * camera.global_position.distance_to(handle.global_position) * tan(deg_to_rad(camera.fov * 0.5))
		handle.scale = Vector3.ONE * maxf(world_height * 6.0 / viewport_height, 0.1)

func _update_drag() -> void:
	_mouse_dirty = false
	var camera := get_viewport().get_camera_3d()
	if camera == null or _dragged < 0:
		return
	var mouse := get_viewport().get_mouse_position()
	var hit = simulation_node.intersect_world_surface(camera.project_ray_origin(mouse), camera.project_ray_normal(mouse))
	if hit == null:
		return
	var point := Vector2(hit.x, hit.z)
	if _preview[_dragged] == point:
		return
	_preview[_dragged] = point
	var validation: Dictionary = simulation_node.validate_field_polygon(_building_id, _preview)
	var pending := bool(validation.get("pending", false))
	_valid = pending or bool(validation.get("ok", false))
	_hint.text = (HELP if _valid and not pending else str(validation.get("error", "Invalid field"))) + "\n" + ProductionPlotOverlay.LEGEND
	_redraw()

func _release_drag() -> void:
	_dragged = -1
	if _preview == _committed:
		return
	var result: Dictionary = simulation_node.resize_field_polygon(_building_id, _center, _committed, _preview)
	if bool(result.get("ok", false)):
		_committed = _preview.duplicate()
		_hint.text = HELP + "\n" + ProductionPlotOverlay.LEGEND
		terrain_node.mark_field_overlay_dirty()
		field_changed.emit(_building_id, result)
	else:
		_preview = _committed.duplicate()
		_hint.text = "Move rejected: " + str(result.get("error", "Invalid field")) + "\n" + ProductionPlotOverlay.LEGEND
	_valid = true
	_redraw()

func _redraw() -> void:
	_material.albedo_color = Color(0.95, 0.8, 0.2) if _valid else Color(1.0, 0.2, 0.15)
	_outline.clear_surfaces()
	_outline.surface_begin(Mesh.PRIMITIVE_LINES, _material)
	for index in _preview.size():
		var point := _preview[index]
		_handles[index].position = Vector3(point.x, simulation_node.get_world_surface_height(point) + 0.3, point.y)
	for index in _handles.size():
		_outline.surface_add_vertex(_handles[index].position)
		_outline.surface_add_vertex(_handles[(index + 1) % _handles.size()].position)
	_outline.surface_end()
