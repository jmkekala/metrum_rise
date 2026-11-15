# SPDX-License-Identifier: GPL-2.0-only

## The road tiles' hover card (`ROAD-101`, `ROAD-119`): the type's name beside its speed limit
## sign, then its layout as small symbol chips (lane arrows, width, median swatch, parking sign,
## turn pockets, and only the unusual sidewalk, lighting and zoning cases), then Rust's cost per
## metre with the money sign, as the road builder shows it. Built from a `get_road_types` entry;
## the Roads menu hands it to `HudTooltip` through `CARD_META`. No Rust calls.
extends RefCounted

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const Glyph = preload("res://scripts/ui/road_builder/road_part_glyph.gd")
const SpeedSignPicker = preload("res://scripts/ui/speed_sign_picker.gd")
const RoadThumbnail = preload("res://scripts/ui/hud_road_thumbnail.gd")
const ICON_DIR := "res://assets/ui/icons/"
# The card's width: the chips wrap within it.
const WIDTH := 250.0
# Lanes drawn as arrows per direction; more show as a count.
const MAX_ARROWS := 3
const CHIP_ICON := 14.0
const SIGN_SIDE := 34.0

## A chip's icon: a road part's surface and glyph, as the builder's tiles draw them.
class PartIcon extends Control:
	var parts: Array = []

	func _draw() -> void:
		var side := size.y
		for index in parts.size():
			var box := Rect2(Vector2(index * (side + 1.0), 0.0), Vector2(side, side))
			Glyph.draw_surface(self, box, parts[index])
			Glyph.draw_glyph(self, box.grow(-1.0), parts[index])

## Turn arrows for the type's pockets, as the tile's badge draws them; `sides` holds false for
## left and true for right.
class TurnIcon extends Control:
	var sides: Array = []

	func _draw() -> void:
		var side := size.y
		for index in sides.size():
			RoadThumbnail.paint_turn_arrow(self, Rect2(Vector2(index * side, 0.0), Vector2.ONE * side), sides[index])

## The speed limit sign beside the name.
class Sign extends Control:
	var kmh := 0
	var font: Font

	func _draw() -> void:
		SpeedSignPicker.paint_sign(self, font, size * 0.5, size.y * 0.46, kmh)

## The card's content for `type`, with `cost` Rust's build cost per metre and upkeep per metre
## and day as shown ("15", "0.10"; `RoadTypeMenu.cost_values`).
static func build(type: Dictionary, cost: PackedStringArray) -> Control:
	var column := VBoxContainer.new()
	column.mouse_filter = Control.MOUSE_FILTER_IGNORE
	column.add_theme_constant_override("separation", 7)
	column.custom_minimum_size.x = UIStyle.scaled_px(WIDTH)
	var heading := HBoxContainer.new()
	heading.add_theme_constant_override("separation", 10)
	column.add_child(heading)
	var title := _label(String(type.get("name", "Road")), 16, UIStyle.TEXT_PRIMARY)
	title.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	title.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	heading.add_child(title)
	var kmh := int(type.get("speed_kmh", 0))
	if kmh > 0:
		var sign := Sign.new()
		sign.kmh = kmh
		sign.font = SpeedSignPicker.sign_font()
		sign.custom_minimum_size = Vector2.ONE * UIStyle.scaled_px(SIGN_SIDE)
		heading.add_child(sign)
	var chips := HFlowContainer.new()
	chips.add_theme_constant_override("h_separation", 5)
	chips.add_theme_constant_override("v_separation", 5)
	column.add_child(chips)
	for chip in _chips(type):
		chips.add_child(chip)
	var money := load(ICON_DIR + "money.svg") as Texture2D
	var costs := HBoxContainer.new()
	costs.add_theme_constant_override("separation", 6)
	costs.add_child(_icon(money))
	costs.add_child(_label("%s / m" % cost[0], 13, UIStyle.TEXT_PRIMARY))
	var gap := Control.new()
	gap.custom_minimum_size.x = 8
	costs.add_child(gap)
	costs.add_child(_icon(money))
	costs.add_child(_label("%s / m / day" % cost[1], 13, UIStyle.TEXT_PRIMARY))
	column.add_child(costs)
	return column

