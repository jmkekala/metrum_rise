# SPDX-License-Identifier: GPL-2.0-only

## Gameplay preferences with pending edits managed by the shared Options footer.
extends VBoxContainer

const GameSettings = preload("res://scripts/core/game_settings.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

signal dirty_changed(has_pending_changes: bool)

var _initial_mode := GameSettings.DEFAULT_ROAD_PREVIEW_MODE
var _pending_mode := GameSettings.DEFAULT_ROAD_PREVIEW_MODE
var _mode: OptionButton

func _ready() -> void:
	add_theme_constant_override("separation", 14)
	var title := Label.new()
	title.text = "Gameplay"
	UIStyle.set_font_size(title, 18)
	add_child(title)
	var label := Label.new()
	label.text = "Road construction preview"
	add_child(label)
	_mode = OptionButton.new()
	_mode.add_item("Road only (faster)")
	_mode.add_item("Road and terrain")
	_mode.item_selected.connect(func(index: int):
		_pending_mode = index
		dirty_changed.emit(has_pending_changes())
	)
	add_child(_mode)
	var description := Label.new()
	description.text = "Road only previews roads and junctions; terrain updates when you place them. Road and terrain also previews the ground changes, but may respond more slowly."
	description.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	add_child(description)
	refresh()

func refresh() -> void:
	_initial_mode = GameSettings.get_road_preview_mode()
	_pending_mode = _initial_mode
	_mode.select(_pending_mode)
	dirty_changed.emit(false)

func has_pending_changes() -> bool:
	return _initial_mode != _pending_mode

func apply_changes() -> Error:
	var error := GameSettings.set_value(GameSettings.SECTION_GAMEPLAY, GameSettings.KEY_ROAD_PREVIEW_MODE, _pending_mode)
	if error != OK:
		return error
	_initial_mode = _pending_mode
	get_tree().call_group("road_preview_tools", "set_road_preview_mode", _pending_mode)
	dirty_changed.emit(false)
	return OK

func reset_defaults() -> void:
	_pending_mode = GameSettings.DEFAULT_ROAD_PREVIEW_MODE
	_mode.select(_pending_mode)
	dirty_changed.emit(has_pending_changes())
