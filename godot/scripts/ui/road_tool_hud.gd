# SPDX-License-Identifier: GPL-2.0-only

## The road tool's HUD (`ROAD-112` rounds D and V): label chips on the geometry, each a small
## dark rounded panel with a vector icon and one value, the same pixel size at every zoom.
## Rust's frame decides which chips exist, their values, tones, world anchors and the side each
## is pushed to (`label_*`); `SimulationNode.road_tool_place_labels` places them without overlap
## (one deterministic rule). This script only words the values, projects the anchors and draws.
## A notice chip words the frame's refusal or the preview's verdict through
## `road_reason_text.gd` (red for a refusal, amber for a warning or a stroke being sent). An
## open freeform draft adds a hint chip (Rust's wording) and, in its edit phase, the clickable
## Build and Cancel chips beside its last node: their clicks are the GUI's (they never reach the
## world), emitting `build_pressed` and `cancel_pressed`. Chips keep clear of the handles.
extends CanvasLayer

## The Build chip was clicked.
signal build_pressed
## The Cancel chip was clicked.
signal cancel_pressed

const RoadReasonText := preload("res://scripts/ui/road_reason_text.gd")
const UiStyle := preload("res://scripts/ui/ui_style.gd")

## Chip kinds as `road_tool_frame` numbers them (`label_kinds`).
enum Kind { PIECE_LENGTH, TOTAL_LENGTH, RADIUS, ANGLE, SLOPE, COST, BEARING, DESIGN_SPEED, NOTICE, HINT, BUILD, CANCEL }
## Chip tones (`label_tones`).
enum Tone { NORMAL, WARNING, REFUSED }

## The chips' look, from the UI theme (`ui_style.gd`): panel, text and the two alert tones.
const CHIP_BG := Color(0.07, 0.07, 0.07, 0.82)
const CHIP_TEXT := Color(1.0, 1.0, 1.0, 0.96)
const CHIP_ICON := Color(0.72, 0.74, 1.0, 0.95)
const TONE_COLORS := [CHIP_TEXT, Color(1.0, 0.76, 0.18, 0.98), Color(1.0, 0.40, 0.30, 0.98)]
const FONT_SIZE := 14
const ICON_PX := 12.0
const PAD_X := 7.0
const PAD_Y := 4.0
const ICON_GAP := 5.0
const CORNER := 5
## Pixels between a chip and its anchor, and between chips.
const GAP_PX := 6.0
## The square kept clear about each handle, in pixels.
const HANDLE_CLEAR_PX := 18.0
const BUTTON_BG := Color(0.16, 0.16, 0.24, 0.92)
const COMPASS := ["N", "NE", "E", "SE", "S", "SW", "W", "NW"]

## The node that places the chips (`road_tool_place_labels`); without one a chip sits on its
## anchor.
var simulation_node: Object = null

var _canvas: Control
var _font: Font
var _style := StyleBoxFlat.new()
# This frame's chips: kind, tone, text, size, world anchor and side, and their screen corners.
var _kinds := PackedInt32Array()
var _tones := PackedInt32Array()
var _texts := PackedStringArray()
var _sizes := PackedVector2Array()
var _anchors := PackedVector3Array()
var _sides := PackedVector3Array()
var _corners := PackedVector2Array()
var _shown := PackedByteArray()
# The frame arrays the chips were last built from: an unchanged frame rewords nothing.
var _values := PackedFloat64Array()
var _verdict := ""
var _handles := PackedVector3Array()
# The chips' font size (`UiStyle.scaled_font_size` reads the settings: taken again whenever the
# chips are cleared, not every frame).
var _font_size := 0
# Character advances in pixels at `_font_size` (`_text_width`).
var _advances := {}
# What the chips were last placed for (camera and chips): an unchanged frame places nothing.
var _placed_view := Transform3D()
var _placed_fov := 0.0
var _placed_size := Vector2.ZERO
var _placed_dirty := true
# The clickable chips' hit areas (GUI controls, so their clicks stop at the GUI).
var _build_hit: Control
var _cancel_hit: Control
var _button_style := StyleBoxFlat.new()

