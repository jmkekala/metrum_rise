# SPDX-License-Identifier: GPL-2.0-only

## The game's own inspector chrome, shared by the Building Inspector and Road Properties: a
## borderless embedded Window holding a dark rounded card with a soft shadow and a thin accent
## stripe on top, instead of the engine's grey title bar. The header (an icon well, a small
## caps eyebrow, the name, a dim line and chips) drags the window; × closes it (emits the
## window's `close_requested`). Also builds the card's parts: KPI tiles with a meter, level pips,
## status pills and dense label/value grids.
extends RefCounted

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const HudIcons = preload("res://scripts/ui/hud_icons.gd")

const CARD_BG := Color(0.075, 0.08, 0.10, 0.96)
const WELL_BG := Color(1, 1, 1, 0.045)
const DIVIDER := Color(1, 1, 1, 0.07)
const GOOD := Color(0.42, 0.85, 0.52)
const BAD := Color(0.96, 0.42, 0.42)
## Transparent margin around the card the shadow draws into, in base pixels.
const SHADOW_PAD := 10
const PAD := 14

## Zone accents, brighter than the zoning overlay's fills so they read on the dark card.
const ACCENTS := {
	"residential": Color(0.40, 0.80, 0.48),
	"commercial": Color(0.42, 0.64, 0.98),
	"industrial": Color(0.95, 0.74, 0.28),
	"utility": UIStyle.HUD_ACCENT,
}

## Turns `window` into a card and returns its parts: `card` (the PanelContainer), `icon` (the
## HudIcons tile in the well), `eyebrow`, `title`, `subtitle`, `chips` (an HBox under the title),
## `close` and `content` (the VBox below the header).
static func build(window: Window, accent: Color = UIStyle.HUD_ACCENT) -> Dictionary:
	window.borderless = true
	window.transparent = true
	window.transparent_bg = true
	window.unresizable = true

	var shadow_margin := MarginContainer.new()
	shadow_margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	shadow_margin.mouse_filter = Control.MOUSE_FILTER_IGNORE
	for side in ["left", "right", "top", "bottom"]:
		shadow_margin.add_theme_constant_override("margin_" + side, SHADOW_PAD)
	window.add_child(shadow_margin)

	var card := PanelContainer.new()
	card.add_theme_stylebox_override("panel", card_style(accent))
	shadow_margin.add_child(card)

	var root := VBoxContainer.new()
	root.add_theme_constant_override("separation", 10)
	card.add_child(root)

	var header := HBoxContainer.new()
	header.add_theme_constant_override("separation", 12)
	header.mouse_filter = Control.MOUSE_FILTER_STOP
	header.mouse_default_cursor_shape = Control.CURSOR_MOVE
	root.add_child(header)
	_make_draggable(header, window)

	var well := PanelContainer.new()
	well.mouse_filter = Control.MOUSE_FILTER_IGNORE
	well.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	header.add_child(well)
	var icon := HudIcons.icon_button("services", "", "", 40)
	icon.mouse_filter = Control.MOUSE_FILTER_IGNORE
	well.add_child(icon)

	var names := VBoxContainer.new()
	names.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	names.alignment = BoxContainer.ALIGNMENT_CENTER
	names.mouse_filter = Control.MOUSE_FILTER_IGNORE
	names.add_theme_constant_override("separation", 1)
	header.add_child(names)
	var eyebrow := _label("", 10, accent)
	eyebrow.uppercase = true
	eyebrow.add_theme_constant_override("outline_size", 0)
	names.add_child(eyebrow)
	var title := _label("", 17, UIStyle.TEXT_PRIMARY)
	title.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	title.clip_text = true
	title.add_theme_font_override("font", bold_font())
	names.add_child(title)
	var chips := HBoxContainer.new()
	chips.add_theme_constant_override("separation", 6)
	chips.mouse_filter = Control.MOUSE_FILTER_IGNORE
	names.add_child(chips)
	var subtitle := _label("", 12, UIStyle.TEXT_DIM)
	subtitle.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	subtitle.clip_text = true
	subtitle.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	chips.add_child(subtitle)

	var close := Button.new()
	close.text = "✕"
	close.flat = true
	close.focus_mode = Control.FOCUS_NONE
	close.size_flags_vertical = Control.SIZE_SHRINK_BEGIN
	close.tooltip_text = "Close (Esc)"
	close.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
	UIStyle.set_font_size(close, 13)
	close.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	close.add_theme_color_override("font_hover_color", UIStyle.TEXT_PRIMARY)
	close.add_theme_color_override("font_pressed_color", BAD)
	var hover := StyleBoxFlat.new()
	hover.bg_color = Color(1, 1, 1, 0.08)
	hover.set_corner_radius_all(6)
	hover.set_content_margin_all(4)
	var plain := StyleBoxEmpty.new()
	plain.set_content_margin_all(4)
	close.add_theme_stylebox_override("normal", plain)
	close.add_theme_stylebox_override("hover", hover)
	close.add_theme_stylebox_override("pressed", hover)
	close.pressed.connect(func(): window.close_requested.emit())
	header.add_child(close)

	var content := VBoxContainer.new()
	content.add_theme_constant_override("separation", 10)
	root.add_child(content)

	var parts := {
		"card": card, "well": well, "icon": icon, "eyebrow": eyebrow, "title": title,
		"subtitle": subtitle, "chips": chips, "close": close, "content": content,
	}
	set_accent(parts, accent)
	return parts

