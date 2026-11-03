# SPDX-License-Identifier: GPL-2.0-only

## The road builder's strip (`ROAD-101`): the layout seen across the road, left to right looking
## the way it is drawn, drawn to scale with a floor of MIN_PART_PX per part so a 0.5 m painted
## median stays clickable. Rows, top to bottom: a "+" slot marker on every boundary a part may be
## inserted at (between the two ends: the sidewalks, or for a road without sidewalks its shoulder
## bands), each part's glyph, its surface, and its width. An invalid
## layout is tinted red. Clicking a slot selects it and clicking a part (not an end) asks for
## its popup; parts drag along the strip to move, off it to be removed, and the palette's tiles
## drop into it. A view: it shows the parts and Rust's widths it is given and reports gestures
## through its signals; the builder edits the layout and asks Rust's check.
extends Control

## The player selected insert slot `slot` (a part inserted there gets index `slot`).
signal slot_clicked(slot: int)
## The player clicked interior part `index`.
signal part_clicked(index: int)
## A palette tile was dropped at slot `slot`.
signal tile_dropped(part: Dictionary, slot: int)
## Interior part `from` was dropped at slot `slot` (the boundary it lands on, before removal).
signal part_moved(from: int, slot: int)
## Interior part `index` was dragged off the strip.
signal part_dragged_out(index: int)

const RoadPartGlyph = preload("res://scripts/ui/road_builder/road_part_glyph.gd")
const RoadPartTile = preload("res://scripts/ui/road_builder/road_part_tile.gd")
## The narrowest a part is drawn, in px.
const MIN_PART_PX := 24.0
## The widest scale, in px per metre (a 3.5 m lane at 84 px).
const MAX_PX_PER_M := 24.0
const PAD_X := 12.0
const SLOT_Y := 10.0
const SLOT_RADIUS := 8.0
const GLYPH_TOP := 22.0
const GLYPH_SIZE := 24.0
const SURFACE_TOP := 50.0
const SURFACE_HEIGHT := 56.0
const LABEL_TOP := 110.0
const INVALID_TINT := Color(0.85, 0.12, 0.10, 0.38)
const INVALID_BORDER := Color(1.0, 0.35, 0.3)
const ACCENT := Color(1.0, 0.82, 0.30)
const SLOT_IDLE := Color(0.55, 0.60, 0.75)

var parts: Array = []
var widths_m := PackedFloat32Array()
var valid := true
## The slot the next clicked tile goes to (1 .. parts.size() - 1).
var selected_slot := 1
## The part whose popup is open, or -1.
var selected_part := -1

# Each part's left edge and width in px, from _layout_parts.
var _xs := PackedFloat32Array()
var _ws := PackedFloat32Array()
var _press_part := -1
# The slot a drag over the strip would drop at, or -1.
var _drop_slot := -1
# The part this strip is dragging, and whether it landed back on the strip.
var _drag_index := -1
var _dropped_on_strip := false

func _init() -> void:
	custom_minimum_size = Vector2(0.0, LABEL_TOP + 22.0)
	mouse_filter = Control.MOUSE_FILTER_STOP
	resized.connect(_relayout)

## Shows `layout` with Rust's per-part widths; `is_valid` false tints it red.
func set_layout(layout: Array, part_widths_m: PackedFloat32Array, is_valid: bool) -> void:
	parts = layout
	widths_m = part_widths_m
	valid = is_valid
	selected_slot = clampi(selected_slot, 1, maxi(1, parts.size() - 1))
	_relayout()

## Selects insert slot `slot` (clamped to the slots between the two ends).
func set_slot(slot: int) -> void:
	selected_slot = clampi(slot, 1, maxi(1, parts.size() - 1))
	queue_redraw()

## Part `index`'s surface rectangle, in the strip's coordinates.
func part_rect(index: int) -> Rect2:
	return Rect2(_xs[index], SURFACE_TOP, _ws[index], SURFACE_HEIGHT)

