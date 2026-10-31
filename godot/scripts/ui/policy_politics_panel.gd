# SPDX-License-Identifier: GPL-2.0-only

## Unified Georgist Policy & Politics UI panel for EconomyOverview.
##
## Integrates Land Value Tax (LVT) and Citizen's Dividend (UBI) sliders, real-time Rust
## policy reform previews (Ghost Bars on the 4 political factions), contextual reservation-wage
## labor shortage warning badges, and the municipal election countdown clock.
extends HSplitContainer

signal reform_applied(updated_overview: Dictionary)
signal selected_policy_changed(policy_id: String)

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const FactionApprovalBar = preload("res://scripts/ui/faction_approval_bar.gd")
const TrendGraph = preload("res://scripts/ui/economy_trend_graph.gd")

const POLICY_LVT_RATE := "lvt_rate"
const POLICY_UBI_SURPLUS_SHARE := "ubi_surplus_share"
const POLICY_PUBLIC_WAGE_MULTIPLIER := "public_sector_wage_multiplier"
const POLICY_AUTOMATION_LEVEL := "automation_level"
const POLICY_RES_PROP_TAX := "residential_property_tax_per_home_per_day"
const POLICY_COM_PROP_TAX := "commercial_property_tax_per_building_per_day"
const POLICY_IND_PROP_TAX := "industrial_property_tax_per_building_per_day"

const PRIMARY_GEORGIST_IDS := [
	POLICY_LVT_RATE,
	POLICY_UBI_SURPLUS_SHARE,
	POLICY_PUBLIC_WAGE_MULTIPLIER,
	POLICY_AUTOMATION_LEVEL,
]

const TRADITIONAL_PROPERTY_IDS := [
	POLICY_RES_PROP_TAX,
	POLICY_COM_PROP_TAX,
	POLICY_IND_PROP_TAX,
	"property_tax_level_multiplier",
]

var simulation_node: Node
var selected_policy_id: String = POLICY_LVT_RATE

var _overview: Dictionary = {}
var _live_policies: Dictionary = {}
var _staged_policies: Dictionary = {}
var _preview_data: Dictionary = {}
var _syncing_sliders: bool = false
var _show_legacy_taxes: bool = false

var _sliders: Dictionary = {}
var _value_labels: Dictionary = {}
var _row_panels: Dictionary = {}

# Contextual Georgist readouts & warning badges
var _lvt_subtitle_label: Label
var _ubi_subtitle_label: Label
var _deficit_badge: PanelContainer
var _low_wage_warning_badge: PanelContainer
var _public_wage_warning_badge: PanelContainer
var _legacy_taxes_container: VBoxContainer
var _legacy_toggle_btn: Button
var _apply_btn: Button
var _reset_btn: Button

# Politics & Election widgets
var _election_countdown_label: Label
var _election_progress: ProgressBar
var _overall_approval_label: Label
var _overall_delta_label: Label
var _volatility_label: Label
var _faction_bars: Dictionary = {}

# Policy details & trend graph
var _timeframe_selector: OptionButton
var _details_title_label: Label
var _policy_details_box: VBoxContainer
var _policy_graph: Control
var _right_split: VSplitContainer

func _init() -> void:
	size_flags_horizontal = Control.SIZE_EXPAND_FILL
	size_flags_vertical = Control.SIZE_EXPAND_FILL
	_build_panel()

