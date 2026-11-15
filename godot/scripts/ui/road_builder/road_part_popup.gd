# SPDX-License-Identifier: GPL-2.0-only

## The road builder's part popup (`ROAD-101`), opened by clicking a part in the strip: one row,
## the clicked part being highlighted on the strip already. For a median, its style swatches and
## width (the reusable median controls, compact and without "None"); then move left and move
## right as arrows and remove as the bulldozer in red (a cross would read as closing the popup),
## named by their tooltips. A view: it reports the player's
## choice through its signals and the builder edits the layout.
extends PopupPanel

## The player changed the median's style or width.
signal median_changed(style: int, width_m: float)
## The player asked to move the part one place left (-1) or right (+1).
signal move_requested(step: int)
## The player asked to remove the part.
signal remove_requested

const MedianControls = preload("res://scripts/ui/median_controls.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const ICON_SIDE := 30.0
const BULLDOZE := preload("res://assets/ui/icons/bulldoze.svg")

enum Glyph { LEFT, RIGHT, REMOVE }

# A square button drawing its glyph: an arrow, or the bulldozer in the alert red.
class GlyphButton extends Button:
	var glyph := Glyph.LEFT

	func _draw() -> void:
		var colour := UIStyle.TEXT_ALERT if glyph == Glyph.REMOVE else UIStyle.TEXT_PRIMARY
		if disabled:
			colour = Color(colour, 0.3)
		var c := size * 0.5
		var r := minf(size.x, size.y) * 0.22
		if glyph == Glyph.REMOVE:
			var side := r * 3.6
			draw_texture_rect(BULLDOZE, Rect2(c - Vector2(side, side) * 0.5, Vector2(side, side)), false, colour)
			return
		var dir := -1.0 if glyph == Glyph.LEFT else 1.0
		var tip := c + Vector2(dir * r * 1.15, 0.0)
		draw_line(c - Vector2(dir * r * 1.15, 0.0), tip, colour, 2.2, true)
		draw_polyline(PackedVector2Array([
			tip + Vector2(-dir * r * 0.8, -r * 0.8), tip, tip + Vector2(-dir * r * 0.8, r * 0.8),
		]), colour, 2.2, true)

## The median controls (shown for a median part only).
var median_controls: MedianControls
var move_left_button: Button
var move_right_button: Button
var remove_button: Button

var _median_gap: Control

func _init() -> void:
	add_theme_stylebox_override("panel", UIStyle.panel_style(UIStyle.BG_DARK, UIStyle.CORNER_WINDOW))
	var margin := MarginContainer.new()
	for side in ["left", "right", "top", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, 6)
	add_child(margin)
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 2)
	margin.add_child(row)
	median_controls = MedianControls.new(13, true)
	median_controls.offer_none(false)
	median_controls.changed.connect(func(style: int, width: float): median_changed.emit(style, width))
	row.add_child(median_controls)
	_median_gap = VSeparator.new()
	row.add_child(_median_gap)
	move_left_button = _action(row, Glyph.LEFT, "Move left", func(): move_requested.emit(-1))
	move_right_button = _action(row, Glyph.RIGHT, "Move right", func(): move_requested.emit(1))
	remove_button = _action(row, Glyph.REMOVE, "Remove", func(): remove_requested.emit())
	# The popup has the keyboard while open: Backspace or Delete removes the part, except while
	# the median width is being typed.
	window_input.connect(func(event: InputEvent):
		var key := event as InputEventKey
		if key == null or not key.pressed or key.echo:
			return
		if key.keycode != KEY_BACKSPACE and key.keycode != KEY_DELETE:
			return
		var owner := gui_get_focus_owner()
		if owner is LineEdit or owner is SpinBox:
			return
		set_input_as_handled()
		remove_requested.emit())

func _action(row: HBoxContainer, glyph: Glyph, tip: String, on_pressed: Callable) -> Button:
	var button := GlyphButton.new()
	button.glyph = glyph
	button.tooltip_text = tip
	button.focus_mode = Control.FOCUS_NONE
	button.custom_minimum_size = Vector2.ONE * UIStyle.scaled_px(ICON_SIDE)
	button.pressed.connect(on_pressed)
	row.add_child(button)
	return button

## Opens the popup centred under `anchor` (a screen rectangle) for `part`; `can_left` / `can_right` say
## whether a neighbour it may swap with is there. `median_styles` is Rust's `get_median_styles`.
func show_for(part: Dictionary, can_left: bool, can_right: bool, median_styles: Dictionary, anchor: Rect2) -> void:
	show_part(part, can_left, can_right, median_styles)
	var at := Vector2(anchor.get_center().x - size.x * 0.5, anchor.end.y + 4.0)
	popup(Rect2i(Vector2i(at), Vector2i.ZERO))

## Shows `part` in the open popup (after a change) without moving it.
func show_part(part: Dictionary, can_left: bool, can_right: bool, median_styles: Dictionary) -> void:
	var median := String(part.get("kind", "")) == "median"
	median_controls.visible = median
	_median_gap.visible = median
	if median:
		median_controls.median_styles = median_styles
		median_controls.set_median(int(part.get("style", 0)), float(part.get("width_m", 0.0)))
	move_left_button.disabled = not can_left
	move_right_button.disabled = not can_right
	# A median's row is wider than a lane's; fit the part shown.
	reset_size()
