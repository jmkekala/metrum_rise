# SPDX-License-Identifier: GPL-3.0-or-later

## The Roads menu's thumbnail grid (`ROAD-101`): one toggle tile per usable built-in type in
## Rust's `get_road_types` order, then one "Custom roads" drop-down listing the player's usable
## custom types, a separator and "Road builder..." (it only emits `road_builder_requested`; the
## builder window connects to it). The drop-down keeps one fixed width however many types the
## library holds, so the row fits the bottom toolbar at 1280 x 720. The road tool's current type
## is marked: its built-in button pressed, or, for a custom type, the drop-down showing its layout
## in the pressed style with the item checked. A view: a pick emits `type_picked`, and the host
## activates the tool and calls `RoadTool.set_road_type`. `refresh` rebuilds the row from Rust
## after a library edit; unusable entries are never listed (the builder lists them). A tile's
## hover card is `RoadTypeCard` (speed sign, symbol chips, cost); the custom roads key shows the
## card briefly on the type it lands on. The drop-down's items keep a text tooltip: layout,
## speed, sidewalk side, "No zoning", the one building side and Rust's cost per metre.
## Rust methods called: get_road_types().
extends HBoxContainer

## The player picked road type `id`; `walkway` when it draws a walkway (no car lane).
signal type_picked(id: String, walkway: bool)
## The player asked for the road builder ("Road builder..." in the drop-down).
signal road_builder_requested

const MedianControls = preload("res://scripts/ui/median_controls.gd")
const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const HudTooltip = preload("res://scripts/ui/hud_tooltip.gd")
const RoadTypeCard = preload("res://scripts/ui/road_type_card.gd")
## The drop-down's text while the tool draws a built-in type.
## Rust's sidewalk side codes (`ROAD-107`), in a road's forward direction: a road type's and a
## road's `sidewalks`, and what the road tool and an in-place edit send.
const SIDEWALKS_NONE := 0
const SIDEWALK_LEFT := 1
const SIDEWALK_RIGHT := 2
const SIDEWALKS_BOTH := 3
const CUSTOM_ROADS := "Custom roads"
## The drop-down item id of "Road builder..."; custom type `i` is item id `i + 1`.
const BUILDER_ITEM := 0
# The custom library stays a single square, irrespective of name length or library size.
const CUSTOM_BUTTON_WIDTH := HudIcons.TILE_SIZE

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
var _grid := GridContainer.new()
var _builder: Button

func _init() -> void:
	add_theme_constant_override("separation", 10)
	_grid.columns = 5
	_grid.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_grid.add_theme_constant_override("h_separation", 6)
	_grid.add_theme_constant_override("v_separation", 6)
	add_child(_grid)
	_builder = HudIcons.icon_button("plus", "New custom road", "", HudIcons.TILE_SIZE)
	_builder.pressed.connect(func(): road_builder_requested.emit())
	add_child(_builder)
	custom_menu = HudIcons.menu_tile(CUSTOM_ROADS, "")
	custom_menu.get_popup().id_pressed.connect(_on_custom_item)
	add_child(custom_menu)

## Lists the types of `simulation_node` (the host's SimulationNode).
func setup(simulation_node: Node) -> void:
	_simulation_node = simulation_node
	refresh()

## Rebuilds the row from Rust's current list, keeping the mark on the same type id.
func refresh() -> void:
	for button: Button in _buttons.values():
		_grid.remove_child(button)
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
		var button := HudIcons.road_tile(type, tooltip(type))
		set_card(button, type)
		button.button_group = _group
		button.pressed.connect(func(): type_picked.emit(id, _is_walkway(type)))
		_grid.add_child(button)
		_buttons[id] = button
	if not custom_ids.is_empty():
		popup.add_separator()
	popup.add_item("Road builder...", BUILDER_ITEM)
	popup.set_item_tooltip(popup.item_count - 1, "Build, edit and delete custom road types")
	mark(_marked_id)

