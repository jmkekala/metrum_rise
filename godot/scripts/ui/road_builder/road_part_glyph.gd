# SPDX-License-Identifier: GPL-2.0-only

## The road builder's procedural art (`ROAD-101`): each road part's name, its surface colour
## (after the roadbed's asphalt, sidewalk paving, grass and median paving) and its `_draw()`
## glyph: an arrow for a lane's direction (up runs the way the road is drawn), 45-degree
## hatching for a painted median, a grass fill, a paving fill, a pedestrian on a band for a
## sidewalk and a parking sign over painted bay lines for a parking lane (`PARK-05`). Shared by
## the strip and the part tiles; no assets. A part is Rust's part dictionary (`kind`, `forward`,
## `style`, `width_m`), as `check_road_layout` takes it, or the builder's `shoulder` end (a road
## without sidewalks: the roadbed's 0.75 m shoulder band, never sent to Rust), drawn as gravel
## with the carriageway's edge line.
extends RefCounted

## Median style codes, as Rust's `get_median_styles` keys them.
const PAINTED := 1
const GRASS := 2
const PAVING := 3

const ASPHALT := Color(0.22, 0.22, 0.24)
const SIDEWALK := Color(0.58, 0.56, 0.52)
const GRASS_FILL := Color(0.29, 0.47, 0.22)
const PAVING_FILL := Color(0.63, 0.59, 0.52)
const SHOULDER_FILL := Color(0.42, 0.40, 0.36)
const MARKING := Color(0.92, 0.92, 0.88)
const KERB := Color(0.78, 0.77, 0.74)
const INK := Color(0.95, 0.95, 0.95)
const SIGN_BLUE := Color8(0, 94, 184)  # Väylä sign blue #005EB8

## The part's name for tiles, tooltips and the popup.
static func part_name(part: Dictionary) -> String:
	match String(part.get("kind", "")):
		"sidewalk":
			return "Sidewalk"
		"shoulder":
			return "Shoulder"
		"footpath":
			return "Footpath"
		"parking":
			return "Parking lane"
		"car_lane":
			return "Forward lane" if bool(part.get("forward", true)) else "Backward lane"
		"median":
			match int(part.get("style", 0)):
				PAINTED:
					return "Painted median"
				GRASS:
					return "Grass median"
				PAVING:
					return "Paving median"
	return "Unknown part"

## One line on what the part is, for its tooltip.
static func part_hint(part: Dictionary) -> String:
	match String(part.get("kind", "")):
		"sidewalk":
			return "A sidewalk on this side (the Left sidewalk and Right sidewalk checkboxes)"
		"parking":
			return "Cars park along the kerb"
		"shoulder":
			return "No sidewalk: a shoulder on this side that walkers do not use"
		"car_lane":
			if bool(part.get("forward", true)):
				return "Traffic runs the way the road is drawn (right of the median)"
			return "Traffic runs against the way the road is drawn (left of the median)"
		"median":
			match int(part.get("style", 0)):
				PAINTED:
					return "Painted hatching between the two directions"
				GRASS:
					return "A kerbed grass strip between the two directions"
				PAVING:
					return "A kerbed paved strip between the two directions"
	return ""

## The colour the part's surface is drawn in.
static func surface_color(part: Dictionary) -> Color:
	match String(part.get("kind", "")):
		"sidewalk", "footpath":
			return SIDEWALK
		"shoulder":
			return SHOULDER_FILL
		"median":
			match int(part.get("style", 0)):
				GRASS:
					return GRASS_FILL
				PAVING:
					return PAVING_FILL
	return ASPHALT

