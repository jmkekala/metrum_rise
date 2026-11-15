# SPDX-License-Identifier: GPL-2.0-only

## The road builder (`ROAD-101`): a movable, resizable, non-exclusive window where the player
## builds a road type from parts seen across the road, names it and creates it, and manages the
## custom-road library. The camera and the road tool keep working while it is open.
##
## Left: the library column (built-ins to duplicate; the player's types to edit, duplicate or
## delete, unusable ones greyed with their reason; the library error when the file cannot be
## written). The card's header is one row: the type's name, editable (Rust's suggestion until the
## player types one; Rust's description as its tooltip), and beside it the first reason the type
## cannot be created, if any. Right: the strip with the layout to scale on a stage (each end a
## fixed sidewalk, or on a side without one a 0.75 m shoulder band, drawn but not a part) with
## the total width and, under it, the type's cost per metre from Rust; a check box over each end
## toggles its sidewalk (`ROAD-107`, left and right as the road is drawn). Below the stage a
## warning when a road zones with a side or both without a sidewalk (nothing is built there),
## six part tiles, and one row with the type's attributes (speed limit signs on Rust's grid,
## hidden for a footpath; Zoning as an icon toggle, disabled with no sidewalk on either side,
## `ROAD-109`: the type's value stays; Street lights as an icon toggle, on for a new type) and,
## at its right end, Cancel and Create road (Save when editing). A new
## type whose speed first reaches Rust's `no_zoning_from_kmh` (80) gets zoning off once, unless
## the player set the Zoning toggle; the type's flag decides from then on. Parts go
## in by clicking a "+" slot and then a tile, or by dragging a tile into the strip; a click on a
## part opens its popup (median swatches and width, move arrows, remove); parts drag along
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
## Digit `digit` (1-9) was pressed while the builder had focus but no text field did: the host's
## Roads palette numbers apply, so 6 closes the builder again.
signal number_key(digit: int)

