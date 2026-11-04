# SPDX-License-Identifier: GPL-2.0-only

## The Roads menu's road-type row (`ROAD-101`): one toggle button per usable built-in type in
## Rust's `get_road_types` order, then one "Custom roads" drop-down listing the player's usable
## custom types, a separator and "Road builder..." (it only emits `road_builder_requested`; the
## builder window connects to it). The drop-down keeps one fixed width however many types the
## library holds, so the row fits the bottom toolbar at 1280 x 720. The road tool's current type
## is marked: its built-in button pressed, or, for a custom type, the drop-down showing its name
## in the pressed style with the item checked. A view: a pick emits `type_picked`, and the host
## activates the tool and calls `RoadTool.set_road_type`. `refresh` rebuilds the row from Rust
## after a library edit; unusable entries are never listed (the builder lists them). Each type's
## tooltip gives its layout, speed, "no sidewalks", "No zoning" and Rust's cost per metre.
## Rust methods called: get_road_types().
extends HBoxContainer

## The player picked road type `id`; `walkway` when it draws a walkway (no car lane).
signal type_picked(id: String, walkway: bool)
## The player asked for the road builder ("Road builder..." in the drop-down).
signal road_builder_requested

const MedianControls = preload("res://scripts/ui/median_controls.gd")
## The drop-down's text while the tool draws a built-in type.
const CUSTOM_ROADS := "Custom roads"
## The drop-down item id of "Road builder..."; custom type `i` is item id `i + 1`.
const BUILDER_ITEM := 0
# Wide enough for "Custom roads" and a short name; a longer name is trimmed with an ellipsis.
const CUSTOM_BUTTON_WIDTH := 150.0

## The drop-down of custom types and "Road builder...".
var custom_menu: MenuButton
## The usable custom type ids, in the drop-down's order.
var custom_ids: Array[String] = []

var _simulation_node: Node
var _group := ButtonGroup.new()
# Built-in type id -> its Button, in row order.
var _buttons: Dictionary = {}
# Type id -> its get_road_types entry, for every listed type.
var _types: Dictionary = {}
var _marked_id := ""

func _init() -> void:
	add_theme_constant_override("separation", 10)
	custom_menu = MenuButton.new()
	custom_menu.text = CUSTOM_ROADS
	custom_menu.flat = false
	custom_menu.clip_text = true
	custom_menu.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	custom_menu.custom_minimum_size.x = CUSTOM_BUTTON_WIDTH
	custom_menu.focus_mode = Control.FOCUS_NONE
	custom_menu.get_popup().id_pressed.connect(_on_custom_item)
	add_child(custom_menu)

## Lists the types of `simulation_node` (the host's SimulationNode).
func setup(simulation_node: Node) -> void:
	_simulation_node = simulation_node
	refresh()

## Rebuilds the row from Rust's current list, keeping the mark on the same type id.
func refresh() -> void:
	for button: Button in _buttons.values():
		remove_child(button)
		button.queue_free()
	_buttons.clear()
	_types.clear()
	custom_ids.clear()
	var popup := custom_menu.get_popup()
	popup.clear()
	if _simulation_node == null:
		return
	for type: Dictionary in _simulation_node.get_road_types().get("types", []):
		if not bool(type.get("usable", false)):
			continue
		var id := String(type.get("id", ""))
		_types[id] = type
		if not bool(type.get("builtin", false)):
			custom_ids.append(id)
			popup.add_radio_check_item(String(type.get("name", id)), custom_ids.size())
			popup.set_item_tooltip(popup.item_count - 1, tooltip(type))
			continue
		var button := Button.new()
		button.text = String(type.get("name", id))
		button.toggle_mode = true
		button.button_group = _group
		button.focus_mode = Control.FOCUS_NONE
		button.tooltip_text = tooltip(type)
		button.pressed.connect(func(): type_picked.emit(id, _is_walkway(type)))
		add_child(button)
		move_child(button, _buttons.size())
		_buttons[id] = button
	if not custom_ids.is_empty():
		popup.add_separator()
	popup.add_item("Road builder...", BUILDER_ITEM)
	popup.set_item_tooltip(popup.item_count - 1, "Build, edit and delete custom road types")
	mark(_marked_id)

## Marks type `id` as the road tool's: its built-in button pressed, or the drop-down showing the
## custom type's name in the pressed style (nothing marked when the row does not list `id`).
func mark(id: String) -> void:
	_marked_id = id
	for type_id: String in _buttons:
		_buttons[type_id].set_pressed_no_signal(type_id == id)
	var custom := custom_ids.find(id)
	var popup := custom_menu.get_popup()
	for index in custom_ids.size():
		popup.set_item_checked(popup.get_item_index(index + 1), index == custom)
	if custom >= 0:
		custom_menu.text = String(_types[id].get("name", id))
		custom_menu.tooltip_text = tooltip(_types[id])
		for state in ["normal", "hover"]:
			custom_menu.add_theme_stylebox_override(state, custom_menu.get_theme_stylebox("pressed"))
		custom_menu.add_theme_color_override("font_color", custom_menu.get_theme_color("font_pressed_color"))
	else:
		custom_menu.text = CUSTOM_ROADS
		custom_menu.tooltip_text = "Your road types and the road builder"
		for state in ["normal", "hover"]:
			custom_menu.remove_theme_stylebox_override(state)
		custom_menu.remove_theme_color_override("font_color")

