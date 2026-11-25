# SPDX-License-Identifier: GPL-3.0-or-later

## Rendered (compatibility): the ground's relief light (`RENDER-78`). The terrain shader is drawn
## on a sloped patch and on a flat one under a low sun, beside a copy whose light() uses the plain
## term, mix(0.82, 1, N.L), written out. Pinned: the shipped default is (1, add); strength 0
## draws the plain term's bytes in every form, so the light from before stays reachable; at the
## default and at the console's ratio 2 the slope facing away darkens, but never below its sky
## light (the ratio's extrapolated factor is floored at 0), and flat ground keeps the plain light.
## The Graphics setting drives the same global: each stored preset, applied, draws what the global
## set directly to it draws (a headless run cannot read the global back).
extends SceneTree

const Settings := preload("res://scripts/core/game_settings.gd")
const SURFACE := "res://scripts/shaders/terrain_surface.gdshaderinc"
const WRAPPER := "res://assets/materials/terrain.gdshader"
const RELIEF_CALL := "terrain_relief_facing(mix(0.82, 1.0, facing), facing, LIGHT, INV_VIEW_MATRIX)"
const PLAIN_TERM := "mix(0.82, 1.0, facing)"
const GLOBAL := "terrain_relief_light"
# The user's pick, 2026-11-25: add form, strength 1, sun and moon.
const SHIPPED := Vector2(1.0, 0.0)
# Ratio past strength 1, reachable from the console: its factor must not go negative.
const RATIO_2 := Vector2(2.0, 1.0)

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

# The plain shader: the wrapper with the surface include inlined and the relief call replaced by
# the term it wraps. Every other include is read from its file, as the real shader reads it.
func _plain_shader() -> Shader:
	var surface := FileAccess.get_file_as_string(SURFACE)
	_expect(surface.count(RELIEF_CALL) == 1, "light() must call the relief term exactly once")
	var code := FileAccess.get_file_as_string(WRAPPER).replace(
		'#include "%s"' % SURFACE, surface.replace(RELIEF_CALL, PLAIN_TERM))
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
	material.set_shader_parameter("ground_shadow_ambient", 0.3)
	material.set_shader_parameter("ground_shadow_sun_strength", 1.0)
	return material

func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("This regression needs a rendering display, such as gamescope with OpenGL compatibility.")
		quit(2)
		return
	var default: Variant = ProjectSettings.get_setting("shader_globals/%s" % GLOBAL, {}).get("value")
	_expect(default == SHIPPED, "the relief light's project default must be %s (add at 1, sun and moon): %s" % [SHIPPED, default])
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
	var plain := _plain_shader()
	var shots := {}
	for ground in ["slope", "flat"]:
		var heightmap := _heightmap(4.0 if ground == "slope" else 0.0)
		mesh.material_override = _material(plain, heightmap)
		shots["%s/plain" % ground] = await _capture(viewport)
		sun.visible = false
		shots["%s/no sun" % ground] = await _capture(viewport)
		sun.visible = true
		mesh.material_override = _material(relief, heightmap)
		for setting in [Vector2(0.0, 0.0), Vector2(0.0, 1.0), Vector2(0.0, 3.0), SHIPPED, RATIO_2]:
			RenderingServer.global_shader_parameter_set(GLOBAL, setting)
			shots["%s/%s" % [ground, setting]] = await _capture(viewport)
	# Each stored preset, applied as the game applies it, against the global set directly.
	var heightmap_slope := _heightmap(4.0)
	mesh.material_override = _material(relief, heightmap_slope)
	var had_config := FileAccess.file_exists(Settings.CFG_PATH)
	var original := FileAccess.get_file_as_bytes(Settings.CFG_PATH) if had_config else PackedByteArray()
	Settings.terrain_relief_override = null
	for preset: float in Settings.TERRAIN_RELIEF_CHOICES:
		RenderingServer.global_shader_parameter_set(GLOBAL, Vector2(preset, 0.0))
		var direct := await _capture(viewport)
		RenderingServer.global_shader_parameter_set(GLOBAL, Vector2(9.0, 1.0))
		var config := Settings.load_config()
		config.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_RELIEF, preset)
		Settings.save_config(config)
		Settings.apply_terrain_relief()
		var applied := await _capture(viewport)
		_expect(applied.get_data() == direct.get_data(), "the stored preset %s must draw what the global at it draws" % preset)
	if had_config:
		var file := FileAccess.open(Settings.CFG_PATH, FileAccess.WRITE)
		file.store_buffer(original)
		file.close()
	else:
		DirAccess.remove_absolute(Settings.CFG_PATH)
	RenderingServer.global_shader_parameter_set(GLOBAL, SHIPPED)
	for ground in ["slope", "flat"]:
		var plain_bytes: PackedByteArray = shots["%s/plain" % ground].get_data()
		for setting in [Vector2(0.0, 0.0), Vector2(0.0, 1.0), Vector2(0.0, 3.0)]:
			_expect(shots["%s/%s" % [ground, setting]].get_data() == plain_bytes,
				"%s ground at relief %s must draw the plain term's bytes" % [ground, setting])
		var centre_plain: Color = shots["%s/plain" % ground].get_pixel(32, 32)
		var centre_no_sun: Color = shots["%s/no sun" % ground].get_pixel(32, 32)
		for setting in [SHIPPED, RATIO_2]:
			var centre: Color = shots["%s/%s" % [ground, setting]].get_pixel(32, 32)
			print("terrain_relief %s %s plain=%s relief=%s no_sun=%s" % [ground, setting, centre_plain, centre, centre_no_sun])
			if ground == "slope":
				_expect(centre.g < centre_plain.g - 0.01, "a slope facing away from the sun must darken at %s" % [setting])
				_expect(centre.g >= centre_no_sun.g - 1.5 / 255.0, "the sun must never take light away (%s)" % [setting])
			else:
				_expect(shots["%s/%s" % [ground, setting]].get_data() == plain_bytes,
					"flat ground must keep the plain light at %s" % [setting])
	if _failures == 0:
		print("terrain_relief_light_test: PASS")
	quit(1 if _failures else 0)
