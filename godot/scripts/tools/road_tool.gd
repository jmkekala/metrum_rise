## Road drawing tool — straight and spline modes with live compiled preview and lane configuration.
##
## Extends NetworkTool. Adds: Rust-compiled roadbed preview, G1 continuity guard at junctions,
## angle snapping (Shift, 15° steps, no road snap), distance + angle HUD label, and SimCity-style ghost guide
## lines projected from existing road endpoints (toggle with G key).
## Commits via NetworkTool.add_road() on left-click after cheap Rust placement validation;
## expensive async road-surface previews are visual feedback only.
## State machine: IDLE → SETTING_CONTROL (spline handle) → SETTING_END → commit → IDLE.
extends "res://scripts/tools/network_tool.gd"

enum State { IDLE, SETTING_CONTROL, SETTING_END }
var current_state = State.IDLE

var start_pos: Vector3
var control_pos: Vector3

var current_path: Path3D

var fwd_lanes: int = 1
var bkw_lanes: int = 1
var lanes_label: Label
var altitude_offset: float = 0.0

var draw_mode: int = 0 # 0: straight, 1: spline

# Border-check positions queued while roads are in-flight on the sim thread.
# Drained by NetworkRenderer._process once the road is confirmed in the graph.
var _pending_border_checks: Array = []

# ── HUD label: distance + angle readout (screen-space, zoom-independent) ────
var _hud_canvas: CanvasLayer = null
var _info_label: Label = null
# World-space preview-label anchor — used each frame to project to screen.
var _label_world_pos: Vector3 = Vector3.ZERO

# ── Ghost guide lines (SimCity-style grid overlay) ───────────────────────────
var _ghost_enabled: bool = true  # toggled with G key
# Cached guide data so we only call Rust when the network changes.
var _ghost_guides_dirty: bool = true
var _ghost_rebuild_queued: bool = false
var _road_debug_enabled: bool = false
var _preview_cache_points: PackedVector3Array = PackedVector3Array()
var _preview_cache_surface: Dictionary = {}
var _preview_cache_fwd_lanes: int = -1
var _preview_cache_bkw_lanes: int = -1
var _preview_cache_snap_to_roads: bool = true
var _preview_request_points: PackedVector3Array = PackedVector3Array()
var _preview_request_fwd_lanes: int = -1
var _preview_request_bkw_lanes: int = -1
var _preview_request_snap_to_roads: bool = true
var _preview_request_id: int = 0
var _preview_drawn_request_id: int = 0
var _preview_update_pending: bool = false
var _preview_result_pending: bool = false
var _preview_idle_exact_delay_sec: float = 0.0
var _preview_exact_waiting: bool = false
var _candidate_cache_points: PackedVector3Array = PackedVector3Array()
var _candidate_cache_validation: Dictionary = {}
var _candidate_cache_fwd_lanes: int = -1
var _candidate_cache_bkw_lanes: int = -1
var _candidate_cache_snap_to_roads: bool = true
var _candidate_cache_surface_generation: int = -1
var _curve_shape_valid: bool = true
var _last_snap_to_roads_enabled: bool = true

const ROAD_PROFILE_SLOW_MS := 50.0
const ROAD_SURFACE_CURVE_STEP_M := 4.0
const ROAD_SURFACE_POINT_EPS_M := 0.05
const ROAD_PREVIEW_EXACT_IDLE_DELAY_SEC := 0.10
const ROAD_PREVIEW_RENDER_OFFSET_M := 0.08
const ROAD_PREVIEW_LANE_WIDTH_M := 3.5
const ROAD_PREVIEW_SIDEWALK_WIDTH_M := 1.5
const ROAD_PREVIEW_MIN_WIDTH_M := 2.0
const MAP_BORDER_SNAP_DIST_M := 25.0
const ROAD_NETWORK_SNAP_CONFIRM_EPS_M := 0.25
const ROAD_NETWORK_SNAP_RELEASE_DIST_M := 8.0
const ROAD_SELF_SNAP_DIST_M := 2.5

# ── Angle-snap reference ─────────────────────────────────────────────────────
# Base angle (radians) for Shift snapping — set to the road tangent at start_pos
# so that 90° snap gives a true perpendicular to the road, not to the world grid.
var _start_tangent_angle: float = 0.0
# True when a real road tangent was found at start_pos (false = open terrain, show world angle).
var _has_road_tangent: bool = false
var _sticky_network_snap_active: bool = false
var _sticky_network_snap_pos: Vector3 = Vector3.ZERO

func _ready():
	super._ready()
	if blueprint_mesh:
		blueprint_mesh.position.y = ROAD_PREVIEW_RENDER_OFFSET_M
	_road_debug_enabled = _road_debug_is_enabled()
	_hud_canvas = CanvasLayer.new()
	_hud_canvas.layer = 10
	add_child(_hud_canvas)

	_info_label = Label.new()
	_info_label.add_theme_font_size_override("font_size", 18)
	_info_label.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0, 0.95))
	_info_label.add_theme_color_override("font_shadow_color", Color(0.0, 0.0, 0.0, 0.8))
	_info_label.add_theme_constant_override("shadow_offset_x", 1)
	_info_label.add_theme_constant_override("shadow_offset_y", 1)
	_info_label.visible = false
	_hud_canvas.add_child(_info_label)

func _process(delta):
	super._process(delta)
	if not active:
		_clear_sticky_network_snap()
	var snap_to_roads := _snap_to_roads_enabled()
	if snap_to_roads != _last_snap_to_roads_enabled:
		_last_snap_to_roads_enabled = snap_to_roads
		if not snap_to_roads:
			_clear_sticky_network_snap()
		_clear_preview_cache()
		if current_path != null:
			_preview_update_pending = true
	_preview_idle_exact_delay_sec = maxf(_preview_idle_exact_delay_sec - delta, 0.0)
	if (
		current_path != null
		and _preview_exact_waiting
		and _preview_idle_exact_delay_sec <= 0.0
		and not _preview_update_pending
	):
		_preview_update_pending = true
	if _preview_update_pending:
		_preview_update_pending = false
		_update_preview()
	elif _preview_result_pending:
		_poll_pending_preview_result()
	# Project the HUD label world position to screen space each frame so the label
	# stays the same pixel size regardless of camera zoom.
	if _info_label and _info_label.visible:
		var camera := get_viewport().get_camera_3d()
		if camera:
			var screen_pos: Vector2 = camera.unproject_position(_label_world_pos)
			# Offset slightly so the label sits above-right of the midpoint.
			_info_label.position = screen_pos + Vector2(8.0, -24.0)

