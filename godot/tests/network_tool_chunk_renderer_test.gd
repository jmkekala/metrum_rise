# SPDX-License-Identifier: GPL-2.0-only

## Headless contract test for atomic road-chunk hydration and replacement, and for the road
## tool's synchronous preview (`ROAD-68-E`): one frame samples, previews and shows a stroke.
## A load's hydration streams its chunks nearest first through the stream node (`RENDER-32-P`).
extends SceneTree

const NetworkRendererScript := preload("res://scripts/renderers/network_renderer.gd")
const TerrainRendererScript := preload("res://scripts/renderers/terrain.gd")
const RoadToolScript := preload("res://scripts/tools/road_tool.gd")
const RoadToolHudScript := preload("res://scripts/ui/road_tool_hud.gd")
const InputManagerScript := preload("res://scripts/core/input_manager.gd")
const WorldMaterialsScript := preload("res://scripts/renderers/world_materials.gd")
const LAYERS := ["skirt", "curb", "raised_step", "sidewalk", "road", "marking", "concrete", "lining"]
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
	# The road hydrations taken (`RENDER-32-P`): their generations, and whether one is held.
	var hydration_requests: Array[int] = []
	var hydration: Dictionary = {}
	var surfaces_added: Dictionary = {}
	# The chunk renderer that builds a hydrated chunk's mesh (the streamed hydration's test).
	var chunk_renderer: Node
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

	# The response's full snapshot, taken for a hydration as Rust's SimulationNode takes it.
	func begin_road_mesh_hydration(expected: int) -> Dictionary:
		hydration_requests.append(expected)
		if not response.get("full_replace", false) or int(response["surface_generation"]) != expected:
			return {}
		hydration = {}
		surfaces_added = {}
		var keys := PackedInt32Array()
		for chunk: Dictionary in response["chunks"]:
			hydration[Vector2i(chunk["chunk_x"], chunk["chunk_z"])] = chunk
			keys.append_array([chunk["chunk_x"], chunk["chunk_z"]])
		return {"surface_generation": expected, "chunk_span_m": response["chunk_span_m"],
			"chunk_origin_x_m": response["chunk_origin_x_m"], "chunk_origin_z_m": response["chunk_origin_z_m"],
			"keys": keys}

	# A hydration's parts as Rust's source runs them (`RENDER-52` part 2): one surface part a
	# chunk, then the swap of a mesh the chunk renderer builds from the payload (its lights in it).
	func set_road_hydration_materials(materials: Array) -> bool:
		return materials.size() == 10

	func prepare_road_mesh_hydration(_keys: PackedInt32Array) -> void:
		pass

	func get_road_mesh_hydration_part_us(x: int, z: int) -> float:
		return 1.0 if hydration.has(Vector2i(x, z)) else -2.0

	func run_road_mesh_hydration_part(x: int, z: int) -> int:
		var key := Vector2i(x, z)
		if not hydration.has(key) or surfaces_added.has(key):
			return 0
		surfaces_added[key] = true
		return 1

	func take_road_mesh_hydration_chunk(x: int, z: int) -> Dictionary:
		var chunk: Dictionary = hydration.get(Vector2i(x, z), {})
		if chunk.is_empty():
			return {}
		var mesh := ArrayMesh.new()
		if not chunk_renderer.fill_chunk_mesh(mesh, chunk):
			return {}
		var data := chunk.duplicate()
		data["mesh"] = mesh
		data["lining_mesh"] = null
		for meta in ["layer_surfaces", "layer_counts", "layer_formats"]:
			data[meta] = mesh.get_meta("road_" + meta)
		return data

	func end_road_mesh_hydration() -> void:
		hydration = {}

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

class TestChunks:
	extends "res://scripts/renderers/road_chunk_renderer.gd"

	var advance_generation_after_stage: bool = false

	func update(expected_generation: int = -1, stage_only: bool = false) -> int:
		var result := super.update(expected_generation, stage_only)
		if stage_only and result == expected_generation and advance_generation_after_stage:
			simulation_node.generation += 1
		return result

# The chunk renderer's host, as the road tool is (`RENDER-52`): the renderer is its child `chunks`.
class TestRoadTool:
	extends Node3D

	var chunks: TestChunks
	var topology_dirty_count: int = 0
	var nodes_dirty_count: int = 0

	func _init() -> void:
		chunks = TestChunks.new()
		chunks.host = self
		add_child(chunks)

	func _ready() -> void:
		chunks.simulation_node = get_node("../SimulationNode")
		chunks.terrain_node = get_node("../Terrain")

	func road_chunks_installed(_generation: int) -> void:
		pass

	func road_chunks_released() -> void:
		pass

	func mark_network_nodes_dirty() -> void:
		nodes_dirty_count += 1

	func mark_network_topology_dirty() -> void:
		topology_dirty_count += 1
		mark_network_nodes_dirty()

