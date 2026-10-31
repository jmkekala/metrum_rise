# SPDX-License-Identifier: GPL-2.0-only

## Procedural political faction card with a real-time ghost approval bar and dominant issue pill.
##
## Displays a faction's current approval rating (0..100%), electorate weight, dominant grievance
## or praise factor, and a shaded +/- % ghost delta overlay when the player drags policy sliders
## prior to clicking "Apply Reform".
extends PanelContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const POSITIVE_GHOST_FILL := Color(0.24, 0.86, 0.56, 0.48)
const POSITIVE_GHOST_EDGE := Color(0.32, 0.96, 0.66, 0.95)
const NEGATIVE_GHOST_FILL := Color(0.96, 0.34, 0.28, 0.52)
const NEGATIVE_GHOST_EDGE := Color(1.00, 0.46, 0.38, 0.95)
const BAR_BG_COLOR := Color(0.06, 0.07, 0.10, 0.92)
const THRESHOLD_LINE_COLOR := Color(0.85, 0.85, 0.95, 0.45)

var faction_id: String = ""
var faction_name: String = ""
var electorate_share: float = 0.25
var current_approval: float = 60.0
var approval_delta: float = 0.0
var dominant_issue: String = "neutral"

var _name_label: Label
var _share_label: Label
var _issue_badge: PanelContainer
var _issue_label: Label
var _approval_label: Label
var _delta_label: Label
var _bar_canvas: Control

class _GhostBarCanvas extends Control:
	var approval: float = 60.0
	var delta: float = 0.0

	func _draw() -> void:
		var rect := Rect2(Vector2.ZERO, size)
		if rect.size.x <= 2.0 or rect.size.y <= 2.0:
			return
		draw_rect(rect, BAR_BG_COLOR, true)

		var base_ratio := clampf(approval / 100.0, 0.0, 1.0)
		var preview_ratio := clampf((approval + delta) / 100.0, 0.0, 1.0)

		# Base live approval fill color scales from red (<40%) to amber (40-55%) to green (>55%).
		var fill_color := Color(0.24, 0.68, 0.88, 0.88)
		if approval < 40.0:
			fill_color = Color(0.84, 0.36, 0.32, 0.88)
		elif approval >= 55.0:
			fill_color = Color(0.24, 0.74, 0.52, 0.88)

		# If delta is negative, draw the retained portion in solid and the lost portion as a shaded ghost bar.
		var solid_ratio := minf(base_ratio, preview_ratio) if delta < -0.05 else base_ratio
		if solid_ratio > 0.0:
			draw_rect(Rect2(0.0, 0.0, rect.size.x * solid_ratio, rect.size.y), fill_color, true)

		if delta > 0.05 and preview_ratio > base_ratio:
			var gx := rect.size.x * base_ratio
			var gw := rect.size.x * (preview_ratio - base_ratio)
			draw_rect(Rect2(gx, 0.0, gw, rect.size.y), POSITIVE_GHOST_FILL, true)
			draw_line(
				Vector2(gx + gw, 0.0),
				Vector2(gx + gw, rect.size.y),
				POSITIVE_GHOST_EDGE,
				2.0
			)
		elif delta < -0.05 and base_ratio > preview_ratio:
			var gx := rect.size.x * preview_ratio
			var gw := rect.size.x * (base_ratio - preview_ratio)
			draw_rect(Rect2(gx, 0.0, gw, rect.size.y), NEGATIVE_GHOST_FILL, true)
			var step := 6.0
			var x := gx
			while x < gx + gw:
				var x_end := minf(x + 4.0, gx + gw)
				draw_line(
					Vector2(x, rect.size.y),
					Vector2(x_end, 0.0),
					NEGATIVE_GHOST_EDGE,
					1.0
				)
				x += step
			draw_line(
				Vector2(gx, 0.0),
				Vector2(gx, rect.size.y),
				NEGATIVE_GHOST_EDGE,
				2.0
			)

		# 50% majority threshold tick line.
		var mid_x := rect.size.x * 0.5
		draw_line(Vector2(mid_x, 0.0), Vector2(mid_x, rect.size.y), THRESHOLD_LINE_COLOR, 1.5)
		draw_rect(rect, Color(0.32, 0.34, 0.48, 0.55), false, 1.0)

func _init() -> void:
	size_flags_horizontal = Control.SIZE_EXPAND_FILL
	add_theme_stylebox_override(
		"panel",
		UIStyle.panel_style(Color(0.10, 0.11, 0.15, 0.86), 6, Color(0.28, 0.30, 0.44, 0.45), 1)
	)
	_build_card()

