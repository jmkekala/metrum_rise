# SPDX-License-Identifier: GPL-2.0-only

## Persistent player-facing settings backed by Godot ConfigFile.
##
## This owns general runtime/UI preferences. Boot-critical mod activation stays
## in ModPackConfig because content-pack loading has a separate lifecycle.
extends RefCounted

const CFG_PATH := "user://settings.cfg"

const SECTION_OPTIONS_WINDOW := "options_window"
const SECTION_GAMEPLAY := "gameplay"
const KEY_ROAD_PREVIEW_MODE := "road_preview_mode"
const DEFAULT_ROAD_PREVIEW_MODE := 0

const SECTION_GRAPHICS := "graphics"
const SECTION_ACCESSIBILITY := "accessibility"
const SECTION_LAYOUT_PREFIX := "layout/"

const KEY_ACTIVE_CATEGORY := "active_category"
const KEY_WINDOW_WIDTH := "window_width"
const KEY_WINDOW_HEIGHT := "window_height"
const KEY_WINDOW_X := "window_x"
const KEY_WINDOW_Y := "window_y"
const KEY_FULLSCREEN := "fullscreen"
const KEY_BUILDING_LOD_QUALITY := "building_lod_quality"
const KEY_VSYNC := "vsync"
const KEY_MAX_FPS := "max_fps"
const KEY_SHOW_FPS := "show_fps"
const KEY_RENDER_SCALE := "render_scale"
const KEY_SHADOW_QUALITY := "shadow_quality"
const KEY_VIEW_DISTANCE := "view_distance"
const KEY_UI_SCALE := "ui_scale"
const KEY_HAS_POSITION := "has_position"

const DEFAULT_OPTIONS_CATEGORY := "mods"
const DEFAULT_OPTIONS_WINDOW_WIDTH := 820
const DEFAULT_OPTIONS_WINDOW_HEIGHT := 540
const DEFAULT_FULLSCREEN := false
const DEFAULT_BUILDING_LOD_QUALITY := 1
const DEFAULT_VSYNC := true
## 0 is unlimited.
const MAX_FPS_CHOICES := [0, 30, 60, 120, 144]
const DEFAULT_MAX_FPS := 0
## The main menu draws next to nothing, so uncapped it spins at hundreds of frames per second.
const MENU_MAX_FPS := 120
const LAUNCH_WINDOW_SCREEN_COVERAGE := 0.9
const DEFAULT_SHOW_FPS := false
## 3D render scale; below 1.0 the frame is rendered smaller and upscaled with FSR 2.
const RENDER_SCALE_CHOICES := [1.0, 0.77, 0.67, 0.5]
const DEFAULT_RENDER_SCALE := 1.0
const SHADOW_QUALITY_HIGH := 0
## Plain PCF, two cascades and no cascade blending: the measured low-end shadow levers.
const SHADOW_QUALITY_LOW := 1
const DEFAULT_SHADOW_QUALITY := SHADOW_QUALITY_HIGH
const VIEW_DISTANCE_FULL := 0
## Caps the camera far plane at REDUCED_VIEW_DISTANCE_M instead of growing it with zoom.
const VIEW_DISTANCE_REDUCED := 1
const DEFAULT_VIEW_DISTANCE := VIEW_DISTANCE_FULL
const REDUCED_VIEW_DISTANCE_M := 3000.0
const DEFAULT_UI_SCALE := 1.0
const MIN_UI_SCALE := 0.8
const MAX_UI_SCALE := 2.0
const UI_SCALE_STEP := 0.05

static var _menu_frame_cap := false

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

static func get_road_preview_mode() -> int:
	var value := int(get_value(SECTION_GAMEPLAY, KEY_ROAD_PREVIEW_MODE, DEFAULT_ROAD_PREVIEW_MODE))
	return value if value in [0, 1] else DEFAULT_ROAD_PREVIEW_MODE

static func get_fullscreen_enabled() -> bool:
	return bool(_graphics_value(KEY_FULLSCREEN, DEFAULT_FULLSCREEN))

static func save_fullscreen_enabled(enabled: bool) -> Error:
	return set_value(SECTION_GRAPHICS, KEY_FULLSCREEN, enabled)

static func get_building_lod_quality() -> int:
	var value := int(_graphics_value(KEY_BUILDING_LOD_QUALITY, DEFAULT_BUILDING_LOD_QUALITY))
	return value if value >= 0 and value <= 2 else DEFAULT_BUILDING_LOD_QUALITY

