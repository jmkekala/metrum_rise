# SPDX-License-Identifier: GPL-2.0-only

## City status in the middle of the menu bar: clock, speed, treasury, agents and demand. It moves
## into the bar's own layer once the menu exists, so it is part of the bar, not an island below.
extends Control

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const TopMenu = preload("res://scripts/ui/top_menu.gd")
const BUTTON_SIDE := 26.0
var clock_label: Label
var city_status_panel: Control
var demand_meter: Control
var _notch: PanelContainer
var _pause_button: Button
var _speed_label: Label

func build(input_manager: Node) -> void:
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	set_anchors_and_offsets_preset(Control.PRESET_CENTER_TOP)
	offset_top = TopMenu.BAR_HEIGHT
	_notch = PanelContainer.new()
	_notch.add_theme_stylebox_override("panel", StyleBoxEmpty.new())
	_notch.add_to_group("debug_console_keep_clear")
	# Centred on this zero-width anchor whatever its content width.
	_notch.resized.connect(func(): _notch.position.x = -_notch.size.x * 0.5)
	add_child(_notch)
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 10)
	_notch.add_child(row)

	clock_label = _label("Day 1 00:00", 12, UIStyle.TEXT_PRIMARY)
	row.add_child(clock_label)
	var speed := HBoxContainer.new()
	speed.add_theme_constant_override("separation", 2)
	row.add_child(speed)
	_pause_button = HudIcons.icon_button("pause", "Pause", "", BUTTON_SIDE)
	HudIcons.set_shortcut(_pause_button, "Space")
	_pause_button.pressed.connect(input_manager._toggle_pause)
	speed.add_child(_pause_button)
	var slower := HudIcons.icon_button("slower", "Slower", "", BUTTON_SIDE)
	HudIcons.set_shortcut(slower, ",")
	slower.pressed.connect(func(): input_manager.step_simulation_speed(-1))
	speed.add_child(slower)
	_speed_label = _label("Paused", 12, UIStyle.TEXT_PRIMARY)
	_speed_label.custom_minimum_size.x = UIStyle.scaled_px(48)
	_speed_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	speed.add_child(_speed_label)
	var faster := HudIcons.icon_button("fast", "Faster", "", BUTTON_SIDE)
	HudIcons.set_shortcut(faster, ".")
	faster.pressed.connect(func(): input_manager.step_simulation_speed(1))
	speed.add_child(faster)

	row.add_child(_divider())
	city_status_panel = preload("res://scripts/ui/city_status_panel.gd").new()
	row.add_child(city_status_panel)
	row.add_child(_divider())
	demand_meter = preload("res://scripts/ui/demand_meter.gd").new()
	demand_meter.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	row.add_child(demand_meter)
	_bind_menu.call_deferred()

## Shows the running speed ("4x") or "Paused", and swaps the pause/resume icon.
func mark_speed(speed: float) -> void:
	var paused := speed <= 0.001
	_speed_label.text = "Paused" if paused else speed_text(speed)
	_speed_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM if paused else UIStyle.TEXT_PRIMARY)
	HudIcons.set_icon(_pause_button, "play" if paused else "pause")
	_pause_button.text = "Resume" if paused else "Pause"

## "4x" for whole speeds, "0.5x" otherwise.
static func speed_text(speed: float) -> String:
	if is_equal_approx(speed, roundf(speed)):
		return "%dx" % int(roundf(speed))
	return "%.1fx" % speed

func _label(text: String, font_size: int, color: Color) -> Label:
	var label := Label.new()
	label.text = text
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	UIStyle.set_font_size(label, font_size)
	label.add_theme_color_override("font_color", color)
	return label

func _divider() -> Control:
	var line := ColorRect.new()
	line.color = Color(1, 1, 1, 0.12)
	line.custom_minimum_size = Vector2(1, 16)
	line.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	line.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return line

# MainScene attaches TopMenu after child _ready calls. The bar is a higher CanvasLayer, so the
# status row joins that layer (drawn above the bar's background) and takes the bar's height.
func _bind_menu(attempts_left: int = 30) -> void:
	for control in get_tree().get_nodes_in_group("debug_console_keep_clear"):
		if control.get_parent().get_parent() is TopMenu:
			reparent(control.get_parent(), false)
			set_anchors_and_offsets_preset(Control.PRESET_CENTER_TOP)
			var follow := func():
				_notch.custom_minimum_size.y = control.size.y
				_notch.position.y = control.position.y
			control.resized.connect(follow)
			follow.call()
			return
	# The menu may attach a few frames later; give up quietly in scenes without one.
	if attempts_left > 0:
		get_tree().process_frame.connect(_bind_menu.bind(attempts_left - 1), CONNECT_ONE_SHOT)
