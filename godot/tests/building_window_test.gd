# SPDX-License-Identifier: GPL-2.0-only

## Rendered schedule/preview parity regression and optional matched material benchmark.
## Requires a real renderer: headless dummy rendering cannot validate shader execution.
extends SceneTree

const Windows := preload("res://scripts/renderers/window_materials.gd")
var failures := 0
var viewport: SubViewport
var batch: MultiMeshInstance3D
var preview: MeshInstance3D
var source: StandardMaterial3D
var material: ShaderMaterial
var preview_material: ShaderMaterial

func expect(value: bool, message: String) -> void:
	if not value:
		failures += 1
		push_error(message)

func _initialize() -> void:
	call_deferred("run")

func sample(hour: float, elevation: float, schedule: Color, use_preview: bool = false) -> float:
	RenderingServer.global_shader_parameter_set("scene_window_clock", Vector2(hour, elevation))
	batch.multimesh.set_instance_custom_data(0, schedule)
	preview_material.set_shader_parameter("window_preview_clock", Vector2(hour, elevation))
	preview_material.set_shader_parameter("window_preview_schedule", Vector4(schedule.r, schedule.g, schedule.b, schedule.a))
	batch.visible = not use_preview
	preview.visible = use_preview
	await process_frame
	await RenderingServer.frame_post_draw
	return viewport.get_texture().get_image().get_pixel(32, 32).get_luminance()

func run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("building_window_test needs GPU rendering; run without --headless")
		quit(1)
		return
	viewport = SubViewport.new()
	viewport.size = Vector2i(64, 64)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color.BLACK
	environment.environment.ambient_light_source = Environment.AMBIENT_SOURCE_DISABLED
	viewport.add_child(environment)
	var camera := Camera3D.new()
	camera.position.z = 3.0
	viewport.add_child(camera)
	camera.current = true
	var image := Image.create(4, 4, false, Image.FORMAT_RGB8)
	image.fill(Color.WHITE)
	source = StandardMaterial3D.new()
	source.albedo_color = Color.BLACK
	source.emission_texture = ImageTexture.create_from_image(image)
	# Common glTF export: mask exists but its authored emissive factor is zero.
	source.emission = Color.BLACK
	material = Windows.create(source)
	preview_material = Windows.create(source, true)
	var mesh := QuadMesh.new()
	mesh.size = Vector2(2, 2)
	batch = MultiMeshInstance3D.new()
	batch.multimesh = MultiMesh.new()
	batch.multimesh.transform_format = MultiMesh.TRANSFORM_3D
	batch.multimesh.use_custom_data = true
	batch.multimesh.mesh = mesh
	batch.multimesh.instance_count = 1
	batch.multimesh.set_instance_transform(0, Transform3D.IDENTITY)
	batch.material_override = material
	viewport.add_child(batch)
	preview = MeshInstance3D.new()
	preview.mesh = mesh
	preview.material_override = preview_material
	viewport.add_child(preview)
	var home := Color(4, 24.5, 6, 1)
	var bright := await sample(21, -5, home)
	expect(bright > 0.15, "a default-off imported emission mask lights at night")
	for entry in [[12.0, 45.0, false], [19.0, 5.0, false], [19.0, 3.0, true],
		[23.99, -15.0, true], [0.01, -15.0, true], [2.0, -15.0, false],
		[6.2, -5.0, true], [9.0, 20.0, false]]:
		var value := await sample(entry[0], entry[1], home)
		expect(value > bright * 0.95 if entry[2] else value < 0.01, "schedule at hour %s, sun %s" % [entry[0], entry[1]])
		var preview_value := await sample(entry[0], entry[1], home, true)
		expect(absf(value - preview_value) < 0.01, "preview and MultiMesh render the same schedule")
	var dusk_mid := await sample(19, 4, home)
	expect(dusk_mid > 0.01 and dusk_mid < bright * 0.95, "dusk fades instead of switching abruptly")
	var sleep_mid := await sample(0.55, -15, home)
	expect(sleep_mid > 0.01 and sleep_mid < bright * 0.95, "bedtime fades across midnight")
	expect(await sample(3, -15, Color(4, 24.5, 6, 2)) > bright * 0.95, "overnight profile remains lit")
	expect(await sample(21, -5, Color(4, 24.5, 6, 0)) < 0.01, "abandoned and unfinished buildings remain dark")
	expect(await sample(19, 5, Color(8, 24.5, 6, 1)) > bright * 0.95, "different houses turn on before sunset at different times")
	expect(source.emission == Color.BLACK and not source.emission_enabled, "runtime lighting leaves authored material untouched")
	await check_materials()
	if "--benchmark-windows" in OS.get_cmdline_user_args():
		await benchmark()
	viewport.free()
	print("building_window_test: %s" % ("PASS" if failures == 0 else "FAIL"))
	quit(failures)