func _road_debug_is_enabled() -> bool:
	var debug_value := OS.get_environment("METRUM_DEBUG").strip_edges()
	if debug_value != "1":
		return false
	var filter := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	if filter.is_empty():
		return true
	for entry_variant in filter.split(","):
		var entry := String(entry_variant).strip_edges()
		if entry == "road":
			return true
	return false

func _snap_to_roads_enabled() -> bool:
	return not Input.is_key_pressed(KEY_SHIFT)

func _update_lanes_label():
	if active and current_path != null:
		_queue_preview_update()

func adjust_lanes(fwd_delta: int, bkw_delta: int):
	if fwd_delta != 0: fwd_lanes = clamp(fwd_lanes + fwd_delta, 0, 4)
	if bkw_delta != 0: bkw_lanes = clamp(bkw_lanes + bkw_delta, 0, 4)
	# Allow 0,0 for walkways
	_update_lanes_label()

func adjust_altitude(delta: float):
	altitude_offset += delta
	_queue_preview_update()

func _unhandled_input(event):
	if active and event is InputEventMouseMotion:
		_queue_preview_update()

	# G toggle works whenever the road tool is the active tool (not just mid-draw).
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_G:
			_set_ghost_guides_enabled(not _ghost_enabled)

	if active and event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_handle_click()


func _handle_click():
	var total_start_us := Time.get_ticks_usec()
	var state_before: int = current_state
	var mouse_start_us := Time.get_ticks_usec()
	var pos = get_world_mouse_pos()
	var mouse_ms := float(Time.get_ticks_usec() - mouse_start_us) / 1000.0
	var tangent_ms := 0.0
	var ghost_queue_ms := 0.0
	var path_ms := 0.0
	var commit_ms := 0.0
	if not is_valid:
		_log_click_detail(
			state_before,
			false,
			mouse_ms,
			tangent_ms,
			ghost_queue_ms,
			path_ms,
			commit_ms,
			total_start_us
		)
		return
	
	match current_state:
		State.IDLE:
			start_pos = pos
			active = true
			_clear_sticky_network_snap()
			if draw_mode == 0:
				current_state = State.SETTING_END
				control_pos = start_pos
			else:
				current_state = State.SETTING_CONTROL

			# Store the road tangent at start_pos so Shift snap is relative to the road,
			# not the world grid. Falls back to Vector2(0,1) on open terrain.
			var tangent_start_us := Time.get_ticks_usec()
			var _st: Vector2 = simulation_node.get_road_tangent_at(start_pos, 6.0)
			tangent_ms = float(Time.get_ticks_usec() - tangent_start_us) / 1000.0
			_start_tangent_angle = atan2(_st.y, _st.x)  # atan2(z, x) = East-0° convention
			# Detect fallback: (0,1) means no road found within range.
			_has_road_tangent = not (_st.x == 0.0 and _st.y == 1.0)

			# Build ghost guides on first placement click (network is settled at this point).
			if _ghost_guides_dirty:
				var ghost_queue_start_us := Time.get_ticks_usec()
				_request_deferred_ghost_rebuild()
				ghost_queue_ms = float(Time.get_ticks_usec() - ghost_queue_start_us) / 1000.0

			var path_start_us := Time.get_ticks_usec()
			current_path = Path3D.new()
			current_path.curve = Curve3D.new()
			current_path.curve.bake_interval = 0.5
			current_path.curve.up_vector_enabled = false # Prevent 'looking_at' errors on degenerate paths
			add_child(current_path)
			path_ms = float(Time.get_ticks_usec() - path_start_us) / 1000.0
			
		State.SETTING_CONTROL:
			control_pos = pos
			_clear_sticky_network_snap()
			current_state = State.SETTING_END
			
		State.SETTING_END:
			var commit_start_us := Time.get_ticks_usec()
			_commit_segment(pos)
			commit_ms = float(Time.get_ticks_usec() - commit_start_us) / 1000.0

	_log_click_detail(
		state_before,
		is_valid,
		mouse_ms,
		tangent_ms,
		ghost_queue_ms,
		path_ms,
		commit_ms,
		total_start_us
	)

func _update_preview():
	if current_path == null: return
	var points := _refresh_preview_curve()
	var validation := _candidate_validation_for_points(points)
	_draw_candidate_preview(points, validation)
	if bool(validation.get("is_valid", false)):
		_draw_blueprint(points, validation)

func _refresh_preview_curve() -> PackedVector3Array:
	if current_path == null:
		return PackedVector3Array()
	var mouse_pos: Vector3 = _last_world_mouse_pos if _has_last_world_mouse_pos else get_world_mouse_pos()
	
	current_path.curve.clear_points()
	is_valid = true
	_curve_shape_valid = true
	
	match current_state:
		State.SETTING_CONTROL:
			# Ghost line from A to Mouse
			current_path.curve.add_point(start_pos)
			if start_pos.distance_to(mouse_pos) > 0.1:
				current_path.curve.add_point(mouse_pos)
			
		State.SETTING_END:
			if start_pos.distance_to(mouse_pos) < 0.1:
				current_path.curve.add_point(start_pos)
			else:
				if draw_mode == 0:
					current_path.curve.add_point(start_pos)
					current_path.curve.add_point(mouse_pos)
				else:
					var t_start = (control_pos - start_pos)
					var t_end = (mouse_pos - control_pos) # Direction of the second half
					
					# Curvature Guard: Check angle between 'in' and 'out' tangents
					if t_start.length() > 0.1 and t_end.length() > 0.1:
						var angle = t_start.angle_to(t_end)
						if angle > PI * 0.5: # 90 degrees
							_curve_shape_valid = false
					
					current_path.curve.add_point(start_pos, Vector3.ZERO, t_start)
					current_path.curve.add_point(mouse_pos, -t_end, Vector3.ZERO)

	return _road_surface_points_from_curve(current_path.curve)