static func get_vsync_enabled() -> bool:
	return bool(_graphics_value(KEY_VSYNC, DEFAULT_VSYNC))

static func get_max_fps() -> int:
	var value := int(_graphics_value(KEY_MAX_FPS, DEFAULT_MAX_FPS))
	return value if value in MAX_FPS_CHOICES else DEFAULT_MAX_FPS

static func get_show_fps() -> bool:
	return bool(_graphics_value(KEY_SHOW_FPS, DEFAULT_SHOW_FPS))

static func get_render_scale() -> float:
	var value := float(_graphics_value(KEY_RENDER_SCALE, DEFAULT_RENDER_SCALE))
	for choice in RENDER_SCALE_CHOICES:
		if is_equal_approx(value, choice):
			return choice
	return DEFAULT_RENDER_SCALE

static func get_shadow_quality() -> int:
	var value := int(_graphics_value(KEY_SHADOW_QUALITY, DEFAULT_SHADOW_QUALITY))
	return value if value in [SHADOW_QUALITY_HIGH, SHADOW_QUALITY_LOW] else DEFAULT_SHADOW_QUALITY

static func get_view_distance() -> int:
	var value := int(_graphics_value(KEY_VIEW_DISTANCE, DEFAULT_VIEW_DISTANCE))
	return value if value in [VIEW_DISTANCE_FULL, VIEW_DISTANCE_REDUCED] else DEFAULT_VIEW_DISTANCE

# Benchmarks measure the authored frame, so the player's render settings do not reach them.
static func _graphics_value(key: String, default_value: Variant) -> Variant:
	if _is_benchmark_run():
		return default_value
	return get_value(SECTION_GRAPHICS, key, default_value)

static func _is_benchmark_run() -> bool:
	var args := OS.get_cmdline_user_args()
	return "--gameplay-road-benchmark" in args or "--benchmark" in args

## Applies the player's frame rate cap, held to MENU_MAX_FPS or lower while the main menu is up.
static func apply_max_fps() -> void:
	var cap := get_max_fps()
	if _menu_frame_cap:
		cap = MENU_MAX_FPS if cap == 0 else mini(cap, MENU_MAX_FPS)
	Engine.max_fps = cap

## The main menu sets this while it is in the tree.
static func set_menu_frame_cap(enabled: bool) -> void:
	_menu_frame_cap = enabled
	apply_max_fps()

static func apply_display_settings() -> void:
	apply_fullscreen_enabled(get_fullscreen_enabled())
	apply_frame_settings()
	var root := (Engine.get_main_loop() as SceneTree).root
	apply_display_scale()
	if not root.dpi_changed.is_connected(apply_display_scale):
		root.dpi_changed.connect(apply_display_scale)

## Sizes a windowed launch for the screen's backing scale. The project's window size is in
## pixels, which on a Retina screen is a quarter of the area it was laid out for. The window
## keeps the project's aspect, grows by the backing scale and is held to 90% of the usable
## screen, centred. Call once at launch: a size the player picks afterwards stays theirs.
static func fit_launch_window() -> void:
	# Benchmarks render at the project's fixed size so their frames stay comparable.
	if _is_benchmark_run() or DisplayServer.window_get_mode() != DisplayServer.WINDOW_MODE_WINDOWED:
		return
	var screen := DisplayServer.window_get_current_screen()
	var usable := DisplayServer.screen_get_usable_rect(screen)
	if usable.size.x <= 0 or usable.size.y <= 0:
		return
	var wanted := Vector2(
		ProjectSettings.get_setting("display/window/size/viewport_width"),
		ProjectSettings.get_setting("display/window/size/viewport_height")
	) * maxf(1.0, DisplayServer.screen_get_scale(screen))
	var fit := minf(1.0, LAUNCH_WINDOW_SCREEN_COVERAGE * minf(usable.size.x / wanted.x, usable.size.y / wanted.y))
	var size := Vector2i((wanted * fit).round())
	DisplayServer.window_set_size(size)
	DisplayServer.window_set_position(usable.position + (usable.size - size) / 2)

## Scales all 2D content by the screen's backing scale (2 on a Retina Mac, 1 elsewhere), so
## 100% UI scale is the size the OS draws its own interface. The 3D view keeps rendering at the
## window's full pixel size. Follows the window onto screens with a different scale. Benchmark
## runs keep 1, so the HUD covers the same share of the frame on every machine.
static func apply_display_scale() -> void:
	var root := (Engine.get_main_loop() as SceneTree).root
	root.content_scale_factor = 1.0 if _is_benchmark_run() else maxf(
		1.0, DisplayServer.screen_get_scale(root.current_screen))

