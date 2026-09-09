# SPDX-License-Identifier: GPL-2.0-only

## Continuous-motion regression and latency capture through the production RoadTool process path.
## Uses fixed world-space input, not OS raycasts or GPU presentation. No midpoint idle hold.
extends "res://tests/road_junction_preview_test.gd"

const Metrics := preload("res://scripts/benchmarks/road_benchmark_metrics.gd")
const MOTION_FRAMES := 96
const INPUT_INTERVAL_SEC := 1.0 / 60.0

func _run() -> void:
	simulation = SimulationNode.new()
	root.add_child(simulation)
	var fixtures := []
	for fixture in [
		{"name": "t", "end_z": 0.0, "forward": 1},
		{"name": "cross", "end_z": 48.0, "forward": 1},
		{"name": "wide_t", "end_z": 0.0, "forward": 2},
		{"name": "bend", "end_z": -48.0, "forward": 1, "endpoint_join": true},
		{"name": "wide_bend", "end_z": -48.0, "forward": 2, "endpoint_join": true},
	]:
		fixtures.append(await _stream_fixture(fixture))
	var capture := {
		"schema_version": 1, "success": _failures == 0, "fixtures": fixtures,
		"input_frames": MOTION_FRAMES, "input_interval_sec": INPUT_INTERVAL_SEC,
		"binary_sha256": FileAccess.get_sha256("res://bin/libmetrum_rise.so"),
		"tool_sha256": FileAccess.get_sha256("res://scripts/tools/road_tool.gd"),
		"harness_sha256": FileAccess.get_sha256(get_script().resource_path),
		"godot": Engine.get_version_info(), "cpu": OS.get_processor_name(),
		"rayon_num_threads": OS.get_environment("RAYON_NUM_THREADS"),
	}
	var capture_path := OS.get_environment("METRUM_PREVIEW_STREAM_METRICS")
	if not capture_path.is_empty():
		var file := FileAccess.open(capture_path, FileAccess.WRITE)
		_expect(file != null, "stream capture must be writable")
		if file != null:
			file.store_string(JSON.stringify(capture, "\t"))
	simulation.free()
	if _failures == 0:
		print("road_preview_stream_test: PASS")
	quit(0 if _failures == 0 else 1)

