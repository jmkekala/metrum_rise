# SPDX-License-Identifier: GPL-3.0-or-later

## Continuous-motion regression and latency capture through the production RoadTool process path
## with the synchronous preview (`ROAD-68-E`): every moved frame previews and shows its own pose.
## Uses fixed world-space input, not OS raycasts or GPU presentation. No midpoint idle hold.
extends "res://tests/road_junction_preview_test.gd"

const Metrics := preload("res://scripts/benchmarks/road_benchmark_metrics.gd")
const MOTION_FRAMES := 96
const INPUT_INTERVAL_SEC := 1.0 / 60.0
var motion_frames: int = MOTION_FRAMES
# Seconds between inputs. A headless run idles through each wait, and on an otherwise idle
# machine the CPU then holds the stages at its lowest clock; METRUM_PREVIEW_STREAM_INTERVAL_SEC=0
# drives the frames back to back, as continuously rendered frames keep the clock up.
var input_interval_sec: float = INPUT_INTERVAL_SEC

func _run() -> void:
	var requested_frames := int(OS.get_environment("METRUM_PREVIEW_STREAM_FRAMES"))
	if requested_frames > 0:
		motion_frames = requested_frames
	var requested_interval := OS.get_environment("METRUM_PREVIEW_STREAM_INTERVAL_SEC")
	if not requested_interval.is_empty():
		input_interval_sec = maxf(float(requested_interval), 0.0)
	simulation = SimulationNode.new()
	root.add_child(simulation)
	var fixtures := []
	for fixture in [
		{"name": "t", "end_z": 0.0, "forward": 1},
		{"name": "control_curve_t", "end_z": 0.0, "forward": 1, "curve": true},
		{"name": "cross", "end_z": 48.0, "forward": 1},
		{"name": "wide_t", "end_z": 0.0, "forward": 2},
		{"name": "bend", "end_z": -48.0, "forward": 1, "endpoint_join": true},
		{"name": "wide_bend", "end_z": -48.0, "forward": 2, "endpoint_join": true},
		{"name": "sloped_t", "end_z": 0.0, "forward": 1, "sloped": true},
		{"name": "chunk_boundary_t", "end_z": 0.0, "forward": 1, "offset_x": 128.0},
		{"name": "dense_t", "end_z": 0.0, "forward": 1, "dense": true},
	]:
		fixtures.append(await _stream_fixture(fixture))
	var capture := {
		"schema_version": 1, "success": _failures == 0, "fixtures": fixtures,
		"input_frames": motion_frames, "input_interval_sec": input_interval_sec,
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
	if fixture.get("sloped", false):
		simulation.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128.0, 0.0), 0.0, Vector2(128.0, 0.0), 6.4, 20.0)
	var endpoint_join: bool = fixture.get("endpoint_join", false)
	if not await _commit(PackedVector3Array([_stream_point(fixture, -60.0, 0.0), _stream_point(fixture, 0.0 if endpoint_join else 60.0, 0.0)])):
		return {"case": fixture.name, "success": false}
	if not await _commit(PackedVector3Array([_stream_point(fixture, 24.0, 72.0), _stream_point(fixture, 60.0, 72.0)])):
		return {"case": fixture.name, "success": false}
	if fixture.get("dense", false):
		for x in [-36.0, 36.0]:
			if not await _commit(PackedVector3Array([_stream_point(fixture, x, 48.0), _stream_point(fixture, x, 0.0)])):
				return {"case": fixture.name, "success": false}
	var tool := RoadToolScript.new()
	tool.name = "RoadTool"
	tool.simulation_node = simulation
	tool.make_chunks()
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	# Manually drive the real process path at a fixed input cadence, without debug/node overlays.
	tool.active = true
	tool.set_lanes(fixture.forward, 1)
	tool.draw_mode = 1 if fixture.get("curve", false) else 0
	var start := _stream_point(fixture, 0.0, 0.0 if endpoint_join else -48.0)
	var control := _stream_point(fixture, 0.0, -24.0)
	tool._begin_stroke(start)
	if tool.draw_mode == 1:
		# The curve's free control point (U1), even where the stroke starts on a road.
		_expect(not tool._commit_segment(control), "a curve's control-point click sends nothing")
	var generation := simulation.get_network_render_generation()
	_expect(tool.chunks.update(generation) == generation, "stream source meshes must hydrate")
	var before := simulation.get_road_benchmark_state()
	var samples := []
	var process_times := []
	var stage_times := {"plan_ms": [], "roadbed_ms": [], "payload_ms": [], "graded_ms": [], "packing_ms": [], "godot_ms": []}
	var started_us := Time.get_ticks_usec()
	var shown_frames := 0
	var final_points := PackedVector3Array()
	for index in range(motion_frames):
		# Slide the endpoint along/across the trunk and reverse without any stationary samples.
		var phase := index % 48
		var x := -16.0 + float(phase if phase < 24 else 47 - phase) * (32.0 / 23.0)
		# Offset the return leg by half a step so turning points also move.
		if phase >= 24:
			x -= 16.0 / 23.0
		var point := _stream_point(fixture, x, fixture.end_z)
		tool.set_scripted_pointer(true, point)
		tool._queue_preview_update()
		var input_us := Time.get_ticks_usec()
		tool._process(INPUT_INTERVAL_SEC)
		var displayed_us := Time.get_ticks_usec()
		var frame_ms := float(displayed_us - input_us) / 1000.0
		process_times.append(frame_ms)
		var preview: Dictionary = tool._preview
		# The frame that sampled the pose previewed it; a valid pose is shown in that frame.
		if tool.draw_mode == 1:
			# The session's simple curve: the exact fillet arc from the start to this frame's pointer
			# whose radius is T / tan(delta / 2) for the legs start -> control -> pointer.
			final_points = tool.simulation_node.road_tool_points()
			_expect(tool.stroke_preview_current() and final_points[0] == start and final_points[-1] == point, "every frame previews its own pose: fixture=%s frame=%d" % [fixture.name, index])
			var leg_in := Vector2(control.x - start.x, control.z - start.z)
			var leg_out := Vector2(point.x - control.x, point.z - control.z)
			var deflection := absf(leg_in.angle_to(leg_out))
			if deflection > 0.001:
				var expected_radius := minf(leg_in.length(), leg_out.length()) / tan(0.5 * deflection)
				_expect(absf(float(tool.simulation_node.road_tool_snapshot()["hud"].get("radius_m", -1.0)) - expected_radius) < 0.01, "the curve is the fillet arc of its clicks: fixture=%s frame=%d" % [fixture.name, index])
		else:
			# The session's straight stroke: the start and this frame's scripted pointer exactly.
			final_points = tool.simulation_node.road_tool_points()
			_expect(final_points.size() == 2 and final_points[1] == point, "every frame previews its own pose: fixture=%s frame=%d" % [fixture.name, index])
		var valid: bool = preview.get("is_valid", false)
		var shown: bool = tool.simulation_node.road_tool_overlay_counts()[1] == int(preview.get("sequence", -1))
		_expect(not valid or shown, "a valid pose is shown in the frame that sampled it: fixture=%s frame=%d" % [fixture.name, index])
		if valid and shown:
			shown_frames += 1
			var native := 0.0
			for stage in ["plan_ms", "roadbed_ms", "payload_ms", "graded_ms", "packing_ms"]:
				var ms := float(preview.get(stage, 0.0))
				stage_times[stage].append(ms)
				native += ms
			# The rest of the frame: the bridge call and the road and terrain staging and install.
			stage_times["godot_ms"].append(maxf(frame_ms - native, 0.0))
		samples.append({"frame": index, "input_us": input_us, "display_us": displayed_us, "sequence": int(preview.get("sequence", 0)), "valid": valid})
		# Optional rendered pose diagnostics deliberately stall this trace; exclude them from timings.
		if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty() and index in [16, 24, 40]:
			await _capture(tool.chunks.chunk_root, fixture.name + "_motion_" + str(index))
		if input_interval_sec > 0.0:
			await create_timer(input_interval_sec).timeout
		else:
			await process_frame
	var motion_ms := float(Time.get_ticks_usec() - started_us) / 1000.0
	_expect(shown_frames >= motion_frames / 2, "the junction must show during motion: fixture=%s shown=%d" % [fixture.name, shown_frames])
	var after := simulation.get_road_benchmark_state()
	for field in ["generation", "live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings"]:
		_expect(before[field] == after[field], "stream must not mutate authoritative " + field)
	tool.cancel_road()
	for original in tool.chunks.chunks.values():
		_expect(original.visible, "stream cancellation must restore originals")
	tool.free()
	var stages := {}
	for stage in stage_times:
		stages[stage] = Metrics.distribution(stage_times[stage])
	return {
		"case": fixture.name, "motion_ms": motion_ms, "shown_motion_frames": shown_frames,
		"tool_process_ms": Metrics.distribution(process_times), "stage_ms": stages, "samples": samples,
		"state_before": before, "state_after": after,
	}

func _stream_point(fixture: Dictionary, x: float, z: float) -> Vector3:
	x += float(fixture.get("offset_x", 0.0))
	return _ground_point(x, z) if fixture.get("sloped", false) else Vector3(x, 0.0, z)
