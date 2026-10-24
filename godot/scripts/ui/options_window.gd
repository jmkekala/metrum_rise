# SPDX-License-Identifier: GPL-2.0-only

## Shared options window for main menu and gameplay contexts.
##
## Owns the category shell and global footer actions while category panels own
## their own pending state and persistence.
extends Window

const AccessibilityOptions = preload("res://scripts/ui/accessibility_options.gd")
const GameSettings = preload("res://scripts/core/game_settings.gd")
const GraphicsOptions = preload("res://scripts/ui/graphics_options.gd")
const PackManager = preload("res://scripts/ui/pack_manager.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")

const CONTEXT_MAIN_MENU := "main_menu"
const CONTEXT_GAMEPLAY := "gameplay"
const BASE_WINDOW_SIZE := Vector2i(GameSettings.DEFAULT_OPTIONS_WINDOW_WIDTH, GameSettings.DEFAULT_OPTIONS_WINDOW_HEIGHT)
const BASE_MIN_SIZE := Vector2i(680, 430)

const CATEGORY_GAMEPLAY := "gameplay"
const CATEGORY_GRAPHICS := "graphics"
const CATEGORY_AUDIO := "audio"
const CATEGORY_CONTROLS := "controls"
const CATEGORY_ACCESSIBILITY := "accessibility"
const CATEGORY_MODS := "mods"

var context: String = CONTEXT_GAMEPLAY

var _category_buttons: Dictionary = {}
var _category_contents: Dictionary = {}
var _content_root: VBoxContainer
var _active_category := CATEGORY_MODS
var _apply_btn: Button
var _reset_btn: Button
var _status_label: Label
var _opened_once := false
var _has_saved_position := false
var _saved_position := Vector2i.ZERO

func _ready() -> void:
	title = "Options"
	unresizable = false
	exclusive = true
	close_requested.connect(_on_cancel_pressed)
	UIStyle.set_window_base_size(self, BASE_WINDOW_SIZE, BASE_MIN_SIZE, _owner_viewport())

	var state := GameSettings.load_options_window_state()
	size = _clamped_window_size(_restored_window_size(state.get("size", BASE_WINDOW_SIZE)))
	_saved_position = state.get("position", Vector2i.ZERO)
	_has_saved_position = bool(state.get("has_position", false))
	var initial_category := str(state.get("active_category", CATEGORY_MODS))

	_build_ui()
	_select_category(_valid_category_id(initial_category))
	_sync_footer_state()
	WindowResizeHandles.install(self)

func popup_options(next_context: String) -> void:
	context = next_context
	_discard_pending_changes()
	if _opened_once:
		show()
		if _has_saved_position:
			position = _clamped_window_position(_saved_position)
		grab_focus()
	else:
		popup_centered(size)
		if _has_saved_position:
			position = _clamped_window_position(_saved_position)
		_opened_once = true

