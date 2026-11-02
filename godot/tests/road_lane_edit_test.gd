# SPDX-License-Identifier: GPL-2.0-only

## The Road Properties window's cross-section controls against the native bridge (`ROAD-80`):
## stepping a selected road's lanes shows Rust's verdict beside Apply (a refusal disables it,
## with its text); Apply widens the road in place, publishes a new render generation and the road
## reports its new lanes; undo restores them. A median style picked in the window is applied in
## place the same way, refused on a one-way road, and undone. The window's default size holds
## every row. The road options panel's controls follow the tool.
extends SceneTree

const SelectTool := preload("res://scripts/tools/select_tool.gd")
const RoadPropertiesWindow := preload("res://scripts/ui/road_properties_window.gd")
const CrossSectionControls := preload("res://scripts/ui/cross_section_controls.gd")
var _failures := 0
var simulation: SimulationNode

class UiStub extends Node:
	func hide_road_properties() -> void:
		pass
	func show_road_properties_multi(_edges: Array[int], _point: Vector2) -> void:
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

# Presses the window's stepper `index` (forward -, forward +, backward -, backward +).
func _step(window: Node, index: int) -> void:
	window._lanes._buttons[index].pressed.emit()

func _lanes(edge: int) -> Vector2i:
	var section: Dictionary = simulation.get_edge_cross_section(edge)
	return Vector2i(int(section.get("fwd_lanes", -1)), int(section.get("bkw_lanes", -1)))

func _run() -> void:
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "fixture world must load")
	simulation.set_simulation_speed(0.0)
	if not await _commit(PackedVector3Array([_ground_point(-60.0, 0.0), _ground_point(60.0, 0.0)])) \
			or not await _commit(PackedVector3Array([_ground_point(0.0, 0.0), _ground_point(0.0, 60.0)])):
		_finish(scene)
		return
	var ui := UiStub.new()
	ui.name = "MainUI"
	scene.add_child(ui)
	var overlay := Node3D.new()
	overlay.name = "ZoningOverlay"
	scene.add_child(overlay)
	var tool = SelectTool.new()
	scene.add_child(tool)
	tool.set_process(false)
	tool.active = true
	var edge: int = simulation.get_hovered_edge(0.0, 40.0)
	_expect(edge >= 0, "the side road is found")
	var selection: Array[int] = [edge]
	tool.selected_edges = selection
	var window = RoadPropertiesWindow.new()
	window.setup(simulation, tool)
	scene.add_child(window)
	window.show_for_edges(selection)
	await process_frame
	_expect(window._lanes.fwd_lanes == 1 and window._lanes.bkw_lanes == 1, "the window shows 1 + 1 lanes")
	var content: Control = window._window.get_child(0)
	# The window's own size is clamped to the headless viewport; its default is what a player gets.
	_expect(window.DEFAULT_SIZE.y >= int(content.get_combined_minimum_size().y), "the default height holds every row: %d < %d" % [window.DEFAULT_SIZE.y, int(content.get_combined_minimum_size().y)])
	_expect(window._apply_button.disabled, "nothing to apply before a change")

	# Down to 0 + 0: a road keeps a lane; Rust refuses and Apply stays off.
	_step(window, 0)
	_step(window, 2)
	_expect(window._apply_button.disabled, "a refused change cannot be applied")
	_expect(window._verdict_label.text == "A road keeps at least one lane", "the refusal's text: %s" % window._verdict_label.text)

	# Up to 2 + 2: accepted, applied, published.
	for index in [1, 1, 3, 3]:
		_step(window, index)
	_expect(not window._apply_button.disabled, "an accepted change can be applied")
	_expect(window._verdict_label.text == "2 + 2 lanes", "the verdict: %s" % window._verdict_label.text)
	var generation := simulation.get_network_render_generation()
	window._apply_button.pressed.emit()
	_expect(_lanes(edge) == Vector2i(2, 2), "Apply widens the road: %s" % _lanes(edge))
	_expect(is_equal_approx(float(simulation.get_edge_cross_section(edge).get("width", 0.0)), 14.0), "a 2 + 2 road is 14 m wide")
	_expect(simulation.get_network_render_generation() > generation, "the edit publishes a new render generation")
	_expect(window._lanes.fwd_lanes == 2 and window._apply_button.disabled, "the window shows the applied road")

	generation = simulation.get_network_render_generation()
	_expect(simulation.undo_action(), "undo is queued")
	_expect(await _await_generation(generation), "undo publishes a new render generation")
	_expect(_lanes(edge) == Vector2i(1, 1), "undo restores 1 + 1 lanes: %s" % _lanes(edge))

	# A median in place: Grass at its 3 m default, applied, then undone.
	window.show_for_edges(selection)
	window._lanes._style_buttons[2].pressed.emit()
	_expect(window._verdict_label.text == "1 + 1 lanes, Grass 3.0 m", "the median verdict: %s" % window._verdict_label.text)
	generation = simulation.get_network_render_generation()
	window._apply_button.pressed.emit()
	var section: Dictionary = simulation.get_edge_cross_section(edge)
	_expect(int(section.get("median_style", -1)) == 2 and is_equal_approx(float(section.get("median_width", 0.0)), 3.0), "Apply gives the road its median: %s" % section)
	_expect(is_equal_approx(float(section.get("width", 0.0)), 10.0), "a 1 + 1 road with a 3 m median is 10 m wide")
	_expect(simulation.get_network_render_generation() > generation, "the median edit publishes")
	# One way with the median: Rust refuses it with its reason.
	_step(window, 2)
	_expect(window._apply_button.disabled, "a one-way median cannot be applied")
	_expect(window._verdict_label.text == "A median needs lanes in both directions", "the one-way refusal: %s" % window._verdict_label.text)
	generation = simulation.get_network_render_generation()
	_expect(simulation.undo_action(), "the median's undo is queued")
	_expect(await _await_generation(generation), "the median's undo publishes")
	_expect(int(simulation.get_edge_cross_section(edge).get("median_style", -1)) == 0, "undo removes the median")

	# The road options panel's steppers drive the tool's next stroke.
	var controls := CrossSectionControls.new()
	scene.add_child(controls)
	var seen: Array = []
	controls.changed.connect(func(fwd: int, bkw: int, style: int, width: float): seen.append([fwd, bkw, style, width]))
	controls.set_lanes(1, 0)
	controls.set_median(0, 0.0)
	_expect(seen.is_empty(), "set_lanes and set_median show without emitting")
	_expect(controls._width_buttons[1].disabled, "no width to step without a median")
	controls._buttons[3].pressed.emit()
	_expect(seen == [[1, 1, 0, 0.0]], "a step emits the cross-section: %s" % [seen])
	controls._style_buttons[1].pressed.emit()
	controls._width_buttons[1].pressed.emit()
	controls._width_buttons[1].pressed.emit()
	_expect(seen.back() == [1, 1, 1, 2.0], "painted steps up to its 2 m limit: %s" % [seen.back()])

	tool.active = false
	_finish(scene)

func _finish(scene: Node) -> void:
	scene.free()
	if _failures == 0:
		print("road_lane_edit_test: PASS")
	quit(_failures)
