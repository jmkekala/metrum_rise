# SPDX-License-Identifier: GPL-2.0-only

## Building inspector: one card per inspected building, in the game's own chrome
## (`inspector_card.gd`: borderless, dragged by its header, × or Esc closes).
##
## Call try_inspect() with a world position to populate and show a per-building
## window. Only one inspector is open at a time: inspecting another building
## replaces it, and inspecting the same building again closes it. The open window
## refreshes only when the in-game hour changes, so the steady-state cost is O(1)
## per frame. The window never keeps the keyboard, so the camera can move while it is open.
##
## Layout, top to bottom: the header in the zone's accent (icon, zone, name, level pips, the
## business status pill and a one-line summary); alert banners (broken in red; deserted,
## rezoning and construction in yellow); a strip of KPI tiles with meters (a business's workers,
## production, stock fill and today's profit; a home's households, residents by age, supplies and
## money); then tabs of dense label/value grids: People, Money, Site, Power, Stock and Parking.
## A tab with nothing to show is left out, the last tab picked stays picked for the next
## building, and Parking carries a warning icon when places are rejected. The card fits its
## height to the open tab; the tabs sit above the content, so only the bottom edge moves.
##
## The inspected building is ringed on the map in the card's accent (`footprint_halo.gd`, from
## the info's `footprint`) until its card closes.
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const Card = preload("res://scripts/ui/inspector_card.gd")
const FootprintHalo = preload("res://scripts/renderers/footprint_halo.gd")

const SECTION_FONT_SIZE := 12
const ROW_FONT_SIZE := 13
## The card's width at UI scale 1; the height follows the content.
const CARD_WIDTH := 400
## Tab order; empty tabs are dropped per building.
const TABS: Array[String] = ["People", "Money", "Site", "Power", "Stock", "Parking"]
## Residents' age bands in the KPI meter and its legend.
const AGE_COLORS := [Color(0.98, 0.80, 0.36), Color(0.42, 0.80, 0.95), Color(0.78, 0.62, 0.95)]

# The tab the player last picked, kept for the next building inspected this session.
static var _last_tab := "People"

@onready var simulation_node = $"../SimulationNode"

var _open_windows: Dictionary = {}
var _last_observed_hour: int = -1
var _halo := FootprintHalo.new()

func _ready() -> void:
	add_child(_halo)

func _process(_delta: float) -> void:
	if _open_windows.is_empty() or simulation_node == null:
		return

	var absolute_hour := _current_absolute_hour()
	if absolute_hour < 0 or absolute_hour == _last_observed_hour:
		return

	_last_observed_hour = absolute_hour
	_refresh_open_windows()

## Opens the inspector for the building at `world_pos`: with `click`, the building whose
## footprint holds it (`pick_building_at`; nothing on a road or bare ground), otherwise the
## building whose lot center it is (a link). Inspecting the open building again closes it,
## unless `toggle` is false (a link to the building), which keeps it open.
func try_inspect(world_pos: Vector3, screen_pos: Vector2 = Vector2.ZERO, toggle := true, click := true) -> bool:
	var info := {}
	if click:
		# A click inspects the building it lands on, never the nearest one beside it.
		var center = simulation_node.pick_building_at(world_pos.x, world_pos.z)
		if center != null:
			info = simulation_node.get_building_info_at(center.x, center.y)
	else:
		info = simulation_node.get_building_info_at(world_pos.x, world_pos.z)
	if info.is_empty():
		close_window()
		return false

	var key := _building_key(info)
	var entry: Dictionary = _open_windows.get(key, {})
	if not entry.is_empty():
		if toggle:
			_close_entry(key)
		return true

	close_window()
	entry = _create_window_entry(key)
	_open_windows[key] = entry

	_populate(entry, info)
	_update_anchor(entry, info)

	var window: Window = entry["window"]
	if entry.has("body"):
		_fit_window(window, entry["body"])
	if screen_pos == Vector2.ZERO:
		screen_pos = get_viewport().get_mouse_position()
	entry["cursor"] = screen_pos
	_place_window_near_cursor(window, screen_pos)
	window.show()
	_last_observed_hour = _current_absolute_hour()
	return true

## Whether the inspector currently owns a selection, used by the two-stage Escape action.
func is_inspecting() -> bool:
	return not _open_windows.is_empty()

func close_window() -> void:
	var keys := _open_windows.keys()
	for key_variant in keys:
		_close_entry(str(key_variant))

