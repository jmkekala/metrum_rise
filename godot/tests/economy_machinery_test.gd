# SPDX-License-Identifier: GPL-2.0-only

## Checks native economy export/playback and the incoming asset's profile selector.
## Editor controllers stay detached so this test never changes user configuration.
extends SceneTree

const AssetEditor = preload("res://scripts/editors/asset_editor.gd")
const EconomyEditor = preload("res://scripts/editors/economy_editor.gd")
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
	var simulation := SimulationNode.new()
	var source := ProjectSettings.globalize_path("res://../economy")
	var payload: Dictionary = JSON.parse_string(simulation.load_economy_project(source))
	_expect(payload.get("ok", false), "Native economy project loads")
	for message in payload.get("validation", []):
		_expect(message.get("severity") != "error", str(message))
	var project: Dictionary = payload.get("project", {})
	var destination := OS.get_temp_dir().path_join("metrum_machinery_editor_%d" % OS.get_process_id())
	var exported: Dictionary = JSON.parse_string(simulation.export_economy_project(JSON.stringify(project), destination))
	_expect(exported.get("ok", false), "Editor exports Machinery and import-only resource definitions: %s" % exported)
	var reloaded: Dictionary = JSON.parse_string(simulation.load_economy_project(destination))
	_expect(reloaded.get("project", {}).get("resources") == project.get("resources"), "Resource identities/prices survive native export")
	var playback: Dictionary = JSON.parse_string(simulation.run_economy_sandbox(JSON.stringify(project), "grocery_bottleneck"))
	_expect(playback.get("ok", false), "Food scenario runs with paid Machinery imports: %s" % playback)
	_expect(float(playback.get("result", {}).get("total_delivered_units", 0.0)) > 0.0, "Maintenance imports keep the food chain operating")

	var asset_editor := AssetEditor.new()
	asset_editor.sim = simulation
	asset_editor._economy_profile_btn = OptionButton.new()
	asset_editor._zone_type_btn = OptionButton.new()
	asset_editor._zone_type_btn.add_item("Industrial")
	asset_editor._zone_types.assign(["industrial"])
	asset_editor._workers_spin = SpinBox.new()
	asset_editor._load_economy_profiles()
	asset_editor._set_economy_profile_selection("machinery_factory_basic")
	_expect(asset_editor._selected_economy_profile_id() == "machinery_factory_basic", "Asset editor offers the new processor profile")
	_expect(asset_editor._workers_spin.value == 4 and not asset_editor._workers_spin.editable, "Machinery profile owns its four jobs")
	asset_editor._economy_profile_btn.free()
	asset_editor._zone_type_btn.free()
	asset_editor._workers_spin.free()
	asset_editor.free()

	var economy_editor := EconomyEditor.new()
	economy_editor._project = project
	economy_editor._selected_kind = "scenario"
	economy_editor._selected_id = "grocery_bottleneck"
	economy_editor._update_scenario_imports(" machinery, steel, machinery ")
	_expect(economy_editor._project.scenarios[0].owa_import_resources == ["machinery", "steel"], "Scenario import editing trims and deduplicates IDs")
	economy_editor.free()
	simulation.free()
	for filename in ["profiles.toml", "controllers.toml", "scenarios.toml", "economy.index.bin"]:
		DirAccess.remove_absolute(destination.path_join(filename))
	DirAccess.remove_absolute(destination)
	if _failures == 0:
		print("economy_machinery_test: PASS")
	quit(0 if _failures == 0 else 1)
