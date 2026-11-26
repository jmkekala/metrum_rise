# SPDX-License-Identifier: GPL-3.0-or-later

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
## METRUM_STREAM_PHASES picks phases (still, turns, pan, zoom, jump, night, water, water_turns, post_load),
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
## vegetation upload pending, for SETTLED_FRAMES frames in a row: what the view draws is in step;
## the base terrain budget's background fill and releases beyond it are not waited for. `./run.sh --benchmark-stream` runs it pinned and isolated.
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
## water's resident patches with their mesh steps in `patch_digests` (`RENDER-32-M`; with every
## built terrain patch's land cover by digest and how many are stale, `RENDER-40`, and the wet
## resident water patches that draw no mesh of their own step, `TERRAIN-41` L2), and the road
## chunks drawn (per chunk its key, place and every surface's arrays) in `road_digests` and the
## parked cars drawn (per chunk and bucket its rows) in `parked_digests` (`RENDER-32-P`), and the
## vegetation patches drawn (per visible installed patch its key, bands, detail and every batch's
## identity and rows) in `vegetation_digests` (AUDIT-stream-2 finding 7). Settled
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
## The `water_turns` phase (`RENDER-27`, round WR1) turns with the water reflection drawing: from
## the settled base pose when the reflection has a plane there (the Kuopio city, the user's turn
## pose), else from the `water` phase's shore pose (the pond copy of the 30k city), a still
## (`water_turns_still`) and the 180-degree turns of every duration (`water_turn_out`, `_back`,
## `_repeat`), each duration from the settled pose. The pose it took is `water_turns_pose`, with
## `water_turns_from` "base" or "shore"; a world with neither skips it.
##
## METRUM_STREAM_SHADOWS (`RENDER-15`, benchmark-only, unset by default) is a comma-separated
## sequence of the sun-shadow variants of `shadow_variants.gd`, applied once the load has settled
## (after `SceneLighting` set the sun) and before the base pose settles. One entry runs the phases
## as usual with that variant. Several run the `still` and `turns` phases once per entry, in
## order, each block from the settled base pose, and tag every sample with its `shadows` variant
## and `block` index; a quartet `full,X,X,full` (or a chain `full,X,X,full,Y,Y,full`) gives each
## variant a C B B C verdict from one load (`tools/stream_benchmark.py split`, then `compare`).
## The flight phases (`TERRAIN-41`; not in the default matrix, each meant for a fresh load of a large
## world) travel FLIGHT_M along x through the world's centre at the game's default pitch, looking
## along the flight, the pivot on the ground: `flight_low` and `flight_high` at keyboard pan speed
## for orbit distances FLIGHT_LOW_M and FLIGHT_HIGH_M (CameraNode's speed scaled by the distance),
## `flight_jump` in one frame. Each records its start's settle from the load's pose
## (`<phase>_start`), the motion, a 1 s 180-degree turn right after it (`<phase>_turn`) and the
## settle after it (`<phase>_settle`, at most FLIGHT_SETTLE_MAX_FRAMES). With the drawn probe each
## row carries the terrain's and water's backlog (`backlog`: waiting, oldest wait in frames, waiting
## with the payload ready, in-view holes); the end of every leg but the turn records what a built
## terrain patch holds (`patch_bytes`). METRUM_STREAM_SHOTS=<dir> (an untimed run: it reads the
## frame back) saves the view at a quarter, half and three quarters of each motion and after its
## turn, and of every sample that waits for the view to settle (a jump, a load's settle) its first
## frame, its first frame at 0.5 s and its settled frame (`TERRAIN-41` D2). Every sample records
## the process's memory at its end (`memory`).
## METRUM_TERRAIN_BUDGET_MB (read by terrain.gd, `TERRAIN-41` B1) sets the base terrain's memory
## budget (unset: the Terrain memory setting's default, Automatic, `TERRAIN-41` B2; 0: every patch
## fits); each sample records the budget's state (`terrain_budget`), and
## METRUM_STREAM_WAIT_FILL=1 makes every settle wait for the budget's fill as well.
## METRUM_STREAM_PRELOAD=1 (`TERRAIN-59`, an untimed run) loads as a player's load does
## (`input_manager.player_load`, the loading screen up until the hand-over, `UI-15`; `load_call_ms`
## then includes the screen's first frame): the hand-over waits for the base terrain's preload
## within the target; the hand-over frame's budget state is recorded (`ready_budget`). Unset, the load hands over as before the preload (the tools' load).
## METRUM_STREAM_TAIL_FRAMES=<n> (`TERRAIN-41` B2) records n more frames after a flight's last settle
## (`<phase>_tail`): the budget's background releases and fill, which a settle does not wait for.
## METRUM_STREAM_PROXY_CHECK=1 (`RENDER-75`, an untimed run) records per row the water reflection's
## terrain proxies against the patches the frame drew (`proxies`).
## METRUM_STREAM_BED_CHECK=1 (`TERRAIN-58`, an untimed run: it walks the resident terrain patches and
## the map's patches after each draw) records per row `bed`: [drawn terrain patches in the camera's
## view whose wet water patch does not draw (no water node, hidden, or no mesh of its cover and
## depth), the same over the map, drawn terrain patches in view]; and per sample `cell_spans`:
## [hand-overs from a backdrop cell to the patch, frames a resident patch stayed a cell before
## its hand-over at the median, the 90th percentile and most], read from the stream node's
## whole-map probe, and `bed_max`, the sample's most lake beds in view in a frame.
## METRUM_WATER_RECT_CHECK=1 (`RENDER-76`, an untimed run: the reflection makes every screen
## rectangle the old way too) records per row the reflection's running counts `rect_check`:
## rectangles checked, of them unequal to the walk's, walks the kept hull fell back to.
## The `trip` phase (`VEG-35`; not in the default matrix): from a settled 200 m orbit at the game's
## default pitch (TRIP_LOW_M), the camera jumps TRIP_M along x (at most 0.3 of the world) to a
## TRIP_HIGH_M orbit and zooms in geometrically to TRIP_LOW_M over TRIP_ZOOM_S (`trip_in`, its
## first frame at the high orbit), then holds until settled for SETTLED_FRAMES frames
## (`trip_hold`, capped at TRIP_HOLD_MAX_S after the zoom: a capped run fails). From the jump each
## row records `trip_ms` and, on a library with `count_vegetation_drawn_near`, the wanted
## vegetation patches within TRIP_NEAR_M of the pivot and of them drawn (`near`); the hold records
## `settle_ms`, `veg_done_ms` (the vegetation's last pending work) and `near_ms` (the near ring
## drawn), ms from the jump. Every sample records the vegetation's installs since its start
## (`vegetation_installs`: waits from delivery to install, classes, passes stalled with a hole
## product waiting). With the perf channel on, the report's `trip.keys` holds per wanted patch its
## wanted, product ready, own drawn and left ms and its distance to the pivot
## (`tools/stream_benchmark.py trip`). With `trip` the only phase the base pose is the trip's start.
## With METRUM_STREAM_GPU_PASSES=1 and Godot's `--gpu-profile` (Godot then captures its render
## timestamps) every sample records
## `gpu_passes`: per pass, named by its starting mark and that mark's occurrence in the frame, the
## statistics of its GPU milliseconds a frame.
extends RefCounted

