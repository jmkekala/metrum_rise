# SPDX-License-Identifier: GPL-2.0-only

## Generated gameplay LOD regression and matched renderer benchmark. Rust prepares saves;
## this harness prepares model resources outside measurements, without personal asset packs.
extends SceneTree

const Buildings := preload("res://scripts/renderers/buildings.gd")
const ModPackConfig := preload("res://scripts/core/mod_pack_config.gd")
var failures := 0
var fixture_dir := OS.get_environment("METRUM_BUILDING_LOD_FIXTURE_DIR")
var world: Node3D
var simulation: Node
var renderer: Node3D
var camera: Camera3D
var modern := false

func _initialize() -> void:
	call_deferred("_run")

func expect(value: bool, message: String) -> void:
	if not value:
		failures += 1
		push_error(message)

func write(path: String, content: String) -> void:
	var file := FileAccess.open(path, FileAccess.WRITE)
	expect(file != null, "fixture must open: " + path)
	if file:
		file.store_string(content)

func make_model(path: String, lod: int) -> void:
	var scene := Node3D.new()
	var node := MeshInstance3D.new()
	var mesh := SphereMesh.new()
	mesh.radius = 6.0
	mesh.height = 12.0
	mesh.radial_segments = maxi(4, 64 >> lod)
	mesh.rings = maxi(2, 32 >> lod)
	var material := StandardMaterial3D.new()
	material.resource_name = "authored_emissive_windows"
	material.albedo_color = Color(0.7, 0.35, 0.2)
	material.emission_enabled = true
	material.emission = Color(0.2, 0.12, 0.02)
	var image := Image.create(2, 2, false, Image.FORMAT_RGBA8)
	image.fill(Color(1, 0.8, 0.4))
	material.emission_texture = ImageTexture.create_from_image(image)
	mesh.material = material
	node.mesh = mesh
	node.position.y = 6.0
	scene.add_child(node)
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	expect(document.append_from_scene(scene, state) == OK, "generated model must export")
	expect(document.write_to_filesystem(state, path) == OK, "generated model must write")
	scene.free()

func prepare_pack() -> String:
	var directory := ProjectSettings.globalize_path("user://mods/test")
	DirAccess.make_dir_recursive_absolute(directory)
	write(directory.path_join("pack.toml"), 'pack_id = "test"\nschema_version = 1\ndisplay_name = "Generated LOD fixtures"\nversion = "1.0.0"\nauthor = "Test"\nlicense = "CC0"\n')
	for lod in 5:
		make_model(directory.path_join("lod%d.glb" % lod), lod)
	for asset in 3:
		var asset_dir := directory.path_join("house%d" % asset)
		DirAccess.make_dir_recursive_absolute(asset_dir)
		for lod in [1, 4, 5][asset]:
			expect(DirAccess.copy_absolute(directory.path_join("lod%d.glb" % lod), asset_dir.path_join("lod%d.glb" % lod)) == OK, "fixture model must copy")
		var manifest := """asset_id = "building.residential.lod_house_%d"
display_name = "Generated LOD house"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
household_capacity = 1
[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 6.0]
forward = [0.0, 0.0, 1.0]
""" % asset
		for part in (2 if asset == 0 else 1):
			manifest += '\n[[mesh_parts]]\nname = "part_%d"\n' % part
			if part == 1:
				manifest += 'scale = 0.2\nposition = [0.0, 14.0, 0.0]\n'
			for lod in [1, 4, 5][asset]:
				manifest += '[[mesh_parts.lods]]\nfile = "lod%d.glb"\ndistance_min_m = %d.0\ndistance_max_m = %d.0\n' % [lod, lod * 100, (lod + 1) * 100]
		write(asset_dir.path_join("asset.toml"), manifest)
	expect(ModPackConfig.save_enabled_pack_ids(["test"]) == OK, "fixture pack selection must save")
	return directory

func pose(position: Vector3, target: Vector3) -> void:
	camera.position = position
	camera.look_at(target)

func refresh() -> void:
	if modern:
		expect(renderer.lod_renderer.update(camera), "LOD frame must be available while paused")
		renderer._update_building_render_frame()
	else:
		# Baseline uses its original LOD0-only collection path. Test-only compatibility
		# allows identical saves, assets, cameras and timing on the pre-change build.
		renderer._update_building_render_frame()

func counts() -> Dictionary:
	var result := {"instances": 0, "tiers": {}, "deserted": 0, "groups": 0}
	for key: Vector4i in renderer.lod_renderer.batches:
		var node: MultiMeshInstance3D = renderer.lod_renderer.batches[key]
		var count := node.multimesh.visible_instance_count
		result.instances += count
		if count > 0:
			result.groups += 1
			result.tiers[key.w / 2] = true
			if key.w % 2 == 1:
				result.deserted += count
	return result

