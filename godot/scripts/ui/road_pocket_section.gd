# SPDX-License-Identifier: GPL-2.0-only

## The Road Properties window's "Turn pocket" section (`ROAD-95`, D12): the widgets for one road
## end, the end nearest the player's click. A heading names the end by its junction's direction
## ("towards north-east"), a line says what the end has now, Left and Right check boxes pick the
## pocket lanes, a Length stepper sets the body from the junction, and the verdict line sits
## beside Apply. It holds no rule: the host fills it from Rust (`get_road_pocket`: the roles the
## road can carry, the length range, the pocket there) and shows Rust's verdict; the view reports
## the player's changes through `changed` and `apply_pressed`.
extends VBoxContainer

## The player changed a control: the pocket the controls now ask for (both off removes it).
signal changed(left: bool, right: bool, length_m: float)
## The player pressed Apply.
signal apply_pressed

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const RoadReasonText = preload("res://scripts/ui/road_reason_text.gd")
const VERDICT_READY := Color(0.55, 0.95, 0.55)
const VERDICT_REFUSED := Color.YELLOW
# Compass names by 45 degree sector from north (-Z), clockwise (east is +X).
const COMPASS: Array[String] = ["north", "north-east", "east", "south-east", "south", "south-west",
	"west", "north-west"]
# How far back along the road the end's direction is read: the road's heading into its junction.
const HEADING_RUN_M := 20.0

var _heading_label: Label
var _state_label: Label
var _left_check: CheckBox
var _right_check: CheckBox
var _length_spin: SpinBox
var _verdict_label: Label
var _apply_button: Button

func _init(font_size: int = 14) -> void:
	add_theme_constant_override("separation", 6)
	_heading_label = _label(font_size)
	_heading_label.add_theme_color_override("font_color", UIStyle.TEXT_SECTION)
	add_child(_heading_label)
	_state_label = _label(font_size - 1)
	add_child(_state_label)

	var roles := HBoxContainer.new()
	_left_check = _check("Left", font_size)
	roles.add_child(_left_check)
	_right_check = _check("Right", font_size)
	roles.add_child(_right_check)
	add_child(roles)

	var length_row := HBoxContainer.new()
	var length_label := _label(font_size)
	length_label.text = "Length"
	length_label.custom_minimum_size.x = 60.0
	length_row.add_child(length_label)
	_length_spin = SpinBox.new()
	_length_spin.suffix = "m"
	_length_spin.rounded = true
	UIStyle.set_font_size(_length_spin.get_line_edit(), font_size)
	_length_spin.value_changed.connect(func(_value: float) -> void: _emit_changed())
	length_row.add_child(_length_spin)
	var from_label := _label(font_size)
	from_label.autowrap_mode = TextServer.AUTOWRAP_OFF
	from_label.text = "from the junction"
	length_row.add_child(from_label)
	add_child(length_row)

	var apply_row := HBoxContainer.new()
	_verdict_label = _label(font_size - 1)
	_verdict_label.size_flags_horizontal = Control.SIZE_EXPAND_FILL
	apply_row.add_child(_verdict_label)
	_apply_button = Button.new()
	_apply_button.text = "Apply"
	_apply_button.disabled = true
	UIStyle.set_font_size(_apply_button, font_size)
	_apply_button.pressed.connect(func() -> void: apply_pressed.emit())
	apply_row.add_child(_apply_button)
	add_child(apply_row)

## Shows road end `info` (`get_road_pocket`'s dictionary) without a request: the heading, what
## the end has, the check boxes and length from its pocket (or none and the default length),
## each role enabled when the road can carry it or the pocket has it (so it can be removed), a
## refused role's reason as its tooltip.
func show_end(info: Dictionary) -> void:
	_heading_label.text = "Turn pocket at the end towards %s" % heading_name(info.get("road", PackedVector3Array()))
	var has_pocket := bool(info.get("has_pocket", false))
	var left := has_pocket and bool(info.get("left", false))
	var right := has_pocket and bool(info.get("right", false))
	var length := float(info.get("length_m", 0.0)) if has_pocket else float(info.get("default_length_m", 60.0))
	_state_label.text = describe(left, right, length) if has_pocket else "No turn pocket at this end"
	_set_role(_left_check, left, bool(info.get("left_offered", false)), String(info.get("left_refusal", "")))
	_set_role(_right_check, right, bool(info.get("right_offered", false)), String(info.get("right_refusal", "")))
	_length_spin.min_value = float(info.get("min_length_m", 20.0))
	_length_spin.max_value = float(info.get("max_length_m", 300.0))
	_length_spin.step = float(info.get("step_m", 10.0))
	_length_spin.set_value_no_signal(length)
	show_verdict({}, "")

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
	_verdict_label.text = accepted_text if ready else RoadReasonText.text(verdict, "This road end cannot take this pocket")

## Shows a refused commit's line (the verdict changed between the preview and Apply).
func show_refused(text: String) -> void:
	_apply_button.disabled = true
	_verdict_label.add_theme_color_override("font_color", VERDICT_REFUSED)
	_verdict_label.text = text

## The request the controls show: [left, right, length_m].
func request() -> Array:
	return [_left_check.button_pressed, _right_check.button_pressed, _length_spin.value]

## "Left pocket, 60 m", "Left and right pockets, 80 m", or "No turn pocket" with both off.
static func describe(left: bool, right: bool, length_m: float) -> String:
	if not left and not right:
		return "No turn pocket"
	var roles := "Left and right pockets" if left and right else ("Left pocket" if left else "Right pocket")
	return "%s, %d m from the junction" % [roles, roundi(length_m)]

## The compass direction a road heads into its junction, from `road` (the polyline from the
## junction outwards): the bearing from the point HEADING_RUN_M back along it (or its far end) to
## the junction, by eight 45 degree sectors from north (-Z).
static func heading_name(road: PackedVector3Array) -> String:
	if road.size() < 2:
		return "the junction"
	var back := road[road.size() - 1]
	var run := 0.0
	for i in range(1, road.size()):
		var step := Vector2(road[i].x - road[i - 1].x, road[i].z - road[i - 1].z).length()
		if run + step >= HEADING_RUN_M:
			back = road[i - 1].lerp(road[i], (HEADING_RUN_M - run) / step)
			break
		run += step
	var towards := Vector2(road[0].x - back.x, road[0].z - back.z)
	if towards.length_squared() < 1e-6:
		return "the junction"
	# Bearing clockwise from north: atan2(east, north) with north = -Z.
	var bearing := fposmod(rad_to_deg(atan2(towards.x, -towards.y)), 360.0)
	return COMPASS[int(roundf(bearing / 45.0)) % COMPASS.size()]

func _set_role(check: CheckBox, pressed: bool, offered: bool, refusal: String) -> void:
	check.set_pressed_no_signal(pressed)
	check.disabled = not offered and not pressed
	check.tooltip_text = "" if offered else RoadReasonText.text({"invalid_reason": refusal}, "")

func _emit_changed() -> void:
	changed.emit(_left_check.button_pressed, _right_check.button_pressed, _length_spin.value)

func _label(font_size: int) -> Label:
	var label := Label.new()
	label.autowrap_mode = TextServer.AUTOWRAP_WORD_SMART
	UIStyle.set_font_size(label, font_size)
	return label

func _check(text: String, font_size: int) -> CheckBox:
	var check := CheckBox.new()
	check.text = text
	UIStyle.set_font_size(check, font_size)
	check.toggled.connect(func(_on: bool) -> void: _emit_changed())
	return check
