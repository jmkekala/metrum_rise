# SPDX-License-Identifier: GPL-2.0-only

## Preview-only matrix using the existing gameplay harness and production RoadTool.
## Setup/commits are excluded; each mode starts from the same deterministic world.
extends RefCounted

const Metrics := preload("res://scripts/benchmarks/road_benchmark_metrics.gd")
const CASES := ["flat_t", "sloped_t", "flat_multi", "sloped_multi", "dense", "residency_retry",
	"flat_branch", "sloped_branch", "flat_cross", "sloped_cross"]
# Road and terrain only: it is the mode with terrain preview work to measure.
const MODE := 1
# Moving-phase pointer speed: 0.5 m per 60 Hz input.
const MOVE_SPEED_MPS := 30.0
var _report: Dictionary

func run(bench: Node) -> void:
	bench.input_manager.set_process(false)
	bench.input_manager.set_process_input(false)
	bench.input_manager.set_process_unhandled_input(false)
	Engine.max_fps = bench._environment_int("METRUM_GAMEPLAY_BENCHMARK_MAX_FPS", 60, 1)
	if bench.mode != "headless":
		DisplayServer.window_set_title("Metrum Rise - road preview benchmark")
	_report = {
		"schema_version": 1, "benchmark": "road_preview_latency", "success": false,
		"runtime": bench._runtime_metadata(), "cases": [], "phases": [], "mode": bench.mode, "modes": [MODE],
		"expected_cases": CASES.filter(func(name): return bench.selected_case_ids.is_empty() or name in bench.selected_case_ids),
		"samples": bench.repetitions, "warmups": bench.warmup_repetitions,
		"timing_contract": "scripted world-space input to frame_post_draw; headless uses process_frame and is not rendering; no GPU upload or monitor presentation claim",
		"helper_sha256": FileAccess.get_sha256(get_script().resource_path),
	}
	_report.runtime["display_server"] = DisplayServer.get_name()
	_report.runtime["world_contract"] = "512m_cell8m_chunk128m_flat_or_2.5_percent_slope"
	_report.runtime["world_sha256"] = "road30_synthetic_v1"
	_report.runtime["fixture_isolation"] = "reset_each_case_and_mode"
	_report.runtime["trace_sha256"] = FileAccess.get_sha256("res://scripts/benchmarks/road_preview_metrics.gd")
	for name in CASES:
		if not bench.selected_case_ids.is_empty() and not name in bench.selected_case_ids:
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

func _case(bench: Node, name: String, mode: int) -> Dictionary:
	bench._stop_scripted_tool()
	var setup_us := Time.get_ticks_usec()
	var sim: Node = bench.simulation_node
	if not sim.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0):
		return {"ok": false, "error": "world creation"}
	bench.input_manager._refresh_after_world_load()
	bench.input_manager.set_simulation_speed(0.0)
	if name.begins_with("sloped"):
		sim.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128, 0), 0.0, Vector2(128, 0), 6.4, 1.0)
	bench.camera.set("orthogonal", true)
	bench.camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	bench.camera.focus_on(bench._surface_point(Vector2.ZERO), 150.0)
	var scenario := _scenario(name)
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
	tool.set_road_preview_mode(mode)
	var state: Dictionary = sim.get_road_benchmark_state()
	var result := {"case_id": name, "mode": mode, "ok": false,
		"setup_ms": float(Time.get_ticks_usec() - setup_us) / 1000.0,
		"stationary": [], "moving": [], "state_before": state}
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
		if name == "residency_retry" and mode == 1:
			# Withhold reference patch lookup for three retry frames. Do not destroy its resources.
			tool._clear_preview_cache()
			tool._preview_metric_row = {}
			var held: Dictionary = bench.terrain.patches
			bench.terrain.set_process(false)
			bench.terrain.patches = {}
			var deadline := Time.get_ticks_msec() + 30000
			while tool._preview_cache_surface.is_empty() and Time.get_ticks_msec() < deadline:
				await bench.get_tree().process_frame
			while tool._preview_metric_row.get("terrain_retries", 0) < 3 and Time.get_ticks_msec() < deadline:
				await bench.get_tree().process_frame
			bench.terrain.patches = held
			bench.terrain.set_process(true)
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
		input_lateness_ms.append(float(Time.get_ticks_usec() - due_us) / 1000.0)
		_set_pointer(bench, _moving_pose(scenario, index))
	while Time.get_ticks_usec() < move_start + int(float(input_count) * 1000000.0 / 60.0):
		await bench.get_tree().process_frame
	result["scheduled_input_count"] = input_count
	result["scheduled_input_hz"] = 60
	result["input_lateness_ms"] = Metrics.distribution(input_lateness_ms)
	var stop_us := Time.get_ticks_usec()
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
	result["frames_over_33ms"] = trace.frame_ms.filter(func(ms): return ms > 33.333).size()
	result["frames_over_50ms"] = trace.frame_ms.filter(func(ms): return ms > 50.0).size()
	result["stationary_ms"] = Metrics.distribution(result.stationary.map(func(row): return row.input_to_frame_ms))
	result["new_result_age_ms"] = Metrics.distribution(result.moving.map(func(row): return row.input_to_frame_ms))
	result["moving_display_age_samples"] = trace.display_age_samples.filter(func(row): return row.frame_us >= move_start and row.frame_us <= stop_us)
	result["moving_display_age_ms"] = Metrics.distribution(result.moving_display_age_samples.map(func(row): return row.age_ms))
	result["state_after"] = sim.get_road_benchmark_state()
	result["dropped"] = trace.dropped + trace.dropped_frames
	result.ok = result.dropped == 0 and not result.moving.is_empty()
	for field in ["generation", "live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings"]:
		result.ok = result.ok and result.state_before[field] == result.state_after[field]
	if OS.get_environment("METRUM_DEBUG_PERF") == "1":
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
	tool.cancel_road()
	return result

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
		if not row.is_empty() and row.input_id == tool.preview_metrics.input_id and row.request_id == tool._preview_drawn_request_id and not tool._preview_update_pending and not tool._preview_result_pending:
			return true
	return false
