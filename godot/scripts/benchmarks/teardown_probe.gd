# SPDX-License-Identifier: GPL-2.0-only

## Teardown probe (`RENDER-39`, `RENDER-41`; the gameplay harness's `teardown` matrix, meant for
## a rendered run): a new game on the harness's world, a 3 x 3 road grid and a separate road near
## the world's `close` pose, the tool switched to Zoning and back, then METRUM_TEARDOWN_SAVE loaded
## METRUM_TEARDOWN_LOADS times (default 0), the tool switched to Zoning and back in that city,
## with METRUM_TEARDOWN_NEW_GAME=1 a new game over it, and quit. Every step prints
## `[TEARDOWN] begin <step>` and `[TEARDOWN] end <step>`, so Godot's own error lines in the log
## (`Parameter "mesh" is null`, leaked RIDs at exit) are counted per step. With a library that has
## the road hydration's mesh ledger (`RENDER-52` part 2) it also counts, at each step's end, the
## meshes the hydrations made that are still alive: each must be drawn by a chunk node or held by
## a build not swapped in yet, else the probe fails. METRUM_TEARDOWN_PAN=1 moves the camera while
## each load's hydration runs (its builds left behind are dropped), and a road is added in the
## loaded city (the edit replaces hydrated chunks). Otherwise the probe checks only that each
## step ran.
extends RefCounted

const SETTLE_FRAMES := 60
const ZONING_FRAMES := 120
# The roads' place: the idle matrix's `close` pose (land on Kuopio), offset from the world centre.
const ROADS_OFFSET := Vector2(-1500.0, 2000.0)
const GRID_SPACING_M := 90.0

var _bench: Node
# The hydration mesh ledger is on (the library has it); its failures.
var _ledger := false
var _ledger_failures: Array[String] = []
var _pan := false
# The camera moves the last `_settle(true)` made while the hydration ran.
var _pan_moves := 0

func run(bench: Node) -> void:
	_bench = bench
	bench._metrics = {"schema_version": 1, "benchmark": "teardown", "success": false, "steps": []}
	if not bench._nodes_are_ready():
		_finish("unavailable scene dependencies")
		return
	bench.input_manager.set_process_input(false)
	bench.input_manager.set_process_unhandled_input(false)
	_ledger = bench.simulation_node.has_method("set_road_hydration_mesh_ledger")
	if _ledger:
		bench.simulation_node.set_road_hydration_mesh_ledger(true)
	_pan = OS.get_environment("METRUM_TEARDOWN_PAN") == "1"
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
		if not await _settle(_pan):
			_finish("%s did not settle" % step)
			return
		_end(step, {"buildings": bench.simulation_node.get_building_lod_work(), "pan_moves": _pan_moves})
	if loads > 0 and _pan:
		# A road dragged with the road tool inside a hydrated chunk (its preview collapses runs of
		# hydrated chunks), committed and undone: its chunks replace hydrated ones, whose meshes
		# must be freed (with the ledger, the step fails if none was replaced). The first of a
		# fixed list of places whose stroke the tool accepts (a city's chunk is mostly taken).
		_begin("city_road")
		var before: int = bench.simulation_node.get_road_hydration_meshes_alive().size() if _ledger else 0
		var drag := {"committed": false}
		var attempts := 0
		for middle in _hydrated_chunk_places():
			bench.camera.focus_on(bench._surface_point(middle), 400.0)
			await _frames(SETTLE_FRAMES)
			attempts += 1
			drag = await _tool_road(middle - Vector2(60.0, 15.0), middle + Vector2(60.0, 15.0))
			if drag.committed:
				break
		drag["attempts"] = attempts
		if drag.committed:
			var generation: int = bench.simulation_node.get_network_render_generation()
			drag["undone"] = bench.simulation_node.undo_action() and bool(
				(await bench._wait_for_generation(generation, bench.settle_timeout_sec)).get("ok", false))
		await _frames(SETTLE_FRAMES)
		if _ledger:
			drag["replaced"] = before - bench.simulation_node.get_road_hydration_meshes_alive().size()
			if drag.replaced <= 0:
				_ledger_failures.append("city_road replaced no hydrated chunk")
		_end("city_road", drag)
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

# Places (world x, z) inside the chunks whose mesh the hydration made (any resident chunk without
# the ledger): the centre and four quarter points of up to four chunks, from the middle one by
# sorted key outwards.
func _hydrated_chunk_places() -> Array[Vector2]:
	var renderer: Node = _bench.road_tool.chunks
	var alive := {}
	if _ledger:
		for id in _bench.simulation_node.get_road_hydration_meshes_alive():
			alive[id] = true
	var keys: Array = renderer.chunks.keys().filter(func(key: Vector2i) -> bool:
		return not _ledger or alive.has(renderer.chunks[key].mesh.get_instance_id()))
	keys.sort()
	var places: Array[Vector2] = []
	var origin := Vector2(renderer.origin_x_m, renderer.origin_z_m)
	for step in range(mini(4, keys.size())):
		var index := floori(keys.size() / 2.0) + ceili(step / 2.0) * (1 if step % 2 == 1 else -1)
		var key: Vector2i = keys[clampi(index, 0, keys.size() - 1)]
		for quarter: Vector2 in [Vector2(0.5, 0.5), Vector2(0.25, 0.25), Vector2(0.75, 0.25), Vector2(0.25, 0.75), Vector2(0.75, 0.75)]:
			places.append(origin + (Vector2(key) + quarter) * renderer.span_m)
	return places

