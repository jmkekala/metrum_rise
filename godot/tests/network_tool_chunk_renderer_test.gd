# SPDX-License-Identifier: GPL-2.0-only

## Headless contract test for atomic road-chunk hydration and replacement, and for the road
## tool's synchronous preview (`ROAD-68-E`): one frame samples, previews and shows a stroke.
extends SceneTree

const NetworkRendererScript := preload("res://scripts/renderers/network_renderer.gd")
const TerrainRendererScript := preload("res://scripts/renderers/terrain.gd")
const RoadToolScript := preload("res://scripts/tools/road_tool.gd")
const InputManagerScript := preload("res://scripts/core/input_manager.gd")
const WorldMaterialsScript := preload("res://scripts/renderers/world_materials.gd")
const LAYERS := ["skirt", "curb", "raised_step", "sidewalk", "road", "marking", "concrete"]
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
	var terrain_dirty: bool = false
	var full_snapshot_requests: Array[bool] = []
	var acknowledged_generations: Array[int] = []
	var acknowledged_terrain_states: Array[PackedInt64Array] = []
	var reject_acknowledgement: bool = false
	var network_edit_on_border_query: bool = false
	var rejection_reports: Array[Dictionary] = []

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

	func get_road_tool_surface_generation() -> int:
		return generation

	func get_vegetation_land_cover(_key: Vector2i) -> Dictionary:
		return {"width": 1, "height": 1, "bytes": PackedByteArray([0]),
			"world_bounds": Vector3.ZERO, "generations": PackedInt64Array([generation])}

	func is_vegetation_land_cover_current(_key: Vector2i, revisions: PackedInt64Array) -> bool:
		return revisions == PackedInt64Array([generation])

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

	func log_road_tool_rejection(points, fwd, bkw, snap, validation) -> int:
		rejection_reports.append({"points": points, "lanes": Vector2i(fwd, bkw), "snap": snap, "validation": validation})
		return rejection_reports.size()

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

class MockWater:
	extends Node

	var refresh_count: int = 0

	func refresh_road_covered_patches(_keys: PackedInt32Array) -> void:
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

class MockRoadCandidateSimulation:
	extends MockSimulation
	var validation: Dictionary = {}
	# preview_road calls and the exact stroke of the last one.
	var preview_calls: int = 0
	var last_preview_points := PackedVector3Array()
	var commit_requests: int = 0
	var commit_points := PackedVector3Array()
	var commit_result: Variant = null
	var border_node: int = -1
	var start_direction := {"kind": 0, "direction": Vector3.ZERO}

	func get_road_tool_start_direction(_pos, _node, _edge, _generation) -> Dictionary:
		return start_direction

	func add_road_with_snap(points, _fwd, _bkw, _snap) -> int:
		commit_requests += 1
		commit_points = points
		return commit_requests

	func get_road_commit_result(_request_id: int) -> Variant:
		return commit_result

	func check_border_candidate(_pos: Vector3) -> int:
		return border_node

	# The preview's graded tiles (TERRAIN-07-E): the mock shows none.
	func advance_road_preview_terrain() -> Dictionary:
		return {}

	func release_road_preview_terrain() -> int:
		return 0

	func preview_road(points, _fwd, _bkw, _snap) -> Dictionary:
		preview_calls += 1
		last_preview_points = points
		var result := validation.duplicate(true)
		result["prepared_points"] = points
		result["surface_generation"] = generation
		return result

class ClickRoadTool:
	extends RoadToolScript
	var pointer := Vector3.ZERO

	func get_world_mouse_pos() -> Vector3:
		is_valid = true
		return pointer

class PointerRoadTool:
	extends RoadToolScript
	var pointer: Vector3 = Vector3.ZERO
	var pointer_queries: int = 0
	var draws: int = 0
	var last_drawn_points := PackedVector3Array()

	func get_world_mouse_pos() -> Vector3:
		pointer_queries += 1
		is_valid = true
		return pointer

	func _update_blueprint_visuals() -> void:
		pass

	func _update_node_visuals() -> void:
		pass

	func _show_preview(points: PackedVector3Array) -> void:
		draws += 1
		last_drawn_points = points
		super._show_preview(points)

class ProcessPhaseProbe:
	extends Node
	var phase := ""
	var events: Array[String]
	var frames: Array[int]

	func _process(_delta: float) -> void:
		events.append(phase)
		frames.append(Engine.get_process_frames())
		set_process(false)

