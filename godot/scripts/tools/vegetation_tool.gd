# SPDX-License-Identifier: GPL-2.0-only

## Player vegetation input and ground-ring preview; all placement decisions stay in Rust.
## Rust methods called: add_vegetation_at(), remove_vegetation_at(), paint_vegetation(),
## plant_vegetation_line(), intersect_world_surface().
extends Node3D

enum Mode { PLANT, REMOVE }

signal option_changed(index: int)
signal mode_changed(value: Mode)

const UIStyle = preload("res://scripts/ui/ui_style.gd")
# Display order, which is not the preset table's order: `preset` indexes the table in
# vegetation_api/brush.rs, which owns every decision about what a stroke plants. Rust decides
# the species mix, the fraction of the lattice kept and the scale band; this list only names
# the presets for the player and fixes the order Shift and the wheel cycle them in.
#
# The table's unpinned conifer and broadleaf are deliberately absent. They are what the
# generator plants and what a cleared cell regrows, not an offering: with pine and spruce both
# nameable, "conifer" is only an unnamed two-to-one mix of them, and a mix belongs in a mix
# preset where its ratio is written down.
#
# "ground" marks the presets Rust bounds to a 64 m brush. "line" marks the hedges: a press sets
# one end of the row, the release the other, and the radius plays no part.
const BRUSH_OPTIONS := [
	{"label": "Pine", "preset": 4},
	{"label": "Spruce", "preset": 5},
	{"label": "Birch", "preset": 6},
	{"label": "Aspen", "preset": 7},
	{"label": "Lilac", "preset": 11, "ground": true},
	{"label": "Spirea", "preset": 12, "ground": true},
	{"label": "Rugosa rose", "preset": 13, "ground": true},
	{"label": "Cotoneaster", "preset": 14, "ground": true},
	{"label": "Mountain pine", "preset": 15, "ground": true},
	{"label": "Juniper", "preset": 16, "ground": true},
	{"label": "Hedge, low", "preset": 17, "line": true},
	{"label": "Hedge, medium", "preset": 18, "line": true},
	{"label": "Hedge, tall", "preset": 19, "line": true},
	{"label": "Bush", "preset": 2, "ground": true},
	{"label": "Rock", "preset": 3},
	{"label": "Mixed forest 1", "preset": 8},
	{"label": "Mixed forest 2", "preset": 9},
	{"label": "Mixed forest 3", "preset": 10},
]
const LABEL_HOLD_SECONDS := 1.0
const LABEL_FADE_SECONDS := 0.25
# Clearance between the ring's highest point on screen and the bottom of the text, in pixels.
const LABEL_OFFSET_PX := 24.0

# The radius is the only size control, and it also chooses between a point edit and a brush: at
# the minimum a plant lands exactly under the cursor, and above it the disc scatters spaced plants.
# The ground ring is therefore the mode readout as well as the footprint.
const MIN_RADIUS_M := 1.0
const MAX_RADIUS_M := 1024.0
# Matches the native paint bound; removal retains its 1 km radius.
const MAX_PAINT_RADIUS_M := 256.0
# Geometric, because a fixed step cannot serve both a one-tree cursor and a 1 km clear-cut.
const RADIUS_STEP := 1.4
# A held plant brush restamps in place this often. Each stamp adds Rust's thin share of the
# preset, 10 stems/ha for trees, so holding thickens a stand at about 100 stems/ha a second.
const HOLD_REPEAT_S := 0.1
# Frames the tool keeps ignoring a stamp after an embedded popup last held the input grab. The
# preset dropdown takes that grab, and the click that dismisses it is delivered here as well, so
# one click used to close the menu and plant a tree. The tool cannot ask the popup directly
# without reaching into the UI layer, so it watches the viewport's own subwindow list instead.
# Two frames covers the dismissal and stays far under the interval a player can click again in.
const MENU_DISMISS_FRAMES := 2

const PLANT_RING_COLOR := Color(0.85, 0.95, 0.45)
# Cross-section of the bar that previews a hedge row on the ground, in metres.
const LINE_PREVIEW_SIZE := Vector2(0.25, 0.6)
const REMOVE_RING_COLOR := Color(0.95, 0.45, 0.35)

# Ring stroke, in pixels. A fixed world-space stroke is honest in metres and useless on screen: at
# the former 0.3 m it covered two pixels at 100 m and a quarter of one at 900 m, so the cursor
# faded out exactly where a large brush needs it most. The radius stays a true world measurement,
# because it is the footprint the stamp will cover; only the stroke follows the screen.
const RING_STROKE_PX := 3.0
# Bounds on what that resolves to in world metres. The lower bound is the former fixed stroke and
# keeps the torus from degenerating at the point-edit radius. The upper keeps a 256 m brush
# reading as a ring rather than filling in as a disc.
const RING_STROKE_MIN_M := 0.3
const RING_STROKE_MAX_RATIO := 0.06

