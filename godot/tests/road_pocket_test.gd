# SPDX-License-Identifier: GPL-2.0-only

## The Junction window's Turn pockets rows against the native bridge (`ROAD-95`, round R5c2). A
## junction click opens the window: on a 1 + 1 crossing four rows named by the direction their
## road comes from, clockwise from north; the hovered row's road (else the last changed one's) is
## highlighted from the junction. A right pocket on the west arm shows Rust's verdict and "Apply
## (+N)", a length the road cannot hold shows Rust's refusal, Apply builds it and the window stays
## on the junction (another row's unapplied request kept), a resize, Ctrl+Z (undo) back to 60 m
## and the removal; the east arm (the other stored end) takes one too. A pocket's lane
## transition node has no pocket section. A one-way arm offers Left; its leaving arm has no row.
## On a divided 2 + 2 road (5 m grass) Left and Right are both offered and a left pocket narrows
## the median. Road Properties no longer has a pocket section. Left is offered on an undivided
## two-way road too (round R5c1).
## Ported from Veeti's road-chain snapshot `efd58129` (`road_pockets_test.gd`: the pocket edit
## through the tool, add, resize, remove), adapted to the Junction window.
extends SceneTree

const SelectTool := preload("res://scripts/tools/select_tool.gd")
const JunctionWindow := preload("res://scripts/ui/junction_window.gd")
const RoadPropertiesWindow := preload("res://scripts/ui/road_properties_window.gd")
var _failures := 0
var simulation: SimulationNode

class OverlayStub extends Node3D:
	func mark_no_build_dirty() -> void:
		pass
	func _rebuild_no_build_overlay() -> void:
		pass

# The select tool's host: junction clicks open the real Junction window.
class UiStub extends Node:
	var junction: Node
	func hide_road_properties() -> void:
		pass
	func show_road_properties_multi(_edges: Array[int], _point: Vector2) -> void:
		pass
	func show_junction_window(node: int, point: Vector2) -> void:
		junction.show_for_node(node, point)
	func hide_junction_window() -> void:
		junction.close_window()

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _ground(x: float, z: float) -> Vector3:
	return Vector3(x, simulation.get_world_surface_height(Vector2(x, z)), z)

# Waits for the render generation to pass `generation`; false after 20 s.
func _await_generation(generation: int) -> bool:
	var deadline := Time.get_ticks_msec() + 20000
	while Time.get_ticks_msec() < deadline:
		if simulation.get_network_render_generation() > generation:
			return true
		await process_frame
	return false

func _commit(a: Vector3, b: Vector3, fwd: int = 1, bkw: int = 1) -> bool:
	var generation := simulation.get_network_render_generation()
	simulation.add_road_with_snap(PackedVector3Array([a, b]), fwd, bkw, true)
	var done := await _await_generation(generation)
	_expect(done, "fixture road must commit")
	return done

# Clicks the junction nearest (x, z) as the select tool does: selects it, opens the window.
func _click_node(tool: Node, x: float, z: float) -> int:
	var node: int = simulation.get_closest_node(_ground(x, z), 8.0)
	_expect(node >= 0, "a node at (%.0f, %.0f)" % [x, z])
	tool._last_click_screen_pos = Vector2(200, 200)
	tool._select_node(node)
	var none: Array[int] = []
	tool._set_selection(none)
	tool._build_node_visuals(node)
	await process_frame
	await process_frame
	return node

# The row whose name starts with "From <direction>".
func _row(window: Node, direction: String):
	for i in range(window.row_count()):
		if window.row(i)._name_label.text.begins_with("From %s ·" % direction):
			return window.row(i)
	_expect(false, "a row from %s" % direction)
	return null

# Sets a row's controls as the player does (each change asks Rust's verdict).
func _request(row, left: bool, right: bool, length_m: float) -> void:
	if row._left_check.button_pressed != left:
		row._left_check.button_pressed = left
	if row._right_check.button_pressed != right:
		row._right_check.button_pressed = right
	if row._length_spin.value != length_m:
		row._length_spin.value = length_m

# Presses a row's Apply and checks it published; the window reads the arms again.
func _apply(row, what: String) -> void:
	_expect(not row._apply_button.disabled, "%s: Apply is enabled (%s)" % [what, row._verdict_label.text])
	var generation := simulation.get_network_render_generation()
	row._apply_button.pressed.emit()
	_expect(await _await_generation(generation), "%s: the edit publishes" % what)
	await process_frame
	await process_frame

