# SPDX-License-Identifier: GPL-2.0-only

## The road builder (`ROAD-101`): a movable, resizable, non-exclusive window where the player
## builds a road type from parts seen across the road, names it and creates it, and manages the
## custom-road library. The camera and the road tool keep working while it is open.
##
## Left: the library column (built-ins to duplicate; the player's types to edit, duplicate or
## delete, unusable ones greyed with their reason; the library error when the file cannot be
## written). Right: five part tiles, the strip with the layout to scale (each end a fixed sidewalk,
## or on a side without one a 0.75 m shoulder band, drawn but not a part), the lane split and
## widths, the type's attributes ("Left sidewalk" and "Right sidewalk" checkboxes, one per end
## and independent, `ROAD-107`, left and right as the road is drawn, a "Speed" stepper on Rust's
## grid, hidden for a footpath, a "Zoning" checkbox and a "Street lights" checkbox, ticked for a
## new type), Rust's verdict under them with the type's cost per metre from Rust and a warning
## when a road zones with a side or both without a sidewalk (buildings there are reached by car
## only), and a footer with the name (Rust's
## suggestion until the player types one), Cancel and Create road (Save when editing). A new
## type whose speed first reaches Rust's `no_zoning_from_kmh` (80) gets zoning off once, unless
## the player set the Zoning checkbox; the type's flag decides from then on. Parts go
## in by clicking a "+" slot and then a tile, or by dragging a tile into the strip; a click on a
## part opens its popup (median style and width, Move left / right, Remove); parts drag along
## the strip and off it. Every change asks Rust's `check_road_layout`; Create is enabled only
## when the layout and the name both pass. Esc and Cancel close without writing; Enter creates.
## Create and Save write the library, emit `road_types_changed` and close; Delete asks first.
## Library edits are not undoable, and roads already built keep their own layout.
## This node is the controller; the strip, tiles, popup and list are views.
## Rust methods called: get_road_types(), reload_road_types(), check_road_layout(),
## check_road_type_name(), create_road_type(), update_road_type(), delete_road_type(),
## get_median_styles().
extends Node

## The library changed (created, saved or deleted); `created_id` is the new type's id after a
## create, else empty. The host refreshes the Roads row and the road tool.
signal road_types_changed(created_id: String)

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const RoadTypeMenu = preload("res://scripts/ui/road_type_menu.gd")
const RoadPartGlyph = preload("res://scripts/ui/road_builder/road_part_glyph.gd")
const RoadPartTile = preload("res://scripts/ui/road_builder/road_part_tile.gd")
const RoadBuilderStrip = preload("res://scripts/ui/road_builder/road_builder_strip.gd")
const RoadPartPopup = preload("res://scripts/ui/road_builder/road_part_popup.gd")
const RoadTypeLibraryList = preload("res://scripts/ui/road_builder/road_type_library_list.gd")

const DEFAULT_SIZE := Vector2i(960, 440)
const MIN_SIZE := Vector2i(760, 400)
const SIDEWALK := {"kind": "sidewalk"}
## A layout's end without a sidewalk: the roadbed's shoulder band, drawn on the strip, never sent
## to Rust.
const SHOULDER := {"kind": "shoulder"}
## A new layout: the 2-Lane Road's.
const NEW_LAYOUT := [
	{"kind": "sidewalk"}, {"kind": "car_lane", "forward": false},
	{"kind": "car_lane", "forward": true}, {"kind": "sidewalk"},
]
const STATUS_REFUSED := Color(1.0, 0.45, 0.38)
const STATUS_NAME := Color.YELLOW
const STATUS_READY := Color(0.55, 0.95, 0.55)
const STATUS_COST := Color(0.85, 0.85, 0.85)
## The warning for a type without sidewalks that zones (design D1), and per side for a type with
## one (`ROAD-107`), which `CAR_ONLY_SIDE_WARNING` names.
const CAR_ONLY_WARNING := "No sidewalks: buildings here are reached by car only"
const CAR_ONLY_SIDE_WARNING := "No sidewalk on the %s: buildings on that side are reached by car only"

