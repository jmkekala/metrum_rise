# SPDX-License-Identifier: GPL-2.0-only

## Rendered camera-move benchmark (`PERF-23`, `RENDER-32`): the wall-clock companion of the idle
## matrix's fixed-step paths. Loads a world or save, holds the simulation at
## METRUM_STREAM_SPEED (0 by default) and the clock at 10:00, takes the save's own view at pitch
## -0.1 rad and an 800 m orbit, and records every frame of: a still view; 180-degree orbit turns
## over 0.5 s and 1 s (out, back, repeat); a 3 s keyboard-speed pan (out, back, repeat); a zoom
## 800 -> 60 -> 800 m over 1.5 s each; a 3 km camera jump and back, each until settled; and at
## 23:00 a still, a 1 s turn and, from the settled base pose as by day, a pan (`RENDER-32-I`: the
## night pan started where the night turns left the terrain catching up). `post_load` records
## the frames from the load call's return until settled. METRUM_STREAM_PHASES picks phases
## (still, turns, pan, zoom, jump, night, post_load), METRUM_STREAM_TURNS the turn durations in
## seconds.
##
## A row is one frame: its time from one `process_frame` to the next (the frame's scripts and its
## draw), the renderer's CPU and GPU measures, draw counts, the streaming counters of every chunked
## renderer and their change, the reflection's voted levels, and with the perf channel on
## (METRUM_DEBUG_PERF=1, shares only) that frame's PerfDebug records and the frames each vegetation
## patch drawn that frame waited from wanted to drawn. "Settled" means the gameplay
## harness's idle state, `terrain.is_render_settled()` and no vegetation upload pending, for
## SETTLED_FRAMES frames in a row. `./run.sh --benchmark-stream` runs it pinned and isolated.
## With METRUM_STREAM_DIGESTS=1 (an untimed run: it reads the pages back) every settle point
## records the graded tiles' product in `graded_digests`: per installed tile its level, step,
## token, flag, walls and the bytes of each of its copies (the slot layer with its run's place in
## the header masked, and the run), so two builds' settled installs compare by digest whatever
## slots they took (`RENDER-32-B`). At night it also records the street lights' lamp grid and far
## map textures by digest in `lamp_digests`, and adds two night settle points, the jump's pose
## and back, each far enough to re-centre both (`RENDER-32-H`).
extends RefCounted

const SCHEMA_VERSION := 1
const WARMUP_FRAMES := 120
const STILL_FRAMES := 180
const SETTLED_FRAMES := 30
const TURN_RADIANS := PI
const PITCH_RAD := -0.1
const DISTANCE_M := 800.0
const ZOOM_NEAR_M := 60.0
const ZOOM_S := 1.5
const PAN_S := 3.0
const JUMP_M := 3000.0
const JUMP_MAX_FRAMES := 3000
const POST_LOAD_MAX_FRAMES := 6000
# CameraNode::pan scales its speed by the orbit distance over INITIAL_DISTANCE_M (400 m).
const PAN_REFERENCE_DISTANCE_M := 400.0
const DEFAULT_PHASES: Array[String] = ["still", "turns", "pan", "zoom", "jump", "night"]
## The keys every frame row carries (`scripts` is empty without the perf channel).
const ROW_KEYS: Array[String] = ["frame", "frame_ms", "process_ms", "render_cpu_ms", "render_gpu_ms",
	"draw_calls", "objects", "primitives", "video_mem", "streams", "stream_delta",
	"reflection_levels", "scripts"]
const STAT_KEYS: Array[String] = ["mean", "p50", "p95", "p99", "max"]

var _terrain: Node
var _water: Node
var _vegetation: Node
var _buildings: Node
var _road_tool: Node
var _grass: Node
var _reflection: Node
var _report: Dictionary
var _viewport_rid: RID
var _scripts := false
var _last_us := 0
var _previous_streams: Dictionary
var _digests := false
var _settle_points := 0
# Wanted vegetation patches drawing nothing yet, by the engine frame they were first seen so,
# and the wanted set of the last frame.
var _vegetation_waiting := {}
var _vegetation_wanted := {}

