# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: traffic_report.gd
#  script_path: godot/scripts/ui/traffic_report.gd
#  module_name: traffic_report
#  version: 0.1.0
#  description: The traffic report window. A congestion heatmap says where
#           traffic is bad; this says what held it there, which is the
#           question a player asks next. Reads the per-junction hold
#           tally the movement pass records and ranks the worst
#           junctions with the cause that dominates each.
#  kind: ui
#  spec: none
#  internal_dependencies: [ui_style.gd]
#  external_dependencies: [Godot 4.x]
#  features: [worst-junction ranking, dominant cause, per-cause breakdown]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-28
# =========================================================================

## Traffic report window: which junctions held cars, and why.
##
## Toggle with toggle(). Refreshes on the in-game hour boundary while open, so
## the steady-state cost is one query an hour rather than one a frame. A hold
## tally is per tick, so what is shown is the most recent completed movement
## pass rather than an average.
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const TITLE_FONT_SIZE := 16
const SECTION_FONT_SIZE := 13
const ROW_FONT_SIZE := 14

## How many junctions to list. The tail of a long list is junctions that held
## one car once, which is noise rather than a finding.
const MAX_ROWS := 12

@onready var simulation_node = $"../SimulationNode"

var _window: Window = null
var _body: VBoxContainer = null
var _title: Label = null
var _last_observed_hour: int = -1

func _process(_delta: float) -> void:
	if _window == null or not _window.visible or simulation_node == null:
		return

	var absolute_hour := _current_absolute_hour()
	if absolute_hour < 0 or absolute_hour == _last_observed_hour:
		return

	_last_observed_hour = absolute_hour
	_populate()

## Opens the report, or closes it when already open.
func toggle() -> void:
	if _window == null:
		_create_window()

	if _window.visible:
		_window.hide()
		return

	_populate()
	_window.popup_centered()

func _current_absolute_hour() -> int:
	if simulation_node == null:
		return -1
	var hour_of_day := floori(float(simulation_node.get_current_minute_of_day()) / 60.0)
	return int(simulation_node.get_current_day()) * 24 + hour_of_day

func _create_window() -> void:
	_window = Window.new()
	_window.title = "Traffic Report"
	_window.unresizable = false
	_window.exclusive = false
	_window.visible = false
	_window.close_requested.connect(func(): _window.hide())
	UIStyle.set_persistent_window_layout(
		_window,
		"traffic_report",
		Vector2i(460, 480),
		Vector2i(360, 300),
		get_viewport(),
		false
	)

	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	_window.add_child(body)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_right", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_top", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_bottom", UIStyle.PAD_WINDOW)
	body.add_child(margin)

	var root_vbox := VBoxContainer.new()
	root_vbox.add_theme_constant_override("separation", 6)
	margin.add_child(root_vbox)

	_title = Label.new()
	UIStyle.set_font_size(_title, TITLE_FONT_SIZE)
	_title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	_title.clip_text = true
	root_vbox.add_child(_title)

	root_vbox.add_child(HSeparator.new())

	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root_vbox.add_child(scroll)

	_body = VBoxContainer.new()
	_body.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_body.add_theme_constant_override("separation", 4)
	scroll.add_child(_body)

	add_child(_window)

func _populate() -> void:
	if _body == null or simulation_node == null:
		return

	for child in _body.get_children():
		child.queue_free()

	var rows: Array = simulation_node.get_traffic_report(MAX_ROWS)

	if rows.is_empty():
		_title.text = "Traffic is flowing"
		var label := Label.new()
		label.text = "No junction held a car on the last tick."
		label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
		UIStyle.set_font_size(label, ROW_FONT_SIZE)
		label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
		_body.add_child(label)
		return

	var total := 0
	for row in rows:
		total += int(row.get("total", 0))
	_title.text = "%d holds across %d junctions" % [total, rows.size()]

	for row in rows:
		_add_junction(row)

func _add_junction(row: Dictionary) -> void:
	var node_id := int(row.get("node_id", -1))
	var total := int(row.get("total", 0))
	var cause_label := str(row.get("cause_label", ""))
	var cause_count := int(row.get("cause_count", 0))

	_add_section(_body, "Junction %d" % node_id)

	# The finding, stated first: not that this junction is slow, but what is
	# making it slow. Everything below is the supporting breakdown.
	if cause_label != "":
		_add_row(_body, "Mostly", "%s (%d of %d)" % [cause_label, cause_count, total])
	else:
		_add_row(_body, "Holds", str(total))

	var causes: Dictionary = row.get("causes", {})
	for code in causes.keys():
		var n := int(causes[code])
		if n <= 0:
			continue
		_add_row(_body, "  %s" % _cause_name(int(code)), str(n))

	# What a way around would have cost. A car that priced an alternative and
	# stayed is the delay a player is looking at, and the two numbers say
	# whether a detour exists or whether every route out is equally bad.
	var declined := int(row.get("reroute_declined", 0))
	if declined > 0 and row.has("route_cost_s"):
		var cur := float(row.get("route_cost_s", 0.0))
		var alt := float(row.get("alternative_cost_s", 0.0))
		_add_row(_body, "  stayed on route", "%d cars" % declined)
		_add_row(_body, "  this route", "%.0f s" % cur)
		_add_row(_body, "  best alternative", "%.0f s" % alt)

	var taken := int(row.get("reroute_taken", 0))
	if taken > 0:
		_add_row(_body, "  rerouted away", "%d cars" % taken)

## Cause codes as the Rust side reports them. Kept here rather than inferred
## from `cause_label`, because only the dominant cause carries a label and the
## breakdown is keyed by code.
func _cause_name(code: int) -> String:
	match code:
		0: return "signal"
		1: return "priority sign"
		2: return "gave way"
		3: return "crossing traffic"
		4: return "connector busy"
		5: return "exit jammed"
		_: return "unknown"

func _add_section(parent: VBoxContainer, title: String) -> void:
	parent.add_child(HSeparator.new())
	var label := Label.new()
	label.text = title
	label.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	UIStyle.set_font_size(label, SECTION_FONT_SIZE)
	parent.add_child(label)

func _add_row(parent: VBoxContainer, label_text: String, value_text: String) -> void:
	var hbox := HBoxContainer.new()
	parent.add_child(hbox)

	var label := Label.new()
	label.text = label_text
	label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(label, ROW_FONT_SIZE)
	hbox.add_child(label)

	var value := Label.new()
	value.text = value_text
	value.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(value, ROW_FONT_SIZE)
	hbox.add_child(value)