func _build_panel() -> void:
	# Left Column: Georgist Policy Sliders, Warnings & Action Bar
	var left_vbox := VBoxContainer.new()
	left_vbox.custom_minimum_size.x = UIStyle.scaled_px(420.0)
	left_vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	left_vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	left_vbox.add_theme_constant_override("separation", 8)
	add_child(left_vbox)

	var scroll := ScrollContainer.new()
	scroll.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	scroll.size_flags_vertical = Control.SIZE_EXPAND_FILL
	left_vbox.add_child(scroll)

	var controls_box := VBoxContainer.new()
	controls_box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	controls_box.add_theme_constant_override("separation", 10)
	scroll.add_child(controls_box)

	_build_georgist_core_section(controls_box)
	_build_labor_levers_section(controls_box)
	_build_legacy_taxes_section(controls_box)

	var action_bar := HBoxContainer.new()
	action_bar.add_theme_constant_override("separation", 8)
	left_vbox.add_child(action_bar)

	var single_tax_btn := Button.new()
	single_tax_btn.text = "Single Tax Preset (Zero Prop. Taxes)"
	single_tax_btn.tooltip_text = "Set traditional building property taxes to 0% to focus on pure Land Value Tax."
	UIStyle.set_font_size(single_tax_btn, 12)
	single_tax_btn.pressed.connect(_on_single_tax_preset_pressed)
	action_bar.add_child(single_tax_btn)

	var action_spacer := Control.new()
	action_spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	action_bar.add_child(action_spacer)

	_reset_btn = Button.new()
	_reset_btn.text = "Reset Preview"
	_reset_btn.disabled = true
	UIStyle.set_font_size(_reset_btn, 13)
	_reset_btn.pressed.connect(_on_reset_preview_pressed)
	action_bar.add_child(_reset_btn)

	_apply_btn = Button.new()
	_apply_btn.text = "Apply Reform"
	_apply_btn.disabled = true
	UIStyle.set_font_size(_apply_btn, 13)
	_apply_btn.pressed.connect(_on_apply_reform_pressed)
	action_bar.add_child(_apply_btn)

	# Right Column: Election Clock, 4 Faction Ghost Bars & Policy Trend
	var right_vbox := VBoxContainer.new()
	right_vbox.custom_minimum_size.x = UIStyle.scaled_px(400.0)
	right_vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	right_vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	right_vbox.add_theme_constant_override("separation", 8)
	add_child(right_vbox)

	_right_split = VSplitContainer.new()
	_right_split.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_right_split.size_flags_vertical = Control.SIZE_EXPAND_FILL
	_right_split.split_offset = int(UIStyle.scaled_px(310.0))
	right_vbox.add_child(_right_split)

	var politics_vbox := VBoxContainer.new()
	politics_vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	politics_vbox.add_theme_constant_override("separation", 6)
	_build_election_header(politics_vbox)
	_build_factions_section(politics_vbox)
	_right_split.add_child(_section_panel("Political Factions & Election Forecast", politics_vbox))

	var trend_vbox := VBoxContainer.new()
	trend_vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	trend_vbox.size_flags_vertical = Control.SIZE_EXPAND_FILL
	trend_vbox.add_theme_constant_override("separation", 6)
	_build_trend_section(trend_vbox)
	_right_split.add_child(_section_panel("Selected Policy Fiscal Impact", trend_vbox))

func _build_georgist_core_section(parent: VBoxContainer) -> void:
	var box := VBoxContainer.new()
	box.add_theme_constant_override("separation", 8)

	var lvt_row := _create_slider_row(
		POLICY_LVT_RATE,
		"Land Value Tax (LVT)",
		0.0,
		1.0,
		0.01,
		"percent"
	)
	_lvt_subtitle_label = Label.new()
	_lvt_subtitle_label.text = "Taxes unimproved spatial location value; building upgrades are 100% exempt."
	_lvt_subtitle_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(_lvt_subtitle_label, 11)
	lvt_row.get_child(0).get_child(0).add_child(_lvt_subtitle_label)
	box.add_child(lvt_row)

	var ubi_row := _create_slider_row(
		POLICY_UBI_SURPLUS_SHARE,
		"Citizen's Dividend (UBI Surplus Share)",
		0.0,
		1.0,
		0.01,
		"percent"
	)
	var ubi_inner: VBoxContainer = ubi_row.get_child(0).get_child(0)
	_ubi_subtitle_label = Label.new()
	_ubi_subtitle_label.text = "Paid strictly from net fiscal surplus after Mandatory OpEx & Debt Service."
	_ubi_subtitle_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(_ubi_subtitle_label, 11)
	ubi_inner.add_child(_ubi_subtitle_label)

	_deficit_badge = _create_warning_badge(
		"DEFICIT CIRCUIT-BREAKER: Net surplus <= $0 — UBI automatically paused ($0/day)",
		Color(0.96, 0.62, 0.28),
		Color(0.28, 0.16, 0.06, 0.92)
	)
	_deficit_badge.visible = false
	ubi_inner.add_child(_deficit_badge)

	_low_wage_warning_badge = _create_warning_badge(
		"⚠ LOW-WAGE LABOR SHORTAGE: Reservation wage exceeds retail/farm net pay. Raise Workplace Automation or moderate UBI.",
		Color(1.00, 0.44, 0.36),
		Color(0.30, 0.10, 0.10, 0.94)
	)
	_low_wage_warning_badge.visible = false
	ubi_inner.add_child(_low_wage_warning_badge)

	_public_wage_warning_badge = _create_warning_badge(
		"⚠ PUBLIC UTILITY RETENTION RISK: Reservation wage exceeds municipal utility pay. Raise Public Wage Multiplier.",
		Color(0.98, 0.76, 0.30),
		Color(0.28, 0.20, 0.06, 0.94)
	)
	_public_wage_warning_badge.visible = false
	ubi_inner.add_child(_public_wage_warning_badge)

	box.add_child(ubi_row)
	parent.add_child(_section_panel("Georgist Core Reform (LVT & Citizen's Dividend)", box))

