## Shared style constants and helper factories for procedural UI.
##
## New UI code should use these helpers instead of open-coding StyleBoxFlat
## setup in each script.
extends RefCounted

const GameSettings = preload("res://scripts/core/game_settings.gd")

const FONT_SIZE_META := "metrum_base_font_size"
const WINDOW_BASE_SIZE_META := "metrum_base_window_size"
const WINDOW_BASE_MIN_SIZE_META := "metrum_base_window_min_size"
const WINDOW_LAYOUT_ID_META := "metrum_window_layout_id"
const WINDOW_PERSIST_POSITION_META := "metrum_window_persist_position"
const WINDOW_LAYOUT_CONNECTED_META := "metrum_window_layout_connected"
const WINDOW_HAS_RESTORED_POSITION_META := "metrum_window_has_restored_position"

const BG_DARK := Color(0.08, 0.08, 0.12, 0.93)
const BG_PANEL := Color(0.10, 0.10, 0.10, 0.80)
const BG_SUBMENU := Color(0.15, 0.15, 0.15, 0.70)
const BG_HUD_SHELL := Color(0.07, 0.07, 0.07, 0.72)
const BG_HUD_GROUP := Color(0.07, 0.07, 0.07, 0.56)
const BORDER_ACCENT := Color(0.30, 0.30, 0.45, 0.60)
const TEXT_PRIMARY := Color.WHITE
const TEXT_DIM := Color(0.72, 0.72, 0.72)
const TEXT_SECTION := Color(0.65, 0.65, 0.90)
const TEXT_ALERT := Color(1.00, 0.40, 0.30)
const HUD_TEXT_SIZE := 16
const ZONE_RESIDENTIAL := Color(0.20, 0.45, 0.25, 0.75)
const ZONE_COMMERCIAL := Color(0.20, 0.34, 0.62, 0.75)
const ZONE_INDUSTRIAL := Color(0.55, 0.47, 0.14, 0.75)

const CORNER_WINDOW := 8
const CORNER_PANEL := 12
const CORNER_SUB := 10

const PAD_WINDOW := 12
const PAD_PANEL := 15
const PAD_INNER := 8
const CURSOR_WINDOW_GAP := 40.0
const HUD_STRIP_HEIGHT := 60.0
const HUD_BUTTON_HEIGHT := 60.0
const HUD_BOTTOM_MARGIN := 20.0
const HUD_PANEL_GAP := 12.0
const HUD_LEFT_MARGIN := 20.0
const HUD_SHELL_CORNER := 15
const HUD_SHELL_PAD_X := 15
const HUD_SHELL_PAD_Y := 10
const WINDOW_REFERENCE_VIEWPORT := Vector2(1920.0, 1080.0)
const MAX_WINDOW_LAYOUT_SCALE := 1.6
const WINDOW_MAX_VIEWPORT_COVERAGE := 0.92

static func ui_scale() -> float:
	return GameSettings.get_ui_scale()

static func scaled_font_size(base_size: int) -> int:
	return maxi(8, int(roundf(float(base_size) * ui_scale())))

static func scaled_px(base_size: float) -> float:
	return maxf(0.0, roundf(base_size * ui_scale()))

static func scaled_vector2(base_size: Vector2) -> Vector2:
	return Vector2(scaled_px(base_size.x), scaled_px(base_size.y))

static func window_layout_scale(viewport: Viewport = null) -> float:
	var scale := maxf(1.0, ui_scale())
	if viewport != null:
		var viewport_size := viewport.get_visible_rect().size
		if viewport_size.x > 1.0 and viewport_size.y > 1.0:
			var resolution_scale := sqrt(minf(
				viewport_size.x / WINDOW_REFERENCE_VIEWPORT.x,
				viewport_size.y / WINDOW_REFERENCE_VIEWPORT.y
			))
			scale = maxf(scale, resolution_scale)
	return clampf(scale, 1.0, MAX_WINDOW_LAYOUT_SCALE)

static func scaled_window_size(
	base_size: Vector2i,
	viewport: Viewport = null,
	coverage: float = WINDOW_MAX_VIEWPORT_COVERAGE
) -> Vector2i:
	var scale := window_layout_scale(viewport)
	var scaled := Vector2i(
		int(roundf(float(base_size.x) * scale)),
		int(roundf(float(base_size.y) * scale))
	)
	return _clamped_window_size_for_viewport(scaled, viewport, coverage)

