# SPDX-License-Identifier: GPL-2.0-only

## Oblique reference renders of scripted road junctions through the production road chunk renderer.
## Set METRUM_ROAD_JUNCTION_REFERENCE_DIR for PNGs and manifest.json; use a real rendering device.
## Without it the test exits at once. Each fixture is rendered flat and on a 2.5% slope, in three views:
## oblique (terrain), oblique_roads (terrain hidden, grey) and plan (top-down orthographic, 60 x 37.5 m);
## bridges add a fourth, side (the deck's side from 12 m, TERRAIN-07-B's fascias).
## Every fixture renders through the production graded terrain in the close, plan, and distance
## views and counts background pinholes in plan_terrain. METRUM_ROAD_JUNCTION_FIXTURES limits the
## fixtures (comma list of names), so a long set can run as several sessions. Structures render
## over ground sculpted
## after the commits (a hollow under the bridge, a mound over the tunnel), with a view from under
## the bridge at its road mouth and one at the tunnel's portal; kuopio_t2_wall loads the Kuopio
## reference save at its conflict wall site (T2). METRUM_ROAD_JUNCTION_CONNECTORS=1 draws the
## `junction_connectors` debug overlay (arcs) over every view (TRAFFIC-12). METRUM_ROAD_JUNCTION_SAVE
## names another save and METRUM_ROAD_JUNCTION_SAVE_FOCI its junctions to render (comma list of
## name:x:z), each as a fixture of that name.
extends SceneTree

