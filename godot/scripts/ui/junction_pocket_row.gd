# SPDX-License-Identifier: GPL-2.0-only

## One row of the Junction window's "Turn pockets" section (`ROAD-95`, round R5c2): the widgets
## for one incoming car road. A name line says where the road comes from and what it is ("From
## west · 2-Lane Road") with what its end has now, Left and Right check boxes pick the pocket
## lanes, a Length stepper sets the body from the junction, and the verdict line sits beside
## Apply. It holds no rule: the host fills it from Rust (a `get_junction_pockets` entry: the roles
## the road can carry, the length range, the pocket there) and shows Rust's verdict; the row
## reports the player's changes through `changed` and `apply_pressed`, and is highlighted while
## its road is (the host draws the strip on the road).
extends PanelContainer

## The player changed a control: the pocket the controls now ask for (both off removes it).
signal changed(left: bool, right: bool, length_m: float)
## The player pressed Apply.
signal apply_pressed

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const RoadTypeMenu = preload("res://scripts/ui/road_type_menu.gd")
const VERDICT_READY := Color(0.55, 0.95, 0.55)
const VERDICT_REFUSED := Color.YELLOW
## The highlighted row's border: the road strip's cyan.
const HIGHLIGHT := Color(0.2, 0.9, 1.0)
# Compass names by 45 degree sector from north (-Z), clockwise (east is +X).
const COMPASS: Array[String] = ["north", "north-east", "east", "south-east", "south", "south-west",
	"west", "north-west"]

## The arm this row shows (its `get_junction_pockets` entry).
var info: Dictionary = {}

var _name_label: Label
var _state_label: Label
var _left_check: CheckBox
var _right_check: CheckBox
var _length_spin: SpinBox
var _verdict_label: Label
var _apply_button: Button
var _style: StyleBoxFlat

func _init(font_size: int = 13) -> void:
	_style = StyleBoxFlat.new()
	_style.bg_color = Color(1.0, 1.0, 1.0, 0.04)
	_style.set_corner_radius_all(UIStyle.CORNER_WINDOW)
	_style.set_border_width_all(1)
	_style.set_content_margin_all(UIStyle.PAD_INNER)
	add_theme_stylebox_override("panel", _style)
	set_highlighted(false)
	var rows := VBoxContainer.new()
	rows.add_theme_constant_override("separation", 4)
	add_child(rows)

	var name_row := HBoxContainer.new()
	_name_label = _label(font_size)
	_name_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	_name_label.text_overrun_behavior = TextServer.OVERRUN_TRIM_ELLIPSIS
	_name_label.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	name_row.add_child(_name_label)
	_state_label = _label(font_size - 1)
	_state_label.add_theme_color_override("font_color", UIStyle.TEXT_DIM)
	name_row.add_child(_state_label)
	rows.add_child(name_row)

	var controls := HBoxContainer.new()
	_left_check = _check("Left", font_size)
	controls.add_child(_left_check)
	_right_check = _check("Right", font_size)
	controls.add_child(_right_check)
	var gap := Control.new()
	gap.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	controls.add_child(gap)
	_length_spin = SpinBox.new()
	_length_spin.suffix = "m"
	_length_spin.rounded = true
	_length_spin.tooltip_text = "Pocket length"
	# The arrows step it and hand the keyboard straight back to the game (Ctrl+Z); a click on the
	# number keeps it for typing until Enter or a click elsewhere (`keep_keyboard_with_game`). Its
	# text stays editable: Godot's spin box ignores its arrows when its text is not.
	UIStyle.set_font_size(_length_spin.get_line_edit(), font_size)
	_length_spin.value_changed.connect(func(_value: float) -> void: _emit_changed())
	controls.add_child(_length_spin)
	var from_label := _label(font_size)
	from_label.text = "from the junction"
	controls.add_child(from_label)
	rows.add_child(controls)

	var apply_row := HBoxContainer.new()
	_verdict_label = _label(font_size - 1)
	_verdict_label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	_verdict_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	apply_row.add_child(_verdict_label)
	_apply_button = Button.new()
	_apply_button.text = "Apply"
	_apply_button.disabled = true
	UIStyle.set_font_size(_apply_button, font_size)
	_apply_button.pressed.connect(func() -> void: apply_pressed.emit())
	apply_row.add_child(_apply_button)
	rows.add_child(apply_row)
	# The row stays hovered over its own controls: a control that stops the mouse ends Godot's
	# hover chain at itself, so the row read as left over each check box, the length box, the
	# spacer and Apply, and the window's highlight flickered. Passing still gives them their
	# clicks and the wheel.
	pass_mouse_to_row(self)