@onready var simulation_node = $"../SimulationNode"
var active := false
var mode: Mode = Mode.PLANT:
	set(value):
		if mode == value:
			return
		mode = value
		radius = minf(radius, _paint_radius_limit() if mode == Mode.PLANT else MAX_RADIUS_M)
		_show_option_label()
		mode_changed.emit(mode)
var option_index := 0:
	set(value):
		value = posmod(value, BRUSH_OPTIONS.size())
		if option_index == value:
			return
		option_index = value
		radius = radius
		_show_option_label()
		option_changed.emit(option_index)
var preset: int:
	get:
		return BRUSH_OPTIONS[option_index].preset
var radius := MIN_RADIUS_M:
	set(value):
		radius = minf(value, _paint_radius_limit() if mode == Mode.PLANT else MAX_RADIUS_M)
var preview: MeshInstance3D
var _ring: TorusMesh
var _ring_material: StandardMaterial3D
var _preview_radius := -1.0
var _preview_stroke := -1.0
var _preview_mode := -1
var _option_label: Label3D
var _label_time_left := 0.0
var _painting := false
# Identity of the gesture in progress, minted on the press that opens it. Rust folds stamps
# into one undo entry by this id rather than by stamp order: a stamp that changes nothing
# records nothing, so "not the first stamp" cannot prove which gesture the entry on top of
# the history belongs to. Zero is reserved for a standalone edit.
var _stroke := 0
var _last_stamp := Vector2.INF
var _hold_left := 0.0
# Where a stamp last added nothing. A full stand stays full, so a hold does not repeat there:
# the largest tree stamp costs about 22 ms even when it adds nothing.
var _idle_stamp := Vector2.INF
# Where the ring sits while the pointer is on a popup instead of on the ground.
var _last_hit := Vector3.INF
var _menu_grab_frames := 0
# Unspent part of a two-finger scroll, in the gesture's own units. One unit is one step.
var _gesture_steps := 0.0
# Ground position a hedge row starts from while the button is held, or INF with no row open.
var _line_start := Vector3.INF
var _line_preview: MeshInstance3D
var _line_bar: BoxMesh

# This mirrors the native class budget; the preview must show the accepted footprint.
func _paint_radius_limit() -> float:
	return 64.0 if BRUSH_OPTIONS[option_index].get("ground", false) else MAX_PAINT_RADIUS_M

## Whether the selected option lays hedge rows rather than stamping plants.
func is_line_option() -> bool:
	return mode == Mode.PLANT and BRUSH_OPTIONS[option_index].get("line", false)

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
	_line_bar = BoxMesh.new()
	_line_preview = MeshInstance3D.new()
	_line_preview.mesh = _line_bar
	_line_preview.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_line_preview.top_level = true
	_line_preview.visible = false
	_line_preview.material_override = _ring_material
	add_child(_line_preview)
	_option_label = Label3D.new()
	_option_label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	_option_label.no_depth_test = true
	_option_label.font_size = 32
	_option_label.pixel_size = 0.01
	_option_label.outline_size = 8
	_option_label.outline_modulate = UIStyle.BG_DARK
	# Both the ring and this text skip the depth test, so draw order alone decides which of
	# them survives an overlap. Sort the text and its outline above the ring, and the text
	# above its own outline.
	_option_label.render_priority = 2
	_option_label.outline_render_priority = 1
	_option_label.top_level = true
	_option_label.visible = false
	add_child(_option_label)