var window: Window
var library_list: RoadTypeLibraryList
var strip: RoadBuilderStrip
var tiles: Array[RoadPartTile] = []
var part_popup: RoadPartPopup
var header_label: Label
var lanes_label: Label
var widths_label: Label
var status_label: Label
var cost_label: Label
var warning_label: Label
var left_sidewalk_check: CheckBox
var right_sidewalk_check: CheckBox
var speed_box: SpinBox
var speed_row: Control
var zoning_check: CheckBox
var street_lights_check: CheckBox
var name_edit: LineEdit
var cancel_button: Button
var create_button: Button
var delete_dialog: ConfirmationDialog

## The layout being built, left to right (Rust's part dictionaries; an end without a sidewalk is
## SHOULDER, which `rust_parts` leaves out).
var parts: Array = []
## The type's speed limit in km/h, whether it zones (lots and buildings along it) and whether it
## has street lights.
var speed_kmh := 50
var zoning := true
var street_lights := true
## The custom type being edited, or empty for a new one.
var editing_id := ""
## Rust's verdict on `parts` (`check_road_layout`).
var check: Dictionary = {}

var _simulation_node: Node
var _library: Dictionary = {}
var _median_styles: Dictionary = {}
# The player typed the name: Rust's suggestion no longer replaces it.
var _name_typed := false
# The part whose popup is open, or -1.
var _popup_index := -1
var _pending_delete: Dictionary = {}
# Rust's speed grid (`get_road_types().speeds`).
var _speeds: Dictionary = {}
# The player set the Zoning checkbox, or the 80 km/h suggestion was made: no further suggestion.
var _zoning_settled := false

func setup(simulation_node: Node) -> void:
	_simulation_node = simulation_node

func _ready() -> void:
	_build_window()

func _build_window() -> void:
	window = Window.new()
	window.title = "Road Builder"
	window.unresizable = false
	window.exclusive = false
	window.visible = false
	window.close_requested.connect(close)
	window.window_input.connect(_on_window_input)
	UIStyle.set_persistent_window_layout(window, "road_builder", DEFAULT_SIZE, MIN_SIZE, get_viewport())

	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	# A part dropped anywhere in the window off the strip is removed (the strip's drag end).
	body.set_drag_forwarding(Callable(), _can_drop_off_strip, func(_at, _data): pass)
	window.add_child(body)
	var margin := MarginContainer.new()
	for side in ["left", "right", "top", "bottom"]:
		margin.add_theme_constant_override("margin_" + side, UIStyle.PAD_WINDOW)
	body.add_child(margin)
	var columns := HBoxContainer.new()
	columns.add_theme_constant_override("separation", 12)
	margin.add_child(columns)

	library_list = RoadTypeLibraryList.new()
	library_list.type_selected.connect(_on_type_selected)
	library_list.new_requested.connect(func(): _start(NEW_LAYOUT, "", ""))
	library_list.duplicate_requested.connect(func(type: Dictionary): _start(type.get("parts", []), "", "", type))
	library_list.delete_requested.connect(_on_delete_requested)
	columns.add_child(library_list)
	columns.add_child(VSeparator.new())

	var editor := VBoxContainer.new()
	editor.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	editor.add_theme_constant_override("separation", 8)
	columns.add_child(editor)
	header_label = _label(editor, 15)
	header_label.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	editor.add_child(_build_tiles())

	var strip_row := HBoxContainer.new()
	strip_row.add_theme_constant_override("separation", 10)
	editor.add_child(strip_row)
	strip = RoadBuilderStrip.new()
	strip.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	strip.slot_clicked.connect(func(_slot: int): _close_popup())
	strip.part_clicked.connect(_open_popup)
	strip.tile_dropped.connect(insert_part)
	strip.part_moved.connect(move_part)
	strip.part_dragged_out.connect(remove_part)
	strip_row.add_child(strip)
	var summary := VBoxContainer.new()
	summary.custom_minimum_size.x = 110.0
	summary.alignment = BoxContainer.ALIGNMENT_CENTER
	strip_row.add_child(summary)
	lanes_label = _label(summary, 22)
	widths_label = _label(summary, 13)
	editor.add_child(_build_attributes())
	status_label = _label(editor, 13)
	status_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	cost_label = _label(editor, 13)
	cost_label.add_theme_color_override("font_color", STATUS_COST)
	warning_label = _label(editor, 13)
	warning_label.add_theme_color_override("font_color", STATUS_NAME)
	warning_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART

	var spacer := Control.new()
	spacer.size_flags_vertical = Control.SIZE_EXPAND_FILL
	editor.add_child(spacer)
	editor.add_child(_build_footer())

	part_popup = RoadPartPopup.new()
	part_popup.median_changed.connect(func(style: int, width: float): set_median(_popup_index, style, width))
	part_popup.move_requested.connect(func(step: int): move_part_by(_popup_index, step))
	part_popup.remove_requested.connect(func(): remove_part(_popup_index))
	# The hide arrives after a frame: a popup opened again meanwhile keeps its part.
	part_popup.popup_hide.connect(func():
		if part_popup.visible:
			return
		_popup_index = -1
		strip.selected_part = -1
		strip.queue_redraw())
	window.add_child(part_popup)

	delete_dialog = ConfirmationDialog.new()
	delete_dialog.title = "Delete road type"
	delete_dialog.ok_button_text = "Delete"
	delete_dialog.confirmed.connect(_on_delete_confirmed)
	window.add_child(delete_dialog)

	add_child(window)
	WindowResizeHandles.install(window)
	window.hide()

