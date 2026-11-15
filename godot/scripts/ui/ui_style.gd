# SPDX-License-Identifier: GPL-2.0-only

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
const WINDOW_FOLLOWS_VIEWPORT_META := "metrum_window_follows_viewport"

const BG_DARK := Color(0.08, 0.08, 0.12, 0.93)
const BG_PANEL := Color(0.10, 0.10, 0.10, 0.80)
const BG_SUBMENU := Color(0.15, 0.15, 0.15, 0.70)
const BG_HUD_SHELL := Color(0.07, 0.07, 0.07, 0.72)
const BG_HUD_GROUP := Color(0.07, 0.07, 0.07, 0.56)
const BORDER_ACCENT := Color(0.30, 0.30, 0.45, 0.60)
const HUD_ACCENT := Color(0.38, 0.80, 0.88)
const TEXT_PRIMARY := Color.WHITE
const TEXT_DIM := Color(0.72, 0.72, 0.72)
const TEXT_SECTION := Color(0.65, 0.65, 0.90)
# Finnish traffic sign colours, used with the signs' own meanings so the HUD and the street
# signs agree: yellow warns, red forbids or removes.
const SIGN_YELLOW := Color(1.00, 0.808, 0.020)
const SIGN_RED := Color(0.886, 0.118, 0.184)
const SIGN_BLACK := Color(0.07, 0.07, 0.07)
# Sign red lifted for small text on the dark panels, where the plate red reads too dark.
const TEXT_ALERT := Color(0.96, 0.36, 0.40)
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
# The space a window placed or kept on screen leaves above it (the HUD's top bar) and below it.
const WINDOW_TOP_MARGIN := 40.0
const WINDOW_BOTTOM_MARGIN := 12.0
const HUD_STRIP_HEIGHT := 60.0
const HUD_BUTTON_HEIGHT := 60.0
const HUD_BOTTOM_MARGIN := 20.0
const HUD_PANEL_GAP := 12.0
const HUD_LEFT_MARGIN := 20.0
const HUD_SHELL_CORNER := 15
const HUD_SHELL_PAD_X := 15
const HUD_SHELL_PAD_Y := 10
const WINDOW_REFERENCE_VIEWPORT := Vector2(1920.0, 1080.0)
const MAX_WINDOW_LAYOUT_SCALE := 2.0
const WINDOW_MAX_VIEWPORT_COVERAGE := 0.92

## Consistent title/section typography for the road and building inspectors.
static func inspector_heading(text: String, section: bool = false) -> Label:
	var label := Label.new()
	label.text = text
	label.add_theme_color_override("font_color", TEXT_SECTION if section else TEXT_PRIMARY)
	set_font_size(label, 13 if section else 16)
	return label

## Accent action style for inspector commits; keeps disabled state visibly muted.
static func style_primary_button(button: Button) -> void:
	set_font_size(button, 14)
	button.focus_mode = Control.FOCUS_NONE
	for state in ["normal", "hover", "pressed", "disabled"]:
		var color := HUD_ACCENT.darkened(0.55)
		if state == "hover":
			color = color.lightened(0.12)
		elif state == "pressed":
			color = color.darkened(0.12)
		elif state == "disabled":
			color = BG_SUBMENU
		var style := panel_style(color, CORNER_SUB, Color.TRANSPARENT, 0)
		style.content_margin_left = 14
		style.content_margin_right = 14
		style.content_margin_top = 8
		style.content_margin_bottom = 8
		button.add_theme_stylebox_override(state, style)

static func ui_scale() -> float:
	return GameSettings.get_ui_scale()

static func scaled_font_size(base_size: int) -> int:
	return _font_size_at_scale(base_size, ui_scale())

static func _font_size_at_scale(base_size: int, scale: float) -> int:
	return maxi(8, int(roundf(float(base_size) * scale)))

static func scaled_px(base_size: float) -> float:
	return maxf(0.0, roundf(base_size * ui_scale()))

static func scaled_vector2(base_size: Vector2) -> Vector2:
	return Vector2(scaled_px(base_size.x), scaled_px(base_size.y))

static func _window_layout_scale(viewport: Viewport, current_ui_scale: float) -> float:
	var scale := maxf(1.0, current_ui_scale)
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
	return _window_size_at_scale(base_size, viewport, _window_layout_scale(viewport, ui_scale()), coverage)

static func _window_size_at_scale(
	base_size: Vector2i,
	viewport: Viewport,
	scale: float,
	coverage: float = WINDOW_MAX_VIEWPORT_COVERAGE
) -> Vector2i:
	var scaled := Vector2i(
		int(roundf(float(base_size.x) * scale)),
		int(roundf(float(base_size.y) * scale))
	)
	return _clamped_window_size_for_viewport(scaled, viewport, coverage)