func _init() -> void:
	layer = 10
	_canvas = Control.new()
	_canvas.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_canvas.set_anchors_preset(Control.PRESET_FULL_RECT)
	_canvas.draw.connect(_draw_chips)
	add_child(_canvas)
	_font = ThemeDB.fallback_font
	_style.bg_color = CHIP_BG
	_style.set_corner_radius_all(CORNER)
	_style.anti_aliasing = true
	_button_style.bg_color = BUTTON_BG
	_button_style.set_corner_radius_all(CORNER)
	_button_style.set_border_width_all(1)
	_button_style.border_color = CHIP_ICON
	_button_style.anti_aliasing = true
	_build_hit = _hit_area(build_pressed)
	_cancel_hit = _hit_area(cancel_pressed)

# An invisible GUI control that stops a left press on it and emits `pressed`.
func _hit_area(pressed: Signal) -> Control:
	var hit := Control.new()
	hit.mouse_filter = Control.MOUSE_FILTER_STOP
	hit.visible = false
	hit.gui_input.connect(func(event: InputEvent) -> void:
		if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
			hit.accept_event()
			if event.pressed:
				pressed.emit())
	add_child(hit)
	return hit

## Takes the frame's chips: words them (only when the frame's chips or the verdict changed) from
## the `label_*` arrays, the draft `refusal` key and `preview` (the stroke's preview, for a
## notice). Returns whether there are chips.
func show_labels(frame: Dictionary, preview: Dictionary) -> bool:
	var kinds: PackedInt32Array = frame.get("label_kinds", PackedInt32Array())
	var values: PackedFloat64Array = frame.get("label_values", PackedFloat64Array())
	var tones: PackedInt32Array = frame.get("label_tones", PackedInt32Array())
	_anchors = frame.get("label_anchors", PackedVector3Array())
	_sides = frame.get("label_sides", PackedVector3Array())
	var handles: PackedVector3Array = frame.get("gizmo_handles", PackedVector3Array())
	var refusal := String(frame.get("refusal", ""))
	var hint := String(frame.get("label_hint", ""))
	var verdict := "%s|%s|%s|%s|%s" % [refusal, hint, preview.get("invalid_reason", ""), preview.get("warning_reason", ""), preview.get("demolish_count", 0)]
	if kinds == _kinds and values == _values and tones == _tones and verdict == _verdict and handles == _handles:
		return not kinds.is_empty()
	_handles = handles
	var reword := verdict != _verdict or kinds.size() != _kinds.size()
	var old_kinds := _kinds
	var old_values := _values
	var old_tones := _tones
	_kinds = kinds
	_values = values
	_tones = tones
	_verdict = verdict
	_placed_dirty = true
	_texts.resize(kinds.size())
	_sizes.resize(kinds.size())
	var hud: Dictionary = frame.get("hud", {})
	if _font_size == 0:
		_font_size = UiStyle.scaled_font_size(FONT_SIZE)
		_advances.clear()
	var font_size := _font_size
	for index in kinds.size():
		# Only a chip whose kind, value or tone changed is reworded (the verdict rewords all).
		if not reword and index < old_kinds.size() and kinds[index] == old_kinds[index] and values[index] == old_values[index] and tones[index] == old_tones[index]:
			continue
		_texts[index] = hint if kinds[index] == Kind.HINT else chip_text(kinds[index], values[index], tones[index], refusal, hud, preview)
		_sizes[index] = Vector2(PAD_X * 2.0 + ICON_PX + ICON_GAP + _text_width(_texts[index], font_size), maxf(_font.get_height(font_size), ICON_PX) + PAD_Y * 2.0)
	return not kinds.is_empty()

# A text's width as the sum of its characters' advances, cached per character: a drag rewords
# chips every frame, and shaping each new string costs several times this.
func _text_width(text: String, font_size: int) -> float:
	var width := 0.0
	for index in text.length():
		var code := text.unicode_at(index)
		var advance = _advances.get(code)
		if advance == null:
			advance = _font.get_char_size(code, font_size).x
			_advances[code] = advance
		width += advance
	return ceilf(width)

