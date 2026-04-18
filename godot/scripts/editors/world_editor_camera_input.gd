## Orbit/pan/zoom camera controller for the world editor.
## Keeps panning on the terrain XZ plane and adds keyboard pan so the editor
## is usable even on a blank, featureless map.
extends Node

var panel_left_w := 0.0
var panel_right_w := 0.0
var panel_top_h := 28.0
var panel_bot_h := 104.0

var _cam: Camera3D

var pivot := Vector3.ZERO
var yaw := -0.785
var pitch := -0.785
var distance := 20.0

var _orbit_active := false
var _pan_active := false

func _ready() -> void:
	_cam = get_parent().find_child("CameraNode", true, false) as Camera3D
	if not _cam:
		push_error("WorldEditorCameraInput: no CameraNode found in parent scene")
		return
	_update_transform()

func _process(delta: float) -> void:
	if not _cam:
		return
	if get_viewport().gui_get_focus_owner() != null:
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

	pan_axis = pan_axis.normalized()
	var pan_scale := maxf(distance * 0.8, 10.0) * delta
	pivot += _horizontal_right() * pan_axis.x * pan_scale
	pivot += _horizontal_forward() * pan_axis.y * pan_scale
	_update_transform()

## Point the camera at `center` with enough distance to see a sphere of `radius`.
func focus_on(center: Vector3, radius: float) -> void:
	pivot = center
	distance = max(radius * 2.5, 3.0)
	_update_transform()

func _input(event: InputEvent) -> void:
	if not _cam:
		return

	var over_ui := get_viewport().gui_get_hovered_control() != null or not _is_mouse_in_3d_area()

	if event is InputEventMouseButton:
		match event.button_index:
			MOUSE_BUTTON_MIDDLE:
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
					distance = minf(50000.0, distance * 1.2)
					_update_transform()
					get_viewport().set_input_as_handled()

	elif event is InputEventMouseMotion:
		if _orbit_active:
			yaw -= event.relative.x * 0.005
			pitch = clampf(pitch - event.relative.y * 0.005, -1.5, -0.05)
			_update_transform()
			get_viewport().set_input_as_handled()
		elif _pan_active:
			var pan_scale := maxf(distance * 0.0025, 0.5)
			pivot -= _horizontal_right() * event.relative.x * pan_scale
			pivot -= _horizontal_forward() * event.relative.y * pan_scale
			_update_transform()
			get_viewport().set_input_as_handled()

func _horizontal_right() -> Vector3:
	var right := _cam.global_transform.basis.x
	right.y = 0.0
	if right.length_squared() < 0.0001:
		return Vector3.RIGHT
	return right.normalized()

func _horizontal_forward() -> Vector3:
	var forward := -_cam.global_transform.basis.z
	forward.y = 0.0
	if forward.length_squared() < 0.0001:
		return Vector3.FORWARD
	return forward.normalized()

func _is_mouse_in_3d_area() -> bool:
	var mouse_pos := get_viewport().get_mouse_position()
	var vp_size := get_viewport().get_visible_rect().size
	return (
		mouse_pos.x > panel_left_w
		and mouse_pos.x < vp_size.x - panel_right_w
		and mouse_pos.y > panel_top_h
		and mouse_pos.y < vp_size.y - panel_bot_h
	)

func _update_transform() -> void:
	if not _cam:
		return
	var rotation := Basis(Vector3.UP, yaw) * Basis(Vector3.RIGHT, pitch)
	_cam.global_position = pivot + rotation * Vector3(0.0, 0.0, distance)
	_cam.look_at(pivot)
