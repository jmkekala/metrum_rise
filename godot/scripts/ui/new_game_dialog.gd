# SPDX-License-Identifier: GPL-2.0-only

## New-game dialog for choosing the starting forest of a selected world.
##
## The defaults and the accepted density range come from the Rust `VegetationOptions` class,
## which is their only source; drifting copies here would outlive the renderer limit that sets
## the ceiling. This script renders those numbers and reports the player's choice. Rust
## sanitises whatever it receives, so nothing here guarantees a valid parameter.
extends Window

const UIStyle = preload("res://scripts/ui/ui_style.gd")

## Emitted with the chosen vegetation parameters when the player confirms the dialog.
signal start_requested(world_path: String, vegetation: Dictionary)

# Seeds are a 32-bit generator input; the spin box offers the whole range.
const SEED_MAX := 4294967295.0

var _world_path := ""
var _options := VegetationOptions.new()
var _defaults: Dictionary = {}
var _bounds: Dictionary = {}
var _enabled_check: CheckBox
var _seed_spin: SpinBox
var _coverage_slider: HSlider
var _coverage_value: Label
var _density_slider: HSlider
var _density_value: Label
var _world_label: Label
var _randomize_button: Button

func _ready() -> void:
	_defaults = _options.get_default_config()
	_bounds = _options.get_density_bounds()
	_build_ui()
	reset_to_defaults()

func _build_ui() -> void:
	title = "New Game"
	unresizable = false
	exclusive = true
	visible = false
	close_requested.connect(_on_close_requested)
	# No viewport argument: the helper resolves the parent's, which is the surface this window
	# opens on. Passing this window would clamp its size against its own current size.
	UIStyle.set_window_base_size(self, Vector2i(560, 320), Vector2i(480, 280))

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

	_enabled_check = CheckBox.new()
	_enabled_check.text = "Generate a forest"
	UIStyle.set_font_size(_enabled_check, 14)
	_enabled_check.toggled.connect(_on_enabled_toggled)
	content.add_child(_enabled_check)

	var seed_row := _make_row(content, "Seed")
	_seed_spin = SpinBox.new()
	_seed_spin.min_value = 0.0
	_seed_spin.max_value = SEED_MAX
	_seed_spin.step = 1.0
	_seed_spin.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(_seed_spin, 14)
	seed_row.add_child(_seed_spin)

	_randomize_button = Button.new()
	_randomize_button.text = "Randomize"
	UIStyle.set_font_size(_randomize_button, 13)
	_randomize_button.pressed.connect(_on_randomize_pressed)
	seed_row.add_child(_randomize_button)

	var coverage_row := _make_row(content, "Coverage")
	_coverage_slider = HSlider.new()
	_coverage_slider.min_value = 0.0
	_coverage_slider.max_value = 100.0
	# Continuous, because a stepped slider cannot represent the shipped defaults exactly and an
	# untouched dialog has to start the world the defaults were measured on. The readouts round,
	# and the generator quantises the grid these values choose, so the extra precision is inert.
	_coverage_slider.step = 0.0
	_coverage_slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_coverage_slider.value_changed.connect(_on_coverage_changed)
	coverage_row.add_child(_coverage_slider)
	_coverage_value = _make_value_label(coverage_row)

	var density_row := _make_row(content, "Density")
	_density_slider = HSlider.new()
	_density_slider.min_value = float(_bounds.get("min_canopy_stems_per_ha", 0.0))
	_density_slider.max_value = float(_bounds.get("max_canopy_stems_per_ha", 0.0))
	_density_slider.step = 0.0
	_density_slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_density_slider.value_changed.connect(_on_density_changed)
	density_row.add_child(_density_slider)
	_density_value = _make_value_label(density_row)

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
	label.custom_minimum_size.x = UIStyle.scaled_px(96.0)
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(label, 14)
	label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	row.add_child(label)
	return row

func _make_value_label(row: Control) -> Label:
	var value := Label.new()
	value.custom_minimum_size.x = UIStyle.scaled_px(88.0)
	value.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	value.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(value, 14)
	value.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	row.add_child(value)
	return value

## Opens the dialog for one world definition, resetting every parameter to its default.
func popup_for_world(world_path: String) -> void:
	_world_path = world_path
	reset_to_defaults()
	if _world_label:
		_world_label.text = "World: " + world_path.get_file()
	popup_centered(size)

## Restores every control to the parameters a new game starts with by default.
func reset_to_defaults() -> void:
	if _enabled_check == null:
		return
	_enabled_check.set_pressed_no_signal(bool(_defaults.get("enabled", true)))
	_seed_spin.set_value_no_signal(float(_defaults.get("seed", 0)))
	_coverage_slider.set_value_no_signal(float(_defaults.get("coverage", 0.0)) * 100.0)
	_density_slider.set_value_no_signal(float(_defaults.get("canopy_stems_per_ha", 0.0)))
	_refresh_readouts()

## Returns the parameters the controls currently describe, keyed as the Rust config fields are.
func selected_vegetation() -> Dictionary:
	return {
		"enabled": _enabled_check.button_pressed,
		"seed": int(_seed_spin.value),
		"coverage": _coverage_slider.value / 100.0,
		"canopy_stems_per_ha": _density_slider.value,
	}

func _on_enabled_toggled(_pressed: bool) -> void:
	_refresh_readouts()

func _on_coverage_changed(_value: float) -> void:
	_refresh_readouts()

func _on_density_changed(_value: float) -> void:
	_refresh_readouts()

func _on_randomize_pressed() -> void:
	_seed_spin.set_value_no_signal(float(randi()))
	_refresh_readouts()

func _on_close_requested() -> void:
	hide()

func _on_start_pressed() -> void:
	hide()
	emit_signal("start_requested", _world_path, selected_vegetation())

func _refresh_readouts() -> void:
	var generating: bool = _enabled_check.button_pressed
	_seed_spin.editable = generating
	_coverage_slider.editable = generating
	_density_slider.editable = generating
	_randomize_button.disabled = not generating
	var tint := Color(1.0, 1.0, 1.0, 1.0 if generating else 0.45)
	for control in [_seed_spin, _randomize_button, _coverage_slider, _density_slider]:
		control.modulate = tint
	_coverage_value.text = "%d%%" % int(roundf(_coverage_slider.value))
	_density_value.text = "%d / ha" % int(roundf(_density_slider.value))