## Removes every chip.
func clear() -> void:
	_kinds = PackedInt32Array()
	_values = PackedFloat64Array()
	_texts = PackedStringArray()
	_corners = PackedVector2Array()
	_verdict = ""
	_placed_dirty = true
	_font_size = 0
	_build_hit.visible = false
	_cancel_hit.visible = false
	_canvas.queue_redraw()

## The chips' texts in frame order, joined by two spaces (tests read the HUD this way).
func text() -> String:
	return "  ".join(_texts)

## Projects the chips' anchors and sides with `camera` and places them (Rust's rule); returns the
## milliseconds it took (0 without chips).
func place(camera: Camera3D) -> float:
	if _kinds.is_empty() or camera == null:
		_build_hit.visible = false
		_cancel_hit.visible = false
		if not _corners.is_empty():
			_corners = PackedVector2Array()
			_canvas.queue_redraw()
		return 0.0
	var view := camera.global_transform
	var viewport_size := camera.get_viewport().get_visible_rect().size
	if not _placed_dirty and view == _placed_view and camera.fov == _placed_fov and viewport_size == _placed_size:
		return 0.0
	var start_us := Time.get_ticks_usec()
	_placed_view = view
	_placed_fov = camera.fov
	_placed_size = viewport_size
	_placed_dirty = false
	var anchors := PackedVector2Array()
	var sides := PackedVector2Array()
	var sizes := PackedVector2Array()
	_shown.resize(_kinds.size())
	for index in _kinds.size():
		var anchor := _anchors[index]
		_shown[index] = 0 if camera.is_position_behind(anchor) else 1
		if _shown[index] == 0:
			continue
		var at := camera.unproject_position(anchor)
		anchors.append(at)
		sides.append(camera.unproject_position(anchor + _sides[index] * 4.0) - at)
		sizes.append(_sizes[index])
	var obstacles := PackedVector2Array()
	for handle in _handles:
		if not camera.is_position_behind(handle):
			obstacles.append(camera.unproject_position(handle))
	if simulation_node != null:
		_corners = simulation_node.road_tool_place_labels(anchors, sides, sizes, obstacles, HANDLE_CLEAR_PX, GAP_PX)
	else:
		_corners = anchors
	_place_hit(_build_hit, Kind.BUILD)
	_place_hit(_cancel_hit, Kind.CANCEL)
	_canvas.queue_redraw()
	return float(Time.get_ticks_usec() - start_us) / 1000.0

## The screen rectangle of the shown chip of `kind`, or an empty one.
func chip_rect(kind: int) -> Rect2:
	var shown := 0
	for index in _kinds.size():
		if index >= _shown.size() or _shown[index] == 0:
			continue
		if _kinds[index] == kind and shown < _corners.size():
			return Rect2(_corners[shown], _sizes[index])
		shown += 1
	return Rect2()

# Puts a clickable chip's hit area over its chip, or hides it.
func _place_hit(hit: Control, kind: int) -> void:
	var rect := chip_rect(kind)
	hit.visible = rect.has_area()
	if hit.visible:
		hit.position = rect.position
		hit.size = rect.size

## A chip's text: `value` of `kind` worded, or for a notice the refusal (`refusal`, a draft's;
## else `preview`'s) or the warning the preview carries.
static func chip_text(kind: int, value: float, tone: int, refusal: String, hud: Dictionary, preview: Dictionary) -> String:
	match kind:
		Kind.PIECE_LENGTH, Kind.TOTAL_LENGTH:
			return ("%.0f m" if value >= 100.0 else "%.1f m") % value
		Kind.RADIUS:
			return "R %.0f m" % value if value >= 100.0 else "R %.1f m" % value
		Kind.ANGLE:
			return "%d°" % int(value)
		Kind.SLOPE:
			return "%.1f %%" % value
		Kind.COST:
			return _money(value)
		Kind.BEARING:
			return "%s %d°" % [COMPASS[int(round(value / 45.0)) % 8], int(round(value)) % 360]
		Kind.DESIGN_SPEED:
			return "%d km/h" % int(value)
		Kind.NOTICE:
			return _notice_text(tone, refusal, hud, preview)
		Kind.BUILD:
			return "Build"
		Kind.CANCEL:
			return "Cancel"
	return ""

