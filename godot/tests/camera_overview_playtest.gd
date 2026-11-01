# SPDX-License-Identifier: GPL-2.0-only

## Opt-in rendered overview sweep using the real gameplay scene; never saves the loaded world.
## METRUM_OVERVIEW_WORLD and METRUM_OVERVIEW_OUTPUT select the input and a fresh artifact folder.
## METRUM_OVERVIEW_CLOUDS=1 adds cloud-layer readbacks and verifies retained ground shadows.
## METRUM_OVERVIEW_VIDEO_ONLY=1 records a 30 FPS PNG sequence instead of timing samples.
## Reuses the day-cycle capture's bounded terrain/water/vegetation settling check.
extends "res://tests/day_cycle_capture.gd"

func run() -> void:
	output_dir = OS.get_environment("METRUM_OVERVIEW_OUTPUT")
	if output_dir.is_empty() or DirAccess.dir_exists_absolute(output_dir):
		push_error("Set METRUM_OVERVIEW_OUTPUT to a fresh directory")
		quit(1)
		return
	DirAccess.make_dir_recursive_absolute(output_dir)
	main = load("res://scenes/Main.tscn").instantiate()
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	var input := main.get_node("InputManager")
	input.set_process(false)
	input.set_process_input(false)
	input.set_process_unhandled_input(false)
	root.set_disable_input(true)
	if not input.menu_load_world_definition(OS.get_environment("METRUM_OVERVIEW_WORLD")):
		push_error("Overview world failed to load")
		quit(1)
		return
	input.set_simulation_speed(0.0)
	var sim := main.get_node("SimulationNode")
	sim.set_weather_scenario(1, 0.0)
	camera = main.get_node("CameraNode")
	camera.set_process(false)
	terrain = main.get_node("Terrain")
	water = main.get_node("Water")
	vegetation = main.get_node("Vegetation")
	lighting = main.get_node("SceneLighting")
	lighting.pin_hour_of_day(10.0)
	pivot.y = sim.get_world_surface_height(Vector2.ZERO) + 0.25
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0
	RenderingServer.viewport_set_measure_render_time(root.get_viewport_rid(), true)
	var poses := [400.0, 1800.0, 2200.0, 10000.0, 12000.0, 30000.0, 40000.0, 400.0]
	if OS.get_environment("METRUM_OVERVIEW_VIDEO_ONLY") == "1":
		camera.focus_on(pivot, 160.0)
		if not await _settle():
			return
		await _record_zoom()
		quit(0)
		return
	for index in poses.size():
		var distance: float = poses[index]
		camera.zoom(log(float(camera.get_view_state()[5]) / distance) / log(camera.zoom_speed))
		# Exercise the same response while renderers stream the changing view.
		var transition_gpu: Array[float] = []
		for frame in 90:
			camera.advance_motion(1.0 / 60.0)
			await process_frame
			transition_gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(root.get_viewport_rid()))
		transition_gpu.sort()
		if not await _settle():
			return
		var samples: Array[float] = []
		var gpu: Array[float] = []
		var previous := Time.get_ticks_usec()
		for frame in 120:
			await process_frame
			var now := Time.get_ticks_usec()
			samples.append(float(now - previous) / 1000.0)
			previous = now
			gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(root.get_viewport_rid()))
		samples.sort()
		gpu.sort()
		await RenderingServer.frame_post_draw
		root.get_texture().get_image().save_png(output_dir.path_join("%02d-%05d.png" % [index, distance]))
		records.append({"distance": camera.get_view_state()[5], "eye": camera.global_position,
			"world_size": sim.get_terrain_world_size(), "frame_median_ms": samples[60],
			"gpu_median_ms": gpu[60], "transition_gpu_p95_ms": transition_gpu[85], "far": camera.far})
	# Push beyond each edge at maximum zoom and verify the map focus remains visible.
	camera.focus_on(pivot, 16000.0)
	var half_size: Vector2 = sim.get_terrain_world_size() * 0.5
	for direction in [Vector3.RIGHT, Vector3.FORWARD, Vector3.LEFT, Vector3.BACK]:
		for frame in 90:
			camera.pan(direction, 1.0)
			camera.advance_motion(1.0 / 60.0)
			await process_frame
		var focus: Vector3 = camera.get_focus_position()
		if absf(focus.x) > half_size.x or absf(focus.z) > half_size.y or camera.is_position_behind(focus):
			push_error("Overview navigation lost map focus")
			quit(1)
			return
	if not await _settle():
		return
	await RenderingServer.frame_post_draw
	root.get_texture().get_image().save_png(output_dir.path_join("bounded-pan.png"))
	if OS.get_environment("METRUM_OVERVIEW_CLOUDS") == "1":
		if not await _cloud_layers():
			quit(1)
			return
	var file := FileAccess.open(output_dir.path_join("results.json"), FileAccess.WRITE)
	file.store_string(JSON.stringify(records, "  "))
	file.close()
	quit(0)

