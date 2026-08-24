## Floating road-properties window for SelectTool edge editing.
##
## The window mirrors the previous side panel behavior but uses Godot's built-in
## draggable Window surface and preserves its placement within the session.
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")

var _simulation_node: Node
var _select_tool: Node

var _window: Window
var _warning_label: Label
var _no_build_check: CheckBox
var _class_buttons: Array[Button] = []
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
		Vector2i(380, 300),
		Vector2i(320, 260),
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
	if _window == null:
		return

	if screen_pos == Vector2.ZERO:
		screen_pos = get_viewport().get_mouse_position()

	var viewport_size := get_viewport().get_visible_rect().size
	var window_size := Vector2(_window.size)
	var padding := UIStyle.CURSOR_WINDOW_GAP
	var top_margin := 40.0
	var bottom_margin := 12.0

	var x := screen_pos.x + padding
	if x + window_size.x > viewport_size.x - padding:
		x = screen_pos.x - window_size.x - padding
	if x < padding:
		x = padding

	var y := screen_pos.y - 24.0
	var max_y := maxf(top_margin, viewport_size.y - window_size.y - bottom_margin)
	y = clampf(y, top_margin, max_y)

	_window.position = Vector2i(int(round(x)), int(round(y)))

func _refresh() -> void:
	if _selected_edges.is_empty() or _simulation_node == null:
		return

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

func _on_class_button_pressed(class_index: int) -> void:
	if _select_tool and _select_tool.has_method("set_selected_edge_class"):
		_select_tool.set_selected_edge_class(class_index)
	_refresh()

func _on_no_build_toggled(enabled: bool) -> void:
	if _select_tool and _select_tool.has_method("set_selected_edge_no_building_spawn"):
		_select_tool.set_selected_edge_no_building_spawn(enabled)
