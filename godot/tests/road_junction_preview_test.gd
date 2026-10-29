# SPDX-License-Identifier: GPL-2.0-only

## The road tool's synchronous preview against the native bridge (`ROAD-68-E`): one frame
## previews a stroke and shows its compiled chunks in place of the resident ones; a click commits
## exactly that preview, which stays shown until the committed chunks are installed. The
## preview's graded tiles (`TERRAIN-07-E`), staged at the levels of a fixed camera, equal the
## commit's, a fresh bake and the bake after save and load, bitwise.
extends SceneTree

# What a terrain renderer's graded tiles would draw of the road tool's preview: the tiles shown,
# by key, as the tool hands them over (show_preview, release_preview).
class GradedRecorder extends RefCounted:
	var shown := {}

	func show_preview(frame: Dictionary) -> float:
		var removed: PackedInt32Array = frame.get("graded_removed", PackedInt32Array())
		for index in range(0, removed.size(), 2):
			shown.erase(Vector2i(removed[index], removed[index + 1]))
		for tile: Dictionary in frame.get("graded_tiles", []):
			shown[Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))] = tile
		return 0.0

	func release_preview(first_epoch: int, last_epoch: int) -> void:
		for key: Vector2i in shown.keys():
			var token := int(shown[key]["token"])
			if token >= first_epoch and token <= last_epoch:
				shown.erase(key)

class TerrainStub extends Node3D:
	var graded_tiles := GradedRecorder.new()

# The camera the graded levels are staged for: 90 m above the junction, so its tiles take GL1
# and GL2 by distance.
const GRADED_CAMERA := Vector3(0.0, 90.0, 0.0)
const GRADED_FIELDS := ["level", "step", "samples_x", "samples_z", "graded", "seam_count", "edge_count", "wall_count", "texels", "rows", "walls"]