func _label(parent: Control, font_size: int) -> Label:
	var label := Label.new()
	UIStyle.set_font_size(label, font_size)
	parent.add_child(label)
	return label

# The five part tiles: the lanes by direction and the three medians at their default widths.
func _build_tiles() -> Control:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 6)
	var tile_parts: Array[Dictionary] = [
		{"kind": "car_lane", "forward": false}, {"kind": "car_lane", "forward": true},
	]
	for style in [RoadPartGlyph.PAINTED, RoadPartGlyph.GRASS, RoadPartGlyph.PAVING]:
		tile_parts.append({"kind": "median", "style": style, "width_m": 0.0})
	for part in tile_parts:
		var tile := RoadPartTile.new(part)
		tile.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		tile.picked.connect(func(picked: Dictionary): insert_part(picked, strip.selected_slot))
		row.add_child(tile)
		tiles.append(tile)
	return row

# One end's sidewalk checkbox.
func _sidewalk_check(row: Control, left: bool) -> CheckBox:
	var side := "left" if left else "right"
	var check_box := CheckBox.new()
	check_box.text = "%s sidewalk" % side.capitalize()
	check_box.focus_mode = Control.FOCUS_NONE
	check_box.tooltip_text = "A sidewalk on the %s of the direction the road is drawn; without it a 0.75 m shoulder that walkers do not use" % side
	UIStyle.set_font_size(check_box, 14)
	check_box.toggled.connect(func(on: bool): set_sidewalk(left, on))
	row.add_child(check_box)
	return check_box

# The type's attributes: a sidewalk checkbox per end, Speed, Zoning and Street lights.
func _build_attributes() -> Control:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 18)
	left_sidewalk_check = _sidewalk_check(row, true)
	right_sidewalk_check = _sidewalk_check(row, false)
	var speed := HBoxContainer.new()
	speed.add_theme_constant_override("separation", 6)
	var label := _label(speed, 14)
	label.text = "Speed"
	speed_box = SpinBox.new()
	speed_box.suffix = "km/h"
	speed_box.rounded = true
	# Wide enough for "120 km/h": a narrower line edit scrolls to the text's end ("0 km/h").
	speed_box.custom_minimum_size.x = 118.0
	speed_box.tooltip_text = "The speed limit cars drive at; faster roads cost more and are louder"
	UIStyle.set_font_size(speed_box.get_line_edit(), 14)
	speed_box.value_changed.connect(func(value: float): set_speed(int(value)))
	speed.add_child(speed_box)
	speed_row = speed
	row.add_child(speed)
	zoning_check = CheckBox.new()
	zoning_check.text = "Zoning"
	zoning_check.focus_mode = Control.FOCUS_NONE
	zoning_check.tooltip_text = "Lots and buildings along the road; off for roads nothing is built on"
	UIStyle.set_font_size(zoning_check, 14)
	zoning_check.toggled.connect(func(on: bool):
		_zoning_settled = true
		set_zoning(on))
	row.add_child(zoning_check)
	street_lights_check = CheckBox.new()
	street_lights_check.text = "Street lights"
	street_lights_check.focus_mode = Control.FOCUS_NONE
	street_lights_check.tooltip_text = "Lamp posts along the road, lit at night; they cost nothing"
	UIStyle.set_font_size(street_lights_check, 14)
	street_lights_check.toggled.connect(set_street_lights)
	row.add_child(street_lights_check)
	return row