const RoadToolScript := preload("res://scripts/tools/road_tool.gd")
const TerrainScript := preload("res://scripts/renderers/terrain.gd")
const GrassScript := preload("res://scripts/renderers/grass.gd")
const JunctionOverlayScript := preload("res://scripts/renderers/junction_debug_overlay.gd")
const WORLD_M := 512.0
const ARM_M := 80.0
const SLOPE_RISE_M := 6.4 # over 256 m along +x: 2.5%
const STEEP_RISE_M := 20.48 # over 256 m along +x: 8%
# Distance renders (TERRAIN-07-D): the camera this far from the junction, 25 degrees down.
const DISTANCES_M := [200.0, 400.0, 1000.0, 3000.0]
const DISTANCE_PITCH_DEG := 25.0
const PINHOLE := Color(1.0, 0.0, 1.0)
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
	"side": "bridge_* only: perspective, 40 degree fov, 1 m above the surface 12 m beside the bridge's middle, looking at its deck edge 0.3 m below the surface",
}
var _failures := 0
var _records: Array[Dictionary] = []
var _output := ""
var simulation: SimulationNode
var _pinholes := {}

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
	var only := OS.get_environment("METRUM_ROAD_JUNCTION_FIXTURES").split(",", false)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	root.add_child(simulation)
	for fixture in _fixtures():
		if not only.is_empty() and not only.has(fixture.name):
			continue
		for slope: Array in _slopes(fixture):
			await _reference(fixture, slope[0], slope[1])
	var manifest := {"engine": Engine.get_version_info(), "adapter": RenderingServer.get_video_adapter_name(),
		"vendor": RenderingServer.get_video_adapter_vendor(), "capture_size": [CAPTURE_SIZE.x, CAPTURE_SIZE.y],
		"world": {"size_m": WORLD_M, "cell_m": 8.0, "chunk_m": 128.0, "slope": "2.5% rising toward +x"},
		"conventions": CONVENTIONS, "lighting": "fixed directional light and ambient colour; weather scenario 0 (clear); no time-of-day API is used",
		"fixtures": _records, "failures": _failures, "terrain_mode": "graded", "pinholes": _pinholes}
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
	# the continuation of a road end (pass-through span mouth), made structures with set_edge_class
	# (each asserts its edge class), rendered over graded ground sculpted after the commits below.
	for structure in [["bridge", 1], ["tunnel", 2]]:
		var t := {"name": "%s_t" % structure[0], "class_at": Vector2(0, arm * 0.5), "class": structure[1], "structure": true,
			"roads": [_road(Vector2(-arm, 0), Vector2(arm, 0)), _road(Vector2(0, 0), Vector2(0, arm))]}
		var continuation := {"name": "%s_continuation" % structure[0], "class_at": Vector2(arm * 0.5, 0), "class": structure[1], "structure": true,
			"roads": [_road(Vector2(-arm, 0), Vector2.ZERO), _road(Vector2.ZERO, Vector2(arm, 0))]}
		if structure[0] == "bridge":
			# The side view's eye and the deck point it looks at.
			t["side"] = [Vector2(12.0, arm * 0.5), Vector2(0, arm * 0.5)]
			continuation["side"] = [Vector2(arm * 0.5, 12.0), Vector2(arm * 0.5, 0)]
		result.append(t)
		result.append(continuation)
	# The B spike's fixtures (TERRAIN-07-D item 6). A street grid of closed 96 m blocks with a
	# T split at the origin, the last commit.
	result.append({"name": "city_grid_t", "distance": true, "roads": [
		_road(Vector2(-150, 0), Vector2(150, 0)), _road(Vector2(-150, 96), Vector2(150, 96)),
		_road(Vector2(-150, -96), Vector2(150, -96)), _road(Vector2(-96, -150), Vector2(-96, 150)),
		_road(Vector2(96, -150), Vector2(96, 150)), _road(Vector2(0, 0), Vector2(0, 96))]})
	# The T on flat ground, then the terrain sloped under it (roads stay): cut beside the east
	# arm, fill beside the west arm, cross-fall across the side arm. fill_corner looks at the
	# kerb curve between the west arm and the side arm from about 7 m, at the road's height,
	# where the level road stands on a fill of about 0.5 m: off the lattice lines the graded
	# cut edge may sag onto the skirt by up to k s / 2 (T34 (b), 12.5 cm at GL0); the
	# straight edges lie on GL0 lines and do not sag.
	result.append({"name": "t_cutfill", "terrain_after": true, "roads": [_road(Vector2(-arm, 0), Vector2(arm, 0)), _road(Vector2(0, 0), Vector2(0, arm))],
		"edge_views": [{"view": "fill_corner", "from": Vector2(-11.0, 11.0), "eye_m": 0.8, "at": Vector2(-6.1, 6.1), "at_m": -0.1}]})
	# A 3 m mound across the side arm and a 2 m hollow under the west arm, sculpted before the
	# roads, so the roads' profiles cut and fill and the mound's flanks fall across them.
	result.append({"name": "t_mound", "sculpt": [[Vector2(0, 40), 30.0, 0.15], [Vector2(-45, 0), 25.0, -0.10]],
		"roads": [_road(Vector2(-arm, 0), Vector2(arm, 0)), _road(Vector2(0, 0), Vector2(0, arm))]})
	result[0]["distance"] = true
	# TERRAIN-07-C's structure walls at a road mouth (TERRAIN-07-D round 3): the side road of
	# the T made a bridge over a hollow sculpted after the commits, and a tunnel under a
	# mound, each seen at its mouth.
	for fixture in result:
		if fixture.name == "bridge_t":
			fixture["sculpt_after"] = [[Vector2(0, 32), 26.0, -0.25]]
			fixture["mouth_views"] = [
				{"view": "under_bridge", "from": Vector2(7.0, 24.0), "eye_m": 1.0, "at": Vector2(0.0, 7.0), "at_m": -0.6},
				{"view": "mouth_side", "from": Vector2(18.0, 14.0), "eye_m": 2.0, "at": Vector2(0.0, 9.0), "at_m": -1.0}]
		elif fixture.name == "tunnel_t":
			fixture["sculpt_after"] = [[Vector2(0, 46), 30.0, 0.3]]
			fixture["mouth_views"] = [
				{"view": "portal", "from": Vector2(6.0, -12.0), "eye_m": 2.5, "at": Vector2(0.0, 14.0), "at_m": 2.0}]
		# The continuations (TERRAIN-07-G round 4 restored them on the graded path, with the
		# bridge's side view of TERRAIN-07-B's fascias): the same hollow and mound across the
		# east arm. Tunnels are not a focus (the user, 2026-10-26): theirs is a no-crash run.
		elif fixture.name == "bridge_continuation":
			fixture["sculpt_after"] = [[Vector2(32, 0), 26.0, -0.25]]
		elif fixture.name == "tunnel_continuation":
			fixture["sculpt_after"] = [[Vector2(46, 0), 30.0, 0.3]]
	# The Kuopio reference save's conflict wall site (TERRAIN-07-C round 4, T2 (c)).
	result.append({"name": "kuopio_t2_wall", "save": "res://../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite",
		"focus": Vector2(-16.3, 1182.8), "roads": []})
	# TERRAIN-07-F: exercise the production grass mask on Kuopio before and after the
	# established 60 m probe spur changes the graded cells under it.
	result.append({"name": "kuopio_grass_before", "save": "res://../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite",
		"focus": Vector2(31.45, -6.71), "roads": [], "grass": true})
	result.append({"name": "kuopio_grass_after", "save": "res://../benchmarks/fixtures/kuopio-terrain/kuopio-terrain-map.sqlite",
		"focus": Vector2(31.45, -6.71), "grass": true,
		"roads": [_road(Vector2(31.45, -6.71), Vector2(57.23, 47.47))]})
	var save := OS.get_environment("METRUM_ROAD_JUNCTION_SAVE")
	for focus: String in OS.get_environment("METRUM_ROAD_JUNCTION_SAVE_FOCI").split(",", false):
		var parts := focus.split(":")
		result.append({"name": parts[0], "save": save, "focus": Vector2(float(parts[1]), float(parts[2])), "roads": []})
	return result