func _refresh_open_windows() -> void:
	var keys := _open_windows.keys()
	for key_variant in keys:
		var key := str(key_variant)
		var entry: Dictionary = _open_windows.get(key, {})
		if entry.is_empty():
			continue

		var anchor_pos: Vector3 = entry.get("anchor_pos", Vector3.ZERO)
		var info: Dictionary = simulation_node.get_building_info_at(anchor_pos.x, anchor_pos.z)
		if info.is_empty():
			_close_entry(key)
			continue

		_populate(entry, info)
		_update_anchor(entry, info)

func _current_absolute_hour() -> int:
	var hour_of_day := floori(float(simulation_node.get_current_minute_of_day()) / 60.0)
	return int(simulation_node.get_current_day()) * 24 + hour_of_day

func _create_window_entry(key: String) -> Dictionary:
	var window := Window.new()
	window.title = "Building"
	window.exclusive = false
	window.visible = false
	window.close_requested.connect(_close_entry.bind(key))
	# The camera keys, zoom and Esc keep working while the window is open.
	UIStyle.keep_keyboard_with_game(window)
	var parts := Card.build(window)
	UIStyle.set_window_base_size(window, Vector2i(CARD_WIDTH, 200), Vector2i(CARD_WIDTH, 120), get_viewport())
	add_child(window)
	window.hide()

	var entry := {
		"window": window,
		"body": window.get_child(0),
		"parts": parts,
		"title_label": parts["title"],
		"stats_body": parts["content"],
		"anchor_pos": Vector3.ZERO,
	}
	return entry

# The card's height follows the open tab, capped to the screen. Labels never autowrap, so the
# minimum size is right in the same frame and the window is placed at its real size.
func _fit_window(window: Window, body: Control) -> void:
	if not is_instance_valid(window) or not is_instance_valid(body):
		return
	var viewport_size := get_viewport().get_visible_rect().size
	var wanted := body.get_combined_minimum_size()
	window.size = Vector2i(
		maxi(window.min_size.x, int(wanted.x)),
		mini(int(wanted.y), int(viewport_size.y * 0.9))
	)
	# A taller tab must not hang off the bottom of the screen.
	UIStyle.keep_window_bottom_on_screen(window, get_viewport())

func _close_entry(key: String) -> void:
	var entry: Dictionary = _open_windows.get(key, {})
	if entry.is_empty():
		return

	_open_windows.erase(key)
	_halo.hide_halo()
	var window: Window = entry["window"]
	if is_instance_valid(window):
		window.queue_free()

func _update_anchor(entry: Dictionary, info: Dictionary) -> void:
	entry["anchor_pos"] = Vector3(
		float(info.get("center_x", 0.0)),
		0.0,
		float(info.get("center_z", 0.0))
	)

func _building_key(info: Dictionary) -> String:
	return "%.3f:%.3f" % [
		float(info.get("center_x", 0.0)),
		float(info.get("center_z", 0.0)),
	]

func _display_name(info: Dictionary) -> String:
	var asset_display_name := str(info.get("asset_display_name", "")).strip_edges()
	if not asset_display_name.is_empty():
		return asset_display_name
	var asset_id: String = info.get("asset_id", "Building")
	var colon := asset_id.find(":")
	return asset_id.substr(colon + 1) if colon != -1 else asset_id

func _place_window_near_cursor(window: Window, screen_pos: Vector2) -> void:
	UIStyle.place_window_near_cursor(window, screen_pos, get_viewport())

