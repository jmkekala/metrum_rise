# SPDX-License-Identifier: GPL-2.0-only

## Floating road-properties window for SelectTool edge editing.
##
## The window mirrors the previous side panel behavior but uses Godot's built-in
## draggable Window surface, sized to content and placed beside each new selection.
## Road type (`ROAD-101`, `ROAD-80`): a picker lists the usable road types, each with its speed
## and Rust's build cost per metre, and shows the selected road's (Rust's `road_type` for its
## cross-section, parking, sidewalk sides and speed, or "Custom layout" when no type matches;
## zoning is not matched). A pick asks Rust's verdict for the type's lanes, median, parking, speed,
## sidewalk sides (`ROAD-107`: left and right in each road's own drawn direction) and zoning on
## every selected road through the select tool (the in-place edit's check, no geometry
## preview) and shows it beside Apply, which reads "Apply (+1,230)" with the selection's cost
## difference from Rust (charged, never refunded) and commits an accepted change (Ctrl+Z undoes
## the roads). When zoning off would demolish buildings, Apply first asks in a confirmation
## ("Apply removes N buildings. This cannot be undone.": undo brings back the road, never the
## buildings); Cancel leaves the roads as they are. The picker stays enabled on a walkway so the
## refusal (walkway <-> road) shows with its text. The window leads with the selected road (its
## thumbnail, name and lanes); "Change road type" unfolds the other types, the verdict and Apply.
## Kerbside parking (`PARK-05`) has its own Left and Right toggles under the road, which change only
## that lane, in place; a refusal shows in the warning line. Hovering one lights that half of the
## selected road. The speed limit shows as its sign (the first selected car road's; hidden for
## walkways only); turning the sign changes only the speed of every selected car road, in place,
## once the picker rests, so a run of scroll steps is one edit and one undo. "No buildings" is disabled on a road
## with no sidewalk on either side (`ROAD-109`: nothing is built there whatever it says). A type
## that would take a sidewalk from under a building's door is refused with that reason. Turn
## pockets are edited from the Junction window (`junction_window.gd`, `ROAD-95`).
## Rust methods called: get_road_types(), get_edge_cross_section(), get_edge_class(),
## get_no_building_spawn(), get_edge_geometry_3d(), get_height_at().
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const RoadTypeMenu = preload("res://scripts/ui/road_type_menu.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const Card = preload("res://scripts/ui/inspector_card.gd")
const SpeedSignPicker = preload("res://scripts/ui/speed_sign_picker.gd")
# The speed sign's height in the window, and how long the picker rests before its limit applies:
# scrolling 50 to 100 is one edit and one undo record, not five.
const SPEED_SIGN_HEIGHT := 40.0
const SPEED_SETTLE_S := 0.4
## The picker's text for a road whose cross-section no type has.
const CUSTOM_LAYOUT := "Custom layout"
const VERDICT_READY := Color(0.55, 0.95, 0.55)
const VERDICT_REFUSED := UIStyle.SIGN_YELLOW
# Every row fits at the default size: the class buttons, the road type and its layout, the
# verdict with Apply, the parking lane and No buildings.
const DEFAULT_SIZE := Vector2i(428, 320)
const MIN_SIZE := Vector2i(428, 140)
## Parking code bits of the left and right kerb, as Rust's `parking` reports them.
const PARKING_LEFT := 1
const PARKING_RIGHT := 2

var _simulation_node: Node
var _select_tool: Node

var _window: Window
var _warning_label: Label
var _no_build_check: CheckButton
var _class_buttons: Array[Button] = []
# Native selection/metadata model retained for the existing edit API and regression drivers.
# The visible tiles feed the same item_selected path; there is only one pending type.
var _type_picker: OptionButton
var _type_grid: GridContainer
var _type_tiles: Dictionary = {}
var _type_scroll: ScrollContainer
# The selected road: its type's thumbnail and name above the lanes line.
var _current_tile: Button
var _name_label: Label
# "Change road type" reveals the other types, the verdict and Apply.
var _change_button: Button
var _change_box: VBoxContainer
var _body: Control
var _layout_label: Label
var _verdict_label: Label
var _apply_button: Button
var _parking_checks: Dictionary = {}
var _speed_row: HBoxContainer
var _speed_picker: SpeedSignPicker
var _speed_timer: Timer
var _demolish_dialog: ConfirmationDialog
# The select tool's verdict on the picked type (`preview_selected_edge_cross_section`).
var _verdict: Dictionary = {}
# Rust's speed grid (`get_road_types().speeds`), for a walkway's attributes.
var _speeds: Dictionary = {}
var _selected_edges: Array[int] = []

func setup(simulation_node: Node, select_tool: Node) -> void:
	_simulation_node = simulation_node
	_select_tool = select_tool

func _ready() -> void:
	_build_window()

func _build_window() -> void:
	_window = Window.new()
	_window.title = "Road Properties"
	_window.exclusive = false
	_window.visible = false
	_window.close_requested.connect(_close_selection)
	UIStyle.keep_keyboard_with_game(_window)
	# The game's card chrome (`inspector_card.gd`): the header drags it, × closes it.
	var parts := Card.build(_window)
	# Content-sized: stale persisted dimensions from the former large panel do not apply.
	UIStyle.set_window_base_size(_window, DEFAULT_SIZE, MIN_SIZE, get_viewport())
	_body = _window.get_child(0)
	var root_vbox: VBoxContainer = parts["content"]
	(parts["eyebrow"] as Label).text = "Road properties"

	# The selected road: its type's thumbnail in the icon well, its name and its layout.
	_current_tile = parts["icon"]
	_current_tile.custom_minimum_size = Vector2.ONE * 52.0
	_name_label = parts["title"]
	(parts["chips"] as Control).hide()
	_layout_label = Label.new()
	_layout_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_layout_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	_layout_label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	UIStyle.set_font_size(_layout_label, 12)
	_name_label.get_parent().add_child(_layout_label)

	_warning_label = Label.new()
	_warning_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_warning_label.add_theme_color_override("font_color", UIStyle.SIGN_YELLOW)
	UIStyle.set_font_size(_warning_label, 13)
	_warning_label.hide()
	root_vbox.add_child(_warning_label)

	var settings := HBoxContainer.new()
	settings.add_theme_constant_override("separation", 2)
	root_vbox.add_child(settings)
	for i in range(3):
		var title: String = ["Standard", "Bridge", "Tunnel"][i]
		var button := HudIcons.icon_button(["road", "bridge", "tunnel"][i], title, "", 36)
		button.toggle_mode = true
		button.pressed.connect(_on_class_button_pressed.bind(i))
		settings.add_child(button)
		_class_buttons.append(button)
	var gap := Control.new()
	gap.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	settings.add_child(gap)
	_no_build_check = CheckButton.new()
	_no_build_check.focus_mode = Control.FOCUS_NONE
	_no_build_check.text = "No buildings"
	_no_build_check.tooltip_text = "Keep buildings from spawning along this road."
	UIStyle.set_font_size(_no_build_check, 13)
	_no_build_check.toggled.connect(_on_no_build_toggled)
	settings.add_child(_no_build_check)

	# Kerbside parking per side (`PARK-05`), switched like No buildings.
	var parking_row := HBoxContainer.new()
	parking_row.add_theme_constant_override("separation", 8)
	root_vbox.add_child(parking_row)
	var parking_label := Label.new()
	parking_label.text = "Parking lane"
	parking_label.tooltip_text = "Left and right looking along the direction the road was drawn."
	parking_label.mouse_filter = Control.MOUSE_FILTER_PASS
	parking_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	parking_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(parking_label, 13)
	parking_row.add_child(parking_label)
	for side in [[PARKING_LEFT, "Left"], [PARKING_RIGHT, "Right"]]:
		var check := CheckButton.new()
		check.focus_mode = Control.FOCUS_NONE
		check.text = side[1]
		UIStyle.set_font_size(check, 13)
		check.toggled.connect(_on_parking_toggled.bind(side[0]))
		# Hovering a side lights that half of the selected road, so Left and Right read off it.
		check.mouse_entered.connect(_light_side.bind(side[0]))
		check.mouse_exited.connect(_light_side.bind(0))
		parking_row.add_child(check)
		_parking_checks[side[0]] = check

	# The speed limit as its sign; turning it changes only the speed, in place.
	_speed_row = HBoxContainer.new()
	_speed_row.add_theme_constant_override("separation", 8)
	root_vbox.add_child(_speed_row)
	var speed_label := Label.new()
	speed_label.text = "Speed limit"
	speed_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	speed_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(speed_label, 13)
	_speed_row.add_child(speed_label)
	_speed_picker = SpeedSignPicker.new()
	_speed_picker.base_height = SPEED_SIGN_HEIGHT
	_speed_picker.tooltip_text = "Scroll or click a side sign to change only the speed limit."
	_speed_picker.value_changed.connect(func(_kmh: float): _speed_timer.start())
	_speed_row.add_child(_speed_picker)
	_speed_timer = Timer.new()
	_speed_timer.one_shot = true
	_speed_timer.wait_time = SPEED_SETTLE_S
	_speed_timer.timeout.connect(_apply_speed)
	_speed_row.add_child(_speed_timer)

	_change_button = Button.new()
	_change_button.text = "Change road type"
	_change_button.toggle_mode = true
	_change_button.focus_mode = Control.FOCUS_NONE
	_change_button.alignment = HORIZONTAL_ALIGNMENT_LEFT
	_change_button.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
	UIStyle.set_font_size(_change_button, 12)
	# A quiet well that lights on hover, like the card's tiles.
	for state in ["normal", "hover", "pressed", "hover_pressed"]:
		var well := StyleBoxFlat.new()
		well.bg_color = Color(1, 1, 1, 0.09) if state.begins_with("hover") else Card.WELL_BG
		well.set_corner_radius_all(8)
		well.content_margin_left = 10
		well.content_margin_right = 10
		well.content_margin_top = 6
		well.content_margin_bottom = 7
		_change_button.add_theme_stylebox_override(state, well)
	_change_button.add_theme_stylebox_override("focus", StyleBoxEmpty.new())
	_change_button.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	_change_button.add_theme_color_override("font_hover_color", UIStyle.TEXT_PRIMARY)
	_change_button.add_theme_color_override("font_pressed_color", UIStyle.TEXT_PRIMARY)
	_change_button.add_theme_color_override("font_hover_pressed_color", UIStyle.TEXT_PRIMARY)
	_change_button.toggled.connect(_show_change)
	root_vbox.add_child(_change_button)

	_change_box = VBoxContainer.new()
	_change_box.add_theme_constant_override("separation", 8)
	root_vbox.add_child(_change_box)
	_type_picker = OptionButton.new()
	_type_picker.item_selected.connect(_on_type_selected)
	_change_box.add_child(_type_picker)
	_type_picker.hide()
	_type_scroll = ScrollContainer.new()
	_type_scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	_type_scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_change_box.add_child(_type_scroll)
	_type_grid = GridContainer.new()
	_type_grid.columns = 5
	_type_grid.add_theme_constant_override("h_separation", 4)
	_type_grid.add_theme_constant_override("v_separation", 4)
	_type_scroll.add_child(_type_grid)
	_verdict_label = Label.new()
	_verdict_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_verdict_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(_verdict_label, 13)
	_change_box.add_child(_verdict_label)
	_apply_button = Button.new()
	_apply_button.text = "Apply"
	_apply_button.disabled = true
	UIStyle.style_primary_button(_apply_button)
	_apply_button.pressed.connect(_on_apply_pressed)
	_change_box.add_child(_apply_button)
	_show_change(false)

	_demolish_dialog = ConfirmationDialog.new()
	_demolish_dialog.title = "Remove buildings"
	_demolish_dialog.ok_button_text = "Apply"
	_demolish_dialog.confirmed.connect(_apply)
	_window.add_child(_demolish_dialog)

	add_child(_window)
	_window.size_changed.connect(_fit_columns)
	_body.minimum_size_changed.connect(_refit_visible, CONNECT_DEFERRED)
	_window.hide()

func show_for_edges(edge_indices: Array[int], screen_pos: Vector2 = Vector2.ZERO) -> void:
	_selected_edges = edge_indices.duplicate()
	if _selected_edges.is_empty():
		close_window()
		return
	_refresh()
	if not _window.visible:
		_fit_content()
		_place_window_near_cursor(screen_pos)
	_window.show()

func close_window() -> void:
	_light_side(0)
	_selected_edges.clear()
	if _window:
		_demolish_dialog.hide()
		_window.hide()

# Lights the selected road's `side_bit` half (PARKING_LEFT or PARKING_RIGHT), or neither for 0.
func _light_side(side_bit: int) -> void:
	if _select_tool and _select_tool.has_method("light_side"):
		_select_tool.light_side(side_bit)

func _close_selection() -> void:
	if _select_tool:
		_select_tool.clear_selection()
	close_window()

# The other types, the verdict and Apply stay folded until the player asks to change the type.
func _show_change(open: bool) -> void:
	_change_box.visible = open
	_change_button.text = "▾  Hide road types" if open else "▸  Change road type"

func _fit_columns() -> void:
	if _type_grid:
		_type_grid.columns = clampi(int((_window.size.x - UIStyle.PAD_WINDOW * 2 - 16) / 76), 1, 5)

func _refit_visible() -> void:
	if _window.visible:
		_fit_content()

func _fit_content() -> void:
	_fit_columns()
	var rows := ceili(float(_type_tiles.size()) / float(_type_grid.columns))
	_type_scroll.custom_minimum_size.y = minf(rows, 2) * 76.0
	_window.size.y = mini(int(_body.get_combined_minimum_size().y), int(get_viewport().get_visible_rect().size.y * 0.9))
	# Unfolding the road types grows the window down: it must not hang off the screen.
	UIStyle.keep_window_bottom_on_screen(_window, get_viewport())

func _place_window_near_cursor(screen_pos: Vector2) -> void:
	UIStyle.place_window_near_cursor(_window, screen_pos, get_viewport())

func _refresh() -> void:
	if _selected_edges.is_empty() or _simulation_node == null:
		return

	_refresh_type()
	_refresh_parking()
	_refresh_speed()
	_no_build_check.disabled = _selected_edges.all(_has_no_sidewalk)
	if _selected_edges.size() == 1:
		var edge_idx := _selected_edges[0]
		_warning_label.text = ""
		_no_build_check.set_pressed_no_signal(_simulation_node.get_no_building_spawn(edge_idx))
		var current_class: int = _simulation_node.get_edge_class(edge_idx)
		for i in range(_class_buttons.size()):
			_class_buttons[i].set_pressed_no_signal(i == current_class)

		var geometry = _simulation_node.get_edge_geometry_3d(edge_idx)
		if geometry.size() >= 2:
			var p0 = geometry[0]
			var p1 = geometry[-1]
			var h0 = _simulation_node.get_height_at(Vector2(p0.x, p0.z))
			var h1 = _simulation_node.get_height_at(Vector2(p1.x, p1.z))
			var midpoint = geometry[geometry.size() / 2]
			var h_mid = _simulation_node.get_height_at(Vector2(midpoint.x, midpoint.z))
			if p0.y > h0 + 1.0 and p1.y > h1 + 1.0:
				if midpoint.y < h_mid + 2.0:
					_warning_label.text = "Warning: Bridge may clash with terrain!"
			elif p0.y < h0 - 1.0 and p1.y < h1 - 1.0:
				if midpoint.y > h_mid - 2.0:
					_warning_label.text = "Warning: Tunnel might be above surface!"
	else:
		_warning_label.text = "%d edges selected" % _selected_edges.size()
		var all_no_build := _selected_edges.all(
			func(idx): return _simulation_node.get_no_building_spawn(idx)
		)
		_no_build_check.set_pressed_no_signal(all_no_build)
		var first_class: int = _simulation_node.get_edge_class(_selected_edges[0])
		var shared_class := _selected_edges.all(
			func(idx): return _simulation_node.get_edge_class(idx) == first_class
		)
		for i in range(_class_buttons.size()):
			_class_buttons[i].set_pressed_no_signal(shared_class and i == first_class)

	_warning_label.visible = not _warning_label.text.is_empty()

# Whether `edge_idx` is a car road with no sidewalk on either side. A walkway is its own path.
func _has_no_sidewalk(edge_idx: int) -> bool:
	var section: Dictionary = _simulation_node.get_edge_cross_section(edge_idx)
	return bool(section.get("editable", false)) and int(section.get("sidewalks", 0)) == 0

## Lists the road types again (after a library edit) while the window shows a road.
func refresh_types() -> void:
	if _window and _window.visible and not _selected_edges.is_empty():
		_refresh_type()

# The picker lists the usable types with the first selected road's marked, or "Custom layout"
# (not pickable) when no type has its cross-section. Every item's metadata is its type.
func _refresh_type() -> void:
	var first: Dictionary = _simulation_node.get_edge_cross_section(_selected_edges[0])
	var current := String(first.get("road_type", ""))
	_type_picker.clear()
	for tile in _type_grid.get_children():
		_type_grid.remove_child(tile)
		tile.queue_free()
	_type_tiles.clear()
	var library: Dictionary = _simulation_node.get_road_types()
	_speeds = library.get("speeds", {})
	if current.is_empty():
		_type_picker.add_item(CUSTOM_LAYOUT)
		_type_picker.set_item_disabled(0, true)
		_type_picker.set_item_metadata(0, {})
	for type: Dictionary in library.get("types", []):
		if not bool(type.get("usable", false)):
			continue
		_type_picker.add_item(RoadTypeMenu.picker_text(type))
		_type_picker.set_item_metadata(_type_picker.item_count - 1, type)
		var index := _type_picker.item_count - 1
		if String(type.get("id", "")) == current:
			_type_picker.select(index)
			continue
		var tile := HudIcons.road_tile(type, RoadTypeMenu.tooltip(type))
		tile.pressed.connect(func():
			_type_picker.select(index)
			_type_picker.item_selected.emit(index))
		_type_grid.add_child(tile)
		_type_tiles[index] = tile
	if current.is_empty():
		_type_picker.select(0)
	_mark_type_tiles()
	var current_type := _picked_type()
	_name_label.text = String(current_type.get("name", CUSTOM_LAYOUT))
	HudIcons.set_road(_current_tile, current_type)
	# The speed is the sign row's.
	_layout_label.text = RoadTypeMenu.describe(first, false, false)
	_change_button.set_pressed_no_signal(false)
	_show_change(false)
	_verdict_label.text = ""
	_verdict_label.hide()
	_verdict = {}
	_apply_button.text = "Apply"
	_apply_button.disabled = true
	_refit_visible.call_deferred()

# A side's box is ticked when every selected road parks there.
func _refresh_parking() -> void:
	for bit: int in _parking_checks:
		var on := _selected_edges.all(
			func(idx): return (int(_simulation_node.get_edge_cross_section(idx).get("parking", 0)) & bit) != 0
		)
		(_parking_checks[bit] as CheckButton).set_pressed_no_signal(on)

# The first selected car road's limit on the sign; hidden when only walkways are selected.
func _refresh_speed() -> void:
	_speed_timer.stop()
	var speed := 0
	for idx in _selected_edges:
		speed = int(_simulation_node.get_edge_cross_section(idx).get("speed_kmh", 0))
		if speed > 0:
			break
	_speed_row.visible = speed > 0
	_speed_picker.values = PackedInt32Array(_speeds.get("kmh", PackedInt32Array()))
	if speed > 0:
		_speed_picker.set_value_no_signal(speed)

func _apply_speed() -> void:
	if _select_tool == null or not _select_tool.has_method("set_selected_edge_speed"):
		return
	var result: Dictionary = _select_tool.set_selected_edge_speed(int(_speed_picker.value))
	_refresh()
	var refused := int(result.get("refused", 0))
	if refused > 0:
		_warning_label.text = RoadReasonText.text(result.get("verdict", {}), "%d road(s) refused the speed limit" % refused)
		_warning_label.show()

func _on_parking_toggled(enabled: bool, side_bit: int) -> void:
	if _select_tool == null or not _select_tool.has_method("set_selected_edge_parking_side"):
		return
	var result: Dictionary = _select_tool.set_selected_edge_parking_side(side_bit, enabled)
	_refresh()
	var refused := int(result.get("refused", 0))
	if refused > 0:
		# The verdict line is folded away with "Change road type"; the warning line always shows.
		_warning_label.text = RoadReasonText.text(result.get("verdict", {}), "%d road(s) refused the parking lane" % refused)
		_warning_label.show()

# The type picked in the picker, or {} for "Custom layout".
func _picked_type() -> Dictionary:
	var index := _type_picker.selected
	return _type_picker.get_item_metadata(index) if index >= 0 else {}

# The in-place arguments for `type`: lanes, median, kerbside parking, then speed, sidewalks, zoning
# and street lights (`RoadTypeMenu.tool_attributes`: a walkway sends the built-ins').
func _type_arguments(type: Dictionary) -> Array:
	return [int(type.get("fwd_lanes", 0)), int(type.get("bkw_lanes", 0)), int(type.get("median_style", 0)),
		float(type.get("median_width", 0.0)), int(type.get("parking", 0))] + RoadTypeMenu.tool_attributes(type, _speeds)

func _mark_type_tiles() -> void:
	for index in _type_tiles:
		HudIcons.mark(_type_tiles[index], index == _type_picker.selected)

func _on_type_selected(_index: int) -> void:
	_mark_type_tiles()
	var type := _picked_type()
	if type.is_empty() or _select_tool == null or not _select_tool.has_method("preview_selected_edge_cross_section"):
		return
	show_verdict(_select_tool.callv("preview_selected_edge_cross_section", _type_arguments(type)))

## Shows Rust's verdict on the picked type and enables Apply when it accepts it; Apply shows the
## cost difference Rust charges ("Apply (+1,230)").
func show_verdict(verdict: Dictionary) -> void:
	_verdict_label.show()
	_verdict = verdict
	var ready := bool(verdict.get("is_valid", false))
	_apply_button.disabled = not ready
	var cost := roundi(float(verdict.get("cost_delta", 0.0))) if ready else 0
	_apply_button.text = "Apply (+%s)" % UIStyle.thousands(cost) if cost > 0 else "Apply"
	_verdict_label.add_theme_color_override("font_color", VERDICT_READY if ready else VERDICT_REFUSED)
	_verdict_label.text = RoadTypeMenu.describe(_picked_type(), false) if ready \
		else RoadReasonText.text(verdict, "This road cannot change its type")

## The confirmation Apply asks for, or "" when the change demolishes nothing.
func demolition_text() -> String:
	var count := int(_verdict.get("buildings_removed", 0))
	if count <= 0:
		return ""
	return "Apply removes %d building%s. This cannot be undone." % [count, "" if count == 1 else "s"]

func _on_apply_pressed() -> void:
	var question := demolition_text()
	if question.is_empty():
		_apply()
		return
	_demolish_dialog.dialog_text = question
	_demolish_dialog.popup_centered()

# Commits the picked type on the selection (after the confirmation when it demolishes).
func _apply() -> void:
	var type := _picked_type()
	if not type.is_empty() and _select_tool and _select_tool.has_method("set_selected_edge_cross_section"):
		var refused: int = _select_tool.callv("set_selected_edge_cross_section", _type_arguments(type))
		_refresh()
		if refused > 0:
			_verdict_label.show()
			_verdict_label.add_theme_color_override("font_color", VERDICT_REFUSED)
			_verdict_label.text = "%d road(s) refused the change" % refused

func _on_class_button_pressed(class_index: int) -> void:
	if _select_tool and _select_tool.has_method("set_selected_edge_class"):
		_select_tool.set_selected_edge_class(class_index)
	_refresh()

func _on_no_build_toggled(enabled: bool) -> void:
	if _select_tool and _select_tool.has_method("set_selected_edge_no_building_spawn"):
		_select_tool.set_selected_edge_no_building_spawn(enabled)
