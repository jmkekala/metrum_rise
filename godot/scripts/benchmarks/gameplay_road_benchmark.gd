## Deterministic end-to-end road-building profiler workload for Samply captures.
##
## Loads a real authored world, drives the production RoadTool preview and commit paths, waits for
## the matching terrain/road/water render work to settle, and verifies bend, T, and four-way nodes.
## The harness is only attached when `--gameplay-road-benchmark` is passed on the command line.
extends Node

const RoadToolScript = preload("res://scripts/tools/road_tool.gd")
const BENCHMARK_NAME := "gameplay_roads"
const DEFAULT_WORLD_PATH := "res://../maps/processed/Kuopio/kuopio_324km2_10m.sqlite"
const DEFAULT_RESULTS_PATH := "res://../benchmark-results/gameplay-roads-direct.json"
const ROAD_HALF_SPAN_M := 90.0
const DEFAULT_FIXTURE_SPACING_M := 640
const FIXTURE_MARGIN_M := 260.0
const CAMERA_RADIUS_M := 190.0
const IDLE_STABLE_FRAMES := 5

var simulation_node: Node
var terrain: Node
var water: Node
var road_tool: Node
var input_manager: Node
var camera: Node

var mode := "windowed"
var world_path := ""
var results_path := ""
var run_id := ""
var repetitions := 3
var warmup_repetitions := 1
var settle_timeout_sec := 180.0
var fixture_spacing_m := float(DEFAULT_FIXTURE_SPACING_M)
var _metrics: Dictionary = {}
var _phase_sequence := 0

func _ready() -> void:
	# A profiling run owns the gameplay scene. Prevent UI/tool events from contaminating it before
	# the deferred workload starts; scripted RoadTool calls do not travel through viewport input.
	get_viewport().set_disable_input(true)