const PocketControls = preload("res://scripts/ui/road_builder/pocket_controls.gd")
const SpeedSignPicker = preload("res://scripts/ui/speed_sign_picker.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const RoadTypeMenu = preload("res://scripts/ui/road_type_menu.gd")
const RoadPartGlyph = preload("res://scripts/ui/road_builder/road_part_glyph.gd")
const RoadPartTile = preload("res://scripts/ui/road_builder/road_part_tile.gd")
const RoadBuilderStrip = preload("res://scripts/ui/road_builder/road_builder_strip.gd")
const RoadPartPopup = preload("res://scripts/ui/road_builder/road_part_popup.gd")
const RoadTypeLibraryList = preload("res://scripts/ui/road_builder/road_type_library_list.gd")
const Card = preload("res://scripts/ui/inspector_card.gd")
const HudIcons = preload("res://scripts/ui/hud_icons.gd")

const DEFAULT_SIZE := Vector2i(980, 600)
const MIN_SIZE := Vector2i(780, 590)
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
const STATUS_COST := Color(0.85, 0.85, 0.85)
# The money sign's side in the cost line, in px.
const MONEY_PX := 15
# The road's stage, darker than the card so the road reads on it.
const STAGE_BG := Color(0.0, 0.0, 0.0, 0.28)
## The warning for a type without sidewalks that zones (design D1), and per side for a type with
## one (`ROAD-107`), which `CAR_ONLY_SIDE_WARNING` names.
const NO_SIDEWALK_WARNING := "No sidewalks: nothing is built along this road"
## The space between the build cost and the upkeep in the cost line; the units tell them apart.
const COST_GAP := "      "
const NO_SIDEWALK_SIDE_WARNING := "No sidewalk on the %s: nothing is built on that side"

var window: Window
var library_list: RoadTypeLibraryList
var strip: RoadBuilderStrip
var tiles: Array[RoadPartTile] = []
var part_popup: RoadPartPopup
var status_label: Label
var cost_label: RichTextLabel
var warning_label: Label
var speed_box: SpeedSignPicker
var speed_row: Control
var zoning_check: Button
var street_lights_check: Button
var pocket_controls: PocketControls
var pocket_badge: Label
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
# The part pressed while its own popup was open: its click closes the popup instead.
var _toggle_off_index := -1
var _pending_delete: Dictionary = {}
# Rust's speed limits (`get_road_types().speeds`).
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
	window.exclusive = false
	window.visible = false
	window.close_requested.connect(close)
	window.window_input.connect(_on_window_input)
	# The game's card chrome (`inspector_card.gd`): the header drags it, × closes it; the edge
	# handles below resize it.
	var card := Card.build(window)
	window.unresizable = false
	UIStyle.set_persistent_window_layout(window, "road_builder", DEFAULT_SIZE, MIN_SIZE, get_viewport())
	HudIcons.set_icon(card["icon"], "road")
	# Header, one row: the type's name, editable, with the first reason it cannot be created
	# beside it (nothing when it can; Rust's description is the name's tooltip). Whether it is
	# a new type or an edit shows in the library and on the Create / Save button.
	for unused in ["eyebrow", "title", "chips"]:
		(card[unused] as Control).visible = false
	var name_row := HBoxContainer.new()
	name_row.add_theme_constant_override("separation", 12)
	(card["title"] as Control).get_parent().add_child(name_row)
	name_row.add_child(_build_name_edit())
	status_label = _label(name_row, 13)
	status_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	status_label.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	status_label.clip_text = true
	status_label.mouse_filter = Control.MOUSE_FILTER_PASS
	var body: Control = window.get_child(0)
	# A part dropped anywhere in the window off the strip is removed (the strip's drag end).
	body.set_drag_forwarding(Callable(), _can_drop_off_strip, func(_at, _data): pass)

	var columns := HBoxContainer.new()
	columns.size_flags_vertical = Control.SIZE_EXPAND_FILL
	columns.add_theme_constant_override("separation", 14)
	# The card's content takes the window's height, so a taller window grows the stage.
	(card["content"] as Control).size_flags_vertical = Control.SIZE_EXPAND_FILL
	card["content"].add_child(columns)

	library_list = RoadTypeLibraryList.new()
	library_list.type_selected.connect(_on_type_selected)
	library_list.new_requested.connect(func(): _start(NEW_LAYOUT, "", ""))
	library_list.duplicate_requested.connect(func(type: Dictionary): _start(type.get("parts", []), "", "", type))
	library_list.delete_requested.connect(_on_delete_requested)
	columns.add_child(_well(library_list, 6))

	var editor := VBoxContainer.new()
	editor.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	editor.add_theme_constant_override("separation", 10)
	columns.add_child(editor)
	editor.add_child(_build_stage())
	warning_label = _label(editor, 13)
	warning_label.add_theme_color_override("font_color", STATUS_NAME)
	warning_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	editor.add_child(_build_tiles())
	editor.add_child(_well(_build_bottom_row(), 8))

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

# The road on its stage: the strip large and centred with its cost per metre under it (a price
# of the cross-section, read with its width). The stage takes any height the window has to spare,
# so a taller window shows more road, not empty rows.
func _build_stage() -> Control:
	var stage := PanelContainer.new()
	stage.size_flags_vertical = Control.SIZE_EXPAND_FILL
	var style := StyleBoxFlat.new()
	style.bg_color = STAGE_BG
	style.border_color = Color(1, 1, 1, 0.06)
	style.set_border_width_all(1)
	style.set_corner_radius_all(10)
	style.set_content_margin_all(12)
	stage.add_theme_stylebox_override("panel", style)
	var column := VBoxContainer.new()
	column.alignment = BoxContainer.ALIGNMENT_CENTER
	column.add_theme_constant_override("separation", 6)
	stage.add_child(column)
	strip = RoadBuilderStrip.new()
	strip.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	strip.slot_clicked.connect(func(_slot: int): _close_popup())
	strip.part_pressed.connect(func(index: int):
		_toggle_off_index = index if part_popup.visible and _popup_index == index else -1)
	strip.part_clicked.connect(_open_popup)
	strip.end_clicked.connect(func(left: bool): set_sidewalk(left, not has_sidewalk(left)))
	strip.tile_dropped.connect(insert_part)
	strip.part_moved.connect(move_part)
	strip.part_dragged_out.connect(remove_part)
	column.add_child(strip)
	cost_label = RichTextLabel.new()
	cost_label.bbcode_enabled = true
	cost_label.fit_content = true
	cost_label.autowrap_mode = TextServer.AUTOWRAP_OFF
	cost_label.scroll_active = false
	cost_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	cost_label.add_theme_color_override("default_color", STATUS_COST)
	cost_label.add_theme_font_size_override("normal_font_size", UIStyle.scaled_font_size(14))
	column.add_child(cost_label)
	pocket_badge = _label(column, 13)
	pocket_badge.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	pocket_badge.add_theme_color_override("font_color", UIStyle.HUD_ACCENT)
	return stage

# `content` in a rounded well, `pad` px inside.
func _well(content: Control, pad: int) -> PanelContainer:
	var well := PanelContainer.new()
	var style := StyleBoxFlat.new()
	style.bg_color = Card.WELL_BG
	style.set_corner_radius_all(8)
	style.set_content_margin_all(pad)
	well.add_theme_stylebox_override("panel", style)
	well.add_child(content)
	return well

func _label(parent: Control, font_size: int) -> Label:
	var label := Label.new()
	UIStyle.set_font_size(label, font_size)
	parent.add_child(label)
	return label

# The six part tiles: the lanes by direction, the three medians at their default widths and the
# kerbside parking lane (`PARK-05`).
func _build_tiles() -> Control:
	# One row: the tiles share the width and their names wrap.
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 6)
	var tile_parts: Array[Dictionary] = [
		{"kind": "car_lane", "forward": false}, {"kind": "car_lane", "forward": true},
	]
	for style in [RoadPartGlyph.PAINTED, RoadPartGlyph.GRASS, RoadPartGlyph.PAVING]:
		tile_parts.append({"kind": "median", "style": style, "width_m": 0.0})
	tile_parts.append({"kind": "parking"})
	for part in tile_parts:
		var tile := RoadPartTile.new(part)
		tile.picked.connect(func(picked: Dictionary): insert_part(picked, _click_slot(picked)))
		row.add_child(tile)
		tiles.append(tile)
	return row