func _populate(entry: Dictionary, info: Dictionary) -> void:
	entry["info"] = info
	var title_label: Label = entry["title_label"]
	var stats_body: VBoxContainer = entry["stats_body"]
	var zone := str(info.get("zone_type", ""))
	var accent: Color = Card.ACCENTS.get(zone, UIStyle.HUD_ACCENT)
	_halo.show_footprint(info.get("footprint", PackedVector3Array()), accent)
	title_label.text = _display_name(info)
	var parts: Dictionary = entry.get("parts", {})
	if not parts.is_empty():
		Card.set_accent(parts, accent)
		HudIcons.set_icon(parts["icon"], _icon_name(info))
		(parts["eyebrow"] as Label).text = _eyebrow(info)
		_fill_chips(parts["chips"], parts["subtitle"], info, accent)

	for child in stats_body.get_children():
		stats_body.remove_child(child)
		child.queue_free()

	_add_alerts(stats_body, info)
	_add_kpis(stats_body, info, accent)

	var pages := {}
	for title in TABS:
		pages[title] = _new_page(title)
	_fill_people(pages["People"], info)
	_fill_money(pages["Money"], info)
	if info.has("extractor_resource"):
		_add_extractor_reserve_section(pages["Site"], info)
	if info.has("field_resource"):
		_add_field_section(pages["Site"], info)
	_fill_power(pages["Power"], info)
	_fill_stock(pages["Stock"], info)
	_row(pages["Parking"], "Valid places", str(info.get("parking_valid_places", 0)))
	var parking_rejected: PackedStringArray = info.get("parking_rejected", PackedStringArray())
	for reason in parking_rejected:
		_add_alert(pages["Parking"], reason, false)

	var tabs := TabContainer.new()
	_style_tabs(tabs, accent)
	stats_body.add_child(tabs)
	for title in TABS:
		var page: VBoxContainer = pages[title]
		_flush(page)
		if page.get_child_count() == 0:
			page.free()
			continue
		tabs.add_child(page)
		if title == "Parking" and not parking_rejected.is_empty():
			var index := tabs.get_tab_count() - 1
			tabs.set_tab_icon(index, load(HudIcons.ICON_DIR + "warning.svg") as Texture2D)
			tabs.set_tab_icon_max_width(index, int(UIStyle.scaled_px(13.0)))
	var picked := tabs.get_node_or_null(NodePath(_last_tab))
	tabs.current_tab = tabs.get_tab_idx_from_control(picked) if picked else 0
	tabs.tab_changed.connect(_on_tab_changed.bind(entry, tabs))
	if entry.has("body"):
		_fit_window(entry["window"], entry["body"])

func _on_tab_changed(index: int, entry: Dictionary, tabs: TabContainer) -> void:
	_last_tab = tabs.get_tab_title(index)
	if entry.has("body"):
		_fit_window.call_deferred(entry["window"], entry["body"])

# ── Header ──────────────────────────────────────────────────────────────────

# "Industrial", or a utility's service ("Power utility").
func _eyebrow(info: Dictionary) -> String:
	var service := str(info.get("utility_service", ""))
	if not service.is_empty():
		return "%s utility" % service.capitalize()
	if info.has("field_resource"):
		return "Farm"
	return _zone_label(str(info.get("zone_type", "")))

# Level pips, the status pill and a one-line summary.
func _fill_chips(chips: HBoxContainer, summary: Label, info: Dictionary, accent: Color) -> void:
	for child in chips.get_children():
		if child != summary:
			chips.remove_child(child)
			child.queue_free()
	var pips := Card.level_pips(int(info.get("level", 0)), accent)
	chips.add_child(pips)
	chips.move_child(pips, 0)
	if info.get("business_summary", false):
		var status := str(info.get("business_status", "-"))
		var pill := Card.pill(status, _status_color(status))
		chips.add_child(pill)
		chips.move_child(pill, 1)
	summary.text = _summary(info)

func _status_color(status: String) -> Color:
	var lower := status.to_lower()
	if lower.begins_with("run") or lower.begins_with("open") or lower.begins_with("active"):
		return Card.GOOD
	if lower.contains("bankrupt") or lower.contains("closed") or lower.contains("broken"):
		return Card.BAD
	return UIStyle.SIGN_YELLOW

func _summary(info: Dictionary) -> String:
	# A home's households are a KPI tile; a business's budget is said only here.
	if str(info.get("zone_type", "")) == "residential":
		return ""
	return "Budget %s" % _money(float(info.get("operating_budget", 0.0)))

# A utility's service icon, a farm's field, an extractor's pit, else the zone's.
func _icon_name(info: Dictionary) -> String:
	var candidates: Array[String] = [str(info.get("utility_service", ""))]
	if info.has("field_resource"):
		candidates.append("field")
	if info.has("extractor_resource"):
		candidates.append("extractor")
	candidates.append(str(info.get("zone_type", "")))
	for icon_name in candidates:
		if not icon_name.is_empty() and ResourceLoader.exists(HudIcons.ICON_DIR + icon_name + ".svg"):
			return icon_name
	return "services"

# ── KPI strip ───────────────────────────────────────────────────────────────

func _add_kpis(stats_body: VBoxContainer, info: Dictionary, accent: Color) -> void:
	var strip := HBoxContainer.new()
	strip.add_theme_constant_override("separation", 6)
	stats_body.add_child(strip)
	if str(info.get("zone_type", "")) == "residential":
		_add_home_kpis(strip, info, accent)
		stats_body.add_child(_age_legend(info))
	else:
		_add_business_kpis(strip, info, accent)