static func set_font_size(control: Control, base_size: int) -> void:
	control.set_meta(FONT_SIZE_META, base_size)
	control.add_theme_font_size_override("font_size", scaled_font_size(base_size))

static func set_window_base_size(
	window: Window,
	base_size: Vector2i,
	base_min_size: Vector2i,
	viewport: Viewport = null
) -> void:
	if window == null:
		return
	window.set_meta(WINDOW_BASE_SIZE_META, base_size)
	window.set_meta(WINDOW_BASE_MIN_SIZE_META, base_min_size)
	_apply_window_base_size(window, viewport, false)

static func set_persistent_window_layout(
	window: Window,
	layout_id: String,
	base_size: Vector2i,
	base_min_size: Vector2i,
	viewport: Viewport = null,
	persist_position: bool = true
) -> void:
	if window == null:
		return
	set_window_base_size(window, base_size, base_min_size, viewport)
	window.set_meta(WINDOW_LAYOUT_ID_META, layout_id)
	window.set_meta(WINDOW_PERSIST_POSITION_META, persist_position)
	_restore_persistent_window_layout(window, viewport)
	_connect_persistent_window_saves(window)

static func popup_persistent_window(window: Window, center_without_restored_position: bool = true) -> void:
	if window == null:
		return
	var has_restored_position := has_persistent_window_position(window)
	if center_without_restored_position and not has_restored_position:
		window.popup_centered(window.size)
	else:
		window.popup()
	if window.has_meta(WINDOW_LAYOUT_ID_META):
		_restore_persistent_window_layout(window, _parent_viewport_for_window(window))

static func has_persistent_window_position(window: Window) -> bool:
	return window != null and bool(window.get_meta(WINDOW_HAS_RESTORED_POSITION_META, false))

static func save_persistent_window_layout(window: Window) -> Error:
	if window == null or not window.has_meta(WINDOW_LAYOUT_ID_META):
		return OK
	return GameSettings.save_window_layout(
		str(window.get_meta(WINDOW_LAYOUT_ID_META)),
		window.size,
		window.position,
		bool(window.get_meta(WINDOW_PERSIST_POSITION_META, true))
	)

static func refresh_scaled_font_sizes(root: Node) -> void:
	if root is Window:
		var window := root as Window
		if window.has_meta(WINDOW_BASE_SIZE_META) and window.has_meta(WINDOW_BASE_MIN_SIZE_META):
			_apply_window_base_size(window, _parent_viewport_for_window(window), true)
	if root is Control:
		var control: Control = root
		if control.has_meta(FONT_SIZE_META):
			control.add_theme_font_size_override(
				"font_size",
				scaled_font_size(int(control.get_meta(FONT_SIZE_META)))
			)
		control.queue_redraw()
	for child in root.get_children():
		refresh_scaled_font_sizes(child)

static func _restore_persistent_window_layout(window: Window, viewport: Viewport) -> void:
	var layout := GameSettings.load_window_layout(str(window.get_meta(WINDOW_LAYOUT_ID_META)))
	var resolved_viewport := viewport if viewport != null else _parent_viewport_for_window(window)
	if bool(layout.get("has_size", false)):
		var persisted_size: Vector2i = layout.get("size", Vector2i.ZERO)
		window.size = _clamped_window_size_for_viewport(
			Vector2i(
				maxi(window.min_size.x, persisted_size.x),
				maxi(window.min_size.y, persisted_size.y)
			),
			resolved_viewport
		)
	elif window.has_meta(WINDOW_BASE_SIZE_META) and window.has_meta(WINDOW_BASE_MIN_SIZE_META):
		_apply_window_base_size(window, resolved_viewport, true)
	var has_position := bool(window.get_meta(WINDOW_PERSIST_POSITION_META, true)) and bool(layout.get("has_position", false))
	window.set_meta(WINDOW_HAS_RESTORED_POSITION_META, has_position)
	if has_position:
		var persisted_position: Vector2i = layout.get("position", Vector2i.ZERO)
		window.position = persisted_position
	_clamp_window_position(window, resolved_viewport)

