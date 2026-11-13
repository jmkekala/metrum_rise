# SPDX-License-Identifier: GPL-2.0-only

## Rendered camera-move benchmark (`PERF-23`, `RENDER-32`): the wall-clock companion of the idle
## matrix's fixed-step paths. Loads a world or save, holds the simulation at
## METRUM_STREAM_SPEED (0 by default) and the clock at 10:00, takes the save's own view at pitch
## -0.1 rad and an 800 m orbit, and records every frame of: a still view; 180-degree orbit turns
## over 0.5 s and 1 s (out, back, repeat); a 3 s keyboard-speed pan (out, back, repeat); a zoom
## 800 -> 60 -> 800 m over 1.5 s each, then at the game's default pitch a far zoom-out (800 m to the
## camera's maximum distance) and a far-to-ground zoom (the maximum to street level), each from a
## settled start, the last followed by a settle (`RENDER-32-N`); a 3 km camera jump and back, each until settled; and at
## 23:00 a still, a 1 s turn and, from the settled base pose as by day, a pan (`RENDER-32-I`: the
## night pan started where the night turns left the terrain catching up). `post_load` records
## the frames from the load call's return until settled, with the hand-over's time (`ready_ms`).
## METRUM_STREAM_PHASES picks phases (still, turns, pan, zoom, jump, night, post_load),
## METRUM_STREAM_TURNS the turn durations in seconds, METRUM_STREAM_STILL_S the still's length in
## seconds of wall time instead of its frames (a still with the simulation running, `PL1`). Each row
## also records the wall clock at its frame's end (`t_unix`), to join a profile's samples to frames.
## METRUM_STREAM_SETTLE_CAP_S caps every settle wait (`unsettled_starts` counts those it ended).
## The `census` phase (`RENDER-44`, untimed) writes `draw_census.gd`'s census of the settled base
## pose, three street-level views, a mid-height view and the middle of a turn into
## METRUM_STREAM_CENSUS_DIR. Every sample starts with a `[STREAM] BEGIN <label>` line.
## METRUM_STREAM_DRAWN=1 (an instrumented run, `RENDER-32-N`, `CODE-12`) runs the drawn probe
## (`StreamDrawnProbe`) after every drawn frame: per layer (terrain, water, graded patches and
## tiles, vegetation coarse and fine, buildings) the frames from a chunk entering its layer's
## installed set to drawn, and every in-view hole with its length, counted for the sample the
## chunk entered in (`drawn_probe`, `tools/stream_benchmark.py drawn`); each row also records
## the road hydration's pending chunks and the parked chunks drawn (neither follows the camera).
##
## A row is one frame: its time from one `process_frame` to the next (the frame's scripts and its
## draw), the renderer's CPU and GPU measures, draw counts, the streaming counters of every chunked
## renderer and their change, the reflection's voted levels, and with the perf channel on
## (METRUM_DEBUG_PERF=1, shares only) that frame's PerfDebug records, the frames each vegetation
## patch drawn that frame waited from wanted to drawn, the main thread's core locks by call site
## (`core_locks`, `CODE-26`) and the frame's time split at its draw (`draw_split`, `PL1`).
## "Settled" means the gameplay harness's idle state, `terrain.is_render_settled()` and no
## vegetation upload pending, for SETTLED_FRAMES frames in a row. `./run.sh --benchmark-stream` runs it pinned and isolated.
## With METRUM_STREAM_DIGESTS=1 (an untimed run: it reads the pages back) every settle point
## records the graded tiles' product in `graded_digests`: per installed tile its level, step,
## token, flag, walls and the bytes of each of its copies (the slot layer with its run's place in
## the header masked, and the run), so two builds' settled installs compare by digest whatever
## slots they took (`RENDER-32-B`), with the camera the levels were measured from
## (`level_camera`: XZ and the height above the ground under it), and once the level rule's ends
## in `graded_levels`, read from the rule itself, so `tools/stream_benchmark.py digests` can tell a
## level end's hysteresis band from a difference (`TOOLS-20`). At night it also records the
## street lights' lamp grid and far map textures by digest in `lamp_digests`, and adds two night
## settle points, the jump's pose and back, each far enough to re-centre both (`RENDER-32-H`).
## Every settle point also records the building LOD batches in `building_digests` (per batch its
## chunk, group, count and rows), whichever side draws them (`RENDER-32-L`), and the terrain's and
## water's resident patches with their mesh steps in `patch_digests` (`RENDER-32-M`), and the road
## chunks drawn (per chunk its key, place and every surface's arrays) in `road_digests` and the
## parked cars drawn (per chunk and bucket its rows) in `parked_digests` (`RENDER-32-P`). Settled
## also waits for a load's road chunks to stream in (`RENDER-32-P`; a control draws them at once).
##
## The `water` phase (`RENDER-32-L`) looks at a shore across water, so the water reflection
## draws: the first wet point of a fixed scan out from the world's centre whose neighbour WATER_STEP_M
## south is dry, viewed southwards from over the water at WATER_PITCH_RAD and WATER_DISTANCE_M. It
## records a still sample there (`water_still`, its rows carrying the voted levels) and a settle
## point, and the pose in `water_pose`; a world without such a shore skips it. No input of record
## has water: the pond copy of the 30k save (a 600 x 400 m pond, 3 m deep, north of its last
## street) is the input of this phase.
##
## METRUM_STREAM_SHADOWS (`RENDER-15`, benchmark-only, unset by default) is a comma-separated
## sequence of the sun-shadow variants of `shadow_variants.gd`, applied once the load has settled
## (after `SceneLighting` set the sun) and before the base pose settles. One entry runs the phases
## as usual with that variant. Several run the `still` and `turns` phases once per entry, in
## order, each block from the settled base pose, and tag every sample with its `shadows` variant
## and `block` index; a quartet `full,X,X,full` (or a chain `full,X,X,full,Y,Y,full`) gives each
## variant a C B B C verdict from one load (`tools/stream_benchmark.py split`, then `compare`).
## With METRUM_STREAM_GPU_PASSES=1 and Godot's `--gpu-profile` (Godot then captures its render
## timestamps) every sample records
## `gpu_passes`: per pass, named by its starting mark and that mark's occurrence in the frame, the
## statistics of its GPU milliseconds a frame.
extends RefCounted

