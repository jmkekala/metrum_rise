# SPDX-License-Identifier: GPL-2.0-only

## Event-driven workflow, shape and erase controls; all authoring stays in the zoning tool.
extends HBoxContainer

const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const IconChoice = preload("res://scripts/ui/hud_icon_choice.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
signal workflow_changed(cells: bool)
var zoning_tool: Node3D
var _workflow: IconChoice
var _shape: IconChoice
var _erase: Button
var _radius: SpinBox

func _ready() -> void:
	alignment = BoxContainer.ALIGNMENT_CENTER
	add_theme_constant_override("separation", 6)
	_workflow = IconChoice.new()
	_workflow.add_item("cells", "Cells", "")
	_workflow.add_item("parcel", "Parcels", "")
	_workflow.item_selected.connect(func(index: int): zoning_tool.set_cell_workflow(index == 0))
	add_child(_workflow)
	add_child(VSeparator.new())
	_shape = IconChoice.new()
	for label in ["Cell", "Marquee", "Fill", "Brush"]:
		_shape.add_item(label.to_lower(), label, "")
	_shape.item_selected.connect(func(index: int): zoning_tool.cell_shape = index)
	add_child(_shape)
	_erase = HudIcons.icon_button("erase", "Erase", "")
	_erase.toggle_mode = true
	HudIcons.set_accent(_erase, UIStyle.SIGN_RED)
	_erase.toggled.connect(func(pressed: bool): zoning_tool.cell_erase = pressed)
	add_child(_erase)
	_radius = SpinBox.new()
	_radius.min_value = 5.0
	_radius.max_value = 100.0
	_radius.step = 5.0
	_radius.value = zoning_tool.cell_brush_radius_m
	_radius.suffix = "m"
	_radius.tooltip_text = "Brush radius"
	_radius.custom_minimum_size.x = 100
	UIStyle.set_font_size(_radius, 13)
	_radius.value_changed.connect(func(value: float): zoning_tool.cell_brush_radius_m = value)
	add_child(_radius)
	zoning_tool.controls_changed.connect(_sync)
	_sync()

## Number-key tiles in row order: the workflow it would switch to, the four shapes, erase.
func number_tiles() -> Array[Button]:
	var tiles: Array[Button] = [_workflow.button(1 if zoning_tool.workflow_cells else 0)]
	for index in range(4):
		tiles.append(_shape.button(index))
	tiles.append(_erase)
	return tiles

func _sync() -> void:
	_workflow.select(0 if zoning_tool.workflow_cells else 1)
	_shape.select(zoning_tool.cell_shape)
	_shape.visible = zoning_tool.workflow_cells
	_erase.visible = zoning_tool.workflow_cells
	HudIcons.mark(_erase, zoning_tool.cell_erase)
	_radius.visible = zoning_tool.workflow_cells and zoning_tool.cell_shape == 3
	workflow_changed.emit(zoning_tool.workflow_cells)