func run(bench: Node) -> void:
	var main := bench.get_parent()
	_terrain = main.get_node("Terrain")
	_water = main.get_node("Water")
	_vegetation = main.get_node("Vegetation")
	_buildings = main.get_node("Buildings")
	_road_tool = main.get_node("RoadTool")
	_grass = main.get_node("Grass")
	_reflection = main.get_node_or_null("SceneLighting/WaterReflection")
	Engine.max_fps = 0
	if bench.mode != "headless":
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	_viewport_rid = bench.get_viewport().get_viewport_rid()
	RenderingServer.viewport_set_measure_render_time(_viewport_rid, true)
	var phases: Array = Array(OS.get_environment("METRUM_STREAM_PHASES").split(",", false))
	if phases.is_empty():
		phases = DEFAULT_PHASES.duplicate()
	var turns: Array = [0.5, 1.0]
	var turn_text := OS.get_environment("METRUM_STREAM_TURNS")
	if not turn_text.is_empty():
		turns = Array(turn_text.split(",", false)).map(func(token: String) -> float: return float(token))
	_scripts = PerfDebug.is_enabled()
	_digests = OS.get_environment("METRUM_STREAM_DIGESTS") == "1"
	_report = {
		"schema_version": SCHEMA_VERSION, "benchmark": "stream", "success": false, "mode": bench.mode,
		"runtime": bench._runtime_metadata(), "save_path": OS.get_environment("METRUM_IDLE_BENCH_SAVE_PATH"),
		"world_path": bench.world_path, "simulation_speed": float(OS.get_environment("METRUM_STREAM_SPEED")),
		"pitch_radians": PITCH_RAD, "distance_m": DISTANCE_M, "phases": phases, "turn_durations_s": turns,
		"perf_records": _scripts, "load": {}, "samples": [], "graded_digests": [], "lamp_digests": [],
		"timing_contract": "frame_ms is the wall time from one process_frame to the next; scripts are the perf channel's records of that same frame (instrumented runs give shares, not times)",
	}
	bench._metrics = _report
	if _scripts:
		PerfDebug.begin_frame_capture()
	var settled := false
	var load_start := Time.get_ticks_usec()
	print("[STREAM] LOAD_BEGIN save=%s world=%s" % [_report.save_path, bench.world_path])
	var loaded: bool
	if not str(_report.save_path).is_empty():
		loaded = bench.input_manager.menu_load_game_from_path(str(_report.save_path))
	else:
		loaded = bench._load_benchmark_world()
	if not loaded:
		bench._fail("stream benchmark input failed to load")
		return
	var load_call_ms: float = bench._elapsed_ms(load_start)
	bench.input_manager.set_simulation_speed(float(_report.simulation_speed))
	main.menu_set_time_of_day(10.0)
	if phases.has("post_load"):
		var after_load := await _capture_until_settled(bench, "post_load", POST_LOAD_MAX_FRAMES, Callable())
		_report.samples.append(after_load)
		if not bool(after_load.settled):
			bench._fail("the world did not settle after its load", {"settle_ms": after_load.settle_ms})
			return
	settled = await _wait_settled(bench)
	if not settled:
		bench._fail("the world did not settle after its load")
		return
	var saved_view: PackedFloat32Array = bench.camera.get_view_state()
	var base := saved_view.duplicate()
	base[4] = PITCH_RAD
	base[5] = DISTANCE_M
	settled = await _start_from(bench, base)
	if not settled:
		bench._fail("the base pose did not settle")
		return
	var perf_stats: Dictionary = bench.simulation_node.get_perf_stats()
	var world: Vector2 = bench.simulation_node.get_terrain_world_size()
	_report.load = {
		"load_call_ms": load_call_ms, "elapsed_ms": bench._elapsed_ms(load_start),
		"agent_count": int(perf_stats.get("agent_count", -1)), "saved_view": Array(saved_view),
		"world_size": [world.x, world.y], "streams": _streams(),
	}
	print("[STREAM] LOAD_END agents=%d total_ms=%.1f load_call_ms=%.1f" % [
		_report.load.agent_count, _report.load.elapsed_ms, load_call_ms])
	var pan_m: float = float(bench.camera.speed) * DISTANCE_M / PAN_REFERENCE_DISTANCE_M * PAN_S
	if phases.has("still"):
		_report.samples.append(await _capture_frames(bench, "still", STILL_FRAMES))
	if phases.has("turns"):
		for duration_variant: Variant in turns:
			var duration := float(duration_variant)
			settled = await _start_from(bench, base)
			if not settled:
				bench._fail("the turn's start did not settle")
				return
			for turn: Array in [["turn_out", 1.0], ["turn_back", -1.0], ["turn_repeat", 1.0]]:
				_report.samples.append(await _capture_turn(bench, turn[0], duration, turn[1]))
	if phases.has("pan"):
		settled = await _start_from(bench, base)
		if not settled:
			bench._fail("the pan's start did not settle")
			return
		for pan: Array in [["pan_out", 0.0, pan_m], ["pan_back", pan_m, 0.0], ["pan_repeat", 0.0, pan_m]]:
			_report.samples.append(await _capture_view(bench, pan[0], PAN_S, base, 0, pan[1], pan[2], false))
	if phases.has("zoom"):
		settled = await _start_from(bench, base)
		if not settled:
			bench._fail("the zoom's start did not settle")
			return
		for zoom: Array in [["zoom_in", DISTANCE_M, ZOOM_NEAR_M], ["zoom_out", ZOOM_NEAR_M, DISTANCE_M],
				["zoom_in_repeat", DISTANCE_M, ZOOM_NEAR_M]]:
			_report.samples.append(await _capture_view(bench, zoom[0], ZOOM_S, base, 5, zoom[1], zoom[2], true))
	if phases.has("jump"):
		# A camera jump (a minimap click, a focus on a far building): the pivot moves 3 km towards
		# the world's centre, so the jump stays inside any world, and back.
		settled = await _start_from(bench, base)
		if not settled:
			bench._fail("the jump's start did not settle")
			return
		var direction := -Vector2(base[0], base[2])
		if direction.length() < 1.0:
			direction = Vector2(1.0, 0.0)
		var step := direction.normalized() * minf(JUMP_M, world.x * 0.3)
		var jumped := base.duplicate()
		jumped[0] = base[0] + step.x
		jumped[2] = base[2] + step.y
		for jump: Array in [["jump", jumped], ["jump_back", base]]:
			var sample := await _capture_until_settled(bench, jump[0], JUMP_MAX_FRAMES,
				func() -> void: bench.camera.set_view_state(jump[1]))
			_report.samples.append(sample)
			if not bool(sample.settled):
				bench._fail("%s did not settle" % jump[0], {"settle_ms": sample.settle_ms})
				return
	if phases.has("night"):
		# Street lamps on: their lamp grid, far light map and light pool follow the camera.
		main.menu_set_time_of_day(23.0)
		settled = await _start_from(bench, base)
		if not settled:
			bench._fail("the night pose did not settle")
			return
		_report.samples.append(await _capture_frames(bench, "night_still", STILL_FRAMES))
		for turn: Array in [["night_turn_out", 1.0], ["night_turn_back", -1.0], ["night_turn_repeat", 1.0]]:
			_report.samples.append(await _capture_turn(bench, turn[0], 1.0, turn[1]))
		# The pans start settled, as the day's do.
		settled = await _start_from(bench, base)
		if not settled:
			bench._fail("the night pan's start did not settle")
			return
		for pan: Array in [["night_pan_out", 0.0, pan_m], ["night_pan_back", pan_m, 0.0]]:
			_report.samples.append(await _capture_view(bench, pan[0], PAN_S, base, 0, pan[1], pan[2], false))
		if _digests:
			var direction := -Vector2(base[0], base[2])
			if direction.length() < 1.0:
				direction = Vector2(1.0, 0.0)
			var step := direction.normalized() * minf(JUMP_M, world.x * 0.3)
			var jumped := base.duplicate()
			jumped[0] = base[0] + step.x
			jumped[2] = base[2] + step.y
			for view: PackedFloat32Array in [jumped, base]:
				if not await _start_from(bench, view):
					bench._fail("a night digest pose did not settle")
					return
		main.menu_set_time_of_day(10.0)
	if _scripts:
		PerfDebug.end_frame_capture()
	var errors := report_errors(_report)
	if not errors.is_empty():
		bench._fail("stream report schema", {"errors": Array(errors)})
		return
	_report.success = true
	bench.get_tree().quit(0 if bench._write_metrics() else 1)

