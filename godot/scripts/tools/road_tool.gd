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

# ── Angle-snap reference ─────────────────────────────────────────────────────
# Base angle (radians) for Shift snapping — set to the road tangent at start_pos
# so that 90° snap gives a true perpendicular to the road, not to the world grid.
var _start_tangent_angle: float = 0.0
# True when a real road tangent was found at start_pos (false = open terrain, show world angle).
var _has_road_tangent: bool = false

func _ready():
	super._ready()
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
				_rebuild_ghost_lines()
			elif ghost_mesh:
				ghost_mesh.visible = false

	if active and event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_handle_click()


func _handle_click():
	var pos = get_world_mouse_pos()
	if not is_valid: return
	
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
			var _st: Vector2 = simulation_node.get_road_tangent_at(start_pos, 6.0)
			_start_tangent_angle = atan2(_st.y, _st.x)  # atan2(z, x) = East-0° convention
			# Detect fallback: (0,1) means no road found within range.
			_has_road_tangent = not (_st.x == 0.0 and _st.y == 1.0)

			# Build ghost guides on first placement click (network is settled at this point).
			if _ghost_guides_dirty:
				_rebuild_ghost_lines()

			current_path = Path3D.new()
			current_path.curve = Curve3D.new()
			current_path.curve.bake_interval = 0.5
			current_path.curve.up_vector_enabled = false # Prevent 'looking_at' errors on degenerate paths
			add_child(current_path)
			
		State.SETTING_CONTROL:
			control_pos = pos
			current_state = State.SETTING_END
			
		State.SETTING_END:
			_commit_segment(pos)

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
	if not is_valid: return

	var preview := _get_compiled_preview_surface()
	var points := current_path.curve.get_baked_points() if current_path else PackedVector3Array()
	if points.size() > 1 and not preview.is_empty() and bool(preview.get("is_valid", false)):
		simulation_node.add_road(points, fwd_lanes, bkw_lanes)
		# Do NOT trigger the terrain/network visual refresh here — the road
		# is queued to the sim thread and is not in the graph yet.  _process polls
		# is_network_dirty() and NetworkRenderer rebuilds the visuals once the road lands.

		# Queue border check — must run AFTER the road is in the graph (nodes exist).
		# NetworkRenderer drains _pending_border_checks when network_dirty fires.
		_pending_border_checks.push_back([start_pos, end_pos])
		# Ghost guides must be rebuilt once the road lands in the graph.
		_ghost_guides_dirty = true
	
	var dist = start_pos.distance_to(end_pos)
	
	if draw_mode == 0:
		start_pos = end_pos
		control_pos = end_pos
		current_state = State.SETTING_END
	else:
		start_pos = end_pos
		# For G1 continuity, the NEW control point should be a projection 
		# of the PREVIOUS end-tangent.
		var final_tangent = (end_pos - control_pos).normalized()
		if final_tangent.length() < 0.001:
			final_tangent = (end_pos - start_pos).normalized()
		control_pos = end_pos + final_tangent * min(dist * 0.5, 10.0)
		current_state = State.SETTING_END

func cancel_road():
	current_state = State.IDLE
	if current_path:
		blueprint_mesh.mesh = null
		current_path.queue_free()
	current_path = null
	if _info_label:
		_info_label.visible = false

