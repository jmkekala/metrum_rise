## Road drawing tool — straight and spline modes with live compiled preview and lane configuration.
##
## Extends NetworkTool. Adds: Rust-compiled roadbed preview, G1 continuity guard at junctions,
## angle snapping (Shift, 15° steps), distance + angle HUD label, and SimCity-style ghost guide
## lines projected from existing road endpoints (toggle with G key).
## Commits via NetworkTool.add_road() on left-click while preview geometry is solved through the
## shared road-surface compiler.
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
# World-space midpoint of the preview — used each frame to project to screen.
var _label_world_pos: Vector3 = Vector3.ZERO

# ── Ghost guide lines (SimCity-style grid overlay) ───────────────────────────
var _ghost_enabled: bool = true  # toggled with G key
# Cached guide data so we only call Rust when the network changes.
var _ghost_guides_dirty: bool = true
var _ghost_rebuild_queued: bool = false
var _road_debug_enabled: bool = false
var _profile_next_mouse_pos: bool = false
var _preview_cache_points: PackedVector3Array = PackedVector3Array()
var _preview_cache_surface: Dictionary = {}

const ROAD_PROFILE_SLOW_MS := 50.0
const ROAD_SURFACE_CURVE_STEP_M := 4.0
const ROAD_SURFACE_POINT_EPS_M := 0.05

# ── Angle-snap reference ─────────────────────────────────────────────────────
# Base angle (radians) for Shift snapping — set to the road tangent at start_pos
# so that 90° snap gives a true perpendicular to the road, not to the world grid.
var _start_tangent_angle: float = 0.0
# True when a real road tangent was found at start_pos (false = open terrain, show world angle).
var _has_road_tangent: bool = false

func _ready():
	super._ready()
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
		if entry == "road" or entry == "road-geometry":
			return true
	return false

func _update_lanes_label():
	if active and current_path != null:
		_draw_blueprint()

func adjust_lanes(fwd_delta: int, bkw_delta: int):
	if fwd_delta != 0: fwd_lanes = clamp(fwd_lanes + fwd_delta, 0, 4)
	if bkw_delta != 0: bkw_lanes = clamp(bkw_lanes + bkw_delta, 0, 4)
	# Allow 0,0 for walkways
	_update_lanes_label()

func adjust_altitude(delta: float):
	altitude_offset += delta
	_update_preview()

func _unhandled_input(event):
	if active and event is InputEventMouseMotion:
		_update_preview()

	# G toggle works whenever the road tool is the active tool (not just mid-draw).
	if event is InputEventKey and event.pressed and not event.echo:
		if event.keycode == KEY_G:
			_ghost_enabled = not _ghost_enabled
			if _ghost_enabled:
				_request_deferred_ghost_rebuild()
			elif ghost_mesh:
				ghost_mesh.visible = false

	if active and event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_handle_click()


func _handle_click():
	var total_start_us := Time.get_ticks_usec()
	var state_before: int = current_state
	var mouse_start_us := Time.get_ticks_usec()
	_profile_next_mouse_pos = _road_debug_enabled
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
	var mouse_pos = get_world_mouse_pos()
	
	current_path.curve.clear_points()
	is_valid = true
	
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
							is_valid = false
					
					current_path.curve.add_point(start_pos, Vector3.ZERO, t_start)
					current_path.curve.add_point(mouse_pos, -t_end, Vector3.ZERO)

	_draw_blueprint()

