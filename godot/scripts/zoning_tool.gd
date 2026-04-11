## Profile-driven zoning paint tool — supports rectangle and brush paint through one patch API.
##
## Rust methods called: get_zone_profiles(), capture_zoning_patch(), apply_zoning_patch(),
##   restore_zoning_patch(), intersect_terrain(), get_zone_grid_size()
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var zoning_overlay = $"../ZoningOverlay"

var active: bool = false
var current_profile_runtime_id: int = 0
var paint_mode: String = "rectangle"
var brush_radius_cells: int = 1

var zone_grid_w: int = 0
var zone_grid_h: int = 0
var profiles: Array[Dictionary] = []
var profiles_by_runtime_id: Dictionary = {}

var dragging: bool = false
var drag_start_cell: Vector2i = Vector2i.ZERO
var drag_current_cell: Vector2i = Vector2i.ZERO
var last_brush_cell: Vector2i = Vector2i.ZERO
var stroke_cells: Dictionary = {}

var undo_stack: Array = []
const UNDO_MAX: int = 20

var preview_mesh: MeshInstance3D

func _ready():
	var size: Vector2i = simulation_node.get_zone_grid_size()
	zone_grid_w = size.x
	zone_grid_h = size.y
	_reload_profiles()

	preview_mesh = MeshInstance3D.new()
	var mat := StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.albedo_color = Color(0.2, 0.8, 0.2, 0.35)
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	preview_mesh.material_override = mat
	preview_mesh.top_level = true
	preview_mesh.visible = false
	add_child(preview_mesh)

func _process(_delta):
	if not active:
		preview_mesh.visible = false
		return

	if dragging:
		var cell = _mouse_grid_cell()
		if cell != null:
			var current: Vector2i = cell
			if paint_mode == "brush":
				if current != last_brush_cell:
					_stamp_supercover_line(last_brush_cell, current)
					last_brush_cell = current
				drag_current_cell = current
			else:
				drag_current_cell = current
			_update_preview()

func _unhandled_input(event):
	if not active:
		return

	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			var cell = _mouse_grid_cell()
			if cell != null:
				_begin_drag(cell)
		else:
			if dragging:
				_commit_paint()
			dragging = false
			preview_mesh.visible = false

func _reload_profiles() -> void:
	profiles.clear()
	profiles_by_runtime_id.clear()
	var payload = simulation_node.get_zone_profiles()
	if payload is Array:
		for entry in payload:
			if entry is Dictionary:
				var profile: Dictionary = entry
				profiles.append(profile.duplicate(true))
				profiles_by_runtime_id[int(profile.get("runtime_id", 0))] = profile
	if current_profile_runtime_id == 0 and not profiles.is_empty():
		current_profile_runtime_id = int(profiles[0].get("runtime_id", 0))

func select_profile(runtime_id: int) -> void:
	if runtime_id == 0 or profiles_by_runtime_id.has(runtime_id):
		current_profile_runtime_id = runtime_id
		_update_preview_material()

func select_profile_by_zone_type(zone_type: String) -> void:
	for profile in profiles:
		if str(profile.get("zone_type", "")).strip_edges() == zone_type:
			select_profile(int(profile.get("runtime_id", 0)))
			return

func set_paint_mode(mode: String) -> void:
	if mode == "rectangle" or mode == "brush":
		paint_mode = mode

func undo() -> void:
	if undo_stack.is_empty():
		return
	var op: Dictionary = undo_stack.pop_back()
	simulation_node.restore_zoning_patch(
		int(op.get("grid_x", 0)),
		int(op.get("grid_y", 0)),
		int(op.get("width_cells", 0)),
		int(op.get("height_cells", 0)),
		op.get("bytes", PackedByteArray())
	)
	if zoning_overlay:
		zoning_overlay.mark_zone_dirty()

func _begin_drag(cell: Vector2i) -> void:
	dragging = true
	drag_start_cell = cell
	drag_current_cell = cell
	last_brush_cell = cell
	stroke_cells.clear()
	if paint_mode == "brush":
		_add_brush_stamp(cell)
	_update_preview()