# [label, rise over 256 m]: flat and 2.5%, with the spike's 8% on the T, four-way, bend and grid.
func _slopes(fixture: Dictionary) -> Array:
	if fixture.has("save") or fixture.has("structure"):
		return [["flat", 0.0]]
	if fixture.get("terrain_after", false):
		return [["steep", STEEP_RISE_M]]
	if fixture.has("sculpt"):
		return [["flat", 0.0]]
	var slopes := [["flat", 0.0], ["slope", SLOPE_RISE_M]]
	if fixture.name in ["t", "four_way", "bend_45", "city_grid_t"]:
		slopes.append(["steep", STEEP_RISE_M])
	return slopes

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

func _reference(fixture: Dictionary, slope_name: String, rise_m: float) -> void:
	var label := "%s_%s" % [fixture.name, slope_name]
	var failures_before := _failures
	if fixture.has("save"):
		_expect(simulation.load_game(ProjectSettings.globalize_path(fixture.save)), "%s: the save must load" % label)
	else:
		_expect(simulation.create_blank_world(WORLD_M, WORLD_M, 8.0, 128.0, 0.0), "Reference world must load")
	simulation.set_simulation_speed(0.0)
	simulation.set_weather_scenario(0, 0.0)
	for brush: Array in fixture.get("sculpt", []):
		simulation.sculpt_terrain(brush[0], brush[1], brush[2])
	var after: bool = fixture.get("terrain_after", false)
	if rise_m > 0.0 and not after:
		simulation.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128, 0), 0.0, Vector2(128, 0), rise_m, 1.0)
	for road: Dictionary in fixture.roads:
		if not await _commit(road):
			return
	if rise_m > 0.0 and after:
		# Centred on the junction: the ground rises beside the east arm's end and falls at the west's.
		simulation.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128, 0), -rise_m * 0.5, Vector2(128, 0), rise_m * 0.5, 1.0)
		for frame in range(10):
			await process_frame
	if fixture.has("class_at"):
		var edge: int = simulation.get_hovered_edge(fixture.class_at.x, fixture.class_at.y)
		_expect(edge >= 0, "%s: an edge at %s" % [label, fixture.class_at])
		simulation.set_edge_class(edge, fixture["class"])
		_expect(simulation.get_edge_class(edge) == fixture["class"], "%s: edge %d takes class %d" % [label, edge, fixture["class"]])
		for frame in range(4):
			await process_frame
	# Ground moved under (or over) a structure after it was built: the roads stay.
	for brush: Array in fixture.get("sculpt_after", []):
		simulation.sculpt_terrain(brush[0], brush[1], brush[2])
	for frame in range(10):
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
	var focus := _surface(fixture.get("focus", Vector2.ZERO))
	var camera := Camera3D.new()
	scene.add_child(camera)
	var offset := CAMERA_HEIGHT_M / tan(deg_to_rad(CAMERA_PITCH_DEG))
	camera.position = focus + Vector3(0.0, CAMERA_HEIGHT_M, offset)
	camera.rotation_degrees = Vector3(-CAMERA_PITCH_DEG, 0.0, 0.0)
	camera.fov = 70.0
	camera.far = 5000.0
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
	if OS.get_environment("METRUM_ROAD_JUNCTION_CONNECTORS") == "1":
		_expect(MetrumDebug.set_visual_mode("junction_connectors", "arcs"), "junction_connectors must switch on")
		var overlay := JunctionOverlayScript.new()
		overlay.name = "JunctionDebugOverlay"
		scene.add_child(overlay)
	_expect(road_chunks > 0, "%s must have road chunk meshes" % label)
	var images: Array[Dictionary] = []
	await _compare_views(fixture, label, scene, camera, environment.environment, focus, images)
	_finish_record(fixture, label, rise_m, state, road_chunks, images, failures_before)
	simulation.reparent(root)
	road.free()
	viewport.free()
	await process_frame

