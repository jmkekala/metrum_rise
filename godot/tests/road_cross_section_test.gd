# SPDX-License-Identifier: GPL-2.0-only

## Medians through the native bridge (`ROAD-80`), for each style: the road tool's median is
## refused on a one-way stroke with its reason, previews and commits on a 1 + 1 stroke, and the
## committed road reports it; widening the road in place keeps it, and a crossing road splits it
## into two halves that both keep it; only a grass median's chunks carry the grass layer. Out-of-range widths and unknown styles are refused by
## `set_road_tool_median`, which keeps the previous median. Ported from Veeti's
## `road_cross_section_test.gd` (bridge half) onto dev's API.
extends SceneTree

const TIMEOUT_FRAMES := 1200

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

func _commit(points: PackedVector3Array) -> Dictionary:
	var request: int = simulation.add_road_with_snap(points, 1, 1, true)
	for frame in TIMEOUT_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			return result
		await process_frame
	_expect(false, "road commit timed out")
	return {}

func _section_at(x: float, z: float) -> Dictionary:
	return simulation.get_edge_cross_section(simulation.get_hovered_edge(x, z))

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	scene.add_child(simulation)
	for style in [1, 2, 3]:
		var width: float = [0.0, 1.0, 5.0, 2.0][style]
		_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "style %d: the blank world loads" % style)
		simulation.set_simulation_speed(0.0)
		_expect(simulation.set_road_tool_median(style, 40.0) == "median_width_out_of_range", "style %d: a width out of range is refused" % style)
		_expect(simulation.set_road_tool_median(7, width) == "unknown_median_style", "an unknown style is refused")
		_expect(simulation.set_road_tool_median(style, width).is_empty(), "style %d: the median is set" % style)
		var points := PackedVector3Array([_ground(-100.0, 0.0), _ground(100.0, 0.0)])
		var refused: Dictionary = simulation.validate_road_candidate_with_snap(points, 1, 0, true)
		_expect(not bool(refused.get("is_valid", true)) and refused.get("invalid_reason", "") == "median_requires_both_directions", "style %d: a one-way median is refused: %s" % [style, refused.get("invalid_reason", "")])
		var preview: Dictionary = simulation.preview_road(points, 1, 1, true)
		_expect(bool(preview.get("is_valid", false)), "style %d: the divided stroke previews: %s" % [style, preview.get("invalid_reason", "")])
		var built := await _commit(points)
		_expect(bool(built.get("committed", false)), "style %d: the divided road commits: %s" % [style, built.get("reason", "")])
		var edge: int = simulation.get_hovered_edge(0.0, 0.0)
		var section: Dictionary = simulation.get_edge_cross_section(edge)
		_expect(int(section.get("median_style", -1)) == style and is_equal_approx(float(section.get("median_width", 0.0)), width), "style %d: the road carries its median: %s" % [style, section])
		_expect(simulation.set_edge_cross_section(edge, 2, 2, style, width), "style %d: widened in place" % style)
		_expect(int(simulation.get_edge_cross_section(edge).get("fwd_lanes", 0)) == 2, "style %d: the edit keeps the edge id" % style)
		# Only a kerbed grass median sends the grass layer (ROAD-80).
		var payload := str(simulation.get_road_mesh_data(true))
		_expect(payload.contains("median_grass_vertices") == (style == 2), "style %d: the grass layer is the grass median's alone" % style)
		# The crossing road is undivided.
		_expect(simulation.set_road_tool_median(0, 0.0).is_empty(), "the median is cleared")
		var crossing := await _commit(PackedVector3Array([_ground(0.0, -90.0), _ground(0.0, 90.0)]))
		_expect(bool(crossing.get("committed", false)), "style %d: the crossing commits: %s" % [style, crossing.get("reason", "")])
		for x in [-50.0, 50.0]:
			var half := _section_at(x, 0.0)
			_expect(int(half.get("median_style", -1)) == style and int(half.get("fwd_lanes", 0)) == 2, "style %d: the half at x %.0f keeps the median: %s" % [style, x, half])
	scene.free()
	if _failures == 0:
		print("road_cross_section_test: PASS")
	quit(_failures)
