# SPDX-License-Identifier: GPL-2.0-only

## Preview-only matrix using the existing gameplay harness and production RoadTool.
## Setup/commits are excluded; each mode starts from the same deterministic world.
## The populated cases (`docs/roads.md` question 61 (a)) load the saved populated city
## (METRUM_GAMEPLAY_BENCHMARK_SAVE_PATH) instead, run its simulation at
## METRUM_GAMEPLAY_PREVIEW_SPEED (default 1x) and drag sloped_t's and sloped_branch's pointer
## paths at the commit breakdown's first edit site; they run only when selected by name.
extends RefCounted

const Metrics := preload("res://scripts/benchmarks/road_benchmark_metrics.gd")
const CASES := ["flat_t", "sloped_t", "flat_multi", "sloped_multi", "dense",
	"flat_branch", "sloped_branch", "flat_cross", "sloped_cross"]
const POPULATED_CASES := ["populated_t", "populated_branch"]
# D3: the one preview mode, road and terrain together (`ROAD-68-E`).
const MODE := 1
# Moving-phase pointer speed: 0.5 m per 60 Hz input.
const MOVE_SPEED_MPS := 30.0
var _report: Dictionary

func run(bench: Node) -> void:
	bench.input_manager.set_process(false)
	bench.input_manager.set_process_input(false)
	bench.input_manager.set_process_unhandled_input(false)
	# Unset keeps the Method's 60 FPS cap; 0 removes the cap (vsync alone paces the frames).
	Engine.max_fps = bench._environment_int("METRUM_GAMEPLAY_BENCHMARK_MAX_FPS", 60, 0)
	if bench.mode != "headless":
		DisplayServer.window_set_title("Metrum Rise - road preview benchmark")
	_report = {
		"schema_version": 2, "benchmark": "road_preview_latency", "success": false,
		"runtime": bench._runtime_metadata(), "cases": [], "phases": [], "mode": bench.mode, "modes": [MODE],
		"expected_cases": (CASES + POPULATED_CASES).filter(func(name): return _selected(bench, name)),
		"samples": bench.repetitions, "warmups": bench.warmup_repetitions,
		"timing_contract": "scripted world-space input to frame_post_draw; headless uses process_frame and is not rendering; no GPU upload or monitor presentation claim",
		"helper_sha256": FileAccess.get_sha256(get_script().resource_path),
	}
	_report.runtime["display_server"] = DisplayServer.get_name()
	_report.runtime["world_contract"] = "512m_cell8m_chunk128m_flat_or_2.5_percent_slope"
	_report.runtime["world_sha256"] = "road30_synthetic_v1"
	_report.runtime["fixture_isolation"] = "reset_each_case_and_mode"
	_report.runtime["trace_sha256"] = FileAccess.get_sha256("res://scripts/benchmarks/road_preview_metrics.gd")
	for name in CASES + POPULATED_CASES:
		if not _selected(bench, name):
			continue
		var result: Dictionary = await _case(bench, name, MODE)
		_report.cases.append(result)
		bench._metrics = _report
		if not bench._write_metrics() or not result.get("ok", false):
			bench._fail("preview matrix failed", result)
			return
		print("[PREVIEW_BENCH] %s mode=%d stationary=%d moving=%d" % [name, MODE, result.stationary.size(), result.moving.size()])
	_report.success = not _report.cases.is_empty()
	bench._stop_scripted_tool()
	bench._metrics = _report
	var written: bool = bench._write_metrics()
	bench.get_tree().quit(0 if written and _report.success else 1)

# The populated cases run only when named; every other case runs unless the selection omits it.
func _selected(bench: Node, name: String) -> bool:
	if bench.selected_case_ids.is_empty():
		return not name in POPULATED_CASES
	return name in bench.selected_case_ids