func _finish_record(fixture: Dictionary, label: String, rise_m: float, state: Dictionary, road_chunks: int, images: Array[Dictionary], failures_before: int) -> void:
	var record := {"name": label, "fixture": fixture.name, "slope": "%.1f%%" % (rise_m / 256.0 * 100.0),
		"nodes": state.get("nodes", -1), "live_edges": state.get("live_edges", -1), "road_chunks": road_chunks,
		"images": images, "passed": _failures == failures_before}
	if fixture.has("angle_deg"):
		record["arm_angle_deg"] = fixture.angle_deg
	_records.append(record)
	print("road_junction_reference %s nodes=%s edges=%s chunks=%d" % [label, record.nodes, record.live_edges, road_chunks])

# The production terrain renderer uploads nothing while the network generation is unacknowledged
# (network_renderer.gd acknowledges it in the game); each terrain built after this reads the
# published state whole.
func _acknowledge_network(label: String) -> void:
	var deadline := Time.get_ticks_msec() + 10000
	while simulation.is_network_dirty() and Time.get_ticks_msec() < deadline:
		simulation.acknowledge_network_render(simulation.get_network_render_generation())
		await process_frame
	_expect(not simulation.is_network_dirty(), "%s: the network render must be acknowledged" % label)

# The production graded terrain renderer beside the SimulationNode. Water is an empty stand-in.
func _production_terrain(scene: Node3D) -> Node3D:
	if scene.get_node_or_null("Water") == null:
		var water := Node3D.new()
		water.name = "Water"
		scene.add_child(water)
	var terrain := TerrainScript.new()
	terrain.name = "Terrain"
	scene.add_child(terrain)
	return terrain

# Waits until the terrain renderer has settled (no pending patch work, graded tile, list, count,
# edit batch, cell set build or patch drawing nothing), for three frames in a row at the current
# camera. In every frame of the wait, while the residency streams the fixture in and the patches
# change step with the camera, no patch draws its PlaneMesh where its list names a graded tile
# (or has not arrived) and no set draws with a listed tile missing (T53 (a)); settled, no patch
# draws nothing (`TERRAIN-07-G` round 6: the base ground over the graded road in a cut).
func _settle_terrain(terrain, label: String) -> void:
	var stable := 0
	var violations := 0
	var partial := 0
	var deadline := Time.get_ticks_msec() + 60000
	while stable < 3 and Time.get_ticks_msec() < deadline:
		await process_frame
		violations = max(violations, terrain.graded_tiles.plane_violations())
		partial = max(partial, terrain.graded_tiles.partial_sets())
		stable = stable + 1 if terrain.is_render_settled() else 0
	_expect(stable >= 3, "%s: the terrain must settle" % label)
	_expect(violations == 0, "%s: up to %d patches drew their PlaneMesh over listed graded tiles" % [label, violations])
	_expect(partial == 0, "%s: up to %d sets drew with a listed tile missing" % [label, partial])
	var holes: int = terrain.graded_tiles.hole_patches()
	_expect(holes == 0, "%s: %d settled patches draw nothing" % [label, holes])