func _commit_paint() -> void:
	var patch := _build_patch_from_drag()
	if patch.is_empty():
		return

	var grid_x: int = patch["grid_x"]
	var grid_y: int = patch["grid_y"]
	var width_cells: int = patch["width_cells"]
	var height_cells: int = patch["height_cells"]
	var write_mask: PackedByteArray = patch["write_mask"]
	if width_cells <= 0 or height_cells <= 0 or write_mask.is_empty():
		return

	var before: PackedByteArray = simulation_node.capture_zoning_patch(
		grid_x,
		grid_y,
		width_cells,
		height_cells
	)
	_push_undo(grid_x, grid_y, width_cells, height_cells, before)
	simulation_node.apply_zoning_patch(
		grid_x,
		grid_y,
		width_cells,
		height_cells,
		current_profile_runtime_id,
		write_mask
	)
	if zoning_overlay:
		zoning_overlay.mark_zone_dirty()

func _push_undo(
	grid_x: int,
	grid_y: int,
	width_cells: int,
	height_cells: int,
	bytes: PackedByteArray
) -> void:
	undo_stack.append({
		"grid_x": grid_x,
		"grid_y": grid_y,
		"width_cells": width_cells,
		"height_cells": height_cells,
		"bytes": bytes,
	})
	if undo_stack.size() > UNDO_MAX:
		undo_stack.pop_front()

func _build_patch_from_drag() -> Dictionary:
	if paint_mode == "brush":
		return _build_brush_patch()
	return _build_rectangle_patch()

func _build_rectangle_patch() -> Dictionary:
	var x0 := mini(drag_start_cell.x, drag_current_cell.x)
	var y0 := mini(drag_start_cell.y, drag_current_cell.y)
	var x1 := maxi(drag_start_cell.x, drag_current_cell.x)
	var y1 := maxi(drag_start_cell.y, drag_current_cell.y)
	var width_cells := x1 - x0 + 1
	var height_cells := y1 - y0 + 1
	if width_cells <= 0 or height_cells <= 0:
		return {}
	var write_mask := PackedByteArray()
	write_mask.resize(width_cells * height_cells)
	for i in range(write_mask.size()):
		write_mask[i] = 1
	return {
		"grid_x": x0,
		"grid_y": y0,
		"width_cells": width_cells,
		"height_cells": height_cells,
		"write_mask": write_mask,
	}

func _build_brush_patch() -> Dictionary:
	if stroke_cells.is_empty():
		return {}

	var min_x := zone_grid_w
	var min_y := zone_grid_h
	var max_x := 0
	var max_y := 0
	for key in stroke_cells.keys():
		var cell := _decode_cell_key(str(key))
		min_x = mini(min_x, cell.x)
		min_y = mini(min_y, cell.y)
		max_x = maxi(max_x, cell.x)
		max_y = maxi(max_y, cell.y)

	var width_cells := max_x - min_x + 1
	var height_cells := max_y - min_y + 1
	if width_cells <= 0 or height_cells <= 0:
		return {}

	var write_mask := PackedByteArray()
	write_mask.resize(width_cells * height_cells)
	for y in range(min_y, max_y + 1):
		for x in range(min_x, max_x + 1):
			var local_idx := (y - min_y) * width_cells + (x - min_x)
			write_mask[local_idx] = 1 if stroke_cells.has(_cell_key(Vector2i(x, y))) else 0

	return {
		"grid_x": min_x,
		"grid_y": min_y,
		"width_cells": width_cells,
		"height_cells": height_cells,
		"write_mask": write_mask,
	}

func _stamp_supercover_line(a: Vector2i, b: Vector2i) -> void:
	for cell in _supercover_line(a, b):
		_add_brush_stamp(cell)

func _supercover_line(a: Vector2i, b: Vector2i) -> Array[Vector2i]:
	var points: Array[Vector2i] = [a]
	var x := a.x
	var y := a.y
	var dx := b.x - a.x
	var dy := b.y - a.y
	var x_inc := 1 if dx >= 0 else -1
	var y_inc := 1 if dy >= 0 else -1
	var abs_dx := absi(dx)
	var abs_dy := absi(dy)

	if abs_dx >= abs_dy:
		var error := abs_dx / 2
		for _step in range(abs_dx):
			x += x_inc
			error -= abs_dy
			if error < 0:
				points.append(Vector2i(x - x_inc, y + y_inc))
				y += y_inc
				error += abs_dx
			points.append(Vector2i(x, y))
	else:
		var error := abs_dy / 2
		for _step in range(abs_dy):
			y += y_inc
			error -= abs_dx
			if error < 0:
				points.append(Vector2i(x + x_inc, y - y_inc))
				x += x_inc
				error += abs_dy
			points.append(Vector2i(x, y))

	return points

