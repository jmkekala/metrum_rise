# SPDX-License-Identifier: GPL-2.0-only

## Advance lane choice through the native bridge (`ROAD-94`): a 4 + 4 arterial with a grass
## median crossed by a 1 + 1 street. A debug car spawned on the arterial's innermost lane with a
## right turn ahead reaches the rightmost approach lane before the junction, then drives the
## right turn onto the cross street without a trip failure. Ported from Veeti's
## `road_turn_lanes_test.gd` onto dev's API.
extends SceneTree

const TIMEOUT_FRAMES := 1200
# Wall-clock limits for the car's pick and its drive, at simulation speed 4.
const PICK_TIMEOUT_MS := 10000
const DRIVE_TIMEOUT_MS := 30000
const MEDIAN_WIDTH_M := 5.0
# Right-hand traffic: the eastbound lanes lie south (+z) of the median. The rightmost of four
# is centred at half the median plus 3.5 lanes, 14.75 m; the next one in at 11.25 m.
const RIGHTMOST_LANE_MIN_Z := 13.5
const CROSS_STREET_Z := 35.0

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

func _commit(points: PackedVector3Array, lanes: int) -> Dictionary:
	var request: int = simulation.add_road_with_snap(points, lanes, lanes, true)
	for frame in TIMEOUT_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			return result
		await process_frame
	_expect(false, "road commit timed out")
	return {}

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(1024.0, 1024.0, 4.0, 128.0, 0.0), "the blank world loads")
	simulation.set_simulation_speed(0.0)
	_expect(simulation.set_road_tool_median(2, MEDIAN_WIDTH_M).is_empty(), "the grass median is set")
	var arterial := await _commit(PackedVector3Array([_ground(-230.0, 0.0), _ground(230.0, 0.0)]), 4)
	_expect(bool(arterial.get("committed", false)), "the 4 + 4 arterial commits: %s" % arterial.get("reason", ""))
	_expect(simulation.set_road_tool_median(0, 0.0).is_empty(), "the median is cleared")
	var street := await _commit(PackedVector3Array([_ground(0.0, -150.0), _ground(0.0, 150.0)]), 1)
	_expect(bool(street.get("committed", false)), "the 1 + 1 cross street commits: %s" % street.get("reason", ""))
	await _check_right_turn()
	scene.free()
	if _failures == 0:
		print("road_turn_lanes_test: PASS")
	quit(_failures)

func _check_right_turn() -> void:
	var start := _ground(-230.0, 0.0)
	var id: int = simulation.spawn_debug_car(start, _ground(0.0, 150.0))
	_expect(id >= 0, "a route through the junction exists")
	if id < 0:
		return
	simulation.set_simulation_speed(4.0)
	# A spawned car is picked once the simulation thread has ticked it onto its lane.
	var deadline := Time.get_ticks_msec() + PICK_TIMEOUT_MS
	var picked := false
	while not picked and Time.get_ticks_msec() < deadline:
		await process_frame
		picked = simulation.pick_debug_agent(start, 35.0) == id
	_expect(picked, "the test car can be inspected")
	deadline = Time.get_ticks_msec() + DRIVE_TIMEOUT_MS
	var prepared := false
	var turned := false
	var last := {}
	while Time.get_ticks_msec() < deadline and not turned:
		await process_frame
		var details: Dictionary = simulation.get_agent_inspection(id)
		if not details.get("active", false):
			continue
		last = details
		var at: Vector3 = details.position
		if at.x < -25.0 and at.z > RIGHTMOST_LANE_MIN_Z:
			prepared = true
		if at.z > CROSS_STREET_Z:
			turned = true
			_expect(str(details.get("last_failure", "")).is_empty(), "the prepared car keeps its turn: %s" % details.get("last_failure", ""))
	_expect(prepared, "the car reaches the rightmost approach lane before the junction: %s" % last)
	_expect(turned, "the car drives the right turn onto the cross street: %s" % last)
	simulation.set_simulation_speed(0.0)