## The report's schema problems, empty when it is well formed. Every sample carries a label, its
## frame count equal to its rows, the frame time statistics and the counts over 16.7, 22 and
## 33.3 ms; every row carries ROW_KEYS.
static func report_errors(report: Dictionary) -> PackedStringArray:
	var errors := PackedStringArray()
	for key: String in ["schema_version", "benchmark", "runtime", "phases", "load", "samples"]:
		if not report.has(key):
			errors.append("report lacks %s" % key)
	if errors.is_empty() and (int(report.schema_version) != SCHEMA_VERSION or report.benchmark != "stream"):
		errors.append("report is not stream schema %d" % SCHEMA_VERSION)
	for sample: Dictionary in report.get("samples", []):
		var label := str(sample.get("label", "?"))
		for key: String in ["label", "duration_s", "frames", "frame_ms", "over_16_7", "over_22", "over_33_3", "rows"]:
			if not sample.has(key):
				errors.append("%s lacks %s" % [label, key])
		var rows: Array = sample.get("rows", [])
		if rows.is_empty() or int(sample.get("frames", -1)) != rows.size():
			errors.append("%s: %d frames for %d rows" % [label, int(sample.get("frames", -1)), rows.size()])
		for key: String in STAT_KEYS:
			if not (sample.get("frame_ms", {}) as Dictionary).has(key):
				errors.append("%s: frame_ms lacks %s" % [label, key])
		for row: Dictionary in rows:
			for key: String in ROW_KEYS:
				if not row.has(key):
					errors.append("%s frame %s lacks %s" % [label, str(row.get("frame", "?")), key])
					return errors
	return errors

