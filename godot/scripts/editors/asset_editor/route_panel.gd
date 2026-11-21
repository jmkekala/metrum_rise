# SPDX-License-Identifier: GPL-3.0-or-later

## The selected loading bay's drive-through (`PARK-21`): its exit Driveway and the waypoints of
## its way in and way out, listed, added, inserted, moved and deleted through the document's
## "route" command, so each change is one undo step. The arc helper sends a radius, an angle and
## a side; Rust lays the chords. Rust proves the drive and the preview draws its result; this
## panel only edits and lists. The editor view picks, drags, inserts and deletes the same
## waypoints through it: a drag shows live on the editor's projection and ends in one "move".
## Above the group, how a truck stands in the bay (`PARK-23`): nose in (it backs out) or tail in
## (it backs in and drives out), sent as the same command's "stand"; Rust refuses tail in with an
## exit.
## A selected parking spot (`PARK-26`) gets the same waypoint tools for its own way in, and no
## stand, exit or way out: "Edit this route" sends the command's "pin", which copies the route the
## lot check shows into `route_in`, and "Back to automatic" its "unpin", each one undo step.
extends RefCounted

const PreviewGeometry = preload("res://scripts/editors/asset_editor/preview_geometry.gd")
const WAYS := {"in": "Way in", "out": "Way out"}
const TRUCK_HINT := "A truck drives in along the way in, stands in the bay, and leaves forward along the way out by the exit Driveway. Without an exit it backs out the way it came. Every bend over 15° needs room for the truck to turn: lay curves with Add arc."
const CAR_HINT := "A car drives in from the street along this spot's route and leaves the same way. Automatic until you choose Edit this route, which makes the shown route the spot's own; its waypoints then move, insert and delete as a loading bay's. Every bend over 30° needs about 5 m by 5 m of paving: lay curves with Add arc. Back to automatic drops the spot's own route."
const GROUPS := {"loading_bay": "Drive-through", "parking": "Car route"}
const STANDS := {"nose_in": "Nose in: drives in, backs out", "tail_in": "Tail in: backs in, drives out"}
# A new waypoint's distance from the point it follows, along the bay: a starting place the
# author then moves.
const NEW_WAYPOINT_STEP_M := 4.0
# A view drag or insertion lands on the centimetre, as the editor's other handles do; the X and Z
# fields show it exactly.
const VIEW_SNAP_M := 0.01

var selected := -1
var way := "in"
var list: ItemList
var _editor: Node
var _root: VBoxContainer
var _stand: OptionButton
var _exit: OptionButton
var _way: OptionButton
var _x: SpinBox
var _z: SpinBox
var _radius: SpinBox
var _angle: SpinBox
var _side: OptionButton
var _point_fields: Control
var _tools: Control
var _hint: Label
var _own: Button
var _automatic: Button
var _group: CheckButton
var _rows: Dictionary
var _updating := false
var _anchor := -1
# The dragged waypoint's stored point before the drag, put back when the drag is cancelled.
var _drag_start: Array = []

func build(view: RefCounted, parent: Control) -> void:
	_editor = view._editor
	_root = VBoxContainer.new()
	parent.add_child(_root)
	_stand = view.choice_field(_root, "_route_stand", "Truck stands", STANDS.keys())
	for index in _stand.item_count:
		_stand.set_item_text(index, STANDS[_stand.get_item_metadata(index)])
	_stand.tooltip_text = "Tail in puts the truck's back at the bay's far end, at a loading door: it reverses in along the way in and drives out forward. A bay backed into has no exit."
	_stand.item_selected.connect(func(index):
		if not _updating:
			_edit({"op": "stand", "value": str(_stand.get_item_metadata(index))}, "Set stand"))
	var box: VBoxContainer = view.advanced(_root, "drive_through", "Drive-through")
	_group = view.advanced_groups["drive_through"]["toggle"]
	_rows = view.rows
	_hint = Label.new()
	_hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	box.add_child(_hint)
	var own := HFlowContainer.new()
	box.add_child(own)
	_own = view.button(own, "Edit this route", pin)
	_own.tooltip_text = "Make the route shown for this spot its own, to edit by its waypoints. Saved in the asset."
	_automatic = view.button(own, "Back to automatic", unpin)
	_automatic.tooltip_text = "Drop this spot's own route: the game routes it again."
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
	_tools = VBoxContainer.new()
	box.add_child(_tools)
	box = _tools
	var actions := HFlowContainer.new()
	box.add_child(actions)
	view.button(actions, "Add waypoint", add)
	view.button(actions, "Insert before", insert)
	view.button(actions, "Delete waypoint", delete)
	list = ItemList.new()
	list.custom_minimum_size.y = 72
	list.item_selected.connect(func(index):
		selected = index
		refresh()
		_editor._selection.context_changed())
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
	# 0.1 mm steps: a SpinBox rounds to its step, and the arc helper's chords are 0.1 mm.
	var spin: SpinBox = view.number_field(_point_fields, key, title, 500, 0.0001)
	spin.min_value = -500
	spin.allow_lesser = true
	spin.value_changed.connect(func(_value):
		if not _updating and selected >= 0:
			# The edited coordinate only; the other keeps its stored value.
			var point: Array = (waypoints()[selected] as Array).duplicate()
			point[0 if spin == _x else 1] = spin.value
			_edit({"op": "move", "way": way, "waypoint": selected, "point": point}, "Move waypoint"))
	return spin

