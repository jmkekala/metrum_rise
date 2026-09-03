## Deterministic headless benchmark for committed-road chunk validation and ArrayMesh upload.
extends SceneTree

const LAYERS: Array[String] = [
	"earthwork",
	"curb",
	"raised_step",
	"sidewalk",
	"road",
	"marking",
	"concrete",
]
const LARGE_LAYER_VERTEX_COUNTS := {
	"earthwork": 1200,
	"curb": 1200,
	"raised_step": 0,
	"sidewalk": 1800,
	"road": 3000,
	"marking": 990,
	"concrete": 0,
}
const RESIDENT_CHUNK_COUNTS: Array[int] = [4, 64, 256, 1024, 4096]
const AFFECTED_CHUNK_COUNTS: Array[int] = [0, 1, 4, 16]
const CHUNK_SPAN_M := 510.0
const CHUNK_ORIGIN_X_M := -10240.0
const CHUNK_ORIGIN_Z_M := 5120.0
const WARMUP_SAMPLES := 20
const MEASURED_SAMPLES := 101
const LARGE_VERTICES_PER_CHUNK := 8190

class MockSimulation:
	extends Node

	var response: Dictionary = {}
	var generation: int = 0

	func is_network_dirty() -> bool:
		return false

	func get_road_mesh_data(_full_snapshot: bool) -> Dictionary:
		return response

	func get_network_render_generation() -> int:
		return generation

class BenchmarkRoadTool:
	extends NetworkTool

	func mark_network_topology_dirty() -> void:
		pass

	func drain_pending_border_checks() -> void:
		pass

var _failures: int = 0
var _large_layer_arrays: Dictionary
var _small_layer_arrays: Dictionary

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	_large_layer_arrays = _build_layer_arrays(LARGE_LAYER_VERTEX_COUNTS)
	_small_layer_arrays = _build_layer_arrays({"road": 3})
	assert(_layer_vertex_count(_large_layer_arrays) == LARGE_VERTICES_PER_CHUNK)

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
	var road_tool := BenchmarkRoadTool.new()
	road_tool.name = "RoadTool"
	host.add_child(road_tool)
	await process_frame
	road_tool.set_process(false)
	road_tool.set_physics_process(false)
	road_tool.road_mesh_root.visible = false
	RenderingServer.force_sync()

	print(
		"ROAD_CHUNK_GODOT_BEGIN warmup=%d samples=%d changed_vertices_per_chunk=%d"
		% [WARMUP_SAMPLES, MEASURED_SAMPLES, LARGE_VERTICES_PER_CHUNK]
	)
	for resident_chunks in RESIDENT_CHUNK_COUNTS:
		await _run_case("resident", road_tool, simulation, resident_chunks, 1)
	for changed_chunks in AFFECTED_CHUNK_COUNTS:
		await _run_case("affected", road_tool, simulation, 1024, changed_chunks)
	print("ROAD_CHUNK_GODOT_END failures=%d" % _failures)

	host.queue_free()
	await process_frame
	quit(_failures)