## Fills `rect` with the part's surface and its pattern (hatching, grass, paving, kerbs).
static func draw_surface(canvas: CanvasItem, rect: Rect2, part: Dictionary) -> void:
	canvas.draw_rect(rect, surface_color(part))
	var kind := String(part.get("kind", ""))
	var style := int(part.get("style", 0))
	if kind == "median" and style == PAINTED:
		_hatch(canvas, rect, MARKING, 7.0)
		canvas.draw_line(rect.position, Vector2(rect.position.x, rect.end.y), MARKING, 1.5)
		canvas.draw_line(Vector2(rect.end.x, rect.position.y), rect.end, MARKING, 1.5)
	elif kind == "median" and style == GRASS:
		_grass(canvas, rect)
	elif kind == "median" and style == PAVING:
		_paving(canvas, rect)
	elif kind == "parking":
		_bay_lines(canvas, rect)
	elif kind == "shoulder":
		_gravel(canvas, rect)
	# Kerbed medians and sidewalks show their kerb as a light edge.
	if kind == "sidewalk" or (kind == "median" and style != PAINTED):
		canvas.draw_rect(Rect2(rect.position, Vector2(2.0, rect.size.y)), KERB)
		canvas.draw_rect(Rect2(Vector2(rect.end.x - 2.0, rect.position.y), Vector2(2.0, rect.size.y)), KERB)

## Draws the part's glyph centred in `rect` (a square about 24 px wide works best).
static func draw_glyph(canvas: CanvasItem, rect: Rect2, part: Dictionary) -> void:
	var kind := String(part.get("kind", ""))
	var side := minf(rect.size.x, rect.size.y)
	var box := Rect2(rect.get_center() - Vector2(side, side) * 0.5, Vector2(side, side))
	match kind:
		"sidewalk", "footpath":
			_pedestrian(canvas, box)
		"car_lane":
			canvas.draw_rect(box, ASPHALT)
			_arrow(canvas, box.grow(-side * 0.18), bool(part.get("forward", true)))
		"median", "shoulder":
			draw_surface(canvas, box, part)
		"parking":
			_parking_sign(canvas, box)
	canvas.draw_rect(box, Color(0.0, 0.0, 0.0, 0.5), false, 1.0)

# Diagonal lines rising left to right every `spacing` px, clipped to `rect`.
static func _hatch(canvas: CanvasItem, rect: Rect2, color: Color, spacing: float) -> void:
	var offset := -rect.size.y
	while offset < rect.size.x:
		# The line y = rect.end.y - (x - x0) from x0 = rect.x + offset, clipped to the rect.
		var x0 := rect.position.x + offset
		var start := Vector2(maxf(x0, rect.position.x), 0.0)
		start.y = rect.end.y - (start.x - x0)
		var end := Vector2(minf(x0 + rect.size.y, rect.end.x), 0.0)
		end.y = rect.end.y - (end.x - x0)
		if end.x > start.x:
			canvas.draw_line(start, end, color, 1.5, true)
		offset += spacing

static func _grass(canvas: CanvasItem, rect: Rect2) -> void:
	var blade := GRASS_FILL.lightened(0.3)
	var step := 7.0
	var y := rect.position.y + 5.0
	var row := 0
	while y < rect.end.y - 2.0:
		var x := rect.position.x + 3.0 + (row % 2) * step * 0.5
		while x < rect.end.x - 3.0:
			canvas.draw_line(Vector2(x, y + 3.0), Vector2(x - 1.5, y), blade, 1.0)
			canvas.draw_line(Vector2(x, y + 3.0), Vector2(x + 1.5, y), blade, 1.0)
			x += step
		y += step
		row += 1

# Gravel dots between two edge lines, one each side of the band.
static func _gravel(canvas: CanvasItem, rect: Rect2) -> void:
	var stone := SHOULDER_FILL.lightened(0.25)
	var step := 5.0
	var y := rect.position.y + 3.0
	var row := 0
	while y < rect.end.y - 1.0:
		var x := rect.position.x + 2.0 + (row % 2) * step * 0.5
		while x < rect.end.x - 1.0:
			canvas.draw_rect(Rect2(x, y, 1.5, 1.5), stone)
			x += step
		y += step
		row += 1
	canvas.draw_line(rect.position, Vector2(rect.position.x, rect.end.y), MARKING, 1.5)
	canvas.draw_line(Vector2(rect.end.x, rect.position.y), rect.end, MARKING, 1.5)

