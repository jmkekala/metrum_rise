## Wide invisible resize handles for embedded Godot windows.
##
## Godot's built-in subwindow border can be difficult to hit precisely. This
## overlay adds forgiving resize strips inside the window content without moving
## simulation or UI policy logic out of Rust/Godot owner scripts.
extends Control

const EDGE_LEFT := 1
const EDGE_RIGHT := 2
const EDGE_TOP := 4
const EDGE_BOTTOM := 8

const EDGE_HANDLE_PX := 14.0
const CORNER_HANDLE_PX := 24.0
const FALLBACK_MIN_SIZE := Vector2(220.0, 160.0)

var _window: Window
var _dragging := false
var _active_edges := 0

static func install(window: Window) -> void:
	if window == null or window.get_node_or_null("WindowResizeHandles") != null:
		return
	var handles = load("res://scripts/ui/window_resize_handles.gd").new()
	handles.name = "WindowResizeHandles"
	window.add_child(handles)
	handles.setup(window)

func setup(window: Window) -> void:
	_window = window
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	_build_handles()

func _input(event: InputEvent) -> void:
	if not _dragging:
		return
	if event is InputEventMouseMotion:
		_apply_resize(event.relative)
		get_viewport().set_input_as_handled()
	elif event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and not event.pressed:
		_dragging = false
		_active_edges = 0
		get_viewport().set_input_as_handled()

func _build_handles() -> void:
	_add_handle(
		"ResizeTop",
		EDGE_TOP,
		Control.CURSOR_VSIZE,
		Vector2(0.0, 0.0),
		Vector2(1.0, 0.0),
		Rect2(CORNER_HANDLE_PX, 0.0, -CORNER_HANDLE_PX, EDGE_HANDLE_PX)
	)
	_add_handle(
		"ResizeBottom",
		EDGE_BOTTOM,
		Control.CURSOR_VSIZE,
		Vector2(0.0, 1.0),
		Vector2(1.0, 1.0),
		Rect2(CORNER_HANDLE_PX, -EDGE_HANDLE_PX, -CORNER_HANDLE_PX, 0.0)
	)
	_add_handle(
		"ResizeLeft",
		EDGE_LEFT,
		Control.CURSOR_HSIZE,
		Vector2(0.0, 0.0),
		Vector2(0.0, 1.0),
		Rect2(0.0, CORNER_HANDLE_PX, EDGE_HANDLE_PX, -CORNER_HANDLE_PX)
	)
	_add_handle(
		"ResizeRight",
		EDGE_RIGHT,
		Control.CURSOR_HSIZE,
		Vector2(1.0, 0.0),
		Vector2(1.0, 1.0),
		Rect2(-EDGE_HANDLE_PX, CORNER_HANDLE_PX, 0.0, -CORNER_HANDLE_PX)
	)
	_add_handle(
		"ResizeTopLeft",
		EDGE_LEFT | EDGE_TOP,
		Control.CURSOR_FDIAGSIZE,
		Vector2.ZERO,
		Vector2.ZERO,
		Rect2(0.0, 0.0, CORNER_HANDLE_PX, CORNER_HANDLE_PX)
	)
	_add_handle(
		"ResizeTopRight",
		EDGE_RIGHT | EDGE_TOP,
		Control.CURSOR_BDIAGSIZE,
		Vector2(1.0, 0.0),
		Vector2(1.0, 0.0),
		Rect2(-CORNER_HANDLE_PX, 0.0, 0.0, CORNER_HANDLE_PX)
	)
	_add_handle(
		"ResizeBottomLeft",
		EDGE_LEFT | EDGE_BOTTOM,
		Control.CURSOR_BDIAGSIZE,
		Vector2(0.0, 1.0),
		Vector2(0.0, 1.0),
		Rect2(0.0, -CORNER_HANDLE_PX, CORNER_HANDLE_PX, 0.0)
	)
	_add_handle(
		"ResizeBottomRight",
		EDGE_RIGHT | EDGE_BOTTOM,
		Control.CURSOR_FDIAGSIZE,
		Vector2.ONE,
		Vector2.ONE,
		Rect2(-CORNER_HANDLE_PX, -CORNER_HANDLE_PX, 0.0, 0.0)
	)

func _add_handle(
	name: String,
	edges: int,
	cursor_shape: int,
	anchor_min: Vector2,
	anchor_max: Vector2,
	offsets: Rect2
) -> void:
	var handle := Control.new()
	handle.name = name
	handle.mouse_filter = Control.MOUSE_FILTER_STOP
	handle.mouse_default_cursor_shape = cursor_shape
	handle.anchor_left = anchor_min.x
	handle.anchor_top = anchor_min.y
	handle.anchor_right = anchor_max.x
	handle.anchor_bottom = anchor_max.y
	handle.offset_left = offsets.position.x
	handle.offset_top = offsets.position.y
	handle.offset_right = offsets.size.x
	handle.offset_bottom = offsets.size.y
	handle.gui_input.connect(_on_handle_gui_input.bind(edges))
	add_child(handle)

func _on_handle_gui_input(event: InputEvent, edges: int) -> void:
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_dragging = true
			_active_edges = edges
		else:
			_dragging = false
			_active_edges = 0
		accept_event()

func _apply_resize(delta: Vector2) -> void:
	if _window == null:
		return

	var pos := Vector2(_window.position)
	var next_size := Vector2(_window.size)
	var old_right := pos.x + next_size.x
	var old_bottom := pos.y + next_size.y

	if _active_edges & EDGE_LEFT:
		pos.x += delta.x
		next_size.x -= delta.x
	if _active_edges & EDGE_RIGHT:
		next_size.x += delta.x
	if _active_edges & EDGE_TOP:
		pos.y += delta.y
		next_size.y -= delta.y
	if _active_edges & EDGE_BOTTOM:
		next_size.y += delta.y

	var min_size := Vector2(_window.min_size)
	if min_size.x <= 0:
		min_size.x = FALLBACK_MIN_SIZE.x
	if min_size.y <= 0:
		min_size.y = FALLBACK_MIN_SIZE.y

	if next_size.x < min_size.x:
		if _active_edges & EDGE_LEFT:
			pos.x = old_right - min_size.x
		next_size.x = min_size.x
	if next_size.y < min_size.y:
		if _active_edges & EDGE_TOP:
			pos.y = old_bottom - min_size.y
		next_size.y = min_size.y

	_window.position = Vector2i(int(round(pos.x)), int(round(pos.y)))
	_window.size = Vector2i(int(round(next_size.x)), int(round(next_size.y)))
