# SPDX-License-Identifier: GPL-2.0-only

## Checks economy overview money formatting and the asset editor's economy profile selector.
## Editor controllers stay detached so this test never changes user configuration.
extends SceneTree

const EconomyOverview = preload("res://scripts/ui/economy_overview.gd")

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
	var authoring := AssetAuthoringPolicy.new()
	_expect(authoring.reload_catalog().is_empty(), "Authoring catalog loads")
	var metadata := {"asset_class": "building", "placement_mode": "zoned_private", "zone_type": "industrial", "economy_profile": "machinery_factory_basic"}
	var descriptor: Dictionary = JSON.parse_string(authoring.inspect_json(JSON.stringify(metadata)))["descriptor"]
	_expect(descriptor["profiles"].any(func(profile): return profile["id"] == "machinery_factory_basic"), "Asset editor offers the new processor profile")
	_expect(descriptor["selected_profile"]["profile"]["worker_capacity"] == 4 and not descriptor["fields"].has("worker_capacity"), "Machinery profile owns four read-only jobs")
	if _failures == 0:
		print("economy_machinery_test: PASS")
	quit(0 if _failures == 0 else 1)
