# SPDX-License-Identifier: GPL-2.0-only

## Cell-zoning gesture controller: retains pointer paths and renders Rust-authored selections.
## One released gesture commits once; cancellation and dependency changes commit nothing.
extends Node3D

var controller: Node3D
var simulation_node: Node
var _active := false
var _dragging := false
var _path := PackedVector2Array()
var _payload: Dictionary = {}
var _settings := PackedFloat64Array()
var _dirty := true

func cancel() -> void:
	_dragging = false
	_path.clear()
	_payload.clear()
	_dirty = true
	_publish_preview({})

func update_tool(enabled: bool) -> void:
	_active = enabled
	if not enabled:
		cancel()
		return
	var settings := _current_settings()
	if settings != _settings:
		cancel()
		_settings = settings
	var point = controller._mouse_world_pos()
	if point == null:
		_dirty = true
		_publish_preview({})
		return
	if _dragging:
		_append_point(point)
	else:
		if _path.is_empty() or _path[0] != point:
			_path = PackedVector2Array([point])
			_dirty = true
	_refresh_preview()

func handle_input(event: InputEvent) -> void:
	if not _active:
		return
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		var point = controller._mouse_world_pos(true)
		if point != null:
			cancel()
			_settings = _current_settings()
			_dragging = true
			_path.append(point)
			_refresh_preview()

func _input(event: InputEvent) -> void:
	if not controller.active or not controller.workflow_cells:
		cancel()
		return
	if not _active or not _dragging:
		return
	if event is InputEventMouseMotion:
		var point = controller._mouse_world_pos()
		if point != null:
			_append_point(point)
	elif event is InputEventKey and event.pressed and event.keycode == KEY_ESCAPE:
		cancel()
	elif event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
			cancel()
		elif event.button_index == MOUSE_BUTTON_LEFT and not event.pressed:
			# A release consumed by a UI control still ends the pending world gesture.
			if get_viewport().gui_get_hovered_control() != null:
				cancel()
			else:
				_finish()

func _append_point(point: Vector2) -> void:
	if controller.cell_shape == 2 or (not _path.is_empty() and _path[-1] == point):
		return
	if controller.cell_shape == 1 and _path.size() > 1:
		_path[1] = point
	else:
		_path.append(point)
	_dirty = true

func _profile() -> int:
	return 0 if controller.cell_erase else controller.current_profile_runtime_id

func _current_settings() -> PackedFloat64Array:
	return PackedFloat64Array([controller.cell_shape, controller.cell_brush_radius_m, _profile()])

func _refresh_preview() -> void:
	if _current_settings() != _settings:
		if _dragging:
			cancel()
			return
		_settings = _current_settings()
		_dirty = true
	var dependencies: PackedInt64Array = simulation_node.get_zoning_cell_dependencies()
	var previous: PackedInt64Array = _payload.get("dependencies", PackedInt64Array())
	if not _same_authoring_dependencies(dependencies, previous):
		if _dragging and not previous.is_empty():
			cancel()
			return
		_dirty = true
	if not _dirty:
		return
	_payload = simulation_node.get_zoning_cells_preview_packed(controller.cell_shape, _path, controller.cell_brush_radius_m, _profile())
	_dirty = false
	_publish_preview(_payload)

func _same_authoring_dependencies(a: PackedInt64Array, b: PackedInt64Array) -> bool:
	if a.size() != b.size():
		return false
	for index in range(a.size()):
		# Loading another visible grid chunk changes only the rebuildable cache epoch.
		if index != 8 and a[index] != b[index]:
			return false
	return true

func _finish() -> void:
	var point = controller._mouse_world_pos(true)
	if point == null:
		cancel()
		return
	_append_point(point)
	_refresh_preview()
	if _dragging and bool(_payload.get("valid", false)):
		simulation_node.apply_zoning_cells_preview(_payload["cells"], _payload["dependencies"], _profile())
	cancel()

func _publish_preview(payload: Dictionary) -> void:
	if controller.zoning_overlay.has_method("set_cell_preview"):
		controller.zoning_overlay.set_cell_preview(payload)
