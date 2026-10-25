# SPDX-License-Identifier: GPL-2.0-only

## Oblique reference renders of scripted road junctions through the production road chunk renderer.
## Set METRUM_ROAD_JUNCTION_REFERENCE_DIR for PNGs and manifest.json; use a real rendering device.
## Without it the test exits at once. Each fixture is rendered flat and on a 2.5% slope, in three views:
## oblique (terrain), oblique_roads (terrain hidden, grey) and plan (top-down orthographic, 60 x 37.5 m).
extends SceneTree

const RoadToolScript := preload("res://scripts/tools/road_tool.gd")
const TerrainScript := preload("res://scripts/renderers/terrain.gd")
const WORLD_M := 512.0
const ARM_M := 80.0
const SLOPE_RISE_M := 6.4 # over 256 m along +x: 2.5%
const CAMERA_HEIGHT_M := 30.0
const CAMERA_PITCH_DEG := 50.0
const PLAN_HEIGHT_M := 37.5
const CAPTURE_SIZE := Vector2i(1600, 1000)
const CONVENTIONS := {
	"axes": "x east (slope rises toward +x), z south (toward camera); junction at the origin",
	"camera": "fixed: 30 m above the surface point under the junction, pitched 50 degrees down, looking toward -z, offset +z",
	"y_junction": "stem runs along +x ending at the origin; branches leave the origin at +/-(angle/2) from +x; drawn stem first, except y_25_branches_first",
	"bend": "first road ends at the origin heading +x; second leaves at +45 degrees (toward +z)",
	"lanes": "default 1 forward + 1 backward lane per road unless the fixture says otherwise; 0 lanes is a 2 m footpath",
	"structures": "bridge_* and tunnel_* set the edge at class_at to class 1 (bridge) or 2 (tunnel) after the commits and render without terrain",
}
var _failures := 0
var _records: Array[Dictionary] = []
var _output := ""
var simulation: SimulationNode

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	_output = OS.get_environment("METRUM_ROAD_JUNCTION_REFERENCE_DIR")
	if _output.is_empty():
		print("road_junction_reference_test: SKIP (METRUM_ROAD_JUNCTION_REFERENCE_DIR unset)")
		quit(0)
		return
	_expect(DisplayServer.get_name() != "headless", "Reference PNGs require a real renderer")
	_expect(DirAccess.make_dir_recursive_absolute(_output) == OK, "Capture directory must exist")
	if _failures != 0:
		quit(1)
		return
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	root.add_child(simulation)
	for fixture in _fixtures():
		for sloped in [false, true]:
			await _reference(fixture, sloped)
	var manifest := {"engine": Engine.get_version_info(), "adapter": RenderingServer.get_video_adapter_name(),
		"vendor": RenderingServer.get_video_adapter_vendor(), "capture_size": [CAPTURE_SIZE.x, CAPTURE_SIZE.y],
		"world": {"size_m": WORLD_M, "cell_m": 8.0, "chunk_m": 128.0, "slope": "2.5% rising toward +x"},
		"conventions": CONVENTIONS, "lighting": "fixed directional light and ambient colour; weather scenario 0 (clear); no time-of-day API is used",
		"fixtures": _records, "failures": _failures}
	var file := FileAccess.open(_output.path_join("manifest.json"), FileAccess.WRITE)
	_expect(file != null, "Reference manifest must open")
	if file != null:
		file.store_string(JSON.stringify(manifest, "\t"))
		file.close()
	simulation.free()
	if _failures == 0:
		print("road_junction_reference_test: PASS (%d images)" % _records.size())
	quit(0 if _failures == 0 else 1)

# Each road is {"pts": [Vector2, Vector2], "lanes": n per direction}.
func _road(a: Vector2, b: Vector2, lanes: int = 1) -> Dictionary:
	return {"pts": [a, b], "lanes": lanes}

