## Headless contract test for atomic road-chunk hydration and replacement.
extends SceneTree

const NetworkRendererScript := preload("res://scripts/renderers/network_renderer.gd")
const LAYERS := ["earthwork", "curb", "raised_step", "sidewalk", "road", "marking", "concrete"]

class MockSimulation:
	extends Node

	var response: Dictionary = {}
	var generation: int = 0
	var dirty: bool = false
	var full_snapshot_requests: Array[bool] = []

	func is_network_dirty() -> bool:
		return dirty

	func get_road_mesh_data(full_snapshot: bool) -> Dictionary:
		full_snapshot_requests.append(full_snapshot)
		return response

	func get_network_render_generation() -> int:
		return generation

class TestRoadTool:
	extends NetworkTool

	var topology_dirty_count: int = 0

	func mark_network_topology_dirty() -> void:
		topology_dirty_count += 1

	func drain_pending_border_checks() -> void:
		pass

var _failures: int = 0

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
	var simulation := MockSimulation.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	var terrain := Node3D.new()
	terrain.name = "Terrain"
	host.add_child(terrain)
	var water := Node.new()
	water.name = "Water"
	host.add_child(water)
	var zoning_overlay := Node.new()
	zoning_overlay.name = "ZoningOverlay"
	host.add_child(zoning_overlay)
	var road_tool := TestRoadTool.new()
	road_tool.name = "RoadTool"
	host.add_child(road_tool)
	var network_renderer: Node = NetworkRendererScript.new()
	network_renderer.name = "NetworkRenderer"
	host.add_child(network_renderer)

	simulation.generation = 1
	simulation.response = _batch(1, true, 510.0, [])
	_expect(road_tool.needs_main_mesh_hydration(), "new renderer must require hydration")
	network_renderer._process(0.0)
	_expect(simulation.full_snapshot_requests == [true], "clean hydration must request a full snapshot")
	_expect(not road_tool.needs_main_mesh_hydration(), "empty hydration must record generation")
	_expect(road_tool._road_mesh_generation == 1, "empty hydration must record the exact generation")
	_expect(road_tool.topology_dirty_count == 1, "clean hydration must refresh topology visuals")

	simulation.generation = 2
	simulation.response = _batch(2, true, 510.0, [
		_triangle_chunk(0, 0),
		_triangle_chunk(3, -2),
	])
	_expect(road_tool.update_main_mesh(2) == 2, "valid full snapshot must commit")
	_expect(road_tool._road_chunk_instances.size() == 2, "full snapshot must add both chunks")
	var sentinel_key := Vector2i(0, 0)
	var changed_key := Vector2i(3, -2)
	var sentinel: MeshInstance3D = road_tool._road_chunk_instances[sentinel_key]
	var previous_changed: MeshInstance3D = road_tool._road_chunk_instances[changed_key]

	simulation.generation = 3
	simulation.response = _batch(3, false, 510.0, [_triangle_chunk(3, -2, 2.0)])
	_expect(road_tool.update_main_mesh(3) == 3, "incremental upsert must commit")
	_expect(road_tool._road_chunk_instances.size() == 2, "upsert must retain untouched chunks")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "upsert must retain sentinel identity")
	_expect(road_tool._road_chunk_instances[changed_key] != previous_changed, "upsert must replace changed identity")

	simulation.generation = 4
	simulation.response = _batch(4, false, 510.0, [_tombstone(3, -2)])
	_expect(road_tool.update_main_mesh(4) == 4, "tombstone batch must commit")
	_expect(road_tool._road_chunk_instances.size() == 1, "tombstone must remove only its chunk")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "tombstone must retain sentinel identity")
	await process_frame
	var orphan_baseline := int(Performance.get_monitor(Performance.OBJECT_ORPHAN_NODE_COUNT))

	simulation.generation = 6
	simulation.response = _batch(5, false, 510.0, [_triangle_chunk(1, 1)])
	_expect(road_tool.update_main_mesh(5) == -1, "stale staged batch must be rejected")
	_expect(road_tool._road_chunk_instances.size() == 1, "stale batch must retain resident set")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "stale batch must retain sentinel identity")
	_expect(road_tool._road_mesh_generation == 4, "stale batch must retain generation")
	_expect(road_tool._road_chunk_span_m == 510.0, "stale batch must retain span")
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
	simulation.response = _batch(6, false, 510.0, [invalid_chunk])
	_expect(road_tool.update_main_mesh(6) == -1, "non-finite geometry must be rejected")
	_expect(road_tool._road_chunk_instances.size() == 1, "invalid batch must retain resident set")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "invalid batch must retain sentinel identity")
	_expect(road_tool._road_mesh_generation == 4, "invalid batch must retain generation")
	_expect(road_tool._road_chunk_span_m == 510.0, "invalid batch must retain span")

	simulation.generation = 7
	simulation.response = _batch(7, false, 512.0, [])
	_expect(road_tool.update_main_mesh(7) == -1, "incremental span change must be rejected")
	_expect(road_tool._road_chunk_instances.size() == 1, "span rejection must retain resident set")
	_expect(road_tool._road_chunk_instances[sentinel_key] == sentinel, "span rejection must retain sentinel identity")
	_expect(road_tool._road_mesh_generation == 4, "span rejection must retain generation")
	_expect(road_tool._road_chunk_span_m == 510.0, "span rejection must retain committed span")

	host.queue_free()
	await process_frame
	if _failures == 0:
		print("network_tool_chunk_renderer_test: PASS")
	quit(_failures)

func _batch(
	generation: int,
	full_replace: bool,
	chunk_span_m: float,
	chunks: Array
) -> Dictionary:
	return {
		"surface_generation": generation,
		"full_replace": full_replace,
		"chunk_span_m": chunk_span_m,
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
