# SPDX-License-Identifier: GPL-2.0-only

## Graded terrain cells (TERRAIN-07-D) through the real shaders: a GL0 cell whose slot names a
## road seam is discarded exactly on the road's side and drawn elsewhere, and a base cell of the
## base variant draws the whole tile. Requires a rendering display.
extends SceneTree

const GradedTiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const SIDE := 131
const ROW_TEXELS := 768
const ROAD_EDGE_M := 20.0
const BACKGROUND := Color(1.0, 0.0, 1.0)

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _texture(color: Color) -> ImageTexture:
	var image := Image.create(2, 2, false, Image.FORMAT_RF)
	image.fill(color)
	return ImageTexture.create_from_image(image)

# One GL0 slot: flat, the road west of x = 20 m (sdf negative there), seam 0 its edge, road on
# its left (from z = 0 to z = 64).
func _slot_pools() -> Array[Texture2DArray]:
	var texels := Image.create_empty(SIDE, SIDE, false, Image.FORMAT_RGBAF)
	for j in SIDE:
		for i in SIDE:
			var x := float(i - 1) * 0.5
			var sdf := clampf(x - ROAD_EDGE_M, -2.0, 2.0)
			texels.set_pixel(i, j, Color(0.0, sdf, 0.0 if absf(sdf) < 2.0 else -1.0, 0.0))
	var rows := Image.create_empty(ROW_TEXELS, 1, false, Image.FORMAT_RGBAF)
	rows.set_pixel(0, 0, Color(ROAD_EDGE_M, 0.0, ROAD_EDGE_M, 64.0))
	var texel_array := Texture2DArray.new()
	texel_array.create_from_images([texels])
	var row_array := Texture2DArray.new()
	row_array.create_from_images([rows])
	return [texel_array, row_array]

func _material(shader_path: String, pools: Array[Texture2DArray]) -> ShaderMaterial:
	var material := ShaderMaterial.new()
	material.shader = load(shader_path)
	material.set_shader_parameter("heightmap", _texture(Color.BLACK))
	material.set_shader_parameter("watermap", _texture(Color.BLACK))
	material.set_shader_parameter("heightmap_texture_size", Vector2(2, 2))
	material.set_shader_parameter("terrain_cell_m", 10.0)
	material.set_shader_parameter("world_size", Vector2(64.0, 64.0))
	material.set_shader_parameter("patch_world_size_m", Vector2(64.0, 64.0))
	material.set_shader_parameter("ground_shadow_ambient", 1.0)
	material.set_shader_parameter("graded_texels", pools[0])
	material.set_shader_parameter("graded_rows", pools[1])
	return material

func _cells(mesh: Mesh, material: ShaderMaterial, custom: Color, mode: float) -> MultiMeshInstance3D:
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.use_custom_data = true
	multimesh.instance_count = 1
	multimesh.mesh = mesh
	multimesh.set_instance_transform(0, Transform3D.IDENTITY)
	multimesh.set_instance_custom_data(0, custom)
	multimesh.custom_aabb = AABB(Vector3(-40.0, -10.0, -40.0), Vector3(80.0, 20.0, 80.0))
	var node := MultiMeshInstance3D.new()
	node.multimesh = multimesh
	node.material_override = material
	node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	node.set_instance_shader_parameter("cell_mode", mode)
	# The patch is the tile (0, 0): 0..128 hm on both axes, the base lattice anchored at 0.
	node.set_instance_shader_parameter("cell_patch_rect_hm", Vector4i(0, 0, 128, 128))
	node.set_instance_shader_parameter("cell_lattice_anchor_hm", Vector2i.ZERO)
	return node

func _render(node: Node3D) -> Image:
	var viewport := SubViewport.new()
	viewport.size = Vector2i(64, 64)
	viewport.own_world_3d = true
	viewport.transparent_bg = false
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var environment := Environment.new()
	environment.background_mode = Environment.BG_COLOR
	environment.background_color = BACKGROUND
	var world_environment := WorldEnvironment.new()
	world_environment.environment = environment
	viewport.add_child(world_environment)
	var camera := Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 64.0
	viewport.add_child(camera)
	# The patch node sits at the patch centre; looking straight down, +x right, +z down.
	camera.position = Vector3(0.0, 50.0, 0.0)
	camera.look_at(Vector3.ZERO, Vector3(0.0, 0.0, -1.0))
	camera.current = true
	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-60.0, 30.0, 0.0)
	viewport.add_child(light)
	viewport.add_child(node)
	for frame in 3:
		await process_frame
	await RenderingServer.frame_post_draw
	var image := viewport.get_texture().get_image()
	viewport.queue_free()
	return image

func _is_background(color: Color) -> bool:
	return color.r > 0.9 and color.g < 0.1 and color.b > 0.9

func _run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("This regression needs a rendering display.")
		quit(2)
		return
	var tiles := GradedTiles.new()
	tiles.terrain_cell_m = 10.0
	tiles._build_meshes()
	var pools := _slot_pools()
	# GL0, slot 0, every side its own code.
	var graded := _cells(tiles._meshes[0], _material("res://assets/materials/terrain_graded.gdshader", pools), Color(0.0, 0.0, 0.0, 0.0), 0.0)
	var image: Image = await _render(graded)
	var road_px := 0
	var ground_px := 0
	for y in range(4, 60):
		for x in range(4, 60):
			# Pixel centres in the tile frame: 1 m per pixel; skip the column at the edge.
			var tile_x := float(x) + 0.5
			if absf(tile_x - ROAD_EDGE_M) < 1.5:
				continue
			var background := _is_background(image.get_pixel(x, y))
			if tile_x < ROAD_EDGE_M:
				road_px += 1
				_expect(background, "the road side must be discarded at pixel %d,%d" % [x, y])
			else:
				ground_px += 1
				_expect(not background, "the ground side must be drawn at pixel %d,%d" % [x, y])
			if _failures > 8:
				break
	_expect(road_px > 0 and ground_px > 0, "both sides must be sampled")
	# A base cell (base variant, the patch material's shader) draws the whole tile. Its codes
	# (every side the base code) travel in two channels below 2048: the Compatibility renderer
	# keeps instance custom data as half floats.
	var base_code := GradedTiles.base_code(1)
	var base := _cells(tiles._meshes[base_code], _material("res://assets/materials/terrain.gdshader", pools), Color(0.0, -1.0, base_code * 73, base_code * 9), 2.0)
	image = await _render(base)
	for y in range(4, 60, 7):
		for x in range(4, 60, 7):
			_expect(not _is_background(image.get_pixel(x, y)), "a base cell must draw pixel %d,%d" % [x, y])
	print("Graded cells shader tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
