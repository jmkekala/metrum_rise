# SPDX-License-Identifier: GPL-2.0-only

## Road types in the game (`ROAD-101`), through Main.tscn and the player's controls: the Roads
## menu lists the built-in types in order and a custom type created through the API; each
## built-in pick picks its tool, marks its button and draws the same edge as dev's buttons
## (the same lanes and no median through `add_road_with_snap`); the custom type draws its lanes
## and median, read back from the graph; editing the type changes the next stroke only (the
## first edge keeps its layout); Road Properties changes a 1 + 1 road to the divided type and
## Ctrl+Z restores it, and refuses walkway -> road with its text; the drawing panel has no lane
## or median rows. Writes the road-type library, so it runs only in the isolated profile.
extends SceneTree

const TIMEOUT_MS := 60000
const BUILTINS := [
	# id, name, Walkway tool, dev's button lanes (fwd, bkw)
	["builtin:walkway", "Walkway", true, 0, 0],
	["builtin:road_2l", "2-Lane Road", false, 1, 1],
	["builtin:road_4l", "4-Lane Road", false, 2, 2],
	["builtin:one_way_1l", "One-Way 1L", false, 1, 0],
	["builtin:one_way_2l", "One-Way 2L", false, 2, 0],
]
const AVENUE := [
	{"kind": "sidewalk"}, {"kind": "car_lane", "forward": false}, {"kind": "car_lane", "forward": false},
	{"kind": "median", "style": 2, "width_m": 3.0},
	{"kind": "car_lane", "forward": true}, {"kind": "car_lane", "forward": true}, {"kind": "sidewalk"},
]
const AVENUE_EDITED := [
	{"kind": "sidewalk"}, {"kind": "car_lane", "forward": false},
	{"kind": "median", "style": 1, "width_m": 1.5},
	{"kind": "car_lane", "forward": true}, {"kind": "sidewalk"},
]

var _failures := 0
var _main: Node
var _simulation: Node
var _input: Node
var _tool: Node
var _ui: Node

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _ground(x: float, z: float) -> Vector3:
	return Vector3(x, _simulation.get_world_surface_height(Vector2(x, z)), z)

# Waits for the render generation to pass `generation`; false after the timeout.
func _await_generation(generation: int) -> bool:
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while Time.get_ticks_msec() < deadline:
		if _simulation.get_network_render_generation() > generation:
			return true
		await process_frame
	return false

# One straight stroke along z = `z` through the road tool, as a click would commit it; returns
# the drawn edge (-1 when it did not commit).
func _draw(z: float) -> int:
	var points := PackedVector3Array([_ground(-100.0, z), _ground(100.0, z)])
	var generation: int = _simulation.get_network_render_generation()
	_tool._begin_stroke(points[0], 0, Vector3.ZERO)
	_tool.set_scripted_pointer(true, points[-1])
	_tool._process(1.0 / 60.0)
	_expect(bool(_tool._preview.get("is_valid", false)), "the stroke at z %.0f previews: %s" % [z, _tool._preview.get("invalid_reason", "")])
	_expect(_tool._commit_segment(points[-1]), "the click at z %.0f queues the road" % z)
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while _tool._commit_request_id > 0 and Time.get_ticks_msec() < deadline:
		await process_frame
	_tool.cancel_road()
	_tool.set_scripted_pointer(false)
	_expect(await _await_generation(generation), "the stroke at z %.0f publishes" % z)
	return _simulation.get_hovered_edge(0.0, z)

# The same stroke with dev's literal button arguments, straight to Rust, with no median.
func _draw_reference(z: float, fwd: int, bkw: int) -> int:
	var points := PackedVector3Array([_ground(-100.0, z), _ground(100.0, z)])
	var generation: int = _simulation.get_network_render_generation()
	_simulation.set_road_tool_median(0, 0.0)
	_simulation.add_road_with_snap(points, fwd, bkw, true)
	_expect(await _await_generation(generation), "the reference stroke at z %.0f publishes" % z)
	return _simulation.get_hovered_edge(0.0, z)

# The edge's cross-section as compared between strokes (the road_type key aside).
func _layout(edge: int) -> Array:
	var section: Dictionary = _simulation.get_edge_cross_section(edge)
	return [int(section.get("fwd_lanes", -1)), int(section.get("bkw_lanes", -1)),
		int(section.get("median_style", -1)), float(section.get("median_width", -1.0)),
		float(section.get("width", -1.0))]

func _labels(node: Node, into: Array) -> Array:
	if node is Label:
		into.append(node.text)
	for child in node.get_children():
		_labels(child, into)
	return into

