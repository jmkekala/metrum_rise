# SPDX-License-Identifier: GPL-2.0-only

## Opt-in camera motion fixture, driven through the native command API at fixed frame rates.
## Run with tools/camera_playtest.py; no gameplay saves, input bindings or camera code change.
## The synthetic unlit scene exposes motion without asset loading or lighting variability.
extends SceneTree

class FrameDriver extends Node:
	var callback: Callable
	func _process(delta: float) -> void:
		callback.call(delta)

const TRIALS := ["pan_close", "pan_city", "wheel_zoom", "mouse_orbit", "hill_pan", "focus", "follow"]
const HEADER := "trial,frame,time_s,input,pivot_x,pivot_y,pivot_z,yaw,pitch,distance,eye_x,eye_y,eye_z,speed_m_s,vertical_speed_m_s,landmark_x_px,landmark_y_px,clearance_m"
var camera: CameraNode
var simulation: SimulationNode
var host: Node3D
var label: Label
var output: String
var rate := 60
var capture := false
var trial_index := 0
var trial_frame := 0
var total_frame := 0
var input_label := "idle"
var previous_eye := Vector3.ZERO
var rows := PackedStringArray([HEADER])
var pending_capture := false
var finished := false
var failures := 0
var base_y := 0.0

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	# Fixed-FPS capture must not redistribute render delta around physics ticks.
	Engine.physics_jitter_fix = 0.0
	root.size = Vector2i(960, 540)
	output = OS.get_environment("METRUM_CAMERA_OUTPUT")
	rate = int(OS.get_environment("METRUM_CAMERA_FPS"))
	capture = OS.get_environment("METRUM_CAMERA_CAPTURE") == "1"
	if output.is_empty() or rate not in [30, 60, 120]:
		push_error("Camera playtest requires an output directory and 30/60/120 FPS")
		quit(1)
		return
	host = Node3D.new()
	root.add_child(host)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	if not simulation.create_blank_world(1024.0, 1024.0, 4.0, 128.0, 50.0):
		push_error("Camera fixture world creation failed")
		quit(1)
		return
	simulation.set_simulation_speed(0.0)
	base_y = simulation.get_height_at(Vector2.ZERO)
	# Authoring heights are source units; query heights include the runtime height scale.
	var height_scale := base_y / 50.0
	simulation.sculpt_terrain(Vector2(0, 160), 64.0, 30.0 / height_scale)
	camera = CameraNode.new()
	camera.name = "CameraNode"
	camera.process_priority = -200
	host.add_child(camera)
	camera.make_current()
	camera.configure_world_camera()
	camera.set_debug_under_terrain_enabled(false)
	if capture:
		_make_scene()
		RenderingServer.frame_post_draw.connect(_capture)
	# Setup and shader warmup are excluded from the trace.
	for unused in 5:
		await process_frame
	var driver := FrameDriver.new()
	driver.process_priority = -300
	driver.callback = _advance
	host.add_child(driver)
	var sampler := FrameDriver.new()
	sampler.process_priority = -100
	sampler.callback = _sample
	host.add_child(sampler)

func _duration_frames() -> int:
	return rate * (4 if TRIALS[trial_index] == "hill_pan" else 3)

func _advance(delta: float) -> void:
	if finished:
		return
	if trial_frame == _duration_frames():
		trial_frame = 0
		trial_index += 1
		if trial_index == TRIALS.size():
			_finish()
			return
	if not is_equal_approx(delta, 1.0 / rate):
		push_error("Use --fixed-fps matching METRUM_CAMERA_FPS: delta=%s expected=%s" % [delta, 1.0 / rate])
		failures += 1
		_finish()
		return
	var trial: String = TRIALS[trial_index]
	if trial_frame == 0:
		camera.stop_follow()
		var distance := 40.0 if trial == "pan_close" else 400.0
		var pivot := Vector3(0, base_y + 0.25, 0)
		if trial == "hill_pan":
			distance = 80.0
			pivot = Vector3(-72, base_y + 0.25, 160)
		if not camera.set_view_state(PackedFloat32Array([pivot.x, pivot.y, pivot.z, 0.0, -PI / 4.0, distance, 0.0])):
			failures += 1
		previous_eye = camera.global_position
	input_label = "idle"
	var t := float(trial_frame) / rate
	match trial:
		"pan_close", "pan_city", "hill_pan":
			var end := 3.5 if trial == "hill_pan" else 1.5
			if t >= 0.5 and t < end:
				input_label = "pan right held"
				camera.pan(Vector3.RIGHT, 1.0)
		"wheel_zoom":
			if trial_frame in [rate / 2, rate, rate * 3 / 2, rate * 2]:
				var amount := -1.0 if trial_frame == rate * 2 else 1.0
				input_label = "wheel %+.0f" % amount
				camera.zoom(amount)
		"mouse_orbit":
			if t >= 0.5 and t < 1.5:
				input_label = "mouse right 120 px/s"
				camera.orbit(Vector2(120.0 * delta, 0))
		"focus":
			if trial_frame == rate:
				input_label = "focus building"
				camera.focus_on(Vector3(60, base_y, 0), 24.0)
		"follow":
			if t >= 0.5 and t <= 2.5:
				input_label = "follow target 12 m/s"
				camera.follow(Vector3(20 + (t - 0.5) * 12, base_y, 0))
			elif trial_frame == rate * 5 / 2 + 1:
				camera.stop_follow()

