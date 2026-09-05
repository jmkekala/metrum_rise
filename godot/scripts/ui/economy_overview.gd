# SPDX-License-Identifier: GPL-2.0-only

## Economy overview window backed by Rust-owned budget ledgers and service policies.
##
## Controls are live: service slider changes are sent to SimulationNode immediately
## and apply on the next relevant economy tick whether the game is paused or running.
extends Window

const GameSettings = preload("res://scripts/core/game_settings.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const TrendGraph = preload("res://scripts/ui/economy_trend_graph.gd")
const WindowResizeHandles = preload("res://scripts/ui/window_resize_handles.gd")

const SERVICE_ELECTRICITY := "electricity"
const INCOME_COLOR := Color(0.22, 0.74, 0.48)
const EXPENSE_COLOR := Color(0.92, 0.42, 0.32)
const NET_COLOR := Color(0.36, 0.68, 0.92)
const TITLE_FONT_SIZE := 16
const CONTROL_FONT_SIZE := 14
const SECTION_FONT_SIZE := 13
const ROW_FONT_SIZE := 14
const LAYOUT_ID := "economy_overview"
const KEY_BUDGET_SPLIT := "budget_split"
const KEY_BUDGET_LEFT_SPLIT := "budget_left_split"
const KEY_BUDGET_GRAPHS_SPLIT := "budget_graphs_split"
const KEY_BUDGET_LOWER_GRAPHS_SPLIT := "budget_lower_graphs_split"
const KEY_POLICY_SPLIT := "policy_split"
const KEY_POLICY_DETAIL_SPLIT := "policy_detail_split"
const KEY_SERVICE_SPLIT := "service_split"
const KEY_SERVICE_DETAIL_SPLIT := "service_detail_split"

var simulation_node: Node

var _overview: Dictionary = {}
var _selected_service_id := SERVICE_ELECTRICITY
var _selected_policy_id := ""
var _refresh_elapsed := 0.0
var _syncing_service_sliders := false
var _syncing_policy_sliders := false

var _budget_timeframe: OptionButton
var _budget_summary: VBoxContainer
var _budget_categories: VBoxContainer
var _income_graph: Control
var _net_graph: Control
var _treasury_graph: Control

var _services_timeframe: OptionButton
var _service_rows: VBoxContainer
var _service_details: VBoxContainer
var _service_graph: Control

var _policy_timeframe: OptionButton
var _policy_rows: VBoxContainer
var _policy_details: VBoxContainer
var _policy_graph: Control
var _budget_split: HSplitContainer
var _budget_left_split: VSplitContainer
var _budget_graphs_split: VSplitContainer
var _budget_lower_graphs_split: VSplitContainer
var _policy_split: HSplitContainer
var _policy_detail_split: VSplitContainer
var _service_split: HSplitContainer
var _service_detail_split: VSplitContainer
var _panel_layout_dirty := false

func _ready() -> void:
	title = "Economy Overview"
	unresizable = false
	exclusive = false
	close_requested.connect(_on_close_requested)
	visibility_changed.connect(_on_visibility_changed)
	tree_exiting.connect(_save_panel_layout)
	UIStyle.set_persistent_window_layout(
		self,
		LAYOUT_ID,
		Vector2i(1040, 640),
		Vector2i(860, 520),
		get_parent().get_viewport()
	)
	_build_ui()
	WindowResizeHandles.install(self)
	refresh()

func _process(delta: float) -> void:
	if not visible:
		return
	_refresh_elapsed += delta
	if _refresh_elapsed < 1.0:
		return
	_refresh_elapsed = 0.0
	refresh()

func refresh() -> void:
	if simulation_node == null or not simulation_node.has_method("get_economy_overview"):
		return
	_overview = simulation_node.get_economy_overview()
	_refresh_budget_tab()
	_refresh_services_tab()
	_refresh_policy_tab()

func _on_visibility_changed() -> void:
	if visible:
		_refresh_elapsed = 0.0
		refresh()
	else:
		_save_panel_layout()

func _on_close_requested() -> void:
	_save_panel_layout()
	UIStyle.save_persistent_window_layout(self)
	hide()

func _build_ui() -> void:
	var body := PanelContainer.new()
	body.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	body.add_theme_stylebox_override("panel", UIStyle.window_body_style())
	add_child(body)

	var margin := MarginContainer.new()
	margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	margin.add_theme_constant_override("margin_left", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_right", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_top", UIStyle.PAD_WINDOW)
	margin.add_theme_constant_override("margin_bottom", UIStyle.PAD_WINDOW)
	body.add_child(margin)

	var tabs := TabContainer.new()
	tabs.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	tabs.size_flags_vertical = Control.SIZE_EXPAND_FILL
	UIStyle.set_font_size(tabs, CONTROL_FONT_SIZE)
	margin.add_child(tabs)

	var budget_tab := _build_budget_tab()
	budget_tab.name = "Budget"
	tabs.add_child(budget_tab)

	var services_tab := _build_services_tab()
	services_tab.name = "Services"
	tabs.add_child(services_tab)

	var policy_tab := _build_policy_tab()
	policy_tab.name = "Policy"
	tabs.add_child(policy_tab)

func _build_budget_tab() -> Control:
	var root := VBoxContainer.new()
	root.add_theme_constant_override("separation", 10)

	var top_row := HBoxContainer.new()
	top_row.add_theme_constant_override("separation", 12)
	root.add_child(top_row)

	_budget_timeframe = _make_timeframe_selector()
	_budget_timeframe.item_selected.connect(_on_budget_timeframe_changed)
	top_row.add_child(_budget_timeframe)

	var treasury_label := Label.new()
	treasury_label.name = "TreasuryLabel"
	treasury_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	treasury_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	treasury_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(treasury_label, TITLE_FONT_SIZE)
	top_row.add_child(treasury_label)

	_budget_split = HSplitContainer.new()
	_budget_split.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_budget_split.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_budget_split.split_offset = _layout_offset(KEY_BUDGET_SPLIT, 300.0)
	_budget_split.dragged.connect(func(_offset: int): _mark_panel_layout_dirty())
	root.add_child(_budget_split)

	var left := VSplitContainer.new()
	_budget_left_split = left
	left.custom_minimum_size.x = UIStyle.scaled_px(280.0)
	left.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	left.size_flags_vertical = Control.SIZE_EXPAND_FILL
	left.split_offset = _layout_offset(KEY_BUDGET_LEFT_SPLIT, 150.0)
	left.dragged.connect(func(_offset: int): _mark_panel_layout_dirty())
	_budget_split.add_child(left)

	_budget_summary = VBoxContainer.new()
	_budget_summary.add_theme_constant_override("separation", 4)
	var summary_panel := _section_panel("Summary", _budget_summary)
	summary_panel.custom_minimum_size.y = UIStyle.scaled_px(120.0)
	left.add_child(summary_panel)

	_budget_categories = VBoxContainer.new()
	_budget_categories.add_theme_constant_override("separation", 4)
	var category_panel := _section_panel("Category Breakdown", _budget_categories)
	category_panel.custom_minimum_size.y = UIStyle.scaled_px(160.0)
	left.add_child(category_panel)

	var graphs := VSplitContainer.new()
	_budget_graphs_split = graphs
	graphs.custom_minimum_size.x = UIStyle.scaled_px(360.0)
	graphs.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	graphs.size_flags_vertical = Control.SIZE_EXPAND_FILL
	graphs.split_offset = _layout_offset(KEY_BUDGET_GRAPHS_SPLIT, 150.0)
	graphs.dragged.connect(func(_offset: int): _mark_panel_layout_dirty())
	_budget_split.add_child(graphs)

	_income_graph = TrendGraph.new()
	var income_panel := _section_panel("Income vs Expenses", _income_graph)
	income_panel.custom_minimum_size.y = UIStyle.scaled_px(120.0)
	graphs.add_child(income_panel)

	var lower_graphs := VSplitContainer.new()
	_budget_lower_graphs_split = lower_graphs
	lower_graphs.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	lower_graphs.size_flags_vertical = Control.SIZE_EXPAND_FILL
	lower_graphs.split_offset = _layout_offset(KEY_BUDGET_LOWER_GRAPHS_SPLIT, 150.0)
	lower_graphs.dragged.connect(func(_offset: int): _mark_panel_layout_dirty())
	graphs.add_child(lower_graphs)

	_net_graph = TrendGraph.new()
	var net_panel := _section_panel("Net Daily Cashflow", _net_graph)
	net_panel.custom_minimum_size.y = UIStyle.scaled_px(120.0)
	lower_graphs.add_child(net_panel)

	_treasury_graph = TrendGraph.new()
	var treasury_panel := _section_panel("Treasury", _treasury_graph)
	treasury_panel.custom_minimum_size.y = UIStyle.scaled_px(120.0)
	lower_graphs.add_child(treasury_panel)

	return root

func _build_policy_tab() -> Control:
	var root := HSplitContainer.new()
	_policy_split = root
	root.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	root.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root.split_offset = _layout_offset(KEY_POLICY_SPLIT, 520.0)
	root.dragged.connect(func(_offset: int): _mark_panel_layout_dirty())

	var scroll := ScrollContainer.new()
	scroll.custom_minimum_size.x = UIStyle.scaled_px(360.0)
	scroll.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL

	_policy_rows = VBoxContainer.new()
	_policy_rows.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_policy_rows.add_theme_constant_override("separation", 10)
	scroll.add_child(_policy_rows)
	root.add_child(_section_panel("Controls", scroll))

	var right := VBoxContainer.new()
	right.custom_minimum_size.x = UIStyle.scaled_px(360.0)
	right.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	right.size_flags_vertical = Control.SIZE_EXPAND_FILL
	right.add_theme_constant_override("separation", 8)
	root.add_child(right)

	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 8)
	right.add_child(header)

	var details_title := Label.new()
	details_title.name = "PolicyDetailsTitle"
	details_title.text = "Policy Impact"
	details_title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	details_title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(details_title, TITLE_FONT_SIZE)
	header.add_child(details_title)

	_policy_timeframe = _make_timeframe_selector()
	_policy_timeframe.item_selected.connect(_on_policy_timeframe_changed)
	header.add_child(_policy_timeframe)

	var policy_split := VSplitContainer.new()
	_policy_detail_split = policy_split
	policy_split.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	policy_split.size_flags_vertical = Control.SIZE_EXPAND_FILL
	policy_split.split_offset = _layout_offset(KEY_POLICY_DETAIL_SPLIT, 250.0)
	policy_split.dragged.connect(func(_offset: int): _mark_panel_layout_dirty())
	right.add_child(policy_split)

	_policy_details = VBoxContainer.new()
	_policy_details.add_theme_constant_override("separation", 4)
	var details_panel := _section_panel("Details", _policy_details)
	details_panel.custom_minimum_size.y = UIStyle.scaled_px(170.0)
	policy_split.add_child(details_panel)

	_policy_graph = TrendGraph.new()
	var graph_panel := _section_panel("Selected Policy Trend", _policy_graph)
	graph_panel.custom_minimum_size.y = UIStyle.scaled_px(150.0)
	policy_split.add_child(graph_panel)

	return root

