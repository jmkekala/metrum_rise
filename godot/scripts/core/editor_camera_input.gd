# SPDX-License-Identifier: GPL-2.0-only

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
# Called with the press position when Alt (Option on a Mac) and the left button come up without
# a drag, so the owner keeps Alt+click for its own action while Alt+drag orbits.
var alt_click := Callable()

const MIN_DISTANCE := 0.5
const MAX_DISTANCE := 1000.0
const MIN_FAR_M := 5000.0
const FAR_MARGIN_M := 1000.0
const FOCUS_PADDING_MULT := 2.5
const INITIAL_FOCUS_RADIUS_M := 8.0
# Pointer travel, in pixels, that turns an Alt press into an orbit rather than a click.
const ALT_DRAG_PX := 4.0

var _cam: CameraNode

var _orbit_active := false
var _pan_active   := false
# Alt with the left button stands in for the middle button, which a touchpad does not have.
var _alt_pressed := false
var _alt_orbit := false
var _alt_start := Vector2.ZERO

# ──────────────────────────────────────────────────────────────────────────────

func _ready() -> void:
	_cam = get_parent().find_child("CameraNode", true, false) as CameraNode
	if not _cam:
		push_error("EditorCameraInput: no CameraNode found in parent scene")
		return
	_cam.set_distance_bounds(MIN_DISTANCE, MAX_DISTANCE)
	_cam.set_clip_policy(MIN_DISTANCE, MIN_FAR_M, FAR_MARGIN_M)
	_cam.set_focus_padding(FOCUS_PADDING_MULT)
	_cam.focus_on(Vector3.ZERO, INITIAL_FOCUS_RADIUS_M)

func _process(delta: float) -> void:
	_update_preview_offset()
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

	_cam.pan(Vector3(pan_axis.x, 0.0, -pan_axis.y), 1.0, delta)

## Point the camera at `center` with enough distance to see a sphere of `radius`.
func focus_on(center: Vector3, radius: float) -> void:
	if not _cam:
		return
	_cam.focus_on(center, radius)
	_update_preview_offset()

## O(1) presentation adjustment: orbit around the asset, centered in the visible pane.
## Camera offsets are preview-only; picking and projected LOD bounds use the same camera transform.
func _update_preview_offset() -> void:
	if _cam == null or not is_instance_valid(viewport_rect_control):
		return
	var size := get_viewport().get_visible_rect().size
	if size.x <= 0 or size.y <= 0:
		return
	var center := viewport_rect_control.get_global_rect().get_center()
	var ndc := Vector2(2.0 * center.x / size.x - 1.0, 1.0 - 2.0 * center.y / size.y)
	var projection := _cam.get_camera_projection()
	var depth := 1.0 if _cam.projection == Camera3D.PROJECTION_ORTHOGONAL else -(_cam.global_transform.affine_inverse() * _cam.get_focus_position()).z
	var horizontal := -ndc.x * depth / projection.x.x
	var vertical := -ndc.y * depth / projection.y.y
	if not is_equal_approx(_cam.h_offset, horizontal):
		_cam.h_offset = horizontal
	if not is_equal_approx(_cam.v_offset, vertical):
		_cam.v_offset = vertical

# ──────────────────────────────────────────────────────────────────────────────

func _input(event: InputEvent) -> void:
	if not _cam:
		return

	# A mouse event carries its own position, which is also right for injected events; the
	# viewport's pointer only stands in for the others.
	var pointer: Vector2 = event.position if event is InputEventMouse else get_viewport().get_mouse_position()
	var over_ui: bool = _ui_has_modal_popup() or not _is_in_3d_area(pointer)

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
			MOUSE_BUTTON_LEFT:
				if event.pressed and event.alt_pressed and not over_ui and not _gui_owns_pointer():
					_alt_pressed = true
					_alt_orbit = false
					_alt_start = event.position
					get_viewport().set_input_as_handled()
				elif not event.pressed and _alt_pressed:
					_alt_pressed = false
					if not _alt_orbit and alt_click.is_valid():
						alt_click.call(_alt_start)
					_alt_orbit = false
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_WHEEL_UP:
				if not over_ui:
					_cam.zoom(1.0)
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_WHEEL_DOWN:
				if not over_ui:
					_cam.zoom(-1.0)
					get_viewport().set_input_as_handled()

	elif event is InputEventMouseMotion:
		if _alt_pressed and not _alt_orbit and event.position.distance_to(_alt_start) > ALT_DRAG_PX:
			_alt_orbit = true
		if _orbit_active or _alt_orbit:
			_cam.orbit(event.relative)
			get_viewport().set_input_as_handled()
		elif _pan_active:
			_cam.pan_screen(event.relative)
			get_viewport().set_input_as_handled()
		elif _alt_pressed:
			get_viewport().set_input_as_handled()

	# A touchpad sends a two-finger scroll and a pinch as gestures, not as wheel buttons. The
	# scroll's delta is negative where a wheel turns up, one unit to a notch.
	elif event is InputEventPanGesture:
		if not over_ui:
			_cam.zoom(-event.delta.y)
			get_viewport().set_input_as_handled()
	elif event is InputEventMagnifyGesture:
		if not over_ui:
			_cam.zoom(log(event.factor) / log(_cam.zoom_speed))
			get_viewport().set_input_as_handled()

# ──────────────────────────────────────────────────────────────────────────────

func _is_in_3d_area(mouse_pos: Vector2) -> bool:
	if viewport_rect_control and is_instance_valid(viewport_rect_control):
		return viewport_rect_control.get_global_rect().has_point(mouse_pos)
	var vp_size   := get_viewport().get_visible_rect().size
	return (mouse_pos.x > panel_left_w and
			mouse_pos.x < vp_size.x - panel_right_w and
			mouse_pos.y > panel_top_h and
			mouse_pos.y < vp_size.y - panel_bot_h)

# Controls inside the 3D pane, such as the thumbnail framing actions, keep their own clicks.
func _gui_owns_pointer() -> bool:
	var control := get_viewport().gui_get_hovered_control()
	return control != null and control.mouse_filter == Control.MOUSE_FILTER_STOP

func _ui_has_modal_popup() -> bool:
	# Dialogs and popup menus own their own viewport, including embedded windows.
	var focused := Window.get_focused_window()
	return focused != null and focused != get_window()

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