func _build_labor_levers_section(parent: VBoxContainer) -> void:
	var box := VBoxContainer.new()
	box.add_theme_constant_override("separation", 8)
	box.add_child(_create_slider_row(
		POLICY_PUBLIC_WAGE_MULTIPLIER,
		"Public Sector Wage Multiplier",
		0.50,
		3.00,
		0.05,
		"multiplier"
	))
	box.add_child(_create_slider_row(
		POLICY_AUTOMATION_LEVEL,
		"Workplace Automation Level (Capital Substitution)",
		0.0,
		0.50,
		0.05,
		"percent"
	))
	parent.add_child(_section_panel("Labor Market & Productivity Levers", box))

func _build_legacy_taxes_section(parent: VBoxContainer) -> void:
	var wrapper := VBoxContainer.new()
	wrapper.add_theme_constant_override("separation", 6)

	_legacy_toggle_btn = Button.new()
	_legacy_toggle_btn.text = "▸ Show Conventional Taxes & Statutory Transfers"
	_legacy_toggle_btn.alignment = HORIZONTAL_ALIGNMENT_LEFT
	UIStyle.set_font_size(_legacy_toggle_btn, 12)
	_legacy_toggle_btn.pressed.connect(func() -> void:
		_show_legacy_taxes = not _show_legacy_taxes
		_legacy_taxes_container.visible = _show_legacy_taxes
		_legacy_toggle_btn.text = (
			"▾ Hide Conventional Taxes & Statutory Transfers"
			if _show_legacy_taxes
			else "▸ Show Conventional Taxes & Statutory Transfers"
		)
	)
	wrapper.add_child(_legacy_toggle_btn)

	_legacy_taxes_container = VBoxContainer.new()
	_legacy_taxes_container.visible = false
	_legacy_taxes_container.add_theme_constant_override("separation", 6)
	for spec in [
		["income_tax", "Income Tax", 0.0, 0.75, 0.01, "percent"],
		["household_vat", "Household VAT", 0.0, 0.50, 0.01, "percent"],
		["business_profit_tax", "Business Profit Tax", 0.0, 0.75, 0.01, "percent"],
		[POLICY_RES_PROP_TAX, "Residential Building Property Tax", 0.0, 500.0, 1.0, "currency_per_day"],
		[POLICY_COM_PROP_TAX, "Commercial Building Property Tax", 0.0, 500.0, 1.0, "currency_per_day"],
		[POLICY_IND_PROP_TAX, "Industrial Building Property Tax", 0.0, 500.0, 1.0, "currency_per_day"],
		["unemployment_benefit", "Unemployment Benefit", 0.0, 200.0, 1.0, "currency_per_day"],
		["unemployment_max_days", "Unemployment Max Duration", 0.0, 365.0, 1.0, "days"],
		["pension", "Elder Pension", 0.0, 200.0, 1.0, "currency_per_day"],
		["child_support", "Child Support", 0.0, 200.0, 1.0, "currency_per_day"],
	]:
		_legacy_taxes_container.add_child(
			_create_slider_row(spec[0], spec[1], spec[2], spec[3], spec[4], spec[5])
		)
	wrapper.add_child(_legacy_taxes_container)
	parent.add_child(wrapper)

