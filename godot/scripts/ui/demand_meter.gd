## Compact R/C/I demand meter for the gameplay HUD.
##
## Displays normalized residential, commercial, and industrial demand pressures
## as vertical bars around a zero baseline. Values are expected in the range
## -1.0..1.0 and are rendered as -100%..100% in bar height.
extends VBoxContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

const BAR_ORDER := ["residential", "commercial", "industrial"]
const BAR_COLORS := {
	"residential": UIStyle.ZONE_RESIDENTIAL,
	"commercial": UIStyle.ZONE_COMMERCIAL,
	"industrial": UIStyle.ZONE_INDUSTRIAL,
}

var _title_label: Label
var _chart: Control
var _baseline: ColorRect
var _bars: Dictionary = {}
var _pressures := {
	"residential": 0.0,
	"commercial": 0.0,
	"industrial": 0.0,
}

func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_theme_constant_override("separation", 6)

	_title_label = Label.new()
	_title_label.text = "RCI"
	_title_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	UIStyle.set_font_size(_title_label, UIStyle.HUD_TEXT_SIZE)
	_title_label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	add_child(_title_label)

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

func set_pressures(residential: float, commercial: float, industrial: float) -> void:
	_pressures["residential"] = clampf(residential, -1.0, 1.0)
	_pressures["commercial"] = clampf(commercial, -1.0, 1.0)
	_pressures["industrial"] = clampf(industrial, -1.0, 1.0)
	_layout_chart()
	_update_tooltip()

func _layout_chart() -> void:
	if _chart == null or _baseline == null or _bars.size() != BAR_ORDER.size():
		return

	var chart_size := _chart.size
	if chart_size.x <= 0.0 or chart_size.y <= 0.0:
		return

	var baseline_y := floorf(chart_size.y * 0.5)
	_baseline.position = Vector2(0.0, baseline_y - 1.0)
	_baseline.size = Vector2(chart_size.x, 2.0)

	var gap := 6.0
	var bar_width := floorf((chart_size.x - gap * float(BAR_ORDER.size() - 1)) / float(BAR_ORDER.size()))
	bar_width = maxf(bar_width, 8.0)
	var used_width := bar_width * float(BAR_ORDER.size()) + gap * float(BAR_ORDER.size() - 1)
	var left := floorf((chart_size.x - used_width) * 0.5)
	var max_up := maxf(1.0, baseline_y - 2.0)
	var max_down := maxf(1.0, chart_size.y - baseline_y - 2.0)

	for i in range(BAR_ORDER.size()):
		var key: String = BAR_ORDER[i]
		var bar: ColorRect = _bars[key]
		var value: float = _pressures[key]
		var magnitude := absf(value)
		if magnitude < 0.01:
			bar.visible = false
			continue

		bar.visible = true
		var height := (max_up if value >= 0.0 else max_down) * magnitude
		var x := left + float(i) * (bar_width + gap)
		if value >= 0.0:
			bar.position = Vector2(x, baseline_y - height)
		else:
			bar.position = Vector2(x, baseline_y)
		bar.size = Vector2(bar_width, maxf(1.0, height))

func _update_tooltip() -> void:
	tooltip_text = "Residential %+.0f%%\nCommercial %+.0f%%\nIndustrial %+.0f%%" % [
		_pressures["residential"] * 100.0,
		_pressures["commercial"] * 100.0,
		_pressures["industrial"] * 100.0,
	]

func _meter_bar_color(key: String) -> Color:
	var base: Color = BAR_COLORS[key]
	return Color(
		lerpf(base.r, 1.0, 0.28),
		lerpf(base.g, 1.0, 0.28),
		lerpf(base.b, 1.0, 0.28),
		1.0
	)
