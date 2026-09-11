# SPDX-License-Identifier: GPL-2.0-only

## Native compiled-road rendering, restoration and generation-fence regressions.
extends SceneTree

const RoadToolScript := preload("res://scripts/tools/road_tool.gd")
const WorldMaterialsScript := preload("res://scripts/renderers/world_materials.gd")
const TerrainScript := preload("res://scripts/renderers/terrain.gd")
const ZoningToolScript := preload("res://scripts/tools/zoning_tool.gd")
var _failures := 0
var simulation: SimulationNode
var _capture_baseline_sky := {}
var _capture_preview_sky := {}

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _commit(points: PackedVector3Array, forward: int = 1, backward: int = 1) -> bool:
	# Terrain/water edits can advance query validity independently of resident road meshes.
	var generation := simulation.get_network_render_generation()
	simulation.add_road_with_snap(points, forward, backward, true)
	var deadline := Time.get_ticks_msec() + 20000
	while Time.get_ticks_msec() < deadline:
		if simulation.get_network_render_generation() > generation:
			return true
		await process_frame
	_expect(false, "native junction fixture commit must finish")
	return false

func _run() -> void:
	await _test_graded_paving_payload()
	simulation = SimulationNode.new()
	root.add_child(simulation)
	await _test_zoning_buildability_feedback()
	for fixture in [
		{"name": "isolated", "end_x": 48.0, "end_z": 0.0, "forward": 1, "backward": 1, "isolated": true},
		{"name": "isolated_bridge", "end_x": 48.0, "end_z": 0.0, "forward": 1, "backward": 1, "isolated": true, "height": 10.0},
		{"name": "isolated_sloped", "end_x": 48.0, "end_z": 0.0, "forward": 1, "backward": 1, "isolated": true, "sloped": true, "neighbor": true},
		{"name": "t", "end_z": 0.0, "forward": 1, "backward": 1},
		{"name": "cross", "end_z": 48.0, "forward": 1, "backward": 1},
		{"name": "wide_t", "end_z": 0.0, "forward": 2, "backward": 1},
		{"name": "water_reset", "end_z": 0.0, "forward": 1, "backward": 1},
		{"name": "bend", "end_z": -48.0, "forward": 1, "backward": 1, "endpoint_join": true},
		{"name": "wide_bend", "end_z": -48.0, "forward": 2, "backward": 1, "endpoint_join": true},
		{"name": "continuation", "end_x": 48.0, "end_z": 0.0, "forward": 1, "backward": 1, "endpoint_join": true},
		{"name": "double_t", "end_z": 0.0, "forward": 1, "backward": 1},
		{"name": "sloped_t", "end_z": 0.0, "forward": 1, "backward": 1, "sloped": true},
	]:
		await _fixture(fixture)
	simulation.free()
	if _failures == 0:
		print("road_junction_preview_test: PASS")
	quit(_failures)

func _test_zoning_buildability_feedback() -> void:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "zoning fixture world must load")
	simulation.set_simulation_speed(0.0)
	if not await _commit(PackedVector3Array([Vector3(-96, 0, 0), Vector3(96, 0, 0)])):
		return
	_expect(simulation.get_registered_asset_ids().is_empty(), "zoning fixture intentionally has no building assets")
	var before := simulation.get_zoning_site_dependencies()
	var rejected := simulation.get_zoning_parcel_preview(0.0, 16.0, 1, 2, 2)
	_expect(not rejected.is_empty() and not rejected.get("valid", true), "unsupported lot must remain visible as invalid")
	_expect(str(rejected.get("reason", "")).contains("asset"), "unsupported lot must explain missing compatible assets")
	_expect(not simulation.apply_zoning_parcel_at(0.0, 16.0, 1, 2, 2), "unsupported zoned parcel must not commit")
	_expect(not simulation.has_zoning_parcel_at(0.0, 16.0), "failed zoning must not insert a parcel")
	_expect(simulation.get_zoning_site_dependencies() == before, "feasibility must not change road, terrain or zoning")
	var drag := simulation.get_zoning_parcel_drag_preview_packed(-60.0, 16.0, 60.0, 16.0, 1, 2, 2, 0.0)
	_expect(int(drag.get("parcel_count", 0)) > 0 and int(drag.get("valid_count", -1)) == 0, "drag retains rejected geometries with no valid lots")
	var colors: PackedColorArray = drag.get("colors", PackedColorArray())
	_expect(colors.size() == int(drag.get("parcel_count", 0)), "every drag parcel needs its feasibility color")
	for color in colors:
		_expect(color.r > color.g, "rejected zoning must be red")
	var tool := ZoningToolScript.new()
	tool._feasibility_label = Label.new()
	tool.add_child(tool._feasibility_label)
	tool._show_feasibility(drag)
	_expect(not tool._feasibility_label.text.is_empty(), "zoning tool must display the Rust rejection reason")
	_expect(tool._build_packed_parcels_mesh(drag, true) != null, "zoning tool must render rejected parcels")
	tool.free()
	_expect(simulation.get_zoning_parcel_preview(0.0, 16.0, 0, 2, 2).get("valid", false), "free parcels do not require a building asset")
	_expect(simulation.apply_zoning_parcel_at(0.0, 16.0, 0, 2, 2), "free parcel placement must remain possible")
	_expect(simulation.get_zoning_site_dependencies() != before, "zoning edits invalidate retained cursor previews")