const RoadToolScript := preload("res://scripts/tools/road_tool.gd")
const WorldMaterialsScript := preload("res://scripts/renderers/world_materials.gd")
const TerrainScript := preload("res://scripts/renderers/terrain.gd")
const ZoningToolScript := preload("res://scripts/tools/zoning_tool.gd")
var _failures := 0
var simulation: SimulationNode
var _capture_baseline_sky := {}
var _capture_preview_sky := {}
# Fixtures whose commit staged graded tiles, and their tiles.
var _graded_fixtures := 0
var _graded_tiles := 0

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
	simulation = SimulationNode.new()
	root.add_child(simulation)
	await _test_zoning_buildability_feedback()
	for fixture in [
		{"name": "isolated", "end_x": 48.0, "end_z": 0.0, "forward": 1, "backward": 1, "isolated": true},
		{"name": "isolated_bridge", "end_x": 48.0, "end_z": 0.0, "forward": 1, "backward": 1, "isolated": true, "height": 10.0},
		{"name": "isolated_sloped", "end_x": 48.0, "end_z": 0.0, "forward": 1, "backward": 1, "isolated": true, "sloped": true, "neighbor": true},
		{"name": "neighbor_cross", "end_z": 0.0, "forward": 1, "backward": 1, "neighbor_cross": true},
		{"name": "sloped_neighbor_cross", "end_z": 0.0, "forward": 1, "backward": 1, "neighbor_cross": true, "sloped": true},
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
	_expect(_graded_fixtures >= 10, "most fixtures stage graded tiles: %d" % _graded_fixtures)
	print("graded preview tiles bitwise: %d fixtures, %d tiles" % [_graded_fixtures, _graded_tiles])
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
	var rejected := simulation.get_zoning_parcel_preview(0.0, 16.0, 65535, 2, 2)
	_expect(not rejected.is_empty() and not rejected.get("valid", true), "unknown profile must remain visible as invalid")
	_expect(str(rejected.get("reason", "")).contains("unknown zoning profile"), "unknown profile retains its rejection reason")
	_expect(not simulation.apply_zoning_parcel_at(0.0, 16.0, 65535, 2, 2), "unknown zoning profile must not commit")
	_expect(not simulation.has_zoning_parcel_at(0.0, 16.0), "failed zoning must not insert a parcel")
	_expect(simulation.get_zoning_site_dependencies() == before, "feasibility must not change road, terrain or zoning")
	var drag := simulation.get_zoning_parcel_drag_preview_packed(-60.0, 16.0, 60.0, 16.0, 65535, 2, 2, 0.0)
	_expect(int(drag.get("parcel_count", 0)) > 0 and int(drag.get("valid_count", -1)) == 0, "drag retains rejected geometries with no valid lots")
	var colors: PackedColorArray = drag.get("colors", PackedColorArray())
	_expect(colors.size() == int(drag.get("parcel_count", 0)), "every drag parcel needs its feasibility color")
	for color in colors:
		_expect(color.r > color.g, "rejected zoning must be red")
	var tool := ZoningToolScript.new()
	_expect(tool._build_packed_parcels_mesh(drag, true) != null, "zoning tool must render rejected parcels")
	tool.free()
	_expect(simulation.get_zoning_parcel_preview(0.0, 16.0, 3, 2, 2).get("valid", false), "high-density preview remains valid without assets")
	_expect(simulation.apply_zoning_parcel_at(0.0, 16.0, 3, 2, 2), "single high-density parcel commits without assets")
	_expect(simulation.get_zoning_parcel_profile_runtime_id_at(0.0, 16.0) == 3, "single placement retains its density")
	_expect(simulation.apply_zoning_parcel_at(0.0, 16.0, 2, 2, 2), "rezoning to medium density works without assets")
	_expect(simulation.get_zoning_parcel_profile_runtime_id_at(0.0, 16.0) == 2, "rezoning retains the new density")
	var supported_drag := simulation.get_zoning_parcel_drag_preview_packed(-60.0, -16.0, 60.0, -16.0, 2, 2, 2, 0.0)
	_expect(supported_drag.get("valid_count", 0) > 0, "medium-density drag previews without assets")
	_expect(simulation.apply_zoning_parcel_drag(-60.0, -16.0, 60.0, -16.0, 2, 2, 2, 0.0), "medium-density drag commits without assets")
	_expect(simulation.get_zoning_parcel_preview(0.0, 16.0, 0, 2, 2).get("valid", false), "free parcels do not require a building asset")
	_expect(simulation.apply_zoning_parcel_at(0.0, 16.0, 0, 2, 2), "free parcel placement must remain possible")
	_expect(simulation.get_zoning_site_dependencies() != before, "zoning edits invalidate retained cursor previews")

func _fixture(fixture: Dictionary) -> void:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "junction fixture world must load")
	simulation.set_simulation_speed(0.0)
	if fixture.get("sloped", false):
		simulation.slope_terrain(Vector2.ZERO, 256.0, Vector2(-128.0, 0.0), 0.0, Vector2(128.0, 0.0), 6.4, 1.0)
	var endpoint_join: bool = fixture.get("endpoint_join", false)
	var isolated: bool = fixture.get("isolated", false)
	if not isolated:
		if not await _commit(PackedVector3Array([_ground_point(-110.0 if fixture.get("neighbor_cross", false) else -60.0, 0.0), _ground_point(0.0 if endpoint_join else 60.0, 0.0)])):
			return
	if fixture.get("neighbor_cross", false):
		if not await _commit(PackedVector3Array([_ground_point(-65.0, -60.0), _ground_point(-65.0, 60.0)])):
			return
	# Unrelated road in the same render chunk must survive the temporary replacement.
	if not isolated or fixture.get("neighbor", false):
		if not await _commit(PackedVector3Array([_ground_point(24.0, 72.0), _ground_point(60.0, 72.0)])):
			return
	if fixture["name"] == "double_t":
		if not await _commit(PackedVector3Array([Vector3(-32.0, 0.0, -48.0), Vector3(-32.0, 0.0, 0.0)])):
			return
	simulation.set_road_preview_terrain_view(GRADED_CAMERA, PackedFloat64Array([64.0, 128.0, 2000.0]), PackedFloat64Array([5000.0, 12000.0, INF]))
	var tool := RoadToolScript.new()
	tool.name = "RoadTool"
	tool.simulation_node = simulation
	var terrain_stub := TerrainStub.new()
	tool.terrain_node = terrain_stub
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
	tool.active = true
	var generation := simulation.get_network_render_generation()
	_expect(tool.update_main_mesh(generation) == generation, "original chunks must hydrate before preview")
	# Drive the real process path without node overlays.
	tool._node_visuals_dirty = false
	tool._node_visuals_visible = true
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
	# One frame samples the stroke, previews it synchronously and shows its chunks.
	tool._begin_stroke(points[0], 0, Vector3.ZERO)
	tool.set_scripted_pointer(true, points[-1])
	tool._process(1.0 / 60.0)
	var preview: Dictionary = tool._preview
	_expect(preview.get("is_valid", false), "%s synchronous preview must validate: %s" % [fixture["name"], preview.get("invalid_reason", "missing")])
	if not preview.get("is_valid", false):
		tool.free()
		return
	_expect(preview.get("has_road_chunks", false), "%s preview must return its compiled chunks" % fixture["name"])
	_expect(tool._preview_display.sequence == int(preview.get("sequence", -1)), "the frame that sampled the stroke must show its chunks")
	_expect(not tool._info_label.text.contains("Checking"), "a synchronous preview is never pending")
	_expect(tool.blueprint_mesh.mesh == null, "the preview shows chunks, never a ribbon")
	if not isolated and not fixture.get("sloped", false):
		_expect_existing_road_height(preview)
	var changed := {}
	for chunk in preview["road_chunks"]:
		changed[Vector2i(chunk["chunk_x"], chunk["chunk_z"])] = true
	var hidden_expected := 0
	for key in originals:
		if changed.has(key):
			hidden_expected += 1
	_expect(tool._preview_display._hidden.size() == hidden_expected, "exactly the changed resident chunks are replaced")
	for original in tool._preview_display._hidden:
		_expect(not original.visible, "old curbs and markings must not show through the preview")
	for key in originals:
		if not changed.has(key):
			_expect(originals[key].visible, "unchanged chunks stay resident and visible")
	for instance in tool._preview_display._instances:
		_expect(instance.position.y == 0.0, "preview must not lift roads away from terrain cutouts")
	var instances: Array = tool._preview_display._instances.duplicate()
	var sequence: int = tool._preview_sequence
	tool._queue_preview_update()
	tool._process(1.0 / 60.0)
	_expect(tool._preview_sequence == sequence and instances == tool._preview_display._instances, "an unchanged stroke must not preview or upload again")
	var malformed := preview.duplicate(true)
	for chunk in malformed["road_chunks"]:
		if not chunk.get("removed", false):
			chunk["road_normals"] = PackedVector3Array()
			break
	_expect(not tool._preview_display.show(tool, malformed, sequence + 1), "incomplete mesh batches must fail atomically")
	_expect(instances == tool._preview_display._instances, "failed staging must preserve the complete previous preview")
	# A scripted pointer moves without a motion event; queue the update as input would.
	tool.set_scripted_pointer(true, points[-1] + Vector3(1.0, 0.0, 0.0))
	tool._queue_preview_update()
	tool._process(1.0 / 60.0)
	_expect(tool._preview_sequence == sequence + 1 and tool._preview_display.sequence == sequence + 1, "a moved pointer is previewed and shown in its own frame")
	var after := simulation.get_road_benchmark_state()
	for field in ["generation", "live_edges", "edge_slots", "nodes", "lanes", "agents", "buildings"]:
		_expect(before[field] == after[field], "preview must not mutate authoritative " + field)
	tool.set_scripted_pointer(true, points[-1])
	tool._queue_preview_update()
	tool._process(1.0 / 60.0)
	if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty():
		await _capture(tool.road_mesh_root, fixture["name"])
	tool.cancel_road()
	for original in originals.values():
		_expect(original.visible, "cancel must restore original road instances")
	_expect(tool._preview_display._instances.is_empty(), "cancel must remove the preview chunks")
	if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty():
		await _capture(tool.road_mesh_root, fixture["name"] + "_cancelled")
	await _commit_previewed(tool, points, preview, not isolated, fixture["name"])
	tool.free()
	terrain_stub.free()
	await process_frame

