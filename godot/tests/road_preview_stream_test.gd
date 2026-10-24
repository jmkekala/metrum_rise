# SPDX-License-Identifier: GPL-2.0-only

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
		simulation.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128.0, 0.0), 0.0, Vector2(128.0, 0.0), 6.4, 1.0)
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
	tool.road_mesh_root = Node3D.new()
	tool.add_child(tool.road_mesh_root)
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._road_preview_material = WorldMaterialsScript.road_preview_material()
	tool.blueprint_mesh.material_override = tool._road_preview_material
	# Manually drive the real process path at a fixed input cadence, without debug/node overlays.
	tool.active = true
	tool.fwd_lanes = fixture.forward
	tool.draw_mode = 1 if fixture.get("curve", false) else 0
	tool._begin_stroke(_stream_point(fixture, 0.0, 0.0 if endpoint_join else -48.0), 1, Vector3.RIGHT)
	if tool.draw_mode == 1:
		_expect(tool.current_state == RoadToolScript.State.SETTING_CONTROL, "curve stream must begin with a free control point despite its road direction")
		tool.control_pos = _stream_point(fixture, 0.0, -24.0)
		tool.current_state = RoadToolScript.State.SETTING_END
	var generation := simulation.get_network_render_generation()
	_expect(tool.update_main_mesh(generation) == generation, "stream source meshes must hydrate")
	tool._node_visuals_dirty = false
	tool._node_visuals_visible = true
	var before := simulation.get_road_benchmark_state()
	var samples := []
	var process_times := []
	var stage_times := {"plan_ms": [], "roadbed_ms": [], "payload_ms": [], "terrain_ms": [], "packing_ms": [], "godot_ms": []}
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
		final_points = tool._road_surface_points_from_curve(tool.current_path.curve)
		if tool.draw_mode == 1:
			_expect(tool.current_path.curve.get_point_out(0) == tool.control_pos - tool.start_pos, "moving curve preview must retain the freely chosen control handle")
			_expect(tool.current_path.curve.get_point_in(1) == tool.control_pos - point, "moving curve preview must follow the control point and current endpoint")
		var preview: Dictionary = tool._preview
		# The frame that sampled the pose previewed it; a valid pose is shown in that frame.
		_expect(tool._preview_is_for(final_points), "every frame previews its own pose: fixture=%s frame=%d" % [fixture.name, index])
		var valid: bool = preview.get("is_valid", false)
		var shown: bool = tool._preview_display.sequence == int(preview.get("sequence", -1))
		_expect(not valid or shown, "a valid pose is shown in the frame that sampled it: fixture=%s frame=%d" % [fixture.name, index])
		if valid and shown:
			shown_frames += 1
			var native := 0.0
			for stage in ["plan_ms", "roadbed_ms", "payload_ms", "terrain_ms", "packing_ms"]:
				var ms := float(preview.get(stage, 0.0))
				stage_times[stage].append(ms)
				native += ms
			# The rest of the frame: the bridge call and the road and terrain staging and install.
			stage_times["godot_ms"].append(maxf(frame_ms - native, 0.0))
		samples.append({"frame": index, "input_us": input_us, "display_us": displayed_us, "sequence": int(preview.get("sequence", 0)), "valid": valid})
		# Optional rendered pose diagnostics deliberately stall this trace; exclude them from timings.
		if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty() and index in [16, 24, 40]:
			await _capture(tool.road_mesh_root, fixture.name + "_motion_" + str(index))
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
	for original in tool._road_chunk_instances.values():
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