func _test_graded_paving_payload() -> void:
	var terrain := TerrainScript.new()
	var data := {
		"terrain_mesh_vertices": PackedVector3Array([Vector3.ZERO, Vector3(4, 1, 0), Vector3(0, 0, 4)]),
		"terrain_mesh_normals": PackedVector3Array([Vector3.UP, Vector3.UP, Vector3.UP]),
		"terrain_mesh_uvs": PackedVector2Array([Vector2.ZERO, Vector2.RIGHT, Vector2.UP]),
		"terrain_mesh_indices": PackedInt32Array([0, 1, 2]),
		"terrain_mesh_colors": PackedColorArray([Color.RED, Color.RED, Color.RED]),
	}
	_expect(terrain._triangle_mesh_payload_is_valid(data, "terrain_mesh", true), "graded paving payload must validate")
	var mesh: ArrayMesh = terrain._baked_terrain_patch_mesh(data)
	var arrays := mesh.surface_get_arrays(0)
	_expect(arrays[Mesh.ARRAY_VERTEX] == data.terrain_mesh_vertices, "paving upload preserves graded vertices")
	_expect(arrays[Mesh.ARRAY_COLOR] == data.terrain_mesh_colors, "paving upload preserves material tags")
	data.terrain_mesh_colors = PackedColorArray([Color.RED])
	_expect(not terrain._triangle_mesh_payload_is_valid(data, "terrain_mesh", true), "incomplete paving tags must fail closed")
	terrain.free()
	if DisplayServer.get_name() == "headless":
		return
	# Exercise the actual terrain shader, not the unshaded coverage-test material below.
	var viewport := SubViewport.new()
	viewport.size = Vector2i(128, 128)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	var material := ShaderMaterial.new()
	material.shader = TerrainScript.TERRAIN_SHADER
	material.set_shader_parameter("height_is_baked", true)
	var texture_image := Image.create(2, 2, false, Image.FORMAT_RGBA8)
	texture_image.fill(Color(1.0, 0.0, 0.0))
	material.set_shader_parameter("site_asphalt_albedo_tex", ImageTexture.create_from_image(texture_image))
	var instance := MeshInstance3D.new()
	instance.mesh = mesh
	instance.material_override = material
	viewport.add_child(instance)
	var camera := Camera3D.new()
	viewport.add_child(camera)
	camera.position = Vector3(2.0, 8.0, 6.0)
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 6.0
	camera.look_at(Vector3(2.0, 0.5, 2.0))
	camera.current = true
	for frame in range(4):
		await process_frame
	await RenderingServer.frame_post_draw
	var rendered := viewport.get_texture().get_image()
	var paved_pixels := 0
	for y in range(rendered.get_height()):
		for x in range(rendered.get_width()):
			var color := rendered.get_pixel(x, y)
			if color.r > 0.2 and color.g < 0.05 and color.b < 0.05:
				paved_pixels += 1
	_expect(paved_pixels > 100, "terrain shader must render tagged graded paving with its site material")
	viewport.queue_free()
	await process_frame