const ShadowVariants := preload("res://scripts/benchmarks/shadow_variants.gd")

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
# The far zooms (`RENDER-32-N`): the game's default pitch (CameraNode's INITIAL_PITCH), the
# gameplay camera's distance bounds (input_manager.gd: 10 m to WORLD_CAMERA_MAX_DISTANCE_M) and
# durations at the near zoom's rate (800 -> 60 m in 1.5 s: an e-fold in 0.58 s).
const FAR_PITCH_RAD := -0.785
const FAR_DISTANCE_M := 40000.0
const STREET_DISTANCE_M := 10.0
const FAR_OUT_S := 2.3
const FAR_DOWN_S := 4.8
# The probed layers' heights below and above the world's terrain range (their frustum test):
# ground layers, plants, buildings (metres).
const PROBE_GROUND_M := 2.0
const PROBE_PLANTS_M := 60.0
const PROBE_BUILDINGS_M := 300.0
const JUMP_M := 3000.0
const JUMP_MAX_FRAMES := 3000
const POST_LOAD_MAX_FRAMES := 6000
# CameraNode::pan scales its speed by the orbit distance over INITIAL_DISTANCE_M (400 m).
const PAN_REFERENCE_DISTANCE_M := 400.0
const DEFAULT_PHASES: Array[String] = ["still", "turns", "pan", "zoom", "jump", "night", "water"]
const WATER_STEP_M := 250.0
const WATER_SCAN_M := 250.0
const WATER_SCAN_REACH_M := 8500.0
const WATER_PITCH_RAD := -0.3
const WATER_DISTANCE_M := 400.0
const WATER_FRAMES := 90
# The census phase's street-level and mid-height views (`RENDER-44`): orbit distance and pitch.
const CENSUS_STREET_M := 30.0
const CENSUS_STREET_PITCH := -0.06
const CENSUS_MID_M := 300.0
const CENSUS_MID_PITCH := -0.35
## The keys every frame row carries (`scripts` is empty without the perf channel).
const ROW_KEYS: Array[String] = ["frame", "frame_ms", "process_ms", "render_cpu_ms", "render_gpu_ms",
	"draw_calls", "objects", "primitives", "video_mem", "shadow_pass", "streams", "stream_delta",
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
var _viewport: Viewport
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
var _gpu_profile := false
# A load's road chunk hydration (`RENDER-32-P`; none on a control) and the parked cars.
var _roads: Node
var _parked: Node
# The simulation node when it counts the main thread's core locks (`CODE-26`, perf channel on).
var _core_locks: Node
# The simulation node, for each row's game minute (`PL1`: a frame joins the hour boundary it falls on).
var _simulation: Node
# The frame's draw, bracketed by RenderingServer's frame_pre_draw and frame_post_draw (perf channel
# on, `PL1`): each row splits its time into the frame's process (scripts, then the pre-draw hooks
# connected before this benchmark), its draw and what follows the draw until the next frame resumes
# the benchmark (the next iteration's events and physics).
var _pre_draw_us := 0
var _post_draw_us := 0
# Per pass key, the GPU milliseconds of each frame of the current sample (`--gpu-profile`).
var _gpu_passes := {}
# The drawn probe (METRUM_STREAM_DRAWN=1, `RENDER-32-N`), the stream node, the camera and the
# world's terrain height range the probe tests the frustum with.
var _probe: RefCounted
var _stream: Node
var _camera: Camera3D
var _probe_heights := Vector2.ZERO

func run(bench: Node) -> void:
	var main := bench.get_parent()
	_terrain = main.get_node("Terrain")
	_water = main.get_node("Water")
	_vegetation = main.get_node("Vegetation")
	_buildings = main.get_node("Buildings")
	_road_tool = main.get_node("RoadTool")
	_roads = main.get_node_or_null("NetworkRenderer/RoadChunkHydration")
	_parked = main.get_node_or_null("ParkedCars")
	_simulation = bench.simulation_node
	_grass = main.get_node("Grass")
	_reflection = main.get_node_or_null("SceneLighting/WaterReflection")
	Engine.max_fps = 0
	if bench.mode != "headless":
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	_viewport = bench.get_viewport()
	_viewport_rid = _viewport.get_viewport_rid()
	RenderingServer.viewport_set_measure_render_time(_viewport_rid, true)
	var phases: Array = Array(OS.get_environment("METRUM_STREAM_PHASES").split(",", false))
	if phases.is_empty():
		phases = DEFAULT_PHASES.duplicate()
	var turns: Array = [0.5, 1.0]
	var turn_text := OS.get_environment("METRUM_STREAM_TURNS")
	if not turn_text.is_empty():
		turns = Array(turn_text.split(",", false)).map(func(token: String) -> float: return float(token))
	_scripts = PerfDebug.is_enabled()
	if OS.get_environment("METRUM_STREAM_DRAWN") == "1":
		# By name: a control library without the class still runs the benchmark's other phases.
		_probe = ClassDB.instantiate("StreamDrawnProbe")
		if _probe == null:
			bench._fail("METRUM_STREAM_DRAWN needs a library with StreamDrawnProbe")
			return
		_stream = main.get_node("StreamNode")
		_camera = bench.camera
	_digests = OS.get_environment("METRUM_STREAM_DIGESTS") == "1"
	_gpu_profile = OS.get_environment("METRUM_STREAM_GPU_PASSES") == "1"
	var shadows: Array = Array(OS.get_environment("METRUM_STREAM_SHADOWS").split(",", false))
	for variant: String in shadows:
		if not ShadowVariants.is_known(variant):
			bench._fail("unknown METRUM_STREAM_SHADOWS variant %s" % variant)
			return
	if shadows.size() > 1 and phases.any(func(phase: String) -> bool: return phase not in ["still", "turns", "post_load"]):
		bench._fail("a METRUM_STREAM_SHADOWS sequence runs only the still and turns phases")
		return
	_report = {
		"schema_version": SCHEMA_VERSION, "benchmark": "stream", "success": false, "mode": bench.mode,
		"runtime": bench._runtime_metadata(), "save_path": OS.get_environment("METRUM_IDLE_BENCH_SAVE_PATH"),
		"world_path": bench.world_path, "simulation_speed": float(OS.get_environment("METRUM_STREAM_SPEED")),
		"pitch_radians": PITCH_RAD, "distance_m": DISTANCE_M, "phases": phases, "turn_durations_s": turns,
		"perf_records": _scripts, "load": {}, "samples": [], "graded_digests": [], "lamp_digests": [],
		"building_digests": [], "patch_digests": [], "road_digests": [], "parked_digests": [],
		"water_pose": [], "shadows": shadows, "drawn_probe": {},
		"timing_contract": "frame_ms is the wall time from one process_frame to the next; scripts are the perf channel's records of that same frame (instrumented runs give shares, not times)",
	}
	bench._metrics = _report
	if _scripts:
		PerfDebug.begin_frame_capture()
		if bench.simulation_node.has_method("take_core_lock_stats"):
			_core_locks = bench.simulation_node
			_core_locks.set_core_lock_stats(true)
		RenderingServer.frame_pre_draw.connect(_on_pre_draw)
		RenderingServer.frame_post_draw.connect(_on_post_draw)
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
	# The clock stays stopped until the world is handed over, so a speed set at the load call
	# alone never ran (`RENDER-32-G`): set it again once settled, where the hand-over is done.
	if float(_report.simulation_speed) > 0.0:
		if not bench.input_manager.is_world_ready():
			bench._fail("the settled world was not handed over; the speed cannot be set")
			return
		bench.input_manager.set_simulation_speed(float(_report.simulation_speed))
	var saved_view: PackedFloat32Array = bench.camera.get_view_state()
	var base := saved_view.duplicate()
	base[4] = PITCH_RAD
	base[5] = DISTANCE_M
	var lighting := main.get_node("SceneLighting")
	var sun: DirectionalLight3D = main.get_node("DirectionalLight3D")
	if not shadows.is_empty():
		ShadowVariants.apply(lighting, sun, shadows[0])
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
		# The node renderers' casters by shadow policy category at the settled base pose, with
		# their instances and those within the sun's shadow range of the camera (`RENDER-15`).
		"node_casters": ShadowVariants.node_casters(main, bench.get_viewport().get_camera_3d().global_position,
			sun.directional_shadow_max_distance),
	}
	if _probe != null:
		_probe_heights = _terrain_heights(bench.simulation_node, world)
		RenderingServer.frame_post_draw.connect(_on_probe_post_draw)
	print("[STREAM] LOAD_END agents=%d total_ms=%.1f load_call_ms=%.1f" % [
		_report.load.agent_count, _report.load.elapsed_ms, load_call_ms])
	if phases.has("census") and not await _census(bench, base):
		return
	var pan_m: float = float(bench.camera.speed) * DISTANCE_M / PAN_REFERENCE_DISTANCE_M * PAN_S
	for block in maxi(1, shadows.size()):
		var first_sample: int = _report.samples.size()
		if block > 0:
			ShadowVariants.apply(lighting, sun, shadows[block])
			if not await _start_from(bench, base):
				bench._fail("the shadow block %d's base pose did not settle" % block)
				return
		if not await _still_and_turns(bench, phases, turns, base):
			return
		if shadows.size() > 1:
			for sample: Dictionary in _report.samples.slice(first_sample):
				sample.merge({"shadows": shadows[block], "block": block})
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
		# The far zooms (`RENDER-32-N`), each from a settled start; the last is followed by a
		# settle, so what entered in its last frames draws inside the run.
		var near := base.duplicate()
		near[4] = FAR_PITCH_RAD
		var far := near.duplicate()
		far[5] = FAR_DISTANCE_M
		for zoom: Array in [["far_zoom_out", near, FAR_OUT_S, DISTANCE_M, FAR_DISTANCE_M],
				["far_to_ground", far, FAR_DOWN_S, FAR_DISTANCE_M, STREET_DISTANCE_M]]:
			if not await _start_from(bench, zoom[1]):
				bench._fail("the %s start did not settle" % zoom[0])
				return
			_report.samples.append(await _capture_view(bench, zoom[0], zoom[2], zoom[1], 5, zoom[3], zoom[4], true))
		if not await _wait_settled(bench):
			bench._fail("the far-to-ground zoom did not settle")
			return
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
	if phases.has("water"):
		var water := _water_pose(bench, base, world)
		if not water.is_empty():
			_report.water_pose = Array(water)
			print("[STREAM] WATER_POSE %s" % str(Array(water)))
			if not await _start_from(bench, water):
				bench._fail("the water pose did not settle")
				return
			_report.samples.append(await _capture_frames(bench, "water_still", WATER_FRAMES))
	if _probe != null:
		RenderingServer.frame_post_draw.disconnect(_on_probe_post_draw)
		_report.drawn_probe = _probe.report()
	if _scripts:
		PerfDebug.end_frame_capture()
		if _core_locks != null:
			_core_locks.set_core_lock_stats(false)
		RenderingServer.frame_pre_draw.disconnect(_on_pre_draw)
		RenderingServer.frame_post_draw.disconnect(_on_post_draw)
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

