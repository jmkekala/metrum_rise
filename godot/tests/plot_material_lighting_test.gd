# SPDX-License-Identifier: GPL-2.0-only

## Rendered grass/paving boundary regression; --benchmark-plots --baseline-dir=PATH
## compares current shaders with *.before snapshots from the specified directory.
extends SceneTree

const Materials := preload("res://scripts/renderers/world_materials.gd")
var failures := 0
var viewport: SubViewport
var surface: MeshInstance3D
var sun: DirectionalLight3D
var terrain: ShaderMaterial
var camera: Camera3D

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

func native_terrain_face() -> ArrayMesh:
	# Exercise Rust's real CDT/regular-buffer export, which a PlaneMesh cannot cover.
	var simulation := SimulationNode.new()
	root.add_child(simulation)
	expect(simulation.create_blank_world(80, 80, 10, 40, 0), "native winding fixture must initialize")
	var request := simulation.request_preview_road_surface_with_snap(
		PackedVector3Array([Vector3(-10, 0, 0), Vector3(10, 0, 0)]), 1, 1, true)
	var result: Variant = null
	var deadline := Time.get_ticks_msec() + 15000
	while result == null and Time.get_ticks_msec() < deadline:
		result = simulation.get_preview_road_surface_result(request, 0)
		await process_frame
	var selected := PackedVector3Array()
	var faces := 0
	var backwards := 0
	if result is Dictionary:
		for patch in result.get("terrain_preview", {}).get("patches", []):
			var vertices: PackedVector3Array = patch.get("terrain_mesh_vertices", PackedVector3Array())
			var indices: PackedInt32Array = patch.get("terrain_mesh_indices", PackedInt32Array())
			for i in range(0, indices.size(), 3):
				var a := vertices[indices[i]]
				var b := vertices[indices[i + 1]]
				var c := vertices[indices[i + 2]]
				var normal := (b - a).cross(c - a)
				faces += 1
				if normal.y > 0:
					backwards += 1
				if selected.is_empty() and absf(normal.y) > 1 and absf(a.y - b.y) < 0.0001 and absf(a.y - c.y) < 0.0001:
					var centre := (a + b + c) / 3.0
					selected = PackedVector3Array([a - centre, b - centre, c - centre])
	simulation.free()
	print("NATIVE_TERRAIN_WINDING faces=", faces, " backwards=", backwards)
	expect(faces > 0 and backwards == 0, "exported terrain must face upward under Godot's clockwise convention")
	expect(not selected.is_empty(), "native fixture must supply a flat terrain face")
	if selected.is_empty():
		return null
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = selected
	arrays[Mesh.ARRAY_NORMAL] = PackedVector3Array([Vector3.UP, Vector3.UP, Vector3.UP])
	arrays[Mesh.ARRAY_COLOR] = PackedColorArray([Color.RED, Color.RED, Color.RED])
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh

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
			var tag := Color.RED if kind == "asphalt" else Color.GREEN
			var frontage := await capture(terrain, tag)
			var yard := await capture(Materials.site_surface_material(kind))
			var delta := difference(frontage, yard)
			print("PLOT_PAVING kind=", kind, " energy=", energy, " delta=", delta)
			expect(delta < 0.012, "frontage must match authored paving in the same light")
			if energy == 0.0:
				expect(frontage.get_pixel(160, 120).get_luminance() < 0.01, "paving must not retain the grass emission floor")
			if kind == "asphalt":
				var sidewalk := await capture(Materials.road_sidewalk_material())
				var rotated := await capture(Materials.road_sidewalk_material(), Color.WHITE, PI * 0.5)
				print("PLOT_SIDEWALK delta=", difference(yard, sidewalk), " rotated=", difference(sidewalk, rotated))
				expect(difference(yard, sidewalk) < 0.012, "yard and sidewalk asphalt must match")
				expect(difference(sidewalk, rotated) < 0.012, "world mapped normals must not depend on mesh orientation")
	var native_mesh := await native_terrain_face()
	if native_mesh != null:
		for energy: float in [1.0, 0.02]:
			sun.light_energy = energy
			world.environment.ambient_light_energy = energy * 0.2
			var reference := await capture(Materials.site_asphalt_material())
			surface.mesh = native_mesh
			surface.material_override = terrain
			await settle()
			var rendered := viewport.get_texture().get_image()
			var a := rendered.get_pixel(160, 120)
			var b := reference.get_pixel(160, 120)
			var delta := Vector3(a.r - b.r, a.g - b.g, a.b - b.b).length()
			print("NATIVE_PAVING energy=", energy, " delta=", delta)
			expect(delta < 0.012, "actual exported terrain must match the yard under day/night lighting")
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
		var frontage := await capture(terrain, Color.RED)
		var yard := await capture(Materials.site_asphalt_material())
		print("PLOT_SHADOW local=", local, " delta=", difference(frontage, yard))
		expect(difference(frontage, yard) < 0.012, "paving must agree under shadows and local lights")
		if local:
			expect(frontage.get_pixel(185, 145).get_luminance() > 0.02, "frontage must receive local lights")
	blocker.free()
	lamp.free()
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
