# SPDX-License-Identifier: GPL-2.0-only

## Frames between a committed road becoming renderable and its cell grid reaching the overlay.
## Run explicitly; METRUM_ZONE_ROAD_LATENCY_OUT optionally names a JSON results file.
## The production cell overlay processes on natural capped frames; setup is not timed.
extends "res://tests/zoning_overlay_benchmark.gd"

const FRAME_CAP := 60
const REPETITIONS := 3
const TIMEOUT_FRAMES := 600
# Short roads around the world-zero chunk corner: inside one chunk, across one boundary,
# and across both, so each commit dirties one to four warm cell chunks.
const ROADS := [
	[Vector2(120, 160), Vector2(300, 160)],
	[Vector2(-90, 300), Vector2(90, 300)],
	[Vector2(-300, -140), Vector2(-120, -140)],
	[Vector2(40, -90), Vector2(40, 90)],
	[Vector2(-60, -60), Vector2(60, 60)],
	[Vector2(160, -300), Vector2(160, -120)],
	[Vector2(-300, 120), Vector2(-150, 250)],
	[Vector2(-40, 420), Vector2(-40, 280)],
]

var _frame := 0
var _frame_us := 0

func _run() -> void:
	Engine.max_fps = FRAME_CAP
	var rows: Array[Dictionary] = []
	for speed: float in [0.0, 1.0]:
		var samples: Array[Dictionary] = []
		for repetition in range(REPETITIONS):
			samples.append_array(await _scenario(speed))
		var row := {"speed": speed, "samples": samples.size(), "road_frames": _latency_summary(samples.map(func(s): return s.road_frames)),
			"road_ms": _latency_summary(samples.map(func(s): return s.road_ms)),
			"cell_lag_frames": _latency_summary(samples.map(func(s): return s.cell_lag_frames)),
			"cell_seen_ms": _latency_summary(samples.map(func(s): return s.cell_seen_ms)),
			"changed_chunks": _latency_summary(samples.map(func(s): return s.changed_chunks)),
			"upload_frames": _latency_summary(samples.map(func(s): return s.upload_frames))}
		rows.append(row)
		print("ZONE_ROAD_LATENCY ", JSON.stringify(row))
	var output := OS.get_environment("METRUM_ZONE_ROAD_LATENCY_OUT")
	if not output.is_empty():
		var file := FileAccess.open(output, FileAccess.WRITE)
		_expect(file != null, "Latency results must open")
		if file != null:
			file.store_string(JSON.stringify({"frame_cap": FRAME_CAP, "rows": rows, "failures": _failures}, "\t"))
	print("zoning_road_commit_latency: %s" % ("PASS" if _failures == 0 else "FAIL"))
	quit(0 if _failures == 0 else 1)

func _scenario(speed: float) -> Array[Dictionary]:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	var samples: Array[Dictionary] = []
	_expect(simulation.create_blank_world(4096.0, 4096.0, 8.0, 512.0, 0.0), "Latency world must load")
	simulation.set_simulation_speed(speed)
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.position = Vector3(0, 750, 170)
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 900.0
	camera.far = 2500.0
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var overlay := OverlayScript.new()
	scene.add_child(overlay)
	overlay.set_tool_active(true)
	await _settle_cell_overlay(overlay)
	for points: Array in ROADS:
		var sample := await _measure_commit(overlay._cell_chunks, _path(points))
		if sample.is_empty():
			break
		samples.append(sample)
	scene.queue_free()
	await process_frame
	return samples

func _measure_commit(cells: Node, points: PackedVector3Array) -> Dictionary:
	# Exact preview first, as the production tool does; its latency is not measured here.
	var request := simulation.request_preview_road_surface_with_snap(points, 1, 1, true)
	var preview: Variant = null
	for attempt in range(TIMEOUT_FRAMES):
		preview = simulation.get_preview_road_surface_result(request, 0, PackedInt64Array())
		if preview != null:
			break
		await process_frame
	_expect(preview is Dictionary and preview.get("is_valid", false), "Latency road preview must be valid: %s" % points)
	if not (preview is Dictionary and preview.get("is_valid", false)):
		return {}
	await _tick()
	var generation := int(simulation.get_network_render_generation())
	var versions: Dictionary = cells._versions.duplicate(true)
	simulation.add_road_with_snap(points, 1, 1, true)
	var submit_frame := _frame
	var submit_us := _frame_us
	var road_frame := -1
	var road_us := 0
	var last_change_frame := -1
	var last_change_us := 0
	var upload_frames := 0
	var changed := {}
	var settled := false
	for attempt in range(TIMEOUT_FRAMES):
		await _tick()
		# A start-of-frame observation reflects the overlay's processing in the previous frame;
		# a newly published road generation is drawn by the renderer during this frame.
		if road_frame < 0 and int(simulation.get_network_render_generation()) != generation:
			road_frame = _frame
			road_us = _frame_us
		var current: Dictionary = cells._versions
		var frame_changed := false
		for key: Vector2i in current:
			if versions.get(key) != current[key]:
				changed[key] = true
				frame_changed = true
		if frame_changed:
			upload_frames += 1
			last_change_frame = _frame - 1
			last_change_us = _frame_us
			versions = current.duplicate(true)
		if road_frame >= 0 and _versions_match(cells):
			settled = true
			break
	_expect(settled, "Road and cells must settle: %s" % points)
	_expect(not changed.is_empty(), "A committed road must change at least one visible cell chunk")
	return {"road_frames": road_frame - submit_frame, "road_ms": (road_us - submit_us) / 1000.0,
		"cell_lag_frames": last_change_frame - road_frame, "cell_seen_ms": (last_change_us - road_us) / 1000.0,
		"changed_chunks": changed.size(), "upload_frames": upload_frames}

func _tick() -> void:
	await process_frame
	_frame += 1
	_frame_us = Time.get_ticks_usec()

func _latency_summary(values: Array) -> Dictionary:
	values.sort()
	if values.is_empty():
		return {}
	return {"min": values[0], "p50": values[values.size() / 2], "p95": values[int(values.size() * 0.95)], "max": values[-1]}