func _build_services_tab() -> Control:
	var root := HSplitContainer.new()
	_service_split = root
	root.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	root.size_flags_vertical = Control.SIZE_EXPAND_FILL
	root.split_offset = _layout_offset(KEY_SERVICE_SPLIT, 310.0)
	root.dragged.connect(func(_offset: int): _mark_panel_layout_dirty())

	_service_rows = VBoxContainer.new()
	_service_rows.custom_minimum_size.x = UIStyle.scaled_px(260.0)
	_service_rows.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_service_rows.add_theme_constant_override("separation", 8)
	root.add_child(_section_panel("Funding", _service_rows))

	var right := VBoxContainer.new()
	right.custom_minimum_size.x = UIStyle.scaled_px(360.0)
	right.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	right.size_flags_vertical = Control.SIZE_EXPAND_FILL
	right.add_theme_constant_override("separation", 8)
	root.add_child(right)

	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 8)
	right.add_child(header)

	var details_title := Label.new()
	details_title.name = "ServiceDetailsTitle"
	details_title.text = "Electricity"
	details_title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	details_title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(details_title, TITLE_FONT_SIZE)
	header.add_child(details_title)

	_services_timeframe = _make_timeframe_selector()
	_services_timeframe.item_selected.connect(_on_services_timeframe_changed)
	header.add_child(_services_timeframe)

	var service_split := VSplitContainer.new()
	_service_detail_split = service_split
	service_split.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	service_split.size_flags_vertical = Control.SIZE_EXPAND_FILL
	service_split.split_offset = _layout_offset(KEY_SERVICE_DETAIL_SPLIT, 250.0)
	service_split.dragged.connect(func(_offset: int): _mark_panel_layout_dirty())
	right.add_child(service_split)

	_service_details = VBoxContainer.new()
	_service_details.add_theme_constant_override("separation", 4)
	var details_panel := _section_panel("Details", _service_details)
	details_panel.custom_minimum_size.y = UIStyle.scaled_px(170.0)
	service_split.add_child(details_panel)

	_service_graph = TrendGraph.new()
	var service_graph_panel := _section_panel("Produced vs Consumed vs Unmet", _service_graph)
	service_graph_panel.custom_minimum_size.y = UIStyle.scaled_px(150.0)
	service_split.add_child(service_graph_panel)

	return root