func _fixture(fixture: Dictionary) -> void:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "junction fixture world must load")
	simulation.set_simulation_speed(0.0)
	if fixture.get("sloped", false):
		simulation.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128.0, 0.0), 0.0, Vector2(128.0, 0.0), 6.4, 1.0)
	var endpoint_join: bool = fixture.get("endpoint_join", false)
	var isolated: bool = fixture.get("isolated", false)
	if not isolated:
		if not await _commit(PackedVector3Array([_ground_point(-60.0, 0.0), _ground_point(0.0 if endpoint_join else 60.0, 0.0)])):
			return
	# Unrelated road in the same render chunk must survive the temporary replacement.
	if not isolated or fixture.get("neighbor", false):
		if not await _commit(PackedVector3Array([_ground_point(24.0, 72.0), _ground_point(60.0, 72.0)])):
			return
	if fixture["name"] == "double_t":
		if not await _commit(PackedVector3Array([Vector3(-32.0, 0.0, -48.0), Vector3(-32.0, 0.0, 0.0)])):
			return
	var tool := RoadToolScript.new()
	tool.name = "RoadTool"
	tool.simulation_node = simulation
	tool.road_mesh_root = Node3D.new()
	tool.add_child(tool.road_mesh_root)
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._info_label = Label.new()
	tool.add_child(tool._info_label)
	tool._road_preview_material = WorldMaterialsScript.road_preview_material()
	tool.blueprint_mesh.material_override = tool._road_preview_material
	tool.fwd_lanes = fixture["forward"]
	tool.bkw_lanes = fixture["backward"]
	var generation := simulation.get_network_render_generation()
	_expect(tool.update_main_mesh(generation) == generation, "original chunks must hydrate before preview")
	var originals: Dictionary = tool._road_chunk_instances.duplicate()
	if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty():
		await _capture(tool.road_mesh_root, fixture["name"] + "_before", true)
	if fixture["name"] == "water_reset":
		simulation.begin_world_open_water_fill_preview(Vector2(160.0, 160.0), 5.0)
		_expect(simulation.cancel_world_open_water_fill_preview(), "water preview must cancel")
		_expect(simulation.get_road_tool_surface_generation() > generation, "water-only query revision must advance")
		_expect(simulation.get_network_render_generation() == generation, "water-only changes must retain resident road meshes")
	var before := simulation.get_road_benchmark_state()
	var points := PackedVector3Array([
		_ground_point(-48.0 if isolated else 0.0, 0.0 if endpoint_join or isolated else -48.0),
		_ground_point(fixture.get("end_x", 0.0), fixture["end_z"]),
	])
	for index in points.size():
		points[index].y += float(fixture.get("height", 0.0))
	var request := simulation.request_preview_road_surface_with_snap(points, tool.fwd_lanes, tool.bkw_lanes, true)
	var deadline := Time.get_ticks_msec() + 20000
	var preview: Variant = null
	while preview == null and Time.get_ticks_msec() < deadline:
		preview = simulation.get_preview_road_surface_result(request, 0)
		await process_frame
	_expect(preview is Dictionary and preview.get("is_valid", false), "%s native junction preview must validate: %s" % [fixture["name"], preview.get("invalid_reason", "missing") if preview is Dictionary else "timeout"])
	if not preview is Dictionary or not preview.get("is_valid", false):
		tool.free()
		return
	_expect(preview.has("junction_preview"), "junction preview must export the compiled local scene")
	_expect(preview.get("plan_state", "") == "ready", "complete backend plan must be ready")
	_expect(preview.get("terrain_plan_state", "") == "compiled", "a ready plan must have successfully compiled terrain products")
	if not preview.has("junction_preview"):
		tool.free()
		return
	var scene: Dictionary = preview["junction_preview"]
	if not isolated and not fixture.get("sloped", false):
		_expect_existing_road_height(scene)
	_expect(scene["retained_chunks"].is_empty() == originals.is_empty(), "retain unrelated roads, including an empty reference map")
	_expect(tool._draw_compiled_preview_surface(points, preview, preview), "exact junction must stage and display")
	var no_terrain_work: bool = preview.has("terrain_preview") and preview["terrain_preview"]["patches"].is_empty()
	_expect(tool._info_label.text.contains("terrain preview pending") != no_terrain_work, "only nonempty terrain batches require missing terrain resources")
	_expect(tool.blueprint_mesh.mesh == null, "exact junction must replace the stroke ribbon")
	_expect(tool._junction_preview._hidden.is_empty() == originals.is_empty(), "replace existing chunk instances only when they exist")
	for original in tool._junction_preview._hidden:
		_expect(not original.visible, "old curbs and markings must not show through the preview")
	var instances: Array = tool._junction_preview._instances.duplicate()
	_expect(tool._draw_compiled_preview_surface(points, preview, preview), "unchanged exact result must remain displayable")
	_expect(instances == tool._junction_preview._instances, "unchanged result must not upload meshes again")
	var stale := scene.duplicate(true)
	stale["source_mesh_generation"] = generation - 1
	_expect(not tool._junction_preview.show_preview(tool, stale, request + 1), "stale scenes must not replace resident geometry")
	stale = scene.duplicate(true)
	stale["surface_generation"] = preview["surface_generation"] - 1
	_expect(not tool._junction_preview.show_preview(tool, stale, request + 1), "stale query validation must be rejected even with matching meshes")
	var malformed := scene.duplicate(true)
	malformed["chunks"][0]["road_normals"] = PackedVector3Array()
	_expect(not tool._junction_preview.show_preview(tool, malformed, request + 1), "incomplete mesh batches must fail atomically")
	_expect(instances == tool._junction_preview._instances, "failed staging must preserve the complete previous preview")
	var retained_instances: Array = tool._junction_preview._retained_instances.duplicate()
	var shifted_points := points.duplicate()
	shifted_points[-1] += Vector3(1.0, 0.0, 0.0)
	var shifted_request := simulation.request_preview_road_surface_with_snap(shifted_points, tool.fwd_lanes, tool.bkw_lanes, true)
	var shifted: Variant = null
	deadline = Time.get_ticks_msec() + 20000
	while shifted == null and Time.get_ticks_msec() < deadline:
		shifted = simulation.get_preview_road_surface_result(shifted_request, tool._junction_preview.retained_revision)
		await process_frame
	_expect(shifted is Dictionary and shifted.get("is_valid", false), "successive pointer input must compile")
	if shifted is Dictionary and shifted.get("is_valid", false):
		_expect(not shifted["junction_preview"].has("retained_chunks"), "unchanged retained buffers must not cross the bridge again")
		_expect(tool._draw_compiled_preview_surface(shifted_points, shifted, shifted), "delta preview must install")
		_expect(retained_instances == tool._junction_preview._retained_instances, "retained GPU meshes must survive pointer updates")
		tool._clear_preview_visual()
		_expect(tool._draw_compiled_preview_surface(shifted_points, shifted, shifted), "detached retained meshes must support re-entering the same delta preview")
	var after := simulation.get_road_benchmark_state()
	for field in ["generation", "live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings"]:
		_expect(before[field] == after[field], "preview must not mutate authoritative " + field)
	if not no_terrain_work and (isolated or fixture["name"] in ["t", "sloped_t"]):
		await _test_paired_terrain_preview(tool, preview, points)
	if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty():
		_expect(tool._draw_compiled_preview_surface(points, preview, preview), "paired capture must restore the original input pose")
		await _capture(tool.road_mesh_root, fixture["name"])
	var moving: Dictionary = simulation.validate_road_candidate_with_snap(points, tool.fwd_lanes, tool.bkw_lanes, true)
	_expect(tool._draw_coarse_preview_surface(points, moving), "pointer motion must retain immediate feedback")
	_expect(tool._junction_preview.generation == -1, "explicit ribbon fallback must detach the junction")
	for original in originals.values():
		_expect(original.visible, "motion must restore original road instances")
	tool._draw_compiled_preview_surface(points, preview, preview)
	tool.cancel_road()
	for original in originals.values():
		_expect(original.visible, "cancel must restore original road instances")
	if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty():
		await _capture(tool.road_mesh_root, fixture["name"] + "_cancelled")
	tool._draw_compiled_preview_surface(points, preview, preview)
	if await _commit(preview["prepared_points"], tool.fwd_lanes, tool.bkw_lanes):
		var old_result: Variant = simulation.get_preview_road_surface_result(shifted_request, 0)
		_expect(old_result == null or old_result.get("plan_state", "") == "stale", "a road revision must stale the whole plan even when terrain did not change")
		generation = simulation.get_network_render_generation()
		_expect(tool.update_main_mesh(generation) == generation, "committed chunks must replace the source generation")
		tool._clear_preview_visual()
		for original in tool._road_chunk_instances.values():
			_expect(original.visible, "restoring a preview must not hide the new committed generation")
		if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty():
			await _capture(tool.road_mesh_root, fixture["name"] + "_committed")
	var detached_cache: Array = tool._junction_preview._retained_instances.duplicate()
	tool.free()
	# A rendered capture may retain VM temporaries until the frame's deferred-free boundary.
	await process_frame
	for instance in detached_cache:
		_expect(not is_instance_valid(instance), "destroying a tool must free detached retained meshes")

