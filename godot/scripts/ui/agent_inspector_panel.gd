# SPDX-License-Identifier: GPL-2.0-only

## Agent inspector window: the inspected agent's trip, filled by `agent_inspector.gd`.
##
## A view only, built like the Building Inspector window: it opens beside the cursor, the
## player can drag and resize it, and its size is kept between sessions. It shows the
## dictionaries `get_agent_inspection` and `get_agent_route_lines` return. Home, work and
## destination are links (`building_pressed`); `follow_pressed` asks to follow the agent again,
## and the close button asks to end the inspection (`close_pressed`). The window holds the
## keyboard focus only while a mouse button is down inside it, so zoom, orbit and Esc keep
## reaching the game while it is open.
extends Window

## The close button was pressed.
signal close_pressed
## The Follow button was pressed.
signal follow_pressed
## A building link was pressed: `building` is the clicked building's dictionary.
signal building_pressed(building: Dictionary)

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")

const LAYOUT_ID := "agent_inspector"
const TITLE_FONT_SIZE := 16
const ROW_FONT_SIZE := 14
const ROWS := [
	"Mode", "State", "Speed", "Activity", "Home", "Work", "Destination", "Inside", "Lane",
	"Route", "Route end",
]
## Rows whose value is a link to the building.
const LINK_ROWS := ["Home", "Work", "Destination"]

var _title_label: Label
var _follow_button: Button
var _values := {}

func _init() -> void:
	title = "Agent Inspector"
	exclusive = false
	unresizable = false
	visible = false

func _ready() -> void:
	close_requested.connect(func() -> void: close_pressed.emit())
	UIStyle.keep_keyboard_with_game(self)
	UIStyle.set_persistent_window_layout(self, LAYOUT_ID, Vector2i(400, 380), Vector2i(300, 260), get_parent().get_viewport(), false)
	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	add_child(body)
	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	for side in ["margin_left", "margin_right", "margin_top", "margin_bottom"]:
		margin.add_theme_constant_override(side, UIStyle.PAD_WINDOW)
	body.add_child(margin)
	var column := VBoxContainer.new()
	column.add_theme_constant_override("separation", 6)
	margin.add_child(column)
	var header := HBoxContainer.new()
	column.add_child(header)
	_title_label = Label.new()
	_title_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_title_label.clip_text = true
	UIStyle.set_font_size(_title_label, TITLE_FONT_SIZE)
	_title_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	header.add_child(_title_label)
	_follow_button = Button.new()
	_follow_button.name = "FollowButton"
	_follow_button.text = "Follow"
	_follow_button.tooltip_text = "Move the camera back to the agent and follow it"
	_follow_button.focus_mode = Control.FOCUS_NONE
	_follow_button.visible = false
	_follow_button.pressed.connect(func() -> void: follow_pressed.emit())
	header.add_child(_follow_button)
	column.add_child(HSeparator.new())
	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	column.add_child(scroll)
	var grid := GridContainer.new()
	grid.columns = 2
	grid.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	grid.add_theme_constant_override("h_separation", 12)
	scroll.add_child(grid)
	for row in ROWS:
		var name_label := Label.new()
		name_label.text = row
		UIStyle.set_font_size(name_label, ROW_FONT_SIZE)
		name_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
		grid.add_child(name_label)
		grid.add_child(_value_control(row))
	WindowResizeHandles.install(self)

## Shows the window beside `screen_pos` (the mouse when zero).
func open_near(screen_pos: Vector2) -> void:
	UIStyle.place_window_near_cursor(self, screen_pos, get_parent().get_viewport())
	show()

## Hides the window and keeps its size for next time.
func close() -> void:
	if visible:
		UIStyle.save_persistent_window_layout(self)
	hide()

## Shows the Follow button while the camera is not following the agent.
func set_following(following: bool) -> void:
	_follow_button.visible = not following

## Fills the agent rows from `get_agent_inspection`.
func show_agent(details: Dictionary) -> void:
	var freight := " (freight)" if details.get("freight", false) else ""
	_title_label.text = "Agent %d%s" % [int(details.get("render_id", -1)), freight]
	var speed := float(details.get("speed_mps", 0.0))
	_set_row("Mode", String(details.get("mode", "")))
	_set_row("State", String(details.get("state", "")))
	_set_row("Speed", "%.1f m/s (%.0f km/h)" % [speed, speed * 3.6])
	_set_row("Activity", String(details.get("activity", "")))
	for row in LINK_ROWS:
		_set_link(row, details.get(row.to_lower(), {}))
	_set_row("Inside", building_text(details.get("inside", {})))
	var lane := int(details.get("lane", -1))
	var edge := int(details.get("lane_edge", -1))
	_set_row("Lane", "none" if lane < 0 else "%d on %s, %.1f of %.1f m" % [
		lane,
		"a junction" if edge < 0 else "edge %d" % edge,
		float(details.get("lane_distance_m", 0.0)),
		float(details.get("lane_length_m", 0.0)),
	])

## Fills the route rows from `get_agent_inspection` and `get_agent_route_lines`.
func show_route(details: Dictionary, route: Dictionary) -> void:
	_set_row("Route", "node %d of %d, %d lanes, %.0f m left" % [
		int(details.get("path_index", 0)),
		int(details.get("path_length", 0)),
		int(route.get("lanes", 0)),
		float(route.get("remaining_m", 0.0)),
	])
	_set_row("Route end", String(route.get("end", "")))

## The text shown for one building dictionary: `none`, or id, kind and profile.
static func building_text(building: Dictionary) -> String:
	if building.is_empty():
		return "none"
	var profile := String(building.get("profile", ""))
	var suffix := "" if profile.is_empty() else " (%s)" % profile
	return "%d %s%s" % [int(building["id"]), building.get("kind", ""), suffix]

## The title text, for tests.
func title_text() -> String:
	return _title_label.text

## The value shown in `row`, for tests.
func value_text(row: String) -> String:
	return _values[row].text

## The link button of `row` (`Home`, `Work` or `Destination`), for tests.
func link(row: String) -> LinkButton:
	return _values[row]

## Whether the Follow button shows, for tests.
func follow_offered() -> bool:
	return _follow_button.visible

func _value_control(row: String) -> Control:
	var value: Control
	if row in LINK_ROWS:
		var button := LinkButton.new()
		button.focus_mode = Control.FOCUS_NONE
		button.underline = LinkButton.UNDERLINE_MODE_ON_HOVER
		button.tooltip_text = "Show this building and open its inspector"
		button.pressed.connect(func() -> void: building_pressed.emit(button.get_meta("building", {})))
		value = button
	else:
		var label := Label.new()
		label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		value = label
	value.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(value, ROW_FONT_SIZE)
	_values[row] = value
	return value

func _set_row(row: String, text: String) -> void:
	_values[row].text = text

func _set_link(row: String, building: Dictionary) -> void:
	var button: LinkButton = _values[row]
	button.text = building_text(building)
	button.disabled = building.is_empty()
	button.set_meta("building", building)
