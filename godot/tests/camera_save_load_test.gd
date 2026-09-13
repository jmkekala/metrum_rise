# SPDX-License-Identifier: GPL-2.0-only

## Native save/load regression for camera framing, controls, and world-bound UI state.
extends SceneTree

var _failures: int = 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _camera(host: Node3D, orthogonal: bool, debug_under_terrain: bool = false) -> CameraNode:
	var camera := CameraNode.new()
	camera.name = "CameraNode"
	camera.set("orthogonal", orthogonal)
	host.add_child(camera)
	camera.make_current()
	camera.configure_world_camera()
	camera.set_debug_under_terrain_enabled(debug_under_terrain)
	return camera

func _continue_controls(camera: CameraNode) -> void:
	camera.pan_screen(Vector2(13.0, -7.0))
	camera.orbit(Vector2(-30.0, 8.0))
	camera.zoom(-1.0)

func _zoom_endpoints(camera: CameraNode) -> Array[Transform3D]:
	var endpoints: Array[Transform3D] = []
	for direction: float in [1.0, -1.0]:
		for step in 80:
			camera.zoom(direction)
		endpoints.append(camera.global_transform)
	return endpoints

func _check_orthographic_bounds(host: Node3D) -> void:
	var camera := _camera(host, true)
	for distance: float in [20.0, 80.0]:
		camera.set_distance_bounds(distance, distance)
		_expect(is_equal_approx(camera.size, distance * 0.5), "changing bounds must immediately update orthographic zoom")
		var bounded_view := camera.global_transform
		camera.zoom(1.0)
		_expect(camera.global_transform.is_equal_approx(bounded_view), "zoom must respect the updated distance bounds")
	camera.free()

func _check_camera_modes(host: Node3D, simulation: SimulationNode, path: String) -> void:
	var normal := _camera(host, false)
	var debug := _camera(host, false, true)
	_expect(debug.global_transform.is_equal_approx(normal.global_transform), "fresh normal and debug cameras must share their initial view")
	# A raised centre checks terrain following while panning and clearance into a hill.
	simulation.sculpt_terrain(Vector2.ZERO, 32.0, 0.5)
	_expect(simulation.get_height_at(Vector2.ZERO) > simulation.get_height_at(Vector2(60.0, 0.0)), "camera fixture must include a hill")
	for camera: CameraNode in [normal, debug]:
		camera.focus_on(Vector3(20.0, 0.0, -20.0), 12.0)
	_expect(debug.global_transform.is_equal_approx(normal.global_transform), "focus and hillside clearance must match across modes")
	var hillside_height := simulation.get_height_at(Vector2(normal.global_position.x, normal.global_position.z))
	_expect(normal.global_position.y >= hillside_height + 1.499, "shared downward view must clear the hillside")
	for camera: CameraNode in [normal, debug]:
		camera.pan(Vector3(1.0, 0.0, -1.0), 1.0, 0.1)
		_continue_controls(camera)
	_expect(debug.global_transform.is_equal_approx(normal.global_transform), "pan, orbit and zoom must follow terrain identically across modes")
	var above_ground := debug.global_transform
	debug.set_debug_under_terrain_enabled(false)
	debug.set_debug_under_terrain_enabled(true)
	_expect(debug.global_transform.is_equal_approx(above_ground), "switching debug mode must preserve an above-ground view")
	for camera: CameraNode in [normal, debug]:
		camera.focus_on(Vector3(80.0, 50.0, 80.0), 12.0)
		camera.orbit(Vector2(0.0, -1000.0))
	_expect(normal.global_basis.z.y > 0.0, "normal orbit must keep looking downward")
	_expect(debug.global_basis.z.y < 0.0, "debug orbit must allow looking upward")
	_expect(debug.global_position.y < simulation.get_height_at(Vector2(debug.global_position.x, debug.global_position.z)), "upward debug orbit must pass below terrain")
	var underground_view := debug.global_transform
	_expect(simulation.save_game(path), "underground debug view must save")
	_continue_controls(debug)
	_expect(simulation.load_game(path), "underground debug view must load")
	_expect(debug.global_transform.is_equal_approx(underground_view), "debug load must restore the underground orbit")
	debug.set_debug_under_terrain_enabled(false)
	_expect(debug.global_transform.is_equal_approx(normal.global_transform), "leaving an underground debug view must restore normal camera constraints")
	_expect(simulation.load_game(path), "normal mode must load a debug save")
	_expect(debug.global_transform.is_equal_approx(normal.global_transform), "normal load must constrain a saved underground debug pitch")
	debug.free()
	normal.free()

