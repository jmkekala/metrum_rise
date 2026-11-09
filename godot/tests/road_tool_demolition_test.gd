# SPDX-License-Identifier: GPL-2.0-only

## Roads over zoned land in the road tool (`ZONE-09`, PLAN-RZ test 9): on a street with one house
## on its cell lot (`METRUM_ROAD_DEMOLITION_FIXTURE`, written by the Rust recorder
## `record_road_demolition_fixture`, with the house stroke beside it in `<fixture>.json`), a
## stroke over the house previews valid, names one demolished building in the HUD and outlines
## its lot with the bulldoze highlight; the click commits and the house is gone. A stroke over a
## placed parking lot previews its refusal and its click commits nothing.
extends SceneTree

const RoadToolScript := preload("res://scripts/tools/road_tool.gd")
const WorldMaterialsScript := preload("res://scripts/renderers/world_materials.gd")
const RoadReasonText := preload("res://scripts/ui/road_reason_text.gd")
const PARKING_LOT := "parking:building.parking.small_lot"
const TIMEOUT_MS := 20000

class GradedStub extends RefCounted:
	func show_preview(_frame: Dictionary) -> float:
		return 0.0
	func installed_edit_serial() -> int:
		return 0
	func release_preview(_first_epoch: int, _last_epoch: int) -> void:
		pass

class TerrainStub extends Node3D:
	var graded_tiles := GradedStub.new()

var _failures := 0
var simulation: SimulationNode

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var fixture := OS.get_environment("METRUM_ROAD_DEMOLITION_FIXTURE")
	_expect(FileAccess.file_exists(fixture), "METRUM_ROAD_DEMOLITION_FIXTURE must name the street save, got '%s'" % fixture)
	var strokes = JSON.parse_string(FileAccess.get_file_as_string(fixture + ".json"))
	_expect(strokes is Dictionary and strokes.has("house_stroke"), "the fixture's strokes must load")
	if _failures > 0:
		_finish()
		return
	simulation = SimulationNode.new()
	root.add_child(simulation)
	var warnings := simulation.load_asset_packs(ProjectSettings.globalize_path("res://bootstrap/mods"), PackedStringArray(["kenney", "parking"]))
	_expect(warnings.is_empty(), "the house and parking packs load: " + warnings)
	_expect(simulation.load_game(fixture), "the street fixture must load")
	simulation.set_simulation_speed(0.0)
	var tool := _road_tool()
	await _test_house(tool, _points(strokes["house_stroke"]))
	await _test_parking_lot(tool)
	tool.free()
	simulation.free()
	_finish()

func _finish() -> void:
	if _failures == 0:
		print("road_tool_demolition_test: PASS")
	quit(0 if _failures == 0 else 1)

func _points(stroke: Array) -> PackedVector3Array:
	var points := PackedVector3Array()
	for point in stroke:
		points.append(Vector3(point[0], point[1], point[2]))
	return points

func _road_tool() -> Node3D:
	var tool := RoadToolScript.new()
	tool.name = "RoadTool"
	tool.simulation_node = simulation
	tool.terrain_node = TerrainStub.new()
	tool.add_child(tool.terrain_node)
	tool.road_mesh_root = Node3D.new()
	tool.add_child(tool.road_mesh_root)
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._info_label = Label.new()
	tool.add_child(tool._info_label)
	tool._road_preview_material = WorldMaterialsScript.road_preview_material()
	tool.blueprint_mesh.material_override = tool._road_preview_material
	tool.fwd_lanes = 1
	tool.bkw_lanes = 1
	tool.active = true
	var generation := simulation.get_network_render_generation()
	_expect(tool.update_main_mesh(generation) == generation, "the resident chunks hydrate before the preview")
	tool._node_visuals_dirty = false
	tool._node_visuals_visible = true
	return tool

# Previews `points` until `done` holds for the preview: the road tool's land follows the city
# a simulation frame after a change, so a stroke previewed earlier may not see it yet.
func _preview_until(tool: Node3D, points: PackedVector3Array, done: Callable) -> Dictionary:
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while true:
		tool._begin_stroke(points[0], 0, Vector3.ZERO)
		tool.set_scripted_pointer(true, points[-1])
		tool._process(1.0 / 60.0)
		if done.call(tool._preview) or Time.get_ticks_msec() > deadline:
			return tool._preview
		tool.cancel_road()
		await process_frame
	return {}

