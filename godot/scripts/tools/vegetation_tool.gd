# SPDX-License-Identifier: GPL-3.0-or-later

## Player vegetation input, ground disc and point planting ghost. Rust decides placement.
## Rust methods called: add_vegetation_pick_at(), remove_vegetation_at(), paint_vegetation(),
## plant_vegetation_line(), snap_vegetation_line_end(), intersect_world_surface(),
## hover_world_surface(), preview_vegetation_pick_at(), get_vegetation_point_revision().
extends Node3D

enum Mode { PLANT, REMOVE }

signal option_changed(index: int)
signal mode_changed(value: Mode)

const UIStyle = preload("res://scripts/ui/ui_style.gd")
const TreeSpecies = preload("res://scripts/renderers/tree_species.gd")
const VegetationPlacements = preload("res://scripts/renderers/vegetation_placements.gd")
# AddRefusal in vegetation_api/placement.rs.
const POINT_REFUSALS := {
	1: "Plant option unavailable",
	2: "Outside the map",
	3: "Ground is underwater",
	4: "Ground is too uneven",
	5: "Blocked by a road, paving, building or field",
	6: "Too close to a path or street light",
	7: "Too close to another plant",
}
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
# "group" is the terrain menu's category the option is listed under (`terrain_panel.gd`), and
# "detail" the tooltip line a mix gets, saying what it plants.
#
# "ground" marks the presets Rust bounds to a 64 m brush. "line" marks the hedges: a press sets
# one end of the row, the release the other, and the radius plays no part, so it neither shows
# nor steps while one is selected.
const BRUSH_OPTIONS := [
	{"label": "Pine", "preset": 4, "group": "trees"},
	{"label": "Spruce", "preset": 5, "group": "trees"},
	{"label": "Birch", "preset": 6, "group": "trees"},
	{"label": "Aspen", "preset": 7, "group": "trees"},
	{"label": "Mixed forest", "preset": 8, "group": "trees", "detail": "Pine-led, with spruce, birch and some aspen"},
	{"label": "Scattered trees", "preset": 9, "group": "trees", "detail": "Broadleaf-led trees standing apart"},
	{"label": "Young forest", "preset": 10, "group": "trees", "detail": "Small birch and pine"},
	{"label": "Lilac", "preset": 11, "group": "shrubs", "ground": true},
	{"label": "Spirea", "preset": 12, "group": "shrubs", "ground": true},
	{"label": "Rugosa rose", "preset": 13, "group": "shrubs", "ground": true},
	{"label": "Cotoneaster", "preset": 14, "group": "shrubs", "ground": true},
	{"label": "Mountain pine", "preset": 15, "group": "shrubs", "ground": true},
	{"label": "Juniper", "preset": 16, "group": "shrubs", "ground": true},
	{"label": "Bush", "preset": 2, "group": "shrubs", "ground": true},
	{"label": "Peony", "preset": 20, "group": "flowers", "ground": true},
	{"label": "Lupine", "preset": 21, "group": "flowers", "ground": true},
	{"label": "Phlox", "preset": 22, "group": "flowers", "ground": true},
	{"label": "Hydrangea", "preset": 23, "group": "flowers", "ground": true},
	{"label": "Rhododendron", "preset": 24, "group": "flowers", "ground": true},
	{"label": "Summer flowers", "preset": 25, "group": "flowers", "ground": true},
	{"label": "Hedge, low", "preset": 17, "group": "hedges", "line": true},
	{"label": "Hedge, medium", "preset": 18, "group": "hedges", "line": true},
	{"label": "Hedge, tall", "preset": 19, "group": "hedges", "line": true},
	{"label": "Rock", "preset": 3, "group": "rocks"},
]
const LABEL_HOLD_SECONDS := 1.0
const LABEL_FADE_SECONDS := 0.25
# Clearance between the footprint's top on screen and the bottom of the text, in pixels.
const LABEL_OFFSET_PX := 24.0

# The radius chooses between a point edit and a brush: at
# the minimum a plant lands exactly under the cursor, and above it the disc scatters spaced plants.
# The ground disc is therefore the mode readout as well as the footprint.
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
# Disc that marks where a hedge end will land, in place of the brush footprint.
const LINE_CURSOR_RADIUS_M := 0.5
const REMOVE_RING_COLOR := Color(0.95, 0.45, 0.35)

