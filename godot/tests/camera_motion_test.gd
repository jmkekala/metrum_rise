# SPDX-License-Identifier: GPL-2.0-only

## Camera response, immediate restore, clearance and UI ownership regression.
## Uses a small blank world and the real input wrappers without gameplay construction.
extends SceneTree

class PickProbe extends Node:
	signal sampled
	var camera: CameraNode
	var observed := Transform3D.IDENTITY
	func _process(_delta: float) -> void:
		observed = camera.global_transform
		sampled.emit()

var failures := 0
var camera: CameraNode

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		failures += 1
		push_error(message)

func _reset(orthogonal := false) -> void:
	_expect(camera.set_view_state(PackedFloat32Array([0, 1000.25, 0, 0, -PI / 4, 400, float(orthogonal)])), "restore valid view")

func _distance() -> float:
	return camera.get_view_state()[5]

func _native_motion() -> void:
	for orthogonal: bool in [false, true]:
		_reset(orthogonal)
		var start := camera.global_transform
		camera.zoom(1.0)
		camera.zoom(1.0)
		_expect(camera.global_transform == start, "wheel events only change the requested view")
		camera.advance_motion(1.0 / 60.0)
		_expect(_distance() > 400.0 / 1.44 and _distance() < 400.0, "burst begins smoothly toward both notches")
		if orthogonal:
			_expect(is_equal_approx(camera.size, _distance() * 0.5), "orthographic size follows displayed distance")
		camera.advance_motion(1.0)
		_expect(absf(_distance() - 400.0 / 1.44) < 0.001, "burst finishes at the accumulated target")
		camera.zoom(-1.0)
		camera.advance_motion(0.02)
		var displayed := camera.get_view_state()
		camera.pan(Vector3.RIGHT, 1.0)
		camera.advance_motion(0.02)
		_expect(camera.set_view_state(displayed), "restore while motion is pending")
		var restored := camera.global_transform
		camera.advance_motion(1.0)
		_expect(camera.global_transform == restored, "restoring the displayed view discards velocity and zoom intent")

	_reset()
	camera.pan(Vector3.RIGHT, 1.0)
	camera.advance_motion(1.0 / 60.0)
	var first := camera.get_focus_position().x
	_expect(first > 0.0 and first < 4.0, "first pan frame accelerates instead of jumping to full speed")
	camera.advance_motion(1.0 / 60.0)
	_expect(camera.get_focus_position().x > first, "released keyboard velocity brakes briefly")
	camera.advance_motion(1.0)
	var stopped := camera.global_transform
	camera.advance_motion(1.0)
	_expect(camera.global_transform == stopped, "released input cannot remain held or drift indefinitely")
	camera.zoom(1.0)
	camera.pan(Vector3.RIGHT, 1.0)
	camera.advance_motion(0.02)
	camera.cancel_motion()
	stopped = camera.global_transform
	camera.advance_motion(1.0)
	_expect(camera.global_transform == stopped, "UI cancellation stops at the displayed view")

	camera.zoom(-2.0)
	camera.pan(Vector3.RIGHT, 1.0)
	camera.focus_on(Vector3(50, 1000, 0), 24)
	stopped = camera.global_transform
	camera.advance_motion(1.0)
	_expect(camera.global_transform == stopped, "explicit framing cancels outstanding motion")
	camera.pan(Vector3.RIGHT, 1.0)
	camera.advance_motion(0.02)
	camera.follow(Vector3(20, 1000, 0))
	camera.zoom(1.0)
	camera.advance_motion(0.02)
	_expect(camera.is_following() and camera.get_focus_position().x == 20.0, "follow owns the pivot while zoom runs")
	camera.pan(Vector3.RIGHT, 1.0)
	_expect(not camera.is_following(), "keyboard input ends follow before the next inspector update")
	camera.cancel_motion()
	camera.pan(Vector3.RIGHT, 1.0)
	camera.pan_screen(Vector2(4, 0))
	stopped = camera.global_transform
	camera.advance_motion(1.0)
	_expect(camera.global_transform == stopped, "direct drag cannot leave previous keyboard motion running")

	_reset()
	camera.zoom(10.0)
	camera.set_distance_bounds(200, 300)
	_expect(_distance() == 300.0, "changing bounds immediately constrains the displayed view")
	camera.advance_motion(1.0)
	_expect(_distance() == 200.0, "changing bounds also constrains the pending target")
	camera.set_distance_bounds(10, 1000)
	_reset()
	camera.zoom(1.0)
	var before_orbit := camera.get_view_state()
	camera.orbit(Vector2(120, 0))
	_expect(absf(camera.get_view_state()[3] - before_orbit[3] + 0.36) < 0.00001, "orbit remains immediate during zoom")
	stopped = camera.global_transform
	camera.advance_motion(0)
	camera.advance_motion(-1)
	_expect(camera.global_transform == stopped, "invalid deltas never move the view")

