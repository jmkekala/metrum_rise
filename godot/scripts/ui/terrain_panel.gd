# SPDX-License-Identifier: GPL-2.0-only

## The Terrain palette, laid out like Services: one row of category tiles, [Raise and lower |
## Trees, Shrubs, Flowers, Hedges, Rocks | Remove], and beside it the open category's planting
## options as picture tiles: the game's meshes planted the way Rust's preset plants them. A
## category tile shows one of its options' pictures. Names, and what a mix plants, are in the
## tooltips.
##
## The pictures are files, `assets/ui/vegetation/<name>.png`, baked offline by
## `tests/bake_vegetation_pictures.gd` from Rust's preset samples and the game's meshes; the
## menu only loads them. An option without its file keeps the brush icon, logged on the
## `events` debug channel.
##
## Number keys 1-7 press the row's tiles in order (`number_slots`); a category's digit while it is
## open steps to its next option.
##
## A view with the tools' wiring: a click selects the vegetation tool's option and mode, or the
## sculpt tool, through the input manager; the tool's own changes (Shift + wheel cycling, E for
## Remove, Esc) come back through its signals and the input manager's `tool_changed`, so the
## marks follow every path. The vegetation option list, its order and categories are the tool's
## `BRUSH_OPTIONS`; Rust decides what each preset plants.
extends VBoxContainer

const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const InputManager = preload("res://scripts/core/input_manager.gd")
const VegetationTool = preload("res://scripts/tools/vegetation_tool.gd")
## Where the baked pictures live; see `picture_path`.
const PICTURE_DIR := "res://assets/ui/vegetation/"

## Categories in menu order: id, title, and the option whose picture is the category's tile.
## Each lists the options whose "group" is its id.
const GROUPS := [["trees", "Trees", "Scattered trees"], ["shrubs", "Shrubs", "Lilac"],
	["flowers", "Flowers", "Lupine"], ["hedges", "Hedges", "Hedge, medium"], ["rocks", "Rocks", "Rock"]]
const CATEGORY_TILE := 64.0

var _input_manager: Node
var _tool: Node
var _sculpt_button: Button
var _remove_button: Button
var _group_buttons: Dictionary = {}
var _option_buttons: Dictionary = {}
var _options_panel: Control
var _option_row: HBoxContainer
var _open_group := ""

## Builds the palette for `input_manager`'s sculpt and vegetation tools.
func build(input_manager: Node) -> void:
	_input_manager = input_manager
	_tool = input_manager.vegetation_tool
	size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	var palette := HBoxContainer.new()
	palette.add_theme_constant_override("separation", 6)
	var shell := HudIcons.shell(palette)
	shell.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	add_child(shell)

	var categories := HBoxContainer.new()
	categories.add_theme_constant_override("separation", 6)
	palette.add_child(categories)
	_sculpt_button = HudIcons.icon_button("sculpt", "Raise and lower", "Raise or lower the ground", CATEGORY_TILE)
	_sculpt_button.toggle_mode = true
	_sculpt_button.pressed.connect(_on_sculpt_pressed)
	categories.add_child(_sculpt_button)
	categories.add_child(VSeparator.new())
	for group in GROUPS:
		var id: String = group[0]
		var button := HudIcons.icon_button("brush", group[1], "", CATEGORY_TILE)
		button.toggle_mode = true
		_show_picture(button, VegetationTool.BRUSH_OPTIONS.map(func(o): return o.label).find(group[2]))
		button.pressed.connect(func(): _on_group_pressed(id))
		categories.add_child(button)
		_group_buttons[id] = button
	categories.add_child(VSeparator.new())
	_remove_button = HudIcons.icon_button("uproot", "Remove", "Remove trees, plants and rocks", CATEGORY_TILE)
	HudIcons.set_shortcut(_remove_button, "E")
	HudIcons.tint(_remove_button, UIStyle.TEXT_ALERT)
	HudIcons.set_accent(_remove_button, UIStyle.TEXT_ALERT)
	_remove_button.toggle_mode = true
	_remove_button.pressed.connect(_on_remove_pressed)
	categories.add_child(_remove_button)

	# The open category's options, beside the categories in the same shell.
	var options := HBoxContainer.new()
	options.add_theme_constant_override("separation", 6)
	options.add_child(VSeparator.new())
	_option_row = HBoxContainer.new()
	_option_row.add_theme_constant_override("separation", 6)
	options.add_child(_option_row)
	_options_panel = options
	_options_panel.hide()
	palette.add_child(_options_panel)


	_tool.option_changed.connect(func(_index: int): _follow_option())
	_tool.mode_changed.connect(func(_mode: int): refresh_marks())
	input_manager.tool_changed.connect(func(_tool_id: int): refresh_marks())
	visibility_changed.connect(_on_visibility_changed)

## Marks the tiles for the active tool, mode and option.
func refresh_marks() -> void:
	var current: int = _input_manager.current_tool
	var planting: bool = current == InputManager.Tool.VEGETATION and _tool.mode == VegetationTool.Mode.PLANT
	HudIcons.mark(_sculpt_button, current == InputManager.Tool.SCULPT)
	HudIcons.mark(_remove_button, current == InputManager.Tool.VEGETATION and _tool.mode == VegetationTool.Mode.REMOVE)
	for id in _group_buttons:
		HudIcons.mark(_group_buttons[id], _options_panel.visible and id == _open_group)
	for index in _option_buttons:
		HudIcons.mark(_option_buttons[index], planting and index == _tool.option_index)

