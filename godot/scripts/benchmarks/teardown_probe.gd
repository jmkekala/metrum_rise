# SPDX-License-Identifier: GPL-2.0-only

## Teardown probe (`RENDER-39`, `RENDER-41`; the gameplay harness's `teardown` matrix, meant for
## a rendered run): a new game on the harness's world, a 3 x 3 road grid and a separate road near
## the world's `close` pose, the tool switched to Zoning and back, then METRUM_TEARDOWN_SAVE loaded
## METRUM_TEARDOWN_LOADS times (default 0), the tool switched to Zoning and back in that city,
## with METRUM_TEARDOWN_NEW_GAME=1 a new game over it, and quit. Every step prints
## `[TEARDOWN] begin <step>` and `[TEARDOWN] end <step>`, so Godot's own error lines in the log
## (`Parameter "mesh" is null`, leaked RIDs at exit) are counted per step; the probe itself checks
## nothing but that each step ran (the log is the measurement).
extends RefCounted

const SETTLE_FRAMES := 60
const ZONING_FRAMES := 120
# The roads' place: the idle matrix's `close` pose (land on Kuopio), offset from the world centre.
const ROADS_OFFSET := Vector2(-1500.0, 2000.0)
const GRID_SPACING_M := 90.0

var _bench: Node

func run(bench: Node) -> void:
	_bench = bench
	bench._metrics = {"schema_version": 1, "benchmark": "teardown", "success": false, "steps": []}
	if not bench._nodes_are_ready():
		_finish("unavailable scene dependencies")
		return
	bench.input_manager.set_process_input(false)
	bench.input_manager.set_process_unhandled_input(false)
	_begin("new_game")
	if not bench.input_manager.menu_start_new_game(bench.world_path, {}):
		_finish("the new game failed to start")
		return
	if not await _settle():
		_finish("the new game did not settle")
		return
	_end("new_game")
	_begin("roads")
	var centre: Vector2 = bench.simulation_node.get_terrain_world_size() * 0.5 + ROADS_OFFSET
	bench.camera.focus_on(bench._surface_point(centre), 400.0)
	var roads := 0
	for index in range(3):
		var offset := Vector2(0.0, index * GRID_SPACING_M)
		roads += int(await _road(centre + offset, centre + offset + Vector2(2.0 * GRID_SPACING_M, 0.0)))
		offset = Vector2(index * GRID_SPACING_M, 0.0)
		roads += int(await _road(centre + offset, centre + offset + Vector2(0.0, 2.0 * GRID_SPACING_M)))
	# A second component, apart from the grid.
	roads += int(await _road(centre + Vector2(400.0, 0.0), centre + Vector2(400.0, 200.0)))
	await _frames(SETTLE_FRAMES)
	_end("roads", {"roads": roads})
	await _zoning("")
	var save := OS.get_environment("METRUM_TEARDOWN_SAVE")
	var loads := int(OS.get_environment("METRUM_TEARDOWN_LOADS")) if not save.is_empty() else 0
	for load in range(loads):
		var step := "load_%d" % (load + 1)
		_begin(step)
		if not bench.input_manager.menu_load_game_from_path(save):
			_finish("%s failed" % step)
			return
		if not await _settle():
			_finish("%s did not settle" % step)
			return
		_end(step, {"buildings": bench.simulation_node.get_building_lod_work()})
	if loads > 0:
		# The tool switched in the loaded city, then (METRUM_TEARDOWN_NEW_GAME=1) a new game over it.
		await _zoning("city_")
	if loads > 0 and OS.get_environment("METRUM_TEARDOWN_NEW_GAME") == "1":
		_begin("new_game_2")
		if not bench.input_manager.menu_start_new_game(bench.world_path, {}) or not await _settle():
			_finish("the second new game failed")
			return
		_end("new_game_2", {"buildings": bench.simulation_node.get_building_lod_work()})
	_finish("")

func _zoning(prefix: String) -> void:
	_begin(prefix + "zoning_on")
	_bench.input_manager._toggle_tool(_bench.input_manager.Tool.ZONING)
	await _frames(ZONING_FRAMES)
	_end(prefix + "zoning_on")
	_begin(prefix + "zoning_off")
	_bench.input_manager._toggle_tool(_bench.input_manager.Tool.ZONING)
	await _frames(SETTLE_FRAMES)
	_end(prefix + "zoning_off")

func _road(from: Vector2, to: Vector2) -> bool:
	var generation: int = _bench.simulation_node.get_network_render_generation()
	_bench.simulation_node.add_road(PackedVector3Array([_bench._surface_point(from), _bench._surface_point(to)]), 1, 1)
	var settled: Dictionary = await _bench._wait_for_generation(generation, _bench.settle_timeout_sec)
	return bool(settled.get("ok", false))

# Until the load hands the game over and the network and terrain settle, then a fixed tail.
func _settle() -> bool:
	var start_us := Time.get_ticks_usec()
	while _bench._elapsed_ms(start_us) < _bench.settle_timeout_sec * 1000.0:
		await _bench.get_tree().process_frame
		if (_bench.input_manager.is_world_ready() and not _bench.simulation_node.is_network_dirty()
				and _bench.terrain.is_render_settled()):
			await _frames(SETTLE_FRAMES)
			return true
	return false

func _frames(count: int) -> void:
	for i in range(count):
		await _bench.get_tree().process_frame

func _begin(step: String) -> void:
	print("[TEARDOWN] begin %s" % step)

func _end(step: String, details: Dictionary = {}) -> void:
	_bench._metrics.steps.append(details.merged({"step": step}))
	print("[TEARDOWN] end %s %s" % [step, JSON.stringify(details)])

func _finish(failure: String) -> void:
	_bench._metrics["success"] = failure.is_empty()
	_bench._metrics["failure"] = failure
	if not failure.is_empty():
		push_error("[TEARDOWN] " + failure)
	_bench._write_metrics()
	print("[TEARDOWN] quit")
	_bench.get_tree().quit(0 if failure.is_empty() else 1)
