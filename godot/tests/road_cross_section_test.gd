# SPDX-License-Identifier: GPL-2.0-only

## Medians through the native bridge (`ROAD-80`), for each style: the road tool's median is
## refused on a one-way stroke with its reason, previews and commits on a 1 + 1 stroke, and the
## committed road reports it; widening the road in place keeps it, and a crossing road splits it
## into two halves that both keep it; only a grass median's chunks carry the grass layer. Out-of-range widths and unknown styles are refused by
## `set_road_tool_median`, which keeps the previous median and accepts the default and range ends
## `get_median_styles` publishes. Ported from Veeti's
## `road_cross_section_test.gd` (bridge half) onto dev's API.
## One side (`ROAD-107`): the tool's side code is checked, kept and drawn; `preview_...` and
## `set_edge_cross_section` add and remove one side and refuse a bad code; a lane change passing the
## road's own code keeps it one-sided; undo restores each. Walkers on a removed side have no
## bridge call: `one_sidewalk_removed_in_place_keeps_the_other_and_undoes` holds them.
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

func _await_generation(generation: int) -> bool:
	for frame in TIMEOUT_FRAMES:
		if simulation.get_network_render_generation() > generation:
			return true
		await process_frame
	return false

func _sides(edge: int) -> int:
	return int(simulation.get_edge_cross_section(edge).get("sidewalks", -1))

func _undo(what: String) -> void:
	var generation: int = simulation.get_network_render_generation()
	_expect(simulation.undo_action(), "%s: undo is queued" % what)
	_expect(await _await_generation(generation), "%s: undo publishes" % what)

# One road's sidewalk sides through the tool, then each in-place change of them.
func _one_sided() -> void:
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "one side: the blank world loads")
	simulation.set_simulation_speed(0.0)
	_expect(simulation.set_road_tool_attributes(50, 4, true, true) == "sidewalks_out_of_range", "a code past both sides is refused")
	_expect(simulation.set_road_tool_attributes(50, -1, true, true) == "sidewalks_out_of_range", "a negative code is refused")
	_expect(simulation.set_road_tool_attributes(50, 1, true, true).is_empty(), "the left side is set")
	var built := await _commit(PackedVector3Array([_ground(-100.0, 40.0), _ground(100.0, 40.0)]))
	_expect(bool(built.get("committed", false)), "the left-only road commits: %s" % built.get("reason", ""))
	var edge: int = simulation.get_hovered_edge(0.0, 40.0)
	_expect(_sides(edge) == 1, "the road reads back left: %d" % _sides(edge))
	_expect(String(simulation.get_edge_cross_section(edge).get("road_type", "x")).is_empty(), "no built-in has one side")
	var bad: Dictionary = simulation.preview_edge_cross_section(edge, 1, 1, 0, 0.0, 50, 7, true, true)
	_expect(bad.get("invalid_reason", "") == "sidewalks_out_of_range", "the preview refuses a bad code: %s" % bad)
	_expect(not simulation.set_edge_cross_section(edge, 1, 1, 0, 0.0, 50, 7, true, true) and _sides(edge) == 1, "the commit refuses it")
	var same: Dictionary = simulation.preview_edge_cross_section(edge, 1, 1, 0, 0.0, 50, 1, true, true)
	_expect(same.get("invalid_reason", "") == "cross_section_unchanged", "its own side is no change: %s" % same)
	# Add the right side, take the left off, each with its undo; `BOTH` costs more, a side less.
	var add: Dictionary = simulation.preview_edge_cross_section(edge, 1, 1, 0, 0.0, 50, 3, true, true)
	_expect(bool(add.get("is_valid", false)) and float(add.get("cost_delta", 0.0)) > 0.0, "adding the right side is accepted and charged: %s" % add)
	_expect(_sides(edge) == 1, "a preview changes nothing")
	_expect(simulation.set_edge_cross_section(edge, 1, 1, 0, 0.0, 50, 3, true, true) and _sides(edge) == 3, "the right side is added")
	var removal: Dictionary = simulation.preview_edge_cross_section(edge, 1, 1, 0, 0.0, 50, 2, true, true)
	_expect(bool(removal.get("is_valid", false)) and is_zero_approx(float(removal.get("cost_delta", 1.0))), "taking a side off is free: %s" % removal)
	_expect(simulation.set_edge_cross_section(edge, 1, 1, 0, 0.0, 50, 2, true, true) and _sides(edge) == 2, "the left side is removed")
	await _undo("the removed side")
	_expect(_sides(edge) == 3, "undo brings the left side back: %d" % _sides(edge))
	await _undo("the added side")
	_expect(_sides(edge) == 1, "undo takes the right side off: %d" % _sides(edge))
	# Lanes changed with the road's own code: never two-sided by accident.
	var own := _sides(edge)
	_expect(simulation.set_edge_cross_section(edge, 2, 2, 0, 0.0, 50, own, true, true), "widened in place")
	_expect(_sides(edge) == 1 and int(simulation.get_edge_cross_section(edge).get("fwd_lanes", 0)) == 2, "the wider road keeps its one side")
	await _undo("the widening")
	# A crossing stroke splits it: both halves keep the side.
	_expect(simulation.set_road_tool_attributes(50, 3, true, true).is_empty(), "two sides for the crossing")
	var crossing := await _commit(PackedVector3Array([_ground(0.0, -40.0), _ground(0.0, 120.0)]))
	_expect(bool(crossing.get("committed", false)), "the crossing commits: %s" % crossing.get("reason", ""))
	for x in [-50.0, 50.0]:
		_expect(int(_section_at(x, 40.0).get("sidewalks", -1)) == 1, "the half at x %.0f keeps the left side" % x)

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
		var styles: Dictionary = simulation.get_median_styles()
		_expect(styles.size() == 3, "get_median_styles has the three styles: %s" % styles)
		var widths: Dictionary = styles[style]
		for accepted in [widths.min_m, widths.default_m, widths.max_m]:
			_expect(simulation.set_road_tool_median(style, accepted).is_empty(), "style %d: the published %.1f m is accepted" % [style, accepted])
		_expect(simulation.set_road_tool_median(style, widths.max_m + 0.5) == "median_width_out_of_range", "style %d: past the published maximum is refused" % style)
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
		_expect(simulation.set_edge_cross_section(edge, 2, 2, style, width, 0, 50, 3, true, true), "style %d: widened in place" % style)
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
	await _one_sided()
	scene.free()
	if _failures == 0:
		print("road_cross_section_test: PASS")
	quit(_failures)
