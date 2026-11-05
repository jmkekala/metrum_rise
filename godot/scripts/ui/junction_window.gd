# SPDX-License-Identifier: GPL-2.0-only

## Floating Junction window for the select tool's junction clicks (`ROAD-95`, round R5c2). It
## opens beside the click (not over the junction), is movable and resizable like Road
## Properties, and keeps the game's keyboard (Ctrl+Z undoes with it open). The lane spheres and
## crosswalk toggles stay in the world (the select tool draws them).
## Turn pockets: one row per incoming car road (`junction_pocket_row.gd`), in Rust's order
## (clockwise from north). Each row's Left / Right / Length ask Rust's verdict on that road end
## (no geometry preview, D11) and its Apply commits it ("Apply (+N)", Ctrl+Z undoes). The
## hovered row's road, else the last changed one's, is highlighted from the junction over the
## requested length. After an edit or an undo (a new render generation) the window reads the
## arms again and keeps each unapplied request whose arm is still there. A lane transition
## node or a node without car roads shows no Turn pockets section, only why.
## Rust methods called: get_junction_pockets(), preview_road_pocket(),
## get_network_render_generation(); through the select tool set_road_pocket().
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")
const JunctionPocketRow = preload("res://scripts/ui/junction_pocket_row.gd")
# Four rows fit without scrolling at the default size; more arms scroll.
const DEFAULT_SIZE := Vector2i(440, 480)
const MIN_SIZE := Vector2i(420, 300)

var _simulation_node: Node
var _select_tool: Node

var _window: Window
var _summary_label: Label
var _pocket_heading: Label
var _scroll: ScrollContainer
var _rows_box: VBoxContainer
var _rows: Array = []
# The junction shown, and the render generation its arms were read at.
var _node := -1
var _generation := -1
# The hovered row, and the key ("edge:end") of the row last changed: the highlighted road.
var _hovered := -1
var _last_changed_key := ""

func setup(simulation_node: Node, select_tool: Node) -> void:
	_simulation_node = simulation_node
	_select_tool = select_tool

func _ready() -> void:
	_build_window()

func _build_window() -> void:
	_window = Window.new()
	_window.title = "Junction"
	_window.unresizable = false
	_window.exclusive = false
	_window.visible = false
	_window.close_requested.connect(close_window)
	# The size persists; the position follows each click.
	UIStyle.set_persistent_window_layout(_window, "junction", DEFAULT_SIZE, MIN_SIZE, get_viewport(), false)
	UIStyle.keep_keyboard_with_game(_window)

	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	_window.add_child(body)
	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	for side in ["margin_left", "margin_right", "margin_top", "margin_bottom"]:
		margin.add_theme_constant_override(side, UIStyle.PAD_WINDOW)
	body.add_child(margin)
	var root_vbox := VBoxContainer.new()
	root_vbox.add_theme_constant_override("separation", 8)
	margin.add_child(root_vbox)

	# Shown only without rows.
	_summary_label = Label.new()
	_summary_label.text = "No turn pockets here: this is a pocket's lane transition or no road comes in. " \
		+ "Edit a pocket from its junction."
	_summary_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	UIStyle.set_font_size(_summary_label, 13)
	root_vbox.add_child(_summary_label)
	_pocket_heading = Label.new()
	_pocket_heading.text = "Turn pockets"
	_pocket_heading.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	UIStyle.set_font_size(_pocket_heading, 14)
	root_vbox.add_child(_pocket_heading)
	_scroll = ScrollContainer.new()
	_scroll.horizontal_scroll_mode = ScrollContainer.SCROLL_MODE_DISABLED
	_scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root_vbox.add_child(_scroll)
	_rows_box = VBoxContainer.new()
	_rows_box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_rows_box.add_theme_constant_override("separation", 6)
	_scroll.add_child(_rows_box)

	add_child(_window)
	WindowResizeHandles.install(_window)
	_window.hide()

## Shows junction `node_id`, clicked at `screen_pos`: opening beside the click, or staying where
## the player put it unless it would cover the click.
func show_for_node(node_id: int, screen_pos: Vector2 = Vector2.ZERO) -> void:
	_node = node_id
	_last_changed_key = ""
	_hovered = -1
	refresh()
	var covers := Rect2(Vector2(_window.position), Vector2(_window.size)).has_point(screen_pos)
	if not _window.visible or covers:
		UIStyle.place_window_near_cursor(_window, screen_pos, get_viewport())
	if not _window.visible:
		_window.popup()