func _record_zoom() -> void:
	var directory := output_dir.path_join("frames")
	DirAccess.make_dir_recursive_absolute(directory)
	var notch := log(40000.0 / 400.0) / log(float(camera.zoom_speed)) / 28.0
	for frame in 540:
		if frame >= 60 and frame < 228 and frame % 6 == 0:
			camera.zoom(-notch)
		elif frame >= 300 and frame < 468 and frame % 6 == 0:
			camera.zoom(notch)
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		if frame % 2 == 0:
			await RenderingServer.frame_post_draw
			root.get_texture().get_image().save_png(directory.path_join("%05d.png" % (frame >> 1)))

func _cloud_layers() -> bool:
	var first_shadow := 0
	# Sample the GPU-only shadow texture through a tiny viewport; production textures
	# deliberately have no CPU readback flag. Keep this outside all timing windows.
	var shadow_view := SubViewport.new()
	shadow_view.size = Vector2i(64, 64)
	shadow_view.disable_3d = true
	shadow_view.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(shadow_view)
	var rectangle := ColorRect.new()
	rectangle.size = Vector2(64, 64)
	var material := ShaderMaterial.new()
	material.shader = Shader.new()
	material.shader.code = "shader_type canvas_item; uniform sampler2D shadow_map; void fragment() { COLOR = vec4(vec3(texture(shadow_map, UV).r), 1.0); }"
	material.set_shader_parameter("shadow_map", lighting._cloud_effect.shadow_texture)
	rectangle.material = material
	shadow_view.add_child(rectangle)
	for altitude in [1000.0, 1350.0, 1500.0, 6500.0, 7250.0, 8000.0, 42000.0, 1000.0]:
		# Horizon-facing diagnostic poses exercise sky pixels on both sides of each layer.
		camera.global_position = Vector3(0, altitude, 0)
		camera.rotation = Vector3.ZERO
		for frame in 30:
			await process_frame
		await RenderingServer.frame_post_draw
		var cloud: Image = lighting._cloud_effect.reflection_texture.get_image()
		var shadow: Image = shadow_view.get_texture().get_image()
		var shadow_hash := hash(shadow.get_data())
		if first_shadow == 0:
			first_shadow = shadow_hash
		var minimum_transmission := 1.0
		for y in cloud.get_height():
			for x in cloud.get_width():
				minimum_transmission = minf(minimum_transmission, cloud.get_pixel(x, y).a)
		if (altitude >= 8000.0 and minimum_transmission < 0.999) or (altitude == 1000.0 and minimum_transmission >= 0.99) or shadow_hash != first_shadow:
			push_error("Cloud visibility/shadow regression at altitude %s, transmission %s" % [altitude, minimum_transmission])
			shadow_view.queue_free()
			return false
		root.get_texture().get_image().save_png(output_dir.path_join("cloud-%05d.png" % altitude))
		records.append({"cloud_altitude": altitude, "minimum_transmission": minimum_transmission, "shadow_hash": shadow_hash})
	shadow_view.queue_free()
	return true