func _build_footer() -> Control:
	var footer := HBoxContainer.new()
	footer.add_theme_constant_override("separation", 8)
	var label := _label(footer, 14)
	label.text = "Name"
	name_edit = LineEdit.new()
	name_edit.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	name_edit.max_length = 40
	name_edit.placeholder_text = "Road type name"
	UIStyle.set_font_size(name_edit, 14)
	name_edit.text_changed.connect(func(_text: String):
		_name_typed = true
		_update_actions())
	name_edit.text_submitted.connect(func(_text: String): create())
	footer.add_child(name_edit)
	cancel_button = Button.new()
	cancel_button.text = "Cancel"
	cancel_button.focus_mode = Control.FOCUS_NONE
	UIStyle.set_font_size(cancel_button, 14)
	cancel_button.pressed.connect(close)
	footer.add_child(cancel_button)
	create_button = Button.new()
	create_button.focus_mode = Control.FOCUS_NONE
	UIStyle.set_font_size(create_button, 14)
	create_button.pressed.connect(create)
	footer.add_child(create_button)
	return footer

## Opens the builder on a new layout (or brings it forward when it is open).
func open() -> void:
	if window.visible:
		window.grab_focus()
		return
	# The file may have been fixed or edited by hand since it was read: re-read it, and let the
	# host relist the types when that changed them.
	var listed := str(_simulation_node.get_road_types())
	_simulation_node.reload_road_types()
	var changed := str(_simulation_node.get_road_types()) != listed
	_median_styles = _simulation_node.get_median_styles()
	for tile in tiles:
		if String(tile.part.get("kind", "")) == "median":
			tile.part["width_m"] = float(_median_styles[int(tile.part.style)].default_m)
	_start(NEW_LAYOUT, "", "")
	if not window.has_meta("opened_once"):
		UIStyle.popup_persistent_window(window, true)
		window.set_meta("opened_once", true)
	else:
		window.show()
		window.grab_focus()
	if changed:
		road_types_changed.emit("")

## Closes the builder without writing.
func close() -> void:
	_close_popup()
	delete_dialog.hide()
	UIStyle.save_persistent_window_layout(window)
	window.hide()

## Whether the builder is open.
func is_open() -> bool:
	return window.visible

# Starts editing `layout`: a new type when `id` is empty (its name follows Rust's suggestion),
# else custom type `id` named `type_name`; `from` is the `get_road_types` entry it starts from
# (its speed, zoning and street lights), or empty for the built-ins' 50 km/h with zoning and
# street lights. A layout without parts
# gets bare sidewalks; a car layout gets a shoulder at each end without a sidewalk.
func _start(layout: Array, id: String, type_name: String, from: Dictionary = {}) -> void:
	_close_popup()
	var start: Array = layout.duplicate(true) if not layout.is_empty() else [SIDEWALK.duplicate(), SIDEWALK.duplicate()]
	if not start.any(func(part: Dictionary): return String(part.get("kind", "")) == "footpath"):
		if String(start[0].get("kind", "")) != "sidewalk":
			start.push_front(SHOULDER.duplicate())
		if String(start[-1].get("kind", "")) != "sidewalk":
			start.push_back(SHOULDER.duplicate())
	parts = start
	editing_id = id
	_name_typed = not id.is_empty()
	name_edit.text = type_name
	_refresh_library()
	# A walkway lists speed 0 (it has none): the stepper then shows the default, hidden.
	var speed := int(from.get("speed_kmh", 0))
	speed_kmh = speed if speed > 0 else int(_speeds.get("default_kmh", 50))
	zoning = bool(from.get("zoning", true))
	street_lights = bool(from.get("street_lights", true))
	_zoning_settled = false
	speed_box.set_value_no_signal(speed_kmh)
	zoning_check.set_pressed_no_signal(zoning)
	street_lights_check.set_pressed_no_signal(street_lights)
	strip.set_slot(parts.size() - 1)
	_layout_changed()