func _process(delta: float) -> void:
	# The latch advances before any early return: the frames a popup holds the grab are exactly
	# the frames this function would otherwise leave through one of them.
	if not get_viewport().get_embedded_subwindows().is_empty():
		_menu_grab_frames = MENU_DISMISS_FRAMES
	elif _menu_grab_frames > 0:
		_menu_grab_frames -= 1
	preview.visible = false
	_line_preview.visible = false
	_option_label.visible = false
	_label_time_left = maxf(0.0, _label_time_left - delta)
	# Losing the tool mid-drag must not leave a stroke armed for the next activation.
	if not active:
		_painting = false
		_line_start = Vector3.INF
	if not active or get_viewport().gui_get_hovered_control() != null:
		return
	# With the species popup open the pointer is over the menu, so the ray lands wherever the
	# menu happens to sit rather than where the player was pointing. The ring holds its last
	# ground position instead, because resizing the brush from an open menu is blind otherwise
	# and being able to resize from there is the point.
	if _menu_grab_frames > 0:
		if _last_hit == Vector3.INF:
			return
	else:
		var probe = _mouse_world_pos()
		if probe == null:
			return
		_last_hit = probe
	var hit := _last_hit
	# TorusMesh rebuilds its surface on every property write, so the stroke is only re-applied
	# once it has drifted enough to see. Camera motion otherwise rebuilds 1024 triangles a frame.
	var stroke := _ring_stroke_m(hit)
	if radius != _preview_radius or absf(stroke - _preview_stroke) > _preview_stroke * 0.05:
		_preview_radius = radius
		_preview_stroke = stroke
		_ring.outer_radius = radius
		_ring.inner_radius = maxf(0.05, radius - stroke)
	# Colour carries the mode, because the panel has no space to and the cursor is where the
	# player is looking when it matters.
	if int(mode) != _preview_mode:
		_preview_mode = int(mode)
		_ring_material.albedo_color = (
			REMOVE_RING_COLOR if mode == Mode.REMOVE else PLANT_RING_COLOR
		)
	preview.global_position = hit + Vector3.UP * 0.15
	preview.visible = true
	if _line_start != Vector3.INF:
		_show_line(_line_start, hit)
	_update_option_label()
	# Removal clears the whole disc on its first stamp, so only planting repeats while held.
	if _painting and mode == Mode.PLANT and _menu_grab_frames == 0:
		_hold_left -= delta
		if _hold_left <= 0.0:
			_hold_left = HOLD_REPEAT_S
			if (_idle_stamp == Vector2.INF
					or Vector2(hit.x, hit.z).distance_to(_idle_stamp) > radius * 0.1):
				_last_stamp = Vector2.INF
				_stamp()

func _unhandled_input(event: InputEvent) -> void:
	if not active:
		return
	if apply_brush_gesture(event):
		get_viewport().set_input_as_handled()
		return
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			# This click is the one that dismisses an open popup. Dismissing is all it does.
			if _menu_grab_frames > 0:
				_menu_grab_frames = 0
				get_viewport().set_input_as_handled()
				return
			_stroke += 1
			if is_line_option():
				var start = _mouse_world_pos()
				if start != null:
					_line_start = start
					get_viewport().set_input_as_handled()
				return
			_painting = mode == Mode.REMOVE or radius > MIN_RADIUS_M
			_last_stamp = Vector2.INF
			_hold_left = HOLD_REPEAT_S
			if _stamp():
				get_viewport().set_input_as_handled()
		elif _line_start != Vector3.INF:
			var end = _mouse_world_pos()
			if end != null:
				apply_line(Vector2(_line_start.x, _line_start.z), Vector2(end.x, end.z), _stroke)
			_line_start = Vector3.INF
			get_viewport().set_input_as_handled()
		elif _painting:
			_painting = false
			get_viewport().set_input_as_handled()
	# Held-button motion continues the stroke, which is what makes a raised radius a brush rather
	# than a stamp. Restamping only after half a radius of travel keeps a slow drag from issuing
	# one native call per pixel; the discs still overlap, so the stroke has no gaps.
	elif event is InputEventMouseMotion and _painting:
		_stamp()

## Applies Ctrl (radius) or Shift (option) with a wheel or a two-finger scroll, and reports
## whether the event was one. macOS turns Shift with a vertical scroll into a horizontal one, so
## wheel left and right step like up and down, and a scroll takes its larger axis.
func apply_brush_gesture(event: InputEvent) -> bool:
	if not (event is InputEventWithModifiers and (event.ctrl_pressed or event.shift_pressed)):
		return false
	var direction := 0
	if event is InputEventMouseButton and event.pressed:
		match event.button_index:
			MOUSE_BUTTON_WHEEL_UP, MOUSE_BUTTON_WHEEL_LEFT:
				direction = 1
			MOUSE_BUTTON_WHEEL_DOWN, MOUSE_BUTTON_WHEEL_RIGHT:
				direction = -1
	elif event is InputEventPanGesture:
		var delta: Vector2 = event.delta
		# A scroll arrives in fractions; whole steps are taken as the swipe accumulates.
		_gesture_steps -= delta.y if absf(delta.y) >= absf(delta.x) else delta.x
		direction = int(_gesture_steps)
		_gesture_steps -= direction
		if direction == 0:
			return true
	if direction == 0:
		return false
	if event.ctrl_pressed:
		step_radius(direction)
	else:
		step_option(direction)
	return true

## Cycles the ordered brush options, including when the dropdown holds the input grab.
func step_option(direction: int) -> void:
	option_index += direction

## Switches planting and erasing without changing the selected planting option.
func toggle_mode() -> void:
	mode = Mode.PLANT if mode == Mode.REMOVE else Mode.REMOVE

func _show_option_label() -> void:
	if _option_label == null:
		return
	_option_label.text = "Remove" if mode == Mode.REMOVE else BRUSH_OPTIONS[option_index].label
	_option_label.modulate = UIStyle.TEXT_ALERT if mode == Mode.REMOVE else UIStyle.TEXT_PRIMARY
	_option_label.transparency = 0.0
	_label_time_left = LABEL_HOLD_SECONDS + LABEL_FADE_SECONDS

