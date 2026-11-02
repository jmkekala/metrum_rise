# SPDX-License-Identifier: GPL-2.0-only

## Floating road-properties window for SelectTool edge editing.
##
## The window mirrors the previous side panel behavior but uses Godot's built-in
## draggable Window surface and preserves its placement within the session.
## Cross-section (`ROAD-80`): the shared cross-section controls show the selected road's lanes
## and median; a change asks Rust's verdict for it through the select tool (the commit's check,
## no geometry preview) and shows it beside Apply, which commits an accepted change (Ctrl+Z
## undoes it). The window opens tall enough for every row.
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")
const CrossSectionControls = preload("res://scripts/ui/cross_section_controls.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const VERDICT_READY := Color(0.55, 0.95, 0.55)
const VERDICT_REFUSED := Color.YELLOW
# Every row fits at the default size: the class buttons, lanes, median style and width, the
# verdict with Apply, and No buildings (the median rows grew it from 400 px).
const DEFAULT_SIZE := Vector2i(400, 500)
const MIN_SIZE := Vector2i(360, 460)

var _simulation_node: Node
var _select_tool: Node

var _window: Window
var _warning_label: Label
var _no_build_check: CheckBox
var _class_buttons: Array[Button] = []
var _lanes: CrossSectionControls
var _verdict_label: Label
var _apply_button: Button
var _selected_edges: Array[int] = []
var _last_screen_pos := Vector2.ZERO

func setup(simulation_node: Node, select_tool: Node) -> void:
	_simulation_node = simulation_node
	_select_tool = select_tool
	_load_median_styles()

func _ready() -> void:
	_build_window()

# Rust's median widths are constant: the controls read them once, as soon as both the simulation
# node and the controls exist (`setup` runs before or after `_ready`, and again on every show).
func _load_median_styles() -> void:
	if _lanes and _simulation_node and _lanes.median_styles.is_empty():
		_lanes.median_styles = _simulation_node.get_median_styles()

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

	_lanes = CrossSectionControls.new(14)
	_lanes.changed.connect(_on_cross_section_changed)
	_load_median_styles()
	root_vbox.add_child(_lanes)
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

func _place_window_near_cursor(screen_pos: Vector2) -> void:
	UIStyle.place_window_near_cursor(_window, screen_pos, get_viewport())

func _refresh() -> void:
	if _selected_edges.is_empty() or _simulation_node == null:
		return

	_refresh_lanes()
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

# The first selected road's cross-section; editable when every selected road can change it.
func _refresh_lanes() -> void:
	var first: Dictionary = _simulation_node.get_edge_cross_section(_selected_edges[0])
	_lanes.set_lanes(int(first.get("fwd_lanes", 0)), int(first.get("bkw_lanes", 0)))
	_lanes.set_median(int(first.get("median_style", 0)), float(first.get("median_width", 0.0)))
	_lanes.set_editable(_selected_edges.all(
		func(idx): return bool(_simulation_node.get_edge_cross_section(idx).get("editable", false))
	))
	_verdict_label.text = ""
	_apply_button.disabled = true

func _on_cross_section_changed(fwd: int, bkw: int, median_style: int, median_width: float) -> void:
	if _select_tool == null or not _select_tool.has_method("preview_selected_edge_cross_section"):
		return
	show_verdict(_select_tool.preview_selected_edge_cross_section(fwd, bkw, median_style, median_width))

## Shows Rust's verdict on the controls' cross-section and enables Apply when it accepts it.
func show_verdict(verdict: Dictionary) -> void:
	var ready := bool(verdict.get("is_valid", false))
	_apply_button.disabled = not ready
	_verdict_label.add_theme_color_override("font_color", VERDICT_READY if ready else VERDICT_REFUSED)
	_verdict_label.text = _cross_section_text() if ready \
		else RoadReasonText.text(verdict, "This road cannot change its cross-section")

# "2 + 2 lanes", with ", Grass 3.0 m" when the controls carry a median.
func _cross_section_text() -> String:
	var text := "%d + %d lanes" % [_lanes.fwd_lanes, _lanes.bkw_lanes]
	if _lanes.median_style != 0:
		text += ", %s %.1f m" % [CrossSectionControls.MEDIAN_STYLE_NAMES[_lanes.median_style], _lanes.median_width]
	return text

func _on_apply_pressed() -> void:
	if _select_tool and _select_tool.has_method("set_selected_edge_cross_section"):
		var refused: int = _select_tool.set_selected_edge_cross_section(
			_lanes.fwd_lanes, _lanes.bkw_lanes, _lanes.median_style, _lanes.median_width
		)
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
