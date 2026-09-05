# SPDX-License-Identifier: GPL-2.0-only

## Accessibility options panel.
##
## Owns pending accessibility preference edits and applies runtime-safe settings
## immediately when the shared Options footer commits them.
extends VBoxContainer

const GameSettings = preload("res://scripts/core/game_settings.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

signal dirty_changed(has_pending_changes: bool)

var _initial_ui_scale := GameSettings.DEFAULT_UI_SCALE
var _pending_ui_scale := GameSettings.DEFAULT_UI_SCALE
var _syncing := false
var _scale_slider: HSlider
var _scale_value_label: Label

func _ready() -> void:
	_build_ui()
	refresh()

func _build_ui() -> void:
	size_flags_horizontal = Control.SIZE_EXPAND_FILL
	size_flags_vertical = Control.SIZE_EXPAND_FILL
	add_theme_constant_override("separation", 14)

	var title := Label.new()
	title.text = "Accessibility"
	UIStyle.set_font_size(title, 18)
	title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	add_child(title)

	var row := HBoxContainer.new()
	row.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_theme_constant_override("separation", 12)
	add_child(row)

	var label := Label.new()
	label.text = "UI Scale"
	label.custom_minimum_size.x = UIStyle.scaled_px(120.0)
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(label, 14)
	label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	row.add_child(label)

	_scale_slider = HSlider.new()
	_scale_slider.min_value = GameSettings.MIN_UI_SCALE
	_scale_slider.max_value = GameSettings.MAX_UI_SCALE
	_scale_slider.step = GameSettings.UI_SCALE_STEP
	_scale_slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_scale_slider.value_changed.connect(_on_scale_slider_changed)
	row.add_child(_scale_slider)

	_scale_value_label = Label.new()
	_scale_value_label.custom_minimum_size.x = UIStyle.scaled_px(72.0)
	_scale_value_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	_scale_value_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(_scale_value_label, 14)
	_scale_value_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	row.add_child(_scale_value_label)

func refresh() -> void:
	_initial_ui_scale = GameSettings.get_ui_scale()
	_pending_ui_scale = _initial_ui_scale
	_sync_slider()
	_emit_dirty_state()

func has_pending_changes() -> bool:
	return not is_equal_approx(_pending_ui_scale, _initial_ui_scale)

func apply_changes() -> Error:
	var err := GameSettings.save_ui_scale(_pending_ui_scale)
	if err != OK:
		return err
	_initial_ui_scale = _pending_ui_scale
	UIStyle.refresh_scaled_font_sizes(get_tree().root)
	_emit_dirty_state()
	return OK

func reset_defaults() -> void:
	_pending_ui_scale = GameSettings.DEFAULT_UI_SCALE
	_sync_slider()
	_emit_dirty_state()

func _on_scale_slider_changed(value: float) -> void:
	if _syncing:
		return
	_pending_ui_scale = GameSettings.normalized_ui_scale(value)
	_update_value_label()
	_emit_dirty_state()

func _sync_slider() -> void:
	_syncing = true
	if _scale_slider:
		_scale_slider.value = _pending_ui_scale
	_update_value_label()
	_syncing = false

func _update_value_label() -> void:
	if not _scale_value_label:
		return
	_scale_value_label.text = "%d%%" % int(roundf(_pending_ui_scale * 100.0))

func _emit_dirty_state() -> void:
	emit_signal("dirty_changed", has_pending_changes())
