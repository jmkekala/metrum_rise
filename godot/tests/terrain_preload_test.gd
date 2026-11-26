# SPDX-License-Identifier: GPL-3.0-or-later

## The load's terrain preload (`TERRAIN-59`): a player's load (`_refresh_after_world_load(true)`)
## hands the game over only once the base terrain the terrain memory target keeps is built, both
## layers, and the backdrop is ready. The load's progress (`load_fraction`, what the loading screen
## shows, `UI-15`) is 0 at the load call, never decreases, stays below 1 until the hand-over and is 1
## from it; the stream node's units read `units_done == units_total - 1` exactly when its preload
## is `done`; these API loads open no loading screen. On a 20 km blank world
## (smaller than every preset: the whole map fits) and at a 45 MB target (the keep rectangle), the
## hand-over finds the fill with nothing left and the console's `terrain` report reads "fill left
## 0"; then, the camera still, no terrain or water patch builds, shows or is released. A load
## without the preload (the tools' and tests' loads) never preloads. A load left in the middle of
## its preload (the scene freed, as a quit to the menu does) and a new load after it hand over
## with the payload request books' asserts silent (`TERRAIN-35`). A 512 m world hands over too.
## Headless; the game scene with its input manager running the load flow.
extends SceneTree

const TIMEOUT_MS := 120000
const SMALL_BUDGET_MB := 45.0
const EYE := Vector2(-9000, -9000)
const STILL_FRAMES := 120

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	await _test_preload(20480.0, 0.0, "whole map")
	await _test_preload(20480.0, SMALL_BUDGET_MB, "45 MB")
	await _test_preload(512.0, 0.0, "512 m world")
	await _test_without_preload()
	await _test_cancel()
	print("Terrain preload tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# A fresh game scene with a blank world of `side` metres, the camera at EYE (clamped into it).
func _new_world(side: float) -> Node:
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	var sim: Node = main.get_node("SimulationNode")
	_expect(sim.create_blank_world(side, side, 8.0, 0.0), "the %.0f m blank world must load" % side)
	sim.set_simulation_speed(0.0)
	var eye := EYE.clampf(-side * 0.4, side * 0.4)
	main.get_node("CameraNode").focus_on(Vector3(eye.x, sim.get_world_surface_height(eye), eye.y), 120.0)
	return main

# A player's load: the hold until the hand-over, then STILL_FRAMES frames with nothing built.
func _test_preload(side: float, budget_mb: float, what: String) -> void:
	var main: Node = await _new_world(side)
	var input: Node = main.get_node("InputManager")
	var stream: Node = main.get_node("StreamNode")
	input._refresh_after_world_load(true)
	# A target set right after the load call (as Options would): the preload follows it.
	if budget_mb > 0.0:
		stream.set_terrain_budget_mb(budget_mb)
	_expect(stream.is_terrain_preloading(), "%s: a player's load preloads" % what)
	# The view's phase first: before the load's first frame nothing is pending, yet the fill waits.
	_expect(int(stream.get_terrain_preload().get("phase", -1)) == 1, "%s: the preload starts with the view" % what)
	_expect(input.load_fraction() == 0.0, "%s: the progress is 0 at the load call" % what)
	var start := Time.get_ticks_msec()
	var fraction := 0.0
	var units_seen := 0
	var frames := 0
	# The first frame the view was in step, and the first a kept (hidden) patch was built.
	var in_step_frame := -1
	var kept_frame := -1
	while not input.is_world_ready() and Time.get_ticks_msec() - start < TIMEOUT_MS:
		if in_step_frame < 0 and bool(stream.get_terrain_preload().get("in_step", false)):
			in_step_frame = frames
		await process_frame
		frames += 1
		if kept_frame < 0 and int(stream.get_terrain_budget().get("terrain_kept", 0)) > 0:
			kept_frame = frames
		# The progress never goes back and stays below 1 until the hand-over.
		var shown: float = input.load_fraction()
		_expect(shown >= fraction, "%s: the progress went back from %f to %f" % [what, fraction, shown])
		_expect(input.is_world_ready() == (shown == 1.0), "%s: progress %f, world ready %s" % [what, shown, input.is_world_ready()])
		fraction = shown
		var state: Dictionary = stream.get_terrain_preload()
		var total := int(state.get("units_total", 0))
		if stream.is_terrain_preloading() and total > 0:
			units_seen = maxi(units_seen, total)
			var done := int(state.get("units_done", -1))
			_expect(done <= total - 1 and (done == total - 1) == bool(state.get("done", false)),
				"%s: units %d of %d, done %s" % [what, done, total, state.get("done")])
		_expect(root.get_node_or_null("LoadingScreen") == null, "%s: an API load opens no loading screen" % what)
	_expect(input.is_world_ready(), "%s: the preload must hand over" % what)
	var status: Dictionary = stream.get_terrain_budget()
	var state: Dictionary = stream.get_terrain_preload()
	print("[terrain_preload] %s: hand-over after %d frames, %.2f s; units %d; %s"
		% [what, frames, float(Time.get_ticks_msec() - start) / 1000.0, units_seen, status])
	_expect(kept_frame < 0 or in_step_frame >= 0 and kept_frame > in_step_frame,
		"%s: the fill builds only after the view is in step (frames %d, %d)" % [what, in_step_frame, kept_frame])
	_expect(not stream.is_terrain_preloading() and input.load_fraction() == 1.0,
		"%s: the hand-over ends the preload, its progress at 1" % what)
	_expect(int(status.get("fill_left", -1)) == 0, "%s: nothing left to fill at the hand-over: %s" % [what, status])
	_expect(int(state.get("left", -1)) == 0, "%s: nothing left of the preload: %s" % [what, state])
	_expect(bool(stream.get_backdrop().get("ready", false)), "%s: the backdrop is ready" % what)
	var built := int(status.get("terrain_built", 0))
	if budget_mb > 0.0:
		_expect(built <= int(status.get("places", 0)) and int(status.get("counted_bytes", 0)) <= int(status.get("budget_bytes", 0)),
			"%s: the preload stays within the target: %s" % [what, status])
		_expect(built < 1600, "%s: the target keeps less than the map: %s" % [what, status])
	else:
		var grid := ceili(side / 512.0) * ceili(side / 512.0)
		_expect(built == grid and int(status.get("water_built", 0)) == grid,
			"%s: the whole map is built, both layers: %s" % [what, status])
	if frames > 2:
		_expect(units_seen > 2, "%s: the preload counted its units" % what)
	var console: Node = root.get_node_or_null("DebugConsole")
	if console != null:
		console._run_command("terrain")
		var text: String = console._view._status.text
		print("[terrain_preload] %s console at the hand-over:\n%s" % [what, text])
		_expect(text.contains("fill left 0"), "%s: the console reads fill left 0: %s" % [what, text])
	# The camera still: no base terrain or water patch builds, shows or is released.
	stream.take_patch_unit_times()
	for frame in STILL_FRAMES:
		await process_frame
	var units: Dictionary = stream.take_patch_unit_times()
	var after: Dictionary = stream.get_terrain_budget()
	for layer in ["terrain", "water"]:
		for kind in ["build", "prewarm", "show", "release"]:
			var count := int((units.get("%s_%s" % [layer, kind], {}) as Dictionary).get("count", 0))
			_expect(count == 0, "%s: %d %s %s units after the hand-over" % [what, count, layer, kind])
		_expect(int(after.get(layer + "_built", -1)) == int(status.get(layer + "_built", -2)),
			"%s: the %s built count holds after the hand-over" % [what, layer])
	main.queue_free()
	await process_frame

# The tools' load: the hold hands over as before, without a preload.
func _test_without_preload() -> void:
	var main: Node = await _new_world(20480.0)
	var input: Node = main.get_node("InputManager")
	var stream: Node = main.get_node("StreamNode")
	input._refresh_after_world_load()
	var start := Time.get_ticks_msec()
	var preloaded: bool = stream.is_terrain_preloading()
	while not input.is_world_ready() and Time.get_ticks_msec() - start < TIMEOUT_MS:
		await process_frame
		preloaded = preloaded or stream.is_terrain_preloading() \
			or input.load_fraction() > 0.0 and not input.is_world_ready()
	_expect(input.is_world_ready() and not preloaded, "a load without the preload hands over as before")
	main.queue_free()
	await process_frame

# A load left during its preload (the scene freed: a quit to the menu), then a new player's load.
func _test_cancel() -> void:
	var main: Node = await _new_world(20480.0)
	var input: Node = main.get_node("InputManager")
	var stream: Node = main.get_node("StreamNode")
	input._refresh_after_world_load(true)
	for frame in 6:
		await process_frame
	var state: Dictionary = stream.get_terrain_preload()
	_expect(stream.is_terrain_preloading() and int(state.get("left", 0)) > 0,
		"the load is left in the middle of its preload: %s" % state)
	main.queue_free()
	for frame in 30:
		await process_frame
	await _test_preload(20480.0, 0.0, "after a left load")
