# SPDX-License-Identifier: GPL-2.0-only

## Zoning palette structure and tiles; MainUI retains the profile selection handlers.
extends VBoxContainer

const HudIcons = preload("res://scripts/ui/hud_icons.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
var profile_panel: PanelContainer
var profile_menu: HBoxContainer
var mode_menu: HBoxContainer
var type_menu: HBoxContainer
var options_button: Button
var cell_controls: HBoxContainer
## The one palette row: the controller adds the cell-controls shell after the zone palette.
var row: HBoxContainer

# One row, so opening a zone's profiles widens the palette instead of stacking another
# panel above it: [gear · R C I | profiles] [tool options].
func build() -> void:
	size_flags_horizontal = Control.SIZE_SHRINK_CENTER
	add_theme_constant_override("separation", 8)
	cell_controls = preload("res://scripts/ui/zoning_cell_controls.gd").new()
	row = HBoxContainer.new()
	row.alignment = BoxContainer.ALIGNMENT_CENTER
	row.add_theme_constant_override("separation", 8)
	add_child(row)
	var palette := HBoxContainer.new()
	palette.add_theme_constant_override("separation", 6)
	row.add_child(HudIcons.shell(palette))
	mode_menu = HBoxContainer.new()
	palette.add_child(mode_menu)
	options_button = HudIcons.icon_button("gear", "Parcel options", "")
	mode_menu.add_child(options_button)
	options_button.hide()
	type_menu = HBoxContainer.new()
	type_menu.add_theme_constant_override("separation", 6)
	palette.add_child(type_menu)
	# Shown with its divider when a zone type is open; no frame of its own.
	var profile_row := HBoxContainer.new()
	profile_row.add_theme_constant_override("separation", 6)
	profile_row.add_child(VSeparator.new())
	profile_menu = HBoxContainer.new()
	profile_menu.alignment = BoxContainer.ALIGNMENT_CENTER
	profile_menu.add_theme_constant_override("separation", 6)
	profile_row.add_child(profile_menu)
	profile_panel = PanelContainer.new()
	profile_panel.add_theme_stylebox_override("panel", StyleBoxEmpty.new())
	profile_panel.add_child(profile_row)
	palette.add_child(profile_panel)
	profile_panel.hide()

## A zone or profile tile. Medium and high density profiles get their own building silhouette.
static func tile(zone: String, title: String, detail: String, profile_color: String = "", density: String = "") -> Button:
	var icon := zone if zone in ["residential", "commercial", "industrial"] else "zoning"
	if icon != "zoning" and density in ["medium", "high"]:
		icon += "_" + density
	var button := HudIcons.icon_button(icon, title, detail, 64)
	button.toggle_mode = true
	var color: Color = {
		"residential": UIStyle.ZONE_RESIDENTIAL,
		"commercial": UIStyle.ZONE_COMMERCIAL,
		"industrial": UIStyle.ZONE_INDUSTRIAL,
	}.get(zone, UIStyle.TEXT_PRIMARY)
	if not profile_color.is_empty():
		color = Color.from_string(profile_color, color)
	HudIcons.tint(button, color.lightened(0.25))
	return button
