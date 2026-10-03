# SPDX-License-Identifier: GPL-2.0-only

## Frame rate readout in the top-right corner, toggled by Options -> Graphics -> Show FPS.
##
## An autoload so it survives scene changes and sits above every scene's UI. It shows frames
## per second and the mean frame time over the last half second; while hidden it does no work.
extends CanvasLayer

const GameSettings := preload("res://scripts/core/game_settings.gd")
const UIStyle := preload("res://scripts/ui/ui_style.gd")

const REFRESH_INTERVAL_S := 0.5
# Below the gameplay menu bar (TopMenu.BAR_HEIGHT), so the readout never covers a menu.
const TOP_OFFSET_PX := 34.0
const RIGHT_OFFSET_PX := 12.0

var _label: Label
var _elapsed_s := 0.0
var _frames := 0

func _ready() -> void:
	layer = 128
	_label = Label.new()
	_label.set_anchors_and_offsets_preset(Control.PRESET_TOP_RIGHT)
	_label.grow_horizontal = Control.GROW_DIRECTION_BEGIN
	_label.offset_top = TOP_OFFSET_PX
	_label.offset_right = -RIGHT_OFFSET_PX
	_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	_label.add_theme_color_override("font_outline_color", Color.BLACK)
	_label.add_theme_constant_override("outline_size", 6)
	UIStyle.set_font_size(_label, 14)
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
	_label.text = "%.0f FPS  %.1f ms" % [_frames / _elapsed_s, 1000.0 * _elapsed_s / _frames]
	_elapsed_s = 0.0
	_frames = 0