## The options of category `id` in option order, as indices into `BRUSH_OPTIONS`.
static func group_options(id: String) -> PackedInt32Array:
	var indices := PackedInt32Array()
	for i in VegetationTool.BRUSH_OPTIONS.size():
		if VegetationTool.BRUSH_OPTIONS[i].get("group", "") == id:
			indices.append(i)
	return indices

## [tile, action] per number key from 1: Raise and lower, the categories, Remove. Each action
## plays its tile's click.
func number_slots() -> Array:
	var slots: Array = [[_sculpt_button, _on_sculpt_pressed]]
	for group in GROUPS:
		slots.append([_group_buttons[group[0]], _number_group.bind(group[0])])
	slots.append([_remove_button, _on_remove_pressed])
	for slot in slots:
		var tile: Button = slot[0]
		var action: Callable = slot[1]
		slot[1] = func():
			action.call()
			HudIcons.pulse(tile)
	return slots

# A category's digit opens it; while it is open it plants (leaving Remove), and while it is
# already planting it steps to its next option. A click on the open category's tile closes it.
func _number_group(id: String) -> void:
	if not (_options_panel.visible and _open_group == id):
		_on_group_pressed(id)
		return
	if _input_manager.current_tool == InputManager.Tool.VEGETATION and _tool.mode == VegetationTool.Mode.PLANT:
		var options := group_options(id)
		_tool.option_index = options[(options.find(_tool.option_index) + 1) % options.size()]
	_plant()

func _on_sculpt_pressed() -> void:
	_close_group()
	_input_manager._toggle_tool(InputManager.Tool.SCULPT)
	refresh_marks()

# A category opens with its option planting: the tool's own option when it is in the category,
# else the category's first. Its tile again closes it.
func _on_group_pressed(id: String) -> void:
	if _options_panel.visible and _open_group == id:
		_close_group()
		if _input_manager.current_tool == InputManager.Tool.VEGETATION:
			_input_manager._cancel_active_tool()
		refresh_marks()
		return
	var options := group_options(id)
	if options.is_empty():
		return
	_open(id)
	if not options.has(_tool.option_index):
		_tool.option_index = options[0]
	_plant()

func _on_option_pressed(index: int) -> void:
	_tool.option_index = index
	_plant()

func _on_remove_pressed() -> void:
	var removing: bool = _input_manager.current_tool == InputManager.Tool.VEGETATION \
		and _tool.mode == VegetationTool.Mode.REMOVE
	if removing:
		_tool.mode = VegetationTool.Mode.PLANT
	else:
		_activate(VegetationTool.Mode.REMOVE)
	refresh_marks()

func _plant() -> void:
	_activate(VegetationTool.Mode.PLANT)
	refresh_marks()

func _activate(mode: int) -> void:
	if _input_manager.current_tool != InputManager.Tool.VEGETATION:
		_input_manager._toggle_tool(InputManager.Tool.VEGETATION)
	_tool.mode = mode

# Shift + wheel steps through every option; the menu shows the category it lands in.
func _follow_option() -> void:
	var group: String = VegetationTool.BRUSH_OPTIONS[_tool.option_index].get("group", "")
	if visible and group != _open_group:
		_open(group)
	refresh_marks()

func _open(id: String) -> void:
	_open_group = id
	for child in _option_row.get_children():
		_option_row.remove_child(child)
		child.queue_free()
	_option_buttons.clear()
	for index in group_options(id):
		var option: Dictionary = VegetationTool.BRUSH_OPTIONS[index]
		var button := HudIcons.icon_button("brush", option.label, option.get("detail", ""), HudIcons.TILE_SIZE)
		button.toggle_mode = true
		_show_picture(button, index)
		button.pressed.connect(func(): _on_option_pressed(index))
		_option_row.add_child(button)
		_option_buttons[index] = button
	_options_panel.show()

func _close_group() -> void:
	_open_group = ""
	_options_panel.hide()

## The baked picture of option `index`.
static func picture_path(index: int) -> String:
	var label: String = VegetationTool.BRUSH_OPTIONS[index].label
	return PICTURE_DIR + label.to_lower().replace(",", "").replace(" ", "_") + ".png"

# Puts option `index`'s picture on `button`; a missing file keeps the brush icon and says so.
func _show_picture(button: Button, index: int) -> void:
	var path := picture_path(index) if index >= 0 else ""
	if index < 0 or not ResourceLoader.exists(path):
		MetrumDebug.log("events", "terrain menu: no picture %s" % path)
		return
	HudIcons.set_picture(button, load(path) as Texture2D)

# Closing leaves Remove, so the next visit plants.
func _on_visibility_changed() -> void:
	if visible:
		if _input_manager.current_tool == InputManager.Tool.VEGETATION:
			_follow_option()
		refresh_marks()
	else:
		_tool.mode = VegetationTool.Mode.PLANT
		_close_group()