func _draw_blueprint():
	var preview := _get_compiled_preview_surface()
	if preview.is_empty():
		blueprint_mesh.mesh = null
		if _info_label:
			_info_label.visible = false
		return

	var preview_verts: PackedVector3Array = preview.get("prepared_points", PackedVector3Array())
	var surface_vertices: PackedVector3Array = preview.get("surface_vertices", PackedVector3Array())
	is_valid = is_valid and bool(preview.get("is_valid", false))

	if surface_vertices.size() >= 3:
		var arr_mesh = ArrayMesh.new()
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = surface_vertices
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		blueprint_mesh.mesh = arr_mesh

		# ── HUD: distance + angle readout ──────────────────────────────────
		if _info_label and preview_verts.size() >= 2:
			var total_m := 0.0
			for i in range(1, preview_verts.size()):
				total_m += preview_verts[i].distance_to(preview_verts[i - 1])

			# Angle display:
			# - Connected to a road: show angle relative to that road (0°=parallel, 90°=perpendicular).
			# - Open terrain: show world orientation (0°=E-W, 90°=N-S).
			# In both cases folded to [0°, 90°] — a road and its reverse are identical.
			var d: Vector3 = preview_verts[-1] - preview_verts[0]
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

			var snap_str := " [snap]" if (active and Input.is_key_pressed(KEY_SHIFT)) else ""
			_info_label.text = "%.1f m  %.0f°%s" % [total_m, angle_deg, snap_str]

			# Store world midpoint — projected to screen each frame in _process.
			var mid: Vector3 = preview_verts[preview_verts.size() / 2]
			mid.y += 2.0
			_label_world_pos = mid
			_info_label.visible = true
	else:
		blueprint_mesh.mesh = null
		if _info_label:
			_info_label.visible = false

