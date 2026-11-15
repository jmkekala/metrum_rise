# SPDX-License-Identifier: GPL-2.0-only

## A build-menu asset tile shows the asset's own icon (its manifest `icon`), else its class
## icon; a named icon the game lacks falls back to the class icon too.
extends SceneTree

const AssetPanel = preload("res://scripts/ui/asset_panel.gd")

var _failures := 0

func _initialize() -> void:
	_expect(AssetPanel.asset_icon({"icon": "parking_lot"}, "parking") == "parking_lot",
		"An asset's own icon wins over its class icon")
	_expect(AssetPanel.asset_icon({}, "power") == "power", "Without one the tile shows the class icon")
	_expect(AssetPanel.asset_icon({"asset_id": "x", "icon": "no_such_icon"}, "power") == "power",
		"An icon the game lacks falls back to the class icon")
	var tile := AssetPanel.asset_tile({"display_name": "Lot", "icon": "parking_lot_small"}, "parking")
	_expect(tile.text == "Lot", "The tile keeps the asset's name")
	tile.free()
	if _failures == 0:
		print("PASS asset_menu_icon_test")
	quit(1 if _failures > 0 else 0)

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)
