# SPDX-License-Identifier: GPL-2.0-only

## Persistent player-facing settings backed by Godot ConfigFile.
##
## This owns general runtime/UI preferences. Boot-critical mod activation stays
## in ModPackConfig because content-pack loading has a separate lifecycle.
extends RefCounted

const CFG_PATH := "user://settings.cfg"

const SECTION_OPTIONS_WINDOW := "options_window"
const SECTION_GRAPHICS := "graphics"
const SECTION_ACCESSIBILITY := "accessibility"
const SECTION_LAYOUT_PREFIX := "layout/"

const KEY_ACTIVE_CATEGORY := "active_category"
const KEY_WINDOW_WIDTH := "window_width"
const KEY_WINDOW_HEIGHT := "window_height"
const KEY_WINDOW_X := "window_x"
const KEY_WINDOW_Y := "window_y"
const KEY_FULLSCREEN := "fullscreen"
const KEY_UI_SCALE := "ui_scale"
const KEY_HAS_POSITION := "has_position"

const DEFAULT_OPTIONS_CATEGORY := "mods"
const DEFAULT_OPTIONS_WINDOW_WIDTH := 820
const DEFAULT_OPTIONS_WINDOW_HEIGHT := 540
const DEFAULT_FULLSCREEN := false
const DEFAULT_UI_SCALE := 1.0
const MIN_UI_SCALE := 0.8
const MAX_UI_SCALE := 1.5
const UI_SCALE_STEP := 0.05

static func load_config() -> ConfigFile:
	var cfg := ConfigFile.new()
	if not FileAccess.file_exists(CFG_PATH):
		_write_defaults(cfg)
		return cfg
	var err := cfg.load(CFG_PATH)
	if err != OK:
		push_warning("Could not read settings config '%s' (error %d)." % [CFG_PATH, err])
		_write_defaults(cfg)
	return cfg

static func save_config(cfg: ConfigFile) -> Error:
	var err := cfg.save(CFG_PATH)
	if err != OK:
		push_warning("Could not save settings config '%s' (error %d)." % [CFG_PATH, err])
	return err

static func seed_default_config_if_missing() -> void:
	if FileAccess.file_exists(CFG_PATH):
		return
	var cfg := ConfigFile.new()
	_write_defaults(cfg)
	save_config(cfg)

static func get_value(section: String, key: String, default_value: Variant) -> Variant:
	var cfg := load_config()
	return cfg.get_value(section, key, default_value)

static func set_value(section: String, key: String, value: Variant) -> Error:
	var cfg := load_config()
	cfg.set_value(section, key, value)
	return save_config(cfg)

static func get_fullscreen_enabled() -> bool:
	return bool(get_value(
		SECTION_GRAPHICS,
		KEY_FULLSCREEN,
		DEFAULT_FULLSCREEN
	))

static func save_fullscreen_enabled(enabled: bool) -> Error:
	return set_value(SECTION_GRAPHICS, KEY_FULLSCREEN, enabled)

static func apply_display_settings() -> void:
	apply_fullscreen_enabled(get_fullscreen_enabled())

static func apply_fullscreen_enabled(enabled: bool) -> void:
	var target_mode := DisplayServer.WINDOW_MODE_FULLSCREEN if enabled else DisplayServer.WINDOW_MODE_WINDOWED
	if DisplayServer.window_get_mode() != target_mode:
		DisplayServer.window_set_mode(target_mode)

static func get_ui_scale() -> float:
	return normalized_ui_scale(float(get_value(
		SECTION_ACCESSIBILITY,
		KEY_UI_SCALE,
		DEFAULT_UI_SCALE
	)))

static func save_ui_scale(ui_scale: float) -> Error:
	return set_value(SECTION_ACCESSIBILITY, KEY_UI_SCALE, normalized_ui_scale(ui_scale))

static func normalized_ui_scale(ui_scale: float) -> float:
	var clamped := clampf(ui_scale, MIN_UI_SCALE, MAX_UI_SCALE)
	return snappedf(clamped, UI_SCALE_STEP)