func _run_case(
	kind: String,
	road_tool: BenchmarkRoadTool,
	simulation: MockSimulation,
	resident_chunks: int,
	changed_chunks: int
) -> void:
	road_tool.reset_main_mesh_chunks()
	await process_frame
	await process_frame

	var initial_chunks: Array = []
	var updates: Array = []
	for index in resident_chunks:
		var key := _chunk_key(index)
		var arrays := _large_layer_arrays if index < changed_chunks else _small_layer_arrays
		var chunk := _chunk_payload(key.x, key.y, arrays)
		initial_chunks.append(chunk)
		if index < changed_chunks:
			updates.append(chunk)

	simulation.generation += 1
	simulation.response = _batch(simulation.generation, true, initial_chunks)
	if road_tool.update_main_mesh(simulation.generation) != simulation.generation:
		_failures += 1
		push_error("road chunk benchmark failed to create deterministic resident fixture")
		return
	if road_tool._road_chunk_instances.size() != resident_chunks:
		_failures += 1
		push_error("road chunk benchmark resident fixture has the wrong chunk count")
		return
	var fixture_key := _chunk_key(resident_chunks - 1)
	var fixture_instance: MeshInstance3D = road_tool._road_chunk_instances[fixture_key]
	if fixture_instance.position != _chunk_position(fixture_key):
		_failures += 1
		push_error("road chunk benchmark fixture has the wrong chunk origin placement")
		return
	await process_frame
	RenderingServer.force_sync()

	for _sample in WARMUP_SAMPLES:
		var warmup_times := await _run_update(
			road_tool,
			simulation,
			updates,
			resident_chunks
		)
		if warmup_times.x < 0:
			return

	var stage_samples_us: Array[int] = []
	var complete_samples_us: Array[int] = []
	for _sample in MEASURED_SAMPLES:
		var sample_times := await _run_update(
			road_tool,
			simulation,
			updates,
			resident_chunks
		)
		if sample_times.x < 0:
			return
		stage_samples_us.append(sample_times.x)
		complete_samples_us.append(sample_times.y)
	stage_samples_us.sort()
	complete_samples_us.sort()

	var changed_vertices := changed_chunks * LARGE_VERTICES_PER_CHUNK
	print(
		(
			"ROAD_CHUNK_GODOT kind=%s resident_chunks=%d changed_chunks=%d "
			+ "vertices_per_changed=%d changed_vertices=%d samples=%d "
			+ "stage_median_us=%d stage_p95_us=%d stage_min_us=%d stage_max_us=%d "
			+ "complete_median_us=%d complete_p95_us=%d complete_min_us=%d complete_max_us=%d"
		)
		% [
			kind,
			resident_chunks,
			changed_chunks,
			LARGE_VERTICES_PER_CHUNK,
			changed_vertices,
			stage_samples_us.size(),
			_percentile(stage_samples_us, 0.50),
			_percentile(stage_samples_us, 0.95),
			stage_samples_us.front(),
			stage_samples_us.back(),
			_percentile(complete_samples_us, 0.50),
			_percentile(complete_samples_us, 0.95),
			complete_samples_us.front(),
			complete_samples_us.back(),
		]
	)

func _run_update(
	road_tool: BenchmarkRoadTool,
	simulation: MockSimulation,
	updates: Array,
	expected_resident_chunks: int
) -> Vector2i:
	simulation.generation += 1
	simulation.response = _batch(simulation.generation, false, updates)
	var previous_changed_instances := {}
	for chunk in updates:
		var key := Vector2i(int(chunk["chunk_x"]), int(chunk["chunk_z"]))
		previous_changed_instances[key] = road_tool._road_chunk_instances.get(key)
	var sentinel_key := _chunk_key(expected_resident_chunks - 1)
	var sentinel_instance: MeshInstance3D = road_tool._road_chunk_instances[sentinel_key]

	RenderingServer.force_sync()
	var started_usec := Time.get_ticks_usec()
	var committed_generation := road_tool.update_main_mesh(simulation.generation)
	var stage_usec := Time.get_ticks_usec() - started_usec
	RenderingServer.force_sync()
	var complete_usec := Time.get_ticks_usec() - started_usec
	if committed_generation != simulation.generation:
		_failures += 1
		push_error("road chunk benchmark update did not commit")
		return Vector2i(-1, -1)
	if road_tool._road_chunk_instances.size() != expected_resident_chunks:
		_failures += 1
		push_error("road chunk benchmark update changed the resident count")
		return Vector2i(-1, -1)
	if (
		road_tool._road_mesh_generation != simulation.generation
		or road_tool._road_chunk_span_m != CHUNK_SPAN_M
		or road_tool._road_chunk_origin_x_m != CHUNK_ORIGIN_X_M
		or road_tool._road_chunk_origin_z_m != CHUNK_ORIGIN_Z_M
	):
		_failures += 1
		push_error("road chunk benchmark update committed incoherent metadata")
		return Vector2i(-1, -1)
	if road_tool._road_chunk_instances[sentinel_key] != sentinel_instance:
		_failures += 1
		push_error("road chunk benchmark replaced an untouched sentinel")
		return Vector2i(-1, -1)
	if sentinel_instance.position != _chunk_position(sentinel_key):
		_failures += 1
		push_error("road chunk benchmark moved the untouched sentinel")
		return Vector2i(-1, -1)
	for key in previous_changed_instances:
		if road_tool._road_chunk_instances[key] == previous_changed_instances[key]:
			_failures += 1
			push_error("road chunk benchmark did not replace a changed chunk")
			return Vector2i(-1, -1)
		if road_tool._road_chunk_instances[key].position != _chunk_position(key):
			_failures += 1
			push_error("road chunk benchmark placed a changed chunk at the wrong origin")
			return Vector2i(-1, -1)
	await process_frame
	RenderingServer.force_sync()
	return Vector2i(stage_usec, complete_usec)