func check_materials() -> void:
	# Compile every supported alpha mode and exercise normal/ORM bindings on the GPU.
	var sun := DirectionalLight3D.new()
	sun.rotation_degrees = Vector3(-20, -25, 0)
	viewport.add_child(sun)
	for transparency in [BaseMaterial3D.TRANSPARENCY_DISABLED, BaseMaterial3D.TRANSPARENCY_ALPHA,
		BaseMaterial3D.TRANSPARENCY_ALPHA_SCISSOR, BaseMaterial3D.TRANSPARENCY_ALPHA_HASH,
		BaseMaterial3D.TRANSPARENCY_ALPHA_DEPTH_PRE_PASS]:
		var authored := ORMMaterial3D.new()
		authored.transparency = transparency
		authored.cull_mode = BaseMaterial3D.CULL_DISABLED
		authored.orm_texture = source.emission_texture
		authored.emission_texture = source.emission_texture
		authored.normal_enabled = true
		authored.albedo_color = Color(0.3, 0.4, 0.5, 1.0)
		var converted := Windows.create(authored)
		expect(converted.get_shader_parameter("texture_roughness") == authored.orm_texture
			and converted.get_shader_parameter("roughness_channel") == Vector4(0, 1, 0, 0), "ORM channels survive conversion")
		expect(converted.get_shader_parameter("albedo") == authored.albedo_color, "authored albedo tint survives conversion")
		batch.material_override = converted
		await sample(21, -5, Color(4, 24.5, 6, 1))
		# Daytime must retain the built-in material's PBR response, not just its bindings.
		var candidate := await sample(12, 45, Color(4, 24.5, 6, 1))
		batch.material_override = authored
		var original := await sample(12, 45, Color(4, 24.5, 6, 1))
		expect(absf(candidate - original) < 0.015, "daytime ORM shading matches the imported material")
	sun.free()
	batch.material_override = material

func benchmark() -> void:
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0
	viewport.size = Vector2i(1024, 1024)
	batch.multimesh.instance_count = 4096
	for index in 4096:
		batch.multimesh.set_instance_transform(index, Transform3D(Basis.from_scale(Vector3.ONE * 0.025), Vector3((index % 64 - 31.5) * 0.04, (index / 64 - 31.5) * 0.04, 0)))
		batch.multimesh.set_instance_custom_data(index, Color(4, 24.5, 6, 1))
	batch.visible = true
	preview.visible = false
	var baseline := source.duplicate() as StandardMaterial3D
	baseline.emission_enabled = true
	baseline.emission = Windows.REFERENCE_COLOR.linear_to_srgb()
	baseline.emission_operator = BaseMaterial3D.EMISSION_OP_MULTIPLY
	RenderingServer.viewport_set_measure_render_time(viewport.get_viewport_rid(), true)
	for candidate in [false, true, false, true]:
		batch.material_override = material if candidate else baseline
		var gpu: Array[float] = []
		var cpu: Array[float] = []
		for frame in 360:
			var start := Time.get_ticks_usec()
			RenderingServer.global_shader_parameter_set("scene_window_clock", Vector2(fposmod(21.0 + frame * 0.02, 24.0), -5))
			var elapsed := Time.get_ticks_usec() - start
			await process_frame
			await RenderingServer.frame_post_draw
			if frame >= 60:
				gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(viewport.get_viewport_rid()))
				cpu.append(elapsed)
		gpu.sort()
		cpu.sort()
		print("WINDOW_BENCH ", JSON.stringify({"candidate": candidate, "instances": 4096, "gpu_median_ms": gpu[150], "clock_median_us": cpu[150]}))
