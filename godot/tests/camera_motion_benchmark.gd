# SPDX-License-Identifier: GPL-2.0-only

## Isolated headless camera command/update benchmark, with setup outside timed batches.
## METRUM_CAMERA_BENCH_OUTPUT selects the JSON artifact. Run identical fixtures against
## same-session release control/candidate libraries. No rendering or per-frame trace capture.
extends SceneTree

const BATCHES := 21
const FRAMES := 10000
const DELTA := 1.0 / 60.0

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	if not simulation.create_blank_world(2048, 2048, 8, 128, 50):
		quit(1)
		return
	simulation.set_simulation_speed(0.0)
	var deadline := Time.get_ticks_msec() + 2000
	while float(simulation.get_road_benchmark_state()["simulation_speed"]) != 0.0:
		if Time.get_ticks_msec() > deadline:
			push_error("Camera benchmark simulation did not pause")
			quit(1)
			return
		await process_frame
	# Dynamic dispatch permits comparing the old immediate API to the new frame API.
	var camera = ClassDB.instantiate("CameraNode")
	camera.name = "CameraNode"
	host.add_child(camera)
	camera.configure_world_camera()
	if camera.has_method("set_world_bounds_enabled"):
		camera.set_world_bounds_enabled(true)
	camera.set_debug_under_terrain_enabled(false)
	camera.set_process(false)
	var smooth: bool = camera.has_method("advance_motion")
	var results := {"smooth": smooth, "batches": BATCHES, "frames_per_batch": FRAMES, "cases": {}}
	for workload: String in ["idle", "pan", "zoom", "pan_zoom"]:
		var samples: Array[float] = []
		for batch in BATCHES + 1:
			camera.focus_on(Vector3(0, 1000, 0), 160.0)
			var begin := Time.get_ticks_usec()
			for frame in FRAMES:
				var direction := Vector3.RIGHT if (frame / 128) % 2 == 0 else Vector3.LEFT
				if workload == "pan" or workload == "pan_zoom":
					if smooth:
						camera.pan(direction, 1.0)
					else:
						camera.pan(direction, 1.0, DELTA)
				if (workload == "zoom" or workload == "pan_zoom") and frame % 8 == 0:
					camera.zoom(1.0 if (frame / 8) % 2 == 0 else -1.0)
				if smooth:
					camera.advance_motion(DELTA)
				elif workload == "idle":
					# Baseline no-op native dispatch; isolates the added idle update cost.
					camera.pan(Vector3.ZERO, 1.0, DELTA)
			var elapsed := float(Time.get_ticks_usec() - begin) / FRAMES
			if batch > 0:
				samples.append(elapsed)
		var ordered := samples.duplicate()
		ordered.sort()
		results.cases[workload] = {"median_us_per_frame": ordered[BATCHES / 2], "samples_us_per_frame": samples}
	var file := FileAccess.open(OS.get_environment("METRUM_CAMERA_BENCH_OUTPUT"), FileAccess.WRITE)
	if file == null:
		push_error("Set METRUM_CAMERA_BENCH_OUTPUT to a writable output file")
		quit(1)
		return
	file.store_string(JSON.stringify(results, "\t") + "\n")
	file.close()
	host.free()
	quit(0)