# Zero-based steps match LAST_SIZE_STEP in vegetation_api/placement.rs.
const POINT_SIZE_STEPS := 5
const SIZE_BAR_WIDTH_PX := 3
const SIZE_BAR_PITCH_PX := 6
const SIZE_BAR_HEIGHT_PX := 17
# An odd stride cycles the 32-bit pick space. Rust hashes the pick into each appearance lane.
const POINT_PICK_STRIDE := 0x9E3779B9
const POINT_SWAY_LIMIT := deg_to_rad(6.0)
const POINT_SWAY_GAIN := 0.005 # Radians per metre/second of horizontal cursor motion.
# Natural frequency 2.5 Hz and damping ratio 0.85: one small overshoot, no wobble.
const POINT_SWAY_DECAY := 2.5 * TAU * 0.85
const POINT_SWAY_FREQUENCY := 2.5 * TAU * sqrt(1.0 - 0.85 * 0.85)

@onready var simulation_node = $"../SimulationNode"
var active := false:
	set(value):
		active = value
		if not active:
			_hide_brush_preview()
var mode: Mode = Mode.PLANT:
	set(value):
		if mode == value:
			return
		mode = value
		_hide_point_preview()
		radius = minf(radius, _paint_radius_limit() if mode == Mode.PLANT else MAX_RADIUS_M)
		_show_option_label()
		mode_changed.emit(mode)
var option_index := 0:
	set(value):
		value = posmod(value, BRUSH_OPTIONS.size())
		if option_index == value:
			return
		option_index = value
		_advance_point_pick()
		radius = radius
		_show_option_label()
		option_changed.emit(option_index)
var preset: int:
	get:
		return BRUSH_OPTIONS[option_index].preset
var radius := MIN_RADIUS_M:
	set(value):
		radius = minf(value, _paint_radius_limit() if mode == Mode.PLANT else MAX_RADIUS_M)
		if radius > MIN_RADIUS_M:
			_hide_point_preview()
		if _option_label != null:
			_set_option_label_text()
var _preview_visible := false
var _preview_footprint := Vector3.INF
var _preview_mode := -1
var _option_label: Label3D
var _size_indicator: Sprite3D
var _size_indicator_textures: Array[ImageTexture] = []
var _label_time_left := 0.0
var _point_ghost: MultiMeshInstance3D
var _point_placements: VegetationPlacements
var _point_pos := Vector2.INF
var _point_epoch := -1
var _point_reason := ""
var _point_pick := 1
var _point_size_step := 2
var _point_preview_step := -1
var _point_model_pick := -1
var _point_model_step := -1
var _point_model_tint := Color.WHITE
# Whether the ghost's instance colour carries the blocked tint. Hiding the preview clears the
# reason but not the colour, so a refused ghost shown again on clear ground must still repaint.
var _point_tinted_blocked := false
var _point_name := ""
var _point_height_text := ""
var _point_root := Vector3.ZERO
var _point_sway_pos := Vector2.INF
var _point_tilt := Vector2.ZERO
var _point_tilt_velocity := Vector2.ZERO
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
# Where the disc sits while the pointer is on a popup instead of on the ground.
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

func _is_point_planting() -> bool:
	return mode == Mode.PLANT and radius <= MIN_RADIUS_M and not is_line_option()

func _ready() -> void:
	var line_material := StandardMaterial3D.new()
	line_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	line_material.albedo_color = PLANT_RING_COLOR
	line_material.no_depth_test = true
	_line_bar = BoxMesh.new()
	_line_preview = MeshInstance3D.new()
	_line_preview.mesh = _line_bar
	_line_preview.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_line_preview.top_level = true
	_line_preview.visible = false
	_line_preview.material_override = line_material
	add_child(_line_preview)
	_option_label = Label3D.new()
	_option_label.billboard = BaseMaterial3D.BILLBOARD_ENABLED
	_option_label.no_depth_test = true
	_option_label.font = ThemeDB.fallback_font
	_option_label.font_size = 32
	_option_label.pixel_size = 0.01
	_option_label.outline_size = 8
	_option_label.outline_modulate = UIStyle.BG_DARK
	# Sort the label above the hedge bar and the text above its own outline.
	_option_label.render_priority = 2
	_option_label.outline_render_priority = 1
	_option_label.top_level = true
	_option_label.visible = false
	add_child(_option_label)
	_build_size_indicator()

