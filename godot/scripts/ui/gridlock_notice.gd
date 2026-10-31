# SPDX-License-Identifier: GPL-2.0-only

## Player-facing gridlock notice (`TRAFFIC-14`): one HUD line at the top of the screen naming
## how many junctions the last gridlock check found locked with no way out. A view only:
## `GridlockDisplay` decides when it shows and what it says from the Rust report.
extends PanelContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

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
	offset_top = UIStyle.HUD_BOTTOM_MARGIN
	_label = Label.new()
	UIStyle.set_font_size(_label, UIStyle.HUD_TEXT_SIZE)
	_label.add_theme_color_override("font_color", UIStyle.TEXT_ALERT)
	_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	add_child(_label)
	visible = false

## Shows the notice for `junction_count` locked junctions, or hides it for 0.
func show_junctions(junction_count: int) -> void:
	visible = junction_count > 0
	if visible:
		_label.text = "Gridlock: traffic is locked at %d %s with no way out" % [
			junction_count, "junction" if junction_count == 1 else "junctions"]

## The text the notice shows while visible.
func text() -> String:
	return _label.text if visible else ""
