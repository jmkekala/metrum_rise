# SPDX-License-Identifier: GPL-2.0-only

## Player-facing gridlock notice (`TRAFFIC-14`): one HUD line at the top of the screen naming
## how many junctions the last gridlock check found locked with no way out. The stranded-walker
## notice (`ROAD-74`) is the same line one row lower. A view only: `GridlockDisplay` decides when
## it shows and what it says from the Rust report.
extends PanelContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

## HUD rows below the top line this notice sits; a second notice stacks under the first.
var row := 0
var _label: Label

func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	var style := UIStyle.hud_shell_style()
	style.set_content_margin(SIDE_LEFT, UIStyle.HUD_SHELL_PAD_X)
	style.set_content_margin(SIDE_RIGHT, UIStyle.HUD_SHELL_PAD_X)
	style.set_content_margin(SIDE_TOP, UIStyle.HUD_SHELL_PAD_Y)
	style.set_content_margin(SIDE_BOTTOM, UIStyle.HUD_SHELL_PAD_Y)
	add_theme_stylebox_override("panel", style)
	set_anchors_and_offsets_preset(Control.PRESET_CENTER_TOP)
	grow_horizontal = Control.GROW_DIRECTION_BOTH
	# One HUD line plus its shell padding per row above it.
	offset_top = UIStyle.HUD_BOTTOM_MARGIN + row * (UIStyle.HUD_TEXT_SIZE + 2 * UIStyle.HUD_SHELL_PAD_Y + 8)
	_label = Label.new()
	UIStyle.set_font_size(_label, UIStyle.HUD_TEXT_SIZE)
	_label.add_theme_color_override("font_color", UIStyle.TEXT_ALERT)
	_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	add_child(_label)
	visible = false

## Shows `message`, or hides the notice for an empty one.
func show_text(message: String) -> void:
	visible = message != ""
	_label.text = message

## The text the notice shows while visible.
func text() -> String:
	return _label.text if visible else ""