## Applies vsync, the frame rate cap and the 3D render scale. Shadow quality and view distance
## belong to the world scene and are applied by its owners on the `graphics_settings` group.
static func apply_frame_settings() -> void:
	DisplayServer.window_set_vsync_mode(
		DisplayServer.VSYNC_ENABLED if get_vsync_enabled() else DisplayServer.VSYNC_DISABLED)
	apply_max_fps()
	var root := (Engine.get_main_loop() as SceneTree).root
	var render_scale := get_render_scale()
	if is_equal_approx(render_scale, 1.0):
		root.scaling_3d_mode = Viewport.SCALING_3D_MODE_BILINEAR
		root.scaling_3d_scale = 1.0
		root.screen_space_aa = ProjectSettings.get_setting("rendering/anti_aliasing/quality/screen_space_aa")
	else:
		root.scaling_3d_mode = Viewport.SCALING_3D_MODE_FSR2
		root.scaling_3d_scale = render_scale
		# FSR 2 resolves its own edges from temporal samples. Blurring its output again with a
		# post-process filter costs a pass and softens what the upscaler just reconstructed.
		root.screen_space_aa = Viewport.SCREEN_SPACE_AA_DISABLED

static func apply_fullscreen_enabled(enabled: bool) -> void:
	if enabled != is_window_fullscreen():
		DisplayServer.window_set_mode(
			DisplayServer.WINDOW_MODE_FULLSCREEN if enabled else DisplayServer.WINDOW_MODE_WINDOWED)

## Whether the window is fullscreen now, however it got there: the setting, the macOS green
## button or Ctrl+Cmd+F all leave it in one of the two fullscreen modes.
static func is_window_fullscreen() -> bool:
	return DisplayServer.window_get_mode() in [
		DisplayServer.WINDOW_MODE_FULLSCREEN, DisplayServer.WINDOW_MODE_EXCLUSIVE_FULLSCREEN]

static func get_ui_scale() -> float:
	return normalized_ui_scale(float(get_value(
		SECTION_ACCESSIBILITY,
		KEY_UI_SCALE,
		DEFAULT_UI_SCALE
	)))

static func save_ui_scale(ui_scale: float) -> Error:
	return set_value(SECTION_ACCESSIBILITY, KEY_UI_SCALE, normalized_ui_scale(ui_scale))

static func normalized_ui_scale(ui_scale: float) -> float:
	if not is_finite(ui_scale):
		return DEFAULT_UI_SCALE
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
	# A failed ConfigFile load can retain values parsed before the error.
	cfg.clear()
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_ACTIVE_CATEGORY, DEFAULT_OPTIONS_CATEGORY)
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_WIDTH, DEFAULT_OPTIONS_WINDOW_WIDTH)
	cfg.set_value(SECTION_OPTIONS_WINDOW, KEY_WINDOW_HEIGHT, DEFAULT_OPTIONS_WINDOW_HEIGHT)
	cfg.set_value(SECTION_GAMEPLAY, KEY_ROAD_PREVIEW_MODE, DEFAULT_ROAD_PREVIEW_MODE)
	cfg.set_value(SECTION_GRAPHICS, KEY_FULLSCREEN, DEFAULT_FULLSCREEN)
	cfg.set_value(SECTION_GRAPHICS, KEY_BUILDING_LOD_QUALITY, DEFAULT_BUILDING_LOD_QUALITY)
	cfg.set_value(SECTION_GRAPHICS, KEY_VSYNC, DEFAULT_VSYNC)
	cfg.set_value(SECTION_GRAPHICS, KEY_MAX_FPS, DEFAULT_MAX_FPS)
	cfg.set_value(SECTION_GRAPHICS, KEY_SHOW_FPS, DEFAULT_SHOW_FPS)
	cfg.set_value(SECTION_GRAPHICS, KEY_RENDER_SCALE, DEFAULT_RENDER_SCALE)
	cfg.set_value(SECTION_GRAPHICS, KEY_SHADOW_QUALITY, DEFAULT_SHADOW_QUALITY)
	cfg.set_value(SECTION_GRAPHICS, KEY_VIEW_DISTANCE, DEFAULT_VIEW_DISTANCE)
	cfg.set_value(SECTION_ACCESSIBILITY, KEY_UI_SCALE, DEFAULT_UI_SCALE)

static func _layout_section(layout_id: String) -> String:
	return SECTION_LAYOUT_PREFIX + layout_id