func _queue_preview_update() -> void:
	if current_path == null:
		return
	var points := _refresh_preview_curve()
	var validation := _candidate_validation_for_points(points)
	_draw_candidate_preview(points, validation)
	if not bool(validation.get("is_valid", false)):
		_preview_update_pending = false
		_preview_exact_waiting = false
		_preview_idle_exact_delay_sec = 0.0
		return
	_preview_update_pending = false
	_preview_exact_waiting = true
	_preview_idle_exact_delay_sec = ROAD_PREVIEW_EXACT_IDLE_DELAY_SEC

func _candidate_validation_for_points(points: PackedVector3Array) -> Dictionary:
	if points.size() < 2:
		return {
			"is_valid": false,
			"invalid_reason": "too_short"
		}
	if not _curve_shape_valid:
		return {
			"is_valid": false,
			"invalid_reason": "surface_geometry_invalid"
		}
	if (
		not _candidate_cache_validation.is_empty()
		and _candidate_cache_fwd_lanes == fwd_lanes
		and _candidate_cache_bkw_lanes == bkw_lanes
		and _candidate_cache_snap_to_roads == _snap_to_roads_enabled()
		and _candidate_cache_surface_generation == simulation_node.get_road_tool_surface_generation()
		and _road_surface_points_match(points, _candidate_cache_points)
	):
		return _candidate_cache_validation.duplicate(true)

	var validation_variant = simulation_node.validate_road_candidate_with_snap(
		points,
		fwd_lanes,
		bkw_lanes,
		_snap_to_roads_enabled()
	)
	var validation: Dictionary = validation_variant if validation_variant is Dictionary else {
		"is_valid": false,
		"invalid_reason": "validation_unavailable"
	}
	_remember_candidate_validation(points, validation)
	return validation

func _commit_validation_for_points(points: PackedVector3Array) -> Dictionary:
	if points.size() < 2:
		return {
			"is_valid": false,
			"invalid_reason": "too_short"
		}
	if not _curve_shape_valid:
		return {
			"is_valid": false,
			"invalid_reason": "surface_geometry_invalid"
		}

	var preview := _cached_preview_surface_for_points(points)
	if preview.is_empty():
		preview = _get_compiled_preview_surface("commit")
	if _preview_surface_generation_is_current(preview):
		_remember_candidate_validation(points, preview)
		return preview

	return _candidate_validation_for_points(points)

func _remember_candidate_validation(points: PackedVector3Array, validation: Dictionary) -> void:
	_candidate_cache_points = points
	_candidate_cache_validation = validation.duplicate(true)
	_candidate_cache_fwd_lanes = fwd_lanes
	_candidate_cache_bkw_lanes = bkw_lanes
	_candidate_cache_snap_to_roads = _snap_to_roads_enabled()
	_candidate_cache_surface_generation = simulation_node.get_road_tool_surface_generation()

func _preview_surface_generation_is_current(preview: Dictionary) -> bool:
	if preview.is_empty():
		return false
	var generation := int(preview.get("surface_generation", -1))
	if generation <= 0:
		return false
	return generation == simulation_node.get_road_tool_surface_generation()

func _poll_pending_preview_result() -> void:
	if current_path == null or _preview_request_id <= 0:
		_preview_result_pending = false
		return

	var preview = simulation_node.get_preview_road_surface_result(_preview_request_id)
	if preview == null:
		return

	var points: PackedVector3Array = _road_surface_points_from_curve(current_path.curve)
	if not _preview_request_matches(points):
		_preview_result_pending = false
		_queue_preview_update()
		return

	if not _preview_surface_generation_is_current(preview):
		_preview_result_pending = false
		_preview_request_id = 0
		_queue_preview_update()
		return

	_preview_result_pending = false
	_remember_preview_surface(points, preview)
	var validation := _candidate_validation_for_points(points)
	if bool(validation.get("is_valid", false)):
		if not _draw_compiled_preview_surface(points, preview, validation):
			_draw_candidate_preview(points, validation)
	else:
		_draw_candidate_preview(points, validation)

func _draw_blueprint(points: PackedVector3Array, validation: Dictionary) -> void:
	var preview := _get_compiled_preview_surface()
	if preview.is_empty():
		return

	if not _draw_compiled_preview_surface(points, preview, validation):
		_draw_candidate_preview(points, validation)

func _draw_candidate_preview(points: PackedVector3Array, validation: Dictionary) -> void:
	is_valid = bool(validation.get("is_valid", false))
	if points.size() > 1:
		_draw_coarse_preview_surface(points, validation)
	else:
		_clear_preview_visual()

func _clear_preview_visual() -> void:
	blueprint_mesh.mesh = null
	_preview_drawn_request_id = 0
	if _info_label:
		_info_label.visible = false

func _draw_compiled_preview_surface(
	points: PackedVector3Array,
	preview: Dictionary,
	validation: Dictionary
) -> bool:
	var preview_verts: PackedVector3Array = preview.get("prepared_points", PackedVector3Array())
	var surface_vertices: PackedVector3Array = preview.get("surface_vertices", PackedVector3Array())
	if surface_vertices.size() < 3 or not bool(preview.get("is_valid", true)):
		return false

	var preview_request_id := int(preview.get("request_id", 0))
	if preview_request_id > 0 and preview_request_id == _preview_drawn_request_id:
		_update_preview_measurement_label(preview_verts if preview_verts.size() > 1 else points, validation)
		return true

	var arr_mesh = ArrayMesh.new()
	var arrays = []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = surface_vertices
	arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	blueprint_mesh.mesh = arr_mesh
	_preview_drawn_request_id = preview_request_id
	_update_preview_measurement_label(preview_verts if preview_verts.size() > 1 else points, validation)
	return true

