# SPDX-License-Identifier: GPL-3.0-or-later

## Rendered (compatibility): the terrain shader's backdrop variant (`TERRAIN-41` D2). One cell (a
## 512 m patch on the 64 m lattice, `TerrainBackdrop`'s mesh) drawn by a MultiMesh with its custom
## lane, as the StreamNode draws the cells, seen from above under a low sun. Pinned: the variant
## compiles and draws; it draws flat water where the block's water surface lies above its ground
## and ground elsewhere; it lights with the terrain's own light(): the relief term darkens a slope
## facing away from the sun and leaves flat ground as strength 0 draws it.
extends SceneTree

const BACKDROP_SHADER := "res://assets/materials/terrain_backdrop.gdshader"
const TerrainBackdrop := preload("res://scripts/renderers/terrain_backdrop.gd")
const GLOBAL := "terrain_relief_light"
const LATTICE := 8
const SPAN_M := 512.0

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

# A block channel of (LATTICE + 1)^2 lattice points: `value(x, z)` metres.
func _channel(value: Callable) -> ImageTexture:
	var image := Image.create(LATTICE + 1, LATTICE + 1, false, Image.FORMAT_RF)
	for z in LATTICE + 1:
		for x in LATTICE + 1:
			image.set_pixel(x, z, Color(float(value.call(x, z)), 0.0, 0.0))
	return ImageTexture.create_from_image(image)

func _material(ground: Callable, water: Callable) -> ShaderMaterial:
	var grass := Image.create(2, 2, false, Image.FORMAT_RGBA8)
	grass.fill(Color(0.6, 0.8, 0.4))
	var material := ShaderMaterial.new()
	material.shader = load(BACKDROP_SHADER)
	material.set_shader_parameter("heightmap", _channel(ground))
	material.set_shader_parameter("watermap", _channel(water))
	material.set_shader_parameter("terrain_cell_m", SPAN_M / LATTICE)
	material.set_shader_parameter("patch_world_size_m", Vector2(SPAN_M, SPAN_M))
	material.set_shader_parameter("terrain_grass_albedo", ImageTexture.create_from_image(grass))
	material.set_shader_parameter("world_size", Vector2(SPAN_M, SPAN_M))
	material.set_shader_parameter("ground_shadow_ambient", 0.3)
	material.set_shader_parameter("ground_shadow_sun_strength", 1.0)
	return material

func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("This regression needs a rendering display, such as gamescope with OpenGL compatibility.")
		quit(2)
		return
	var viewport := SubViewport.new()
	viewport.size = Vector2i(64, 64)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var camera := Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = SPAN_M * 0.75
	camera.far = 4000.0
	viewport.add_child(camera)
	camera.position = Vector3(0.0, 2000.0, 0.0)
	camera.look_at(Vector3.ZERO, Vector3(0.0, 0.0, -1.0))
	camera.current = true
	# A low sun from +x: a slope rising along +x faces away from it.
	var sun := DirectionalLight3D.new()
	sun.light_energy = 4.0
	viewport.add_child(sun)
	sun.look_at_from_position(Vector3.ZERO, Vector3(-cos(deg_to_rad(20.0)), -sin(deg_to_rad(20.0)), 0.0))
	# One cell, its corner at (-256, -256): the StreamNode's row (translation, then the patch's
	# place in its block and its size).
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.use_custom_data = true
	multimesh.mesh = TerrainBackdrop._cell_mesh(LATTICE)
	multimesh.instance_count = 1
	multimesh.set_instance_transform(0, Transform3D(Basis.IDENTITY, Vector3(-SPAN_M * 0.5, 0.0, -SPAN_M * 0.5)))
	multimesh.set_instance_custom_data(0, Color(0.0, 0.0, SPAN_M, SPAN_M))
	multimesh.custom_aabb = AABB(Vector3(-SPAN_M, -500.0, -SPAN_M), Vector3(SPAN_M * 2.0, 3000.0, SPAN_M * 2.0))
	var cell := MultiMeshInstance3D.new()
	cell.multimesh = multimesh
	viewport.add_child(cell)
	var flat := func(_x: int, _z: int) -> float: return 10.0
	var slope := func(x: int, _z: int) -> float: return 10.0 + x * 32.0
	# The lake: the western half's water surface 5 m above its ground; dry points equal the ground.
	var lake := func(x: int, _z: int) -> float: return 15.0 if x < LATTICE / 2 else 10.0
	cell.material_override = _material(flat, lake)
	RenderingServer.global_shader_parameter_set(GLOBAL, Vector2(1.0, 0.0))
	var water := await _capture(viewport)
	var wet: Color = water.get_pixel(12, 32)
	var dry: Color = water.get_pixel(52, 32)
	print("terrain_backdrop water=%s ground=%s" % [wet, dry])
	_expect(wet.b > wet.g and wet.b > wet.r, "a wet lattice point must draw the flat water colour: %s" % wet)
	_expect(dry.g > dry.b, "a dry lattice point must draw ground: %s" % dry)
	# The relief term of the terrain's light(): a slope facing away darkens, flat ground does not.
	var shots := {}
	for ground in ["slope", "flat"]:
		cell.material_override = _material(slope if ground == "slope" else flat, slope if ground == "slope" else flat)
		for setting in [Vector2(0.0, 0.0), Vector2(1.0, 0.0)]:
			RenderingServer.global_shader_parameter_set(GLOBAL, setting)
			shots["%s/%s" % [ground, setting]] = await _capture(viewport)
	RenderingServer.global_shader_parameter_set(GLOBAL, Vector2(1.0, 0.0))
	var plain: Color = shots["slope/(0.0, 0.0)"].get_pixel(32, 32)
	var relief: Color = shots["slope/(1.0, 0.0)"].get_pixel(32, 32)
	print("terrain_backdrop slope plain=%s relief=%s" % [plain, relief])
	_expect(relief.g < plain.g - 0.01, "the backdrop's slope facing away must darken under the relief term")
	_expect(shots["flat/(0.0, 0.0)"].get_data() == shots["flat/(1.0, 0.0)"].get_data(),
		"flat backdrop ground must keep the plain light under the relief term")
	if _failures == 0:
		print("terrain_backdrop_shader_test: PASS")
	quit(1 if _failures else 0)