func _fixtures() -> Array[Dictionary]:
	var arm := ARM_M
	var result: Array[Dictionary] = [
		{"name": "t", "roads": [_road(Vector2(-arm, 0), Vector2(arm, 0)), _road(Vector2(0, 0), Vector2(0, arm))]},
		{"name": "four_way", "roads": [_road(Vector2(-arm, 0), Vector2(arm, 0)), _road(Vector2(0, -arm), Vector2(0, arm))]},
	]
	for angle in [25, 40, 60]:
		var half := deg_to_rad(angle * 0.5)
		result.append({"name": "y_%d" % angle, "angle_deg": angle, "roads": [
			_road(Vector2(-arm, 0), Vector2.ZERO),
			_road(Vector2.ZERO, Vector2.from_angle(half) * arm),
			_road(Vector2.ZERO, Vector2.from_angle(-half) * arm)]})
	# The same 25-degree Y drawn branches first: invariant 6 wants images identical to y_25's.
	var y25: Array = result[2].roads
	result.append({"name": "y_25_branches_first", "angle_deg": 25, "roads": [y25[1], y25[2], y25[0]]})
	result.append({"name": "bend_45", "roads": [_road(Vector2(-arm, 0), Vector2.ZERO), _road(Vector2.ZERO, Vector2.from_angle(deg_to_rad(45.0)) * arm)]})
	result.append({"name": "terminal", "roads": [_road(Vector2(-arm, 0), Vector2.ZERO)]})
	result.append({"name": "t_mixed_4lane_through", "roads": [_road(Vector2(-arm, 0), Vector2(arm, 0), 2), _road(Vector2(0, 0), Vector2(0, arm))]})
	# A footpath (no lanes) 30 degrees off the T's side road: it meets that arm's sidewalk, so the arm's mouth moves out and its zebra stays at the corner (D5, ROAD-68 open question 17).
	result.append({"name": "t_footpath_beside_arm", "roads": [_road(Vector2(-arm, 0), Vector2(arm, 0)), _road(Vector2(0, 0), Vector2(0, arm)), _road(Vector2.ZERO, Vector2.from_angle(deg_to_rad(120.0)) * 60.0, 0)]})
	# ROAD-68-F part 2: a bridge deck and a tunnel road piece as a T's side road (node piece) and as
	# the continuation of a road end (pass-through span mouth), made structures with set_edge_class.
	# Their terrain patches fail at the open chain end the abutment or portal will close
	# (TERRAIN-07), so they render without terrain.
	for structure in [["bridge", 1], ["tunnel", 2]]:
		result.append({"name": "%s_t" % structure[0], "class_at": Vector2(0, arm * 0.5), "class": structure[1], "terrain": false,
			"roads": [_road(Vector2(-arm, 0), Vector2(arm, 0)), _road(Vector2(0, 0), Vector2(0, arm))]})
		result.append({"name": "%s_continuation" % structure[0], "class_at": Vector2(arm * 0.5, 0), "class": structure[1], "terrain": false,
			"roads": [_road(Vector2(-arm, 0), Vector2.ZERO), _road(Vector2.ZERO, Vector2(arm, 0))]})
	return result

func _surface(point: Vector2) -> Vector3:
	return Vector3(point.x, float(simulation.get_world_surface_height(point)), point.y)

func _commit(road: Dictionary) -> bool:
	var points := PackedVector3Array([_surface(road.pts[0]), _surface(road.pts[1])])
	var request: int = simulation.add_road_with_snap(points, road.lanes, road.lanes, true)
	var deadline := Time.get_ticks_msec() + 30000
	while Time.get_ticks_msec() < deadline:
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			_expect(result.get("committed", false), "Reference road %s: %s" % [points, result])
			return bool(result.get("committed", false))
		await process_frame
	_expect(false, "Reference road commit did not settle: %s" % points)
	return false

