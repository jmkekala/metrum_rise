# SPDX-License-Identifier: GPL-2.0-only

## Asset-editor-only spacing, applied after palette styling in either theme.
## Duplicate style boxes so shared editor/gameplay themes remain unchanged.
extends RefCounted

const CONTENT_PADDING := 12
const CONTROL_GAP := 8
const TEXT_PADDING_Y := 6
const LINE_SPACING := 4

static func pad_container(margin: MarginContainer) -> void:
	for side in ["left", "top", "right", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, CONTENT_PADDING)

static func apply_to_tree(root: Node) -> void:
	_apply(root)
	for child in root.find_children("*", "", true, false):
		_apply(child)

static func _apply(node: Node) -> void:
	if node is BoxContainer:
		node.add_theme_constant_override("separation", CONTROL_GAP)
	elif node is FlowContainer:
		node.add_theme_constant_override("h_separation", CONTROL_GAP)
		node.add_theme_constant_override("v_separation", CONTROL_GAP)
	elif node is Label or node is RichTextLabel:
		node.add_theme_constant_override("line_spacing", LINE_SPACING)
	if node is Button:
		_pad_styles(node, ["normal", "hover", "pressed", "focus", "disabled"])
		if node is OptionButton:
			_apply(node.get_popup())
	elif node is SpinBox:
		_apply(node.get_line_edit())
	elif node is LineEdit:
		_pad_styles(node, ["normal", "focus", "read_only"])
	elif node is TabContainer:
		_pad_styles(node, ["tab_selected", "tab_unselected", "tab_hovered", "tab_disabled"])
	elif node is Tree or node is ItemList:
		_pad_styles(node, ["panel", "focus"])
		node.add_theme_constant_override("v_separation", CONTROL_GAP)
	elif node is RichTextLabel:
		_pad_styles(node, ["normal", "focus"])
	elif node is PopupMenu:
		_pad_styles(node, ["panel"])
		node.add_theme_constant_override("v_separation", CONTROL_GAP)

static func _pad_styles(control: Node, names: Array) -> void:
	for name: String in names:
		var style: StyleBox = control.get_theme_stylebox(name).duplicate()
		style.content_margin_left = CONTENT_PADDING
		style.content_margin_right = CONTENT_PADDING
		style.content_margin_top = TEXT_PADDING_Y
		style.content_margin_bottom = TEXT_PADDING_Y
		control.add_theme_stylebox_override(name, style)