func _build_election_header(parent: VBoxContainer) -> void:
	var header_panel := PanelContainer.new()
	header_panel.add_theme_stylebox_override(
		"panel",
		UIStyle.panel_style(Color(0.12, 0.14, 0.20, 0.90), 6, Color(0.32, 0.42, 0.62, 0.65), 1)
	)
	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 10)
	margin.add_theme_constant_override("margin_right", 10)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_bottom", 8)
	header_panel.add_child(margin)

	var vbox := VBoxContainer.new()
	vbox.add_theme_constant_override("separation", 5)
	margin.add_child(vbox)

	var row1 := HBoxContainer.new()
	row1.add_theme_constant_override("separation", 8)
	vbox.add_child(row1)

	_election_countdown_label = Label.new()
	_election_countdown_label.text = "Next Election: 180 Days"
	_election_countdown_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_election_countdown_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(_election_countdown_label, 14)
	row1.add_child(_election_countdown_label)

	_overall_approval_label = Label.new()
	_overall_approval_label.text = "Popular Support: 60%"
	_overall_approval_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(_overall_approval_label, 14)
	row1.add_child(_overall_approval_label)

	_overall_delta_label = Label.new()
	_overall_delta_label.text = ""
	_overall_delta_label.visible = false
	UIStyle.set_font_size(_overall_delta_label, 13)
	row1.add_child(_overall_delta_label)

	_election_progress = ProgressBar.new()
	_election_progress.min_value = 0.0
	_election_progress.max_value = 180.0
	_election_progress.value = 0.0
	_election_progress.show_percentage = false
	_election_progress.custom_minimum_size.y = UIStyle.scaled_px(8.0)
	vbox.add_child(_election_progress)

	_volatility_label = Label.new()
	_volatility_label.text = "Campaign Sensitivity: 1.00x  |  Terms Won: 0/0"
	_volatility_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(_volatility_label, 11)
	vbox.add_child(_volatility_label)

	parent.add_child(header_panel)

func _build_factions_section(parent: VBoxContainer) -> void:
	for spec in [
		["property_owners", "Property Owners"],
		["workers_and_youth", "Workers & Youth"],
		["industrialists", "Industrialists"],
		["environmentalists", "Environmentalists"],
	]:
		var bar := FactionApprovalBar.new()
		bar.set_faction_state(spec[0], spec[1], 0.25, 60.0, 0.0, "neutral")
		_faction_bars[spec[0]] = bar
		parent.add_child(bar)

func _build_trend_section(parent: VBoxContainer) -> void:
	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 8)
	parent.add_child(header)

	_details_title_label = Label.new()
	_details_title_label.text = "Land Value Tax"
	_details_title_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_details_title_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(_details_title_label, 14)
	header.add_child(_details_title_label)

	_timeframe_selector = OptionButton.new()
	_timeframe_selector.add_item("Today", 1)
	_timeframe_selector.add_item("7D", 7)
	_timeframe_selector.add_item("30D", 30)
	_timeframe_selector.select(1)
	_timeframe_selector.custom_minimum_size.x = UIStyle.scaled_px(96.0)
	UIStyle.set_font_size(_timeframe_selector, 13)
	_timeframe_selector.item_selected.connect(func(_idx: int): _refresh_selected_policy_trend())
	header.add_child(_timeframe_selector)

	_policy_details_box = VBoxContainer.new()
	_policy_details_box.add_theme_constant_override("separation", 3)
	parent.add_child(_policy_details_box)

	_policy_graph = TrendGraph.new()
	_policy_graph.custom_minimum_size.y = UIStyle.scaled_px(110.0)
	_policy_graph.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_policy_graph.size_flags_vertical = Control.SIZE_EXPAND_FILL
	parent.add_child(_policy_graph)

func update_from_overview(overview: Dictionary) -> void:
	_overview = overview
	var fp: Dictionary = overview.get("fiscal_policy", {})
	var had_staged := has_staged_changes()
	_live_policies = fp.duplicate()
	if not had_staged:
		_staged_policies = fp.duplicate()
		_sync_sliders_to_staged()
		_preview_data = {}
	else:
		_run_preview()
	_refresh_politics_and_badges()
	_refresh_selected_policy_trend()
	_update_action_buttons()