## Where slot `slot`'s marker is drawn, in the strip's coordinates.
func slot_position(slot: int) -> Vector2:
	return Vector2(_xs[slot], SLOT_Y)

## Whether part `index` may be moved, removed or opened (everything but the two ends and any
## sidewalk).
func is_interior(index: int) -> bool:
	return index > 0 and index < parts.size() - 1 and String(parts[index].get("kind", "")) != "sidewalk"

# The scale fitting every part into the width, with each part at least MIN_PART_PX: the parts
# held at the floor take their floor, the rest share what is left by their metres. Each pass
# can only add floored parts, so it settles within parts.size() passes. O(parts^2), parts <= 12.
func _relayout() -> void:
	var count := parts.size()
	_xs.resize(count)
	_ws.resize(count)
	if count == 0:
		queue_redraw()
		return
	var room := maxf(size.x - 2.0 * PAD_X, MIN_PART_PX * count)
	var px_per_m := MAX_PX_PER_M
	for _attempt in count:
		var floored_px := 0.0
		var free_m := 0.0
		for index in count:
			var metres := _metres(index)
			if metres * px_per_m < MIN_PART_PX:
				floored_px += MIN_PART_PX
			else:
				free_m += metres
		if free_m <= 0.0 or floored_px + free_m * px_per_m <= room:
			break
		px_per_m = (room - floored_px) / free_m
	var x := PAD_X
	for index in count:
		_xs[index] = x
		_ws[index] = maxf(MIN_PART_PX, _metres(index) * px_per_m)
		x += _ws[index]
	queue_redraw()

func _metres(index: int) -> float:
	return widths_m[index] if index < widths_m.size() else 0.0

func _draw() -> void:
	if parts.is_empty():
		return
	var font := get_theme_default_font()
	for index in parts.size():
		var rect := part_rect(index)
		RoadPartGlyph.draw_surface(self, rect, parts[index])
		var glyph_x := rect.get_center().x - GLYPH_SIZE * 0.5
		RoadPartGlyph.draw_glyph(self, Rect2(glyph_x, GLYPH_TOP, GLYPH_SIZE, GLYPH_SIZE), parts[index])
		var label := _width_text(_metres(index))
		var label_width := font.get_string_size(label, HORIZONTAL_ALIGNMENT_LEFT, -1, 12).x
		draw_string(font, Vector2(rect.get_center().x - label_width * 0.5, LABEL_TOP + 14.0), label,
			HORIZONTAL_ALIGNMENT_LEFT, -1, 12, Color(0.85, 0.85, 0.85))
		if index > 0:
			draw_line(Vector2(rect.position.x, SURFACE_TOP), Vector2(rect.position.x, SURFACE_TOP + SURFACE_HEIGHT),
				Color(0.0, 0.0, 0.0, 0.55), 1.0)
	var whole := Rect2(_xs[0], SURFACE_TOP, _xs[-1] + _ws[-1] - _xs[0], SURFACE_HEIGHT)
	if not valid:
		draw_rect(whole, INVALID_TINT)
		draw_rect(whole, INVALID_BORDER, false, 2.0)
	if selected_part >= 0 and selected_part < parts.size():
		draw_rect(part_rect(selected_part).grow(-1.0), ACCENT, false, 2.0)
	for slot in range(1, parts.size()):
		var at := slot_position(slot)
		var chosen := slot == selected_slot
		draw_circle(at, SLOT_RADIUS, ACCENT if chosen else SLOT_IDLE)
		var ink := Color(0.1, 0.1, 0.1) if chosen else Color(0.05, 0.05, 0.1)
		draw_line(at - Vector2(4.0, 0.0), at + Vector2(4.0, 0.0), ink, 2.0)
		draw_line(at - Vector2(0.0, 4.0), at + Vector2(0.0, 4.0), ink, 2.0)
	if _drop_slot >= 1:
		var x := _xs[_drop_slot]
		draw_line(Vector2(x, SLOT_Y - SLOT_RADIUS), Vector2(x, SURFACE_TOP + SURFACE_HEIGHT + 4.0), ACCENT, 3.0)

