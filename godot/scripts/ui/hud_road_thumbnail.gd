# SPDX-License-Identifier: GPL-2.0-only

## Top-down road tile art, using the builder's glyphs and Rust's per-part widths, with a turn
## arrow badge in the top-right corner for a type with turn pockets (`ROAD-128`). Geometry is
## prepared once per type refresh; painting is O(parts), independent of world population.
extends RefCounted

const Glyph = preload("res://scripts/ui/road_builder/road_part_glyph.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

var _parts: Array = []
var _widths := PackedFloat32Array()
var _total := 0.0
# The type's turn pockets: left, right.
var _pockets := [false, false]

## Uses a get_road_types entry, including the widths shared with check_road_layout.
func setup(type: Dictionary) -> void:
	_parts = Array(type.get("parts", [])).duplicate()
	_widths = type.get("part_widths_m", PackedFloat32Array())
	# As in RoadBuilderStrip, rural roads show Rust's shoulder allowance at both edges.
	var shoulder := 0.5 * (float(type.get("total_m", 0.0)) - float(type.get("carriageway_m", 0.0)))
	if not bool(type.get("sidewalks", true)) and shoulder > 0.0:
		_parts.push_front({"kind": "shoulder"})
		_parts.append({"kind": "shoulder"})
		var widths := PackedFloat32Array([shoulder])
		widths.append_array(_widths)
		widths.append(shoulder)
		_widths = widths
	_total = 0.0
	for width in _widths:
		_total += width
	var pockets: Dictionary = type.get("pockets", {})
	_pockets = [bool(pockets.get("left", false)), bool(pockets.get("right", false))]

## Draws each surface and directional glyph inside the supplied square.
func paint(canvas: CanvasItem, rect: Rect2) -> void:
	if _total <= 0.0 or _parts.size() != _widths.size():
		return
	# Cap pixels/metre so the footpath does not look as wide as a four-lane road.
	var px_per_m := minf(rect.size.x / _total, 7.0)
	var x := rect.get_center().x - _total * px_per_m * 0.5
	for index in _parts.size():
		var band := Rect2(x, rect.position.y, _widths[index] * px_per_m, rect.size.y)
		Glyph.draw_surface(canvas, band, _parts[index])
		var kind := String(_parts[index].get("kind", ""))
		if kind == "car_lane" or kind == "sidewalk" or kind == "footpath":
			var side := minf(band.size.x - 2.0, 20.0)
			if side > 2.0:
				var box := Rect2(band.get_center() - Vector2.ONE * side * 0.5, Vector2.ONE * side)
				Glyph.draw_glyph(canvas, box, _parts[index])
		x = band.end.x
	if _pockets[0] or _pockets[1]:
		_paint_pocket_badge(canvas, rect)

# A dark rounded badge at the top-right corner with an accent turn arrow per pocket side.
func _paint_pocket_badge(canvas: CanvasItem, rect: Rect2) -> void:
	var side := roundf(rect.size.x * 0.26)
	var sides := [0, 1].filter(func(i: int): return _pockets[i])
	var size := Vector2(side * sides.size() + 4.0, side + 4.0)
	var badge := Rect2(Vector2(rect.end.x - size.x + 3.0, rect.position.y - 3.0), size)
	var style := StyleBoxFlat.new()
	style.bg_color = Color(0.07, 0.08, 0.10, 0.92)
	style.border_color = Color(UIStyle.HUD_ACCENT, 0.8)
	style.set_border_width_all(1)
	style.set_corner_radius_all(int(side * 0.3))
	style.anti_aliasing = true
	canvas.draw_style_box(style, badge)
	for slot in sides.size():
		var box := Rect2(badge.position + Vector2(2.0 + slot * side, 2.0), Vector2.ONE * side)
		paint_turn_arrow(canvas, box.grow(-side * 0.12), sides[slot] == 1)

## An L-shaped turn arrow in `box`, up and then left or right (`right`) with its head, in the
## HUD accent; the road type card's pocket chip draws the same.
static func paint_turn_arrow(canvas: CanvasItem, box: Rect2, right: bool) -> void:
	var s := box.size.x
	var dir := 1.0 if right else -1.0
	var stem_x := box.get_center().x - dir * s * 0.22
	var bend := box.position.y + s * 0.38
	var tip := Vector2(stem_x + dir * s * 0.62, bend)
	var width := maxf(1.5, s * 0.14)
	var colour := UIStyle.HUD_ACCENT
	canvas.draw_polyline(PackedVector2Array([Vector2(stem_x, box.end.y), Vector2(stem_x, bend),
		tip - Vector2(dir * s * 0.2, 0.0)]), colour, width, true)
	var head := s * 0.26
	canvas.draw_colored_polygon(PackedVector2Array([tip, tip + Vector2(-dir * head, -head),
		tip + Vector2(-dir * head, head)]), colour)
