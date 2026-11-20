# SPDX-License-Identifier: GPL-3.0-or-later

## Building blocks shared by the Options panels: a section heading, a setting row (name at the
## left, its value at the right, a hairline under it, the whole row lit under the mouse) and the
## value controls that go in a row: a ‹ value › cycler and a short slider. A row's hint goes to
## the Options menu (group `options_hint`), which shows it under the list.
extends RefCounted

const UIStyle = preload("res://scripts/ui/ui_style.gd")

## Width of the value column at the right of every row, so values line up down the page.
const VALUE_WIDTH := 240.0
## Height of one setting row.
const ROW_HEIGHT := 40.0
## Group of the node that shows a hovered row's hint; it implements `show_hint(text)`.
const HINT_GROUP := "options_hint"
## Group of the menus that Options and the save browser open from (main menu, pause menu): their
## text fades out under them, so it does not show through the dimmed backdrop beside the rows.
const COVERED_GROUP := "covered_by_options"

## Fades the covered menus to `alpha` (0 when an overlay opens, 1 when it closes).
static func fade_covered(tree: SceneTree, alpha: float) -> void:
	for node in tree.get_nodes_in_group(COVERED_GROUP):
		var item := node as CanvasItem
		item.create_tween().tween_property(item, "modulate:a", alpha, 0.12)

## A full-width 1 px rule between the header, the list and the footer.
static func hairline() -> ColorRect:
	var line := ColorRect.new()
	line.color = Color(1, 1, 1, 0.12)
	line.custom_minimum_size.y = 1
	line.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return line

## Uppercase section title with room above it.
static func heading(text: String) -> Label:
	var label := Label.new()
	label.text = text.to_upper()
	label.custom_minimum_size.y = UIStyle.scaled_px(34)
	label.vertical_alignment = VERTICAL_ALIGNMENT_BOTTOM
	UIStyle.set_font_size(label, 12)
	label.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	return label

## A setting row: `text` at the left, `value` (cycler, slider) in the value column at the right.
static func row(text: String, value: Control, hint: String = "") -> PanelContainer:
	var line := HBoxContainer.new()
	line.add_theme_constant_override("separation", 12)
	var label := Label.new()
	label.text = text
	label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(label, 15)
	label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	line.add_child(label)
	value.custom_minimum_size.x = UIStyle.scaled_px(VALUE_WIDTH)
	value.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	line.add_child(value)
	return frame(line, hint)

## Wraps `content` in the row look: hairline below, lit under the mouse, hint on hover.
static func frame(content: Control, hint: String = "") -> PanelContainer:
	var panel := PanelContainer.new()
	panel.custom_minimum_size.y = UIStyle.scaled_px(ROW_HEIGHT)
	panel.mouse_filter = Control.MOUSE_FILTER_PASS
	var idle := StyleBoxFlat.new()
	idle.bg_color = Color.TRANSPARENT
	idle.border_width_bottom = 1
	idle.border_color = Color(1, 1, 1, 0.07)
	idle.content_margin_left = 12
	idle.content_margin_right = 8
	idle.content_margin_top = 4
	idle.content_margin_bottom = 4
	var lit: StyleBoxFlat = idle.duplicate()
	lit.bg_color = Color(1, 1, 1, 0.05)
	lit.border_width_left = 2
	lit.border_color = UIStyle.HUD_ACCENT
	panel.add_theme_stylebox_override("panel", idle)
	# Godot counts the row's children as inside it, so passing over a value button keeps it lit.
	panel.mouse_entered.connect(func():
		panel.add_theme_stylebox_override("panel", lit)
		panel.get_tree().call_group(HINT_GROUP, "show_hint", hint))
	panel.mouse_exited.connect(func():
		panel.add_theme_stylebox_override("panel", idle)
		panel.get_tree().call_group(HINT_GROUP, "show_hint", ""))
	panel.add_child(content)
	return panel

## A thin slider in the accent colour; the caller puts it beside its value text.
static func slider(min_value: float, max_value: float, step: float) -> HSlider:
	var bar := HSlider.new()
	bar.min_value = min_value
	bar.max_value = max_value
	bar.step = step
	bar.focus_mode = Control.FOCUS_NONE
	bar.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	bar.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	var track := StyleBoxFlat.new()
	track.bg_color = Color(1, 1, 1, 0.14)
	track.content_margin_top = 2
	track.content_margin_bottom = 2
	track.set_corner_radius_all(2)
	var fill: StyleBoxFlat = track.duplicate()
	fill.bg_color = UIStyle.HUD_ACCENT.darkened(0.15)
	bar.add_theme_stylebox_override("slider", track)
	bar.add_theme_stylebox_override("grabber_area", fill)
	bar.add_theme_stylebox_override("grabber_area_highlight", fill)
	return bar

## A value picker for a short fixed list: ‹ VALUE ›. The arrows step back and forth, wrapping;
## clicking the value steps forward. Mirrors the OptionButton calls the panels use (`add_item`,
## `select`, `selected`, `item_selected`) so a panel can swap one for the other.
class Cycler extends HBoxContainer:
	## The player stepped to `index`; not emitted by `select`.
	signal item_selected(index: int)

	## Index of the shown item, -1 while empty.
	var selected := -1
	var _items: PackedStringArray = []
	var _value: Button

	func _init() -> void:
		add_theme_constant_override("separation", 0)
		_arrow("‹", -1)
		_value = _flat("")
		_value.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		_value.alignment = HORIZONTAL_ALIGNMENT_CENTER
		_value.pressed.connect(_step.bind(1))
		add_child(_value)
		_arrow("›", 1)

	## Appends an item; the first one becomes the shown value.
	func add_item(text: String) -> void:
		_items.append(text)
		if selected < 0:
			select(0)

	## Number of items, as OptionButton's `item_count`.
	var item_count: int:
		get:
			return _items.size()

	## Number of items.
	func get_item_count() -> int:
		return _items.size()

	## Shows `index` without emitting `item_selected`; out-of-range indices are ignored.
	func select(index: int) -> void:
		if index < 0 or index >= _items.size():
			return
		selected = index
		_value.text = _items[index].to_upper()

	func _step(direction: int) -> void:
		if _items.size() < 2:
			return
		select(posmod(selected + direction, _items.size()))
		item_selected.emit(selected)

	func _arrow(text: String, direction: int) -> void:
		var arrow := _flat(text)
		UIStyle.set_font_size(arrow, 20)
		arrow.custom_minimum_size.x = UIStyle.scaled_px(30)
		arrow.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
		arrow.add_theme_color_override("font_hover_color", UIStyle.HUD_ACCENT)
		arrow.pressed.connect(_step.bind(direction))
		add_child(arrow)

	func _flat(text: String) -> Button:
		var button := Button.new()
		button.text = text
		button.flat = true
		button.focus_mode = Control.FOCUS_NONE
		button.mouse_filter = Control.MOUSE_FILTER_PASS
		UIStyle.set_font_size(button, 14)
		var empty := StyleBoxEmpty.new()
		for state in ["normal", "hover", "pressed", "focus", "hover_pressed"]:
			button.add_theme_stylebox_override(state, empty)
		button.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
		button.add_theme_color_override("font_hover_color", UIStyle.HUD_ACCENT.lightened(0.3))
		button.add_theme_color_override("font_pressed_color", UIStyle.HUD_ACCENT)
		return button