func _water_clearance(simulation: SimulationNode) -> void:
	simulation.begin_world_open_water_fill_preview(Vector2.ZERO, 1004.0)
	_expect(simulation.commit_world_open_water_fill_preview(), "water clearance fixture")
	for debug_mode: bool in [false, true]:
		camera.set_debug_under_terrain_enabled(debug_mode)
		var pitch := 0.5 if debug_mode else -0.1
		_expect(camera.set_view_state(PackedFloat32Array([0, 1000.25, 0, 0, pitch, 100, 0])), "shallow water view")
		camera.zoom(80.0)
		for frame in 60:
			camera.advance_motion(1.0 / 60.0)
			var eye := camera.global_position
			var surface := simulation.get_camera_water_surface_at(Vector2(eye.x, eye.z))
			_expect(is_finite(surface) and eye.y >= surface + 1.499, "every zoom frame clears water, including upward debug orbit")
	camera.set_debug_under_terrain_enabled(false)

func _input_wrappers(host: Node3D) -> void:
	var pick := PickProbe.new()
	pick.camera = camera
	host.add_child(pick)
	for script_path: String in ["res://scripts/core/input_manager.gd", "res://scripts/core/editor_camera_input.gd", "res://scripts/editors/world_editor_camera_input.gd"]:
		_reset()
		var input := Node.new()
		host.add_child(input)
		input.set_script(load(script_path))
		var gameplay := script_path.ends_with("/input_manager.gd")
		if gameplay:
			# Match the gameplay shell's process order without constructing all tools/renderers.
			input.process_priority = input.CAMERA_INPUT_PROCESS_PRIORITY
			camera.process_priority = input.CAMERA_SETTLE_PROCESS_PRIORITY
		else:
			input._ready()
		_reset()
		camera.set_process(true)
		input.set_process(true)
		# Let the window's initial input/focus setup finish before injecting a held key.
		await process_frame
		await process_frame
		var press := InputEventKey.new()
		press.keycode = KEY_D
		press.pressed = true
		Input.parse_input_event(press)
		Input.flush_buffered_events()
		await pick.sampled
		_expect(camera.get_focus_position().x > 0.0, script_path + ": keyboard moves the camera in its first frame")
		_expect(pick.observed == camera.global_transform, script_path + ": picking observes the advanced view")
		var release := press.duplicate()
		release.pressed = false
		Input.parse_input_event(release)
		# A focused text field must discard both braking and an outstanding zoom target.
		var field := LineEdit.new()
		root.add_child(field)
		field.grab_focus()
		camera.zoom(1.0)
		var captured := camera.global_transform
		await process_frame
		await process_frame
		_expect(camera.global_transform == captured, script_path + ": text input cancels pending motion before picking")
		field.free()
		camera.zoom(1.0)
		camera.pan(Vector3.RIGHT, 1.0)
		if gameplay:
			input._on_camera_focus_exited()
		else:
			input._on_focus_exited()
		captured = camera.global_transform
		await process_frame
		await process_frame
		_expect(camera.global_transform == captured, script_path + ": focus loss cancels pending navigation")
		input.free()
		camera.set_process(false)
	pick.free()