func _add_business_kpis(strip: HBoxContainer, info: Dictionary, accent: Color) -> void:
	var workers := int(info.get("worker_count", 0))
	var capacity := int(info.get("business_active_worker_capacity", info.get("worker_capacity", 0)))
	var is_power := str(info.get("utility_service", "")) == "power"
	var staffing := float(workers) / float(capacity) if capacity > 0 else 0.0
	strip.add_child(Card.kpi("Workers", str(workers) if is_power or capacity <= 0 else "%d / %d" % [workers, capacity],
		staffing if capacity > 0 else -1.0, accent))
	if not info.get("business_summary", false):
		return
	var production := float(info.get("business_production_ratio", 0.0))
	strip.add_child(Card.kpi("Output", "%.0f%%" % (production * 100.0), production, accent))
	if is_power:
		var produced := float(info.get("utility_power_production_today", 0.0))
		var consumed := float(info.get("utility_power_consumed_today", 0.0))
		strip.add_child(Card.kpi("Used", _electricity_quantity(consumed, "kWh", "MWh"),
			consumed / produced if produced > 0.0 else 0.0, UIStyle.HUD_ACCENT,
			UIStyle.TEXT_PRIMARY, "%s produced today" % _electricity_quantity(produced, "kWh", "MWh")))
	elif info.get("business_has_inventory_fill", false):
		var fill := float(info.get("business_inventory_fill_ratio", 0.0))
		strip.add_child(Card.kpi("Stock", "%.0f%%" % (fill * 100.0), fill, Color(0.80, 0.66, 0.46)))
	var today := float(info.get("business_profit_today", 0.0))
	strip.add_child(Card.kpi("Today", _signed_money(today), -1.0, accent,
		Card.GOOD if today >= 0.0 else Card.BAD,
		"Yesterday %s" % _signed_money(float(info.get("business_profit_yesterday", 0.0)))))

# Households, residents (a meter split by age, with a legend under the strip), supplies, money.
func _add_home_kpis(strip: HBoxContainer, info: Dictionary, accent: Color) -> void:
	var ages := [int(info.get("child_count", 0)), int(info.get("adult_count", 0)), int(info.get("elder_count", 0))]
	var residents: int = ages[0] + ages[1] + ages[2]
	strip.add_child(Card.kpi("Households", str(info.get("household_count", info.get("occupancy", 0)))))
	var residents_tile := Card.kpi("Residents", str(residents))
	if residents > 0:
		var segments := []
		for i in 3:
			segments.append([float(ages[i]) / float(residents), AGE_COLORS[i]])
		residents_tile.get_child(0).add_child(Card.meter(segments, 4))
	strip.add_child(residents_tile)
	var days := float(info.get("household_stock_days_avg", 0.0))
	var min_days := float(info.get("household_stock_days_min", 0.0))
	strip.add_child(Card.kpi("Supplies", "%.1f d" % days, clampf(min_days / 7.0, 0.0, 1.0),
		Card.GOOD if min_days >= 2.0 else UIStyle.SIGN_YELLOW, UIStyle.TEXT_PRIMARY,
		"%.1f days on average, %.1f at the lowest household" % [days, min_days]))
	strip.add_child(Card.kpi("Money", _money(float(info.get("household_budget_avg", 0.0))), -1.0, accent,
		UIStyle.TEXT_PRIMARY, "Average per household; %s in all" % _money(float(info.get("household_budget_total", 0.0)))))

# The residents meter's key: a dot and a count per age band.
func _age_legend(info: Dictionary) -> HBoxContainer:
	var legend := HBoxContainer.new()
	legend.add_theme_constant_override("separation", 5)
	var counts := [int(info.get("child_count", 0)), int(info.get("adult_count", 0)), int(info.get("elder_count", 0))]
	for i in 3:
		var dot := ColorRect.new()
		dot.custom_minimum_size = UIStyle.scaled_vector2(Vector2(7, 7))
		dot.size_flags_vertical = Control.SIZE_SHRINK_CENTER
		dot.color = AGE_COLORS[i]
		legend.add_child(dot)
		var text := Card._label("%d %s" % [counts[i], ["children", "adults", "elders"][i]], 12, UIStyle.TEXT_DIM)
		text.custom_minimum_size.x = UIStyle.scaled_px(78.0)
		legend.add_child(text)
	return legend

# ── Tabs ────────────────────────────────────────────────────────────────────

