# SPDX-License-Identifier: GPL-2.0-only

## The Road Properties window's road-type picker against the native bridge (`ROAD-101`,
## `ROAD-80`): the picker shows the selected road's type (or "Custom layout"); a pick shows
## Rust's verdict beside Apply (a refusal disables it, with its text: road -> walkway, walkway ->
## road, a one-way median); Apply changes the road in place to the 4-Lane Road (14 m) or a
## divided custom type created through the API (10 m with a 3 m grass median), publishes a new
## render generation, and undo restores it. The window's default size holds every row. The
## median controls kept for the road builder step within the style's range. Attributes
## (`ROAD-101`): the picker gives each type's speed and Rust's cost; Apply shows the cost
## difference; a 100 km/h type without sidewalks and zoning applies to the 1 + 1 road (speed,
## sidewalks and zoning read back, the no-build overlay refreshed) and Ctrl+Z restores it; a
## verdict that demolishes buildings asks first ("This cannot be undone"): Cancel leaves the road,
## Apply changes it. Street lights (`ROAD-101` RB3e): a 2-Lane Road type without them applies in
## place at no cost ("no street lights" in the verdict), reads back unlit as that type, and
## Ctrl+Z lights it again. Writes the road-type library, so it runs only in the isolated profile.
extends SceneTree

const SelectTool := preload("res://scripts/tools/select_tool.gd")
const RoadPropertiesWindow := preload("res://scripts/ui/road_properties_window.gd")
const MedianControls := preload("res://scripts/ui/median_controls.gd")
var _failures := 0
var simulation: SimulationNode

# The select tool's zoning overlay: counts the no-build refreshes.
class OverlayStub extends Node3D:
	var refreshed := 0
	func mark_no_build_dirty() -> void:
		refreshed += 1
	func _rebuild_no_build_overlay() -> void:
		pass

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

func _commit(points: PackedVector3Array, fwd: int = 1, bkw: int = 1) -> bool:
	var generation := simulation.get_network_render_generation()
	simulation.add_road_with_snap(points, fwd, bkw, true)
	var done := await _await_generation(generation)
	_expect(done, "fixture road must commit")
	return done

# Picks type `id` in the window's picker as the player would; false when it is not listed.
func _pick(window: Node, id: String) -> bool:
	var picker: OptionButton = window._type_picker
	for index in picker.item_count:
		if String(Dictionary(picker.get_item_metadata(index)).get("id", "")) == id:
			picker.select(index)
			picker.item_selected.emit(index)
			return true
	_expect(false, "the picker lists %s" % id)
	return false

func _picked_text(window: Node) -> String:
	return window._type_picker.get_item_text(window._type_picker.selected)

# Undoes the last edit as Ctrl+Z does and waits for its publication.
func _undo(what: String) -> void:
	var generation := simulation.get_network_render_generation()
	_expect(simulation.undo_action(), "%s: undo is queued" % what)
	_expect(await _await_generation(generation), "%s: undo publishes a new render generation" % what)

func _lanes(edge: int) -> Vector2i:
	var section: Dictionary = simulation.get_edge_cross_section(edge)
	return Vector2i(int(section.get("fwd_lanes", -1)), int(section.get("bkw_lanes", -1)))

