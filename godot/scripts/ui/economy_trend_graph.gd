## Minimal multi-series line graph used by Economy Overview.
##
## The control draws already-aggregated backend values. It does not compute
## simulation outcomes or accounting buckets.
extends Control

const UIStyle = preload("res://scripts/ui/ui_style.gd")

var _series: Array = []
var _colors: Array = []
var _labels: Array = []
var _x_labels: Array = []
var _value_prefix := ""

func _ready() -> void:
	custom_minimum_size = Vector2(0.0, 116.0)
	size_flags_horizontal = Control.SIZE_EXPAND_FILL

func set_series(
	series: Array,
	colors: Array,
	labels: Array = [],
	x_labels: Array = [],
	value_prefix: String = ""
) -> void:
	_series = series
	_colors = colors
	_labels = labels
	_x_labels = x_labels
	_value_prefix = value_prefix
	queue_redraw()

func _draw() -> void:
	var rect := Rect2(Vector2.ZERO, size)
	draw_rect(rect, Color(0.07, 0.07, 0.10, 0.82), true)
	draw_rect(rect, Color(0.30, 0.30, 0.45, 0.45), false, 1.0)

	var font := get_theme_default_font()
	var font_size := maxi(10, get_theme_default_font_size() - 2)

	var value_min := INF
	var value_max := -INF
	var point_count := 0
	for raw_series in _series:
		var values: Array = raw_series
		point_count = maxi(point_count, values.size())
		for value_variant in values:
			var value := float(value_variant)
			value_min = minf(value_min, value)
			value_max = maxf(value_max, value)

	if point_count == 0 or not is_finite(value_min) or not is_finite(value_max):
		draw_string(
			font,
			Vector2(10.0, 24.0),
			"No data",
			HORIZONTAL_ALIGNMENT_LEFT,
			-1.0,
			font_size,
			UIStyle.TEXT_DIM
		)
		return

	if absf(value_max - value_min) < 0.001:
		value_max += 1.0
		value_min -= 1.0

	_draw_legend(font, font_size)

	var graph_rect := Rect2(
		Vector2(58.0, 28.0),
		Vector2(maxf(1.0, rect.size.x - 70.0), maxf(1.0, rect.size.y - 52.0))
	)
	graph_rect.size.x = maxf(graph_rect.size.x, 1.0)
	graph_rect.size.y = maxf(graph_rect.size.y, 1.0)

	_draw_axes(font, font_size, graph_rect, value_min, value_max)

	draw_line(
		Vector2(graph_rect.position.x, graph_rect.end.y),
		Vector2(graph_rect.end.x, graph_rect.end.y),
		Color(0.45, 0.45, 0.55, 0.45),
		1.0
	)

	for series_idx in range(_series.size()):
		var values: Array = _series[series_idx]
		if values.is_empty():
			continue
		var color := UIStyle.TEXT_SECTION
		if series_idx < _colors.size():
			color = _colors[series_idx]
		var points := PackedVector2Array()
		for idx in range(values.size()):
			var x_ratio := 0.5 if values.size() == 1 else float(idx) / float(values.size() - 1)
			var value := float(values[idx])
			var y_ratio := (value - value_min) / (value_max - value_min)
			points.push_back(Vector2(
				graph_rect.position.x + graph_rect.size.x * x_ratio,
				graph_rect.end.y - graph_rect.size.y * y_ratio
			))
		_draw_series(points, color, series_idx)

func _draw_legend(font: Font, font_size: int) -> void:
	var x := 8.0
	var y := 18.0
	for series_idx in range(_series.size()):
		var label := _series_label(series_idx)
		if label.is_empty():
			continue
		var color := _series_color(series_idx)
		_draw_styled_segment(Vector2(x, y - 7.0), Vector2(x + 16.0, y - 7.0), color, series_idx, 3.0)
		draw_string(
			font,
			Vector2(x + 22.0, y),
			label,
			HORIZONTAL_ALIGNMENT_LEFT,
			-1.0,
			font_size,
			UIStyle.TEXT_PRIMARY
		)
		x += 118.0
		if x + 110.0 > size.x:
			x = 8.0
			y += 16.0

func _draw_series(points: PackedVector2Array, color: Color, series_idx: int) -> void:
	if points.is_empty():
		return
	if points.size() == 1:
		_draw_marker(points[0], color, series_idx)
		return
	for idx in range(points.size() - 1):
		_draw_styled_segment(points[idx], points[idx + 1], color, series_idx, 2.0)
	for point in points:
		_draw_marker(point, color, series_idx)

