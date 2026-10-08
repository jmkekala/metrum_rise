# SPDX-License-Identifier: GPL-2.0-only

## Compact R/C/I demand meter for the gameplay HUD.
##
## Displays selected, queued and constructing project counts from the last hourly
## assessment. Rust supplies the explanations; the HUD only scales and draws counts.
extends VBoxContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const BAR_ORDER := ["residential", "commercial", "industrial"]
const BAR_COLORS := {
	"residential": UIStyle.ZONE_RESIDENTIAL,
	"commercial": UIStyle.ZONE_COMMERCIAL,
	"industrial": UIStyle.ZONE_INDUSTRIAL,
}

var _chart: Control
var _baseline: ColorRect
var _bars: Dictionary = {}
var _counts: Dictionary = {}
var _details: Dictionary = {}
var _projects := {
	"residential": 0,
	"commercial": 0,
	"industrial": 0,
}

func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_STOP
	add_theme_constant_override("separation", 6)

	var counts := HBoxContainer.new()
	counts.add_theme_constant_override("separation", 2)
	add_child(counts)
	for key in BAR_ORDER:
		var label := Label.new()
		label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
		label.mouse_filter = Control.MOUSE_FILTER_PASS
		label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
		UIStyle.set_font_size(label, 12)
		label.add_theme_color_override("font_color", _meter_bar_color(key))
		counts.add_child(label)
		_counts[key] = label

	var chart_shell := PanelContainer.new()
	chart_shell.size_flags_vertical = Control.SIZE_EXPAND_FILL
	chart_shell.add_theme_stylebox_override(
		"panel",
		UIStyle.panel_style(Color(0.05, 0.05, 0.07, 0.85), 8, Color(0.65, 0.70, 0.80, 0.45))
	)
	add_child(chart_shell)

	var chart_margin := MarginContainer.new()
	chart_margin.set_anchors_and_offsets_preset(Control.PRESET_FULL_RECT)
	chart_margin.add_theme_constant_override("margin_left", 6)
	chart_margin.add_theme_constant_override("margin_right", 6)
	chart_margin.add_theme_constant_override("margin_top", 5)
	chart_margin.add_theme_constant_override("margin_bottom", 5)
	chart_shell.add_child(chart_margin)

	_chart = Control.new()
	_chart.custom_minimum_size = Vector2(56, 34)
	_chart.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_chart.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_chart.size_flags_vertical = Control.SIZE_EXPAND_FILL
	chart_margin.add_child(_chart)

	_baseline = ColorRect.new()
	_baseline.color = Color(0.82, 0.86, 0.92, 0.55)
	_chart.add_child(_baseline)

	for key in BAR_ORDER:
		var bar := ColorRect.new()
		bar.color = _meter_bar_color(key)
		bar.mouse_filter = Control.MOUSE_FILTER_IGNORE
		_chart.add_child(bar)
		_bars[key] = bar

	_chart.resized.connect(_layout_chart)

	_layout_chart()
	_update_tooltip()

func set_growth(growth: Dictionary) -> void:
	for key in BAR_ORDER:
		var entry: Dictionary = growth.get(key, {})
		_projects[key] = maxi(0, int(entry.get("projects", 0)))
		_details[key] = str(entry.get("detail", "Waiting for the next hourly growth assessment."))
	_layout_chart()
	_update_tooltip()

func _layout_chart() -> void:
	if _chart == null or _baseline == null or _bars.size() != BAR_ORDER.size():
		return

	var chart_size := _chart.size
	if chart_size.x <= 0.0 or chart_size.y <= 0.0:
		return

	var baseline_y := chart_size.y - 2.0
	_baseline.position = Vector2(0, baseline_y - 1.0)
	_baseline.size = Vector2(chart_size.x, 2.0)

	var gap := 6.0
	var bar_width := floorf((chart_size.x - gap * float(BAR_ORDER.size() - 1)) / float(BAR_ORDER.size()))
	bar_width = maxf(bar_width, 8.0)
	var used_width := bar_width * float(BAR_ORDER.size()) + gap * float(BAR_ORDER.size() - 1)
	var left := floorf((chart_size.x - used_width) * 0.5)
	var max_height := maxf(1.0, baseline_y - 2.0)
	var scale := 1
	for key in BAR_ORDER:
		scale = maxi(scale, int(_projects[key]))

	for i in range(BAR_ORDER.size()):
		var key: String = BAR_ORDER[i]
		var bar: ColorRect = _bars[key]
		var count: int = _projects[key]
		bar.visible = count > 0
		var height := max_height * float(count) / float(scale)
		bar.position = Vector2(left + float(i) * (bar_width + gap), baseline_y - height)
		bar.size = Vector2(bar_width, maxf(1.0, height))

func _update_tooltip() -> void:
	var sections: PackedStringArray = []
	for key in BAR_ORDER:
		var detail := str(_details.get(key, "Waiting for the next hourly growth assessment."))
		sections.append("%s — %d projects\n%s" % [key.capitalize(), int(_projects[key]), detail])
		if _counts.has(key):
			_counts[key].text = "%s %d" % [key.left(1).to_upper(), int(_projects[key])]
			_counts[key].tooltip_text = key.capitalize() + "\n" + detail
	tooltip_text = "Growth: selected, queued and constructing projects. Bars share a scale.\n\n" + "\n\n".join(sections)

func _meter_bar_color(key: String) -> Color:
	var base: Color = BAR_COLORS[key]
	return Color(
		lerpf(base.r, 1.0, 0.28),
		lerpf(base.g, 1.0, 0.28),
		lerpf(base.b, 1.0, 0.28),
		1.0
	)