# Clicks the previewed stroke: the commit installs the preview's pieces, and the preview stays
# shown until the committed generation's chunks are installed, which replace it in one call.
# `crossed`: the stroke's closure holds crossings, whose paint and signs the preview must carry.
func _commit_previewed(tool: Node3D, points: PackedVector3Array, preview: Dictionary, crossed: bool, label: String) -> void:
	tool._begin_stroke(points[0], 0, Vector3.ZERO)
	tool.set_scripted_pointer(true, points[-1])
	tool._process(1.0 / 60.0)
	# The frames after the pose stage the rest of its graded tiles (T27 (d)); the click then
	# commits what the preview shows.
	var catch_up := 0
	while tool._graded_pending > 0 and catch_up < 64:
		tool._process(1.0 / 60.0)
		catch_up += 1
	_expect(tool._graded_pending == 0, "%s: the preview's graded tiles converge" % label)
	var shown: Dictionary = tool.terrain_node.graded_tiles.shown.duplicate()
	# The first poll starts the graded edit journal, which carries the commit's tiles.
	simulation.poll_graded_tile_results(0)
	var generation := simulation.get_network_render_generation()
	_expect(tool._commit_segment(points[-1]), "the click must queue the previewed stroke")
	var epoch: int = tool._graded_commit_epoch
	var held = tool._preview_display
	var deadline := Time.get_ticks_msec() + 20000
	while (tool._commit_request_id > 0 or simulation.get_network_render_generation() == generation) and Time.get_ticks_msec() < deadline:
		tool._process(1.0 / 60.0)
		await process_frame
	_expect(tool._commit_request_id == 0 and simulation.get_network_render_generation() > generation, "the commit must finish")
	_expect(simulation.get_road_benchmark_state()["command"].get("preview_plan_reused", false), "a current preview is the commit's plan")
	_expect(tool._swap_display == held and not held._instances.is_empty(), "the preview stays shown until the committed chunks are installed")
	# The next stroke previews on the committed context before its chunks are installed.
	tool.cancel_road()
	tool._begin_stroke(_ground_point(-48.0, 40.0), 0, Vector3.ZERO)
	tool.set_scripted_pointer(true, _ground_point(48.0, 40.0))
	tool._process(1.0 / 60.0)
	var next_keys: Array[Vector2i] = tool._preview_display._keys.duplicate()
	generation = simulation.get_network_render_generation()
	_expect(tool.update_main_mesh(generation) == generation, "committed chunks must replace the source generation")
	_expect(tool._swap_display == null and held._instances.is_empty(), "installing the committed chunks ends the preview in the same call")
	for key in tool._road_chunk_instances:
		_expect(tool._road_chunk_instances[key].visible != (key in next_keys), "the next stroke's preview hides exactly the installed chunks it replaces")
	tool.cancel_road()
	for instance in tool._road_chunk_instances.values():
		_expect(instance.visible, "the committed generation must be visible")
	# The preview chunks are the committed chunks, crosswalk paint, lights and signs included:
	# both are built from the same pieces (`ROAD-68-F`, open question 26).
	var committed := {}
	for chunk in simulation.get_road_mesh_data(true)["chunks"]:
		committed[Vector2i(chunk["chunk_x"], chunk["chunk_z"])] = chunk
	var painted := false
	var signed := false
	for chunk in preview["road_chunks"]:
		var key := Vector2i(chunk["chunk_x"], chunk["chunk_z"])
		_expect(committed.has(key) != chunk.get("removed", false), "a previewed chunk exists after the commit exactly when it was not removed")
		if not committed.has(key) or chunk.get("removed", false):
			continue
		var names: Array[String] = ["road_crossings", "street_lights", "road_signs"]
		for layer in ["skirt", "curb", "raised_step", "sidewalk", "road", "marking", "concrete"]:
			for field in ["vertices", "normals", "uvs", "colors"]:
				names.append("%s_%s" % [layer, field])
		for name in names:
			_expect(chunk.get(name) == committed[key].get(name), "preview chunk %s %s equals the committed one" % [key, name])
		var crossings: PackedFloat32Array = chunk.get("road_crossings", PackedFloat32Array())
		for index in range(2, crossings.size(), 4):
			painted = painted or crossings[index] > 0.0
		signed = signed or not chunk.get("road_signs", PackedFloat32Array()).is_empty()
	if crossed:
		_expect(painted and signed, "a crossing stroke's preview carries crosswalk paint (%s) and signs (%s)" % [painted, signed])
	if not OS.get_environment("METRUM_JUNCTION_PREVIEW_CAPTURE").is_empty():
		await _capture(tool.road_mesh_root, preview.get("name", "fixture") + "_committed")
	await _check_graded_tiles(shown, epoch, label)

