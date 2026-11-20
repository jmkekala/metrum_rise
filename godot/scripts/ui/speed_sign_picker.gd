# SPDX-License-Identifier: GPL-3.0-or-later

## A speed limit picker drawn as Finnish speed limit signs (a yellow disc in a red ring, black
## numerals; the sign alone, no pole): a strip of `full_signs` whole signs (1 or 3) with the
## chosen limit in the middle, larger and lit, and one more sign on each side cut in half by
## the edge, so the strip reads as continuing. The mouse wheel steps one limit (down is
## faster), a click on a sign left or right of the middle picks it.
##
## The signs sit in a recessed track whose ends, half signs included, fade into the panel (a
## canvas mask on the strip). A step slides the strip and the chosen sign swells slightly for a
## moment; scrolling past the first or last limit leans the strip that way and eases it back. A side
## sign under the cursor lights.
##
## Its callers set `values` (the limits, slowest first), `value` (setting it emits
## `value_changed`, as SpinBox does) and `set_value_no_signal`; a value off the list snaps to the
## nearest limit. The limits come from Rust (`get_road_types().speeds.kmh`); the picker does not
## decide which are allowed. The strip steps one limit at a time, so 80 is next to 100.
extends Control

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const RING := Color(0.80, 0.10, 0.14)
const DISC := UIStyle.SIGN_YELLOW
const INK := Color(0.06, 0.06, 0.07)
## Base height; the width follows from `full_signs`.
const BASE_HEIGHT := 56.0
const SLIDE_S := 0.12
# The chosen sign's swell: up to POP_SCALE larger over POP_IN_S, back over POP_OUT_S.
const POP_IN_S := 0.07
const POP_OUT_S := 0.18
const NUDGE_S := 0.20
# How far (in slots) the strip leans when scrolled past an end.
const NUDGE_SLOTS := 0.03
const POP_SCALE := 0.015
# The neighbours' size and opacity next to the chosen sign; a hovered one lights to HOVER_ALPHA.
const SIDE_SCALE := 0.78
const SIDE_ALPHA := 0.55
const HOVER_ALPHA := 0.85
# The faded band at each end, in sign spacings.
const FADE_SPACINGS := 0.45
# Fades the strip's ends to transparent; `fade_px` is the band's width.
const FADE_SHADER := """
shader_type canvas_item;
uniform float width_px = 100.0;
uniform float fade_px = 20.0;
varying float x;
void vertex() { x = VERTEX.x; }
void fragment() {
	COLOR.a *= smoothstep(0.0, fade_px, x) * smoothstep(0.0, fade_px, width_px - x);
}
"""

static var _fade_shader: Shader

## Emitted with the new limit (km/h) when the player picks one or `value` is set.
signal value_changed(value: float)

## The strip's height before UI scaling (`BASE_HEIGHT` in the road builder); the width follows.
var base_height := BASE_HEIGHT:
	set(v):
		base_height = v
		_resize()

## Whole signs in the strip: 1 (the chosen one) or 3; a half sign shows at each edge too.
var full_signs := 1:
	set(v):
		full_signs = 3 if v >= 3 else 1
		_resize()

## The limits in km/h, slowest first; never empty.
var values := PackedInt32Array([20, 30, 40, 50, 60, 70, 80, 100, 120]):
	set(v):
		values = v if not v.is_empty() else PackedInt32Array([50])
		_set_silently(_clamped(value))
		_shown = _index()
var value := 50.0:
	set(v):
		var snapped_value := _clamped(v)
		var changed := not is_equal_approx(snapped_value, value)
		value = snapped_value
		if changed:
			_animate_to(_index())
			value_changed.emit(value)

# Animated draw state, each redrawing on change: the slot (index into `values`) the strip is drawn
# at (it eases to the chosen limit's, so the signs slide), the chosen sign's swell (0 → 1 → 0) and the end-of-range lean
# in slots.
var _shown := 3.0:
	set(v):
		_shown = v
		queue_redraw()
var _pop := 0.0:
	set(v):
		_pop = v
		queue_redraw()
var _nudge := 0.0:
	set(v):
		_nudge = v
		queue_redraw()
var _slide_tween: Tween
var _pop_tween: Tween
var _nudge_tween: Tween
# The cursor's x over the strip, or NAN when it is elsewhere.
var _hover_x := NAN
var _font: Font
var _track := StyleBoxFlat.new()