func _overview_policy(host: Node3D, simulation: SimulationNode) -> void:
	var input := Node.new()
	host.add_child(input)
	input.set_script(load("res://scripts/core/input_manager.gd"))
	input._configure_world_camera_policy()
	_reset()
	camera.zoom(-100.0)
	camera.advance_motion(1.0)
	_expect(_distance() == 40000.0, "gameplay reaches the fixed 40 km overview limit even on a small map")
	_expect(camera.far > 40000.0 + Vector2(16000, 16000).length(), "far clipping clears a 32 km map")
	for yaw in [0.0, -PI / 4.0]:
		camera.set_view_state(PackedFloat32Array([0, 1000.25, 0, yaw, -PI / 4, 40000, 0]))
		for x in [-9000.0, 9000.0]:
			for z in [-9000.0, 9000.0]:
				var corner := Vector3(x, 1000.25, z)
				_expect(not camera.is_position_behind(corner) and Rect2(Vector2.ZERO, Vector2(root.size)).has_point(camera.unproject_position(corner)), "18 km map corners fit at overview distance")
	camera.zoom(100.0)
	camera.advance_motion(2.0)
	_expect(_distance() == 10.0, "overview preserves close-up zoom")
	input.free()
	# Each direction must stop at the live world's edge, even at overview pan speeds.
	for direction in [Vector3.RIGHT, Vector3.LEFT, Vector3.FORWARD, Vector3.BACK]:
		_reset()
		camera.zoom(-100.0)
		camera.advance_motion(1.0)
		camera.pan(direction, 1.0)
		camera.advance_motion(1.0)
		var edge := camera.get_view_state()
		_expect(absf(edge[0]) <= 256.0 and absf(edge[2]) <= 256.0, "overview pan remains on map")
		_expect(maxf(absf(edge[0]), absf(edge[2])) == 256.0, "pan reaches boundary")
		camera.advance_motion(1.0)
		_expect(camera.get_view_state() == edge, "release cannot drift beyond boundary")
		camera.pan(-direction, 1.0)
		camera.advance_motion(0.001)
		var inward := camera.get_view_state()
		_expect(absf(inward[0]) + absf(inward[2]) < 256.0, "reversing responds immediately")
	_reset()
	camera.pan_screen(Vector2(100000, -100000))
	var dragged := camera.get_view_state()
	_expect(absf(dragged[0]) == 256.0 and absf(dragged[2]) == 256.0, "drag clamps both axes")
	camera.set_view_state(PackedFloat32Array([9999, 1000.25, -9999, 0, -PI / 4, 400, 0]))
	camera.advance_motion(0.016)
	var restored := camera.get_view_state()
	_expect(restored[0] == 256.0 and restored[2] == -256.0, "old off-map saves clamp on deferred update")
	camera.set_view_state(PackedFloat32Array([256, 1000.25, 0, 0, -PI / 4, 400, 0]))
	camera.pan(Vector3(1, 0, 1), 1.0)
	camera.advance_motion(0.1)
	var slide := camera.get_view_state()
	_expect(slide[0] == 256.0 and slide[2] > 0.0, "diagonal pan slides along boundary")
	camera.set_view_state(PackedFloat32Array([0, 1000.25, 0, 0, -PI / 4, 60000, 0]))
	_expect(_distance() == 40000.0, "older 60 km saves respect new gameplay limit")
	_expect(simulation.create_blank_world(256, 256, 8, 128, 50), "replacement world")
	camera.focus_on(Vector3(1000, 1000, -1000), 160.0)
	var resized := camera.get_view_state()
	_expect(resized[0] == 128.0 and resized[2] == -128.0, "focus uses replacement world's dimensions")
	camera.set_world_bounds_enabled(false)
	camera.focus_on(Vector3(1000, 1000, 1000), 160.0)
	_expect(camera.get_view_state()[0] == 1000.0, "editor policy can remain unbounded")

func _run() -> void:
	root.size = Vector2i(960, 540)
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(512, 512, 8, 128, 50), "fixture world")
	camera = CameraNode.new()
	camera.name = "CameraNode"
	host.add_child(camera)
	camera.make_current()
	camera.configure_world_camera()
	camera.set_debug_under_terrain_enabled(false)
	camera.set_process(false)
	_native_motion()
	_water_clearance(simulation)
	await _input_wrappers(host)
	_overview_policy(host, simulation)
	host.free()
	if failures == 0:
		print("camera_motion_test: PASS")
	quit(failures)
