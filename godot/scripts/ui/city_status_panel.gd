## Compact city status panel for treasury balance and live agent count.
##
## This HUD panel is read-only and intended to sit between the clock/speed panel
## and the R/C/I demand meter. Call `set_stats()` whenever the displayed values
## may have changed; the panel updates its labels only when the values differ.
extends VBoxContainer

const UIStyle = preload("res://scripts/ui/ui_style.gd")

var _treasury_label: Label
var _agents_label: Label
var _displayed_treasury: float = INF
var _displayed_agents: int = -1

func _ready() -> void:
	mouse_filter = Control.MOUSE_FILTER_IGNORE
	add_theme_constant_override("separation", 8)

	_treasury_label = _make_value_label()
	add_child(_treasury_label)

	_agents_label = _make_value_label()
	add_child(_agents_label)

	set_stats(0.0, 0)

func set_stats(treasury_balance: float, agent_count: int) -> void:
	var rounded_treasury := snappedf(treasury_balance, 0.01)
	if is_equal_approx(rounded_treasury, _displayed_treasury) and agent_count == _displayed_agents:
		return

	_displayed_treasury = rounded_treasury
	_displayed_agents = agent_count

	_treasury_label.text = _format_currency(rounded_treasury)
	_agents_label.text = "Agents  %s" % _format_int_with_commas(agent_count)

func _make_value_label() -> Label:
	var label := Label.new()
	UIStyle.set_font_size(label, UIStyle.HUD_TEXT_SIZE)
	label.add_theme_color_override("font_color", UIStyle.TEXT_PRIMARY)
	label.horizontal_alignment = HORIZONTAL_ALIGNMENT_LEFT
	label.clip_text = true
	return label

func _format_currency(value: float) -> String:
	var abs_rounded := int(round(absf(value)))
	var digits := _format_int_with_commas(abs_rounded)
	return "-$%s" % digits if value < 0.0 else "$%s" % digits

func _format_int_with_commas(value: int) -> String:
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
