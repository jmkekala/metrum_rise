# SPDX-License-Identifier: GPL-2.0-only

## A T junction whose stem has no sidewalks (`ROAD-101`, design D2 (a)) against the native bridge:
## the through road drawn as the 2-Lane Road, the stem by the road tool's attributes without
## sidewalks. The stem reads back without sidewalks, has no zebra and offers no crossing toggle;
## the through road's two arms keep their zebras, so a walker passing the stem crosses the T by
## the far side (the walk itself is the Rust test
## `a_t_whose_stem_has_no_sidewalks_is_crossed_by_the_far_side`). Writes no road-type library.
extends SceneTree

var _failures := 0
var simulation: SimulationNode

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _ground_point(x: float, z: float) -> Vector3:
	return Vector3(x, simulation.get_world_surface_height(Vector2(x, z)), z)

# Waits for the render generation to pass `generation`; false after 20 s.
func _await_generation(generation: int) -> bool:
	var deadline := Time.get_ticks_msec() + 20000
	while Time.get_ticks_msec() < deadline:
		if simulation.get_network_render_generation() > generation:
			return true
		await process_frame
	return false

func _commit(points: PackedVector3Array) -> bool:
	var generation := simulation.get_network_render_generation()
	simulation.add_road_with_snap(points, 1, 1, true)
	var done := await _await_generation(generation)
	_expect(done, "fixture road must commit")
	return done

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "fixture world must load")
	simulation.set_simulation_speed(0.0)
	_expect(simulation.set_road_tool_attributes(50, true, true, true).is_empty(), "the 2-Lane Road's attributes")
	if not await _commit(PackedVector3Array([_ground_point(-60.0, 0.0), _ground_point(60.0, 0.0)])):
		_finish(scene)
		return
	_expect(simulation.set_road_tool_attributes(50, false, true, true).is_empty(), "a stem without sidewalks")
	if not await _commit(PackedVector3Array([_ground_point(0.0, 0.0), _ground_point(0.0, 60.0)])):
		_finish(scene)
		return
	simulation.set_road_tool_attributes(50, true, true, true)

	var west: int = simulation.get_hovered_edge(-30.0, 0.0)
	var east: int = simulation.get_hovered_edge(30.0, 0.0)
	var stem: int = simulation.get_hovered_edge(0.0, 30.0)
	_expect(west >= 0 and east >= 0 and stem >= 0 and west != east, "the T's three arms: %d %d %d" % [west, east, stem])
	var ends: Vector2i = simulation.get_edge_nodes(stem)
	var west_ends: Vector2i = simulation.get_edge_nodes(west)
	var hub := ends.x if ends.x == west_ends.x or ends.x == west_ends.y else ends.y
	_expect(simulation.get_node_connection_count(hub) == 3, "the hub joins three arms")

	var stem_section: Dictionary = simulation.get_edge_cross_section(stem)
	_expect(not bool(stem_section.get("sidewalks", true)) and int(stem_section.get("speed_kmh", 0)) == 50, "the stem has no sidewalks: %s" % stem_section)
	for arm in [west, east]:
		_expect(bool(simulation.get_edge_cross_section(arm).get("sidewalks", false)), "through arm %d keeps its sidewalks" % arm)
		_expect(simulation.has_crosswalk(hub, arm), "through arm %d keeps its zebra" % arm)
	_expect(not simulation.has_crosswalk(hub, stem), "the stem has no zebra")
	_expect(not simulation.can_toggle_crosswalk(hub, stem), "the stem offers no crossing toggle")
	_finish(scene)

func _finish(scene: Node) -> void:
	scene.free()
	if _failures == 0:
		print("road_mixed_junction_test: PASS")
	quit(_failures)