func _refresh_library() -> void:
	_library = _simulation_node.get_road_types()
	_speeds = _library.get("speeds", {})
	speed_box.min_value = int(_speeds.get("min_kmh", 20))
	speed_box.max_value = int(_speeds.get("max_kmh", 120))
	speed_box.step = int(_speeds.get("step_kmh", 10))
	library_list.show_library(_library, editing_id)

## The layout as Rust takes it: `parts` without the shoulder ends.
func rust_parts() -> Array:
	return parts.filter(func(part: Dictionary): return String(part.get("kind", "")) != "shoulder")

## Whether the layout has a sidewalk on its left (`left`) or right end, not a shoulder.
func has_sidewalk(left: bool) -> bool:
	return parts.is_empty() or String(parts[0 if left else -1].get("kind", "")) != "shoulder"

# A footpath layout (a walkway, only from a hand-written entry): no speed and no sidewalks.
func _is_footpath() -> bool:
	return parts.any(func(part: Dictionary): return String(part.get("kind", "")) == "footpath")

## Gives the layout's left (`left`) or right end a sidewalk (`on`) or a shoulder: the other end
## and the parts between keep their places.
func set_sidewalk(left: bool, on: bool) -> void:
	parts[0 if left else -1] = (SIDEWALK if on else SHOULDER).duplicate()
	_close_popup()
	strip.set_slot(strip.selected_slot)
	_layout_changed()

## Sets the type's speed (Rust checks the grid). A new type reaching `no_zoning_from_kmh` gets
## zoning off once, unless the player set the Zoning checkbox.
func set_speed(kmh: int) -> void:
	speed_kmh = kmh
	if editing_id.is_empty() and not _zoning_settled and kmh >= int(_speeds.get("no_zoning_from_kmh", 80)):
		_zoning_settled = true
		zoning = false
		zoning_check.set_pressed_no_signal(false)
	speed_box.set_value_no_signal(kmh)
	_layout_changed()

## Sets whether the type zones.
func set_zoning(on: bool) -> void:
	zoning = on
	zoning_check.set_pressed_no_signal(on)
	_layout_changed()

## Sets whether the type has street lights.
func set_street_lights(on: bool) -> void:
	street_lights = on
	street_lights_check.set_pressed_no_signal(on)
	_layout_changed()

func _library_writable() -> bool:
	return String(_library.get("error", "")).is_empty()

## Inserts `part` at `slot` (between the sidewalks) and selects the slot after it.
func insert_part(part: Dictionary, slot: int) -> void:
	slot = clampi(slot, 1, parts.size() - 1)
	parts.insert(slot, part.duplicate())
	strip.set_slot(slot + 1)
	_layout_changed()

## Moves interior part `from` to land on slot `slot` (the boundary it was dropped on).
func move_part(from: int, slot: int) -> void:
	if not strip.is_interior(from):
		return
	slot = clampi(slot, 1, parts.size() - 1)
	if slot == from or slot == from + 1:
		return
	var part: Dictionary = parts[from]
	parts.remove_at(from)
	parts.insert(slot if slot < from else slot - 1, part)
	_layout_changed()

