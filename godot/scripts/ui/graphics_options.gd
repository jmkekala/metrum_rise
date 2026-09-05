# SPDX-License-Identifier: GPL-2.0-only

## Graphics options panel.
##
## Owns pending display preference edits and commits them through the shared
## Options footer.
extends VBoxContainer

const GameSettings = preload("res://scripts/core/game_settings.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

signal dirty_changed(has_pending_changes: bool)

var _initial_fullscreen := GameSettings.DEFAULT_FULLSCREEN
var _pending_fullscreen := GameSettings.DEFAULT_FULLSCREEN
var _syncing := false
var _fullscreen_check: CheckBox

func _ready() -> void:
	_build_ui()
	refresh()

func _build_ui() -> void:
	size_flags_horizontal = Control.SIZE_EXPAND_FILL
	size_flags_vertical = Control.SIZE_EXPAND_FILL
	add_theme_constant_override("separation", 14)

	var title := Label.new()
	title.text = "Graphics"
	UIStyle.set_font_size(title, 18)
	title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	add_child(title)

	_fullscreen_check = CheckBox.new()
	_fullscreen_check.text = "Fullscreen"
	UIStyle.set_font_size(_fullscreen_check, 14)
	_fullscreen_check.toggled.connect(_on_fullscreen_toggled)
	add_child(_fullscreen_check)

func refresh() -> void:
	_initial_fullscreen = GameSettings.get_fullscreen_enabled()
	_pending_fullscreen = _initial_fullscreen
	_sync_controls()
	_emit_dirty_state()

func has_pending_changes() -> bool:
	return _pending_fullscreen != _initial_fullscreen

func apply_changes() -> Error:
	var err := GameSettings.save_fullscreen_enabled(_pending_fullscreen)
	if err != OK:
		return err
	_initial_fullscreen = _pending_fullscreen
	GameSettings.apply_fullscreen_enabled(_pending_fullscreen)
	_emit_dirty_state()
	return OK

func reset_defaults() -> void:
	_pending_fullscreen = GameSettings.DEFAULT_FULLSCREEN
	_sync_controls()
	_emit_dirty_state()

func _on_fullscreen_toggled(enabled: bool) -> void:
	if _syncing:
		return
	_pending_fullscreen = enabled
	_emit_dirty_state()

func _sync_controls() -> void:
	_syncing = true
	if _fullscreen_check:
		_fullscreen_check.button_pressed = _pending_fullscreen
	_syncing = false

func _emit_dirty_state() -> void:
	emit_signal("dirty_changed", has_pending_changes())