# TERRAIN-07-E: the tiles the converged preview showed are the commit's staged tiles, a fresh bake
# of them and their bake after save and load, bitwise, at the same levels.
func _check_graded_tiles(shown: Dictionary, epoch: int, label: String) -> void:
	var levels := {}
	for tile: Dictionary in shown.values():
		levels[int(tile["level"])] = true
	_expect(not levels.has(0), "%s: the camera's distance rule stages no GL0 tile: %s" % [label, levels.keys()])
	var staged := await _graded_staged(epoch, label)
	_expect(_same_tiles(shown, staged), "%s: the preview's graded tiles are the commit's (%d, %d)" % [label, shown.size(), staged.size()])
	if staged.is_empty():
		return
	_graded_fixtures += 1
	_graded_tiles += staged.size()
	var fresh := await _graded_bakes(staged, label)
	_expect(_same_tiles(staged, fresh), "%s: the committed graded tiles are a fresh bake" % label)
	var path := OS.get_user_data_dir().path_join("graded_preview_%s.sav" % label)
	_expect(simulation.save_game(path), "%s: the fixture saves" % label)
	_expect(simulation.load_game(path), "%s: the fixture loads" % label)
	simulation.set_simulation_speed(0.0)
	DirAccess.remove_absolute(path)
	var loaded := await _graded_bakes(staged, label)
	_expect(_same_tiles(staged, loaded), "%s: the committed graded tiles bake the same after save and load" % label)