## Swaps interior part `index` with its neighbour `step` (-1 left, +1 right); the popup follows.
func move_part_by(index: int, step: int) -> void:
	var to := index + step
	if not strip.is_interior(index) or not strip.is_interior(to):
		return
	var part: Dictionary = parts[index]
	parts[index] = parts[to]
	parts[to] = part
	if _popup_index == index:
		_popup_index = to
	_layout_changed()

## Removes interior part `index`.
func remove_part(index: int) -> void:
	if not strip.is_interior(index):
		return
	_close_popup()
	parts.remove_at(index)
	if strip.selected_slot > index:
		strip.selected_slot -= 1
	_layout_changed()

## Gives median part `index` style `style` and width `width_m`.
func set_median(index: int, style: int, width_m: float) -> void:
	if index < 0 or index >= parts.size() or String(parts[index].get("kind", "")) != "median":
		return
	parts[index] = {"kind": "median", "style": style, "width_m": width_m}
	_layout_changed()

# Asks Rust's verdict on the layout and shows it. The strip draws a shoulder band at each end
# without a sidewalk, sharing what Rust's total has beyond its parts.
func _layout_changed() -> void:
	check = _simulation_node.check_road_layout(rust_parts(), speed_kmh, zoning, street_lights)
	var valid := bool(check.get("is_valid", false))
	var widths: PackedFloat32Array = check.get("part_widths_m", PackedFloat32Array())
	var shoulders := int(not has_sidewalk(true)) + int(not has_sidewalk(false))
	if shoulders > 0:
		var parts_m := 0.0
		for width in widths:
			parts_m += width
		var shoulder := (float(check.get("total_m", 0.0)) - parts_m) / shoulders
		var drawn := PackedFloat32Array()
		if not has_sidewalk(true):
			drawn.append(shoulder)
		drawn.append_array(widths)
		if not has_sidewalk(false):
			drawn.append(shoulder)
		widths = drawn
	strip.set_layout(parts, widths, valid)
	var footpath := _is_footpath()
	left_sidewalk_check.set_pressed_no_signal(has_sidewalk(true))
	right_sidewalk_check.set_pressed_no_signal(has_sidewalk(false))
	left_sidewalk_check.disabled = footpath
	right_sidewalk_check.disabled = footpath
	speed_row.visible = not footpath
	strip.selected_part = _popup_index
	lanes_label.text = "%d + %d" % [int(check.get("fwd_lanes", 0)), int(check.get("bkw_lanes", 0))] if valid else "-"
	widths_label.text = "Road %.1f m\nTotal %.1f m" % [float(check.get("carriageway_m", 0.0)), float(check.get("total_m", 0.0))]
	if not _name_typed and valid:
		name_edit.text = String(check.get("suggested_name", ""))
	if editing_id.is_empty():
		header_label.text = "New road type"
	else:
		header_label.text = "Editing %s" % _type_name(editing_id)
	if _popup_index >= 0:
		part_popup.show_part(parts[_popup_index], strip.is_interior(_popup_index - 1),
			strip.is_interior(_popup_index + 1), _median_styles)
	_update_actions()

func _type_name(id: String) -> String:
	for type: Dictionary in _library.get("types", []):
		if String(type.get("id", "")) == id:
			return String(type.get("name", id))
	return id

# Create / Save is enabled when the layout and the name both pass and the library is writable;
# the status line gives the first reason it is not.
func _update_actions() -> void:
	create_button.text = "Create road" if editing_id.is_empty() else "Save"
	var valid := bool(check.get("is_valid", false))
	var name_reason: String = _simulation_node.check_road_type_name(name_edit.text, editing_id)
	create_button.disabled = not valid or not name_reason.is_empty() or not _library_writable()
	cost_label.text = RoadTypeMenu.cost_text(check) if valid else ""
	warning_label.text = _car_only_warning() if valid and zoning and not _is_footpath() else ""
	if not valid:
		_status(RoadReasonText.text(check, "This layout cannot be built"), STATUS_REFUSED)
	elif not _library_writable():
		_status(RoadReasonText.text({"invalid_reason": _library.get("error", "")}), STATUS_REFUSED)
	elif not name_reason.is_empty():
		_status(RoadReasonText.text({"invalid_reason": name_reason}), STATUS_NAME)
	else:
		_status(RoadTypeMenu.describe(check), STATUS_READY)