func _case(bench: Node, name: String, mode: int) -> Dictionary:
	bench._stop_scripted_tool()
	var setup_us := Time.get_ticks_usec()
	var sim: Node = bench.simulation_node
	var populated: bool = name in POPULATED_CASES
	var scenario: Dictionary
	if populated:
		var loaded: Dictionary = await _load_populated(bench)
		if not loaded.ok:
			return loaded
		scenario = _populated_scenario(bench, name)
	else:
		if not sim.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0):
			return {"ok": false, "error": "world creation"}
		bench.input_manager._refresh_after_world_load()
		bench.input_manager.set_simulation_speed(0.0)
		if name.begins_with("sloped"):
			sim.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128, 0), 0.0, Vector2(128, 0), 6.4, 1.0)
		scenario = _scenario(name)
	bench.camera.set("orthogonal", true)
	bench.camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	bench.camera.focus_on(bench._surface_point(scenario.get("centre", Vector2.ZERO)), 150.0)
	for road in scenario.roads:
		var generation: int = sim.get_network_render_generation()
		sim.add_road_with_snap(PackedVector3Array([bench._surface_point(road[0]), bench._surface_point(road[1])]), 1, 1, true)
		var deadline := Time.get_ticks_msec() + 30000
		while sim.get_network_render_generation() == generation and Time.get_ticks_msec() < deadline:
			await bench.get_tree().process_frame
		if sim.get_network_render_generation() == generation:
			return {"ok": false, "error": "fixture commit timeout"}
		var committed: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
		if not committed.get("ok", false):
			return {"ok": false, "error": "fixture commit settlement", "wait": committed}
	var idle: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
	if not idle.get("ok", false):
		return {"ok": false, "error": "fixture settlement", "wait": idle}
	var tool: Node = bench.road_tool
	var speed := 0.0
	if populated:
		var speed_env := OS.get_environment("METRUM_GAMEPLAY_PREVIEW_SPEED")
		speed = float(speed_env) if speed_env.is_valid_float() else 1.0
		bench.input_manager.set_simulation_speed(speed)
		# The sim thread applies the speed with its next commands.
		for frame in range(120):
			if float(sim.get_road_benchmark_state().simulation_speed) == speed:
				break
			await bench.get_tree().process_frame
	var state: Dictionary = sim.get_road_benchmark_state()
	var result := {"case_id": name, "mode": mode, "ok": false,
		"setup_ms": float(Time.get_ticks_usec() - setup_us) / 1000.0,
		"stationary": [], "moving": [], "state_before": state, "simulation_speed": speed,
		"anchor": [scenario.get("centre", Vector2.ZERO).x, scenario.get("centre", Vector2.ZERO).y]}
	if populated and float(state.simulation_speed) != speed:
		return {"ok": false, "error": "simulation speed not applied", "state": state}
	# The populated cases sample the simulation's published state once a frame (moving phase).
	var sim_frames: Array = []
	var segment := {"draw_mode": 0, "fwd_lanes": 1, "bkw_lanes": 1}
	bench._begin_scripted_road(Vector2.ZERO, segment, bench._surface_point(scenario.start), bench._surface_point(_pose(scenario, 0.0)))
	# Warmup uses the same motion sequence but is excluded from the capture collector.
	tool.begin_preview_measurement()
	for index in range(bench.warmup_repetitions):
		_set_pointer(bench, _pose(scenario, float(index % 8) / 8.0))
		if not await _wait_exact(bench):
			return {"ok": false, "error": "warmup preview timeout"}
	var trace: RefCounted = tool.begin_preview_measurement()
	# Force a new result after resetting the collector, including when warmup was disabled.
	tool._clear_preview_cache()
	for index in range(bench.repetitions):
		_set_pointer(bench, _pose(scenario, float((index + 1) % 8) / 8.0))
		if not await _wait_exact(bench):
			return {"ok": false, "error": "stationary preview timeout", "case_id": name, "mode": mode}
		result.stationary.append(trace.last_rendered.duplicate(true))
	var stationary_count: int = trace.rows.size()
	var moving_service_start: float = trace.worker_service_ms
	var move_start := Time.get_ticks_usec()
	# Absolute deadlines avoid relative timers drifting to alternate-frame input at 60 FPS.
	# Late frames coalesce overdue samples, matching the tool's latest-input contract.
	var input_count: int = maxi(120, bench.repetitions * 4)
	var input_lateness_ms: Array = []
	for index in range(input_count):
		var due_us := move_start + int(float(index) * 1000000.0 / 60.0)
		while Time.get_ticks_usec() < due_us:
			await bench.get_tree().process_frame
			if populated:
				_sample_simulation(sim, sim_frames)
		var emitted_us := Time.get_ticks_usec()
		input_lateness_ms.append(float(emitted_us - due_us) / 1000.0)
		trace.schedule(due_us, emitted_us)
		_set_pointer(bench, _moving_pose(scenario, index))
	while Time.get_ticks_usec() < move_start + int(float(input_count) * 1000000.0 / 60.0):
		await bench.get_tree().process_frame
		if populated:
			_sample_simulation(sim, sim_frames)
	result["simulation_frames"] = sim_frames
	result["scheduled_input_count"] = input_count
	result["scheduled_input_hz"] = 60
	result["input_lateness_ms"] = Metrics.distribution(input_lateness_ms)
	var stop_us := Time.get_ticks_usec()
	trace.finish_scheduled_inputs(stop_us)
	var observed_service_ms: float = trace.worker_service_ms - moving_service_start
	_set_pointer(bench, _pose(scenario, 9.0 / 16.0))
	if not await _wait_exact(bench):
		return {"ok": false, "error": "final pose timeout"}
	for row in trace.rows.slice(stationary_count):
		if int(row.frame_us) <= stop_us:
			result.moving.append(row)
	result["stop_to_exact_frame_ms"] = float(trace.last_rendered.frame_us - stop_us) / 1000.0
	result["moving_interval_ms"] = float(stop_us - move_start) / 1000.0
	result["observed_worker_service_fraction"] = observed_service_ms / result.moving_interval_ms if trace.worker_results > 0 else null
	result["worker_utilization_contract"] = "polled worker service wall time excluding core wait / moving interval; boundary jobs may be incomplete; not per-core CPU utilization"
	result["viewport_cpu_ms"] = Metrics.distribution(trace.viewport_cpu_ms)
	result["viewport_gpu_ms"] = Metrics.distribution(trace.viewport_gpu_ms)
	result["gpu_contract"] = "latest engine viewport timing, potentially delayed; not a per-request upload timer; zero/unavailable samples are not evidence of zero GPU work"
	result["final"] = trace.last_rendered
	result["frame_ms"] = Metrics.distribution(trace.frame_ms)
	result["frame_wall_ms"] = trace.frame_ms.duplicate()
	result["frames_over_16_7ms"] = trace.frame_ms.filter(func(ms): return ms > 16.7).size()
	result["frames_over_33ms"] = trace.frame_ms.filter(func(ms): return ms > 33.333).size()
	result["frames_over_50ms"] = trace.frame_ms.filter(func(ms): return ms > 50.0).size()
	result["stationary_ms"] = Metrics.distribution(result.stationary.map(func(row): return row.input_to_frame_ms))
	result["new_result_age_ms"] = Metrics.distribution(result.moving.map(func(row): return row.input_to_frame_ms))
	result["moving_display_age_samples"] = trace.display_age_samples.filter(func(row): return row.frame_us >= move_start and row.frame_us <= stop_us)
	result["moving_display_age_ms"] = Metrics.distribution(result.moving_display_age_samples.map(func(row): return row.age_ms))
	result["scheduled_input_lifecycle"] = trace.scheduled_input_lifecycle()
	var sampled_lifecycle: Array = result.scheduled_input_lifecycle.filter(func(row): return row.has("sampled_us"))
	var exact_lifecycle: Array = sampled_lifecycle.filter(func(row): return row.disposition in ["exact_on_time", "late_exact"])
	var stale_lifecycle: Array = sampled_lifecycle.filter(func(row): return row.disposition == "stale_displayed")
	var disposition_counts := {}
	for row in result.scheduled_input_lifecycle:
		disposition_counts[row.disposition] = int(disposition_counts.get(row.disposition, 0)) + 1
	result["sampled_input_deadline"] = {
		"contract": "RoadTool pose sample to first frame boundary showing that exact latest pose; every sampled pose is in the denominator",
		"budget_ms": 16.7,
		"scheduled_count": result.scheduled_input_lifecycle.size(),
		"sampled_count": sampled_lifecycle.size(),
		"exact_frame_count": exact_lifecycle.size(),
		"met_count": sampled_lifecycle.filter(func(row): return bool(row.get("strict_16_7_met", false))).size(),
		"miss_count": sampled_lifecycle.filter(func(row): return not bool(row.get("strict_16_7_met", false))).size(),
		"disposition_counts": disposition_counts,
		"sample_to_resolution_ms": Metrics.distribution(sampled_lifecycle.map(func(row): return row.sample_to_resolution_ms)),
		"sample_to_exact_frame_ms": Metrics.distribution(exact_lifecycle.map(func(row): return row.sample_to_frame_ms)),
		"sample_to_stale_frame_ms": Metrics.distribution(stale_lifecycle.map(func(row): return row.sample_to_stale_frame_ms)),
	}
	result["state_after"] = sim.get_road_benchmark_state()
	result["dropped"] = trace.dropped + trace.dropped_frames + trace.dropped_lifecycle
	result.ok = (
		result.dropped == 0
		and not result.moving.is_empty()
		and result.scheduled_input_lifecycle.size() == input_count
		and int(result.sampled_input_deadline.sampled_count) + int(disposition_counts.get("coalesced_before_sampling", 0)) == input_count
	)
	# A running simulation moves agents and grows buildings; the road state must not change.
	var unchanged := ["generation", "live_edges", "edge_slots", "nodes", "lanes"]
	if not populated:
		unchanged += ["agents", "buildings"]
	for field in unchanged:
		result.ok = result.ok and result.state_before[field] == result.state_after[field]
	if populated:
		bench.input_manager.set_simulation_speed(0.0)
	if MetrumDebug.is_enabled("perf"):
		# Outside latency/frame distributions: this drains CPU rendering commands, not GPU scanout.
		var drain_us := Time.get_ticks_usec()
		RenderingServer.force_sync()
		result["post_workload_cpu_command_drain_ms"] = float(Time.get_ticks_usec() - drain_us) / 1000.0
	var capture_dir := OS.get_environment("METRUM_GAMEPLAY_PREVIEW_CAPTURE_DIR")
	if not capture_dir.is_empty() and bench.mode != "headless":
		DirAccess.make_dir_recursive_absolute(capture_dir)
		await RenderingServer.frame_post_draw
		result["capture"] = capture_dir.path_join("%s_%d.png" % [name, mode])
		result.ok = result.ok and bench.get_viewport().get_texture().get_image().save_png(result.capture) == OK
	# The final pose's payload by key (packed lengths only), after every timed frame.
	var payload_bytes := {}
	for key in tool._preview:
		payload_bytes[key] = preload("res://scripts/benchmarks/road_preview_metrics.gd").packed_bytes(tool._preview[key])
	result["payload_bytes"] = payload_bytes
	tool.cancel_road()
	return result