# Puts the camera at `view`, waits WARMUP_FRAMES frames and then until settled.
func _start_from(bench: Node, view: PackedFloat32Array) -> bool:
	bench.camera.set_view_state(view)
	for _frame in WARMUP_FRAMES:
		await bench.get_tree().process_frame
	return await _wait_settled(bench)

func _is_settled(bench: Node) -> bool:
	return bench._is_idle() and _terrain.is_render_settled() and not _vegetation.has_pending_work()

func _wait_settled(bench: Node) -> bool:
	var start_us := Time.get_ticks_usec()
	var stable := 0
	while stable < SETTLED_FRAMES:
		if bench._elapsed_ms(start_us) > bench.settle_timeout_sec * 1000.0:
			return false
		await bench.get_tree().process_frame
		stable = stable + 1 if _is_settled(bench) else 0
	_record_digest("settled")
	return true

# The graded tiles' settled product (METRUM_STREAM_DIGESTS=1), as the header describes. Reads
# every copy back from its page: a settle point costs seconds, so timed runs leave it off.
func _record_digest(label: String) -> void:
	if not _digests:
		return
	_settle_points += 1
	_record_lamp_digest()
	var tiles: Variant = _terrain.graded_tiles
	if tiles == null:
		return
	var lines := PackedStringArray()
	var by_tile := {}
	var keys: Array = tiles.installed.keys()
	keys.sort()
	for tile: Vector2i in keys:
		var entry: Dictionary = tiles.installed[tile]
		var copies := PackedStringArray()
		for page_id: int in entry["copies"]:
			var page: Dictionary = tiles.page_store.by_id[page_id]
			var copy: Vector3i = entry["copies"][page_id]
			var side := int(tiles.page_store.levels[int(page["code"])]["side"])
			var texels: PackedByteArray = tiles.page_store.layer_data(page, true, copy.x).duplicate()
			# The header's first channel is the copy's first atlas layer: a place, not a product.
			texels.encode_float(side * side * 16, 0.0)
			var rows := PackedByteArray()
			for index in copy.z:
				rows.append_array(tiles.page_store.layer_data(page, false, copy.y + index))
			copies.append("%s/%s" % [_sha(texels), _sha(rows)])
		var unique := {}
		for text in copies:
			unique[text] = true
		var hashes: Array = unique.keys()
		hashes.sort()
		var line := "%d,%d,%d,%d,%d,%s,%s,%s" % [tile.x, tile.y, int(entry["level"]), int(entry["step"]),
			int(entry["token"]), str(bool(entry["behind"])), _sha((entry["walls"] as PackedFloat32Array).to_byte_array()),
			"+".join(PackedStringArray(hashes))]
		lines.append(line)
		by_tile["%d,%d" % [tile.x, tile.y]] = line
	var camera: Vector3 = _terrain.get_viewport().get_camera_3d().global_position
	_report.graded_digests.append({"point": _settle_points, "label": label,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else "",
		"camera": [camera.x, camera.y, camera.z], "tiles": lines.size(),
		"digest": _sha("\n".join(lines).to_utf8_buffer()), "by_tile": by_tile})