## Hides the window and its road highlight.
func close_window() -> void:
	_node = -1
	if _window:
		UIStyle.save_persistent_window_layout(_window)
		_window.hide()
	if _select_tool:
		_select_tool.clear_pocket_end()

## Reads the junction's arms again (`get_junction_pockets`) and rebuilds the rows, keeping each
## unapplied request whose arm (edge and end) is still there.
func refresh() -> void:
	if _simulation_node == null or _node < 0:
		return
	_generation = _simulation_node.get_network_render_generation()
	var kept := {}
	for row in _rows:
		if row.has_request():
			kept[_key(row.info)] = row.request()
		# Hidden at once (out of the layout), freed after the signal that may be running.
		row.visible = false
		row.queue_free()
	_rows.clear()
	_hovered = -1
	var arms: Array = _simulation_node.get_junction_pockets(_node)
	for arm: Dictionary in arms:
		var row = JunctionPocketRow.new()
		row.show_arm(arm)
		var index := _rows.size()
		row.changed.connect(_on_row_changed.bind(index))
		row.apply_pressed.connect(_on_row_apply.bind(index))
		row.mouse_entered.connect(_on_row_hovered.bind(index, true))
		row.mouse_exited.connect(_on_row_hovered.bind(index, false))
		_rows_box.add_child(row)
		_rows.append(row)
		if kept.has(_key(arm)):
			row.set_request(kept[_key(arm)])
			_ask_verdict(index)
	_pocket_heading.visible = not _rows.is_empty()
	_scroll.visible = not _rows.is_empty()
	_summary_label.visible = _rows.is_empty()
	_update_highlight()

## The rows' count (for the tests and the stills).
func row_count() -> int:
	return _rows.size()

## Row `index`'s view.
func row(index: int):
	return _rows[index]

# After an edit or an undo publishes, the select tool redraws the junction's lanes and the rows
# are read again.
func _process(_delta: float) -> void:
	if _window == null or not _window.visible or _node < 0 or _simulation_node == null:
		return
	if _simulation_node.get_network_render_generation() == _generation:
		return
	_select_tool.refresh_junction_visuals()
	refresh()

func _key(arm: Dictionary) -> String:
	return "%d:%d" % [int(arm.get("edge", -1)), int(arm.get("end", -1))]

func _on_row_hovered(index: int, inside: bool) -> void:
	if inside:
		_hovered = index
	elif _hovered == index:
		_hovered = -1
	_update_highlight()

func _on_row_changed(_left: bool, _right: bool, _length_m: float, index: int) -> void:
	_last_changed_key = _key(_rows[index].info)
	_ask_verdict(index)
	_update_highlight()

# Rust's verdict on row `index`'s request, shown in the row.
func _ask_verdict(index: int) -> void:
	var row = _rows[index]
	var request: Array = row.request()
	var verdict: Dictionary = _simulation_node.preview_road_pocket(int(row.info.edge), int(row.info.end),
		request[0], request[1], request[2])
	var cost := roundi(float(verdict.get("cost_delta", 0.0)))
	var accepted := "Removes the turn pocket" if not request[0] and not request[1] \
		else JunctionPocketRow.describe(request[0], request[1], request[2])
	row.show_verdict(verdict, accepted, "Apply (+%s)" % UIStyle.thousands(cost) if cost > 0 else "Apply")

func _on_row_apply(index: int) -> void:
	var row = _rows[index]
	var request: Array = row.request()
	_last_changed_key = ""
	if _select_tool.set_junction_pocket(int(row.info.edge), int(row.info.end), request[0], request[1], request[2]):
		refresh()
		return
	# The verdict changed since the preview: show why now.
	_ask_verdict(index)

# The hovered row's road, else the last changed row's, from the junction over its requested
# length; nothing when neither is shown. Every row's border follows.
func _update_highlight() -> void:
	var index := _hovered
	if index < 0:
		for i in range(_rows.size()):
			if _key(_rows[i].info) == _last_changed_key:
				index = i
	for i in range(_rows.size()):
		_rows[i].set_highlighted(i == index)
	if _select_tool == null:
		return
	if index < 0:
		_select_tool.clear_pocket_end()
	else:
		_select_tool.show_pocket_end(_rows[index].info, float(_rows[index].request()[2]))