func run() -> void:
	_resolve_configuration()
	_resolve_nodes()
	_metrics = {
		"schema_version": 1,
		"benchmark": BENCHMARK_NAME,
		"mode": mode,
		"run_id": run_id,
		"world_path": world_path,
		"repetitions": repetitions,
		"warmup_repetitions": warmup_repetitions,
		"fixture_spacing_m": fixture_spacing_m,
		"started_unix_ms": _unix_time_ms(),
		"phases": [],
		"fixtures": [],
		"success": false,
	}

	if not _nodes_are_ready():
		_fail("main scene benchmark dependencies are unavailable")
		return
	if not FileAccess.file_exists(world_path):
		_fail("Kuopio world definition not found: %s" % world_path)
		return
	# InputManager polls the Input singleton for camera motion, so viewport event suppression alone
	# is insufficient to make the camera deterministic while keys or mouse buttons are held.
	input_manager.set_process(false)
	input_manager.set_process_input(false)
	input_manager.set_process_unhandled_input(false)
	if mode == "headless":
		# Headless mode is throughput-oriented and has no display cadence to preserve.
		Engine.max_fps = 0

	print(
		"[GAMEPLAY_BENCH] START run_id=%s mode=%s repetitions=%d warmups=%d world=%s"
		% [run_id, mode, repetitions, warmup_repetitions, world_path]
	)
	var load_phase := _phase_begin("world_load", {})
	var load_call_start_us := Time.get_ticks_usec()
	var loaded: bool = input_manager.menu_load_world_definition(world_path)
	var load_call_ms := _elapsed_ms(load_call_start_us)
	if not loaded:
		_phase_end(load_phase, {"ok": false, "load_call_ms": load_call_ms})
		_fail("failed to load world definition: %s" % world_path)
		return
	var load_wait: Dictionary = await _wait_for_idle(settle_timeout_sec)
	_phase_end(
		load_phase,
		{
			"ok": bool(load_wait.get("ok", false)),
			"load_call_ms": load_call_ms,
			"settle_ms": float(load_wait.get("elapsed_ms", 0.0)),
			"network_generation": simulation_node.get_network_render_generation(),
		}
	)
	if not bool(load_wait.get("ok", false)):
		_fail("world render work did not settle", load_wait)
		return

	var fixture_count := (repetitions + warmup_repetitions) * 3
	var selection_phase := _phase_begin("fixture_selection", {"requested": fixture_count})
	var anchors := _select_fixture_anchors(fixture_count)
	_phase_end(selection_phase, {"selected": anchors.size()})
	if anchors.size() < fixture_count:
		_fail(
			"only %d of %d deterministic road fixture sites were valid"
			% [anchors.size(), fixture_count]
		)
		return

	input_manager._cancel_active_tool()
	input_manager.current_tool = input_manager.Tool.ROAD
	input_manager._activate_tool_logic(input_manager.Tool.ROAD, true)
	road_tool.draw_mode = 0
	road_tool.fwd_lanes = 1
	road_tool.bkw_lanes = 1

	var anchor_index := 0
	for warmup_index in range(warmup_repetitions):
		for topology in ["bend", "t_junction", "four_way"]:
			var warmup_result: Dictionary = await _run_fixture(
				topology,
				warmup_index,
				true,
				anchors[anchor_index]
			)
			anchor_index += 1
			_metrics.fixtures.append(warmup_result)
			if not bool(warmup_result.get("ok", false)):
				_fail("warmup fixture failed", warmup_result)
				return

	for repetition in range(repetitions):
		for topology in ["bend", "t_junction", "four_way"]:
			var fixture_result: Dictionary = await _run_fixture(
				topology,
				repetition,
				false,
				anchors[anchor_index]
			)
			anchor_index += 1
			_metrics.fixtures.append(fixture_result)
			if not bool(fixture_result.get("ok", false)):
				_fail("measured fixture failed", fixture_result)
				return

	_stop_scripted_tool()
	_metrics["summary"] = _build_summary()
	_metrics["success"] = true
	_metrics["finished_unix_ms"] = _unix_time_ms()
	if not _write_metrics():
		push_error("Gameplay benchmark completed but could not write metrics: %s" % results_path)
		get_tree().quit(1)
		return
	print("[GAMEPLAY_BENCH] COMPLETE run_id=%s results=%s" % [run_id, results_path])
	get_tree().quit(0)

func _resolve_configuration() -> void:
	mode = _environment_or("METRUM_GAMEPLAY_BENCHMARK_MODE", "windowed").to_lower()
	if mode != "headless":
		mode = "windowed"
	world_path = ProjectSettings.globalize_path(
		_environment_or("METRUM_GAMEPLAY_BENCHMARK_WORLD_PATH", DEFAULT_WORLD_PATH)
	)
	results_path = ProjectSettings.globalize_path(
		_environment_or("METRUM_GAMEPLAY_BENCHMARK_METRICS_PATH", DEFAULT_RESULTS_PATH)
	)
	run_id = _environment_or(
		"METRUM_GAMEPLAY_BENCHMARK_RUN_ID",
		str(int(Time.get_unix_time_from_system()))
	)
	repetitions = _environment_int("METRUM_GAMEPLAY_BENCHMARK_REPETITIONS", 3, 1)
	warmup_repetitions = _environment_int(
		"METRUM_GAMEPLAY_BENCHMARK_WARMUP_REPETITIONS",
		1,
		0
	)
	settle_timeout_sec = float(
		_environment_int("METRUM_GAMEPLAY_BENCHMARK_TIMEOUT_SEC", 180, 10)
	)
	fixture_spacing_m = float(
		_environment_int(
			"METRUM_GAMEPLAY_BENCHMARK_FIXTURE_SPACING_M",
			DEFAULT_FIXTURE_SPACING_M,
			int(ROAD_HALF_SPAN_M * 2.0 + 20.0)
		)
	)