# The street lights' two textures as uploaded, read back (night only: by day neither is built).
func _record_lamp_digest() -> void:
	var lights: Variant = _road_tool.get("_street_lights")
	if lights == null:
		return
	var cells: Variant = lights.get("_cell_texture")
	var far: Variant = lights.get("_far_texture")
	if cells == null or far == null:
		return
	var camera: Vector3 = _terrain.get_viewport().get_camera_3d().global_position
	_report.lamp_digests.append({"point": _settle_points,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else "",
		"camera": [camera.x, camera.y, camera.z],
		"cells": _sha((cells as ImageTexture).get_image().get_data()),
		"far": _sha((far as ImageTexture).get_image().get_data())})

static func _sha(bytes: PackedByteArray) -> String:
	var context := HashingContext.new()
	context.start(HashingContext.HASH_SHA256)
	context.update(bytes)
	return context.finish().hex_encode()

# Starts a sample: drops the perf records and the counters' history from before it.
func _begin() -> Array:
	if _scripts:
		PerfDebug.take_frame()
	_previous_streams = _streams()
	_last_us = Time.get_ticks_usec()
	return []

func _capture_frames(bench: Node, label: String, frames: int) -> Dictionary:
	var rows := _begin()
	for _frame in frames:
		await bench.get_tree().process_frame
		rows.append(_row(rows.size()))
	return _finish(label, 0.0, rows)

# Orbits by TURN_RADIANS over `duration_s` of wall time, each frame by the share of the turn its
# time asks for, so a slow frame turns further (as the player's drag does).
func _capture_turn(bench: Node, label: String, duration_s: float, direction: float) -> Dictionary:
	var rows := _begin()
	var start_us := _last_us
	var done := 0.0
	while done < 1.0:
		var target := minf(1.0, float(Time.get_ticks_usec() - start_us) / (duration_s * 1000000.0))
		if target > done:
			bench.camera.orbit(Vector2(-direction * TURN_RADIANS * (target - done) / bench.camera.sensitivity, 0.0))
			done = target
		await bench.get_tree().process_frame
		var row := _row(rows.size())
		row["fraction"] = done
		rows.append(row)
	return _finish(label, duration_s, rows)

# Moves one view-state component (0: the pivot's x, 5: the orbit distance) from `from` to `to` over
# `duration_s` of wall time, linearly or geometrically.
func _capture_view(bench: Node, label: String, duration_s: float, base: PackedFloat32Array,
		component: int, from: float, to: float, geometric: bool) -> Dictionary:
	var rows := _begin()
	var start_us := _last_us
	var fraction := 0.0
	while fraction < 1.0:
		fraction = minf(1.0, float(Time.get_ticks_usec() - start_us) / (duration_s * 1000000.0))
		var view := base.duplicate()
		view[component] = from * pow(to / from, fraction) if geometric else base[component] + lerpf(from, to, fraction)
		bench.camera.set_view_state(view)
		await bench.get_tree().process_frame
		var row := _row(rows.size())
		row["fraction"] = fraction
		rows.append(row)
	return _finish(label, duration_s, rows)

# Runs `start` (if valid) and records frames until settled for SETTLED_FRAMES in a row. The
# sample's settle_ms and settle_frames end at the first frame of that run.
func _capture_until_settled(bench: Node, label: String, max_frames: int, start: Callable) -> Dictionary:
	var rows := _begin()
	var start_us := _last_us
	if start.is_valid():
		start.call()
	var stable := 0
	var settle_ms := -1.0
	var settle_frames := -1
	while rows.size() < max_frames and stable < SETTLED_FRAMES:
		await bench.get_tree().process_frame
		rows.append(_row(rows.size()))
		if _is_settled(bench):
			if stable == 0:
				settle_ms = bench._elapsed_ms(start_us)
				settle_frames = rows.size()
			stable += 1
		else:
			stable = 0
	var sample := _finish(label, 0.0, rows)
	sample.merge({"settled": stable >= SETTLED_FRAMES, "settle_ms": settle_ms, "settle_frames": settle_frames})
	if stable >= SETTLED_FRAMES:
		_record_digest(label)
	return sample