const ShadowVariants := preload("res://scripts/benchmarks/shadow_variants.gd")
const PipelineInventory := preload("res://scripts/benchmarks/pipeline_inventory.gd")

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
# The flight phases (`TERRAIN-41`): the travel, the two orbit distances, the settle cap.
const FLIGHT_M := 20000.0
const FLIGHT_LOW_M := 800.0
const FLIGHT_HIGH_M := 4000.0
const FLIGHT_SETTLE_MAX_FRAMES := 30000
# Built patches whose held bytes a flight settle point measures.
const PATCH_BYTES_SAMPLE := 16
# The trip (`VEG-35`): the low and high orbits, the jump, the zoom-in, the hold's cap after the
# zoom and the near ring's radius.
const TRIP_LOW_M := 200.0
const TRIP_HIGH_M := 20000.0
const TRIP_M := 20000.0
const TRIP_ZOOM_S := 1.5
const TRIP_HOLD_MAX_S := 6.0
const TRIP_NEAR_M := 800.0
const DEFAULT_PHASES: Array[String] = ["still", "turns", "pan", "zoom", "jump", "night", "water", "water_turns"]
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
	"reflection_levels", "scripts", "pipelines"]
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
# METRUM_STREAM_WAIT_FILL=1 (`TERRAIN-41` B1): every settle also waits for the base terrain
# budget's fill (the kept terrain and water patches built hidden), so a phase starts on a filled
# budget.
var _wait_fill := false
# METRUM_STREAM_PRELOAD=1 (`TERRAIN-59`): the load preloads, and a settle waits for its hand-over.
var _preload := false
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
# The open settle wait's frames (`RENDER-43`): what the last sample was, its frame count, worst
# frame and its compilations, the counters at its start and its last frame's end.
var _wait := {}
# METRUM_STREAM_PROXY_CHECK=1 (`RENDER-75`, an untimed run: it walks every built terrain patch
# after each draw): the water reflection's terrain proxies against the patches the frame drew,
# recorded per row as `proxies` (see _on_proxy_check_post_draw).
var _proxy_checking := false
var _proxy_check := []
# METRUM_STREAM_BED_CHECK=1 (`TERRAIN-58`): the frame's lake-bed count, the patches drawn as a cell
# since which frame, and the sample's hand-overs' spans in frames.
var _bed_checking := false
var _bed_check := []
var _bed_max := 0
var _bed_frame := 0
var _cell_since := {}
var _cell_spans := PackedInt32Array()

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
	_wait_fill = OS.get_environment("METRUM_STREAM_WAIT_FILL") == "1"
	_preload = OS.get_environment("METRUM_STREAM_PRELOAD") == "1"
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
		"vegetation_digests": [],
		"water_pose": [], "water_turns_pose": [], "water_turns_from": "", "shadows": shadows, "drawn_probe": {},
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
	var pipelines_at_load := _pipelines()
	var load_start := Time.get_ticks_usec()
	var load_begin_ms := Time.get_ticks_msec()
	print("[STREAM] LOAD_BEGIN save=%s world=%s" % [_report.save_path, bench.world_path])
	var loaded: bool
	if _preload:
		# A player's load (`UI-15`): the loading screen up from here until the hand-over.
		var player_save := not str(_report.save_path).is_empty()
		loaded = await bench.input_manager.player_load(
			bench.input_manager.LoadKind.SAVE if player_save else bench.input_manager.LoadKind.WORLD_DEFINITION,
			str(_report.save_path) if player_save else bench.world_path)
	elif not str(_report.save_path).is_empty():
		loaded = bench.input_manager.menu_load_game_from_path(str(_report.save_path))
	else:
		loaded = bench._load_benchmark_world()
	if not loaded:
		bench._fail("stream benchmark input failed to load")
		return
	var load_call_ms: float = bench._elapsed_ms(load_start)
	var pipelines_after_call := _pipelines()
	bench.input_manager.set_simulation_speed(float(_report.simulation_speed))
	main.menu_set_time_of_day(10.0)
	if phases.has("post_load"):
		# The drawn probe from the load's first frame on (`TERRAIN-41` D2: what a slow machine
		# shows while the patches arrive).
		if _probe != null:
			_probe_heights = _terrain_heights(bench.simulation_node, bench.simulation_node.get_terrain_world_size())
			RenderingServer.frame_post_draw.connect(_on_probe_post_draw)
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
	if phases == ["trip"]:
		# The trip alone (`VEG-35`'s 60 km row): the base pose is its start, one settle fewer.
		base = _trip_pose(bench.simulation_node.get_terrain_world_size(), false, TRIP_LOW_M)
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
	var menu_ms := _menu_ms(bench)
	var menu_exit_ms: int = int(menu_ms[1]) if menu_ms.size() == 2 else -1
	_report.load = {
		"load_call_ms": load_call_ms, "elapsed_ms": bench._elapsed_ms(load_start),
		"agent_count": int(perf_stats.get("agent_count", -1)), "saved_view": Array(saved_view),
		"world_size": [world.x, world.y], "streams": _streams(),
		# Godot's pipeline compilations (`_pipelines`) at the load call's start, its return and the
		# settled base pose (`RENDER-43`).
		"pipelines": [pipelines_at_load, pipelines_after_call, _pipelines()],
		# The menu's prewarm start and exit (ms from the launch) when launched through the menu; by
		# path, as a test runs this script without the autoloads.
		"menu_ms": menu_ms,
		# From the menu's exit to the load call's start (the scene change and the world scene's
		# start), and what the menu reported of its frames and its prewarm (`RENDER-43`); -1 and
		# empty without the menu. Exit to settled is gap_ms + elapsed_ms.
		"load_begin_ms": load_begin_ms,
		"gap_ms": load_begin_ms - menu_exit_ms if menu_exit_ms >= 0 else -1,
		"menu": _menu_report(bench),
		# The node renderers' casters by shadow policy category at the settled base pose, with
		# their instances and those within the sun's shadow range of the camera (`RENDER-15`).
		"node_casters": ShadowVariants.node_casters(main, bench.get_viewport().get_camera_3d().global_position,
			sun.directional_shadow_max_distance),
	}
	if _probe != null and not RenderingServer.frame_post_draw.is_connected(_on_probe_post_draw):
		_probe_heights = _terrain_heights(bench.simulation_node, world)
		RenderingServer.frame_post_draw.connect(_on_probe_post_draw)
	_proxy_checking = OS.get_environment("METRUM_STREAM_PROXY_CHECK") == "1" and _reflection != null
	if _proxy_checking:
		RenderingServer.frame_post_draw.connect(_on_proxy_check_post_draw)
	_bed_checking = OS.get_environment("METRUM_STREAM_BED_CHECK") == "1"
	if _bed_checking and _stream == null:
		_stream = main.get_node("StreamNode")
		_camera = bench.camera
	if _bed_checking:
		RenderingServer.frame_post_draw.connect(_on_bed_check_post_draw)
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
	if phases.has("trip") and not await _trip(bench, world):
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
	if phases.has("water_turns") and not await _water_turns(bench, turns, base, world):
		return
	for flight: String in ["flight_low", "flight_high", "flight_jump"]:
		if phases.has(flight) and not await _flight(bench, flight, world):
			return
	if _probe != null:
		RenderingServer.frame_post_draw.disconnect(_on_probe_post_draw)
		_report.drawn_probe = _probe.report()
	if _proxy_checking:
		RenderingServer.frame_post_draw.disconnect(_on_proxy_check_post_draw)
	if _bed_checking:
		RenderingServer.frame_post_draw.disconnect(_on_bed_check_post_draw)
	if OS.get_environment("METRUM_STREAM_INVENTORY") == "1":
		_report["inventory"] = PipelineInventory.collect(main)
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
# `still_label` and `turn_prefix` name the samples (the `water_turns` phase's own).
func _still_and_turns(bench: Node, phases: Array, turns: Array, base: PackedFloat32Array,
		still_label := "still", turn_prefix := "") -> bool:
	if phases.has("still"):
		var still_s := float(OS.get_environment("METRUM_STREAM_STILL_S"))
		if still_s > 0.0:
			_report.samples.append(await _capture_for(bench, still_label, still_s))
		else:
			_report.samples.append(await _capture_frames(bench, still_label, STILL_FRAMES))
	if phases.has("turns"):
		for duration_variant: Variant in turns:
			var duration := float(duration_variant)
			if not await _start_from(bench, base):
				bench._fail("the turn's start did not settle")
				return false
			for turn: Array in [["turn_out", 1.0], ["turn_back", -1.0], ["turn_repeat", 1.0]]:
				_report.samples.append(await _capture_turn(bench, turn_prefix + turn[0], duration, turn[1]))
	return true

