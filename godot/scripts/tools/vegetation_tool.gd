# SPDX-License-Identifier: GPL-2.0-only

## Player vegetation input and ground-ring preview; all placement decisions stay in Rust.
## Rust methods called: add_vegetation_at(), remove_vegetation_at(), paint_vegetation(),
## intersect_world_surface().
extends Node3D

enum Mode { PLANT, REMOVE }

# The radius is the only size control, and it also chooses between a point edit and a brush: at
# the minimum a plant lands exactly under the cursor, and above it the disc fills a 4 m lattice.
# The ground ring is therefore the mode readout as well as the footprint.
const MIN_RADIUS_M := 1.0
const MAX_RADIUS_M := 1024.0
# Matches the native paint bound; removal retains its 1 km radius.
const MAX_PAINT_RADIUS_M := 256.0
# Geometric, because a fixed step cannot serve both a one-tree cursor and a 1 km clear-cut.
const RADIUS_STEP := 1.4

const PLANT_RING_COLOR := Color(0.85, 0.95, 0.45)
const REMOVE_RING_COLOR := Color(0.95, 0.45, 0.35)

@onready var simulation_node = $"../SimulationNode"
var active := false
var mode: Mode = Mode.PLANT:
	set(value):
		mode = value
		radius = minf(radius, MAX_PAINT_RADIUS_M if mode == Mode.PLANT else MAX_RADIUS_M)
var species := 0
var radius := MIN_RADIUS_M:
	set(value):
		radius = minf(value, MAX_PAINT_RADIUS_M if mode == Mode.PLANT else MAX_RADIUS_M)
var preview: MeshInstance3D
var _ring: TorusMesh
var _ring_material: StandardMaterial3D
var _preview_radius := -1.0
var _preview_mode := -1
var _painting := false
var _last_stamp := Vector2.INF

func _ready() -> void:
	_ring = TorusMesh.new()
	_ring.rings = 64
	_ring.ring_segments = 8
	preview = MeshInstance3D.new()
	preview.mesh = _ring
	preview.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	preview.top_level = true
	preview.visible = false
	_ring_material = StandardMaterial3D.new()
	_ring_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_ring_material.albedo_color = PLANT_RING_COLOR
	# This is a cursor footprint, so uneven ground must not hide half the ring.
	_ring_material.no_depth_test = true
	preview.material_override = _ring_material
	add_child(preview)

func _process(_delta: float) -> void:
	preview.visible = false
	# Losing the tool mid-drag must not leave a stroke armed for the next activation.
	if not active:
		_painting = false
	if not active or get_viewport().gui_get_hovered_control() != null:
		return
	var hit = _mouse_world_pos()
	if hit == null:
		return
	if radius != _preview_radius:
		_preview_radius = radius
		_ring.outer_radius = radius
		_ring.inner_radius = maxf(0.05, radius - 0.3)
	# Colour carries the mode, because the panel has no space to and the cursor is where the
	# player is looking when it matters.
	if int(mode) != _preview_mode:
		_preview_mode = int(mode)
		_ring_material.albedo_color = (
			REMOVE_RING_COLOR if mode == Mode.REMOVE else PLANT_RING_COLOR
		)
	preview.global_position = hit + Vector3.UP * 0.15
	preview.visible = true

func _unhandled_input(event: InputEvent) -> void:
	if not active:
		return
	if event is InputEventMouseButton and event.pressed and event.ctrl_pressed:
		var step := 0.0
		if event.button_index == MOUSE_BUTTON_WHEEL_UP:
			step = RADIUS_STEP
		elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			step = 1.0 / RADIUS_STEP
		if step != 0.0:
			# Clamping at the minimum is what makes the point edit reachable again after any
			# number of steps up, since a geometric walk never lands back on it exactly.
			radius = clampf(radius * step, MIN_RADIUS_M, MAX_RADIUS_M)
			get_viewport().set_input_as_handled()
			return
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_painting = mode == Mode.REMOVE or radius > MIN_RADIUS_M
			_last_stamp = Vector2.INF
			if _stamp():
				get_viewport().set_input_as_handled()
		elif _painting:
			_painting = false
			get_viewport().set_input_as_handled()
	# Held-button motion continues the stroke, which is what makes a raised radius a brush rather
	# than a stamp. Restamping only after half a radius of travel keeps a slow drag from issuing
	# one native call per pixel; the discs still overlap, so the stroke has no gaps.
	elif event is InputEventMouseMotion and _painting:
		_stamp()

# Applies one stamp at the cursor. Returns whether the surface was hit at all, since a click on
# sky must not be swallowed from the camera.
func _stamp() -> bool:
	var hit = _mouse_world_pos()
	if hit == null:
		return false
	var pos := Vector2(hit.x, hit.z)
	if _painting:
		if _last_stamp != Vector2.INF and pos.distance_to(_last_stamp) < radius * 0.5:
			return true
		_last_stamp = pos
	apply_at(pos)
	return true

## Applies one stamp of the active mode and returns how many plants Rust actually changed.
func apply_at(pos: Vector2) -> int:
	if mode == Mode.REMOVE:
		return simulation_node.remove_vegetation_at(pos, radius)
	if radius > MIN_RADIUS_M:
		return simulation_node.paint_vegetation(pos, radius, species)
	return int(simulation_node.add_vegetation_at(pos, species))

func _mouse_world_pos() -> Variant:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return null
	var mouse := get_viewport().get_mouse_position()
	return simulation_node.intersect_world_surface(
		camera.project_ray_origin(mouse), camera.project_ray_normal(mouse)
	)