func _row(index: int) -> Dictionary:
	var now_us := Time.get_ticks_usec()
	var streams := _streams()
	var row := {
		"frame": index, "engine_frame": Engine.get_process_frames(),
		"frame_ms": float(now_us - _last_us) / 1000.0,
		"process_ms": Performance.get_monitor(Performance.TIME_PROCESS) * 1000.0,
		"render_cpu_ms": RenderingServer.viewport_get_measured_render_time_cpu(_viewport_rid)
			+ RenderingServer.get_frame_setup_time_cpu(),
		"render_gpu_ms": RenderingServer.viewport_get_measured_render_time_gpu(_viewport_rid),
		"draw_calls": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_DRAW_CALLS_IN_FRAME),
		"objects": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_OBJECTS_IN_FRAME),
		"primitives": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_PRIMITIVES_IN_FRAME),
		"video_mem": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_VIDEO_MEM_USED),
		"streams": streams, "stream_delta": _delta(_previous_streams, streams),
		"reflection_levels": _reflection_levels(),
		# Taken as this frame's process_frame resumes the benchmark: the records of the frame just
		# timed (its scripts and its pre-draw hooks), not of the one starting.
		"scripts": PerfDebug.take_frame() if _scripts else {},
	}
	if _scripts:
		row["vegetation_drawn_frames"] = _vegetation_drawn_frames()
	_previous_streams = streams
	_last_us = now_us
	return row

# The water reflection's voted plane levels, NaN (no plane) as null.
func _reflection_levels() -> Array:
	var levels: Variant = _reflection.get("_voted_levels") if _reflection != null else null
	var out := []
	if levels is PackedFloat32Array:
		for level: float in levels:
			out.append(null if is_nan(level) else level)
	return out

func _finish(label: String, duration_s: float, rows: Array) -> Dictionary:
	var times := PackedFloat64Array()
	var over := {"over_16_7": 0, "over_22": 0, "over_33_3": 0}
	for row: Dictionary in rows:
		var value := float(row.frame_ms)
		times.append(value)
		over.over_16_7 += 1 if value > 16.7 else 0
		over.over_22 += 1 if value > 22.0 else 0
		over.over_33_3 += 1 if value > 33.3 else 0
	var sample := {"label": label, "duration_s": duration_s, "frames": rows.size(),
		"frame_ms": stats(times), "rows": rows}
	sample.merge(over)
	print("[STREAM] %s %.1fs frames=%d p50=%.3f p95=%.3f max=%.3f over16=%d over22=%d over33=%d levels=%s" % [
		label, duration_s, rows.size(), sample.frame_ms.p50, sample.frame_ms.p95, sample.frame_ms.max,
		over.over_16_7, over.over_22, over.over_33_3, str(rows.back().reflection_levels if not rows.is_empty() else [])])
	return sample

## Mean and nearest-rank percentiles of `values`; zeros when empty.
static func stats(values: PackedFloat64Array) -> Dictionary:
	var count := values.size()
	if count == 0:
		return {"mean": 0.0, "p50": 0.0, "p95": 0.0, "p99": 0.0, "max": 0.0}
	var sorted := values.duplicate()
	sorted.sort()
	var total := 0.0
	for value in sorted:
		total += value
	return {"mean": total / count, "p50": sorted[count / 2], "p95": sorted[mini(count - 1, int(count * 0.95))],
		"p99": sorted[mini(count - 1, int(count * 0.99))], "max": sorted[count - 1]}

