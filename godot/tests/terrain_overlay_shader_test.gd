# SPDX-License-Identifier: GPL-2.0-only

## Actual terrain shader intensity regression; requires a rendering display.
extends SceneTree

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _texture(color: Color) -> ImageTexture:
	var image := Image.create(2, 2, false, Image.FORMAT_RGBA8)
	image.fill(color)
	return ImageTexture.create_from_image(image)

func _capture(viewport: SubViewport) -> Image:
	await process_frame
	await RenderingServer.frame_post_draw
	return viewport.get_texture().get_image()

func _rgb(color: Color) -> Vector3:
	return Vector3(color.r, color.g, color.b)

func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("This regression needs a rendering display, such as xvfb-run with OpenGL compatibility.")
		quit(2)
		return
	var viewport := SubViewport.new()
	viewport.size = Vector2i(64, 64)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var camera := Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 2.4
	viewport.add_child(camera)
	camera.position = Vector3(0.0, 10.0, 0.0)
	camera.look_at(Vector3.ZERO, Vector3(0.0, 0.0, -1.0))
	camera.current = true
	var material := ShaderMaterial.new()
	material.shader = load("res://assets/materials/terrain.gdshader")
	material.set_shader_parameter("height_is_baked", true)
	material.set_shader_parameter("heightmap", _texture(Color.BLACK))
	material.set_shader_parameter("watermap", _texture(Color.BLACK))
	material.set_shader_parameter("terrain_grass_albedo", _texture(Color(0.25, 0.45, 0.15)))
	material.set_shader_parameter("terrain_grass_height", _texture(Color(0.5, 0.5, 0.5)))
	material.set_shader_parameter("world_size", Vector2(2.0, 2.0))
	material.set_shader_parameter("patch_world_size_m", Vector2(2.0, 2.0))
	material.set_shader_parameter("ground_shadow_ambient", 1.0)
	var mesh := MeshInstance3D.new()
	mesh.mesh = PlaneMesh.new()
	mesh.material_override = material
	viewport.add_child(mesh)
	for mode in [1, 2, 3]:
		material.set_shader_parameter("overlay_mode", mode)
		material.set_shader_parameter("overlay_texture", _texture(Color(1.0, 0.2, 0.2, 0.0)))
		var base_image := await _capture(viewport)
		var base := base_image.get_pixel(32, 32)
		var previous := 0.0
		for intensity in [0.2, 0.4, 200.0 / 255.0]:
			material.set_shader_parameter("overlay_texture", _texture(Color(1.0, 0.2, 0.2, intensity)))
			var image := await _capture(viewport)
			var color := image.get_pixel(32, 32)
			var distance := _rgb(color).distance_to(_rgb(base))
			print("terrain_overlay mode=%d intensity=%.6f color=%s distance=%.6f" % [mode, intensity, color, distance])
			_expect(distance > previous + 0.01, "higher environmental intensity must visibly strengthen the overlay")
			previous = distance
	material.set_shader_parameter("overlay_mode", 1)
	var edges := Image.create(2, 1, false, Image.FORMAT_RGBA8)
	edges.set_pixel(0, 0, Color(1.0, 0.0, 0.0, 200.0 / 255.0))
	edges.set_pixel(1, 0, Color(0.0, 1.0, 0.0, 200.0 / 255.0))
	material.set_shader_parameter("overlay_texture", ImageTexture.create_from_image(edges))
	var edge_image := await _capture(viewport)
	# These pairs lie between each world edge and its nearest texture centre.
	for pair in [Vector2i(7, 15), Vector2i(56, 48)]:
		var edge := edge_image.get_pixel(pair.x, 32)
		var interior := edge_image.get_pixel(pair.y, 32)
		var difference := _rgb(edge).distance_to(_rgb(interior))
		print("terrain_overlay_edge pixels=%s difference=%.6f" % [pair, difference])
		_expect(difference < 0.02, "world edges must clamp to their adjacent overlay texel")
	viewport.queue_free()
	await process_frame
	print("Terrain overlay shader tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