# The `water_turns` phase (see the header); false (the run failed) when its pose did not settle.
func _water_turns(bench: Node, turns: Array, base: PackedFloat32Array, world: Vector2) -> bool:
	if not await _start_from(bench, base):
		bench._fail("the water turns' base pose did not settle")
		return false
	var pose := base
	var from := "base"
	if _reflection_levels().all(func(level: Variant) -> bool: return level == null):
		pose = _water_pose(bench, base, world)
		from = "shore"
		if pose.is_empty():
			return true
		if not await _start_from(bench, pose):
			bench._fail("the water turns' shore pose did not settle")
			return false
	_report.water_turns_pose = Array(pose)
	_report.water_turns_from = from
	print("[STREAM] WATER_TURNS_POSE %s %s" % [from, str(Array(pose))])
	return await _still_and_turns(bench, ["still", "turns"], turns, pose, "water_turns_still", "water_")

# A flight phase (see the header); false (the run failed) when a settle did not come.
func _flight(bench: Node, phase: String, world: Vector2) -> bool:
	var half := minf(FLIGHT_M, world.x * 0.6) * 0.5
	var distance := FLIGHT_HIGH_M if phase == "flight_high" else FLIGHT_LOW_M
	# Yaw -PI/2: the camera looks along +x, the way a forward pan moves it.
	var pose := func(x: float) -> PackedFloat32Array:
		return PackedFloat32Array([x, _simulation.get_height_at(Vector2(x, 0.0)), 0.0, -PI * 0.5,
			FAR_PITCH_RAD, distance, 0.0])
	var start: PackedFloat32Array = pose.call(-half)
	var end: PackedFloat32Array = pose.call(half)
	for leg: Array in [["_start", start], ["", end]]:
		var label: String = phase + leg[0]
		var sample: Dictionary
		if leg[0] == "" and phase != "flight_jump":
			var speed_m_s: float = float(bench.camera.speed) * distance / PAN_REFERENCE_DISTANCE_M
			sample = await _capture_flight(bench, label, 2.0 * half / speed_m_s, -half, half, pose)
		else:
			sample = await _capture_until_settled(bench, label, FLIGHT_SETTLE_MAX_FRAMES,
				func() -> void: bench.camera.set_view_state(leg[1]))
		_report.samples.append(sample)
		_record_patch_bytes(label, sample)
	_report.samples.append(await _capture_turn(bench, phase + "_turn", 1.0, 1.0))
	_save_shot(phase + "_turn")
	var settle := await _capture_until_settled(bench, phase + "_settle", FLIGHT_SETTLE_MAX_FRAMES, Callable())
	_report.samples.append(settle)
	_record_patch_bytes(phase + "_settle", settle)
	if not bool(settle.settled):
		bench._fail("%s did not settle" % phase, {"settle_ms": settle.settle_ms})
		return false
	var tail := int(OS.get_environment("METRUM_STREAM_TAIL_FRAMES"))
	if tail > 0:
		_report.samples.append(await _capture_frames(bench, phase + "_tail", tail))
	return true

# Moves the pivot along x from `from` to `to` over `duration_s` of wall time, on the ground under it.
func _capture_flight(bench: Node, label: String, duration_s: float, from: float, to: float,
		pose: Callable) -> Dictionary:
	var rows := _begin(label)
	var start_us := _last_us
	var fraction := 0.0
	var shots := [0.25, 0.5, 0.75]
	while fraction < 1.0:
		fraction = minf(1.0, float(Time.get_ticks_usec() - start_us) / (duration_s * 1000000.0))
		bench.camera.set_view_state(pose.call(lerpf(from, to, fraction)))
		await bench.get_tree().process_frame
		var row := _row(rows.size())
		row["fraction"] = fraction
		rows.append(row)
		if not shots.is_empty() and fraction >= float(shots[0]):
			_save_shot("%s_%02d" % [label, int(float(shots.pop_front()) * 100.0)])
	return _finish(label, duration_s, rows)

# METRUM_STREAM_SHOTS (an untimed run): the last drawn frame as `<dir>/<name>.png`.
func _save_shot(name: String) -> void:
	var dir := OS.get_environment("METRUM_STREAM_SHOTS")
	if dir.is_empty():
		return
	DirAccess.make_dir_recursive_absolute(dir)
	_viewport.get_texture().get_image().save_png(dir.path_join(name + ".png"))