## Shows arm `arm` (a `get_junction_pockets` entry) without a request: the name, what the end
## has, the check boxes and length from its pocket (or none and the default length), each role
## enabled when the road can carry it or the pocket has it (so it can be removed), a refused
## role's reason as its tooltip.
func show_arm(arm: Dictionary) -> void:
	info = arm
	_name_label.text = arm_name(arm)
	_name_label.tooltip_text = _name_label.text
	var has_pocket := bool(arm.get("has_pocket", false))
	var left := has_pocket and bool(arm.get("left", false))
	var right := has_pocket and bool(arm.get("right", false))
	var length := float(arm.get("length_m", 0.0)) if has_pocket else float(arm.get("default_length_m", 60.0))
	_state_label.text = short_state(left, right, length) if has_pocket else "No pocket"
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
## `apply_text`. An empty verdict clears the line and disables Apply.
func show_verdict(verdict: Dictionary, accepted_text: String, apply_text: String = "Apply") -> void:
	var ready := bool(verdict.get("is_valid", false))
	_apply_button.disabled = not ready
	_apply_button.text = apply_text if ready else "Apply"
	if verdict.is_empty():
		_verdict_label.text = ""
		return
	_verdict_label.add_theme_color_override("font_color", VERDICT_READY if ready else VERDICT_REFUSED)
	_verdict_label.text = accepted_text if ready else RoadReasonText.text(verdict, "This road cannot take this pocket")

## Makes every control under `node` (internal children too, such as the spin box's text field)
## pass the mouse on instead of stopping it, so the row's `mouse_entered` / `mouse_exited` follow
## the cursor over the whole row.
static func pass_mouse_to_row(node: Node) -> void:
	for child in node.get_children(true):
		if child is Control and child.mouse_filter == Control.MOUSE_FILTER_STOP:
			child.mouse_filter = Control.MOUSE_FILTER_PASS
		pass_mouse_to_row(child)

## Draws the row's border in the strip's cyan while its road is highlighted.
func set_highlighted(on: bool) -> void:
	_style.border_color = HIGHLIGHT if on else Color(1.0, 1.0, 1.0, 0.12)

## "From west · 2-Lane Road", or the road's lanes and median when no road type matches
## ("From north-east · 2 + 2 lanes, Grass 5.0 m").
static func arm_name(arm: Dictionary) -> String:
	var road := String(arm.get("road_type", ""))
	if road.is_empty():
		road = RoadTypeMenu.describe(arm, false)
	return "From %s · %s" % [compass(float(arm.get("bearing_deg", 0.0))), road]

## The compass sector of `bearing_deg` (clockwise from north): eight 45 degree sectors.
static func compass(bearing_deg: float) -> String:
	return COMPASS[int(roundf(fposmod(bearing_deg, 360.0) / 45.0)) % COMPASS.size()]

## "Left pocket, 60 m from the junction", "Left and right pockets, 80 m from the junction", or
## "No turn pocket" with both off.
static func describe(left: bool, right: bool, length_m: float) -> String:
	if not left and not right:
		return "No turn pocket"
	return "%s, %d m from the junction" % [_roles(left, right), roundi(length_m)]

## The name line's state: "Right, 60 m", "Left + right, 80 m".
static func short_state(left: bool, right: bool, length_m: float) -> String:
	var roles := "Left + right" if left and right else ("Left" if left else "Right")
	return "%s, %d m" % [roles, roundi(length_m)]

static func _roles(left: bool, right: bool) -> String:
	return "Left and right pockets" if left and right else ("Left pocket" if left else "Right pocket")

func _set_role(check: CheckBox, pressed: bool, offered: bool, refusal: String) -> void:
	check.set_pressed_no_signal(pressed)
	check.disabled = not offered and not pressed
	check.tooltip_text = "" if offered else RoadReasonText.text({"invalid_reason": refusal}, "")

func _emit_changed() -> void:
	changed.emit(_left_check.button_pressed, _right_check.button_pressed, _length_spin.value)

func _label(font_size: int) -> Label:
	var label := Label.new()
	UIStyle.set_font_size(label, font_size)
	return label

func _check(text: String, font_size: int) -> CheckBox:
	var check := CheckBox.new()
	check.text = text
	UIStyle.set_font_size(check, font_size)
	check.toggled.connect(func(_on: bool) -> void: _emit_changed())
	return check