func _make_timeframe_selector() -> OptionButton:
	var selector := OptionButton.new()
	selector.add_item("Today", 1)
	selector.add_item("7D", 7)
	selector.add_item("30D", 30)
	selector.select(1)
	selector.custom_minimum_size.x = UIStyle.scaled_px(112.0)
	UIStyle.set_font_size(selector, CONTROL_FONT_SIZE)
	return selector

func _section_panel(title: String, child: Control) -> PanelContainer:
	var panel := PanelContainer.new()
	panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_theme_stylebox_override("panel", UIStyle.panel_style(Color(0.10, 0.10, 0.14, 0.78), 6))

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 8)
	margin.add_theme_constant_override("margin_right", 8)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_bottom", 8)
	panel.add_child(margin)

	var box := VBoxContainer.new()
	box.add_theme_constant_override("separation", 6)
	margin.add_child(box)

	var label := Label.new()
	label.text = title
	label.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	UIStyle.set_font_size(label, SECTION_FONT_SIZE)
	box.add_child(label)

	child.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	child.size_flags_vertical = Control.SIZE_EXPAND_FILL
	box.add_child(child)
	return panel

func _refresh_budget_tab() -> void:
	var entries := _entries_for_days(_budget_timeframe.get_selected_id())
	var latest := _latest_entry(entries)
	var day_labels := _day_labels(entries)
	var top_label := find_child("TreasuryLabel", true, false) as Label
	if top_label:
		top_label.text = "Treasury %s" % _money(float(_overview.get("treasury", latest.get("treasury", 0.0))))

	_clear(_budget_summary)
	_add_metric(_budget_summary, "Income", _money(_sum(entries, "income")), UIStyle.TEXT_DIM, INCOME_COLOR)
	_add_metric(_budget_summary, "Expenses", _expense_money(_sum(entries, "expenses")), UIStyle.TEXT_DIM, EXPENSE_COLOR)
	_add_metric(_budget_summary, "Net", _signed_money(_sum(entries, "net")), UIStyle.TEXT_DIM, NET_COLOR)
	_add_metric(_budget_summary, "Treasury", _money(float(latest.get("treasury", _overview.get("treasury", 0.0)))))

	_clear(_budget_categories)
	_add_category_header(_budget_categories, "Income", _money(_sum(entries, "income")), INCOME_COLOR)
	_add_metric(_budget_categories, "Tax Income", _money(_sum(entries, "tax_income")), UIStyle.TEXT_DIM, INCOME_COLOR, 10.0)
	_add_metric(_budget_categories, "Utility/Service Revenue", _money(_sum(entries, "utility_service_revenue")), UIStyle.TEXT_DIM, INCOME_COLOR, 10.0)
	_add_category_header(_budget_categories, "Expenses", _expense_money(_sum(entries, "expenses")), EXPENSE_COLOR)
	_add_metric(_budget_categories, "Benefits", _expense_money(_sum(entries, "benefits")), UIStyle.TEXT_DIM, EXPENSE_COLOR, 10.0)
	_add_metric(_budget_categories, "Unemployment", _expense_money(_sum(entries, "unemployment_benefits")), UIStyle.TEXT_DIM, EXPENSE_COLOR, 20.0)
	_add_metric(_budget_categories, "Pensions", _expense_money(_sum(entries, "pensions")), UIStyle.TEXT_DIM, EXPENSE_COLOR, 20.0)
	_add_metric(_budget_categories, "Child Support", _expense_money(_sum(entries, "child_support")), UIStyle.TEXT_DIM, EXPENSE_COLOR, 20.0)
	_add_metric(_budget_categories, "City Wages", _expense_money(_sum(entries, "city_wages")), UIStyle.TEXT_DIM, EXPENSE_COLOR, 10.0)
	_add_metric(_budget_categories, "Fuel/Input Purchases", _expense_money(_sum(entries, "fuel_input_purchases")), UIStyle.TEXT_DIM, EXPENSE_COLOR, 10.0)
	_add_metric(_budget_categories, "Imports/OWA", _expense_money(_sum(entries, "imports_owa")), UIStyle.TEXT_DIM, EXPENSE_COLOR, 10.0)
	_add_metric(_budget_categories, "Construction/Service Costs", _expense_money(_sum(entries, "construction_service_costs")), UIStyle.TEXT_DIM, EXPENSE_COLOR, 10.0)

	_income_graph.set_series(
		[_series(entries, "income"), _series(entries, "expenses")],
		[INCOME_COLOR, EXPENSE_COLOR],
		["Income", "Expenses"],
		day_labels,
		"$"
	)
	_net_graph.set_series(
		[_series(entries, "net")],
		[NET_COLOR],
		["Net"],
		day_labels,
		"$"
	)
	_treasury_graph.set_series(
		[_series(entries, "treasury")],
		[Color(0.80, 0.72, 0.36)],
		["Treasury"],
		day_labels,
		"$"
	)

