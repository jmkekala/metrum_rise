# SPDX-License-Identifier: GPL-3.0-or-later

## The road tool's HUD (`ROAD-112` rounds D, V and V2): label chips on the geometry, each a small
## dark rounded panel with a vector icon and one text, the same pixel size at every zoom.
## Rust's frame decides which chips exist, their texts (values with their units, the hint, the
## notice's warning or refusal: `road_tool::wording`), tones, world anchors and the side each is
## pushed to, and their sizes in the font this script sends it once (`road_tool_set_chip_font`);
## the road tool's tick places them for the camera without overlap (one deterministic rule) and
## says whether anything changed (rounds R2-5, R2-2): this script fetches them (typed) and draws
## only then, so a still frame draws nothing. Each chip's panel and icon is its own canvas
## item, redrawn only when its kind, tone or size changes and otherwise only moved; the texts are
## drawn together above the panels whenever a text or a corner changed (drawn at the corner
## itself, as the font snaps its glyphs to the pixel grid). A notice chip is red for a refusal,
## amber for a warning or a stroke being sent. An
## open freeform draft adds a hint chip and, in its edit phase, the clickable Build and Cancel
## chips beside its last node: their clicks are the GUI's (they never reach the world),
## emitting `build_pressed` and `cancel_pressed`. Chips keep clear of the handles.
extends CanvasLayer

## The Build chip was clicked.
signal build_pressed
## The Cancel chip was clicked.
signal cancel_pressed

const UiStyle := preload("res://scripts/ui/ui_style.gd")

## Chip kinds as `road_tool_chips` numbers them (`kinds`).
enum Kind { PIECE_LENGTH, TOTAL_LENGTH, RADIUS, ANGLE, SLOPE, COST, BEARING, DESIGN_SPEED, NOTICE, HINT, BUILD, CANCEL, END_HEIGHT }
## Chip tones (`tones`).
enum Tone { NORMAL, WARNING, REFUSED }

## The chips' look, from the UI theme (`ui_style.gd`): panel, text and the two alert tones.
const CHIP_BG := Color(0.07, 0.07, 0.07, 0.82)
const CHIP_TEXT := Color(1.0, 1.0, 1.0, 0.96)
const CHIP_ICON := Color(0.72, 0.74, 1.0, 0.95)
const TONE_COLORS := [CHIP_TEXT, Color(1.0, 0.76, 0.18, 0.98), Color(1.0, 0.40, 0.30, 0.98)]
const FONT_SIZE := 14
const ICON_PX := 12.0
const PAD_X := 7.0
const PAD_Y := 4.0
const ICON_GAP := 5.0
const CORNER := 5
## Pixels between a chip and its anchor, and between chips.
const GAP_PX := 6.0
## The square kept clear about each handle, in pixels.
const HANDLE_CLEAR_PX := 18.0
const BUTTON_BG := Color(0.16, 0.16, 0.24, 0.92)

## The node whose road tool the chips come from: it sizes them in the font this HUD sends it
## (`road_tool_set_chip_font`) and places them in its tick; without one no chip shows.
var simulation_node: Object = null:
	set(node):
		if node != simulation_node:
			simulation_node = node
			_send_font()

## Chip draws so far: a panel's or the texts' draw callback each (tests count them).
var draws := 0

# The panels' parent (drawn first) and the texts' canvas above it.
var _panel_layer: Node2D
var _canvas: Control
var _font: Font
var _style := StyleBoxFlat.new()
# The fetched chips: kind, tone, text and size (Rust's), and their screen corners.
var _kinds := PackedInt32Array()
var _tones := PackedInt32Array()
var _texts := PackedStringArray()
var _sizes := PackedVector2Array()
var _corners := PackedVector2Array()
# Chip `i`'s panel and icon, made on first use and kept; hidden while no chip `i` shows.
var _panels: Array[Node2D] = []
## Nothing fetched since the chips were cleared: the tool fetches them on its next frame whatever
## Rust says.
var stale := true
# The chips' font size (`UiStyle.scaled_font_size` reads the settings: taken again, and the font
# sent to Rust again, whenever the chips are cleared, not every frame).
var _font_size := 0
# The clickable chips' hit areas (GUI controls, so their clicks stop at the GUI).
var _build_hit: Control
var _cancel_hit: Control
var _button_style := StyleBoxFlat.new()