# The layout's chips in reading order. Both sidewalks, lights and zoning are the usual case and
# go unsaid.
static func _chips(type: Dictionary) -> Array[Control]:
	var chips: Array[Control] = []
	var fwd := int(type.get("fwd_lanes", 0))
	var bkw := int(type.get("bkw_lanes", 0))
	if fwd + bkw == 0:
		chips.append(_chip(_parts_icon([{"kind": "footpath"}]), "Footpath"))
	else:
		# The arrows are the count; a count past MAX_ARROWS is written out.
		var count := "%d + %d" % [fwd, bkw] if maxi(fwd, bkw) > MAX_ARROWS else ""
		chips.append(_chip(_lane_icon(bkw, fwd), count))
	chips.append(_chip(null, "↔ %s m" % _metres(float(type.get("total_m", 0.0)))))
	var style := int(type.get("median_style", 0))
	if style > 0:
		var median := {"kind": "median", "style": style}
		chips.append(_chip(_parts_icon([median]), "%s m" % _metres(float(type.get("median_width", 0.0)))))
	var parking := int(type.get("parking", 0)) & 3
	if parking > 0:
		chips.append(_chip(_parts_icon([{"kind": "parking"}]), ["", "Left", "Right", "Both"][parking]))
	var pockets: Dictionary = type.get("pockets", {})
	var left := bool(pockets.get("left", false))
	var right := bool(pockets.get("right", false))
	if fwd + bkw > 0 and (left or right):
		var turns := TurnIcon.new()
		turns.sides = ([false] if left else []) + ([true] if right else [])
		var side := UIStyle.scaled_px(CHIP_ICON)
		turns.custom_minimum_size = Vector2(side * turns.sides.size(), side)
		turns.size_flags_vertical = Control.SIZE_SHRINK_CENTER
		turns.mouse_filter = Control.MOUSE_FILTER_IGNORE
		var chip := _chip(turns, "%d m" % int(pockets.get("body_m", 0.0)))
		_accent(chip)
		chips.append(chip)
	if fwd + bkw > 0:
		match int(type.get("sidewalks", 3)):
			0:
				chips.append(_chip(_parts_icon([{"kind": "sidewalk"}]), "None", true))
			1:
				chips.append(_chip(_parts_icon([{"kind": "sidewalk"}]), "Left only"))
			2:
				chips.append(_chip(_parts_icon([{"kind": "sidewalk"}]), "Right only"))
	if not bool(type.get("street_lights", true)):
		chips.append(_chip(_icon(load(ICON_DIR + "street_light.svg")), "Off", true))
	if not bool(type.get("zoning", true)):
		chips.append(_chip(_icon(load(ICON_DIR + "zoning.svg")), "No zoning", true))
	return chips

# Up to MAX_ARROWS lane arrows each way, backward lanes on the left as the builder draws them.
static func _lane_icon(bkw: int, fwd: int) -> Control:
	var parts: Array = []
	for i in mini(bkw, MAX_ARROWS):
		parts.append({"kind": "car_lane", "forward": false})
	for i in mini(fwd, MAX_ARROWS):
		parts.append({"kind": "car_lane", "forward": true})
	return _parts_icon(parts)

static func _parts_icon(parts: Array) -> Control:
	var icon := PartIcon.new()
	icon.parts = parts
	var side := UIStyle.scaled_px(CHIP_ICON)
	icon.custom_minimum_size = Vector2(parts.size() * (side + 1.0) - 1.0, side)
	icon.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	icon.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return icon

static func _icon(texture: Texture2D) -> TextureRect:
	var rect := TextureRect.new()
	rect.texture = texture
	rect.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	rect.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	rect.custom_minimum_size = Vector2.ONE * UIStyle.scaled_px(CHIP_ICON + 2.0)
	rect.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	rect.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return rect

# A rounded chip: an optional icon and an optional short text; `muted` for a missing feature.
static func _chip(icon: Control, text: String, muted := false) -> PanelContainer:
	var chip := PanelContainer.new()
	chip.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var style := StyleBoxFlat.new()
	style.bg_color = Color(1, 1, 1, 0.06)
	style.border_color = Color(1, 1, 1, 0.10)
	style.set_border_width_all(1)
	style.set_corner_radius_all(6)
	style.content_margin_left = 6
	style.content_margin_right = 7
	style.content_margin_top = 2
	style.content_margin_bottom = 2
	chip.add_theme_stylebox_override("panel", style)
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 5)
	chip.add_child(row)
	if icon != null:
		if muted:
			icon.modulate = Color(1, 1, 1, 0.55)
		row.add_child(icon)
	if not text.is_empty():
		row.add_child(_label(text, 13, UIStyle.TEXT_DIM if muted else UIStyle.TEXT_PRIMARY))
	return chip

static func _accent(chip: PanelContainer) -> void:
	var style := chip.get_theme_stylebox("panel") as StyleBoxFlat
	style.bg_color = Color(UIStyle.HUD_ACCENT, 0.14)
	style.border_color = Color(UIStyle.HUD_ACCENT, 0.55)
	for label in chip.find_children("*", "Label", true, false):
		(label as Label).add_theme_color_override("font_color", UIStyle.HUD_ACCENT)

static func _label(text: String, font_size: int, color: Color) -> Label:
	var label := Label.new()
	label.text = text
	label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	UIStyle.set_font_size(label, font_size)
	label.add_theme_color_override("font_color", color)
	return label

# "10", "2.5": metres without a needless ".0".
static func _metres(value: float) -> String:
	return str(snappedf(value, 0.1)).trim_suffix(".0")
