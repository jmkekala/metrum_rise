# SPDX-License-Identifier: GPL-2.0-only

## GPU regression for scheduled window spill onto buildings and both ground shaders.
## Optional --benchmark-spill measures matched SSIL off/on viewport GPU times.
extends SceneTree

const Windows := preload("res://scripts/renderers/window_materials.gd")
const GROUND_SHADERS := ["res://assets/materials/terrain.gdshader", "res://scripts/shaders/site_ground.gdshader"]
var failures := 0
var viewport: SubViewport
var environment: Environment
var camera: Camera3D
var ground: MeshInstance3D
var window: MultiMeshInstance3D

func _initialize() -> void:
	call_deferred("run")

func expect(value: bool, message: String) -> void:
	if not value:
		failures += 1
		push_error(message)

func settle(frames: int = 32) -> void:
	for frame in frames:
		await process_frame
		await RenderingServer.frame_post_draw

func receiver_colour(point: Vector3 = Vector3(0, 0, 0.7)) -> Color:
	var image := viewport.get_texture().get_image()
	var pixel := Vector2i(camera.unproject_position(point))
	var total := Color(0, 0, 0, 0)
	for y in range(-3, 4):
		for x in range(-3, 4):
			total += image.get_pixelv(pixel + Vector2i(x, y))
	return total / 49.0

func run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("window_spill_test requires Forward+ GPU rendering")
		quit(1)
		return
	viewport = SubViewport.new()
	viewport.size = Vector2i(640, 360)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var world_environment := WorldEnvironment.new()
	environment = load("res://default_environment.tres").duplicate()
	expect(environment.ssil_enabled, "shared gameplay/editor environment enables spill")
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = Color.BLACK
	environment.ambient_light_source = Environment.AMBIENT_SOURCE_DISABLED
	world_environment.environment = environment
	viewport.add_child(world_environment)
	camera = Camera3D.new()
	viewport.add_child(camera)
	camera.position = Vector3(-4, 4, 7)
	camera.look_at(Vector3(0, 0.6, 0))
	camera.current = true
	ground = MeshInstance3D.new()
	var plane := PlaneMesh.new()
	plane.size = Vector2(12, 12)
	ground.mesh = plane
	viewport.add_child(ground)
	var white := Image.create(4, 4, false, Image.FORMAT_RGB8)
	white.fill(Color.WHITE)
	var texture := ImageTexture.create_from_image(white)
	var source := StandardMaterial3D.new()
	source.albedo_color = Color.BLACK
	source.emission_texture = texture
	window = MultiMeshInstance3D.new()
	window.multimesh = MultiMesh.new()
	window.multimesh.transform_format = MultiMesh.TRANSFORM_3D
	window.multimesh.use_custom_data = true
	var quad := QuadMesh.new()
	quad.size = Vector2(2, 1.5)
	window.multimesh.mesh = quad
	window.multimesh.instance_count = 1
	window.multimesh.set_instance_transform(0, Transform3D(Basis.IDENTITY, Vector3(0, 1, 0)))
	window.multimesh.set_instance_custom_data(0, Color(4, 24.5, 6, 1))
	window.material_override = Windows.create(source)
	viewport.add_child(window)
	RenderingServer.global_shader_parameter_set("scene_ambient_light", Color.BLACK)
	RenderingServer.global_shader_parameter_set("scene_ambient_desaturation", 0.0)
	var standard := StandardMaterial3D.new()
	standard.albedo_color = Color(0.5, 0.5, 0.5)
	var wall := MeshInstance3D.new()
	var box := BoxMesh.new()
	box.size = Vector3(0.2, 2, 3)
	wall.mesh = box
	wall.material_override = standard
	wall.position = Vector3(1.8, 1, 1)
	viewport.add_child(wall)
	var receivers: Array[Material] = [standard]
	for path in GROUND_SHADERS:
		var material := ShaderMaterial.new()
		material.shader = load(path)
		material.set_shader_parameter("height_is_baked", true)
		material.set_shader_parameter("terrain_grass_albedo", texture)
		receivers.append(material)
	for receiver in receivers:
		ground.material_override = receiver
		RenderingServer.global_shader_parameter_set("scene_window_clock", Vector2(21, -5))
		environment.ssil_enabled = false
		await settle()
		var baseline := receiver_colour()
		var wall_baseline := receiver_colour(Vector3(1.7, 0.65, 0.7))
		environment.ssil_enabled = true
		await settle()
		var lit := receiver_colour()
		expect(receiver_colour(Vector3(1.7, 0.65, 0.7)).r > wall_baseline.r + 0.01, "window illuminates neighbouring wall")
		expect(lit.r > baseline.r + 0.01, "window illuminates receiver %s: %s -> %s" % [receiver, baseline, lit])
		expect(lit.r > lit.b * 1.1, "spill retains warm window colour")
		if "--capture-spill" in OS.get_cmdline_user_args():
			viewport.get_texture().get_image().save_png("user://spill-%d.png" % receivers.find(receiver))
		RenderingServer.global_shader_parameter_set("scene_window_clock", Vector2(3, -15))
		await settle()
		var sleeping := receiver_colour()
		expect(sleeping.get_luminance() < lit.get_luminance() * 0.2, "spill disappears when residents sleep")
		RenderingServer.global_shader_parameter_set("scene_window_clock", Vector2(12, 45))
		await settle()
		expect(receiver_colour().get_luminance() < 0.01, "no window spill during daylight")
		if receiver is ShaderMaterial:
			# Isolate the authored sky floor from SSIL: enabling the ambient receiver must
			# not add a second copy of the environment's daylight to terrain/site ground.
			environment.ssil_enabled = false
			RenderingServer.global_shader_parameter_set("scene_ambient_light", Color(0.3, 0.4, 0.5))
			await settle(2)
			var authored_floor := receiver_colour()
			environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
			environment.ambient_light_color = Color.WHITE
			environment.ambient_light_energy = 2.0
			await settle(2)
			expect(absf(receiver_colour().get_luminance() - authored_floor.get_luminance()) < 0.005, "ground does not double-count environment ambient")
			environment.ambient_light_source = Environment.AMBIENT_SOURCE_DISABLED
			RenderingServer.global_shader_parameter_set("scene_ambient_light", Color.BLACK)
	if "--benchmark-spill" in OS.get_cmdline_user_args():
		await benchmark()
	viewport.free()
	print("window_spill_test: %s" % ("PASS" if failures == 0 else "FAIL"))
	quit(failures)

func benchmark() -> void:
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0
	RenderingServer.global_shader_parameter_set("scene_window_clock", Vector2(21, -5))
	RenderingServer.viewport_set_measure_render_time(viewport.get_viewport_rid(), true)
	print("SPILL_SETTINGS ", JSON.stringify({"quality": ProjectSettings.get_setting("rendering/environment/ssil/quality"), "half_size": ProjectSettings.get_setting("rendering/environment/ssil/half_size")}))
	for size in [Vector2i(640, 360), Vector2i(1920, 1080)]:
		viewport.size = size
		for enabled in [false, true, false, true]:
			environment.ssil_enabled = enabled
			await settle(60)
			var gpu: Array[float] = []
			for frame in 120:
				await settle(1)
				gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(viewport.get_viewport_rid()))
			gpu.sort()
			print("SPILL_BENCH ", JSON.stringify({"ssil": enabled, "size": str(size), "gpu_median_ms": gpu[60]}))