func _press_ctrl_z() -> void:
	var event := InputEventKey.new()
	event.keycode = KEY_Z
	event.ctrl_pressed = true
	event.pressed = true
	_input._unhandled_input(event)

func _run() -> void:
	# The test writes user://road_types.json: never in a real profile.
	if not OS.get_user_data_dir().to_lower().contains("godot-tests"):
		push_error("road_types_test must run in the isolated profile from ./run.sh --test")
		quit(1)
		return
	_main = load("res://scenes/Main.tscn").instantiate()
	# Own scene startup without the menu launch flow.
	_main.set_script(null)
	root.add_child(_main)
	current_scene = _main
	await process_frame
	await process_frame
	_simulation = _main.get_node("SimulationNode")
	_input = _main.get_node("InputManager")
	_tool = _main.get_node("RoadTool")
	_ui = _main.get_node("MainUI")
	_expect(_simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "the blank world must load")
	_input._refresh_after_world_load()
	_input.set_simulation_speed(0.0)
	var ready_deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while not _input._world_ready and Time.get_ticks_msec() < ready_deadline:
		await process_frame
	_expect(_input._world_ready, "the world must become ready for input")
	var menu = _ui.road_type_menu

	# The drawing panel keeps Tool mode, Snapping and Structure, and has no lane or median rows.
	var labels := _labels(_ui.road_options_menu, [])
	for kept in ["Tool mode", "Snapping", "Structure"]:
		_expect(labels.has(kept), "the drawing panel keeps %s: %s" % [kept, labels])
	for removed in ["Lanes forward", "Lanes backward", "Median", "Median width"]:
		_expect(not labels.has(removed), "the drawing panel has no %s row" % removed)

	# The menu lists the built-ins in order, then a custom type once the library has it.
	var builtin_ids: Array[String] = []
	for builtin in BUILTINS:
		builtin_ids.append(builtin[0])
	_expect(str(menu.listed_ids().slice(0, BUILTINS.size())) == str(builtin_ids), "the built-ins lead the row: %s" % [menu.listed_ids()])
	for builtin in BUILTINS:
		var button: Button = menu.button_for(builtin[0])
		_expect(button != null and button.text == builtin[1], "the %s button: %s" % [builtin[1], button.text if button else "missing"])
	var created: Dictionary = _simulation.create_road_type("Menu Test Avenue", AVENUE)
	var avenue := String(created.get("id", ""))
	_expect(not avenue.is_empty(), "the custom type is created: %s" % created)
	_expect(not menu.listed_ids().has(avenue), "the row lists the library as it was read")
	_ui.refresh_road_types()
	_expect(menu.listed_ids().back() == avenue and menu.button_for(avenue).text == "Menu Test Avenue", "the row lists the custom type last: %s" % [menu.listed_ids()])
	_expect(menu._builder_button.get_index() == menu.get_child_count() - 1, "the row ends with Road builder...")
	var builder_requests := [0]
	menu.road_builder_requested.connect(func(): builder_requests[0] += 1)
	menu._builder_button.pressed.emit()
	_expect(builder_requests[0] == 1, "Road builder... emits road_builder_requested")

	# Each built-in picks its tool, marks its button and draws dev's edge.
	var z := -200.0
	var walkway_edge := -1
	for builtin in BUILTINS:
		menu.button_for(builtin[0]).pressed.emit()
		var tool_kind = _input.Tool.WALKWAY if builtin[2] else _input.Tool.ROAD
		_expect(_input.current_tool == tool_kind, "%s picks its tool" % builtin[1])
		_expect(_tool.road_type_id == builtin[0] and _tool.fwd_lanes == builtin[3] and _tool.bkw_lanes == builtin[4] and _tool.median_style == 0,
			"%s gives dev's button arguments: %d + %d, median %d" % [builtin[1], _tool.fwd_lanes, _tool.bkw_lanes, _tool.median_style])
		for other in builtin_ids:
			_expect(menu.button_for(other).button_pressed == (other == builtin[0]), "%s is marked alone (%s)" % [builtin[1], other])
		var drawn := await _draw(z)
		var reference := await _draw_reference(z + 25.0, builtin[3], builtin[4])
		_expect(drawn >= 0 and reference >= 0 and drawn != reference, "%s: both strokes commit" % builtin[1])
		_expect(_layout(drawn) == _layout(reference), "%s draws dev's edge: %s vs %s" % [builtin[1], _layout(drawn), _layout(reference)])
		_expect(String(_simulation.get_edge_cross_section(drawn).get("road_type", "")) == builtin[0], "%s reads back as its type" % builtin[1])
		if builtin[2]:
			walkway_edge = drawn
		z += 50.0

	# The custom type draws its lanes and median; an edit changes the next stroke only.
	menu.button_for(avenue).pressed.emit()
	_expect(_input.current_tool == _input.Tool.ROAD and menu.button_for(avenue).button_pressed, "the custom type picks the Road tool and is marked")
	var first := await _draw(z)
	_expect(_layout(first) == [2, 2, 2, 3.0, 17.0], "the custom type draws 2 + 2 with a 3 m grass median: %s" % [_layout(first)])
	_expect(String(_simulation.get_edge_cross_section(first).get("road_type", "")) == avenue, "the road reads back as the custom type")
	_expect(_simulation.update_road_type(avenue, "Menu Test Avenue", AVENUE_EDITED).is_empty(), "the custom type is edited")
	_ui.refresh_road_types()
	_expect(_tool.road_type_id == avenue and _tool.fwd_lanes == 1 and _tool.median_style == 1, "the tool draws the edited type: %d + %d, median %d" % [_tool.fwd_lanes, _tool.bkw_lanes, _tool.median_style])
	var second := await _draw(z + 50.0)
	_expect(_layout(second) == [1, 1, 1, 1.5, 8.5], "the second stroke has the new layout: %s" % [_layout(second)])
	_expect(_layout(first) == [2, 2, 2, 3.0, 17.0], "the first edge keeps its layout: %s" % [_layout(first)])
	_expect(String(_simulation.get_edge_cross_section(first).get("road_type", "")) == "", "the first edge is a layout no type has now")

	# Road Properties: the 2-Lane Road becomes the divided type; Ctrl+Z restores it.
	_input._toggle_tool(_input.Tool.SELECT)
	var road: int = _simulation.get_hovered_edge(0.0, -150.0)
	_expect(_layout(road) == [1, 1, 0, 0.0, 7.0], "the 2-Lane Road is 1 + 1: %s" % [_layout(road)])
	var selection: Array[int] = [road]
	_input.select_tool.selected_edges = selection
	_ui.show_road_properties_multi(selection)
	var window = _ui.road_properties_panel
	var picker: OptionButton = window._type_picker
	_expect(picker.get_item_text(picker.selected) == "2-Lane Road", "the window shows the road's type: %s" % picker.get_item_text(picker.selected))
	var avenue_index := -1
	for index in picker.item_count:
		if String(Dictionary(picker.get_item_metadata(index)).get("id", "")) == avenue:
			avenue_index = index
	_expect(avenue_index >= 0, "the picker lists the custom type")
	picker.select(avenue_index)
	picker.item_selected.emit(avenue_index)
	_expect(not window._apply_button.disabled, "the divided type can be applied: %s" % window._verdict_label.text)
	var generation: int = _simulation.get_network_render_generation()
	window._apply_button.pressed.emit()
	_expect(_layout(road) == [1, 1, 1, 1.5, 8.5], "Apply gives the road the divided type: %s" % [_layout(road)])
	_expect(await _await_generation(generation), "the type change publishes")
	# A focused tool window keeps the HUD keys (InputManager._ui_has_modal_popup): the player's
	# Ctrl+Z reaches the game once the window is closed or the map has focus.
	_ui.hide_road_properties()
	generation = _simulation.get_network_render_generation()
	_press_ctrl_z()
	_expect(await _await_generation(generation), "Ctrl+Z publishes")
	_expect(_layout(road) == [1, 1, 0, 0.0, 7.0], "Ctrl+Z restores the 2-Lane Road: %s" % [_layout(road)])
	# Walkway -> road is refused with its text.
	var walkway_selection: Array[int] = [walkway_edge]
	_input.select_tool.selected_edges = walkway_selection
	_ui.show_road_properties_multi(walkway_selection)
	_expect(picker.get_item_text(picker.selected) == "Walkway" and not picker.disabled, "a walkway shows its type, the picker enabled")
	for index in picker.item_count:
		if String(Dictionary(picker.get_item_metadata(index)).get("id", "")) == "builtin:road_2l":
			picker.select(index)
			picker.item_selected.emit(index)
	_expect(window._apply_button.disabled and window._verdict_label.text == "A walkway cannot become a road: draw a road in its place", "walkway -> road is refused: %s" % window._verdict_label.text)
	_ui.hide_road_properties()

	_expect(_simulation.delete_road_type(avenue).is_empty(), "the fixture type is deleted")
	_ui.refresh_road_types()
	_expect(not menu.listed_ids().has(avenue), "a deleted type leaves the row")
	_input._toggle_tool(_input.Tool.SELECT)
	_main.queue_free()
	await process_frame
	if _failures == 0:
		print("road_types_test: PASS")
	quit(_failures)