func _ground_point(x: float, z: float) -> Vector3:
	return Vector3(x, simulation.get_world_surface_height(Vector2(x, z)), z)

func _expect_existing_road_height(scene: Dictionary) -> void:
	# Far from the edited connection, the existing flat carriageway must stay at its committed
	# height even when whole neighbor spans are exported. This also runs without a GPU.
	var checked := 0
	var raised := 0
	for chunk in scene["chunks"]:
		var origin := Vector3(scene["chunk_origin_x_m"] + chunk["chunk_x"] * scene["chunk_span_m"], 0.0, scene["chunk_origin_z_m"] + chunk["chunk_z"] * scene["chunk_span_m"])
		for local in chunk.get("road_vertices", PackedVector3Array()):
			var point: Vector3 = local + origin
			if point.x < -32.0 and absf(point.z) < 3.51:
				if absf(point.y) >= 0.00001:
					raised += 1
				checked += 1
	_expect(checked > 0, "fixture must exercise an existing neighbor span")
	_expect(raised == 0, "existing road must not be lifted above its terrain cutout (%d vertices)" % raised)

func _test_paired_terrain_preview(tool: Node3D, preview: Dictionary, points: PackedVector3Array) -> void:
	_expect(preview.has("terrain_preview"), "valid ground-road plan must publish its terrain batch")
	if not preview.has("terrain_preview"):
		return
	var terrain := TerrainScript.new()
	terrain.simulation_node = simulation
	var pending := {}
	var requests := PackedInt32Array()
	for data in preview["terrain_preview"]["patches"]:
		var key := Vector2i(data["patch_x"], data["patch_z"])
		pending[key] = true
		requests.append_array(PackedInt32Array([key.x, key.y, 2000]))
	simulation.request_terrain_patch_payloads(requests)
	var deadline := Time.get_ticks_msec() + 20000
	while not pending.is_empty() and Time.get_ticks_msec() < deadline:
		var ready: Dictionary = simulation.poll_ready_terrain_patch_payloads(64)
		_retry_terrain_payloads(ready, pending)
		for data in ready.get("patches", []):
			var key := Vector2i(data["patch_x"], data["patch_z"])
			if not pending.has(key):
				continue
			pending.erase(key)
			var patch: Dictionary = terrain._new_terrain_patch_resources()
			patch["node"].position = Vector3(data["world_origin_x"] + data["world_size_x"] * 0.5, 0.0, data["world_origin_z"] + data["world_size_z"] * 0.5)
			patch["node"].mesh = terrain._terrain_patch_mesh_from_data(data, 1, 1)
			patch["retaining_wall_node"].mesh = terrain._retaining_wall_patch_mesh(data)
			patch["world_size_x"] = data["world_size_x"]
			patch["world_size_z"] = data["world_size_z"]
			patch["last_patch_data"] = data
			terrain.patches[key] = patch
			terrain.resident_patch_lookup[key] = true
		await process_frame
	_expect(pending.is_empty(), "terrain preview reference patches must load")
	if not pending.is_empty():
		terrain.free()
		return
	var originals := {}
	for key in terrain.patches:
		originals[key] = terrain.patches[key]["node"].mesh
	tool._clear_preview_visual()
	tool.terrain_node = terrain
	_expect(tool._draw_compiled_preview_surface(points, preview, preview), "paired road/terrain display must stage")
	_expect(tool._terrain_preview.request_id == preview["request_id"], "terrain and canonical roads must publish as one pose")
	_expect(tool._junction_preview._terrain_coupled, "paired display must use unlifted road buffers")
	_expect(not tool._info_label.text.contains("pending"), "complete paired display must expose plan readiness")
	tool._update_preview_measurement_label(points, preview, "provisional")
	_expect(tool._info_label.text.contains("pending"), "retained older paired poses must not claim readiness for new pointer inputs")
	_expect(tool._draw_compiled_preview_surface(points, preview, preview), "exact current pose must restore readiness")
	_expect(not tool._info_label.text.contains("pending"), "only the exact current paired pose is ready")
	var nodes: Array = []
	for key in originals:
		_expect(terrain.patches[key]["node"].mesh == null, "resident draw mesh must not overlap its replacement")
		var entry: Dictionary = tool._terrain_preview._patches[key]
		nodes.append(entry["node"])
		var patch_data: Dictionary = terrain.patches[key]["last_patch_data"]
		terrain._apply_patch_visibility_for_residency(key, terrain.patches[key], patch_data)
		_expect(terrain.patches[key]["node"].mesh == null, "residency updates must not restore old draw geometry")
	# A failed stage cannot partially replace the current terrain batch.
	var malformed: Array = preview["terrain_preview"]["patches"].duplicate(true)
	malformed[-1]["sample_width"] = 0
	_expect(tool._terrain_preview.stage(terrain, malformed, preview["surface_generation"]).is_empty(), "malformed terrain must fail staging")
	_expect(tool._terrain_preview.request_id == preview["request_id"], "failed staging preserves the previous complete pose")
	# Updating an affected patch invalidates both displays before mutating/recycling its resources.
	terrain.patch_render_will_change.emit(originals.keys()[0])
	_expect(tool._terrain_preview.request_id == 0 and tool._junction_preview.generation == -1, "terrain changes must retire both halves of the pose")
	for key in originals:
		_expect(terrain.patches[key]["node"].mesh == originals[key], "cancellation must restore exact original mesh identity")
	for node in nodes:
		_expect(not is_instance_valid(node), "invalidated preview resources must be freed")
	_expect(tool._draw_compiled_preview_surface(points, preview, preview), "cancelled pose must be reusable")
	tool._clear_preview_visual()
	for key in originals:
		_expect(terrain.patches[key]["node"].mesh == originals[key], "explicit clear restores terrain")
	_expect(tool._draw_compiled_preview_surface(points, preview, preview), "paired pose must stage before recycling a resident patch")
	terrain._remove_patch(originals.keys()[0])
	_expect(tool._terrain_preview.request_id == 0 and tool._junction_preview.generation == -1, "patch recycling must invalidate both displays")
	for key in terrain.patches:
		_expect(terrain.patches[key]["node"].mesh == originals[key], "patch recycling restores the remaining original meshes")
	_expect(tool._draw_compiled_preview_surface(points, preview, preview), "missing terrain residency keeps a provisional road preview")
	_expect(tool._terrain_preview.request_id == 0 and not tool._junction_preview._terrain_coupled, "an incomplete resident terrain batch must not publish canonical roads")
	_expect(tool._info_label.text.contains("terrain preview pending"), "missing terrain residency must remain visibly provisional")
	tool._clear_preview_visual()
	tool.terrain_node = null
	terrain.free()
	# A backend-certified empty terrain batch must not wait for nonexistent patch resources.
	var empty_batch := preview.duplicate(true)
	empty_batch["terrain_preview"]["patches"] = []
	_expect(tool._draw_compiled_preview_surface(points, empty_batch, empty_batch), "empty terrain batch must stage canonical roads")
	_expect(tool._terrain_preview.request_id == preview["request_id"] and tool._junction_preview._terrain_coupled, "empty batch still records complete paired publication")
	_expect(not tool._info_label.text.contains("pending"), "no terrain work must not remain pending")
	tool._clear_preview_visual()

