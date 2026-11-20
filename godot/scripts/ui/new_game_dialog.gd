# SPDX-License-Identifier: GPL-3.0-or-later

## New-game dialog for choosing the starting forest of a selected world.
##
## The defaults and presets come from the Rust `VegetationOptions` class,
## which is their only source; drifting copies here would outlive the renderer limit that sets
## the ceiling. This script renders those numbers and reports the player's choice. Rust
## sanitises whatever it receives, so nothing here guarantees a valid parameter. A line points to
## the pack screen when Rust finds no zone a building in the enabled packs (`TOOLS-32`).
extends Window

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const ModPackConfig = preload("res://scripts/core/mod_pack_config.gd")
const MODS_DIR := "user://mods/"
const BASE_SIZE := Vector2i(480, 150)
const BASE_MIN_SIZE := Vector2i(400, 140)
## The no-building-pack line's height with its row gap, unscaled.
const PACK_NOTICE_HEIGHT := 32

## Emitted with the chosen vegetation parameters when the player confirms the dialog.
signal start_requested(world_path: String, vegetation: Dictionary)

# Preset keys of VegetationOptions.get_presets(), in menu order.
const FOREST_KEYS: Array[String] = ["less", "more"]

var _world_path := ""
var _options := VegetationOptions.new()
var _defaults: Dictionary = {}
var _presets: Dictionary = {}
var _forest_buttons: Array[Button] = []
var _world_label: Label
# Shown when no zoning profile has a building in the enabled packs; starting stays allowed.
var _pack_notice: Label

func _ready() -> void:
	_defaults = _options.get_default_config()
	_presets = _options.get_presets()
	_build_ui()
	reset_to_defaults()

func _build_ui() -> void:
	title = "New Game"
	unresizable = false
	exclusive = true
	visible = false
	close_requested.connect(_on_close_requested)
	window_input.connect(func(event: InputEvent):
		if UIStyle.is_mouse_back(event):
			set_input_as_handled()
			_on_close_requested())
	# No viewport argument: the helper resolves the parent's, which is the surface this window
	# opens on. Passing this window would clamp its size against its own current size.
	UIStyle.set_window_base_size(self, BASE_SIZE, BASE_MIN_SIZE)

	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	add_child(body)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	for side in ["left", "right", "top", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, UIStyle.PAD_PANEL)
	body.add_child(margin)

	var content := VBoxContainer.new()
	content.add_theme_constant_override("separation", 12)
	margin.add_child(content)

	_world_label = Label.new()
	_world_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	UIStyle.set_font_size(_world_label, 15)
	_world_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	content.add_child(_world_label)

	_pack_notice = Label.new()
	_pack_notice.text = "No building pack is enabled. Add one in Options > Mods."
	_pack_notice.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	UIStyle.set_font_size(_pack_notice, 13)
	_pack_notice.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	_pack_notice.visible = false
	content.add_child(_pack_notice)

	_forest_buttons = _make_choices(_make_row(content, "Forest"), FOREST_KEYS)

	var spacer := Control.new()
	spacer.size_flags_vertical = Control.SIZE_EXPAND_FILL
	content.add_child(spacer)

	var buttons := HBoxContainer.new()
	buttons.alignment = BoxContainer.ALIGNMENT_END
	buttons.add_theme_constant_override("separation", 10)
	content.add_child(buttons)

	var start := Button.new()
	start.text = "Start Game"
	start.custom_minimum_size.x = UIStyle.scaled_px(140.0)
	UIStyle.set_font_size(start, 14)
	start.pressed.connect(_on_start_pressed)
	buttons.add_child(start)

func _make_row(parent: Control, label_text: String) -> HBoxContainer:
	var row := HBoxContainer.new()
	row.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_theme_constant_override("separation", 12)
	parent.add_child(row)

	var label := Label.new()
	label.text = label_text
	label.custom_minimum_size.x = UIStyle.scaled_px(120.0)
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(label, 14)
	label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	row.add_child(label)
	return row

# One toggle button per key, side by side, of which exactly one is pressed.
func _make_choices(row: Control, keys: Array[String]) -> Array[Button]:
	var group := ButtonGroup.new()
	var buttons: Array[Button] = []
	# The chosen button takes the accent so the selection reads at a glance.
	var chosen := UIStyle.panel_style(Color(0.30, 0.30, 0.45, 0.95), 4, Color(0.65, 0.65, 0.90), 1)
	for key in keys:
		var button := Button.new()
		button.text = key.capitalize()
		button.toggle_mode = true
		button.button_group = group
		button.focus_mode = Control.FOCUS_NONE
		button.add_theme_stylebox_override("pressed", chosen)
		button.add_theme_stylebox_override("hover_pressed", chosen)
		button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		UIStyle.set_font_size(button, 14)
		row.add_child(button)
		buttons.append(button)
	return buttons

## Opens the dialog for one world definition, resetting every parameter to its default.
func popup_for_world(world_path: String) -> void:
	_world_path = world_path
	reset_to_defaults()
	if _world_label:
		_world_label.text = "World: " + world_path.get_file()
	if _pack_notice:
		# Rust answers with the game's own spawn filter; the selection may have changed since.
		_pack_notice.visible = not SimulationNode.enabled_packs_stock_zones(
			ProjectSettings.globalize_path(MODS_DIR), PackedStringArray(ModPackConfig.load_enabled_pack_ids()))
		var extra := Vector2i(0, PACK_NOTICE_HEIGHT if _pack_notice.visible else 0)
		UIStyle.set_window_base_size(self, BASE_SIZE + extra, BASE_MIN_SIZE + extra)
	popup_centered(size)

## Restores every control to the parameters a new game starts with by default.
func reset_to_defaults() -> void:
	if _forest_buttons.is_empty():
		return
	_forest_buttons[FOREST_KEYS.find("less")].button_pressed = true

## Returns the parameters the controls currently describe, keyed as the Rust config fields are.
func selected_vegetation() -> Dictionary:
	var forest: Dictionary = _presets[FOREST_KEYS[_pressed_index(_forest_buttons)]]
	return {
		"enabled": true,
		"seed": int(_defaults["seed"]),
		"coverage": float(forest["coverage"]),
		"canopy_stems_per_ha": float(forest["canopy_stems_per_ha"]),
	}

func _pressed_index(buttons: Array[Button]) -> int:
	for index in buttons.size():
		if buttons[index].button_pressed:
			return index
	return 0

func _on_close_requested() -> void:
	hide()

func _on_start_pressed() -> void:
	hide()
	emit_signal("start_requested", _world_path, selected_vegetation())
