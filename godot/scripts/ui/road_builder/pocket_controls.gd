# SPDX-License-Identifier: GPL-2.0-only

## Road-type turn pockets (`ROAD-119`): two independent role toggles and a length stepper.
## Rust supplies the Junction window's length grid and applies every placement rule.
extends HBoxContainer

signal changed

const UIStyle = preload("res://scripts/ui/ui_style.gd")

var left_button: Button
var right_button: Button
var shorter_button: Button
var longer_button: Button
var length_label: Label
var body_m := 60.0
var _limits: Dictionary = {}

func _init() -> void:
	# The arrows say what they are; the tooltips name them.
	add_theme_constant_override("separation", 4)
	left_button = _button("↰", true)
	left_button.tooltip_text = "Left turn pocket"
	right_button = _button("↱", true)
	right_button.tooltip_text = "Right turn pocket"
	for button in [left_button, right_button]:
		# As tall as the zoning and light toggles beside them.
		button.custom_minimum_size = Vector2.ONE * UIStyle.scaled_px(40.0)
		UIStyle.set_font_size(button, 18)
	left_button.toggled.connect(func(_on: bool): _emit())
	right_button.toggled.connect(func(_on: bool): _emit())
	var gap := Control.new()
	gap.custom_minimum_size.x = 4.0
	add_child(gap)
	shorter_button = _button("−", false)
	shorter_button.pressed.connect(func(): _step(-1))
	length_label = Label.new()
	length_label.custom_minimum_size.x = 46.0
	length_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	length_label.tooltip_text = "From the junction"
	UIStyle.set_font_size(length_label, 13)
	add_child(length_label)
	longer_button = _button("+", false)
	longer_button.pressed.connect(func(): _step(1))
	_show()

func _button(text: String, toggle: bool) -> Button:
	var button := Button.new()
	button.text = text
	button.toggle_mode = toggle
	button.focus_mode = Control.FOCUS_NONE
	UIStyle.set_font_size(button, 13)
	for state in ["normal", "hover", "pressed", "hover_pressed", "disabled", "focus"]:
		var style := StyleBoxFlat.new()
		var selected: bool = state == "pressed" or state == "hover_pressed"
		style.bg_color = Color(UIStyle.HUD_ACCENT, 0.15) if selected else Color(1, 1, 1, 0.04)
		style.border_color = Color(UIStyle.HUD_ACCENT, 0.65) if selected else Color(1, 1, 1, 0.12)
		style.set_border_width_all(1)
		style.set_corner_radius_all(6)
		style.set_content_margin_all(6)
		button.add_theme_stylebox_override(state, style)
	button.add_theme_color_override("font_pressed_color", UIStyle.HUD_ACCENT)
	button.add_theme_color_override("font_hover_pressed_color", UIStyle.HUD_ACCENT)
	add_child(button)
	return button

## Loads one type without emitting a change; missing settings mean neither role.
func set_value(pockets: Dictionary, limits: Dictionary) -> void:
	_limits = limits
	left_button.set_pressed_no_signal(bool(pockets.get("left", false)))
	right_button.set_pressed_no_signal(bool(pockets.get("right", false)))
	body_m = float(pockets.get("body_m", limits.get("default_m", 60.0)))
	_show()

## The configuration sent to Rust, including the remembered length with both roles off.
func value() -> Dictionary:
	return {"left": left_button.button_pressed, "right": right_button.button_pressed, "body_m": body_m}

## The cross-section stage's annotation; pockets are local to junctions, not full-length lanes.
func description() -> String:
	var roles := PackedStringArray()
	if left_button.button_pressed:
		roles.append("↰")
	if right_button.button_pressed:
		roles.append("↱")
	return "  ".join(roles) + "  %d m at junctions" % int(body_m) if not roles.is_empty() else ""

func _step(direction: int) -> void:
	body_m = clampf(body_m + direction * float(_limits.get("step_m", 10.0)),
		float(_limits.get("min_m", 20.0)), float(_limits.get("max_m", 300.0)))
	_emit()

func _emit() -> void:
	_show()
	changed.emit()

func _show() -> void:
	length_label.text = "%d m" % int(body_m)
	var enabled := left_button.button_pressed or right_button.button_pressed
	shorter_button.disabled = not enabled or body_m <= float(_limits.get("min_m", 20.0))
	longer_button.disabled = not enabled or body_m >= float(_limits.get("max_m", 300.0))