## Called by NetworkRenderer after the road is confirmed in the graph.
## Drains _pending_border_checks and shows the border-connection dialog if relevant.
## Also rebuilds ghost guides now that the new road is in the graph.
func drain_pending_border_checks() -> void:
	if _ghost_guides_dirty:
		_rebuild_ghost_lines()
	while not _pending_border_checks.is_empty():
		var pair = _pending_border_checks.pop_front()
		var candidate: int = simulation_node.check_border_candidate(pair[0])
		if candidate < 0:
			candidate = simulation_node.check_border_candidate(pair[1])
		if candidate >= 0:
			_prompt_border_connection(candidate)

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
	var terrain_world_size: Vector2 = simulation_node.get_terrain_world_size()
	var half_w: float = terrain_world_size.x * 0.5
	var half_h: float = terrain_world_size.y * 0.5
	# How close to the edge (in metres) before snapping to it.
	var border_snap_dist: float = minf(half_w, half_h) * 0.08  # ~8% of half-extent

	var pos_variant = get_terrain_interaction()

	if pos_variant == null:
		# Cursor is off the terrain — project ray onto a flat plane and snap to map border.
		var mouse_screen := get_viewport().get_mouse_position()
		var camera := get_viewport().get_camera_3d()
		if camera == null:
			is_valid = false
			return Vector3.ZERO
		var ray_origin := camera.project_ray_origin(mouse_screen)
		var ray_dir    := camera.project_ray_normal(mouse_screen)
		if ray_dir.y >= -0.001:
			# Ray points upward — cannot hit the ground plane.
			is_valid = false
			return Vector3.ZERO
		var t_plane: float = -ray_origin.y / ray_dir.y
		var hit := ray_origin + ray_dir * t_plane
		# Clamp to map bounds, then snap to the nearest edge.
		hit.x = clampf(hit.x, -half_w, half_w)
		hit.z = clampf(hit.z, -half_h, half_h)
		hit = _snap_to_map_border(hit, half_w, half_h)
		hit.y = simulation_node.get_world_surface_height(Vector2(hit.x, hit.z))
		is_valid = true
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
	var snapped_pos = simulation_node.get_closest_network_point(pos, 5.0)
	if snapped_pos != null:
		is_valid = true
		return snapped_pos

	# 3. Ghost-line snap — snaps to both outward tangent rays AND parallel offset curves.
	if _ghost_enabled and not Input.is_key_pressed(KEY_SHIFT):
		var best_dist := 10.0  # snap radius in metres — wide enough to be usable top-down
		var best_x := 0.0
		var best_z := 0.0
		var found := false

		# 3a. Outward tangent rays from road endpoints.
		var guides: PackedFloat32Array = simulation_node.get_road_ghost_guides()
		var n_guides := guides.size() / 4
		for i in range(n_guides):
			var ax: float = guides[i * 4 + 0]
			var az: float = guides[i * 4 + 1]
			var dx: float = guides[i * 4 + 2]
			var dz: float = guides[i * 4 + 3]
			var px := pos.x - ax;  var pz := pos.z - az
			var t := px * dx + pz * dz
			if t < 0.0: continue
			var cx := ax + dx * t;  var cz := az + dz * t
			var d := sqrt((pos.x - cx) * (pos.x - cx) + (pos.z - cz) * (pos.z - cz))
			if d < best_dist:
				best_dist = d;  best_x = cx;  best_z = cz;  found = true

		# 3b. Parallel offset curves (80 m, 160 m, 240 m from each edge).
		# Uses the same polyline data as the visual overlay so snap aligns exactly.
		const GHOST_OFFSETS := [80.0, 160.0, 240.0]
		var polylines: PackedFloat32Array = simulation_node.get_road_edge_polylines()
		var idx := 0
		while idx < polylines.size():
			var n_pts := int(polylines[idx]); idx += 1
			if n_pts < 2: idx += n_pts * 2; continue
			var edge_pts: Array = []
			for j in range(n_pts):
				edge_pts.append(Vector2(polylines[idx], polylines[idx + 1])); idx += 2
			for off_idx in range(GHOST_OFFSETS.size()):
				for side in [-1.0, 1.0]:
					var actual_offset: float = GHOST_OFFSETS[off_idx] * side
					for k in range(edge_pts.size() - 1):
						var a: Vector2 = edge_pts[k]
						var b: Vector2 = edge_pts[k + 1]
						var seg: Vector2 = b - a
						if seg.length_squared() < 0.01: continue
						var seg_norm: Vector2 = seg.normalized()
						var perp: Vector2 = Vector2(-seg_norm.y, seg_norm.x)
						var oa: Vector2 = a + perp * actual_offset
						var ob: Vector2 = b + perp * actual_offset
						# Skip collapsed inside-curve segments.
						if (ob - oa).dot(seg_norm) < 0.0: continue
						# Find closest point on this offset segment to cursor.
						var pa: Vector2 = Vector2(pos.x, pos.z) - oa
						var ab: Vector2 = ob - oa
						var t2: float = clampf(pa.dot(ab) / ab.length_squared(), 0.0, 1.0)
						var closest: Vector2 = oa + ab * t2
						var d: float = Vector2(pos.x - closest.x, pos.z - closest.y).length()
						if d < best_dist:
							best_dist = d;  best_x = closest.x;  best_z = closest.y;  found = true

		if found:
			is_valid = true
			return Vector3(best_x, simulation_node.get_world_surface_height(Vector2(best_x, best_z)) + altitude_offset, best_z)

	# 4. Snap to map border when cursor is within border_snap_dist of any edge.
	if _is_near_border(pos, half_w, half_h, border_snap_dist):
		pos = _snap_to_map_border(pos, half_w, half_h)
		pos.y = simulation_node.get_world_surface_height(Vector2(pos.x, pos.z))
		is_valid = true
		return pos

	# 5. Self-snapping (to start or control point).
	if active:
		if pos.distance_to(start_pos) < 2.5:
			return start_pos
		if current_state == State.SETTING_END:
			if pos.distance_to(control_pos) < 2.5:
				return control_pos

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
func _get_compiled_preview_surface() -> Dictionary:
	if current_path == null:
		return {}

	var points: PackedVector3Array = current_path.curve.get_baked_points()
	if points.size() <= 1:
		return {
			"prepared_points": points,
			"surface_vertices": PackedVector3Array(),
			"is_valid": true
		}

	var preview = simulation_node.get_preview_road_surface(points, fwd_lanes, bkw_lanes)
	if preview == null:
		return {
			"prepared_points": points,
			"surface_vertices": PackedVector3Array(),
			"is_valid": false
		}

	return preview