static func _paving(canvas: CanvasItem, rect: Rect2) -> void:
	var joint := PAVING_FILL.darkened(0.25)
	var step := 6.0
	var y := rect.position.y + step
	var row := 0
	while y < rect.end.y:
		canvas.draw_line(Vector2(rect.position.x, y), Vector2(rect.end.x, y), joint, 1.0)
		var x := rect.position.x + step * (0.5 if row % 2 else 1.0)
		while x < rect.end.x:
			canvas.draw_line(Vector2(x, y - step), Vector2(x, y), joint, 1.0)
			x += step
		y += step
		row += 1

# An arrow along the lane: up when it runs the way the road is drawn, down when against.
static func _arrow(canvas: CanvasItem, box: Rect2, forward: bool) -> void:
	var tip := Vector2(box.get_center().x, box.position.y if forward else box.end.y)
	var tail := Vector2(box.get_center().x, box.end.y if forward else box.position.y)
	var head := box.size.y * 0.42 * (1.0 if forward else -1.0)
	canvas.draw_line(tail, tip, INK, 2.0, true)
	canvas.draw_colored_polygon(PackedVector2Array([
		tip, tip + Vector2(-box.size.x * 0.32, head), tip + Vector2(box.size.x * 0.32, head),
	]), INK)

# A walking figure on a paving band.
static func _pedestrian(canvas: CanvasItem, box: Rect2) -> void:
	canvas.draw_rect(box, SIDEWALK)
	var s := box.size.x / 24.0
	var c := box.position
	canvas.draw_circle(c + Vector2(12.5, 4.5) * s, 2.2 * s, INK)
	canvas.draw_line(c + Vector2(12.0, 7.5) * s, c + Vector2(11.0, 14.0) * s, INK, 2.0 * s, true)
	canvas.draw_line(c + Vector2(11.0, 14.0) * s, c + Vector2(8.0, 20.5) * s, INK, 1.8 * s, true)
	canvas.draw_line(c + Vector2(11.0, 14.0) * s, c + Vector2(14.5, 20.5) * s, INK, 1.8 * s, true)
	canvas.draw_line(c + Vector2(12.0, 9.0) * s, c + Vector2(8.5, 12.5) * s, INK, 1.6 * s, true)
	canvas.draw_line(c + Vector2(12.0, 9.0) * s, c + Vector2(15.5, 11.5) * s, INK, 1.6 * s, true)

# Short painted ticks across the lane every 12 px: the ends of parallel bays.
static func _bay_lines(canvas: CanvasItem, rect: Rect2) -> void:
	var y := rect.position.y + 6.0
	while y < rect.end.y:
		canvas.draw_line(Vector2(rect.position.x + 2.0, y), Vector2(rect.end.x - 2.0, y), MARKING, 1.0)
		y += 12.0

# A blue square with a white border and a white P, like the E1 parking sign.
static func _parking_sign(canvas: CanvasItem, box: Rect2) -> void:
	canvas.draw_rect(box, INK)
	canvas.draw_rect(box.grow(-box.size.x * 0.08), SIGN_BLUE)
	var font := ThemeDB.fallback_font
	var size := int(box.size.y * 0.8)
	var glyph := font.get_string_size("P", HORIZONTAL_ALIGNMENT_LEFT, -1, size)
	var baseline := Vector2(box.get_center().x - glyph.x * 0.5, box.get_center().y + font.get_ascent(size) * 0.5 - font.get_descent(size) * 0.25)
	canvas.draw_string(font, baseline, "P", HORIZONTAL_ALIGNMENT_LEFT, -1, size, INK)
