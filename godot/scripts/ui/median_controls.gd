# SPDX-License-Identifier: GPL-2.0-only

## A median's controls (`ROAD-80`, `ROAD-101`): a style row (None, Painted, Grass, Paving) and a
## width stepper in 0.5 m steps within the style's range. Kept from the removed drawing-panel
## cross-section controls for the road builder's median part (RB2). It holds no rule: Rust
## validates every request (a width outside its style's range, a median a layout cannot carry);
## the view shows the values and reports changes through `changed`.
extends VBoxContainer

## The player changed a control: the median the controls now show (style 0 is none, 1 painted,
## 2 kerbed grass, 3 kerbed paving; width in metres, kerbs included).
signal changed(median_style: int, median_width: float)

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const LABEL_WIDTH := 92.0
## Median style names by Rust's style code.
const MEDIAN_STYLE_NAMES: Array[String] = ["None", "Painted", "Grass", "Paving"]
const MEDIAN_WIDTH_STEP := 0.5

var median_style := 0
var median_width := 0.0
## Rust's median widths per style code, as `get_median_styles` returns them (`default_m`,
## `min_m`, `max_m`): the host sets it once from the simulation node, before the player picks a
## style or steps the width. Only for picking a style's first width and stepping within range.
var median_styles: Dictionary = {}

var _style_buttons: Array[Button] = []
var _width_buttons: Array[Button] = []
var _width_label: Label
var _editable := true

func _init(font_size: int = 14) -> void:
	add_theme_constant_override("separation", 4)
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
			_width_label = Label.new()
			_width_label.custom_minimum_size.x = 52.0
			_width_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
			UIStyle.set_font_size(_width_label, font_size)
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

## Shows the median `style` of `width` metres without emitting `changed`.
func set_median(style: int, width: float) -> void:
	median_style = clampi(style, 0, MEDIAN_STYLE_NAMES.size() - 1)
	median_width = width if median_style != 0 else 0.0
	_show()

## Enables or disables every control.
func set_editable(editable: bool) -> void:
	_editable = editable
	_show()

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
	changed.emit(median_style, median_width)

func _show() -> void:
	for style in _style_buttons.size():
		_style_buttons[style].set_pressed_no_signal(style == median_style)
		_style_buttons[style].disabled = not _editable
	_width_label.text = "%.1f m" % median_width if median_style != 0 else "-"
	for button in _width_buttons:
		button.disabled = not _editable or median_style == 0
