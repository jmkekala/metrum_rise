# SPDX-License-Identifier: GPL-2.0-only

## Height handles on the real road tool (`ROAD-112` round I), on flat ground: a straight draft's
## start grip, grabbed with a press and dragged up 5 m, makes the draft one bridge stroke that
## previews as any stroke; PageDown lowers the live end 8 m below the ground, so the draft
## becomes a bridge to the crossing and a tunnel from it (a cut approach and the bore), a batch
## whose ghost shows while it is a new pose and whose exact pieces replace it once the pose
## rests; the click builds it as one road (a bridge and a tunnel) and one undo takes all of it
## back. The pointer is scripted, as the tool's tests drive it: during the drag the scripted point
## is the grip's.
extends SceneTree

const TIMEOUT_MS := 60000
const START := Vector2(-100.0, 0.0)
const END := Vector2(100.0, 0.0)
# `height::handles::HANDLE_STEM_M`: the grip stands this far above its end.
const STEM_M := 4.0

var _failures := 0
var _simulation: Node

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	# Own scene startup without the menu launch flow.
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	await process_frame
	_simulation = main.get_node("SimulationNode")
	var input_manager: Node = main.get_node("InputManager")
	_expect(_simulation.create_blank_world(640.0, 640.0, 4.0, 128.0, 0.0), "the blank world must load")
	input_manager._refresh_after_world_load()
	input_manager.set_simulation_speed(0.0)
	var ready_deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while not input_manager._world_ready and Time.get_ticks_msec() < ready_deadline:
		await process_frame
	_expect(input_manager._world_ready, "the world must become ready")
	var tool: Node = main.get_node("RoadTool")
	tool.active = true
	tool.set_lanes(1, 1)
	input_manager.current_tool = input_manager.Tool.ROAD

	var start := _ground(START)
	var end := _ground(END)
	tool._begin_stroke(start)
	tool.set_scripted_pointer(true, end)
	var frame := _frames(tool, 1)
	_expect(Array(frame.get("draft_profiles", [])) == [0], "an untouched draft follows the ground, got %s" % [frame.get("draft_profiles")])
	var handles: Array = frame.get("height_handles", [])
	_expect(handles.size() == 2 and not handles[0]["locked"], "a free start and the live end have handles: %s" % [handles])

	# The drag: a press on the start's grip, the pointer up 5 m, the release.
	var grip: Vector3 = start + Vector3(0.0, STEM_M, 0.0)
	tool.set_scripted_pointer(true, grip)
	_frames(tool, 1)
	var pressed: Dictionary = _simulation.road_tool_press(0)
	_expect(int(pressed.get("placed", 0)) == 1 and not pressed.has("points"), "the press grabs the grip: %s" % [pressed])
	tool.set_scripted_pointer(true, grip + Vector3(0.0, 5.2, 0.0))
	_frames(tool, 1)
	_simulation.road_tool_release()
	tool.set_scripted_pointer(true, end)
	frame = _frames(tool, 1)
	var raised: Array = frame.get("height_handles", [])
	_expect(raised.size() == 2 and is_equal_approx(float(raised[0]["offset_m"]), 5.0), "the drag steps to 5 m: %s" % [raised])
	_expect(Array(frame.get("draft_profiles", [])) == [2], "a raised start over flat ground is one bridge, got %s" % [frame.get("draft_profiles")])
	var bridge: Dictionary = tool._preview
	_expect(bridge.get("is_valid", false) and bridge.get("has_road_chunks", false), "the bridge previews as a stroke: %s" % bridge.get("invalid_reason", "missing"))

	# PageDown 16 times: the live end 8 m below the ground. A new batch pose draws its ghost.
	tool.step_height(-16)
	frame = _frames(tool, 1)
	_expect(Array(frame.get("draft_profiles", [])) == [2, 3], "a bridge then a tunnel, got %s (%s)" % [frame.get("draft_profiles"), frame.get("draft_reason")])
	_expect(int(_simulation.road_tool_drawn().get("ghost_surfaces", 0)) > 0, "the batch pose draws its ghost")
	_expect(tool._preview.get("is_pending", false), "the batch pose is pending")
	# Resting: tier 2 accepts the batch and its exact pieces replace the ghost.
	for rest in 60:
		if tool._preview.get("has_road_chunks", false) or not tool._preview.get("is_valid", false):
			break
		_frames(tool, 1)
	_expect(tool._preview.get("is_valid", false) and tool._preview.get("has_road_chunks", false), "the batch must preview its pieces: %s" % tool._preview.get("invalid_reason", "missing"))
	_expect(int(_simulation.road_tool_drawn().get("ghost_surfaces", -1)) == 0, "the exact pieces replace the ghost")

	# The click builds the draft as one road; one undo takes it back.
	_expect(tool._commit_segment(end), "the click must queue the draft")
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while tool._commit_request_id > 0 and Time.get_ticks_msec() < deadline:
		await process_frame
	var classes := _classes()
	_expect(classes.has(1) and classes.has(2), "the draft builds a bridge and a tunnel, got %s" % [classes])
	tool.cancel_road()
	tool.set_scripted_pointer(false)
	_expect(_simulation.undo_action(), "the undo must queue")
	deadline = Time.get_ticks_msec() + TIMEOUT_MS
	while int(_simulation.get_road_benchmark_state()["live_edges"]) > 0 and Time.get_ticks_msec() < deadline:
		await process_frame
	_expect(int(_simulation.get_road_benchmark_state()["live_edges"]) == 0, "one undo takes back the whole draft")
	_finish()

func _ground(at: Vector2) -> Vector3:
	return Vector3(at.x, _simulation.get_world_surface_height(at), at.y)

# `count` tool frames; returns the heights of the session's frame after them.
func _frames(tool: Node, count: int) -> Dictionary:
	for i in count:
		tool._process(1.0 / 60.0)
	return _simulation.road_tool_heights()

func _classes() -> Array:
	var classes := []
	for edge in int(_simulation.get_road_benchmark_state()["edge_slots"]):
		classes.append(_simulation.get_edge_class(edge))
	return classes

func _finish() -> void:
	if _failures == 0:
		print("road_height_handles_test: PASS")
		quit(0)
	else:
		push_error("road_height_handles_test: %d failures" % _failures)
		quit(1)