static func _connect_persistent_window_saves(window: Window) -> void:
	if window.has_meta(WINDOW_LAYOUT_CONNECTED_META):
		return
	window.set_meta(WINDOW_LAYOUT_CONNECTED_META, true)
	window.visibility_changed.connect(func() -> void:
		if is_instance_valid(window) and not window.visible:
			save_persistent_window_layout(window)
	)
	window.tree_exiting.connect(func() -> void:
		if is_instance_valid(window):
			save_persistent_window_layout(window)
	)

static func _apply_window_base_size(window: Window, viewport: Viewport, keep_larger_size: bool) -> void:
	var base_size: Vector2i = window.get_meta(WINDOW_BASE_SIZE_META)
	var base_min_size: Vector2i = window.get_meta(WINDOW_BASE_MIN_SIZE_META)
	var resolved_viewport := viewport if viewport != null else _parent_viewport_for_window(window)
	var scaled_min := scaled_window_size(base_min_size, resolved_viewport)
	var scaled_default := scaled_window_size(base_size, resolved_viewport)
	window.min_size = scaled_min
	if keep_larger_size:
		window.size = _clamped_window_size_for_viewport(
			Vector2i(maxi(window.size.x, scaled_default.x), maxi(window.size.y, scaled_default.y)),
			resolved_viewport
		)
	else:
		window.size = scaled_default
	_clamp_window_position(window, resolved_viewport)

static func _parent_viewport_for_window(window: Window) -> Viewport:
	if window != null and window.get_parent() != null:
		return window.get_parent().get_viewport()
	if window != null:
		return window.get_viewport()
	return null

static func _clamped_window_size_for_viewport(
	requested: Vector2i,
	viewport: Viewport,
	coverage: float = WINDOW_MAX_VIEWPORT_COVERAGE
) -> Vector2i:
	if viewport == null:
		return requested
	var viewport_size := viewport.get_visible_rect().size
	if viewport_size.x <= 1.0 or viewport_size.y <= 1.0:
		return requested
	var max_size := Vector2i(
		maxi(220, int(roundf(viewport_size.x * coverage))),
		maxi(160, int(roundf(viewport_size.y * coverage)))
	)
	return Vector2i(mini(requested.x, max_size.x), mini(requested.y, max_size.y))

static func _clamp_window_position(window: Window, viewport: Viewport) -> void:
	if window == null or viewport == null:
		return
	var viewport_size := Vector2i(viewport.get_visible_rect().size)
	if viewport_size.x <= 0 or viewport_size.y <= 0:
		return
	var clamped_size := Vector2i(mini(window.size.x, viewport_size.x), mini(window.size.y, viewport_size.y))
	var max_position := Vector2i(
		maxi(0, viewport_size.x - clamped_size.x),
		maxi(0, viewport_size.y - clamped_size.y)
	)
	window.position = Vector2i(
		clampi(window.position.x, 0, max_position.x),
		clampi(window.position.y, 0, max_position.y)
	)

static func panel_style(
	bg_color: Color = BG_PANEL,
	corner_radius: int = CORNER_PANEL,
	border_color: Color = BORDER_ACCENT,
	border_width: int = 1
) -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = bg_color
	style.set_corner_radius_all(corner_radius)
	style.border_width_left = border_width
	style.border_width_right = border_width
	style.border_width_top = border_width
	style.border_width_bottom = border_width
	style.border_color = border_color
	return style

static func window_body_style() -> StyleBoxFlat:
	return panel_style(BG_DARK, CORNER_WINDOW)

static func submenu_style() -> StyleBoxFlat:
	return panel_style(BG_SUBMENU, CORNER_SUB)

static func hud_shell_style() -> StyleBoxFlat:
	return panel_style(BG_HUD_SHELL, HUD_SHELL_CORNER, BORDER_ACCENT, 0)

static func hud_group_style() -> StyleBoxFlat:
	return panel_style(BG_HUD_GROUP, HUD_SHELL_CORNER, BORDER_ACCENT, 0)

static func hud_clear_style() -> StyleBoxFlat:
	return panel_style(Color(0.0, 0.0, 0.0, 0.0), HUD_SHELL_CORNER, BORDER_ACCENT, 0)