# The staged tiles of the commit that adopted preview epoch `epoch`, from the graded edit journal.
func _graded_staged(epoch: int, label: String) -> Dictionary:
	var deadline := Time.get_ticks_msec() + 20000
	while Time.get_ticks_msec() < deadline:
		var result: Dictionary = simulation.poll_graded_tile_results(0)
		for edit: Dictionary in result.get("edits", []):
			if int(edit.get("preview_epoch", -1)) != epoch:
				continue
			var staged := {}
			for tile: Dictionary in edit["staged"]:
				staged[Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))] = tile
			return staged
		await process_frame
	_expect(false, "%s: the commit's edit carries its staged tiles" % label)
	return {}

# Fresh bakes of `tiles` at their levels, through the residency requests.
func _graded_bakes(tiles: Dictionary, label: String) -> Dictionary:
	var baked := {}
	var wanted := tiles.keys()
	var deadline := Time.get_ticks_msec() + 20000
	while baked.size() < tiles.size() and Time.get_ticks_msec() < deadline:
		if not wanted.is_empty():
			var flat := PackedInt64Array()
			for key: Vector2i in wanted:
				flat.append_array(PackedInt64Array([key.x, key.y, int(tiles[key]["level"]), int(tiles[key]["step"]), 1]))
			simulation.request_graded_tiles(flat)
			wanted = []
		var result: Dictionary = simulation.poll_graded_tile_results(64)
		for tile: Dictionary in result.get("tiles", []):
			baked[Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))] = tile
		for failure: Dictionary in result.get("failed", []):
			var keys: PackedInt64Array = failure["keys"]
			_expect(bool(failure["retry"]), "%s: a fresh bake is refused: %s" % [label, failure])
			wanted.append(Vector2i(keys[0], keys[1]))
		await process_frame
	_expect(baked.size() == tiles.size(), "%s: every fresh bake arrives" % label)
	return baked

static func _same_tiles(a: Dictionary, b: Dictionary) -> bool:
	if a.size() != b.size():
		return false
	for key: Vector2i in a.keys():
		if not b.has(key):
			return false
		for field: String in GRADED_FIELDS:
			if a[key][field] != b[key][field]:
				return false
	return true

func _ground_point(x: float, z: float) -> Vector3:
	return Vector3(x, simulation.get_world_surface_height(Vector2(x, z)), z)

