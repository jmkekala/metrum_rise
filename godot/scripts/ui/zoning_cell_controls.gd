# SPDX-License-Identifier: GPL-2.0-only

## Workflow, selection shape and erase controls; delegates authoring to the zoning tool.
extends HBoxContainer

signal workflow_changed(cells: bool)
var zoning_tool: Node3D
var _shape: OptionButton
var _erase: Button
var _radius: SpinBox

func _ready() -> void:
	alignment = BoxContainer.ALIGNMENT_CENTER
	add_theme_constant_override("separation", 10)
	var workflow := OptionButton.new()
	workflow.add_item("Cells")
	workflow.add_item("Parcels")
	workflow.custom_minimum_size = Vector2(125, 42)
	workflow.focus_mode = Control.FOCUS_NONE
	workflow.item_selected.connect(func(index: int):
		zoning_tool.set_cell_workflow(index == 0)
		workflow_changed.emit(index == 0))
	add_child(workflow)
	_shape = OptionButton.new()
	for label in ["Cell", "Marquee", "Fill", "Brush"]:
		_shape.add_item(label)
	_shape.custom_minimum_size = Vector2(130, 42)
	_shape.focus_mode = Control.FOCUS_NONE
	_shape.item_selected.connect(func(index: int): zoning_tool.cell_shape = index)
	add_child(_shape)
	_erase = Button.new()
	_erase.text = "Erase"
	_erase.tooltip_text = "Remove zoning with the selected tool"
	_erase.toggle_mode = true
	_erase.focus_mode = Control.FOCUS_NONE
	_erase.custom_minimum_size = Vector2(90, 42)
	_erase.toggled.connect(func(pressed: bool): zoning_tool.cell_erase = pressed)
	add_child(_erase)
	_radius = SpinBox.new()
	_radius.min_value = 5.0
	_radius.max_value = 100.0
	_radius.step = 5.0
	_radius.value = zoning_tool.cell_brush_radius_m
	_radius.suffix = "m"
	_radius.tooltip_text = "Brush radius"
	_radius.custom_minimum_size = Vector2(120, 42)
	_radius.value_changed.connect(func(value: float): zoning_tool.cell_brush_radius_m = value)
	add_child(_radius)

func _process(_delta: float) -> void:
	_shape.visible = zoning_tool.workflow_cells
	_erase.visible = zoning_tool.workflow_cells
	_erase.set_pressed_no_signal(zoning_tool.cell_erase)
	_radius.visible = zoning_tool.workflow_cells and zoning_tool.cell_shape == 3
