# SPDX-License-Identifier: GPL-2.0-only

## Road edits in place over zoned land (`ZONE-10`, round RZ10b, PLAN-RZ 10.5 test 9): on a T
## junction whose street is lined with houses (`METRUM_ROAD_EDIT_DEMOLITION_FIXTURE`, written by
## the Rust recorder `record_road_edit_demolition_fixture`, with the junction, a point on the
## widened arm and the move's target in `<fixture>.json`; ids are found by position, as a save
## compacts them), the roundabout tool's hover over the junction outlines
## the lots the ring takes with the bulldoze highlight and names the buildings in its label, and
## the click builds the ring; the Road Properties window's confirmation reads "Apply removes N";
## the move tool's drag into the block is pending (outlined, "Release to demolish N"), moves
## nothing, and the release commits it. Each part loads the fixture afresh.
extends SceneTree

const RoundaboutTool := preload("res://scripts/tools/roundabout_tool.gd")
const MoveTool := preload("res://scripts/tools/move_tool.gd")
const RoadPropertiesWindow := preload("res://scripts/ui/road_properties_window.gd")
const TIMEOUT_MS := 20000

class RoadToolStub extends Node3D:
	var fwd_lanes := 1
	var bkw_lanes := 1
	func mark_network_topology_dirty() -> void:
		pass
	func update_main_mesh() -> void:
		pass

var _failures := 0
var simulation: SimulationNode
var _fixture := ""
var _spec: Dictionary = {}

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	_fixture = OS.get_environment("METRUM_ROAD_EDIT_DEMOLITION_FIXTURE")
	_expect(FileAccess.file_exists(_fixture), "METRUM_ROAD_EDIT_DEMOLITION_FIXTURE must name the T's save, got '%s'" % _fixture)
	var spec = JSON.parse_string(FileAccess.get_file_as_string(_fixture + ".json"))
	_expect(spec is Dictionary and spec.has("junction"), "the fixture's spec must load")
	if _failures > 0:
		_finish(null)
		return
	_spec = spec
	var scene := Node3D.new()
	root.add_child(scene)
	await _test_ring(scene)
	await _test_road_properties(scene)
	await _test_move(scene)
	_finish(scene)

func _finish(scene: Node) -> void:
	if scene != null:
		scene.free()
	if _failures == 0:
		print("road_edit_demolition_test: PASS")
	quit(0 if _failures == 0 else 1)

func _vector(value: Array) -> Vector3:
	return Vector3(value[0], value[1], value[2])

# A fresh simulation on the fixture, under `scene` with the stubs the tools look up.
func _load(scene: Node3D) -> void:
	for child in scene.get_children():
		child.free()
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	var warnings := simulation.load_asset_packs(ProjectSettings.globalize_path("res://bootstrap/mods"), PackedStringArray(["kenney"]))
	_expect(warnings.is_empty(), "the house pack loads: " + warnings)
	_expect(simulation.load_game(_fixture), "the T fixture must load")
	simulation.set_simulation_speed(0.0)
	var terrain := Node3D.new()
	terrain.name = "Terrain"
	scene.add_child(terrain)
	var road := RoadToolStub.new()
	road.name = "RoadTool"
	scene.add_child(road)

func _buildings() -> int:
	return int(simulation.get_road_benchmark_state()["buildings"])

# Waits for the render generation to pass `generation`; false after the deadline.
func _await_generation(generation: int) -> bool:
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while Time.get_ticks_msec() < deadline:
		if simulation.get_network_render_generation() > generation:
			return true
		await process_frame
	return false