func _draw_styled_segment(
	from_point: Vector2,
	to_point: Vector2,
	color: Color,
	series_idx: int,
	width: float
) -> void:
	var style := series_idx % 3
	if style == 0:
		draw_line(from_point, to_point, color, width, true)
		return
	var dash_len := 8.0 if style == 1 else 2.0
	var gap_len := 5.0 if style == 1 else 4.0
	var segment := to_point - from_point
	var length := segment.length()
	if length <= 0.01:
		return
	var direction := segment / length
	var cursor := 0.0
	while cursor < length:
		var end_cursor := minf(cursor + dash_len, length)
		draw_line(
			from_point + direction * cursor,
			from_point + direction * end_cursor,
			color,
			width,
			true
		)
		cursor += dash_len + gap_len

func _draw_marker(point: Vector2, color: Color, series_idx: int) -> void:
	match series_idx % 3:
		0:
			draw_circle(point, 2.4, color)
		1:
			draw_rect(Rect2(point - Vector2(2.4, 2.4), Vector2(4.8, 4.8)), color, true)
		_:
			draw_colored_polygon(
				PackedVector2Array([
					point + Vector2(0.0, -3.0),
					point + Vector2(3.0, 0.0),
					point + Vector2(0.0, 3.0),
					point + Vector2(-3.0, 0.0),
				]),
				color
			)

func _draw_axes(
	font: Font,
	font_size: int,
	graph_rect: Rect2,
	value_min: float,
	value_max: float
) -> void:
	var ticks := [
		{"ratio": 0.0, "value": value_min},
		{"ratio": 0.5, "value": (value_min + value_max) * 0.5},
		{"ratio": 1.0, "value": value_max},
	]
	for tick in ticks:
		var ratio := float(tick["ratio"])
		var y := graph_rect.end.y - graph_rect.size.y * ratio
		var color := Color(0.35, 0.35, 0.45, 0.30 if ratio == 0.5 else 0.45)
		draw_line(Vector2(graph_rect.position.x, y), Vector2(graph_rect.end.x, y), color, 1.0)
		draw_string(
			font,
			Vector2(4.0, y + 4.0),
			_format_axis_value(float(tick["value"])),
			HORIZONTAL_ALIGNMENT_RIGHT,
			graph_rect.position.x - 8.0,
			font_size,
			UIStyle.TEXT_DIM
		)

	draw_line(
		Vector2(graph_rect.position.x, graph_rect.position.y),
		Vector2(graph_rect.position.x, graph_rect.end.y),
		Color(0.45, 0.45, 0.55, 0.55),
		1.0
	)
	draw_line(
		Vector2(graph_rect.position.x, graph_rect.end.y),
		Vector2(graph_rect.end.x, graph_rect.end.y),
		Color(0.45, 0.45, 0.55, 0.55),
		1.0
	)
	_draw_x_labels(font, font_size, graph_rect)

func _draw_x_labels(font: Font, font_size: int, graph_rect: Rect2) -> void:
	if _x_labels.is_empty():
		return
	var indices: Array[int] = [0]
	if _x_labels.size() > 2:
		indices.append(_x_labels.size() / 2)
	if _x_labels.size() > 1:
		indices.append(_x_labels.size() - 1)

	var used: Dictionary = {}
	for idx in indices:
		if used.has(idx):
			continue
		used[idx] = true
		var ratio := 0.5 if _x_labels.size() == 1 else float(idx) / float(_x_labels.size() - 1)
		var x := graph_rect.position.x + graph_rect.size.x * ratio
		var label := str(_x_labels[idx])
		var alignment := HORIZONTAL_ALIGNMENT_CENTER
		var width := 80.0
		var draw_x := x - width * 0.5
		if idx == 0:
			alignment = HORIZONTAL_ALIGNMENT_LEFT
			draw_x = x
		elif idx == _x_labels.size() - 1:
			alignment = HORIZONTAL_ALIGNMENT_RIGHT
			draw_x = x - width
		draw_string(
			font,
			Vector2(draw_x, graph_rect.end.y + 17.0),
			label,
			alignment,
			width,
			font_size,
			UIStyle.TEXT_DIM
		)

func _series_label(series_idx: int) -> String:
	if series_idx < _labels.size():
		return str(_labels[series_idx])
	return "Series %d" % (series_idx + 1)

func _series_color(series_idx: int) -> Color:
	if series_idx < _colors.size():
		return _colors[series_idx]
	return UIStyle.TEXT_SECTION

func _format_axis_value(value: float) -> String:
	var sign := "-" if value < 0.0 else ""
	var amount := absf(value)
	var prefix := _value_prefix
	if amount >= 1000000.0:
		return "%s%s%.1fM" % [sign, prefix, amount / 1000000.0]
	if amount >= 1000.0:
		return "%s%s%.1fk" % [sign, prefix, amount / 1000.0]
	if amount < 10.0 and absf(amount - roundf(amount)) > 0.01:
		return "%s%s%.1f" % [sign, prefix, amount]
	return "%s%s%.0f" % [sign, prefix, amount]