## Recolours the stripe, the eyebrow and the icon well, e.g. when another zone is inspected.
static func set_accent(parts: Dictionary, accent: Color) -> void:
	(parts["card"] as PanelContainer).add_theme_stylebox_override("panel", card_style(accent))
	(parts["eyebrow"] as Label).add_theme_color_override("font_color", accent)
	var well_style := StyleBoxFlat.new()
	well_style.bg_color = Color(accent, 0.16)
	well_style.border_color = Color(accent, 0.35)
	well_style.set_border_width_all(1)
	well_style.set_corner_radius_all(10)
	well_style.set_content_margin_all(4)
	(parts["well"] as PanelContainer).add_theme_stylebox_override("panel", well_style)
	HudIcons.tint(parts["icon"], accent.lerp(Color.WHITE, 0.35))

# The card: dark, rounded, with the accent as a stripe on top and a soft shadow for its edge.
static func card_style(accent: Color) -> StyleBoxFlat:
	var style := StyleBoxFlat.new()
	style.bg_color = CARD_BG
	style.set_corner_radius_all(12)
	style.border_color = Color(accent, 0.9)
	style.border_width_top = 3
	style.shadow_color = Color(0, 0, 0, 0.45)
	style.shadow_size = SHADOW_PAD - 2
	style.shadow_offset = Vector2(0, 3)
	style.content_margin_left = PAD
	style.content_margin_right = PAD
	style.content_margin_top = PAD - 2
	style.content_margin_bottom = PAD
	return style

## The theme font, emboldened, for names and big numbers.
static func bold_font() -> Font:
	var font := FontVariation.new()
	font.base_font = ThemeDB.fallback_font
	font.variation_embolden = 0.55
	return font

## A small rounded pill, e.g. a business status, in `color`.
static func pill(text: String, color: Color) -> PanelContainer:
	var panel := PanelContainer.new()
	panel.mouse_filter = Control.MOUSE_FILTER_IGNORE
	panel.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	var style := StyleBoxFlat.new()
	style.bg_color = Color(color, 0.16)
	style.border_color = Color(color, 0.45)
	style.set_border_width_all(1)
	style.set_corner_radius_all(8)
	style.content_margin_left = 7
	style.content_margin_right = 7
	style.content_margin_top = 0
	style.content_margin_bottom = 1
	panel.add_theme_stylebox_override("panel", style)
	var label := _label(text, 11, color.lerp(Color.WHITE, 0.25))
	panel.add_child(label)
	return panel

## Five pips, `level` of them lit in `accent`, with "Lv N" beside them.
static func level_pips(level: int, accent: Color, of := 5) -> HBoxContainer:
	var row := HBoxContainer.new()
	row.mouse_filter = Control.MOUSE_FILTER_IGNORE
	row.add_theme_constant_override("separation", 2)
	row.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	row.tooltip_text = "Level %d of %d" % [level, of]
	for i in of:
		var pip := ColorRect.new()
		pip.mouse_filter = Control.MOUSE_FILTER_IGNORE
		pip.custom_minimum_size = UIStyle.scaled_vector2(Vector2(5, 9))
		pip.size_flags_vertical = Control.SIZE_SHRINK_CENTER
		pip.color = accent if i < level else Color(1, 1, 1, 0.14)
		row.add_child(pip)
	return row