# The `census` phase (`RENDER-44`, untimed; METRUM_STREAM_CENSUS_DIR): `draw_census.gd` at the
# settled base pose, three street-level views (CENSUS_STREET_M from the base pivot, pitch
# CENSUS_STREET_PITCH, a third of a turn apart), a mid-height view, all with the depth image, and
# half a 1 s turn into the base pose's quarter turn (`turn_mid`, unsettled). False when a pose did
# not settle.
func _census(bench: Node, base: PackedFloat32Array) -> bool:
	var census: RefCounted = load("res://scripts/benchmarks/draw_census.gd").new(bench,
		OS.get_environment("METRUM_STREAM_CENSUS_DIR"))
	await census.capture("still", true)
	var poses: Array = []
	for index in 3:
		poses.append(["street_%d" % index, base[3] + TAU * index / 3.0, CENSUS_STREET_PITCH, CENSUS_STREET_M])
	poses.append(["mid", base[3], CENSUS_MID_PITCH, CENSUS_MID_M])
	for pose: Array in poses:
		var view := base.duplicate()
		view[3] = pose[1]
		view[4] = pose[2]
		view[5] = pose[3]
		if not await _start_from(bench, view):
			bench._fail("the census pose %s did not settle" % pose[0])
			return false
		await census.capture(pose[0], true)
	if not await _start_from(bench, base):
		bench._fail("the census turn's start did not settle")
		return false
	# Half of the 1 s turn's 180 degrees, at its pace, then the census of the frame it left.
	var start_us := Time.get_ticks_usec()
	var done := 0.0
	while done < 0.5:
		var target := minf(0.5, float(Time.get_ticks_usec() - start_us) / 1000000.0)
		bench.camera.orbit(Vector2(-TURN_RADIANS * (target - done) / bench.camera.sensitivity, 0.0))
		done = target
		await bench.get_tree().process_frame
	await census.capture("turn_mid", false)
	return await _start_from(bench, base)