# What a built terrain patch holds, measured at the end of a flight leg (`TERRAIN-41`): over up to
# PATCH_BYTES_SAMPLE built patches, the bytes of the textures only they name (read back) and of the
# images and arrays they keep in system memory, a patch's mean; textures shared between patches and
# one layer of the shared mask pages apart. Untimed: it reads textures back.
func _record_patch_bytes(label: String, sample: Dictionary) -> void:
	var seen := {}
	var counts := {"gpu_own": 0, "gpu_shared": 0, "cpu": 0}
	var keys: Array = _terrain.patches.keys().slice(0, PATCH_BYTES_SAMPLE)
	for key: Vector2i in keys:
		_held_bytes(_terrain.patches[key], seen, counts)
	var shared_uses := 0
	for rid: RID in seen:
		if int(seen[rid][0]) > 1:
			counts.gpu_shared += int(seen[rid][1])
			shared_uses += 1
		else:
			counts.gpu_own += int(seen[rid][1])
	var pages: Array = _terrain._mask_pages
	var mask_layer := 0
	if not pages.is_empty():
		var layer: Image = (pages[0] as Texture2DArray).get_layer_data(0)
		mask_layer = layer.get_data_size() if layer != null else 0
	var sampled := maxi(1, keys.size())
	sample["patch_bytes"] = {"sampled": keys.size(), "gpu_own": counts.gpu_own / sampled,
		"cpu": counts.cpu / sampled, "gpu_shared_total": counts.gpu_shared, "shared_textures": shared_uses,
		"mask_layer": mask_layer, "mask_pages": pages.size()}
	# The water patches the same way: their own textures and what they keep (dry ones share the
	# empty depth texture).
	var water_seen := {}
	var water_counts := {"gpu_own": 0, "gpu_shared": 0, "cpu": 0}
	var water_keys: Array = _water.patches.keys().slice(0, PATCH_BYTES_SAMPLE)
	for key: Vector2i in water_keys:
		_held_bytes(_water.patches[key], water_seen, water_counts)
	for rid: RID in water_seen:
		if int(water_seen[rid][0]) == 1:
			water_counts.gpu_own += int(water_seen[rid][1])
	var water_sampled := maxi(1, water_keys.size())
	sample["water_patch_bytes"] = {"sampled": water_keys.size(), "gpu_own": water_counts.gpu_own / water_sampled,
		"cpu": water_counts.cpu / water_sampled}
	sample["patches_built"] = {"terrain": _terrain.patches.size(), "water": _water.patches.size()}
	print("[STREAM] PATCH_BYTES %s %s water=%s built=%s" % [label, str(sample.patch_bytes),
		str(sample.water_patch_bytes), str(sample.patches_built)])

# Sums what `value` holds into `counts` (see _record_patch_bytes); textures by RID in `seen` as
# [uses, bytes].
static func _held_bytes(value: Variant, seen: Dictionary, counts: Dictionary) -> void:
	if value is Texture2D:
		var rid: RID = (value as Texture2D).get_rid()
		if seen.has(rid):
			seen[rid][0] = int(seen[rid][0]) + 1
		else:
			var image: Image = (value as Texture2D).get_image()
			seen[rid] = [1, image.get_data_size() if image != null else 0]
	elif value is Image:
		counts.cpu += (value as Image).get_data_size()
	elif value is Dictionary:
		for item: Variant in (value as Dictionary).values():
			_held_bytes(item, seen, counts)
	elif value is Array:
		for item: Variant in value:
			_held_bytes(item, seen, counts)
	elif value is PackedByteArray:
		counts.cpu += (value as PackedByteArray).size()
	elif value is PackedFloat32Array or value is PackedInt32Array:
		counts.cpu += value.size() * 4
	elif value is PackedVector2Array:
		counts.cpu += value.size() * 8
	elif value is PackedVector3Array:
		counts.cpu += value.size() * 12
	elif value is PackedColorArray:
		counts.cpu += value.size() * 16

# The process's memory: resident set (Linux), Godot's static allocations, and the renderer's
# texture, buffer and video memory, bytes (`TERRAIN-41`).
static func _memory() -> Dictionary:
	var rss := -1
	var status := FileAccess.open("/proc/self/status", FileAccess.READ)
	# /proc files report no length: read by lines.
	while status != null and not status.eof_reached():
		var line := status.get_line()
		if line.begins_with("VmRSS:"):
			rss = int(line.split(":")[1].strip_edges().split(" ")[0]) * 1024
			break
	return {"rss": rss, "static": OS.get_static_memory_usage(),
		"texture": int(Performance.get_monitor(Performance.RENDER_TEXTURE_MEM_USED)),
		"buffer": int(Performance.get_monitor(Performance.RENDER_BUFFER_MEM_USED)),
		"video": int(Performance.get_monitor(Performance.RENDER_VIDEO_MEM_USED))}

# Puts the camera at `view`, waits WARMUP_FRAMES frames and then until settled.
func _start_from(bench: Node, view: PackedFloat32Array) -> bool:
	_wait_begin()
	bench.camera.set_view_state(view)
	for _frame in WARMUP_FRAMES:
		await bench.get_tree().process_frame
		_wait_frame()
	return await _wait_settled(bench)

# A settle wait's frames are recorded too (`RENDER-43`): a first view reached by a pose move
# between two samples (the pond's mirror) compiles in them. One entry a wait in `settle_waits`.
func _wait_begin() -> void:
	if not _wait.is_empty():
		return
	var samples: Array = _report.samples
	var at := _pipelines()
	_wait = {"after": str(samples.back().label) if not samples.is_empty() else "load", "frames": 0,
		"worst_ms": 0.0, "worst_pipelines": [], "start": at, "last": at, "us": Time.get_ticks_usec()}

func _wait_frame() -> void:
	var now := Time.get_ticks_usec()
	var at := _pipelines()
	var frame_ms := float(now - int(_wait.us)) / 1000.0
	_wait.frames = int(_wait.frames) + 1
	if frame_ms > float(_wait.worst_ms):
		_wait.worst_ms = frame_ms
		_wait.worst_pipelines = _counter_delta(_wait.last, at)
	_wait.last = at
	_wait.us = now

func _wait_end() -> void:
	if _wait.is_empty():
		return
	if not _report.has("settle_waits"):
		_report["settle_waits"] = []
	_report.settle_waits.append({"after": _wait.after, "frames": _wait.frames, "worst_ms": _wait.worst_ms,
		"worst_pipelines": _wait.worst_pipelines, "pipelines": _counter_delta(_wait.start, _wait.last)})
	_wait = {}

static func _counter_delta(before: Array, after: Array) -> Array:
	var out := []
	for index in after.size():
		out.append(int(after[index]) - int(before[index]))
	return out

func _is_settled(bench: Node) -> bool:
	return (bench._is_idle() and _terrain.is_render_settled() and not _vegetation.has_pending_work()
		and (_roads == null or not _roads.is_pending()) and not (_wait_fill and _fill_left() > 0)
		and (not _preload or bench.input_manager.is_world_ready()))

# The base terrain budget's fill still to build, both layers (`TERRAIN-41` B1; 0 without the
# method).
func _fill_left() -> int:
	var budget_node: Node = _terrain.get("_stream") if _terrain != null else null
	if budget_node == null or not budget_node.has_method("get_terrain_budget"):
		return 0
	return int((budget_node.get_terrain_budget() as Dictionary).get("fill_left", 0))

func _wait_settled(bench: Node) -> bool:
	_wait_begin()
	var start_us := Time.get_ticks_usec()
	var stable := 0
	# METRUM_STREAM_SETTLE_CAP_S (`PL1`): a city whose simulation edits it every few frames (100k
	# at 32x) may never settle; past the cap the phase starts unsettled and the report counts it.
	var cap_s := float(OS.get_environment("METRUM_STREAM_SETTLE_CAP_S"))
	while stable < SETTLED_FRAMES:
		if cap_s > 0.0 and bench._elapsed_ms(start_us) > cap_s * 1000.0:
			_report["unsettled_starts"] = int(_report.get("unsettled_starts", 0)) + 1
			_wait_end()
			return true
		if bench._elapsed_ms(start_us) > bench.settle_timeout_sec * 1000.0:
			_wait_end()
			return false
		await bench.get_tree().process_frame
		_wait_frame()
		stable = stable + 1 if _is_settled(bench) else 0
	_wait_end()
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
	_record_vegetation_digest()
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
		# The built set's keys too (`TERRAIN-59`: a preload's set must not depend on timing).
		var built := PackedStringArray()
		for key: Vector2i in renderer.patches:
			built.append("%d,%d" % [key.x, key.y])
		built.sort()
		entry[layer] = {"resident": lines.size(), "built": renderer.patches.size(),
			"digest": _sha("\n".join(lines).to_utf8_buffer()), "lines": lines,
			"built_digest": _sha("\n".join(built).to_utf8_buffer())}
	entry["land_cover"] = _land_cover_digest()
	entry["water_meshes"] = _water_mesh_check()
	_report.patch_digests.append(entry)