func _check_world_load_controls(host: Node3D, simulation: SimulationNode, save_path: String, world_path: String) -> void:
	var input_manager = load("res://scripts/core/input_manager.gd").new()
	# Bind only the dependencies used by world replacement, without creating gameplay tools.
	input_manager.simulation_node = simulation
	var inspector = load("res://scripts/ui/building_inspector.gd").new()
	host.add_child(inspector)
	input_manager.building_inspector = inspector
	for new_world: bool in [false, true]:
		inspector._open_windows["previous_world"] = inspector._create_window_entry("previous_world")
		input_manager.set_simulation_speed(4.0)
		var loaded: bool = input_manager.menu_load_world_definition(world_path) if new_world else input_manager.menu_load_game_from_path(save_path)
		_expect(loaded, "input-manager world replacement must succeed")
		_expect(is_zero_approx(input_manager._simulation_speed), "loading a paused world must reset the displayed speed")
		_expect(inspector._open_windows.is_empty(), "world replacement must close inspectors anchored to the previous world")
		input_manager._toggle_pause()
		_expect(is_equal_approx(input_manager._simulation_speed, 1.0), "the first pause toggle after load must resume simulation")
		input_manager.set_simulation_speed(0.0)
	input_manager.free()
	inspector.free()

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(256.0, 256.0, 8.0, 128.0, 50.0), "camera fixture world must be created")
	var path := OS.get_temp_dir().path_join("metrum_camera_test_%d.sqlite" % OS.get_process_id())
	for orthogonal: bool in [false, true]:
		var camera := _camera(host, orthogonal)
		camera.focus_on(Vector3(40.0, 50.0, -30.0), 70.0)
		camera.orbit(Vector2(185.0, -45.0))
		camera.zoom(1.0)
		var saved_transform := camera.global_transform
		var saved_projection := camera.projection
		var saved_size := camera.size
		_expect(simulation.save_game(path), "camera fixture must save")
		_continue_controls(camera)
		var continued_transform := camera.global_transform
		var continued_size := camera.size
		_expect(not saved_transform.is_equal_approx(continued_transform), "test controls must move the camera")
		_expect(simulation.load_game(path), "camera fixture must load")
		_expect(camera.global_transform.is_equal_approx(saved_transform), "load must restore position and rotation in the existing scene")
		camera.free()
		# A fresh camera starts with the opposite projection and a different orbit pivot.
		camera = _camera(host, not orthogonal)
		camera.focus_on(Vector3(-60.0, 50.0, 20.0), 100.0)
		_expect(simulation.load_game(path), "camera fixture must load into a fresh camera")
		_expect(camera.global_transform.is_equal_approx(saved_transform), "fresh scene must restore the saved camera transform")
		_expect(camera.projection == saved_projection, "load must restore camera projection")
		if orthogonal:
			_expect(is_equal_approx(camera.size, saved_size), "load must restore orthographic zoom")
		_continue_controls(camera)
		_expect(camera.global_transform.is_equal_approx(continued_transform), "the first pan/orbit/zoom after load must use the saved controls")
		if orthogonal:
			_expect(is_equal_approx(camera.size, continued_size), "orthographic zoom controls must continue from the saved distance")
		camera.free()
	_expect(simulation.save_game(path), "simulation-only saves must remain supported")
	var camera := _camera(host, false)
	camera.focus_on(Vector3(65.0, 50.0, -15.0), 90.0)
	var unchanged_transform := camera.global_transform
	_expect(simulation.load_game(path), "camera-less save must load")
	_expect(camera.global_transform.is_equal_approx(unchanged_transform), "camera-less save must not reuse another save's view")
	var world_path := OS.get_temp_dir().path_join("metrum_new_world_test_%d.sqlite" % OS.get_process_id())
	_expect(simulation.save_world_definition(world_path, "New game camera test"), "world definition must save")
	_expect(simulation.load_world_definition(world_path), "new game must load in the existing scene")
	var new_game_transform := camera.global_transform
	_expect(not new_game_transform.is_equal_approx(unchanged_transform), "new game must reset the old city's camera")
	_continue_controls(camera)
	_expect(simulation.load_world_definition(world_path), "a second new game must load")
	_expect(camera.global_transform.is_equal_approx(new_game_transform), "new games must start with the same camera framing")
	_expect(camera.global_position.y > 50.0, "new-game camera must frame the new terrain elevation")
	var normal_endpoints: Array[Transform3D] = []
	for debug_under_terrain: bool in [false, true]:
		camera.set_debug_under_terrain_enabled(debug_under_terrain)
		_expect(simulation.load_world_definition(world_path), "zoom fixture must reset its world")
		_expect(camera.global_transform.is_equal_approx(new_game_transform), "New Game must use the same view in normal and debug modes")
		var endpoints := _zoom_endpoints(camera)
		# A fresh camera provides the pre-reset zoom range for the same scene policy.
		var fresh := _camera(host, false, debug_under_terrain)
		var fresh_endpoints := _zoom_endpoints(fresh)
		_expect(camera.get("min_distance") == fresh.get("min_distance") and camera.get("max_distance") == fresh.get("max_distance"), "New Game must preserve zoom bounds")
		for index in 2:
			_expect(endpoints[index].is_equal_approx(fresh_endpoints[index]), "New Game must preserve the fresh camera's zoom endpoints (debug=%s, endpoint=%d)" % [debug_under_terrain, index])
		var ground := simulation.get_height_at(Vector2.ZERO)
		_expect(endpoints[0].origin.y > ground, "downward-facing zoom must stay above ground in both modes")
		if debug_under_terrain:
			for index in 2:
				_expect(endpoints[index].is_equal_approx(normal_endpoints[index]), "normal and debug zoom endpoints must match")
		else:
			normal_endpoints = endpoints
		fresh.free()
		camera.make_current()
	_check_camera_modes(host, simulation, path)
	_check_orthographic_bounds(host)
	_check_world_load_controls(host, simulation, path, world_path)
	DirAccess.remove_absolute(world_path)
	host.free()
	DirAccess.remove_absolute(path)
	if _failures == 0:
		print("camera_save_load_test: PASS")
	quit(_failures)