func has_staged_changes() -> bool:
	for key in _staged_policies.keys():
		var live_val := float(_live_policies.get(key, 0.0))
		var staged_val := float(_staged_policies.get(key, 0.0))
		if absf(staged_val - live_val) > 0.0005:
			return true
	return false

func _sync_sliders_to_staged() -> void:
	_syncing_sliders = true
	var controls: Array = _overview.get("fiscal_policy_controls", [])
	var unit_by_id: Dictionary = {}
	for c in controls:
		unit_by_id[str(c.get("id", ""))] = str(c.get("unit", ""))

	for policy_id in _sliders.keys():
		var slider: HSlider = _sliders[policy_id]
		var val := float(_staged_policies.get(policy_id, slider.value))
		slider.value = val
		if _value_labels.has(policy_id):
			var unit := str(unit_by_id.get(policy_id, _default_unit_for_policy(policy_id)))
			_value_labels[policy_id].text = _format_policy_value(val, unit, policy_id)
	_syncing_sliders = false
	_refresh_row_selection_styles()

func _on_slider_staged(value: float, policy_id: String, unit: String) -> void:
	if _syncing_sliders:
		return
	_staged_policies[policy_id] = value
	if _value_labels.has(policy_id):
		_value_labels[policy_id].text = _format_policy_value(value, unit, policy_id)
	if selected_policy_id != policy_id:
		selected_policy_id = policy_id
		_refresh_row_selection_styles()
		_refresh_selected_policy_trend()
		selected_policy_changed.emit(policy_id)
	_run_preview()
	_refresh_politics_and_badges()
	_update_action_buttons()

func _run_preview() -> void:
	if simulation_node != null and simulation_node.has_method("preview_policy_reform"):
		_preview_data = simulation_node.preview_policy_reform(_staged_policies)
	else:
		_preview_data = {}

func _on_apply_reform_pressed() -> void:
	if simulation_node != null and simulation_node.has_method("apply_policy_reform"):
		var updated: Dictionary = simulation_node.apply_policy_reform(_staged_policies)
		_overview = updated
		_live_policies = updated.get("fiscal_policy", {}).duplicate()
		_staged_policies = _live_policies.duplicate()
		_preview_data = {}
		_sync_sliders_to_staged()
		_refresh_politics_and_badges()
		_refresh_selected_policy_trend()
		_update_action_buttons()
		reform_applied.emit(updated)
	elif simulation_node != null and simulation_node.has_method("set_economy_policy_value"):
		for key in _staged_policies.keys():
			simulation_node.set_economy_policy_value(str(key), float(_staged_policies[key]))
		_live_policies = _staged_policies.duplicate()
		_preview_data = {}
		_update_action_buttons()

func _on_reset_preview_pressed() -> void:
	_staged_policies = _live_policies.duplicate()
	_preview_data = {}
	_sync_sliders_to_staged()
	_refresh_politics_and_badges()
	_update_action_buttons()

func _on_single_tax_preset_pressed() -> void:
	_staged_policies[POLICY_RES_PROP_TAX] = 0.0
	_staged_policies[POLICY_COM_PROP_TAX] = 0.0
	_staged_policies[POLICY_IND_PROP_TAX] = 0.0
	if float(_staged_policies.get(POLICY_LVT_RATE, 0.0)) < 0.20:
		_staged_policies[POLICY_LVT_RATE] = 0.25
	_sync_sliders_to_staged()
	_run_preview()
	_refresh_politics_and_badges()
	_update_action_buttons()