# Every wet resident water patch draws a mesh of its own step (`TERRAIN-41` L2): `wet` counted,
# `mismatches` those that do not, with their first keys. The node's held mesh where the renderer
# records it (its step, cover generation and depth signature), else (a control) its mesh stats'
# step with a mesh other than the renderer's shared empty one (a baked mesh may emit no triangle).
func _water_mesh_check() -> Dictionary:
	var wet := 0
	var keys := PackedStringArray()
	for key: Vector2i in _water.resident_patch_lookup:
		var patch: Dictionary = _water.patches.get(key, {})
		if int(patch.get("depth_nonzero_count", 0)) <= 0:
			continue
		wet += 1
		var step := maxi(1, int(patch.get("lod_step", 1)))
		var drawn := false
		if patch.has("held_mesh"):
			var held: PackedInt64Array = patch["held_mesh"]
			drawn = (held.size() == 3 and held[0] == step
				and held[1] == int(patch.get("cover_generation", 0))
				and held[2] == int(patch.get("depth_signature", 0)))
		else:
			var stats: Dictionary = patch.get("mesh_stats", {})
			var node: MeshInstance3D = patch.get("node", null)
			drawn = (int(stats.get("lod_step", 0)) == step and node != null
				and node.mesh != _water.get("empty_water_mesh"))
		if not drawn:
			keys.append("%d,%d" % [key.x, key.y])
	keys.sort()
	# Hidden patches too (`TERRAIN-63`): a hidden node holds no mesh of an old cover or depth (its
	# mesh is dropped in the frame it went stale), counted `hidden_stale` with its first keys;
	# `hidden_wet` the hidden wet patches, `hidden_meshes` those holding a mesh, `kept` those whose
	# mesh came by the kept cause.
	var hidden_wet := 0
	var hidden_meshes := 0
	var kept := 0
	var stale := PackedStringArray()
	for key: Vector2i in _water.patches:
		if _water.resident_patch_lookup.has(key):
			continue
		var patch: Dictionary = _water.patches[key]
		var held: PackedInt64Array = patch.get("held_mesh", PackedInt64Array())
		if held.size() == 3:
			hidden_meshes += 1
			if held[1] != int(patch.get("cover_generation", 0)) or held[2] != int(patch.get("depth_signature", 0)):
				stale.append("%d,%d" % [key.x, key.y])
		hidden_wet += int(int(patch.get("depth_nonzero_count", 0)) > 0)
		kept += int(bool(patch.get("kept_mesh", false)))
	stale.sort()
	return {"wet": wet, "mismatches": keys.size(), "keys": keys.slice(0, 8), "hidden_wet": hidden_wet,
		"hidden_meshes": hidden_meshes, "kept": kept, "hidden_stale": stale.size(),
		"hidden_stale_keys": stale.slice(0, 8)}

# The built terrain patches' land covers at a settle point (`RENDER-40`): per patch its key, the
# bytes' digest and its revisions, from the script's own book, so a control's main-thread query
# and the land cover jobs compare by digest; `stale` counts those not current by the published
# revisions. Read from the book only: asking the land cover jobs here would take the renderer's
# own requests and hand-overs, which are one per key.
func _land_cover_digest() -> Dictionary:
	var simulation: Node = _terrain.simulation_node
	var lines := PackedStringArray()
	var stale := 0
	for key: Vector2i in _terrain.patches:
		var cover: Dictionary = (_terrain.patches[key] as Dictionary).get("land_cover", {})
		var image: Image = cover.get("image", null)
		var bytes := image.get_data() if image != null else PackedByteArray()
		var revisions: PackedInt64Array = cover.get("generations", PackedInt64Array())
		lines.append("%d,%d,%s,%s" % [key.x, key.y, _sha(bytes), str(revisions)])
		if not simulation.is_vegetation_land_cover_current(key, revisions):
			stale += 1
	lines.sort()
	return {"patches": lines.size(), "stale": stale,
		"digest": _sha("\n".join(lines).to_utf8_buffer()), "lines": lines}

# The road chunks by key: the chunk renderer's (`RENDER-52`), or a control's road tool's index.
func _road_chunks() -> Variant:
	var renderer: Variant = _road_tool.get("chunks")
	return renderer.chunks if renderer != null else _road_tool.get("_road_chunk_instances")

# The road chunks drawn at a settle point (`RENDER-32-P`): per chunk its key, place, surface
# layout and every surface as the RenderingServer holds it (`RENDER-52` part 2: a hydrated
# chunk's surfaces are only there), from the chunks' own index on either side.
func _record_road_digest() -> void:
	var chunks: Variant = _road_chunks()
	if chunks == null:
		return
	var lines := PackedStringArray()
	for key: Vector2i in chunks:
		var node: MeshInstance3D = chunks[key]
		if not is_instance_valid(node) or not node.visible or not node.is_inside_tree():
			continue
		var mesh: ArrayMesh = node.mesh
		var surfaces := PackedStringArray()
		var rid := mesh.get_rid()
		for surface in RenderingServer.mesh_get_surface_count(rid):
			surfaces.append(_sha(var_to_bytes(RenderingServer.mesh_get_surface(rid, surface))))
		lines.append("%d,%d|%s|%s|%s|%d" % [key.x, key.y, node.position, mesh.get_meta("road_layer_surfaces", []),
			",".join(surfaces), node.get_child_count()])
	lines.sort()
	_report.road_digests.append({"point": _settle_points,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else "",
		"chunks": lines.size(), "digest": _sha("\n".join(lines).to_utf8_buffer())})

