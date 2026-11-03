# SPDX-License-Identifier: GPL-2.0-only

## One tile of the road builder's part palette (`ROAD-101`): the part's glyph and name. A click
## emits `picked` (the builder inserts the part at the selected slot); a drag carries the part
## to the strip (`{"road_builder": "tile", "part": ...}`, which only the strip accepts).
extends Button

const RoadPartGlyph = preload("res://scripts/ui/road_builder/road_part_glyph.gd")
const UIStyle = preload("res://scripts/ui/ui_style.gd")
const GLYPH_SIZE := 24.0

## The player clicked the tile.
signal picked(part: Dictionary)

## The part this tile adds (Rust's part dictionary).
var part: Dictionary

func _init(tile_part: Dictionary) -> void:
	part = tile_part
	text = RoadPartGlyph.part_name(part)
	tooltip_text = "%s\n%s\nClick to add it at the selected +, or drag it into the road" % [
		text, RoadPartGlyph.part_hint(part)]
	focus_mode = Control.FOCUS_NONE
	alignment = HORIZONTAL_ALIGNMENT_LEFT
	custom_minimum_size = Vector2(0.0, 36.0)
	# Room on the left for the glyph.
	add_theme_constant_override("h_separation", 0)
	for state in ["normal", "hover", "pressed", "disabled", "focus"]:
		var style := get_theme_stylebox(state).duplicate() as StyleBox
		if style:
			style.content_margin_left = GLYPH_SIZE + 12.0
			add_theme_stylebox_override(state, style)
	UIStyle.set_font_size(self, 13)
	pressed.connect(func(): picked.emit(part.duplicate()))

func _draw() -> void:
	var top := (size.y - GLYPH_SIZE) * 0.5
	RoadPartGlyph.draw_glyph(self, Rect2(Vector2(6.0, top), Vector2(GLYPH_SIZE, GLYPH_SIZE)), part)

func _get_drag_data(_at_position: Vector2) -> Variant:
	if get_viewport().gui_is_dragging():
		set_drag_preview(drag_preview(text))
	return {"road_builder": "tile", "part": part.duplicate()}

## The label that follows the cursor while a part is dragged.
static func drag_preview(label: String) -> Control:
	var preview := Label.new()
	preview.text = label
	var style := UIStyle.panel_style(UIStyle.BG_DARK, 6)
	style.set_content_margin_all(6.0)
	preview.add_theme_stylebox_override("normal", style)
	return preview