func _refresh_services_tab() -> void:
	_refresh_service_rows()
	_refresh_service_details()

func _refresh_policy_tab() -> void:
	if _policy_rows == null:
		return
	_clear(_policy_rows)
	var controls: Array = _overview.get("fiscal_policy_controls", [])
	if controls.is_empty():
		_add_metric(_policy_rows, "Policy", "No live data")
		_refresh_policy_details()
		return

	_ensure_selected_policy_id(controls)
	_syncing_policy_sliders = true
	var current_group := ""
	for control in controls:
		var group := str(control.get("group", "Policy"))
		if group != current_group:
			current_group = group
			var header := Label.new()
			header.text = group
			header.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
			UIStyle.set_font_size(header, SECTION_FONT_SIZE)
			_policy_rows.add_child(header)

		var policy_id := str(control.get("id", ""))
		var row := PanelContainer.new()
		row.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		row.add_theme_stylebox_override("panel", _policy_row_style(policy_id == _selected_policy_id))
		row.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
		row.gui_input.connect(_on_policy_row_input.bind(policy_id))
		_policy_rows.add_child(row)

		var margin := MarginContainer.new()
		margin.add_theme_constant_override("margin_left", 8)
		margin.add_theme_constant_override("margin_right", 8)
		margin.add_theme_constant_override("margin_top", 7)
		margin.add_theme_constant_override("margin_bottom", 7)
		row.add_child(margin)

		var box := VBoxContainer.new()
		box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		box.add_theme_constant_override("separation", 3)
		margin.add_child(box)

		var line := HBoxContainer.new()
		line.add_theme_constant_override("separation", 8)
		box.add_child(line)

		var label := Label.new()
		label.text = str(control.get("label", control.get("id", "")))
		label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
		UIStyle.set_font_size(label, ROW_FONT_SIZE)
		line.add_child(label)

		var unit := str(control.get("unit", ""))
		var value_label := Label.new()
		value_label.text = _policy_value_text(float(control.get("value", 0.0)), unit)
		value_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
		value_label.custom_minimum_size.x = UIStyle.scaled_px(84.0)
		value_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
		UIStyle.set_font_size(value_label, ROW_FONT_SIZE)
		line.add_child(value_label)

		var slider := HSlider.new()
		slider.min_value = float(control.get("min", 0.0))
		slider.max_value = float(control.get("max", 1.0))
		slider.step = float(control.get("step", 0.01))
		slider.value = float(control.get("value", 0.0))
		slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		slider.value_changed.connect(_on_policy_slider_changed.bind(policy_id, value_label, unit))
		box.add_child(slider)
	_syncing_policy_sliders = false
	_refresh_policy_details()