func _refresh_politics_and_badges() -> void:
	var budget: Dictionary = _overview.get("city_budget", {})
	var lm: Dictionary = _overview.get("labor_market", {})
	var politics: Dictionary = _overview.get("politics", {})
	var using_preview := not _preview_data.is_empty() and has_staged_changes()

	var assessed_lv := float(budget.get("total_assessed_land_value", 0.0))
	var proj_lvt := float(
		_preview_data.get("projected_lvt_revenue", budget.get("last_day_lvt_revenue", 0.0))
	)
	if _lvt_subtitle_label != null:
		_lvt_subtitle_label.text = (
			"Assessed Land Base: %s  |  Est. LVT Revenue: %s/day (buildings exempt)"
			% [_money(assessed_lv), _money(proj_lvt)]
		)

	var proj_ubi := float(
		_preview_data.get("projected_daily_ubi_per_adult", budget.get("daily_ubi_per_adult", 0.0))
	)
	var proj_surplus := float(
		_preview_data.get("projected_pre_ubi_surplus", budget.get("last_day_pre_ubi_surplus", 0.0))
	)
	var res_wage := float(
		_preview_data.get("projected_reservation_wage", lm.get("reservation_wage", 25.0))
	)
	if _ubi_subtitle_label != null:
		_ubi_subtitle_label.text = (
			"Net Pre-UBI Surplus: %s/day  |  Dividend: $%.2f/adult/day  |  Reservation Wage: $%.1f/day"
			% [_signed_money(proj_surplus), proj_ubi, res_wage]
		)

	var deficit_paused := bool(
		_preview_data.get(
			"deficit_circuit_breaker_active",
			proj_surplus <= 0.0 or float(_overview.get("treasury", 0.0)) <= 0.0
		)
	)
	var ubi_share := float(_staged_policies.get(POLICY_UBI_SURPLUS_SHARE, 0.0))
	if _deficit_badge != null:
		_deficit_badge.visible = deficit_paused and ubi_share > 0.0

	var low_wage_warn := bool(_preview_data.get("low_wage_shortage_warning", false))
	var pub_wage_warn := bool(_preview_data.get("public_sector_wage_warning", false))
	if not using_preview:
		var sectors: Dictionary = lm.get("sectors", {})
		var low_sec: Dictionary = sectors.get("low_wage_commercial", {})
		var pub_sec: Dictionary = sectors.get("public_sector", {})
		low_wage_warn = float(low_sec.get("shortage_ratio", 0.0)) > 0.15 and proj_ubi > 0.0
		pub_wage_warn = float(pub_sec.get("shortage_ratio", 0.0)) > 0.15 and proj_ubi > 0.0
	if _low_wage_warning_badge != null:
		_low_wage_warning_badge.visible = low_wage_warn
	if _public_wage_warning_badge != null:
		_public_wage_warning_badge.visible = pub_wage_warn

	# Election Clock & Overall Approval
	var cycle_days := maxi(1, int(politics.get("election_cycle_days", 180)))
	var days_left := int(politics.get("days_until_election", cycle_days))
	var overall := float(politics.get("overall_approval", 60.0))
	var overall_delta := float(_preview_data.get("overall_delta", 0.0)) if using_preview else 0.0
	var volatility := float(politics.get("campaign_volatility", 1.0))
	var won := int(politics.get("elections_won", 0))
	var completed := int(politics.get("elections_completed", 0))

	if _election_countdown_label != null:
		_election_countdown_label.text = "Next Election: %d Days" % days_left
	if _election_progress != null:
		_election_progress.max_value = float(cycle_days)
		_election_progress.value = float(clampi(cycle_days - days_left, 0, cycle_days))
	if _overall_approval_label != null:
		_overall_approval_label.text = "Popular Support: %d%%" % int(roundf(overall))
	if _overall_delta_label != null:
		if absf(overall_delta) >= 0.1:
			_overall_delta_label.visible = true
			var prefix := "+" if overall_delta > 0.0 else ""
			_overall_delta_label.text = "(%s%.1f%%)" % [prefix, overall_delta]
			_overall_delta_label.add_theme_color_override(
				"font_color",
				Color(0.32, 0.92, 0.62) if overall_delta > 0.0 else Color(0.98, 0.44, 0.36)
			)
		else:
			_overall_delta_label.visible = false
	if _volatility_label != null:
		_volatility_label.text = (
			"Campaign Sensitivity: %.2fx  |  Elections Won: %d/%d"
			% [volatility, won, completed]
		)

	var faction_list: Array = (
		_preview_data.get("factions", [])
		if using_preview and _preview_data.has("factions")
		else politics.get("factions", [])
	)
	for f_dict in faction_list:
		var fid := str(f_dict.get("id", ""))
		if _faction_bars.has(fid):
			var bar = _faction_bars[fid]
			var base_app := float(f_dict.get("current_approval", f_dict.get("approval", 60.0)))
			var delta := float(f_dict.get("approval_delta", 0.0)) if using_preview else 0.0
			bar.set_faction_state(
				fid,
				str(f_dict.get("name", fid)),
				float(f_dict.get("electorate_share", 0.25)),
				base_app,
				delta,
				str(f_dict.get("dominant_issue", "neutral"))
			)

