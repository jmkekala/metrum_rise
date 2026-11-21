# SPDX-License-Identifier: GPL-3.0-or-later

## The selected loading bay's drive-through (`PARK-21`): its exit Driveway and the waypoints of
## its way in and way out, listed, added, inserted, moved and deleted through the document's
## "route" command, so each change is one undo step. The arc helper sends a radius, an angle and
## a side; Rust lays the chords. Rust proves the drive and the preview draws its result; this
## panel only edits and lists.
extends RefCounted

const PreviewGeometry = preload("res://scripts/editors/asset_editor/preview_geometry.gd")
const WAYS := {"in": "Way in", "out": "Way out"}
# A new waypoint's distance from the point it follows, along the bay: a starting place the
# author then moves.
const NEW_WAYPOINT_STEP_M := 4.0

var selected := -1
var way := "in"
var list: ItemList
var _editor: Node
var _root: VBoxContainer
var _exit: OptionButton
var _way: OptionButton
var _x: SpinBox
var _z: SpinBox
var _radius: SpinBox
var _angle: SpinBox
var _side: OptionButton
var _point_fields: Control
var _updating := false
var _anchor := -1

func build(view: RefCounted, parent: Control) -> void:
	_editor = view._editor
	_root = VBoxContainer.new()
	parent.add_child(_root)
	var box: VBoxContainer = view.advanced(_root, "drive_through", "Drive-through")
	var hint := Label.new()
	hint.text = "A truck drives in along the way in, stands in the bay, and leaves forward along the way out by the exit Driveway. Without an exit it backs out the way it came. Every bend over 15° needs room for the truck to turn: lay curves with Add arc."
	hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	box.add_child(hint)
	_exit = view.choice_field(box, "_route_exit", "Exit Driveway")
	_exit.item_selected.connect(func(index):
		if not _updating:
			_edit({"op": "exit", "value": str(_exit.get_item_metadata(index))}, "Set exit"))
	_way = view.choice_field(box, "_route_way", "Waypoints of", WAYS.keys())
	for index in _way.item_count:
		_way.set_item_text(index, WAYS[_way.get_item_metadata(index)])
	_way.item_selected.connect(func(index):
		way = str(_way.get_item_metadata(index))
		selected = -1
		refresh())
	var actions := HFlowContainer.new()
	box.add_child(actions)
	view.button(actions, "Add waypoint", add)
	view.button(actions, "Insert before", insert)
	view.button(actions, "Delete waypoint", delete)
	list = ItemList.new()
	list.custom_minimum_size.y = 72
	list.item_selected.connect(func(index):
		selected = index
		refresh())
	box.add_child(list)
	_point_fields = VBoxContainer.new()
	box.add_child(_point_fields)
	_x = _coordinate(view, "_route_x", "Waypoint X (m)")
	_z = _coordinate(view, "_route_z", "Waypoint Z (m)")
	_radius = view.number_field(box, "_route_radius", "Arc radius (m)", 200, 0.1)
	_radius.min_value = 0.5
	_radius.value = 7.5
	_angle = view.number_field(box, "_route_angle", "Arc angle (°)", 360, 1)
	_angle.min_value = 1
	_angle.allow_greater = false
	_angle.value = 90
	_side = view.choice_field(box, "_route_side", "Arc turns", ["left", "right"])
	view.button(box, "Add arc after the selected waypoint", add_arc)
	_root.visible = false

func _coordinate(view: RefCounted, key: String, title: String) -> SpinBox:
	var spin: SpinBox = view.number_field(_point_fields, key, title, 500, 0.01)
	spin.min_value = -500
	spin.allow_lesser = true
	spin.value_changed.connect(func(_value):
		if not _updating and selected >= 0:
			_edit({"op": "move", "way": way, "waypoint": selected, "point": [_x.value, _z.value]}, "Move waypoint"))
	return spin

## The selected anchor as the document holds it, or {} when it is not a loading bay.
func anchor() -> Dictionary:
	var index: int = _editor._selected_site_anchor_index
	var anchors: Array = _editor._session.document.snapshot().get("params", {}).get("anchors", [])
	if index < 0 or index >= anchors.size():
		return {}
	var entry: Dictionary = anchors[index]
	return entry if str(entry.get("anchor_type", "")) == "loading_bay" else {}

## The waypoints of the shown way.
func waypoints() -> Array:
	return anchor().get("route_in" if way == "in" else "route_out", [])

