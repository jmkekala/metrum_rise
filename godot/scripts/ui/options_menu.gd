# SPDX-License-Identifier: GPL-2.0-only

## Options, drawn in the game's own UI over a dimmed screen (main menu and gameplay): a Display
## tab (graphics and interface scale) and a Mods tab (installed packs), as one centred column of
## setting rows with no card around it. The panels own their pending values and persistence;
## this shell owns the tabs, the hovered row's hint and the Reset / Cancel / Apply footer. Esc
## and Cancel discard pending edits; it takes the keyboard while open.
extends CanvasLayer

const AccessibilityOptions = preload("res://scripts/ui/accessibility_options.gd")
const GameSettings = preload("res://scripts/core/game_settings.gd")
const GraphicsOptions = preload("res://scripts/ui/graphics_options.gd")
const OptionsRows = preload("res://scripts/ui/options_rows.gd")
const PackManager = preload("res://scripts/ui/pack_manager.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")

## Opened from the main menu.
const CONTEXT_MAIN_MENU := "main_menu"
## Opened in game (File menu or the pause menu).
const CONTEXT_GAMEPLAY := "gameplay"
const TAB_DISPLAY := "display"
const TAB_MODS := "mods"
# The column's design width; it shrinks to 92 % of a narrower screen.
const COLUMN_WIDTH := 760.0

## The menu closed (Cancel, Esc or the close button).
signal closed

var context: String = CONTEXT_GAMEPLAY

# Tab id -> its button, its page and the option panels on it.
var _tab_buttons: Dictionary = {}
var _pages: Dictionary = {}
var _panels: Dictionary = {}
var _active_tab := TAB_DISPLAY
var _column: VBoxContainer
var _tab_marks: Dictionary = {}
var _hint: Label
var _apply_btn: Button
var _reset_btn: Button
var _status_label: Label

func _ready() -> void:
	# Above the pause menu; option drop-downs are subwindows and still draw over it.
	layer = 120
	visible = false
	_build_ui()
	var saved := str(GameSettings.load_options_window_state().get("active_category", TAB_DISPLAY))
	_select_tab(TAB_MODS if saved == TAB_MODS else TAB_DISPLAY)

## Shows the menu with every panel re-read from the saved settings.
func popup_options(next_context: String) -> void:
	context = next_context
	_discard_pending_changes()
	for panel in _all_panels():
		if panel is PackManager:
			panel.in_game = context == CONTEXT_GAMEPLAY
	var screen := get_viewport().get_visible_rect().size
	_column.custom_minimum_size.x = minf(UIStyle.scaled_px(COLUMN_WIDTH), screen.x * 0.92)
	show_hint("")
	OptionsRows.fade_covered(get_tree(), 0.0)
	show()

## Shows a hovered setting's description under the list (OptionsRows calls it).
func show_hint(text: String) -> void:
	if _hint:
		_hint.text = text

## Whether the menu is up.
func is_open() -> bool:
	return visible

func _input(event: InputEvent) -> void:
	if visible and UIStyle.is_mouse_back(event):
		_on_cancel_pressed()
		get_viewport().set_input_as_handled()
		return
	if not visible or not (event is InputEventKey) or not event.pressed:
		return
	if event.keycode == KEY_ESCAPE and not event.echo:
		_on_cancel_pressed()
	# Nothing behind the menu reacts to keys while it is open.
	get_viewport().set_input_as_handled()

func _build_ui() -> void:
	add_to_group(OptionsRows.HINT_GROUP)
	var backdrop := ColorRect.new()
	backdrop.color = Color(0.02, 0.03, 0.04, 0.86)
	backdrop.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	backdrop.mouse_filter = Control.MOUSE_FILTER_STOP
	add_child(backdrop)
	# The column runs from below the top edge to above the bottom edge, centred across.
	var frame := MarginContainer.new()
	frame.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	frame.mouse_filter = Control.MOUSE_FILTER_IGNORE
	frame.add_theme_constant_override("margin_top", int(UIStyle.scaled_px(72)))
	frame.add_theme_constant_override("margin_bottom", int(UIStyle.scaled_px(48)))
	# Inside the backdrop: a click a row passes up ends at the backdrop instead of reaching
	# the city as unhandled input.
	backdrop.add_child(frame)
	_column = VBoxContainer.new()
	_column.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	_column.add_theme_constant_override("separation", 0)
	frame.add_child(_column)

	var title := Label.new()
	title.text = "OPTIONS"
	UIStyle.set_font_size(title, 30)
	title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	_column.add_child(title)
	var tabs := HBoxContainer.new()
	tabs.add_theme_constant_override("separation", int(UIStyle.scaled_px(28)))
	_column.add_child(tabs)
	_column.add_child(OptionsRows.hairline())

	var pages := MarginContainer.new()
	pages.size_flags_vertical = Control.SIZE_EXPAND_FILL
	pages.add_theme_constant_override("margin_top", 6)
	_column.add_child(pages)
	_add_tab(tabs, pages, TAB_DISPLAY, "Display", [GraphicsOptions.new(), AccessibilityOptions.new()])
	_add_tab(tabs, pages, TAB_MODS, "Mods", [PackManager.new()])

	_hint = Label.new()
	_hint.custom_minimum_size.y = UIStyle.scaled_px(40)
	_hint.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	_hint.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	UIStyle.set_font_size(_hint, 13)
	_hint.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	_column.add_child(_hint)
	_column.add_child(OptionsRows.hairline())

	var footer := HBoxContainer.new()
	footer.add_theme_constant_override("separation", int(UIStyle.scaled_px(24)))
	_column.add_child(footer)
	_reset_btn = _footer_button(footer, "Reset Defaults", _on_reset_defaults_pressed)
	_status_label = Label.new()
	_status_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_status_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	_status_label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(_status_label, 13)
	_status_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	footer.add_child(_status_label)
	_footer_button(footer, "Cancel", _on_cancel_pressed)
	_apply_btn = _footer_button(footer, "Apply", _on_apply_pressed)
	_apply_btn.add_theme_color_override("font_color", UIStyle.HUD_ACCENT)
	_apply_btn.add_theme_color_override("font_hover_color", UIStyle.HUD_ACCENT.lightened(0.4))