func _add_brush_stamp(center: Vector2i) -> void:
	for dy in range(-brush_radius_cells, brush_radius_cells + 1):
		for dx in range(-brush_radius_cells, brush_radius_cells + 1):
			if dx * dx + dy * dy > brush_radius_cells * brush_radius_cells:
				continue
			var cell := Vector2i(center.x + dx, center.y + dy)
			if cell.x < 0 or cell.y < 0 or cell.x >= zone_grid_w or cell.y >= zone_grid_h:
				continue
			stroke_cells[_cell_key(cell)] = true

func _update_preview() -> void:
	var bbox := _preview_bbox()
	if bbox.is_empty():
		preview_mesh.visible = false
		return

	var min_x: int = bbox["min_x"]
	var min_y: int = bbox["min_y"]
	var width_cells: int = bbox["width_cells"]
	var height_cells: int = bbox["height_cells"]
	if width_cells <= 0 or height_cells <= 0:
		preview_mesh.visible = false
		return

	var quad := QuadMesh.new()
	quad.size = Vector2(float(width_cells), float(height_cells))
	preview_mesh.mesh = quad
	preview_mesh.position = Vector3(
		float(min_x) + float(width_cells) * 0.5 - float(zone_grid_w - 1) * 0.5 - 0.5,
		0.3,
		float(min_y) + float(height_cells) * 0.5 - float(zone_grid_h - 1) * 0.5 - 0.5
	)
	preview_mesh.rotation_degrees = Vector3(-90.0, 0.0, 0.0)
	preview_mesh.visible = true
	_update_preview_material()

func _preview_bbox() -> Dictionary:
	if paint_mode == "brush":
		if dragging:
			return {
				"min_x": maxi(0, drag_current_cell.x - brush_radius_cells),
				"min_y": maxi(0, drag_current_cell.y - brush_radius_cells),
				"width_cells": mini(zone_grid_w - 1, drag_current_cell.x + brush_radius_cells) - maxi(0, drag_current_cell.x - brush_radius_cells) + 1,
				"height_cells": mini(zone_grid_h - 1, drag_current_cell.y + brush_radius_cells) - maxi(0, drag_current_cell.y - brush_radius_cells) + 1,
			}
		return {}

	var x0 := mini(drag_start_cell.x, drag_current_cell.x)
	var y0 := mini(drag_start_cell.y, drag_current_cell.y)
	var x1 := maxi(drag_start_cell.x, drag_current_cell.x)
	var y1 := maxi(drag_start_cell.y, drag_current_cell.y)
	return {
		"min_x": x0,
		"min_y": y0,
		"width_cells": x1 - x0 + 1,
		"height_cells": y1 - y0 + 1,
	}

func _update_preview_material() -> void:
	if not preview_mesh or not preview_mesh.material_override:
		return
	var mat := preview_mesh.material_override as StandardMaterial3D
	mat.albedo_color = _selected_profile_color()

func _selected_profile_color() -> Color:
	if current_profile_runtime_id == 0:
		return Color(1.0, 0.2, 0.2, 0.35)
	var profile: Dictionary = profiles_by_runtime_id.get(current_profile_runtime_id, {})
	return _color_from_hex(str(profile.get("ui_color", "#44CC44")), 0.35)

func _color_from_hex(hex: String, alpha: float) -> Color:
	if hex.length() == 7 and hex.begins_with("#"):
		var r := hex.substr(1, 2).hex_to_int()
		var g := hex.substr(3, 2).hex_to_int()
		var b := hex.substr(5, 2).hex_to_int()
		return Color8(r, g, b, int(clampf(alpha, 0.0, 1.0) * 255.0))
	return Color(0.25, 0.8, 0.25, alpha)

func _cell_key(cell: Vector2i) -> String:
	return "%d:%d" % [cell.x, cell.y]

func _decode_cell_key(key: String) -> Vector2i:
	var parts := key.split(":")
	if parts.size() != 2:
		return Vector2i.ZERO
	return Vector2i(int(parts[0]), int(parts[1]))

func _mouse_grid_cell() -> Variant:
	var wp = _mouse_world_pos()
	if wp == null:
		return null
	var hw := float(zone_grid_w - 1) * 0.5
	var hh := float(zone_grid_h - 1) * 0.5
	return Vector2i(
		clampi(int(round(wp.x + hw)), 0, zone_grid_w - 1),
		clampi(int(round(wp.y + hh)), 0, zone_grid_h - 1)
	)

func _mouse_world_pos() -> Variant:
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if camera == null:
		return null
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var hit = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if hit == null:
		return null
	return Vector2(hit.x, hit.z)
