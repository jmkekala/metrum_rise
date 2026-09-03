## Headless contract test for atomic road-chunk hydration and replacement.
extends SceneTree

const NetworkRendererScript := preload("res://scripts/renderers/network_renderer.gd")
const TerrainRendererScript := preload("res://scripts/renderers/terrain.gd")
const LAYERS := ["earthwork", "curb", "raised_step", "sidewalk", "road", "marking", "concrete"]
const CHUNK_SPAN_M := 510.0
const CHUNK_ORIGIN_X_M := 125.5
const CHUNK_ORIGIN_Z_M := -240.25

class MockSimulation:
	extends Node

	var response: Dictionary = {}
	var generation: int = 0
	var dirty: bool = false
	var dirty_terrain_patches: PackedInt32Array = PackedInt32Array()
	var dirty_terrain_states: PackedInt64Array = PackedInt64Array()
	var engineered_terrain_patches: PackedInt32Array = PackedInt32Array()
	var terrain_dirty: bool = false
	var full_snapshot_requests: Array[bool] = []
	var acknowledged_generations: Array[int] = []
	var acknowledged_terrain_states: Array[PackedInt64Array] = []
	var reject_acknowledgement: bool = false
	var network_edit_on_border_query: bool = false

	func is_network_dirty() -> bool:
		return dirty

	func get_road_mesh_data(full_snapshot: bool) -> Dictionary:
		full_snapshot_requests.append(full_snapshot)
		return response

	func get_network_render_generation() -> int:
		return generation

	func get_dirty_terrain_patches() -> PackedInt32Array:
		return dirty_terrain_patches

	func get_dirty_terrain_patch_payload_states() -> PackedInt64Array:
		return dirty_terrain_states

	func get_engineered_terrain_patches() -> PackedInt32Array:
		return engineered_terrain_patches

	func get_road_tool_surface_generation() -> int:
		return generation

	func is_terrain_dirty() -> bool:
		return terrain_dirty

	func poll_ready_terrain_patch_payloads(_budget: int) -> Dictionary:
		return {"patches": [], "retry_requests": PackedInt64Array()}

	func request_terrain_patch_payloads(_requests: PackedInt32Array) -> Dictionary:
		return {"tracked_requests": PackedInt64Array()}

	func acknowledge_terrain_patches(states: PackedInt64Array) -> bool:
		acknowledged_terrain_states.append(states.duplicate())
		if reject_acknowledgement or states != dirty_terrain_states:
			return false
		terrain_dirty = false
		dirty_terrain_states = PackedInt64Array()
		dirty_terrain_patches = PackedInt32Array()
		return true

	func get_terrain_border_loop() -> PackedVector3Array:
		if network_edit_on_border_query:
			network_edit_on_border_query = false
			generation += 1
			dirty = true
		return PackedVector3Array()

	func acknowledge_network_render(acknowledged_generation: int) -> bool:
		acknowledged_generations.append(acknowledged_generation)
		if reject_acknowledgement or acknowledged_generation != generation:
			return false
		dirty = false
		return true

class MockTerrain:
	extends Node3D

	var prepare_ready: bool = false
	var commit_ready: bool = true
	var prepare_count: int = 0
	var commit_count: int = 0

	func prepare_terrain_visual_update() -> Dictionary:
		prepare_count += 1
		if not prepare_ready:
			return {}
		return {
			"dirty_states": PackedInt64Array([0, 0, 1]),
			"dirty_keys": [Vector2i.ZERO],
		}

	func commit_prepared_terrain_visual_update(_prepared: Dictionary) -> bool:
		commit_count += 1
		return commit_ready

class TestTerrainRenderer:
	extends TerrainRendererScript
	var fail_next_stage: bool = false

	func _ready() -> void:
		pass

	func _mesh_lod_step_for_patch(
		_key: Vector2i,
		_center_x: float,
		_center_z: float
	) -> int:
		return 1

	func _desired_patch_bounds() -> Dictionary:
		return {
			"min_x": 0,
			"max_x": patch_cols - 1,
			"min_z": 0,
			"max_z": patch_rows - 1,
		}

	func _current_camera_patch_key() -> Vector2i:
		return Vector2i.ZERO

	func _stage_terrain_patch_update(
		key: Vector2i,
		patch_data: Dictionary,
		expected_generation: int,
		expected_render_step_mm: int
	) -> Dictionary:
		if fail_next_stage:
			fail_next_stage = false
			return {}
		return super._stage_terrain_patch_update(
			key,
			patch_data,
			expected_generation,
			expected_render_step_mm
		)

class MockWater:
	extends Node

	var refresh_count: int = 0

	func refresh_road_clipped_patches(_keys: PackedInt32Array) -> void:
		refresh_count += 1

