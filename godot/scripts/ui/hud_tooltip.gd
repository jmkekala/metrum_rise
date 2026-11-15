# SPDX-License-Identifier: GPL-2.0-only

## Event-driven HUD tooltip. Reads the owner's text and tooltip_text when shown, so library
## edits and selection changes need no second copy of the content. Never takes mouse focus.
extends Node

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const DELAY := 0.15
## Control meta holding the shortcut key drawn as a keycap; see HudIcons.set_shortcut.
const SHORTCUT_META := &"hud_shortcut"
## Control meta replacing tooltip_text as the card's body ("" for a title-only card).
const BODY_META := &"hud_tooltip_body"
## Control meta holding a Callable that builds the card's whole content (a Control), in place of
## the title and body, e.g. the road tiles' `RoadTypeCard`.
const CARD_META := &"hud_tooltip_card"
## How long `flash` shows the card for a key press, in seconds.
const FLASH_S := 1.6

var _button: Control
var _title := ""
var _timer: Timer
var _layer: CanvasLayer
var _card: PanelContainer
var _flash_timer: Timer
# The button's rect the open card was placed against.
var _anchor := Rect2()

## Installs one delayed tooltip on a HUD button; the button suppresses its native tooltip.
func attach(button: Control, title: String = "") -> void:
	_button = button
	_title = title
	button.add_child(self)
	_timer = Timer.new()
	_timer.one_shot = true
	_timer.wait_time = DELAY
	add_child(_timer)
	_timer.timeout.connect(_show)
	button.mouse_entered.connect(func(): _timer.start())
	button.mouse_exited.connect(_hide)
	if button is BaseButton:
		button.button_down.connect(_hide)
	button.visibility_changed.connect(_hide)
	button.resized.connect(_hide)
	set_process(false)

# While a card is open: a button that moves without resizing (its row laid out after a key
# opened the palette and flashed the card in the same frame) takes its card along.
func _process(_delta: float) -> void:
	if is_instance_valid(_card) and _button.get_global_rect() != _anchor:
		_place()

func _show() -> void:
	if not _button.is_visible_in_tree() or not _hovered():
		return
	_open()

## Shows the card for `FLASH_S` without a hover, for a key that changed what the button shows
## (the Roads menu's custom road key). Hovering keeps it open as usual.
func flash() -> void:
	if not _button.is_visible_in_tree():
		return
	_hide()
	_open()
	if _flash_timer == null:
		_flash_timer = Timer.new()
		_flash_timer.one_shot = true
		add_child(_flash_timer)
		_flash_timer.timeout.connect(func():
			if not _hovered():
				_hide())
	_flash_timer.start(FLASH_S)

func _hovered() -> bool:
	return _button.get_global_rect().has_point(_button.get_global_mouse_position())

func _open() -> void:
	_layer = CanvasLayer.new()
	_layer.layer = 100
	add_child(_layer)
	var viewport_size := _button.get_viewport_rect().size
	var width := minf(UIStyle.scaled_px(310), viewport_size.x - 60)
	if _button.has_meta(CARD_META):
		_card = panel(_button.get_meta(CARD_META).call())
	else:
		var title: String = _button.text if _button is Button else _title
		var body := str(_button.get_meta(BODY_META, _button.tooltip_text))
		_card = view(title, body, width, str(_button.get_meta(SHORTCUT_META, "")))
	_card.visible = false
	_card.resized.connect(_place, CONNECT_DEFERRED)
	_layer.add_child(_card)
	_place.call_deferred()

func _place() -> void:
	if not is_instance_valid(_card):
		return
	_card.reset_size()
	var bounds := _button.get_viewport_rect()
	var anchor := _button.get_global_rect()
	_anchor = anchor
	var at := Vector2(
		anchor.get_center().x - _card.size.x * 0.5, anchor.position.y - _card.size.y - 12)
	if at.y < 12:
		at.y = anchor.end.y + 12
	at.x = clampf(at.x, 12, maxf(12, bounds.size.x - _card.size.x - 12))
	at.y = clampf(at.y, 12, maxf(12, bounds.size.y - _card.size.y - 12))
	_card.position = at.round()
	_card.show()
	set_process(true)

func _hide() -> void:
	_timer.stop()
	if _flash_timer != null:
		_flash_timer.stop()
	if is_instance_valid(_layer):
		_layer.hide()
		_layer.queue_free()
	_layer = null
	_card = null
	set_process(false)

## The card's shell around `content`: the HUD's dark panel with an accent rim.
static func panel(content: Control) -> PanelContainer:
	var shell := PanelContainer.new()
	shell.mouse_filter = Control.MOUSE_FILTER_IGNORE
	var style := UIStyle.panel_style(
		UIStyle.BG_DARK, UIStyle.CORNER_SUB, UIStyle.HUD_ACCENT.darkened(0.45))
	style.content_margin_left = 14
	style.content_margin_right = 14
	style.content_margin_top = 12
	style.content_margin_bottom = 12
	style.shadow_color = Color(0, 0, 0, 0.25)
	style.shadow_size = 6
	shell.add_theme_stylebox_override("panel", style)
	shell.add_child(content)
	return shell

## Builds a wrapped, scale-aware card; road cost/upkeep (the last line) receives emphasis.
static func view(title: String, body: String, width: float, key: String = "") -> PanelContainer:
	var column := VBoxContainer.new()
	column.mouse_filter = Control.MOUSE_FILTER_IGNORE
	column.add_theme_constant_override("separation", 8)
	var card := panel(column)
	var style := card.get_theme_stylebox("panel") as StyleBoxFlat
	var lines := body.split("\n", false)
	# A name alone (most icons) is a compact label; `width` wraps cards that carry figures.
	if not lines.is_empty():
		column.custom_minimum_size.x = width
	var heading := HBoxContainer.new()
	heading.mouse_filter = Control.MOUSE_FILTER_IGNORE
	heading.add_theme_constant_override("separation", 10)
	column.add_child(heading)
	var title_label := _label(heading, title, 16 if lines else 14, UIStyle.TEXT_PRIMARY)
	title_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	if lines.is_empty():
		title_label.autowrap_mode = TextServer.AUTOWRAP_OFF
		style.content_margin_top = 7
		style.content_margin_bottom = 7
		style.content_margin_left = 11
		style.content_margin_right = 11
	if not key.is_empty():
		heading.add_child(_keycap(key))
	for line in lines:
		_label(column, line, 13, UIStyle.HUD_ACCENT if line.contains("upkeep") else UIStyle.TEXT_DIM)
	return card

static func _label(parent: Control, text: String, font_size: int, color: Color) -> Label:
	var label := Label.new()
	label.text = text
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	label.mouse_filter = Control.MOUSE_FILTER_IGNORE
	UIStyle.set_font_size(label, font_size)
	label.add_theme_color_override("font_color", color)
	parent.add_child(label)
	return label

# A small raised key: light rim, darker face, a thicker bottom edge like a physical key.
static func _keycap(key: String) -> PanelContainer:
	var cap := PanelContainer.new()
	cap.mouse_filter = Control.MOUSE_FILTER_IGNORE
	cap.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	var style := UIStyle.panel_style(Color(1, 1, 1, 0.10), 5, Color(1, 1, 1, 0.45))
	style.border_width_bottom = 3
	style.content_margin_left = 7
	style.content_margin_right = 7
	style.content_margin_top = 1
	style.content_margin_bottom = 1
	cap.add_theme_stylebox_override("panel", style)
	_label(cap, key, 12, UIStyle.TEXT_PRIMARY).autowrap_mode = TextServer.AUTOWRAP_OFF
	return cap