# The still and turns phases, if selected; false (the run failed) when a turn's start did not settle.
func _still_and_turns(bench: Node, phases: Array, turns: Array, base: PackedFloat32Array) -> bool:
	if phases.has("still"):
		var still_s := float(OS.get_environment("METRUM_STREAM_STILL_S"))
		if still_s > 0.0:
			_report.samples.append(await _capture_for(bench, "still", still_s))
		else:
			_report.samples.append(await _capture_frames(bench, "still", STILL_FRAMES))
	if phases.has("turns"):
		for duration_variant: Variant in turns:
			var duration := float(duration_variant)
			if not await _start_from(bench, base):
				bench._fail("the turn's start did not settle")
				return false
			for turn: Array in [["turn_out", 1.0], ["turn_back", -1.0], ["turn_repeat", 1.0]]:
				_report.samples.append(await _capture_turn(bench, turn[0], duration, turn[1]))
	return true

# Puts the camera at `view`, waits WARMUP_FRAMES frames and then until settled.
func _start_from(bench: Node, view: PackedFloat32Array) -> bool:
	bench.camera.set_view_state(view)
	for _frame in WARMUP_FRAMES:
		await bench.get_tree().process_frame
	return await _wait_settled(bench)

func _is_settled(bench: Node) -> bool:
	return (bench._is_idle() and _terrain.is_render_settled() and not _vegetation.has_pending_work()
		and (_roads == null or not _roads.is_pending()))

func _wait_settled(bench: Node) -> bool:
	var start_us := Time.get_ticks_usec()
	var stable := 0
	# METRUM_STREAM_SETTLE_CAP_S (`PL1`): a city whose simulation edits it every few frames (100k
	# at 32x) may never settle; past the cap the phase starts unsettled and the report counts it.
	var cap_s := float(OS.get_environment("METRUM_STREAM_SETTLE_CAP_S"))
	while stable < SETTLED_FRAMES:
		if cap_s > 0.0 and bench._elapsed_ms(start_us) > cap_s * 1000.0:
			_report["unsettled_starts"] = int(_report.get("unsettled_starts", 0)) + 1
			return true
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
	_record_building_digest()
	_record_patch_digest()
	_record_road_digest()
	_record_parked_digest()
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
	# Since `RENDER-32-K` part 7 the frame and its camera are the book's.
	var level_camera: Vector3 = tiles._book.level_camera() if tiles._book.has_method("level_camera") else tiles.get("_camera")
	_report.graded_digests.append({"point": _settle_points, "label": label,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else "",
		"camera": [camera.x, camera.y, camera.z], "tiles": lines.size(),
		"level_camera": [level_camera.x, level_camera.y, level_camera.z],
		"digest": _sha("\n".join(lines).to_utf8_buffer()), "by_tile": by_tile})
	if not _report.has("graded_levels"):
		_report.graded_levels = _graded_levels(tiles)

