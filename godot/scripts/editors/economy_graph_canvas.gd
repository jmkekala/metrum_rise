# SPDX-License-Identifier: GPL-2.0-only

## Lightweight economy-graph canvas for the developer economy editor.
## Draws scenario node links, exposes node selection, and allows simple
## drag-based position updates for authored graph nodes.
extends Control

signal node_selected(node_id: String)
signal node_moved(node_id: String, position: Vector2)

const NODE_SIZE := Vector2(200.0, 82.0)

var _nodes: Array = []
var _edges: Array = []
var _selected_node_id := ""
var _node_controls: Dictionary = {}
var _drag_node_id := ""
var _drag_offset := Vector2.ZERO

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and not event.pressed:
		_drag_node_id = ""

func set_graph(nodes: Array, edges: Array, selected_node_id: String) -> void:
	_nodes = nodes.duplicate(true)
	_edges = edges.duplicate(true)
	_selected_node_id = selected_node_id
	_rebuild()

func _rebuild() -> void:
	for child in get_children():
		child.queue_free()
	_node_controls.clear()

	for node in _nodes:
		var button := Button.new()
		button.text = "%s\n%s" % [str(node.get("title", node.get("id", ""))), str(node.get("subtitle", ""))]
		button.alignment = HORIZONTAL_ALIGNMENT_LEFT
		button.vertical_icon_alignment = VERTICAL_ALIGNMENT_CENTER
		button.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		button.size = NODE_SIZE
		button.position = _node_position(node)
		button.tooltip_text = str(node.get("tooltip", ""))
		button.pressed.connect(_on_node_pressed.bind(str(node.get("id", ""))))
		button.gui_input.connect(_on_node_gui_input.bind(str(node.get("id", "")), button))
		_apply_style(button, str(node.get("kind", "profile")), str(node.get("id", "")) == _selected_node_id)
		add_child(button)
		_node_controls[str(node.get("id", ""))] = button

	queue_redraw()

func _draw() -> void:
	for edge in _edges:
		var from_id := str(edge.get("from", ""))
		var to_id := str(edge.get("to", ""))
		if not _node_controls.has(from_id) or not _node_controls.has(to_id):
			continue
		var from_node: Control = _node_controls[from_id]
		var to_node: Control = _node_controls[to_id]
		var from_pos := from_node.position + from_node.size * 0.5
		var to_pos := to_node.position + to_node.size * 0.5
		var edge_kind := str(edge.get("kind", "resource"))
		var color := Color(0.76, 0.78, 0.84)
		var width := 3.0
		if edge_kind == "controller":
			color = Color(0.86, 0.55, 0.25)
			width = 2.0
		draw_line(from_pos, to_pos, color, width, true)

func _on_node_pressed(node_id: String) -> void:
	_selected_node_id = node_id
	emit_signal("node_selected", node_id)
	_refresh_selection_styles()

func _on_node_gui_input(event: InputEvent, node_id: String, button: Control) -> void:
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_drag_node_id = node_id
			_drag_offset = button.position - get_local_mouse_position()
			emit_signal("node_selected", node_id)
			_selected_node_id = node_id
			_refresh_selection_styles()
		elif _drag_node_id == node_id:
			_drag_node_id = ""
	if event is InputEventMouseMotion and _drag_node_id == node_id:
		if (event.button_mask & MOUSE_BUTTON_MASK_LEFT) == 0:
			_drag_node_id = ""
			return
		var new_position := get_local_mouse_position() + _drag_offset
		new_position.x = clampf(new_position.x, 12.0, max(12.0, size.x - NODE_SIZE.x - 12.0))
		new_position.y = clampf(new_position.y, 12.0, max(12.0, size.y - NODE_SIZE.y - 12.0))
		button.position = new_position.round()
		_update_node_position(node_id, button.position)
		emit_signal("node_moved", node_id, button.position)
		queue_redraw()

func _refresh_selection_styles() -> void:
	for node_id in _node_controls.keys():
		var button: Button = _node_controls[node_id]
		var kind := "profile"
		for node in _nodes:
			if str(node.get("id", "")) == str(node_id):
				kind = str(node.get("kind", "profile"))
				break
		_apply_style(button, kind, str(node_id) == _selected_node_id)

func _update_node_position(node_id: String, position: Vector2) -> void:
	for node in _nodes:
		if str(node.get("id", "")) != node_id:
			continue
		node["position"] = [position.x, position.y]
		return

func _apply_style(button: Button, kind: String, selected: bool) -> void:
	var style := StyleBoxFlat.new()
	style.bg_color = Color(0.13, 0.14, 0.17)
	if kind == "controller":
		style.bg_color = Color(0.20, 0.16, 0.11)
	style.border_width_left = 2
	style.border_width_top = 2
	style.border_width_right = 2
	style.border_width_bottom = 2
	style.border_color = Color(0.34, 0.36, 0.43)
	if selected:
		style.border_color = Color(0.36, 0.74, 0.96)
	button.add_theme_stylebox_override("normal", style)
	button.add_theme_stylebox_override("hover", style)
	button.add_theme_stylebox_override("pressed", style)
	button.add_theme_color_override("font_color", Color(0.92, 0.94, 0.98))

func _node_position(node: Dictionary) -> Vector2:
	var raw_pos = node.get("position", [24.0, 24.0])
	if raw_pos is Array and raw_pos.size() >= 2:
		return Vector2(float(raw_pos[0]), float(raw_pos[1]))
	return Vector2(24.0, 24.0)
