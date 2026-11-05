# SPDX-License-Identifier: GPL-2.0-only

## The Road Properties window's Turn pocket section against the native bridge (`ROAD-95`, D12).
## On a 1 + 1 crossing the section edits the clicked road's end nearest the click, named by its
## junction's direction and highlighted: Left is off on a two-way road without a median (its
## refusal as the tooltip), Right adds a pocket ("Apply (+N)", Rust's verdict line), a length the
## road cannot hold and an unchanged request show Rust's refusal texts, Apply adds, resizes and
## removes the pocket (the selection follows the road under the click as the edit splits and
## joins it), Ctrl+Z (undo) brings the previous pocket back into the section, and the other
## stored end (the east arm starts at the junction) takes one too. On a divided 2 + 2 road (5 m
## grass) Left and Right are both offered beside the median (a right pocket's transition carries
## it through), and a left pocket narrows the median through the window. The window's default size (and minimum width) holds every row with
## the section shown.
## Ported from Veeti's road-chain snapshot `efd58129` (`road_pockets_test.gd`: the pocket edit
## through the tool, add, resize, remove), adapted to the Road Properties window.
extends SceneTree

const SelectTool := preload("res://scripts/tools/select_tool.gd")
const RoadPropertiesWindow := preload("res://scripts/ui/road_properties_window.gd")
var _failures := 0
var simulation: SimulationNode

class OverlayStub extends Node3D:
	func mark_no_build_dirty() -> void:
		pass
	func _rebuild_no_build_overlay() -> void:
		pass

class UiStub extends Node:
	func hide_road_properties() -> void:
		pass
	func show_road_properties_multi(_edges: Array[int], _point: Vector2) -> void:
		pass

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

# Selects the road under (x, z) as a click there does, and opens the window on it.
func _click(tool: Node, window: Node, x: float, z: float) -> int:
	var edge: int = simulation.get_hovered_edge(x, z)
	_expect(edge >= 0, "a road at (%.0f, %.0f)" % [x, z])
	var selection: Array[int] = [edge]
	tool._click_world = _ground(x, z)
	tool.selected_edges = selection
	window.show_for_edges(selection)
	return edge

# Sets the section's controls as the player does (each change asks Rust's verdict).
func _request(window: Node, left: bool, right: bool, length_m: float) -> void:
	var section = window._pocket_section
	if section._left_check.button_pressed != left:
		section._left_check.button_pressed = left
	if section._right_check.button_pressed != right:
		section._right_check.button_pressed = right
	if section._length_spin.value != length_m:
		section._length_spin.value = length_m

# Presses Apply and checks it published.
func _apply(window: Node, what: String) -> void:
	var section = window._pocket_section
	_expect(not section._apply_button.disabled, "%s: Apply is enabled (%s)" % [what, section._verdict_label.text])
	var generation := simulation.get_network_render_generation()
	section._apply_button.pressed.emit()
	_expect(await _await_generation(generation), "%s: the edit publishes" % what)

# Undoes the last edit as Ctrl+Z does; the window reads the end again on the new generation.
func _undo(what: String) -> void:
	var generation := simulation.get_network_render_generation()
	_expect(simulation.undo_action(), "%s: undo is queued" % what)
	_expect(await _await_generation(generation), "%s: undo publishes" % what)
	await process_frame
	await process_frame

func _state(window: Node) -> String:
	return window._pocket_section._state_label.text

