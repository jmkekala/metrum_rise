# SPDX-License-Identifier: GPL-2.0-only

## Rendered road-paint regression: darkness, daylight and local light reception.
## --benchmark-markings compares the former unshaded material with lit paint.
extends SceneTree

const Network := preload("res://scripts/tools/network_tool.gd")
var failures := 0
var viewport: SubViewport
var material: StandardMaterial3D
var camera: Camera3D

func _initialize() -> void:
	call_deferred("run")

func expect(condition: bool, message: String) -> void:
	if not condition:
		failures += 1
		push_error(message)

func settle(frames: int = 32) -> void:
	for frame in frames:
		await process_frame
		await RenderingServer.frame_post_draw

func samples() -> Vector2:
	var image := viewport.get_texture().get_image()
	return Vector2(
		image.get_pixelv(Vector2i(camera.unproject_position(Vector3(-1, 0, 0)))).get_luminance(),
		image.get_pixelv(Vector2i(camera.unproject_position(Vector3(1, 0, 0)))).get_luminance())

func run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("road_marking_lighting_test requires GPU rendering")
		quit(1)
		return
	var network := Network.new()
	network._ensure_road_mesh_materials()
	material = network._marking_mat
	network.free()
	viewport = SubViewport.new()
	viewport.size = Vector2i(640, 360)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var world := WorldEnvironment.new()
	world.environment = load("res://default_environment.tres").duplicate()
	world.environment.background_color = Color.BLACK
	world.environment.ambient_light_source = Environment.AMBIENT_SOURCE_DISABLED
	viewport.add_child(world)
	camera = Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 8
	camera.position = Vector3(0, 6, 0)
	camera.rotation_degrees.x = -90
	viewport.add_child(camera)
	for index in 2:
		var plane := PlaneMesh.new()
		plane.size = Vector2(1.5, 3)
		var arrays := plane.get_mesh_arrays()
		var colours := PackedColorArray()
		colours.resize(arrays[Mesh.ARRAY_VERTEX].size())
		colours.fill(Color.WHITE if index == 0 else Color(1, 0.8, 0))
		arrays[Mesh.ARRAY_COLOR] = colours
		var mesh := ArrayMesh.new()
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		var instance := MeshInstance3D.new()
		instance.mesh = mesh
		instance.material_override = material
		instance.position.x = -1 if index == 0 else 1
		viewport.add_child(instance)
	var sun := DirectionalLight3D.new()
	sun.rotation_degrees.x = -90
	sun.light_energy = 0
	viewport.add_child(sun)
	await settle()
	var dark := samples()
	sun.light_energy = 1
	await settle()
	var day := samples()
	sun.light_energy = 0.02
	await settle()
	var night := samples()
	sun.light_energy = 0
	var lamp := OmniLight3D.new()
	lamp.position = Vector3(0, 2, 0)
	lamp.omni_range = 8
	lamp.light_energy = 4
	viewport.add_child(lamp)
	await settle()
	var local := samples()
	for index in 2:
		expect(dark[index] < 0.01, "paint must not emit in darkness")
		expect(day[index] > 0.1, "paint must remain visible in daylight")
		expect(night[index] < day[index] * 0.5, "paint must dim with night lighting")
		expect(local[index] > dark[index] + 0.05, "paint must receive local lights")
	print("MARKING_SAMPLES dark=", dark, " day=", day, " night=", night, " local=", local)
	if "--benchmark-markings" in OS.get_cmdline_user_args():
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
		RenderingServer.viewport_set_measure_render_time(viewport.get_viewport_rid(), true)
		sun.light_energy = 1
		for size in [Vector2i(640, 360), Vector2i(1920, 1080)]:
			viewport.size = size
			for lit in [false, true, false, true]:
				material.shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL if lit else BaseMaterial3D.SHADING_MODE_UNSHADED
				material.roughness = 1.0 if lit else 0.5
				await settle(60)
				var gpu: Array[float] = []
				for frame in 120:
					await settle(1)
					gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(viewport.get_viewport_rid()))
				gpu.sort()
				print("MARKING_BENCH size=", size, " lit=", lit, " median_ms=", gpu[60])
	viewport.free()
	print("road_marking_lighting_test: ", "PASS" if failures == 0 else "FAIL")
	quit(failures)
