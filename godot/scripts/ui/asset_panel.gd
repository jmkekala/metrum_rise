# SPDX-License-Identifier: GPL-2.0-only

## Service/industry palette view. Packs keep arbitrary class IDs; unknown classes get a glyph.
extends VBoxContainer

const HudIcons = preload("res://scripts/ui/hud_icons.gd")
var asset_panel: PanelContainer
var asset_menu: HBoxContainer
var category_menu: HBoxContainer

# One row: [classes | assets of the open class], so opening a class widens the palette
# instead of stacking a second panel above it. Industry has only the asset tiles.
func build(with_categories: bool) -> void:
	size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	add_theme_constant_override("separation", 8)
	var palette := HBoxContainer.new()
	palette.add_theme_constant_override("separation", 6)
	var shell := HudIcons.shell(palette)
	shell.size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	add_child(shell)
	category_menu = HBoxContainer.new()
	category_menu.alignment = BoxContainer.ALIGNMENT_CENTER
	category_menu.add_theme_constant_override("separation", 6)
	var assets_row := HBoxContainer.new()
	assets_row.add_theme_constant_override("separation", 6)
	asset_menu = HBoxContainer.new()
	asset_menu.alignment = BoxContainer.ALIGNMENT_CENTER
	asset_menu.add_theme_constant_override("separation", 6)
	asset_panel = PanelContainer.new()
	asset_panel.add_theme_stylebox_override("panel", StyleBoxEmpty.new())
	asset_panel.add_child(assets_row)
	if with_categories:
		palette.add_child(category_menu)
		assets_row.add_child(VSeparator.new())
		asset_panel.hide()
	else:
		category_menu.free()
		category_menu = null
	assets_row.add_child(asset_menu)
	palette.add_child(asset_panel)

static func class_icon(class_id: String) -> String:
	return class_id if class_id in ["parking", "power", "water", "waste", "sewage",
		"police", "fire", "healthcare", "education", "transit", "parks", "government", "field", "extractor"] else "asset"

static func class_tile(class_id: String, title: String) -> Button:
	var button := HudIcons.icon_button(class_icon(class_id), title, "", 64)
	button.toggle_mode = true
	return button

static func asset_tile(asset: Dictionary, class_id: String) -> Button:
	var title := str(asset.get("display_name", asset.get("asset_id", "Asset")))
	var details := PackedStringArray()
	if asset.has("resource_id"):
		details.append("Resource: " + str(asset.resource_id).capitalize())
	if asset.has("lot_width_cells") and asset.has("lot_depth_cells"):
		details.append("Lot: %d × %d cells" % [asset.lot_width_cells, asset.lot_depth_cells])
	# Lots and other unstaffed assets have no jobs to report.
	if int(asset.get("worker_capacity", 0)) > 0:
		details.append("Workers: %d" % int(asset.worker_capacity))
	# Only show economics supplied by the bridge; today's asset index has none.
	if asset.has("build_cost"):
		details.append("Cost: $%s" % str(asset.build_cost))
	if asset.has("upkeep"):
		details.append("upkeep: $%s" % str(asset.upkeep))
	var button := HudIcons.icon_button(asset_icon(asset, class_id), title, "\n".join(details), 72)
	button.toggle_mode = true
	return button

## The asset's own menu icon (its manifest `icon`), else its class icon. A named icon the game
## lacks falls back too, logged on the always-on `events` debug channel.
static func asset_icon(asset: Dictionary, class_id: String) -> String:
	var icon_name := str(asset.get("icon", ""))
	if icon_name.is_empty():
		return class_icon(class_id)
	if not ResourceLoader.exists(HudIcons.ICON_DIR + icon_name + ".svg"):
		MetrumDebug.log("events", "%s: no icon '%s'" % [asset.get("asset_id", "?"), icon_name])
		return class_icon(class_id)
	return icon_name