# The spike's views of one terrain mode, then the distance renders where the fixture asks:
# oblique, the side arm's two sidewalk edges and a corner from 2.5 m, plan_terrain (counting
# background pixels: every one is a pinhole), and the junction from 200 m to 3 km.
func _compare_views(fixture: Dictionary, label: String, scene: Node3D, camera: Camera3D, environment: Environment, focus: Vector3, images: Array[Dictionary]) -> void:
	var mode := "graded"
	await _acknowledge_network("%s %s" % [label, mode])
	var terrain := _production_terrain(scene)
	var grass: Node3D = null
	if fixture.get("grass", false):
		grass = GrassScript.new()
		grass.name = "Grass"
		scene.add_child(grass)
	var offset := CAMERA_HEIGHT_M / tan(deg_to_rad(CAMERA_PITCH_DEG))
	var views: Array = [{
		"view": "grass", "from": Vector2(-18.0, 31.0), "eye_m": 5.0,
		"at": Vector2(13.0, 19.0), "at_m": 0.0,
	}] if fixture.get("grass", false) else [
		{"view": "oblique"},
		{"view": "close_east", "from": Vector2(15.0, 30.0), "at": Vector2(5.0, 22.0)},
		{"view": "close_west", "from": Vector2(-15.0, 30.0), "at": Vector2(-5.0, 22.0)},
		{"view": "close_corner", "from": Vector2(16.0, 20.0), "at": Vector2(6.0, 8.0)},
		{"view": "plan_terrain", "ortho": true},
	]
	views.append_array(fixture.get("mouth_views", []))
	views.append_array(fixture.get("edge_views", []))
	# A bridge deck's side from 12 m (TERRAIN-07-B's fascias), at 40 degrees.
	var side: Array = fixture.get("side", [])
	if not side.is_empty():
		views.append({"view": "side", "from": side[0], "eye_m": 1.0, "at": side[1], "at_m": -0.3, "fov_deg": 40.0})
	# Close and mouth views are placed relative to the focus (the origin but on a save).
	var focus_xz := Vector2(focus.x, focus.z)
	if fixture.get("distance", false):
		for distance_m: float in DISTANCES_M:
			views.append({"view": "far_%d" % int(distance_m), "distance": distance_m})
	for view: Dictionary in views:
		var ortho: bool = view.get("ortho", false)
		camera.projection = Camera3D.PROJECTION_ORTHOGONAL if ortho else Camera3D.PROJECTION_PERSPECTIVE
		camera.fov = float(view.get("fov_deg", 70.0))
		camera.position = focus + Vector3(0.0, CAMERA_HEIGHT_M, offset)
		camera.rotation_degrees = Vector3(-CAMERA_PITCH_DEG, 0.0, 0.0)
		var target := focus
		if view.has("from"):
			camera.position = _surface(focus_xz + view.from) + Vector3(0.0, float(view.get("eye_m", 2.5)), 0.0)
			target = _surface(focus_xz + view.at) + Vector3(0.0, float(view.get("at_m", 0.0)), 0.0)
			camera.look_at(target, Vector3.UP)
		elif view.has("distance"):
			var pitch := deg_to_rad(DISTANCE_PITCH_DEG)
			var distance_m: float = view.distance
			camera.position = focus + Vector3(0.0, distance_m * sin(pitch), distance_m * cos(pitch))
			camera.fov = 30.0
			camera.look_at(focus, Vector3.UP)
		elif ortho:
			camera.size = PLAN_HEIGHT_M # keep-height: width = 37.5 * 1.6 = 60 m
			camera.position = focus + Vector3(0.0, 100.0, 0.0)
			camera.rotation_degrees = Vector3(-90.0, 0.0, 0.0)
		# plan_terrain looks only at ground: a background pixel there is a hole in the surface.
		environment.background_color = PINHOLE if ortho else Color(0.62, 0.74, 0.88)
		await _settle_terrain(terrain, "%s %s %s" % [label, mode, view.view])
		for frame in range(6):
			await process_frame
		await RenderingServer.frame_post_draw
		var image: Image = (camera.get_viewport() as SubViewport).get_texture().get_image()
		var png := "%s_%s_%s.png" % [label, mode, view.view]
		_expect(image.save_png(_output.path_join(png)) == OK, "Reference image must save")
		var entry := {"view": view.view, "terrain_mode": mode, "image": png, "terrain_visible": true,
			"camera": {"position": [camera.position.x, camera.position.y, camera.position.z],
				"look_target": [target.x, target.y, target.z], "fov_deg": camera.fov, "orthogonal": ortho}}
		if ortho:
			var holes := _count_colour(image, PINHOLE)
			_pinholes["%s_%s" % [label, mode]] = holes
			entry["pinholes"] = holes
			_expect(holes == 0, "%s %s: %d background pixels in plan_terrain" % [label, mode, holes])
		images.append(entry)
	if grass != null:
		grass.free()
	terrain.free()
	await process_frame

func _count_colour(image: Image, colour: Color) -> int:
	var count := 0
	for y in image.get_height():
		for x in image.get_width():
			var pixel := image.get_pixel(x, y)
			if absf(pixel.r - colour.r) < 0.02 and absf(pixel.g - colour.g) < 0.02 and absf(pixel.b - colour.b) < 0.02:
				count += 1
	return count