func _resolve_nodes() -> void:
	var main := get_parent()
	if main == null:
		return
	simulation_node = main.get_node_or_null("SimulationNode")
	terrain = main.get_node_or_null("Terrain")
	water = main.get_node_or_null("Water")
	road_tool = main.get_node_or_null("RoadTool")
	input_manager = main.get_node_or_null("InputManager")
	camera = main.get_node_or_null("CameraNode")

func _nodes_are_ready() -> bool:
	return (
		simulation_node != null
		and terrain != null
		and water != null
		and road_tool != null
		and input_manager != null
		and camera != null
	)

func _select_fixture_anchors(required_count: int) -> Array[Vector2]:
	var anchors: Array[Vector2] = []
	var world_size: Vector2 = simulation_node.get_terrain_world_size()
	var half_width := world_size.x * 0.5 - FIXTURE_MARGIN_M
	var half_depth := world_size.y * 0.5 - FIXTURE_MARGIN_M
	for ring in range(0, 20):
		for grid_z in range(-ring, ring + 1):
			for grid_x in range(-ring, ring + 1):
				if maxi(absi(grid_x), absi(grid_z)) != ring:
					continue
				var anchor := Vector2(
					float(grid_x) * fixture_spacing_m,
					float(grid_z) * fixture_spacing_m
				)
				if absf(anchor.x) + ROAD_HALF_SPAN_M > half_width:
					continue
				if absf(anchor.y) + ROAD_HALF_SPAN_M > half_depth:
					continue
				if not _anchor_supports_all_topologies(anchor):
					continue
				anchors.append(anchor)
				if anchors.size() >= required_count:
					return anchors
	return anchors

func _anchor_supports_all_topologies(anchor: Vector2) -> bool:
	var left := anchor + Vector2(-ROAD_HALF_SPAN_M, 0.0)
	var right := anchor + Vector2(ROAD_HALF_SPAN_M, 0.0)
	var down := anchor + Vector2(0.0, -ROAD_HALF_SPAN_M)
	var up := anchor + Vector2(0.0, ROAD_HALF_SPAN_M)
	for endpoints in [[left, anchor], [anchor, up], [left, right], [down, up]]:
		var points := PackedVector3Array([
			_surface_point(endpoints[0]),
			_surface_point(endpoints[1]),
		])
		var validation_variant = simulation_node.validate_road_candidate_with_snap(
			points,
			1,
			1,
			true
		)
		if not validation_variant is Dictionary:
			return false
		if not bool(validation_variant.get("is_valid", false)):
			return false
	return true

func _run_fixture(
	topology: String,
	repetition: int,
	is_warmup: bool,
	anchor: Vector2
) -> Dictionary:
	var identity := {
		"topology": topology,
		"repetition": repetition,
		"warmup": is_warmup,
		"anchor_x": anchor.x,
		"anchor_z": anchor.y,
	}
	var camera_phase := _phase_begin("camera_settle", identity)
	var anchor_y := float(simulation_node.get_world_surface_height(anchor))
	camera.focus_on(Vector3(anchor.x, anchor_y, anchor.y), CAMERA_RADIUS_M)
	var camera_wait: Dictionary = await _wait_for_idle(settle_timeout_sec)
	_phase_end(
		camera_phase,
		{
			"ok": bool(camera_wait.get("ok", false)),
			"elapsed_ms": float(camera_wait.get("elapsed_ms", 0.0)),
		}
	)
	if not bool(camera_wait.get("ok", false)):
		return _fixture_failure(identity, "camera render work did not settle", camera_wait)

	var fixture_phase := _phase_begin("road_fixture", identity)
	var fixture_start_us := Time.get_ticks_usec()
	var fixture := identity.duplicate(true)
	fixture["segments"] = []
	var segments := _topology_segments(topology, anchor)
	for segment_index in range(segments.size()):
		var endpoints: Array = segments[segment_index]
		var segment_result: Dictionary = await _run_segment(
			topology,
			repetition,
			is_warmup,
			segment_index,
			endpoints[0],
			endpoints[1]
		)
		fixture.segments.append(segment_result)
		if not bool(segment_result.get("ok", false)):
			fixture["ok"] = false
			fixture["error"] = "segment %d failed" % segment_index
			fixture["total_ms"] = _elapsed_ms(fixture_start_us)
			_phase_end(fixture_phase, {"ok": false, "error": fixture.error})
			return fixture

	var junction_point := _surface_point(anchor)
	var junction_id: int = simulation_node.get_closest_node(junction_point, 8.0)
	var actual_degree := (
		int(simulation_node.get_node_connection_count(junction_id)) if junction_id >= 0 else -1
	)
	var expected_degree := _expected_junction_degree(topology)
	fixture["junction_node_id"] = junction_id
	fixture["junction_degree"] = actual_degree
	fixture["expected_junction_degree"] = expected_degree
	fixture["total_ms"] = _elapsed_ms(fixture_start_us)
	fixture["ok"] = junction_id >= 0 and actual_degree == expected_degree
	if not fixture.ok:
		fixture["error"] = "junction degree mismatch: expected %d, got %d" % [
			expected_degree,
			actual_degree,
		]
	_phase_end(
		fixture_phase,
		{
			"ok": fixture.ok,
			"total_ms": fixture.total_ms,
			"junction_degree": actual_degree,
		}
	)
	return fixture

