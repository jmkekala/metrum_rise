# SPDX-License-Identifier: GPL-2.0-only

## Segmented icon choice; the selected index and item_selected signal mirror a native picker.
extends HBoxContainer

const HudIcons = preload("res://scripts/ui/hud_icons.gd")
signal item_selected(index: int)
var _buttons: Array[Button] = []

func add_item(icon: String, title: String, detail: String) -> void:
	var index := _buttons.size()
	var button := HudIcons.icon_button(icon, title, detail)
	button.toggle_mode = true
	button.pressed.connect(func():
		select(index)
		item_selected.emit(index))
	add_child(button)
	_buttons.append(button)

func select(index: int) -> void:
	for i in range(_buttons.size()):
		HudIcons.mark(_buttons[i], i == index)

## The tile of item `index`.
func button(index: int) -> Button:
	return _buttons[index]