func _refresh_service_rows() -> void:
	_clear(_service_rows)
	var services: Array = _overview.get("services", [])
	if services.is_empty():
		return
	if _selected_service_id.is_empty():
		_selected_service_id = str(services[0].get("id", SERVICE_ELECTRICITY))

	_syncing_service_sliders = true
	for service in services:
		var service_id := str(service.get("id", ""))
		var row := PanelContainer.new()
		row.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		row.add_theme_stylebox_override("panel", _service_row_style(service_id == _selected_service_id))
		row.gui_input.connect(_on_service_row_input.bind(service_id))
		_service_rows.add_child(row)

		var margin := MarginContainer.new()
		margin.add_theme_constant_override("margin_left", 8)
		margin.add_theme_constant_override("margin_right", 8)
		margin.add_theme_constant_override("margin_top", 8)
		margin.add_theme_constant_override("margin_bottom", 8)
		row.add_child(margin)

		var box := VBoxContainer.new()
		box.add_theme_constant_override("separation", 4)
		margin.add_child(box)

		var line := HBoxContainer.new()
		line.add_theme_constant_override("separation", 6)
		box.add_child(line)

		var name_label := Label.new()
		name_label.text = str(service.get("name", service_id))
		name_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		name_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
		UIStyle.set_font_size(name_label, ROW_FONT_SIZE)
		line.add_child(name_label)

		var status_label := Label.new()
		status_label.text = "%s  %s" % [
			_percent(float(service.get("coverage", 0.0))),
			str(service.get("status", "inactive")).capitalize(),
		]
		status_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
		status_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
		UIStyle.set_font_size(status_label, ROW_FONT_SIZE)
		line.add_child(status_label)

		var slider := HSlider.new()
		slider.min_value = 0.0
		slider.max_value = 1.0
		slider.step = 0.05
		slider.value = float(service.get("funding", 1.0))
		slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		slider.value_changed.connect(_on_service_funding_changed.bind(service_id))
		box.add_child(slider)
	_syncing_service_sliders = false

