# SPDX-License-Identifier: GPL-2.0-only

## In-game pause menu: Esc with nothing left to cancel opens it over a dimmed screen. A view:
## each button emits `action_requested` with a TopMenu file action id (or RESUME), and TopMenu
## runs it. Leaving the game (main menu, quit) asks first, as nothing tracks unsaved changes.
extends CanvasLayer

const OptionsRows = preload("res://scripts/ui/options_rows.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
## Closes the menu; not a TopMenu action.
const RESUME := -1

## The player picked `id`: RESUME or a TopMenu.ActionId file action.
signal action_requested(id: int)

var _confirm: ConfirmationDialog
var _pending_id := RESUME

## Builds the menu, hidden. `items` lists [label, id, confirm title] in order; a non-empty
## confirm title asks before emitting (leaving the game).
func build(items: Array) -> void:
	# Above the HUD and the menu bar; dialogs are subwindows and still draw over it.
	layer = 110
	visible = false
	var backdrop := ColorRect.new()
	backdrop.color = Color(0.02, 0.03, 0.04, 0.72)
	backdrop.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	backdrop.mouse_filter = Control.MOUSE_FILTER_STOP
	add_child(backdrop)
	# Plain text entries down the left side, like the main menu.
	var frame := MarginContainer.new()
	frame.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	frame.mouse_filter = Control.MOUSE_FILTER_IGNORE
	frame.add_theme_constant_override("margin_left", int(UIStyle.scaled_px(96)))
	frame.add_to_group(OptionsRows.COVERED_GROUP)
	# Inside the backdrop: a click a row passes up ends at the backdrop instead of reaching
	# the city as unhandled input.
	backdrop.add_child(frame)
	var column := VBoxContainer.new()
	column.size_flags_horizontal = Control.SIZE_SHRINK_BEGIN
	column.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	column.add_theme_constant_override("separation", int(UIStyle.scaled_px(4)))
	frame.add_child(column)
	var inset := MarginContainer.new()
	inset.add_theme_constant_override("margin_left", 14)
	inset.add_theme_constant_override("margin_bottom", int(UIStyle.scaled_px(24)))
	column.add_child(inset)
	var title := Label.new()
	title.text = "PAUSED"
	UIStyle.set_font_size(title, 36)
	title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	inset.add_child(title)
	for item in items:
		var button := UIStyle.menu_text_button(str(item[0]), 20)
		button.custom_minimum_size.y = UIStyle.scaled_px(44)
		var id := int(item[1])
		var ask := str(item[2])
		button.pressed.connect(func():
			if ask.is_empty():
				action_requested.emit(id)
			else:
				_ask(id, ask))
		column.add_child(button)
	_confirm = ConfirmationDialog.new()
	_confirm.dialog_text = "Unsaved progress will be lost."
	_confirm.confirmed.connect(func(): action_requested.emit(_pending_id))
	# The open confirmation takes the mouse: the back button dismisses it there.
	_confirm.window_input.connect(func(event: InputEvent):
		if UIStyle.is_mouse_back(event):
			_confirm.set_input_as_handled()
			_confirm.hide())
	add_child(_confirm)

# The mouse's back button resumes. Options and the save browser open over the menu later in the
# tree, so they take it first; the leave confirmation takes it in its own window.
func _input(event: InputEvent) -> void:
	if not visible or _confirm.visible or not UIStyle.is_mouse_back(event):
		return
	get_viewport().set_input_as_handled()
	action_requested.emit(RESUME)

func _ask(id: int, title: String) -> void:
	_pending_id = id
	_confirm.title = title
	_confirm.ok_button_text = title
	_confirm.popup_centered()
