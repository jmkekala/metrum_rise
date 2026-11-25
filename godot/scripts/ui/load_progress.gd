# SPDX-License-Identifier: GPL-3.0-or-later

## The load's progress line (`TERRAIN-59`): while a player's load preloads the base terrain within
## the terrain memory target, one HUD line at the screen's centre says how many terrain and water
## patches are built of those the preload builds. A view only: the input manager's load flow
## shows it, feeds it the stream node's counts and frees it at the hand-over. A real loading
## screen is a later item.
extends CanvasLayer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

var _label: Label

func _ready() -> void:
	var plate := PanelContainer.new()
	plate.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var style := UIStyle.hud_shell_style()
	style.set_content_margin(SIDE_LEFT, UIStyle.HUD_SHELL_PAD_X)
	style.set_content_margin(SIDE_RIGHT, UIStyle.HUD_SHELL_PAD_X)
	style.set_content_margin(SIDE_TOP, UIStyle.HUD_SHELL_PAD_Y)
	style.set_content_margin(SIDE_BOTTOM, UIStyle.HUD_SHELL_PAD_Y)
	plate.add_theme_stylebox_override("panel", style)
	plate.set_anchors_and_offsets_preset(Control.PRESET_CENTER)
	plate.grow_horizontal = Control.GROW_DIRECTION_BOTH
	plate.grow_vertical = Control.GROW_DIRECTION_BOTH
	_label = Label.new()
	UIStyle.set_font_size(_label, UIStyle.HUD_TEXT_SIZE)
	_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	plate.add_child(_label)
	add_child(plate)

## Shows `built` of `total` patches (before the first frame of the load knows them, no count).
func show_counts(built: int, total: int) -> void:
	_label.text = ("Loading terrain: %d of %d patches" % [built, maxi(total, built)] if total > 0
		else "Loading terrain")

## The line as shown.
func text() -> String:
	return _label.text