func _build_size_indicator() -> void:
	# Five tiny prebuilt frames keep the bars on the same pixel grid as the billboard text.
	var width := (POINT_SIZE_STEPS - 1) * SIZE_BAR_PITCH_PX + SIZE_BAR_WIDTH_PX + 2
	var border := Color(UIStyle.BG_DARK, 1.0)
	for step in range(POINT_SIZE_STEPS):
		var image := Image.create_empty(width, SIZE_BAR_HEIGHT_PX + 2, false, Image.FORMAT_RGBA8)
		for bar in range(POINT_SIZE_STEPS):
			var height := SIZE_BAR_HEIGHT_PX - (POINT_SIZE_STEPS - 1 - bar) * 3
			var rect := Rect2i(1 + bar * SIZE_BAR_PITCH_PX, 1 + SIZE_BAR_HEIGHT_PX - height,
				SIZE_BAR_WIDTH_PX, height)
			image.fill_rect(rect.grow(1), border)
			image.fill_rect(rect, UIStyle.TEXT_PRIMARY if bar == step else UIStyle.TEXT_DIM.darkened(0.35))
		_size_indicator_textures.append(ImageTexture.create_from_image(image))
	_size_indicator = Sprite3D.new()
	_size_indicator.billboard = _option_label.billboard
	_size_indicator.no_depth_test = true
	_size_indicator.pixel_size = _option_label.pixel_size
	_size_indicator.texture_filter = BaseMaterial3D.TEXTURE_FILTER_NEAREST
	_size_indicator.render_priority = _option_label.render_priority
	_size_indicator.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_size_indicator.hide()
	_option_label.add_child(_size_indicator)

func _exit_tree() -> void:
	_hide_brush_preview()

func _process(delta: float) -> void:
	# The latch advances before any early return: the frames a popup holds the grab are exactly
	# the frames this function would otherwise leave through one of them.
	var popup_open := not get_viewport().get_embedded_subwindows().is_empty()
	if popup_open:
		_menu_grab_frames = MENU_DISMISS_FRAMES
	elif _menu_grab_frames > 0:
		_menu_grab_frames -= 1
	_line_preview.visible = false
	_option_label.visible = false
	_label_time_left = maxf(0.0, _label_time_left - delta)
	# Losing the tool mid-drag must not leave a stroke armed for the next activation.
	if not active:
		_painting = false
		_line_start = Vector3.INF
	if not active or (not popup_open and get_viewport().gui_get_hovered_control() != null):
		_hide_brush_preview()
		return
	# With the species popup open the pointer is over the menu, so the ray lands wherever the
	# menu happens to sit rather than where the player was pointing. The disc holds its last
	# ground position instead, because resizing the brush from an open menu is blind otherwise
	# and being able to resize from there is the point.
	if _menu_grab_frames > 0:
		if _last_hit == Vector3.INF:
			_hide_brush_preview()
			return
	else:
		var probe = _mouse_world_pos()
		if probe == null:
			_hide_brush_preview()
			return
		# While the simulation thread holds the core the disc keeps its last ground position.
		if probe is Vector3:
			_last_hit = probe
		elif _last_hit == Vector3.INF:
			_hide_brush_preview()
			return
	var hit := _last_hit
	var line := is_line_option()
	# A hedge end lands on a hedge it is drawn near, so the cursor shows it there.
	if line:
		hit = _line_end(hit)
	var preview_radius := LINE_CURSOR_RADIUS_M if line else radius
	_show_brush_preview(hit, preview_radius)
	if mode == Mode.PLANT and not line and radius <= MIN_RADIUS_M and _menu_grab_frames == 0:
		_update_point_preview(Vector2(hit.x, hit.z), delta)
	else:
		_hide_point_preview()
	if _line_start != Vector3.INF:
		_show_line(_line_start, hit)
	_update_option_label(hit + Vector3.UP * 0.15, preview_radius)
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
	if event is InputEventKey and event.pressed and _menu_grab_frames == 0 and _is_point_planting():
		if event.keycode == KEY_UP or event.keycode == KEY_DOWN:
			if not event.echo:
				var direction := 1 if event.keycode == KEY_UP else -1
				_point_size_step = clampi(_point_size_step + direction, 0, POINT_SIZE_STEPS - 1)
				_show_option_label()
			get_viewport().set_input_as_handled()
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
				var start = _mouse_world_pos(true)
				if start != null:
					_line_start = _line_end(start)
					get_viewport().set_input_as_handled()
				return
			_painting = mode == Mode.REMOVE or radius > MIN_RADIUS_M
			_last_stamp = Vector2.INF
			_hold_left = HOLD_REPEAT_S
			if _stamp(true):
				get_viewport().set_input_as_handled()
		elif _line_start != Vector3.INF:
			var end = _mouse_world_pos(true)
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
	_set_option_label_text()
	_option_label.transparency = 0.0
	_label_time_left = LABEL_HOLD_SECONDS + LABEL_FADE_SECONDS