func _refresh_service_details() -> void:
	var entries := _entries_for_days(_services_timeframe.get_selected_id())
	var latest := _latest_entry(entries)
	var day_labels := _day_labels(entries)
	var title_label := find_child("ServiceDetailsTitle", true, false) as Label
	if title_label:
		title_label.text = _service_name(_selected_service_id)

	_clear(_service_details)
	if _selected_service_id != SERVICE_ELECTRICITY:
		_add_metric(_service_details, "Status", "No live data")
		_service_graph.set_series([], [])
		return

	var produced := _sum(entries, "power_produced")
	var consumed := _sum(entries, "power_consumed")
	var unmet := _sum(entries, "power_unmet")
	var demand := consumed + unmet
	var coverage := 0.0 if demand <= 0.001 else consumed / demand

	_add_metric(_service_details, "Production", "%.1f units" % produced)
	_add_metric(_service_details, "Consumed", "%.1f units" % consumed)
	_add_metric(_service_details, "Unmet Demand", "%.1f units" % unmet)
	_add_metric(_service_details, "Coverage", _percent(coverage))
	_add_metric(_service_details, "Coal Inventory", "%.1f" % float(latest.get("coal_inventory", 0.0)))
	_add_metric(_service_details, "Coal Bought", "%.1f" % _sum(entries, "coal_bought"))
	_add_metric(_service_details, "Coal Consumed", "%.1f" % _sum(entries, "coal_consumed"))
	_add_metric(_service_details, "Fuel Cost", _money(_sum(entries, "electricity_fuel_cost")))
	_add_metric(_service_details, "Wage Cost", _money(_sum(entries, "electricity_wage_cost")))
	_add_metric(_service_details, "Utility Revenue", _money(_sum(entries, "electricity_revenue")))
	_add_metric(_service_details, "Net Balance", _signed_money(_sum(entries, "electricity_net")))

	_service_graph.set_series(
		[
			_series(entries, "power_produced"),
			_series(entries, "power_consumed"),
			_series(entries, "power_unmet"),
		],
		[Color(0.22, 0.74, 0.48), Color(0.26, 0.64, 0.92), Color(0.92, 0.42, 0.32)],
		["Produced", "Consumed", "Unmet"],
		day_labels,
		""
	)

func _refresh_policy_details() -> void:
	if _policy_details == null or _policy_graph == null or _policy_timeframe == null:
		return
	var controls: Array = _overview.get("fiscal_policy_controls", [])
	_ensure_selected_policy_id(controls)
	var control := _selected_policy_control(controls)
	var title_label := find_child("PolicyDetailsTitle", true, false) as Label

	_clear(_policy_details)
	if control.is_empty():
		if title_label:
			title_label.text = "Policy Impact"
		_add_metric(_policy_details, "Status", "No live data")
		_policy_graph.set_series([], [])
		return

	var entries := _entries_for_days(_policy_timeframe.get_selected_id())
	var day_labels := _day_labels(entries)
	var selected_label := str(control.get("label", control.get("id", "Policy")))
	var impact_label := str(control.get("impact_label", selected_label))
	var impact_field := str(control.get("impact_field", ""))
	var impact_kind := str(control.get("impact_kind", "revenue"))
	var unit := str(control.get("unit", ""))
	var total := _sum(entries, impact_field)
	var latest := _latest_entry(entries)
	var latest_value := float(latest.get(impact_field, 0.0))
	var value_color := INCOME_COLOR
	var graph_label := "Revenue"
	if impact_kind == "expense":
		value_color = EXPENSE_COLOR
		graph_label = "Cost"

	var total_text := _money(total)
	var latest_text := _money(latest_value)
	if impact_kind == "expense":
		total_text = _expense_money(total)
		latest_text = _expense_money(latest_value)

	if title_label:
		title_label.text = selected_label

	_add_category_header(_policy_details, impact_label, total_text, value_color)
	_add_metric(_policy_details, "Current Value", _policy_value_text(float(control.get("value", 0.0)), unit))
	_add_metric(_policy_details, "Latest Day", latest_text, UIStyle.TEXT_DIM, value_color)
	_add_metric(_policy_details, "Period Total", total_text, UIStyle.TEXT_DIM, value_color)
	_add_metric(_policy_details, "Days", "%d" % entries.size())

	_policy_graph.set_series(
		[
			_series(entries, impact_field),
		],
		[value_color],
		[graph_label],
		day_labels,
		"$"
	)

