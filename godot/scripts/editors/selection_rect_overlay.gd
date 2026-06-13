## Draws the asset editor mesh-part drag-selection rectangle over the 3D viewport.
extends Control

const FILL_COLOR := Color(0.10, 0.55, 1.0, 0.16)
const BORDER_COLOR := Color(0.10, 0.70, 1.0, 0.85)

var _active := false
var _start := Vector2.ZERO
var _end := Vector2.ZERO

## Show or update the selection rectangle from viewport-global mouse coordinates.
func set_rect_global(start_pos: Vector2, end_pos: Vector2, active: bool) -> void:
	_active = active
	var origin := global_position
	_start = start_pos - origin
	_end = end_pos - origin
	queue_redraw()

## Hide the selection rectangle.
func clear() -> void:
	_active = false
	queue_redraw()

func _draw() -> void:
	if not _active:
		return
	var top_left := Vector2(minf(_start.x, _end.x), minf(_start.y, _end.y))
	var size := Vector2(absf(_start.x - _end.x), absf(_start.y - _end.y))
	if size.x < 1.0 or size.y < 1.0:
		return
	var rect := Rect2(top_left, size)
	draw_rect(rect, FILL_COLOR, true)
	draw_rect(rect, BORDER_COLOR, false, 2.0)
