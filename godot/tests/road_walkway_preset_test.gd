# SPDX-License-Identifier: GPL-2.0-only

## ROAD-92 on the road-type path (`ROAD-101`): a divided custom type (created through the API)
## picked on the Road tool carries its median; X picks the Walkway type, which clears it, and a
## walkway stroke then previews valid. Rust stays the guard: a walkway stroke carrying a median
## is refused (`median_requires_vehicle_road`). R draws the divided type again. The removed lane
## and median keys (] [ Up Down . , N ; ') change nothing. The game has no node move tool
## (`ROAD-111`): neither the tool list, the scene, the toolbar nor M offers one. Writes the
## road-type library, so it runs only in the isolated profile. Ported from Veeti's
## `road_walkway_preset_test.gd`.
extends SceneTree

const TIMEOUT_MS := 60000

var _failures := 0
var _main: Node
var _simulation: Node

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _ground(x: float, z: float) -> Vector3:
	return Vector3(x, _simulation.get_world_surface_height(Vector2(x, z)), z)

func _press(input_manager: Node, keycode: Key) -> void:
	var event := InputEventKey.new()
	event.keycode = keycode
	event.pressed = true
	input_manager._unhandled_input(event)

func _run() -> void:
	# The test writes user://road_types.json: never in a real profile.
	if not OS.get_user_data_dir().to_lower().contains("godot-tests"):
		push_error("road_walkway_preset_test must run in the isolated profile from ./run.sh --test")
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
	var input_manager: Node = _main.get_node("InputManager")
	_expect(_simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "the blank world must load")
	input_manager._refresh_after_world_load()
	input_manager.set_simulation_speed(0.0)
	var ready_deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while not input_manager._world_ready and Time.get_ticks_msec() < ready_deadline:
		await process_frame
	_expect(input_manager._world_ready, "the world must become ready for input")
	var tool: Node = _main.get_node("RoadTool")
	var created: Dictionary = _simulation.create_road_type("Grass Avenue", [
		{"kind": "sidewalk"}, {"kind": "car_lane", "forward": false},
		{"kind": "median", "style": 2, "width_m": 3.5},
		{"kind": "car_lane", "forward": true}, {"kind": "sidewalk"}], 50, true, true)
	var avenue := String(created.get("id", ""))
	_expect(not avenue.is_empty(), "the divided type is created: %s" % created)

	input_manager._toggle_tool(input_manager.Tool.ROAD)
	_expect(tool.fwd_lanes == 1 and tool.bkw_lanes == 1 and tool.median_style == 0, "the Road tool starts as the 2-Lane Road")
	_expect(tool.set_road_type(avenue).is_empty(), "the divided type is picked")
	_expect(tool.median_style == 2 and is_equal_approx(tool.median_width, 3.5), "the type carries grass at 3.5 m: %d %.1f" % [tool.median_style, tool.median_width])
	for key in [KEY_BRACKETRIGHT, KEY_BRACKETLEFT, KEY_UP, KEY_DOWN, KEY_PERIOD, KEY_COMMA, KEY_N, KEY_SEMICOLON, KEY_APOSTROPHE]:
		_press(input_manager, key)
	_expect(tool.fwd_lanes == 1 and tool.bkw_lanes == 1 and tool.median_style == 2 and is_equal_approx(tool.median_width, 3.5), "the removed lane and median keys change nothing")

	# Rust guards the walkway: a 0 + 0 stroke carrying the tool's median is refused.
	var points := PackedVector3Array([_ground(-80.0, 40.0), _ground(80.0, 40.0)])
	var refused: Dictionary = _simulation.validate_road_candidate_with_snap(points, 0, 0, true)
	_expect(refused.get("invalid_reason", "") == "median_requires_vehicle_road", "a walkway with a median is refused: %s" % refused.get("invalid_reason", ""))

	_press(input_manager, KEY_X)
	_expect(input_manager.current_tool == input_manager.Tool.WALKWAY, "X picks the Walkway tool")
	_expect(tool.road_type_id == tool.WALKWAY_ROAD_TYPE, "X picks the Walkway type")
	_expect(tool.fwd_lanes == 0 and tool.bkw_lanes == 0, "the Walkway type is 0 + 0")
	_expect(tool.median_style == 0, "the Walkway type clears the median")
	var walkway: Dictionary = _simulation.validate_road_candidate_with_snap(points, 0, 0, true)
	_expect(bool(walkway.get("is_valid", false)), "the walkway stroke is valid without the median: %s" % walkway.get("invalid_reason", ""))

	_press(input_manager, KEY_R)
	_expect(input_manager.current_tool == input_manager.Tool.ROAD, "R picks the Road tool")
	_expect(tool.road_type_id == avenue and tool.median_style == 2, "R draws the last road type again: %s %d" % [tool.road_type_id, tool.median_style])

	# No node move tool (`ROAD-111`).
	var tools: Array = input_manager.Tool.keys()
	_expect(tools == ["NONE", "ROAD", "WALKWAY", "ZONING", "SERVICES", "INDUSTRY", "AGENT", "SCULPT", "SELECT", "BULLDOZE", "VEGETATION", "FIELD_EDIT", "ROUNDABOUT"], "the tool list: %s" % [tools])
	_expect(_main.get_node_or_null("MoveTool") == null, "the scene has no move tool")
	var toolbar: Array = []
	for button in _main.get_node("MainUI").main_toolbar.get_children():
		toolbar.append(String(button.text))
	_expect(toolbar == ["Roads", "Zoning", "Services", "Industry", "Terrain", "Inspect"], "the toolbar: %s" % [toolbar])
	_press(input_manager, KEY_M)
	_expect(input_manager.current_tool == input_manager.Tool.ROAD, "M picks no tool")
	_expect(_simulation.delete_road_type(avenue).is_empty(), "the fixture type is deleted")
	tool.active = false
	_main.queue_free()
	await process_frame
	if _failures == 0:
		print("road_walkway_preset_test: PASS")
	quit(_failures)