# The parked cars drawn at a settle point (`RENDER-32-P`): per chunk, vehicle bucket and cell
# (`RENDER-69`) its rows, from the stream backend's slots (`drawn_rows`) or the control's chunk
# nodes.
func _record_parked_digest() -> void:
	if _parked == null:
		return
	var lines := PackedStringArray()
	if _parked.has_method("drawn_rows"):
		for batch: Dictionary in _parked.drawn_rows():
			lines.append("%d,%d|%d|%d|%s" % [batch.chunk.x, batch.chunk.y, batch.bucket, batch.cell,
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

# The vegetation patches drawn at a settle point (AUDIT-stream-2 finding 7): per visible installed
# patch its key, its record's bands, its detail and shadow state, and per batch its identity, count,
# main instance's material and shadow, and the digest of its first `count` rows read back. Hidden
# patches (cached ones) are left out: which are kept hidden depends on the path, not the view.
func _record_vegetation_digest() -> void:
	var stream: Node = _vegetation.get("stream")
	if stream == null:
		return
	var lines := PackedStringArray()
	var records: Dictionary = stream.get_vegetation_records()
	for key: Vector3i in records:
		var patch: Dictionary = stream.get_vegetation_patch(key)
		if patch.is_empty() or not bool(patch.visible):
			continue
		var record: Dictionary = records[key]
		var batches := PackedStringArray()
		for batch: Dictionary in patch.batches:
			var count := int(batch.count)
			# The slot's row layout: 12 transform floats, then colour and custom data when used.
			var stride := 12 + (4 if bool(batch.colors) else 0) + (4 if bool(batch.custom) else 0)
			var rows := RenderingServer.multimesh_get_buffer(batch.multimesh).slice(0, count * stride)
			var main: Dictionary = batch.get("main", {})
			batches.append("%d,%d,%d,%d,%d,%d,%d|%s" % [int(batch.species), int(batch.lod), int(batch.variant),
				int(batch.mesh), count, int(main.get("material", -2)), int(main.get("shadow", -1)),
				_sha(rows.to_byte_array())])
		lines.append("%d,%d,%d|%d%d%d|%d,%d,%d|%s" % [key.x, key.y, key.z, int(bool(record.understory)),
			int(bool(record.near_band)), int(record.shadow_caster), int(patch.detail), int(bool(patch.branched)),
			int(bool(patch.has_caster)), ";".join(batches)])
	lines.sort()
	_report.vegetation_digests.append({"point": _settle_points,
		"sample": (_report.samples.back() as Dictionary).get("label", "") if not _report.samples.is_empty() else "",
		"patches": lines.size(), "digest": _sha("\n".join(lines).to_utf8_buffer()), "lines": lines})

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
	var renderer: Variant = _road_tool.get("chunks")
	var lights: Variant = renderer.street_lights if renderer != null else _road_tool.get("_street_lights")
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
	# The vegetation's install counters count from here (`VEG-35`; absent on an older library).
	var stream: Node = _vegetation.stream if _vegetation != null else null
	if stream != null and stream.has_method("take_vegetation_install_waits"):
		stream.take_vegetation_install_waits()
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
	var ready_budget := {}
	var half_second_shot := false
	while rows.size() < max_frames and stable < SETTLED_FRAMES:
		await bench.get_tree().process_frame
		rows.append(_row(rows.size()))
		# METRUM_STREAM_SHOTS (`TERRAIN-41` D2): the first frame drawn after the move and the
		# first at 0.5 s; the settled frame below.
		if rows.size() == 1:
			_save_shot(label + "_first")
		elif not half_second_shot and bench._elapsed_ms(start_us) >= 500.0:
			half_second_shot = true
			_save_shot(label + "_0500ms")
		if ready_ms < 0.0 and bench.input_manager.is_world_ready():
			ready_ms = bench._elapsed_ms(start_us)
			# The budget at the hand-over (`TERRAIN-59`: what a preload built).
			var budget_node := bench.get_parent().get_node_or_null("StreamNode")
			if budget_node != null:
				ready_budget = budget_node.get_terrain_budget()
		if _is_settled(bench):
			if stable == 0:
				settle_ms = bench._elapsed_ms(start_us)
				settle_frames = rows.size()
			stable += 1
		else:
			stable = 0
	var sample := _finish(label, 0.0, rows)
	sample.merge({"settled": stable >= SETTLED_FRAMES, "settle_ms": settle_ms, "settle_frames": settle_frames,
		"ready_ms": ready_ms, "ready_budget": ready_budget})
	if stable >= SETTLED_FRAMES:
		_record_digest(label)
		_save_shot(label + "_settled")
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
		"pipelines": _pipelines(),
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
		# The probe's last drawn frame (`TERRAIN-41`); a library without `backlog` records none.
		if _probe.has_method("backlog"):
			# `terrain_map`: every patch of the map (`TERRAIN-41` D2); its [3] is the in-view patches
			# not drawn as patches, [4] those nothing drew over (sky).
			row["backlog"] = {"terrain": Array(_probe.backlog("terrain")), "water": Array(_probe.backlog("water")),
				"terrain_map": Array(_probe.backlog("terrain_map"))}
	if _proxy_checking:
		row["proxies"] = _proxy_check
	if _bed_checking:
		row["bed"] = _bed_check
	# A reflection from before `RENDER-76` has no such property (null).
	if _reflection != null and _reflection.get("_rect_checking") == true:
		row["rect_check"] = [_reflection.get("_rect_checked"), _reflection.get("_rect_differences"),
			_reflection.get("_rect_walks_total")]
	_previous_streams = streams
	_last_us = now_us
	return row

# The drawn probe (`RENDER-32-N`), once the frame has drawn: every layer's installed set as its
# owner reports it, against the camera that frame drew with. O(the installed sets).
func _on_probe_post_draw() -> void:
	_probe.begin_frame(_camera.get_frustum())
	var low := _probe_heights.x - PROBE_GROUND_M
	var ground := _probe_heights.y + PROBE_GROUND_M
	for layer: String in ["terrain", "terrain_map", "water", "graded", "graded_tiles"]:
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

# The water reflection's terrain proxies against the terrain patches the frame just drew
# (`RENDER-75`): [a mirror drew, patches drawn, of them with a proxy, with a proxy holding the
# patch's mesh, transform, material copy, culling box and margin as drawn, proxies in all, an
# order-free hash of the drawn keys]. A proxy book in step has drawn = held = matching = proxies.
# Reads the reflection's private book, keyed by patch key or (before `RENDER-75`) by node id.
func _on_proxy_check_post_draw() -> void:
	var proxies: Dictionary = _reflection.get("_proxies")
	var copies: Dictionary = _reflection.get("_grid_materials")
	var mirrors: Array = _reflection.get("_mirrors")
	var drawn := 0
	var held := 0
	var matching := 0
	var keys_hash := 0
	for key: Vector2i in _terrain.patches:
		var node := (_terrain.patches[key] as Dictionary).get("node") as MeshInstance3D
		if node == null or not node.is_visible_in_tree() or node.mesh == null:
			continue
		drawn += 1
		keys_hash = (keys_hash + hash(key)) & 0x7fffffff
		var proxy: Array = proxies.get(key, proxies.get(node.get_instance_id(), []))
		if proxy.is_empty():
			continue
		held += 1
		if (proxy.size() >= 6 and proxy[1] == node.mesh.get_rid()
				and proxy[2] == node.global_transform
				and proxy[3] == copies.get(node.material_override)
				and proxy[4] == node.custom_aabb and proxy[5] == node.extra_cull_margin):
			matching += 1
	var active := mirrors.any(func(mirror: Array) -> bool: return mirror[3])
	_proxy_check = [active, drawn, held, matching, proxies.size(), keys_hash]

# The lake-bed check (`TERRAIN-58`), once the frame has drawn, from the scripts' own state: a
# terrain patch's node draws when it is visible in the tree and its PlaneMesh is not collapsed (a
# held or blank patch's is); its water does not draw when it is wet and has no node, is hidden, or
# holds no mesh of its cover and depth. And the hand-overs from cell to patch, by the whole-map
# probe's flags. O(resident patches + the map's patches).
func _on_bed_check_post_draw() -> void:
	var planes: Array[Plane] = _camera.get_frustum()
	var water_patches: Dictionary = _water.get("patches")
	var water_shown: Dictionary = _water.get("resident_patch_lookup")
	var bed := [0, 0, 0]
	for key: Vector2i in _terrain.resident_patch_lookup:
		var land: Dictionary = _terrain.patches.get(key, {})
		var node := land.get("node") as MeshInstance3D
		if node == null or not node.is_visible_in_tree() or node.mesh == null:
			continue
		if node.visibility_range_end > 0.0 and node.visibility_range_end < 0.01:
			continue
		var size := Vector2(float(land.get("world_size_x", 512.0)), float(land.get("world_size_z", 512.0)))
		var at := node.global_position
		var seen := _box_in_view(planes, Vector3(at.x - size.x * 0.5, -500.0, at.z - size.y * 0.5),
			Vector3(at.x + size.x * 0.5, 2500.0, at.z + size.y * 0.5))
		bed[2] += 1 if seen else 0
		var water: Dictionary = water_patches.get(key, {})
		var wet := not water.is_empty() and int(water.get("depth_nonzero_count", 0)) > 0
		var unknown := water.is_empty()
		var lacking: bool = wet and (not water_shown.has(key) or _water._mesh_need(water) == 1)
		if unknown:
			# No water node yet: wet if the simulation has water over the patch's centre.
			lacking = not is_nan(_simulation.get_camera_water_surface_at(Vector2(at.x, at.z)))
		if lacking:
			bed[1] += 1
			bed[0] += 1 if seen else 0
	_bed_check = bed
	_bed_max = maxi(_bed_max, bed[0])
	_bed_frame += 1
	# A patch resident in the terrain's scripts but drawn as a cell since which frame; its span
	# closes when it is drawn as a patch (a patch that leaves first closes none).
	var map: PackedFloat32Array = _stream.probe_stream_layer("terrain_map")
	for index in range(0, map.size(), 8):
		var key := Vector2i(int(map[index]), int(map[index + 1]))
		var drawn := (int(map[index + 3]) & 1) != 0
		if not _terrain.resident_patch_lookup.has(key):
			_cell_since.erase(key)
		elif not drawn:
			if not _cell_since.has(key):
				_cell_since[key] = _bed_frame
		elif _cell_since.has(key):
			_cell_spans.append(_bed_frame - int(_cell_since[key]))
			_cell_since.erase(key)

static func _box_in_view(planes: Array[Plane], low: Vector3, high: Vector3) -> bool:
	for plane in planes:
		var corner := Vector3(low.x if plane.normal.x > 0.0 else high.x,
			low.y if plane.normal.y > 0.0 else high.y, low.z if plane.normal.z > 0.0 else high.z)
		if plane.is_point_over(corner):
			return false
	return true

# At a sample's end (`RENDER-75`): per material parameter, the drawn patches whose reflection copy
# holds another value than the patch's material, and the patches with any such parameter.
func _proxy_stale() -> Dictionary:
	var copies: Dictionary = _reflection.get("_grid_materials")
	var by_parameter := {}
	var patches := 0
	for key: Vector2i in _terrain.patches:
		var node := (_terrain.patches[key] as Dictionary).get("node") as MeshInstance3D
		if node == null or not node.is_visible_in_tree() or node.mesh == null:
			continue
		var source := node.material_override as ShaderMaterial
		var copy := copies.get(source) as ShaderMaterial
		if source == null or copy == null:
			continue
		var stale := false
		for entry: Dictionary in source.shader.get_shader_uniform_list():
			var held: Variant = copy.get_shader_parameter(entry["name"])
			var value: Variant = source.get_shader_parameter(entry["name"])
			if typeof(held) != typeof(value) or held != value:
				by_parameter[entry["name"]] = int(by_parameter.get(entry["name"], 0)) + 1
				stale = true
		patches += 1 if stale else 0
	return {"patches": patches, "parameters": by_parameter}

func _on_pre_draw() -> void:
	_pre_draw_us = Time.get_ticks_usec()

func _on_post_draw() -> void:
	_post_draw_us = Time.get_ticks_usec()

# Godot's pipeline compilations since the start, by source: canvas, mesh, surface (queued in the
# background at a surface's creation), draw (a draw that waited for its pipeline) and
# specialization (queued in the background by a draw that took the ubershader) (`RENDER-43`).
# `LaunchState.benchmark_menu_ms`, or none without the autoload or a build without the menu's.
static func _menu_ms(bench: Node) -> Array:
	var state := bench.get_node_or_null("/root/LaunchState")
	var times = state.get("benchmark_menu_ms") if state != null else null
	return times if times is Array else []

# `LaunchState.benchmark_menu`: the menu's longest frame and its prewarm's report, or empty.
static func _menu_report(bench: Node) -> Dictionary:
	var state := bench.get_node_or_null("/root/LaunchState")
	var report = state.get("benchmark_menu") if state != null else null
	return report if report is Dictionary else {}

static func _pipelines() -> Array:
	return [RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_PIPELINE_COMPILATIONS_CANVAS),
		RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_PIPELINE_COMPILATIONS_MESH),
		RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_PIPELINE_COMPILATIONS_SURFACE),
		RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_PIPELINE_COMPILATIONS_DRAW),
		RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_PIPELINE_COMPILATIONS_SPECIALIZATION)]

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
		"frame_ms": stats(times), "rows": rows, "memory": _memory()}
	# The base terrain's budget at the sample's end (`TERRAIN-41` B1): built patches of both layers,
	# counted bytes, places, the keep ring and what the fill has left.
	var budget_node: Node = _terrain.get("_stream") if _terrain != null else null
	if budget_node != null and budget_node.has_method("get_terrain_budget"):
		sample["terrain_budget"] = budget_node.get_terrain_budget()
	if _proxy_checking:
		sample["proxy_stale"] = _proxy_stale()
	if _bed_checking:
		var spans := _cell_spans.duplicate()
		spans.sort()
		var count := spans.size()
		sample["cell_spans"] = [count, spans[count / 2] if count > 0 else 0,
			spans[count * 9 / 10] if count > 0 else 0, spans[count - 1] if count > 0 else 0]
		sample["bed_max"] = _bed_max
		_cell_spans.clear()
		_bed_max = 0
	# The patch units' measured times by layer and kind since the last sample (`TERRAIN-41` L1;
	# absent on an older library).
	var stream: Node = _vegetation.stream if _vegetation != null else null
	if stream != null and stream.has_method("take_patch_unit_times"):
		sample["patch_units"] = stream.take_patch_unit_times()
	# The backdrop's product and uploads at the sample's end (`TERRAIN-41` D1; absent on an older
	# library).
	if stream != null and stream.has_method("get_backdrop"):
		sample["backdrop"] = stream.get_backdrop()
	# The vegetation's installs since the sample's start (`VEG-35`; absent on an older library).
	if stream != null and stream.has_method("take_vegetation_install_waits"):
		sample["vegetation_installs"] = stream.take_vegetation_install_waits()
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
	terrain.merge({"resident": _terrain.resident_patch_lookup.size(), "built": _terrain.patches.size(),
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
		# `cell_sets_made`, `cell_set_made_us`: sets made past the prewarm and their objects' time
		# (`TERRAIN-41` round G0; 0 on a control before it).
		for key: String in ["uploads", "same_level_uploads", "unready_in_view", "pool_exhausted",
				"cell_sets_made", "cell_set_made_us", "cell_set_made_us_max", "cell_sets_prewarmed", "cell_prewarm_ms"]:
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
		"roads": {"chunks": (_road_chunks() as Dictionary).size()},
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
		"prepared_replaced": int(status.get("prepared_replaced", -1)),
		# The installs' running counts (`VEG-35`; -1 on a build without them): waits summed and
		# past the bar, passes stalled with a hole product waiting.
		"install_wait_frames_sum": int(status.get("install_wait_frames_sum", -1)),
		"install_waited_over_bar": int(status.get("install_waited_over_bar", -1)),
		"install_stalled_passes": int(status.get("install_stalled_passes", -1)),
		# The residency's jobs and frees (`VEG-36`; -1 on a build without them): requests and
		# prepares started, patches freed.
		"requests_total": int(status.get("requests_total", -1)),
		"prepares_total": int(status.get("prepares_total", -1)),
		"frees_total": int(status.get("frees_total", -1))}

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

# The trip's pose: the start or the end of its jump along x through the world's centre, looking
# along it at the game's default pitch, at orbit `distance`, the pivot on the ground.
func _trip_pose(world: Vector2, end: bool, distance: float) -> PackedFloat32Array:
	var half := minf(TRIP_M, world.x * 0.3) * 0.5
	var x := half if end else -half
	return PackedFloat32Array([x, _simulation.get_height_at(Vector2(x, 0.0)), 0.0, -PI * 0.5,
		FAR_PITCH_RAD, distance, 0.0])

# The trip phase (see the header); false (the run failed) when its start or its hold did not settle.
func _trip(bench: Node, world: Vector2) -> bool:
	if not await _start_from(bench, _trip_pose(world, false, TRIP_LOW_M)):
		bench._fail("the trip's start did not settle")
		return false
	var stream: Node = _vegetation.stream
	var end := _trip_pose(world, true, TRIP_HIGH_M)
	var trip := {"low_m": TRIP_LOW_M, "high_m": TRIP_HIGH_M, "zoom_s": TRIP_ZOOM_S, "near_m": TRIP_NEAR_M,
		"from_x": _trip_pose(world, false, TRIP_LOW_M)[0], "to_x": end[0], "keys": []}
	_report["trip"] = trip
	var probe := {"start_us": 0, "pivot": Vector2(end[0], end[2]), "keys": {},
		"near": stream.has_method("count_vegetation_drawn_near"), "near_ms": -1.0}
	var rows := _begin("trip_in")
	probe.start_us = _last_us
	var fraction := 0.0
	while fraction < 1.0:
		fraction = minf(1.0, float(Time.get_ticks_usec() - int(probe.start_us)) / (TRIP_ZOOM_S * 1000000.0))
		var view := end.duplicate()
		view[5] = TRIP_HIGH_M * pow(TRIP_LOW_M / TRIP_HIGH_M, fraction)
		bench.camera.set_view_state(view)
		await bench.get_tree().process_frame
		var row := _trip_row(rows.size(), probe, stream)
		row["fraction"] = fraction
		rows.append(row)
	_report.samples.append(_finish("trip_in", TRIP_ZOOM_S, rows))
	rows = _begin("trip_hold")
	var stable := 0
	var settle_ms := -1.0
	var veg_done_ms := -1.0
	var cap_ms := (TRIP_ZOOM_S + TRIP_HOLD_MAX_S) * 1000.0
	while stable < SETTLED_FRAMES:
		await bench.get_tree().process_frame
		var row := _trip_row(rows.size(), probe, stream)
		rows.append(row)
		if _vegetation.has_pending_work():
			veg_done_ms = -1.0
		elif veg_done_ms < 0.0:
			veg_done_ms = float(row.trip_ms)
		if _is_settled(bench):
			if stable == 0:
				settle_ms = float(row.trip_ms)
			stable += 1
		else:
			stable = 0
		if float(row.trip_ms) > cap_ms:
			break
	var sample := _finish("trip_hold", 0.0, rows)
	sample.merge({"settled": stable >= SETTLED_FRAMES, "settle_ms": settle_ms, "veg_done_ms": veg_done_ms,
		"near_ms": probe.near_ms})
	_report.samples.append(sample)
	for key: Vector3i in probe.keys:
		trip.keys.append([key.x, key.y, key.z] + probe.keys[key])
	if not bool(sample.settled):
		bench._fail("the trip did not settle within %.0f s of its zoom" % TRIP_HOLD_MAX_S, {"settle_ms": settle_ms})
		return false
	_record_digest("trip_hold")
	return true

# One row of the trip: the row, its ms from the jump, the near ring and, with the perf channel on,
# every wanted vegetation patch's times (`_trip_keys`).
func _trip_row(index: int, probe: Dictionary, stream: Node) -> Dictionary:
	var row := _row(index)
	var now := float(Time.get_ticks_usec() - int(probe.start_us)) / 1000.0
	row["trip_ms"] = now
	if probe.near:
		var near: Vector2i = stream.count_vegetation_drawn_near(probe.pivot, TRIP_NEAR_M)
		row["near"] = [near.x, near.y]
		if near.x == 0 or near.x != near.y:
			probe.near_ms = -1.0
		elif float(probe.near_ms) < 0.0:
			probe.near_ms = now
	if _scripts:
		_trip_keys(probe, stream, now)
	return row

# Per wanted vegetation patch (perf channel on, `O(wanted)` a frame): [first wanted, product
# ready, own drawn, left the wanted set (ms from the jump, -1 unset), its centre's distance to the
# pivot (m)]; a patch that leaves and comes back starts a new entry.
func _trip_keys(probe: Dictionary, stream: Node, now: float) -> void:
	var keys: Dictionary = probe.keys
	var seen := {}
	for layer: String in ["vegetation_coarse", "vegetation_fine"]:
		var data: PackedFloat32Array = stream.probe_stream_layer(layer)
		var at := 0
		while at + 8 <= data.size():
			var key := Vector3i(int(data[at]), int(data[at + 1]), int(data[at + 2]))
			var flags := int(data[at + 3])
			seen[key] = true
			if not keys.has(key) or float(keys[key][3]) >= 0.0:
				var centre := Vector2((data[at + 4] + data[at + 6]) * 0.5, (data[at + 5] + data[at + 7]) * 0.5)
				keys[key] = [now, -1.0, -1.0, -1.0, centre.distance_to(probe.pivot)]
			var entry: Array = keys[key]
			if flags & 4 and float(entry[1]) < 0.0:
				entry[1] = now
			if flags & 1 and float(entry[2]) < 0.0:
				entry[2] = now
			at += 8
	for key: Vector3i in keys:
		if not seen.has(key) and float(keys[key][3]) < 0.0:
			keys[key][3] = now