# Drags a road with the road tool from `from` to `to` in four poses and clicks: whether its preview
# collapsed runs of resident chunks and whether the commit drew.
func _tool_road(from: Vector2, to: Vector2) -> Dictionary:
	var tool: Node = _bench.road_tool
	var generation: int = _bench.simulation_node.get_network_render_generation()
	var segment := {"draw_mode": 0, "fwd_lanes": 1, "bkw_lanes": 1}
	_bench._begin_scripted_road(Vector2.ZERO, segment, _bench._surface_point(from), _bench._surface_point(from.lerp(to, 0.25)))
	var collapsed := 0
	for pose in range(2, 5):
		await _frames(1)
		tool.set_scripted_pointer(true, _bench._surface_point(from.lerp(to, pose / 4.0)))
		tool._queue_preview_update()
	for frame in range(10):
		await _frames(1)
		collapsed = maxi(collapsed, tool._preview_display._collapsed.size())
	var out := {"collapsed_chunks": collapsed, "committed": false}
	if tool._commit_segment(_bench._surface_point(to)):
		tool.active = false
		out.committed = bool((await _bench._wait_for_generation(generation, _bench.settle_timeout_sec)).get("ok", false))
	else:
		tool.cancel_road()
	return out

# Until the load hands the game over and the network and terrain settle, then a fixed tail. With
# `pan`, the camera moves between four points around the world's centre every few frames while
# the road hydration runs.
func _settle(pan: bool = false) -> bool:
	var start_us := Time.get_ticks_usec()
	var stream: Node = _bench.get_parent().get_node_or_null("StreamNode")
	var frame := 0
	var moves := 0
	_pan_moves = 0
	while _bench._elapsed_ms(start_us) < _bench.settle_timeout_sec * 1000.0:
		await _bench.get_tree().process_frame
		frame += 1
		if pan and stream != null and stream.has_road_hydration_pending() and frame % 3 == 0 and moves < 40:
			var corner := Vector2(1.0 if moves % 2 == 0 else -1.0, 1.0 if moves % 4 < 2 else -1.0)
			var middle: Vector2 = _bench.simulation_node.get_terrain_world_size() * 0.5
			_bench.camera.focus_on(_bench._surface_point(middle + corner * 1200.0), 400.0)
			moves += 1
			_pan_moves = moves
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
	if _ledger:
		details = details.merged({"hydration_meshes": _ledger_counts(step)})
	_bench._metrics.steps.append(details.merged({"step": step}))
	print("[TEARDOWN] end %s %s" % [step, JSON.stringify(details)])

# The ledger's meshes still alive: `alive`, `drawn` (a chunk node's mesh or lining caster's,
# shown, held or hidden), `building` (a build's), and `unaccounted` (alive, neither: a leak).
func _ledger_counts(step: String) -> Dictionary:
	var alive: PackedInt64Array = _bench.simulation_node.get_road_hydration_meshes_alive()
	var renderer: Node = _bench.road_tool.chunks
	var drawn := {}
	for list: Array in [renderer.chunks.values(), renderer._held_chunks, renderer._hidden_chunks]:
		for instance in list:
			if not is_instance_valid(instance) or instance.mesh == null:
				continue
			drawn[instance.mesh.get_instance_id()] = true
			var caster := instance.get_node_or_null("LiningShadow") as MeshInstance3D
			if caster != null and caster.mesh != null:
				drawn[caster.mesh.get_instance_id()] = true
	var shown := 0
	for id in alive:
		shown += int(drawn.has(id))
	var building: int = _bench.simulation_node.get_road_hydration_build_meshes()
	var counts := {"made": _bench.simulation_node.get_road_hydration_meshes_made(),
		"alive": alive.size(), "drawn": shown, "building": building,
		"unaccounted": alive.size() - shown - building}
	if counts.unaccounted != 0:
		_ledger_failures.append("%s: %s" % [step, JSON.stringify(counts)])
	return counts

func _finish(failure: String) -> void:
	if failure.is_empty() and not _ledger_failures.is_empty():
		failure = "hydration meshes unaccounted for: " + ", ".join(_ledger_failures)
	_bench._metrics["success"] = failure.is_empty()
	_bench._metrics["failure"] = failure
	if not failure.is_empty():
		push_error("[TEARDOWN] " + failure)
	_bench._write_metrics()
	print("[TEARDOWN] quit")
	_bench.get_tree().quit(0 if failure.is_empty() else 1)
