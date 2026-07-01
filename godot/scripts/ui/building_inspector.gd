## Building inspector window manager backed by Godot's built-in Window chrome.
##
## Call try_inspect() with a world position to populate and show a per-building
## window. Multiple inspector windows may be open at the same time. Open windows
## refresh only when the in-game hour changes, so the steady-state cost is O(1)
## per frame and O(open_windows) on each hour boundary.
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")

@onready var simulation_node = $"../SimulationNode"

var _open_windows: Dictionary = {}
var _last_observed_hour: int = -1

func _process(_delta: float) -> void:
	if _open_windows.is_empty() or simulation_node == null:
		return

	var absolute_hour := _current_absolute_hour()
	if absolute_hour < 0 or absolute_hour == _last_observed_hour:
		return

	_last_observed_hour = absolute_hour
	_refresh_open_windows()

func try_inspect(world_pos: Vector3, screen_pos: Vector2 = Vector2.ZERO) -> bool:
	var info: Dictionary = simulation_node.get_building_info_at(world_pos.x, world_pos.z)
	if info.is_empty():
		return false

	var key := _building_key(info)
	var entry: Dictionary = _open_windows.get(key, {})
	if not entry.is_empty():
		_close_entry(key)
		return true

	entry = _create_window_entry(key)
	_open_windows[key] = entry

	_populate(entry, info)
	_update_anchor(entry, info)

	var window: Window = entry["window"]
	_place_window_near_cursor(window, screen_pos)
	window.show()
	window.grab_focus()
	_last_observed_hour = _current_absolute_hour()
	return true

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
	window.title = "Building Inspector"
	window.size = Vector2i(340, 420)
	window.unresizable = false
	window.exclusive = false
	window.visible = false
	window.close_requested.connect(_close_entry.bind(key))

	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	window.add_child(body)

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

	var title_label := Label.new()
	title_label.add_theme_font_size_override("font_size", 14)
	title_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	title_label.clip_text = true
	root_vbox.add_child(title_label)

	root_vbox.add_child(HSeparator.new())

	var scroll := ScrollContainer.new()
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root_vbox.add_child(scroll)

	var stats_body := VBoxContainer.new()
	stats_body.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	stats_body.add_theme_constant_override("separation", 4)
	scroll.add_child(stats_body)

	add_child(window)
	window.hide()

	return {
		"window": window,
		"title_label": title_label,
		"stats_body": stats_body,
		"anchor_pos": Vector3.ZERO,
	}

func _close_entry(key: String) -> void:
	var entry: Dictionary = _open_windows.get(key, {})
	if entry.is_empty():
		return

	_open_windows.erase(key)
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
	if window == null:
		return

	if screen_pos == Vector2.ZERO:
		screen_pos = get_viewport().get_mouse_position()

	var viewport_size := get_viewport().get_visible_rect().size
	var window_size := Vector2(window.size)
	var padding := UIStyle.CURSOR_WINDOW_GAP
	var top_margin := 40.0
	var bottom_margin := 12.0

	var x := screen_pos.x + padding
	if x + window_size.x > viewport_size.x - padding:
		x = screen_pos.x - window_size.x - padding
	if x < padding:
		x = padding

	var y := screen_pos.y - 24.0
	var max_y := maxf(top_margin, viewport_size.y - window_size.y - bottom_margin)
	y = clampf(y, top_margin, max_y)

	window.position = Vector2i(int(round(x)), int(round(y)))