func _stream_fixture(fixture: Dictionary) -> Dictionary:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "stream world must load")
	simulation.set_simulation_speed(0.0)
	var endpoint_join: bool = fixture.get("endpoint_join", false)
	if not await _commit(PackedVector3Array([Vector3(-60.0, 0.0, 0.0), Vector3(0.0 if endpoint_join else 60.0, 0.0, 0.0)])):
		return {"case": fixture.name, "success": false}
	if not await _commit(PackedVector3Array([Vector3(24.0, 0.0, 72.0), Vector3(60.0, 0.0, 72.0)])):
		return {"case": fixture.name, "success": false}
	var tool := RoadToolScript.new()
	tool.name = "RoadTool"
	tool.simulation_node = simulation
	tool.road_mesh_root = Node3D.new()
	tool.add_child(tool.road_mesh_root)
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._road_preview_material = WorldMaterialsScript.road_preview_material()
	tool.blueprint_mesh.material_override = tool._road_preview_material
	# Manually drive the real process path at a fixed input cadence, without debug/node overlays.
	tool._ghost_enabled = false
	tool.active = true
	tool.fwd_lanes = fixture.forward
	tool.start_pos = Vector3.ZERO if endpoint_join else Vector3(0.0, 0.0, -48.0)
	tool.current_state = RoadToolScript.State.SETTING_END
	tool.current_path = Path3D.new()
	tool.current_path.curve = Curve3D.new()
	tool.current_path.curve.bake_interval = 0.5
	tool.add_child(tool.current_path)
	var generation := simulation.get_network_render_generation()
	_expect(tool.update_main_mesh(generation) == generation, "stream source meshes must hydrate")
	tool._node_visuals_dirty = false
	tool._node_visuals_visible = true
	var before := simulation.get_road_benchmark_state()
	var request_inputs := {}
	var samples := []
	var update_latencies := []
	var update_gaps := []
	var pointer_ages := []
	var process_times := []
	var started_us := Time.get_ticks_usec()
	var last_display_us := started_us
	var displayed_id := 0
	var first_display_ms := -1.0
	var final_points := PackedVector3Array()
	for index in range(MOTION_FRAMES):
		# Slide the endpoint along/across the trunk and reverse without any stationary samples.
		var phase := index % 48
		var x := -16.0 + float(phase if phase < 24 else 47 - phase) * (32.0 / 23.0)
		# Offset the return leg by half a step so turning points also move.
		if phase >= 24:
			x -= 16.0 / 23.0
		var point := Vector3(x, 0.0, fixture.end_z)
		final_points = PackedVector3Array([tool.start_pos, point])
		tool.set_scripted_pointer(true, point)
		tool._queue_preview_update()
		var input_us := Time.get_ticks_usec()
		tool._process(INPUT_INTERVAL_SEC)
		var displayed_us := Time.get_ticks_usec()
		process_times.append(float(displayed_us - input_us) / 1000.0)
		var requested_id: int = tool._preview_request_id
		if requested_id > 0 and not request_inputs.has(requested_id):
			request_inputs[requested_id] = input_us
		var shown_id: int = tool._junction_preview.request_id
		_expect(shown_id > 0 or displayed_id == 0, "valid continuous motion must retain the junction: fixture=%s frame=%d valid=%s" % [fixture.name, index, tool.is_valid])
		if shown_id > 0:
			_expect(request_inputs.has(shown_id), "rendered request must come from this drag")
			var age_ms := float(displayed_us - int(request_inputs.get(shown_id, displayed_us))) / 1000.0
			pointer_ages.append(age_ms)
			if shown_id != displayed_id:
				_expect(shown_id > displayed_id, "displayed inputs must advance monotonically")
				update_latencies.append(age_ms)
				if displayed_id > 0:
					update_gaps.append(float(displayed_us - last_display_us) / 1000.0)
				else:
					first_display_ms = float(displayed_us - started_us) / 1000.0
				last_display_us = displayed_us
				displayed_id = shown_id
		# An older rendered pose must not become exact validation for the current pointer.
		var exact: Dictionary = tool._cached_preview_surface_for_points(final_points)
		_expect(exact.is_empty() or tool._preview_cache_points == final_points, "click cache must match exact current input")
		samples.append({"frame": index, "input_us": input_us, "display_us": displayed_us, "request_id": requested_id, "displayed_id": shown_id})
		# Optional rendered pose diagnostics deliberately stall this trace; exclude them from timings.
		if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty() and index in [16, 24, 40]:
			await _capture(tool.road_mesh_root, fixture.name + "_motion_" + str(index))
		await create_timer(INPUT_INTERVAL_SEC).timeout
	var motion_ms := float(Time.get_ticks_usec() - started_us) / 1000.0
	# Settling must catch up to the final input; movement is not allowed to starve compilation.
	var deadline := Time.get_ticks_msec() + 20000
	while Time.get_ticks_msec() < deadline:
		tool._process(INPUT_INTERVAL_SEC)
		if not tool._cached_preview_surface_for_points(final_points).is_empty() and not tool._preview_result_pending:
			break
		await create_timer(INPUT_INTERVAL_SEC).timeout
	_expect(not tool._cached_preview_surface_for_points(final_points).is_empty(), "final pointer must receive exact validation")
	_expect(update_latencies.size() >= 4, "junction must update repeatedly before the pointer stops")
	var after := simulation.get_road_benchmark_state()
	for field in ["generation", "live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings"]:
		_expect(before[field] == after[field], "stream must not mutate authoritative " + field)
	tool.cancel_road()
	for original in tool._road_chunk_instances.values():
		_expect(original.visible, "stream cancellation must restore originals")
	tool.free()
	return {
		"case": fixture.name, "motion_ms": motion_ms, "first_display_ms": first_display_ms,
		"updates_during_motion": update_latencies.size(), "visible_motion_frames": pointer_ages.size(),
		"input_to_display_ms": Metrics.distribution(update_latencies),
		"displayed_input_age_ms": Metrics.distribution(pointer_ages),
		"update_interval_ms": Metrics.distribution(update_gaps),
		"tool_process_ms": Metrics.distribution(process_times), "samples": samples,
		"state_before": before, "state_after": after,
	}