func _commit_segment(end_pos):
	var total_start_us := Time.get_ticks_usec()
	if not is_valid: return

	var baked_start_us := Time.get_ticks_usec()
	var raw_points := current_path.curve.get_baked_points() if current_path else PackedVector3Array()
	var points := _road_surface_points_from_curve(current_path.curve) if current_path else PackedVector3Array()
	var baked_ms := float(Time.get_ticks_usec() - baked_start_us) / 1000.0
	var preview_start_us := Time.get_ticks_usec()
	var preview := _cached_preview_surface_for_points(points)
	if preview.is_empty():
		preview = _get_compiled_preview_surface("commit")
	var preview_ms := float(Time.get_ticks_usec() - preview_start_us) / 1000.0
	var committed := false
	var preview_valid := not preview.is_empty() and bool(preview.get("is_valid", false))
	var add_road_ms := 0.0
	var bookkeeping_ms := 0.0
	var cancel_ms := 0.0
	if points.size() > 1 and not preview.is_empty() and bool(preview.get("is_valid", false)):
		var add_road_start_us := Time.get_ticks_usec()
		simulation_node.add_road(points, fwd_lanes, bkw_lanes)
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
			"[DEBUG:road] commit_segment_detail raw_points=%d points=%d preview_empty=%s preview_valid=%s committed=%s preview_ms=%.3f baked_ms=%.3f add_road_ms=%.3f bookkeeping_ms=%.3f cancel_ms=%.3f total_ms=%.3f"
			% [
				raw_points.size(),
				points.size(),
				str(preview.is_empty()),
				str(preview_valid),
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
	if current_path:
		blueprint_mesh.mesh = null
		current_path.queue_free()
	current_path = null
	_preview_cache_points = PackedVector3Array()
	_preview_cache_surface = {}
	if _info_label:
		_info_label.visible = false

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


# Override to add Self-Snapping, Angle Snapping, and Map-Border Snapping.
# When the cursor moves beyond the terrain edge the terrain raycast returns null.
# In that case we project the camera ray onto a flat plane and clamp to the map boundary
# so the endpoint snaps cleanly to the border instead of showing an invalid (red) preview.
func get_world_mouse_pos() -> Vector3:
	var profile_this_call := _profile_next_mouse_pos
	_profile_next_mouse_pos = false
	var total_start_us := Time.get_ticks_usec()
	var terrain_world_start_us := Time.get_ticks_usec()
	var terrain_world_size: Vector2 = simulation_node.get_terrain_world_size()
	var terrain_world_ms := float(Time.get_ticks_usec() - terrain_world_start_us) / 1000.0
	var terrain_interaction_ms := 0.0
	var fallback_ms := 0.0
	var closest_ms := 0.0
	var ghost_snap_ms := 0.0
	var border_height_ms := 0.0
	var half_w: float = terrain_world_size.x * 0.5
	var half_h: float = terrain_world_size.y * 0.5
	# How close to the edge (in metres) before snapping to it.
	var border_snap_dist: float = minf(half_w, half_h) * 0.08  # ~8% of half-extent

	var terrain_interaction_start_us := Time.get_ticks_usec()
	var pos_variant = get_terrain_interaction()
	terrain_interaction_ms = float(Time.get_ticks_usec() - terrain_interaction_start_us) / 1000.0

	if pos_variant == null:
		var fallback_start_us := Time.get_ticks_usec()
		# Cursor is off the terrain — project ray onto a flat plane and snap to map border.
		var mouse_screen := get_viewport().get_mouse_position()
		var camera := get_viewport().get_camera_3d()
		fallback_ms = float(Time.get_ticks_usec() - fallback_start_us) / 1000.0
		if camera == null:
			is_valid = false
			_log_mouse_pos_detail(
				profile_this_call,
				"off_terrain_no_camera",
				terrain_world_ms,
				terrain_interaction_ms,
				fallback_ms,
				closest_ms,
				ghost_snap_ms,
				border_height_ms,
				total_start_us
			)
			return Vector3.ZERO
		fallback_start_us = Time.get_ticks_usec()
		var ray_origin := camera.project_ray_origin(mouse_screen)
		var ray_dir    := camera.project_ray_normal(mouse_screen)
		fallback_ms += float(Time.get_ticks_usec() - fallback_start_us) / 1000.0
		if ray_dir.y >= -0.001:
			# Ray points upward — cannot hit the ground plane.
			is_valid = false
			_log_mouse_pos_detail(
				profile_this_call,
				"off_terrain_ray_up",
				terrain_world_ms,
				terrain_interaction_ms,
				fallback_ms,
				closest_ms,
				ghost_snap_ms,
				border_height_ms,
				total_start_us
			)
			return Vector3.ZERO
		fallback_start_us = Time.get_ticks_usec()
		var t_plane: float = -ray_origin.y / ray_dir.y
		var hit := ray_origin + ray_dir * t_plane
		# Clamp to map bounds, then snap to the nearest edge.
		hit.x = clampf(hit.x, -half_w, half_w)
		hit.z = clampf(hit.z, -half_h, half_h)
		hit = _snap_to_map_border(hit, half_w, half_h)
		fallback_ms += float(Time.get_ticks_usec() - fallback_start_us) / 1000.0
		var border_height_start_us := Time.get_ticks_usec()
		hit.y = simulation_node.get_world_surface_height(Vector2(hit.x, hit.z))
		border_height_ms = float(Time.get_ticks_usec() - border_height_start_us) / 1000.0
		is_valid = true
		_log_mouse_pos_detail(
			profile_this_call,
			"off_terrain_border",
			terrain_world_ms,
			terrain_interaction_ms,
			fallback_ms,
			closest_ms,
			ghost_snap_ms,
			border_height_ms,
			total_start_us
		)
		return hit

	var pos: Vector3 = pos_variant

	# Apply altitude offset early so angle-snap and network-snap work in real space.
	pos.y += altitude_offset

	# 1. Angle + length snap (Shift) — applied FIRST so network snap honours the constrained direction.
	if active and Input.is_key_pressed(KEY_SHIFT):
		var ref_pos: Vector3 = start_pos if current_state == State.SETTING_CONTROL else control_pos
		var dir: Vector3 = pos - ref_pos
		var length: float = dir.length()
		if length > 0.1:
			var angle := atan2(dir.z, dir.x)
			var snap_rad := PI / 12.0  # 15-degree increments
			# Snap relative to road tangent at start so 90° gives a true perpendicular.
			var relative := angle - _start_tangent_angle
			relative = round(relative / snap_rad) * snap_rad
			angle = _start_tangent_angle + relative
			# Snap length to nearest 10 m so block distances are exact.
			const LENGTH_SNAP := 10.0
			length = maxf(LENGTH_SNAP, round(length / LENGTH_SNAP) * LENGTH_SNAP)
			pos = ref_pos + Vector3(cos(angle), 0.0, sin(angle)) * length

	# 2. Snap to existing network node/edge on the (possibly angle-constrained) position.
	var closest_start_us := Time.get_ticks_usec()
	var snapped_pos = simulation_node.get_closest_network_point(pos, 5.0)
	closest_ms = float(Time.get_ticks_usec() - closest_start_us) / 1000.0
	if snapped_pos != null:
		is_valid = true
		_log_mouse_pos_detail(
			profile_this_call,
			"network_snap",
			terrain_world_ms,
			terrain_interaction_ms,
			fallback_ms,
			closest_ms,
			ghost_snap_ms,
			border_height_ms,
			total_start_us
		)
		return snapped_pos

	# 3. Ghost-line snap — snaps to both outward tangent rays AND parallel offset curves.
	if _ghost_enabled and not Input.is_key_pressed(KEY_SHIFT):
		var ghost_snap_start_us := Time.get_ticks_usec()
		var ghost_snap = simulation_node.get_road_ghost_snap(pos, 10.0, altitude_offset)
		ghost_snap_ms = float(Time.get_ticks_usec() - ghost_snap_start_us) / 1000.0
		if ghost_snap != null:
			is_valid = true
			_log_mouse_pos_detail(
				profile_this_call,
				"ghost_snap",
				terrain_world_ms,
				terrain_interaction_ms,
				fallback_ms,
				closest_ms,
				ghost_snap_ms,
				border_height_ms,
				total_start_us
			)
			return ghost_snap

	# 4. Snap to map border when cursor is within border_snap_dist of any edge.
	if _is_near_border(pos, half_w, half_h, border_snap_dist):
		pos = _snap_to_map_border(pos, half_w, half_h)
		var border_height_start_us := Time.get_ticks_usec()
		pos.y = simulation_node.get_world_surface_height(Vector2(pos.x, pos.z))
		border_height_ms = float(Time.get_ticks_usec() - border_height_start_us) / 1000.0
		is_valid = true
		_log_mouse_pos_detail(
			profile_this_call,
			"border_snap",
			terrain_world_ms,
			terrain_interaction_ms,
			fallback_ms,
			closest_ms,
			ghost_snap_ms,
			border_height_ms,
			total_start_us
		)
		return pos

	# 5. Self-snapping (to start or control point).
	if active:
		if pos.distance_to(start_pos) < 2.5:
			_log_mouse_pos_detail(
				profile_this_call,
				"self_start",
				terrain_world_ms,
				terrain_interaction_ms,
				fallback_ms,
				closest_ms,
				ghost_snap_ms,
				border_height_ms,
				total_start_us
			)
			return start_pos
		if current_state == State.SETTING_END:
			if pos.distance_to(control_pos) < 2.5:
				_log_mouse_pos_detail(
					profile_this_call,
					"self_control",
					terrain_world_ms,
					terrain_interaction_ms,
					fallback_ms,
					closest_ms,
					ghost_snap_ms,
					border_height_ms,
					total_start_us
				)
				return control_pos

	_log_mouse_pos_detail(
		profile_this_call,
		"terrain",
		terrain_world_ms,
		terrain_interaction_ms,
		fallback_ms,
		closest_ms,
		ghost_snap_ms,
		border_height_ms,
		total_start_us
	)
	return pos

## Returns true when pos is within threshold metres of any map edge.
func _is_near_border(pos: Vector3, half_w: float, half_h: float, threshold: float) -> bool:
	return (pos.x < -half_w + threshold or pos.x > half_w - threshold or
			pos.z < -half_h + threshold or pos.z > half_h - threshold)

## Snaps pos to the nearest map edge by clamping whichever axis is closest to its limit.
func _snap_to_map_border(pos: Vector3, half_w: float, half_h: float) -> Vector3:
	var d_left  := pos.x + half_w   # distance from left  edge (X = -half_w)
	var d_right := half_w - pos.x   # distance from right edge (X = +half_w)
	var d_top   := pos.z + half_h   # distance from top   edge (Z = -half_h)
	var d_bot   := half_h - pos.z   # distance from bot   edge (Z = +half_h)
	var min_d   := minf(d_left, minf(d_right, minf(d_top, d_bot)))
	if min_d == d_left:
		pos.x = -half_w
	elif min_d == d_right:
		pos.x = half_w
	elif min_d == d_top:
		pos.z = -half_h
	else:
		pos.z = half_h
	return pos

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
	if points.size() <= 1:
		_log_preview_surface_detail(profile_label, points.size(), 0, true, baked_ms, 0.0, total_start_us)
		var empty_preview := {
			"prepared_points": points,
			"surface_vertices": PackedVector3Array(),
			"is_valid": true
		}
		_remember_preview_surface(points, empty_preview)
		return empty_preview

	var rust_start_us := Time.get_ticks_usec()
	var preview = simulation_node.get_preview_road_surface(points, fwd_lanes, bkw_lanes)
	var rust_ms := float(Time.get_ticks_usec() - rust_start_us) / 1000.0
	if preview == null:
		_log_preview_surface_detail(profile_label, points.size(), 0, false, baked_ms, rust_ms, total_start_us)
		var invalid_preview := {
			"prepared_points": points,
			"surface_vertices": PackedVector3Array(),
			"is_valid": false
		}
		_remember_preview_surface(points, invalid_preview)
		return invalid_preview

	var surface_vertices: PackedVector3Array = preview.get("surface_vertices", PackedVector3Array())
	_log_preview_surface_detail(
		profile_label,
		points.size(),
		surface_vertices.size(),
		bool(preview.get("is_valid", false)),
		baked_ms,
		rust_ms,
		total_start_us
	)
	_remember_preview_surface(points, preview)
	return preview

func _remember_preview_surface(points: PackedVector3Array, preview: Dictionary) -> void:
	_preview_cache_points = points
	_preview_cache_surface = preview.duplicate(true)

func _cached_preview_surface_for_points(points: PackedVector3Array) -> Dictionary:
	if _preview_cache_surface.is_empty():
		return {}
	if not _road_surface_points_match(points, _preview_cache_points):
		return {}
	return _preview_cache_surface.duplicate(true)

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

func _log_mouse_pos_detail(
	enabled: bool,
	branch: String,
	terrain_world_ms: float,
	terrain_interaction_ms: float,
	fallback_ms: float,
	closest_ms: float,
	ghost_snap_ms: float,
	border_height_ms: float,
	total_start_us: int
) -> void:
	if not enabled or not _road_debug_enabled:
		return
	print(
		"[DEBUG:road] mouse_pos_detail branch=%s terrain_world_ms=%.3f terrain_interaction_ms=%.3f fallback_ms=%.3f closest_ms=%.3f ghost_snap_ms=%.3f border_height_ms=%.3f total_ms=%.3f"
		% [
			branch,
			terrain_world_ms,
			terrain_interaction_ms,
			fallback_ms,
			closest_ms,
			ghost_snap_ms,
			border_height_ms,
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
	total_start_us: int
) -> void:
	if not _road_debug_enabled:
		return
	var total_ms := float(Time.get_ticks_usec() - total_start_us) / 1000.0
	var log_label := label
	if log_label.is_empty() and total_ms < ROAD_PROFILE_SLOW_MS:
		return
	if log_label.is_empty():
		log_label = "slow"
	print(
		"[DEBUG:road] preview_surface_godot label=%s points=%d surface_vertices=%d valid=%s baked_ms=%.3f rust_ms=%.3f total_ms=%.3f"
		% [
			log_label,
			point_count,
			surface_vertex_count,
			str(valid),
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
