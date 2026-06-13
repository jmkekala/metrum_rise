## Orbit/pan/zoom camera controller for the asset editor sandbox.
## Uses _input with an explicit viewport-position guard: input is only processed
## when the mouse is inside the 3D viewport area (between the side panels).
extends Node

var panel_left_w := 270.0
var panel_right_w := 300.0
var panel_top_h := 28.0
var panel_bot_h := 140.0
var viewport_rect_control: Control
var right_mouse_pan_enabled := true

const MIN_DISTANCE := 0.5
const MAX_DISTANCE := 1000.0
const MIN_FAR_M := 5000.0
const FAR_MARGIN_M := 1000.0
const FOCUS_PADDING_MULT := 2.5
const INITIAL_FOCUS_RADIUS_M := 8.0

var _cam: Camera3D

var pivot    := Vector3.ZERO
var yaw      := -0.785   # -45 deg
var pitch    := -0.785   # -45 deg
var distance := 20.0

var _orbit_active := false
var _pan_active   := false

# ──────────────────────────────────────────────────────────────────────────────

func _ready() -> void:
	_cam = get_parent().find_child("CameraNode", true, false) as Camera3D
	if not _cam:
		push_error("EditorCameraInput: no CameraNode found in parent scene")
		return
	if _cam.has_method("set_distance_bounds"):
		_cam.set_distance_bounds(MIN_DISTANCE, MAX_DISTANCE)
	if _cam.has_method("set_clip_policy"):
		_cam.set_clip_policy(MIN_DISTANCE, MIN_FAR_M, FAR_MARGIN_M)
	if _cam.has_method("set_focus_padding"):
		_cam.set_focus_padding(FOCUS_PADDING_MULT)
	if _cam.has_method("focus_on"):
		_cam.focus_on(Vector3.ZERO, INITIAL_FOCUS_RADIUS_M)
	else:
		_update_transform()

func _process(delta: float) -> void:
	if not _cam or _ui_captures_editor_keyboard_input():
		return

	var pan_axis := Vector2.ZERO
	if Input.is_key_pressed(KEY_A) or Input.is_key_pressed(KEY_LEFT):
		pan_axis.x -= 1.0
	if Input.is_key_pressed(KEY_D) or Input.is_key_pressed(KEY_RIGHT):
		pan_axis.x += 1.0
	if Input.is_key_pressed(KEY_W) or Input.is_key_pressed(KEY_UP):
		pan_axis.y += 1.0
	if Input.is_key_pressed(KEY_S) or Input.is_key_pressed(KEY_DOWN):
		pan_axis.y -= 1.0

	if pan_axis.length_squared() == 0.0:
		return

	if _cam.has_method("pan"):
		pan_axis = pan_axis.normalized()
		_cam.pan(Vector3(pan_axis.x, 0.0, -pan_axis.y), 1.0, delta)

## Point the camera at `center` with enough distance to see a sphere of `radius`.
func focus_on(center: Vector3, radius: float) -> void:
	if not _cam:
		return
	if _cam.has_method("focus_on"):
		_cam.focus_on(center, radius)
		return
	pivot    = center
	distance = max(radius * 2.5, 3.0)
	_update_transform()

# ──────────────────────────────────────────────────────────────────────────────

func _input(event: InputEvent) -> void:
	if not _cam:
		return

	var over_ui: bool = _ui_has_modal_popup() or not _is_mouse_in_3d_area()

	if event is InputEventMouseButton:
		match event.button_index:
			MOUSE_BUTTON_MIDDLE:
				# Start orbit only when pressing over the 3D area;
				# always allow release so drag doesn't get stuck.
				if not over_ui or not event.pressed:
					_orbit_active = event.pressed
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_RIGHT:
				if right_mouse_pan_enabled and (not over_ui or not event.pressed):
					_pan_active = event.pressed
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_WHEEL_UP:
				if not over_ui:
					if _cam.has_method("zoom"):
						_cam.zoom(1.0)
					else:
						distance = maxf(MIN_DISTANCE, distance / 1.2)
						_update_transform()
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_WHEEL_DOWN:
				if not over_ui:
					if _cam.has_method("zoom"):
						_cam.zoom(-1.0)
					else:
						distance = minf(MAX_DISTANCE, distance * 1.2)
						_update_transform()
					get_viewport().set_input_as_handled()

	elif event is InputEventMouseMotion:
		if _orbit_active:
			if _cam.has_method("orbit"):
				_cam.orbit(event.relative)
			else:
				yaw   -= event.relative.x * 0.005
				pitch  = clampf(pitch - event.relative.y * 0.005, -1.5, -0.05)
				_update_transform()
			get_viewport().set_input_as_handled()
		elif _pan_active:
			if _cam.has_method("pan_screen"):
				_cam.pan_screen(event.relative)
			else:
				var right: Vector3 = _cam.global_transform.basis.x
				pivot -= right * event.relative.x * distance * 0.001
				pivot += Vector3.UP * event.relative.y * distance * 0.001
				_update_transform()
			get_viewport().set_input_as_handled()

# ──────────────────────────────────────────────────────────────────────────────

func _is_mouse_in_3d_area() -> bool:
	var mouse_pos := get_viewport().get_mouse_position()
	if viewport_rect_control and is_instance_valid(viewport_rect_control):
		return viewport_rect_control.get_global_rect().has_point(mouse_pos)
	var vp_size   := get_viewport().get_visible_rect().size
	return (mouse_pos.x > panel_left_w and
			mouse_pos.x < vp_size.x - panel_right_w and
			mouse_pos.y > panel_top_h and
			mouse_pos.y < vp_size.y - panel_bot_h)

func _ui_has_modal_popup() -> bool:
	var viewport := get_viewport()
	var window := viewport as Window
	return (
		window != null
		and window.has_method("has_visible_popup")
		and window.has_visible_popup()
	)

func _ui_captures_editor_keyboard_input() -> bool:
	var viewport := get_viewport()
	var focus_owner := viewport.gui_get_focus_owner()
	var editing_focus := (
		focus_owner is SpinBox
		or focus_owner is LineEdit
		or focus_owner is TextEdit
		or focus_owner is CodeEdit
	)
	return _ui_has_modal_popup() or editing_focus

func _update_transform() -> void:
	if not _cam:
		return
	var rotation := Basis(Vector3.UP, yaw) * Basis(Vector3.RIGHT, pitch)
	_cam.global_position = pivot + rotation * Vector3(0.0, 0.0, distance)
	_cam.look_at(pivot)