func _init() -> void:
	layer = 10
	_panel_layer = Node2D.new()
	add_child(_panel_layer)
	_canvas = Control.new()
	_canvas.mouse_filter = Control.MOUSE_FILTER_IGNORE
	_canvas.set_anchors_preset(Control.PRESET_FULL_RECT)
	_canvas.draw.connect(_draw_texts)
	add_child(_canvas)
	_font = ThemeDB.fallback_font
	_style.bg_color = CHIP_BG
	_style.set_corner_radius_all(CORNER)
	_style.anti_aliasing = true
	_button_style.bg_color = BUTTON_BG
	_button_style.set_corner_radius_all(CORNER)
	_button_style.set_border_width_all(1)
	_button_style.border_color = CHIP_ICON
	_button_style.anti_aliasing = true
	_build_hit = _hit_area(build_pressed)
	_cancel_hit = _hit_area(cancel_pressed)

# An invisible GUI control that stops a left press on it and emits `pressed`.
func _hit_area(pressed: Signal) -> Control:
	var hit := Control.new()
	hit.mouse_filter = Control.MOUSE_FILTER_STOP
	hit.visible = false
	hit.gui_input.connect(func(event: InputEvent) -> void:
		if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
			hit.accept_event()
			if event.pressed:
				pressed.emit())
	add_child(hit)
	return hit

# Sends the chips' font to Rust, which sizes the texts in it: the advance of every character the
# wording writes, the panel beside the text and its height.
func _send_font() -> void:
	_font_size = UiStyle.scaled_font_size(FONT_SIZE)
	if simulation_node == null:
		return
	var characters: String = simulation_node.road_tool_chip_characters()
	var advances := PackedFloat32Array()
	advances.resize(characters.length())
	for index in characters.length():
		advances[index] = _font.get_char_size(characters.unicode_at(index), _font_size).x
	simulation_node.road_tool_set_chip_font(characters, advances, PAD_X * 2.0 + ICON_PX + ICON_GAP, maxf(_font.get_height(_font_size), ICON_PX) + PAD_Y * 2.0, HANDLE_CLEAR_PX, GAP_PX)

## Removes every chip; nothing to do when none was fetched since the last clear.
func clear() -> void:
	if stale:
		return
	stale = true
	_kinds = PackedInt32Array()
	_tones = PackedInt32Array()
	_texts = PackedStringArray()
	_sizes = PackedVector2Array()
	_corners = PackedVector2Array()
	_font_size = 0
	_build_hit.visible = false
	_cancel_hit.visible = false
	for panel in _panels:
		panel.visible = false
	_canvas.queue_redraw()

## The chips' texts in frame order, joined by two spaces (tests read the HUD this way).
func text() -> String:
	return "  ".join(_texts)

## On the tick's chips or corners flag (or while `stale`): fetches the chips when they changed
## (`all`, or nothing fetched since the last clear) and their corners, redraws the changed panels,
## moves the panels and redraws the texts.
func show_chips(all: bool) -> void:
	if simulation_node == null:
		return
	if _font_size == 0:
		_send_font()
	if all or stale:
		stale = false
		var redraw: PackedInt32Array = simulation_node.road_tool_chips_fetch()
		_kinds = simulation_node.road_tool_chip_kinds()
		_tones = simulation_node.road_tool_chip_tones()
		_texts = simulation_node.road_tool_chip_texts()
		_sizes = simulation_node.road_tool_chip_sizes()
		for index in redraw:
			_panel(index).queue_redraw()
	_corners = simulation_node.road_tool_chip_corners()
	_place_panels()
	if _kinds.has(Kind.BUILD) or _kinds.has(Kind.CANCEL) or _build_hit.visible or _cancel_hit.visible:
		_place_hit(_build_hit, Kind.BUILD)
		_place_hit(_cancel_hit, Kind.CANCEL)
	_canvas.queue_redraw()

# Chip `index`'s panel, made (and drawn on its first frame) when first needed.
func _panel(index: int) -> Node2D:
	while _panels.size() <= index:
		var panel := Node2D.new()
		panel.draw.connect(_draw_panel.bind(panel, _panels.size()))
		_panel_layer.add_child(panel)
		_panels.append(panel)
		panel.queue_redraw()
	return _panels[index]