# A width with one decimal, or two where one would round it (a 0.75 m shoulder).
static func _width_text(metres: float) -> String:
	return "%.1f" % metres if is_equal_approx(metres * 10.0, roundf(metres * 10.0)) else "%.2f" % metres

## The interior slot nearest strip x `x`.
func slot_at(x: float) -> int:
	var best := 1
	for slot in range(1, parts.size()):
		if absf(_xs[slot] - x) < absf(_xs[best] - x):
			best = slot
	return best

## The part under strip point `at` (glyph and surface rows), or -1.
func part_at(at: Vector2) -> int:
	if at.y < GLYPH_TOP or at.y > SURFACE_TOP + SURFACE_HEIGHT:
		return -1
	for index in parts.size():
		if at.x >= _xs[index] and at.x < _xs[index] + _ws[index]:
			return index
	return -1

func _slot_under(at: Vector2) -> int:
	if at.y > GLYPH_TOP or parts.size() < 2:
		return -1
	var slot := slot_at(at.x)
	return slot if absf(_xs[slot] - at.x) <= SLOT_RADIUS + 3.0 else -1

func _gui_input(event: InputEvent) -> void:
	var button := event as InputEventMouseButton
	if button == null or button.button_index != MOUSE_BUTTON_LEFT:
		return
	if button.pressed:
		var slot := _slot_under(button.position)
		if slot >= 1:
			set_slot(slot)
			slot_clicked.emit(slot)
			accept_event()
			return
		_press_part = part_at(button.position)
	else:
		var index := part_at(button.position)
		if index >= 0 and index == _press_part and is_interior(index):
			part_clicked.emit(index)
			accept_event()
		_press_part = -1

func _get_tooltip(at_position: Vector2) -> String:
	if get_viewport().gui_is_dragging():
		return ""
	var index := part_at(at_position)
	if index < 0:
		return "Click a + to choose where a clicked part goes" if _slot_under(at_position) >= 1 else ""
	var hint := "Click to change, move or remove it; drag it along the road or off it" if is_interior(index) \
		else RoadPartGlyph.part_hint(parts[index])
	return "%s, %s m\n%s" % [RoadPartGlyph.part_name(parts[index]), _width_text(_metres(index)), hint]

func _get_drag_data(at_position: Vector2) -> Variant:
	var index := part_at(at_position)
	if index < 0 or not is_interior(index):
		return null
	_drag_index = index
	_dropped_on_strip = false
	if get_viewport().gui_is_dragging():
		set_drag_preview(RoadPartTile.drag_preview(RoadPartGlyph.part_name(parts[index])))
	return {"road_builder": "part", "index": index}

func _can_drop_data(at_position: Vector2, data: Variant) -> bool:
	if not (data is Dictionary and data.has("road_builder")) or parts.size() < 2:
		return false
	_drop_slot = slot_at(at_position.x)
	queue_redraw()
	return true

func _drop_data(at_position: Vector2, data: Variant) -> void:
	var slot := slot_at(at_position.x)
	_drop_slot = -1
	queue_redraw()
	if data.road_builder == "tile":
		tile_dropped.emit(Dictionary(data.part).duplicate(), slot)
	elif data.road_builder == "part":
		_dropped_on_strip = true
		part_moved.emit(int(data.index), slot)

func _notification(what: int) -> void:
	if what == NOTIFICATION_MOUSE_EXIT and _drop_slot >= 0:
		_drop_slot = -1
		queue_redraw()
	elif what == NOTIFICATION_DRAG_END:
		_drop_slot = -1
		queue_redraw()
		# A part released anywhere but on the strip leaves the road.
		if _drag_index >= 0 and not _dropped_on_strip:
			part_dragged_out.emit(_drag_index)
		_drag_index = -1