func _sample(delta: float) -> void:
	if finished:
		return
	var state := camera.get_view_state()
	var eye := camera.global_position
	var velocity := (eye - previous_eye) / delta
	var landmark := camera.unproject_position(Vector3(35, base_y, 0))
	var clearance := eye.y - simulation.get_height_at(Vector2(eye.x, eye.z))
	if not eye.is_finite() or clearance < 1.499:
		push_error("Invalid camera pose or terrain penetration")
		failures += 1
	rows.append("%s,%d,%.9f,%s,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f,%.9f" % [
		TRIALS[trial_index], trial_frame, float(trial_frame) / rate, input_label,
		state[0], state[1], state[2], state[3], state[4], state[5],
		eye.x, eye.y, eye.z, velocity.length(), velocity.y, landmark.x, landmark.y, clearance])
	if capture:
		label.text = "%s  |  %.2f s\n%s\nDistance %.1f m   Speed %.1f m/s\n10 m ground grid | Camera command replay | %d FPS" % [
			TRIALS[trial_index].replace("_", " ").to_upper(), float(trial_frame) / rate,
			input_label, state[5], velocity.length(), rate]
		pending_capture = true
	previous_eye = eye
	trial_frame += 1
	total_frame += 1

func _capture() -> void:
	if not pending_capture:
		return
	pending_capture = false
	var error := root.get_texture().get_image().save_png(output.path_join("frames/%05d.png" % (total_frame - 1)))
	if error != OK:
		push_error("Camera capture failed: %s" % error)
		failures += 1

func _finish() -> void:
	finished = true
	var file := FileAccess.open(output.path_join("trace.csv"), FileAccess.WRITE)
	if file == null:
		push_error("Cannot write camera trace")
		quit(1)
		return
	file.store_string("\n".join(rows) + "\n")
	file.close()
	quit(0 if failures == 0 else 1)

func _make_scene() -> void:
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color("18232f")
	host.add_child(environment)
	var vertices := PackedVector3Array()
	var indices := PackedInt32Array()
	for z in 129:
		for x in 129:
			var p := Vector2(x * 4 - 256, z * 4 - 256)
			vertices.append(Vector3(p.x, simulation.get_height_at(p), p.y))
	for z in 128:
		for x in 128:
			var i := z * 129 + x
			indices.append_array(PackedInt32Array([i, i + 1, i + 129, i + 1, i + 130, i + 129]))
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_INDEX] = indices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	var ground := MeshInstance3D.new()
	ground.mesh = mesh
	var material := ShaderMaterial.new()
	material.shader = Shader.new()
	material.shader.code = """shader_type spatial;
render_mode unshaded, cull_disabled;
uniform float base_height;
varying vec3 world;
void vertex() { world = VERTEX; }
void fragment() {
    vec2 grid = abs(fract(world.xz / 10.0 - 0.5) - 0.5) / max(fwidth(world.xz / 10.0), vec2(0.0001));
    float line = 1.0 - min(min(grid.x, grid.y), 1.0);
    vec3 base = mix(vec3(0.15, 0.27, 0.25), vec3(0.5, 0.42, 0.22), clamp((world.y - base_height) / 30.0, 0.0, 1.0));
    ALBEDO = mix(base, vec3(0.52, 0.65, 0.62), line * 0.5);
}"""
	material.set_shader_parameter("base_height", base_y)
	ground.material_override = material
	host.add_child(ground)
	for z in [-60, -30, 0, 30, 60]:
		for x in [-60, -30, 0, 30, 60]:
			var block := MeshInstance3D.new()
			var box := BoxMesh.new()
			box.size = Vector3(8, 5 + abs(x + z) % 17, 8)
			block.mesh = box
			var paint := StandardMaterial3D.new()
			paint.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
			paint.albedo_color = Color("e7bb78") if x == 0 else Color("7494af")
			block.material_override = paint
			host.add_child(block)
			block.position = Vector3(x, base_y + box.size.y / 2, z)
	var canvas := CanvasLayer.new()
	root.add_child(canvas)
	label = Label.new()
	label.position = Vector2(18, 16)
	label.add_theme_font_size_override("font_size", 20)
	label.add_theme_color_override("font_shadow_color", Color.BLACK)
	label.add_theme_constant_override("shadow_offset_x", 2)
	label.add_theme_constant_override("shadow_offset_y", 2)
	canvas.add_child(label)