func _expect_existing_road_height(preview: Dictionary) -> void:
	# The previewed chunks keep the existing road at its committed, solved height.
	var checked := 0
	var raised := 0
	for chunk in preview["road_chunks"]:
		var origin := Vector3(preview["chunk_origin_x_m"] + (chunk["chunk_x"] + 0.5) * preview["chunk_span_m"], 0.0, preview["chunk_origin_z_m"] + (chunk["chunk_z"] + 0.5) * preview["chunk_span_m"])
		for local in chunk.get("road_vertices", PackedVector3Array()):
			var point: Vector3 = local + origin
			if point.x < -32.0 and absf(point.z) < 3.51:
				if absf(point.y) >= 0.00001:
					raised += 1
				checked += 1
	_expect(checked > 0, "fixture must exercise an existing neighbor span")
	_expect(raised == 0, "existing road must not be lifted above its terrain cutout (%d vertices)" % raised)

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
	# Use the production plain terrain payload beneath the graded preview tiles.
	var result := Node3D.new()
	var renderer := TerrainScript.new()
	var requests := PackedInt32Array()
	var pending := {}
	for z in [1, 2]:
		for x in [1, 2]:
			requests.append_array(PackedInt32Array([x, z, 0]))
			pending[Vector2i(x, z)] = true
	simulation.request_terrain_patch_payloads(requests)
	var deadline := Time.get_ticks_msec() + 20000
	while not pending.is_empty() and Time.get_ticks_msec() < deadline:
		var ready: Dictionary = simulation.poll_ready_terrain_patch_payloads(16)
		_retry_terrain_payloads(ready, pending)
		for data in ready.get("patches", []):
			var key := Vector2i(data["patch_x"], data["patch_z"])
			if not pending.has(key):
				continue
			pending.erase(key)
			var instance := MeshInstance3D.new()
			instance.mesh = renderer._terrain_patch_mesh_from_data(data, 1, 1)
			instance.position = Vector3(data["world_origin_x"] + data["world_size_x"] * 0.5, 0.0, data["world_origin_z"] + data["world_size_z"] * 0.5)
			var material := StandardMaterial3D.new()
			# Both sides draw, so the capture does not depend on the patch mesh's winding.
			material.cull_mode = BaseMaterial3D.CULL_DISABLED
			material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
			material.albedo_color = Color(0.22, 0.34, 0.18)
			instance.material_override = material
			result.add_child(instance)
		await process_frame
	# Elevated bridges can have no ground cutout.
	_expect(pending.is_empty(), "terrain capture must load all four central patches with current road ownership")
	renderer.free()
	return result

func _capture(mesh_root: Node3D, label: String, baseline: bool = false, staged_terrain: Node3D = null) -> void:
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
	if staged_terrain == null:
		viewport.add_child(await _capture_terrain())
	else:
		# Copy the actual substituted draw meshes, including unchanged resident patches.
		var material := StandardMaterial3D.new()
		material.cull_mode = BaseMaterial3D.CULL_DISABLED
		material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		material.albedo_color = Color(0.22, 0.34, 0.18)
		for patch in staged_terrain.patches.values():
			var original: MeshInstance3D = patch["node"]
			var sources: Array = [original]
			sources.append_array(original.get_children())
			for source in sources:
				if source is MeshInstance3D and source.mesh != null:
					var instance := MeshInstance3D.new()
					instance.mesh = source.mesh
					instance.position = original.position
					instance.material_override = material
					viewport.add_child(instance)
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
		# Road-only previews can expose old cutouts while the terrain stays committed. Record
		# their exact extent for review; final terrain geometry is checked against cold output
		# in the native suite, and cancellation still requires exact original coverage above.
		var holes := 0
		for pixel in _capture_preview_sky:
			if not _capture_baseline_sky.has(pixel) and not sky.has(pixel):
				holes += 1
		print("ROAD_PREVIEW_COVERAGE " + JSON.stringify({"case": label, "preview_only_sky_pixels": holes, "committed_sky_pixels": sky.size(), "baseline_sky_pixels": _capture_baseline_sky.size()}))
	else:
		_capture_preview_sky = sky
	viewport.free()
