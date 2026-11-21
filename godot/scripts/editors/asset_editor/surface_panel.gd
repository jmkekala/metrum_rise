# SPDX-License-Identifier: GPL-3.0-or-later

## The selected site surface's vertices as numbers (`TOOLS-40`): its vertices listed, the selected
## one's X and Z editable, and the whole outline set to a rectangle by two corners, each change one
## undo step through the document's "set_vertex" and "set_rectangle" commands. Rust refuses, as a
## drag is held back, a vertex off the lot or an outline that crosses itself; this panel only edits
## and lists. Dragging, inserting and deleting vertices in the view work as before.
extends RefCounted

const TaskPanels = preload("res://scripts/editors/asset_editor/task_panels.gd")

var selected := -1
var list: ItemList
var _editor: Node
var _root: VBoxContainer
var _x: SpinBox
var _z: SpinBox
var _point_fields: Control
# The rectangle's corners: first X, first Z, opposite X, opposite Z.
var _corners: Array[SpinBox] = []
var _updating := false
var _surface := -1

func build(view: RefCounted, parent: Control) -> void:
	_editor = view._editor
	_root = VBoxContainer.new()
	parent.add_child(_root)
	var box: VBoxContainer = view.advanced(_root, "surface_vertices", "Vertices as numbers")
	list = ItemList.new()
	list.custom_minimum_size.y = 72
	list.item_selected.connect(func(index):
		selected = index
		refresh())
	box.add_child(list)
	_point_fields = VBoxContainer.new()
	box.add_child(_point_fields)
	_x = _coordinate(view, _point_fields, "_surface_vertex_x", "Vertex X (m)")
	_z = _coordinate(view, _point_fields, "_surface_vertex_z", "Vertex Z (m)")
	for spin in [_x, _z]:
		spin.value_changed.connect(func(_value):
			if not _updating and selected >= 0:
				# The edited coordinate only; the other keeps its stored value.
				var point: Array = (vertices()[selected] as Array).duplicate()
				point[0 if spin == _x else 1] = spin.value
				set_vertex(selected, point))
	var hint := Label.new()
	hint.text = "Rectangle by two opposite corners: replaces the outline with four vertices."
	hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	box.add_child(hint)
	for title in ["Corner X (m)", "Corner Z (m)", "Opposite corner X (m)", "Opposite corner Z (m)"]:
		_corners.append(_coordinate(view, box, "_surface_rect_%d" % _corners.size(), title))
	view.button(box, "Set rectangle", func():
		set_rectangle([_corners[0].value, _corners[1].value], [_corners[2].value, _corners[3].value]))
	_root.visible = false

func _coordinate(view: RefCounted, parent: Control, key: String, title: String) -> SpinBox:
	var spin: SpinBox = view.number_field(parent, key, title, 500, TaskPanels.POSITION_STEP_M)
	spin.min_value = -500
	spin.allow_lesser = true
	spin.custom_arrow_step = TaskPanels.POSITION_ARROW_STEP_M
	return spin

## The selected surface's vertices as the editor projects the document.
func vertices() -> Array:
	var index: int = _editor._selected_site_surface_index
	var surfaces: Array = _editor._site_surfaces_data
	if index < 0 or index >= surfaces.size():
		return []
	return surfaces[index].get("vertices", [])

## Fill the list, the selected vertex's fields and, for a newly selected surface, the rectangle
## corners from its bounds.
func refresh() -> void:
	if _root == null:
		return
	var points := vertices()
	_root.visible = not points.is_empty()
	if _surface != _editor._selected_site_surface_index:
		_surface = _editor._selected_site_surface_index
		selected = -1
		_fill_corners(points)
	_updating = true
	list.clear()
	for index in points.size():
		list.add_item("%d   x %.4f   z %.4f" % [index + 1, float(points[index][0]), float(points[index][1])])
	selected = clampi(selected, -1, points.size() - 1)
	_point_fields.visible = selected >= 0
	if selected >= 0:
		list.select(selected)
		_x.value = float(points[selected][0])
		_z.value = float(points[selected][1])
	_updating = false

func _fill_corners(points: Array) -> void:
	if points.is_empty():
		return
	var low := Vector2(INF, INF)
	var high := Vector2(-INF, -INF)
	for point in points:
		var p := Vector2(float(point[0]), float(point[1]))
		low = low.min(p)
		high = high.max(p)
	for slot in 4:
		_corners[slot].set_value_no_signal([low.x, low.y, high.x, high.y][slot])

## Put vertex `index` at `point` (`[x, z]`), one undo step.
func set_vertex(index: int, point: Array) -> bool:
	return _edit("set_vertex", {"vertex": index, "point": point}, "Move vertex")

## Make the outline the rectangle with corners `a` and `b` (`[x, z]`), one undo step.
func set_rectangle(a: Array, b: Array) -> bool:
	return _edit("set_rectangle", {"a": a, "b": b}, "Set rectangle")

func _edit(action: String, args: Dictionary, label: String) -> bool:
	var targets: Array[Dictionary] = [{"kind": "surface", "index": _editor._selected_site_surface_index}]
	var document = _editor._session.document
	var result: Dictionary = document.prepare_edit(action, targets, args)
	if result.has("error"):
		_editor._session.message(str(result.error))
		refresh()
		return false
	document.apply(result.document, label)
	refresh()
	return true
