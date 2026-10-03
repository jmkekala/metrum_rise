# SPDX-License-Identifier: GPL-2.0-only

## Graphics options panel.
##
## Owns pending display and render preference edits and commits them through the shared
## Options footer. Pending values are keyed by their GameSettings graphics key.
extends VBoxContainer

const GameSettings = preload("res://scripts/core/game_settings.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

signal dirty_changed(has_pending_changes: bool)

const MAX_FPS_LABELS := ["Unlimited", "30", "60", "120", "144"]
const RENDER_SCALE_LABELS := ["Native", "77% (FSR 2)", "67% (FSR 2)", "50% (FSR 2)"]

var _initial := {}
var _pending := {}
var _syncing := false
var _fullscreen_check: CheckBox
var _vsync_check: CheckBox
var _show_fps_check: CheckBox
var _max_fps: OptionButton
var _render_scale: OptionButton
var _shadow_quality: OptionButton
var _view_distance: OptionButton
var _lod_quality: OptionButton

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

	_fullscreen_check = _add_check("Fullscreen", GameSettings.KEY_FULLSCREEN, "")
	_vsync_check = _add_check("V-Sync", GameSettings.KEY_VSYNC,
		"Waits for the display refresh before showing a frame, which prevents tearing.")
	_show_fps_check = _add_check("Show FPS", GameSettings.KEY_SHOW_FPS,
		"Shows frames per second and frame time in the top-right corner.")

	var grid := GridContainer.new()
	grid.columns = 2
	grid.add_theme_constant_override("h_separation", 16)
	grid.add_theme_constant_override("v_separation", 10)
	add_child(grid)
	_max_fps = _add_choice(grid, "Max FPS", GameSettings.KEY_MAX_FPS, MAX_FPS_LABELS,
		"Caps the frame rate. With V-Sync on, the display refresh rate also caps it.")
	_render_scale = _add_choice(grid, "Render scale", GameSettings.KEY_RENDER_SCALE, RENDER_SCALE_LABELS,
		"Renders the 3D view at a lower resolution and upscales it with FSR 2. The interface stays sharp.")
	_shadow_quality = _add_choice(grid, "Shadows", GameSettings.KEY_SHADOW_QUALITY, ["High", "Low"],
		"Low uses hard-edged shadows with two cascades instead of four.")
	_view_distance = _add_choice(grid, "View distance", GameSettings.KEY_VIEW_DISTANCE, ["Full", "Reduced"],
		"Reduced stops drawing the world beyond about 3 km from the camera, unless zoomed far out.")
	_lod_quality = _add_choice(grid, "Building detail", GameSettings.KEY_BUILDING_LOD_QUALITY,
		["Performance", "Balanced", "Quality"],
		"Controls when buildings switch to simpler meshes. Does not affect simulation or remove distant buildings.")

func _add_check(text: String, key: String, tooltip: String) -> CheckBox:
	var check := CheckBox.new()
	check.text = text
	check.tooltip_text = tooltip
	UIStyle.set_font_size(check, 14)
	check.toggled.connect(func(enabled: bool): _set_pending(key, enabled))
	add_child(check)
	return check

func _add_choice(grid: GridContainer, text: String, key: String, labels: Array, tooltip: String) -> OptionButton:
	var label := Label.new()
	label.text = text
	grid.add_child(label)
	var option := OptionButton.new()
	for item in labels:
		option.add_item(item)
	option.tooltip_text = tooltip
	option.item_selected.connect(func(index: int): _set_pending(key, _value_for_index(key, index)))
	grid.add_child(option)
	return option

func refresh() -> void:
	_initial = _persisted_values()
	_pending = _initial.duplicate()
	_sync_controls()
	_emit_dirty_state()

func has_pending_changes() -> bool:
	return _pending != _initial

func apply_changes() -> Error:
	var config := GameSettings.load_config()
	for key in _pending:
		config.set_value(GameSettings.SECTION_GRAPHICS, key, _pending[key])
	var err := GameSettings.save_config(config)
	if err != OK:
		return err
	_initial = _pending.duplicate()
	GameSettings.apply_display_settings()
	get_tree().call_group("building_lod_renderers", "set_quality", _pending[GameSettings.KEY_BUILDING_LOD_QUALITY])
	get_tree().call_group("graphics_settings", "apply_graphics_settings")
	var overlay := get_tree().root.get_node_or_null("FpsOverlay")
	if overlay:
		overlay.set_enabled(_pending[GameSettings.KEY_SHOW_FPS])
	_emit_dirty_state()
	return OK

func reset_defaults() -> void:
	_pending = {
		GameSettings.KEY_FULLSCREEN: GameSettings.DEFAULT_FULLSCREEN,
		GameSettings.KEY_VSYNC: GameSettings.DEFAULT_VSYNC,
		GameSettings.KEY_SHOW_FPS: GameSettings.DEFAULT_SHOW_FPS,
		GameSettings.KEY_MAX_FPS: GameSettings.DEFAULT_MAX_FPS,
		GameSettings.KEY_RENDER_SCALE: GameSettings.DEFAULT_RENDER_SCALE,
		GameSettings.KEY_SHADOW_QUALITY: GameSettings.DEFAULT_SHADOW_QUALITY,
		GameSettings.KEY_VIEW_DISTANCE: GameSettings.DEFAULT_VIEW_DISTANCE,
		GameSettings.KEY_BUILDING_LOD_QUALITY: GameSettings.DEFAULT_BUILDING_LOD_QUALITY,
	}
	_sync_controls()
	_emit_dirty_state()

func _persisted_values() -> Dictionary:
	return {
		# The window itself, not the saved value: the player can leave or enter fullscreen
		# outside Options, and Apply must not undo that.
		GameSettings.KEY_FULLSCREEN: GameSettings.is_window_fullscreen(),
		GameSettings.KEY_VSYNC: GameSettings.get_vsync_enabled(),
		GameSettings.KEY_SHOW_FPS: GameSettings.get_show_fps(),
		GameSettings.KEY_MAX_FPS: GameSettings.get_max_fps(),
		GameSettings.KEY_RENDER_SCALE: GameSettings.get_render_scale(),
		GameSettings.KEY_SHADOW_QUALITY: GameSettings.get_shadow_quality(),
		GameSettings.KEY_VIEW_DISTANCE: GameSettings.get_view_distance(),
		GameSettings.KEY_BUILDING_LOD_QUALITY: GameSettings.get_building_lod_quality(),
	}

# Option indices are stored directly except where the setting is a value from a choice list.
func _value_for_index(key: String, index: int) -> Variant:
	match key:
		GameSettings.KEY_MAX_FPS:
			return GameSettings.MAX_FPS_CHOICES[index]
		GameSettings.KEY_RENDER_SCALE:
			return GameSettings.RENDER_SCALE_CHOICES[index]
	return index

func _index_for_value(key: String, value: Variant) -> int:
	match key:
		GameSettings.KEY_MAX_FPS:
			return GameSettings.MAX_FPS_CHOICES.find(value)
		GameSettings.KEY_RENDER_SCALE:
			return GameSettings.RENDER_SCALE_CHOICES.find(value)
	return int(value)

func _set_pending(key: String, value: Variant) -> void:
	if _syncing:
		return
	_pending[key] = value
	_emit_dirty_state()

func _sync_controls() -> void:
	if not _fullscreen_check:
		return
	_syncing = true
	_fullscreen_check.button_pressed = _pending[GameSettings.KEY_FULLSCREEN]
	_vsync_check.button_pressed = _pending[GameSettings.KEY_VSYNC]
	_show_fps_check.button_pressed = _pending[GameSettings.KEY_SHOW_FPS]
	for pair in [
		[_max_fps, GameSettings.KEY_MAX_FPS],
		[_render_scale, GameSettings.KEY_RENDER_SCALE],
		[_shadow_quality, GameSettings.KEY_SHADOW_QUALITY],
		[_view_distance, GameSettings.KEY_VIEW_DISTANCE],
		[_lod_quality, GameSettings.KEY_BUILDING_LOD_QUALITY],
	]:
		(pair[0] as OptionButton).select(_index_for_value(pair[1], _pending[pair[1]]))
	_syncing = false

func _emit_dirty_state() -> void:
	emit_signal("dirty_changed", has_pending_changes())