# An icon that toggles a yes / no attribute: lit in the accent when on, dim when off.
func _icon_toggle(icon_name: String, tip: String) -> Button:
	var button := Button.new()
	button.toggle_mode = true
	button.focus_mode = Control.FOCUS_NONE
	button.tooltip_text = tip
	button.icon = load("res://assets/ui/icons/%s.svg" % icon_name) as Texture2D
	button.expand_icon = true
	button.icon_alignment = HORIZONTAL_ALIGNMENT_CENTER
	button.custom_minimum_size = Vector2.ONE * UIStyle.scaled_px(40.0)
	button.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	for state in ["normal", "hover", "pressed", "hover_pressed", "disabled"]:
		var style := StyleBoxFlat.new()
		var on := String(state).contains("pressed")
		style.bg_color = Color(UIStyle.HUD_ACCENT, 0.16) if on else Color(1, 1, 1, 0.05 if state == "hover" else 0.0)
		style.border_color = Color(UIStyle.HUD_ACCENT, 0.7) if on else Color(1, 1, 1, 0.14)
		style.set_border_width_all(1)
		style.set_corner_radius_all(8)
		style.set_content_margin_all(UIStyle.scaled_px(8.0))
		button.add_theme_stylebox_override(state, style)
	for color in ["icon_pressed_color", "icon_hover_pressed_color"]:
		button.add_theme_color_override(color, UIStyle.HUD_ACCENT)
	for color in ["icon_normal_color", "icon_hover_color"]:
		button.add_theme_color_override(color, Color(1, 1, 1, 0.45))
	button.add_theme_color_override("icon_disabled_color", Color(1, 1, 1, 0.18))
	return button

# The type's attributes: the speed signs, the turn pockets, then Zoning and Street lights as icon
# toggles. The sidewalks are toggled on the strip's ends.
func _build_attributes() -> Control:
	# Wraps onto a second row in a narrow window.
	var row := HFlowContainer.new()
	row.add_theme_constant_override("h_separation", 18)
	row.add_theme_constant_override("v_separation", 6)
	var speed := HBoxContainer.new()
	speed.add_theme_constant_override("separation", 6)
	# Speed limit signs on Rust's grid: scroll or click a neighbouring sign. The sign says what
	# it is, so it has no label or tooltip.
	speed_box = SpeedSignPicker.new()
	speed_box.value_changed.connect(func(value: float): set_speed(int(value)))
	speed.add_child(speed_box)
	speed_row = speed
	row.add_child(speed)
	pocket_controls = PocketControls.new()
	pocket_controls.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	pocket_controls.changed.connect(func(): _layout_changed())
	row.add_child(pocket_controls)
	var toggles := HBoxContainer.new()
	toggles.add_theme_constant_override("separation", 6)
	row.add_child(toggles)
	zoning_check = _icon_toggle("zoning", "Zoning")
	zoning_check.toggled.connect(func(on: bool):
		_zoning_settled = true
		set_zoning(on))
	toggles.add_child(zoning_check)
	street_lights_check = _icon_toggle("street_light", "Street lights")
	street_lights_check.toggled.connect(set_street_lights)
	toggles.add_child(street_lights_check)
	return row