static func _notice_text(tone: int, refusal: String, hud: Dictionary, preview: Dictionary) -> String:
	if tone == Tone.REFUSED:
		if not refusal.is_empty():
			return RoadReasonText.text({
				"is_valid": false,
				"invalid_reason": refusal,
				"geometry_measured": float(hud.get("radius_m", 0.0)),
				"geometry_limit": float(hud.get("illegal_radius_m", 0.0)),
			})
		return RoadReasonText.text(preview)
	var parts := PackedStringArray()
	# A grade above the design tier is shown, not refused (ROAD-68 question 37).
	if String(preview.get("warning_reason", "")) == "road_steep":
		parts.append("steep %.0f%% (design %.0f%%)" % [float(preview.get("max_grade", 0.0)) * 100.0, float(preview.get("warning_limit", 0.0)) * 100.0])
	var demolish := int(preview.get("demolish_count", 0))
	if demolish > 0:
		parts.append("demolishes %d building%s" % [demolish, "" if demolish == 1 else "s"])
	var state := String(preview.get("plan_state", ""))
	if not state.is_empty() and state != "ready":
		parts.append("checking road")
	return "Placing…" if parts.is_empty() else " · ".join(parts)

static func _money(value: float) -> String:
	var digits := str(int(round(absf(value))))
	var grouped := ""
	while digits.length() > 3:
		grouped = "," + digits.right(3) + grouped
		digits = digits.left(digits.length() - 3)
	return ("-$" if value < 0.0 else "$") + digits + grouped

func _draw_chips() -> void:
	var font_size := _font_size if _font_size > 0 else UiStyle.scaled_font_size(FONT_SIZE)
	var shown := 0
	for index in _kinds.size():
		if index >= _shown.size() or _shown[index] == 0:
			continue
		if shown >= _corners.size():
			break
		var rect := Rect2(_corners[shown], _sizes[index])
		shown += 1
		var button := _kinds[index] == Kind.BUILD or _kinds[index] == Kind.CANCEL
		_canvas.draw_style_box(_button_style if button else _style, rect)
		var tone := _tones[index]
		var color: Color = TONE_COLORS[tone]
		var icon := Rect2(rect.position + Vector2(PAD_X, (rect.size.y - ICON_PX) * 0.5), Vector2(ICON_PX, ICON_PX))
		_draw_icon(_kinds[index], tone, icon, CHIP_ICON if tone == Tone.NORMAL else color)
		var baseline := rect.position.y + (rect.size.y + _font.get_ascent(font_size) - _font.get_descent(font_size)) * 0.5
		_canvas.draw_string(_font, Vector2(icon.end.x + ICON_GAP, baseline), _texts[index], HORIZONTAL_ALIGNMENT_LEFT, -1, font_size, color)