func _update_option_label() -> void:
	if _label_time_left <= 0.0:
		return
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	# Project a pixel at the ring's depth: size and separation stay constant on screen for
	# either camera projection, independent of brush radius. The held preview position also
	# keeps this text anchored while a popup owns the pointer.
	var anchor := preview.global_position
	var depth := -camera.to_local(anchor).z
	if depth <= 0.0:
		return
	var screen := camera.unproject_position(anchor)
	var above := camera.project_position(screen - Vector2(0.0, 1.0), depth)
	var metres_per_px := anchor.distance_to(above)
	# The ring lies flat on the ground, so how far up the screen it reaches follows the
	# camera's pitch as well as the brush radius: seen from near ground level its far edge
	# climbs far past any fixed offset from the centre, which is what put the text across it.
	# The far edge is the highest point of a ground circle for any camera above the ground.
	var away := anchor - camera.global_position
	away.y = 0.0
	var reach := 0.0
	if away.length_squared() > 0.0:
		var far_edge := anchor + away.normalized() * _preview_radius
		reach = maxf(0.0, screen.y - camera.unproject_position(far_edge).y)
	_option_label.global_position = (anchor
		+ camera.global_basis.y * (reach + LABEL_OFFSET_PX) * metres_per_px)
	_option_label.scale = Vector3.ONE * (metres_per_px / _option_label.pixel_size)
	_option_label.transparency = 1.0 - minf(1.0, _label_time_left / LABEL_FADE_SECONDS)
	_option_label.visible = true

## Steps the radius one geometric notch, up for a positive direction. Public because the species
## dropdown forwards ctrl and the wheel here: an open popup holds the input grab, so those events
## never reach `_unhandled_input`.
func step_radius(direction: int) -> void:
	# Clamping at the minimum is what makes the point edit reachable again after any number of
	# steps up, since a geometric walk never lands back on it exactly.
	radius = clampf(
		radius * (RADIUS_STEP if direction > 0 else 1.0 / RADIUS_STEP),
		MIN_RADIUS_M,
		MAX_RADIUS_M
	)

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
	# A held drag is one action to the player, so all of its stamps carry one stroke id.
	var changed := apply_at(pos, _stroke if _painting else 0)
	_idle_stamp = pos if changed == 0 else Vector2.INF
	return true

## Applies one stamp of the active mode and returns how many plants Rust actually changed.
## `stroke` folds this stamp into the undo entry of the gesture that minted the id; zero
## stands alone.
func apply_at(pos: Vector2, stroke := 0) -> int:
	if mode == Mode.REMOVE:
		return simulation_node.remove_vegetation_at(pos, radius, stroke)
	if radius > MIN_RADIUS_M:
		return simulation_node.paint_vegetation(pos, radius, preset, stroke)
	return int(simulation_node.add_vegetation_at(pos, preset))

## Lays the selected hedge from `from` to `to` and returns how many modules Rust planted.
func apply_line(from: Vector2, to: Vector2, stroke := 0) -> int:
	return simulation_node.plant_vegetation_line(from, to, preset, stroke)

# Stretches the preview bar along the ground from the row's start to the cursor.
func _show_line(start: Vector3, end: Vector3) -> void:
	var along := Vector3(end.x - start.x, 0.0, end.z - start.z)
	var length := along.length()
	if length < 0.01:
		return
	_line_bar.size = Vector3(length, LINE_PREVIEW_SIZE.y, LINE_PREVIEW_SIZE.x)
	_line_preview.global_transform = Transform3D(
		Basis(Vector3.UP, atan2(-along.z, along.x)),
		(start + end) * 0.5 + Vector3.UP * (LINE_PREVIEW_SIZE.y * 0.5))
	_line_preview.visible = true

# World metres that cover RING_STROKE_PX at the cursor. Same projection term as the field edit
# tool's handle scaling, which is the existing idiom for this in the tool layer.
func _ring_stroke_m(hit: Vector3) -> float:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return RING_STROKE_MIN_M
	var height_px := maxf(float(get_viewport().get_visible_rect().size.y), 1.0)
	var metres_per_px := 2.0 * camera.global_position.distance_to(hit) * tan(
		deg_to_rad(camera.fov * 0.5)
	) / height_px
	# The upper bound never falls below the lower one, which it would at the point-edit radius.
	var upper := maxf(RING_STROKE_MIN_M, radius * RING_STROKE_MAX_RATIO)
	return clampf(RING_STROKE_PX * metres_per_px, RING_STROKE_MIN_M, upper)

func _mouse_world_pos() -> Variant:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return null
	var mouse := get_viewport().get_mouse_position()
	return simulation_node.intersect_world_surface(
		camera.project_ray_origin(mouse), camera.project_ray_normal(mouse)
	)
