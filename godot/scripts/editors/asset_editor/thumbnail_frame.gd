# SPDX-License-Identifier: GPL-2.0-only

## Draws the fixed-aspect thumbnail capture frame over the asset editor preview.
## The capture crops exactly this rectangle, so the dimmed border is what gets discarded.
extends Control

const DIM_COLOR := Color(0.0, 0.0, 0.0, 0.55)
const BORDER_COLOR := Color(0.10, 0.70, 1.0, 0.90)
const LABEL_COLOR := Color(1.0, 1.0, 1.0, 0.90)
const LABEL_BACKING := Color(0.0, 0.0, 0.0, 0.55)
# Keeps the frame edge clear of the pane border so it stays readable while orbiting.
const MARGIN := 12.0
const CORNER_TICK := 20.0
const LABEL_SIZE := 16
const LABEL_INSET := 8.0
const LABEL_PADDING := 6.0

var _aspect := 4.0 / 3.0
var _caption := ""
var _active := false

## Largest rect of `aspect` that fits `pane`, in pane-local coordinates.
## Shared with the capture so the drawn frame and the cropped region cannot drift apart.
static func frame_in(pane: Vector2, aspect: float) -> Rect2:
	var usable := Vector2(maxf(1.0, pane.x - MARGIN * 2.0), maxf(1.0, pane.y - MARGIN * 2.0))
	var extent := Vector2(minf(usable.x, usable.y * aspect), 0.0)
	extent.y = extent.x / aspect
	return Rect2(((pane - extent) * 0.5).floor(), extent.floor())

func _ready() -> void:
	resized.connect(queue_redraw)

## Show the frame for an output of `output` pixels; only its aspect affects the drawn rect.
func activate(output: Vector2i) -> void:
	_aspect = float(output.x) / float(maxi(1, output.y))
	_caption = "%d × %d" % [output.x, output.y]
	_active = true
	queue_redraw()

## Hide the frame without disturbing the pane's other overlays.
func clear() -> void:
	_active = false
	queue_redraw()

func _draw() -> void:
	if not _active:
		return
	var rect := frame_in(size, _aspect)
	draw_rect(Rect2(0.0, 0.0, size.x, rect.position.y), DIM_COLOR)
	draw_rect(Rect2(0.0, rect.end.y, size.x, size.y - rect.end.y), DIM_COLOR)
	draw_rect(Rect2(0.0, rect.position.y, rect.position.x, rect.size.y), DIM_COLOR)
	draw_rect(Rect2(rect.end.x, rect.position.y, size.x - rect.end.x, rect.size.y), DIM_COLOR)
	draw_rect(rect, BORDER_COLOR, false, 2.0)
	# Corner ticks read as a camera frame rather than a drag-selection box.
	for corner in [[rect.position, Vector2(1.0, 1.0)], [Vector2(rect.end.x, rect.position.y), Vector2(-1.0, 1.0)],
		[Vector2(rect.position.x, rect.end.y), Vector2(1.0, -1.0)], [rect.end, Vector2(-1.0, -1.0)]]:
		var at: Vector2 = corner[0]
		var direction: Vector2 = corner[1]
		draw_line(at, at + Vector2(CORNER_TICK * direction.x, 0.0), BORDER_COLOR, 4.0)
		draw_line(at, at + Vector2(0.0, CORNER_TICK * direction.y), BORDER_COLOR, 4.0)
	var font := get_theme_default_font()
	if font == null:
		return
	# Inside the frame: above it the caption collides with the pane's hover readout, and a
	# backing plate keeps it legible against both a bright sky and a night scene.
	var text := font.get_string_size(_caption, HORIZONTAL_ALIGNMENT_LEFT, -1, LABEL_SIZE)
	var box := Rect2(rect.position + Vector2(LABEL_INSET, LABEL_INSET),
		text + Vector2(LABEL_PADDING, LABEL_PADDING) * 2.0)
	draw_rect(box, LABEL_BACKING)
	draw_string(font, box.position + Vector2(LABEL_PADDING, LABEL_PADDING + font.get_ascent(LABEL_SIZE)),
		_caption, HORIZONTAL_ALIGNMENT_LEFT, -1, LABEL_SIZE, LABEL_COLOR)