func _retry_terrain_payloads(ready: Dictionary, pending: Dictionary) -> void:
	# Fresh-world payload work can be deferred by the native generation fence. Honor the same
	# explicit retry requests as the renderer rather than losing those requested patch keys.
	var retries: PackedInt64Array = ready.get("retry_requests", PackedInt64Array())
	var requests := PackedInt32Array()
	for index in range(0, retries.size(), 4):
		if pending.has(Vector2i(retries[index], retries[index + 1])):
			requests.append_array(PackedInt32Array([retries[index], retries[index + 1], retries[index + 2]]))
	if not requests.is_empty():
		simulation.request_terrain_patch_payloads(requests)

func _capture_terrain() -> Node3D:
	# Use the production clipped/stitched payload, not a solid plane under road cutouts.
	var result := Node3D.new()
	var renderer := TerrainScript.new()
	var requests := PackedInt32Array()
	var pending := {}
	for z in [1, 2]:
		for x in [1, 2]:
			requests.append_array(PackedInt32Array([x, z, 2000]))
			pending[Vector2i(x, z)] = true
	simulation.request_terrain_patch_payloads(requests)
	var deadline := Time.get_ticks_msec() + 20000
	var clipped := 0
	while not pending.is_empty() and Time.get_ticks_msec() < deadline:
		var ready: Dictionary = simulation.poll_ready_terrain_patch_payloads(16)
		_retry_terrain_payloads(ready, pending)
		for data in ready.get("patches", []):
			var key := Vector2i(data["patch_x"], data["patch_z"])
			if not pending.has(key):
				continue
			pending.erase(key)
			if data.get("terrain_requires_road_clipping", false):
				clipped += 1
				_expect(renderer._patch_uses_cdt_terrain_mesh(data), "road fixture terrain must use real clipped CDT geometry")
			var instance := MeshInstance3D.new()
			instance.mesh = renderer._terrain_patch_mesh_from_data(data, 1, 1)
			instance.position = Vector3(data["world_origin_x"] + data["world_size_x"] * 0.5, 0.0, data["world_origin_z"] + data["world_size_z"] * 0.5)
			var material := StandardMaterial3D.new()
			# The production terrain shader renders both sides of the CDT winding.
			material.cull_mode = BaseMaterial3D.CULL_DISABLED
			material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
			material.albedo_color = Color(0.22, 0.34, 0.18)
			instance.material_override = material
			result.add_child(instance)
		await process_frame
	var needs_clipping: bool = simulation.get_road_benchmark_state()["live_edges"] > 0
	_expect(pending.is_empty() and (clipped > 0 or not needs_clipping), "terrain capture must load all four central patches with current road ownership")
	renderer.free()
	return result