# Every chunked renderer's queue and residency counters and running totals. O(1) a renderer: sizes
# and counters only, no walk.
func _streams() -> Dictionary:
	var terrain: Dictionary = _terrain.get_pending_render_work_counts()
	terrain.merge({"resident": _terrain.resident_patch_lookup.size(),
		"creates_total": _terrain._terrain_debug_patch_creates, "removes_total": _terrain._terrain_debug_patch_removes,
		"uploads_total": _terrain._terrain_debug_patch_uploads})
	var water: Dictionary = _water.get_pending_render_work_counts()
	water.merge({"resident": _water.resident_patch_lookup.size(),
		"creates_total": _water._water_debug_patch_creates, "removes_total": _water._water_debug_patch_removes,
		"uploads_total": _water._water_debug_patch_uploads})
	var graded := {}
	var tiles: Variant = _terrain.graded_tiles
	if tiles != null:
		for key: String in ["uploads", "same_level_uploads", "unready_in_view", "pool_exhausted"]:
			graded[key] = tiles.counters.get(key, 0)
		# A control build before RENDER-32-B keeps its levels and wanted tiles in GDScript.
		var rust_levels: bool = tiles.has_method("wanted_count")
		graded.merge({"wanted": tiles.wanted_count() if rust_levels else tiles._wanted.size(),
			"desired": tiles._levels.size() if rust_levels else tiles._desired.size(),
			"installed": tiles.installed.size(), "requested": tiles.requested.size(),
			"cell_builds_pending": tiles.cell_builds_pending()})
	var lods: Variant = _buildings.lod_renderer
	return {
		"terrain": terrain, "water": water, "graded": graded,
		"vegetation": _vegetation_counters(),
		"buildings": {"lod_parts": lods.evaluated_parts, "lod_chunks": lods.queried_chunks,
			"upload_bytes": lods.uploaded_bytes},
		"roads": {"chunks": _road_tool._road_chunk_instances.size()},
		"grass": {"mask_renders": _grass.mask_renders, "mask_syncs": _grass.mask_mirror_syncs},
	}

# The vegetation's queue and residency counters. From `RENDER-32-D` on also its requests in
# flight and Rust's count of synchronous vegetation queries (`queries_total`: a frame that moves
# it asked one on the main thread); -1 on a control build without them.
func _vegetation_counters() -> Dictionary:
	var simulation: Node = _vegetation.simulation
	var requested: Variant = _vegetation.get("requested")
	return {"queue": _vegetation.queue.size(), "patches": _vegetation.patches.size(),
		"cache": _vegetation.cache.size(), "standins": _vegetation.standins.size(),
		"trees": _vegetation.tree_count, "generated_total": _vegetation.generated_patches,
		"requested": (requested as Dictionary).size() if requested is Dictionary else -1,
		"queries_total": int(simulation.call("get_vegetation_main_thread_queries"))
			if simulation.has_method("get_vegetation_main_thread_queries") else -1}

# Wanted-to-drawn of the vegetation patches, with the perf channel on only (it walks the wanted
# set every frame): a patch that enters the wanted set drawing nothing waits from that frame,
# and the frames it waited are recorded on the frame it draws; one that enters already drawn
# (built in the frame it was wanted, or kept from before) records 0. A patch no longer wanted
# stops waiting.
func _vegetation_drawn_frames() -> Array:
	var frame := Engine.get_process_frames()
	var drawn := []
	var wanted: Dictionary = _vegetation.wanted_keys
	for key: Vector3i in wanted:
		var has_patch: bool = _vegetation.patches.has(key)
		if not _vegetation_wanted.has(key):
			if has_patch:
				drawn.append(0)
			else:
				_vegetation_waiting[key] = frame
		elif has_patch and _vegetation_waiting.has(key):
			drawn.append(frame - int(_vegetation_waiting[key]))
			_vegetation_waiting.erase(key)
	for key: Vector3i in _vegetation_waiting.keys():
		if not wanted.has(key):
			_vegetation_waiting.erase(key)
	_vegetation_wanted = wanted.duplicate()
	return drawn

# The numeric changes between two counter snapshots, by group; unchanged values left out.
static func _delta(before: Dictionary, after: Dictionary) -> Dictionary:
	var out := {}
	for group: String in after.keys():
		var previous: Dictionary = before.get(group, {})
		var changed := {}
		for key: String in (after[group] as Dictionary).keys():
			var value: Variant = after[group][key]
			if (value is int or value is float) and previous.has(key) and value != previous[key]:
				changed[key] = float(value) - float(previous[key])
		if not changed.is_empty():
			out[group] = changed
	return out