# Loads the saved populated city and waits until the load flow would hand it over (T53 (a)): the
# terrain renderer settled, the network clean; the preview's input handling stays disabled.
func _load_populated(bench: Node) -> Dictionary:
	if bench.save_path.is_empty() or not FileAccess.file_exists(bench.save_path):
		return {"ok": false, "error": "populated cases need METRUM_GAMEPLAY_BENCHMARK_SAVE_PATH"}
	var load_us := Time.get_ticks_usec()
	if not bench.input_manager.menu_load_game_from_path(bench.save_path):
		return {"ok": false, "error": "populated load"}
	bench.input_manager.set_simulation_speed(0.0)
	while bench._elapsed_ms(load_us) < bench.settle_timeout_sec * 1000.0:
		await bench.get_tree().process_frame
		bench.input_manager._await_world_ready()
		if bench.input_manager.is_world_ready() and not bench.simulation_node.is_network_dirty():
			var idle: Dictionary = await bench._wait_for_idle(bench.settle_timeout_sec)
			idle["load_ms"] = bench._elapsed_ms(load_us)
			return idle
	return {"ok": false, "error": "populated load did not settle"}

# sloped_t's and sloped_branch's pointer paths at the commit breakdown's first edit site
# (`trunk_branch_starts`: 100 m west of the street grid on the west border trunk nearest z = 0),
# mirrored to the trunk's north side, where the breakdown's branch goes, and the T's sweep
# mirrored away from the grid: the T's start 80 m north with its end sliding along the trunk
# from 40 m east to 90 m west of the site; the branch from the site on the trunk, its end circling
# 15 m around a point 5 m east and 55 m north. No setup roads.
func _populated_scenario(bench: Node, name: String) -> Dictionary:
	var half: float = bench.simulation_node.get_terrain_world_size().x * 0.5
	var west_x := INF
	var trunk_z := INF
	for pos in bench.simulation_node.get_network_nodes():
		if pos.x > 50.0 - half:
			west_x = minf(west_x, pos.x)
		elif absf(pos.z) < absf(trunk_z):
			trunk_z = pos.z
	var site := Vector2(west_x - 100.0, trunk_z)
	if name == "populated_branch":
		return {"roads": [], "centre": site, "start": site, "orbit": [site + Vector2(5, 55), 15.0]}
	return {"roads": [], "centre": site, "start": site + Vector2(0, 80),
		"sweep": [site + Vector2(40, 0), site + Vector2(-90, 0)]}

