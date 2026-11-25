# SPDX-License-Identifier: GPL-3.0-or-later

## Rendered (compatibility): the ground's relief light (`RENDER-78`) is today's light by default.
## The terrain shader is drawn on a sloped patch and on a flat one under a low sun, beside a copy
## whose light() uses today's term, mix(0.82, 1, N.L), written out. With the global at its
## project default, and at strength 0 in every form, the two draw the same bytes; at strength 1
## the slope changes and flat ground does not, so the comparison is not blind.
extends SceneTree

const SURFACE := "res://scripts/shaders/terrain_surface.gdshaderinc"
const WRAPPER := "res://assets/materials/terrain.gdshader"
const RELIEF_CALL := "terrain_relief_facing(mix(0.82, 1.0, facing), facing, LIGHT, INV_VIEW_MATRIX)"
const TODAY_TERM := "mix(0.82, 1.0, facing)"
const GLOBAL := "terrain_relief_light"

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _capture(viewport: SubViewport) -> Image:
	await process_frame
	await process_frame
	await RenderingServer.frame_post_draw
	return viewport.get_texture().get_image()

# Today's shader: the wrapper with the surface include inlined and the relief call replaced by
# the term it wraps. Every other include is read from its file, as the real shader reads it.
func _today_shader() -> Shader:
	var surface := FileAccess.get_file_as_string(SURFACE)
	_expect(surface.count(RELIEF_CALL) == 1, "light() must call the relief term exactly once")
	var code := FileAccess.get_file_as_string(WRAPPER).replace(
		'#include "%s"' % SURFACE, surface.replace(RELIEF_CALL, TODAY_TERM))
	var shader := Shader.new()
	shader.code = code
	return shader

# A heightmap rising `rise_m` a texel along x: slope rise / 8 m.
func _heightmap(rise_m: float) -> ImageTexture:
	var image := Image.create(8, 8, false, Image.FORMAT_RF)
	for y in 8:
		for x in 8:
			image.set_pixel(x, y, Color(x * rise_m, 0.0, 0.0))
	return ImageTexture.create_from_image(image)

func _material(shader: Shader, heightmap: Texture2D) -> ShaderMaterial:
	var flat := Image.create(2, 2, false, Image.FORMAT_RGBA8)
	flat.fill(Color(0.6, 0.8, 0.4))
	var material := ShaderMaterial.new()
	material.shader = shader
	material.set_shader_parameter("heightmap", heightmap)
	material.set_shader_parameter("heightmap_texture_size", Vector2(8.0, 8.0))
	material.set_shader_parameter("inner_sample_offset_texels", Vector2(2.0, 2.0))
	material.set_shader_parameter("inner_sample_size_texels", Vector2(4.0, 4.0))
	material.set_shader_parameter("terrain_grass_albedo", ImageTexture.create_from_image(flat))
	var dry := Image.create(2, 2, false, Image.FORMAT_RGBA8)
	dry.fill(Color.BLACK)
	material.set_shader_parameter("watermap", ImageTexture.create_from_image(dry))
	material.set_shader_parameter("world_size", Vector2(24.0, 24.0))
	material.set_shader_parameter("patch_world_size_m", Vector2(24.0, 24.0))
	material.set_shader_parameter("ground_shadow_ambient", 0.0)
	material.set_shader_parameter("ground_shadow_sun_strength", 1.0)
	return material

func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("This regression needs a rendering display, such as gamescope with OpenGL compatibility.")
		quit(2)
		return
	var default: Variant = ProjectSettings.get_setting("shader_globals/%s" % GLOBAL, {}).get("value")
	_expect(default == Vector2.ZERO, "the relief light's project default must be (0, 0), today's light: %s" % [default])
	var viewport := SubViewport.new()
	viewport.size = Vector2i(64, 64)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var camera := Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 12.0
	viewport.add_child(camera)
	camera.position = Vector3(0.0, 200.0, 0.0)
	camera.look_at(Vector3.ZERO, Vector3(0.0, 0.0, -1.0))
	camera.current = true
	# A low sun from +x, so the slope (rising along +x) faces away from it.
	var sun := DirectionalLight3D.new()
	# Bright enough that the term is not lost in the capture's eight bits.
	sun.light_energy = 4.0
	viewport.add_child(sun)
	sun.look_at_from_position(Vector3.ZERO, Vector3(-cos(deg_to_rad(20.0)), -sin(deg_to_rad(20.0)), 0.0))
	var mesh := MeshInstance3D.new()
	var plane := PlaneMesh.new()
	plane.size = Vector2(24.0, 24.0)
	plane.subdivide_width = 6
	plane.subdivide_depth = 6
	mesh.mesh = plane
	viewport.add_child(mesh)
	var relief: Shader = load(WRAPPER)
	var today := _today_shader()
	var shots := {}
	for ground in ["slope", "flat"]:
		var heightmap := _heightmap(4.0 if ground == "slope" else 0.0)
		mesh.material_override = _material(today, heightmap)
		RenderingServer.global_shader_parameter_set(GLOBAL, Vector2.ZERO)
		shots["%s/today" % ground] = await _capture(viewport)
		mesh.material_override = _material(relief, heightmap)
		for setting in [Vector2(0.0, 0.0), Vector2(0.0, 1.0), Vector2(0.0, 3.0), Vector2(1.0, 0.0), Vector2(1.0, 1.0)]:
			RenderingServer.global_shader_parameter_set(GLOBAL, setting)
			shots["%s/%s" % [ground, setting]] = await _capture(viewport)
	RenderingServer.global_shader_parameter_set(GLOBAL, Vector2.ZERO)
	for ground in ["slope", "flat"]:
		var today_bytes: PackedByteArray = shots["%s/today" % ground].get_data()
		for setting in [Vector2(0.0, 0.0), Vector2(0.0, 1.0), Vector2(0.0, 3.0)]:
			_expect(shots["%s/%s" % [ground, setting]].get_data() == today_bytes,
				"%s ground at relief %s must draw today's bytes" % [ground, setting])
		var centre_today: Color = shots["%s/today" % ground].get_pixel(32, 32)
		for setting in [Vector2(1.0, 0.0), Vector2(1.0, 1.0)]:
			var centre: Color = shots["%s/%s" % [ground, setting]].get_pixel(32, 32)
			print("terrain_relief %s %s today=%s relief=%s" % [ground, setting, centre_today, centre])
			if ground == "slope":
				_expect(centre.g < centre_today.g - 0.01, "a slope facing away from the sun must darken at strength 1")
			else:
				_expect(shots["%s/%s" % [ground, setting]].get_data() == today_bytes,
					"flat ground must keep today's light at strength 1")
	if _failures == 0:
		print("terrain_relief_light_test: PASS")
	quit(1 if _failures else 0)
