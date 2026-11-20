# SPDX-License-Identifier: GPL-3.0-or-later

## One road coming into the Junction window's junction (`ROAD-95`, round R5c2), drawn as a card
## well in the inspector style: where it comes from ("From west") over its type in dim text, with
## what its end has now as a pill; Left and Right as toggles drawn with the road marking's turn
## arrow; the pocket's length from the junction as a − / + stepper whose number can be typed;
## Rust's verdict beside Apply, shown only while the controls ask for a change. It holds no rule:
## the host fills it from Rust (a `get_junction_pockets` entry: the roles the road can carry, the
## length range, the pocket there) and shows Rust's verdict; the row reports the player's changes
## through `changed` and `apply_pressed`, and is outlined while its road is highlighted (the host
## draws the strip on the road).
extends PanelContainer

## The player changed a control: the pocket the controls now ask for (both off removes it).
signal changed(left: bool, right: bool, length_m: float)
## The player pressed Apply.
signal apply_pressed

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const Card = preload("res://scripts/ui/inspector_card.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const RoadTypeMenu = preload("res://scripts/ui/road_type_menu.gd")
const VERDICT_READY := Card.GOOD
const VERDICT_REFUSED := UIStyle.SIGN_YELLOW
## The highlighted row's border: the road strip's cyan.
const HIGHLIGHT := Color(0.2, 0.9, 1.0)
# Compass names by 45 degree sector from north (-Z), clockwise (east is +X).
const COMPASS: Array[String] = ["north", "north-east", "east", "south-east", "south", "south-west",
	"west", "north-west"]

# A Left or Right toggle: a quiet well with the road marking's turn arrow before its name, lit in
# the accent while on.
class TurnToggle extends Button:
	var turn := -1.0

	func _init(right: bool) -> void:
		turn = 1.0 if right else -1.0
		text = "Right" if right else "Left"
		toggle_mode = true
		focus_mode = Control.FOCUS_NONE
		mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
		alignment = HORIZONTAL_ALIGNMENT_LEFT
		UIStyle.set_font_size(self, 13)
		for state in ["normal", "hover", "pressed", "hover_pressed", "disabled"]:
			var style := StyleBoxFlat.new()
			style.set_corner_radius_all(8)
			style.set_border_width_all(1)
			style.content_margin_left = 34
			style.content_margin_right = 12
			style.content_margin_top = 6
			style.content_margin_bottom = 7
			style.bg_color = Card.WELL_BG
			style.border_color = Color(1, 1, 1, 0.0)
			if state == "hover":
				style.bg_color = Color(1, 1, 1, 0.09)
			elif state.ends_with("pressed"):
				style.bg_color = Color(UIStyle.HUD_ACCENT, 0.22 if state == "hover_pressed" else 0.16)
				style.border_color = Color(UIStyle.HUD_ACCENT, 0.7)
			add_theme_stylebox_override(state, style)
		add_theme_stylebox_override("focus", StyleBoxEmpty.new())
		add_theme_color_override("font_color", UIStyle.TEXT_DIM)
		add_theme_color_override("font_hover_color", UIStyle.TEXT_PRIMARY)
		add_theme_color_override("font_pressed_color", UIStyle.TEXT_PRIMARY)
		add_theme_color_override("font_hover_pressed_color", UIStyle.TEXT_PRIMARY)
		add_theme_color_override("font_disabled_color", Color(UIStyle.TEXT_DIM, 0.35))
		toggled.connect(func(_on: bool): queue_redraw())

	# The arrow: a stem up from the bottom that bends a quarter turn to the side, with its head.
	func _draw() -> void:
		var colour := UIStyle.HUD_ACCENT if button_pressed else UIStyle.TEXT_DIM
		if disabled:
			colour = Color(UIStyle.TEXT_DIM, 0.3)
		var r := size.y * 0.27
		var stem_x := 19.0 - turn * r * 0.45
		var bottom := size.y * 0.5 + r
		var bend := size.y * 0.5
		var points := PackedVector2Array([Vector2(stem_x, bottom), Vector2(stem_x, bend)])
		var q := r * 0.7
		for i in range(1, 7):
			var t := float(i) / 6.0 * PI * 0.5
			points.append(Vector2(stem_x + turn * (q - q * cos(t)), bend - q * sin(t)))
		var tip := Vector2(stem_x + turn * (q + r * 0.55), bend - q)
		points.append(tip)
		var width := maxf(1.6, size.y * 0.06)
		draw_polyline(points, colour, width, true)
		var head := r * 0.42
		draw_polyline(PackedVector2Array([tip + Vector2(-turn * head, -head), tip, tip + Vector2(-turn * head, head)]),
			colour, width, true)

# The pocket's length: − and + beside the number, which can be typed (Enter or a click elsewhere
# takes it); the wheel over it steps. Its value snaps to `step` within the range, as a spin box's.
class LengthStepper extends HBoxContainer:
	signal value_changed(value: float)

	var min_value := 20.0
	var max_value := 300.0
	var step := 10.0
	var value := 60.0:
		set(v):
			var snapped_value := _snapped(v)
			var changed := not is_equal_approx(snapped_value, value)
			value = snapped_value
			_show()
			if changed:
				value_changed.emit(value)

	var _field: LineEdit
	var _minus: Button
	var _plus: Button

	func _init() -> void:
		add_theme_constant_override("separation", 0)
		tooltip_text = "Pocket length from the junction"
		_minus = _step_button("−", -1.0)
		add_child(_minus)
		_field = LineEdit.new()
		_field.alignment = HORIZONTAL_ALIGNMENT_CENTER
		_field.custom_minimum_size.x = UIStyle.scaled_px(58.0)
		_field.select_all_on_focus = true
		_field.context_menu_enabled = false
		UIStyle.set_font_size(_field, 13)
		var well := StyleBoxFlat.new()
		well.bg_color = Color(0, 0, 0, 0.25)
		well.content_margin_top = 4
		well.content_margin_bottom = 4
		_field.add_theme_stylebox_override("normal", well)
		var typing := well.duplicate() as StyleBoxFlat
		typing.border_color = Color(UIStyle.HUD_ACCENT, 0.7)
		typing.set_border_width_all(1)
		_field.add_theme_stylebox_override("focus", typing)
		_field.text_submitted.connect(func(_text: String): _take_text())
		_field.focus_exited.connect(_take_text)
		add_child(_field)
		_plus = _step_button("+", 1.0)
		add_child(_plus)
		_show()

	## Sets the length without emitting `value_changed`.
	func set_value_no_signal(v: float) -> void:
		set_block_signals(true)
		value = v
		set_block_signals(false)

	## The number's text field (the window keeps the keyboard for it while it is typed in).
	func get_line_edit() -> LineEdit:
		return _field

	func _gui_input(event: InputEvent) -> void:
		var wheel := event as InputEventMouseButton
		if wheel == null or not wheel.pressed:
			return
		if wheel.button_index == MOUSE_BUTTON_WHEEL_UP:
			value += step
			accept_event()
		elif wheel.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			value -= step
			accept_event()

	func _step_button(glyph: String, direction: float) -> Button:
		var button := Button.new()
		button.text = glyph
		button.focus_mode = Control.FOCUS_NONE
		button.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
		button.custom_minimum_size.x = UIStyle.scaled_px(28.0)
		UIStyle.set_font_size(button, 14)
		for state in ["normal", "hover", "pressed", "disabled"]:
			var style := StyleBoxFlat.new()
			style.bg_color = Card.WELL_BG if state == "normal" or state == "disabled" \
				else Color(1, 1, 1, 0.10 if state == "hover" else 0.16)
			if direction < 0.0:
				style.corner_radius_top_left = 8
				style.corner_radius_bottom_left = 8
			else:
				style.corner_radius_top_right = 8
				style.corner_radius_bottom_right = 8
			button.add_theme_stylebox_override(state, style)
		button.add_theme_stylebox_override("focus", StyleBoxEmpty.new())
		button.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
		button.add_theme_color_override("font_hover_color", UIStyle.TEXT_PRIMARY)
		button.add_theme_color_override("font_pressed_color", UIStyle.HUD_ACCENT)
		button.add_theme_color_override("font_disabled_color", Color(UIStyle.TEXT_DIM, 0.3))
		button.pressed.connect(func(): value += step * direction)
		return button

	func _take_text() -> void:
		var digits := _field.text.strip_edges().trim_suffix("m").strip_edges()
		if digits.is_valid_float():
			value = float(digits)
		_show()

	func _show() -> void:
		if _field == null:
			return
		_field.text = "%d m" % roundi(value)
		_minus.disabled = value <= min_value
		_plus.disabled = value >= max_value

	func _snapped(v: float) -> float:
		var stepped := min_value + snappedf(v - min_value, step) if step > 0.0 else v
		return clampf(stepped, min_value, max_value)

## The arm this row shows (its `get_junction_pockets` entry).
var info: Dictionary = {}

var _name_label: Label
var _type_label: Label
var _state_pill: PanelContainer
var _state_label: Label
var _left_check: Button
var _right_check: Button
var _length_spin: LengthStepper
var _apply_row: HBoxContainer
var _verdict_label: Label
var _apply_button: Button
var _style: StyleBoxFlat

func _init() -> void:
	_style = StyleBoxFlat.new()
	_style.bg_color = Card.WELL_BG
	_style.set_corner_radius_all(10)
	_style.set_border_width_all(1)
	_style.content_margin_left = 12
	_style.content_margin_right = 12
	_style.content_margin_top = 9
	_style.content_margin_bottom = 10
	add_theme_stylebox_override("panel", _style)
	set_highlighted(false)
	var rows := VBoxContainer.new()
	rows.add_theme_constant_override("separation", 8)
	add_child(rows)

	var name_row := HBoxContainer.new()
	name_row.add_theme_constant_override("separation", 8)
	rows.add_child(name_row)
	var names := VBoxContainer.new()
	names.add_theme_constant_override("separation", 0)
	names.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	name_row.add_child(names)
	_name_label = Card._label("", 14, UIStyle.TEXT_PRIMARY)
	_name_label.add_theme_font_override("font", Card.bold_font())
	names.add_child(_name_label)
	_type_label = Card._label("", 12, UIStyle.TEXT_DIM)
	_type_label.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	_type_label.clip_text = true
	names.add_child(_type_label)
	_state_pill = Card.pill("", UIStyle.TEXT_DIM)
	_state_label = _state_pill.get_child(0)
	name_row.add_child(_state_pill)

	var controls := HBoxContainer.new()
	controls.add_theme_constant_override("separation", 6)
	rows.add_child(controls)
	_left_check = _turn(false)
	controls.add_child(_left_check)
	_right_check = _turn(true)
	controls.add_child(_right_check)
	var gap := Control.new()
	gap.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	controls.add_child(gap)
	_length_spin = LengthStepper.new()
	_length_spin.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_length_spin.value_changed.connect(func(_value: float) -> void: _emit_changed())
	controls.add_child(_length_spin)

	_apply_row = HBoxContainer.new()
	_apply_row.add_theme_constant_override("separation", 10)
	rows.add_child(_apply_row)
	_verdict_label = Card._label("", 12, VERDICT_READY)
	_verdict_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_verdict_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_verdict_label.size_flags_vertical = Control.SIZE_SHRINK_CENTER
	_apply_row.add_child(_verdict_label)
	_apply_button = Button.new()
	_apply_button.text = "Apply"
	_apply_button.disabled = true
	# The same accent Apply as the Road Properties window.
	UIStyle.style_primary_button(_apply_button)
	UIStyle.set_font_size(_apply_button, 13)
	_apply_button.mouse_default_cursor_shape = Control.CURSOR_POINTING_HAND
	_apply_button.pressed.connect(func() -> void: apply_pressed.emit())
	_apply_row.add_child(_apply_button)
	_apply_row.hide()
	# The row stays hovered over its own controls: a control that stops the mouse ends Godot's
	# hover chain at itself, so the row read as left over each toggle, the length, the spacer
	# and Apply, and the window's highlight flickered. Passing still gives them their clicks and
	# the wheel.
	pass_mouse_to_row(self)

## Shows arm `arm` (a `get_junction_pockets` entry) without a request: the name, what the end
## has, the toggles and length from its pocket (or none and the default length), each role
## enabled when the road can carry it or the pocket has it (so it can be removed), a refused
## role's reason as its tooltip.
func show_arm(arm: Dictionary) -> void:
	info = arm
	_name_label.text = arm_name(arm)
	_type_label.text = arm_road(arm)
	_type_label.tooltip_text = _type_label.text
	var has_pocket := bool(arm.get("has_pocket", false))
	var left := has_pocket and bool(arm.get("left", false))
	var right := has_pocket and bool(arm.get("right", false))
	var length := float(arm.get("length_m", 0.0)) if has_pocket else float(arm.get("default_length_m", 60.0))
	_show_state(short_state(left, right, length) if has_pocket else "No pocket", has_pocket)
	_set_role(_left_check, left, bool(arm.get("left_offered", false)), String(arm.get("left_refusal", "")))
	_set_role(_right_check, right, bool(arm.get("right_offered", false)), String(arm.get("right_refusal", "")))
	_length_spin.min_value = float(arm.get("min_length_m", 20.0))
	_length_spin.max_value = float(arm.get("max_length_m", 300.0))
	_length_spin.step = float(arm.get("step_m", 10.0))
	_length_spin.set_value_no_signal(length)
	show_verdict({}, "")

## Sets the controls to `request` ([left, right, length_m]) without asking: a kept unapplied
## request after the host read the arms again.
func set_request(request: Array) -> void:
	_left_check.set_pressed_no_signal(bool(request[0]))
	_right_check.set_pressed_no_signal(bool(request[1]))
	_left_check.queue_redraw()
	_right_check.queue_redraw()
	_length_spin.set_value_no_signal(float(request[2]))

## The request the controls show: [left, right, length_m].
func request() -> Array:
	return [_left_check.button_pressed, _right_check.button_pressed, _length_spin.value]

## Whether the controls ask for something other than the arm's pocket (an unapplied request).
func has_request() -> bool:
	var has_pocket := bool(info.get("has_pocket", false))
	var current := [has_pocket and bool(info.get("left", false)), has_pocket and bool(info.get("right", false)),
		float(info.get("length_m", 0.0)) if has_pocket else float(info.get("default_length_m", 60.0))]
	var asked := request()
	if not asked[0] and not asked[1] and not current[0] and not current[1]:
		return false
	return asked != current

## Shows Rust's `verdict` on the request: its text (`accepted_text` when accepted, Rust's refusal
## worded by `road_reason_text.gd` otherwise) and Apply, enabled when accepted and labelled with
## `apply_text`. An empty verdict hides the line and Apply.
func show_verdict(verdict: Dictionary, accepted_text: String, apply_text: String = "Apply") -> void:
	var ready := bool(verdict.get("is_valid", false))
	_apply_button.disabled = not ready
	_apply_button.text = apply_text if ready else "Apply"
	_apply_row.visible = not verdict.is_empty()
	if verdict.is_empty():
		_verdict_label.text = ""
		return
	_verdict_label.add_theme_color_override("font_color", VERDICT_READY if ready else VERDICT_REFUSED)
	_verdict_label.text = accepted_text if ready else RoadReasonText.text(verdict, "This road cannot take this pocket")

## Makes every control under `node` (internal children too, such as the length's text field)
## pass the mouse on instead of stopping it, so the row's `mouse_entered` / `mouse_exited` follow
## the cursor over the whole row.
static func pass_mouse_to_row(node: Node) -> void:
	for child in node.get_children(true):
		if child is Control and child.mouse_filter == Control.MOUSE_FILTER_STOP:
			child.mouse_filter = Control.MOUSE_FILTER_PASS
		pass_mouse_to_row(child)

## Outlines the row in the strip's cyan while its road is highlighted.
func set_highlighted(on: bool) -> void:
	_style.border_color = HIGHLIGHT if on else Color(1.0, 1.0, 1.0, 0.0)
	_style.bg_color = Color(HIGHLIGHT, 0.06) if on else Card.WELL_BG

## "From west": the compass side the road comes in from.
static func arm_name(arm: Dictionary) -> String:
	return "From %s" % compass(float(arm.get("bearing_deg", 0.0)))

## The road's type ("2-Lane Road"), or its lanes and median when no road type matches
## ("2 + 2 lanes, Grass 5.0 m").
static func arm_road(arm: Dictionary) -> String:
	var road := String(arm.get("road_type", ""))
	return road if not road.is_empty() else RoadTypeMenu.describe(arm, false)

## The compass sector of `bearing_deg` (clockwise from north): eight 45 degree sectors.
static func compass(bearing_deg: float) -> String:
	return COMPASS[int(roundf(fposmod(bearing_deg, 360.0) / 45.0)) % COMPASS.size()]

## "Left pocket, 60 m from the junction", "Left and right pockets, 80 m from the junction", or
## "No turn pocket" with both off.
static func describe(left: bool, right: bool, length_m: float) -> String:
	if not left and not right:
		return "No turn pocket"
	return "%s, %d m from the junction" % [_roles(left, right), roundi(length_m)]

## The pill's state: "Right, 60 m", "Left + right, 80 m".
static func short_state(left: bool, right: bool, length_m: float) -> String:
	var roles := "Left + right" if left and right else ("Left" if left else "Right")
	return "%s, %d m" % [roles, roundi(length_m)]

static func _roles(left: bool, right: bool) -> String:
	return "Left and right pockets" if left and right else ("Left pocket" if left else "Right pocket")

# The pill: the accent while the end has a pocket, dim without.
func _show_state(text: String, has_pocket: bool) -> void:
	var colour := UIStyle.HUD_ACCENT if has_pocket else UIStyle.TEXT_DIM
	var style := (_state_pill.get_theme_stylebox("panel") as StyleBoxFlat).duplicate() as StyleBoxFlat
	style.bg_color = Color(colour, 0.16)
	style.border_color = Color(colour, 0.45)
	_state_pill.add_theme_stylebox_override("panel", style)
	_state_label.add_theme_color_override("font_color", colour.lerp(Color.WHITE, 0.25))
	_state_label.text = text

func _set_role(toggle: Button, pressed: bool, offered: bool, refusal: String) -> void:
	toggle.set_pressed_no_signal(pressed)
	toggle.disabled = not offered and not pressed
	toggle.tooltip_text = "" if offered else RoadReasonText.text({"invalid_reason": refusal}, "")
	toggle.queue_redraw()

func _emit_changed() -> void:
	changed.emit(_left_check.button_pressed, _right_check.button_pressed, _length_spin.value)

func _turn(right: bool) -> Button:
	var toggle := TurnToggle.new(right)
	toggle.toggled.connect(func(_on: bool) -> void: _emit_changed())
	return toggle
