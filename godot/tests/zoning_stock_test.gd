# SPDX-License-Identifier: GPL-3.0-or-later

## Zoning profiles without stock (`TOOLS-32`): Rust names the profiles the enabled packs have a
## building for, the zoning palette greys the others and says why in their tooltips, and the
## new-game dialog's world-free check agrees with a loaded simulation.
extends SceneTree

const TestPacks := preload("res://tests/test_packs.gd")
const ZoningPanel := preload("res://scripts/ui/zoning_panel.gd")
const HudIcons := preload("res://scripts/ui/hud_icons.gd")
const NewGameDialog := preload("res://scripts/ui/new_game_dialog.gd")
const ModPackConfig := preload("res://scripts/core/mod_pack_config.gd")

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var mods := TestPacks.mods_root()
	# The test pack's buildings are all low density; the parking pack has none.
	var with_buildings := PackedStringArray(["kenney", "parking"])
	var without := PackedStringArray(["parking"])
	_expect(SimulationNode.enabled_packs_stock_zones(mods, with_buildings), "the test pack stocks a zone")
	_expect(not SimulationNode.enabled_packs_stock_zones(mods, without), "the parking pack stocks no zone")
	_expect(not SimulationNode.enabled_packs_stock_zones(mods, PackedStringArray()), "no pack stocks no zone")

	var simulation := SimulationNode.new()
	root.add_child(simulation)
	var ids := {}
	for profile in simulation.get_zone_profiles():
		ids[str(profile["id"])] = int(profile["runtime_id"])
	var low: int = ids.get("res_low_housing", 0)
	var medium: int = ids.get("res_medium_housing", 0)
	_expect(low != 0 and medium != 0, "the shipped residential profiles exist")
	simulation.load_asset_packs(mods, without)
	_expect(simulation.get_stocked_zone_profile_ids().is_empty(), "a load without buildings stocks no profile")
	_expect(simulation.load_asset_packs(mods, with_buildings).is_empty(), "the test packs load cleanly")
	var stocked: PackedInt32Array = simulation.get_stocked_zone_profile_ids()
	_expect(low in stocked, "the test pack stocks residential low")
	_expect(not (medium in stocked), "the test pack leaves residential medium empty")

	var stocked_tile := ZoningPanel.tile("residential", "Low", "Houses.", "", "low", low in stocked)
	var empty_tile := ZoningPanel.tile("residential", "Medium", "Rows.", "", "medium", medium in stocked)
	root.add_child(stocked_tile)
	root.add_child(empty_tile)
	var stocked_ink: Color = stocked_tile.get_node(NodePath(HudIcons.FEEDBACK)).ink
	var empty_ink: Color = empty_tile.get_node(NodePath(HudIcons.FEEDBACK)).ink
	_expect(empty_ink == ZoningPanel.NO_STOCK_INK and stocked_ink != ZoningPanel.NO_STOCK_INK, "only the empty profile's tile is greyed")
	_expect(empty_tile.tooltip_text == "Rows.\n\n" + ZoningPanel.NO_STOCK_NOTE, "the empty tile's tooltip says it has no buildings")
	_expect(stocked_tile.tooltip_text == "Houses.", "a stocked tile's tooltip is its description")
	_expect(empty_tile.toggle_mode and not empty_tile.disabled, "the empty tile stays selectable")

	var dialog: Window = NewGameDialog.new()
	root.add_child(dialog)
	await process_frame
	dialog.popup_for_world("world.sqlite")
	await process_frame
	var expected := not SimulationNode.enabled_packs_stock_zones(
		ProjectSettings.globalize_path(NewGameDialog.MODS_DIR), PackedStringArray(ModPackConfig.load_enabled_pack_ids()))
	_expect(dialog.visible and dialog._pack_notice.visible == expected, "the dialog's notice follows the enabled packs")

	dialog.free()
	stocked_tile.free()
	empty_tile.free()
	simulation.free()
	if _failures == 0:
		print("zoning_stock_test: PASS")
	quit(0 if _failures == 0 else 1)