func _set_option_label_text() -> void:
	var show_size := false
	if not _point_reason.is_empty():
		_option_label.text = _point_reason
	elif _is_point_planting():
		show_size = _point_model_pick == _point_pick
		_option_label.text = "%s %s m" % [_point_name, _point_height_text] if show_size else ""
	else:
		_option_label.text = "Remove" if mode == Mode.REMOVE else BRUSH_OPTIONS[option_index].label
	_option_label.modulate = UIStyle.TEXT_ALERT if mode == Mode.REMOVE or not _point_reason.is_empty() else UIStyle.TEXT_PRIMARY
	_size_indicator.visible = show_size
	if show_size:
		_size_indicator.texture = _size_indicator_textures[_point_model_step]
		var width := _option_label.font.get_string_size(_option_label.text,
			HORIZONTAL_ALIGNMENT_LEFT, -1, _option_label.font_size).x
		# Offset within the billboard, so both stay registered as the camera turns or zooms.
		_size_indicator.offset.x = ceilf(width * 0.5) + 9.0 + _size_indicator.texture.get_width() * 0.5

func _update_option_label(anchor: Vector3, preview_radius: float) -> void:
	var blocked := not _point_reason.is_empty()
	if _option_label.text.is_empty() or (not blocked and _label_time_left <= 0.0):
		return
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return
	# Project a pixel at the disc's depth: size and separation stay constant on screen for
	# either camera projection, independent of brush radius. The held preview position also
	# keeps this text anchored while a popup owns the pointer.
	var depth := -camera.to_local(anchor).z
	if depth <= 0.0:
		return
	var screen := camera.unproject_position(anchor)
	var above := camera.project_position(screen - Vector2(0.0, 1.0), depth)
	var metres_per_px := anchor.distance_to(above)
	# Sample the far edge on the published terrain so the label clears an uphill footprint.
	var away := anchor - camera.global_position
	away.y = 0.0
	var reach := 0.0
	if away.length_squared() > 0.0:
		var far_edge := anchor + away.normalized() * preview_radius
		far_edge.y = maxf(far_edge.y, simulation_node.get_height_at(Vector2(far_edge.x, far_edge.z)))
		reach = maxf(0.0, screen.y - camera.unproject_position(far_edge).y)
	_option_label.global_position = (anchor
		+ camera.global_basis.y * (reach + LABEL_OFFSET_PX) * metres_per_px)
	_option_label.scale = Vector3.ONE * (metres_per_px / _option_label.pixel_size)
	var opacity := 1.0 if blocked else minf(1.0, _label_time_left / LABEL_FADE_SECONDS)
	_option_label.transparency = 1.0 - opacity
	if _size_indicator.visible:
		_size_indicator.modulate.a = opacity
	_option_label.visible = true

## Steps the radius one geometric notch, up for a positive direction. Public so that a control
## holding the input grab (a popup) can forward ctrl and the wheel, which then never reach
## `_unhandled_input`.
func step_radius(direction: int) -> void:
	if is_line_option():
		return
	# Clamping at the minimum is what makes the point edit reachable again after any number of
	# steps up, since a geometric walk never lands back on it exactly.
	radius = clampf(
		radius * (RADIUS_STEP if direction > 0 else 1.0 / RADIUS_STEP),
		MIN_RADIUS_M,
		MAX_RADIUS_M
	)

# Applies one stamp at the cursor. Returns whether the surface was hit at all, since a click on
# sky must not be swallowed from the camera. A click's stamp waits for the core; a held drag's
# stamps skip the frames the simulation thread holds it.
func _stamp(click := false) -> bool:
	var hit = _mouse_world_pos(click)
	if not (hit is Vector3):
		return false
	# A busy hover may still show the previous root or size. The click uses that visible answer.
	if click and _is_point_planting() and _point_ghost != null and _point_ghost.visible:
		_apply_point(_point_pos, _point_preview_step)
		return true
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
	return _apply_point(pos, _point_size_step)

