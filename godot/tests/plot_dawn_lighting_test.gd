# SPDX-License-Identifier: GPL-2.0-only

## Morning paving parity with the actual day-cycle sky, shadows and angled camera.
## Reuses the material fixture; --benchmark-dawn --baseline-shader=PATH measures GPU cost.
extends "res://tests/plot_material_lighting_test.gd"

const Lighting := preload("res://scripts/core/scene_lighting.gd")

func run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("plot_dawn_lighting_test requires GPU rendering")
		quit(1)
		return
	viewport = SubViewport.new()
	viewport.size = Vector2i(640, 480)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var world := WorldEnvironment.new()
	world.name = "WorldEnvironment"
	world.environment = load("res://default_environment.tres").duplicate()
	viewport.add_child(world)
	sun = DirectionalLight3D.new()
	sun.name = "DirectionalLight3D"
	viewport.add_child(sun)
	var lighting := Lighting.new()
	viewport.add_child(lighting)
	lighting.set_process(false)
	camera = Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 24
	camera.position = Vector3(0, 15, 20)
	viewport.add_child(camera)
	camera.look_at(Vector3.ZERO)
	surface = MeshInstance3D.new()
	surface.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	viewport.add_child(surface)
	terrain = Materials.flat_terrain_material().duplicate()
	Materials.apply_terrain_paving_parameters(terrain)
	var blocker := MeshInstance3D.new()
	var box := BoxMesh.new()
	box.size = Vector3(6, 8, 6)
	blocker.mesh = box
	blocker.position = Vector3(0, 4, -9)
	viewport.add_child(blocker)
	for hour: float in [5, 6, 7, 8, 12]:
		lighting.pin_hour_of_day(hour)
		# The live sky uses incremental radiance updates. Wait for its cubemap and SSIL.
		await settle(64)
		for kind: String in ["asphalt", "concrete"]:
			var frontage := await capture(terrain, Color.RED if kind == "asphalt" else Color.GREEN)
			var yard := await capture(Materials.site_surface_material(kind))
			var delta := difference(frontage, yard)
			print("DAWN_PAVING hour=", hour, " kind=", kind, " delta=", delta)
			expect(delta < 0.002, "frontage and yard must match with the live coloured sky")
			if kind == "asphalt":
				var sidewalk := await capture(Materials.road_sidewalk_material())
				expect(difference(frontage, sidewalk) < 0.002, "sidewalk must share the same morning lighting")
		# Put the comparison area fully in shadow: ambient/reflections must also agree alone.
		blocker.visible = false
		sun.light_energy = 0
		var shadow_frontage := await capture(terrain, Color.RED)
		var shadow_yard := await capture(Materials.site_asphalt_material())
		var shadow_delta := difference(shadow_frontage, shadow_yard)
		print("DAWN_INDIRECT hour=", hour, " delta=", shadow_delta)
		expect(shadow_delta < 0.002, "shadowed paving must agree under sky and ambient light")
		blocker.visible = true
	if "--benchmark-dawn" in OS.get_cmdline_user_args():
		var baseline_path := ""
		for argument in OS.get_cmdline_user_args():
			if argument.begins_with("--baseline-shader="):
				baseline_path = argument.trim_prefix("--baseline-shader=")
		if not FileAccess.file_exists(baseline_path):
			push_error("--benchmark-dawn requires --baseline-shader with the prior terrain shader")
			quit(1)
			return
		var before := terrain.duplicate() as ShaderMaterial
		var shader := Shader.new()
		shader.code = FileAccess.get_file_as_string(baseline_path)
		before.shader = shader
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
		RenderingServer.viewport_set_measure_render_time(viewport.get_viewport_rid(), true)
		viewport.size = Vector2i(1920, 1080)
		lighting.pin_hour_of_day(6)
		surface.mesh = mesh_with_tag(Color.RED)
		for old: bool in [true, false, true, false]:
			surface.material_override = before if old else terrain
			await settle(60)
			var gpu: Array[float] = []
			for frame in 120:
				await settle(1)
				gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(viewport.get_viewport_rid()))
			gpu.sort()
			print("DAWN_BENCH old=", old, " median_ms=", gpu[60])
	viewport.free()
	print("plot_dawn_lighting_test: ", "PASS" if failures == 0 else "FAIL")
	quit(failures)