static func load_options_window_state() -> Dictionary:
	var cfg := load_config()
	var state := {
		"active_category": str(cfg.get_value(
			SECTION_OPTIONS_WINDOW,
			KEY_ACTIVE_CATEGORY,
			DEFAULT_OPTIONS_CATEGORY
		)),
		"size": Vector2i(
			int(cfg.get_value(
				SECTION_OPTIONS_WINDOW,
				KEY_WINDOW_WIDTH,
				DEFAULT_OPTIONS_WINDOW_WIDTH
			)),
			int(cfg.get_value(
				SECTION_OPTIONS_WINDOW,
				KEY_WINDOW_HEIGHT,
				DEFAULT_OPTIONS_WINDOW_HEIGHT
			))
		),
		"has_position": (
			cfg.has_section_key(SECTION_OPTIONS_WINDOW, KEY_WINDOW_X)
			and cfg.has_section_key(SECTION_OPTIONS_WINDOW, KEY_WINDOW_Y)
		),
		"position": Vector2i(
			int(cfg.get_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_X, 0)),
			int(cfg.get_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_Y, 0))
		),
	}
	return state

static func save_options_window_state(
	active_category: String,
	window_size: Vector2i,
	window_position: Vector2i
) -> Error:
	var cfg := load_config()
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_ACTIVE_CATEGORY, active_category)
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_WIDTH, window_size.x)
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_HEIGHT, window_size.y)
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_X, window_position.x)
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_Y, window_position.y)
	return save_config(cfg)

static func load_window_layout(layout_id: String) -> Dictionary:
	var cfg := load_config()
	var section := _layout_section(layout_id)
	var has_size := (
		cfg.has_section_key(section, KEY_WINDOW_WIDTH)
		and cfg.has_section_key(section, KEY_WINDOW_HEIGHT)
	)
	var has_position := (
		bool(cfg.get_value(section, KEY_HAS_POSITION, false))
		and cfg.has_section_key(section, KEY_WINDOW_X)
		and cfg.has_section_key(section, KEY_WINDOW_Y)
	)
	return {
		"has_size": has_size,
		"size": Vector2i(
			int(cfg.get_value(section, KEY_WINDOW_WIDTH, 0)),
			int(cfg.get_value(section, KEY_WINDOW_HEIGHT, 0))
		),
		"has_position": has_position,
		"position": Vector2i(
			int(cfg.get_value(section, KEY_WINDOW_X, 0)),
			int(cfg.get_value(section, KEY_WINDOW_Y, 0))
		),
	}

static func save_window_layout(
	layout_id: String,
	window_size: Vector2i,
	window_position: Vector2i,
	persist_position: bool
) -> Error:
	var cfg := load_config()
	var section := _layout_section(layout_id)
	cfg.set_value(section, KEY_WINDOW_WIDTH, window_size.x)
	cfg.set_value(section, KEY_WINDOW_HEIGHT, window_size.y)
	cfg.set_value(section, KEY_HAS_POSITION, persist_position)
	if persist_position:
		cfg.set_value(section, KEY_WINDOW_X, window_position.x)
		cfg.set_value(section, KEY_WINDOW_Y, window_position.y)
	return save_config(cfg)

static func get_layout_int(layout_id: String, key: String, default_value: int) -> int:
	var cfg := load_config()
	return int(cfg.get_value(_layout_section(layout_id), key, default_value))

static func save_layout_values(layout_id: String, values: Dictionary) -> Error:
	var cfg := load_config()
	var section := _layout_section(layout_id)
	for key_variant in values.keys():
		cfg.set_value(section, str(key_variant), values[key_variant])
	return save_config(cfg)

static func _write_defaults(cfg: ConfigFile) -> void:
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_ACTIVE_CATEGORY, DEFAULT_OPTIONS_CATEGORY)
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_WIDTH, DEFAULT_OPTIONS_WINDOW_WIDTH)
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_HEIGHT, DEFAULT_OPTIONS_WINDOW_HEIGHT)
	cfg.set_value(SECTION_GRAPHICS, KEY_FULLSCREEN, DEFAULT_FULLSCREEN)
	cfg.set_value(SECTION_ACCESSIBILITY, KEY_UI_SCALE, DEFAULT_UI_SCALE)

static func _layout_section(layout_id: String) -> String:
	return SECTION_LAYOUT_PREFIX + layout_id