# The type's name as the card's title: a bold field that shows it can be typed in only under
# the cursor or while editing.
func _build_name_edit() -> LineEdit:
	name_edit = LineEdit.new()
	name_edit.custom_minimum_size.x = 280.0
	name_edit.max_length = 40
	name_edit.placeholder_text = "Road type name"
	name_edit.add_theme_font_override("font", Card.bold_font())
	UIStyle.set_font_size(name_edit, 17)
	for state in ["normal", "hover", "focus"]:
		var style := StyleBoxFlat.new()
		style.bg_color = Color(1, 1, 1, 0.0 if state == "normal" else 0.06)
		style.border_color = Color(UIStyle.HUD_ACCENT, 0.7) if state == "focus" else Color(1, 1, 1, 0.0 if state == "normal" else 0.14)
		style.set_border_width_all(1)
		style.set_corner_radius_all(6)
		style.content_margin_left = 6.0
		style.content_margin_right = 6.0
		style.content_margin_top = 2.0
		style.content_margin_bottom = 2.0
		name_edit.add_theme_stylebox_override(state, style)
	name_edit.text_changed.connect(func(_text: String):
		_name_typed = true
		_update_actions())
	name_edit.text_submitted.connect(func(_text: String): create())
	return name_edit

# The type's cost from Rust on the left; Cancel and Create on the right.
# The bottom row: the attributes, and Cancel and Create road at the right end.
func _build_bottom_row() -> Control:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 8)
	var attributes := _build_attributes()
	attributes.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_child(attributes)
	cancel_button = Button.new()
	cancel_button.text = "Cancel"
	cancel_button.focus_mode = Control.FOCUS_NONE
	cancel_button.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	UIStyle.set_font_size(cancel_button, 14)
	cancel_button.pressed.connect(close)
	row.add_child(cancel_button)
	create_button = Button.new()
	create_button.focus_mode = Control.FOCUS_NONE
	create_button.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	UIStyle.style_primary_button(create_button)
	UIStyle.set_font_size(create_button, 14)
	create_button.pressed.connect(create)
	row.add_child(create_button)
	return row

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
	pocket_controls.set_value(from.get("pockets", {}), _library.get("pocket_lengths", {}))
	_zoning_settled = false
	speed_box.set_value_no_signal(speed_kmh)
	zoning_check.set_pressed_no_signal(zoning)
	street_lights_check.set_pressed_no_signal(street_lights)
	strip.set_slot(parts.size() - 1)
	_layout_changed()

func _refresh_library() -> void:
	_library = _simulation_node.get_road_types()
	_speeds = _library.get("speeds", {})
	speed_box.values = PackedInt32Array(_speeds.get("kmh", PackedInt32Array()))
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

# Where a clicked tile goes: the selected "+" slot, except a parking lane, which may only stand
# at a kerb (`PARK-05`): the kerb on the selected slot's side, the right kerb on a tie (right-hand
# traffic); the other kerb when that one already parks. A dragged tile still lands where it is
# dropped.
func _click_slot(part: Dictionary) -> int:
	var slot := strip.selected_slot
	if String(part.get("kind", "")) != "parking":
		return slot
	var left := 1
	var right := parts.size() - 1
	var chosen := left if slot - left < right - slot else right
	var parks := func(kerb: int) -> bool:
		var beside: Variant = parts[kerb] if kerb == left else parts[kerb - 1]
		return String(Dictionary(beside).get("kind", "")) == "parking"
	if parks.call(chosen):
		chosen = right if chosen == left else left
	return chosen

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
	# A footpath hides the pocket controls, so its name never gains them.
	check = _simulation_node.check_road_layout(rust_parts(), speed_kmh, zoning, street_lights,
		{} if _is_footpath() else pocket_controls.value())
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
	strip.ends_editable = not footpath
	pocket_controls.visible = not footpath
	pocket_badge.text = pocket_controls.description() if not footpath else ""
	pocket_badge.visible = not pocket_badge.text.is_empty()
	zoning_check.disabled = not has_sidewalk(true) and not has_sidewalk(false)
	speed_row.visible = not footpath
	strip.selected_part = _popup_index
	if not _name_typed and valid:
		name_edit.text = String(check.get("suggested_name", ""))
	if _popup_index >= 0:
		part_popup.show_part(parts[_popup_index], strip.is_interior(_popup_index - 1),
			strip.is_interior(_popup_index + 1), _median_styles)
	_update_actions()