# The graded level rule's ends, metres from the camera: where each of `GL0`-`GL2` ends for a tile
# not held (`level_end_m`) and for one held at that level (`held_level_end_m`, the hysteresis
# band's far side), `GL3`'s step ends, the levels as `[code, step]` and the tile size. The held
# ends are found by asking the rule at a distance. The rule comes from the library under test:
# GradedTileBook's statics, or, on a control built before `RENDER-32-K` part 5 (a series runs
# these scripts on both sides), GradedTileLevels' camera over tile (0, 0)'s centre at height h,
# which is h from it.
func _graded_levels(tiles: Object) -> Dictionary:
	var constants: Dictionary = tiles.get_script().get_script_constant_map()
	var tile_m: float = constants["TILE_M"]
	var book := ClassDB.class_has_method("GradedTileBook", "level_at", true)
	var rule_class := "GradedTileBook" if book else "GradedTileLevels"
	var levels: Object = null if book else ClassDB.instantiate("GradedTileLevels")
	var level_end: PackedFloat64Array = ClassDB.class_call_static(rule_class, "level_end_m")
	var held_ends: Array[float] = []
	for code in level_end.size():
		var lo := 0.0
		var hi := 100000.0
		for _step in 64:
			var mid := (lo + hi) * 0.5
			var level: Vector2i
			if book:
				level = ClassDB.class_call_static("GradedTileBook", "level_at", mid, code)
			else:
				levels.call("move_camera", Vector3(tile_m * 0.5, mid, tile_m * 0.5))
				level = levels.call("rule_level", Vector2i.ZERO, code)
			if level.x <= code:
				lo = mid
			else:
				hi = mid
		held_ends.append(snappedf(hi, 0.001))
	var finite_gl3: Array[float] = []
	for end in ClassDB.class_call_static(rule_class, "gl3_end_m") as PackedFloat64Array:
		if is_finite(end):
			finite_gl3.append(end)
	var steps: Array = []
	for step: Vector2i in constants["LEVEL_STEPS"]:
		steps.append([step.x, step.y])
	return {"tile_m": tile_m, "level_end_m": Array(level_end),
		"held_level_end_m": held_ends, "gl3_end_m": finite_gl3, "level_steps": steps}

# The view of the water phase (see the header): rings of WATER_SCAN_M about the world's centre,
# each walked in a fixed order; empty when no shore is found.
func _water_pose(bench: Node, base: PackedFloat32Array, world: Vector2) -> PackedFloat32Array:
	var reach := minf(WATER_SCAN_REACH_M, minf(world.x, world.y) * 0.5 - WATER_STEP_M)
	var rings := int(reach / WATER_SCAN_M)
	for ring in rings + 1:
		for index in maxi(1, 8 * ring):
			# The ring's square perimeter, from its north-west corner clockwise.
			var side := 2 * ring
			var x := -ring
			var z := -ring
			if ring > 0:
				var leg := index / side
				var along := index % side
				match leg:
					0:
						x = -ring + along
					1:
						x = ring
						z = -ring + along
					2:
						x = ring - along
						z = ring
					_:
						z = ring - along
			var point := Vector2(x, z) * WATER_SCAN_M
			var level: float = bench.simulation_node.get_camera_water_surface_at(point)
			if is_nan(level):
				continue
			if not is_nan(bench.simulation_node.get_camera_water_surface_at(point + Vector2(0.0, -WATER_STEP_M))):
				continue
			var view := base.duplicate()
			view[0] = point.x
			view[1] = level
			view[2] = point.y
			# The camera north of the pivot looking south, across the water to the shore.
			view[3] = 0.0
			view[4] = WATER_PITCH_RAD
			view[5] = WATER_DISTANCE_M
			return view
	return PackedFloat32Array()

# The terrain and water patches at a settle point (`RENDER-32-M`): per resident patch its key,
# mesh step and whether its node draws, from the scripts' own books, so the control's residency
# and the streaming core's compare whichever decides them.
func _record_patch_digest() -> void:
	var entry := {"point": _settle_points,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else ""}
	for layer: String in ["terrain", "water"]:
		var renderer: Node = _terrain if layer == "terrain" else _water
		var lines := PackedStringArray()
		for key: Vector2i in renderer.resident_patch_lookup:
			var patch: Dictionary = renderer.patches.get(key, {})
			var node: MeshInstance3D = patch.get("node", null)
			lines.append("%d,%d,%d,%d" % [key.x, key.y, int(patch.get("lod_step", -1)),
				1 if node != null and node.visible else 0])
		lines.sort()
		entry[layer] = {"resident": lines.size(), "built": renderer.patches.size(),
			"digest": _sha("\n".join(lines).to_utf8_buffer()), "lines": lines}
	_report.patch_digests.append(entry)

# The road chunks drawn at a settle point (`RENDER-32-P`): per chunk its key, place, surface
# layout and every surface's arrays, from the road tool's own index on either side.
func _record_road_digest() -> void:
	var chunks: Variant = _road_tool.get("_road_chunk_instances")
	if chunks == null:
		return
	var lines := PackedStringArray()
	for key: Vector2i in chunks:
		var node: MeshInstance3D = chunks[key]
		if not is_instance_valid(node) or not node.visible or not node.is_inside_tree():
			continue
		var mesh: ArrayMesh = node.mesh
		var surfaces := PackedStringArray()
		for surface in mesh.get_surface_count():
			surfaces.append(_sha(var_to_bytes(mesh.surface_get_arrays(surface))))
		lines.append("%d,%d|%s|%s|%s|%d" % [key.x, key.y, node.position, mesh.get_meta("road_layer_surfaces", []),
			",".join(surfaces), node.get_child_count()])
	lines.sort()
	_report.road_digests.append({"point": _settle_points,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else "",
		"chunks": lines.size(), "digest": _sha("\n".join(lines).to_utf8_buffer())})