func _terrain_node() -> Node3D:
	# Production terrain payload meshes with a lit material: a 20 m checker gives the slope a visible cue.
	var result := Node3D.new()
	var renderer := TerrainScript.new()
	var layout: Dictionary = simulation.get_terrain_patch_layout()
	var requests := PackedInt32Array()
	var pending := {}
	for z in int(layout.patch_rows):
		for x in int(layout.patch_cols):
			requests.append_array(PackedInt32Array([x, z, 2000]))
			pending[Vector2i(x, z)] = true
	simulation.request_terrain_patch_payloads(requests)
	var material := StandardMaterial3D.new()
	material.cull_mode = BaseMaterial3D.CULL_DISABLED
	var checker := Image.create(2, 2, false, Image.FORMAT_RGB8)
	checker.set_pixel(0, 0, Color(0.30, 0.45, 0.24))
	checker.set_pixel(1, 1, Color(0.30, 0.45, 0.24))
	checker.set_pixel(1, 0, Color(0.36, 0.52, 0.28))
	checker.set_pixel(0, 1, Color(0.36, 0.52, 0.28))
	material.albedo_texture = ImageTexture.create_from_image(checker)
	material.texture_filter = BaseMaterial3D.TEXTURE_FILTER_NEAREST
	material.uv1_triplanar = true
	material.uv1_world_triplanar = true
	material.uv1_scale = Vector3.ONE / 40.0
	var deadline := Time.get_ticks_msec() + 30000
	while not pending.is_empty() and Time.get_ticks_msec() < deadline:
		var ready: Dictionary = simulation.poll_ready_terrain_patch_payloads(16)
		var retries: PackedInt64Array = ready.get("retry_requests", PackedInt64Array())
		var again := PackedInt32Array()
		for index in range(0, retries.size(), 4):
			if pending.has(Vector2i(retries[index], retries[index + 1])):
				again.append_array(PackedInt32Array([retries[index], retries[index + 1], retries[index + 2]]))
		if not again.is_empty():
			simulation.request_terrain_patch_payloads(again)
		for data in ready.get("patches", []):
			var key := Vector2i(data["patch_x"], data["patch_z"])
			if not pending.has(key):
				continue
			pending.erase(key)
			var instance := MeshInstance3D.new()
			instance.mesh = renderer._terrain_patch_mesh_from_data(data, 1, 1)
			instance.position = Vector3(data["world_origin_x"] + data["world_size_x"] * 0.5, 0.0, data["world_origin_z"] + data["world_size_z"] * 0.5)
			instance.material_override = material
			result.add_child(instance)
		await process_frame
	_expect(pending.is_empty(), "terrain capture must load every patch")
	renderer.free()
	return result

