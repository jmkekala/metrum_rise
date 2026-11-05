# SPDX-License-Identifier: GPL-2.0-only

## Floating road-properties window for SelectTool edge editing.
##
## The window mirrors the previous side panel behavior but uses Godot's built-in
## draggable Window surface and preserves its placement within the session.
## Road type (`ROAD-101`, `ROAD-80`): a picker lists the usable road types, each with its speed
## and Rust's build cost per metre, and shows the selected road's (Rust's `road_type` for its
## cross-section, sidewalks and speed, or "Custom layout" when no type matches; zoning is not
## matched). A pick asks Rust's verdict for the type's lanes, median, speed, sidewalks and zoning
## on every selected road through the select tool (the in-place edit's check, no geometry
## preview) and shows it beside Apply, which reads "Apply (+1,230)" with the selection's cost
## difference from Rust (charged, never refunded) and commits an accepted change (Ctrl+Z undoes
## the roads). When zoning off would demolish buildings, Apply first asks in a confirmation
## ("Apply removes N buildings. This cannot be undone.": undo brings back the road, never the
## buildings); Cancel leaves the roads as they are. The picker stays enabled on a walkway so the
## refusal (walkway <-> road) shows with its text. The window opens tall enough for every row.
## Turn pocket (`ROAD-95`, D12; widgets in `road_pocket_section.gd`): with one car road selected,
## the section edits the end nearest the click (highlighted by the select tool): Left / Right and
## the length from the junction ask Rust's verdict (no geometry preview, D11) and Apply commits it
## ("Apply (+N)", Ctrl+Z undoes). After an edit or an undo (a new render generation) the section
## reads the end again, following the road under the click when the edit split or joined it.
## Rust methods called: get_road_types(), get_edge_cross_section(), get_edge_class(),
## get_no_building_spawn(), get_edge_geometry_3d(), get_height_at(),
## get_network_render_generation(); through the select tool get_road_pocket(),
## nearest_road_pocket_end(), preview_road_pocket(), set_road_pocket().
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")
const RoadTypeMenu = preload("res://scripts/ui/road_type_menu.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const RoadPocketSection = preload("res://scripts/ui/road_pocket_section.gd")
## The picker's text for a road whose cross-section no type has.
const CUSTOM_LAYOUT := "Custom layout"
const VERDICT_READY := Color(0.55, 0.95, 0.55)
const VERDICT_REFUSED := Color.YELLOW
# Every row fits at the default size: the class buttons, the road type and its layout, the
# verdict with Apply, No buildings and the Turn pocket section.
const DEFAULT_SIZE := Vector2i(400, 600)
const MIN_SIZE := Vector2i(360, 540)

var _simulation_node: Node
var _select_tool: Node

var _window: Window
var _warning_label: Label
var _no_build_check: CheckBox
var _class_buttons: Array[Button] = []
var _type_picker: OptionButton
var _layout_label: Label
var _verdict_label: Label
var _apply_button: Button
var _demolish_dialog: ConfirmationDialog
var _pocket_separator: HSeparator
var _pocket_section: VBoxContainer
# The road end the pocket section shows (the select tool's `selected_pocket_end`), and the
# render generation it was read at.
var _pocket_info: Dictionary = {}
var _pocket_generation := -1
# The select tool's verdict on the picked type (`preview_selected_edge_cross_section`).
var _verdict: Dictionary = {}
# Rust's speed grid (`get_road_types().speeds`), for a walkway's attributes.
var _speeds: Dictionary = {}
var _selected_edges: Array[int] = []
var _last_screen_pos := Vector2.ZERO

func setup(simulation_node: Node, select_tool: Node) -> void:
	_simulation_node = simulation_node
	_select_tool = select_tool

func _ready() -> void:
	_build_window()

func _build_window() -> void:
	_window = Window.new()
	_window.title = "Road Properties"
	_window.unresizable = false
	_window.exclusive = false
	_window.visible = false
	_window.close_requested.connect(_window.hide)
	UIStyle.set_persistent_window_layout(
		_window,
		"road_properties",
		DEFAULT_SIZE,
		MIN_SIZE,
		get_viewport()
	)

	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	_window.add_child(body)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_right", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_top", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_bottom", UIStyle.PAD_WINDOW)
	body.add_child(margin)

	var root_vbox := VBoxContainer.new()
	root_vbox.add_theme_constant_override("separation", 10)
	margin.add_child(root_vbox)

	_warning_label = Label.new()
	_warning_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_warning_label.add_theme_color_override("font_color", Color.YELLOW)
	UIStyle.set_font_size(_warning_label, 13)
	root_vbox.add_child(_warning_label)

	for i in range(3):
		var button := Button.new()
		button.toggle_mode = true
		button.text = ["Standard", "Bridge", "Tunnel"][i]
		UIStyle.set_font_size(button, 14)
		button.pressed.connect(_on_class_button_pressed.bind(i))
		root_vbox.add_child(button)
		_class_buttons.append(button)

	root_vbox.add_child(HSeparator.new())

	var type_row := HBoxContainer.new()
	var type_label := Label.new()
	type_label.text = "Road type"
	type_label.custom_minimum_size.x = 92.0
	UIStyle.set_font_size(type_label, 14)
	type_row.add_child(type_label)
	_type_picker = OptionButton.new()
	_type_picker.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_type_picker.fit_to_longest_item = false
	UIStyle.set_font_size(_type_picker, 14)
	_type_picker.item_selected.connect(_on_type_selected)
	type_row.add_child(_type_picker)
	root_vbox.add_child(type_row)
	_layout_label = Label.new()
	_layout_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	UIStyle.set_font_size(_layout_label, 13)
	root_vbox.add_child(_layout_label)
	var apply_row := HBoxContainer.new()
	_verdict_label = Label.new()
	_verdict_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_verdict_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(_verdict_label, 13)
	apply_row.add_child(_verdict_label)
	_apply_button = Button.new()
	_apply_button.text = "Apply"
	_apply_button.disabled = true
	UIStyle.set_font_size(_apply_button, 14)
	_apply_button.pressed.connect(_on_apply_pressed)
	apply_row.add_child(_apply_button)
	root_vbox.add_child(apply_row)

	root_vbox.add_child(HSeparator.new())

	_no_build_check = CheckBox.new()
	_no_build_check.text = "No buildings"
	UIStyle.set_font_size(_no_build_check, 14)
	_no_build_check.toggled.connect(_on_no_build_toggled)
	root_vbox.add_child(_no_build_check)

	_pocket_separator = HSeparator.new()
	root_vbox.add_child(_pocket_separator)
	_pocket_section = RoadPocketSection.new(14)
	_pocket_section.changed.connect(_on_pocket_changed)
	_pocket_section.apply_pressed.connect(_on_pocket_apply)
	root_vbox.add_child(_pocket_section)

	_demolish_dialog = ConfirmationDialog.new()
	_demolish_dialog.title = "Remove buildings"
	_demolish_dialog.ok_button_text = "Apply"
	_demolish_dialog.confirmed.connect(_apply)
	_window.add_child(_demolish_dialog)

	add_child(_window)
	WindowResizeHandles.install(_window)
	_window.hide()

func show_for_edges(edge_indices: Array[int], screen_pos: Vector2 = Vector2.ZERO) -> void:
	_selected_edges = edge_indices.duplicate()
	if _selected_edges.is_empty():
		close_window()
		return
	_refresh()
	_last_screen_pos = screen_pos
	if not _window.has_meta("opened_once") and not UIStyle.has_persistent_window_position(_window):
		_place_window_near_cursor(screen_pos)
	if not _window.has_meta("opened_once"):
		UIStyle.popup_persistent_window(_window, false)
		_window.set_meta("opened_once", true)
	else:
		_window.show()
		_window.grab_focus()

func close_window() -> void:
	if _window:
		UIStyle.save_persistent_window_layout(_window)
		_window.hide()
	if _select_tool and _select_tool.has_method("clear_pocket_end"):
		_select_tool.clear_pocket_end()

func _place_window_near_cursor(screen_pos: Vector2) -> void:
	UIStyle.place_window_near_cursor(_window, screen_pos, get_viewport())

func _refresh() -> void:
	if _selected_edges.is_empty() or _simulation_node == null:
		return

	_refresh_type()
	refresh_pocket()
	if _selected_edges.size() == 1:
		var edge_idx := _selected_edges[0]
		_warning_label.text = ""
		_no_build_check.set_pressed_no_signal(_simulation_node.get_no_building_spawn(edge_idx))
		var current_class: int = _simulation_node.get_edge_class(edge_idx)
		for i in range(_class_buttons.size()):
			_class_buttons[i].set_pressed_no_signal(i == current_class)

		var geometry = _simulation_node.get_edge_geometry_3d(edge_idx)
		if geometry.size() >= 2:
			var p0 = geometry[0]
			var p1 = geometry[-1]
			var h0 = _simulation_node.get_height_at(Vector2(p0.x, p0.z))
			var h1 = _simulation_node.get_height_at(Vector2(p1.x, p1.z))
			var midpoint = geometry[geometry.size() / 2]
			var h_mid = _simulation_node.get_height_at(Vector2(midpoint.x, midpoint.z))
			if p0.y > h0 + 1.0 and p1.y > h1 + 1.0:
				if midpoint.y < h_mid + 2.0:
					_warning_label.text = "Warning: Bridge may clash with terrain!"
			elif p0.y < h0 - 1.0 and p1.y < h1 - 1.0:
				if midpoint.y > h_mid - 2.0:
					_warning_label.text = "Warning: Tunnel might be above surface!"
	else:
		_warning_label.text = "%d edges selected" % _selected_edges.size()
		var all_no_build := _selected_edges.all(
			func(idx): return _simulation_node.get_no_building_spawn(idx)
		)
		_no_build_check.set_pressed_no_signal(all_no_build)
		var first_class: int = _simulation_node.get_edge_class(_selected_edges[0])
		var shared_class := _selected_edges.all(
			func(idx): return _simulation_node.get_edge_class(idx) == first_class
		)
		for i in range(_class_buttons.size()):
			_class_buttons[i].set_pressed_no_signal(shared_class and i == first_class)

## Lists the road types again (after a library edit) while the window shows a road.
func refresh_types() -> void:
	if _window and _window.visible and not _selected_edges.is_empty():
		_refresh_type()

# The picker lists the usable types with the first selected road's marked, or "Custom layout"
# (not pickable) when no type has its cross-section. Every item's metadata is its type.
func _refresh_type() -> void:
	var first: Dictionary = _simulation_node.get_edge_cross_section(_selected_edges[0])
	var current := String(first.get("road_type", ""))
	_type_picker.clear()
	var library: Dictionary = _simulation_node.get_road_types()
	_speeds = library.get("speeds", {})
	if current.is_empty():
		_type_picker.add_item(CUSTOM_LAYOUT)
		_type_picker.set_item_disabled(0, true)
		_type_picker.set_item_metadata(0, {})
	for type: Dictionary in library.get("types", []):
		if not bool(type.get("usable", false)):
			continue
		_type_picker.add_item(RoadTypeMenu.picker_text(type))
		_type_picker.set_item_metadata(_type_picker.item_count - 1, type)
		if String(type.get("id", "")) == current:
			_type_picker.select(_type_picker.item_count - 1)
	if current.is_empty():
		_type_picker.select(0)
	_layout_label.text = RoadTypeMenu.describe(first, false)
	_verdict_label.text = ""
	_verdict = {}
	_apply_button.text = "Apply"
	_apply_button.disabled = true

# The type picked in the picker, or {} for "Custom layout".
func _picked_type() -> Dictionary:
	var index := _type_picker.selected
	return _type_picker.get_item_metadata(index) if index >= 0 else {}

# The in-place arguments for `type`: lanes, median, then speed, sidewalks, zoning and street lights
# (`RoadTypeMenu.tool_attributes`: a walkway sends the built-ins').
func _type_arguments(type: Dictionary) -> Array:
	return [int(type.get("fwd_lanes", 0)), int(type.get("bkw_lanes", 0)), int(type.get("median_style", 0)),
		float(type.get("median_width", 0.0))] + RoadTypeMenu.tool_attributes(type, _speeds)

func _on_type_selected(_index: int) -> void:
	var type := _picked_type()
	if type.is_empty() or _select_tool == null or not _select_tool.has_method("preview_selected_edge_cross_section"):
		return
	show_verdict(_select_tool.callv("preview_selected_edge_cross_section", _type_arguments(type)))

## Shows Rust's verdict on the picked type and enables Apply when it accepts it; Apply shows the
## cost difference Rust charges ("Apply (+1,230)").
func show_verdict(verdict: Dictionary) -> void:
	_verdict = verdict
	var ready := bool(verdict.get("is_valid", false))
	_apply_button.disabled = not ready
	var cost := roundi(float(verdict.get("cost_delta", 0.0))) if ready else 0
	_apply_button.text = "Apply (+%s)" % _thousands(cost) if cost > 0 else "Apply"
	_verdict_label.add_theme_color_override("font_color", VERDICT_READY if ready else VERDICT_REFUSED)
	_verdict_label.text = RoadTypeMenu.describe(_picked_type(), false) if ready \
		else RoadReasonText.text(verdict, "This road cannot change its type")

# 1230 -> "1,230".
static func _thousands(value: int) -> String:
	var digits := str(absi(value))
	var text := ""
	while digits.length() > 3:
		text = "," + digits.substr(digits.length() - 3) + text
		digits = digits.substr(0, digits.length() - 3)
	return ("-" if value < 0 else "") + digits + text

## The confirmation Apply asks for, or "" when the change demolishes nothing.
func demolition_text() -> String:
	var count := int(_verdict.get("buildings_removed", 0))
	if count <= 0:
		return ""
	return "Apply removes %d building%s. This cannot be undone." % [count, "" if count == 1 else "s"]

func _on_apply_pressed() -> void:
	var question := demolition_text()
	if question.is_empty():
		_apply()
		return
	_demolish_dialog.dialog_text = question
	_demolish_dialog.popup_centered()

# Commits the picked type on the selection (after the confirmation when it demolishes).
func _apply() -> void:
	var type := _picked_type()
	if not type.is_empty() and _select_tool and _select_tool.has_method("set_selected_edge_cross_section"):
		var refused: int = _select_tool.callv("set_selected_edge_cross_section", _type_arguments(type))
		_refresh()
		if refused > 0:
			_verdict_label.add_theme_color_override("font_color", VERDICT_REFUSED)
			_verdict_label.text = "%d road(s) refused the change" % refused

func _on_class_button_pressed(class_index: int) -> void:
	if _select_tool and _select_tool.has_method("set_selected_edge_class"):
		_select_tool.set_selected_edge_class(class_index)
	_refresh()

func _on_no_build_toggled(enabled: bool) -> void:
	if _select_tool and _select_tool.has_method("set_selected_edge_no_building_spawn"):
		_select_tool.set_selected_edge_no_building_spawn(enabled)

## Reads the selected road end again (`selected_pocket_end`) and shows it, or hides the section
## when it has none (several roads, a junction click, a walkway: no car road). Resets the
## request to the end's pocket.
func refresh_pocket() -> void:
	_pocket_info = _select_tool.selected_pocket_end() if _select_tool and _select_tool.has_method("selected_pocket_end") else {}
	_pocket_generation = _simulation_node.get_network_render_generation() if _simulation_node else -1
	# A road that is no car road (a walkway) refuses both roles as `invalid_road`.
	var shown: bool = not _pocket_info.is_empty() and not (_pocket_info.get("left_refusal", "") == "invalid_road" \
		and _pocket_info.get("right_refusal", "") == "invalid_road")
	_pocket_separator.visible = shown
	_pocket_section.visible = shown
	if not shown:
		if _select_tool and _select_tool.has_method("clear_pocket_end"):
			_select_tool.clear_pocket_end()
		return
	_pocket_section.show_end(_pocket_info)
	_select_tool.show_pocket_end(_pocket_info, float(_pocket_section.request()[2]))

# After an edit or an undo publishes (a new render generation), the road under the click and its
# end are read again; the player's unapplied request stays unless the end changed.
func _process(_delta: float) -> void:
	if _window == null or not _window.visible or _pocket_info.is_empty() or _simulation_node == null:
		return
	var generation: int = _simulation_node.get_network_render_generation()
	if generation == _pocket_generation:
		return
	_pocket_generation = generation
	if _select_tool.follow_click_selection():
		_selected_edges = _select_tool.selected_edges.duplicate()
		_refresh()
		return
	var info: Dictionary = _select_tool.selected_pocket_end()
	if info != _pocket_info:
		refresh_pocket()

func _on_pocket_changed(left: bool, right: bool, length_m: float) -> void:
	if _pocket_info.is_empty():
		return
	_select_tool.show_pocket_end(_pocket_info, length_m)
	var verdict: Dictionary = _select_tool.preview_selected_road_pocket(_pocket_info, left, right, length_m)
	var cost := roundi(float(verdict.get("cost_delta", 0.0)))
	var accepted := "Removes the turn pocket" if not left and not right \
		else RoadPocketSection.describe(left, right, length_m)
	_pocket_section.show_verdict(verdict, accepted, "Apply (+%s)" % _thousands(cost) if cost > 0 else "Apply")

func _on_pocket_apply() -> void:
	var request: Array = _pocket_section.request()
	if _select_tool.set_selected_road_pocket(_pocket_info, request[0], request[1], request[2]):
		_selected_edges = _select_tool.selected_edges.duplicate()
		_refresh()
		return
	# The verdict changed since the preview: show why now.
	_on_pocket_changed(request[0], request[1], request[2])
