## Orbit/pan/zoom camera controller for the asset editor sandbox.
## Uses _input with an explicit viewport-position guard: input is only processed
## when the mouse is inside the 3D viewport area (between the side panels).
extends Node

var panel_left_w := 270.0
var panel_right_w := 300.0
var panel_top_h := 28.0
var panel_bot_h := 140.0

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
	_update_transform()

## Point the camera at `center` with enough distance to see a sphere of `radius`.
func focus_on(center: Vector3, radius: float) -> void:
	pivot    = center
	distance = max(radius * 2.5, 3.0)
	_update_transform()

# ──────────────────────────────────────────────────────────────────────────────

func _input(event: InputEvent) -> void:
	if not _cam:
		return

	var over_ui: bool = get_viewport().gui_get_hovered_control() != null or not _is_mouse_in_3d_area()

	if event is InputEventMouseButton:
		match event.button_index:
			MOUSE_BUTTON_MIDDLE:
				# Start orbit only when pressing over the 3D area;
				# always allow release so drag doesn't get stuck.
				if not over_ui or not event.pressed:
					_orbit_active = event.pressed
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_RIGHT:
				if not over_ui or not event.pressed:
					_pan_active = event.pressed
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_WHEEL_UP:
				if not over_ui:
					distance = maxf(0.5, distance / 1.2)
					_update_transform()
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_WHEEL_DOWN:
				if not over_ui:
					distance = minf(500.0, distance * 1.2)
					_update_transform()
					get_viewport().set_input_as_handled()

	elif event is InputEventMouseMotion:
		if _orbit_active:
			yaw   -= event.relative.x * 0.005
			pitch  = clampf(pitch - event.relative.y * 0.005, -1.5, -0.05)
			_update_transform()
			get_viewport().set_input_as_handled()
		elif _pan_active:
			var right: Vector3 = _cam.global_transform.basis.x
			pivot -= right * event.relative.x * distance * 0.001
			pivot += Vector3.UP * event.relative.y * distance * 0.001
			_update_transform()
			get_viewport().set_input_as_handled()

# ──────────────────────────────────────────────────────────────────────────────

func _is_mouse_in_3d_area() -> bool:
	var mouse_pos := get_viewport().get_mouse_position()
	var vp_size   := get_viewport().get_visible_rect().size
	return (mouse_pos.x > panel_left_w and
			mouse_pos.x < vp_size.x - panel_right_w and
			mouse_pos.y > panel_top_h and
			mouse_pos.y < vp_size.y - panel_bot_h)

func _update_transform() -> void:
	if not _cam:
		return
	var rotation := Basis(Vector3.UP, yaw) * Basis(Vector3.RIGHT, pitch)
	_cam.global_position = pivot + rotation * Vector3(0.0, 0.0, distance)
	_cam.look_at(pivot)