## Marks type `id` as the road tool's: its built-in tile pressed, or the custom tile accented
## with its layout and name on hover (nothing marked when the row does not list `id`).
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
		set_card(custom_menu, _types[id])
		HudIcons.set_road(custom_menu, _types[id])
	else:
		custom_menu.text = CUSTOM_ROADS
		custom_menu.tooltip_text = "Your road types and the road builder"
		set_card(custom_menu, {})
		HudIcons.set_road(custom_menu, {})
	HudIcons.mark(custom_menu, custom >= 0)

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

## The row's tiles in order, for number keys: built-in types, "New custom road", custom roads.
func number_tiles() -> Array[Button]:
	var tiles: Array[Button] = []
	tiles.assign(_buttons.values())
	tiles.append(_builder)
	tiles.append(custom_menu)
	return tiles

## The "New custom road" tile.
func builder_tile() -> Button:
	return _builder

## Picks the custom type after the marked one, wrapping (the first when a built-in is marked);
## with no custom types, opens the drop-down, which offers the road builder.
func pick_next_custom() -> void:
	if custom_ids.is_empty():
		custom_menu.show_popup()
		return
	pick(custom_ids[(custom_ids.find(_marked_id) + 1) % custom_ids.size()])
	# A custom type's art is dense; its card names the type the key landed on. Deferred: the
	# host marks the pick after `type_picked`.
	HudIcons.flash_tooltip.call_deferred(custom_menu)

## Gives `button`'s hover card the road type card of `type`; an empty `type` keeps the plain one.
static func set_card(button: Control, type: Dictionary) -> void:
	if type.is_empty():
		if button.has_meta(HudTooltip.CARD_META):
			button.remove_meta(HudTooltip.CARD_META)
	else:
		button.set_meta(HudTooltip.CARD_META, func(): return RoadTypeCard.build(type, cost_values(type)))

func _on_custom_item(item_id: int) -> void:
	if item_id == BUILDER_ITEM:
		road_builder_requested.emit()
		return
	var id: String = custom_ids[item_id - 1]
	type_picked.emit(id, _is_walkway(_types[id]))

## The attributes the road tool and an in-place edit send for `type` (`get_road_types` entry),
## `[speed_kmh, sidewalks, zoning, street_lights]` (`sidewalks` is Rust's side code): the type's
## own, except that a walkway, which has neither a speed nor sidewalks of its own, sends
## `speeds.default_kmh` with both sidewalks, as every stroke did before road types (its stroke
## builds the same footpath either way).
static func tool_attributes(type: Dictionary, speeds: Dictionary) -> Array:
	var zoning := bool(type.get("zoning", true))
	var lights := bool(type.get("street_lights", true))
	if _is_walkway(type):
		return [int(speeds.get("default_kmh", 50)), SIDEWALKS_BOTH, zoning, lights]
	return [int(type.get("speed_kmh", 0)), int(type.get("sidewalks", SIDEWALKS_BOTH)), zoning, lights]

static func _is_walkway(type: Dictionary) -> bool:
	return int(type.get("fwd_lanes", 0)) + int(type.get("bkw_lanes", 0)) == 0

# Kerbside parking (`PARK-05`) by Rust's code: bit 0 the left kerb, bit 1 the right.
const PARKING_TEXT: Array[String] = ["", ", parking left", ", parking right", ", parking both sides"]

