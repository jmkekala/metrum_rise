## Passive building inspector — fires whenever no other tool is active.
##
## Clicking any building in default (no-tool) mode opens a floating stats panel
## anchored to the left of the screen. The panel has an X button in the top-right
## corner to close it. Call close_window() to dismiss it programmatically when
## another tool activates.
extends Node

@onready var simulation_node = $"../SimulationNode"

var _panel: PanelContainer = null
var _title_label: Label = null
var _stats_body: VBoxContainer = null

func _ready() -> void:
	_build_panel()

# ── Panel construction ────────────────────────────────────────────────────────

func _build_panel() -> void:
	_panel = PanelContainer.new()
	_panel.visible = false
	_panel.custom_minimum_size = Vector2(290, 0)

	var style := StyleBoxFlat.new()
	style.bg_color = Color(0.08, 0.08, 0.12, 0.93)
	style.set_corner_radius_all(8)
	style.border_width_left = 1
	style.border_width_right = 1
	style.border_width_top = 1
	style.border_width_bottom = 1
	style.border_color = Color(0.3, 0.3, 0.45, 0.6)
	_panel.add_theme_stylebox_override("panel", style)

	# Left side, vertically centered
	_panel.set_anchors_and_offsets_preset(Control.PRESET_CENTER_LEFT)
	_panel.offset_left = 20
	_panel.offset_right = 310

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 12)
	margin.add_theme_constant_override("margin_right", 12)
	margin.add_theme_constant_override("margin_top", 10)
	margin.add_theme_constant_override("margin_bottom", 12)
	_panel.add_child(margin)

	var root_vbox := VBoxContainer.new()
	root_vbox.add_theme_constant_override("separation", 6)
	margin.add_child(root_vbox)

	# Header: building name on the left, X button on the right
	var header := HBoxContainer.new()
	root_vbox.add_child(header)

	_title_label = Label.new()
	_title_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_title_label.add_theme_font_size_override("font_size", 13)
	_title_label.add_theme_color_override("font_color", Color.WHITE)
	_title_label.clip_text = true
	header.add_child(_title_label)

	var close_btn := Button.new()
	close_btn.text = "X"
	close_btn.custom_minimum_size = Vector2(26, 26)
	close_btn.pressed.connect(close_window)
	header.add_child(close_btn)

	var sep := HSeparator.new()
	root_vbox.add_child(sep)

	_stats_body = VBoxContainer.new()
	_stats_body.add_theme_constant_override("separation", 4)
	root_vbox.add_child(_stats_body)

	# Attach to MainUI so the panel renders above the 3D world
	var main_ui := get_node_or_null("../MainUI")
	if main_ui:
		main_ui.add_child(_panel)
	else:
		# Fallback: add to scene root
		get_parent().add_child(_panel)

# ── Public API ────────────────────────────────────────────────────────────────

## Picks the nearest building at world_pos and opens the stats panel.
## Does nothing (and closes any open panel) when no building is within range.
func try_inspect(world_pos: Vector3) -> void:
	var info: Dictionary = simulation_node.get_building_info_at(world_pos.x, world_pos.z)
	if info.is_empty():
		close_window()
		return
	_populate(info)
	_panel.visible = true

## Hides the stats panel.
func close_window() -> void:
	if _panel:
		_panel.visible = false

# ── Panel population ──────────────────────────────────────────────────────────

func _populate(info: Dictionary) -> void:
	# Title: strip the pack prefix ("mypack:building_name" -> "building_name")
	var aid: String = info.get("asset_id", "Building")
	var colon := aid.find(":")
	_title_label.text = aid.substr(colon + 1) if colon != -1 else aid

	for child in _stats_body.get_children():
		child.queue_free()

	var zone: String = info.get("zone_type", "")
	_add_row("Type", _zone_label(zone) + "   Level " + str(info.get("level", 0)))

	# Residents only matter for residential; workers for everything else
	if zone == "residential":
		_add_row("Residents", str(info.get("occupancy", 0)))
	else:
		_add_section("Staff")
		_add_row("Workers", "%d / %d" % [info.get("worker_count", 0), info.get("worker_capacity", 0)])

	_add_section("Economy")
	_add_row("Profile", str(info.get("economy_profile", "-")))
	_add_row("Budget", "$%.1f" % float(info.get("operating_budget", 0.0)))
	_add_row("Revenue", "$%.1f" % float(info.get("revenue", 0.0)))
	_add_row("Utility", "Yes" if info.get("utility_service_available", false) else "No")

	var inv: Array = info.get("inventory", [])
	if inv.size() > 0:
		_add_section("Inventory")
		for entry in inv:
			_add_row(str(entry.get("name", "?")), "%.1f" % float(entry.get("amount", 0.0)))

	# Status flags — only shown when something is wrong
	var flags: Array[String] = []
	if info.get("broken", false):
		flags.append("Asset broken")
	if info.get("economy_broken", false):
		flags.append("Economy broken")
	if info.get("pending_redevelopment", false):
		flags.append("Rezone (%d d grace)" % info.get("rezone_grace_days", 0))
	if flags.size() > 0:
		_add_section("Alerts")
		for f in flags:
			_add_alert(f)

# ── Row helpers ───────────────────────────────────────────────────────────────

func _zone_label(zone: String) -> String:
	match zone:
		"residential": return "Residential"
		"commercial":  return "Commercial"
		"industrial":  return "Industrial"
		"utility":     return "Utility"
		_:             return zone.capitalize()

func _add_section(title: String) -> void:
	var sep := HSeparator.new()
	_stats_body.add_child(sep)
	var lbl := Label.new()
	lbl.text = title
	lbl.add_theme_color_override("font_color", Color(0.65, 0.65, 0.9))
	lbl.add_theme_font_size_override("font_size", 11)
	_stats_body.add_child(lbl)

func _add_row(label: String, value: String) -> void:
	var hbox := HBoxContainer.new()
	_stats_body.add_child(hbox)

	var lbl := Label.new()
	lbl.text = label
	lbl.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	lbl.add_theme_color_override("font_color", Color(0.72, 0.72, 0.72))
	lbl.add_theme_font_size_override("font_size", 12)
	hbox.add_child(lbl)

	var val := Label.new()
	val.text = value
	val.add_theme_color_override("font_color", Color.WHITE)
	val.add_theme_font_size_override("font_size", 12)
	hbox.add_child(val)

func _add_alert(text: String) -> void:
	var lbl := Label.new()
	lbl.text = "! " + text
	lbl.add_theme_color_override("font_color", Color(1.0, 0.4, 0.3))
	lbl.add_theme_font_size_override("font_size", 12)
	_stats_body.add_child(lbl)
