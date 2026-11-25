# SPDX-License-Identifier: GPL-3.0-or-later

## The base terrain's memory budget is render state only (`TERRAIN-41` B1): the save written after
## one scripted camera walk and edit sequence is byte-equal whether every patch fits or the budget
## is the smallest (kept patches released as the camera moves). The sequence runs three times on a
## fresh 20 km blank world, paused: every patch fits, the smallest budget, every patch fits again
## (the last shows the sequence itself saves byte-equal, so a difference is the budget's).
## A save records the second it was written (`save_meta.saved_at_unix`), so two saves are compared
## byte for byte except within one four-byte window (that integer). Headless; the game scene as the load flow builds it, without its input and camera processing.
extends SceneTree

const SETTLE_TIMEOUT_MS := 90000
const COMMIT_TIMEOUT_MS := 30000
# Below the wanted square: the keep ring is 0 and every patch beyond the square's release ring is
# released as the camera moves.
const SMALLEST_BUDGET_MB := 20.0
const EYES := [Vector2(0, 0), Vector2(7000, 0), Vector2(-7000, 3000), Vector2(0, 0)]
const ROADS := [
	[Vector2(-1100, 0), Vector2(1100, 0), 1], [Vector2(0, 0), Vector2(0, 300), 1],
	[Vector2(400, -400), Vector2(640, -640), 2],
]

var _failures := 0
var _sim: Node
var _terrain: Node
var _water: Node
var _stream: Node
var _input: Node
var _camera: Node

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
	_input = main.get_node("InputManager")
	_input.set_process(false)
	_input.set_process_input(false)
	_input.set_process_unhandled_input(false)
	_camera = main.get_node("CameraNode")
	_camera.set_process(false)
	_camera.set_process_input(false)
	_camera.set_process_unhandled_input(false)
	_sim = main.get_node("SimulationNode")
	_terrain = main.get_node("Terrain")
	_water = main.get_node("Water")
	_stream = main.get_node("StreamNode")
	var saves: Array[PackedByteArray] = []
	var released: Array[int] = []
	for budget_mb: float in [0.0, SMALLEST_BUDGET_MB, 0.0]:
		var result: Array = await _sequence(budget_mb, saves.size())
		saves.append(result[0])
		released.append(result[1])
	print("[terrain_budget_save] save bytes %s, releases %s" % [saves.map(func(s): return s.size()), released])
	_expect(not saves[0].is_empty(), "the sequence must save")
	_expect(_equal_but_the_time(saves[0], saves[2]), "the sequence must save equal when run twice with every patch fitting")
	_expect(released[1] > 0, "the smallest budget must release patches on the walk")
	_expect(_equal_but_the_time(saves[0], saves[1]), "the save must be equal under the smallest and the whole-map budget")
	print("Terrain budget save tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# Whether two saves are equal byte for byte but for bytes within one four-byte window: the save's
# own time (`save_meta.saved_at_unix`, the only field that differs run to run). O(bytes).
static func _equal_but_the_time(a: PackedByteArray, b: PackedByteArray) -> bool:
	if a.size() != b.size():
		return false
	const CHUNK := 65536
	var first := -1
	for start in range(0, a.size(), CHUNK):
		var end := mini(start + CHUNK, a.size())
		if a.slice(start, end) == b.slice(start, end):
			continue
		for index in range(start, end):
			if a[index] != b[index]:
				if first < 0:
					first = index
				elif index - first >= 4:
					return false
	return true

# Runs the walk and the edits at `budget_mb` (0: every patch fits) on a fresh paused world and
# answers [the save's bytes, the patches released on the way].
func _sequence(budget_mb: float, index: int) -> Array:
	_expect(_sim.create_blank_world(20480.0, 20480.0, 8.0, 0.0), "the 20 km blank world must load")
	_sim.set_simulation_speed(0.0)
	_input._refresh_after_world_load()
	_stream.set_terrain_budget_mb(budget_mb)
	var released := 0
	for eye: Vector2 in EYES:
		_camera.focus_on(Vector3(eye.x, _sim.get_world_surface_height(eye), eye.y), 120.0)
		released += await _settle()
	for road: Array in ROADS:
		_expect(await _commit(road[0], road[1], road[2]), "road %s must commit" % [road])
	released += await _settle()
	var path := ProjectSettings.globalize_path("user://terrain_budget_save_%d.sqlite" % index)
	if OS.get_environment("METRUM_KEEP_SAVES") != "":
		path = OS.get_environment("METRUM_KEEP_SAVES").path_join("save_%d.sqlite" % index)
	_expect(_sim.save_game(path), "the world must save")
	var bytes := FileAccess.get_file_as_bytes(path)
	if OS.get_environment("METRUM_KEEP_SAVES") == "":
		DirAccess.remove_absolute(path)
	return [bytes, released]

# Frames until the terrain and water settle; answers the terrain patches released meanwhile
# (built before less built after, never below zero a frame).
func _settle() -> int:
	var start := Time.get_ticks_msec()
	var released := 0
	var built := int(_stream.get_terrain_budget().get("terrain_built", 0))
	while Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		_camera.advance_motion(1.0 / 60.0)
		await process_frame
		var now := int(_stream.get_terrain_budget().get("terrain_built", 0))
		released += maxi(0, built - now)
		built = now
		if _terrain.is_render_settled() and not _water.has_pending_render_work(false):
			return released
	_expect(false, "the world must settle")
	return released

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