func _build_card() -> void:
	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 10)
	margin.add_theme_constant_override("margin_right", 10)
	margin.add_theme_constant_override("margin_top", 7)
	margin.add_theme_constant_override("margin_bottom", 7)
	add_child(margin)

	var vbox := VBoxContainer.new()
	vbox.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	vbox.add_theme_constant_override("separation", 5)
	margin.add_child(vbox)

	var top_row := HBoxContainer.new()
	top_row.add_theme_constant_override("separation", 6)
	vbox.add_child(top_row)

	_name_label = Label.new()
	_name_label.text = "Faction"
	_name_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(_name_label, 14)
	top_row.add_child(_name_label)

	_share_label = Label.new()
	_share_label.text = "(25%)"
	_share_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(_share_label, 12)
	top_row.add_child(_share_label)

	var spacer := Control.new()
	spacer.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	top_row.add_child(spacer)

	_issue_badge = PanelContainer.new()
	_issue_badge.add_theme_stylebox_override(
		"panel",
		UIStyle.panel_style(Color(0.16, 0.18, 0.25, 0.90), 4, Color(0.35, 0.38, 0.52, 0.60), 1)
	)
	var badge_margin := MarginContainer.new()
	badge_margin.add_theme_constant_override("margin_left", 6)
	badge_margin.add_theme_constant_override("margin_right", 6)
	badge_margin.add_theme_constant_override("margin_top", 1)
	badge_margin.add_theme_constant_override("margin_bottom", 1)
	_issue_badge.add_child(badge_margin)

	_issue_label = Label.new()
	_issue_label.text = "Stable"
	_issue_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	UIStyle.set_font_size(_issue_label, 11)
	badge_margin.add_child(_issue_label)
	top_row.add_child(_issue_badge)

	_approval_label = Label.new()
	_approval_label.text = "60%"
	_approval_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	_approval_label.custom_minimum_size.x = UIStyle.scaled_px(42.0)
	_approval_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	UIStyle.set_font_size(_approval_label, 14)
	top_row.add_child(_approval_label)

	_delta_label = Label.new()
	_delta_label.text = ""
	_delta_label.visible = false
	_delta_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_RIGHT
	_delta_label.custom_minimum_size.x = UIStyle.scaled_px(54.0)
	UIStyle.set_font_size(_delta_label, 13)
	top_row.add_child(_delta_label)

	var canvas := _GhostBarCanvas.new()
	canvas.custom_minimum_size.y = UIStyle.scaled_px(12.0)
	canvas.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_bar_canvas = canvas
	vbox.add_child(canvas)

func set_faction_state(
	id: String,
	display_name: String,
	share: float,
	approval: float,
	delta: float,
	issue_code: String
) -> void:
	faction_id = id
	faction_name = display_name
	electorate_share = share
	current_approval = clampf(approval, 0.0, 100.0)
	approval_delta = delta
	dominant_issue = issue_code

	if _name_label != null:
		_name_label.text = display_name
	if _share_label != null:
		_share_label.text = "(%d%%)" % int(roundf(clampf(share, 0.0, 1.0) * 100.0))
	if _approval_label != null:
		_approval_label.text = "%d%%" % int(roundf(current_approval))

	if _delta_label != null:
		if absf(delta) >= 0.1:
			_delta_label.visible = true
			var sign_prefix := "+" if delta > 0.0 else ""
			_delta_label.text = "%s%.1f%%" % [sign_prefix, delta]
			_delta_label.add_theme_color_override(
				"font_color",
				Color(0.32, 0.92, 0.62) if delta > 0.0 else Color(0.98, 0.44, 0.36)
			)
		else:
			_delta_label.visible = false
			_delta_label.text = ""

	_update_issue_badge(issue_code)

	if _bar_canvas != null:
		_bar_canvas.approval = current_approval
		_bar_canvas.delta = delta
		_bar_canvas.queue_redraw()

func _update_issue_badge(issue_code: String) -> void:
	if _issue_label == null or _issue_badge == null:
		return
	var text := "Stable"
	var fg := UIStyle.TEXT_DIM
	var bg := Color(0.16, 0.18, 0.25, 0.90)
	var border := Color(0.35, 0.38, 0.52, 0.60)

	match issue_code:
		"lvt_burden":
			text = "LVT Impact"
			fg = Color(0.96, 0.78, 0.34)
			bg = Color(0.26, 0.20, 0.08, 0.90)
			border = Color(0.76, 0.58, 0.20, 0.75)
		"ubi_dividend":
			text = "UBI Dividend"
			fg = Color(0.32, 0.90, 0.62)
			bg = Color(0.08, 0.24, 0.16, 0.90)
			border = Color(0.24, 0.70, 0.46, 0.75)
		"labor_shortage":
			text = "Labor Shortage"
			fg = Color(1.00, 0.46, 0.38)
			bg = Color(0.28, 0.10, 0.10, 0.92)
			border = Color(0.84, 0.30, 0.26, 0.80)
		"tax_reform":
			text = "Tax Burden"
			fg = Color(0.94, 0.62, 0.36)
			bg = Color(0.26, 0.16, 0.08, 0.90)
			border = Color(0.74, 0.44, 0.22, 0.75)
		"pollution_and_noise":
			text = "Environment"
			fg = Color(0.96, 0.48, 0.42)
			bg = Color(0.26, 0.11, 0.12, 0.90)
			border = Color(0.76, 0.32, 0.30, 0.75)
		"service_reliability":
			text = "Utility Grid"
			fg = Color(0.46, 0.76, 0.98)
			bg = Color(0.10, 0.18, 0.28, 0.90)
			border = Color(0.30, 0.56, 0.82, 0.75)

	_issue_label.text = text
	_issue_label.add_theme_color_override("font_color", fg)
	_issue_badge.add_theme_stylebox_override("panel", UIStyle.panel_style(bg, 4, border, 1))

