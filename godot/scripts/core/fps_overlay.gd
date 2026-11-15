# SPDX-License-Identifier: GPL-2.0-only

## Frame rate readout at the right end of the gameplay menu bar (top-right corner below the
## bar elsewhere), toggled by Options -> Graphics -> Show FPS.
##
## An autoload so it survives scene changes and sits above every scene's UI. It shows frames
## per second and the mean frame time over the last half second; while hidden it does no work.
extends CanvasLayer

const GameSettings := preload("res://scripts/core/game_settings.gd")
const UIStyle := preload("res://scripts/ui/ui_style.gd")
const TopMenu := preload("res://scripts/ui/top_menu.gd")

const REFRESH_INTERVAL_S := 0.5
# Below the gameplay menu bar (TopMenu.BAR_HEIGHT), so the readout never covers a menu.
const TOP_OFFSET_PX := 34.0
const RIGHT_OFFSET_PX := 12.0

var _label: Label
var _elapsed_s := 0.0
var _frames := 0
# The menu bar's panel once found; the label takes its height, inside the bar.
var _bar: Control

func _ready() -> void:
	layer = 128
	_label = Label.new()
	_label.set_anchors_and_offsets_preset(Control.PRESET_TOP_RIGHT)
	_label.grow_horizontal = Control.GROW_DIRECTION_BEGIN
	_label.offset_top = TOP_OFFSET_PX
	_label.offset_right = -RIGHT_OFFSET_PX
	_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	_label.add_theme_color_override("font_outline_color", Color.BLACK)
	_label.add_theme_constant_override("outline_size", 6)
	UIStyle.set_font_size(_label, 13)
	add_child(_label)
	set_enabled(GameSettings.get_show_fps())

## Shows or hides the readout. Options calls this when Show FPS is applied.
func set_enabled(enabled: bool) -> void:
	visible = enabled
	set_process(enabled)
	_elapsed_s = 0.0
	_frames = 0
	_label.text = ""

func _process(delta: float) -> void:
	_elapsed_s += delta
	_frames += 1
	if _elapsed_s < REFRESH_INTERVAL_S:
		return
	_follow_bar()
	_label.text = "%.0f FPS  %.1f ms" % [_frames / _elapsed_s, 1000.0 * _elapsed_s / _frames]
	_elapsed_s = 0.0
	_frames = 0

# Twice a second while shown: finds the bar after a scene change (TopMenu's shell carries the
# debug console's keep-clear group) and sits inside it; elsewhere, below the corner as before.
func _follow_bar() -> void:
	if not is_instance_valid(_bar):
		_bar = null
		for control in get_tree().get_nodes_in_group("debug_console_keep_clear"):
			# Editor bars keep their theme switch at the right end; only gameplay's is free.
			var menu := control.get_parent().get_parent()
			if menu is TopMenu and menu.scene_kind == TopMenu.SCENE_GAMEPLAY:
				_bar = control
				break
	var in_bar := _bar != null
	_label.offset_top = 0.0 if in_bar else TOP_OFFSET_PX
	_label.offset_bottom = _bar.size.y if in_bar else TOP_OFFSET_PX
	_label.offset_right = -RIGHT_OFFSET_PX - (6.0 if in_bar else 0.0)
	_label.add_theme_constant_override("outline_size", 0 if in_bar else 6)
	_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM if in_bar else UIStyle.TEXT_PRIMARY)
