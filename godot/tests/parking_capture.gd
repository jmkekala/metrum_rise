# SPDX-License-Identifier: GPL-2.0-only

## Isolated PARK-02 resident fixture. Rust admits households, schedules their departures and
## executes the ordinary parking FSM. The copied meshes provide controlled fixture homes.
extends SceneTree

var main: Node
var sim: SimulationNode
var _failed := false

func _initialize() -> void:
	call_deferred("_run")

func _road(points: PackedVector3Array) -> void:
	var before := sim.get_network_render_generation()
	sim.add_road_with_snap(points, 1, 1, true)
	var deadline := Time.get_ticks_msec() + 20000
	while sim.get_network_render_generation() <= before and Time.get_ticks_msec() < deadline:
		await process_frame
	if sim.get_network_render_generation() <= before:
		push_error("Capture road failed")
		_failed = true

func _run() -> void:
	main = load("res://scenes/Main.tscn").instantiate()
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	main.get_node("MainUI").hide()
	var input: Node = main.get_node("InputManager")
	input.set_process(false)
	input.set_process_input(false)
	input.set_process_unhandled_input(false)
	sim = main.get_node("SimulationNode")
	if not sim.create_blank_world(1536.0, 1024.0, 2.0, 128.0, 0.0):
		quit(1)
		return
	input._refresh_after_world_load()
	sim.set_simulation_speed(0.0)
	sim.debug_reset_parking_capture_clock()
	sim.set_weather_automatic(false)
	sim.set_weather_scenario(0, 0.0)
	main.get_node("Buildings").reload_asset_packs()
	# Two ordinary streets joined at their ends, with mid-span driveways at both districts.
	await _road(PackedVector3Array([Vector3(-500, 0, 0), Vector3(500, 0, 0)]))
	await _road(PackedVector3Array([Vector3(500, 0, 0), Vector3(500, 0, 240)]))
	await _road(PackedVector3Array([Vector3(500, 0, 240), Vector3(-500, 0, 240)]))
	await _road(PackedVector3Array([Vector3(-500, 0, 240), Vector3(-500, 0, 0)]))
	# Dead-end arms provide legal direction changes through ordinary junction routing.
	await _road(PackedVector3Array([Vector3(500, 0, 240), Vector3(620, 0, 240)]))
	await _road(PackedVector3Array([Vector3(-500, 0, 0), Vector3(-620, 0, 0)]))
	var placements := [
		["neukku:building.residential.lamelli_8", -180.0, -24.0],
		["neukku:building.residential.pistetalo_12", -105.0, -24.0],
		["neukku:building.residential.lamelli_8", -30.0, -24.0],
		["kuopio:building.residential.house_1", 120.0, 216.0],
		["kuopio:building.residential.house_2", 160.0, 216.0],
		["kuopio:building.residential.house_3", 200.0, 216.0],
		["kuopio:building.residential.house_4", 240.0, 216.0],
		["parking:building.parking.small_lot", 35.0, -30.0],
		["parking:building.parking.small_lot", 75.0, -30.0],
		["parking:building.parking.small_lot", 65.0, 210.0],
		["parking:building.parking.small_lot", 285.0, 210.0],
	]
	var results := []
	for placement: Array in placements:
		var error: String = sim.place_service_building(placement[0], placement[1], placement[2])
		results.append({"asset": placement[0], "error": error})
		if not error.is_empty():
			push_error("Capture placement %s: %s" % [placement[0], error])
			_failed = true
	for pair: Array in [[0, 3], [1, 4], [2, 5], [0, 6], [3, 0], [4, 1], [5, 2], [6, 0]]:
		if sim.debug_add_parking_commuter(pair[0], pair[1]) < 0:
			push_error("Capture resident admission failed: %s" % [pair])
			_failed = true
	var output := OS.get_environment("METRUM_PARKING_CAPTURE_DIR")
	DirAccess.make_dir_recursive_absolute(output)
	if _failed:
		quit(1)
		return
	# Settle ordinary geometry and routing publication before timing the fixture departures.
	if not sim.save_game(output.path_join("capture-start.sqlite")):
		quit(1)
		return
	var scheduled := sim.debug_schedule_parking_capture(9 * 60, false)
	if scheduled < 4:
		push_error("Capture has too few connected resident cars: %s" % scheduled)
		quit(1)
		return
	var camera: Camera3D = main.get_node("CameraNode")
	camera.set_process(false)
	camera.set_process_input(false)
	camera.set_process_unhandled_input(false)
	camera.far = 2200.0
	camera.fov = 62.0
	Engine.max_fps = 30
	if DisplayServer.get_name() != "headless":
		DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_WINDOWED)
		DisplayServer.window_set_size(Vector2i(1600, 900))
	var records := []
	for hour in [9, 18]:
		if hour == 18:
			sim.debug_advance_parking_capture(14 * 60)
			sim.debug_schedule_parking_capture(18 * 60, true)
		sim.debug_advance_parking_capture(hour * 60)
		var info: Dictionary = sim.get_parking_info()
		records.append({"hour": hour, "parking": info})
		if info["resident_cars_in_buildings"] != 0:
			push_error("A resident car entered a building")
			_failed = true
		if DisplayServer.get_name() == "headless":
			continue
		main.get_node("SceneLighting").pin_hour_of_day(float(hour))
		var views := {
			"slabs": [Vector3(38, 38, 63), Vector3(-12, 0, -22)],
			"houses": [Vector3(185, 65, 345), Vector3(170, 0, 213)],
			"house-lot": [Vector3(91, 27, 256), Vector3(70, 0, 211)],
			"slab-lot": [Vector3(64, 24, 25), Vector3(40, 0, -27)],
			"district": [Vector3(160, 300, 550), Vector3(0, 0, 115)],
		}
		for view: String in views:
			camera.global_position = views[view][0]
			camera.look_at(views[view][1], Vector3.UP)
			var deadline := Time.get_ticks_msec() + 30000
			var stable := 0
			while stable < 12 and Time.get_ticks_msec() < deadline:
				await process_frame
				if main.get_node("Terrain").is_render_settled():
					stable += 1
				else:
					stable = 0
			if stable < 12:
				push_error("Capture terrain did not settle")
				_failed = true
				break
			await RenderingServer.frame_post_draw
			root.get_texture().get_image().save_png(output.path_join("%s-%02d00.png" % [view, hour]))
	var report := FileAccess.open(output.path_join("scene.json"), FileAccess.WRITE)
	report.store_string(JSON.stringify({"placements": results, "scheduled": scheduled, "samples": records}, "  "))
	report.close()
	main.queue_free()
	await process_frame
	quit(1 if _failed else 0)