# The parked cars drawn at a settle point (`RENDER-32-P`): per chunk and vehicle bucket its rows,
# from the stream backend's slots (`drawn_rows`) or the control's chunk nodes.
func _record_parked_digest() -> void:
	if _parked == null:
		return
	var lines := PackedStringArray()
	if _parked.has_method("drawn_rows"):
		for batch: Dictionary in _parked.drawn_rows():
			lines.append("%d,%d|%d|%s" % [batch.chunk.x, batch.chunk.y, batch.bucket,
				_sha((batch.rows as PackedFloat32Array).to_byte_array())])
	else:
		var chunks: Dictionary = _parked.get("_chunks")
		var meshes: Dictionary = _parked.get("_meshes")
		for key: Vector2i in chunks:
			for view: Node in (chunks[key] as Node).get_children():
				var multimesh: MultiMesh = (view as MultiMeshInstance3D).multimesh
				lines.append("%d,%d|%d|%s" % [key.x, key.y, meshes.find_key(multimesh.mesh),
					_sha(multimesh.buffer.to_byte_array())])
	lines.sort()
	_report.parked_digests.append({"point": _settle_points,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else "",
		"batches": lines.size(), "digest": _sha("\n".join(lines).to_utf8_buffer()), "lines": lines})

# The building LOD batches at a settle point, whichever side draws them: Rust's stream backend
# (`drawn_batches`) or the control's batch nodes (`batches`). Rows read back from the engine.
func _record_building_digest() -> void:
	var lods: Variant = _buildings.get("lod_renderer") if _buildings != null else null
	if lods == null:
		return
	var lines := PackedStringArray()
	if lods.has_method("drawn_batches"):
		for batch: Dictionary in lods.drawn_batches():
			var buffer := RenderingServer.multimesh_get_buffer(batch.multimesh)
			lines.append(_building_line(batch.chunk, batch.part, batch.lod, batch.deserted, batch.scheme,
				int(batch.count), buffer))
	else:
		var nodes: Dictionary = lods.get("batches")
		for key: Vector4i in nodes:
			var node: MultiMeshInstance3D = nodes[key]
			var count := node.multimesh.visible_instance_count if node.visible else 0
			var identity: Dictionary = lods.batch_identity(key)
			lines.append(_building_line(identity.chunk, identity.part, identity.lod, identity.deserted,
				identity.scheme, count, node.multimesh.buffer))
	lines.sort()
	var empty := lines.count("")
	var camera: Vector3 = _terrain.get_viewport().get_camera_3d().global_position
	_report.building_digests.append({"point": _settle_points,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else "",
		"camera": [camera.x, camera.y, camera.z], "batches": lines.size() - empty,
		"digest": _sha("\n".join(lines).to_utf8_buffer())})

# One batch's line: its identity, count and the digest of its first `count` rows; empty when it
# draws nothing.
func _building_line(chunk: Vector2i, part: int, lod: int, deserted: bool, scheme: int, count: int,
		buffer: PackedFloat32Array) -> String:
	if count <= 0:
		return ""
	return "%d,%d|%d|%d|%d|%d|%d|%s" % [chunk.x, chunk.y, part, lod, int(deserted), scheme, count,
		_sha(buffer.slice(0, count * 16).to_byte_array())]

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

# Starts a sample: drops the perf records and the counters' history from before it. Prints
# `[STREAM] BEGIN <label>`, so an outside tracer (`RENDER-46`'s Intel PT captures) can start on it;
# the drawn probe counts the frames that follow for `label`.
func _begin(label: String) -> Array:
	print("[STREAM] BEGIN %s" % label)
	if _probe != null:
		_probe.set_epoch(label)
	if _scripts:
		PerfDebug.take_frame()
	if _core_locks != null:
		_core_locks.take_core_lock_stats()
	_previous_streams = _streams()
	_gpu_passes = {}
	_last_us = Time.get_ticks_usec()
	return []

func _capture_frames(bench: Node, label: String, frames: int) -> Dictionary:
	var rows := _begin(label)
	for _frame in frames:
		await bench.get_tree().process_frame
		rows.append(_row(rows.size()))
	return _finish(label, 0.0, rows)

# Records frames for `duration_s` of wall time (METRUM_STREAM_STILL_S: a still with the simulation
# running, `PL1`).
func _capture_for(bench: Node, label: String, duration_s: float) -> Dictionary:
	var rows := _begin(label)
	var start_us := _last_us
	while float(Time.get_ticks_usec() - start_us) < duration_s * 1000000.0:
		await bench.get_tree().process_frame
		rows.append(_row(rows.size()))
	return _finish(label, duration_s, rows)

# Orbits by TURN_RADIANS over `duration_s` of wall time, each frame by the share of the turn its
# time asks for, so a slow frame turns further (as the player's drag does).
func _capture_turn(bench: Node, label: String, duration_s: float, direction: float) -> Dictionary:
	var rows := _begin(label)
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
	var rows := _begin(label)
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
	var rows := _begin(label)
	var start_us := _last_us
	if start.is_valid():
		start.call()
	var stable := 0
	var settle_ms := -1.0
	var settle_frames := -1
	# The hand-over (`TERRAIN-15`): the first frame the input manager reports the world ready.
	var ready_ms := -1.0
	while rows.size() < max_frames and stable < SETTLED_FRAMES:
		await bench.get_tree().process_frame
		rows.append(_row(rows.size()))
		if ready_ms < 0.0 and bench.input_manager.is_world_ready():
			ready_ms = bench._elapsed_ms(start_us)
		if _is_settled(bench):
			if stable == 0:
				settle_ms = bench._elapsed_ms(start_us)
				settle_frames = rows.size()
			stable += 1
		else:
			stable = 0
	var sample := _finish(label, 0.0, rows)
	sample.merge({"settled": stable >= SETTLED_FRAMES, "settle_ms": settle_ms, "settle_frames": settle_frames,
		"ready_ms": ready_ms})
	if stable >= SETTLED_FRAMES:
		_record_digest(label)
	return sample