## "2 + 2 lanes, Grass 3.0 m, parking both sides, 100 km/h, no sidewalks, no street lights,
## 17.0 m wide" ("sidewalk on the left" or "... right" for a road with one; or "Footpath, 2.0 m
## wide") from Rust's figures for a `get_road_types` entry, a
## `check_road_layout` verdict or a road's `get_edge_cross_section`; `with_width` false leaves
## the width out, `with_speed` false the speed (shown as its sign instead). A footpath has no speed.
static func describe(type: Dictionary, with_width: bool = true, with_speed: bool = true) -> String:
	var fwd := int(type.get("fwd_lanes", 0))
	var bkw := int(type.get("bkw_lanes", 0))
	var text := "Footpath" if fwd + bkw == 0 else "%d + %d lanes" % [fwd, bkw]
	var style := int(type.get("median_style", 0))
	if style > 0 and style < MedianControls.MEDIAN_STYLE_NAMES.size():
		text += ", %s %.1f m" % [MedianControls.MEDIAN_STYLE_NAMES[style], float(type.get("median_width", 0.0))]
	text += PARKING_TEXT[int(type.get("parking", 0)) & 3]
	var pockets: Dictionary = type.get("pockets", {})
	var turns := ("↰" if bool(pockets.get("left", false)) else "") + ("↱" if bool(pockets.get("right", false)) else "")
	if fwd + bkw > 0 and not turns.is_empty():
		text += ", turn pockets %s %d m" % [turns, int(pockets.get("body_m", 0.0))]
	if fwd + bkw > 0:
		if with_speed and int(type.get("speed_kmh", 0)) > 0:
			text += ", %d km/h" % int(type.get("speed_kmh", 0))
		text += _sidewalk_text(int(type.get("sidewalks", SIDEWALKS_BOTH)))
	if not bool(type.get("street_lights", true)):
		text += ", no street lights"
	if with_width:
		text += ", %.1f m wide" % float(type.get("total_m", 0.0))
	return text

# A car road's sidewalks in a description; both sides, the usual case, go unsaid.
static func _sidewalk_text(sides: int) -> String:
	match sides:
		SIDEWALKS_NONE:
			return ", no sidewalks"
		SIDEWALK_LEFT:
			return ", sidewalk on the left"
		SIDEWALK_RIGHT:
			return ", sidewalk on the right"
	return ""

## Rust's cost of a type per metre on the ground, "15 / m, upkeep 0.10 / m / day", from its
## `build_per_m` and `upkeep_per_m_day` (`get_road_types`, `check_road_layout`).
static func cost_text(type: Dictionary) -> String:
	var values := cost_values(type)
	return "%s / m, upkeep %s / m / day" % [values[0], values[1]]

## Rust's build cost per metre and upkeep per metre and day of `type` (or a layout check), as
## shown: "15" and "0.10".
static func cost_values(type: Dictionary) -> PackedStringArray:
	return PackedStringArray([
		_decimals(float(type.get("build_per_m", 0.0)), 2, 0),
		_decimals(float(type.get("upkeep_per_m_day", 0.0)), 3, 2),
	])

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

## A type's tooltip: its description, "No zoning" when it builds no lots, by its flag or for want
## of a sidewalk (`ROAD-109`), the one side a one-sided road builds on, and its cost.
static func tooltip(type: Dictionary) -> String:
	var text := describe(type)
	var car_road := int(type.get("fwd_lanes", 0)) + int(type.get("bkw_lanes", 0)) > 0
	var sides := int(type.get("sidewalks", SIDEWALKS_BOTH))
	if not bool(type.get("zoning", true)) or (car_road and sides == SIDEWALKS_NONE):
		text += "\nNo zoning: nothing is built along it"
	elif car_road and sides != SIDEWALKS_BOTH:
		text += "\nBuildings on the sidewalk side only"
	return text + "\n" + cost_text(type)

## A type's line in a picker: "4-Lane Road (50 km/h, 27 / m)", "Walkway (3.75 / m)".
static func picker_text(type: Dictionary) -> String:
	var cost := "%s / m" % _decimals(float(type.get("build_per_m", 0.0)), 2, 0)
	var speed := int(type.get("speed_kmh", 0))
	if speed > 0:
		cost = "%d km/h, %s" % [speed, cost]
	return "%s (%s)" % [String(type.get("name", "")), cost]