func _run_segment(
	topology: String,
	repetition: int,
	is_warmup: bool,
	segment_index: int,
	start_xz: Vector2,
	end_xz: Vector2
) -> Dictionary:
	var identity := {
		"topology": topology,
		"repetition": repetition,
		"warmup": is_warmup,
		"segment": segment_index,
	}
	var start_pos := _surface_point(start_xz)
	var end_pos := _surface_point(end_xz)
	var preview_phase := _phase_begin("road_preview", identity)
	var preview_start_us := Time.get_ticks_usec()
	_begin_scripted_road(start_pos, end_pos)
	var preview_wait: Dictionary = await _wait_for_preview(settle_timeout_sec)
	var preview_ms := _elapsed_ms(preview_start_us)
	_phase_end(
		preview_phase,
		{
			"ok": bool(preview_wait.get("ok", false)),
			"elapsed_ms": preview_ms,
			"request_id": road_tool._preview_request_id,
		}
	)
	if not bool(preview_wait.get("ok", false)):
		return {
			"ok": false,
			"error": "preview did not complete",
			"preview_ms": preview_ms,
			"wait": preview_wait,
		}

	var generation_before: int = simulation_node.get_network_render_generation()
	var commit_phase := _phase_begin("road_commit", identity)
	var commit_start_us := Time.get_ticks_usec()
	var dispatch_start_us := Time.get_ticks_usec()
	var committed: bool = road_tool._commit_segment(end_pos)
	var dispatch_ms := _elapsed_ms(dispatch_start_us)
	if not committed:
		var rejected := {
			"ok": false,
			"error": "RoadTool rejected the commit",
			"preview_ms": preview_ms,
			"commit_dispatch_ms": dispatch_ms,
		}
		_phase_end(commit_phase, rejected)
		return rejected

	var expected_generation := generation_before + 1
	var settle: Dictionary = await _wait_for_generation(generation_before, settle_timeout_sec)
	var commit_ms := _elapsed_ms(commit_start_us)
	var result := {
		"ok": bool(settle.get("ok", false)),
		"preview_ms": preview_ms,
		"commit_dispatch_ms": dispatch_ms,
		"commit_ms": commit_ms,
		"generation_before": generation_before,
		"generation_expected": expected_generation,
		"generation_after": simulation_node.get_network_render_generation(),
		"start": [start_pos.x, start_pos.y, start_pos.z],
		"end": [end_pos.x, end_pos.y, end_pos.z],
	}
	if not result.ok:
		result["error"] = settle.get(
			"error",
			"committed generation did not render and settle"
		)
		result["wait"] = settle
	_phase_end(
		commit_phase,
		{
			"ok": result.ok,
			"elapsed_ms": commit_ms,
			"dispatch_ms": dispatch_ms,
			"expected_generation": expected_generation,
			"generation": result.generation_after,
		}
	)
	return result