func _on_budget_timeframe_changed(_idx: int) -> void:
	_refresh_budget_tab()

func _on_services_timeframe_changed(_idx: int) -> void:
	_refresh_service_details()

func _on_policy_timeframe_changed(_idx: int) -> void:
	_refresh_policy_details()

func _on_policy_row_input(event: InputEvent, policy_id: String) -> void:
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
		_select_policy(policy_id)

func _on_service_row_input(event: InputEvent, service_id: String) -> void:
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
		_select_service(service_id)

func _on_service_funding_changed(value: float, service_id: String) -> void:
	if _syncing_service_sliders:
		return
	_select_service(service_id)
	if simulation_node and simulation_node.has_method("set_economy_service_funding"):
		simulation_node.set_economy_service_funding(service_id, value)
	_refresh_elapsed = 0.0
	if simulation_node != null and simulation_node.has_method("get_economy_overview"):
		_overview = simulation_node.get_economy_overview()
		_refresh_service_details()

func _on_policy_slider_changed(value: float, policy_id: String, value_label: Label, unit: String) -> void:
	if _syncing_policy_sliders:
		return
	var changed_selection := false
	if not policy_id.is_empty() and policy_id != _selected_policy_id:
		_selected_policy_id = policy_id
		changed_selection = true
	if value_label != null:
		value_label.text = _policy_value_text(value, unit)
	if simulation_node and simulation_node.has_method("set_economy_policy_value"):
		simulation_node.set_economy_policy_value(policy_id, value)
	_refresh_elapsed = 0.0
	if simulation_node != null and simulation_node.has_method("get_economy_overview"):
		_overview = simulation_node.get_economy_overview()
		_refresh_budget_tab()
		if changed_selection:
			_refresh_policy_tab()
		else:
			_refresh_policy_details()

func _select_service(service_id: String) -> void:
	if service_id.is_empty() or service_id == _selected_service_id:
		return
	_selected_service_id = service_id
	_refresh_services_tab()

func _select_policy(policy_id: String) -> void:
	if policy_id.is_empty() or policy_id == _selected_policy_id:
		return
	_selected_policy_id = policy_id
	_refresh_policy_tab()

func _service_row_style(selected: bool) -> StyleBoxFlat:
	if selected:
		return UIStyle.panel_style(Color(0.16, 0.22, 0.27, 0.92), 6, Color(0.28, 0.70, 0.80, 0.75), 1)
	return UIStyle.panel_style(Color(0.09, 0.09, 0.12, 0.70), 6, Color(0.30, 0.30, 0.45, 0.35), 1)

func _policy_row_style(selected: bool) -> StyleBoxFlat:
	if selected:
		return UIStyle.panel_style(Color(0.16, 0.22, 0.27, 0.92), 6, Color(0.28, 0.70, 0.80, 0.75), 1)
	return UIStyle.panel_style(Color(0.09, 0.09, 0.12, 0.70), 6, Color(0.30, 0.30, 0.45, 0.35), 1)

func _entries_for_days(days: int) -> Array:
	var history: Array = _overview.get("history", [])
	var out: Array = []
	if history.is_empty():
		var latest: Dictionary = _overview.get("latest", {})
		if not latest.is_empty():
			out.append(latest)
		return out
	var start := maxi(0, history.size() - maxi(days, 1))
	for idx in range(start, history.size()):
		out.append(history[idx])
	return out

func _latest_entry(entries: Array) -> Dictionary:
	if not entries.is_empty():
		return entries[entries.size() - 1]
	return _overview.get("latest", {})

func _ensure_selected_policy_id(controls: Array) -> void:
	var first_id := ""
	var selected_exists := false
	for control in controls:
		var policy_id := str(control.get("id", ""))
		if first_id.is_empty() and not policy_id.is_empty():
			first_id = policy_id
		if policy_id == _selected_policy_id:
			selected_exists = true
	if not selected_exists:
		_selected_policy_id = first_id

func _selected_policy_control(controls: Array) -> Dictionary:
	for control in controls:
		var dict: Dictionary = control
		if str(dict.get("id", "")) == _selected_policy_id:
			return dict
	return {}

func _sum(entries: Array, field: String) -> float:
	var total := 0.0
	for entry in entries:
		total += float(entry.get(field, 0.0))
	return total

func _series(entries: Array, field: String) -> Array:
	var values: Array = []
	for entry in entries:
		values.append(float(entry.get(field, 0.0)))
	return values