# A road tool whose pointer is a scripted world point sent to the session exactly, counting the
# pointer queries and the previews it shows.
class PointerRoadTool:
	extends RoadToolScript
	var pointer: Vector3 = Vector3.ZERO
	var pointer_queries: int = 0
	var draws: int = 0

	func get_world_mouse_pos() -> Vector3:
		pointer_queries += 1
		is_valid = true
		simulation_node.road_tool_pointer_at(pointer)
		return pointer

	func _update_node_visuals() -> void:
		pass

	func _show_preview(points: PackedVector3Array) -> void:
		draws += 1
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
	await _test_native_road_cursor_contract()
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
	_expect(road_tool.chunks.needs_hydration(), "new renderer must require hydration")
	network_renderer._process(0.0)
	_expect(simulation.hydration_requests == [1], "clean hydration must take the full network once")
	_expect(not road_tool.chunks.needs_hydration(), "empty hydration must record generation")
	_expect(road_tool.chunks.generation == 1, "empty hydration must record the exact generation")
	_expect(road_tool.topology_dirty_count == 1, "clean hydration must refresh topology visuals")

	simulation.generation = 2
	simulation.response = _batch(2, true, CHUNK_SPAN_M, [
		_triangle_chunk(0, 0),
		_triangle_chunk(3, -2),
	], CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M)
	_expect(road_tool.chunks.update(2) == 2, "valid full snapshot must commit")
	_expect(road_tool.chunks.chunks.size() == 2, "full snapshot must add both chunks")
	var sentinel_key := Vector2i(0, 0)
	var changed_key := Vector2i(3, -2)
	var sentinel: MeshInstance3D = road_tool.chunks.chunks[sentinel_key]
	var previous_changed: MeshInstance3D = road_tool.chunks.chunks[changed_key]
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
	_expect(road_tool.chunks.origin_x_m == CHUNK_ORIGIN_X_M, "full snapshot must record x origin")
	_expect(road_tool.chunks.origin_z_m == CHUNK_ORIGIN_Z_M, "full snapshot must record z origin")

	simulation.generation = 3
	simulation.response = _batch(
		3,
		false,
		CHUNK_SPAN_M,
		[_triangle_chunk(3, -2, 2.0)],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.chunks.update(3) == 3, "incremental upsert must commit")
	_expect(road_tool.chunks.chunks.size() == 2, "upsert must retain untouched chunks")
	_expect(road_tool.chunks.chunks[sentinel_key] == sentinel, "upsert must retain sentinel identity")
	_expect(road_tool.chunks.chunks[changed_key] != previous_changed, "upsert must replace changed identity")

	simulation.generation = 4
	simulation.response = _batch(
		4,
		false,
		CHUNK_SPAN_M,
		[_tombstone(3, -2)],
		CHUNK_ORIGIN_X_M,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.chunks.update(4) == 4, "tombstone batch must commit")
	_expect(road_tool.chunks.chunks.size() == 1, "tombstone must remove only its chunk")
	_expect(road_tool.chunks.chunks[sentinel_key] == sentinel, "tombstone must retain sentinel identity")
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
	_expect(road_tool.chunks.update(5) == -1, "stale staged batch must be rejected")
	_expect(road_tool.chunks.chunks.size() == 1, "stale batch must retain resident set")
	_expect(road_tool.chunks.chunks[sentinel_key] == sentinel, "stale batch must retain sentinel identity")
	_expect(road_tool.chunks.generation == 4, "stale batch must retain generation")
	_expect(road_tool.chunks.span_m == CHUNK_SPAN_M, "stale batch must retain span")
	_expect(road_tool.chunks.origin_x_m == CHUNK_ORIGIN_X_M, "stale batch must retain x origin")
	_expect(road_tool.chunks.origin_z_m == CHUNK_ORIGIN_Z_M, "stale batch must retain z origin")
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
	_expect(road_tool.chunks.update(6) == -1, "non-finite geometry must be rejected")
	_expect(road_tool.chunks.chunks.size() == 1, "invalid batch must retain resident set")
	_expect(road_tool.chunks.chunks[sentinel_key] == sentinel, "invalid batch must retain sentinel identity")
	_expect(road_tool.chunks.generation == 4, "invalid batch must retain generation")
	_expect(road_tool.chunks.span_m == CHUNK_SPAN_M, "invalid batch must retain span")
	_expect(road_tool.chunks.origin_x_m == CHUNK_ORIGIN_X_M, "invalid batch must retain x origin")
	_expect(road_tool.chunks.origin_z_m == CHUNK_ORIGIN_Z_M, "invalid batch must retain z origin")
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
	_expect(road_tool.chunks.update(7) == -1, "incremental span change must be rejected")
	_expect(road_tool.chunks.chunks.size() == 1, "span rejection must retain resident set")
	_expect(road_tool.chunks.chunks[sentinel_key] == sentinel, "span rejection must retain sentinel identity")
	_expect(road_tool.chunks.generation == 4, "span rejection must retain generation")
	_expect(road_tool.chunks.span_m == CHUNK_SPAN_M, "span rejection must retain committed span")
	_expect(road_tool.chunks.origin_x_m == CHUNK_ORIGIN_X_M, "span rejection must retain x origin")
	_expect(road_tool.chunks.origin_z_m == CHUNK_ORIGIN_Z_M, "span rejection must retain z origin")

	simulation.generation = 8
	simulation.response = _batch(
		8,
		false,
		CHUNK_SPAN_M,
		[_triangle_chunk(5, 5)],
		CHUNK_ORIGIN_X_M + 1.0,
		CHUNK_ORIGIN_Z_M
	)
	_expect(road_tool.chunks.update(8) == -1, "incremental origin change must be rejected")
	_expect(road_tool.chunks.chunks.size() == 1, "origin rejection must retain resident set")
	_expect(road_tool.chunks.chunks[sentinel_key] == sentinel, "origin rejection must retain sentinel identity")
	_expect(road_tool.chunks.generation == 4, "origin rejection must retain generation")
	_expect(road_tool.chunks.span_m == CHUNK_SPAN_M, "origin rejection must retain span")
	_expect(road_tool.chunks.origin_x_m == CHUNK_ORIGIN_X_M, "origin rejection must retain x origin")
	_expect(road_tool.chunks.origin_z_m == CHUNK_ORIGIN_Z_M, "origin rejection must retain z origin")

	var atomic_previous_instance: MeshInstance3D = road_tool.chunks.chunks[sentinel_key]
	var atomic_previous_generation := road_tool.chunks.generation
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
		road_tool.chunks.generation == atomic_previous_generation,
		"an unprepared terrain batch must retain the previous road generation"
	)
	_expect(
		road_tool.chunks.chunks[sentinel_key] == atomic_previous_instance,
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
		road_tool.chunks.generation == atomic_previous_generation,
		"a rejected terrain commit must retain the previous road generation"
	)
	_expect(
		road_tool.chunks.chunks[sentinel_key] == atomic_previous_instance,
		"a rejected terrain commit must retain previous road instances"
	)
	_expect(
		road_tool.chunks._staged.is_empty(),
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
	road_tool.chunks.advance_generation_after_stage = true
	network_renderer._process(0.0)
	_expect(
		road_tool.chunks.generation == atomic_previous_generation,
		"a generation change during road staging must retain the previous visual pair"
	)
	_expect(
		terrain.commit_count == 1,
		"a generation change during road staging must abort before terrain commit"
	)
	_expect(
		road_tool.chunks._staged.is_empty(),
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

	road_tool.chunks.advance_generation_after_stage = false
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
	_expect(road_tool.chunks.generation == 10, "a complete terrain batch must publish its paired road generation")
	_expect(
		road_tool.chunks.chunks[sentinel_key] != atomic_previous_instance,
		"a complete terrain batch must swap the staged road instance"
	)
	_expect(simulation.acknowledged_generations == [10], "the complete pair must reach the exact acknowledgement fence")
	_expect(simulation.dirty, "a rejected exact acknowledgement must retain network dirtiness")
	_expect(water.refresh_count == 0, "water clipping must wait when the exact acknowledgement fails")
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
	_expect(zoning_overlay.dirty_count == 1, "zoning visuals must refresh after exact acknowledgement")
	_expect(
		road_tool.nodes_dirty_count == atomic_previous_nodes_dirty_count + 1,
		"node visuals must invalidate exactly once after exact acknowledgement"
	)

	host.queue_free()
	await process_frame
	await _test_streamed_hydration()
	if _failures == 0:
		print("network_tool_chunk_renderer_test: PASS")
	quit(_failures)

# `RENDER-32-P`: with a stream node, a load's chunks install as its units, nearest first, until
# none is left; the hydration's source is then let go.
func _test_streamed_hydration() -> void:
	var host := Node3D.new()
	root.add_child(host)
	var simulation := MockSimulation.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	for mock: Node in [MockTerrain.new(), MockWater.new(), MockZoningOverlay.new()]:
		mock.name = ["Terrain", "Water", "ZoningOverlay"][host.get_child_count() - 1]
		host.add_child(mock)
	var road_tool := TestRoadTool.new()
	road_tool.name = "RoadTool"
	host.add_child(road_tool)
	var stream := StreamNode.new()
	stream.name = "StreamNode"
	host.add_child(stream)
	var camera := Camera3D.new()
	host.add_child(camera)
	camera.position = Vector3(CHUNK_ORIGIN_X_M + 0.5 * CHUNK_SPAN_M, 40.0, CHUNK_ORIGIN_Z_M + 0.5 * CHUNK_SPAN_M)
	camera.current = true
	var network_renderer: Node = NetworkRendererScript.new()
	network_renderer.name = "NetworkRenderer"
	host.add_child(network_renderer)
	simulation.chunk_renderer = road_tool.chunks
	simulation.generation = 1
	simulation.response = _batch(1, true, CHUNK_SPAN_M,
		[_triangle_chunk(3, 0), _triangle_chunk(0, 0), _triangle_chunk(1, 0)], CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M)
	network_renderer._process(0.0)
	_expect(not road_tool.chunks.needs_hydration(), "a streamed hydration records its generation at once")
	var hydration: Node = network_renderer.get_node("RoadChunkHydration")
	var order: Array[Vector2i] = []
	for frame in 30:
		await process_frame
		for key: Vector2i in road_tool.chunks.chunks:
			if not order.has(key):
				order.append(key)
		if not hydration.is_pending():
			break
	_expect(order.size() == 3, "every chunk of the hydration installs: %s" % [order])
	_expect(order.size() == 0 or order[0] == Vector2i(0, 0), "the chunk under the camera installs first: %s" % [order])
	_expect(not stream.has_road_hydration_pending() and simulation.hydration.is_empty(),
		"the finished hydration lets its source go")
	_expect(road_tool.chunks.generation == 1, "the units keep the hydration's generation")
	host.queue_free()
	await process_frame

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

func _test_road_payload_certificate(road_tool: Node) -> void:
	# The Rust certificate may skip only the per-vertex scan, never the boundary shape checks.
	var key := Vector2i(1, 1)
	var uncertified: MeshInstance3D = road_tool.chunks.build_chunk(
		_triangle_chunk(1, 1), key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M
	)
	var certified_chunk := _triangle_chunk(1, 1)
	certified_chunk["road_mesh_payload_validated"] = true
	var certified: MeshInstance3D = road_tool.chunks.build_chunk(
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
		road_tool.chunks.build_chunk(bad_crossing, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"certified crossing payload must still pass attribute count checks"
	)
	var wrong_type := _triangle_chunk(1, 1)
	wrong_type["road_mesh_payload_validated"] = 1
	_expect(
		road_tool.chunks.build_chunk(wrong_type, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"non-bool road payload certificate must be rejected"
	)
	var certified_bad_count := _triangle_chunk(1, 1)
	certified_bad_count["road_mesh_payload_validated"] = true
	certified_bad_count["road_normals"] = PackedVector3Array([Vector3.UP])
	_expect(
		road_tool.chunks.build_chunk(certified_bad_count, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"certified payload must still pass attribute count checks"
	)
	var certified_bad_type := _triangle_chunk(1, 1)
	certified_bad_type["road_mesh_payload_validated"] = true
	certified_bad_type["road_uvs"] = PackedVector3Array()
	_expect(
		road_tool.chunks.build_chunk(certified_bad_type, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
		"certified payload must still pass Variant type checks"
	)
	var uncertified_nan := _triangle_chunk(1, 1)
	uncertified_nan["road_mesh_payload_validated"] = false
	uncertified_nan["road_colors"] = PackedColorArray([Color(NAN, 0.0, 0.0), Color.WHITE, Color.WHITE])
	_expect(
		road_tool.chunks.build_chunk(uncertified_nan, key, CHUNK_SPAN_M, CHUNK_ORIGIN_X_M, CHUNK_ORIGIN_Z_M) == null,
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
	# The session's cursor (`ROAD-112`): a vertical ray onto free ground moves it continuously.
	for index in range(5):
		var x := 30.0 + index * 0.1
		var cursor: Variant = simulation.road_tool_pointer(Vector3(x, 100.0, 20.0), Vector3.DOWN, false)
		_expect(cursor is Vector3 and absf(cursor.x - x) < 0.0001 and absf(cursor.z - 20.0) < 0.0001, "free cursor motion must remain continuous through the session")
		_expect(String(simulation.road_tool_frame().get("snap", "")) == "free", "free terrain takes no snap")
	await _test_native_road_preview_contract(simulation)
	var points := PackedVector3Array([Vector3(-60.0, 0.0, 32.0), Vector3(60.0, 0.0, 32.0)])
	var request := simulation.add_road_with_snap(points, 1, 1, true)
	var accepted: Dictionary = await _await_road_commit(simulation, request)
	_expect(accepted.get("committed", false) and simulation.get_road_benchmark_state()["live_edges"] == 1, "native acknowledgment must reflect an accepted road")
	# Near the committed road's dead end the session's cursor takes its node.
	var end_cursor: Variant = simulation.road_tool_pointer(Vector3(60.5, 100.0, 32.5), Vector3.DOWN, false)
	_expect(end_cursor is Vector3 and Vector2(end_cursor.x, end_cursor.z).distance_to(Vector2(60.0, 32.0)) < 0.01, "the cursor snaps onto the dead end")
	_expect(String(simulation.road_tool_frame().get("snap", "")) == "node", "the dead end's node takes the cursor")
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
	_expect(modes.draw_mode == 1 and ui.curve_btn.button_pressed and not ui.straight_btn.button_pressed, "the Curve button picks the simple curve")
	_expect(not ui.zoning_grid_snap_check.disabled and modes._zoning_snap_enabled(), "the session snaps to the zoning grid in every mode")
	ui.continuous_check.button_pressed = true
	_expect(modes.continuous, "the Continuous checkbox sets the tool's continuous option")
	ui._set_draw_mode(2)
	_expect(modes.draw_mode == 2 and ui.s_curve_btn.button_pressed and not ui.curve_btn.button_pressed, "the S-curve button picks the complex curve")
	ui._set_draw_mode(3)
	_expect(modes.draw_mode == 3 and ui.freeform_btn.button_pressed and not ui.s_curve_btn.button_pressed, "the Freeform button picks freeform")
	ui._set_draw_mode(0)
	_expect(modes.draw_mode == 0, "the Straight button picks straight mode")
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
	var shift := InputEventKey.new()
	shift.keycode = KEY_SHIFT
	shift.pressed = true
	shift.shift_pressed = true
	Input.parse_input_event(shift)
	Input.flush_buffered_events()
	_expect(not modes._zoning_snap_enabled() and modes._snap_to_roads_enabled(), "Shift must not enable zoning snap or disable road connections")
	modes.zoning_grid_snap_enabled = true
	_expect(modes._zoning_snap_enabled(), "checkbox controls zoning snap while Shift is held")
	shift = InputEventKey.new()
	shift.keycode = KEY_SHIFT
	Input.parse_input_event(shift)
	Input.flush_buffered_events()
	_expect(modes._zoning_snap_enabled(), "releasing Shift must not change the checkbox setting")
	modes.cancel_road()
	modes.free()
	var invalid := PackedVector3Array([Vector3.ZERO])
	var abandoned := simulation.add_road_with_snap(invalid, 1, 1, true)
	request = simulation.add_road_with_snap(invalid, 1, 1, true)
	var superseded: Dictionary = simulation.get_road_commit_result(abandoned)
	_expect(not superseded["committed"] and superseded["detail"] == "road_request_replaced", "superseded requests must not wait forever")
	var rejected: Dictionary = await _await_road_commit(simulation, request)
	_expect(not rejected.get("committed", true) and simulation.get_road_benchmark_state()["live_edges"] == 1, "native rejection must acknowledge that no topology was added")
	await _test_session_strokes(simulation)
	await _test_s_curve_and_freeform(simulation)
	_test_snapping_options_row(simulation)
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

func _test_native_road_preview_contract(simulation: SimulationNode) -> void:
	var tool := RoadToolScript.new()
	tool.name = "RoadTool"
	tool.simulation_node = simulation
	tool.make_chunks()
	var generation := simulation.get_network_render_generation()
	_expect(tool.chunks.update(generation) == generation, "isolated preview must hydrate the empty committed chunk grid")
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._road_preview_material = WorldMaterialsScript.road_preview_material()
	tool.blueprint_mesh.material_override = tool._road_preview_material
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
		_expect(not chunks.is_empty() and chunks.size() * 2 <= PackedInt32Array(preview.get("changed_chunks", PackedInt32Array())).size(), "only changed chunks carry an update")
		for chunk in chunks:
			_expect(chunk.get("road_mesh_payload_validated", false), "preview overlays are sealed as committed chunks are")
			_expect(chunk.get("hidden") is PackedInt32Array and chunk.get("hidden_positions") is PackedVector3Array, "every preview chunk names the resident runs it replaces")
	# The check waits for the core for its corridor clearance, so it never answers pending.
	var hover: Dictionary = simulation.validate_road_candidate_with_snap(points, 1, 1, false)
	_expect(hover.get("is_valid", false), "hover validation keeps its prepared profile: %s" % hover.get("invalid_reason", ""))
	tool.fwd_lanes = 2
	tool.bkw_lanes = 1
	tool.active = true
	tool._node_visuals_dirty = false
	tool._node_visuals_visible = true
	tool._begin_stroke(points[0])
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
	_expect(not tool._preview_display.show(tool.chunks, malformed, tool._preview_sequence + 1), "incomplete mesh batches must fail atomically")
	_expect(tool._preview_display._instances == instances, "failed staging must preserve the complete previous preview")
	var rejected: Dictionary = tool._preview.duplicate(false)
	rejected["is_valid"] = false
	rejected["invalid_reason"] = "terrain_clip_missing_outer_boundary_owner"
	rejected["sequence"] = tool._preview_sequence + 1
	tool._preview = rejected
	tool._show_preview(points)
	_expect(tool._preview_display.sequence == 0 and tool._preview_display._instances.is_empty(), "a rejected stroke shows no road")
	var notice: String = RoadWording.reason_text(rejected, RoadWording.default_refusal())
	_expect(notice.contains("terrain boundary"), "terrain rejection must retain an explanatory notice: '%s'" % notice)
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

# A road tool on the real node with a scripted pointer, active and with a HUD label, outside the
# tree (the frames are driven by hand).
func _session_road_tool(simulation: SimulationNode) -> PointerRoadTool:
	var tool := PointerRoadTool.new()
	tool.name = "RoadTool"
	tool.simulation_node = simulation
	tool.make_chunks()
	tool.chunks.update(simulation.get_network_render_generation())
	tool.attach_hud(RoadToolHudScript.new())
	tool._node_visuals_dirty = false
	tool._node_visuals_visible = true
	tool.active = true
	return tool

func _road_frame(tool: PointerRoadTool) -> void:
	tool._process(1.0 / 60.0)

# Waits until the tool's sent stroke is acknowledged (the frame polls it).
func _await_ack(tool: PointerRoadTool) -> void:
	var deadline := Time.get_ticks_msec() + 10000
	while tool._commit_request_id > 0 and Time.get_ticks_msec() < deadline:
		await process_frame
		_road_frame(tool)
	_expect(tool._commit_request_id == 0, "the sent stroke must be acknowledged")

# The bridge on the real node (`ROAD-112` rounds C, D): every frame previews its own pose once,
# a click submits that preview and the chain continues; the curve modes, their gizmos and the
# HUD; right-click and Escape end the chain.
func _test_session_strokes(simulation: SimulationNode) -> void:
	var tool := _session_road_tool(simulation)
	# Straight: input bursts coalesce into one preview per moved frame, none for a still one.
	tool._begin_stroke(Vector3(-60.0, 0.0, -60.0))
	for index in range(32):
		tool.pointer = Vector3(-20.0 + index, 0.0, -60.0)
		tool._queue_preview_update()
	var shown := tool._preview_sequence
	_road_frame(tool)
	_expect(tool.pointer_queries == 1 and tool._preview_sequence == shown + 1 and tool.draws == 1, "32 queued updates coalesce into one synchronous preview")
	_expect(tool._session_points[-1] == tool.pointer and tool.stroke_preview_current(), "the frame previews this frame's pointer")
	for index in range(4):
		tool.pointer.x += 0.25
		_road_frame(tool)
	_expect(tool._preview_sequence == shown + 5, "every moved pose is previewed in its own frame")
	_road_frame(tool)
	_expect(tool._preview_sequence == shown + 5, "an unchanged stroke is not previewed again")
	tool.set_lanes(2, 1)
	_road_frame(tool)
	_expect(tool._preview_sequence == shown + 6, "a lane change previews the new width")
	tool.set_lanes(1, 1)
	_road_frame(tool)
	var gizmo_kinds: PackedInt32Array = tool.gizmos._handle_kinds
	# The free start's height grip draws as a handle (5) on a stem in the guide look (rounds I, V2).
	_expect(gizmo_kinds == PackedInt32Array([0, 3, 5]), "a straight stroke shows its start, live end and the start's height grip: %s" % gizmo_kinds)
	_expect(tool.hud.text().contains("$") and tool.hud.text().contains(" m") and not tool.hud.text().contains("/day"), "the HUD's chips show the stroke's numbers, no upkeep: '%s'" % tool.hud.text())
	# The click submits the standing preview; a second click waits for the acknowledgement.
	var end := tool.pointer
	_expect(tool._commit_segment(end), "a valid click queues the previewed stroke")
	_expect(not tool._commit_segment(end) and tool.is_drawing(), "a pending placement blocks another")
	await _await_ack(tool)
	_road_frame(tool)
	var frame: Dictionary = simulation.road_tool_frame()
	_expect(int(frame["phase"]) == 1 and int(frame["placed"]) == 1, "an accepted stroke starts the next at its end")
	_expect(PackedInt32Array(frame["gizmo_handle_kinds"])[0] == 2, "the chained stroke's start is the chain start")
	_expect(tool.chunks.holds.held_preview() >= 0 or tool._swap_display == null, "the committed preview is held or already swapped")
	# Right-click ends the chain and keeps the tool; Escape ends the tool.
	var manager = preload("res://scripts/core/input_manager.gd").new()
	manager.road_tool = tool
	manager.current_tool = manager.Tool.ROAD
	var right_click := InputEventMouseButton.new()
	right_click.button_index = MOUSE_BUTTON_RIGHT
	right_click.pressed = true
	manager._handle_mouse(right_click)
	_expect(not tool.is_drawing() and tool.active, "right-click ends the chain and keeps the road tool")
	# Simple curve (U1): start, a free control point (no commit), the end.
	tool.draw_mode = RoadToolScript.DrawMode.SIMPLE_CURVE
	tool._begin_stroke(Vector3(-100.0, 0.0, 60.0))
	var requests := tool._commit_request_id
	_expect(not tool._commit_segment(Vector3(-60.0, 0.0, 60.0)) and tool._commit_request_id == requests, "a control-point click commits nothing")
	_expect(int(simulation.road_tool_frame()["placed"]) == 2, "the control point is placed")
	tool.pointer = Vector3(-60.0, 0.0, 100.0)
	_road_frame(tool)
	var points := tool._session_points
	_expect(points.size() > 3 and points[0] == Vector3(-100.0, 0.0, 60.0) and points[-1] == tool.pointer, "the curve runs from the start to the cursor through the arc")
	_expect(tool.gizmos._handle_kinds == PackedInt32Array([0, 1, 3, 5]) and tool.gizmos._line_classes.slice(0, 2) == PackedInt32Array([3, 0]) and tool.gizmos._line_classes.count(3) == 2 and tool.gizmos._line_classes.has(4), "a curve shows its start, control point, live end, tangent legs, centreline, zoning guides and the start's height grip on its stem")
	_expect(absf(float(tool._session_hud["radius_m"]) - 40.0) < 0.001, "the HUD's radius is the fillet's: %s" % tool._session_hud.get("radius_m"))
	_expect(tool.hud.text().contains("R 40.0 m"), "the HUD names the radius: '%s'" % tool.hud.text())
	_expect(tool._commit_segment(tool.pointer), "the end click sends the curve")
	await _await_ack(tool)
	_road_frame(tool)
	_expect(int(simulation.road_tool_frame()["placed"]) == 1, "an accepted curve chains at its end with a free control point next")
	# A curve tighter than the road type allows is refused before any preview.
	tool.cancel_road()
	tool._begin_stroke(Vector3(-100.0, 0.0, -100.0))
	tool._commit_segment(Vector3(-96.0, 0.0, -100.0))
	tool.pointer = Vector3(-96.0, 0.0, -96.0)
	_road_frame(tool)
	frame = simulation.road_tool_frame()
	_expect(String(frame["refusal"]) == "road_tight_curve" and PackedVector3Array(frame["points"]).is_empty(), "a 4 m radius is refused as too tight")
	_expect(tool.hud.text().contains("Curve too tight"), "the HUD words the tight curve: '%s'" % tool.hud.text())
	_expect(not tool._commit_segment(tool.pointer), "a refused draft is not sent")
	manager._handle_escape()
	_expect(not tool.is_drawing() and not tool.active, "Escape ends the stroke and the tool")
	tool.draw_mode = RoadToolScript.DrawMode.STRAIGHT
	manager.free()
	tool.free()

# The gizmo handles of `kind` (`road_tool_gizmos.gd`'s last upload), in frame order.
func _gizmo_handles(tool: PointerRoadTool, kind: int) -> PackedVector3Array:
	var found := PackedVector3Array()
	for index in tool.gizmos._handles.size():
		if tool.gizmos._handle_kinds[index] == kind:
			found.append(tool.gizmos._handles[index])
	return found

# Whether the handle points `a` and `b` lie on one line through `node`, on opposite sides.
func _collinear(a: Vector3, node: Vector3, b: Vector3) -> bool:
	var u := Vector2(a.x - node.x, a.z - node.z).normalized()
	var v := Vector2(b.x - node.x, b.z - node.z).normalized()
	return absf(u.cross(v)) < 1.0e-4 and u.dot(v) < 0.0

# Round N (`ROAD-112`) on the real node: an S-curve drawn (round SC: previewed as road from the
# first click with a hint chip a step; its gizmos and both radii) and committed; a freeform
# draft placed, taken into its edit phase with Enter, edited (a handle drag keeps its joint
# collinear, Ctrl breaks one) and built with Enter; Escape on a draft in its edit phase leaves
# nothing.
func _test_s_curve_and_freeform(simulation: SimulationNode) -> void:
	var tool := _session_road_tool(simulation)
	var manager = preload("res://scripts/core/input_manager.gd").new()
	manager.road_tool = tool
	manager.current_tool = manager.Tool.ROAD
	tool.draw_mode = RoadToolScript.DrawMode.COMPLEX_CURVE
	tool._begin_stroke(Vector3(0.0, 0.0, -30.0))
	# Round SC: the road is previewed from the first click, with a hint chip a step.
	tool.pointer = Vector3(15.0, 0.0, -30.0)
	_road_frame(tool)
	_expect(tool.stroke_preview_current() and tool._session_points == PackedVector3Array([Vector3(0.0, 0.0, -30.0), tool.pointer]), "after the first click the leg is previewed as road: %s" % tool._session_points)
	_expect(tool.hud.text().contains("Click to set the start direction"), "the first step's hint: %s" % tool.hud.text())
	_expect(not tool._commit_segment(Vector3(15.0, 0.0, -30.0)), "the start aim point commits nothing")
	tool.pointer = Vector3(45.0, 0.0, -10.0)
	_road_frame(tool)
	_expect(tool.stroke_preview_current() and tool._session_points.size() > 3, "after the second click the curve through the start aim is previewed")
	_expect(tool.hud.text().contains("Click to set the end direction") and not tool.hud.text().contains("start direction"), "the second step's hint: %s" % tool.hud.text())
	_expect(3 in tool.gizmos._line_classes and 0 in tool.gizmos._line_classes, "the aim legs stay tangent gizmos over the draft's centreline")
	_expect(not tool._commit_segment(tool.pointer), "the end aim point commits nothing")
	tool.pointer = Vector3(60.0, 0.0, -10.0)
	_road_frame(tool)
	_expect(tool.hud.text().contains("Click to place the end"), "the last step's hint: %s" % tool.hud.text())
	var points := tool._session_points
	_expect(points.size() > 3 and points[0] == Vector3(0.0, 0.0, -30.0) and points[-1] == tool.pointer, "the S-curve runs from the start to the cursor")
	_expect(_gizmo_handles(tool, 4).size() == 1 and _gizmo_handles(tool, 5).size() == 3, "the S-curve shows its joint, both arcs' control points and the start's height grip")
	var radius := float(tool._session_hud.get("radius_m", -1.0))
	var other := float(tool._session_hud.get("other_radius_m", -1.0))
	_expect(radius > 0.0 and other >= radius, "the HUD gives both radii, the tighter first: %s, %s" % [radius, other])
	var lengths := PackedFloat64Array(simulation.road_tool_frame()["piece_lengths_m"])
	_expect(lengths.size() == 2 and absf(lengths[0] + lengths[1] - float(tool._session_hud["length_m"])) < 0.5, "the frame gives each arc's length: %s" % lengths)
	var edges := int(simulation.get_road_benchmark_state()["live_edges"])
	_expect(tool._commit_segment(tool.pointer), "the end click sends the S-curve")
	await _await_ack(tool)
	_expect(int(simulation.get_road_benchmark_state()["live_edges"]) > edges, "the S-curve is built")
	tool.cancel_road()

	# Freeform: three clicks, the live fourth node, a click on it, Enter.
	tool.draw_mode = RoadToolScript.DrawMode.FREEFORM
	tool._begin_stroke(Vector3(-115.0, 0.0, -35.0))
	for node in [Vector3(-80.0, 0.0, -20.0), Vector3(-45.0, 0.0, -35.0)]:
		_expect(not tool._commit_segment(node), "a freeform node commits nothing")
	tool.pointer = Vector3(-10.0, 0.0, -20.0)
	_road_frame(tool)
	_expect(tool._session_points.size() > 4 and tool._session_points[-1] == tool.pointer and tool.stroke_preview_current(), "the live node is previewed as the draft's end")
	_expect(not tool._commit_segment(tool.pointer), "the fourth node commits nothing")
	manager._handle_confirm()
	_road_frame(tool)
	var frame: Dictionary = simulation.road_tool_frame()
	_expect(int(frame["phase"]) == 3 and int(frame["placed"]) == 4, "Enter starts the edit phase: %s" % frame["phase"])
	var handles := _gizmo_handles(tool, 5)
	# Six Bezier handles, then the two ends' height grips (round I).
	_expect(handles.size() == 8, "every inner node has two handles, each end one and a height grip: %d" % handles.size())
	# Drag node 1's outward handle; its inward handle turns with it.
	var shown := tool._preview_sequence
	tool.pointer = handles[2]
	_road_frame(tool)
	_expect(not tool._session_press(), "a press on a handle grabs it")
	tool.pointer = handles[2] + Vector3(6.0, 0.0, 5.0)
	_road_frame(tool)
	var release := InputEventMouseButton.new()
	release.button_index = MOUSE_BUTTON_LEFT
	release.pressed = false
	tool._unhandled_input(release)
	_road_frame(tool)
	var joints := _gizmo_handles(tool, 4)
	handles = _gizmo_handles(tool, 5)
	_expect(tool._preview_sequence > shown, "the edited draft is previewed")
	_expect(handles[2].distance_to(tool.pointer) < 0.01, "the handle follows the pointer")
	_expect(_collinear(handles[1], joints[0], handles[2]), "the joint stays collinear")
	# Ctrl breaks node 2's joint: its inward handle moves alone.
	tool.pointer = handles[3]
	_road_frame(tool)
	tool._session_press()
	tool.pointer = handles[3] + Vector3(-4.0, 0.0, -6.0)
	simulation.road_tool_pointer_at(tool.pointer, true)
	_road_frame(tool)
	tool._unhandled_input(release)
	_road_frame(tool)
	var broken := _gizmo_handles(tool, 5)
	_expect(broken[4] == handles[4] and not _collinear(broken[3], joints[1], broken[4]), "Ctrl breaks the joint")
	# Enter builds it.
	edges = int(simulation.get_road_benchmark_state()["live_edges"])
	manager._handle_confirm()
	_expect(tool._commit_request_id > 0, "Enter sends the edited draft")
	await _await_ack(tool)
	_expect(int(simulation.get_road_benchmark_state()["live_edges"]) > edges, "the freeform road is built")
	_road_frame(tool)
	frame = simulation.road_tool_frame()
	_expect(int(frame["phase"]) == 1 and int(frame["placed"]) == 1, "an accepted freeform road chains at its end")
	# Escape on a draft in its edit phase leaves nothing.
	tool.cancel_road()
	tool._begin_stroke(Vector3(-90.0, 0.0, 10.0))
	tool._commit_segment(Vector3(-60.0, 0.0, 20.0))
	manager._handle_confirm()
	_road_frame(tool)
	_expect(int(simulation.road_tool_frame()["phase"]) == 3, "a two-node draft takes Enter into its edit phase")
	edges = int(simulation.get_road_benchmark_state()["live_edges"])
	manager._handle_escape()
	_road_frame(tool)
	frame = simulation.road_tool_frame()
	_expect(int(frame["phase"]) == 0 and PackedVector3Array(frame["points"]).is_empty() and PackedVector3Array(frame["gizmo_handles"]).is_empty(), "Escape drops the draft")
	_expect(not tool.gizmos.visible and tool._preview.is_empty() and int(simulation.get_road_benchmark_state()["live_edges"]) == edges, "Escape leaves nothing")
	await _test_freeform_chips(simulation, tool)
	tool.draw_mode = RoadToolScript.DrawMode.STRAIGHT
	manager.free()
	tool.free()

# Round V item 7 (on open ground east of the curve tests): an open freeform draft shows its hint; in the edit phase the Build chip builds
# it and the Cancel chip drops it, each click taken by the GUI (it never reaches the world). The
# HUD is moved into the tree under a camera looking down so its chips are placed and clickable.
func _test_freeform_chips(simulation: SimulationNode, tool: PointerRoadTool) -> void:
	var camera := Camera3D.new()
	root.add_child(camera)
	camera.position = Vector3(60.0, 260.0, 90.0)
	camera.look_at(Vector3(60.0, 0.0, 89.99))
	camera.current = true
	var hud = tool.hud
	hud.reparent(root)
	hud.simulation_node = simulation
	tool.cancel_road()
	tool.draw_mode = RoadToolScript.DrawMode.FREEFORM
	tool._begin_stroke(Vector3(30.0, 0.0, 85.0))
	tool.pointer = Vector3(60.0, 0.0, 95.0)
	_road_frame(tool)
	_expect(hud.text().contains("Click to add a node"), "a placing draft hints how to go on: '%s'" % hud.text())
	for action in ["build", "cancel"]:
		tool.cancel_road()
		tool._begin_stroke(Vector3(30.0, 0.0, 85.0))
		tool._commit_segment(Vector3(60.0, 0.0, 95.0))
		tool._commit_segment(Vector3(90.0, 0.0, 85.0))
		tool.confirm_road()
		_road_frame(tool)
		_expect(int(simulation.road_tool_frame()["phase"]) == 3, "the draft is in its edit phase")
		_expect(hud.text().contains("Enter or click to build") and hud.text().contains("Build") and hud.text().contains("Cancel"), "the edit phase hints and offers Build and Cancel: '%s'" % hud.text())
		hud.place(camera)
		var rect: Rect2 = hud.chip_rect(hud.Kind.BUILD if action == "build" else hud.Kind.CANCEL)
		_expect(rect.has_area(), "the %s chip is placed" % action)
		var edges := int(simulation.get_road_benchmark_state()["live_edges"])
		var press := InputEventMouseButton.new()
		press.button_index = MOUSE_BUTTON_LEFT
		press.pressed = true
		press.position = rect.get_center()
		press.global_position = press.position
		root.push_input(press)
		_expect(root.is_input_handled(), "the %s chip's click is the GUI's" % action)
		var release := press.duplicate()
		release.pressed = false
		root.push_input(release)
		if action == "build":
			_expect(tool._commit_request_id > 0, "the Build chip sends the draft")
			await _await_ack(tool)
			_expect(int(simulation.get_road_benchmark_state()["live_edges"]) > edges, "the Build chip builds the freeform road")
		else:
			_road_frame(tool)
			var frame: Dictionary = simulation.road_tool_frame()
			_expect(int(frame["phase"]) == 0 and PackedVector3Array(frame["points"]).is_empty(), "the Cancel chip drops the draft")
			_expect(int(simulation.get_road_benchmark_state()["live_edges"]) == edges and hud.text().is_empty(), "the Cancel chip leaves nothing")
	tool.cancel_road()
	hud.reparent(tool)
	camera.free()

# The options row's Snapping and Steps rows (`ROAD-112` round E) on the real node: each control
# reaches Rust's session under its option name (read back through what takes a camera ray's
# cursor beside the first test road's dead end at (60, 32) and on open ground), and a tool
# switched off and on again re-sends them over whatever set Rust's options meanwhile (the debug
# replay).
func _test_snapping_options_row(simulation: SimulationNode) -> void:
	var tool := _session_road_tool(simulation)
	var manager = preload("res://scripts/core/input_manager.gd").new()
	var vegetation = preload("res://scripts/tools/vegetation_tool.gd").new()
	manager.vegetation_tool = vegetation
	var ui = preload("res://scripts/ui/main_ui.gd").new()
	ui.input_manager = manager
	ui.simulation_node = simulation
	ui.road_tool = tool
	ui._build_ui()
	var snap_at := func(x: float, z: float) -> String:
		simulation.road_tool_pointer(Vector3(x, 100.0, z), Vector3.DOWN, false)
		return String(simulation.road_tool_frame().get("snap", ""))
	_expect(ui.network_snap_check.button_pressed and ui.guide_lines_check.button_pressed and ui.angle_step_option.selected == 0 and not ui.length_step_check.button_pressed, "the row starts at the session's defaults")
	_expect(snap_at.call(60.5, 32.5) == "node", "network snap on: the dead end's node")
	# 20 m out along the extension: clear of the node's reach and the perpendicular's corner.
	var extension: String = snap_at.call(80.0, 32.5)
	_expect(extension == "guide", "the dead end's extension: %s" % extension)
	ui.network_snap_check.button_pressed = false
	_expect(not tool.network_snap_enabled and snap_at.call(60.5, 32.5) != "node", "network snap off: the node does not take the cursor")
	ui.guide_lines_check.button_pressed = false
	_expect(not tool.guide_lines_enabled and snap_at.call(80.0, 32.5) == "free" and snap_at.call(60.5, 32.5) == "free", "guide lines off: the cursor is free")
	# The angle lock from a start on open ground: 6.8 degrees off east locks to east at 15.
	simulation.road_tool_pointer(Vector3(40.0, 100.0, -100.0), Vector3.DOWN, false)
	simulation.road_tool_event("press")
	ui.angle_step_option.select(1)
	ui.angle_step_option.item_selected.emit(1)
	_expect(tool.angle_step_deg == 15.0 and snap_at.call(75.0, -95.8) == "angle", "a 15 degree step locks the stroke's direction")
	ui.angle_step_option.select(0)
	ui.angle_step_option.item_selected.emit(0)
	_expect(tool.angle_step_deg == 0.0 and snap_at.call(75.0, -95.8) == "free", "Free drops the angle lock")
	# The length step rounds a free end's distance from the start (32.2 m to 30 m at 10 m).
	ui.length_step_check.button_pressed = true
	var stepped: Variant = simulation.road_tool_pointer(Vector3(65.3, 100.0, -80.1), Vector3.DOWN, false)
	_expect(tool.length_step_m == ui.length_step_spin.value and ui.length_step_spin.value == 10.0, "the length step defaults to the zoning cell")
	_expect(stepped is Vector3 and absf(Vector2(stepped.x - 40.0, stepped.z + 100.0).length() - 30.0) < 0.001, "the length step rounds the stroke: %s" % [stepped])
	# A scripted value (like `select`) emits nothing outside the tree; a player's edit does.
	ui.length_step_spin.value = 5.0
	ui.length_step_spin.value_changed.emit(5.0)
	stepped = simulation.road_tool_pointer(Vector3(65.3, 100.0, -80.1), Vector3.DOWN, false)
	_expect(stepped is Vector3 and absf(Vector2(stepped.x - 40.0, stepped.z + 100.0).length() - 30.0) < 0.001 and tool.length_step_m == 5.0, "a 5 m step rounds 32.2 m to 30 m: %s, step %s" % [stepped, tool.length_step_m])
	ui.length_step_check.button_pressed = false
	stepped = simulation.road_tool_pointer(Vector3(65.3, 100.0, -80.1), Vector3.DOWN, false)
	_expect(tool.length_step_m == 0.0 and stepped is Vector3 and absf(stepped.x - 65.3) < 0.001, "the length step off leaves the cursor")
	simulation.road_tool_event("cancel")
	# Something sets Rust's options directly; the tool's next activation re-sends the row's.
	simulation.road_tool_set_option("network_snap", true)
	simulation.road_tool_set_option("guide_lines", true)
	_expect(snap_at.call(60.5, 32.5) == "node", "Rust's options were changed behind the tool")
	tool.active = false
	_road_frame(tool)
	tool.active = true
	_road_frame(tool)
	_expect(snap_at.call(60.5, 32.5) == "free", "a re-activated tool re-sends the row's options")
	ui.network_snap_check.button_pressed = true
	ui.guide_lines_check.button_pressed = true
	_expect(snap_at.call(60.5, 32.5) == "node", "both back on: the node again")
	ui.free()
	manager.free()
	vegetation.free()
	tool.free()

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

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