func _populate(entry: Dictionary, info: Dictionary) -> void:
	var window: Window = entry["window"]
	var title_label: Label = entry["title_label"]
	var stats_body: VBoxContainer = entry["stats_body"]
	var display_name := _display_name(info)

	window.title = display_name
	title_label.text = display_name

	for child in stats_body.get_children():
		child.queue_free()

	var zone: String = info.get("zone_type", "")
	_add_row(stats_body, "Type", "%s   Level %d" % [_zone_label(zone), int(info.get("level", 0))])
	if info.get("under_construction", false):
		_add_row(
			stats_body,
			"Construction",
			"%d h left / %.0f%%" % [
				int(info.get("construction_remaining_hours", 0)),
				float(info.get("construction_progress", 0.0)) * 100.0,
			]
		)

	if zone == "residential":
		_add_row(stats_body, "Households", str(info.get("household_count", info.get("occupancy", 0))))
		_add_row(stats_body, "Children", str(info.get("child_count", 0)))
		_add_row(stats_body, "Adults", str(info.get("adult_count", 0)))
		_add_row(stats_body, "Elders", str(info.get("elder_count", 0)))
		_add_section(stats_body, "Household Economy")
		_add_row(
			stats_body,
			"Household Money",
			"$%.1f total / $%.1f avg" % [
				float(info.get("household_budget_total", 0.0)),
				float(info.get("household_budget_avg", 0.0)),
			]
		)
		_add_row(
			stats_body,
			"Stock",
			"%.1f d avg / %.1f d min" % [
				float(info.get("household_stock_days_avg", 0.0)),
				float(info.get("household_stock_days_min", 0.0)),
			]
		)
		_add_row(stats_body, "Stock Units", "%.1f" % float(info.get("household_stock_total", 0.0)))
		_add_row(stats_body, "Restock", str(info.get("household_replenishment_state", "-")))
		if int(info.get("household_replenishment_active", 0)) > 0:
			_add_row(
				stats_body,
				"Active Restocks",
				str(info.get("household_replenishment_active", 0))
			)
	else:
		_add_section(stats_body, "Business")
		if info.get("business_summary", false):
			_add_row(stats_body, "Status", str(info.get("business_status", "-")))
			_add_row(stats_body, "Budget", _money(float(info.get("operating_budget", 0.0))))
			_add_row(stats_body, "Today", _signed_money(float(info.get("business_profit_today", 0.0))))
			_add_row(stats_body, "Yesterday", _signed_money(float(info.get("business_profit_yesterday", 0.0))))
			var workers := int(info.get("worker_count", 0))
			var active_capacity := int(info.get("business_active_worker_capacity", info.get("worker_capacity", 0)))
			var max_capacity := int(info.get("worker_capacity", 0))
			var worker_text := "%d / %d" % [workers, max_capacity]
			if active_capacity != max_capacity:
				worker_text = "%d / %d active (%d max)" % [workers, active_capacity, max_capacity]
			_add_row(stats_body, "Workers", worker_text)
			_add_row(
				stats_body,
				"Production",
				"%.0f%%" % (float(info.get("business_production_ratio", 0.0)) * 100.0)
			)
			if info.has("utility_fuel_name"):
				var fuel_units := float(info.get("utility_fuel_units", 0.0))
				var fuel_days := float(info.get("utility_fuel_days", 0.0))
				_add_row(
					stats_body,
					"Fuel Stock",
					"%.1f %s / %.1f d" % [
						fuel_units,
						str(info.get("utility_fuel_name", "fuel")),
						fuel_days,
					]
				)
			elif info.get("business_has_inventory_fill", false):
				_add_row(
					stats_body,
					"Inventory",
					"%.0f%% full" % (float(info.get("business_inventory_fill_ratio", 0.0)) * 100.0)
				)
			if info.has("utility_service"):
				var utility_name := str(info.get("utility_service", "")).capitalize()
				var utility_state := "active" if info.get("utility_service_available", false) else "inactive"
				_add_row(stats_body, "Utility", "%s %s" % [utility_name, utility_state])
				_add_row(stats_body, "Utility Revenue", _money(float(info.get("utility_local_revenue", 0.0))))
				if str(info.get("utility_service", "")) == "power":
					var power_produced := float(info.get("utility_power_production_today", 0.0))
					var power_consumed := float(info.get("utility_power_consumed_today", 0.0))
					_add_row(
						stats_body,
						"Power Output",
						"%.1f units" % power_produced
					)
					_add_power_consumption_bar(stats_body, power_consumed, power_produced)
				_add_row(stats_body, "City Fuel Today", _money(float(info.get("city_fuel_cost_today", 0.0))))
		else:
			_add_row(
				stats_body,
				"Workers",
				"%d / %d" % [info.get("worker_count", 0), info.get("worker_capacity", 0)]
			)
			_add_row(stats_body, "Budget", _money(float(info.get("operating_budget", 0.0))))
		if info.has("utility_service_available") and not info.has("utility_service"):
			_add_row(stats_body, "Utility", "Yes" if info["utility_service_available"] else "No")

	var inventory: Array = info.get("inventory", [])
	if inventory.size() > 0 and (not info.get("business_summary", false) or info.has("utility_service")):
		_add_section(stats_body, "Inventory")
		for item in inventory:
			_add_row(stats_body, str(item.get("name", "?")), "%.1f" % float(item.get("amount", 0.0)))

	var flags: Array[String] = []
	if info.get("broken", false):
		flags.append("Asset broken")
	if info.get("economy_broken", false):
		flags.append("Economy broken")
	if info.get("is_deserted", false):
		flags.append("Deserted")
	if info.get("pending_redevelopment", false):
		flags.append("Rezone (%d d grace)" % info.get("rezone_grace_days", 0))
	if info.get("under_construction", false):
		flags.append("Under construction")
	if flags.size() > 0:
		_add_section(stats_body, "Alerts")
		for flag in flags:
			_add_alert(stats_body, flag)