# Small vector icons, drawn in `box`: a double arrow (a piece's length; with end bars, the total),
# an arc and its radius, an angle, a slope, a coin, a compass needle, a speed gauge, a warning
# triangle or a refusal's crossed circle.
func _draw_icon(kind: int, tone: int, box: Rect2, color: Color) -> void:
	var c := box.get_center()
	var h := box.size.x * 0.5
	match kind:
		Kind.PIECE_LENGTH, Kind.TOTAL_LENGTH:
			var a := c - Vector2(h, 0.0)
			var b := c + Vector2(h, 0.0)
			_canvas.draw_line(a, b, color, 1.5, true)
			_canvas.draw_polyline(PackedVector2Array([a + Vector2(3.5, -3.0), a, a + Vector2(3.5, 3.0)]), color, 1.5, true)
			_canvas.draw_polyline(PackedVector2Array([b + Vector2(-3.5, -3.0), b, b + Vector2(-3.5, 3.0)]), color, 1.5, true)
			if kind == Kind.TOTAL_LENGTH:
				_canvas.draw_line(a + Vector2(0.0, -h), a + Vector2(0.0, h), color, 1.5, true)
				_canvas.draw_line(b + Vector2(0.0, -h), b + Vector2(0.0, h), color, 1.5, true)
		Kind.RADIUS:
			var centre := c + Vector2(-h, h)
			_canvas.draw_arc(centre, h * 2.0, -PI * 0.5, 0.0, 10, color, 1.5, true)
			_canvas.draw_line(centre, centre + Vector2(h, -h) * 1.41 * 0.7, color, 1.5, true)
		Kind.ANGLE:
			var corner := c + Vector2(-h, h)
			_canvas.draw_polyline(PackedVector2Array([c + Vector2(h, -h), corner, c + Vector2(h, h)]), color, 1.5, true)
			_canvas.draw_arc(corner, h, -PI * 0.25, 0.0, 6, color, 1.2, true)
		Kind.SLOPE:
			_canvas.draw_colored_polygon(PackedVector2Array([c + Vector2(-h, h), c + Vector2(h, h), c + Vector2(h, -h * 0.4)]), color)
		Kind.COST:
			_canvas.draw_arc(c, h, 0.0, TAU, 16, color, 1.5, true)
			_canvas.draw_line(c + Vector2(0.0, -h * 0.55), c + Vector2(0.0, h * 0.55), color, 1.5, true)
		Kind.BEARING:
			_canvas.draw_arc(c, h, 0.0, TAU, 16, color, 1.2, true)
			_canvas.draw_colored_polygon(PackedVector2Array([c + Vector2(0.0, -h * 0.85), c + Vector2(h * 0.35, 0.0), c + Vector2(-h * 0.35, 0.0)]), color)
		Kind.DESIGN_SPEED:
			_canvas.draw_arc(c + Vector2(0.0, h * 0.4), h, PI, TAU, 12, color, 1.5, true)
			_canvas.draw_line(c + Vector2(0.0, h * 0.4), c + Vector2(h * 0.55, -h * 0.35), color, 1.5, true)
		Kind.HINT:
			_canvas.draw_arc(c, h, 0.0, TAU, 16, color, 1.5, true)
			_canvas.draw_line(c + Vector2(0.0, -h * 0.1), c + Vector2(0.0, h * 0.55), color, 1.5, true)
			_canvas.draw_circle(c + Vector2(0.0, -h * 0.45), 1.2, color)
		Kind.BUILD:
			_canvas.draw_polyline(PackedVector2Array([c + Vector2(-h, 0.0), c + Vector2(-h * 0.3, h * 0.7), c + Vector2(h, -h * 0.7)]), Color(0.35, 0.95, 0.55), 2.0, true)
		Kind.CANCEL:
			_canvas.draw_line(c + Vector2(-h, -h) * 0.75, c + Vector2(h, h) * 0.75, TONE_COLORS[Tone.REFUSED], 2.0, true)
			_canvas.draw_line(c + Vector2(h, -h) * 0.75, c + Vector2(-h, h) * 0.75, TONE_COLORS[Tone.REFUSED], 2.0, true)
		Kind.NOTICE:
			if tone == Tone.REFUSED:
				_canvas.draw_arc(c, h, 0.0, TAU, 16, color, 1.5, true)
				_canvas.draw_line(c + Vector2(-h, -h) * 0.45, c + Vector2(h, h) * 0.45, color, 1.5, true)
				_canvas.draw_line(c + Vector2(h, -h) * 0.45, c + Vector2(-h, h) * 0.45, color, 1.5, true)
			else:
				_canvas.draw_polyline(PackedVector2Array([c + Vector2(0.0, -h), c + Vector2(h, h), c + Vector2(-h, h), c + Vector2(0.0, -h)]), color, 1.5, true)
				_canvas.draw_line(c + Vector2(0.0, -h * 0.3), c + Vector2(0.0, h * 0.35), color, 1.5, true)