class MockZoningOverlay:
	extends Node

	var dirty_count: int = 0

	func mark_no_build_dirty() -> void:
		dirty_count += 1

class TestRoadTool:
	extends NetworkTool

	var topology_dirty_count: int = 0
	var nodes_dirty_count: int = 0
	var border_check_count: int = 0
	var advance_generation_after_stage: bool = false

	func update_main_mesh(expected_generation: int = -1, stage_only: bool = false) -> int:
		var result := super.update_main_mesh(expected_generation, stage_only)
		if stage_only and result == expected_generation and advance_generation_after_stage:
			simulation_node.generation += 1
		return result

	func mark_network_nodes_dirty() -> void:
		nodes_dirty_count += 1

	func mark_network_topology_dirty() -> void:
		topology_dirty_count += 1
		mark_network_nodes_dirty()

	func drain_pending_border_checks() -> void:
		border_check_count += 1

var _failures: int = 0

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	_test_real_terrain_atomic_transaction()
	var host := Node3D.new()
	root.add_child(host)
	var simulation := MockSimulation.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	var terrain := MockTerrain.new()
	terrain.name = "Terrain"
	host.add_child(terrain)
	var water := MockWater.new()
	water.name = "Water"
	host.add_child(water)
	var zoning_overlay := MockZoningOverlay.new()
	zoning_overlay.name = "ZoningOverlay"
	host.add_child(zoning_overlay)
	var road_tool := TestRoadTool.new()
	road_tool.name = "RoadTool"
	host.add_child(road_tool)
	var network_renderer: Node = NetworkRendererScript.new()
	network_renderer.name = "NetworkRenderer"
	host.add_child(network_renderer)

	simulation.generation = 1
	simulation.response = _batch(1, true, CHUNK_SPAN_M, [])
	_expect(road_tool.needs_main_mesh_hydration(), "new renderer must require hydration")
	network_renderer._process(0.0)
	_expect(simulation.full_snapshot_requests == [true], "clean hydration must request a full snapshot")
	_expect(not road_tool.needs_main_mesh_hydration(), "empty hydration must record generation")
	_expect(road_tool._road_mesh_generation == 1, "empty hydration must record the exact generation")
	_expect(road_tool.topology_dirty_count == 1, "clean hydration must refresh topology visuals")

	simulation.generation = 2
	simulation.response = _batch(2, true, CHUNK_SPAN_M, [
		_triangle_chunk(0, 0),
		_triangle_chunk(3, -2),
	], CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M)
	_expect(road_tool.update_main_mesh(2) == 2, "valid full snapshot must commit")
	_expect(road_tool._road_chunk_instances.size() == 2, "full snapshot must add both chunks")
	var sentinel_key := Vector2i(0, 0)
	var changed_key := Vector2i(3, -2)
	var sentinel: MeshInstance3D = road_tool._road_chunk_instances[sentinel_key]
	var previous_changed: MeshInstance3D = road_tool._road_chunk_instances[changed_key]
	_expect(
		sentinel.position == Vector3(CHUNK_ORIGIN_X_M, 0.0, CHUNK_ORIGIN_Z_M),
		"chunk zero must be placed at the published origin"
	)
	_expect(
		previous_changed.position == Vector3(
			CHUNK_ORIGIN_X_M + 3.0 * CHUNK_SPAN_M,
			0.0,
			CHUNK_ORIGIN_Z_M - 2.0 * CHUNK_SPAN_M
		),
		"nonzero chunk must be placed relative to the published origin"
	)
	_expect(road_tool._road_chunk_origin_x_m == CHUNK_ORIGIN_X_M, "full snapshot must record x origin")
	_expect(road_tool._road_chunk_origin_z_m == CHUNK_ORIGIN_Z_M, "full snapshot must record z origin")

	simulation.generation = 3
	simulation.response = _batch(
		3,
		false,
		CHUNK_SPAN_M,
		[_triangle_chunk(3, -2, 2.0)],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.update_main_mesh(3) == 3, "incremental upsert must commit")
	_expect(road_tool._road_chunk_instances.size() == 2, "upsert must retain untouched chunks")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "upsert must retain sentinel identity")
	_expect(road_tool._road_chunk_instances[changed_key] != previous_changed, "upsert must replace changed identity")

	simulation.generation = 4
	simulation.response = _batch(
		4,
		false,
		CHUNK_SPAN_M,
		[_tombstone(3, -2)],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.update_main_mesh(4) == 4, "tombstone batch must commit")
	_expect(road_tool._road_chunk_instances.size() == 1, "tombstone must remove only its chunk")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "tombstone must retain sentinel identity")
	await process_frame
	var orphan_baseline := int(Performance.get_monitor(Performance.OBJECT_ORPHAN_NODE_COUNT))

	simulation.generation = 6
	simulation.response = _batch(
		5,
		false,
		CHUNK_SPAN_M,
		[_triangle_chunk(1, 1)],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.update_main_mesh(5) == -1, "stale staged batch must be rejected")
	_expect(road_tool._road_chunk_instances.size() == 1, "stale batch must retain resident set")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "stale batch must retain sentinel identity")
	_expect(road_tool._road_mesh_generation == 4, "stale batch must retain generation")
	_expect(road_tool._road_chunk_span_m == CHUNK_SPAN_M, "stale batch must retain span")
	_expect(road_tool._road_chunk_origin_x_m == CHUNK_ORIGIN_X_M, "stale batch must retain x origin")
	_expect(road_tool._road_chunk_origin_z_m == CHUNK_ORIGIN_Z_M, "stale batch must retain z origin")
	await process_frame
	_expect(
		int(Performance.get_monitor(Performance.OBJECT_ORPHAN_NODE_COUNT)) == orphan_baseline,
		"stale staged nodes must be freed"
	)

	simulation.generation = 6
	var invalid_chunk := _triangle_chunk(1, 1)
	invalid_chunk["road_vertices"] = PackedVector3Array([
		Vector3(NAN, 0.0, 0.0),
		Vector3(1.0, 0.0, 0.0),
		Vector3(0.0, 0.0, 1.0),
	])
	simulation.response = _batch(
		6,
		false,
		CHUNK_SPAN_M,
		[invalid_chunk],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.update_main_mesh(6) == -1, "non-finite geometry must be rejected")
	_expect(road_tool._road_chunk_instances.size() == 1, "invalid batch must retain resident set")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "invalid batch must retain sentinel identity")
	_expect(road_tool._road_mesh_generation == 4, "invalid batch must retain generation")
	_expect(road_tool._road_chunk_span_m == CHUNK_SPAN_M, "invalid batch must retain span")
	_expect(road_tool._road_chunk_origin_x_m == CHUNK_ORIGIN_X_M, "invalid batch must retain x origin")
	_expect(road_tool._road_chunk_origin_z_m == CHUNK_ORIGIN_Z_M, "invalid batch must retain z origin")

	simulation.generation = 7
	simulation.response = _batch(
		7,
		false,
		512.0,
		[],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.update_main_mesh(7) == -1, "incremental span change must be rejected")
	_expect(road_tool._road_chunk_instances.size() == 1, "span rejection must retain resident set")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "span rejection must retain sentinel identity")
	_expect(road_tool._road_mesh_generation == 4, "span rejection must retain generation")
	_expect(road_tool._road_chunk_span_m == CHUNK_SPAN_M, "span rejection must retain committed span")
	_expect(road_tool._road_chunk_origin_x_m == CHUNK_ORIGIN_X_M, "span rejection must retain x origin")
	_expect(road_tool._road_chunk_origin_z_m == CHUNK_ORIGIN_Z_M, "span rejection must retain z origin")

	simulation.generation = 8
	simulation.response = _batch(
		8,
		false,
		CHUNK_SPAN_M,
		[_triangle_chunk(5, 5)],
		CHUNK_ORIGIN_X_M + 1.0,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.update_main_mesh(8) == -1, "incremental origin change must be rejected")
	_expect(road_tool._road_chunk_instances.size() == 1, "origin rejection must retain resident set")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "origin rejection must retain sentinel identity")
	_expect(road_tool._road_mesh_generation == 4, "origin rejection must retain generation")
	_expect(road_tool._road_chunk_span_m == CHUNK_SPAN_M, "origin rejection must retain span")
	_expect(road_tool._road_chunk_origin_x_m == CHUNK_ORIGIN_X_M, "origin rejection must retain x origin")
	_expect(road_tool._road_chunk_origin_z_m == CHUNK_ORIGIN_Z_M, "origin rejection must retain z origin")

	var atomic_previous_instance: MeshInstance3D = road_tool._road_chunk_instances[sentinel_key]
	var atomic_previous_generation := road_tool._road_mesh_generation
	var atomic_previous_nodes_dirty_count := road_tool.nodes_dirty_count
	simulation.generation = 9
	simulation.response = _batch(
		9,
		false,
		CHUNK_SPAN_M,
		[_triangle_chunk(0, 0, 9.0)],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	simulation.dirty = true
	simulation.dirty_terrain_patches = PackedInt32Array([0, 0])
	terrain.prepare_ready = false
	network_renderer._process(0.0)
	_expect(
		road_tool._road_mesh_generation == atomic_previous_generation,
		"an unprepared terrain batch must retain the previous road generation"
	)
	_expect(
		road_tool._road_chunk_instances[sentinel_key] == atomic_previous_instance,
		"an unprepared terrain batch must retain previous road instances"
	)
	_expect(
		simulation.acknowledged_generations.is_empty(),
		"an unprepared terrain batch must not acknowledge the network"
	)
	_expect(
		road_tool.nodes_dirty_count == atomic_previous_nodes_dirty_count,
		"terrain preparation must not invalidate node visuals before publication"
	)

	terrain.prepare_ready = true
	terrain.commit_ready = false
	network_renderer._process(0.0)
	_expect(
		road_tool._road_mesh_generation == atomic_previous_generation,
		"a rejected terrain commit must retain the previous road generation"
	)
	_expect(
		road_tool._road_chunk_instances[sentinel_key] == atomic_previous_instance,
		"a rejected terrain commit must retain previous road instances"
	)
	_expect(
		road_tool._staged_road_mesh_update.is_empty(),
		"a rejected terrain commit must discard staged road instances"
	)
	_expect(
		simulation.acknowledged_generations.is_empty(),
		"a rejected terrain commit must not acknowledge the network"
	)
	_expect(
		road_tool.nodes_dirty_count == atomic_previous_nodes_dirty_count,
		"a rejected terrain commit must not invalidate node visuals"
	)

	terrain.commit_ready = true
	road_tool.advance_generation_after_stage = true
	network_renderer._process(0.0)
	_expect(
		road_tool._road_mesh_generation == atomic_previous_generation,
		"a generation change during road staging must retain the previous visual pair"
	)
	_expect(
		terrain.commit_count == 1,
		"a generation change during road staging must abort before terrain commit"
	)
	_expect(
		road_tool._staged_road_mesh_update.is_empty(),
		"a stale staged road batch must release its detached instances"
	)
	_expect(
		road_tool.nodes_dirty_count == atomic_previous_nodes_dirty_count,
		"a stale staged road batch must not invalidate node visuals"
	)
	_expect(
		simulation.acknowledged_generations.is_empty(),
		"a stale staged road batch must not acknowledge the network"
	)

	road_tool.advance_generation_after_stage = false
	simulation.response = _batch(
		10,
		false,
		CHUNK_SPAN_M,
		[_triangle_chunk(0, 0, 10.0)],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	simulation.reject_acknowledgement = true
	network_renderer._process(0.0)
	_expect(road_tool._road_mesh_generation == 10, "a complete terrain batch must publish its paired road generation")
	_expect(
		road_tool._road_chunk_instances[sentinel_key] != atomic_previous_instance,
		"a complete terrain batch must swap the staged road instance"
	)
	_expect(simulation.acknowledged_generations == [10], "the complete pair must reach the exact acknowledgement fence")
	_expect(simulation.dirty, "a rejected exact acknowledgement must retain network dirtiness")
	_expect(water.refresh_count == 0, "water clipping must wait when the exact acknowledgement fails")
	_expect(road_tool.border_check_count == 0, "border checks must wait when the exact acknowledgement fails")
	_expect(zoning_overlay.dirty_count == 0, "zoning visuals must wait when the exact acknowledgement fails")
	_expect(
		road_tool.nodes_dirty_count == atomic_previous_nodes_dirty_count,
		"node visuals must wait when the exact acknowledgement fails"
	)

	simulation.reject_acknowledgement = false
	network_renderer._process(0.0)
	_expect(simulation.acknowledged_generations == [10, 10], "the exact paired generation must retry acknowledgement")
	_expect(not simulation.dirty, "the successful exact acknowledgement must clear network dirtiness")
	_expect(water.refresh_count == 1, "water clipping must refresh after exact acknowledgement")
	_expect(road_tool.border_check_count == 1, "border checks must run after exact acknowledgement")
	_expect(zoning_overlay.dirty_count == 1, "zoning visuals must refresh after exact acknowledgement")
	_expect(
		road_tool.nodes_dirty_count == atomic_previous_nodes_dirty_count + 1,
		"node visuals must invalidate exactly once after exact acknowledgement"
	)

	host.queue_free()
	await process_frame
	if _failures == 0:
		print("network_tool_chunk_renderer_test: PASS")
	quit(_failures)

func _test_real_terrain_atomic_transaction() -> void:
	var simulation := MockSimulation.new()
	var terrain := TestTerrainRenderer.new()
	terrain.simulation_node = simulation
	terrain.patch_cols = 8
	terrain.patch_rows = 8
	terrain.patch_interval_cells = 1
	terrain.terrain_cell_m = 1.0
	var valid_key := Vector2i(2, 2)
	var blocked_key := Vector2i(3, 2)
	_install_terrain_patch(terrain, valid_key, "valid_previous")
	_install_terrain_patch(terrain, blocked_key, "blocked_previous")
	var valid_previous_mesh: Mesh = terrain.patches[valid_key]["node"].mesh
	var valid_previous_texture: ImageTexture = terrain.patches[valid_key]["height_texture"]
	var blocked_previous_mesh: Mesh = terrain.patches[blocked_key]["node"].mesh
	var blocked_previous_texture: ImageTexture = terrain.patches[blocked_key]["height_texture"]
	simulation.engineered_terrain_patches = PackedInt32Array([
		valid_key.x, valid_key.y, blocked_key.x, blocked_key.y,
	])

	var blocked_cases := [
		{"label": "failed", "status": "failed"},
		{"label": "conflicted", "status": "conflicted"},
		{"label": "pathological", "status": "pathological"},
		{"label": "empty", "status": "empty", "suppressed": true},
		{"label": "missing status", "remove_status": true},
		{"label": "wrong contract", "status": "ok", "contract": 3},
		{"label": "malformed metadata", "status": "ok", "bad_metadata": true},
		{"label": "missing normals", "status": "ok", "empty_normals": true},
		{"label": "missing uvs", "status": "ok", "empty_uvs": true},
		{"label": "malformed mesh", "status": "ok", "bad_index": true},
	]
	var generation := 20
	for blocked_case in blocked_cases:
		generation += 1
		_configure_dirty_terrain_batch(simulation, [valid_key, blocked_key], generation)
		terrain.patch_payload_ready[valid_key] = _engineered_terrain_payload(valid_key, generation)
		var blocked_payload := _engineered_terrain_payload(blocked_key, generation)
		if blocked_case.has("status"):
			blocked_payload["terrain_cdt_status"] = blocked_case["status"]
		if bool(blocked_case.get("suppressed", false)):
			blocked_payload["terrain_cdt_mesh_suppressed"] = true
		if bool(blocked_case.get("remove_status", false)):
			blocked_payload.erase("terrain_cdt_status")
		if blocked_case.has("contract"):
			blocked_payload["terrain_cdt_contract_revision"] = blocked_case["contract"]
		if bool(blocked_case.get("bad_metadata", false)):
			blocked_payload["terrain_cdt_mesh_suppressed"] = "false"
		if bool(blocked_case.get("empty_normals", false)):
			blocked_payload["terrain_mesh_normals"] = PackedVector3Array()
		if bool(blocked_case.get("empty_uvs", false)):
			blocked_payload["terrain_mesh_uvs"] = PackedVector2Array()
		if bool(blocked_case.get("bad_index", false)):
			blocked_payload["terrain_mesh_indices"] = PackedInt32Array([0, 1, 99])
		terrain.patch_payload_ready[blocked_key] = blocked_payload
		var prepared := terrain.prepare_terrain_visual_update(false)
		_expect(
			prepared.is_empty(),
			"%s engineered payload must block the complete real terrain transaction"
			% blocked_case["label"]
		)
		_expect(
			terrain.patches[valid_key]["last_patch_data"]["marker"] == "valid_previous",
			"%s rejection must retain the valid sibling terrain" % blocked_case["label"]
		)
		_expect(
			terrain.patches[blocked_key]["last_patch_data"]["marker"] == "blocked_previous",
			"%s rejection must retain the blocked terrain patch" % blocked_case["label"]
		)
		_expect(
			terrain.patches[valid_key]["node"].mesh == valid_previous_mesh
			and terrain.patches[valid_key]["height_texture"] == valid_previous_texture
			and terrain.patches[valid_key]["material"].get_shader_parameter("heightmap")
				== valid_previous_texture,
			"%s rejection must retain the valid sibling's visible resources"
			% blocked_case["label"]
		)
		_expect(
			terrain.patches[blocked_key]["node"].mesh == blocked_previous_mesh
			and terrain.patches[blocked_key]["height_texture"] == blocked_previous_texture
			and terrain.patches[blocked_key]["material"].get_shader_parameter("heightmap")
				== blocked_previous_texture,
			"%s rejection must retain the blocked patch's visible resources"
			% blocked_case["label"]
		)
	_expect(
		simulation.acknowledged_terrain_states.is_empty(),
		"blocked real terrain transactions must not acknowledge any generation"
	)

	generation += 1
	_configure_dirty_terrain_batch(simulation, [valid_key], generation)
	simulation.engineered_terrain_patches = PackedInt32Array([valid_key.x, valid_key.y])
	terrain.patch_payload_ready[valid_key] = _engineered_terrain_payload(valid_key, generation)
	terrain.fail_next_stage = true
	_expect(
		terrain.prepare_terrain_visual_update(false).is_empty(),
		"a transient detached-resource staging failure must abort the transaction"
	)
	_expect(
		terrain.patch_payload_ready.has(valid_key)
		and not terrain._dirty_engineered_patch_has_handled_bad_cdt(valid_key),
		"a resource staging failure must preserve the payload for same-generation retry"
	)
	_expect(
		not terrain.prepare_terrain_visual_update(false).is_empty(),
		"a valid engineered payload must retry after a transient staging failure"
	)

	var road_tool := TestRoadTool.new()
	road_tool.name = "RoadTool"
	road_tool.simulation_node = simulation
	road_tool.road_mesh_root = Node3D.new()
	road_tool.add_child(road_tool.road_mesh_root)
	simulation.generation = 1
	simulation.response = _batch(1, true, CHUNK_SPAN_M, [_triangle_chunk(0, 0)])
	_expect(road_tool.update_main_mesh(1) == 1, "real-terrain integration needs a resident road baseline")
	var old_road_instance: MeshInstance3D = road_tool._road_chunk_instances[Vector2i.ZERO]
	var water := MockWater.new()
	var zoning_overlay := MockZoningOverlay.new()
	var network_renderer: Node = NetworkRendererScript.new()
	network_renderer.simulation_node = simulation
	network_renderer.terrain = terrain
	network_renderer.water = water
	network_renderer.road_tool = road_tool
	network_renderer.zoning_overlay = zoning_overlay

	generation += 1
	_configure_dirty_terrain_batch(simulation, [valid_key, blocked_key], generation)
	simulation.dirty = true
	simulation.response = _batch(generation, false, CHUNK_SPAN_M, [_triangle_chunk(0, 0, 5.0)])
	terrain.patch_payload_ready[valid_key] = _engineered_terrain_payload(valid_key, generation)
	var pathological_payload := _engineered_terrain_payload(blocked_key, generation)
	pathological_payload["terrain_cdt_status"] = "pathological"
	terrain.patch_payload_ready[blocked_key] = pathological_payload
	network_renderer._process(0.0)
	_expect(road_tool._road_mesh_generation == 1, "real pathological terrain must block the road swap")
	_expect(
		road_tool._road_chunk_instances[Vector2i.ZERO] == old_road_instance,
		"real pathological terrain must retain the previous road instance"
	)
	_expect(simulation.acknowledged_generations.is_empty(), "blocked real terrain must not acknowledge roads")
	_expect(
		terrain.patches[valid_key]["node"].mesh == valid_previous_mesh
		and terrain.patches[valid_key]["height_texture"] == valid_previous_texture
		and terrain.patches[valid_key]["material"].get_shader_parameter("heightmap")
			== valid_previous_texture
		and terrain.patches[blocked_key]["node"].mesh == blocked_previous_mesh
		and terrain.patches[blocked_key]["height_texture"] == blocked_previous_texture
		and terrain.patches[blocked_key]["material"].get_shader_parameter("heightmap")
			== blocked_previous_texture,
		"coordinated pathological terrain must retain the complete visible terrain pair"
	)

	var newly_resident_key := Vector2i(4, 2)
	generation += 1
	_configure_dirty_terrain_batch(simulation, [newly_resident_key, blocked_key], generation)
	simulation.dirty = true
	simulation.engineered_terrain_patches = PackedInt32Array([
		newly_resident_key.x, newly_resident_key.y, blocked_key.x, blocked_key.y,
	])
	terrain.patch_payload_ready[newly_resident_key] = _engineered_terrain_payload(
		newly_resident_key,
		generation
	)
	terrain._refresh_engineered_patch_lookup()
	var blocked_residency_payload := _engineered_terrain_payload(blocked_key, generation)
	blocked_residency_payload["terrain_cdt_status"] = "failed"
	terrain.patch_payload_ready[blocked_key] = blocked_residency_payload
	terrain._sync_patch_residency(false, false)
	_expect(
		not terrain.patches.has(newly_resident_key),
		"network-pending residency must not publish a newly visible terrain patch"
	)
	# Simulate a stale frame-start flag that still allows additions after network dirtiness appears.
	terrain._sync_patch_residency(false, true)
	_expect(
		terrain.patches.has(newly_resident_key)
		and not terrain.resident_patch_lookup.has(newly_resident_key)
		and not terrain.patches[newly_resident_key]["node"].visible,
		"activation must keep a staged patch hidden when network publication becomes pending"
	)
	_expect(
		terrain.prepare_terrain_visual_update(false).is_empty(),
		"a failed sibling must still block the coordinated terrain transaction"
	)

	var race_key := Vector2i(0, 2)
	_install_terrain_patch(terrain, race_key, "race_previous")
	var race_previous_mesh: Mesh = terrain.patches[race_key]["node"].mesh
	var race_previous_texture: ImageTexture = terrain.patches[race_key]["height_texture"]
	generation += 1
	_configure_dirty_terrain_batch(simulation, [race_key], generation)
	simulation.dirty = false
	simulation.engineered_terrain_patches = PackedInt32Array([race_key.x, race_key.y])
	terrain.patch_payload_ready[race_key] = _engineered_terrain_payload(race_key, generation)
	simulation.network_edit_on_border_query = true
	_expect(
		not terrain._try_commit_standalone_terrain_visual_update(),
		"a network generation published during standalone staging must abort the terrain commit"
	)
	_expect(
		terrain.patches[race_key]["node"].mesh == race_previous_mesh
		and terrain.patches[race_key]["height_texture"] == race_previous_texture
		and terrain.patches[race_key]["material"].get_shader_parameter("heightmap")
			== race_previous_texture,
		"the standalone generation fence must retain the previous visible terrain"
	)
	_expect(
		simulation.acknowledged_terrain_states.is_empty(),
		"an aborted standalone terrain transaction must not acknowledge its generation"
	)

	generation += 1
	_configure_dirty_terrain_batch(simulation, [valid_key, blocked_key], generation)
	simulation.dirty = true
	simulation.engineered_terrain_patches = PackedInt32Array([
		valid_key.x, valid_key.y, blocked_key.x, blocked_key.y,
	])
	simulation.generation = generation
	simulation.response = _batch(generation, false, CHUNK_SPAN_M, [_triangle_chunk(0, 0, 6.0)])
	terrain.patch_payload_ready[valid_key] = _engineered_terrain_payload(valid_key, generation)
	var contained_payload := _engineered_terrain_payload(blocked_key, generation)
	contained_payload["terrain_cdt_pathological_faces_omitted"] = 1
	terrain.patch_payload_ready[blocked_key] = contained_payload
	var contained_preflight := terrain.prepare_terrain_visual_update(false)
	_expect(
		not contained_preflight.is_empty(),
		"current-contract ok terrain with omitted pathological faces must pass real preflight"
	)
	network_renderer._process(0.0)
	_expect(road_tool._road_mesh_generation == generation, "accepted baked terrain must publish its road pair")
	_expect(
		terrain.patches[blocked_key]["node"].mesh is ArrayMesh,
		"contained pathological faces must keep the clipped baked ArrayMesh"
	)
	_expect(
		bool(terrain.patches[blocked_key]["height_is_baked"]),
		"accepted engineered terrain must never fall back to a heightmap PlaneMesh"
	)
	_expect(simulation.acknowledged_terrain_states.size() == 1, "complete terrain pair must acknowledge once")
	_expect(simulation.acknowledged_generations == [generation], "complete road pair must acknowledge exactly")

	var regular_generation := generation + 1
	simulation.engineered_terrain_patches = PackedInt32Array()
	_configure_dirty_terrain_batch(simulation, [valid_key], regular_generation)
	terrain.patch_payload_ready[valid_key] = _regular_terrain_payload(valid_key, regular_generation)
	_expect(terrain.update_terrain_visuals(), "statusless non-engineered terrain must remain publishable")
	_expect(
		terrain.patches[valid_key]["node"].mesh is PlaneMesh,
		"only non-engineered regular terrain may use the heightmap PlaneMesh"
	)
	terrain.free()
	road_tool.free()
	water.free()
	zoning_overlay.free()
	network_renderer.free()
	simulation.free()

func _install_terrain_patch(terrain: Node, key: Vector2i, marker: String) -> void:
	var patch_node := MeshInstance3D.new()
	var retaining_wall_node := MeshInstance3D.new()
	patch_node.add_child(retaining_wall_node)
	var material := ShaderMaterial.new()
	material.shader = load("res://assets/materials/terrain.gdshader")
	patch_node.material_override = material
	var height_image := Image.create(2, 2, false, Image.FORMAT_RF)
	height_image.fill(Color.BLACK)
	var height_texture := ImageTexture.create_from_image(height_image)
	patch_node.mesh = _sentinel_terrain_mesh()
	material.set_shader_parameter("heightmap", height_texture)
	terrain.add_child(patch_node)
	terrain.patches[key] = {
		"node": patch_node,
		"retaining_wall_node": retaining_wall_node,
		"material": material,
		"height_image": height_image,
		"height_texture": height_texture,
		"spare_height_image": null,
		"spare_height_texture": null,
		"spare_height_texture_width": 0,
		"spare_height_texture_height": 0,
		"texture_width": 2,
		"texture_height": 2,
		"lod_step": 1,
		"subdivision_factor": 1,
		"height_is_baked": true,
		"engineered_bad_cdt_blocked": false,
		"last_patch_data": {"marker": marker},
	}
	terrain.resident_patch_lookup[key] = true

func _sentinel_terrain_mesh() -> ArrayMesh:
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = PackedVector3Array([
		Vector3(-0.5, 0.0, -0.5),
		Vector3(0.5, 0.0, -0.5),
		Vector3(-0.5, 0.0, 0.5),
	])
	arrays[Mesh.ARRAY_NORMAL] = PackedVector3Array([Vector3.UP, Vector3.UP, Vector3.UP])
	arrays[Mesh.ARRAY_TEX_UV] = PackedVector2Array([Vector2.ZERO, Vector2.RIGHT, Vector2.DOWN])
	arrays[Mesh.ARRAY_INDEX] = PackedInt32Array([0, 1, 2])
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh

func _configure_dirty_terrain_batch(
	simulation: MockSimulation,
	keys: Array[Vector2i],
	generation: int
) -> void:
	simulation.generation = generation
	simulation.terrain_dirty = true
	simulation.dirty_terrain_states = PackedInt64Array()
	simulation.dirty_terrain_patches = PackedInt32Array()
	for key in keys:
		simulation.dirty_terrain_states.append(key.x)
		simulation.dirty_terrain_states.append(key.y)
		simulation.dirty_terrain_states.append(generation)
		simulation.dirty_terrain_patches.append(key.x)
		simulation.dirty_terrain_patches.append(key.y)

func _terrain_payload_metadata(key: Vector2i, generation: int, engineered: bool) -> Dictionary:
	var height_bytes := PackedByteArray()
	height_bytes.resize(16)
	return {
		"patch_x": key.x,
		"patch_z": key.y,
		"surface_generation": generation,
		"render_step_mm": 2000 if engineered else 0,
		"terrain_requires_engineered_refinement": engineered,
		"sample_width": 2,
		"sample_height": 2,
		"texture_width": 2,
		"texture_height": 2,
		"inner_offset_x": 0,
		"inner_offset_z": 0,
		"world_origin_x": float(key.x),
		"world_origin_z": float(key.y),
		"world_size_x": 1.0,
		"world_size_z": 1.0,
		"height_bytes": height_bytes,
	}

func _engineered_terrain_payload(key: Vector2i, generation: int) -> Dictionary:
	var payload := _terrain_payload_metadata(key, generation, true)
	payload.merge({
		"terrain_cdt_status": "ok",
		"terrain_cdt_contract_revision": 4,
		"terrain_cdt_mesh_suppressed": false,
		"terrain_mesh_vertices": PackedVector3Array([
			Vector3(-0.5, 0.0, -0.5),
			Vector3(0.5, 0.0, -0.5),
			Vector3(-0.5, 0.0, 0.5),
		]),
		"terrain_mesh_normals": PackedVector3Array([Vector3.UP, Vector3.UP, Vector3.UP]),
		"terrain_mesh_uvs": PackedVector2Array([Vector2.ZERO, Vector2.RIGHT, Vector2.DOWN]),
		"terrain_mesh_indices": PackedInt32Array([0, 1, 2]),
		"terrain_retaining_wall_mesh_vertices": PackedVector3Array(),
		"terrain_retaining_wall_mesh_normals": PackedVector3Array(),
		"terrain_retaining_wall_mesh_uvs": PackedVector2Array(),
		"terrain_retaining_wall_mesh_indices": PackedInt32Array(),
	})
	return payload

func _regular_terrain_payload(key: Vector2i, generation: int) -> Dictionary:
	return _terrain_payload_metadata(key, generation, false)

func _batch(
	generation: int,
	full_replace: bool,
	chunk_span_m: float,
	chunks: Array,
	chunk_origin_x_m: float = 0.0,
	chunk_origin_z_m: float = 0.0
) -> Dictionary:
	return {
		"surface_generation": generation,
		"full_replace": full_replace,
		"chunk_span_m": chunk_span_m,
		"chunk_origin_x_m": chunk_origin_x_m,
		"chunk_origin_z_m": chunk_origin_z_m,
		"chunks": chunks,
	}

func _triangle_chunk(chunk_x: int, chunk_z: int, x_offset: float = 0.0) -> Dictionary:
	var chunk := {
		"chunk_x": chunk_x,
		"chunk_z": chunk_z,
		"removed": false,
	}
	for layer in LAYERS:
		chunk[layer + "_vertices"] = PackedVector3Array()
		chunk[layer + "_normals"] = PackedVector3Array()
		chunk[layer + "_uvs"] = PackedVector2Array()
		chunk[layer + "_colors"] = PackedColorArray()
	chunk["road_vertices"] = PackedVector3Array([
		Vector3(x_offset, 0.0, 0.0),
		Vector3(x_offset + 1.0, 0.0, 0.0),
		Vector3(x_offset, 0.0, 1.0),
	])
	chunk["road_normals"] = PackedVector3Array([
		Vector3.UP,
		Vector3.UP,
		Vector3.UP,
	])
	chunk["road_uvs"] = PackedVector2Array([
		Vector2.ZERO,
		Vector2.RIGHT,
		Vector2.DOWN,
	])
	chunk["road_colors"] = PackedColorArray([
		Color.WHITE,
		Color.WHITE,
		Color.WHITE,
	])
	return chunk

func _tombstone(chunk_x: int, chunk_z: int) -> Dictionary:
	return {
		"chunk_x": chunk_x,
		"chunk_z": chunk_z,
		"removed": true,
	}

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