func _begin_scripted_road(start_pos: Vector3, end_pos: Vector3) -> void:
	road_tool.cancel_road()
	road_tool.active = true
	road_tool.draw_mode = 0
	road_tool.fwd_lanes = 1
	road_tool.bkw_lanes = 1
	road_tool.start_pos = start_pos
	road_tool.control_pos = start_pos
	road_tool.current_state = RoadToolScript.State.SETTING_END
	road_tool.set_scripted_pointer(true, end_pos)
	road_tool.current_path = Path3D.new()
	road_tool.current_path.curve = Curve3D.new()
	road_tool.current_path.curve.bake_interval = 0.5
	road_tool.current_path.curve.up_vector_enabled = false
	road_tool.add_child(road_tool.current_path)
	road_tool._queue_preview_update()

func _wait_for_preview(timeout_sec: float) -> Dictionary:
	var start_us := Time.get_ticks_usec()
	while _elapsed_ms(start_us) < timeout_sec * 1000.0:
		await get_tree().process_frame
		var validation: Dictionary = road_tool._candidate_cache_validation
		if not validation.is_empty() and not bool(validation.get("is_valid", false)):
			return {
				"ok": false,
				"elapsed_ms": _elapsed_ms(start_us),
				"invalid_reason": validation.get("invalid_reason", "unknown"),
			}
		var preview: Dictionary = road_tool._preview_cache_surface
		if (
			not preview.is_empty()
			and bool(preview.get("is_valid", false))
			and not road_tool._preview_update_pending
			and not road_tool._preview_result_pending
			and not road_tool._preview_exact_waiting
			and road_tool._preview_surface_generation_is_current(preview)
		):
			await get_tree().process_frame
			return {"ok": true, "elapsed_ms": _elapsed_ms(start_us)}
	return {
		"ok": false,
		"elapsed_ms": _elapsed_ms(start_us),
		"pending": _pending_work_snapshot(),
	}

func _wait_for_generation(previous_generation: int, timeout_sec: float) -> Dictionary:
	var start_us := Time.get_ticks_usec()
	var expected_generation := previous_generation + 1
	var generation_advanced := false
	var stable_frames := 0
	while _elapsed_ms(start_us) < timeout_sec * 1000.0:
		await get_tree().process_frame
		if terrain.has_blocked_dirty_patch_failure():
			var terrain_failures: Array[Dictionary] = terrain.get_blocked_dirty_patch_failures()
			return {
				"ok": false,
				"error": "terrain refinement rejected the committed generation",
				"elapsed_ms": _elapsed_ms(start_us),
				"expected_generation": expected_generation,
				"actual_generation": simulation_node.get_network_render_generation(),
				"terrain_failures": terrain_failures,
				"pending": _pending_work_snapshot(),
			}
		var current_generation: int = simulation_node.get_network_render_generation()
		if current_generation > expected_generation:
			return {
				"ok": false,
				"error": "unexpected concurrent network mutation",
				"elapsed_ms": _elapsed_ms(start_us),
				"expected_generation": expected_generation,
				"actual_generation": current_generation,
				"pending": _pending_work_snapshot(),
			}
		if current_generation == expected_generation:
			generation_advanced = true
		if generation_advanced and _is_idle():
			stable_frames += 1
			if stable_frames >= IDLE_STABLE_FRAMES:
				return {"ok": true, "elapsed_ms": _elapsed_ms(start_us)}
		else:
			stable_frames = 0
	return {
		"ok": false,
		"elapsed_ms": _elapsed_ms(start_us),
		"generation_advanced": generation_advanced,
		"expected_generation": expected_generation,
		"actual_generation": simulation_node.get_network_render_generation(),
		"pending": _pending_work_snapshot(),
	}

