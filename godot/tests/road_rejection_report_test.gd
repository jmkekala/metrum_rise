# SPDX-License-Identifier: GPL-2.0-only

## Refused road strokes are explained on `events` with their exact inputs, the console dump names
## its save and road generation, and a replay against that save reproduces each refusal.
extends SceneTree

const DebugConsoleScript := preload("res://scripts/ui/debug_console/debug_console.gd")
const Replay := preload("res://scripts/ui/debug_console/road_attempt_replay.gd")
const WORLD_M := 2048.0
const TIMEOUT_FRAMES := 600
# Half a metre: too short for every check. Sent straight to the commit, as a stale click would be.
const SHORT_EARLY := [Vector3(-60.25, 0.0, 30.5), Vector3(-59.75, 0.0, 30.5)]
const SHORT_LATE := [Vector3(80.125, 0.0, -20.0), Vector3(80.625, 0.0, -20.0)]
const ROAD := [Vector3(-200, 0, -100), Vector3(-100, 0, -100)]
# A loop out of and back into the road's east end: the tool check refuses it.
const LOOP := [Vector3(-100, 0, -100), Vector3(-150, 0, -50), Vector3(-100.2, 0, -100.1)]

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var console: Node = root.get_node_or_null("DebugConsole")
	if console == null:
		console = DebugConsoleScript.new()
		console.name = "DebugConsole"
		root.add_child(console)
	var host := Node3D.new()
	host.name = "Main"
	root.add_child(host)
	# The console finds the simulation in the current scene, as in the game.
	current_scene = host
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(WORLD_M, WORLD_M, 8.0, 512.0, 0.0), "fixture world must be created")
	var first_seq: int = MetrumDebug.read_since(0, 0)["last_seq"]

	# Commit refusal: the simulation explains and records the stroke.
	var result := await _commit(simulation, SHORT_EARLY, 2, 1, false)
	_expect(not result.get("committed", true) and result.get("reason") == "too_short", "a short stroke must be refused as too_short, got %s" % result)
	# A built road changes the road generation, so the first attempt predates the save.
	result = await _commit(simulation, ROAD, 1, 1, true)
	_expect(result.get("committed", false), "the fixture road must commit: %s" % result)
	# Tool refusal: the road tool hands its validation to Rust for the same report.
	var points := PackedVector3Array(LOOP)
	var validation: Dictionary = simulation.validate_road_candidate_for_commit_with_snap(points, 1, 1, true)
	_expect(validation.get("invalid_reason") == "same_node_connection", "the tool check must refuse the loop: %s" % validation.get("invalid_reason"))
	var tool_attempt: int = simulation.log_road_tool_rejection(points, 1, 1, true, validation)
	_expect(tool_attempt > 0, "the tool report must return its attempt number")
	result = await _commit(simulation, SHORT_LATE, 1, 1, true)
	_expect(result.get("reason") == "too_short", "the late stroke must be refused: %s" % result)

	var lines := _event_lines(first_seq)
	var commit_why := _find(lines, "road rejected by commit: too_short")
	_expect(commit_why.contains("Stroke (-60.2, 30.5) -> (-59.8, 30.5), 2 points, 0.5 m, lanes 2/1, snap off"), "the commit report must place the stroke: %s" % commit_why)
	var tool_why := _find(lines, "road rejected by tool: same_node_connection: both ends snap to node ")
	_expect(tool_why.contains("Stroke (-100.0, -100.0) -> (-100.2, -100.1), 3 points"), "the tool report must place the stroke: %s" % tool_why)
	_expect(tool_why.ends_with("Attempt %d." % tool_attempt), "the tool report must end with its attempt number: %s" % tool_why)
	var records := Replay.parse_dump("\n".join(lines))["attempts"] as Array
	_expect(records.size() == 3, "each refusal must record one road_attempt, got %d" % records.size())
	if records.size() == 3:
		_expect(records.map(func(r: Dictionary) -> String: return r["by"]) == ["commit", "tool", "commit"], "attempts must keep their order and source")
		_expect(int(records[1]["attempt"]) == tool_attempt, "the record must carry the tool's attempt number")
		var replayed: Array = records[0]["points"]
		_expect(Vector3(replayed[0][0], replayed[0][1], replayed[0][2]) == SHORT_EARLY[0], "records must keep exact coordinates")

	# The console dump saves the game first and names that save and its road generation.
	await console._dump()
	var status: String = console._view._status.text
	var dump_path := status.get_slice("\n", 0).trim_prefix("Dump written to ")
	var parsed := Replay.parse_dump(FileAccess.get_file_as_string(dump_path))
	_expect(parsed["attempts"].size() == 3, "the dump must hold all three attempts, got %d" % parsed["attempts"].size())
	_expect(parsed["save_path"].ends_with(".sqlite") and FileAccess.file_exists(parsed["save_path"]), "the dump must name its save: %s" % parsed["save_path"])
	_expect(parsed["save_generation"] == simulation.get_road_tool_surface_generation(), "the dump must record the saved road generation")
	host.queue_free()
	await process_frame

	# Replay against a fresh load of the dumped save.
	var replay_host := Node3D.new()
	root.add_child(replay_host)
	var replay := SimulationNode.new()
	replay_host.add_child(replay)
	_expect(replay.load_game(parsed["save_path"]), "the dumped save must load")
	var verdicts: Array[Dictionary] = []
	for attempt in parsed["attempts"]:
		var outcome: Dictionary = await Replay.replay_attempt(replay, self, attempt)
		verdicts.append(Replay.verdict(attempt, outcome, parsed["save_generation"]))
	if verdicts.size() == 3:
		_expect(verdicts.all(func(v: Dictionary) -> bool: return v["reproduced"]), "every refusal must reproduce: %s" % [verdicts.map(func(v: Dictionary) -> String: return v["line"])])
		_expect(not verdicts[0]["same_network"], "the attempt before the built road must be flagged as made on an older network")
		_expect(verdicts[1]["same_network"] and verdicts[2]["same_network"], "attempts after the built road were made on the saved network")
		_expect(verdicts[1]["line"].contains("reproduced, refused by tool"), "the loop must be refused by the tool again: %s" % verdicts[1]["line"])
		_expect(verdicts[0]["line"].contains("the network changed since"), "an older attempt must say so: %s" % verdicts[0]["line"])
		# A record whose reason no longer happens is reported, not hidden.
		var changed: Dictionary = parsed["attempts"][2].duplicate()
		changed["reason"] = "bridge_clearance"
		var outcome: Dictionary = await Replay.replay_attempt(replay, self, changed)
		var verdict := Replay.verdict(changed, outcome, parsed["save_generation"])
		_expect(not verdict["reproduced"] and verdict["line"].contains("NOT reproduced: refused by tool for too_short"), "a different reason must not count as reproduced: %s" % verdict["line"])
		# ROAD-53: the click check now refuses what the commit refuses.
		_expect(verdicts[2]["line"].contains("reproduced, refused by tool"), "the short stroke must be refused at the click: %s" % verdicts[2]["line"])
	replay_host.queue_free()
	await process_frame
	print("Road rejection report tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

func _commit(simulation: Node, stroke: Array, fwd: int, bkw: int, snap: bool) -> Dictionary:
	var request: int = simulation.add_road_with_snap(PackedVector3Array(stroke), fwd, bkw, snap)
	for frame in TIMEOUT_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			return result
		await process_frame
	_expect(false, "road commit timed out")
	return {}

func _event_lines(after_seq: int) -> Array:
	var read: Dictionary = MetrumDebug.read_since(after_seq, 4096)
	var lines := []
	for i in read["message"].size():
		if read["channel"][i] == "events":
			lines.append(read["message"][i])
	return lines

func _find(lines: Array, prefix: String) -> String:
	for line in lines:
		if String(line).begins_with(prefix):
			return line
	_expect(false, "events must contain a line starting with '%s'" % prefix)
	return ""
