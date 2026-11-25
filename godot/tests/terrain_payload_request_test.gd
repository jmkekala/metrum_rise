# SPDX-License-Identifier: GPL-3.0-or-later

## A payload request a road commit overtakes is asked again (`TERRAIN-35`): roads committed in a
## fresh 20 km world's first frames, while the streaming core's terrain and water payload requests
## are in flight, and the world must still settle with every wanted patch resident. On `dev`
## `fefcc00cb` the terrain dropped such a request without telling the core: the patches along the
## road stayed wanted, marked requested and missing (sky), and the world never settled.
## Headless; the game scene as the load flow builds it, without its input and camera processing.
extends SceneTree

# Real time the world is given to settle after its roads (a headless 20 km world settles in a few
# seconds; the commits themselves wait for the core).
const SETTLE_TIMEOUT_MS := 90000
const COMMIT_TIMEOUT_MS := 30000

var _failures := 0
var _sim: Node
var _terrain: Node
var _water: Node

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	var input: Node = main.get_node("InputManager")
	input.set_process(false)
	input.set_process_input(false)
	input.set_process_unhandled_input(false)
	var camera: Node = main.get_node("CameraNode")
	camera.set_process(false)
	camera.set_process_input(false)
	camera.set_process_unhandled_input(false)
	_sim = main.get_node("SimulationNode")
	_terrain = main.get_node("Terrain")
	_water = main.get_node("Water")
	_expect(_sim.create_blank_world(20480.0, 20480.0, 8.0, 0.0), "the 20 km blank world must load")
	input._refresh_after_world_load()
	camera.focus_on(Vector3(0, _sim.get_world_surface_height(Vector2.ZERO), 0), 120.0)
	camera.advance_motion(1.0 / 60.0)
	await process_frame
	# The commits run while the first payload requests are in flight (none of the near patches is
	# resident yet): a commit moves the generation of every patch its road touches.
	var counts: Dictionary = _terrain.get_pending_render_work_counts()
	_expect(int(counts.get("payload_requested", 0)) > 0, "payload requests must be in flight at the first commit: %s" % counts)
	var roads := [
		[Vector2(-1100, 0), Vector2(1100, 0), 1], [Vector2(0, 0), Vector2(0, 300), 1],
		[Vector2(400, -400), Vector2(640, -640), 2], [Vector2(-512, -200), Vector2(-512, 200), 1],
	]
	for road: Array in roads:
		_expect(await _commit(road[0], road[1], road[2]), "road %s must commit" % [road])
	var start := Time.get_ticks_msec()
	var settled := false
	while Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		if _terrain.is_render_settled() and not _water.has_pending_render_work(false):
			settled = true
			break
	counts = _terrain.get_pending_render_work_counts()
	var water_counts: Dictionary = _water.get_pending_render_work_counts()
	print("[terrain_payload_request] settled=%s after %d ms terrain %s water %s" % [
		settled, Time.get_ticks_msec() - start, counts, water_counts])
	_expect(settled, "the world must settle after roads committed during its first requests")
	_expect(int(counts.get("terrain_missing", -1)) == 0, "every wanted terrain patch must be resident: %s" % counts)
	# The streaming core's counts hold both layers' (the water wants the terrain's resident patches).
	_expect(int(counts.get("water_missing", -1)) == 0, "every wanted water patch must be resident: %s" % counts)
	print("Terrain payload request tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

func _commit(a: Vector2, b: Vector2, lanes: int) -> bool:
	var points := PackedVector3Array([
		Vector3(a.x, _sim.get_world_surface_height(a), a.y), Vector3(b.x, _sim.get_world_surface_height(b), b.y)])
	var request: int = _sim.add_road_with_snap(points, lanes, lanes, true)
	var deadline := Time.get_ticks_msec() + COMMIT_TIMEOUT_MS
	while Time.get_ticks_msec() < deadline:
		var result: Variant = _sim.get_road_commit_result(request)
		if result is Dictionary:
			if not result.get("committed", false):
				push_error("road %s: %s" % [points, result])
			return bool(result.get("committed", false))
		await process_frame
	return false