func _zone_label(zone: String) -> String:
	match zone:
		"residential": return "Residential"
		"commercial": return "Commercial"
		"industrial": return "Industrial"
		"utility": return "Utility"
		_: return zone.capitalize()

func _money(amount: float) -> String:
	return "$%.0f" % amount

func _signed_money(amount: float) -> String:
	var sign := "+" if amount >= 0.0 else "-"
	return "%s$%.0f" % [sign, absf(amount)]

func _add_section(stats_body: VBoxContainer, title: String) -> void:
	stats_body.add_child(HSeparator.new())
	var label := Label.new()
	label.text = title
	label.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	label.add_theme_font_size_override("font_size", 11)
	stats_body.add_child(label)

func _add_row(stats_body: VBoxContainer, label_text: String, value_text: String) -> void:
	var hbox := HBoxContainer.new()
	stats_body.add_child(hbox)

	var label := Label.new()
	label.text = label_text
	label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	label.add_theme_font_size_override("font_size", 12)
	hbox.add_child(label)

	var value := Label.new()
	value.text = value_text
	value.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	value.add_theme_font_size_override("font_size", 12)
	hbox.add_child(value)

func _add_power_consumption_bar(stats_body: VBoxContainer, consumed_units: float, produced_units: float) -> void:
	produced_units = maxf(produced_units, 0.0)
	consumed_units = clampf(consumed_units, 0.0, produced_units)
	var ratio := 0.0
	if produced_units > 0.0:
		ratio = clampf(consumed_units / produced_units, 0.0, 1.0)

	var hbox := HBoxContainer.new()
	hbox.add_theme_constant_override("separation", 8)
	stats_body.add_child(hbox)

	var label := Label.new()
	label.text = "Power Use"
	label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	label.add_theme_font_size_override("font_size", 12)
	hbox.add_child(label)

	var bar_holder := Control.new()
	bar_holder.custom_minimum_size = Vector2(0.0, 18.0)
	bar_holder.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	hbox.add_child(bar_holder)

	var bar := ProgressBar.new()
	bar.min_value = 0.0
	bar.max_value = 1.0
	bar.value = ratio
	bar.show_percentage = false
	bar.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	bar.add_theme_stylebox_override(
		"background",
		UIStyle.panel_style(Color(0.18, 0.18, 0.20, 0.90), 4, Color.TRANSPARENT, 0)
	)
	bar.add_theme_stylebox_override(
		"fill",
		UIStyle.panel_style(Color(0.20, 0.62, 0.72, 0.95), 4, Color.TRANSPARENT, 0)
	)
	bar_holder.add_child(bar)

	var value := Label.new()
	value.text = "%.1f / %.1f units" % [consumed_units, produced_units]
	value.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	value.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	value.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	value.add_theme_font_size_override("font_size", 11)
	value.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	bar_holder.add_child(value)

func _add_alert(stats_body: VBoxContainer, text: String) -> void:
	var label := Label.new()
	label.text = "! " + text
	label.add_theme_color_override("font_color", UIStyle.TEXT_ALERT)
	label.add_theme_font_size_override("font_size", 12)
	stats_body.add_child(label)