var _failures: int = 0

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	await _test_road_preview_process_phases()
	_test_road_pointer_frame_updates()
	_test_road_preview_swaps_on_installed_commit()
	await _test_native_road_cursor_contract()
	_test_road_preview_context_revision()
	_test_road_commit_acknowledgment()
	_test_free_control_curve_chain()
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
		sentinel.position == Vector3(
			CHUNK_ORIGIN_X_M + 0.5 * CHUNK_SPAN_M, 0.0, CHUNK_ORIGIN_Z_M + 0.5 * CHUNK_SPAN_M
		),
		"chunk zero must be placed at its centre from the published origin"
	)
	_expect(
		previous_changed.position == Vector3(
			CHUNK_ORIGIN_X_M + 3.5 * CHUNK_SPAN_M,
			0.0,
			CHUNK_ORIGIN_Z_M - 1.5 * CHUNK_SPAN_M
		),
		"nonzero chunk must be placed at its centre relative to the published origin"
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
	_test_road_payload_certificate(road_tool)

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

func _test_road_payload_certificate(road_tool: Node) -> void:
	# The Rust certificate may skip only the per-vertex scan, never the boundary shape checks.
	var key := Vector2i(1, 1)
	var uncertified: MeshInstance3D = road_tool._build_road_chunk_instance(
		_triangle_chunk(1, 1), key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M
	)
	var certified_chunk := _triangle_chunk(1, 1)
	certified_chunk["road_mesh_payload_validated"] = true
	var certified: MeshInstance3D = road_tool._build_road_chunk_instance(
		certified_chunk, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M
	)
	_expect(uncertified != null and certified != null, "valid road payloads must build")
	if uncertified != null and certified != null:
		_expect(
			certified.mesh.get_surface_count() == uncertified.mesh.get_surface_count()
			and certified.mesh.surface_get_arrays(0) == uncertified.mesh.surface_get_arrays(0),
			"certified payload must upload identical geometry"
		)
		_expect(
			certified.mesh.surface_get_arrays(0)[Mesh.ARRAY_CUSTOM0] == certified_chunk["road_crossings"],
			"crossing metres and age must survive upload at float precision"
		)
		certified.free()
		uncertified.free()

	var bad_crossing := _triangle_chunk(1, 1)
	bad_crossing["road_mesh_payload_validated"] = true
	bad_crossing["road_crossings"] = PackedFloat32Array([0, 0, 0, 0])
	_expect(
		road_tool._build_road_chunk_instance(bad_crossing, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"certified crossing payload must still pass attribute count checks"
	)
	var wrong_type := _triangle_chunk(1, 1)
	wrong_type["road_mesh_payload_validated"] = 1
	_expect(
		road_tool._build_road_chunk_instance(wrong_type, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"non-bool road payload certificate must be rejected"
	)
	var certified_bad_count := _triangle_chunk(1, 1)
	certified_bad_count["road_mesh_payload_validated"] = true
	certified_bad_count["road_normals"] = PackedVector3Array([Vector3.UP])
	_expect(
		road_tool._build_road_chunk_instance(certified_bad_count, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"certified payload must still pass attribute count checks"
	)
	var certified_bad_type := _triangle_chunk(1, 1)
	certified_bad_type["road_mesh_payload_validated"] = true
	certified_bad_type["road_uvs"] = PackedVector3Array()
	_expect(
		road_tool._build_road_chunk_instance(certified_bad_type, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"certified payload must still pass Variant type checks"
	)
	var uncertified_nan := _triangle_chunk(1, 1)
	uncertified_nan["road_mesh_payload_validated"] = false
	uncertified_nan["road_colors"] = PackedColorArray([Color(NAN, 0.0, 0.0), Color.WHITE, Color.WHITE])
	_expect(
		road_tool._build_road_chunk_instance(uncertified_nan, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"uncertified non-finite payload must be rejected"
	)

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
	chunk["road_crossings"] = PackedFloat32Array([0, 0, 1.5, 0, 1, 0, 1.5, 1, 0, 1, 1.5, 0])
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

func _test_native_road_cursor_contract() -> void:
	var host := Node3D.new()
	root.add_child(host)
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(256.0, 256.0, 8.0, 128.0, 0.0), "cursor bridge fixture must create a flat world")
	var previous: Dictionary = {}
	for index in range(5):
		var x := 30.0 + index * 0.1
		var result: Variant = simulation.get_road_tool_cursor_pos(
			Vector3(x, 100.0, 20.0), Vector3.DOWN, 0.0, false, 0,
			Vector3.ZERO, Vector3.ZERO, false, 1, 1, 25.0, previous, 8.0, Vector3.ZERO
		)
		_expect(result is Dictionary, "native cursor result must include position and target identity")
		if result is Dictionary:
			var position: Vector3 = result["position"]
			_expect(absf(position.x - x) < 0.0001 and absf(position.z - 20.0) < 0.0001, "free cursor motion must remain continuous through the native bridge")
			_expect(result["snap_edge"] == -1 and result["snap_node"] == -1, "free terrain must not retain a network target")
			_expect(result["surface_generation"] == simulation.get_road_tool_surface_generation(), "cursor target must carry its source generation")
			previous = result
	await _test_native_road_preview_contract(simulation)
	var points := PackedVector3Array([Vector3(-60.0, 0.0, 32.0), Vector3(60.0, 0.0, 32.0)])
	var request := simulation.add_road_with_snap(points, 1, 1, true)
	var accepted: Dictionary = await _await_road_commit(simulation, request)
	_expect(accepted.get("committed", false) and simulation.get_road_benchmark_state()["live_edges"] == 1, "native acknowledgment must reflect an accepted road")
	var grid_cursor: Dictionary = simulation.get_road_tool_cursor_pos(
		Vector3(6.0, 100.0, 104.0), Vector3.DOWN, 0.0, true, 2,
		Vector3(5.0, 0.0, 32.0), Vector3.ZERO, true, 1, 1, 0.0, {}, 8.0, Vector3.ZERO
	)
	_expect(grid_cursor["position"].distance_to(Vector3(5.0, 0.0, 102.0)) < 0.01, "zoning snap must use the curb grid without guide snapping")
	var angled_cursor: Dictionary = simulation.get_road_tool_cursor_pos(
		Vector3(45.0, 100.0, 102.0), Vector3.DOWN, 0.0, true, 2,
		Vector3(5.0, 0.0, 32.0), Vector3.ZERO, true, 1, 1, 0.0, {}, 8.0, Vector3.ZERO
	)
	_expect(angled_cursor["position"].distance_to(Vector3(45.0, 0.0, 102.0)) < 0.01, "zoning snap must allow free angles away from grid axes")
	var free_cursor: Dictionary = simulation.get_road_tool_cursor_pos(
		Vector3(80.0, 100.0, 33.0), Vector3.DOWN, 0.0, true, 2,
		Vector3(60.0, 0.0, 32.0), Vector3.ZERO, false, 1, 1, 0.0, {}, 8.0, Vector3.ZERO)
	_expect(free_cursor["position"].distance_to(Vector3(80.0, 0.0, 33.0)) < 0.01, "removed continuation guides must not snap the cursor")
	_expect(not free_cursor.has("guide_vertices"), "cursor bridge must not generate guide geometry")
	var snapped_cursor: Dictionary = simulation.get_road_tool_cursor_pos(
		Vector3(80.0, 100.0, 33.0), Vector3.DOWN, 0.0, true, 2,
		Vector3(60.0, 0.0, 32.0), Vector3.ZERO, false, 1, 1, 0.0, {}, 8.0, Vector3(1.0, 0.0, 0.0))
	_expect(absf(snapped_cursor["position"].z - 32.0) < 0.01 and snapped_cursor.get("angle_snapped", false), "a stroke 3° off its start direction must snap onto it")
	var end_cursor: Dictionary = simulation.get_road_tool_cursor_pos(
		Vector3(60.5, 100.0, 32.5), Vector3.DOWN, 0.0, false, 0,
		Vector3.ZERO, Vector3.ZERO, false, 1, 1, 0.0, {}, 8.0, Vector3.ZERO)
	var continuation: Dictionary = simulation.get_road_tool_start_direction(
		end_cursor["position"], end_cursor["snap_node"], end_cursor["snap_edge"], end_cursor["surface_generation"])
	_expect(end_cursor["snap_node"] >= 0 and continuation["kind"] == 1 and continuation["direction"].distance_to(Vector3(1.0, 0.0, 0.0)) < 0.01, "a stroke from a dead end must continue its road")
	var branch_cursor: Dictionary = simulation.get_road_tool_cursor_pos(
		Vector3(3.0, 100.0, 33.0), Vector3.DOWN, 0.0, false, 0,
		Vector3.ZERO, Vector3.ZERO, true, 1, 1, 0.0, {}, 8.0, Vector3.ZERO
	)
	_expect(absf(branch_cursor["position"].x - 5.0) < 0.01 and absf(branch_cursor["position"].z - 32.0) < 0.01, "grid-aligned branch start must remain on the supplying road")
	_expect(branch_cursor["snap_edge"] >= 0, "zoning snap must retain real road connections")
	var modes := RoadToolScript.new()
	var ui = preload("res://scripts/ui/main_ui.gd").new()
	var manager = preload("res://scripts/core/input_manager.gd").new()
	var vegetation = preload("res://scripts/tools/vegetation_tool.gd").new()
	manager.vegetation_tool = vegetation
	ui.input_manager = manager
	ui.simulation_node = simulation
	ui.road_tool = modes
	ui._build_ui()
	_expect(ui.get_meta("options_panel").get_index() < ui.road_sub_menu.get_parent().get_parent().get_index(), "road options must sit to the left of road types")
	ui.zoning_grid_snap_check.button_pressed = true
	_expect(modes._zoning_snap_enabled(), "the actual checkbox must enable zoning snap")
	ui._set_draw_mode(1)
	_expect(ui.zoning_grid_snap_check.disabled and not modes._zoning_snap_enabled(), "spline mode disables zoning snap")
	ui._set_draw_mode(0)
	_expect(not ui.zoning_grid_snap_check.disabled and modes._zoning_snap_enabled(), "returning to straight mode restores the checkbox setting")
	ui.zoning_grid_snap_check.button_pressed = false
	ui.free()
	manager.free()
	vegetation.free()
	_expect(not modes._zoning_snap_enabled(), "zoning snapping starts disabled")
	modes.zoning_grid_snap_enabled = true
	_expect(modes._zoning_snap_enabled(), "checkbox enables the zoning snap path")
	modes.zoning_grid_snap_enabled = false
	modes.blueprint_mesh = MeshInstance3D.new()
	modes.add_child(modes.blueprint_mesh)
	modes._begin_stroke(Vector3.ZERO, 1, Vector3.RIGHT)
	_expect(modes._angle_snap() == Vector3.RIGHT, "straight strokes retain their start direction for angle snapping")
	var shift := InputEventKey.new()
	shift.keycode = KEY_SHIFT
	shift.pressed = true
	shift.shift_pressed = true
	Input.parse_input_event(shift)
	Input.flush_buffered_events()
	_expect(not modes._zoning_snap_enabled() and modes._snap_to_roads_enabled(), "Shift must not enable zoning snap or disable road connections")
	_expect(modes._angle_snap() == Vector3.ZERO, "Shift makes straight-road angle snapping free")
	modes.zoning_grid_snap_enabled = true
	_expect(modes._zoning_snap_enabled(), "checkbox controls zoning snap while Shift is held")
	shift = InputEventKey.new()
	shift.keycode = KEY_SHIFT
	Input.parse_input_event(shift)
	Input.flush_buffered_events()
	_expect(modes._zoning_snap_enabled(), "releasing Shift must not change the checkbox setting")
	_expect(modes._angle_snap() == Vector3.RIGHT, "releasing Shift restores straight-road angle snapping")
	modes.zoning_grid_snap_enabled = true
	modes.draw_mode = 1
	_expect(not modes._zoning_snap_enabled(), "spline drawing does not promise a square zoning lattice")
	_expect(modes._angle_snap() == Vector3.ZERO, "curve endpoints and control points remain free of angle snapping")
	modes.cancel_road()
	modes.free()
	var invalid := PackedVector3Array([Vector3.ZERO])
	var abandoned := simulation.add_road_with_snap(invalid, 1, 1, true)
	request = simulation.add_road_with_snap(invalid, 1, 1, true)
	var superseded: Dictionary = simulation.get_road_commit_result(abandoned)
	_expect(not superseded["committed"] and superseded["detail"] == "road_request_replaced", "superseded requests must not wait forever")
	var rejected: Dictionary = await _await_road_commit(simulation, request)
	_expect(not rejected.get("committed", true) and simulation.get_road_benchmark_state()["live_edges"] == 1, "native rejection must acknowledge that no topology was added")
	host.free()

func _await_road_commit(simulation: SimulationNode, request: int) -> Dictionary:
	var deadline := Time.get_ticks_msec() + 10000
	while Time.get_ticks_msec() < deadline:
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			return result
		await process_frame
	_expect(false, "native road completion must finish")
	return {}

# Hover validation never waits for the simulation lock: while the sim thread holds it, the
# answer is pending (`is_pending`, not valid), and the road tool asks again next frame.
func _await_hover_validation(simulation: SimulationNode, points: PackedVector3Array) -> Dictionary:
	var deadline := Time.get_ticks_msec() + 10000
	var hover: Dictionary = {}
	while Time.get_ticks_msec() < deadline:
		hover = simulation.validate_road_candidate_with_snap(points, 1, 1, false)
		if not hover.get("is_pending", false):
			return hover
		await process_frame
	_expect(false, "hover validation must stop pending")
	return hover

func _test_native_road_preview_contract(simulation: SimulationNode) -> void:
	var tool := RoadToolScript.new()
	tool.name = "RoadTool"
	tool.simulation_node = simulation
	tool.road_mesh_root = Node3D.new()
	tool.add_child(tool.road_mesh_root)
	var generation := simulation.get_network_render_generation()
	_expect(tool.update_main_mesh(generation) == generation, "isolated preview must hydrate the empty committed chunk grid")
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._road_preview_material = WorldMaterialsScript.road_preview_material()
	tool.blueprint_mesh.material_override = tool._road_preview_material
	tool._info_label = Label.new()
	tool.add_child(tool._info_label)
	var points := PackedVector3Array([Vector3(-40.0, 0.0, -8.0), Vector3(0.0, 0.0, 8.0), Vector3(40.0, 0.0, -8.0)])
	for lanes in [Vector2i(1, 1), Vector2i(3, 1), Vector2i(1, 0), Vector2i(0, 0)]:
		var preview: Dictionary = simulation.preview_road(points, lanes.x, lanes.y, false)
		_expect(preview.get("is_valid", false), "isolated %s preview must validate: %s" % [lanes, preview.get("invalid_reason", "")])
		_expect(preview.get("has_road_chunks", false), "the synchronous preview must return its road chunks")
		_expect(int(preview.get("surface_generation", -1)) == simulation.get_road_tool_surface_generation(), "the preview names its context generation")
		_expect(not preview.has("surface_vertices") and not preview.has("request_id"), "no ribbon and no request id remain")
		for stage in ["plan_ms", "roadbed_ms", "payload_ms", "packing_ms"]:
			_expect(float(preview.get(stage, -1.0)) >= 0.0, "the preview reports its %s" % stage)
		var chunks: Array = preview.get("road_chunks", [])
		_expect(not chunks.is_empty() and chunks.size() * 2 == PackedInt32Array(preview.get("changed_chunks", PackedInt32Array())).size(), "every changed chunk has its update")
		for chunk in chunks:
			if not chunk.get("removed", false):
				_expect(chunk.get("road_mesh_payload_validated", false), "preview chunks are sealed as committed chunks are")
	var hover: Dictionary = await _await_hover_validation(simulation, points)
	_expect(hover.get("is_valid", false), "hover validation keeps its prepared profile: %s" % hover.get("invalid_reason", ""))
	tool.fwd_lanes = 2
	tool.bkw_lanes = 1
	tool.active = true
	tool._node_visuals_dirty = false
	tool._node_visuals_visible = true
	tool._begin_stroke(points[0], 0, Vector3.ZERO)
	tool.set_scripted_pointer(true, points[-1])
	tool._process(1.0 / 60.0)
	_expect(tool._preview_display.sequence > 0, "one frame previews and shows the stroke's chunks")
	var instances: Array = tool._preview_display._instances.duplicate()
	_expect(not instances.is_empty() and tool.blueprint_mesh.mesh == null, "the preview shows compiled chunks, never a ribbon")
	var calls := tool._preview_sequence
	tool._queue_preview_update()
	tool._process(1.0 / 60.0)
	_expect(tool._preview_sequence == calls and tool._preview_display._instances == instances, "an unchanged stroke neither previews nor uploads again")
	for state in [{"is_valid": true}, {"is_pending": true}, {"is_valid": false}]:
		tool._update_road_preview_material(state)
		var expected := 1 if state.has("is_pending") else (0 if state["is_valid"] else 2)
		_expect(tool._road_preview_material.get_shader_parameter("placement_state") == expected, "road material must distinguish valid, checking, and rejected placement")
	var malformed: Dictionary = tool._preview.duplicate(true)
	malformed["road_chunks"][0]["road_normals"] = PackedVector3Array()
	_expect(not tool._preview_display.show(tool, malformed, tool._preview_sequence + 1), "incomplete mesh batches must fail atomically")
	_expect(tool._preview_display._instances == instances, "failed staging must preserve the complete previous preview")
	var rejected: Dictionary = tool._preview.duplicate(false)
	rejected["is_valid"] = false
	rejected["invalid_reason"] = "terrain_clip_missing_outer_boundary_owner"
	rejected["sequence"] = tool._preview_sequence + 1
	tool._preview = rejected
	tool._show_preview(points)
	_expect(tool._preview_display.sequence == 0 and tool._preview_display._instances.is_empty(), "a rejected stroke shows no road")
	_expect(tool._info_label.visible and tool._info_label.text.contains("terrain boundary"), "terrain rejection must retain an explanatory label")
	_expect(tool._road_preview_material.get_shader_parameter("placement_state") == 2, "rejected placement must be red")
	if OS.get_environment("METRUM_ROAD_PREVIEW_TIMING") == "1":
		for lane_count in [1, 4]:
			var timings: Array[int] = []
			for iteration in range(220):
				points[-1].x = 40.0 + float(iteration % 20) * 0.01
				var start := Time.get_ticks_usec()
				simulation.preview_road(points, lane_count, lane_count, false)
				if iteration >= 20:
					timings.append(Time.get_ticks_usec() - start)
			timings.sort()
			print("ROAD_PREVIEW_TIMING lanes=%d samples=%d preview_road_p50_us=%d p95_us=%d" % [lane_count * 2, timings.size(), timings[100], timings[190]])
	tool.cancel_road()
	tool.free()

func _pointer_road_tool(simulation: MockRoadCandidateSimulation) -> PointerRoadTool:
	simulation.generation = 1
	simulation.validation = {"is_valid": true}
	var tool := PointerRoadTool.new()
	tool.simulation_node = simulation
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool.active = true
	tool.current_state = RoadToolScript.State.SETTING_END
	tool.start_pos = Vector3.ZERO
	tool.control_pos = Vector3.ZERO
	tool.current_path = Path3D.new()
	tool.current_path.curve = Curve3D.new()
	tool.current_path.curve.bake_interval = 0.5
	tool.add_child(tool.current_path)
	tool._last_world_mouse_pos = Vector3(10.0, 0.0, 0.0)
	tool._has_last_world_mouse_pos = true
	return tool

func _road_frame(tool: PointerRoadTool) -> void:
	tool._process(1.0 / 60.0)

func _test_road_preview_process_phases() -> void:
	# Input and camera settle first; the road tool previews this frame's pointer after them.
	var events: Array[String] = []
	var frames: Array[int] = []
	for phase_spec in [
		["input", InputManagerScript.CAMERA_INPUT_PROCESS_PRIORITY],
		["camera", InputManagerScript.CAMERA_SETTLE_PROCESS_PRIORITY],
		["sample", RoadToolScript.PREVIEW_SAMPLE_PROCESS_PRIORITY],
		["completion", 0],
	]:
		var probe := ProcessPhaseProbe.new()
		probe.phase = phase_spec[0]
		probe.process_priority = phase_spec[1]
		probe.events = events
		probe.frames = frames
		root.add_child(probe)
	await process_frame
	await process_frame
	_expect(events == ["input", "camera", "sample", "completion"], "the preview samples after input and camera settle, before ordinary completion")
	_expect(frames.size() == 4 and frames.all(func(frame): return frame == frames[0]), "all preview phases must execute in one rendered frame")
	for child in root.get_children():
		if child is ProcessPhaseProbe:
			child.free()

func _test_road_pointer_frame_updates() -> void:
	var simulation := MockRoadCandidateSimulation.new()
	var tool := _pointer_road_tool(simulation)
	for index in range(32):
		tool.pointer = Vector3(20.0 + index, 0.0, 0.0)
		tool._unhandled_input(InputEventMouseMotion.new())
		tool._queue_preview_update()
	_expect(simulation.preview_calls == 0 and tool.draws == 0, "input bursts must not preview before the frame update")
	_road_frame(tool)
	_expect(tool.pointer_queries == 1, "the frame must resolve the cursor once")
	_expect(simulation.preview_calls == 1 and tool.draws == 1, "32 queued updates must coalesce into one synchronous preview")
	_expect(tool.last_drawn_points[-1] == tool.pointer and simulation.last_preview_points[-1] == tool.pointer, "the frame previews and shows this frame's pointer")
	tool.pointer = Vector3(60.0, 0.0, 0.0)
	_road_frame(tool)
	_expect(simulation.preview_calls == 2 and tool.last_drawn_points[-1] == tool.pointer, "camera-only pointer movement must preview again without a motion event")
	for index in range(8):
		tool.pointer.x = 60.25 + float(index) * 0.25
		_road_frame(tool)
		_expect(simulation.last_preview_points[-1] == tool.pointer, "every moved pose is previewed in its own frame")
	_expect(simulation.preview_calls == 10, "one preview per moved frame, no backlog")
	for index in range(16):
		tool._unhandled_input(InputEventMouseMotion.new())
	_road_frame(tool)
	_expect(simulation.preview_calls == 10, "an unchanged stroke must not preview again")
	tool.pointer.x += 0.01
	_road_frame(tool)
	_expect(simulation.preview_calls == 11, "a sub-5 cm move must preview the exact new stroke")
	tool.adjust_lanes(1, 0)
	_road_frame(tool)
	_expect(simulation.preview_calls == 12, "a lane change must preview the new width")
	# Click between frames: the click previews the freshly resolved cursor and submits it.
	simulation.validation = {"is_valid": false, "invalid_reason": "test_rejection"}
	tool.pointer = Vector3(75.0, 0.0, 0.0)
	tool._handle_click()
	_expect(simulation.last_preview_points[-1] == tool.pointer and simulation.commit_requests == 0, "a refused click previews the clicked stroke and commits nothing")
	_expect(simulation.rejection_reports.size() == 1, "a refused click must report once")
	var report: Dictionary = simulation.rejection_reports[-1] if simulation.rejection_reports.size() > 0 else {}
	_expect(report.get("points") == simulation.last_preview_points and report.get("validation", {}).get("invalid_reason") == "test_rejection", "the report must carry the clicked stroke and its validation")
	tool.reset_main_mesh_chunks()
	_expect(tool._preview.is_empty(), "renderer reset must retire the preview")
	tool.free()
	simulation.free()

func _test_road_preview_swaps_on_installed_commit() -> void:
	var simulation := MockRoadCandidateSimulation.new()
	var tool := _pointer_road_tool(simulation)
	tool.pointer = Vector3(40.0, 0.0, 0.0)
	_road_frame(tool)
	var shown = tool._preview_display
	_expect(tool._commit_segment(tool.pointer), "a valid click queues the previewed stroke")
	_expect(simulation.commit_points == simulation.last_preview_points, "the click submits exactly the previewed stroke")
	simulation.commit_result = {"committed": true, "generation": 5}
	tool._poll_road_commit_result()
	_expect(tool._swap_display == shown and tool._swap_generation == 5, "an accepted commit keeps its preview shown until its chunks are installed")
	tool._road_mesh_generation = 4
	tool._road_mesh_installed(4)
	_expect(tool._swap_display == shown, "an older generation's chunks do not end the preview")
	tool._road_mesh_generation = 5
	tool._road_mesh_installed(5)
	_expect(tool._swap_display == null and shown._instances.is_empty(), "the committed chunks replace the preview in the same call")
	tool.free()
	simulation.free()

func _test_road_preview_context_revision() -> void:
	var simulation := MockRoadCandidateSimulation.new()
	var tool := _pointer_road_tool(simulation)
	tool._info_label = Label.new()
	tool.add_child(tool._info_label)
	simulation.validation = {"is_valid": false, "invalid_reason": "parcel_overlap"}
	tool.pointer = Vector3(50.0, 0.0, 0.0)
	_road_frame(tool)
	_expect(not tool.is_valid and tool._info_label.text.contains("zoning"), "a refused preview must explain the rejection")
	var calls := simulation.preview_calls
	_road_frame(tool)
	_expect(simulation.preview_calls == calls, "a stationary stroke on the same context is previewed once")
	# A publication (commit, undo, edit) moves the context on; the stationary stroke previews again.
	simulation.generation += 1
	simulation.validation = {"is_valid": true}
	_road_frame(tool)
	_expect(simulation.preview_calls == calls + 1 and tool.is_valid, "a new context generation previews a stationary stroke again")
	tool.free()
	simulation.free()

func _test_road_commit_acknowledgment() -> void:
	var simulation := MockRoadCandidateSimulation.new()
	simulation.generation = 1
	simulation.validation = {"is_valid": true}
	var tool := RoadToolScript.new()
	tool.simulation_node = simulation
	tool.active = true
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._info_label = Label.new()
	tool.add_child(tool._info_label)
	tool.current_state = RoadToolScript.State.SETTING_END
	tool.current_path = Path3D.new()
	tool.current_path.curve = Curve3D.new()
	tool.add_child(tool.current_path)
	var path := tool.current_path
	var endpoint := Vector3(40.0, 0.0, 0.0)
	tool._last_world_mouse_pos = endpoint
	tool._has_last_world_mouse_pos = true
	tool._update_preview()
	_expect(simulation.preview_calls == 1, "hover previews the stroke before the click")
	_expect(tool._commit_segment(endpoint), "valid click must queue a placement")
	_expect(simulation.preview_calls == 1, "the click reuses the preview of the same stroke")
	tool._poll_road_commit_result()
	_expect(tool.current_path == path and tool._commit_request_id > 0, "pending placement must retain its stroke")
	_expect(not tool._commit_segment(endpoint) and simulation.commit_requests == 1, "pending placement must block duplicate dispatch")
	tool._update_preview()
	_expect(simulation.preview_calls == 1, "a pending placement must not preview")
	simulation.commit_result = {"committed": false, "detail": "rejected=road_plan_invalid"}
	tool._poll_road_commit_result()
	_expect(tool.current_path == path and tool._commit_request_id == 0, "rejection must keep the stroke editable")
	_expect(tool._info_label.visible and tool._info_label.text.contains("could not be built"), "rejection must explain that no road was built")
	_expect(tool._pending_border_checks.is_empty(), "rejection must retire its border check")
	simulation.commit_result = null
	_expect(not tool._commit_segment(endpoint) and simulation.commit_requests == 1, "the same stroke on the same context keeps its rejection")
	endpoint = Vector3(44.0, 0.0, 0.0)
	_expect(tool._commit_segment(endpoint), "a moved stroke permits another attempt")
	simulation.commit_result = {"committed": true, "generation": 2}
	tool._poll_road_commit_result()
	_expect(tool.current_path != null and tool.current_path != path and tool.current_state == RoadToolScript.State.SETTING_END, "acceptance must start the next straight stroke")
	_expect(tool.start_pos == endpoint and tool._angle_snap() == Vector3.RIGHT, "straight continuation must retain the accepted endpoint and end direction")
	tool._angle_snapped = true
	tool._update_preview_measurement_label(PackedVector3Array([endpoint, endpoint + Vector3.RIGHT * 20.0]))
	_expect(tool._info_label.text.contains("[snapped]"), "angle-snapped straight preview must retain its HUD label")
	tool.cancel_road()
	tool.free()
	simulation.free()

func _test_free_control_curve_chain() -> void:
	var simulation := MockRoadCandidateSimulation.new()
	simulation.generation = 1
	simulation.validation = {"is_valid": true}
	var tool := ClickRoadTool.new()
	tool.simulation_node = simulation
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._info_label = Label.new()
	tool.add_child(tool._info_label)
	tool.draw_mode = 1
	tool.active = true
	# Free terrain, dead ends and through roads all need the same control-point gesture.
	for kind in [0, 1, 2]:
		simulation.start_direction = {"kind": kind, "direction": Vector3.RIGHT if kind != 0 else Vector3.ZERO}
		tool.pointer = Vector3.ZERO
		tool._handle_click()
		_expect(tool.current_state == RoadToolScript.State.SETTING_CONTROL, "every curve start must request a free control point")
		tool.pointer = Vector3(20.0, 0.0, 10.0)
		tool._last_world_mouse_pos = tool.pointer
		var ghost := tool._refresh_preview_curve()
		_expect(ghost[0] == Vector3.ZERO and ghost[-1] == tool.pointer, "control selection previews the original start-to-cursor leg")
		tool._handle_click()
		_expect(tool.current_state == RoadToolScript.State.SETTING_END and tool.control_pos == tool.pointer, "the second click must retain the freely chosen control point")
		_expect(simulation.commit_requests == 0, "a control-point click must not commit a road")
		tool.cancel_road()
	# Both curves bend the same way; their start handles must not inherit the preceding tangent.
	tool.pointer = Vector3.ZERO
	tool._handle_click()
	tool.pointer = Vector3(20.0, 0.0, 10.0)
	tool._handle_click()
	tool.pointer = Vector3(40.0, 0.0, 0.0)
	tool._handle_click()
	var first_points := tool._commit_points
	_expect(tool.current_path.curve.get_point_out(0) == Vector3(20.0, 0.0, 10.0), "curve preview must use the old control-point start handle")
	_expect(tool.current_path.curve.get_point_in(1) == Vector3(-20.0, 0.0, 10.0), "curve preview must use the old control-point end handle")
	_expect(first_points[0] == Vector3.ZERO and first_points[-1] == tool.pointer, "committed samples must retain the preview endpoints")
	tool._poll_road_commit_result()
	_expect(tool.current_state == RoadToolScript.State.SETTING_END, "pending acceptance must not advance the curve chain")
	simulation.commit_result = {"committed": true}
	tool._poll_road_commit_result()
	_expect(tool.current_state == RoadToolScript.State.SETTING_CONTROL and tool.start_pos == Vector3(40.0, 0.0, 0.0), "accepted curve must chain at its endpoint with a new free control point")
	tool.pointer = Vector3(60.0, 0.0, 10.0)
	tool._handle_click()
	tool.pointer = Vector3(80.0, 0.0, 0.0)
	tool._handle_click()
	_expect(tool.current_path.curve.get_point_out(0) == Vector3(20.0, 0.0, 10.0), "a chained curve may bend the same way as the previous curve")
	_expect(tool._commit_points.size() == first_points.size(), "translated identical curves must have matching preview sampling")
	for index in range(mini(first_points.size(), tool._commit_points.size())):
		_expect(tool._commit_points[index].distance_to(first_points[index] + Vector3(40.0, 0.0, 0.0)) < 0.001, "chained preview must reproduce the freely chosen curve geometry")
	tool._poll_road_commit_result()
	var manager = preload("res://scripts/core/input_manager.gd").new()
	manager.road_tool = tool
	manager.current_tool = manager.Tool.ROAD
	var right_click := InputEventMouseButton.new()
	right_click.button_index = MOUSE_BUTTON_RIGHT
	right_click.pressed = true
	manager._handle_mouse(right_click)
	_expect(tool.current_state == RoadToolScript.State.IDLE and tool.current_path == null and tool.active, "right-click must end the curve chain while retaining the road tool")
	tool._begin_stroke(Vector3.ZERO, 1, Vector3.RIGHT)
	manager._handle_escape()
	_expect(tool.current_state == RoadToolScript.State.IDLE and tool.current_path == null and not tool.active, "Escape must end the curve chain and deactivate the tool")
	manager.free()
	# Network-connected endpoints end either drawing mode, for both node and edge targets.
	for mode in [0, 1]:
		for target in [{"snap_node": 7}, {"snap_edge": 9}]:
			tool.draw_mode = mode
			tool._begin_stroke(Vector3.ZERO, 1, Vector3.RIGHT)
			if mode == 1:
				tool.pointer = Vector3(20.0, 0.0, 10.0)
				tool._handle_click()
			tool._sticky_network_snap = target
			_expect(tool._commit_segment(Vector3(40.0, 0.0, 0.0)), "network-connected endpoint must queue a road")
			tool._poll_road_commit_result()
			_expect(tool.current_state == RoadToolScript.State.IDLE and tool.current_path == null, "ending on existing network must end the chain")
	# A road that prompts for an external connection ends either drawing mode.
	simulation.border_node = 3
	for mode in [0, 1]:
		tool.draw_mode = mode
		tool._begin_stroke(Vector3.ZERO, 1, Vector3.RIGHT)
		if mode == 1:
			tool.pointer = Vector3(20.0, 0.0, 10.0)
			tool._handle_click()
		_expect(tool._commit_segment(Vector3(40.0, 0.0, 0.0)), "map-edge endpoint must queue a road")
		tool._poll_road_commit_result()
		_expect(tool.current_state == RoadToolScript.State.IDLE and tool.current_path == null, "reaching the map edge must end the chain")
	simulation.border_node = -1
	# A valid curve can have its control point at the end, leaving a zero analytical end handle.
	tool.draw_mode = 1
	tool._begin_stroke(Vector3.ZERO, 1, Vector3.RIGHT)
	tool.pointer = Vector3(40.0, 0.0, 0.0)
	tool._handle_click()
	tool._handle_click()
	tool._poll_road_commit_result()
	_expect(tool.current_state == RoadToolScript.State.SETTING_CONTROL, "a zero end handle must not stop an accepted curve chain")
	tool.cancel_road()
	tool.free()
	simulation.free()

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