func _new_page(title: String) -> VBoxContainer:
	var page := VBoxContainer.new()
	page.name = title
	page.add_theme_constant_override("separation", 6)
	page.set_meta("pairs", [])
	return page

# A label/value pair, laid out with its neighbours in one dense grid by `_flush`.
func _row(page: VBoxContainer, label_text: String, value_text: String, tone := UIStyle.TEXT_PRIMARY) -> void:
	(page.get_meta("pairs") as Array).append([label_text, value_text, tone])

# Lays out the pending pairs before the next control (a heading, meter, slider or button).
func _flush(page: VBoxContainer) -> void:
	var pairs: Array = page.get_meta("pairs")
	if pairs.is_empty():
		return
	page.add_child(Card.grid(pairs.duplicate()))
	pairs.clear()

func _add_control(page: VBoxContainer, control: Control) -> void:
	_flush(page)
	page.add_child(control)

# Pill tabs: the picked one filled with the accent, the rest dim text on the card.
func _style_tabs(tabs: TabContainer, accent: Color) -> void:
	tabs.get_tab_bar().focus_mode = Control.FOCUS_NONE
	tabs.tab_alignment = TabBar.ALIGNMENT_LEFT
	UIStyle.set_font_size(tabs, 12)
	var panel := StyleBoxEmpty.new()
	panel.content_margin_top = 10
	tabs.add_theme_stylebox_override("panel", panel)
	var bar := StyleBoxFlat.new()
	bar.bg_color = Card.WELL_BG
	bar.set_corner_radius_all(9)
	bar.set_content_margin_all(3)
	tabs.add_theme_stylebox_override("tabbar_background", bar)
	var styles := {
		"tab_unselected": Color.TRANSPARENT,
		"tab_hovered": Color(1, 1, 1, 0.07),
		"tab_selected": Color(accent, 0.22),
	}
	for state: String in styles:
		var style := StyleBoxFlat.new()
		style.bg_color = styles[state]
		style.set_corner_radius_all(7)
		style.content_margin_left = 9
		style.content_margin_right = 9
		style.content_margin_top = 3
		style.content_margin_bottom = 4
		tabs.add_theme_stylebox_override(state, style)
	tabs.add_theme_stylebox_override("tab_focus", StyleBoxEmpty.new())
	tabs.add_theme_color_override("font_selected_color", UIStyle.TEXT_PRIMARY)
	tabs.add_theme_color_override("font_hovered_color", UIStyle.TEXT_PRIMARY)
	tabs.add_theme_color_override("font_unselected_color", UIStyle.TEXT_DIM)
	tabs.add_theme_constant_override("icon_separation", 4)

# Broken assets and economies in red; deserted, rezoning and construction in yellow.
func _add_alerts(stats_body: VBoxContainer, info: Dictionary) -> void:
	if info.get("broken", false):
		_add_alert(stats_body, "Asset broken", true)
	if info.get("economy_broken", false):
		_add_alert(stats_body, "Economy broken", true)
	if info.get("is_deserted", false):
		_add_alert(stats_body, "Deserted", false)
	if info.get("pending_redevelopment", false):
		_add_alert(stats_body, "Rezoned: redevelops in %d d" % info.get("rezone_grace_days", 0), false)
	if info.get("under_construction", false):
		_add_alert(
			stats_body,
			"Under construction: %d h left (%.0f%%)" % [
				int(info.get("construction_remaining_hours", 0)),
				float(info.get("construction_progress", 0.0)) * 100.0,
			],
			false
		)

# Residents, staffing in detail and the utility's state and funding.
func _fill_people(page: VBoxContainer, info: Dictionary) -> void:
	var zone: String = info.get("zone_type", "")
	var is_farm := info.has("field_resource")
	# A home's households and ages are the KPI strip and its legend already.
	if is_farm:
		_row(page, "Farm Household", "%d / %d" % [int(info.get("occupancy", 0)), int(info.get("household_capacity", 1))])
		_row(page, "Children", str(info.get("child_count", 0)))
		_row(page, "Adults", str(info.get("adult_count", 0)))
		_row(page, "Elders", str(info.get("elder_count", 0)))
	if zone == "residential":
		return
	# Workers and output are KPIs; the tab adds only a reduced shift and the utility's controls.
	var is_power_utility := str(info.get("utility_service", "")) == "power"
	var max_capacity := int(info.get("worker_capacity", 0))
	var active_capacity := int(info.get("business_active_worker_capacity", max_capacity))
	if not is_power_utility and info.get("business_summary", false) and active_capacity != max_capacity:
		_row(page, "Job slots", "%d active of %d" % [active_capacity, max_capacity])
	if info.get("business_summary", false):
		if info.has("utility_service"):
			var utility_name := str(info.get("utility_service", "")).capitalize()
			var utility_state := "active" if info.get("utility_service_available", false) else "inactive"
			_row(page, "Utility", "%s %s" % [utility_name, utility_state])
			if is_power_utility:
				_add_service_funding_slider(page, info)
	if info.has("utility_service_available") and not info.has("utility_service"):
		_row(page, "Utility", "Yes" if info["utility_service_available"] else "No")

