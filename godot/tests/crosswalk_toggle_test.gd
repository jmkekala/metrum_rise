# SPDX-License-Identifier: GPL-2.0-only

## The select tool's crossing toggle against the native bridge (`ROAD-74`): a click on an arm's
## toggle turns its zebra off, the next render generation's road chunks, installed by the road
## tool, carry less crossing paint and the arm reports no crossing; undo restores the paint. The
## user's play-test found the paint staying drawn because the toggle published no generation.
extends SceneTree

const SelectTool := preload("res://scripts/tools/select_tool.gd")
const RoadToolScript := preload("res://scripts/tools/road_tool.gd")
const WorldMaterialsScript := preload("res://scripts/renderers/world_materials.gd")
var _failures := 0
var simulation: SimulationNode

class UiStub extends Node:
	func hide_road_properties() -> void:
		pass
	func show_road_properties_multi(_edges: Array[int], _point: Vector2) -> void:
		pass

class TerrainStub extends Node3D:
	pass

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

# Vertices of the installed road chunks' road layers that carry crossing paint (CUSTOM0.z > 0, as
# the road shader reads it).
func _painted_vertices(road_tool: Node3D) -> int:
	var painted := 0
	for instance: MeshInstance3D in road_tool._road_chunk_instances.values():
		var mesh: ArrayMesh = instance.mesh
		if mesh == null:
			continue
		for surface in mesh.get_surface_count():
			var crossings = mesh.surface_get_arrays(surface)[Mesh.ARRAY_CUSTOM0]
			if crossings == null:
				continue
			for index in range(2, crossings.size(), 4):
				if crossings[index] > 0.0:
					painted += 1
	return painted

# The road tool installs the current render generation; returns whether it did.
func _install(road_tool: Node3D) -> bool:
	var generation := simulation.get_network_render_generation()
	return road_tool.update_main_mesh(generation) == generation

func _click(camera: Camera3D, world: Vector3) -> void:
	for pressed in [true, false]:
		var event := InputEventMouseButton.new()
		event.button_index = MOUSE_BUTTON_LEFT
		event.pressed = pressed
		event.position = camera.unproject_position(world)
		root.push_input(event, true)

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "fixture world must load")
	simulation.set_simulation_speed(0.0)
	if not await _commit(PackedVector3Array([_ground_point(-60.0, 0.0), _ground_point(60.0, 0.0)])) \
			or not await _commit(PackedVector3Array([_ground_point(0.0, -60.0), _ground_point(0.0, 60.0)])):
		_finish(scene)
		return
	var ui := UiStub.new()
	ui.name = "MainUI"
	scene.add_child(ui)
	var overlay := Node3D.new()
	overlay.name = "ZoningOverlay"
	scene.add_child(overlay)
	# The road tool renders the chunks; it stays out of the tree so the select tool's own
	# refresh call is left to the renderer's generation poll, as in the game.
	var road_tool := RoadToolScript.new()
	road_tool.name = "RoadTool"
	road_tool.simulation_node = simulation
	road_tool.terrain_node = TerrainStub.new()
	road_tool.road_mesh_root = Node3D.new()
	road_tool.add_child(road_tool.road_mesh_root)
	road_tool.blueprint_mesh = MeshInstance3D.new()
	road_tool.add_child(road_tool.blueprint_mesh)
	road_tool._info_label = Label.new()
	road_tool.add_child(road_tool._info_label)
	road_tool._road_preview_material = WorldMaterialsScript.road_preview_material()
	_expect(_install(road_tool), "the cross's chunks must install")
	var painted_before := _painted_vertices(road_tool)
	_expect(painted_before > 0, "the cross is drawn with zebras")

	var tool = SelectTool.new()
	scene.add_child(tool)
	tool.set_process(false)
	tool.active = true
	var node: int = simulation.get_closest_node(Vector3.ZERO, 8.0)
	_expect(node >= 0, "the cross has a junction node")
	tool.selected_node = node
	tool._build_node_visuals(node)
	_expect(tool.crosswalk_toggles.size() == 4, "every arm of the cross has a toggle: %d" % tool.crosswalk_toggles.size())
	var toggle: Dictionary = tool.crosswalk_toggles[0]
	_expect(toggle.has_cw, "the arm crosses before the toggle")
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.position = toggle.pos + Vector3(0.0, 30.0, 0.01)
	camera.look_at(toggle.pos)
	camera.current = true
	await process_frame

	var generation := simulation.get_network_render_generation()
	_click(camera, toggle.pos)
	await process_frame
	_expect(not simulation.has_crosswalk(node, toggle.edge_id), "the click turns the arm's crossing off")
	_expect(simulation.get_network_render_generation() > generation, "the toggle publishes a new render generation")
	_expect(simulation.is_network_dirty(), "the renderer is told to refresh")
	_expect(_install(road_tool), "the toggle's chunks install")
	var painted_off := _painted_vertices(road_tool)
	_expect(painted_off < painted_before, "the arm's zebra paint is gone from the drawn chunks: %d -> %d" % [painted_before, painted_off])
	_expect(not tool.crosswalk_toggles[0].has_cw, "the toggle shows the arm off")

	generation = simulation.get_network_render_generation()
	_expect(simulation.undo_action(), "undo is queued")
	_expect(await _await_generation(generation), "undo publishes a new render generation")
	_expect(simulation.has_crosswalk(node, toggle.edge_id), "undo restores the arm's crossing")
	_expect(_install(road_tool), "the undo's chunks install")
	_expect(_painted_vertices(road_tool) == painted_before, "undo restores the zebra paint: %d" % _painted_vertices(road_tool))
	tool.active = false
	road_tool.free()
	_finish(scene)

func _finish(scene: Node) -> void:
	scene.free()
	if _failures == 0:
		print("crosswalk_toggle_test: PASS")
	quit(_failures)
