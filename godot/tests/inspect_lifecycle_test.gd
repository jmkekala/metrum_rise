# SPDX-License-Identifier: GPL-3.0-or-later

## Isolated Inspect lifecycle regression: real input routing and windows, a minimal road stub.
## Closing a building's card also takes its ring off the map.
extends SceneTree

const InputManager = preload("res://scripts/core/input_manager.gd")
const SelectTool = preload("res://scripts/tools/select_tool.gd")
const RoadWindow = preload("res://scripts/ui/road_properties_window.gd")
const BuildingInspector = preload("res://scripts/ui/building_inspector.gd")
var _failures := 0

# Keep the real input handlers without constructing the rest of the gameplay tool graph.
class InspectInput extends InputManager:
	func _ready() -> void:
		pass
	func _process(_delta: float) -> void:
		pass

class SimulationStub extends Node3D:
	var hit := Vector3.ZERO
	func intersect_world_surface(_origin: Vector3, _direction: Vector3, _wait: bool = false) -> Vector3:
		return hit
	func get_closest_node(_point: Vector3, _radius: float) -> int:
		return -1
	func get_hovered_edge(x: float, _z: float) -> int:
		return 0 if x < 50 else -1
	func pick_road_piece(_origin: Vector3, _direction: Vector3, _wait := false) -> Variant:
		return Vector2i(get_hovered_edge(hit.x, hit.z), -1)
	func get_edge_nodes(_edge: int) -> Vector2i:
		return Vector2i(0, 1)
	func get_edge_geometry_3d(_edge: int) -> PackedVector3Array:
		return PackedVector3Array([Vector3.ZERO, Vector3(20, 0, 0)])
	func get_edge_ribbon(edge: int, _wait := false) -> Variant:
		return {"points": get_edge_geometry_3d(edge), "width": get_edge_width(edge), "lanes": PackedVector2Array()}
	func get_lane_width() -> float:
		return 3.5
	func get_edge_width(_edge: int) -> float:
		return 10.0
	func get_height_at(_point: Vector2) -> float:
		return 0.0
	func get_no_building_spawn(_edge: int) -> bool:
		return false
	func get_edge_class(_edge: int) -> int:
		return 0
	func get_edge_cross_section(_edge: int) -> Dictionary:
		return {"road_type": "test:road", "fwd_lanes": 1, "bkw_lanes": 1, "speed_kmh": 50}
	func get_road_types() -> Dictionary:
		return {"speeds": {}, "types": [{"id": "test:road", "name": "Test road", "usable": true,
			"fwd_lanes": 1, "bkw_lanes": 1, "speed_kmh": 50, "parts": []}]}
	func pick_building_at(_x: float, _z: float) -> Variant:
		return null
	func get_building_info_at(_x: float, _z: float) -> Dictionary:
		return {}

class UiStub extends Node:
	var inspector: Node
	func hide_road_properties() -> void:
		inspector.close_window()
	func show_road_properties_multi(edges: Array[int], point: Vector2) -> void:
		inspector.show_for_edges(edges, point)
	func show_junction_window(_node: int, _point: Vector2) -> void:
		pass
	func hide_junction_window() -> void:
		pass

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _key(code: Key) -> void:
	var event := InputEventKey.new()
	event.keycode = code
	event.pressed = true
	root.push_input(event, true)

func _run() -> void:
	root.gui_embed_subwindows = true
	var scene := Node3D.new()
	root.add_child(scene)
	var sim := SimulationStub.new()
	sim.name = "SimulationNode"
	scene.add_child(sim)
	var ui := UiStub.new()
	ui.name = "MainUI"
	scene.add_child(ui)
	for name in ["Terrain", "ZoningOverlay", "Water", "RoadTool", "ZoningTool", "VegetationTool", "Agents", "Buildings"]:
		var node := Node3D.new()
		node.name = name
		scene.add_child(node)
	var building := BuildingInspector.new()
	building.name = "BuildingInspector"
	scene.add_child(building)
	building.set_process(false)
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.current = true
	var tool := SelectTool.new()
	scene.add_child(tool)
	tool.set_process(false)
	var inspector := RoadWindow.new()
	inspector.setup(sim, tool)
	scene.add_child(inspector)
	ui.inspector = inspector
	var input := InspectInput.new()
	input.select_tool = tool
	input.building_inspector = building
	scene.add_child(input)
	_key(KEY_V)
	tool._set_selection([0])
	await process_frame
	_expect(inspector._window.visible and inspector._window.unfocusable, "Road inspection opens without keeping the keyboard")
	_expect(not input._ui_captures_keyboard_input(), "Road Properties must not block tool shortcuts")
	_key(KEY_ESCAPE)
	_expect(tool.selected_edges.is_empty() and not inspector._window.visible, "First Esc clears the road and window")
	_expect(input.current_tool == InputManager.Tool.SELECT, "First Esc keeps Inspect active")
	_key(KEY_ESCAPE)
	_expect(input.current_tool == InputManager.Tool.NONE, "Second Esc leaves Inspect")
	_key(KEY_V)
	tool._set_selection([0])
	_key(KEY_V)
	_expect(input.current_tool == InputManager.Tool.NONE and not inspector._window.visible, "V toggles Inspect off with a road selected")
	_key(KEY_V)
	tool._set_selection([0])
	sim.hit = Vector3(100, 0, 0)
	var click := InputEventMouseButton.new()
	click.button_index = MOUSE_BUTTON_LEFT
	click.pressed = true
	click.position = Vector2(8, 600)
	tool._unhandled_input(click)
	_expect(tool.selected_edges.is_empty() and not inspector._window.visible, "Empty ground clears road selection")
	tool._set_selection([0])
	inspector._window.close_requested.emit()
	_expect(tool.selected_edges.is_empty() and not inspector._window.visible, "Window close also clears selection")
	# A registered building window exercises the same dismissal path without constructing a city.
	for action in ["escape", "ground", "toggle"]:
		var window := Window.new()
		window.unfocusable = true
		building.add_child(window)
		building._open_windows["fixture"] = {"window": window}
		building._halo.show_footprint(PackedVector3Array([Vector3(0, 0, 0), Vector3(10, 0, 0), Vector3(10, 0, 10), Vector3(0, 0, 10)]), Color.RED)
		_expect(building._halo.visible and building._halo.mesh != null, "the inspected building is ringed")
		if action == "escape":
			_key(KEY_ESCAPE)
		elif action == "ground":
			tool._unhandled_input(click)
		else:
			_key(KEY_V)
		_expect(not building.is_inspecting() and not building._halo.visible, action + " closes the building inspector and its ring")
	tool.active = false
	scene.free()
	quit(_failures)