func _draw_coarse_preview_surface(points: PackedVector3Array, preview: Dictionary = {}) -> bool:
	if points.size() < 2:
		return false

	var half_width := _coarse_preview_half_width_m()
	var surface_vertices := PackedVector3Array()
	for index in range(points.size() - 1):
		var a: Vector3 = points[index]
		var b: Vector3 = points[index + 1]
		var dir := b - a
		dir.y = 0.0
		if dir.length_squared() <= 0.0001:
			continue
		dir = dir.normalized()
		var side := Vector3(-dir.z, 0.0, dir.x) * half_width
		var a_left := a + side
		var a_right := a - side
		var b_left := b + side
		var b_right := b - side
		surface_vertices.append(a_left)
		surface_vertices.append(b_left)
		surface_vertices.append(a_right)
		surface_vertices.append(a_right)
		surface_vertices.append(b_left)
		surface_vertices.append(b_right)

	if surface_vertices.size() < 3:
		return false

	var arr_mesh := ArrayMesh.new()
	var arrays: Array = []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = surface_vertices
	arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	blueprint_mesh.mesh = arr_mesh
	_preview_drawn_request_id = 0
	_update_preview_measurement_label(points, preview)
	return true

func _coarse_preview_half_width_m() -> float:
	var lane_count: int = max(1, fwd_lanes + bkw_lanes)
	var road_width := maxf(ROAD_PREVIEW_MIN_WIDTH_M, float(lane_count) * ROAD_PREVIEW_LANE_WIDTH_M)
	return road_width * 0.5 + ROAD_PREVIEW_SIDEWALK_WIDTH_M

func _update_preview_measurement_label(points: PackedVector3Array, preview: Dictionary = {}) -> void:
	if not _info_label or points.size() < 2:
		return

	var total_m := 0.0
	for i in range(1, points.size()):
		total_m += points[i].distance_to(points[i - 1])

	# Angle display:
	# - Connected to a road: show angle relative to that road (0°=parallel, 90°=perpendicular).
	# - Open terrain: show world orientation (0°=E-W, 90°=N-S).
	# In both cases folded to [0°, 90°] — a road and its reverse are identical.
	var d: Vector3 = points[-1] - points[0]
	var world_angle: float = atan2(d.z, d.x)  # East=0, North=PI/2
	var angle_deg: float
	if _has_road_tangent:
		var relative: float = world_angle - _start_tangent_angle
		# Normalise to (-PI, PI]
		while relative > PI: relative -= TAU
		while relative <= -PI: relative += TAU
		angle_deg = abs(rad_to_deg(relative))
		# Fold to [0°, 90°]: 180°-parallel == parallel, 90°+x == 90°-x
		if angle_deg > 90.0:
			angle_deg = 180.0 - angle_deg
	else:
		angle_deg = rad_to_deg(world_angle)
		if angle_deg < 0.0: angle_deg += 360.0
		if angle_deg >= 180.0: angle_deg -= 180.0

	var snap_str := " [angle]" if (active and Input.is_key_pressed(KEY_SHIFT)) else ""
	var build_cost := float(preview.get("build_cost", 0.0))
	if bool(preview.get("is_pending", false)):
		_info_label.add_theme_color_override("font_color", Color(1.0, 0.76, 0.18, 0.98))
		_info_label.text = "Checking... %.1f m  %.0f°  cost %.0f%s" % [
			total_m,
			angle_deg,
			build_cost,
			snap_str,
		]
	elif not bool(preview.get("is_valid", true)):
		_info_label.add_theme_color_override("font_color", Color(1.0, 0.28, 0.18, 0.98))
		_info_label.text = _preview_invalid_text(preview)
	else:
		_info_label.add_theme_color_override("font_color", Color(1.0, 1.0, 1.0, 0.95))
		_info_label.text = "%.1f m  %.0f°  cost %.0f%s" % [
			total_m,
			angle_deg,
			build_cost,
			snap_str,
		]

	# Store the preview endpoint — projected to screen each frame in _process.
	var label_pos: Vector3 = points[points.size() - 1]
	label_pos.y += 2.0
	_label_world_pos = label_pos
	_info_label.visible = true

func _preview_invalid_text(preview: Dictionary) -> String:
	var reason := String(preview.get("invalid_reason", ""))
	match reason:
		"too_steep":
			var max_grade := float(preview.get("max_grade", 0.0)) * 100.0
			var allowed_grade := float(preview.get("allowed_grade", 0.0)) * 100.0
			return "Too steep %.0f%% / %.0f%%" % [max_grade, allowed_grade]
		"bridge_clearance":
			return "Bridge clearance %.1f m / %.1f m" % [
				float(preview.get("clearance_m", 0.0)),
				float(preview.get("required_clearance_m", 0.0)),
			]
		"tunnel_clearance":
			return "Tunnel clearance %.1f m / %.1f m" % [
				float(preview.get("clearance_m", 0.0)),
				float(preview.get("required_clearance_m", 0.0)),
			]
		"water_requires_bridge":
			return "Water crossing requires a bridge"
		"surface_geometry_invalid":
			return "Curve too tight"
		"too_short":
			return "Road too short"
		"validation_unavailable":
			return "Road check unavailable"
		_:
			return "Road cannot be placed"