# The warning for the side or sides whose buildings no walker reaches, or "".
func _car_only_warning() -> String:
	if not has_sidewalk(true) and not has_sidewalk(false):
		return CAR_ONLY_WARNING
	if has_sidewalk(true) and has_sidewalk(false):
		return ""
	return CAR_ONLY_SIDE_WARNING % ("right" if has_sidewalk(true) else "left")

func _status(text: String, color: Color) -> void:
	status_label.text = text
	status_label.add_theme_color_override("font_color", color)

## Creates the new type (or saves the edited one) when enabled; on success the library changes,
## `road_types_changed` is emitted and the builder closes.
func create() -> void:
	if create_button.disabled:
		return
	var created_id := ""
	var refusal := ""
	if editing_id.is_empty():
		var created: Dictionary = _simulation_node.create_road_type(name_edit.text, rust_parts(), speed_kmh, zoning,
			street_lights)
		created_id = String(created.get("id", ""))
		refusal = String(created.get("invalid_reason", ""))
	else:
		refusal = _simulation_node.update_road_type(editing_id, name_edit.text, rust_parts(), speed_kmh, zoning,
			street_lights)
	if not refusal.is_empty():
		# Another game may have changed the file since it was listed.
		_refresh_library()
		_status(RoadReasonText.text({"invalid_reason": refusal}), STATUS_REFUSED)
		return
	close()
	road_types_changed.emit(created_id)

func _on_type_selected(type: Dictionary) -> void:
	if bool(type.get("builtin", false)):
		if String(type.get("id", "")) == RoadTypeLibraryList.WALKWAY_ID:
			_status("The walkway is not built in the road builder", STATUS_NAME)
			return
		# A built-in is a template: its layout starts a new type.
		_start(type.get("parts", []), "", "", type)
		library_list.select_id(String(type.get("id", "")))
		header_label.text = "New road type from %s" % String(type.get("name", ""))
		return
	_start(type.get("parts", []), String(type.get("id", "")), String(type.get("name", "")), type)

func _on_delete_requested(type: Dictionary) -> void:
	if type.is_empty() or bool(type.get("builtin", false)):
		return
	_pending_delete = type
	delete_dialog.dialog_text = "Delete the road type \"%s\"?\nRoads already built with it keep their layout. This cannot be undone." \
		% String(type.get("name", ""))
	delete_dialog.popup_centered()

func _on_delete_confirmed() -> void:
	var id := String(_pending_delete.get("id", ""))
	_pending_delete = {}
	if id.is_empty():
		return
	var refusal: String = _simulation_node.delete_road_type(id)
	if not refusal.is_empty():
		_refresh_library()
		_status(RoadReasonText.text({"invalid_reason": refusal}), STATUS_REFUSED)
		return
	if editing_id == id:
		_start(NEW_LAYOUT, "", "")
	else:
		_refresh_library()
	road_types_changed.emit("")

func _open_popup(index: int) -> void:
	_popup_index = index
	strip.selected_part = index
	strip.queue_redraw()
	var rect := strip.part_rect(index)
	part_popup.show_for(parts[index], strip.is_interior(index - 1), strip.is_interior(index + 1), _median_styles,
		Rect2(strip.get_screen_position() + rect.position, rect.size))

func _close_popup() -> void:
	if part_popup and part_popup.visible:
		part_popup.hide()
	_popup_index = -1

func _can_drop_off_strip(_at: Vector2, data: Variant) -> bool:
	return data is Dictionary and data.get("road_builder", "") == "part"

# Esc closes and Enter creates while the window has the keyboard (the name field submits its
# own Enter).
func _on_window_input(event: InputEvent) -> void:
	var key := event as InputEventKey
	if key == null or not key.pressed or key.echo:
		return
	if key.keycode == KEY_ESCAPE:
		close()
	elif (key.keycode == KEY_ENTER or key.keycode == KEY_KP_ENTER) and window.gui_get_focus_owner() != name_edit:
		create()
