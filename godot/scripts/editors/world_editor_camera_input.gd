# SPDX-License-Identifier: GPL-2.0-only

## Orbit/pan/zoom camera controller for the world editor.
## Routes editor input into the shared CameraNode world-camera core while
## keeping WorldEditor-specific UI capture and zoom/far policy local here.
extends Node

var panel_left_w := 0.0
var panel_right_w := 0.0
var panel_top_h := 28.0
var panel_bot_h := 104.0

var _cam: Camera3D

const MIN_DISTANCE := 0.5
const MAX_DISTANCE := 200000.0
const MIN_FAR_M := 20000.0
const FAR_MARGIN_M := 5000.0
const FOCUS_PADDING_MULT := 3.25
const TERRAIN_PIVOT_CLEARANCE_M := 0.25
const TERRAIN_CAMERA_CLEARANCE_M := 1.5
const WORLD_KEYBOARD_PASSTHROUGH_META := "world_editor_keyboard_passthrough"

var _orbit_active := false
var _pan_active := false

func _ready() -> void:
	_cam = get_parent().find_child("CameraNode", true, false) as Camera3D
	if not _cam:
		push_error("WorldEditorCameraInput: no CameraNode found in parent scene")
		return
	if _cam.has_method("set_distance_bounds"):
		_cam.set_distance_bounds(MIN_DISTANCE, MAX_DISTANCE)
	if _cam.has_method("set_clip_policy"):
		_cam.set_clip_policy(MIN_DISTANCE, MIN_FAR_M, FAR_MARGIN_M)
	if _cam.has_method("set_focus_padding"):
		_cam.set_focus_padding(FOCUS_PADDING_MULT)
	var debug_under_terrain: bool = _debug_camera_can_go_under_terrain()
	if _cam.has_method("set_terrain_clearance_policy"):
		_cam.set_terrain_clearance_policy(
			not debug_under_terrain,
			TERRAIN_PIVOT_CLEARANCE_M,
			TERRAIN_CAMERA_CLEARANCE_M
		)
	if _cam.has_method("set_debug_under_terrain_enabled"):
		_cam.set_debug_under_terrain_enabled(debug_under_terrain)

func _debug_camera_can_go_under_terrain() -> bool:
	var debug_value: String = OS.get_environment("METRUM_DEBUG").strip_edges()
	return not debug_value.is_empty() and debug_value != "0"

func _process(delta: float) -> void:
	if not _cam:
		return
	if _ui_captures_world_keyboard_input():
		_orbit_active = false
		_pan_active = false
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

func _input(event: InputEvent) -> void:
	if not _cam:
		return

	var over_ui := _ui_captures_world_pointer_input() or not _is_mouse_in_3d_area()

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
					if _cam.has_method("zoom"):
						_cam.zoom(1.0)
					get_viewport().set_input_as_handled()
			MOUSE_BUTTON_WHEEL_DOWN:
				if not over_ui:
					if _cam.has_method("zoom"):
						_cam.zoom(-1.0)
					get_viewport().set_input_as_handled()

	elif event is InputEventMouseMotion:
		if _orbit_active:
			if _cam.has_method("orbit"):
				_cam.orbit(event.relative)
			get_viewport().set_input_as_handled()
		elif _pan_active:
			if _cam.has_method("pan_screen"):
				_cam.pan_screen(event.relative)
			get_viewport().set_input_as_handled()

func _is_mouse_in_3d_area() -> bool:
	var mouse_pos := get_viewport().get_mouse_position()
	var vp_size := get_viewport().get_visible_rect().size
	return (
		mouse_pos.x > panel_left_w
		and mouse_pos.x < vp_size.x - panel_right_w
		and mouse_pos.y > panel_top_h
		and mouse_pos.y < vp_size.y - panel_bot_h
	)

func _ui_has_modal_popup() -> bool:
	var viewport := get_viewport()
	var window := viewport as Window
	return (
		window != null
		and window.has_method("has_visible_popup")
		and window.has_visible_popup()
	)

func _ui_captures_world_pointer_input() -> bool:
	var viewport := get_viewport()
	return _ui_has_modal_popup() or viewport.gui_get_hovered_control() != null

func _ui_captures_world_keyboard_input() -> bool:
	var viewport := get_viewport()
	var focus_owner := viewport.gui_get_focus_owner()
	if focus_owner != null and _control_allows_world_keyboard_passthrough(focus_owner):
		return _ui_has_modal_popup()
	var editing_focus := (
		focus_owner is SpinBox
		or
		focus_owner is LineEdit
		or focus_owner is TextEdit
		or focus_owner is CodeEdit
	)
	return _ui_has_modal_popup() or editing_focus

func _control_allows_world_keyboard_passthrough(control: Control) -> bool:
	var node: Node = control
	while node != null:
		if node.has_meta(WORLD_KEYBOARD_PASSTHROUGH_META):
			return bool(node.get_meta(WORLD_KEYBOARD_PASSTHROUGH_META))
		node = node.get_parent()
	return false