# Clicks the previewed stroke and waits for Rust's answer; true when it committed.
func _click(tool: Node3D, points: PackedVector3Array) -> bool:
	var generation := simulation.get_network_render_generation()
	if not tool._commit_segment(points[-1]):
		return false
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while tool._commit_request_id > 0 and Time.get_ticks_msec() < deadline:
		tool._process(1.0 / 60.0)
		await process_frame
	_expect(tool._commit_request_id == 0, "the commit must answer")
	return simulation.get_network_render_generation() > generation

func _test_house(tool: Node3D, points: PackedVector3Array) -> void:
	var before: Dictionary = simulation.get_road_benchmark_state()
	var preview := await _preview_until(tool, points, func(p: Dictionary) -> bool: return int(p.get("demolish_count", 0)) > 0)
	_expect(preview.get("is_valid", false), "a stroke over a built lot is valid: %s" % preview.get("invalid_reason", ""))
	_expect(int(preview.get("demolish_count", 0)) == 1, "the stroke demolishes the one house: %s" % preview.get("demolish_count", 0))
	var lots: PackedVector3Array = preview.get("demolish_lots", PackedVector3Array())
	_expect(lots.size() == 4, "one lot outline, four corners: %d" % lots.size())
	_expect(tool._info_label.text.contains("demolishes 1 building") and not tool._info_label.text.contains("buildings"), "the HUD names the demolition: '%s'" % tool._info_label.text)
	var outline: MeshInstance3D = tool._demolish_mesh
	_expect(outline != null and outline.visible and outline.mesh != null, "the lot is outlined")
	if outline != null and outline.mesh != null:
		_expect(outline.mesh.get_surface_count() == 2, "the outline is a fill and a line surface")
	# The lot lies under the stroke: every corner within a lot's reach of its line.
	for corner in lots:
		var along := Geometry3D.get_closest_point_to_segment(corner, points[0], points[-1])
		_expect(Vector2(corner.x - along.x, corner.z - along.z).length() < 40.0, "the outline lies at the stroke: %s" % corner)
	_expect(await _click(tool, points), "the stroke over the house commits")
	var after: Dictionary = simulation.get_road_benchmark_state()
	_expect(int(after["buildings"]) == int(before["buildings"]) - 1, "the house is gone: %d -> %d" % [before["buildings"], after["buildings"]])
	_expect(tool._demolish_mesh == null or not tool._demolish_mesh.visible, "the outline ends with the stroke")
	tool.cancel_road()

func _test_parking_lot(tool: Node3D) -> void:
	var at := Vector2(100.0, 30.0)
	var placed := simulation.place_service_building(PARKING_LOT, at.x, at.y)
	_expect(placed.is_empty(), "the parking lot places beside the street: " + placed)
	var height := simulation.get_world_surface_height(at)
	var points := PackedVector3Array([Vector3(at.x, height, 80.0), Vector3(at.x, height, 0.0)])
	var before: Dictionary = simulation.get_road_benchmark_state()
	var preview := await _preview_until(tool, points, func(p: Dictionary) -> bool: return not p.get("is_valid", true))
	_expect(not preview.get("is_valid", true) and preview.get("invalid_reason", "") == "plotted_building_overlap", "a placed building's lot refuses the stroke: %s" % preview.get("invalid_reason", ""))
	_expect(tool._info_label.text == RoadReasonText.text(preview), "the HUD words the refusal: '%s'" % tool._info_label.text)
	_expect(int(preview.get("demolish_count", -1)) == 0 and (preview.get("demolish_lots", PackedVector3Array()) as PackedVector3Array).is_empty(), "a refused stroke takes nothing")
	_expect(tool._demolish_mesh == null or not tool._demolish_mesh.visible, "a refused stroke outlines nothing")
	_expect(not tool._commit_segment(points[-1]), "the click does not queue a refused stroke")
	var after: Dictionary = simulation.get_road_benchmark_state()
	for field in ["generation", "live_edges", "buildings"]:
		_expect(before[field] == after[field], "the refused click changes no %s" % field)
	tool.cancel_road()
