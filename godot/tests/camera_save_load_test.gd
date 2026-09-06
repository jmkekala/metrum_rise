# SPDX-License-Identifier: GPL-2.0-only

## Native save/load regression for camera framing and subsequent orbit controls.
extends SceneTree

var _failures: int = 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _camera(host: Node3D, orthogonal: bool) -> CameraNode:
	var camera := CameraNode.new()
	camera.name = "CameraNode"
	camera.set("orthogonal", orthogonal)
	host.add_child(camera)
	camera.make_current()
	camera.set_terrain_clearance_policy(true, 0.25, 1.5)
	return camera

func _continue_controls(camera: CameraNode) -> void:
	camera.pan_screen(Vector2(13.0, -7.0))
	camera.orbit(Vector2(-30.0, 8.0))
	camera.zoom(-1.0)

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
	host.free()
	DirAccess.remove_absolute(path)
	if _failures == 0:
		print("camera_save_load_test: PASS")
	quit(_failures)