# ── Ghost guide lines ────────────────────────────────────────────────────────

## Rebuilds the ImmediateMesh for ghost guide lines.
## Called when the tool activates, when G is toggled, and after a road is committed.
func _rebuild_ghost_lines() -> void:
	if not ghost_mesh:
		return
	if not _ghost_enabled:
		ghost_mesh.visible = false
		return

	var guides: PackedFloat32Array = simulation_node.get_road_ghost_guides()
	if guides.size() == 0:
		ghost_mesh.visible = false
		_ghost_guides_dirty = false
		return

	# City block grid spacing — 80 m matches the most common compact urban standard
	# (Manhattan short blocks ~80 m, Portland ~60 m, Barcelona ~113 m).
	const GRID_SPACING    := 80.0   # metres between parallel road guides
	const MAX_OFFSETS     := 3      # 80 m, 160 m, 240 m — 320 m is invisible so skipped
	const OUTWARD_EXTEND  := 200.0  # metres the outward tangent extends ahead
	const TICK_INTERVAL   := 20.0   # tick every 20 m on the outward guide
	const TICK_HALF       := 1.5    # half-width of each tick mark

	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_LINES)

	var n_guides := guides.size() / 4
	for i in range(n_guides):
		var ax: float = guides[i * 4 + 0]
		var az: float = guides[i * 4 + 1]
		var dx: float = guides[i * 4 + 2]
		var dz: float = guides[i * 4 + 3]
		var ay: float = simulation_node.get_world_surface_height(Vector2(ax, az)) + 0.06
		var perp_x := -dz
		var perp_z :=  dx

		# ── Outward tangent guide (extends ahead from the endpoint) ──────────
		var ex := ax + dx * OUTWARD_EXTEND
		var ez := az + dz * OUTWARD_EXTEND
		var ey: float = simulation_node.get_world_surface_height(Vector2(ex, ez)) + 0.06
		var guide_col := Color(1.0, 1.0, 1.0, 0.30)
		im.surface_set_color(guide_col)
		im.surface_add_vertex(Vector3(ax, ay, az))
		im.surface_set_color(guide_col)
		im.surface_add_vertex(Vector3(ex, ey, ez))

		# Tick marks along the outward guide at each grid interval
		var tick_col := Color(1.0, 1.0, 1.0, 0.30)
		var dist := GRID_SPACING
		while dist <= OUTWARD_EXTEND:
			var tx := ax + dx * dist
			var tz := az + dz * dist
			var ty: float = simulation_node.get_world_surface_height(Vector2(tx, tz)) + 0.07
			im.surface_set_color(tick_col)
			im.surface_add_vertex(Vector3(tx - perp_x * TICK_HALF, ty, tz - perp_z * TICK_HALF))
			im.surface_set_color(tick_col)
			im.surface_add_vertex(Vector3(tx + perp_x * TICK_HALF, ty, tz + perp_z * TICK_HALF))
			dist += GRID_SPACING

	# ── Parallel offset guides from full edge polylines (follows road curvature) ─
	# Each edge's physical geometry is offset perpendicular to the road direction
	# at ±GRID_SPACING intervals, so curved/splined roads get curved parallel guides.
	var polylines: PackedFloat32Array = simulation_node.get_road_edge_polylines()
	var idx := 0
	while idx < polylines.size():
		var n_pts := int(polylines[idx]); idx += 1
		if n_pts < 2:
			idx += n_pts * 2
			continue

		# Collect XZ points for this edge
		var pts: Array = []
		for j in range(n_pts):
			pts.append(Vector2(polylines[idx], polylines[idx + 1]))
			idx += 2

		# Draw offset curves at ±80 m, ±160 m, ±240 m with diminishing alpha.
		# Alpha: k=1 → 0.30, k=2 → 0.12, k=3 → 0.04 (almost invisible at 240 m).
		var alphas := [0.30, 0.12, 0.04]
		for k in range(1, MAX_OFFSETS + 1):
			var col := Color(1.0, 1.0, 1.0, alphas[k - 1])
			for side in [-1.0, 1.0]:
				_draw_offset_curve(im, pts, side * k * GRID_SPACING, col)

	im.surface_end()
	ghost_mesh.mesh = im
	ghost_mesh.visible = true
	_ghost_guides_dirty = false

