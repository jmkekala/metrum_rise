# SPDX-License-Identifier: GPL-2.0-only

## The Roads menu's road-type row (`ROAD-101`): one toggle button per usable road type, the
## built-ins then the player's custom types in Rust's `get_road_types` order, with the road
## tool's current type marked (one ButtonGroup). At most MAX_TYPES buttons fit the bottom
## toolbar; the row ends with "Road builder...", which only emits `road_builder_requested` (the
## builder window connects to it) and names how many usable types the cap left out. A view: a
## pick emits `type_picked`, and the host activates the tool and calls `RoadTool.set_road_type`.
## `refresh` rebuilds the row from Rust after a library edit; unusable entries are never listed.
## Rust methods called: get_road_types().
extends HBoxContainer

## The player picked road type `id`; `walkway` when it draws a walkway (no car lane).
signal type_picked(id: String, walkway: bool)
## The player asked for the road builder ("Road builder..." button).
signal road_builder_requested

## The most type buttons the row holds; the builder lists the rest.
const MAX_TYPES := 10

## Median style names by Rust's style code (0 none, 1 painted, 2 kerbed grass, 3 kerbed paving).
const MEDIAN_STYLE_NAMES: Array[String] = ["None", "Painted", "Grass", "Paving"]

## Usable types the cap left out of the row at the last `refresh`.
var hidden_count := 0

var _simulation_node: Node
var _group := ButtonGroup.new()
# Type id -> its Button, in row order.
var _buttons: Dictionary = {}
var _builder_button: Button
var _marked_id := ""

func _init() -> void:
	add_theme_constant_override("separation", 10)
	_builder_button = Button.new()
	_builder_button.text = "Road builder..."
	_builder_button.focus_mode = Control.FOCUS_NONE
	_builder_button.pressed.connect(func(): road_builder_requested.emit())
	add_child(_builder_button)

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
	hidden_count = 0
	if _simulation_node == null:
		return
	for type: Dictionary in _simulation_node.get_road_types().get("types", []):
		if not bool(type.get("usable", false)):
			continue
		if _buttons.size() >= MAX_TYPES:
			hidden_count += 1
			continue
		var id := String(type.get("id", ""))
		var button := Button.new()
		button.text = String(type.get("name", id))
		button.toggle_mode = true
		button.button_group = _group
		button.focus_mode = Control.FOCUS_NONE
		button.tooltip_text = describe(type)
		var walkway: bool = int(type.get("fwd_lanes", 0)) + int(type.get("bkw_lanes", 0)) == 0
		button.pressed.connect(func(): type_picked.emit(id, walkway))
		add_child(button)
		move_child(button, _buttons.size())
		_buttons[id] = button
	_builder_button.tooltip_text = "Build custom road types"
	if hidden_count > 0:
		_builder_button.tooltip_text += "\n%d more road types are in the builder" % hidden_count
	mark(_marked_id)

## Marks type `id` as the road tool's (no button pressed when the row does not list it).
func mark(id: String) -> void:
	_marked_id = id
	for type_id: String in _buttons:
		_buttons[type_id].set_pressed_no_signal(type_id == id)

## The type ids the row lists, in order.
func listed_ids() -> Array[String]:
	var ids: Array[String] = []
	ids.assign(_buttons.keys())
	return ids

## The row's button for type `id`, or null.
func button_for(id: String) -> Button:
	return _buttons.get(id)

## "2 + 2 lanes, Grass 3.0 m, 17.0 m wide" (or "Footpath, 2.0 m wide") from Rust's figures for a
## `get_road_types` entry; `with_width` false leaves the width out.
static func describe(type: Dictionary, with_width: bool = true) -> String:
	var fwd := int(type.get("fwd_lanes", 0))
	var bkw := int(type.get("bkw_lanes", 0))
	var text := "Footpath" if fwd + bkw == 0 else "%d + %d lanes" % [fwd, bkw]
	var style := int(type.get("median_style", 0))
	if style > 0 and style < MEDIAN_STYLE_NAMES.size():
		text += ", %s %.1f m" % [MEDIAN_STYLE_NAMES[style], float(type.get("median_width", 0.0))]
	if with_width:
		text += ", %.1f m wide" % float(type.get("total_m", 0.0))
	return text