func _batch(generation: int, full_replace: bool, chunks: Array) -> Dictionary:
	return {
		"surface_generation": generation,
		"full_replace": full_replace,
		"chunk_span_m": CHUNK_SPAN_M,
		"chunk_origin_x_m": CHUNK_ORIGIN_X_M,
		"chunk_origin_z_m": CHUNK_ORIGIN_Z_M,
		"chunks": chunks,
	}

func _chunk_payload(chunk_x: int, chunk_z: int, layer_arrays: Dictionary) -> Dictionary:
	var chunk := {
		"chunk_x": chunk_x,
		"chunk_z": chunk_z,
		"removed": false,
	}
	for layer in LAYERS:
		var arrays: Dictionary = layer_arrays[layer]
		chunk[layer + "_vertices"] = arrays["vertices"]
		chunk[layer + "_normals"] = arrays["normals"]
		chunk[layer + "_uvs"] = arrays["uvs"]
		chunk[layer + "_colors"] = arrays["colors"]
	return chunk

func _build_layer_arrays(vertex_counts: Dictionary) -> Dictionary:
	var result := {}
	for layer_index in LAYERS.size():
		var layer := LAYERS[layer_index]
		var vertex_count := int(vertex_counts.get(layer, 0))
		assert(vertex_count % 3 == 0)
		var vertices := PackedVector3Array()
		var normals := PackedVector3Array()
		var uvs := PackedVector2Array()
		var colors := PackedColorArray()
		for triangle_index in vertex_count / 3:
			var base_x := float(triangle_index % 64) * 0.5
			var base_z := float(triangle_index / 64) * 0.5 + float(layer_index) * 8.0
			vertices.append(Vector3(base_x, 0.0, base_z))
			vertices.append(Vector3(base_x + 0.25, 0.0, base_z))
			vertices.append(Vector3(base_x, 0.0, base_z + 0.25))
			normals.append(Vector3.UP)
			normals.append(Vector3.UP)
			normals.append(Vector3.UP)
			uvs.append(Vector2.ZERO)
			uvs.append(Vector2.RIGHT)
			uvs.append(Vector2.DOWN)
			colors.append(Color.WHITE)
			colors.append(Color.WHITE)
			colors.append(Color.WHITE)
		result[layer] = {
			"vertices": vertices,
			"normals": normals,
			"uvs": uvs,
			"colors": colors,
		}
	return result

func _chunk_key(index: int) -> Vector2i:
	return Vector2i(index % 16, index / 16)

func _chunk_position(key: Vector2i) -> Vector3:
	return Vector3(
		CHUNK_ORIGIN_X_M + float(key.x) * CHUNK_SPAN_M,
		0.0,
		CHUNK_ORIGIN_Z_M + float(key.y) * CHUNK_SPAN_M
	)

func _layer_vertex_count(layer_arrays: Dictionary) -> int:
	var count := 0
	for layer in LAYERS:
		count += int(layer_arrays[layer]["vertices"].size())
	return count

func _percentile(sorted_samples: Array[int], percentile: float) -> int:
	var index := clampi(
		int(ceil(float(sorted_samples.size()) * percentile)) - 1,
		0,
		sorted_samples.size() - 1
	)
	return sorted_samples[index]