func _apply_point(pos: Vector2, step: int) -> int:
	if not simulation_node.add_vegetation_pick_at(pos, preset, _point_pick, step):
		return 0
	_advance_point_pick()
	return 1

func _advance_point_pick() -> void:
	_point_pick = (_point_pick + POINT_PICK_STRIDE) & 0xFFFFFFFF
	_hide_point_preview()
	if _option_label != null and _is_point_planting():
		_option_label.hide()
		_set_option_label_text()

## Lays the selected hedge from `from` to `to` and returns how many modules Rust planted. Rust
## joins either end to a hedge already standing near it.
func apply_line(from: Vector2, to: Vector2, stroke := 0) -> int:
	return simulation_node.plant_vegetation_line(from, to, preset, stroke)

# Where Rust will put a hedge end drawn at `hit`, at the ground height under the cursor.
func _line_end(hit: Vector3) -> Vector3:
	var end: Vector2 = simulation_node.snap_vegetation_line_end(Vector2(hit.x, hit.z))
	return Vector3(end.x, hit.y, end.y)

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

func _show_brush_preview(hit: Vector3, preview_radius: float) -> void:
	var footprint := Vector3(hit.x, hit.z, preview_radius)
	if footprint != _preview_footprint:
		_preview_footprint = footprint
		RenderingServer.global_shader_parameter_set("vegetation_brush_footprint", footprint)
	if int(mode) != _preview_mode:
		_preview_mode = int(mode)
		var color := REMOVE_RING_COLOR if mode == Mode.REMOVE else PLANT_RING_COLOR
		RenderingServer.global_shader_parameter_set("vegetation_brush_color", color)
	if not _preview_visible:
		_preview_visible = true
		RenderingServer.global_shader_parameter_set("vegetation_brush_enabled", true)

func _hide_brush_preview() -> void:
	_hide_point_preview()
	if _preview_visible:
		_preview_visible = false
		RenderingServer.global_shader_parameter_set("vegetation_brush_enabled", false)

func _update_point_preview(pos: Vector2, delta := 0.0) -> void:
	var epoch: int = simulation_node.get_vegetation_point_revision()
	var previous_root := _point_root
	# Even a move within one cell can cross a clearance or occupancy boundary.
	if pos != _point_pos or epoch != _point_epoch or _point_size_step != _point_preview_step:
		var result = simulation_node.preview_vegetation_pick_at(pos, preset, _point_pick,
			_point_size_step)
		if not result is bool:
			_point_pos = pos
			_point_epoch = epoch
			_point_preview_step = _point_size_step
			_show_point_result(result)
		# A busy result leaves the last ghost visible and the query outstanding.
	if _point_ghost != null and _point_ghost.visible and (
			previous_root != _point_root
			or not _point_sway_pos.is_finite() or _point_tilt != Vector2.ZERO
			or _point_tilt_velocity != Vector2.ZERO):
		_sway_point(delta)

func _show_point_result(result: Variant) -> void:
	var has_plant := result is PackedFloat32Array
	var refusal := 0
	if not has_plant:
		refusal = int(result)
	elif result.size() == 7:
		refusal = int(result[6])
	var reason: String = POINT_REFUSALS.get(refusal, "")
	var repaint := reason != _point_reason or (refusal != 0) != _point_tinted_blocked
	_point_reason = reason
	if not has_plant:
		if repaint:
			_set_option_label_text()
		if _point_ghost != null:
			_point_ghost.hide()
		_reset_point_sway()
		return
	if _point_ghost == null:
		var world_renderer = get_node_or_null("../Vegetation")
		var meshes: Array = world_renderer.meshes if world_renderer != null else TreeSpecies.build_meshes()
		_point_placements = VegetationPlacements.new(meshes)
		_point_ghost = MultiMeshInstance3D.new()
		_point_ghost.top_level = true
		_point_ghost.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		_point_ghost.transparency = 0.55
		_point_ghost.multimesh = MultiMesh.new()
		_point_ghost.multimesh.transform_format = MultiMesh.TRANSFORM_3D
		_point_ghost.multimesh.use_colors = true
		add_child(_point_ghost)
	var mm := _point_ghost.multimesh
	if _point_model_pick != _point_pick or _point_model_step != _point_preview_step:
		_point_model_tint = _point_placements.set_ghost_plant(mm, result)
		_point_model_pick = _point_pick
		_point_model_step = _point_preview_step
		var packed := int(result[5])
		_point_name = _held_plant_name(packed)
		var height: float = mm.mesh.get_aabb().size.y * result[4]
		var decimal := height < 10.0
		if (packed & VegetationPlacements.Vegetation.SPECIES_MASK) < TreeSpecies.BUSH:
			# Preserve a model-limited fractional step and discard mesh/scale rounding noise.
			height = snappedf(height, 0.1)
			decimal = height < 10.0 or not is_equal_approx(height, roundf(height))
		_point_height_text = ("%.1f" if decimal else "%.0f") % height
		repaint = true
	if repaint:
		# Instance colour preserves cutouts and tints both bark and foliage.
		mm.set_instance_color(0, _point_model_tint * VegetationPlacements.BLOCKED_TINT
			if refusal != 0 else _point_model_tint)
		_point_tinted_blocked = refusal != 0
		_set_option_label_text()
	_point_root = Vector3(result[0], result[1], result[2])
	_point_ghost.show()