func _test_ring(scene: Node3D) -> void:
	_load(scene)
	var tool: Node3D = RoundaboutTool.new()
	tool.name = "RoundaboutTool"
	scene.add_child(tool)
	tool.set_process(false)
	tool.set_tool_active(true)
	tool.set_diameter(float(_spec["ring_diameter_m"]))
	var junction := _vector(_spec["junction"])
	# The road tool's land follows the city a simulation frame after the load: hover until the
	# hover sees the houses.
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while true:
		tool.set_scripted_pointer(true, junction)
		tool._key = []
		tool._update_preview()
		if int(tool._preview.get("demolish_count", 0)) > 0 or Time.get_ticks_msec() > deadline:
			break
		await process_frame
	var preview: Dictionary = tool._preview
	var count := int(preview.get("demolish_count", 0))
	_expect(bool(preview.get("is_valid", false)), "the ring over the T is valid: %s" % preview.get("invalid_reason", ""))
	_expect(count > 0, "the ring demolishes houses: %d" % count)
	var lots: PackedVector3Array = preview.get("demolish_lots", PackedVector3Array())
	_expect(lots.size() >= 4 and lots.size() % 4 == 0, "four corners per taken lot: %d" % lots.size())
	_expect(tool._demolish_mesh.visible and tool._demolish_mesh.mesh != null, "the taken lots are outlined")
	_expect(tool._label.text.contains("demolishes %d building" % count), "the label names the demolition: '%s'" % tool._label.text)
	for corner in lots:
		_expect(Vector2(corner.x - junction.x, corner.z - junction.z).length() < 60.0, "the outline lies at the ring: %s" % corner)
	var before := _buildings()
	var generation := simulation.get_network_render_generation()
	var click := InputEventMouseButton.new()
	click.button_index = MOUSE_BUTTON_LEFT
	click.pressed = true
	tool._unhandled_input(click)
	_expect(await _await_generation(generation), "the click builds the ring")
	_expect(_buildings() == before - count, "the click demolishes what the hover showed: %d -> %d" % [before, _buildings()])
	tool.set_tool_active(false)
	_expect(not tool._demolish_mesh.visible, "the outline goes with the tool")

func _test_road_properties(scene: Node3D) -> void:
	_load(scene)
	var on_arm := _vector(_spec["west_point"])
	var target: Dictionary = simulation.get_bulldoze_target_at(on_arm.x, on_arm.z, true)
	_expect(target.get("kind", "") == "road", "the widened arm is under its point: %s" % target)
	var edge := int(target.get("id", -1))
	var section: Dictionary = simulation.get_edge_cross_section(edge)
	var verdict: Dictionary = simulation.preview_edge_cross_section(edge, 2, 2, int(section.get("median_style", 0)),
		float(section.get("median_width", 0.0)), int(section.get("parking", 0)), int(section.get("speed_kmh", 50)),
		int(section.get("sidewalks", 3)), bool(section.get("zoning", true)), bool(section.get("street_lights", true)))
	var removed := int(verdict.get("buildings_removed", 0))
	_expect(bool(verdict.get("is_valid", false)), "the widening is valid: %s" % verdict)
	_expect(removed > 0 and int(verdict.get("demolish_count", 0)) == removed, "the widening demolishes its front rows: %s" % verdict)
	var lots: PackedVector3Array = verdict.get("demolish_lots", PackedVector3Array())
	_expect(lots.size() >= 4 and lots.size() % 4 == 0, "four corners per taken lot: %d" % lots.size())
	var window: Node = RoadPropertiesWindow.new()
	scene.add_child(window)
	window.show_verdict(verdict)
	var text: String = window.demolition_text()
	_expect(text.begins_with("Apply removes %d building" % removed), "the window asks before demolishing: '%s'" % text)
	window.free()

func _test_move(scene: Node3D) -> void:
	_load(scene)
	var tool: Node3D = MoveTool.new()
	tool.name = "MoveTool"
	scene.add_child(tool)
	tool.set_process(false)
	tool.active = true
	var start := _vector(_spec["junction"])
	var node := simulation.get_closest_node(start, 1.0)
	_expect(node >= 0, "the junction is a node")
	var target := _vector(_spec["move_to"])
	tool.selected_node_id = node
	tool.selected_node_pos = start
	tool.current_state = tool.State.DRAGGING
	var before := _buildings()
	tool._apply_move(target)
	_expect(tool.has_pending_move(), "a drag into the block is pending")
	_expect(simulation.get_node_pos(node) == start, "a pending step moves nothing")
	_expect(_buildings() == before, "a pending step demolishes nothing")
	_expect(tool._demolish_mesh != null and tool._demolish_mesh.visible, "the pending step outlines the lots")
	var count := int(tool._pending.get("demolish_count", 0))
	_expect(count > 0, "the pending step names the buildings: %s" % tool._pending)
	_expect(tool._pending_label.visible and tool._pending_label.text.begins_with("Release to demolish %d building" % count), "the label: '%s'" % tool._pending_label.text)
	var release := InputEventMouseButton.new()
	release.button_index = MOUSE_BUTTON_LEFT
	release.pressed = false
	tool._input(release)
	_expect(not tool.has_pending_move(), "the release ends the pending step")
	_expect(simulation.get_node_pos(node) == target, "the release moves the node: %s" % simulation.get_node_pos(node))
	_expect(_buildings() == before - count, "the release demolishes what the step showed: %d -> %d" % [before, _buildings()])
	_expect(not tool._demolish_mesh.visible, "the outline goes with the release")
	tool.cancel_move()
