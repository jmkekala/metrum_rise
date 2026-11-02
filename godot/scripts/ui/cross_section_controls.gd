# SPDX-License-Identifier: GPL-2.0-only

## A road cross-section's controls (`ROAD-80`): forward and backward lane steppers, 0..4 each,
## and the median: a style row (None, Painted, Grass, Paving) and a width stepper. One view for
## every host: the road tool's options panel (the next stroke's cross-section) and the Road
## Properties window (an existing road's, applied after Rust's verdict). It holds no rule: Rust
## validates every request (a median on a one-way road or a walkway, a width outside its
## style's range); the view shows the values and reports changes through `changed`.
extends VBoxContainer

## The player changed a control: the cross-section the controls now show (median style 0 is
## none, 1 painted, 2 kerbed grass, 3 kerbed paving; width in metres, kerbs included).
signal changed(fwd_lanes: int, bkw_lanes: int, median_style: int, median_width: float)

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const MAX_LANES := 4
const LABEL_WIDTH := 92.0
const MEDIAN_STYLE_NAMES: Array[String] = ["None", "Painted", "Grass", "Paving"]
const MEDIAN_WIDTH_STEP := 0.5

var fwd_lanes := 1
var bkw_lanes := 1
var median_style := 0
var median_width := 0.0
## Rust's median widths per style code, as `get_median_styles` returns them (`default_m`,
## `min_m`, `max_m`): the host sets it once from the simulation node, before the player picks a
## style or steps the width. Only for picking a style's first width and stepping within range.
var median_styles: Dictionary = {}

var _counts: Array[Label] = []
var _buttons: Array[Button] = []
var _style_buttons: Array[Button] = []
var _width_buttons: Array[Button] = []
var _width_label: Label
var _editable := true

func _init(font_size: int = 14) -> void:
	add_theme_constant_override("separation", 4)
	for direction in 2:
		var row := _row(["Lanes forward", "Lanes backward"][direction], font_size)
		for step in [-1, 1]:
			var button := _button("-" if step < 0 else "+", font_size, _on_step.bind(direction, step))
			_buttons.append(button)
			row.add_child(button)
			if step < 0:
				var count := _value_label(24.0, font_size)
				_counts.append(count)
				row.add_child(count)
	var style_row := _row("Median", font_size)
	var group := ButtonGroup.new()
	for style in MEDIAN_STYLE_NAMES.size():
		var button := _button(MEDIAN_STYLE_NAMES[style], font_size, _on_style.bind(style))
		button.toggle_mode = true
		button.button_group = group
		button.custom_minimum_size.x = 0.0
		_style_buttons.append(button)
		style_row.add_child(button)
	var width_row := _row("Median width", font_size)
	for step in [-1, 1]:
		var button := _button("-" if step < 0 else "+", font_size, _on_width_step.bind(step))
		_width_buttons.append(button)
		width_row.add_child(button)
		if step < 0:
			_width_label = _value_label(52.0, font_size)
			width_row.add_child(_width_label)
	_show()

func _row(text: String, font_size: int) -> HBoxContainer:
	var row := HBoxContainer.new()
	var label := Label.new()
	label.text = text
	label.custom_minimum_size.x = LABEL_WIDTH
	UIStyle.set_font_size(label, font_size)
	row.add_child(label)
	add_child(row)
	return row

func _button(text: String, font_size: int, on_pressed: Callable) -> Button:
	var button := Button.new()
	button.text = text
	button.focus_mode = Control.FOCUS_NONE
	button.custom_minimum_size.x = 28.0
	UIStyle.set_font_size(button, font_size)
	button.pressed.connect(on_pressed)
	return button

func _value_label(width: float, font_size: int) -> Label:
	var label := Label.new()
	label.custom_minimum_size.x = width
	label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(label, font_size)
	return label

## Shows `fwd` and `bkw` without emitting `changed`.
func set_lanes(fwd: int, bkw: int) -> void:
	fwd_lanes = clampi(fwd, 0, MAX_LANES)
	bkw_lanes = clampi(bkw, 0, MAX_LANES)
	_show()

## Shows the median `style` of `width` metres without emitting `changed`.
func set_median(style: int, width: float) -> void:
	median_style = clampi(style, 0, MEDIAN_STYLE_NAMES.size() - 1)
	median_width = width if median_style != 0 else 0.0
	_show()

## Enables or disables every control, as for a road whose cross-section cannot change.
func set_editable(editable: bool) -> void:
	_editable = editable
	_show()

func _on_step(direction: int, step: int) -> void:
	if direction == 0:
		fwd_lanes = clampi(fwd_lanes + step, 0, MAX_LANES)
	else:
		bkw_lanes = clampi(bkw_lanes + step, 0, MAX_LANES)
	_emit()

func _on_style(style: int) -> void:
	if style == median_style:
		return
	median_style = style
	median_width = float(median_styles[style].default_m) if style != 0 else 0.0
	_emit()

func _on_width_step(step: int) -> void:
	var widths: Dictionary = median_styles[median_style]
	median_width = clampf(median_width + step * MEDIAN_WIDTH_STEP, widths.min_m, widths.max_m)
	_emit()

func _emit() -> void:
	_show()
	changed.emit(fwd_lanes, bkw_lanes, median_style, median_width)

func _show() -> void:
	_counts[0].text = str(fwd_lanes)
	_counts[1].text = str(bkw_lanes)
	for style in _style_buttons.size():
		_style_buttons[style].set_pressed_no_signal(style == median_style)
	_width_label.text = "%.1f m" % median_width if median_style != 0 else "-"
	for button in _buttons + _style_buttons:
		button.disabled = not _editable
	for button in _width_buttons:
		button.disabled = not _editable or median_style == 0
