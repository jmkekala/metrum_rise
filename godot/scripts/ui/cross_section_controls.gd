# SPDX-License-Identifier: GPL-2.0-only

## A road cross-section's controls (`ROAD-80`): forward and backward lane steppers, 0..4 each.
## One view for every host: the road tool's options panel (the next stroke's lanes) and the
## Road Properties window (an existing road's lanes, applied after Rust's verdict). It holds no
## rule: Rust validates every request; the view shows the counts and reports changes through
## `changed`. Medians join it in round R3b.
extends VBoxContainer

## The player stepped a count: the counts the controls now show.
signal changed(fwd_lanes: int, bkw_lanes: int)

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const MAX_LANES := 4
const LABEL_WIDTH := 92.0

var fwd_lanes := 1
var bkw_lanes := 1

var _counts: Array[Label] = []
var _buttons: Array[Button] = []

func _init(font_size: int = 14) -> void:
	add_theme_constant_override("separation", 4)
	for direction in 2:
		var row := HBoxContainer.new()
		var label := Label.new()
		label.text = ["Lanes forward", "Lanes backward"][direction]
		label.custom_minimum_size.x = LABEL_WIDTH
		UIStyle.set_font_size(label, font_size)
		row.add_child(label)
		for step in [-1, 1]:
			var button := Button.new()
			button.text = "-" if step < 0 else "+"
			button.focus_mode = Control.FOCUS_NONE
			button.custom_minimum_size.x = 28.0
			UIStyle.set_font_size(button, font_size)
			button.pressed.connect(_on_step.bind(direction, step))
			_buttons.append(button)
			row.add_child(button)
			if step < 0:
				var count := Label.new()
				count.custom_minimum_size.x = 24.0
				count.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
				UIStyle.set_font_size(count, font_size)
				_counts.append(count)
				row.add_child(count)
		add_child(row)
	_show()

## Shows `fwd` and `bkw` without emitting `changed`.
func set_lanes(fwd: int, bkw: int) -> void:
	fwd_lanes = clampi(fwd, 0, MAX_LANES)
	bkw_lanes = clampi(bkw, 0, MAX_LANES)
	_show()

## Enables or disables every stepper, as for a road whose lanes cannot change.
func set_editable(editable: bool) -> void:
	for button in _buttons:
		button.disabled = not editable

func _on_step(direction: int, step: int) -> void:
	if direction == 0:
		fwd_lanes = clampi(fwd_lanes + step, 0, MAX_LANES)
	else:
		bkw_lanes = clampi(bkw_lanes + step, 0, MAX_LANES)
	_show()
	changed.emit(fwd_lanes, bkw_lanes)

func _show() -> void:
	_counts[0].text = str(fwd_lanes)
	_counts[1].text = str(bkw_lanes)