func _capture(mesh_root: Node3D, label: String, baseline: bool = false) -> void:
	# Optional real-renderer diagnostic; ordinary headless tests do not render images.
	var viewport := SubViewport.new()
	viewport.size = Vector2i(960, 720)
	viewport.own_world_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	root.add_child(viewport)
	for source in mesh_root.get_children():
		if source is MeshInstance3D and source.visible:
			var instance := MeshInstance3D.new()
			instance.mesh = source.mesh
			instance.transform = source.transform
			viewport.add_child(instance)
	viewport.add_child(await _capture_terrain())
	var environment := WorldEnvironment.new()
	environment.environment = Environment.new()
	environment.environment.background_mode = Environment.BG_COLOR
	environment.environment.background_color = Color.MAGENTA
	environment.environment.ambient_light_source = Environment.AMBIENT_SOURCE_COLOR
	environment.environment.ambient_light_color = Color.WHITE
	environment.environment.ambient_light_energy = 0.4
	viewport.add_child(environment)
	var light := DirectionalLight3D.new()
	light.rotation_degrees = Vector3(-60.0, -25.0, 0.0)
	viewport.add_child(light)
	var camera := Camera3D.new()
	viewport.add_child(camera)
	camera.position = Vector3(30.0, 65.0, 100.0)
	camera.projection = Camera3D.PROJECTION_ORTHOGONAL
	camera.size = 165.0
	camera.look_at(Vector3(0.0, 0.0, 8.0))
	camera.current = true
	for frame in range(5):
		await process_frame
	await RenderingServer.frame_post_draw
	var path := OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE") + "_" + label + ".png"
	var captured := viewport.get_texture().get_image()
	_expect(captured.save_png(path) == OK, "junction diagnostic image must save")
	var sky := {}
	# This fixed camera's central road area lies strictly inside the four loaded patches;
	# the sky beyond that finite terrain square is not a road hole.
	for y in range(160, 590):
		for x in range(160, 800):
			var color := captured.get_pixel(x, y)
			if color.r > 0.8 and color.b > 0.8 and color.g < 0.2:
				sky[Vector2i(x, y)] = true
	if baseline:
		_capture_baseline_sky = sky
		_capture_preview_sky.clear()
	elif label.ends_with("_cancelled"):
		_expect(sky == _capture_baseline_sky, "cancel must restore the exact source terrain/road coverage")
	elif label.ends_with("_committed"):
		# Compare exact locations against both independent references, not an allowed error
		# count. Existing/cold-commit subpixel raster seams are not preview-induced holes.
		var holes := 0
		for pixel in _capture_preview_sky:
			if not _capture_baseline_sky.has(pixel) and not sky.has(pixel):
				holes += 1
		_expect(holes == 0, "%s preview must not expose extra sky through road cutouts (%d pixels)" % [label, holes])
	else:
		_capture_preview_sky = sky
	viewport.free()
