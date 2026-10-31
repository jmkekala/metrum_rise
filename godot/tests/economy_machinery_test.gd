# SPDX-License-Identifier: GPL-2.0-only

## Checks economy money/quantity displays and the asset editor's economy profile selector.
## Editor controllers stay detached so this test never changes user configuration.
extends SceneTree

const EconomyOverview = preload("res://scripts/ui/economy_overview.gd")
const BuildingInspector = preload("res://scripts/ui/building_inspector.gd")
const FactionApprovalBar = preload("res://scripts/ui/faction_approval_bar.gd")
const PolicyPoliticsPanel = preload("res://scripts/ui/policy_politics_panel.gd")

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var overview := EconomyOverview.new()
	_expect(overview._expense_money(100.0) == "-$100", "Input purchases display as expenses")
	_expect(overview._expense_money(-100.0) == "+$100", "Input refunds display as returned money")
	_expect(overview._electricity_energy(500.0) == "500.0 kWh", "Electricity displays kWh")
	_expect(overview._electricity_energy(1500.0) == "1.5 MWh", "Large electricity totals display MWh")
	overview.free()
	var inspector := BuildingInspector.new()
	_expect(inspector._resource_units(750.0) == "750.0 kg", "Small deposits display kilograms")
	_expect(inspector._resource_units(20000.0) == "20.0 t", "Large deposits convert kilograms to tonnes")
	_expect(inspector._electricity_quantity(1500.0, "kW", "MW") == "1.5 MW", "Capacity displays power rather than energy")
	var window := Window.new()
	var title := Label.new()
	var body := VBoxContainer.new()
	inspector._populate({"window": window, "title_label": title, "stats_body": body}, {
		"zone_type": "residential", "household_stock_total": 12.0,
		"electricity_consumption_kwh": 1500.0, "electricity_local_kwh": 1000.0,
		"electricity_imported_kwh": 500.0, "electricity_cost": 325.0,
		"inventory": [{"name": "packaged_food", "unit": "kg", "amount": 100.0, "daily_input_units": 200.0}],
	})
	var labels: Array[String] = []
	for row in body.get_children():
		for child in row.get_children():
			if child is Label:
				labels.append(child.text)
	_expect(labels.has("12.0 kg"), "Household supplies display kilograms")
	_expect(labels.has("1.5 MWh") and labels.has("1.0 MWh") and labels.has("500.0 kWh"), "Building consumption, local supply and imports carry energy units")
	_expect(labels.has("$325"), "Building electricity cost remains money")
	_expect(labels.has("100.0 kg (200.0 kg/day at full operation)"), "Stock and recipe rate carry their resource unit")
	body.free()
	title.free()
	window.free()
	inspector.free()
	var authoring := AssetAuthoringPolicy.new()
	_expect(authoring.reload_catalog().is_empty(), "Authoring catalog loads")
	var metadata := {"asset_class": "building", "placement_mode": "zoned_private", "zone_type": "industrial", "economy_profile": "machinery_factory_basic"}
	var descriptor: Dictionary = JSON.parse_string(authoring.inspect_json(JSON.stringify(metadata)))["descriptor"]
	_expect(descriptor["profiles"].any(func(profile): return profile["id"] == "machinery_factory_basic"), "Asset editor offers the new processor profile")
	_expect(descriptor["selected_profile"]["profile"]["worker_capacity"] == 4 and not descriptor["fields"].has("worker_capacity"), "Machinery profile owns four read-only jobs")

	# Verify FactionApprovalBar ghost bar and PolicyPoliticsPanel preview/commit workflow
	var ghost_bar := FactionApprovalBar.new()
	ghost_bar.set_faction_state("property_owners", "Property Owners", 0.25, 58.0, -14.5, "lvt_burden")
	_expect(ghost_bar._delta_label.visible and ghost_bar._delta_label.text == "-14.5%", "Negative ghost delta displays on faction bar")
	_expect(ghost_bar._issue_label.text == "LVT Impact", "Dominant issue badge displays LVT Impact")
	ghost_bar.free()

	var sim := SimulationNode.new()
	var panel := PolicyPoliticsPanel.new()
	panel.simulation_node = sim
	var initial_overview: Dictionary = sim.get_economy_overview()
	panel.update_from_overview(initial_overview)
	_expect(not panel.has_staged_changes(), "Fresh policy panel has no staged changes")
	_expect(panel._value_labels["ubi_surplus_share"].text == "0% (OFF)", "UBI slider defaults to 0% (OFF)")

	# Stage LVT = 45% and UBI = 85% and verify real-time preview ghost deltas + warning badges
	panel._on_slider_staged(0.45, "lvt_rate", "percent")
	panel._on_slider_staged(0.85, "ubi_surplus_share", "percent")
	_expect(panel.has_staged_changes(), "Dragging LVT/UBI sliders marks staged changes")
	_expect(panel._low_wage_warning_badge.visible, "High UBI triggers low-wage labor shortage warning badge")
	var po_bar = panel._faction_bars["property_owners"]
	var wy_bar = panel._faction_bars["workers_and_youth"]
	_expect(po_bar.approval_delta < 0.0, "Property Owners ghost bar shows negative delta under LVT hike")
	_expect(wy_bar.approval_delta > 0.0, "Workers & Youth ghost bar shows positive delta under UBI + LVT")
	_expect(float(sim.get_economy_overview()["fiscal_policy"]["lvt_rate"]) == 0.0, "Live SimulationNode state is untouched before Apply Reform")

	# Commit via Apply Reform
	panel._on_apply_reform_pressed()
	_expect(not panel.has_staged_changes(), "Apply Reform clears staged state")
	_expect(absf(float(sim.get_economy_overview()["fiscal_policy"]["lvt_rate"]) - 0.45) < 0.001, "Apply Reform commits LVT rate to SimulationNode")
	panel.free()
	sim.free()

	if _failures == 0:
		print("economy_machinery_test: PASS")
	quit(0 if _failures == 0 else 1)
