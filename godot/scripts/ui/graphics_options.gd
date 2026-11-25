# SPDX-License-Identifier: GPL-3.0-or-later

## Graphics options panel.
##
## Owns pending display and render preference edits and commits them through the shared
## Options footer. Pending values are keyed by their GameSettings graphics key.
extends VBoxContainer

const GameSettings = preload("res://scripts/core/game_settings.gd")
const OptionsRows = preload("res://scripts/ui/options_rows.gd")

signal dirty_changed(has_pending_changes: bool)

const MAX_FPS_LABELS := ["Unlimited", "30", "60", "120", "144"]
const RENDER_SCALE_LABELS := ["Native", "77% (FSR 2)", "67% (FSR 2)", "50% (FSR 2)"]
const ON_OFF := ["Off", "On"]
const TOGGLES := [GameSettings.KEY_FULLSCREEN, GameSettings.KEY_VSYNC, GameSettings.KEY_SHOW_FPS]

var _initial := {}
var _pending := {}
var _syncing := false
# Setting key -> its value cycler.
var _choices: Dictionary = {}
var _max_fps: OptionsRows.Cycler
var _shadow_quality: OptionsRows.Cycler
var _lod_quality: OptionsRows.Cycler

func _ready() -> void:
	_build_ui()
	refresh()

func _build_ui() -> void:
	size_flags_horizontal = Control.SIZE_EXPAND_FILL
	add_theme_constant_override("separation", 0)

	add_child(OptionsRows.heading("Screen"))
	_add_choice("Fullscreen", GameSettings.KEY_FULLSCREEN, ON_OFF, "")
	_add_choice("V-Sync", GameSettings.KEY_VSYNC, ON_OFF,
		"Waits for the display refresh before showing a frame, which prevents tearing.")
	_max_fps = _add_choice("Max FPS", GameSettings.KEY_MAX_FPS, MAX_FPS_LABELS,
		"Caps the frame rate. With V-Sync on, the display refresh rate also caps it.")
	_add_choice("Show FPS", GameSettings.KEY_SHOW_FPS, ON_OFF,
		"Shows frames per second and frame time at the right end of the menu bar.")

	add_child(OptionsRows.heading("Graphics"))
	_add_choice("Render scale", GameSettings.KEY_RENDER_SCALE, RENDER_SCALE_LABELS,
		"Renders the 3D view at a lower resolution and upscales it with FSR 2. The interface stays sharp.")
	_shadow_quality = _add_choice("Shadows", GameSettings.KEY_SHADOW_QUALITY, ["High", "Medium", "Low"],
		"Medium keeps soft shadows but draws them in two cascades instead of four, so distant shadows are coarser.\nLow also makes them hard-edged.")
	_add_choice("View distance", GameSettings.KEY_VIEW_DISTANCE, ["Full", "Reduced"],
		"Reduced stops drawing the world beyond about 3 km from the camera, unless zoomed far out.")
	_lod_quality = _add_choice("Building detail", GameSettings.KEY_BUILDING_LOD_QUALITY,
		["Performance", "Balanced", "Quality"],
		"Controls when buildings switch to simpler meshes. Does not affect simulation or remove distant buildings.")
	_add_choice("Terrain relief shading", GameSettings.KEY_TERRAIN_RELIEF, ["Off", "Low", "Normal", "High"],
		"How strongly the sun and moon shade hills and slopes. Off is the flat look of earlier versions.")

func _add_choice(text: String, key: String, labels: Array, hint: String) -> OptionsRows.Cycler:
	var cycler := OptionsRows.Cycler.new()
	for item in labels:
		cycler.add_item(item)
	cycler.item_selected.connect(func(index: int): _set_pending(key, _value_for_index(key, index)))
	add_child(OptionsRows.row(text, cycler, hint))
	_choices[key] = cycler
	return cycler

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
		GameSettings.KEY_TERRAIN_RELIEF: GameSettings.DEFAULT_TERRAIN_RELIEF,
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
		GameSettings.KEY_TERRAIN_RELIEF: GameSettings.get_terrain_relief(),
	}

# Option indices are stored directly except for On/Off toggles and values from a choice list.
func _value_for_index(key: String, index: int) -> Variant:
	if key in TOGGLES:
		return index == 1
	match key:
		GameSettings.KEY_MAX_FPS:
			return GameSettings.MAX_FPS_CHOICES[index]
		GameSettings.KEY_RENDER_SCALE:
			return GameSettings.RENDER_SCALE_CHOICES[index]
		GameSettings.KEY_SHADOW_QUALITY:
			return GameSettings.SHADOW_QUALITY_CHOICES[index]
		GameSettings.KEY_TERRAIN_RELIEF:
			return GameSettings.TERRAIN_RELIEF_CHOICES[index]
	return index

func _index_for_value(key: String, value: Variant) -> int:
	if key in TOGGLES:
		return 1 if value else 0
	match key:
		GameSettings.KEY_MAX_FPS:
			return GameSettings.MAX_FPS_CHOICES.find(value)
		GameSettings.KEY_RENDER_SCALE:
			return GameSettings.RENDER_SCALE_CHOICES.find(value)
		GameSettings.KEY_SHADOW_QUALITY:
			return GameSettings.SHADOW_QUALITY_CHOICES.find(value)
		GameSettings.KEY_TERRAIN_RELIEF:
			return GameSettings.TERRAIN_RELIEF_CHOICES.find(value)
	return int(value)

func _set_pending(key: String, value: Variant) -> void:
	if _syncing:
		return
	_pending[key] = value
	_emit_dirty_state()

func _sync_controls() -> void:
	if _choices.is_empty():
		return
	_syncing = true
	for key in _choices:
		(_choices[key] as OptionsRows.Cycler).select(_index_for_value(key, _pending[key]))
	_syncing = false

func _emit_dirty_state() -> void:
	emit_signal("dirty_changed", has_pending_changes())