# One tab: its text button (underlined while active) and a scrolling page holding `panels`.
func _add_tab(tabs: HBoxContainer, pages: Control, id: String, label: String, panels: Array) -> void:
	var tab := VBoxContainer.new()
	tab.add_theme_constant_override("separation", 2)
	tabs.add_child(tab)
	var button := UIStyle.menu_text_button(label, 15, false)
	button.toggle_mode = true
	button.custom_minimum_size.y = UIStyle.scaled_px(40)
	button.pressed.connect(func(): _select_tab(id))
	tab.add_child(button)
	var mark := ColorRect.new()
	mark.color = UIStyle.HUD_ACCENT
	mark.custom_minimum_size.y = 2
	mark.mouse_filter = Control.MOUSE_FILTER_IGNORE
	tab.add_child(mark)
	_tab_buttons[id] = button
	_tab_marks[id] = mark
	var scroll := ScrollContainer.new()
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	pages.add_child(scroll)
	var column := VBoxContainer.new()
	column.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	column.add_theme_constant_override("separation", int(UIStyle.scaled_px(10)))
	scroll.add_child(column)
	for panel: Control in panels:
		panel.size_flags_vertical = Control.SIZE_SHRINK_BEGIN
		panel.dirty_changed.connect(func(_dirty: bool): _sync_footer_state())
		column.add_child(panel)
	_pages[id] = scroll
	_panels[id] = panels

func _footer_button(footer: HBoxContainer, label: String, action: Callable) -> Button:
	var button := UIStyle.menu_text_button(label, 14, false)
	button.custom_minimum_size.y = UIStyle.scaled_px(48)
	button.pressed.connect(action)
	footer.add_child(button)
	return button

func _select_tab(id: String) -> void:
	_active_tab = id
	for key in _pages:
		(_pages[key] as Control).visible = key == id
		(_tab_buttons[key] as Button).set_pressed_no_signal(key == id)
		(_tab_marks[key] as ColorRect).modulate.a = 1.0 if key == id else 0.0
	GameSettings.save_options_category(id)
	_sync_footer_state()

func _all_panels() -> Array:
	var all := []
	for key in _panels:
		all.append_array(_panels[key])
	return all

func _on_reset_defaults_pressed() -> void:
	for panel in _panels[_active_tab]:
		if panel.has_method("reset_defaults"):
			panel.reset_defaults()
	_sync_footer_state()

func _on_cancel_pressed() -> void:
	_discard_pending_changes()
	hide()
	OptionsRows.fade_covered(get_tree(), 1.0)
	closed.emit()

func _on_apply_pressed() -> void:
	var restart_required := false
	for panel in _all_panels():
		if not panel.has_pending_changes():
			continue
		if panel.apply_changes() != OK:
			_status_label.text = "Could not apply settings."
			_sync_footer_state()
			return
		if panel is PackManager:
			restart_required = true
	_status_label.text = "Saved. Restart to load the pack changes." if restart_required else "Saved."
	_sync_footer_state()

func _discard_pending_changes() -> void:
	for panel in _all_panels():
		panel.refresh()
	if _status_label:
		_status_label.text = ""
	_sync_footer_state()

func _sync_footer_state() -> void:
	if not _apply_btn:
		return
	var dirty := false
	for panel in _all_panels():
		dirty = dirty or panel.has_pending_changes()
	_apply_btn.disabled = not dirty
	var resettable := false
	for panel in _panels.get(_active_tab, []):
		resettable = resettable or panel.has_method("reset_defaults")
	_reset_btn.disabled = not resettable
	if dirty:
		_status_label.text = "Unsaved changes."
	elif _status_label.text == "Unsaved changes.":
		_status_label.text = ""
