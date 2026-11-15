# SPDX-License-Identifier: GPL-2.0-only

## Compact road-option view (`ROAD-112`): the four drawing modes and Continuous, the snapping
## toggles, Straight grade, and the angle and length steps. The host owns tool state and connects
## the handlers; this view only builds the controls.
extends VBoxContainer

const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const SIDE := 38
## The road tool's angle lock steps (the session's); 0 is free.
const ANGLE_STEPS_DEG := [0, 15, 30, 45, 90]

var straight_btn: Button
var curve_btn: Button
var s_curve_btn: Button
var freeform_btn: Button
var continuous_btn: Button
var network_snap_btn: Button
var guide_lines_btn: Button
var snap_btn: Button
var straight_grade_btn: Button
var angle_step_option: OptionButton
var length_step_btn: Button
var length_step_spin: SpinBox

func _init() -> void:
	add_theme_constant_override("separation", 5)
	_heading(self, "Tool mode")
	var modes := HBoxContainer.new()
	modes.alignment = BoxContainer.ALIGNMENT_CENTER
	add_child(modes)
	straight_btn = _toggle(modes, "straight", "Straight", "Two clicks: start and end.")
	curve_btn = _toggle(modes, "curve", "Curve",
		"Three clicks: start, a control point, end. The road follows an exact arc tangent to both legs.")
	s_curve_btn = _toggle(modes, "spline", "S-curve",
		"Four clicks: start, a point it leaves towards, a point it arrives from, end. Two arcs meet tangent to each other.")
	freeform_btn = _toggle(modes, "freeform", "Freeform",
		"Click the road's points; click the last one again or press Enter to edit. Drag points and handles (Ctrl breaks a joint), then Enter or click elsewhere to build.")
	straight_btn.set_pressed_no_signal(true)
	var gap := Control.new()
	gap.custom_minimum_size.x = 8
	modes.add_child(gap)
	continuous_btn = _toggle(modes, "continuous", "Continuous",
		"Continue each new road in the direction the last one ended.")

	var options := HBoxContainer.new()
	options.add_theme_constant_override("separation", 14)
	options.alignment = BoxContainer.ALIGNMENT_CENTER
	add_child(options)
	var snapping := _column(options, "Snapping")
	network_snap_btn = _toggle(snapping, "snap_roads", "Snap to roads",
		"Snap to road junctions, ends and centrelines.")
	guide_lines_btn = _toggle(snapping, "guide_lines", "Guide lines",
		"Snap to the extensions of dead ends, square to nearby junctions and to the stroke's start.")
	snap_btn = _toggle(snapping, "snap_to_grid", "Snap to zoning grid",
		"Snap near parallel or perpendicular cell lines; draw other angles freely. Accounts for road widths. Existing road connections take priority.")
	var structure := _column(options, "Structure")
	straight_grade_btn = _toggle(structure, "straight_grade", "Straight grade",
		"Run the road on a straight grade between its two end heights instead of following the ground: where it passes over the ground it becomes a bridge, through a hill a tunnel. Drag an end's height handle (or PageUp/PageDown) to raise or lower it.")
	HudIcons.set_shortcut(straight_grade_btn, "T")

	var steps := _column(self, "Steps")
	steps.alignment = BoxContainer.ALIGNMENT_CENTER
	var angle_icon := TextureRect.new()
	angle_icon.texture = load("res://assets/ui/icons/angle_step.svg")
	angle_icon.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	angle_icon.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	angle_icon.custom_minimum_size = Vector2.ONE * 20
	angle_icon.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	angle_icon.modulate = Color(1, 1, 1, 0.6)
	angle_icon.tooltip_text = "Angle step"
	steps.add_child(angle_icon)
	angle_step_option = OptionButton.new()
	angle_step_option.focus_mode = Control.FOCUS_NONE
	angle_step_option.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	angle_step_option.tooltip_text = "Lock the stroke's direction to steps from the road it leaves (east on open ground). Shift draws free."
	for step in ANGLE_STEPS_DEG:
		angle_step_option.add_item("Free" if step == 0 else "%d°" % step)
	UIStyle.set_font_size(angle_step_option, 13)
	steps.add_child(angle_step_option)
	var steps_gap := Control.new()
	steps_gap.custom_minimum_size.x = 6
	steps.add_child(steps_gap)
	length_step_btn = _toggle(steps, "length_step", "Length step",
		"Round the stroke's length to steps. Shift draws free.")
	length_step_spin = SpinBox.new()
	length_step_spin.min_value = 1.0
	length_step_spin.max_value = 100.0
	length_step_spin.step = 0.5
	length_step_spin.suffix = "m"
	length_step_spin.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	length_step_spin.tooltip_text = "Length step"
	steps.add_child(length_step_spin)

func _toggle(parent: Control, icon: String, title: String, detail: String) -> Button:
	var button := HudIcons.icon_button(icon, title, detail, SIDE)
	button.toggle_mode = true
	parent.add_child(button)
	return button

# A titled row of controls.
func _column(parent: Control, title: String) -> HBoxContainer:
	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 3)
	parent.add_child(column)
	_heading(column, title)
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 4)
	column.add_child(row)
	return row

func _heading(parent: Control, text: String) -> void:
	var label := Label.new()
	label.text = text
	label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	UIStyle.set_font_size(label, 11)
	label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	parent.add_child(label)
