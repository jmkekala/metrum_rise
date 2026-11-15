# SPDX-License-Identifier: GPL-2.0-only

## Compact city status panel for treasury balance and live agent count.
##
## This HUD panel is read-only and intended to sit between the clock/speed panel
## and the R/C/I demand meter. Call `set_stats()` whenever the displayed values
## may have changed; the panel updates its labels only when the values differ.
extends HBoxContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

var _treasury_label: Label
var _agents_label: Label
var _displayed_treasury: float = INF
var _displayed_agents: int = -1

func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_theme_constant_override("separation", 8)

	var icon := TextureRect.new()
	icon.texture = load("res://assets/ui/icons/money.svg")
	icon.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	icon.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	icon.custom_minimum_size = Vector2(18, 18)
	add_child(icon)
	_treasury_label = _make_value_label()
	add_child(_treasury_label)

	var agents_icon := TextureRect.new()
	agents_icon.texture = load("res://assets/ui/icons/agents.svg")
	agents_icon.expand_mode = TextureRect.EXPAND_IGNORE_SIZE
	agents_icon.stretch_mode = TextureRect.STRETCH_KEEP_ASPECT_CENTERED
	agents_icon.custom_minimum_size = Vector2(16, 16)
	agents_icon.modulate = UIStyle.TEXT_DIM
	add_child(agents_icon)
	_agents_label = _make_value_label()
	add_child(_agents_label)
	UIStyle.set_font_size(_agents_label, 12)
	_agents_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)

	set_stats(0.0, 0)

func set_stats(treasury_balance: float, agent_count: int) -> void:
	var rounded_treasury := snappedf(treasury_balance, 0.01)
	if is_equal_approx(rounded_treasury, _displayed_treasury) and agent_count == _displayed_agents:
		return

	_displayed_treasury = rounded_treasury
	_displayed_agents = agent_count

	_treasury_label.text = format_currency(rounded_treasury)
	_agents_label.text = format_count(agent_count)

func _make_value_label() -> Label:
	var label := Label.new()
	UIStyle.set_font_size(label, 12)
	label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	label.horizontal_alignment = HORIZONTAL_ALIGNMENT_LEFT
	label.vertical_alignment = VERTICAL_ALIGNMENT_CENTER
	return label

## "1,234,567" (the coin icon carries the currency), "-1,234" when negative.
static func format_currency(value: float) -> String:
	var abs_rounded := int(round(absf(value)))
	var digits := format_count(abs_rounded)
	# The coin icon carries the currency.
	return "-%s" % digits if value < 0.0 else digits

## "1,234,567".
static func format_count(value: int) -> String:
	var negative := value < 0
	var digits := str(abs(value))
	var parts: Array[String] = []
	var index := digits.length()
	while index > 3:
		parts.push_front(digits.substr(index - 3, 3))
		index -= 3
	parts.push_front(digits.substr(0, index))
	var joined := ",".join(parts)
	return "-" + joined if negative else joined