func _commit_segment(end_pos):
	var total_start_us := Time.get_ticks_usec()

	var baked_start_us := Time.get_ticks_usec()
	var points := _refresh_preview_curve()
	var raw_points := current_path.curve.get_baked_points() if current_path else PackedVector3Array()
	var baked_ms := float(Time.get_ticks_usec() - baked_start_us) / 1000.0
	var preview_start_us := Time.get_ticks_usec()
	var validation := _commit_validation_for_points(points)
	var preview_ms := float(Time.get_ticks_usec() - preview_start_us) / 1000.0
	var committed := false
	var preview_valid := bool(validation.get("is_valid", false))
	var invalid_reason := String(validation.get("invalid_reason", ""))
	var max_grade := float(validation.get("max_grade", 0.0))
	var allowed_grade := float(validation.get("allowed_grade", 0.0))
	var span_start := float(validation.get("offending_span_start_m", 0.0))
	var span_end := float(validation.get("offending_span_end_m", 0.0))
	var span_run := float(validation.get("offending_span_run_m", 0.0))
	var span_dy := float(validation.get("offending_span_height_delta_m", 0.0))
	var span_start_y := float(validation.get("offending_span_start_height_m", 0.0))
	var span_end_y := float(validation.get("offending_span_end_height_m", 0.0))
	var span_start_terrain_y := float(validation.get("offending_span_start_terrain_height_m", 0.0))
	var span_end_terrain_y := float(validation.get("offending_span_end_terrain_height_m", 0.0))
	var span_start_delta := float(validation.get("offending_span_start_support_delta_m", 0.0))
	var span_end_delta := float(validation.get("offending_span_end_support_delta_m", 0.0))
	var start_snap := int(validation.get("start_endpoint_snapped_node_id", -1))
	var end_snap := int(validation.get("end_endpoint_snapped_node_id", -1))
	var start_endpoint_delta := float(validation.get("start_endpoint_support_delta_m", 0.0))
	var end_endpoint_delta := float(validation.get("end_endpoint_support_delta_m", 0.0))
	var add_road_ms := 0.0
	var bookkeeping_ms := 0.0
	var cancel_ms := 0.0
	if points.size() > 1 and not preview_valid:
		_draw_candidate_preview(points, validation)
	if points.size() > 1 and preview_valid:
		var add_road_start_us := Time.get_ticks_usec()
		simulation_node.add_road_with_snap(points, fwd_lanes, bkw_lanes, _snap_to_roads_enabled())
		add_road_ms = float(Time.get_ticks_usec() - add_road_start_us) / 1000.0
		# Do NOT trigger the terrain/network visual refresh here — the road
		# is queued to the sim thread and is not in the graph yet.  _process polls
		# is_network_dirty() and NetworkRenderer rebuilds the visuals once the road lands.

		var bookkeeping_start_us := Time.get_ticks_usec()
		# Queue border check — must run AFTER the road is in the graph (nodes exist).
		# NetworkRenderer drains _pending_border_checks when network_dirty fires.
		_pending_border_checks.push_back([start_pos, end_pos])
		# Ghost guides must be rebuilt once the road lands in the graph.
		_ghost_guides_dirty = true
		bookkeeping_ms = float(Time.get_ticks_usec() - bookkeeping_start_us) / 1000.0
		committed = true

	if committed:
		var cancel_start_us := Time.get_ticks_usec()
		cancel_road()
		cancel_ms = float(Time.get_ticks_usec() - cancel_start_us) / 1000.0

	if _road_debug_enabled:
		print(
			"[DEBUG:road] commit_segment_detail raw_points=%d points=%d preview_empty=%s preview_valid=%s invalid_reason=%s max_grade=%.3f allowed_grade=%.3f span=(%.3f,%.3f) run=%.3f dy=%.3f span_y=(%.3f,%.3f) span_terrain=(%.3f,%.3f) span_delta=(%.3f,%.3f) endpoint_snap=(%d,%d) endpoint_delta=(%.3f,%.3f) committed=%s preview_ms=%.3f baked_ms=%.3f add_road_ms=%.3f bookkeeping_ms=%.3f cancel_ms=%.3f total_ms=%.3f"
			% [
				raw_points.size(),
				points.size(),
				"false",
				str(preview_valid),
				invalid_reason,
				max_grade,
				allowed_grade,
				span_start,
				span_end,
				span_run,
				span_dy,
				span_start_y,
				span_end_y,
				span_start_terrain_y,
				span_end_terrain_y,
				span_start_delta,
				span_end_delta,
				start_snap,
				end_snap,
				start_endpoint_delta,
				end_endpoint_delta,
				str(committed),
				preview_ms,
				baked_ms,
				add_road_ms,
				bookkeeping_ms,
				cancel_ms,
				float(Time.get_ticks_usec() - total_start_us) / 1000.0,
			]
		)

func cancel_road():
	current_state = State.IDLE
	_clear_sticky_network_snap()
	_clear_preview_visual()
	if current_path:
		current_path.queue_free()
	current_path = null
	_preview_update_pending = false
	_preview_idle_exact_delay_sec = 0.0
	_clear_preview_cache()
	if _info_label:
		_info_label.visible = false

func mark_network_topology_dirty() -> void:
	mark_network_nodes_dirty()
	_clear_sticky_network_snap()
	_ghost_guides_dirty = true
	_request_deferred_ghost_rebuild()

func _set_ghost_guides_enabled(enabled: bool) -> void:
	_ghost_enabled = enabled
	if _ghost_enabled:
		_ghost_guides_dirty = true
		_request_deferred_ghost_rebuild()
	elif ghost_mesh:
		ghost_mesh.visible = false

## Called by NetworkRenderer after the road is confirmed in the graph.
## Drains _pending_border_checks and shows the border-connection dialog if relevant.
## Also queues ghost guide rebuild now that the new road is in the graph.
func drain_pending_border_checks() -> void:
	var total_start_us := Time.get_ticks_usec()
	var pending_count := _pending_border_checks.size()
	var ghost_was_dirty := _ghost_guides_dirty
	var ghost_was_queued := _ghost_rebuild_queued
	var ghost_ms := 0.0
	var candidate_ms := 0.0
	var prompt_ms := 0.0
	var candidate_calls := 0
	var prompt_count := 0
	if _ghost_guides_dirty:
		var ghost_start_us := Time.get_ticks_usec()
		_request_deferred_ghost_rebuild()
		ghost_ms = float(Time.get_ticks_usec() - ghost_start_us) / 1000.0
	while not _pending_border_checks.is_empty():
		var pair = _pending_border_checks.pop_front()
		var candidate_start_us := Time.get_ticks_usec()
		var candidate: int = simulation_node.check_border_candidate(pair[0])
		candidate_calls += 1
		if candidate < 0:
			candidate = simulation_node.check_border_candidate(pair[1])
			candidate_calls += 1
		candidate_ms += float(Time.get_ticks_usec() - candidate_start_us) / 1000.0
		if candidate >= 0:
			var prompt_start_us := Time.get_ticks_usec()
			_prompt_border_connection(candidate)
			prompt_ms += float(Time.get_ticks_usec() - prompt_start_us) / 1000.0
			prompt_count += 1
	if _road_debug_enabled:
		print(
			"[DEBUG:road] border_checks_detail pending=%d ghost_dirty=%s ghost_queued_before=%s ghost_queued_after=%s ghost_ms=%.3f candidate_calls=%d candidate_ms=%.3f prompts=%d prompt_ms=%.3f total_ms=%.3f"
			% [
				pending_count,
				str(ghost_was_dirty),
				str(ghost_was_queued),
				str(_ghost_rebuild_queued),
				ghost_ms,
				candidate_calls,
				candidate_ms,
				prompt_count,
				prompt_ms,
				float(Time.get_ticks_usec() - total_start_us) / 1000.0,
			]
		)