# What the header and KPIs leave out: a business's profit yesterday and a utility's flows, then
# the residents' money and supplies (a home's averages are KPIs, so only the rest here).
func _fill_money(page: VBoxContainer, info: Dictionary) -> void:
	var zone: String = info.get("zone_type", "")
	if zone != "residential":
		if info.get("business_summary", false):
			var yesterday := float(info.get("business_profit_yesterday", 0.0))
			_row(page, "Yesterday", _signed_money(yesterday), Card.GOOD if yesterday >= 0.0 else Card.BAD)
			if info.has("utility_service"):
				_row(page, "Utility Revenue", _money(float(info.get("utility_local_revenue", 0.0))))
				_row(page, "City Inputs Today", _money(float(info.get("city_input_cost_today", 0.0))))
	if zone != "residential" and not info.has("field_resource"):
		return
	if not (page.get_meta("pairs") as Array).is_empty():
		_add_section(page, "Households")
	var total := _money(float(info.get("household_budget_total", 0.0)))
	var lowest := "%.1f d" % float(info.get("household_stock_days_min", 0.0))
	if zone == "residential":
		_row(page, "Money in all", total)
		_row(page, "Lowest supplies", lowest)
	else:
		_row(page, "Household Money", "%s, %s average" % [total, _money(float(info.get("household_budget_avg", 0.0)))])
		_row(page, "Supplies", "%.1f d average, %s lowest" % [float(info.get("household_stock_days_avg", 0.0)), lowest])
	_row(page, "Stored Supplies", "%.1f kg" % float(info.get("household_stock_total", 0.0)))
	_row(page, "Replenishment", str(info.get("household_replenishment_state", "-")))
	if int(info.get("household_replenishment_active", 0)) > 0:
		_row(page, "Active Replenishment", str(info.get("household_replenishment_active", 0)))

# A power plant's output first, then the building's own use over the last day.
func _fill_power(page: VBoxContainer, info: Dictionary) -> void:
	if info.get("business_summary", false) and str(info.get("utility_service", "")) == "power":
		var power_produced := float(info.get("utility_power_production_today", 0.0))
		var power_consumed := float(info.get("utility_power_consumed_today", 0.0))
		_add_section(page, "Plant today")
		_row(page, "Produced", _electricity_quantity(power_produced, "kWh", "MWh"))
		if info.has("power_staffed_capacity_kw"):
			_row(page, "Staffed", _electricity_quantity(float(info["power_staffed_capacity_kw"]), "kW", "MW"))
		_add_power_consumption_bar(page, power_consumed, power_produced)
	elif info.has("power_staffed_capacity_kw"):
		_row(page, "Staffed capacity", _electricity_quantity(float(info["power_staffed_capacity_kw"]), "kW", "MW"))
	# A heading only to set the building's own use apart from a plant's output above it.
	if info.get("business_summary", false) and str(info.get("utility_service", "")) == "power":
		_add_section(page, "Own use")
	_row(page, "Used yesterday", _electricity_quantity(float(info.get("electricity_consumption_kwh", 0.0)), "kWh", "MWh"))
	_row(page, "Local supply", _electricity_quantity(float(info.get("electricity_local_kwh", 0.0)), "kWh", "MWh"))
	_row(page, "Imports", _electricity_quantity(float(info.get("electricity_imported_kwh", 0.0)), "kWh", "MWh"))
	_row(page, "Cost", _money(float(info.get("electricity_cost", 0.0))))
	if float(info.get("household_power_arrears", 0.0)) > 0.0:
		_row(page, "Unpaid bills", _money(float(info["household_power_arrears"])), Card.BAD)