func _held_plant_name(packed: int) -> String:
	var species := packed & VegetationPlacements.Vegetation.SPECIES_MASK
	if species < TreeSpecies.BUSH:
		var variant := VegetationPlacements.variant_pin(packed) - 1
		return TreeSpecies.canopy_model(species, variant).get_slice("_", 0).capitalize()
	return BRUSH_OPTIONS[option_index].label

func _sway_point(delta: float) -> void:
	var pos := Vector2(_point_root.x, _point_root.z)
	if delta > 0.0 and _point_sway_pos.is_finite() and (
			pos != _point_sway_pos or _point_tilt != Vector2.ZERO
			or _point_tilt_velocity != Vector2.ZERO):
		var target := ((_point_sway_pos - pos) * (POINT_SWAY_GAIN / delta)).limit_length(POINT_SWAY_LIMIT)
		# Exact underdamped spring update for this frame's velocity, held constant over delta.
		var offset := _point_tilt - target
		var tangent := (_point_tilt_velocity + POINT_SWAY_DECAY * offset) / POINT_SWAY_FREQUENCY
		var phase := POINT_SWAY_FREQUENCY * delta
		var cosine := cos(phase)
		var sine := sin(phase)
		var envelope := exp(-POINT_SWAY_DECAY * delta)
		var wave := offset * cosine + tangent * sine
		_point_tilt = target + envelope * wave
		_point_tilt_velocity = envelope * (POINT_SWAY_FREQUENCY *
			(tangent * cosine - offset * sine) - POINT_SWAY_DECAY * wave)
		if _point_tilt.length_squared() > POINT_SWAY_LIMIT * POINT_SWAY_LIMIT:
			var outward := _point_tilt.normalized()
			_point_tilt = outward * POINT_SWAY_LIMIT
			_point_tilt_velocity -= outward * maxf(0.0, outward.dot(_point_tilt_velocity))
		if (target == Vector2.ZERO and _point_tilt.length_squared() < 1e-8
				and _point_tilt_velocity.length_squared() < 1e-6):
			_point_tilt = Vector2.ZERO
			_point_tilt_velocity = Vector2.ZERO
	_point_sway_pos = pos
	var tilt := Basis.IDENTITY
	if _point_tilt != Vector2.ZERO:
		# The rotation axis is up × tilt. The MultiMesh's plant starts at the node's origin.
		var axis := Vector3(_point_tilt.y, 0.0, -_point_tilt.x).normalized()
		tilt = Basis(axis, _point_tilt.length())
	_point_ghost.global_transform = Transform3D(tilt, _point_root)

func _reset_point_sway() -> void:
	_point_sway_pos = Vector2.INF
	_point_tilt = Vector2.ZERO
	_point_tilt_velocity = Vector2.ZERO

func _hide_point_preview() -> void:
	_point_pos = Vector2.INF
	_reset_point_sway()
	if _point_ghost != null:
		_point_ghost.hide()
	if not _point_reason.is_empty():
		_point_reason = ""
		_set_option_label_text()
		_option_label.hide()

# The ground point under the pointer, null for none. A click's query waits for the core
# (`CODE-25`); the others answer false while the simulation thread holds it.
func _mouse_world_pos(click := false) -> Variant:
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return null
	var mouse := get_viewport().get_mouse_position()
	var origin := camera.project_ray_origin(mouse)
	var direction := camera.project_ray_normal(mouse)
	if click:
		return simulation_node.intersect_world_surface(origin, direction, true)
	return simulation_node.hover_world_surface(origin, direction)