func _day_labels(entries: Array) -> Array:
	var labels: Array = []
	for idx in range(entries.size()):
		var entry: Dictionary = entries[idx]
		if entry.has("day_index"):
			labels.append("D%d" % int(entry.get("day_index", idx + 1)))
		else:
			labels.append("D%d" % (idx + 1))
	return labels

func _clear(container: Node) -> void:
	for child in container.get_children():
		container.remove_child(child)
		child.queue_free()

func _layout_offset(key: String, default_offset: float) -> int:
	return maxi(0, GameSettings.get_layout_int(
		LAYOUT_ID,
		key,
		int(UIStyle.scaled_px(default_offset))
	))

func _mark_panel_layout_dirty() -> void:
	_panel_layout_dirty = true

func _save_panel_layout() -> void:
	if not _panel_layout_dirty:
		return
	var values := {}
	if _budget_split != null:
		values[KEY_BUDGET_SPLIT] = _budget_split.split_offset
	if _budget_left_split != null:
		values[KEY_BUDGET_LEFT_SPLIT] = _budget_left_split.split_offset
	if _budget_graphs_split != null:
		values[KEY_BUDGET_GRAPHS_SPLIT] = _budget_graphs_split.split_offset
	if _budget_lower_graphs_split != null:
		values[KEY_BUDGET_LOWER_GRAPHS_SPLIT] = _budget_lower_graphs_split.split_offset
	if _policy_split != null:
		values[KEY_POLICY_SPLIT] = _policy_split.split_offset
	if _policy_detail_split != null:
		values[KEY_POLICY_DETAIL_SPLIT] = _policy_detail_split.split_offset
	if _service_split != null:
		values[KEY_SERVICE_SPLIT] = _service_split.split_offset
	if _service_detail_split != null:
		values[KEY_SERVICE_DETAIL_SPLIT] = _service_detail_split.split_offset
	if not values.is_empty():
		GameSettings.save_layout_values(LAYOUT_ID, values)
	_panel_layout_dirty = false

func _add_category_header(parent: VBoxContainer, label_text: String, value_text: String, color: Color) -> void:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 8)
	parent.add_child(row)

	var label := Label.new()
	label.text = label_text
	label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	label.add_theme_color_override("font_color", color)
	UIStyle.set_font_size(label, ROW_FONT_SIZE)
	row.add_child(label)

	var value := Label.new()
	value.text = value_text
	value.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	value.add_theme_color_override("font_color", color)
	UIStyle.set_font_size(value, ROW_FONT_SIZE)
	row.add_child(value)

func _add_metric(
	parent: VBoxContainer,
	label_text: String,
	value_text: String,
	label_color: Color = UIStyle.TEXT_DIM,
	value_color: Color = UIStyle.TEXT_PRIMARY,
	indent: float = 0.0
) -> void:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 8)
	parent.add_child(row)

	if indent > 0.0:
		var spacer := Control.new()
		spacer.custom_minimum_size.x = UIStyle.scaled_px(indent)
		row.add_child(spacer)

	var label := Label.new()
	label.text = label_text
	label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	label.add_theme_color_override("font_color", label_color)
	UIStyle.set_font_size(label, ROW_FONT_SIZE)
	row.add_child(label)

	var value := Label.new()
	value.text = value_text
	value.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	value.add_theme_color_override("font_color", value_color)
	UIStyle.set_font_size(value, ROW_FONT_SIZE)
	row.add_child(value)

func _money(value: float) -> String:
	var sign := "-" if value < 0.0 else ""
	var amount := absf(value)
	if amount >= 1000000.0:
		return "%s$%.1fM" % [sign, amount / 1000000.0]
	if amount >= 1000.0:
		return "%s$%.1fk" % [sign, amount / 1000.0]
	return "%s$%.0f" % [sign, amount]

func _signed_money(value: float) -> String:
	var prefix := "+" if value > 0.0 else ""
	return prefix + _money(value)

func _expense_money(value: float) -> String:
	var amount := absf(value)
	if amount < 0.5:
		return _money(0.0)
	return "-%s" % _money(amount)

func _percent(value: float) -> String:
	return "%.0f%%" % (clampf(value, 0.0, 1.0) * 100.0)

func _policy_value_text(value: float, unit: String) -> String:
	match unit:
		"percent":
			return _percent(value)
		"currency_per_day":
			return "%s/day" % _money(value)
		"currency":
			return _money(value)
		"days":
			return "%dd" % int(round(value))
		"multiplier":
			return "x%.2f" % value
		_:
			return "%.2f" % value

func _service_name(service_id: String) -> String:
	match service_id:
		SERVICE_ELECTRICITY:
			return "Electricity"
		_:
			return service_id.capitalize()