# Each resource's stock, with its daily use at full operation when it is an input.
func _fill_stock(page: VBoxContainer, info: Dictionary) -> void:
	var inventory: Array = info.get("inventory", [])
	for item in inventory:
		var unit := str(item.get("unit", ""))
		var stock_text := "%.1f %s" % [float(item.get("amount", 0.0)), unit]
		if item.has("daily_input_units"):
			stock_text += " · uses %.1f %s/day" % [float(item["daily_input_units"]), unit]
		_row(page, _resource_label(str(item.get("name", "?"))), stock_text)

# ── Formatting ──────────────────────────────────────────────────────────────

func _zone_label(zone: String) -> String:
	match zone:
		"residential": return "Residential"
		"commercial": return "Commercial"
		"industrial": return "Industrial"
		"utility": return "Utility"
		_: return zone.capitalize()

func _electricity_quantity(amount: float, small_unit: String, large_unit: String) -> String:
	if absf(amount) >= 1000.0:
		return "%.1f %s" % [amount / 1000.0, large_unit]
	return "%.1f %s" % [amount, small_unit]

func _money(amount: float) -> String:
	return "$%s" % UIStyle.thousands(roundi(amount))

func _signed_money(amount: float) -> String:
	var sign := "+" if amount >= 0.0 else "−"
	return "%s$%s" % [sign, UIStyle.thousands(roundi(absf(amount)))]

func _resource_label(resource_id: String) -> String:
	return resource_id.replace("_", " ").capitalize()

func _resource_units(amount: float) -> String:
	if absf(amount) >= 1000.0:
		return "%.1f t" % (amount / 1000.0)
	return "%.1f kg" % amount

# A small caps heading inside a tab.
func _add_section(page: VBoxContainer, title: String) -> void:
	_add_control(page, Card.caption(title))

func _add_extractor_reserve_section(page: VBoxContainer, info: Dictionary) -> void:
	_add_section(page, "Extraction pit")
	var resource_name: String = _resource_label(str(info.get("extractor_resource", "resource")))
	if not bool(info.get("extractor_has_site", false)):
		_row(page, "Pit", "No polygon", UIStyle.SIGN_YELLOW)
		return

	var available := maxf(float(info.get("extractor_available_reserve_units", 0.0)), 0.0)
	var consumed := maxf(float(info.get("extractor_consumed_reserve_units", 0.0)), 0.0)
	var total := maxf(float(info.get("extractor_total_reserve_units", 0.0)), 0.0)
	var consumed_ratio := clampf(float(info.get("extractor_consumed_reserve_ratio", 0.0)), 0.0, 1.0)
	var area_m2 := maxf(float(info.get("extractor_area_m2", 0.0)), 0.0)
	_row(page, "%s area" % resource_name, "%s m²" % UIStyle.thousands(roundi(area_m2)))
	_row(page, "Reserve", _resource_units(total))
	_row(page, "Available", _resource_units(available))
	_row(page, "Consumed", _resource_units(consumed))
	_add_reserve_consumption_bar(page, consumed_ratio, consumed, total)

func _add_field_section(page: VBoxContainer, info: Dictionary) -> void:
	_add_section(page, "Field")
	var resource_name: String = _resource_label(str(info.get("field_resource", "resource")))
	if not bool(info.get("field_has_site", false)):
		_row(page, "Field", "No polygon", UIStyle.SIGN_YELLOW)
		return

	var area_m2 := maxf(float(info.get("field_area_m2", 0.0)), 0.0)
	_row(page, "%s area" % resource_name, "%s m²" % UIStyle.thousands(roundi(area_m2)))
	var edit_button := Button.new()
	edit_button.text = "Edit Field"
	edit_button.focus_mode = Control.FOCUS_NONE
	UIStyle.style_primary_button(edit_button)
	UIStyle.set_font_size(edit_button, 12)
	edit_button.pressed.connect(func():
		get_node("../InputManager").edit_farm_field(info)
	)
	_add_control(page, edit_button)

func apply_field_edit(building_id: int, details: Dictionary) -> void:
	# Merge the bounded edit response; avoid querying all occupants on every release.
	for entry: Dictionary in _open_windows.values():
		var info: Dictionary = entry.get("info", {})
		if int(info.get("building_id", -1)) != building_id:
			continue
		if info.get("center_x") != details.get("center_x") or info.get("center_z") != details.get("center_z"):
			continue
		info.merge(details, true)
		_populate(entry, info)