# One moving-phase frame's published simulation state, to show the simulation ran: when it was
# read, the agents and the pathfinds so far (the snapshot, never the core lock).
func _sample_simulation(sim: Node, frames: Array) -> void:
	var stats: Dictionary = sim.get_perf_stats()
	frames.append([Time.get_ticks_usec(), int(stats.agent_count), int(stats.pathfind_calls)])

# Setup roads, preview start and pointer path: a "sweep" segment or an "orbit" [center, radius].
func _scenario(name: String) -> Dictionary:
	var main := [Vector2(-110, 0), Vector2(110, 0)]
	if name.ends_with("branch"):
		# New leg from the T junction at (-65, 0) to free ground; the pointer circles the target.
		return {"roads": [main, [Vector2(-65, 0), Vector2(-65, 60)]], "start": Vector2(-65, 0),
			"orbit": [Vector2(-60, -55), 15.0]}
	if name.ends_with("cross"):
		# Free start and end; the preview crosses four parallel roads, planning four junctions.
		var parallel := []
		for x in [-45.0, -15.0, 15.0, 45.0]:
			parallel.append([Vector2(x, -90), Vector2(x, 90)])
		return {"roads": parallel, "start": Vector2(-90, -30), "orbit": [Vector2(90, 30), 20.0]}
	var roads := [main]
	if name.ends_with("multi"):
		roads.append([Vector2(-65, -60), Vector2(-65, 60)])
	if name == "dense":
		for z in range(32, 113, 16):
			roads.append([Vector2(-100, z), Vector2(100, z)])
	# T onto the main road; the endpoint slides along it, clear of the multi crossing at x = -65.
	return {"roads": roads, "start": Vector2(0, -80), "sweep": [Vector2(-40, 0), Vector2(90, 0)]}