# Undoes the last edit as Ctrl+Z does; the window reads the arms again on the new generation.
func _undo(what: String) -> void:
	var generation := simulation.get_network_render_generation()
	_expect(simulation.undo_action(), "%s: undo is queued" % what)
	_expect(await _await_generation(generation), "%s: undo publishes" % what)
	await process_frame
	await process_frame

func _state(row) -> String:
	return row._state_label.text

func _verdict(row) -> String:
	return row._verdict_label.text

# The highlight strip's centre in XZ, or null without one.
func _strip_centre(tool: Node):
	var mesh: Mesh = tool._pocket_end_mi.mesh
	if mesh == null:
		return null
	var box := mesh.get_aabb()
	return Vector2(box.get_center().x, box.get_center().z)

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(640.0, 640.0, 8.0, 128.0, 0.0), "fixture world must load")
	simulation.set_simulation_speed(0.0)
	# Crossing A at the origin (1 + 1 roads, 150 m arms); crossing B at z = 250, whose 150 m west
	# arm becomes a divided 2 + 2 road; crossing C at z = -250 of a 1 + 1 road and a one-way road
	# drawn north to south.
	if not await _commit(_ground(-150.0, 0.0), _ground(150.0, 0.0)) \
			or not await _commit(_ground(0.0, -150.0), _ground(0.0, 150.0)) \
			or not await _commit(_ground(-150.0, 250.0), _ground(100.0, 250.0)) \
			or not await _commit(_ground(0.0, 200.0), _ground(0.0, 310.0)) \
			or not await _commit(_ground(-150.0, -250.0), _ground(150.0, -250.0)) \
			or not await _commit(_ground(0.0, -310.0), _ground(0.0, -190.0), 1, 0):
		_finish(scene)
		return
	var overlay := OverlayStub.new()
	overlay.name = "ZoningOverlay"
	scene.add_child(overlay)
	var ui := UiStub.new()
	ui.name = "MainUI"
	scene.add_child(ui)
	var tool = SelectTool.new()
	scene.add_child(tool)
	tool.set_process(false)
	tool.active = true
	var window = JunctionWindow.new()
	window.setup(simulation, tool)
	ui.junction = window
	scene.add_child(window)

	# Crossing A: four rows clockwise from north, nothing applied yet.
	var node := await _click_node(tool, 0.0, 0.0)
	_expect(window._window.visible and window.row_count() == 4, "four rows at a four-way: %d" % window.row_count())
	var names := []
	for i in range(window.row_count()):
		names.append(window.row(i)._name_label.text.get_slice(" ·", 0))
	_expect(names == ["From north", "From east", "From south", "From west"], "rows clockwise from north: %s" % [names])
	var west = window.row(3)
	_expect(not west._left_check.disabled, "Left is offered on an undivided two-way road")
	_expect(not west._right_check.disabled and west._length_spin.value == 60.0, "Right is offered, 60 m by default")
	# Godot's spin box ignores its arrows when its text cannot be edited (the user found the
	# length stuck at 60 m, 2026-11-05). Headless Godot routes no click into the window, so the
	# rule is checked on the box.
	_expect(west._length_spin.get_line_edit().editable, "the length box's arrows and number are live")
	# The window keeps the game's keyboard after a click on the arrows, and keeps its own for
	# typing after a click into the number until the number is submitted.
	var number: LineEdit = west._length_spin.get_line_edit()
	var origin := Vector2(window._window.position)
	var arrows_at: Vector2 = origin + west._length_spin.get_global_rect().end - Vector2(4.0, 6.0)
	_window_click(window._window, arrows_at)
	_expect(window._window.unfocusable, "a click on the arrows leaves the keyboard to the game")
	number.grab_focus()
	_window_click(window._window, origin + number.get_global_rect().get_center())
	_expect(not window._window.unfocusable, "a click into the number keeps the keyboard for typing")
	number.text_submitted.emit(number.text)
	_expect(window._window.unfocusable, "Enter gives the keyboard back to the game")
	await process_frame
	_expect(_state(west) == "No pocket" and west._apply_button.disabled, "no pocket and nothing to apply yet")
	_expect(tool._pocket_end_mi.mesh == null, "no road highlighted before a hover or a change")
	# The scroll area's own minimum height is nil: the window's default height must hold the
	# rest of the content and the four rows.
	var content: Control = window._window.get_child(0)
	var needed := int(content.get_combined_minimum_size().y + window._rows_box.get_combined_minimum_size().y)
	_expect(window.DEFAULT_SIZE.y >= needed, "four rows fit without scrolling: %d < %d" % [window.DEFAULT_SIZE.y, needed])
	_expect(window.MIN_SIZE.x >= int(content.get_combined_minimum_size().x),
		"the minimum width holds every row: %d < %d" % [window.MIN_SIZE.x, int(content.get_combined_minimum_size().x)])

	# The highlight follows the hovered row, then the last changed one.
	window.row(1).mouse_entered.emit()
	var centre = _strip_centre(tool)
	_expect(centre != null and centre.x > 10.0 and absf(centre.y) < 5.0, "the hovered east row's road is highlighted: %s" % [centre])
	_expect(window.row(1)._style.border_color == window.row(1).HIGHLIGHT, "the hovered row is outlined")
	window.row(1).mouse_exited.emit()
	_expect(tool._pocket_end_mi.mesh == null, "no highlight once the cursor leaves the rows")
	# Godot's hover chain ends at a control that stops the mouse, and the row then reads as left:
	# the highlight flickered as the cursor crossed a row's controls (the user, 2026-11-05).
	# Headless Godot runs no hover, so the rule is checked on the row itself.
	var stops := []
	_controls_that_stop(window.row(1), stops)
	_expect(stops.is_empty(), "every control in a row passes the cursor on to the row: %s" % [stops])

	# The west arm: a right pocket's verdict and cost, then a length the road cannot hold.
	var west_edge := int(west.info.edge)
	var west_end := int(west.info.end)
	_request(west, false, true, 60.0)
	centre = _strip_centre(tool)
	_expect(centre != null and centre.x < -10.0, "the changed west row's road is highlighted: %s" % [centre])
	var cost := roundi(float(simulation.preview_road_pocket(west_edge, int(west.info.end), false, true, 60.0).get("cost_delta", 0.0)))
	_expect(_verdict(west) == "Right pocket, 60 m from the junction" and cost > 0
		and west._apply_button.text == "Apply (+%s)" % window.UIStyle.thousands(cost), "the add's verdict: %s / %s" % [_verdict(west), west._apply_button.text])
	_request(west, false, true, 300.0)
	_expect(_verdict(west) == "Road too short for a pocket here" and west._apply_button.disabled, "a 300 m pocket is refused: %s" % _verdict(west))
	_request(west, false, true, 60.0)
	# An unapplied request on the north arm survives the west arm's Apply.
	_request(window.row(0), false, true, 40.0)
	await _apply(west, "the add")
	var state: Dictionary = simulation.get_road_pocket(west_edge, west_end)
	_expect(bool(state.get("has_pocket", false)) and bool(state.get("right", false)) and is_equal_approx(float(state.get("length_m", 0.0)), 60.0),
		"the pocket is built: %s" % state)
	_expect(window._node == node and window.row_count() == 4, "the window stays on the junction")
	west = _row(window, "west")
	_expect(_state(west) == "Right, 60 m", "the row shows it: %s" % _state(west))
	var north = _row(window, "north")
	_expect(north.request() == [false, true, 40.0] and _verdict(north) == "Right pocket, 40 m from the junction",
		"the north row keeps its request: %s / %s" % [north.request(), _verdict(north)])

	# Resize, undo back to 60 m, then remove.
	_request(west, false, true, 80.0)
	await _apply(west, "the resize")
	_expect(_state(_row(window, "west")) == "Right, 80 m", "resized: %s" % _state(_row(window, "west")))
	await _undo("the resize")
	west = _row(window, "west")
	_expect(_state(west) == "Right, 60 m", "undo brings 60 m back: %s" % _state(west))
	_request(west, false, false, 60.0)
	_expect(_verdict(west) == "Removes the turn pocket", "the removal's verdict: %s" % _verdict(west))
	await _apply(west, "the removal")
	_expect(_state(_row(window, "west")) == "No pocket", "removed: %s" % _state(_row(window, "west")))
	var joined: Dictionary = simulation.get_edge_cross_section(int(_row(window, "west").info.edge))
	_expect(int(joined.get("fwd_lanes", 0)) == 1 and int(joined.get("bkw_lanes", 0)) == 1, "the road is 1 + 1 again: %s" % joined)

	# The east arm starts at the crossing: the other stored end.
	var east = _row(window, "east")
	_expect(int(east.info.end) == 0, "the east arm's stored end is its start")
	var east_edge := int(east.info.edge)
	_request(east, false, true, 40.0)
	await _apply(east, "the east add")
	state = simulation.get_road_pocket(east_edge, 0)
	_expect(bool(state.get("has_pocket", false)) and is_equal_approx(float(state.get("length_m", 0.0)), 40.0), "the east pocket: %s" % state)
	await _undo("the east add")
	_expect(_state(_row(window, "east")) == "No pocket", "undo removes the east pocket: %s" % _state(_row(window, "east")))

	# A pocket's lane transition: no Turn pockets section.
	west = _row(window, "west")
	_request(west, false, true, 60.0)
	await _apply(west, "the add again")
	var transition := await _click_node(tool, -70.5, 0.0)
	_expect(transition != node and window.row_count() == 0 and not window._pocket_heading.visible and not window._scroll.visible
		and window._summary_label.visible,
		"no pocket section at the lane transition (node %d, %d rows)" % [transition, window.row_count()])
	await _undo("the add again")

	# Crossing C: the one-way road comes in from the north (Left offered) and leaves south (no row).
	await _click_node(tool, 0.0, -250.0)
	names = []
	for i in range(window.row_count()):
		names.append(window.row(i)._name_label.text.get_slice(" ·", 0))
	_expect(names == ["From north", "From east", "From west"], "the leaving one-way arm has no row: %s" % [names])
	north = _row(window, "north")
	_expect(not north._left_check.disabled and not north._right_check.disabled, "a one-way arm offers Left and Right: %s" % north._left_check.tooltip_text)

	# Crossing B's divided west arm: Left and Right offered beside the median; a left pocket
	# narrows it.
	var divided := simulation.get_hovered_edge(-60.0, 250.0)
	var generation := simulation.get_network_render_generation()
	_expect(simulation.set_edge_cross_section(divided, 2, 2, 2, 5.0, 50, true, true, true), "the west arm of B is divided")
	await _await_generation(generation)
	await _click_node(tool, 0.0, 250.0)
	west = _row(window, "west")
	_expect(not west._left_check.disabled and not west._right_check.disabled, "Left and Right are offered beside a 5 m raised median")
	_expect(west._name_label.text.ends_with("2 + 2 lanes, Grass 5.0 m") or not String(west.info.road_type).is_empty(),
		"the divided road's name: %s" % west._name_label.text)
	_request(west, true, false, 60.0)
	_expect(_verdict(west) == "Left pocket, 60 m from the junction", "the left pocket's verdict: %s" % _verdict(west))
	await _apply(west, "the left pocket")
	west = _row(window, "west")
	var pocket: Dictionary = simulation.get_edge_cross_section(int(west.info.edge))
	_expect(is_equal_approx(float(pocket.get("median_width", 0.0)), 1.5), "the median narrows to 1.5 m: %s" % pocket)
	_expect(_state(west) == "Left, 60 m", "the row shows the left pocket: %s" % _state(west))
	_expect(west._name_label.text.ends_with("2 + 2 lanes, Grass 5.0 m") or not String(west.info.road_type).is_empty(),
		"the pocket arm is named by its road without the pocket: %s" % west._name_label.text)
	await _undo("the left pocket")
	_expect(_state(_row(window, "west")) == "No pocket", "undo removes the left pocket: %s" % _state(_row(window, "west")))

	# Road Properties has no pocket section any more.
	var properties = RoadPropertiesWindow.new()
	properties.setup(simulation, tool)
	scene.add_child(properties)
	var selection: Array[int] = [simulation.get_hovered_edge(-30.0, 0.0)]
	properties.show_for_edges(selection)
	await process_frame
	var texts := []
	var stack: Array[Node] = [properties._window]
	while not stack.is_empty():
		var item: Node = stack.pop_back()
		if item is Label or item is Button:
			texts.append(item.text)
		stack.append_array(item.get_children())
	_expect(not " ".join(texts).to_lower().contains("pocket"), "Road Properties shows no pocket section: %s" % [texts])
	properties.close_window()

	# Leaving the junction closes the window and its highlight.
	tool._select_node(-1)
	_expect(not window._window.visible and tool._pocket_end_mi.mesh == null, "the window closes with the junction")
	tool.active = false
	_finish(scene)

# A left click at `at` (the embedder's coordinates) as the window's input signal carries it.
func _window_click(window: Window, at: Vector2) -> void:
	for pressed in [true, false]:
		var event := InputEventMouseButton.new()
		event.button_index = MOUSE_BUTTON_LEFT
		event.pressed = pressed
		event.position = at
		window.window_input.emit(event)

func _controls_that_stop(node: Node, found: Array) -> void:
	for child in node.get_children(true):
		if child is Control and child.mouse_filter == Control.MOUSE_FILTER_STOP:
			found.append(child.get_class())
		_controls_that_stop(child, found)

func _finish(scene: Node) -> void:
	scene.free()
	if _failures == 0:
		print("road_pocket_test: PASS")
	quit(_failures)