func _wait_for_idle(timeout_sec: float) -> Dictionary:
	var start_us := Time.get_ticks_usec()
	var stable_frames := 0
	while _elapsed_ms(start_us) < timeout_sec * 1000.0:
		await get_tree().process_frame
		if terrain.has_blocked_dirty_patch_failure():
			var terrain_failures: Array[Dictionary] = terrain.get_blocked_dirty_patch_failures()
			return {
				"ok": false,
				"error": "terrain refinement rejected a dirty generation",
				"elapsed_ms": _elapsed_ms(start_us),
				"terrain_failures": terrain_failures,
				"pending": _pending_work_snapshot(),
			}
		if _is_idle():
			stable_frames += 1
			if stable_frames >= IDLE_STABLE_FRAMES:
				return {"ok": true, "elapsed_ms": _elapsed_ms(start_us)}
		else:
			stable_frames = 0
	return {
		"ok": false,
		"elapsed_ms": _elapsed_ms(start_us),
		"pending": _pending_work_snapshot(),
	}

func _is_idle() -> bool:
	return (
		not simulation_node.is_network_dirty()
		and not simulation_node.is_terrain_dirty()
		and not road_tool.needs_main_mesh_hydration()
		and road_tool._pending_border_checks.is_empty()
		and not road_tool._ghost_rebuild_queued
		and not terrain.has_pending_render_work(false)
		and not water.has_pending_render_work(false)
	)

func _pending_work_snapshot() -> Dictionary:
	return {
		"network_dirty": simulation_node.is_network_dirty(),
		"terrain_dirty": simulation_node.is_terrain_dirty(),
		"water_dirty": simulation_node.is_water_dirty(),
		"road_hydration": road_tool.needs_main_mesh_hydration(),
		"border_checks": road_tool._pending_border_checks.size(),
		"ghost_rebuild": road_tool._ghost_rebuild_queued,
		"terrain": terrain.get_pending_render_work_counts(),
		"terrain_failures": terrain.get_blocked_dirty_patch_failures(),
		"water": water.get_pending_render_work_counts(),
	}

func _topology_segments(topology: String, anchor: Vector2) -> Array:
	var left := anchor + Vector2(-ROAD_HALF_SPAN_M, 0.0)
	var right := anchor + Vector2(ROAD_HALF_SPAN_M, 0.0)
	var down := anchor + Vector2(0.0, -ROAD_HALF_SPAN_M)
	var up := anchor + Vector2(0.0, ROAD_HALF_SPAN_M)
	match topology:
		"bend":
			return [[left, anchor], [anchor, up]]
		"t_junction":
			return [[left, right], [up, anchor]]
		"four_way":
			return [[left, right], [down, up]]
	return []

func _expected_junction_degree(topology: String) -> int:
	match topology:
		"bend":
			return 2
		"t_junction":
			return 3
		"four_way":
			return 4
	return -1

func _surface_point(position: Vector2) -> Vector3:
	return Vector3(
		position.x,
		float(simulation_node.get_world_surface_height(position)),
		position.y
	)

func _build_summary() -> Dictionary:
	var summary := {}
	for topology in ["bend", "t_junction", "four_way"]:
		var preview_values: Array[float] = []
		var commit_values: Array[float] = []
		var total_values: Array[float] = []
		for fixture_variant in _metrics.fixtures:
			var fixture: Dictionary = fixture_variant
			if bool(fixture.get("warmup", false)) or fixture.get("topology", "") != topology:
				continue
			total_values.append(float(fixture.get("total_ms", 0.0)))
			for segment_variant in fixture.get("segments", []):
				var segment: Dictionary = segment_variant
				preview_values.append(float(segment.get("preview_ms", 0.0)))
				commit_values.append(float(segment.get("commit_ms", 0.0)))
		summary[topology] = {
			"fixture_count": total_values.size(),
			"preview_ms": _distribution(preview_values),
			"commit_ms": _distribution(commit_values),
			"fixture_total_ms": _distribution(total_values),
		}
	return summary