## Shows a dialog asking whether to make this road endpoint an external connection.
## On confirmation, the node is promoted to Border and becomes an immigrant spawn point.
func _prompt_border_connection(node_id: int) -> void:
	var dialog := ConfirmationDialog.new()
	dialog.title = "External Connection"
	dialog.dialog_text = (
		"This road reaches the map boundary.\n" +
		"Create an external connection here?\n\n" +
		"Immigrants will enter and leave the city through this point.\n" +
		"Immigration only flows while the road remains connected."
	)
	dialog.ok_button_text = "Create Connection"
	dialog.cancel_button_text = "No Thanks"
	add_child(dialog)
	dialog.confirmed.connect(func() -> void:
		simulation_node.set_border_connection(node_id)
		update_main_mesh()
		dialog.queue_free()
	)
	dialog.canceled.connect(func() -> void: dialog.queue_free())
	dialog.popup_centered()


# Override to resolve the full road-tool cursor in one Rust-side hot-path query.
func get_world_mouse_pos() -> Vector3:
	var mouse_screen := get_viewport().get_mouse_position()
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		is_valid = false
		return Vector3.ZERO

	var pos_variant = simulation_node.get_road_tool_cursor_pos(
		camera.project_ray_origin(mouse_screen),
		camera.project_ray_normal(mouse_screen),
		altitude_offset,
		active,
		current_state,
		start_pos,
		control_pos,
		Input.is_key_pressed(KEY_SHIFT),
		_start_tangent_angle,
		_ghost_enabled,
		MAP_BORDER_SNAP_DIST_M,
		_sticky_network_snap_active,
		_sticky_network_snap_pos,
		ROAD_NETWORK_SNAP_RELEASE_DIST_M
	)
	if pos_variant == null:
		if _has_last_world_mouse_pos:
			return _last_world_mouse_pos
		is_valid = false
		return Vector3.ZERO
	is_valid = true
	var pos: Vector3 = pos_variant
	if not _snap_to_roads_enabled():
		_clear_sticky_network_snap()
		return pos
	_update_sticky_network_snap(pos)
	return pos

func _update_sticky_network_snap(pos: Vector3) -> void:
	if not _snap_to_roads_enabled():
		_clear_sticky_network_snap()
		return
	if not _can_stick_to_network_snap(pos):
		_clear_sticky_network_snap()
		return
	var network_pos = simulation_node.get_closest_network_point(pos, ROAD_NETWORK_SNAP_CONFIRM_EPS_M)
	if network_pos == null:
		_clear_sticky_network_snap()
		return
	if _xz_distance(pos, network_pos) > ROAD_NETWORK_SNAP_CONFIRM_EPS_M:
		_clear_sticky_network_snap()
		return
	_sticky_network_snap_active = true
	_sticky_network_snap_pos = pos

func _can_stick_to_network_snap(pos: Vector3) -> bool:
	if current_state == State.SETTING_END:
		if _xz_distance(pos, start_pos) < ROAD_SELF_SNAP_DIST_M:
			return false
		if draw_mode != 0 and _xz_distance(pos, control_pos) < ROAD_SELF_SNAP_DIST_M:
			return false
	return true

func _clear_sticky_network_snap() -> void:
	_sticky_network_snap_active = false
	_sticky_network_snap_pos = Vector3.ZERO

func _xz_distance(a: Vector3, b: Vector3) -> float:
	var dx := a.x - b.x
	var dz := a.z - b.z
	return sqrt(dx * dx + dz * dz)

## Returns preview geometry compiled through the shared Rust road-surface pipeline.
## If the sim mutex is momentarily contended, returns an empty invalid preview instead of stale geometry.
func _get_compiled_preview_surface(profile_label: String = "") -> Dictionary:
	var total_start_us := Time.get_ticks_usec()
	if current_path == null:
		_log_preview_surface_detail(profile_label, 0, 0, false, 0.0, 0.0, total_start_us)
		return {}

	var baked_start_us := Time.get_ticks_usec()
	var points: PackedVector3Array = _road_surface_points_from_curve(current_path.curve)
	var baked_ms := float(Time.get_ticks_usec() - baked_start_us) / 1000.0
	var cached_preview := _cached_preview_surface_for_points(points)
	if not cached_preview.is_empty():
		if not _preview_surface_generation_is_current(cached_preview):
			_preview_cache_points = PackedVector3Array()
			_preview_cache_surface = {}
			_preview_cache_fwd_lanes = -1
			_preview_cache_bkw_lanes = -1
			_preview_cache_snap_to_roads = true
			cached_preview = {}
	if not cached_preview.is_empty():
		_preview_result_pending = false
		_preview_exact_waiting = false
		var cached_vertices: PackedVector3Array = cached_preview.get("surface_vertices", PackedVector3Array())
		_log_preview_surface_detail(
			profile_label,
			points.size(),
			cached_vertices.size(),
			bool(cached_preview.get("is_valid", false)),
			baked_ms,
			0.0,
			total_start_us,
			String(cached_preview.get("invalid_reason", "")),
			float(cached_preview.get("max_grade", 0.0)),
			float(cached_preview.get("allowed_grade", 0.0)),
			cached_preview
		)
		return cached_preview
	if points.size() <= 1:
		_preview_result_pending = false
		_preview_exact_waiting = false
		_log_preview_surface_detail(profile_label, points.size(), 0, true, baked_ms, 0.0, total_start_us)
		var empty_preview := {
			"prepared_points": points,
			"surface_vertices": PackedVector3Array(),
			"is_valid": true
		}
		_remember_preview_surface(points, empty_preview)
		return empty_preview

	if profile_label != "commit" and _preview_exact_waiting and _preview_idle_exact_delay_sec > 0.0:
		_log_preview_surface_detail(profile_label, points.size(), 0, false, baked_ms, 0.0, total_start_us)
		return {}

	var rust_start_us := Time.get_ticks_usec()
	if not _preview_request_matches(points):
		_preview_request_id = simulation_node.request_preview_road_surface_with_snap(
			points,
			fwd_lanes,
			bkw_lanes,
			_snap_to_roads_enabled()
		)
		_preview_request_points = points
		_preview_request_fwd_lanes = fwd_lanes
		_preview_request_bkw_lanes = bkw_lanes
		_preview_request_snap_to_roads = _snap_to_roads_enabled()
		_preview_result_pending = true
		_preview_exact_waiting = false
	var preview = simulation_node.get_preview_road_surface_result(_preview_request_id)
	var rust_ms := float(Time.get_ticks_usec() - rust_start_us) / 1000.0
	if preview == null:
		_preview_result_pending = true
		_log_preview_surface_detail(profile_label, points.size(), 0, false, baked_ms, rust_ms, total_start_us)
		return {}

	if not _preview_surface_generation_is_current(preview):
		_preview_result_pending = false
		_preview_request_id = 0
		_log_preview_surface_detail(
			profile_label,
			points.size(),
			0,
			false,
			baked_ms,
			rust_ms,
			total_start_us,
			"stale_surface_generation"
		)
		return {}

	_preview_result_pending = false
	_preview_exact_waiting = false
	var surface_vertices: PackedVector3Array = preview.get("surface_vertices", PackedVector3Array())
	_log_preview_surface_detail(
		profile_label,
		points.size(),
		surface_vertices.size(),
		bool(preview.get("is_valid", false)),
		baked_ms,
		rust_ms,
		total_start_us,
		String(preview.get("invalid_reason", "")),
		float(preview.get("max_grade", 0.0)),
		float(preview.get("allowed_grade", 0.0)),
		preview
	)
	_remember_preview_surface(points, preview)
	return preview