func _row(index: int) -> Dictionary:
	var now_us := Time.get_ticks_usec()
	var streams := _streams()
	var row := {
		"frame": index, "engine_frame": Engine.get_process_frames(),
		# The wall clock (CLOCK_REALTIME) at the frame's end: joins a `perf record -k realtime`
		# profile's samples to their frame (`PL1`).
		"t_unix": Time.get_unix_time_from_system(),
		"minute": _simulation.get_current_minute_of_day(),
		"frame_ms": float(now_us - _last_us) / 1000.0,
		"process_ms": Performance.get_monitor(Performance.TIME_PROCESS) * 1000.0,
		"render_cpu_ms": RenderingServer.viewport_get_measured_render_time_cpu(_viewport_rid)
			+ RenderingServer.get_frame_setup_time_cpu(),
		"render_gpu_ms": RenderingServer.viewport_get_measured_render_time_gpu(_viewport_rid),
		"draw_calls": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_DRAW_CALLS_IN_FRAME),
		"objects": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_OBJECTS_IN_FRAME),
		"primitives": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_PRIMITIVES_IN_FRAME),
		"video_mem": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_VIDEO_MEM_USED),
		# The shadow passes' share of the three above: draw calls, objects, primitives (`RENDER-15`).
		"shadow_pass": [_viewport.get_render_info(Viewport.RENDER_INFO_TYPE_SHADOW, Viewport.RENDER_INFO_DRAW_CALLS_IN_FRAME),
			_viewport.get_render_info(Viewport.RENDER_INFO_TYPE_SHADOW, Viewport.RENDER_INFO_OBJECTS_IN_FRAME),
			_viewport.get_render_info(Viewport.RENDER_INFO_TYPE_SHADOW, Viewport.RENDER_INFO_PRIMITIVES_IN_FRAME)],
		"streams": streams, "stream_delta": _delta(_previous_streams, streams),
		"reflection_levels": _reflection_levels(),
		# Taken as this frame's process_frame resumes the benchmark: the records of the frame just
		# timed (its scripts and its pre-draw hooks), not of the one starting.
		"scripts": PerfDebug.take_frame() if _scripts else {},
	}
	if _scripts:
		row["vegetation_drawn_frames"] = _vegetation_drawn_frames()
		# [process, draw, after the draw] in ms; zeros when a hook did not fire inside this frame.
		var bracketed := _last_us <= _pre_draw_us and _pre_draw_us <= _post_draw_us and _post_draw_us <= now_us
		row["draw_split"] = [float(_pre_draw_us - _last_us) / 1000.0, float(_post_draw_us - _pre_draw_us) / 1000.0,
			float(now_us - _post_draw_us) / 1000.0] if bracketed else [0.0, 0.0, 0.0]
	if _core_locks != null:
		# Per call site: tries, busy, waits, wait us, longest wait us (`CODE-26`).
		row["core_locks"] = _core_locks.take_core_lock_stats()
	if _gpu_profile:
		_record_gpu_passes()
	if _probe != null:
		# The layers that do not follow the camera (`RENDER-32-P`): a load's road chunks still to
		# hydrate, and the parked chunks drawn.
		row["roads_pending"] = _stream.get_road_hydration_counts().x
		row["parked_drawn"] = _stream.get_parked_drawn().size()
	_previous_streams = streams
	_last_us = now_us
	return row

# The drawn probe (`RENDER-32-N`), once the frame has drawn: every layer's installed set as its
# owner reports it, against the camera that frame drew with. O(the installed sets).
func _on_probe_post_draw() -> void:
	_probe.begin_frame(_camera.get_frustum())
	var low := _probe_heights.x - PROBE_GROUND_M
	var ground := _probe_heights.y + PROBE_GROUND_M
	for layer: String in ["terrain", "water", "graded", "graded_tiles"]:
		_probe.observe(layer, _stream.probe_stream_layer(layer), low, ground)
	for layer: String in ["vegetation_coarse", "vegetation_fine"]:
		_probe.observe(layer, _stream.probe_stream_layer(layer), low, _probe_heights.y + PROBE_PLANTS_M)
	_probe.observe("buildings", _simulation.probe_building_chunks(), low, _probe_heights.y + PROBE_BUILDINGS_M)

# The terrain's lowest and highest height on a 65 x 65 lattice over the world (once a run).
static func _terrain_heights(simulation: Node, world: Vector2) -> Vector2:
	var low := INF
	var high := -INF
	for i in 65:
		for j in 65:
			var height: float = simulation.get_height_at(Vector2((i / 64.0 - 0.5) * world.x, (j / 64.0 - 0.5) * world.y))
			low = minf(low, height)
			high = maxf(high, height)
	return Vector2(low, high)

func _on_pre_draw() -> void:
	_pre_draw_us = Time.get_ticks_usec()

func _on_post_draw() -> void:
	_post_draw_us = Time.get_ticks_usec()

# The water reflection's voted plane levels, NaN (no plane) as null.
func _reflection_levels() -> Array:
	var levels: Variant = _reflection.get("_voted_levels") if _reflection != null else null
	var out := []
	if levels is PackedFloat32Array:
		for level: float in levels:
			out.append(null if is_nan(level) else level)
	return out