func _init() -> void:
	focus_mode = Control.FOCUS_NONE
	mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
	clip_contents = true
	_resize()
	_font = sign_font()
	_track.border_width_left = 1
	_track.border_width_top = 1
	_track.border_width_right = 1
	_track.border_width_bottom = 1
	_track.anti_aliasing = true
	if _fade_shader == null:
		_fade_shader = Shader.new()
		_fade_shader.code = FADE_SHADER
	material = ShaderMaterial.new()
	(material as ShaderMaterial).shader = _fade_shader
	resized.connect(_update_mask)
	mouse_exited.connect(func():
		_hover_x = NAN
		queue_redraw())

## Sets the limit without emitting `value_changed`, and without the slide.
func set_value_no_signal(v: float) -> void:
	_set_silently(_clamped(v))

# Writes the value back without the signal and stops any animation; the value already set is a
# no-op, so a caller echoing the limit it was just sent (the builder's `set_speed`, a refresh of
# the grid) does not cut the slide and swell short.
func _set_silently(v: float) -> void:
	if is_equal_approx(v, value):
		return
	set_block_signals(true)
	value = v
	set_block_signals(false)
	for tween in [_slide_tween, _pop_tween, _nudge_tween]:
		if is_instance_valid(tween):
			tween.kill()
	_shown = _index()
	_pop = 0.0
	_nudge = 0.0

func _gui_input(event: InputEvent) -> void:
	var motion := event as InputEventMouseMotion
	if motion:
		_hover_x = motion.position.x
		queue_redraw()
		return
	var button := event as InputEventMouseButton
	if button == null or not button.pressed:
		return
	match button.button_index:
		MOUSE_BUTTON_WHEEL_DOWN, MOUSE_BUTTON_WHEEL_RIGHT:
			_step_by(1)
			accept_event()
		MOUSE_BUTTON_WHEEL_UP, MOUSE_BUTTON_WHEEL_LEFT:
			_step_by(-1)
			accept_event()
		MOUSE_BUTTON_LEFT:
			# Each sign is one slot wide; a click picks the slot's limit.
			var slots := _slot_at(button.position.x)
			if slots != 0:
				_step_by(slots)
			accept_event()

# Steps `slots` limits; at an end of the range the strip leans that way and springs back.
func _step_by(slots: int) -> void:
	var target := clampi(_index() + slots, 0, values.size() - 1)
	if target == _index():
		_lean(signf(slots))
	else:
		value = values[target]

func _draw() -> void:
	var hovered := not is_nan(_hover_x)
	_draw_track(Rect2(Vector2.ZERO, size), hovered)
	var centre := size * 0.5
	var spacing := _spacing()
	var radius := _radius()
	# Slide: while easing, the row is drawn `offset` slots from its rest.
	var chosen := _index()
	var offset := _shown - float(chosen) - _nudge
	var hover_slot := _slot_at(_hover_x) if hovered else 0
	var reach := (full_signs + 1) / 2 + 1
	for i in range(-reach, reach + 1):
		if chosen + i < 0 or chosen + i >= values.size():
			continue
		var kmh := values[chosen + i]
		var at := float(i) + offset
		var near := clampf(1.0 - absf(at), 0.0, 1.0)
		var scale := lerpf(SIDE_SCALE, 1.0, near)
		if i == 0:
			scale *= 1.0 + POP_SCALE * _pop
		var alpha := lerpf(SIDE_ALPHA, 1.0, near)
		if i != 0 and i == hover_slot:
			alpha = maxf(alpha, HOVER_ALPHA)
		_draw_sign(Vector2(centre.x + at * spacing, centre.y), radius * scale, kmh, alpha)

# The recessed pill the signs sit in; a touch lighter under the cursor.
func _draw_track(rect: Rect2, hovered: bool) -> void:
	_track.bg_color = Color(1, 1, 1, 0.07 if hovered else 0.04)
	_track.border_color = Color(1, 1, 1, 0.16 if hovered else 0.09)
	_track.set_corner_radius_all(int(rect.size.y * 0.5))
	draw_style_box(_track, rect.grow_individual(0, -1, 0, -1))

