# SPDX-License-Identifier: GPL-2.0-only

## Rendered grass/paving boundary regression; --benchmark-plots --baseline-dir=PATH
## compares current shaders with *.before snapshots from the specified directory.
extends SceneTree

const Materials := preload("res://scripts/renderers/world_materials.gd")
const GradedTiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const GRADED_SHADER := preload("res://assets/materials/terrain_graded.gdshader")
# A GL0 slot: 131 x 131 texels with the apron; rows at the level's width.
const SLOT_SIDE := 131
# The vertex colour of a roadside sidewalk in the road chunks (`sidewalk_color`):
# road_path.gdshader draws a vertex whose green channel is full as a footpath (ROAD-56).
const SIDEWALK_TAG := Color(0, 0, 0, 1)
var failures := 0
var viewport: SubViewport
var surface: MeshInstance3D
var sun: DirectionalLight3D
var terrain: ShaderMaterial
var camera: Camera3D
var cell_meshes: Dictionary = {}

func _initialize() -> void:
	call_deferred("run")

func settle(frames: int = 8) -> void:
	for frame in frames:
		await process_frame
		await RenderingServer.frame_post_draw

func expect(condition: bool, message: String) -> void:
	if not condition:
		failures += 1
		push_error(message)

func mesh_with_tag(tag: Color, yaw: float = 0.0) -> ArrayMesh:
	var plane := PlaneMesh.new()
	plane.size = Vector2(40, 40)
	var arrays := plane.get_mesh_arrays()
	# Live plot meshes have positions and normals, without UVs or tangents.
	arrays[Mesh.ARRAY_TANGENT] = null
	arrays[Mesh.ARRAY_TEX_UV] = null
	var colours := PackedColorArray()
	colours.resize(arrays[Mesh.ARRAY_VERTEX].size())
	colours.fill(tag)
	arrays[Mesh.ARRAY_COLOR] = colours
	var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
	for i in vertices.size():
		vertices[i] = vertices[i].rotated(Vector3.UP, yaw)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh

func capture(material: Material, tag: Color = Color.WHITE, yaw: float = 0.0) -> Image:
	surface.mesh = mesh_with_tag(tag, yaw)
	surface.material_override = material
	await settle()
	return viewport.get_texture().get_image()

func difference(a: Image, b: Image) -> float:
	var total := 0.0
	var count := 0
	for y in range(32, a.get_height() - 32, 8):
		for x in range(32, a.get_width() - 32, 8):
			var p := a.get_pixel(x, y)
			var q := b.get_pixel(x, y)
			total += Vector3(p.r - q.r, p.g - q.g, p.b - q.b).length()
			count += 1
	return total / count

# The graded renderer's cell meshes must face upward: a cell mesh's vertex holds its lattice
# index in XZ, which terrain_cells.gdshaderinc scales into place without mirroring, so its
# winding is the drawn one.
func check_cell_winding(tiles: GradedTiles) -> void:
	var faces := 0
	var backwards := 0
	for code: int in tiles._meshes.keys():
		var cell_arrays: Array = (tiles._meshes[code] as ArrayMesh).surface_get_arrays(0)
		var vertices: PackedVector3Array = cell_arrays[Mesh.ARRAY_VERTEX]
		var indices: PackedInt32Array = cell_arrays[Mesh.ARRAY_INDEX]
		for i in range(0, indices.size(), 3):
			var a := vertices[indices[i]]
			var normal := (vertices[indices[i + 1]] - a).cross(vertices[indices[i + 2]] - a)
			faces += 1
			if normal.y > 0:
				backwards += 1
	print("NATIVE_TERRAIN_WINDING faces=", faces, " backwards=", backwards)
	expect(faces > 0 and backwards == 0, "graded terrain cells must face upward under Godot's clockwise convention")