func _remember_preview_surface(points: PackedVector3Array, preview: Dictionary) -> void:
	_preview_cache_points = points
	_preview_cache_surface = preview.duplicate(true)
	_preview_cache_fwd_lanes = fwd_lanes
	_preview_cache_bkw_lanes = bkw_lanes
	_preview_cache_snap_to_roads = _snap_to_roads_enabled()

func _cached_preview_surface_for_points(points: PackedVector3Array) -> Dictionary:
	if _preview_cache_surface.is_empty():
		return {}
	if _preview_cache_fwd_lanes != fwd_lanes or _preview_cache_bkw_lanes != bkw_lanes:
		return {}
	if _preview_cache_snap_to_roads != _snap_to_roads_enabled():
		return {}
	if not _road_surface_points_match(points, _preview_cache_points):
		return {}
	return _preview_cache_surface.duplicate(true)

func _preview_request_matches(points: PackedVector3Array) -> bool:
	if _preview_request_id <= 0:
		return false
	if _preview_request_fwd_lanes != fwd_lanes or _preview_request_bkw_lanes != bkw_lanes:
		return false
	if _preview_request_snap_to_roads != _snap_to_roads_enabled():
		return false
	return _road_surface_points_match(points, _preview_request_points)

func _clear_preview_cache() -> void:
	_preview_cache_points = PackedVector3Array()
	_preview_cache_surface = {}
	_preview_cache_fwd_lanes = -1
	_preview_cache_bkw_lanes = -1
	_preview_cache_snap_to_roads = true
	_candidate_cache_points = PackedVector3Array()
	_candidate_cache_validation = {}
	_candidate_cache_fwd_lanes = -1
	_candidate_cache_bkw_lanes = -1
	_candidate_cache_snap_to_roads = true
	_candidate_cache_surface_generation = -1
	_preview_request_points = PackedVector3Array()
	_preview_request_fwd_lanes = -1
	_preview_request_bkw_lanes = -1
	_preview_request_snap_to_roads = true
	_preview_request_id = 0
	_preview_drawn_request_id = 0
	_preview_result_pending = false
	_preview_exact_waiting = false
	_preview_idle_exact_delay_sec = 0.0

func _road_surface_points_match(left: PackedVector3Array, right: PackedVector3Array) -> bool:
	if left.size() != right.size():
		return false
	var epsilon_sq := ROAD_SURFACE_POINT_EPS_M * ROAD_SURFACE_POINT_EPS_M
	for index in range(left.size()):
		if left[index].distance_squared_to(right[index]) > epsilon_sq:
			return false
	return true

func _road_surface_points_from_curve(curve: Curve3D) -> PackedVector3Array:
	var raw_points: PackedVector3Array = curve.get_baked_points()
	if raw_points.size() <= 2:
		return raw_points

	var simplified := PackedVector3Array()
	var first_point: Vector3 = raw_points[0]
	var last_point: Vector3 = raw_points[raw_points.size() - 1]
	if draw_mode == 0:
		simplified.append(first_point)
		if first_point.distance_to(last_point) > ROAD_SURFACE_POINT_EPS_M:
			simplified.append(last_point)
		return simplified

	var length_m := curve.get_baked_length()
	if length_m <= ROAD_SURFACE_POINT_EPS_M:
		return raw_points

	var interval_count: int = max(1, int(ceil(length_m / ROAD_SURFACE_CURVE_STEP_M)))
	for index in range(interval_count + 1):
		var offset_m := length_m * float(index) / float(interval_count)
		simplified = _append_road_surface_point(simplified, curve.sample_baked(offset_m, true))

	if simplified.is_empty():
		return raw_points
	simplified[0] = first_point
	if simplified[simplified.size() - 1].distance_to(last_point) <= ROAD_SURFACE_POINT_EPS_M:
		simplified[simplified.size() - 1] = last_point
	else:
		simplified.append(last_point)
	return simplified

func _append_road_surface_point(points: PackedVector3Array, point: Vector3) -> PackedVector3Array:
	if not points.is_empty() and points[points.size() - 1].distance_to(point) <= ROAD_SURFACE_POINT_EPS_M:
		return points
	points.append(point)
	return points

func _log_click_detail(
	state_before: int,
	valid_after: bool,
	mouse_ms: float,
	tangent_ms: float,
	ghost_queue_ms: float,
	path_ms: float,
	commit_ms: float,
	total_start_us: int
) -> void:
	if not _road_debug_enabled:
		return
	print(
		"[DEBUG:road] road_click_detail state=%d valid=%s mouse_ms=%.3f tangent_ms=%.3f ghost_queue_ms=%.3f path_ms=%.3f commit_ms=%.3f total_ms=%.3f"
		% [
			state_before,
			str(valid_after),
			mouse_ms,
			tangent_ms,
			ghost_queue_ms,
			path_ms,
			commit_ms,
			float(Time.get_ticks_usec() - total_start_us) / 1000.0,
		]
	)