## Whether the drop-down reads as selected (the tool's type is a custom one).
func custom_marked() -> bool:
	return custom_ids.has(_marked_id)

## The type ids the row lists, built-ins then customs, in order.
func listed_ids() -> Array[String]:
	var ids: Array[String] = []
	ids.assign(_buttons.keys())
	ids.append_array(custom_ids)
	return ids

## The row's button for built-in type `id`, or null (custom types are drop-down items).
func button_for(id: String) -> Button:
	return _buttons.get(id)

## Picks listed type `id` as the player's click would (a built-in button or a drop-down item).
func pick(id: String) -> void:
	if _buttons.has(id):
		_buttons[id].pressed.emit()
	elif custom_ids.has(id):
		custom_menu.get_popup().id_pressed.emit(custom_ids.find(id) + 1)

func _on_custom_item(item_id: int) -> void:
	if item_id == BUILDER_ITEM:
		road_builder_requested.emit()
		return
	var id: String = custom_ids[item_id - 1]
	type_picked.emit(id, _is_walkway(_types[id]))

## The attributes the road tool and an in-place edit send for `type` (`get_road_types` entry),
## `[speed_kmh, sidewalks, zoning, street_lights]`: the type's own, except that a walkway, which
## has neither a speed nor sidewalks of its own, sends `speeds.default_kmh` with sidewalks, as
## every stroke did before road types (its stroke builds the same footpath either way).
static func tool_attributes(type: Dictionary, speeds: Dictionary) -> Array:
	var zoning := bool(type.get("zoning", true))
	var lights := bool(type.get("street_lights", true))
	if _is_walkway(type):
		return [int(speeds.get("default_kmh", 50)), true, zoning, lights]
	return [int(type.get("speed_kmh", 0)), bool(type.get("sidewalks", true)), zoning, lights]

static func _is_walkway(type: Dictionary) -> bool:
	return int(type.get("fwd_lanes", 0)) + int(type.get("bkw_lanes", 0)) == 0

## "2 + 2 lanes, Grass 3.0 m, 100 km/h, no sidewalks, no street lights, 17.0 m wide" (or
## "Footpath, 2.0 m wide") from Rust's figures for a `get_road_types` entry, a
## `check_road_layout` verdict or a road's `get_edge_cross_section`; `with_width` false leaves
## the width out. A footpath has no speed.
static func describe(type: Dictionary, with_width: bool = true) -> String:
	var fwd := int(type.get("fwd_lanes", 0))
	var bkw := int(type.get("bkw_lanes", 0))
	var text := "Footpath" if fwd + bkw == 0 else "%d + %d lanes" % [fwd, bkw]
	var style := int(type.get("median_style", 0))
	if style > 0 and style < MedianControls.MEDIAN_STYLE_NAMES.size():
		text += ", %s %.1f m" % [MedianControls.MEDIAN_STYLE_NAMES[style], float(type.get("median_width", 0.0))]
	if fwd + bkw > 0:
		if int(type.get("speed_kmh", 0)) > 0:
			text += ", %d km/h" % int(type.get("speed_kmh", 0))
		if not bool(type.get("sidewalks", true)):
			text += ", no sidewalks"
	if not bool(type.get("street_lights", true)):
		text += ", no street lights"
	if with_width:
		text += ", %.1f m wide" % float(type.get("total_m", 0.0))
	return text

## Rust's cost of a type per metre on the ground, "15 / m, upkeep 0.10 / m / day", from its
## `build_per_m` and `upkeep_per_m_day` (`get_road_types`, `check_road_layout`).
static func cost_text(type: Dictionary) -> String:
	return "%s / m, upkeep %s / m / day" % [
		_decimals(float(type.get("build_per_m", 0.0)), 2, 0),
		_decimals(float(type.get("upkeep_per_m_day", 0.0)), 3, 2),
	]

# `value` with at most `most` decimals, trailing zeros dropped down to `least`: 15, 19.2, 3.75;
# 0.10, 0.128, 0.025.
static func _decimals(value: float, most: int, least: int) -> String:
	var text := String("%." + str(most) + "f") % value
	var dot := text.find(".")
	var keep := text.length()
	while keep > dot + 1 + least and text[keep - 1] == "0":
		keep -= 1
	if keep == dot + 1:
		keep = dot
	return text.substr(0, keep)

## A type's tooltip: its description, "No zoning" when it builds no lots, and its cost.
static func tooltip(type: Dictionary) -> String:
	var text := describe(type)
	if not bool(type.get("zoning", true)):
		text += "\nNo zoning: nothing is built along it"
	return text + "\n" + cost_text(type)

## A type's line in a picker: "4-Lane Road (50 km/h, 27 / m)", "Walkway (3.75 / m)".
static func picker_text(type: Dictionary) -> String:
	var cost := "%s / m" % _decimals(float(type.get("build_per_m", 0.0)), 2, 0)
	var speed := int(type.get("speed_kmh", 0))
	if speed > 0:
		cost = "%d km/h, %s" % [speed, cost]
	return "%s (%s)" % [String(type.get("name", "")), cost]