# Frontage paving as the game draws it (TERRAIN-07-G round 4): a GL0 graded cell of one 64 m tile,
# centred on the origin, whose every sample holds material class `class_id` (1 asphalt, 2
# concrete), no road and no grading, through the graded variant with the flat terrain's
# parameters copied as terrain_graded_tiles.gd copies a patch's. The CDT's vertex tags this test
# rendered before are deleted.
func graded_paving(class_id: int) -> MultiMeshInstance3D:
	# The slot layer and its header line (docs/terrain.md "Pages and the row atlas").
	var texels := Image.create_empty(SLOT_SIDE, SLOT_SIDE + 1, false, Image.FORMAT_RGBAF)
	texels.fill(Color(0.0, 2.0, -1.0, float(class_id)))
	# No sample names a seam or edge: the rows are never read.
	var rows := Image.create_empty(256, 1, false, Image.FORMAT_RGBAF)
	var texel_array := Texture2DArray.new()
	texel_array.create_from_images([texels])
	var row_array := Texture2DArray.new()
	row_array.create_from_images([rows])
	var material := ShaderMaterial.new()
	material.shader = GRADED_SHADER
	for uniform: Dictionary in GRADED_SHADER.get_shader_uniform_list():
		var name := String(uniform["name"])
		if not name.begins_with("graded_") and not name.begins_with("cell_"):
			material.set_shader_parameter(name, terrain.get_shader_parameter(name))
	material.set_shader_parameter("graded_texels", texel_array)
	material.set_shader_parameter("graded_atlas", row_array)
	material.set_shader_parameter("terrain_cell_m", 10.0)
	material.set_shader_parameter("world_size", Vector2(64.0, 64.0))
	material.set_shader_parameter("patch_world_size_m", Vector2(64.0, 64.0))
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.use_custom_data = true
	multimesh.instance_count = 1
	multimesh.mesh = cell_meshes[0]
	multimesh.set_instance_transform(0, Transform3D.IDENTITY)
	# Place 0 in the patch, slot 0, every side GL0.
	multimesh.set_instance_custom_data(0, Color(0.0, 0.0, 0.0, 0.0))
	multimesh.custom_aabb = AABB(Vector3(-40.0, -10.0, -40.0), Vector3(80.0, 20.0, 80.0))
	var node := MultiMeshInstance3D.new()
	node.multimesh = multimesh
	node.material_override = material
	node.set_instance_shader_parameter("cell_mode", 0.0)
	# The patch is the tile (0, 0): 0..128 hm on both axes, drawn about the node at its centre.
	node.set_instance_shader_parameter("cell_patch_rect_hm", Vector4i(0, 0, 128, 128))
	node.set_instance_shader_parameter("cell_lattice_anchor_hm", Vector2i.ZERO)
	return node

# Renders a graded cell node in place of the test surface.
func capture_node(node: Node3D) -> Image:
	surface.visible = false
	viewport.add_child(node)
	await settle()
	var image := viewport.get_texture().get_image()
	viewport.remove_child(node)
	surface.visible = true
	return image