# Rust's build cost per metre and upkeep per metre and day, the money sign before each.
func _cost_bbcode() -> String:
	var values := RoadTypeMenu.cost_values(check)
	var money := "[img=%dx%d]res://assets/ui/icons/money.svg[/img]" % [MONEY_PX, MONEY_PX]
	return "[center]%s %s / m%s%s %s / m / day[/center]" % [money, values[0], COST_GAP, money, values[1]]

# Create / Save is enabled when the layout and the name both pass and the library is writable;
# the status line gives the first reason it is not.
func _update_actions() -> void:
	create_button.text = "Create road" if editing_id.is_empty() else "Save"
	var valid := bool(check.get("is_valid", false))
	var name_reason: String = _simulation_node.check_road_type_name(name_edit.text, editing_id)
	# Rust's refusal of pockets this layout cannot take; the strip still draws the layout.
	var pocket_reason := String(check.get("pocket_refusal", ""))
	create_button.disabled = not valid or not pocket_reason.is_empty() or not name_reason.is_empty() \
		or not _library_writable()
	cost_label.text = _cost_bbcode() if valid else ""
	warning_label.text = _sidewalk_warning() if valid and zoning and not _is_footpath() else ""
	warning_label.visible = not warning_label.text.is_empty()
	if not valid:
		_status(RoadReasonText.text(check, "This layout cannot be built"), STATUS_REFUSED)
	elif not pocket_reason.is_empty():
		_status(RoadReasonText.text({"invalid_reason": pocket_reason}), STATUS_REFUSED)
	elif not _library_writable():
		_status(RoadReasonText.text({"invalid_reason": _library.get("error", "")}), STATUS_REFUSED)
	elif not name_reason.is_empty():
		_status(RoadReasonText.text({"invalid_reason": name_reason}), STATUS_NAME)
	else:
		_status("", UIStyle.TEXT_DIM)
	name_edit.tooltip_text = RoadTypeMenu.describe(check) if valid else ""


# The warning for the side or sides that take no buildings (`ROAD-109`), or "".
func _sidewalk_warning() -> String:
	if not has_sidewalk(true) and not has_sidewalk(false):
		return NO_SIDEWALK_WARNING
	if has_sidewalk(true) and has_sidewalk(false):
		return ""
	return NO_SIDEWALK_SIDE_WARNING % ("right" if has_sidewalk(true) else "left")

func _status(text: String, color: Color) -> void:
	status_label.text = text
	status_label.tooltip_text = text
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
			street_lights, pocket_controls.value())
		created_id = String(created.get("id", ""))
		refusal = String(created.get("invalid_reason", ""))
	else:
		refusal = _simulation_node.update_road_type(editing_id, name_edit.text, rust_parts(), speed_kmh, zoning,
			street_lights, pocket_controls.value())
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
	# A second click on the part whose popup was open closes it: the press already hid it.
	if index == _toggle_off_index:
		_toggle_off_index = -1
		_close_popup()
		strip.selected_part = -1
		strip.queue_redraw()
		return
	_popup_index = index
	strip.selected_part = index
	# The slot on the part's kerb side (a centred part's right), so the next clicked tile goes
	# beside it towards that kerb, also after the click on the tile has closed this popup.
	strip.set_slot(index if 2 * index + 1 < parts.size() else index + 1)
	strip.queue_redraw()
	# Anchored from the part down to below the dimension line, so the widths stay readable.
	var rect := strip.part_rect(index)
	rect.size.y = strip.DIMENSION_Y + 8.0 - rect.position.y
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
	elif key.keycode >= KEY_1 and key.keycode <= KEY_9 and not _typing() \
			and not (key.ctrl_pressed or key.alt_pressed or key.meta_pressed or key.shift_pressed):
		window.set_input_as_handled()
		number_key.emit(key.keycode - KEY_0)
	elif (key.keycode == KEY_ENTER or key.keycode == KEY_KP_ENTER) and window.gui_get_focus_owner() != name_edit:
		create()
	elif (key.keycode == KEY_BACKSPACE or key.keycode == KEY_DELETE) and _popup_index >= 0 and not _typing():
		window.set_input_as_handled()
		remove_part(_popup_index)

# Whether a text field (the name, the speed box, a median width) holds the builder's focus.
func _typing() -> bool:
	var owner := window.gui_get_focus_owner()
	return owner is LineEdit or owner is TextEdit or owner is SpinBox