func _refresh_selected_policy_trend() -> void:
	if _policy_details_box == null or _policy_graph == null:
		return
	for child in _policy_details_box.get_children():
		_policy_details_box.remove_child(child)
		child.queue_free()

	var controls: Array = _overview.get("fiscal_policy_controls", [])
	var control: Dictionary = {}
	for c in controls:
		if str(c.get("id", "")) == selected_policy_id:
			control = c
			break
	if control.is_empty():
		return

	var days := _timeframe_selector.get_selected_id() if _timeframe_selector != null else 7
	var entries := _entries_for_days(days)
	var impact_field := str(control.get("impact_field", "net"))
	var impact_label := str(control.get("impact_label", "Impact"))
	var impact_kind := str(control.get("impact_kind", "revenue"))
	var label_text := str(control.get("label", selected_policy_id))
	var unit := str(control.get("unit", "percent"))
	if _details_title_label != null:
		_details_title_label.text = "%s — %s" % [label_text, impact_label]

	var total := 0.0
	var series: Array = []
	var labels: Array = []
	for idx in range(entries.size()):
		var e: Dictionary = entries[idx]
		var v := float(e.get(impact_field, 0.0))
		total += v
		series.append(v)
		labels.append("D%d" % int(e.get("day_index", idx + 1)))

	var color := Color(0.22, 0.74, 0.48) if impact_kind != "expense" else Color(0.92, 0.42, 0.32)
	var summary := HBoxContainer.new()
	summary.add_theme_constant_override("separation", 12)
	_policy_details_box.add_child(summary)

	var live_val := float(_live_policies.get(selected_policy_id, control.get("value", 0.0)))
	var staged_val := float(_staged_policies.get(selected_policy_id, live_val))
	var info := Label.new()
	info.text = "Live: %s   |   Staged: %s   |   %dD Total: %s" % [
		_format_policy_value(live_val, unit, selected_policy_id),
		_format_policy_value(staged_val, unit, selected_policy_id),
		entries.size(),
		_money(total),
	]
	info.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(info, 12)
	summary.add_child(info)

	_policy_graph.set_series([series], [color], [impact_label], labels, "$")

func _create_slider_row(
	policy_id: String,
	label_text: String,
	min_val: float,
	max_val: float,
	step_val: float,
	unit: String
) -> PanelContainer:
	var row := PanelContainer.new()
	row.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	row.add_theme_stylebox_override("panel", _row_style(policy_id == selected_policy_id))
	row.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
	row.gui_input.connect(func(event: InputEvent) -> void:
		if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
			if selected_policy_id != policy_id:
				selected_policy_id = policy_id
				_refresh_row_selection_styles()
				_refresh_selected_policy_trend()
				selected_policy_changed.emit(policy_id)
	)
	_row_panels[policy_id] = row

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 8)
	margin.add_theme_constant_override("margin_right", 8)
	margin.add_theme_constant_override("margin_top", 6)
	margin.add_theme_constant_override("margin_bottom", 6)
	row.add_child(margin)

	var box := VBoxContainer.new()
	box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	box.add_theme_constant_override("separation", 4)
	margin.add_child(box)

	var top := HBoxContainer.new()
	top.add_theme_constant_override("separation", 8)
	box.add_child(top)

	var title := Label.new()
	title.text = label_text
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	title.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(title, 13)
	top.add_child(title)

	var val_label := Label.new()
	val_label.text = _format_policy_value(min_val, unit, policy_id)
	val_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	val_label.custom_minimum_size.x = UIStyle.scaled_px(80.0)
	val_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(val_label, 13)
	top.add_child(val_label)
	_value_labels[policy_id] = val_label

	var slider := HSlider.new()
	slider.min_value = min_val
	slider.max_value = max_val
	slider.step = step_val
	slider.value = min_val
	slider.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	slider.value_changed.connect(_on_slider_staged.bind(policy_id, unit))
	box.add_child(slider)
	_sliders[policy_id] = slider

	return row