func run() -> void:
	if DisplayServer.get_name() == "headless":
		push_error("plot_material_lighting_test requires GPU rendering")
		quit(1)
		return
	viewport = SubViewport.new()
	viewport.size = Vector2i(320, 240)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var world := WorldEnvironment.new()
	world.environment = load("res://default_environment.tres").duplicate()
	world.environment.background_mode = Environment.BG_COLOR
	world.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	world.environment.ambient_light_color = Color(0.65, 0.75, 1)
	world.environment.ambient_light_energy = 0.2
	viewport.add_child(world)
	camera = Camera3D.new()
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 24
	camera.position = Vector3(0, 20, 0)
	camera.rotation_degrees.x = -90
	viewport.add_child(camera)
	sun = DirectionalLight3D.new()
	sun.rotation_degrees = Vector3(-45, -30, 0)
	viewport.add_child(sun)
	surface = MeshInstance3D.new()
	viewport.add_child(surface)
	terrain = Materials.flat_terrain_material().duplicate()
	Materials.apply_terrain_paving_parameters(terrain)
	var tiles := GradedTiles.new()
	tiles.terrain_cell_m = 10.0
	tiles._build_meshes()
	cell_meshes = tiles._meshes
	check_cell_winding(tiles)
	var frontages := {"asphalt": graded_paving(1), "concrete": graded_paving(2)}
	for energy: float in [1.0, 0.02, 0.0]:
		sun.light_energy = energy
		world.environment.ambient_light_energy = energy * 0.2
		RenderingServer.global_shader_parameter_set("scene_ambient_light", Color(energy, energy, energy))
		RenderingServer.global_shader_parameter_set("scene_ambient_desaturation", 0.0 if energy == 1.0 else 0.65)
		var ground := await capture(terrain)
		var grass := await capture(Materials.site_ground_material())
		var grass_delta := difference(ground, grass)
		print("PLOT_GRASS energy=", energy, " delta=", grass_delta)
		expect(grass_delta < 0.012, "plot grass must match flat open terrain")
		for kind: String in ["asphalt", "concrete"]:
			var frontage := await capture_node(frontages[kind])
			var yard := await capture(Materials.site_surface_material(kind))
			var delta := difference(frontage, yard)
			print("PLOT_PAVING kind=", kind, " energy=", energy, " delta=", delta)
			expect(delta < 0.012, "frontage must match authored paving in the same light")
			if energy == 0.0:
				expect(frontage.get_pixel(160, 120).get_luminance() < 0.01, "paving must not retain the grass emission floor")
			if kind == "asphalt":
				var sidewalk := await capture(Materials.road_sidewalk_material(), SIDEWALK_TAG)
				var rotated := await capture(Materials.road_sidewalk_material(), SIDEWALK_TAG, PI * 0.5)
				print("PLOT_SIDEWALK delta=", difference(yard, sidewalk), " rotated=", difference(sidewalk, rotated))
				expect(difference(yard, sidewalk) < 0.012, "yard and sidewalk asphalt must match")
				expect(difference(sidewalk, rotated) < 0.012, "world mapped normals must not depend on mesh orientation")
	world.environment.ambient_light_energy = 0
	# Directional shadows and local lamps must agree across the paving boundary too.
	var blocker := MeshInstance3D.new()
	var box := BoxMesh.new()
	box.size = Vector3(5, 4, 5)
	blocker.mesh = box
	blocker.position = Vector3(0, 3, 0)
	viewport.add_child(blocker)
	sun.shadow_enabled = true
	var lamp := OmniLight3D.new()
	lamp.position = Vector3(2, 4, 0)
	lamp.omni_range = 15
	lamp.light_energy = 3
	lamp.shadow_enabled = true
	viewport.add_child(lamp)
	for local: bool in [false, true]:
		sun.light_energy = 0.0 if local else 1.0
		lamp.visible = local
		var frontage := await capture_node(frontages["asphalt"])
		var yard := await capture(Materials.site_asphalt_material())
		print("PLOT_SHADOW local=", local, " delta=", difference(frontage, yard))
		expect(difference(frontage, yard) < 0.012, "paving must agree under shadows and local lights")
		if local:
			expect(frontage.get_pixel(185, 145).get_luminance() > 0.02, "frontage must receive local lights")
	blocker.free()
	lamp.free()
	for node: Node in frontages.values():
		node.free()
	# A production-strength headlight must illuminate only its cone, with terrain/road falloff.
	sun.light_energy = 0.0
	RenderingServer.global_shader_parameter_set("scene_ambient_light", Color.BLACK)
	var headlight := SpotLight3D.new()
	var headlights_script := preload("res://scripts/renderers/vehicle_headlights.gd")
	headlight.position = Vector3(0, 0.65, 8)
	headlight.spot_range = headlights_script.RANGE_M
	headlight.spot_attenuation = headlights_script.DECAY
	headlight.spot_angle = headlights_script.CONE_ANGLE_DEG
	headlight.spot_angle_attenuation = headlights_script.CONE_ANGLE_ATTENUATION
	headlight.light_energy = headlights_script.CONE_ENERGY
	headlight.light_specular = 0.0
	viewport.add_child(headlight)
	var headlight_reference := await capture(terrain)
	for material: ShaderMaterial in [Materials.site_ground_material(), Materials.editor_ground_material()]:
		var lit := await capture(material)
		# Outside the cone but inside the light's range; a visibility floor lights this pixel.
		var outside := camera.unproject_position(Vector3(8, 0, 4))
		expect(lit.get_pixelv(Vector2i(outside)).get_luminance() < 0.01, "ground outside the headlight cone must stay dark")
		var inside := camera.unproject_position(Vector3(0, 0, -4))
		expect(lit.get_pixelv(Vector2i(inside)).get_luminance() > 0.02, "ground inside the headlight cone must receive light")
		if material == Materials.site_ground_material():
			expect(difference(lit, headlight_reference) < 0.012, "site grass must retain terrain/road headlight attenuation")
	headlight.free()
	sun.shadow_enabled = false
	if "--benchmark-plots" in OS.get_cmdline_user_args():
		var baseline_dir := ""
		for argument in OS.get_cmdline_user_args():
			if argument.begins_with("--baseline-dir="):
				baseline_dir = argument.trim_prefix("--baseline-dir=")
		if baseline_dir.is_empty():
			push_error("--benchmark-plots requires --baseline-dir with shader snapshots")
			quit(1)
			return
		DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
		RenderingServer.viewport_set_measure_render_time(viewport.get_viewport_rid(), true)
		viewport.size = Vector2i(1920, 1080)
		sun.light_energy = 1
		world.environment.ambient_light_energy = 0.2
		RenderingServer.global_shader_parameter_set("scene_ambient_light", Color.WHITE)
		RenderingServer.global_shader_parameter_set("scene_ambient_desaturation", 0.0)
		for kind: String in ["site_ground", "terrain", "terrain_grass", "site_surface", "road"]:
			var current: ShaderMaterial = terrain if kind.begins_with("terrain") else (Materials.site_ground_material() if kind == "site_ground" else (Materials.site_asphalt_material() if kind == "site_surface" else Materials.road_sidewalk_material()))
			var before := current.duplicate() as ShaderMaterial
			var shader := Shader.new()
			shader.code = FileAccess.get_file_as_string(baseline_dir.path_join(("terrain" if kind == "terrain_grass" else kind) + ".before"))
			before.shader = shader
			if kind == "site_ground":
				before.set_shader_parameter("hillshade_strength", 0.18)
				before.set_shader_parameter("hillshade_ambient", 0.70)
				before.set_shader_parameter("hillshade_contrast", 1.05)
				before.set_shader_parameter("hillshade_shadow_tint", Color(0.84, 0.90, 0.88))
			for old: bool in [true, false, true, false]:
				surface.material_override = before if old else current
				surface.mesh = mesh_with_tag(Color.RED if kind == "terrain" else Color.WHITE)
				await settle(60)
				var gpu: Array[float] = []
				for frame in 120:
					await settle(1)
					gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(viewport.get_viewport_rid()))
				gpu.sort()
				print("PLOT_BENCH kind=", kind, " old=", old, " median_ms=", gpu[60])
	viewport.free()
	print("plot_material_lighting_test: ", "PASS" if failures == 0 else "FAIL")
	quit(failures)