static func set_font_size(control: Control, base_size: int) -> void:
	control.set_meta(FONT_SIZE_META, base_size)
	control.add_theme_font_size_override("font_size", scaled_font_size(base_size))

## Lets the game keep the keyboard while `window` is open, so the camera keys, zoom and Esc keep
## working. The window takes the focus only while the left button is down inside it: an
## unfocusable embedded window stops getting the mouse once it leaves, which would end a resize
## drag at its edge. On release it is shown again without the focus, unless the click went into
## an editable text field (a spin box's number, not its arrows): that keeps the keyboard for
## typing until the field is submitted (Enter) or left, then the game has it again.
static func keep_keyboard_with_game(window: Window) -> void:
	window.unfocusable = true
	window.window_input.connect(func(event: InputEvent) -> void:
		var button := event as InputEventMouseButton
		if button == null or button.button_index != MOUSE_BUTTON_LEFT:
			return
		if button.pressed:
			window.unfocusable = false
			window.grab_focus()
			return
		# The event's position is the embedder's; the field's rect is the window's.
		var field := window.gui_get_focus_owner() as LineEdit
		if field != null and field.editable \
				and field.get_global_rect().has_point(button.position - Vector2(window.position)):
			_give_keyboard_back_after_typing(window, field)
			return
		window.unfocusable = true
		_drop_focus.call_deferred(window)
	)

# The game gets the keyboard back once `field` is submitted or loses the focus. Each signal
# connects once, however often the field is clicked.
static func _give_keyboard_back_after_typing(window: Window, field: LineEdit) -> void:
	var left := _typing_done.bind(window)
	if not field.focus_exited.is_connected(left):
		field.focus_exited.connect(left, CONNECT_ONE_SHOT)
	var submitted := _typing_submitted.bind(window)
	if not field.text_submitted.is_connected(submitted):
		field.text_submitted.connect(submitted, CONNECT_ONE_SHOT)

static func _typing_submitted(_text: String, window: Window) -> void:
	_typing_done(window)

static func _typing_done(window: Window) -> void:
	if is_instance_valid(window):
		window.unfocusable = true
		_drop_focus.call_deferred(window)

# Showing an embedded window again registers it without the focus, in the same frame.
static func _drop_focus(window: Window) -> void:
	if is_instance_valid(window) and window.visible:
		window.hide()
		window.show()

## Puts `window` beside `screen_pos` (the mouse when zero): right of it, or left when it would
## leave the viewport, with its top a little above the point and clear of the top menu.
static func place_window_near_cursor(window: Window, screen_pos: Vector2, viewport: Viewport) -> void:
	if window == null or viewport == null:
		return
	if screen_pos == Vector2.ZERO:
		screen_pos = viewport.get_mouse_position()
	var viewport_size := viewport.get_visible_rect().size
	var window_size := Vector2(window.size)
	var padding := CURSOR_WINDOW_GAP
	var top_margin := WINDOW_TOP_MARGIN
	var bottom_margin := WINDOW_BOTTOM_MARGIN
	var x := screen_pos.x + padding
	if x + window_size.x > viewport_size.x - padding:
		x = screen_pos.x - window_size.x - padding
	if x < padding:
		x = padding
	var y := screen_pos.y - 24.0
	var max_y := maxf(top_margin, viewport_size.y - window_size.y - bottom_margin)
	y = clampf(y, top_margin, max_y)
	window.position = Vector2i(int(round(x)), int(round(y)))

## The mouse's back (thumb) button pressed: a menu takes it as Esc, as a browser takes it as Back.
static func is_mouse_back(event: InputEvent) -> bool:
	var button := event as InputEventMouseButton
	return button != null and button.pressed and button.button_index == MOUSE_BUTTON_XBUTTON1

## Moves `window` up when it hangs off the bottom of the screen (a window that grew to its
## content), keeping the same top and bottom margins as `place_window_near_cursor`.
static func keep_window_bottom_on_screen(window: Window, viewport: Viewport) -> void:
	if window == null or viewport == null:
		return
	var viewport_height := viewport.get_visible_rect().size.y
	if window.position.y + window.size.y > viewport_height - WINDOW_BOTTOM_MARGIN:
		window.position.y = maxi(int(WINDOW_TOP_MARGIN), int(viewport_height - WINDOW_BOTTOM_MARGIN) - window.size.y)

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
	_apply_window_base_size(window, viewport, false, ui_scale())
	_follow_viewport_size(window, viewport)