func _reference(fixture: Dictionary, sloped: bool) -> void:
	var label := "%s_%s" % [fixture.name, "slope" if sloped else "flat"]
	var failures_before := _failures
	_expect(simulation.create_blank_world(WORLD_M, WORLD_M, 8.0, 128.0, 0.0), "Reference world must load")
	simulation.set_simulation_speed(0.0)
	simulation.set_weather_scenario(0, 0.0)
	if sloped:
		simulation.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128, 0), 0.0, Vector2(128, 0), SLOPE_RISE_M, 1.0)
	for road: Dictionary in fixture.roads:
		if not await _commit(road):
			return
	if fixture.has("class_at"):
		var edge: int = simulation.get_hovered_edge(fixture.class_at.x, fixture.class_at.y)
		_expect(edge >= 0, "%s: an edge at %s" % [label, fixture.class_at])
		simulation.set_edge_class(edge, fixture["class"])
		_expect(simulation.get_edge_class(edge) == fixture["class"], "%s: edge %d takes class %d" % [label, edge, fixture["class"]])
		for frame in range(4):
			await process_frame
	var state: Dictionary = simulation.get_road_benchmark_state()
	var viewport := SubViewport.new()
	viewport.size = CAPTURE_SIZE
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var scene := Node3D.new()
	viewport.add_child(scene)
	simulation.reparent(scene)
	var focus := _surface(Vector2.ZERO)
	var camera := Camera3D.new()
	scene.add_child(camera)
	var offset := CAMERA_HEIGHT_M / tan(deg_to_rad(CAMERA_PITCH_DEG))
	camera.position = focus + Vector3(0.0, CAMERA_HEIGHT_M, offset)
	camera.rotation_degrees = Vector3(-CAMERA_PITCH_DEG, 0.0, 0.0)
	camera.fov = 70.0
	camera.far = 3000.0
	camera.current = true
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color(0.62, 0.74, 0.88)
	environment.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.environment.ambient_light_color = Color(0.75, 0.80, 0.90)
	environment.environment.ambient_light_energy = 0.55
	scene.add_child(environment)
	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-50.0, -35.0, 0.0)
	light.light_energy = 1.1
	scene.add_child(light)
	var road := RoadToolScript.new()
	road.name = "RoadTool"
	road.simulation_node = simulation
	road.road_mesh_root = Node3D.new()
	road.add_child(road.road_mesh_root)
	var generation: int = simulation.get_network_render_generation()
	_expect(road.update_main_mesh(generation) == generation, "%s uses the committed road chunks" % label)
	road.remove_child(road.road_mesh_root)
	scene.add_child(road.road_mesh_root)
	var road_chunks: int = road._road_chunk_instances.size()
	_expect(road_chunks > 0, "%s must have road chunk meshes" % label)
	var terrain: Node3D = await _terrain_node() if fixture.get("terrain", true) else Node3D.new()
	scene.add_child(terrain)
	var sky_colour := Color(0.62, 0.74, 0.88)
	var grey := Color(0.5, 0.5, 0.5)
	var views := [
		{"view": "oblique", "terrain": true},
		{"view": "oblique_roads", "terrain": false},
		{"view": "plan", "terrain": false, "ortho": true},
	]
	var images: Array[Dictionary] = []
	for view: Dictionary in views:
		# Hiding terrain nodes is purely visual: it removes the coplanar z-fight over unclipped asphalt.
		terrain.visible = view.terrain
		environment.environment.background_color = sky_colour if view.terrain else grey
		var ortho: bool = view.get("ortho", false)
		if ortho:
			camera.projection = Camera3D.PROJECTION_ORTHOGONAL
			camera.size = PLAN_HEIGHT_M # keep-height: width = 37.5 * 1.6 = 60 m
			camera.position = focus + Vector3(0.0, 100.0, 0.0)
			camera.rotation_degrees = Vector3(-90.0, 0.0, 0.0)
		for frame in range(6):
			await process_frame
		await RenderingServer.frame_post_draw
		var png := "%s_%s.png" % [label, view.view]
		_expect(viewport.get_texture().get_image().save_png(_output.path_join(png)) == OK, "Reference image must save")
		var pose := {"position": [camera.position.x, camera.position.y, camera.position.z], "pitch_deg": camera.rotation_degrees.x, "look_target": [focus.x, focus.y, focus.z]}
		if ortho:
			pose["projection"] = "orthogonal"
			pose["ortho_size_m"] = [PLAN_HEIGHT_M * float(CAPTURE_SIZE.x) / float(CAPTURE_SIZE.y), PLAN_HEIGHT_M]
		else:
			pose["fov_deg"] = camera.fov
		images.append({"view": view.view, "image": png, "camera": pose, "terrain_visible": view.terrain,
			"background": "sky" if view.terrain else "neutral grey"})
	var record := {"name": label, "fixture": fixture.name, "slope": "2.5%" if sloped else "flat",
		"nodes": state.get("nodes", -1), "live_edges": state.get("live_edges", -1), "road_chunks": road_chunks,
		"images": images, "passed": _failures == failures_before}
	if fixture.has("angle_deg"):
		record["arm_angle_deg"] = fixture.angle_deg
	_records.append(record)
	print("road_junction_reference %s nodes=%s edges=%s chunks=%d" % [label, record.nodes, record.live_edges, road_chunks])
	simulation.reparent(root)
	road.free()
	viewport.free()
	await process_frame