func _draw_sign(at: Vector2, radius: float, kmh: int, alpha: float) -> void:
	paint_sign(self, _font, at, radius, kmh, alpha)

## The sign numerals' font: bold, tall and narrow like a road sign's.
static func sign_font() -> Font:
	var bold := FontVariation.new()
	bold.base_font = ThemeDB.fallback_font
	bold.variation_embolden = 0.9
	bold.variation_transform = Transform2D(Vector2(0.86, 0), Vector2(0, 1), Vector2.ZERO)
	return bold

## Draws a `kmh` speed limit sign of `radius` centred at `at` on `canvas`, in `font`
## (`sign_font`); the road type card shows the same sign.
static func paint_sign(canvas: CanvasItem, font: Font, at: Vector2, radius: float, kmh: int, alpha := 1.0) -> void:
	canvas.draw_circle(at + Vector2(0, radius * 0.06), radius, Color(0, 0, 0, 0.35 * alpha))
	canvas.draw_circle(at, radius, Color(RING, alpha))
	canvas.draw_circle(at, radius * 0.80, Color(DISC, alpha))
	var text := str(kmh)
	var font_size := int(radius * (0.95 if text.length() <= 2 else 0.78))
	var width := font.get_string_size(text, HORIZONTAL_ALIGNMENT_LEFT, -1, font_size).x
	var baseline := at.y + (font.get_ascent(font_size) - font.get_descent(font_size)) * 0.5
	canvas.draw_string(font, Vector2(at.x - width * 0.5, baseline), text, HORIZONTAL_ALIGNMENT_LEFT, -1,
		font_size, Color(INK, alpha))

func _update_mask() -> void:
	var mask := material as ShaderMaterial
	mask.set_shader_parameter("width_px", size.x)
	mask.set_shader_parameter("fade_px", _spacing() * FADE_SPACINGS)

# The slot (signs from the middle) under x.
func _slot_at(x: float) -> int:
	return roundi((x - size.x * 0.5) / _spacing())

# Centre to centre of neighbouring signs.
func _spacing() -> float:
	return _radius() * 2.15

# Wide enough for the whole signs and half of one more at each edge.
func _resize() -> void:
	var height := UIStyle.scaled_px(base_height)
	custom_minimum_size = Vector2(height * 0.46 * 2.15 * float(full_signs + 1), height)
	queue_redraw()

func _radius() -> float:
	return size.y * 0.46

# The listed limit nearest `v`; the slower one on a tie.
func _clamped(v: float) -> float:
	var best := values[0]
	for kmh in values:
		if absf(kmh - v) < absf(best - v):
			best = kmh
	return best

# The chosen limit's slot in `values`.
func _index() -> int:
	return maxi(values.find(int(value)), 0)

# The slide to slot `target`, and the chosen sign's swell.
func _animate_to(target: float) -> void:
	if not is_inside_tree():
		_shown = target
		return
	if is_instance_valid(_slide_tween):
		_slide_tween.kill()
	_slide_tween = create_tween().set_trans(Tween.TRANS_CUBIC).set_ease(Tween.EASE_OUT)
	_slide_tween.tween_property(self, "_shown", target, SLIDE_S)
	if is_instance_valid(_pop_tween):
		_pop_tween.kill()
	_pop_tween = create_tween()
	_pop_tween.tween_property(self, "_pop", 1.0, POP_IN_S).set_trans(Tween.TRANS_QUAD).set_ease(Tween.EASE_OUT)
	_pop_tween.tween_property(self, "_pop", 0.0, POP_OUT_S).set_trans(Tween.TRANS_QUAD).set_ease(Tween.EASE_IN_OUT)

# Scrolled past an end: lean `direction` slots-ward and ease back, without a wobble.
func _lean(direction: float) -> void:
	if not is_inside_tree():
		return
	if is_instance_valid(_nudge_tween):
		_nudge_tween.kill()
	_nudge_tween = create_tween()
	_nudge_tween.tween_property(self, "_nudge", direction * NUDGE_SLOTS, NUDGE_S * 0.3) \
		.set_trans(Tween.TRANS_QUAD).set_ease(Tween.EASE_OUT)
	_nudge_tween.tween_property(self, "_nudge", 0.0, NUDGE_S * 0.7) \
		.set_trans(Tween.TRANS_SINE).set_ease(Tween.EASE_IN_OUT)