func _run() -> void:
	# The test writes user://road_types.json: never in a real profile.
	if not OS.get_user_data_dir().to_lower().contains("godot-tests"):
		push_error("road_lane_edit_test must run in the isolated profile from ./run.sh --test")
		quit(1)
		return
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	_expect(simulation.create_blank_world(512.0, 512.0, 8.0, 128.0, 0.0), "fixture world must load")
	simulation.set_simulation_speed(0.0)
	if not await _commit(PackedVector3Array([_ground_point(-60.0, 0.0), _ground_point(60.0, 0.0)])) \
			or not await _commit(PackedVector3Array([_ground_point(0.0, 0.0), _ground_point(0.0, 60.0)])) \
			or not await _commit(PackedVector3Array([_ground_point(-60.0, -80.0), _ground_point(60.0, -80.0)]), 0, 0):
		_finish(scene)
		return
	var created: Dictionary = simulation.create_road_type("Lane Edit Boulevard", [
		{"kind": "sidewalk"}, {"kind": "car_lane", "forward": false},
		{"kind": "median", "style": 2, "width_m": 3.0},
		{"kind": "car_lane", "forward": true}, {"kind": "sidewalk"}], 50, true, true)
	var boulevard := String(created.get("id", ""))
	_expect(not boulevard.is_empty(), "the divided type is created: %s" % created)
	var ui := UiStub.new()
	ui.name = "MainUI"
	scene.add_child(ui)
	var overlay := OverlayStub.new()
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
	_expect(_picked_text(window) == "2-Lane Road (50 km/h, 15 / m)", "the picker shows the 1 + 1 road's type: %s" % _picked_text(window))
	_expect(window._layout_label.text == "1 + 1 lanes, 50 km/h", "the window shows 1 + 1 lanes: %s" % window._layout_label.text)
	var content: Control = window._window.get_child(0)
	# The window's own size is clamped to the headless viewport; its default is what a player gets.
	_expect(window.DEFAULT_SIZE.y >= int(content.get_combined_minimum_size().y), "the default height holds every row: %d < %d" % [window.DEFAULT_SIZE.y, int(content.get_combined_minimum_size().y)])
	_expect(window._apply_button.disabled, "nothing to apply before a change")

	# A road cannot become a walkway: Rust refuses and Apply stays off.
	_pick(window, "builtin:walkway")
	_expect(window._apply_button.disabled, "a refused change cannot be applied")
	_expect(window._verdict_label.text == "A road cannot become a walkway", "the refusal's text: %s" % window._verdict_label.text)

	# The 4-Lane Road: accepted, applied, published.
	_pick(window, "builtin:road_4l")
	_expect(not window._apply_button.disabled, "an accepted change can be applied")
	_expect(window._verdict_label.text == "2 + 2 lanes, 50 km/h", "the verdict: %s" % window._verdict_label.text)
	var widening := roundi(float(window._verdict.get("cost_delta", 0.0)))
	_expect(widening > 0 and window._apply_button.text == "Apply (+%s)" % window._thousands(widening),
		"Apply shows the cost difference: %s" % window._apply_button.text)
	var generation := simulation.get_network_render_generation()
	window._apply_button.pressed.emit()
	_expect(_lanes(edge) == Vector2i(2, 2), "Apply widens the road: %s" % _lanes(edge))
	_expect(is_equal_approx(float(simulation.get_edge_cross_section(edge).get("width", 0.0)), 14.0), "a 2 + 2 road is 14 m wide")
	_expect(simulation.get_network_render_generation() > generation, "the edit publishes a new render generation")
	_expect(_picked_text(window) == "4-Lane Road (50 km/h, 27 / m)" and window._apply_button.disabled and window._apply_button.text == "Apply",
		"the window shows the applied road: %s" % _picked_text(window))
	await _undo("the 4-Lane Road")
	_expect(_lanes(edge) == Vector2i(1, 1), "undo restores 1 + 1 lanes: %s" % _lanes(edge))

	# The divided custom type in place: a 3 m grass median, applied, then undone.
	window.show_for_edges(selection)
	_pick(window, boulevard)
	_expect(window._verdict_label.text == "1 + 1 lanes, Grass 3.0 m, 50 km/h", "the median verdict: %s" % window._verdict_label.text)
	generation = simulation.get_network_render_generation()
	window._apply_button.pressed.emit()
	var section: Dictionary = simulation.get_edge_cross_section(edge)
	_expect(int(section.get("median_style", -1)) == 2 and is_equal_approx(float(section.get("median_width", 0.0)), 3.0), "Apply gives the road its median: %s" % section)
	_expect(is_equal_approx(float(section.get("width", 0.0)), 10.0), "a 1 + 1 road with a 3 m median is 10 m wide")
	_expect(String(section.get("road_type", "")) == boulevard, "the road reads back as the custom type: %s" % section.get("road_type", ""))
	_expect(simulation.get_network_render_generation() > generation, "the median edit publishes")
	_expect(_picked_text(window) == "Lane Edit Boulevard (50 km/h, 21 / m)", "the picker shows the custom type: %s" % _picked_text(window))
	# One way with the median: no type can ask for it (the layout check refuses it), and the
	# in-place check still refuses it with its reason.
	_expect(simulation.check_road_layout([{"kind": "sidewalk"}, {"kind": "median", "style": 2, "width_m": 3.0},
		{"kind": "car_lane", "forward": true}, {"kind": "sidewalk"}], 50, true, true).get("invalid_reason", "") != "", "a one-way median layout is refused")
	window.show_verdict(tool.preview_selected_edge_cross_section(1, 0, 2, 3.0, 50, true, true, true))
	_expect(window._apply_button.disabled, "a one-way median cannot be applied")
	_expect(window._verdict_label.text == "A median needs lanes in both directions", "the one-way refusal: %s" % window._verdict_label.text)
	await _undo("the median")
	_expect(int(simulation.get_edge_cross_section(edge).get("median_style", -1)) == 0, "undo removes the median")

	# A layout no type has shows as "Custom layout", which cannot be picked.
	_expect(simulation.set_edge_cross_section(edge, 3, 1, 0, 0.0, 50, true, true, true), "a 3 + 1 layout is set in place")
	window.show_for_edges(selection)
	_expect(_picked_text(window) == window.CUSTOM_LAYOUT and window._type_picker.is_item_disabled(window._type_picker.selected), "a 3 + 1 road shows Custom layout: %s" % _picked_text(window))
	_expect(window._layout_label.text == "3 + 1 lanes, 50 km/h", "the custom layout's lanes: %s" % window._layout_label.text)
	await _undo("the custom layout")

	# A 100 km/h type without sidewalks or zoning in place: nothing to demolish, applied at once.
	var rural := String(simulation.create_road_type("Lane Edit Rural", [
		{"kind": "car_lane", "forward": false}, {"kind": "car_lane", "forward": true}], 100, false, true).get("id", ""))
	_expect(not rural.is_empty(), "the rural type is created")
	window.show_for_edges(selection)
	_pick(window, rural)
	var dialog: ConfirmationDialog = window._demolish_dialog
	_expect(not window._apply_button.disabled and window.demolition_text().is_empty(), "no buildings: no question")
	_expect(window._verdict_label.text == "1 + 1 lanes, 100 km/h, no sidewalks", "the rural verdict: %s" % window._verdict_label.text)
	var refreshed: int = overlay.refreshed
	window._apply_button.pressed.emit()
	section = simulation.get_edge_cross_section(edge)
	_expect(not dialog.visible and int(section.get("speed_kmh", 0)) == 100 and not bool(section.get("sidewalks", true))
		and not bool(section.get("zoning", true)) and String(section.get("road_type", "")) == rural, "Apply makes it rural: %s" % section)
	_expect(overlay.refreshed > refreshed and window._no_build_check.button_pressed, "zoning off refreshes the overlay and the checkbox")
	_expect(_picked_text(window) == "Lane Edit Rural (100 km/h, 24 / m)", "the picker shows the rural type: %s" % _picked_text(window))
	await _undo("the rural type")
	section = simulation.get_edge_cross_section(edge)
	_expect(int(section.get("speed_kmh", 0)) == 50 and bool(section.get("sidewalks", false)) and bool(section.get("zoning", false)),
		"undo restores 50 km/h, sidewalks and zoning: %s" % section)

	# A verdict that demolishes buildings asks first. Rust's count is its own test
	# (`zoning_off_in_place_removes_the_roads_parcels_and_buildings`); here the window's answer to it.
	window.show_for_edges(selection)
	_pick(window, rural)
	var demolishing: Dictionary = window._verdict.duplicate()
	demolishing.buildings_removed = 3
	demolishing.demolition_permanent = true
	window.show_verdict(demolishing)
	window._apply_button.pressed.emit()
	await process_frame
	_expect(dialog.visible and dialog.dialog_text == "Apply removes 3 buildings. This cannot be undone.", "Apply asks: %s" % dialog.dialog_text)
	dialog.canceled.emit()
	dialog.hide()
	_expect(int(simulation.get_edge_cross_section(edge).get("speed_kmh", 0)) == 50, "Cancel leaves the road as it was")
	window._apply_button.pressed.emit()
	await process_frame
	_expect(dialog.visible, "Apply asks again")
	dialog.confirmed.emit()
	dialog.hide()
	_expect(int(simulation.get_edge_cross_section(edge).get("speed_kmh", 0)) == 100, "the confirmed Apply changes the road")
	await _undo("the confirmed rural type")
	_expect(simulation.delete_road_type(rural).is_empty(), "the rural type is deleted")

	# Street lights off in place: the same lanes, speed and sidewalks, no cost, undo lights it.
	var unlit := String(simulation.create_road_type("Lane Edit Unlit", [{"kind": "sidewalk"},
		{"kind": "car_lane", "forward": false}, {"kind": "car_lane", "forward": true}, {"kind": "sidewalk"}],
		50, true, false).get("id", ""))
	_expect(not unlit.is_empty(), "the unlit type is created")
	window.show_for_edges(selection)
	_expect(_picked_text(window).begins_with("2-Lane Road"), "the road is a 2-Lane Road: %s" % _picked_text(window))
	_pick(window, unlit)
	_expect(not window._apply_button.disabled and window._apply_button.text == "Apply"
		and window._verdict_label.text == "1 + 1 lanes, 50 km/h, no street lights",
		"lights off costs nothing: %s / %s" % [window._apply_button.text, window._verdict_label.text])
	window._apply_button.pressed.emit()
	section = simulation.get_edge_cross_section(edge)
	_expect(not bool(section.get("street_lights", true)) and bool(section.get("sidewalks", false)) and int(section.get("speed_kmh", 0)) == 50
		and bool(section.get("zoning", false)) and String(section.get("road_type", "")) == unlit, "Apply turns its lights off: %s" % section)
	await _undo("the unlit type")
	section = simulation.get_edge_cross_section(edge)
	_expect(bool(section.get("street_lights", false)) and String(section.get("road_type", "")) == "builtin:road_2l",
		"undo lights it again: %s" % section)
	_expect(simulation.delete_road_type(unlit).is_empty(), "the unlit type is deleted")

	# A walkway keeps the picker enabled so the refusal shows.
	var walkway: int = simulation.get_hovered_edge(0.0, -80.0)
	_expect(walkway >= 0 and _lanes(walkway) == Vector2i(0, 0), "the walkway is found")
	var walkway_selection: Array[int] = [walkway]
	tool.selected_edges = walkway_selection
	window.show_for_edges(walkway_selection)
	_expect(_picked_text(window) == "Walkway (3.75 / m)" and not window._type_picker.disabled, "a walkway shows its type with the picker enabled: %s" % _picked_text(window))
	_pick(window, "builtin:road_2l")
	_expect(window._apply_button.disabled, "a walkway cannot become a road")
	_expect(window._verdict_label.text == "A walkway cannot become a road: draw a road in its place", "the walkway refusal: %s" % window._verdict_label.text)

	# The median controls the road builder reuses step within the style's range.
	var controls := MedianControls.new()
	controls.median_styles = simulation.get_median_styles()
	scene.add_child(controls)
	var seen: Array = []
	controls.changed.connect(func(style: int, width: float): seen.append([style, width]))
	controls.set_median(0, 0.0)
	_expect(seen.is_empty(), "set_median shows without emitting")
	_expect(controls._width_buttons[1].disabled, "no width to step without a median")
	controls._style_buttons[1].pressed.emit()
	controls._width_buttons[1].pressed.emit()
	controls._width_buttons[1].pressed.emit()
	_expect(seen.back() == [1, 2.0], "painted steps up to its 2 m limit: %s" % [seen.back()])

	_expect(simulation.delete_road_type(boulevard).is_empty(), "the fixture type is deleted")
	tool.active = false
	_finish(scene)

func _finish(scene: Node) -> void:
	scene.free()
	if _failures == 0:
		print("road_lane_edit_test: PASS")
	quit(_failures)