## Fill the exit choice, the list and the selected waypoint's fields from the document.
func refresh() -> void:
	if _root == null:
		return
	var bay := anchor()
	_root.visible = not bay.is_empty()
	if bay.is_empty():
		return
	if _anchor != _editor._selected_site_anchor_index:
		_anchor = _editor._selected_site_anchor_index
		selected = -1
	_updating = true
	# The exit names one of the asset's freight Driveways; whether it is usable is validation's.
	_exit.clear()
	_exit.add_item("None: back out")
	_exit.set_item_metadata(0, "")
	var exit := str(bay.get("exit", ""))
	for entry: Dictionary in _editor._session.document.snapshot()["params"].get("anchors", []):
		if str(entry.get("anchor_type", "")) == "driveway" and str(entry.get("vehicle_class", "")) == "freight":
			_exit.add_item(str(entry.get("name", "")))
			_exit.set_item_metadata(_exit.item_count - 1, str(entry.get("name", "")))
			if str(entry.get("name", "")) == exit:
				_exit.select(_exit.item_count - 1)
	if exit.is_empty():
		_exit.select(0)
	var points := waypoints()
	list.clear()
	for index in points.size():
		list.add_item("%d   x %.2f   z %.2f" % [index + 1, float(points[index][0]), float(points[index][1])])
	selected = clampi(selected, -1, points.size() - 1)
	_point_fields.visible = selected >= 0
	if selected >= 0:
		list.select(selected)
		_x.value = float(points[selected][0])
		_z.value = float(points[selected][1])
	_updating = false

## Add a waypoint after the selected one (or at the end), a step along the bay from it.
func add() -> void:
	var points := waypoints()
	var after := selected if selected >= 0 else points.size() - 1
	var at := after + 1
	if _edit({"op": "add", "way": way, "waypoint": at, "point": _near(points, after)}, "Add waypoint"):
		selected = at
		refresh()

## Insert a waypoint before the selected one, halfway from the point before it.
func insert() -> void:
	var points := waypoints()
	if selected < 0:
		add()
		return
	var point: Array = _near(points, selected - 1)
	if selected > 0:
		var a: Array = points[selected - 1]
		var b: Array = points[selected]
		point = [(float(a[0]) + float(b[0])) * 0.5, (float(a[1]) + float(b[1])) * 0.5]
	_edit({"op": "add", "way": way, "waypoint": selected, "point": point}, "Insert waypoint")
	refresh()

func delete() -> void:
	if selected >= 0 and _edit({"op": "delete", "way": way, "waypoint": selected}, "Delete waypoint"):
		selected -= 1
		refresh()

## Insert the chords of a turn after the selected waypoint (or the last), from the heading of the
## leg arriving there; Rust computes them.
func add_arc() -> void:
	var points := waypoints()
	var args := {"op": "arc", "way": way, "radius": _radius.value, "angle": _angle.value,
		"side": str(_side.get_item_metadata(_side.selected))}
	if selected >= 0:
		args["waypoint"] = selected
	var before := points.size()
	if _edit(args, "Add arc"):
		var after := selected if selected >= 0 else before - 1
		selected = after + waypoints().size() - before
		refresh()

# A new waypoint's starting place: a step along the bay from waypoint `index`, or from the bay's
# stop when the way has none before it.
func _near(points: Array, index: int) -> Array:
	var bay := anchor()
	var forward: Vector3 = PreviewGeometry.forward(bay)
	var step := forward * NEW_WAYPOINT_STEP_M * (1.0 if way == "out" else -1.0)
	var from: Vector3
	if index >= 0 and index < points.size():
		from = Vector3(float(points[index][0]), 0.0, float(points[index][1]))
	else:
		var position: Vector3 = PreviewGeometry.vector3(bay.get("position"))
		from = position + forward * PreviewGeometry.number(bay, "length_m", 5.0) * 0.5
	return [snappedf(from.x + step.x, 0.01), snappedf(from.z + step.z, 0.01)]

func _edit(args: Dictionary, label: String) -> bool:
	var targets: Array[Dictionary] = [{"kind": "anchor", "index": _editor._selected_site_anchor_index}]
	var document = _editor._session.document
	var result: Dictionary = document.prepare_edit("route", targets, args)
	if result.has("error"):
		_editor._session.message(str(result.error))
		refresh()
		return false
	document.apply(result.document, label)
	return true