func _add_service_funding_slider(page: VBoxContainer, info: Dictionary) -> void:
	var hbox := HBoxContainer.new()
	hbox.add_theme_constant_override("separation", 8)

	var label := Card._label("Plant funding", ROW_FONT_SIZE - 1, UIStyle.TEXT_DIM)
	label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	hbox.add_child(label)

	var slider := HSlider.new()
	slider.min_value = 0.0
	slider.max_value = 1.0
	slider.step = 0.01
	slider.focus_mode = Control.FOCUS_NONE
	slider.value = clampf(float(info.get("service_funding_effective", 1.0)), 0.0, 1.0)
	slider.custom_minimum_size = UIStyle.scaled_vector2(Vector2(120.0, 0.0))
	slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	slider.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	hbox.add_child(slider)

	var value := Card._label("%.0f%%" % (slider.value * 100.0), ROW_FONT_SIZE, UIStyle.TEXT_PRIMARY)
	value.custom_minimum_size = UIStyle.scaled_vector2(Vector2(38.0, 0.0))
	value.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	hbox.add_child(value)

	slider.value_changed.connect(
		_on_service_funding_slider_changed.bind(
			value,
			float(info.get("center_x", 0.0)),
			float(info.get("center_z", 0.0)),
			str(info.get("utility_service", ""))
		)
	)
	slider.drag_ended.connect(_on_service_funding_slider_drag_ended)
	_add_control(page, hbox)

# "Pit used" with a meter of the reserve consumed.
func _add_reserve_consumption_bar(
	page: VBoxContainer,
	consumed_ratio: float,
	_consumed_units: float,
	_total_units: float
) -> void:
	_add_control(page, _labelled_meter("Pit used", "%.0f%%" % (consumed_ratio * 100.0),
		[[consumed_ratio, Color(0.80, 0.62, 0.40)]]))

func _on_service_funding_slider_changed(
	new_value: float,
	value_label: Label,
	center_x: float,
	center_z: float,
	service_id: String
) -> void:
	if is_instance_valid(value_label):
		value_label.text = "%.0f%%" % (new_value * 100.0)
	if simulation_node != null:
		simulation_node.set_building_service_funding_override_at(
			center_x,
			center_z,
			service_id,
			new_value
		)

func _on_service_funding_slider_drag_ended(_value_changed: bool) -> void:
	call_deferred("_refresh_open_windows")

# "Used" with a meter of the energy consumed out of the energy produced.
func _add_power_consumption_bar(page: VBoxContainer, consumed_units: float, produced_units: float) -> void:
	produced_units = maxf(produced_units, 0.0)
	consumed_units = clampf(consumed_units, 0.0, produced_units)
	var ratio := 0.0
	if produced_units > 0.0:
		ratio = clampf(consumed_units / produced_units, 0.0, 1.0)
	_add_control(page, _labelled_meter("Used", "%s / %s" % [
		_electricity_quantity(consumed_units, "kWh", "MWh"),
		_electricity_quantity(produced_units, "kWh", "MWh"),
	], [[ratio, UIStyle.HUD_ACCENT]]))

# A caption and value on one line over a full-width meter.
func _labelled_meter(caption: String, value_text: String, segments: Array) -> VBoxContainer:
	var box := VBoxContainer.new()
	box.add_theme_constant_override("separation", 3)
	var line := HBoxContainer.new()
	box.add_child(line)
	var key := Card._label(caption, ROW_FONT_SIZE - 1, UIStyle.TEXT_DIM)
	key.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	line.add_child(key)
	line.add_child(Card._label(value_text, ROW_FONT_SIZE, UIStyle.TEXT_PRIMARY))
	box.add_child(Card.meter(segments, 5))
	return box

# A banner: tinted, with a stripe on the left; broken states red, the rest yellow.
func _add_alert(parent: VBoxContainer, text: String, broken: bool) -> void:
	var color := UIStyle.TEXT_ALERT if broken else UIStyle.SIGN_YELLOW
	var banner := PanelContainer.new()
	banner.set_meta("alert", true)
	var style := StyleBoxFlat.new()
	style.bg_color = Color(color, 0.10)
	style.border_color = color
	style.border_width_left = 3
	style.set_corner_radius_all(6)
	style.content_margin_left = 9
	style.content_margin_right = 8
	style.content_margin_top = 4
	style.content_margin_bottom = 5
	banner.add_theme_stylebox_override("panel", style)
	var label := Card._label(text, 12, color.lerp(Color.WHITE, 0.2))
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	label.custom_minimum_size.x = UIStyle.scaled_px(300.0)
	banner.add_child(label)
	if parent.has_meta("pairs"):
		_add_control(parent, banner)
	else:
		parent.add_child(banner)