func _run() -> void:
	if fixture_dir.is_empty():
		push_error("Set METRUM_BUILDING_LOD_FIXTURE_DIR to the generated Rust saves; use an isolated user profile")
		quit(1)
		return
	var pack_dir := prepare_pack()
	root.size = Vector2i(1280, 720)
	world = Node3D.new()
	root.add_child(world)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	world.add_child(simulation)
	simulation.set_simulation_speed(0.0)
	simulation.set_process(false)
	camera = Camera3D.new()
	camera.far = 6000.0
	world.add_child(camera)
	camera.make_current()
	var sun := DirectionalLight3D.new()
	sun.name = "DirectionalLight3D"
	sun.rotation_degrees = Vector3(-60, -25, 0)
	sun.shadow_enabled = true
	sun.directional_shadow_max_distance = 420.0
	world.add_child(sun)
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color(0.15, 0.2, 0.27)
	environment.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.environment.ambient_light_color = Color.WHITE
	environment.environment.ambient_light_energy = 0.3
	world.add_child(environment)
	renderer = Buildings.new()
	world.add_child(renderer)
	renderer.set_process(false)
	modern = simulation.has_method("try_get_building_lod_frame")
	expect(simulation.load_game(fixture_dir.path_join("complete.sqlite")), "complete save must load")
	simulation.set_simulation_speed(0.0)
	pose(Vector3(0, 2200, 1800), Vector3.ZERO)
	refresh()
	if "--benchmark-building-lod" in OS.get_cmdline_user_args():
		await benchmark()
	elif modern:
		await correctness(pack_dir)
	else:
		expect(false, "correctness mode requires the new LOD API")
	world.free()
	print("building_lod_test: %s" % ("PASS" if failures == 0 else "FAIL"))
	quit(failures)

func correctness(pack_dir: String) -> void:
	var lods: Node3D = renderer.lod_renderer
	expect(lods.catalog.size() == 5, "one-, four-, five-tier and multipart catalog must load")
	expect(lods.resources.imports == 10, "paths shared between parts import only once")
	for chain: Array in lods.meshes:
		for mesh: Mesh in chain:
			if mesh == lods.meshes[0][0]:
				continue
			var material := mesh.surface_get_material(0) as StandardMaterial3D
			expect(material != null and material.emission_texture != null and material.emission_enabled, "every tier preserves authored emission")
	var far := counts()
	expect(far.instances == 2731, "all 2048 buildings including multipart instances must draw exactly once")
	expect(far.tiers.has(0) and far.tiers.has(3) and far.tiers.has(4), "short chains retain their final tier at distant zoom")
	refresh()
	expect(lods.evaluated_parts == 0 and lods.uploaded_bytes == 0, "stationary view performs no evaluations/uploads")
	var imports: int = lods.resources.imports
	var selected: Dictionary = simulation.get_building_info_at(-1008.0, -992.0)
	expect(not selected.is_empty(), "gameplay selection fixture must exist")
	pose(Vector3(-960, 20, -920), Vector3(-960, 6, -984))
	refresh()
	var close := counts()
	expect(close.tiers.size() > 1 and close.tiers.has(0), "near and distant instances coexist at different tiers")
	for quality in [0, 2, 1]:
		lods.set_quality(quality)
		refresh()
		expect(lods.evaluated_parts > 0, "quality changes reevaluate resident parts")
	root.scaling_3d_scale = 0.5
	refresh()
	expect(lods.evaluated_parts > 0, "render scale change reevaluates LODs")
	root.scaling_3d_scale = 1.0
	pose(Vector3(1800, 2200, 0), Vector3.ZERO)
	refresh()
	expect(counts().instances == 2731, "camera rotation retains every visible building")
	expect(lods.resources.imports == imports, "zoom/quality/rotation never reimport resources")
	expect(simulation.get_building_info_at(-1008.0, -992.0).get("building_id") == selected.get("building_id"), "rendered tier cannot change gameplay selection identity")
	for variant in ["lifecycle", "removed", "complete"]:
		expect(simulation.load_game(fixture_dir.path_join(variant + ".sqlite")), "lifecycle save loads: " + variant)
		simulation.set_simulation_speed(0.0)
		refresh()
		var expected := 2729 if variant == "removed" else 2731
		expect(counts().instances == expected, "world replacement clears old batches: " + variant)
		expect(counts().deserted == (0 if variant == "complete" else 1), "deserted state survives load: " + variant)
		expect(lods.resources.imports == imports, "world replacement retains cached resources")
	# A failed lower tier remains in the authored chain but cannot remove a building.
	write(pack_dir.path_join("house2/lod4.glb"), "deliberately invalid test mesh")
	renderer.reload_asset_packs()
	refresh()
	expect(counts().instances == 2731 and not counts().tiers.has(4), "failed lower tier falls back without holes")
	var failed_imports: int = lods.resources.imports
	pose(Vector3(0, 2300, 1800), Vector3.ZERO)
	refresh()
	expect(lods.resources.imports == failed_imports, "failed imports are not retried while moving")
	make_model(pack_dir.path_join("house2/lod4.glb"), 4)
	renderer.reload_asset_packs()
	refresh()
	expect(counts().tiers.has(4), "explicit pack reload recovers a repaired tier")
	await process_frame

