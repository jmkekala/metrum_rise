# SPDX-License-Identifier: GPL-2.0-only

## Native terrain/water diagnostics must describe the bytes uploaded by each renderer.
extends SceneTree

const TerrainRenderer := preload("res://scripts/renderers/terrain.gd")
const RenderDebug := preload("res://scripts/renderers/render_debug.gd")
const WaterRenderer := preload("res://scripts/renderers/water.gd")
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	_check_geometry_debug()
	var simulation := SimulationNode.new()
	root.add_child(simulation)
	_expect(simulation.create_blank_world(80.0, 80.0, 10.0, 40.0, 0.0), "water fixture must create a blank world")
	simulation.begin_world_open_water_fill_preview(Vector2.ZERO, 5.0)
	_expect(simulation.commit_world_open_water_fill_preview(), "water fixture must commit the authored fill")
	var key := Vector2i.ZERO
	simulation.request_water_patch_payloads(PackedInt32Array([key.x, key.y]))
	var payload: Dictionary = {}
	var deadline := Time.get_ticks_msec() + 5000
	while payload.is_empty() and Time.get_ticks_msec() < deadline:
		var result: Dictionary = simulation.poll_ready_water_patch_payloads(1)
		var ready: Array = result.get("patches", [])
		if not ready.is_empty():
			payload = ready[0]
		else:
			# Payload preparation may yield to an active simulation edit; the renderer retries.
			if int(result.get("failed_count", 0)) > 0 or int(result.get("stale_ready_count", 0)) > 0:
				simulation.request_water_patch_payloads(PackedInt32Array([key.x, key.y]))
			await process_frame
	_expect(not payload.is_empty(), "native water payload must arrive")
	if not payload.is_empty():
		var renderer := WaterRenderer.new()
		renderer.simulation_node = simulation
		var expected_count := int(payload["texture_width"]) * int(payload["texture_height"])
		var stats: Dictionary = renderer._water_patch_depth_stats(payload)
		_expect(int(stats["nonzero"]) == expected_count, "depth diagnostics must count the native byte payload")
		_expect(is_equal_approx(float(stats["min"]), 5.0) and is_equal_approx(float(stats["max"]), 5.0), "depth diagnostics must preserve authored depth")
		_expect(is_equal_approx(float(stats["sum"]), expected_count * 5.0), "depth diagnostics must sum the native byte payload")
		_expect(renderer._water_patch_depth_sample_count(payload) == expected_count, "depth sample count must match the uploaded payload")
		var baseline: Dictionary = {}
		deadline = Time.get_ticks_msec() + 5000
		while baseline.is_empty() and Time.get_ticks_msec() < deadline:
			baseline = simulation.get_water_patch_debug(key.x, key.y)
			if baseline.is_empty():
				await process_frame
		_expect(int(baseline.get("baseline_nonzero", -1)) == expected_count, "live baseline must match the captured water depth")
		_expect(not baseline.has("visible_nonzero"), "one authored water layer must have one set of source statistics")
		renderer.patches[key] = {"last_patch_data": payload}
		var text := ""
		deadline = Time.get_ticks_msec() + 5000
		while Time.get_ticks_msec() < deadline:
			text = "\n".join(renderer.road_geometry_debug_patch_lines(PackedInt32Array([key.x, key.y])))
			if not text.contains("baseline_nonzero=-1/"):
				break
			await process_frame
		_expect(text.contains("depth_nonzero=%d/%d" % [expected_count, expected_count]), "formatted water diagnostics must report the uploaded depth")
		_expect(text.contains("baseline_nonzero=%d/%d" % [expected_count, expected_count]), "formatted water diagnostics must retain the current source comparison")
		renderer.free()
	await _check_terrain_payload(simulation)
	simulation.queue_free()
	await process_frame
	print("Surface patch debug tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

func _check_terrain_payload(simulation: SimulationNode) -> void:
	_expect(simulation.create_blank_world(80.0, 80.0, 10.0, 40.0, -2.5), "terrain fixture must create a nonzero world")
	for render_step_mm in [0, 10000]:
		var request := PackedInt32Array([0, 0, render_step_mm])
		simulation.request_terrain_patch_payloads(request)
		var payload: Dictionary = {}
		var deadline := Time.get_ticks_msec() + 5000
		while payload.is_empty() and Time.get_ticks_msec() < deadline:
			var result: Dictionary = simulation.poll_ready_terrain_patch_payloads(1)
			var ready: Array = result.get("patches", [])
			if not ready.is_empty():
				payload = ready[0]
			else:
				if not (result.get("retry_requests", PackedInt64Array()) as PackedInt64Array).is_empty():
					simulation.request_terrain_patch_payloads(request)
				await process_frame
		_expect(not payload.is_empty(), "native terrain payload must arrive")
		if payload.is_empty():
			continue
		var renderer := TerrainRenderer.new()
		var count := int(payload["texture_width"]) * int(payload["texture_height"])
		var stats: Dictionary = renderer._terrain_patch_height_stats(payload)
		_expect(int(stats["nonzero"]) == count, "terrain statistics must count the native byte payload")
		_expect(is_equal_approx(float(stats["min"]), -2.5) and is_equal_approx(float(stats["max"]), -2.5), "terrain diagnostics must preserve signed height")
		_expect(is_equal_approx(float(stats["sum"]), count * -2.5), "terrain diagnostics must sum the uploaded samples")
		_expect(not payload.has("height_data"), "terrain exports must have one authoritative height buffer")
		renderer.patches[Vector2i.ZERO] = {"last_patch_data": payload}
		var lines: Array[String] = renderer.road_geometry_debug_patch_lines(PackedInt32Array([0, 0]))
		_expect("\n".join(lines).contains("height_min=-2.500"), "formatted terrain diagnostics must report native heights")
		renderer.free()

func _check_geometry_debug() -> void:
	var terrain := TerrainRenderer.new()
	var water := WaterRenderer.new()
	var payload := {
		"road_clip_loop_counts": PackedInt32Array([4, 4, 4]),
		"road_clip_loop_groups": PackedInt32Array([9, 9, 2]),
		"road_clip_loop_roles": PackedInt32Array([TerrainRenderer.ROAD_CLIP_LOOP_ROLE_OUTER, TerrainRenderer.ROAD_CLIP_LOOP_ROLE_HOLE, TerrainRenderer.ROAD_CLIP_LOOP_ROLE_OUTER]),
		"road_clip_loop_points": PackedVector3Array([
			Vector3(0, 0, 0), Vector3(4, 0, 0), Vector3(4, 0, 3), Vector3(0, 0, 3),
			Vector3(1, 0, 1), Vector3(2, 0, 1), Vector3(2, 0, 2), Vector3(1, 0, 2),
			Vector3(-5, 0, -5), Vector3(-3, 0, -5), Vector3(-3, 0, -4), Vector3(-5, 0, -4),
		]),
	}
	for renderer in [terrain, water]:
		var stats: Dictionary = RenderDebug.clip_stats(renderer._road_clip_loop_groups_from_patch_data(payload))
		_expect(int(stats["group_count"]) == 2 and int(stats["loop_count"]) == 3 and int(stats["point_count"]) == 12, "clip diagnostics must preserve group/hole counts")
		_expect(is_equal_approx(float(stats["area"]), 13.0), "clip diagnostics must subtract hole area")
		_expect(is_equal_approx(float(stats["max_bbox_x"]), 4.0) and is_equal_approx(float(stats["max_bbox_z"]), 3.0), "clip diagnostics must retain largest group dimensions")
		_expect(RenderDebug.bounds_label(stats) == "[(-5.000,-5.000)..(4.000,3.000)]", "clip diagnostics must merge group bounds")
		_expect(RenderDebug.bounds_label(RenderDebug.clip_stats(renderer._road_clip_loop_groups_from_patch_data({}))) == "none", "absent clipping must have no bounds")
	var mesh := ArrayMesh.new()
	for count in [3, 6]:
		var vertices := PackedVector3Array()
		for index in count:
			vertices.append(Vector3(index % 3, 0, (index + 1) % 3))
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = vertices
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	_expect(RenderDebug.mesh_label(mesh) == "ArrayMesh surfaces=2 vertices=9", "mesh diagnostics must count all surfaces")
	_expect(RenderDebug.mesh_label(null) == "null", "absent mesh diagnostics must remain explicit")
	_expect(RenderDebug.mesh_label(BoxMesh.new()) == "BoxMesh", "primitive meshes must retain their class label")
	terrain.free()
	water.free()
