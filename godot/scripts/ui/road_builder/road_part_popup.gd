# SPDX-License-Identifier: GPL-2.0-only

## The road builder's part popup (`ROAD-101`), opened by clicking a part in the strip: the part's
## name, for a median its style and width (the reusable median controls, without "None"), and
## Move left / Move right / Remove for any part. A view: it reports the player's choice through
## its signals and the builder edits the layout.
extends PopupPanel

## The player changed the median's style or width.
signal median_changed(style: int, width_m: float)
## The player asked to move the part one place left (-1) or right (+1).
signal move_requested(step: int)
## The player asked to remove the part.
signal remove_requested

const MedianControls = preload("res://scripts/ui/median_controls.gd")
const RoadPartGlyph = preload("res://scripts/ui/road_builder/road_part_glyph.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

## The median controls (shown for a median part only).
var median_controls: MedianControls
var move_left_button: Button
var move_right_button: Button
var remove_button: Button

var _title: Label

func _init() -> void:
	add_theme_stylebox_override("panel", UIStyle.panel_style(UIStyle.BG_DARK, UIStyle.CORNER_WINDOW))
	var margin := MarginContainer.new()
	for side in ["left", "right", "top", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, UIStyle.PAD_INNER)
	add_child(margin)
	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 6)
	margin.add_child(column)
	_title = Label.new()
	UIStyle.set_font_size(_title, 14)
	column.add_child(_title)
	median_controls = MedianControls.new(13)
	median_controls.offer_none(false)
	median_controls.changed.connect(func(style: int, width: float): median_changed.emit(style, width))
	column.add_child(median_controls)
	var actions := HBoxContainer.new()
	column.add_child(actions)
	move_left_button = _action(actions, "Move left", func(): move_requested.emit(-1))
	move_right_button = _action(actions, "Move right", func(): move_requested.emit(1))
	remove_button = _action(actions, "Remove", func(): remove_requested.emit())

func _action(row: HBoxContainer, label: String, on_pressed: Callable) -> Button:
	var button := Button.new()
	button.text = label
	button.focus_mode = Control.FOCUS_NONE
	UIStyle.set_font_size(button, 13)
	button.pressed.connect(on_pressed)
	row.add_child(button)
	return button

## Opens the popup under `anchor` (a screen rectangle) for `part`; `can_left` / `can_right` say
## whether a neighbour it may swap with is there. `median_styles` is Rust's `get_median_styles`.
func show_for(part: Dictionary, can_left: bool, can_right: bool, median_styles: Dictionary, anchor: Rect2) -> void:
	show_part(part, can_left, can_right, median_styles)
	reset_size()
	popup(Rect2i(Vector2i(anchor.position + Vector2(0.0, anchor.size.y + 4.0)), Vector2i.ZERO))

## Shows `part` in the open popup (after a change) without moving it.
func show_part(part: Dictionary, can_left: bool, can_right: bool, median_styles: Dictionary) -> void:
	_title.text = RoadPartGlyph.part_name(part)
	var median := String(part.get("kind", "")) == "median"
	median_controls.visible = median
	if median:
		median_controls.median_styles = median_styles
		median_controls.set_median(int(part.get("style", 0)), float(part.get("width_m", 0.0)))
	move_left_button.disabled = not can_left
	move_right_button.disabled = not can_right