func _build_ui() -> void:
	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	add_child(body)

	var outer_margin := MarginContainer.new()
	outer_margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	outer_margin.add_theme_constant_override("margin_left", UIStyle.PAD_WINDOW)
	outer_margin.add_theme_constant_override("margin_right", UIStyle.PAD_WINDOW)
	outer_margin.add_theme_constant_override("margin_top", UIStyle.PAD_WINDOW)
	outer_margin.add_theme_constant_override("margin_bottom", UIStyle.PAD_WINDOW)
	body.add_child(outer_margin)

	var layout := VBoxContainer.new()
	layout.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	layout.size_flags_vertical = Control.SIZE_EXPAND_FILL
	layout.add_theme_constant_override("separation", 12)
	outer_margin.add_child(layout)

	var main_row := HBoxContainer.new()
	main_row.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	main_row.size_flags_vertical = Control.SIZE_EXPAND_FILL
	main_row.add_theme_constant_override("separation", 12)
	layout.add_child(main_row)

	var category_shell := PanelContainer.new()
	category_shell.custom_minimum_size.x = UIStyle.scaled_px(180.0)
	category_shell.size_flags_vertical = Control.SIZE_EXPAND_FILL
	category_shell.add_theme_stylebox_override("panel", UIStyle.submenu_style())
	main_row.add_child(category_shell)

	var category_margin := MarginContainer.new()
	category_margin.add_theme_constant_override("margin_left", 8)
	category_margin.add_theme_constant_override("margin_right", 8)
	category_margin.add_theme_constant_override("margin_top", 8)
	category_margin.add_theme_constant_override("margin_bottom", 8)
	category_shell.add_child(category_margin)

	var category_list := VBoxContainer.new()
	category_list.add_theme_constant_override("separation", 6)
	category_margin.add_child(category_list)

	_add_category_button(category_list, CATEGORY_GAMEPLAY, "Gameplay")
	_add_category_button(category_list, CATEGORY_GRAPHICS, "Graphics")
	_add_category_button(category_list, CATEGORY_AUDIO, "Audio")
	_add_category_button(category_list, CATEGORY_CONTROLS, "Controls")
	_add_category_button(category_list, CATEGORY_ACCESSIBILITY, "Accessibility")
	_add_category_button(category_list, CATEGORY_MODS, "Mods")

	var content_shell := PanelContainer.new()
	content_shell.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	content_shell.size_flags_vertical = Control.SIZE_EXPAND_FILL
	content_shell.add_theme_stylebox_override("panel", UIStyle.panel_style(Color(0.09, 0.09, 0.11, 0.90), UIStyle.CORNER_WINDOW))
	main_row.add_child(content_shell)

	var content_margin := MarginContainer.new()
	content_margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	content_margin.add_theme_constant_override("margin_left", 14)
	content_margin.add_theme_constant_override("margin_right", 14)
	content_margin.add_theme_constant_override("margin_top", 14)
	content_margin.add_theme_constant_override("margin_bottom", 14)
	content_shell.add_child(content_margin)

	_content_root = VBoxContainer.new()
	_content_root.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_content_root.size_flags_vertical = Control.SIZE_EXPAND_FILL
	content_margin.add_child(_content_root)

	_add_content(CATEGORY_GAMEPLAY, _make_empty_category("Gameplay"))
	_add_content(CATEGORY_GRAPHICS, _make_graphics_category())
	_add_content(CATEGORY_AUDIO, _make_empty_category("Audio"))
	_add_content(CATEGORY_CONTROLS, _make_empty_category("Controls"))
	_add_content(CATEGORY_ACCESSIBILITY, _make_accessibility_category())
	_add_content(CATEGORY_MODS, _make_mods_category())

	var footer := HBoxContainer.new()
	footer.add_theme_constant_override("separation", 8)
	layout.add_child(footer)

	_reset_btn = Button.new()
	_reset_btn.text = "Reset Defaults"
	UIStyle.set_font_size(_reset_btn, 14)
	_reset_btn.pressed.connect(_on_reset_defaults_pressed)
	footer.add_child(_reset_btn)

	_status_label = Label.new()
	_status_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_status_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(_status_label, 12)
	_status_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	footer.add_child(_status_label)

	var cancel_btn := Button.new()
	cancel_btn.text = "Cancel"
	UIStyle.set_font_size(cancel_btn, 14)
	cancel_btn.pressed.connect(_on_cancel_pressed)
	footer.add_child(cancel_btn)

	_apply_btn = Button.new()
	_apply_btn.text = "Apply"
	UIStyle.set_font_size(_apply_btn, 14)
	_apply_btn.pressed.connect(_on_apply_pressed)
	footer.add_child(_apply_btn)

func _add_category_button(parent: Control, category_id: String, label: String) -> void:
	var button := Button.new()
	button.text = label
	button.toggle_mode = true
	button.focus_mode = Control.FOCUS_NONE
	button.custom_minimum_size = UIStyle.scaled_vector2(Vector2(150.0, 38.0))
	button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(button, 14)
	button.pressed.connect(func(): _select_category(category_id))
	parent.add_child(button)
	_category_buttons[category_id] = button

func _add_content(category_id: String, content: Control) -> void:
	content.visible = false
	content.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	content.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_content_root.add_child(content)
	_category_contents[category_id] = content

func _make_empty_category(label: String) -> Control:
	var panel := VBoxContainer.new()
	panel.add_theme_constant_override("separation", 8)

	var heading := Label.new()
	heading.text = label
	UIStyle.set_font_size(heading, 18)
	heading.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	panel.add_child(heading)

	var body := Label.new()
	body.text = "No settings available yet."
	UIStyle.set_font_size(body, 13)
	body.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	panel.add_child(body)
	return panel

func _make_accessibility_category() -> Control:
	var panel := AccessibilityOptions.new()
	panel.dirty_changed.connect(func(_dirty: bool): _sync_footer_state())
	return panel

func _make_graphics_category() -> Control:
	var panel := GraphicsOptions.new()
	panel.dirty_changed.connect(func(_dirty: bool): _sync_footer_state())
	return panel

func _make_mods_category() -> Control:
	var panel := PackManager.new()
	panel.dirty_changed.connect(func(_dirty: bool): _sync_footer_state())
	return panel