func benchmark() -> void:
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0
	RenderingServer.viewport_set_measure_render_time(root.get_viewport_rid(), true)
	var output: Array = []
	for trial in ["wide", "street", "moving", "zoom"]:
		var samples: Array = []
		var cpu: Array = []
		var gpu: Array = []
		var uploads := 0
		var evaluations := 0
		for frame in 360:
			if trial == "wide":
				pose(Vector3(0, 2200, 1800), Vector3.ZERO)
			elif trial == "zoom":
				var target := Vector3(-960, 6, -984)
				var distance := 20.0 + 1880.0 * (0.5 - 0.5 * cos(float(frame) * TAU / 120.0))
				pose(target + Vector3(0, 0.6, 0.8) * distance, target)
			else:
				var offset := float(frame % 120) * 0.3 if trial == "moving" else 0.0
				pose(Vector3(-960 + offset, 25, -900), Vector3(-960 + offset, 6, -984))
			var start := Time.get_ticks_usec()
			# Match the old renderer's 30-frame polling cadence in stationary trials;
			# camera changes previously required no transform refresh (always LOD0).
			if modern or frame % 30 == 0:
				refresh()
			var elapsed := Time.get_ticks_usec() - start
			await process_frame
			await RenderingServer.frame_post_draw
			if frame >= 60:
				samples.append(elapsed)
				cpu.append(RenderingServer.viewport_get_measured_render_time_cpu(root.get_viewport_rid()))
				gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(root.get_viewport_rid()))
				if modern:
					uploads += renderer.lod_renderer.uploaded_bytes
					evaluations += renderer.lod_renderer.evaluated_parts
				elif frame % 30 == 0:
					for groups: Dictionary in [renderer.multimeshes, renderer.deserted_multimeshes, renderer.foundation_multimeshes, renderer.construction_site_multimeshes, renderer.construction_foundation_multimeshes, renderer.construction_scaffold_multimeshes]:
						for node: MultiMeshInstance3D in groups.values():
							uploads += node.multimesh.instance_count * 12 * 4
		var row := {"trial": trial, "update_us": summarize(samples), "render_cpu_ms": summarize(cpu), "gpu_ms": summarize(gpu), "uploaded_bytes": uploads, "evaluated_parts": evaluations,
			"draw_calls": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_DRAW_CALLS_IN_FRAME),
			"primitives": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TOTAL_PRIMITIVES_IN_FRAME),
			"video_bytes": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_VIDEO_MEM_USED),
			"static_bytes": OS.get_static_memory_usage(), "process_rss_bytes": process_rss_bytes()}
		output.append(row)
		print("BUILDING_LOD_BENCHMARK ", JSON.stringify(row))
		if trial == "wide":
			root.get_texture().get_image().save_png(fixture_dir.path_join("candidate.png" if modern else "baseline.png"))
	write(fixture_dir.path_join("candidate.json" if modern else "baseline.json"), JSON.stringify(output, "\t"))

func process_rss_bytes() -> int:
	# Godot's static-memory counter excludes the Rust allocator; Linux RSS includes both.
	if OS.get_name() == "Linux":
		var status := FileAccess.open("/proc/self/status", FileAccess.READ)
		while status != null and not status.eof_reached():
			# procfs reports length zero, so whole-file helpers cannot read it.
			var line := status.get_line()
			if line.begins_with("VmRSS:"):
				return int(line.replace("\t", " ").split(" ", false)[1]) * 1024
	return -1

func summarize(values: Array) -> Dictionary:
	values.sort()
	var total := 0.0
	for value: float in values:
		total += value
	return {"mean": total / values.size(), "p50": values[values.size() / 2], "p95": values[int(values.size() * 0.95)]}
