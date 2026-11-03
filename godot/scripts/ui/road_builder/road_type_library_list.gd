# SPDX-License-Identifier: GPL-2.0-only

## The road builder's library column (`ROAD-101`): the library error on top when the file cannot
## be written (it does not parse, or a newer game wrote it), then every road type from Rust's
## `get_road_types`, the built-ins first and the player's below, the unusable ones greyed with
## their reason, and New, Duplicate and Delete under the list. Built-ins can only be duplicated;
## the walkway not even that (the builder makes car roads). A view: selections and buttons are
## reported through its signals; the builder decides and calls Rust.
extends VBoxContainer

## The player selected a type (a `get_road_types` entry).
signal type_selected(type: Dictionary)
## The player asked for a new layout.
signal new_requested
## The player asked to start a new type from `type`.
signal duplicate_requested(type: Dictionary)
## The player asked to delete custom type `type` (the builder confirms first).
signal delete_requested(type: Dictionary)

const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const RoadTypeMenu = preload("res://scripts/ui/road_type_menu.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const HEADER := Color(0.65, 0.65, 0.90)
const UNUSABLE := Color(0.55, 0.55, 0.55)
## The walkway built-in, which the builder cannot make.
const WALKWAY_ID := "builtin:walkway"

var error_label: Label
var list: ItemList
var new_button: Button
var duplicate_button: Button
var delete_button: Button

# Item index -> its type ({} for a section header).
var _items: Array[Dictionary] = []
var _writable := true

func _init() -> void:
	add_theme_constant_override("separation", 6)
	custom_minimum_size.x = 210.0
	error_label = Label.new()
	error_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	error_label.add_theme_color_override("font_color", UIStyle.TEXT_ALERT)
	UIStyle.set_font_size(error_label, 12)
	error_label.visible = false
	add_child(error_label)
	list = ItemList.new()
	list.size_flags_vertical = Control.SIZE_EXPAND_FILL
	list.select_mode = ItemList.SELECT_SINGLE
	list.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	UIStyle.set_font_size(list, 13)
	list.item_selected.connect(_on_item_selected)
	add_child(list)
	var buttons := HBoxContainer.new()
	add_child(buttons)
	new_button = _button(buttons, "New", "Start a new road type from a 2-lane road")
	new_button.pressed.connect(func(): new_requested.emit())
	duplicate_button = _button(buttons, "Duplicate", "Start a new road type from the selected one")
	duplicate_button.pressed.connect(func(): duplicate_requested.emit(selected_type()))
	delete_button = _button(buttons, "Delete", "Delete the selected road type; roads already built keep their layout")
	delete_button.pressed.connect(func(): delete_requested.emit(selected_type()))
	_update_buttons()

func _button(row: HBoxContainer, label: String, tooltip: String) -> Button:
	var button := Button.new()
	button.text = label
	button.tooltip_text = tooltip
	button.focus_mode = Control.FOCUS_NONE
	button.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(button, 13)
	row.add_child(button)
	return button

## Lists `library` (Rust's `get_road_types` result) and selects type `selected_id` if listed.
func show_library(library: Dictionary, selected_id: String) -> void:
	var error := String(library.get("error", ""))
	_writable = error.is_empty()
	error_label.visible = not _writable
	error_label.text = "" if _writable else RoadReasonText.text({"invalid_reason": error}) + \
		("\n(%s)" % library.get("error_detail", "") if not String(library.get("error_detail", "")).is_empty() else "")
	list.clear()
	_items.clear()
	var section := ""
	for type: Dictionary in library.get("types", []):
		var builtin := bool(type.get("builtin", false))
		var wanted := "Built-in" if builtin else "Your road types"
		if wanted != section:
			section = wanted
			_add_header(section)
		_add_type(type)
	if section != "Your road types":
		_add_header("Your road types")
		var hint := list.add_item("  None yet", null, false)
		list.set_item_custom_fg_color(hint, UNUSABLE)
		_items.append({})
	select_id(selected_id)

func _add_header(text: String) -> void:
	var index := list.add_item(text, null, false)
	list.set_item_custom_fg_color(index, HEADER)
	_items.append({})

func _add_type(type: Dictionary) -> void:
	var type_name := String(type.get("name", ""))
	var index: int
	if bool(type.get("usable", false)):
		index = list.add_item("  " + type_name)
		list.set_item_tooltip(index, RoadTypeMenu.describe(type))
	else:
		var reason := RoadReasonText.text(type, "This road type cannot be used")
		index = list.add_item("  %s: %s" % [type_name if not type_name.is_empty() else "(no name)", reason])
		list.set_item_custom_fg_color(index, UNUSABLE)
		list.set_item_tooltip(index, "Unusable: %s\nSelect it to fix it, or delete it." % reason)
	_items.append(type)

## Selects type `id` in the list (none when it is not listed), without emitting.
func select_id(id: String) -> void:
	list.deselect_all()
	for index in _items.size():
		if not id.is_empty() and String(_items[index].get("id", "")) == id:
			list.select(index)
			list.ensure_current_is_visible()
	_update_buttons()

## The selected type, or {} when none is.
func selected_type() -> Dictionary:
	var selected := list.get_selected_items()
	return _items[selected[0]] if not selected.is_empty() else {}

## The list row of type `id`, or -1.
func index_of(id: String) -> int:
	for index in _items.size():
		if String(_items[index].get("id", "")) == id:
			return index
	return -1

## Selects type `id` as the player's click would (emits `type_selected`).
func pick(id: String) -> void:
	var index := index_of(id)
	if index >= 0:
		list.select(index)
		list.item_selected.emit(index)

func _on_item_selected(index: int) -> void:
	_update_buttons()
	if not _items[index].is_empty():
		type_selected.emit(_items[index])

func _update_buttons() -> void:
	var type := selected_type()
	var builtin := bool(type.get("builtin", false))
	duplicate_button.disabled = type.is_empty() or String(type.get("id", "")) == WALKWAY_ID or type.get("parts", []).is_empty()
	delete_button.disabled = type.is_empty() or builtin or not _writable
	duplicate_button.tooltip_text = "The walkway is not built in the road builder" \
		if String(type.get("id", "")) == WALKWAY_ID else "Start a new road type from the selected one"