func _verdict(window: Node) -> String:
	return window._pocket_section._verdict_label.text

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(640.0, 640.0, 8.0, 128.0, 0.0), "fixture world must load")
	simulation.set_simulation_speed(0.0)
	# Crossing A at the origin (1 + 1 roads, 150 m arms) and crossing B at z = 250, whose 150 m
	# west arm becomes a divided 2 + 2 road.
	if not await _commit(_ground(-150.0, 0.0), _ground(150.0, 0.0)) \
			or not await _commit(_ground(0.0, -150.0), _ground(0.0, 150.0)) \
			or not await _commit(_ground(-150.0, 250.0), _ground(100.0, 250.0)) \
			or not await _commit(_ground(0.0, 200.0), _ground(0.0, 310.0)):
		_finish(scene)
		return
	var ui := UiStub.new()
	ui.name = "MainUI"
	scene.add_child(ui)
	var overlay := OverlayStub.new()
	overlay.name = "ZoningOverlay"
	scene.add_child(overlay)
	var tool = SelectTool.new()
	scene.add_child(tool)
	tool.set_process(false)
	tool.active = true
	var window = RoadPropertiesWindow.new()
	window.setup(simulation, tool)
	scene.add_child(window)

	# The west arm, clicked 30 m from the crossing: its end there, heading east into the crossing.
	var west := _click(tool, window, -30.0, 0.0)
	var section = window._pocket_section
	await process_frame
	_expect(section.visible, "the section shows for one car road")
	_expect(section._heading_label.text == "Turn pocket at the end towards east", "the end's name: %s" % section._heading_label.text)
	_expect(_state(window) == "No turn pocket at this end", "no pocket yet: %s" % _state(window))
	_expect(section._left_check.disabled and section._left_check.tooltip_text == "A left turn pocket needs a one-way road or a raised median",
		"Left is off on an undivided two-way road: %s" % section._left_check.tooltip_text)
	_expect(not section._right_check.disabled and section._length_spin.value == 60.0, "Right is offered, 60 m by default")
	_expect(section._apply_button.disabled, "nothing to apply before a request")
	_expect(tool._pocket_end_mi.mesh != null, "the end is highlighted")
	var content: Control = window._window.get_child(0)
	_expect(window.DEFAULT_SIZE.y >= int(content.get_combined_minimum_size().y),
		"the default height holds every row: %d < %d" % [window.DEFAULT_SIZE.y, int(content.get_combined_minimum_size().y)])
	_expect(window.MIN_SIZE.x >= int(content.get_combined_minimum_size().x),
		"the minimum width holds every row: %d < %d" % [window.MIN_SIZE.x, int(content.get_combined_minimum_size().x)])

	# A right pocket: Rust's verdict and cost, then refusals for a length the road cannot hold.
	_request(window, false, true, 60.0)
	var cost := roundi(float(window._select_tool.preview_selected_road_pocket(window._pocket_info, false, true, 60.0).get("cost_delta", 0.0)))
	_expect(_verdict(window) == "Right pocket, 60 m from the junction" and cost > 0
		and section._apply_button.text == "Apply (+%s)" % window._thousands(cost), "the add's verdict: %s / %s" % [_verdict(window), section._apply_button.text])
	_request(window, false, true, 300.0)
	_expect(_verdict(window) == "Road too short for a pocket here" and section._apply_button.disabled, "a 300 m pocket is refused: %s" % _verdict(window))
	_request(window, false, true, 60.0)
	await _apply(window, "the add")
	var state: Dictionary = simulation.get_road_pocket(west, 1)
	_expect(bool(state.get("has_pocket", false)) and bool(state.get("right", false)) and is_equal_approx(float(state.get("length_m", 0.0)), 60.0),
		"the pocket is built: %s" % state)
	_expect(_state(window) == "Right pocket, 60 m from the junction", "the section shows it: %s" % _state(window))
	_expect(tool.selected_edges[0] == simulation.get_hovered_edge(-30.0, 0.0), "the selection follows the road under the click")
	_request(window, false, false, 60.0)
	_request(window, false, true, 60.0)
	_expect(_verdict(window) == "No change" and section._apply_button.disabled, "an unchanged request: %s" % _verdict(window))

	# Resize, undo back to 60 m, then remove.
	_request(window, false, true, 80.0)
	await _apply(window, "the resize")
	_expect(_state(window) == "Right pocket, 80 m from the junction", "resized: %s" % _state(window))
	await _undo("the resize")
	_expect(_state(window) == "Right pocket, 60 m from the junction", "undo brings 60 m back: %s" % _state(window))
	_request(window, false, false, 60.0)
	_expect(_verdict(window) == "Removes the turn pocket", "the removal's verdict: %s" % _verdict(window))
	await _apply(window, "the removal")
	_expect(_state(window) == "No turn pocket at this end", "removed: %s" % _state(window))
	var joined: Dictionary = simulation.get_edge_cross_section(simulation.get_hovered_edge(-30.0, 0.0))
	_expect(int(joined.get("fwd_lanes", 0)) == 1 and int(joined.get("bkw_lanes", 0)) == 1, "the road is 1 + 1 again: %s" % joined)

	# The east arm starts at the crossing: the other stored end.
	var east := _click(tool, window, 30.0, 0.0)
	_expect(section._heading_label.text == "Turn pocket at the end towards west", "the east arm's end: %s" % section._heading_label.text)
	_request(window, false, true, 40.0)
	await _apply(window, "the east add")
	state = simulation.get_road_pocket(east, 0)
	_expect(bool(state.get("has_pocket", false)) and is_equal_approx(float(state.get("length_m", 0.0)), 40.0), "the east pocket: %s" % state)
	await _undo("the east add")
	_expect(_state(window) == "No turn pocket at this end", "undo removes the east pocket: %s" % _state(window))

	# The divided road: Left and Right offered beside the median; a left pocket narrows it.
	var divided := simulation.get_hovered_edge(-60.0, 250.0)
	var generation := simulation.get_network_render_generation()
	_expect(simulation.set_edge_cross_section(divided, 2, 2, 2, 5.0, 50, true, true, true), "the west arm of B is divided")
	await _await_generation(generation)
	_click(tool, window, -30.0, 250.0)
	_expect(not section._left_check.disabled, "Left is offered beside a 5 m raised median")
	_expect(not section._right_check.disabled, "Right is offered beside a median: %s" % section._right_check.tooltip_text)
	_request(window, false, true, 60.0)
	_expect(_verdict(window) == "Right pocket, 60 m from the junction", "the right pocket's verdict beside a median: %s" % _verdict(window))
	_request(window, true, false, 60.0)
	_expect(_verdict(window) == "Left pocket, 60 m from the junction", "the left pocket's verdict: %s" % _verdict(window))
	await _apply(window, "the left pocket")
	var pocket: Dictionary = simulation.get_edge_cross_section(simulation.get_hovered_edge(-30.0, 250.0))
	_expect(is_equal_approx(float(pocket.get("median_width", 0.0)), 1.5), "the median narrows to 1.5 m: %s" % pocket)
	_expect(_state(window) == "Left pocket, 60 m from the junction", "the section shows the left pocket: %s" % _state(window))
	await _undo("the left pocket")
	_expect(_state(window) == "No turn pocket at this end", "undo removes the left pocket: %s" % _state(window))

	# Two roads selected: no section, no end highlight.
	var both: Array[int] = [simulation.get_hovered_edge(-30.0, 0.0), simulation.get_hovered_edge(30.0, 0.0)]
	tool.selected_edges = both
	window.show_for_edges(both)
	_expect(not section.visible and tool._pocket_end_mi.mesh == null, "no section for several roads")
	tool.active = false
	_finish(scene)

func _finish(scene: Node) -> void:
	scene.free()
	if _failures == 0:
		print("road_pocket_test: PASS")
	quit(_failures)