func _select_category(category_id: String) -> void:
	if not _category_contents.has(category_id):
		return
	_active_category = category_id
	for key in _category_contents:
		(_category_contents[key] as Control).visible = key == category_id
	for key in _category_buttons:
		(_category_buttons[key] as Button).button_pressed = key == category_id
	_sync_footer_state()

func _on_reset_defaults_pressed() -> void:
	var content := _active_content()
	if content and content.has_method("reset_defaults"):
		content.call("reset_defaults")
		_sync_footer_state()

func _on_cancel_pressed() -> void:
	_save_window_state()
	_discard_pending_changes()
	hide()

func _on_apply_pressed() -> void:
	var restart_required := false
	for key in _category_contents:
		var content: Control = _category_contents[key]
		if not content.has_method("has_pending_changes") or not content.call("has_pending_changes"):
			continue
		if not content.has_method("apply_changes"):
			continue
		var err := int(content.call("apply_changes"))
		if err != OK:
			_status_label.text = "Could not apply settings."
			_sync_footer_state()
			return
		if key == CATEGORY_MODS:
			restart_required = true
	_save_window_state()
	_status_label.text = "Changes saved. Restart required." if restart_required else "Changes saved."
	_sync_footer_state()

func _discard_pending_changes() -> void:
	for key in _category_contents:
		var content: Control = _category_contents[key]
		if content.has_method("refresh"):
			content.call("refresh")
	if _status_label:
		_status_label.text = ""
	_sync_footer_state()

func _sync_footer_state() -> void:
	if not _apply_btn:
		return
	var dirty := false
	for key in _category_contents:
		var content: Control = _category_contents[key]
		if content.has_method("has_pending_changes") and content.call("has_pending_changes"):
			dirty = true
			break
	_apply_btn.disabled = not dirty

	var active := _active_content()
	_reset_btn.disabled = not active or not active.has_method("reset_defaults")
	if dirty:
		_status_label.text = "Pending changes."
	elif _status_label.text == "Pending changes.":
		_status_label.text = ""

func _active_content() -> Control:
	if not _category_contents.has(_active_category):
		return null
	return _category_contents[_active_category] as Control

func _save_window_state() -> void:
	_has_saved_position = true
	_saved_position = position
	GameSettings.save_options_window_state(_active_category, size, position)

func _valid_category_id(category_id: String) -> String:
	if category_id in [
		CATEGORY_GAMEPLAY,
		CATEGORY_GRAPHICS,
		CATEGORY_AUDIO,
		CATEGORY_CONTROLS,
		CATEGORY_ACCESSIBILITY,
		CATEGORY_MODS,
	]:
		return category_id
	return CATEGORY_MODS

func _clamped_window_size(value: Variant) -> Vector2i:
	if not (value is Vector2i):
		return UIStyle.scaled_window_size(BASE_WINDOW_SIZE, _owner_viewport())
	var requested: Vector2i = value
	var viewport := _owner_viewport()
	if viewport == null:
		return Vector2i(maxi(requested.x, min_size.x), maxi(requested.y, min_size.y))
	var viewport_size := viewport.get_visible_rect().size
	var max_size := Vector2i(
		maxi(min_size.x, int(roundf(viewport_size.x * UIStyle.WINDOW_MAX_VIEWPORT_COVERAGE))),
		maxi(min_size.y, int(roundf(viewport_size.y * UIStyle.WINDOW_MAX_VIEWPORT_COVERAGE)))
	)
	return Vector2i(
		clampi(requested.x, min_size.x, max_size.x),
		clampi(requested.y, min_size.y, max_size.y)
	)

func _restored_window_size(value: Variant) -> Vector2i:
	if not (value is Vector2i):
		return UIStyle.scaled_window_size(BASE_WINDOW_SIZE, _owner_viewport())
	var requested: Vector2i = value
	if requested == BASE_WINDOW_SIZE:
		return UIStyle.scaled_window_size(BASE_WINDOW_SIZE, _owner_viewport())
	return requested

func _clamped_window_position(requested: Vector2i) -> Vector2i:
	var viewport := _owner_viewport()
	if viewport == null:
		return requested
	var viewport_size := Vector2i(viewport.get_visible_rect().size)
	if viewport_size.x <= 0 or viewport_size.y <= 0:
		return requested
	var clamped_size := Vector2i(mini(size.x, viewport_size.x), mini(size.y, viewport_size.y))
	var max_position := Vector2i(
		maxi(0, viewport_size.x - clamped_size.x),
		maxi(0, viewport_size.y - clamped_size.y)
	)
	return Vector2i(
		clampi(requested.x, 0, max_position.x),
		clampi(requested.y, 0, max_position.y)
	)

func _owner_viewport() -> Viewport:
	if get_parent() != null:
		return get_parent().get_viewport()
	return get_viewport()
