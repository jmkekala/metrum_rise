# SPDX-License-Identifier: GPL-2.0-only

## Checks economy money/quantity displays and the asset editor's economy profile selector.
## Editor controllers stay detached so this test never changes user configuration.
extends SceneTree

const EconomyOverview = preload("res://scripts/ui/economy_overview.gd")
const BuildingInspector = preload("res://scripts/ui/building_inspector.gd")

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
	overview.free()
	var inspector := BuildingInspector.new()
	_expect(inspector._resource_units(750.0) == "750.0 kg", "Small deposits display kilograms")
	_expect(inspector._resource_units(20000.0) == "20.0 t", "Large deposits convert kilograms to tonnes")
	var window := Window.new()
	var title := Label.new()
	var body := VBoxContainer.new()
	inspector._populate({"window": window, "title_label": title, "stats_body": body}, {
		"zone_type": "residential", "household_stock_total": 12.0,
		"inventory": [{"name": "packaged_food", "unit": "kg", "amount": 100.0, "daily_input_units": 200.0}],
	})
	var labels: Array[String] = []
	for row in body.get_children():
		for child in row.get_children():
			if child is Label:
				labels.append(child.text)
	_expect(labels.has("12.0 kg"), "Household supplies display kilograms")
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
	if _failures == 0:
		print("economy_machinery_test: PASS")
	quit(0 if _failures == 0 else 1)