## A KPI tile: a small caps caption, a big value and, with `ratio` >= 0, a thin meter under it.
## `tone` colours the value (e.g. GOOD or BAD for a profit); the meter takes `meter_color`.
static func kpi(caption: String, value: String, ratio := -1.0, meter_color := UIStyle.HUD_ACCENT,
		tone := UIStyle.TEXT_PRIMARY, detail := "") -> PanelContainer:
	var tile := PanelContainer.new()
	tile.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	tile.tooltip_text = detail
	var style := StyleBoxFlat.new()
	style.bg_color = WELL_BG
	style.set_corner_radius_all(8)
	style.content_margin_left = 9
	style.content_margin_right = 9
	style.content_margin_top = 6
	style.content_margin_bottom = 7
	tile.add_theme_stylebox_override("panel", style)
	var box := VBoxContainer.new()
	box.mouse_filter = Control.MOUSE_FILTER_IGNORE
	box.add_theme_constant_override("separation", 2)
	tile.add_child(box)
	var cap := _label(caption, 10, UIStyle.TEXT_DIM)
	cap.uppercase = true
	cap.clip_text = true
	box.add_child(cap)
	var val := _label(value, 17, tone)
	val.add_theme_font_override("font", bold_font())
	val.clip_text = true
	val.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	box.add_child(val)
	if ratio >= 0.0:
		box.add_child(meter([[clampf(ratio, 0.0, 1.0), meter_color]], 4))
	return tile

## A thin bar of coloured segments `[[share, color], ...]` (shares of the whole width).
static func meter(segments: Array, height := 4) -> Control:
	var bar := _Meter.new()
	bar.segments = segments
	bar.custom_minimum_size = UIStyle.scaled_vector2(Vector2(0, height))
	bar.mouse_filter = Control.MOUSE_FILTER_IGNORE
	return bar

## A dense label/value grid: two pairs per line when every value is short, else one.
static func grid(pairs: Array) -> GridContainer:
	var wide := false
	for pair in pairs:
		if String(pair[1]).length() > 11 or String(pair[0]).length() > 14:
			wide = true
	var grid_box := GridContainer.new()
	grid_box.columns = 2 if wide else 4
	grid_box.add_theme_constant_override("h_separation", 10 if wide else 16)
	grid_box.add_theme_constant_override("v_separation", 4)
	for pair in pairs:
		var key := _label(String(pair[0]), 12, UIStyle.TEXT_DIM)
		key.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		grid_box.add_child(key)
		var value := _label(String(pair[1]), 13, pair[2] if pair.size() > 2 else UIStyle.TEXT_PRIMARY)
		value.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
		grid_box.add_child(value)
	return grid_box

## A dim small caps heading inside a tab.
static func caption(text: String) -> Label:
	var label := _label(text, 10, UIStyle.TEXT_SECTION)
	label.uppercase = true
	return label

static func _label(text: String, size: int, color: Color) -> Label:
	var label := Label.new()
	label.text = text
	label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	label.add_theme_color_override("font_color", color)
	UIStyle.set_font_size(label, size)
	return label

# Dragging the header moves the window; the motion is the window's own, so it follows the mouse.
static func _make_draggable(handle: Control, window: Window) -> void:
	handle.gui_input.connect(func(event: InputEvent) -> void:
		var motion := event as InputEventMouseMotion
		if motion and (motion.button_mask & MOUSE_BUTTON_MASK_LEFT) != 0:
			window.position += Vector2i(motion.relative.round())
	)

class _Meter extends Control:
	var segments: Array = []

	func _draw() -> void:
		var radius := size.y * 0.5
		var track := StyleBoxFlat.new()
		track.bg_color = Color(1, 1, 1, 0.08)
		track.set_corner_radius_all(int(radius))
		draw_style_box(track, Rect2(Vector2.ZERO, size))
		var x := 0.0
		for segment in segments:
			var w := size.x * float(segment[0])
			if w <= 0.5:
				continue
			var fill := StyleBoxFlat.new()
			fill.bg_color = segment[1]
			fill.set_corner_radius_all(int(radius))
			draw_style_box(fill, Rect2(Vector2(x, 0), Vector2(w, size.y)))
			x += w
