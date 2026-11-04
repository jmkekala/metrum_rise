# SPDX-License-Identifier: GPL-2.0-only

## Selective turn arrows through the native bridge (`ROAD-75`): a four-way of two 1 + 1 streets
## publishes no marking at all, while a four-way of two 2 + 2 roads paints arrows inside the
## approach lanes before the junction (the dividers lie on the lane edges, so paint between them
## is an arrow). Ported from the arrow half of Veeti's `road_marking_rules_test.gd` onto dev's API.
extends SceneTree

const TIMEOUT_FRAMES := 1200
const STREETS_X := -250.0
const ROADS_X := 250.0
# Right-hand traffic: the eastbound lanes of a 2 + 2 road lie south (+z) of its centre line, the
# divider between them at 3.5 m. Arrow glyphs reach 1.2 m either side of a lane's centre.
const LANE_PAINT_MIN_Z := 0.3
const LANE_PAINT_MAX_Z := 3.2

var _failures := 0
var simulation: SimulationNode

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _ground(x: float, z: float) -> Vector3:
	return Vector3(x, simulation.get_world_surface_height(Vector2(x, z)), z)

func _commit(points: PackedVector3Array, lanes: int) -> void:
	var request: int = simulation.add_road_with_snap(points, lanes, lanes, true)
	for frame in TIMEOUT_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			_expect(bool(result.get("committed", false)), "a %d + %d road commits: %s" % [lanes, lanes, result.get("reason", "")])
			return
		await process_frame
	_expect(false, "road commit timed out")

func _cross(x: float, lanes: int) -> void:
	await _commit(PackedVector3Array([_ground(x - 150.0, 0.0), _ground(x + 150.0, 0.0)]), lanes)
	await _commit(PackedVector3Array([_ground(x, -150.0), _ground(x, 150.0)]), lanes)

# Every published marking vertex within `x_min..x_max` in world space, from a full snapshot of
# the road chunks. A chunk's vertices are local to its frame centre, as network_tool.gd places it.
func _markings(x_min: float, x_max: float) -> PackedVector3Array:
	var found := PackedVector3Array()
	var mesh: Dictionary = simulation.get_road_mesh_data(true)
	var span := float(mesh.get("chunk_span_m", 0.0))
	var origin_x := float(mesh.get("chunk_origin_x_m", 0.0))
	var origin_z := float(mesh.get("chunk_origin_z_m", 0.0))
	for chunk: Dictionary in mesh.get("chunks", []):
		var centre := Vector3(
			origin_x + (float(chunk.get("chunk_x", 0)) + 0.5) * span,
			0.0,
			origin_z + (float(chunk.get("chunk_z", 0)) + 0.5) * span
		)
		for local: Vector3 in chunk.get("marking_vertices", PackedVector3Array()):
			var vertex := local + centre
			if vertex.x >= x_min and vertex.x <= x_max:
				found.append(vertex)
	return found

func _road_chunks() -> int:
	var count := 0
	var mesh: Dictionary = simulation.get_road_mesh_data(true)
	for chunk: Dictionary in mesh.get("chunks", []):
		if not chunk.get("road_vertices", PackedVector3Array()).is_empty():
			count += 1
	return count

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(1024.0, 1024.0, 4.0, 128.0, 0.0), "the blank world loads")
	simulation.set_simulation_speed(0.0)
	await _cross(STREETS_X, 1)
	# The commit publishes its chunks after its result: wait until the streets are drawn.
	var frames := 0
	while _road_chunks() == 0 and frames < TIMEOUT_FRAMES:
		frames += 1
		await process_frame
	_expect(_road_chunks() > 0, "the 1 + 1 four-way publishes its chunks")
	_expect(_markings(-1.0e6, 1.0e6).is_empty(), "a 1 + 1 four-way has no marking, so no arrow")
	await _cross(ROADS_X, 2)
	var arrows := PackedVector3Array()
	frames = 0
	while arrows.is_empty() and frames < TIMEOUT_FRAMES:
		frames += 1
		await process_frame
		for vertex: Vector3 in _markings(ROADS_X - 60.0, ROADS_X - 8.0):
			if vertex.z > LANE_PAINT_MIN_Z and vertex.z < LANE_PAINT_MAX_Z:
				arrows.append(vertex)
	_expect(not arrows.is_empty(), "the 2 + 2 four-way's west approach paints arrows in its lanes")
	_expect(_markings(-1.0e6, 0.0).is_empty(), "the 1 + 1 four-way stays unmarked")
	scene.free()
	if _failures == 0:
		print("road_turn_arrows_test: PASS")
	quit(_failures)