func _create_warning_badge(text: String, fg: Color, bg: Color) -> PanelContainer:
	var panel := PanelContainer.new()
	panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	panel.add_theme_stylebox_override("panel", UIStyle.panel_style(bg, 5, fg, 1))
	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 8)
	margin.add_theme_constant_override("margin_right", 8)
	margin.add_theme_constant_override("margin_top", 4)
	margin.add_theme_constant_override("margin_bottom", 4)
	panel.add_child(margin)
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	label.add_theme_color_override("font_color", fg)
	UIStyle.set_font_size(label, 11)
	margin.add_child(label)
	return panel

func _section_panel(title: String, child: Control) -> PanelContainer:
	var panel := PanelContainer.new()
	panel.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	panel.size_flags_vertical = Control.SIZE_EXPAND_FILL
	panel.add_theme_stylebox_override("panel", UIStyle.panel_style(Color(0.10, 0.10, 0.14, 0.78), 6))
	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 10)
	margin.add_theme_constant_override("margin_right", 10)
	margin.add_theme_constant_override("margin_top", 8)
	margin.add_theme_constant_override("margin_bottom", 8)
	panel.add_child(margin)
	var box := VBoxContainer.new()
	box.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	box.size_flags_vertical = Control.SIZE_EXPAND_FILL
	box.add_theme_constant_override("separation", 6)
	margin.add_child(box)
	var hdr := Label.new()
	hdr.text = title
	hdr.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	UIStyle.set_font_size(hdr, 13)
	box.add_child(hdr)
	child.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	child.size_flags_vertical = Control.SIZE_EXPAND_FILL
	box.add_child(child)
	return panel

func _refresh_row_selection_styles() -> void:
	for pid in _row_panels.keys():
		var panel: PanelContainer = _row_panels[pid]
		panel.add_theme_stylebox_override("panel", _row_style(pid == selected_policy_id))

func _row_style(selected: bool) -> StyleBoxFlat:
	if selected:
		return UIStyle.panel_style(Color(0.16, 0.22, 0.27, 0.92), 6, Color(0.28, 0.70, 0.80, 0.75), 1)
	return UIStyle.panel_style(Color(0.09, 0.09, 0.12, 0.70), 6, Color(0.30, 0.30, 0.45, 0.35), 1)

func _update_action_buttons() -> void:
	var dirty := has_staged_changes()
	if _apply_btn != null:
		_apply_btn.disabled = not dirty
		_apply_btn.text = "Apply Reform *" if dirty else "Apply Reform"
	if _reset_btn != null:
		_reset_btn.disabled = not dirty

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

func _default_unit_for_policy(policy_id: String) -> String:
	if policy_id == POLICY_PUBLIC_WAGE_MULTIPLIER or policy_id == "property_tax_level_multiplier":
		return "multiplier"
	if policy_id == "unemployment_max_days":
		return "days"
	if policy_id in [POLICY_LVT_RATE, POLICY_UBI_SURPLUS_SHARE, POLICY_AUTOMATION_LEVEL, "income_tax", "household_vat", "business_profit_tax"]:
		return "percent"
	return "currency_per_day"

func _format_policy_value(value: float, unit: String, policy_id: String = "") -> String:
	if policy_id == POLICY_UBI_SURPLUS_SHARE and value <= 0.001:
		return "0% (OFF)"
	match unit:
		"percent":
			return "%d%%" % int(roundf(clampf(value, 0.0, 1.0) * 100.0))
		"currency_per_day":
			return "%s/day" % _money(value)
		"days":
			return "%dd" % int(roundf(value))
		"multiplier":
			return "x%.2f" % value
		_:
			return "%.2f" % value

func _money(value: float) -> String:
	var sign_str := "-" if value < 0.0 else ""
	var amount := absf(value)
	if amount >= 1000000.0:
		return "%s$%.1fM" % [sign_str, amount / 1000000.0]
	if amount >= 1000.0:
		return "%s$%.1fk" % [sign_str, amount / 1000000.0 * 1000.0]
	return "%s$%.0f" % [sign_str, amount]

func _signed_money(value: float) -> String:
	var prefix := "+" if value > 0.0 else ""
	return prefix + _money(value)