# Moves each shown chip's panel to its corner and hides the rest (a chip behind the camera, or
# none). `O(panels)`, on frames whose chips or corners changed only.
func _place_panels() -> void:
	var count := mini(_kinds.size(), _corners.size())
	for index in count:
		var panel := _panel(index)
		panel.visible = not is_nan(_corners[index].x)
		if panel.visible:
			panel.position = _corners[index]
	for index in range(count, _panels.size()):
		_panels[index].visible = false

## The screen rectangle of the shown chip of `kind`, or an empty one.
func chip_rect(kind: int) -> Rect2:
	for index in mini(_kinds.size(), _corners.size()):
		if _kinds[index] == kind and not is_nan(_corners[index].x):
			return Rect2(_corners[index], _sizes[index])
	return Rect2()

# Puts a clickable chip's hit area over its chip, or hides it.
func _place_hit(hit: Control, kind: int) -> void:
	var rect := chip_rect(kind)
	hit.visible = rect.has_area()
	if hit.visible:
		hit.position = rect.position
		hit.size = rect.size

# Chip `index`'s panel and icon, in the panel's own coordinates (its corner at the origin).
func _draw_panel(panel: Node2D, index: int) -> void:
	if index >= mini(_kinds.size(), _sizes.size()):
		return
	draws += 1
	var size := _sizes[index]
	var button := _kinds[index] == Kind.BUILD or _kinds[index] == Kind.CANCEL
	panel.draw_style_box(_button_style if button else _style, Rect2(Vector2.ZERO, size))
	var tone := _tones[index]
	var icon := Rect2(Vector2(PAD_X, (size.y - ICON_PX) * 0.5), Vector2(ICON_PX, ICON_PX))
	_draw_icon(panel, _kinds[index], tone, icon, CHIP_ICON if tone == Tone.NORMAL else TONE_COLORS[tone])

# Every shown chip's text beside its icon.
func _draw_texts() -> void:
	var count := mini(_kinds.size(), _corners.size())
	if count == 0:
		return
	draws += 1
	var font_size := _font_size if _font_size > 0 else UiStyle.scaled_font_size(FONT_SIZE)
	var ascent := _font.get_ascent(font_size)
	var descent := _font.get_descent(font_size)
	for index in count:
		if is_nan(_corners[index].x):
			continue
		# The panel's rectangles as `_draw_panel` lays them out, in screen coordinates.
		var rect := Rect2(_corners[index], _sizes[index])
		var icon := Rect2(rect.position + Vector2(PAD_X, (rect.size.y - ICON_PX) * 0.5), Vector2(ICON_PX, ICON_PX))
		var baseline := rect.position.y + (rect.size.y + ascent - descent) * 0.5
		_canvas.draw_string(_font, Vector2(icon.end.x + ICON_GAP, baseline), _texts[index], HORIZONTAL_ALIGNMENT_LEFT, -1, font_size, TONE_COLORS[_tones[index]])

