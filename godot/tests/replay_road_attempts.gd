# SPDX-License-Identifier: GPL-2.0-only

## Replays the refused road strokes of a debug dump against its save (`run.sh --replay-road-attempts`).
##
## Arguments after `--`: `--dump <path>` (a console dump or captured stdout), optional
## `--save <path>` (defaults to the save the dump names) and optional `--packs <dir>` (the asset
## packs to load before the save). Exits 0 when every attempt made on the saved road network is
## refused again for the same reason, 1 when one is not, and 2 on bad input.
extends SceneTree

const Replay := preload("res://scripts/ui/debug_console/road_attempt_replay.gd")

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	var args := _arguments()
	var text := FileAccess.get_file_as_string(String(args.get("dump", "")))
	if text.is_empty():
		_fail("Cannot read the dump %s" % args.get("dump", "(missing --dump)"))
		return
	var parsed := Replay.parse_dump(text)
	var attempts: Array = parsed["attempts"]
	var save := String(args.get("save", parsed["save_path"]))
	if attempts.is_empty():
		_fail("The dump has no road_attempt lines")
		return
	if save.is_empty():
		_fail("The dump names no save; pass --save")
		return
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	root.add_child(simulation)
	simulation.set_simulation_speed(0.0)
	if args.has("packs"):
		var warnings: String = simulation.load_all_asset_packs(String(args["packs"]))
		if not warnings.is_empty():
			print("Asset pack warnings:\n%s" % warnings)
	if not simulation.load_game(save):
		_fail("Cannot load the save %s" % save)
		return
	print("Replaying %d road attempts against %s (road generation %d at save)" % [attempts.size(), save, parsed["save_generation"]])
	var failed := 0
	for attempt in attempts:
		var outcome: Dictionary = await Replay.replay_attempt(simulation, self, attempt)
		var verdict := Replay.verdict(attempt, outcome, parsed["save_generation"])
		print(verdict["line"])
		if verdict["same_network"] and not verdict["reproduced"]:
			failed += 1
	print("Road attempt replay: %d of %d attempts on the saved network not reproduced" % [failed, attempts.size()])
	quit(0 if failed == 0 else 1)

func _arguments() -> Dictionary:
	var args := {}
	var user_args := OS.get_cmdline_user_args()
	var index := 0
	while index < user_args.size():
		var key := user_args[index]
		if key.begins_with("--") and index + 1 < user_args.size():
			args[key.substr(2)] = user_args[index + 1]
			index += 2
		else:
			index += 1
	return args

func _fail(message: String) -> void:
	printerr(message)
	quit(2)
