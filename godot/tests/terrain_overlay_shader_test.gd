# SPDX-License-Identifier: GPL-2.0-only

## Actual terrain shader intensity regression; requires a rendering display.
extends SceneTree

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")

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

	# The forest floor loses the sky its crowns hide at every distance, and past the shadow
	# cascades it also loses the sun their shadows no longer take. The camera moves rather than
	# the ramp, so the shipped begin and end distances are what is measured, and covered ground
	# is compared only with covered ground, because the litter hue the coverage swaps in
	# changes the colour without changing its luminance. The scene has no light, so the
	# ground shows its sky term alone. The capture crushes dark values to black, so the sky
	# share is measured at one half rather than at the shipped constant, and the sky at one.
	var ambient := 1.0
	var sun := 0.5
	var sun_lost := sun * (1.0 - 0.02)
	material.set_shader_parameter("ground_shadow_ambient", ambient)
	material.set_shader_parameter("ground_shadow_sun_strength", sun)
	material.set_shader_parameter("canopy_floor_sun_visibility", 0.02)
	var sky_share := 0.5
	material.set_shader_parameter("overlay_mode", 0)
	material.set_shader_parameter("land_cover_world_bounds", Vector4(-1.0, -1.0, 2.0, 2.0))
	var floor_shade_samples := {}
	for cover in [0.0, 1.0]:
		material.set_shader_parameter("land_cover_texture", _texture(Color(cover, cover, cover)))
		for transmission in [1.0, sky_share]:
			material.set_shader_parameter("canopy_floor_sky_transmission", transmission)
			for strength in [0.0, 1.0]:
				material.set_shader_parameter("canopy_floor_shade", strength)
				for height in [10.0, 600.0]:
					camera.position = Vector3(0.0, height, 0.0)
					camera.look_at(Vector3.ZERO, Vector3(0.0, 0.0, -1.0))
					var shade_image := await _capture(viewport)
					# The compatibility capture holds linear values, so the shade terms read as ratios
					# of linear light without decoding.
					var shade := _rgb(shade_image.get_pixel(32, 32)).length()
					floor_shade_samples[Vector4(cover, transmission, strength, height)] = shade
					print(
						"terrain_floor_shade cover=%.1f sky=%.2f strength=%.1f height=%.0f shade=%.6f"
						% [cover, transmission, strength, height, shade]
					)
	var bare_near: float = floor_shade_samples[Vector4(0.0, sky_share, 1.0, 10.0)]
	var bare_far: float = floor_shade_samples[Vector4(0.0, sky_share, 1.0, 600.0)]
	var open_sky_near: float = floor_shade_samples[Vector4(1.0, 1.0, 1.0, 10.0)]
	var covered_near: float = floor_shade_samples[Vector4(1.0, sky_share, 1.0, 10.0)]
	var covered_far: float = floor_shade_samples[Vector4(1.0, sky_share, 1.0, 600.0)]
	var unshaded_near: float = floor_shade_samples[Vector4(1.0, sky_share, 0.0, 10.0)]
	var unshaded_far: float = floor_shade_samples[Vector4(1.0, sky_share, 0.0, 600.0)]
	_expect(
		absf(bare_far - bare_near) < 0.002,
		"ground with no crown over it must not darken with distance"
	)
	_expect(
		absf(covered_near - unshaded_near) < 0.002,
		"inside shadow range the tree shadows take the sun, so the far term may change nothing"
	)
	_expect(
		absf(covered_near - open_sky_near * sky_share) < 0.02,
		"a closed canopy must pass only its transmission share of the sky to its floor"
	)
	var expected := unshaded_far * (1.0 - sun_lost / (ambient * sky_share + sun))
	_expect(
		absf(covered_far - expected) < 0.02,
		"covered ground past the cascades must lose the sun its crowns would shade"
	)
	# Past the cascades a crown shades the ground down its sun ray, not the ground under it. Ground
	# open overhead must darken when a stand lies toward the sun and not when it lies away from it.
	# The coverage texels are 30 m wide over x = -20..100 m, open in the first, so the camera's
	# ground at x = 0 is under almost no cover and the caster the sun ray reaches 20 m up +x is
	# under most of it.
	var stand := Image.create(4, 1, false, Image.FORMAT_RGBA8)
	stand.fill(Color.WHITE)
	stand.set_pixel(0, 0, Color.BLACK)
	material.set_shader_parameter("land_cover_texture", ImageTexture.create_from_image(stand))
	material.set_shader_parameter("land_cover_world_bounds", Vector4(-20.0, -60.0, 120.0, 120.0))
	material.set_shader_parameter("canopy_floor_sky_transmission", 1.0)
	material.set_shader_parameter("canopy_floor_shade", 1.0)
	var sun_side := {}
	for sun_x in [0.8, -0.8]:
		RenderingServer.global_shader_parameter_set("scene_sun_direction", Vector3(sun_x, 0.6, 0.0))
		for height in [10.0, 600.0]:
			camera.position = Vector3(0.0, height, 0.0)
			camera.look_at(Vector3.ZERO, Vector3(0.0, 0.0, -1.0))
			var cast_image := await _capture(viewport)
			sun_side[Vector2(sun_x, height)] = _rgb(cast_image.get_pixel(32, 32)).length()
			print("terrain_floor_cast sun_x=%.1f height=%.0f shade=%.6f" % [sun_x, height, sun_side[Vector2(sun_x, height)]])
	RenderingServer.global_shader_parameter_set("scene_sun_direction",
		ProjectSettings.get_setting("shader_globals/scene_sun_direction")["value"])
	_expect(
		absf(sun_side[Vector2(0.8, 10.0)] - sun_side[Vector2(-0.8, 10.0)]) < 0.002,
		"inside shadow range the cascades cast the crown shadow, so the far term may change nothing"
	)
	_expect(
		sun_side[Vector2(0.8, 600.0)] < 0.9 * sun_side[Vector2(-0.8, 600.0)],
		"past the cascades a stand toward the sun must shade the open ground before it"
	)
	_expect(
		absf(sun_side[Vector2(-0.8, 600.0)] - sun_side[Vector2(-0.8, 10.0)]) < 0.002,
		"past the cascades a stand away from the sun must not shade the open ground before it"
	)
	material.set_shader_parameter("land_cover_world_bounds", Vector4(-1.0, -1.0, 2.0, 2.0))
	# Past the far range the ground stands in for the crowns, and the crowns see the open sky.
	# Under a closed canopy the floor's sky share must not reach them: it turned a dense stand
	# nearly black. The camera looks straight down, so the stand-in covers the whole view.
	material.set_shader_parameter("land_cover_texture", _texture(Color.WHITE))
	material.set_shader_parameter("canopy_floor_shade", 0.0)
	material.set_shader_parameter("canopy_far_begin_m", 0.0)
	material.set_shader_parameter("canopy_far_end_m", 1.0)
	camera.position = Vector3(0.0, 10.0, 0.0)
	camera.look_at(Vector3.ZERO, Vector3(0.0, 0.0, -1.0))
	var crown_sky := {}
	for transmission in [1.0, sky_share]:
		material.set_shader_parameter("canopy_floor_sky_transmission", transmission)
		var crown_image := await _capture(viewport)
		crown_sky[transmission] = _rgb(crown_image.get_pixel(32, 32)).length()
		print("terrain_far_crowns sky=%.2f shade=%.6f" % [transmission, crown_sky[transmission]])
	_expect(
		absf(crown_sky[sky_share] - crown_sky[1.0]) < 0.01 * crown_sky[1.0] + 0.002,
		"the crowns the far ground stands in for must see the open sky, not the floor's share"
	)
	material.set_shader_parameter("canopy_floor_shade", 1.0)
	viewport.queue_free()
	await process_frame
	print("Terrain overlay shader tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