## The selected anchor as the editor projects the document, or {} when it is neither a loading
## bay nor a parking spot. The projection equals the document except during a view drag, which
## the fields then follow.
func anchor() -> Dictionary:
	var index: int = _editor._selected_site_anchor_index
	var anchors: Array = _editor._site_anchors_data
	if index < 0 or index >= anchors.size():
		return {}
	var entry: Dictionary = anchors[index]
	return entry if GROUPS.has(str(entry.get("anchor_type", ""))) else {}

## Whether the selected anchor is a parking spot, whose route is a way in only.
func car() -> bool:
	return str(anchor().get("anchor_type", "")) == "parking"

## The waypoints of the shown way.
func waypoints() -> Array:
	return anchor().get("route_in" if way == "in" else "route_out", [])

## Fill the exit choice, the list and the selected waypoint's fields from the document.
func refresh() -> void:
	if _root == null:
		return
	var bay := anchor()
	_root.visible = not bay.is_empty()
	# Another bay, or the same one selected again, starts with no waypoint selected.
	var shown: int = -1 if bay.is_empty() else _editor._selected_site_anchor_index
	if _anchor != shown:
		_anchor = shown
		selected = -1
	if bay.is_empty():
		return
	_updating = true
	var spot := car()
	if spot:
		way = "in"
	var own: bool = spot and not bay.get("route_in", []).is_empty()
	_group.text = GROUPS[str(bay.get("anchor_type", ""))]
	_hint.text = CAR_HINT if spot else TRUCK_HINT
	for key in ["_route_stand", "_route_exit", "_route_way"]:
		_rows[key].visible = not spot
	_own.visible = spot and not own
	_automatic.visible = own
	_tools.visible = own or not spot
	var stand := str(bay.get("stand", "nose_in"))
	for index in _stand.item_count:
		if str(_stand.get_item_metadata(index)) == stand:
			_stand.select(index)
	# The exit names one of the asset's freight Driveways; whether it is usable is validation's.
	_exit.clear()
	_exit.add_item("None: back out")
	_exit.set_item_metadata(0, "")
	var exit := str(bay.get("exit", ""))
	for entry: Dictionary in _editor._site_anchors_data:
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

## "Edit this route": the spot's shown route becomes its own (Rust picks and rounds it).
func pin() -> void:
	if _edit({"op": "pin", "way": "in", "part_bounds": _editor._session.part_bounds()}, "Edit this route"):
		_group.button_pressed = true
		select_waypoint("in", -1)

## "Back to automatic": the spot's own route is dropped.
func unpin() -> void:
	if _edit({"op": "unpin", "way": "in"}, "Back to automatic"):
		select_waypoint("in", -1)

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

## Show way `next_way` ("in" or "out") and select its waypoint `index` (-1: none), as a click on
## its marker in the view does; the Drive-through group opens to show it.
func select_waypoint(next_way: String, index: int) -> void:
	way = next_way
	selected = index
	if _root == null:
		return
	for item in _way.item_count:
		if str(_way.get_item_metadata(item)) == way:
			_way.select(item)
	if index >= 0:
		_editor._view.advanced_groups["drive_through"]["toggle"].button_pressed = true
	refresh()
	_editor._selection.context_changed()

## Insert a waypoint at `point` (`[x, z]`) as waypoint `index` of the shown way, one undo step,
## and select it: the view's "Insert waypoint here" on a leg.
func insert_at(index: int, point: Array) -> void:
	if _edit({"op": "add", "way": way, "waypoint": index, "point": _snapped(Vector2(float(point[0]), float(point[1])))}, "Insert waypoint"):
		select_waypoint(way, index)

## Begin dragging the selected waypoint in the view; returns its point as asset-local XZ.
func begin_drag() -> Vector2:
	_drag_start = waypoints()[selected].duplicate()
	return Vector2(float(_drag_start[0]), float(_drag_start[1]))

## Move the dragged waypoint to `point` in the editor's projection only: the preview proves and
## draws the route from it and the fields follow, but the document waits for `end_drag`.
func drag(point: Vector2) -> void:
	# Kept in the lot, as a dragged yard vertex is.
	waypoints()[selected] = _snapped(_editor._clamp_site_surface_vertex_to_lot(point))
	_editor._update_site_anchor_preview()
	refresh()

## Finish a drag as one "move" command; a waypoint dropped where it started adds no undo step.
func end_drag() -> void:
	var point: Array = waypoints()[selected]
	if point == _snapped(Vector2(float(_drag_start[0]), float(_drag_start[1]))) \
			or not _edit({"op": "move", "way": way, "waypoint": selected, "point": point}, "Move waypoint"):
		cancel_drag()

## Put the dragged waypoint back where it was and redraw.
func cancel_drag() -> void:
	waypoints()[selected] = _drag_start
	_editor._update_site_anchor_preview()
	refresh()

func _snapped(point: Vector2) -> Array:
	return [snappedf(point.x, VIEW_SNAP_M), snappedf(point.y, VIEW_SNAP_M)]

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