# Small vector icons, drawn in `box`: a double arrow (a piece's length; with end bars, the total),
# an arc and its radius, an angle, a slope, a coin, a compass needle, a speed gauge, a warning
# triangle or a refusal's crossed circle, an end's height (a vertical double arrow over the ground).
func _draw_icon(item: CanvasItem, kind: int, tone: int, box: Rect2, color: Color) -> void:
	var c := box.get_center()
	var h := box.size.x * 0.5
	match kind:
		Kind.PIECE_LENGTH, Kind.TOTAL_LENGTH:
			var a := c - Vector2(h, 0.0)
			var b := c + Vector2(h, 0.0)
			item.draw_line(a, b, color, 1.5, true)
			item.draw_polyline(PackedVector2Array([a + Vector2(3.5, -3.0), a, a + Vector2(3.5, 3.0)]), color, 1.5, true)
			item.draw_polyline(PackedVector2Array([b + Vector2(-3.5, -3.0), b, b + Vector2(-3.5, 3.0)]), color, 1.5, true)
			if kind == Kind.TOTAL_LENGTH:
				item.draw_line(a + Vector2(0.0, -h), a + Vector2(0.0, h), color, 1.5, true)
				item.draw_line(b + Vector2(0.0, -h), b + Vector2(0.0, h), color, 1.5, true)
		Kind.RADIUS:
			var centre := c + Vector2(-h, h)
			item.draw_arc(centre, h * 2.0, -PI * 0.5, 0.0, 10, color, 1.5, true)
			item.draw_line(centre, centre + Vector2(h, -h) * 1.41 * 0.7, color, 1.5, true)
		Kind.ANGLE:
			var corner := c + Vector2(-h, h)
			item.draw_polyline(PackedVector2Array([c + Vector2(h, -h), corner, c + Vector2(h, h)]), color, 1.5, true)
			item.draw_arc(corner, h, -PI * 0.25, 0.0, 6, color, 1.2, true)
		Kind.SLOPE:
			item.draw_colored_polygon(PackedVector2Array([c + Vector2(-h, h), c + Vector2(h, h), c + Vector2(h, -h * 0.4)]), color)
		Kind.COST:
			item.draw_arc(c, h, 0.0, TAU, 16, color, 1.5, true)
			item.draw_line(c + Vector2(0.0, -h * 0.55), c + Vector2(0.0, h * 0.55), color, 1.5, true)
		Kind.BEARING:
			item.draw_arc(c, h, 0.0, TAU, 16, color, 1.2, true)
			item.draw_colored_polygon(PackedVector2Array([c + Vector2(0.0, -h * 0.85), c + Vector2(h * 0.35, 0.0), c + Vector2(-h * 0.35, 0.0)]), color)
		Kind.DESIGN_SPEED:
			item.draw_arc(c + Vector2(0.0, h * 0.4), h, PI, TAU, 12, color, 1.5, true)
			item.draw_line(c + Vector2(0.0, h * 0.4), c + Vector2(h * 0.55, -h * 0.35), color, 1.5, true)
		Kind.END_HEIGHT:
			# A vertical double arrow over a ground line: an end's height above its ground.
			var top := c + Vector2(0.0, -h)
			var foot := c + Vector2(0.0, h * 0.6)
			item.draw_line(top, foot, color, 1.5, true)
			item.draw_polyline(PackedVector2Array([top + Vector2(-3.0, 3.0), top, top + Vector2(3.0, 3.0)]), color, 1.5, true)
			item.draw_polyline(PackedVector2Array([foot + Vector2(-3.0, -3.0), foot, foot + Vector2(3.0, -3.0)]), color, 1.5, true)
			item.draw_line(c + Vector2(-h, h), c + Vector2(h, h), color, 1.5, true)
		Kind.HINT:
			item.draw_arc(c, h, 0.0, TAU, 16, color, 1.5, true)
			item.draw_line(c + Vector2(0.0, -h * 0.1), c + Vector2(0.0, h * 0.55), color, 1.5, true)
			item.draw_circle(c + Vector2(0.0, -h * 0.45), 1.2, color)
		Kind.BUILD:
			item.draw_polyline(PackedVector2Array([c + Vector2(-h, 0.0), c + Vector2(-h * 0.3, h * 0.7), c + Vector2(h, -h * 0.7)]), Color(0.35, 0.95, 0.55), 2.0, true)
		Kind.CANCEL:
			item.draw_line(c + Vector2(-h, -h) * 0.75, c + Vector2(h, h) * 0.75, TONE_COLORS[Tone.REFUSED], 2.0, true)
			item.draw_line(c + Vector2(h, -h) * 0.75, c + Vector2(-h, h) * 0.75, TONE_COLORS[Tone.REFUSED], 2.0, true)
		Kind.NOTICE:
			if tone == Tone.REFUSED:
				item.draw_arc(c, h, 0.0, TAU, 16, color, 1.5, true)
				item.draw_line(c + Vector2(-h, -h) * 0.45, c + Vector2(h, h) * 0.45, color, 1.5, true)
				item.draw_line(c + Vector2(h, -h) * 0.45, c + Vector2(-h, h) * 0.45, color, 1.5, true)
			else:
				item.draw_polyline(PackedVector2Array([c + Vector2(0.0, -h), c + Vector2(h, h), c + Vector2(-h, h), c + Vector2(0.0, -h)]), color, 1.5, true)
				item.draw_line(c + Vector2(0.0, -h * 0.3), c + Vector2(0.0, h * 0.35), color, 1.5, true)