# Refits the window whenever the surface it opens on is resized. Leaving fullscreen shrinks the
# game's viewport, and a window still sized for the fullscreen one hangs off its edges with its
# footer buttons out of reach.
static func _follow_viewport_size(window: Window, viewport: Viewport) -> void:
	var resolved_viewport := viewport if viewport != null else _parent_viewport_for_window(window)
	if resolved_viewport == null or window.has_meta(WINDOW_FOLLOWS_VIEWPORT_META):
		return
	window.set_meta(WINDOW_FOLLOWS_VIEWPORT_META, true)
	var refit := func() -> void:
		_apply_window_base_size(window, resolved_viewport, true, ui_scale())
	resolved_viewport.size_changed.connect(refit)
	window.tree_exiting.connect(func() -> void:
		resolved_viewport.size_changed.disconnect(refit)
		window.remove_meta(WINDOW_FOLLOWS_VIEWPORT_META)
	, CONNECT_ONE_SHOT)

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
	# One settings snapshot keeps the entire refresh coherent without caching later reads.
	_refresh_at_scale(root, ui_scale())

static func _refresh_at_scale(root: Node, scale: float) -> void:
	if root is Window:
		var window := root as Window
		if window.has_meta(WINDOW_BASE_SIZE_META) and window.has_meta(WINDOW_BASE_MIN_SIZE_META):
			_apply_window_base_size(window, _parent_viewport_for_window(window), true, scale)
	if root is Control:
		var control: Control = root
		if control.has_meta(FONT_SIZE_META):
			control.add_theme_font_size_override(
				"font_size",
				_font_size_at_scale(int(control.get_meta(FONT_SIZE_META)), scale)
			)
		control.queue_redraw()
	for child in root.get_children():
		_refresh_at_scale(child, scale)

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
		_apply_window_base_size(window, resolved_viewport, true, ui_scale())
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

static func _apply_window_base_size(window: Window, viewport: Viewport, keep_larger_size: bool, current_ui_scale: float) -> void:
	var base_size: Vector2i = window.get_meta(WINDOW_BASE_SIZE_META)
	var base_min_size: Vector2i = window.get_meta(WINDOW_BASE_MIN_SIZE_META)
	var resolved_viewport := viewport if viewport != null else _parent_viewport_for_window(window)
	var scale := _window_layout_scale(resolved_viewport, current_ui_scale)
	var scaled_min := _window_size_at_scale(base_min_size, resolved_viewport, scale)
	var scaled_default := _window_size_at_scale(base_size, resolved_viewport, scale)
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

## An amount with thousands separators, for Apply labels: 1230 -> "1,230".
static func thousands(value: int) -> String:
	var digits := str(absi(value))
	var text := ""
	while digits.length() > 3:
		text = "," + digits.substr(digits.length() - 3) + text
		digits = digits.substr(0, digits.length() - 3)
	return ("-" if value < 0 else "") + digits + text

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

## A menu entry drawn as plain uppercase text (main menu, pause menu, Options tabs and
## footer): dim at rest, white under the mouse, with an accent bar at the left edge when `bar`.
## A pressed toggle shows in the accent colour.
static func menu_text_button(text: String, font_size: int, bar: bool = true) -> Button:
	var button := Button.new()
	button.text = text.to_upper()
	button.alignment = HORIZONTAL_ALIGNMENT_LEFT
	button.focus_mode = Control.FOCUS_NONE
	set_font_size(button, font_size)
	var idle := StyleBoxFlat.new()
	idle.bg_color = Color.TRANSPARENT
	idle.content_margin_left = 14 if bar else 0
	idle.content_margin_right = 14 if bar else 0
	idle.content_margin_top = 4
	idle.content_margin_bottom = 4
	var lit: StyleBoxFlat = idle.duplicate()
	if bar:
		lit.border_width_left = 3
		lit.border_color = HUD_ACCENT
	for state in ["normal", "focus", "disabled", "pressed"]:
		button.add_theme_stylebox_override(state, idle)
	for state in ["hover", "hover_pressed"]:
		button.add_theme_stylebox_override(state, lit)
	button.add_theme_color_override("font_color", TEXT_DIM)
	button.add_theme_color_override("font_hover_color", TEXT_PRIMARY)
	button.add_theme_color_override("font_pressed_color", HUD_ACCENT)
	button.add_theme_color_override("font_hover_pressed_color", HUD_ACCENT)
	button.add_theme_color_override("font_disabled_color", Color(TEXT_DIM, 0.35))
	return button
