# SPDX-License-Identifier: GPL-2.0-only

## The road builder (`ROAD-101`): a movable, resizable, non-exclusive window where the player
## builds a road type from parts seen across the road, names it and creates it, and manages the
## custom-road library. The camera and the road tool keep working while it is open.
##
## Left: the library column (built-ins to duplicate; the player's types to edit, duplicate or
## delete, unusable ones greyed with their reason; the library error when the file cannot be
## written). Right: five part tiles, the strip with the layout to scale (sidewalks fixed at both
## ends), the lane split and widths, Rust's verdict under it, and a footer with the name (Rust's
## suggestion until the player types one), Cancel and Create road (Save when editing). Parts go
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

const DEFAULT_SIZE := Vector2i(960, 360)
const MIN_SIZE := Vector2i(760, 300)
const SIDEWALK := {"kind": "sidewalk"}
## A new layout: the 2-Lane Road's.
const NEW_LAYOUT := [
	{"kind": "sidewalk"}, {"kind": "car_lane", "forward": false},
	{"kind": "car_lane", "forward": true}, {"kind": "sidewalk"},
]
const STATUS_REFUSED := Color(1.0, 0.45, 0.38)
const STATUS_NAME := Color.YELLOW
const STATUS_READY := Color(0.55, 0.95, 0.55)

var window: Window
var library_list: RoadTypeLibraryList
var strip: RoadBuilderStrip
var tiles: Array[RoadPartTile] = []
var part_popup: RoadPartPopup
var header_label: Label
var lanes_label: Label
var widths_label: Label
var status_label: Label
var name_edit: LineEdit
var cancel_button: Button
var create_button: Button
var delete_dialog: ConfirmationDialog

## The layout being built, left to right (Rust's part dictionaries).
var parts: Array = []
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
	library_list.duplicate_requested.connect(func(type: Dictionary): _start(type.get("parts", []), "", ""))
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
	status_label = _label(editor, 13)
	status_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART

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
# else custom type `id` named `type_name`. A layout without parts gets bare sidewalks.
func _start(layout: Array, id: String, type_name: String) -> void:
	_close_popup()
	parts = layout.duplicate(true) if not layout.is_empty() else [SIDEWALK.duplicate(), SIDEWALK.duplicate()]
	editing_id = id
	_name_typed = not id.is_empty()
	name_edit.text = type_name
	_refresh_library()
	strip.set_slot(parts.size() - 1)
	_layout_changed()

func _refresh_library() -> void:
	_library = _simulation_node.get_road_types()
	library_list.show_library(_library, editing_id)

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

# Asks Rust's verdict on the layout and shows it.
func _layout_changed() -> void:
	check = _simulation_node.check_road_layout(parts)
	var valid := bool(check.get("is_valid", false))
	strip.set_layout(parts, check.get("part_widths_m", PackedFloat32Array()), valid)
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
	if not valid:
		_status(RoadReasonText.text(check, "This layout cannot be built"), STATUS_REFUSED)
	elif not _library_writable():
		_status(RoadReasonText.text({"invalid_reason": _library.get("error", "")}), STATUS_REFUSED)
	elif not name_reason.is_empty():
		_status(RoadReasonText.text({"invalid_reason": name_reason}), STATUS_NAME)
	else:
		_status(RoadTypeMenu.describe(check), STATUS_READY)

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
		var created: Dictionary = _simulation_node.create_road_type(name_edit.text, parts)
		created_id = String(created.get("id", ""))
		refusal = String(created.get("invalid_reason", ""))
	else:
		refusal = _simulation_node.update_road_type(editing_id, name_edit.text, parts)
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
		_start(type.get("parts", []), "", "")
		library_list.select_id(String(type.get("id", "")))
		header_label.text = "New road type from %s" % String(type.get("name", ""))
		return
	_start(type.get("parts", []), String(type.get("id", "")), String(type.get("name", "")))

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