func _finish(label: String, duration_s: float, rows: Array) -> Dictionary:
	if _probe != null:
		_probe.set_epoch("")
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
	if _gpu_profile:
		var passes := {}
		for key: String in _gpu_passes:
			passes[key] = stats(_gpu_passes[key])
		sample["gpu_passes"] = passes
	print("[STREAM] %s %.1fs frames=%d p50=%.3f p95=%.3f max=%.3f over16=%d over22=%d over33=%d levels=%s" % [
		label, duration_s, rows.size(), sample.frame_ms.p50, sample.frame_ms.p95, sample.frame_ms.max,
		over.over_16_7, over.over_22, over.over_33_3, str(rows.back().reflection_levels if not rows.is_empty() else [])])
	return sample

# The last drawn frame's render timestamps (`--gpu-profile`) as intervals, each named by its
# starting mark and that mark's occurrence in the frame (several viewports draw the same passes),
# plus `TOTAL`, first mark to last. Timestamps are nanoseconds.
func _record_gpu_passes() -> void:
	var device := RenderingServer.get_rendering_device()
	var count := device.get_captured_timestamps_count() if device != null else 0
	if count < 2:
		return
	var seen := {}
	for i in count - 1:
		var mark := device.get_captured_timestamp_name(i)
		var occurrence := int(seen.get(mark, 0))
		seen[mark] = occurrence + 1
		var key := "%s#%d" % [mark, occurrence]
		if not _gpu_passes.has(key):
			_gpu_passes[key] = PackedFloat64Array()
		_gpu_passes[key].append(float(device.get_captured_timestamp_gpu_time(i + 1)
			- device.get_captured_timestamp_gpu_time(i)) / 1000000.0)
	if not _gpu_passes.has("TOTAL"):
		_gpu_passes["TOTAL"] = PackedFloat64Array()
	_gpu_passes["TOTAL"].append(float(device.get_captured_timestamp_gpu_time(count - 1)
		- device.get_captured_timestamp_gpu_time(0)) / 1000000.0)

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
		# One read: since `RENDER-32-K` part 7 `counters` is built by the book on each read.
		var counters: Dictionary = tiles.counters
		for key: String in ["uploads", "same_level_uploads", "unready_in_view", "pool_exhausted"]:
			graded[key] = counters.get(key, 0)
		# A control build before RENDER-32-B keeps its levels and wanted tiles in GDScript.
		# A control before RENDER-32-I keeps its requests in GDScript.
		var rust_levels: bool = tiles.has_method("wanted_count")
		var book: bool = "_book" in tiles
		graded.merge({"wanted": tiles.wanted_count() if rust_levels else tiles._wanted.size(),
			"desired": tiles._book.desired_len() if book else (tiles._levels.size() if rust_levels else tiles._desired.size()),
			# Since RENDER-32-J `installed` is a view built from the book on each change: count only.
			"installed": tiles._book.entry_count(false) if book and tiles._book.has_method("entry_count") else tiles.installed.size(),
			"requested": -1 if book else tiles.requested.size(),
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
# it asked one on the main thread). From `RENDER-32-E` on also the stream node's vegetation status
# (installed, pending, slots); from `RENDER-32-F` on the residency is the stream node's too; from
# `RENDER-32-N` on its prepared set.
func _vegetation_counters() -> Dictionary:
	var simulation: Node = _vegetation.simulation
	var stream: Node = _vegetation.stream
	var status: Dictionary = stream.get_stream_status()["vegetation"]
	var counts: Dictionary = stream.get_vegetation_metrics()
	return {"queue": counts["queue"], "patches": counts["patches"],
		"slots_live": int(status["slots_live"]), "slots_spare": int(status["slots_spare"]),
		"stream_pending": int(status["pending"]), "installs_total": int(status["installs_total"]),
		"cache": counts["cached"], "standins": counts["standins"],
		"trees": counts["trees"], "generated_total": counts["generated"],
		"requested": counts["requested"],
		"queries_total": int(simulation.call("get_vegetation_main_thread_queries")),
		# The prepared set (`RENDER-32-N`; -1 on a build without it): products held, bytes,
		# evictions over the cap, products used, dropped (prepared for other bands or generations)
		# and replaced (a drawn patch's own, asked for again).
		"prepared": int(status.get("prepared", -1)), "prepared_bytes": int(status.get("prepared_bytes", -1)),
		"prepared_over_cap": int(status.get("prepared_over_cap", -1)),
		"prepared_used": int(status.get("prepared_used", -1)), "prepared_dropped": int(status.get("prepared_dropped", -1)),
		"prepared_replaced": int(status.get("prepared_replaced", -1))}

# Wanted-to-drawn of the vegetation patches, with the perf channel on only (it walks the wanted
# set every frame): a patch that enters the wanted set drawing nothing waits from that frame,
# and the frames it waited are recorded on the frame it draws; one that enters already drawn
# (built in the frame it was wanted, or kept from before) records 0. A patch no longer wanted
# stops waiting. Drawn is what the stream node draws (`RENDER-32-E`); a fine patch also counts as
# drawn while the coarse patch of its block draws over it (`RENDER-32-F` keeps the fine patches of
# a block hidden until all are installed, so one grid tier draws over an area).
func _vegetation_drawn_frames() -> Array:
	var frame := Engine.get_process_frames()
	var drawn := []
	var stream: Node = _vegetation.stream
	var wanted := {}
	for key: Vector3i in stream.get_vegetation_wanted():
		wanted[key] = true
	for key: Vector3i in wanted:
		var has_patch: bool = stream.is_vegetation_drawn(key) or (key.z > 1 and stream.is_vegetation_drawn(
			Vector3i(floori(float(key.x) / key.z), floori(float(key.y) / key.z), 1)))
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
	_vegetation_wanted = wanted
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
