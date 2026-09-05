# SPDX-License-Identifier: GPL-2.0-only

## Deterministic end-to-end road-building profiler workload for Samply captures.
##
## Loads a real authored world, drives the production RoadTool preview and commit paths, waits for
## the matching terrain/road/water render work to settle, and verifies a controlled layout matrix.
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
const MATRIX_SCHEMA_VERSION := 1
const DEFAULT_MATRIX_NAME := "controlled"

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
var matrix_name := DEFAULT_MATRIX_NAME
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
	var fixture_definitions := _fixture_definitions()
	_metrics = {
		"schema_version": 2,
		"benchmark": BENCHMARK_NAME,
		"mode": mode,
		"run_id": run_id,
		"world_path": world_path,
		"matrix_name": matrix_name,
		"matrix_schema_version": MATRIX_SCHEMA_VERSION,
		"matrix_cases": _fixture_descriptors(fixture_definitions),
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
	if fixture_definitions.is_empty():
		_fail("unknown gameplay road benchmark matrix: %s" % matrix_name)
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

	var workload := _fixture_workload(fixture_definitions)
	var fixture_count := workload.size()
	var selection_phase := _phase_begin("fixture_selection", {"requested": fixture_count})
	var anchors := _select_fixture_anchors(workload)
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
	var active_cycle_index := -1
	for workload_index in range(workload.size()):
		var entry: Dictionary = workload[workload_index]
		var cycle_index: int = entry["cycle_index"]
		if (
			matrix_name == DEFAULT_MATRIX_NAME
			and active_cycle_index >= 0
			and cycle_index != active_cycle_index
		):
			var reload_result: Dictionary = await _reload_world_for_matrix_cycle(entry)
			if not bool(reload_result.get("ok", false)):
				_fail("world reload between matrix cycles failed", reload_result)
				return
		active_cycle_index = cycle_index
		var fixture_result: Dictionary = await _run_fixture(
			entry["fixture"] as Dictionary,
			int(entry["repetition"]),
			bool(entry["warmup"]),
			anchors[workload_index]
		)
		_metrics.fixtures.append(fixture_result)
		if not bool(fixture_result.get("ok", false)):
			var fixture_kind := "warmup" if bool(entry["warmup"]) else "measured"
			_fail("%s fixture failed" % fixture_kind, fixture_result)
			return

	_stop_scripted_tool()
	_metrics["summary"] = _build_summary(fixture_definitions)
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
	matrix_name = _environment_or(
		"METRUM_GAMEPLAY_BENCHMARK_MATRIX",
		DEFAULT_MATRIX_NAME
	).to_lower()
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

func _reload_world_for_matrix_cycle(entry: Dictionary) -> Dictionary:
	var identity := {
		"cycle_index": entry["cycle_index"],
		"repetition": entry["repetition"],
		"warmup": entry["warmup"],
	}
	var reload_phase := _phase_begin("world_reload", identity)
	_stop_scripted_tool()
	var load_call_start_us := Time.get_ticks_usec()
	var loaded: bool = input_manager.menu_load_world_definition(world_path)
	var load_call_ms := _elapsed_ms(load_call_start_us)
	if not loaded:
		var load_failure := identity.duplicate(true)
		load_failure["ok"] = false
		load_failure["load_call_ms"] = load_call_ms
		_phase_end(reload_phase, load_failure)
		return load_failure
	var load_wait: Dictionary = await _wait_for_idle(settle_timeout_sec)
	var result := identity.duplicate(true)
	result["ok"] = bool(load_wait.get("ok", false))
	result["load_call_ms"] = load_call_ms
	result["settle_ms"] = float(load_wait.get("elapsed_ms", 0.0))
	result["network_generation"] = simulation_node.get_network_render_generation()
	if not result.ok:
		result["wait"] = load_wait
	_phase_end(reload_phase, result)
	if result.ok:
		input_manager.current_tool = input_manager.Tool.ROAD
		input_manager._activate_tool_logic(input_manager.Tool.ROAD, true)
	return result

func _fixture_definitions() -> Array[Dictionary]:
	var baselines: Array[Dictionary] = [
		{
			"case_id": "bend_90_2l",
			"topology": "bend",
			"complexity_axis": "baseline",
			"baseline_case": "",
			"anchor_alignment": "free",
			"segments": [
				_straight_segment(Vector2(-90.0, 0.0), Vector2.ZERO),
				_straight_segment(Vector2.ZERO, Vector2(0.0, 90.0)),
			],
			"junctions": [_junction_check(Vector2.ZERO, 2)],
		},
		{
			"case_id": "t_90_2l",
			"topology": "t_junction",
			"complexity_axis": "baseline",
			"baseline_case": "",
			"anchor_alignment": "free",
			"segments": [
				_straight_segment(Vector2(-90.0, 0.0), Vector2(90.0, 0.0)),
				_straight_segment(Vector2(0.0, 90.0), Vector2.ZERO),
			],
			"junctions": [_junction_check(Vector2.ZERO, 3)],
		},
		{
			"case_id": "four_way_90_2l",
			"topology": "four_way",
			"complexity_axis": "baseline",
			"baseline_case": "",
			"anchor_alignment": "free",
			"segments": [
				_straight_segment(Vector2(-90.0, 0.0), Vector2(90.0, 0.0)),
				_straight_segment(Vector2(0.0, -90.0), Vector2(0.0, 90.0)),
			],
			"junctions": [_junction_check(Vector2.ZERO, 4)],
		},
	]
	if matrix_name == "baseline":
		return baselines
	if matrix_name != DEFAULT_MATRIX_NAME:
		return []

	var definitions := baselines.duplicate(true) as Array[Dictionary]
	definitions.append_array([
		{
			"case_id": "t_oblique_55deg_2l",
			"topology": "t_junction",
			"complexity_axis": "approach_angle",
			"baseline_case": "t_90_2l",
			"anchor_alignment": "free",
			"segments": [
				_straight_segment(Vector2(-90.0, 0.0), Vector2(90.0, 0.0)),
				_straight_segment(_polar_offset(90.0, 55.0), Vector2.ZERO),
			],
			"junctions": [_junction_check(Vector2.ZERO, 3)],
		},
		{
			"case_id": "four_way_mixed_8l_2l",
			"topology": "four_way",
			"complexity_axis": "lane_width",
			"baseline_case": "four_way_90_2l",
			"anchor_alignment": "free",
			"segments": [
				_straight_segment(Vector2(-90.0, 0.0), Vector2(90.0, 0.0), 4, 4),
				_straight_segment(Vector2(0.0, -90.0), Vector2(0.0, 90.0)),
			],
			"junctions": [_junction_check(Vector2.ZERO, 4)],
		},
		{
			"case_id": "bend_curved_2l",
			"topology": "bend",
			"complexity_axis": "spline_curvature",
			"baseline_case": "bend_90_2l",
			"anchor_alignment": "free",
			"segments": [
				_straight_segment(Vector2(-90.0, 0.0), Vector2.ZERO),
				_spline_segment(
					Vector2.ZERO,
					Vector2(0.0, 90.0),
					Vector2(12.0, 45.0)
				),
			],
			"junctions": [_junction_check(Vector2.ZERO, 2)],
		},
		{
			"case_id": "double_t_close_2l",
			"topology": "junction_cluster",
			"complexity_axis": "local_density",
			"baseline_case": "t_90_2l",
			"anchor_alignment": "free",
			"segments": [
				_straight_segment(Vector2(-90.0, 0.0), Vector2(90.0, 0.0)),
				_straight_segment(Vector2(-32.0, 70.0), Vector2(-32.0, 0.0)),
				_straight_segment(Vector2(32.0, -70.0), Vector2(32.0, 0.0)),
			],
			"junctions": [
				_junction_check(Vector2(-32.0, 0.0), 3),
				_junction_check(Vector2(32.0, 0.0), 3),
			],
		},
		{
			"case_id": "four_way_chunk_corner_2l",
			"topology": "four_way",
			"complexity_axis": "chunk_alignment",
			"baseline_case": "four_way_90_2l",
			"anchor_alignment": "chunk_corner",
			"segments": [
				_straight_segment(Vector2(-90.0, 0.0), Vector2(90.0, 0.0)),
				_straight_segment(Vector2(0.0, -90.0), Vector2(0.0, 90.0)),
			],
			"junctions": [_junction_check(Vector2.ZERO, 4)],
		},
	])
	return definitions

func _straight_segment(
	start_offset: Vector2,
	end_offset: Vector2,
	fwd_lanes: int = 1,
	bkw_lanes: int = 1
) -> Dictionary:
	return {
		"draw_mode": 0,
		"start_offset": start_offset,
		"end_offset": end_offset,
		"fwd_lanes": fwd_lanes,
		"bkw_lanes": bkw_lanes,
	}

func _spline_segment(
	start_offset: Vector2,
	end_offset: Vector2,
	control_offset: Vector2,
	fwd_lanes: int = 1,
	bkw_lanes: int = 1
) -> Dictionary:
	return {
		"draw_mode": 1,
		"start_offset": start_offset,
		"end_offset": end_offset,
		"control_offset": control_offset,
		"fwd_lanes": fwd_lanes,
		"bkw_lanes": bkw_lanes,
	}

func _junction_check(offset: Vector2, expected_degree: int) -> Dictionary:
	return {"offset": offset, "expected_degree": expected_degree}

func _polar_offset(radius_m: float, angle_degrees: float) -> Vector2:
	var angle_radians := deg_to_rad(angle_degrees)
	return Vector2(cos(angle_radians), sin(angle_radians)) * radius_m

func _fixture_workload(fixture_definitions: Array[Dictionary]) -> Array[Dictionary]:
	var workload: Array[Dictionary] = []
	var cycle_index := 0
	for warmup_index in range(warmup_repetitions):
		for fixture in fixture_definitions:
			workload.append({
				"fixture": fixture,
				"repetition": warmup_index,
				"warmup": true,
				"cycle_index": cycle_index,
			})
		cycle_index += 1
	for repetition in range(repetitions):
		for fixture in fixture_definitions:
			workload.append({
				"fixture": fixture,
				"repetition": repetition,
				"warmup": false,
				"cycle_index": cycle_index,
			})
		cycle_index += 1
	return workload

func _select_fixture_anchors(workload: Array[Dictionary]) -> Array[Vector2]:
	var anchors: Array[Vector2] = []
	var world_size: Vector2 = simulation_node.get_terrain_world_size()
	var half_width := world_size.x * 0.5 - FIXTURE_MARGIN_M
	var half_depth := world_size.y * 0.5 - FIXTURE_MARGIN_M
	var used_anchors: Dictionary = {}
	var active_cycle_index := -1
	for entry in workload:
		var cycle_index: int = entry["cycle_index"]
		if (
			matrix_name == DEFAULT_MATRIX_NAME
			and active_cycle_index >= 0
			and cycle_index != active_cycle_index
		):
			used_anchors.clear()
		active_cycle_index = cycle_index
		var fixture: Dictionary = entry["fixture"]
		var anchor_result := _find_fixture_anchor(
			fixture,
			used_anchors,
			half_width,
			half_depth
		)
		if not bool(anchor_result.get("ok", false)):
			return anchors
		var anchor: Vector2 = anchor_result["anchor"]
		anchors.append(anchor)
		used_anchors[anchor] = true
	return anchors

func _find_fixture_anchor(
	fixture: Dictionary,
	used_anchors: Dictionary,
	half_width: float,
	half_depth: float
) -> Dictionary:
	for ring in range(0, 20):
		for grid_z in range(-ring, ring + 1):
			for grid_x in range(-ring, ring + 1):
				if maxi(absi(grid_x), absi(grid_z)) != ring:
					continue
				var raw_anchor := Vector2(
					float(grid_x) * fixture_spacing_m,
					float(grid_z) * fixture_spacing_m
				)
				var anchor := _align_fixture_anchor(raw_anchor, fixture)
				if used_anchors.has(anchor):
					continue
				if not _fixture_fits_world(anchor, fixture, half_width, half_depth):
					continue
				if not _anchor_supports_fixture(anchor, fixture):
					continue
				return {"ok": true, "anchor": anchor}
	return {"ok": false}

func _align_fixture_anchor(anchor: Vector2, fixture: Dictionary) -> Vector2:
	if fixture.get("anchor_alignment", "free") != "chunk_corner":
		return anchor
	var chunk_span_m: float = road_tool._road_chunk_span_m
	if chunk_span_m <= 0.0:
		return anchor
	return Vector2(
		road_tool._road_chunk_origin_x_m
			+ round((anchor.x - road_tool._road_chunk_origin_x_m) / chunk_span_m) * chunk_span_m,
		road_tool._road_chunk_origin_z_m
			+ round((anchor.y - road_tool._road_chunk_origin_z_m) / chunk_span_m) * chunk_span_m
	)

func _fixture_fits_world(
	anchor: Vector2,
	fixture: Dictionary,
	half_width: float,
	half_depth: float
) -> bool:
	for segment_variant in fixture["segments"]:
		var segment: Dictionary = segment_variant
		for field in ["start_offset", "end_offset", "control_offset"]:
			if not segment.has(field):
				continue
			var point: Vector2 = anchor + segment[field]
			if absf(point.x) > half_width or absf(point.y) > half_depth:
				return false
	return true

func _anchor_supports_fixture(anchor: Vector2, fixture: Dictionary) -> bool:
	for segment_variant in fixture["segments"]:
		var segment: Dictionary = segment_variant
		var points := _segment_surface_points(anchor, segment)
		var validation_variant = simulation_node.validate_road_candidate_with_snap(
			points,
			int(segment["fwd_lanes"]),
			int(segment["bkw_lanes"]),
			true
		)
		if not validation_variant is Dictionary:
			return false
		if not bool(validation_variant.get("is_valid", false)):
			return false
	return true

func _segment_surface_points(anchor: Vector2, segment: Dictionary) -> PackedVector3Array:
	var start_pos := _surface_point(anchor + segment["start_offset"])
	var end_pos := _surface_point(anchor + segment["end_offset"])
	var curve := Curve3D.new()
	curve.bake_interval = 0.5
	curve.up_vector_enabled = false
	if int(segment["draw_mode"]) == 0:
		curve.add_point(start_pos)
		curve.add_point(end_pos)
	else:
		var control_pos := _surface_point(anchor + segment["control_offset"])
		var start_tangent := control_pos - start_pos
		var end_tangent := end_pos - control_pos
		curve.add_point(start_pos, Vector3.ZERO, start_tangent)
		curve.add_point(end_pos, -end_tangent, Vector3.ZERO)
	var previous_draw_mode: int = road_tool.draw_mode
	road_tool.draw_mode = int(segment["draw_mode"])
	var points: PackedVector3Array = road_tool._road_surface_points_from_curve(curve)
	road_tool.draw_mode = previous_draw_mode
	return points

func _run_fixture(
	fixture_definition: Dictionary,
	repetition: int,
	is_warmup: bool,
	anchor: Vector2
) -> Dictionary:
	var identity := {
		"case_id": fixture_definition["case_id"],
		"topology": fixture_definition["topology"],
		"complexity_axis": fixture_definition["complexity_axis"],
		"baseline_case": fixture_definition["baseline_case"],
		"anchor_alignment": fixture_definition["anchor_alignment"],
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
	var segments: Array = fixture_definition["segments"]
	for segment_index in range(segments.size()):
		var segment_result: Dictionary = await _run_segment(
			fixture_definition,
			repetition,
			is_warmup,
			segment_index,
			anchor,
			segments[segment_index]
		)
		fixture.segments.append(segment_result)
		if not bool(segment_result.get("ok", false)):
			fixture["ok"] = false
			fixture["error"] = "segment %d failed" % segment_index
			fixture["total_ms"] = _elapsed_ms(fixture_start_us)
			_phase_end(fixture_phase, {"ok": false, "error": fixture.error})
			return fixture

	var junction_results: Array[Dictionary] = []
	var junctions_match := true
	for junction_variant in fixture_definition["junctions"]:
		var junction: Dictionary = junction_variant
		var junction_offset: Vector2 = junction["offset"]
		var junction_point := _surface_point(anchor + junction_offset)
		var junction_id: int = simulation_node.get_closest_node(junction_point, 8.0)
		var actual_degree := (
			int(simulation_node.get_node_connection_count(junction_id)) if junction_id >= 0 else -1
		)
		var expected_degree: int = junction["expected_degree"]
		var junction_matches := junction_id >= 0 and actual_degree == expected_degree
		junction_results.append({
			"offset": [junction_offset.x, junction_offset.y],
			"node_id": junction_id,
			"degree": actual_degree,
			"expected_degree": expected_degree,
			"ok": junction_matches,
		})
		junctions_match = junctions_match and junction_matches
	fixture["junctions"] = junction_results
	fixture["total_ms"] = _elapsed_ms(fixture_start_us)
	fixture["ok"] = junctions_match
	if not fixture.ok:
		fixture["error"] = "one or more junction degree checks failed"
	_phase_end(
		fixture_phase,
		{
			"ok": fixture.ok,
			"total_ms": fixture.total_ms,
			"junctions": junction_results,
		}
	)
	return fixture

func _run_segment(
	fixture_definition: Dictionary,
	repetition: int,
	is_warmup: bool,
	segment_index: int,
	anchor: Vector2,
	segment: Dictionary
) -> Dictionary:
	var start_xz: Vector2 = anchor + segment["start_offset"]
	var end_xz: Vector2 = anchor + segment["end_offset"]
	var identity := {
		"case_id": fixture_definition["case_id"],
		"topology": fixture_definition["topology"],
		"repetition": repetition,
		"warmup": is_warmup,
		"segment": segment_index,
	}
	var start_pos := _surface_point(start_xz)
	var end_pos := _surface_point(end_xz)
	var prepared_points := _segment_surface_points(anchor, segment)
	var draw_mode: int = segment["draw_mode"]
	var fwd_lanes: int = segment["fwd_lanes"]
	var bkw_lanes: int = segment["bkw_lanes"]
	var preview_phase := _phase_begin("road_preview", identity)
	var preview_start_us := Time.get_ticks_usec()
	_begin_scripted_road(anchor, segment, start_pos, end_pos)
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
		"draw_mode": "spline" if draw_mode == 1 else "straight",
		"fwd_lanes": fwd_lanes,
		"bkw_lanes": bkw_lanes,
		"surface_point_count": prepared_points.size(),
		"surface_length_m": _polyline_length(prepared_points),
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

func _begin_scripted_road(
	anchor: Vector2,
	segment: Dictionary,
	start_pos: Vector3,
	end_pos: Vector3
) -> void:
	road_tool.cancel_road()
	road_tool.active = true
	road_tool.draw_mode = int(segment["draw_mode"])
	road_tool.fwd_lanes = int(segment["fwd_lanes"])
	road_tool.bkw_lanes = int(segment["bkw_lanes"])
	road_tool.start_pos = start_pos
	road_tool.control_pos = (
		_surface_point(anchor + segment["control_offset"])
		if segment.has("control_offset")
		else start_pos
	)
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

func _surface_point(position: Vector2) -> Vector3:
	return Vector3(
		position.x,
		float(simulation_node.get_world_surface_height(position)),
		position.y
	)

func _build_summary(fixture_definitions: Array[Dictionary]) -> Dictionary:
	var cases := {}
	for fixture_definition in fixture_definitions:
		var case_id: String = fixture_definition["case_id"]
		var preview_values: Array[float] = []
		var commit_values: Array[float] = []
		var total_values: Array[float] = []
		for fixture_variant in _metrics.fixtures:
			var fixture: Dictionary = fixture_variant
			if bool(fixture.get("warmup", false)) or fixture.get("case_id", "") != case_id:
				continue
			total_values.append(float(fixture.get("total_ms", 0.0)))
			for segment_variant in fixture.get("segments", []):
				var segment: Dictionary = segment_variant
				preview_values.append(float(segment.get("preview_ms", 0.0)))
				commit_values.append(float(segment.get("commit_ms", 0.0)))
		cases[case_id] = {
			"topology": fixture_definition["topology"],
			"complexity_axis": fixture_definition["complexity_axis"],
			"baseline_case": fixture_definition["baseline_case"],
			"fixture_count": total_values.size(),
			"preview_ms": _distribution(preview_values),
			"commit_ms": _distribution(commit_values),
			"fixture_total_ms": _distribution(total_values),
		}

	var comparisons := {}
	for fixture_definition in fixture_definitions:
		var case_id: String = fixture_definition["case_id"]
		var baseline_case: String = fixture_definition["baseline_case"]
		if baseline_case.is_empty() or not cases.has(baseline_case):
			continue
		var current: Dictionary = cases[case_id]
		var baseline: Dictionary = cases[baseline_case]
		comparisons[case_id] = {
			"baseline_case": baseline_case,
			"preview_p50_ratio": _distribution_ratio(current, baseline, "preview_ms"),
			"commit_p50_ratio": _distribution_ratio(current, baseline, "commit_ms"),
			"fixture_total_p50_ratio": _distribution_ratio(
				current,
				baseline,
				"fixture_total_ms"
			),
		}
	return {"cases": cases, "comparisons": comparisons}

func _distribution_ratio(current: Dictionary, baseline: Dictionary, field: String) -> float:
	var baseline_p50 := float((baseline[field] as Dictionary).get("p50", 0.0))
	if baseline_p50 <= 0.0:
		return 0.0
	return float((current[field] as Dictionary).get("p50", 0.0)) / baseline_p50

func _fixture_descriptors(fixture_definitions: Array[Dictionary]) -> Array[Dictionary]:
	var descriptors: Array[Dictionary] = []
	for fixture in fixture_definitions:
		var segments: Array[Dictionary] = []
		for segment_variant in fixture["segments"]:
			var segment: Dictionary = segment_variant
			var descriptor := {
				"draw_mode": "spline" if int(segment["draw_mode"]) == 1 else "straight",
				"start_offset": _vector2_array(segment["start_offset"]),
				"end_offset": _vector2_array(segment["end_offset"]),
				"fwd_lanes": segment["fwd_lanes"],
				"bkw_lanes": segment["bkw_lanes"],
			}
			if segment.has("control_offset"):
				descriptor["control_offset"] = _vector2_array(segment["control_offset"])
			segments.append(descriptor)
		var junctions: Array[Dictionary] = []
		for junction_variant in fixture["junctions"]:
			var junction: Dictionary = junction_variant
			junctions.append({
				"offset": _vector2_array(junction["offset"]),
				"expected_degree": junction["expected_degree"],
			})
		descriptors.append({
			"case_id": fixture["case_id"],
			"topology": fixture["topology"],
			"complexity_axis": fixture["complexity_axis"],
			"baseline_case": fixture["baseline_case"],
			"anchor_alignment": fixture["anchor_alignment"],
			"segments": segments,
			"junctions": junctions,
		})
	return descriptors

func _vector2_array(value: Vector2) -> Array[float]:
	return [value.x, value.y]

func _polyline_length(points: PackedVector3Array) -> float:
	var length_m := 0.0
	for index in range(1, points.size()):
		length_m += points[index - 1].distance_to(points[index])
	return length_m

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