## Draws an offset curve parallel to the polyline `pts` (Array of Vector2 XZ),
## displaced by `offset` metres perpendicular to the road tangent.
## Uses per-segment perpendiculars and skips crossing pairs so tight inside curves
## don't produce X artifacts when the offset exceeds the local radius of curvature.
## `col` controls the line colour and alpha (used for distance-based fading).
func _draw_offset_curve(im: ImmediateMesh, pts: Array, offset: float, col: Color) -> void:
	const Y_LIFT := 0.06
	var n := pts.size()

	# 1. Compute all offset segments, discarding any whose direction reversed.
	var off_segs: Array = []   # each entry: [oa: Vector2, ob: Vector2, norm: Vector2]
	for i in range(n - 1):
		var a: Vector2 = pts[i]
		var b: Vector2 = pts[i + 1]
		var seg := b - a
		if seg.length_squared() < 0.01:
			continue
		var seg_norm := seg.normalized()
		var perp := Vector2(-seg_norm.y, seg_norm.x)
		var oa := a + perp * offset
		var ob := b + perp * offset
		if (ob - oa).dot(seg_norm) < 0.0:
			continue  # Collapsed past curve centre — direction reversed.
		off_segs.append([oa, ob, seg_norm])

	# 2. Draw segments, skipping any pair that physically crosses its neighbour.
	#    Two successive inside-curve segments can intersect even though each
	#    individually hasn't reversed — the classic "offset X" on tight curves.
	var skip_next := false
	for i in range(off_segs.size()):
		if skip_next:
			skip_next = false
			continue
		var s: Array = off_segs[i]
		if i + 1 < off_segs.size():
			var nxt: Array = off_segs[i + 1]
			if _segs_cross_2d(s[0], s[1], nxt[0], nxt[1]):
				skip_next = true   # suppress this segment and the next
				continue
		var oa: Vector2 = s[0]
		var ob: Vector2 = s[1]
		im.surface_set_color(col)
		im.surface_add_vertex(Vector3(oa.x, simulation_node.get_world_surface_height(oa) + Y_LIFT, oa.y))
		im.surface_set_color(col)
		im.surface_add_vertex(Vector3(ob.x, simulation_node.get_world_surface_height(ob) + Y_LIFT, ob.y))

## Returns true when 2-D segments (a1→b1) and (a2→b2) properly intersect.
func _segs_cross_2d(a1: Vector2, b1: Vector2, a2: Vector2, b2: Vector2) -> bool:
	var d1 := b1 - a1
	var d2 := b2 - a2
	var denom := d1.x * d2.y - d1.y * d2.x
	if absf(denom) < 1e-6:
		return false  # parallel
	var t := ((a2.x - a1.x) * d2.y - (a2.y - a1.y) * d2.x) / denom
	var u := ((a2.x - a1.x) * d1.y - (a2.y - a1.y) * d1.x) / denom
	return t > 0.0 and t < 1.0 and u > 0.0 and u < 1.0