func _distribution(values: Array[float]) -> Dictionary:
	if values.is_empty():
		return {"count": 0, "p50": 0.0, "p95": 0.0, "max": 0.0}
	values.sort()
	return {
		"count": values.size(),
		"p50": _percentile(values, 0.50),
		"p95": _percentile(values, 0.95),
		"max": values[values.size() - 1],
	}

func _percentile(sorted_values: Array[float], quantile: float) -> float:
	var index := mini(
		sorted_values.size() - 1,
		maxi(0, int(ceil(quantile * float(sorted_values.size()))) - 1)
	)
	return sorted_values[index]

func _phase_begin(phase: String, details: Dictionary) -> int:
	_phase_sequence += 1
	var event := {
		"id": _phase_sequence,
		"event": "begin",
		"phase": phase,
		"ticks_usec": Time.get_ticks_usec(),
		"unix_ms": _unix_time_ms(),
		"details": details.duplicate(true),
	}
	_metrics.phases.append(event)
	print(
		"[GAMEPLAY_BENCH] PHASE_BEGIN id=%d phase=%s unix_ms=%.3f details=%s"
		% [_phase_sequence, phase, event.unix_ms, JSON.stringify(details)]
	)
	return _phase_sequence

func _phase_end(phase_id: int, details: Dictionary) -> void:
	var phase_name := "unknown"
	for event_variant in _metrics.phases:
		var event: Dictionary = event_variant
		if int(event.get("id", -1)) == phase_id and event.get("event", "") == "begin":
			phase_name = String(event.get("phase", "unknown"))
			break
	var end_event := {
		"id": phase_id,
		"event": "end",
		"phase": phase_name,
		"ticks_usec": Time.get_ticks_usec(),
		"unix_ms": _unix_time_ms(),
		"details": details.duplicate(true),
	}
	_metrics.phases.append(end_event)
	print(
		"[GAMEPLAY_BENCH] PHASE_END id=%d phase=%s unix_ms=%.3f details=%s"
		% [phase_id, phase_name, end_event.unix_ms, JSON.stringify(details)]
	)

func _fixture_failure(identity: Dictionary, message: String, details: Dictionary) -> Dictionary:
	var result := identity.duplicate(true)
	result["ok"] = false
	result["error"] = message
	result["details"] = details
	result["segments"] = []
	return result

func _fail(message: String, details: Dictionary = {}) -> void:
	_stop_scripted_tool()
	_metrics["success"] = false
	_metrics["error"] = message
	_metrics["error_details"] = details
	_metrics["finished_unix_ms"] = _unix_time_ms()
	_write_metrics()
	push_error("[GAMEPLAY_BENCH] FAILED: %s" % message)
	get_tree().quit(1)

func _stop_scripted_tool() -> void:
	if input_manager != null:
		input_manager._cancel_active_tool()
	if road_tool != null:
		road_tool.cancel_road()
		road_tool.active = false
		road_tool.set_scripted_pointer(false)

func _write_metrics() -> bool:
	var directory_error := DirAccess.make_dir_recursive_absolute(results_path.get_base_dir())
	if directory_error != OK:
		return false
	var file := FileAccess.open(results_path, FileAccess.WRITE)
	if file == null:
		return false
	file.store_string(JSON.stringify(_metrics, "\t"))
	file.store_line("")
	file.flush()
	return true

func _environment_or(name: String, fallback: String) -> String:
	var value := OS.get_environment(name).strip_edges()
	return fallback if value.is_empty() else value

func _environment_int(name: String, fallback: int, minimum: int) -> int:
	var value := OS.get_environment(name).strip_edges()
	if not value.is_valid_int():
		return fallback
	return maxi(minimum, value.to_int())

func _elapsed_ms(start_usec: int) -> float:
	return float(Time.get_ticks_usec() - start_usec) / 1000.0

func _unix_time_ms() -> float:
	return Time.get_unix_time_from_system() * 1000.0