# u in [0, 1): fraction along the sweep or of a turn around the orbit.
func _pose(scenario: Dictionary, u: float) -> Vector2:
	if scenario.has("sweep"):
		return scenario.sweep[0].lerp(scenario.sweep[1], u)
	return scenario.orbit[0] + Vector2.from_angle(TAU * u) * float(scenario.orbit[1])

# Constant-speed pointer: back and forth along a sweep, round and round an orbit.
func _moving_pose(scenario: Dictionary, index: int) -> Vector2:
	var travelled := float(index) * MOVE_SPEED_MPS / 60.0
	if scenario.has("sweep"):
		var length: float = scenario.sweep[0].distance_to(scenario.sweep[1])
		return _pose(scenario, pingpong(travelled, length) / length)
	var circumference := TAU * float(scenario.orbit[1])
	return _pose(scenario, fposmod(travelled, circumference) / circumference)

func _set_pointer(bench: Node, xz: Vector2) -> void:
	bench.road_tool.set_scripted_pointer(true, bench._surface_point(xz))
	bench.road_tool._queue_preview_update()

func _wait_exact(bench: Node) -> bool:
	var tool: Node = bench.road_tool
	var deadline := Time.get_ticks_msec() + 30000
	while Time.get_ticks_msec() < deadline:
		await bench.get_tree().process_frame
		var row: Dictionary = tool.preview_metrics.last_rendered
		if not row.is_empty() and row.input_id == tool.preview_metrics.input_id and row.request_id == tool._preview_drawn_sequence and not tool._preview_update_pending:
			return true
	return false