func _log_preview_surface_detail(
	label: String,
	point_count: int,
	surface_vertex_count: int,
	valid: bool,
	baked_ms: float,
	rust_ms: float,
	total_start_us: int,
	invalid_reason: String = "",
	max_grade: float = 0.0,
	allowed_grade: float = 0.0,
	validation: Dictionary = {}
) -> void:
	if not _road_debug_enabled:
		return
	var total_ms := float(Time.get_ticks_usec() - total_start_us) / 1000.0
	var log_label := label
	if log_label.is_empty() and valid and total_ms < ROAD_PROFILE_SLOW_MS:
		return
	if log_label.is_empty():
		if valid:
			log_label = "slow"
		elif invalid_reason.is_empty():
			log_label = "pending"
		else:
			log_label = "invalid"
	var span_start := float(validation.get("offending_span_start_m", 0.0))
	var span_end := float(validation.get("offending_span_end_m", 0.0))
	var span_run := float(validation.get("offending_span_run_m", 0.0))
	var span_dy := float(validation.get("offending_span_height_delta_m", 0.0))
	var span_start_y := float(validation.get("offending_span_start_height_m", 0.0))
	var span_end_y := float(validation.get("offending_span_end_height_m", 0.0))
	var span_start_terrain_y := float(validation.get("offending_span_start_terrain_height_m", 0.0))
	var span_end_terrain_y := float(validation.get("offending_span_end_terrain_height_m", 0.0))
	var span_start_delta := float(validation.get("offending_span_start_support_delta_m", 0.0))
	var span_end_delta := float(validation.get("offending_span_end_support_delta_m", 0.0))
	var start_snap := int(validation.get("start_endpoint_snapped_node_id", -1))
	var end_snap := int(validation.get("end_endpoint_snapped_node_id", -1))
	var start_endpoint_delta := float(validation.get("start_endpoint_support_delta_m", 0.0))
	var end_endpoint_delta := float(validation.get("end_endpoint_support_delta_m", 0.0))
	print(
		"[DEBUG:road] preview_surface_godot label=%s points=%d surface_vertices=%d valid=%s reason=%s max_grade=%.3f allowed_grade=%.3f span=(%.3f,%.3f) run=%.3f dy=%.3f span_y=(%.3f,%.3f) span_terrain=(%.3f,%.3f) span_delta=(%.3f,%.3f) endpoint_snap=(%d,%d) endpoint_delta=(%.3f,%.3f) baked_ms=%.3f rust_ms=%.3f total_ms=%.3f"
		% [
			log_label,
			point_count,
			surface_vertex_count,
			str(valid),
			invalid_reason,
			max_grade,
			allowed_grade,
			span_start,
			span_end,
			span_run,
			span_dy,
			span_start_y,
			span_end_y,
			span_start_terrain_y,
			span_end_terrain_y,
			span_start_delta,
			span_end_delta,
			start_snap,
			end_snap,
			start_endpoint_delta,
			end_endpoint_delta,
			baked_ms,
			rust_ms,
			total_ms,
		]
	)

# ── Ghost guide lines ────────────────────────────────────────────────────────

func _request_deferred_ghost_rebuild() -> void:
	if not _ghost_enabled:
		if ghost_mesh:
			ghost_mesh.visible = false
		return
	if _ghost_rebuild_queued:
		return
	_ghost_rebuild_queued = true
	call_deferred("_rebuild_ghost_lines_if_dirty")

func _rebuild_ghost_lines_if_dirty() -> void:
	_ghost_rebuild_queued = false
	if _ghost_guides_dirty:
		_rebuild_ghost_lines()

## Rebuilds the ImmediateMesh for ghost guide lines.
## Called when the tool activates, when G is toggled, and after a road is committed.
func _rebuild_ghost_lines() -> void:
	if not ghost_mesh:
		return
	if not _ghost_enabled:
		ghost_mesh.visible = false
		return

	var total_start_us := Time.get_ticks_usec()
	var fetch_start_us := Time.get_ticks_usec()
	var guide_data: Dictionary = simulation_node.get_road_ghost_line_data()
	var fetch_ms := float(Time.get_ticks_usec() - fetch_start_us) / 1000.0
	var vertices: PackedVector3Array = (
		guide_data.get("vertices", PackedVector3Array())
		as PackedVector3Array
	)
	var colors: PackedColorArray = (
		guide_data.get("colors", PackedColorArray())
		as PackedColorArray
	)
	if vertices.size() < 2:
		ghost_mesh.visible = false
		_ghost_guides_dirty = false
		if _road_debug_enabled:
			print(
				"[DEBUG:road] ghost_lines_godot vertices=%d colors=%d fetch_ms=%.3f upload_ms=0.000 total_ms=%.3f"
				% [
					vertices.size(),
					colors.size(),
					fetch_ms,
					float(Time.get_ticks_usec() - total_start_us) / 1000.0,
				]
			)
		return

	var upload_start_us := Time.get_ticks_usec()
	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)
	var has_colors := colors.size() == vertices.size()
	var fallback_color := Color(1.0, 1.0, 1.0, 0.30)
	for index in range(vertices.size()):
		im.surface_set_color(colors[index] if has_colors else fallback_color)
		im.surface_add_vertex(vertices[index])

	im.surface_end()
	ghost_mesh.mesh = im
	ghost_mesh.visible = true
	_ghost_guides_dirty = false
	var upload_ms := float(Time.get_ticks_usec() - upload_start_us) / 1000.0
	if _road_debug_enabled:
		print(
			"[DEBUG:road] ghost_lines_godot vertices=%d colors=%d fetch_ms=%.3f upload_ms=%.3f total_ms=%.3f"
			% [
				vertices.size(),
				colors.size(),
				fetch_ms,
				upload_ms,
				float(Time.get_ticks_usec() - total_start_us) / 1000.0,
			]
		)
