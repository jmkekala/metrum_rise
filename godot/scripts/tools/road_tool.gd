# SPDX-License-Identifier: GPL-2.0-only

## Road drawing tool — straight and curve modes with a synchronous WYSIWYG preview and lane
## configuration.
##
## Extends NetworkTool. Adds: the synchronous road preview (`ROAD-68-E`): every frame whose
## stroke changed calls Rust's `preview_road`, which plans the stroke on the road tool's
## context and returns the changed road chunks exactly as the commit publishes them, and shows
## them in that frame in place of the resident chunks. A click submits that preview; the
## preview stays shown until the committed generation's chunks are installed, then both swap
## in one call. The preview's graded terrain tiles (`TERRAIN-07-E`) show on the terrain
## renderer's graded tiles from the frame that staged them; frames without a new pose stage the
## pose's rest, and the commit's edit replaces them with the committed tiles in one frame. Also
## checkbox-controlled zoning-grid snapping, angle snapping (Shift draws free) and the
## distance + angle HUD label.
## Rust methods called: preview_road(), advance_road_preview_terrain(),
## release_road_preview_terrain(), add_road_with_snap(), get_road_commit_result(),
## get_road_tool_cursor_pos(), get_road_tool_start_direction().
## Placement stays pending until Rust acknowledges acceptance; rejection retains the editable stroke.
## Drawing is continuous: an accepted road starts the next one at its end, using its end
## direction for straight-road snapping, until right-click/Esc, a connection to existing roads,
## or the map edge (the external-connection prompt).
## Curve mode always takes a free control-point click, including each stroke in a chain.
## State machine: IDLE → [SETTING_CONTROL] → SETTING_END → accepted commit → next stroke.
extends "res://scripts/tools/network_tool.gd"

const PreviewDisplay := preload("res://scripts/renderers/road_preview_display.gd")
# Early, so the preview of this frame's pointer is staged before the frame renders.
const PREVIEW_SAMPLE_PROCESS_PRIORITY := -100

enum State { IDLE, SETTING_CONTROL, SETTING_END }
var current_state = State.IDLE

var start_pos: Vector3
var control_pos: Vector3

var current_path: Path3D

var fwd_lanes: int = 1
var bkw_lanes: int = 1
var altitude_offset: float = 0.0

var draw_mode: int = 0 # 0: straight, 1: spline
var zoning_grid_snap_enabled: bool = false

# Border-check positions queued while roads are in-flight on the sim thread.
# Drained by NetworkRenderer._process once the road is confirmed in the graph.
var _pending_border_checks: Array = []

# ── HUD label: distance + angle readout (screen-space, zoom-independent) ────
var _hud_canvas: CanvasLayer = null
var _info_label: Label = null
# World-space preview-label anchor — used each frame to project to screen.
var _label_world_pos: Vector3 = Vector3.ZERO

var preview_metrics: RefCounted
var _preview_input_us := 0
var _preview_metric_row: Dictionary = {}

# Read live so the debug console takes effect on the next road action.
var _road_debug_enabled: bool:
	get:
		return MetrumDebug.is_enabled("road")
var _preview_update_pending: bool = false
# The latest synchronous preview (`preview_road`'s dictionary) and the exact stroke it is for.
# A new context generation or a different stroke makes it stale.
var _preview: Dictionary = {}
var _preview_points := PackedVector3Array()
var _preview_fwd_lanes: int = -1
var _preview_bkw_lanes: int = -1
var _preview_snap_to_roads: bool = true
# Counts preview_road calls; it names displayed results for the metrics trace only.
var _preview_sequence: int = 0
var _preview_drawn_sequence: int = 0
var _preview_display = PreviewDisplay.new()
# An accepted commit's preview, held until the chunks of `_swap_generation` are installed.
var _swap_display = null
var _swap_generation: int = -1
var _commit_request_id: int = 0
var _commit_path_id: int = 0
var _commit_points := PackedVector3Array()
var _commit_validation: Dictionary = {}
var _commit_border_check: Array = []
var _last_zoning_snap_enabled: bool = false
var _road_preview_material: ShaderMaterial
# The latest pose's graded tiles still to stage (`TERRAIN-07-E`, T27 (d)): each frame without a
# new pose stages the next share until none is left.
var _graded_pending := 0
# The preview epoch a queued commit adopted: its graded tiles show until the commit's edit
# installs the committed ones, or until the commit is refused.
var _graded_commit_epoch := -1

const ROAD_SURFACE_CURVE_STEP_M := 4.0
const ROAD_SURFACE_POINT_EPS_M := 0.05
const MAP_BORDER_SNAP_DIST_M := 25.0
const ROAD_NETWORK_SNAP_RELEASE_DIST_M := 8.0
const ROAD_SELF_SNAP_DIST_M := 2.5

# ── Angle display reference ─────────────────────────────────────────────────
# Base angle (radians) for the HUD — set to the road tangent at start_pos
# so the HUD reports angles relative to the road.
var _start_tangent_angle: float = 0.0
# True when a real road tangent was found at start_pos (false = open terrain, show world angle).
var _has_road_tangent: bool = false
# The Rust cursor result retains a node/edge target only within its snapshot generation.
var _sticky_network_snap: Dictionary = {}

# Reference direction from Rust for straight-road snapping and the HUD.
var _start_dir: Vector3 = Vector3.ZERO
# Direction the current stroke ends in (unit XZ), handed to the next stroke when drawing continues.
var _stroke_end_dir: Vector3 = Vector3.ZERO
var _commit_end_dir: Vector3 = Vector3.ZERO
var _commit_ends_on_network: bool = false
var _angle_snapped: bool = false

func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE:
		_release_graded_preview()

func begin_preview_measurement() -> RefCounted:
	preview_metrics = preload("res://scripts/benchmarks/road_preview_metrics.gd").new()
	if DisplayServer.get_name() != "headless" and not RenderingServer.frame_post_draw.is_connected(_preview_frame_completed):
		RenderingServer.frame_post_draw.connect(_preview_frame_completed)
	if DisplayServer.get_name() != "headless":
		preview_metrics.viewport = get_viewport().get_viewport_rid()
		RenderingServer.viewport_set_measure_render_time(preview_metrics.viewport, true)
	return preview_metrics

func _preview_frame_completed() -> void:
	if preview_metrics != null:
		preview_metrics.frame(_preview_drawn_sequence, DisplayServer.get_name() != "headless")

func _ready():
	super._ready()
	process_priority = PREVIEW_SAMPLE_PROCESS_PRIORITY
	if blueprint_mesh:
		_road_preview_material = WorldMaterials.road_preview_material()
		blueprint_mesh.material_override = _road_preview_material
	if MetrumDebug.is_enabled("perf"):
		begin_preview_measurement()
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
	if preview_metrics != null and DisplayServer.get_name() == "headless":
		preview_metrics.frame(_preview_drawn_sequence, false)
	_poll_road_commit_result()
	_release_swapped_preview()
	var previous_mouse_pos := _last_world_mouse_pos
	var had_mouse_pos := _has_last_world_mouse_pos
	super._process(delta)
	if not active:
		_clear_sticky_network_snap()
		if _info_label:
			_info_label.visible = false
	# The base tool has now resolved this frame's pointer, including camera-only movement.
	if current_path != null and _has_last_world_mouse_pos and (not had_mouse_pos or previous_mouse_pos != _last_world_mouse_pos):
		_queue_preview_update()
	var zoning_snap := _zoning_snap_enabled()
	if zoning_snap != _last_zoning_snap_enabled:
		_last_zoning_snap_enabled = zoning_snap
		_clear_sticky_network_snap()
		if current_path != null:
			_queue_preview_update()
	# A publication (commit, undo, terrain edit) moves the context on: preview again.
	if current_path != null and not _preview.is_empty() and int(_preview.get("surface_generation", -1)) != simulation_node.get_road_tool_surface_generation():
		_preview_update_pending = true
	if _preview_update_pending:
		_preview_update_pending = false
		_update_preview()
	elif _graded_pending > 0 and _commit_request_id == 0 and current_path != null:
		# A frame without a new pose stages the next share of the pose's graded tiles.
		_show_graded(simulation_node.advance_road_preview_terrain())
	# Project the HUD label world position to screen space each frame so the label
	# stays the same pixel size regardless of camera zoom.
	if _info_label and _info_label.visible and is_inside_tree():
		var camera := get_viewport().get_camera_3d()
		if camera:
			var screen_pos: Vector2 = camera.unproject_position(_label_world_pos)
			# Offset slightly so the label sits above-right of the midpoint.
			_info_label.position = screen_pos + Vector2(8.0, -24.0)

func _snap_to_roads_enabled() -> bool:
	return true

func _zoning_snap_enabled() -> bool:
	return draw_mode == 0 and zoning_grid_snap_enabled

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
	if _scripted_pointer_enabled:
		return
	# Pointer motion is sampled by _process, after the base tool resolves the current frame.

	if active and event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_handle_click()


func _handle_click():
	if _scripted_pointer_enabled or _commit_request_id > 0:
		return
	var total_start_us := Time.get_ticks_usec()
	var state_before: int = current_state
	var mouse_start_us := Time.get_ticks_usec()
	var pos = get_world_mouse_pos()
	# A click can arrive before _process: commit the click's fresh cursor, not last frame's.
	_last_world_mouse_pos = pos
	_has_last_world_mouse_pos = is_valid
	var mouse_ms := float(Time.get_ticks_usec() - mouse_start_us) / 1000.0
	var tangent_ms := 0.0
	var path_ms := 0.0
	var commit_ms := 0.0
	if not is_valid:
		_log_click_detail(
			state_before,
			false,
			mouse_ms,
			tangent_ms,
			path_ms,
			commit_ms,
			total_start_us
		)
		return
	
	match current_state:
		State.IDLE:
			var tangent_start_us := Time.get_ticks_usec()
			# Read the start's snap target before the stroke clears it.
			var snap := _sticky_network_snap.duplicate()
			var direction: Dictionary = simulation_node.get_road_tool_start_direction(
				pos,
				int(snap.get("snap_node", -1)),
				int(snap.get("snap_edge", -1)),
				int(snap.get("surface_generation", -1))
			)
			tangent_ms = float(Time.get_ticks_usec() - tangent_start_us) / 1000.0
			var path_start_us := Time.get_ticks_usec()
			_begin_stroke(pos, int(direction.get("kind", 0)), direction.get("direction", Vector3.ZERO))
			path_ms = float(Time.get_ticks_usec() - path_start_us) / 1000.0

		State.SETTING_CONTROL:
			control_pos = pos
			_clear_sticky_network_snap()
			current_state = State.SETTING_END
			_queue_preview_update()
			
		State.SETTING_END:
			var commit_start_us := Time.get_ticks_usec()
			_commit_segment(pos)
			commit_ms = float(Time.get_ticks_usec() - commit_start_us) / 1000.0

	_log_click_detail(
		state_before,
		is_valid,
		mouse_ms,
		tangent_ms,
		path_ms,
		commit_ms,
		total_start_us
	)

func _update_preview():
	if current_path == null or _commit_request_id > 0:
		return
	var points := _refresh_preview_curve()
	if points.size() < 2:
		_clear_preview_visual()
		return
	if preview_metrics != null:
		preview_metrics.sample(points, _preview_input_us if _preview_input_us > 0 else Time.get_ticks_usec())
	_preview_road(points)
	_show_preview(points)

# Previews `points` unless the latest preview is for exactly this stroke on the current context.
func _preview_road(points: PackedVector3Array) -> Dictionary:
	if _preview_is_for(points):
		return _preview
	_preview_sequence += 1
	var start_us := Time.get_ticks_usec()
	_preview = simulation_node.preview_road(points, fwd_lanes, bkw_lanes, _snap_to_roads_enabled())
	if preview_metrics != null:
		preview_metrics.dispatch(_preview_sequence, start_us, Time.get_ticks_usec(), true)
		preview_metrics.poll(_preview_sequence, 0, _preview)
	_preview["sequence"] = _preview_sequence
	_preview_points = points
	_preview_fwd_lanes = fwd_lanes
	_preview_bkw_lanes = bkw_lanes
	_preview_snap_to_roads = _snap_to_roads_enabled()
	return _preview

func _preview_is_for(points: PackedVector3Array) -> bool:
	return (
		not _preview.is_empty()
		and points == _preview_points
		and fwd_lanes == _preview_fwd_lanes
		and bkw_lanes == _preview_bkw_lanes
		and _snap_to_roads_enabled() == _preview_snap_to_roads
		and int(_preview.get("surface_generation", -1)) == simulation_node.get_road_tool_surface_generation()
	)

# Shows the latest preview's chunks in this frame; an invalid stroke shows only its reason.
func _show_preview(points: PackedVector3Array) -> void:
	is_valid = bool(_preview.get("is_valid", false))
	_update_road_preview_material(_preview)
	var sequence := int(_preview.get("sequence", 0))
	if sequence != _preview_drawn_sequence:
		if preview_metrics != null:
			_preview_metric_row = preview_metrics.request(sequence)
		var stage_us := Time.get_ticks_usec()
		var shown := false
		# The pose's graded tiles show in its frame, whatever the road chunks do: Rust handed
		# them over once.
		var graded_ms := _show_graded(_preview)
		if is_valid and bool(_preview.get("has_road_chunks", false)):
			shown = _preview_display.show(self, _preview, sequence)
		if not shown:
			_preview_display.clear()
		_preview_drawn_sequence = sequence
		if preview_metrics != null:
			_preview_metric_row["stage_install_ms"] = float(Time.get_ticks_usec() - stage_us) / 1000.0
			_preview_metric_row["graded_install_ms"] = graded_ms
			preview_metrics.installed(sequence)
	_update_preview_measurement_label(_preview.get("prepared_points", points), _preview)

# The terrain renderer's graded tiles, or null without one (tests without terrain).
func _graded_renderer():
	if not is_instance_valid(terrain_node):
		return null
	return terrain_node.get("graded_tiles")

# Shows one frame's graded tile changes (a pose's or a catch-up frame's) and returns the
# milliseconds the renderer took.
func _show_graded(frame: Dictionary) -> float:
	_graded_pending = int(frame.get("graded_pending", 0))
	var graded = _graded_renderer()
	return graded.show_preview(frame) if graded != null else 0.0

# Ends the preview's graded tiles: Rust starts a new epoch and the renderer drops the tiles of the
# one that ends (a queued commit's epoch stays until its edit installs).
func _release_graded_preview() -> void:
	_graded_pending = 0
	if not is_instance_valid(simulation_node):
		return
	var epoch: int = simulation_node.release_road_preview_terrain()
	var graded = _graded_renderer()
	if graded != null:
		graded.release_preview(epoch, epoch)

func _clear_preview_visual() -> void:
	_preview_display.clear()
	_release_graded_preview()
	_preview_drawn_sequence = 0
	if blueprint_mesh:
		blueprint_mesh.mesh = null
	if _info_label:
		_info_label.visible = false

# After an accepted commit the preview stays shown until the committed generation's chunks are
# installed; `_road_mesh_installed` releases it in the same call, so no frame shows neither.
func _hold_preview_until(generation: int) -> void:
	if _swap_display != null:
		_swap_display.reset()
	_swap_display = _preview_display
	_swap_generation = generation
	_preview_display = PreviewDisplay.new()
	_preview_drawn_sequence = 0
	_release_swapped_preview()

func _release_swapped_preview() -> void:
	if _swap_display != null and _road_mesh_generation >= _swap_generation:
		_swap_display.reset()
		_swap_display = null
		_swap_generation = -1

func _road_mesh_installed(_generation: int) -> void:
	# The acknowledgement may arrive in the same frame as the chunks it names.
	_poll_road_commit_result()
	_release_swapped_preview()
	# A preview shown meanwhile (the next stroke of a chain) hides the chunks just installed.
	_preview_display.refresh_originals(self)

func _begin_stroke(pos: Vector3, kind: int, direction: Vector3) -> void:
	start_pos = pos
	control_pos = pos
	active = true
	_clear_sticky_network_snap()
	_has_road_tangent = kind != 0 and direction.length_squared() > 0.0
	_start_dir = Vector3(direction.x, 0.0, direction.z).normalized() if _has_road_tangent else Vector3.ZERO
	_start_tangent_angle = atan2(_start_dir.z, _start_dir.x)  # East-0° convention
	_stroke_end_dir = Vector3.ZERO
	_angle_snapped = false
	if draw_mode == 0:
		current_state = State.SETTING_END
	else:
		current_state = State.SETTING_CONTROL
	current_path = Path3D.new()
	current_path.curve = Curve3D.new()
	current_path.curve.bake_interval = 0.5
	current_path.curve.up_vector_enabled = false # Prevent 'looking_at' errors on degenerate paths
	add_child(current_path)
	_queue_preview_update()

func _refresh_preview_curve() -> PackedVector3Array:
	if current_path == null:
		return PackedVector3Array()
	var mouse_pos: Vector3 = _last_world_mouse_pos if _has_last_world_mouse_pos else get_world_mouse_pos()
	
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
					_stroke_end_dir = Vector3(mouse_pos.x - start_pos.x, 0.0, mouse_pos.z - start_pos.z).normalized()
				else:
					var t_start = (control_pos - start_pos)
					var t_end = (mouse_pos - control_pos) # Direction of the second half
					
					current_path.curve.add_point(start_pos, Vector3.ZERO, t_start)
					current_path.curve.add_point(mouse_pos, -t_end, Vector3.ZERO)
					_stroke_end_dir = Vector3(t_end.x, 0.0, t_end.z).normalized()

	return _road_surface_points_from_curve(current_path.curve)

func _queue_preview_update() -> void:
	if current_path == null:
		return
	if preview_metrics != null:
		_preview_input_us = Time.get_ticks_usec()
	# Coalesce input bursts; validation, curve baking and mesh upload happen once in _process.
	_preview_update_pending = true

func _update_blueprint_visuals() -> void:
	# Road status is applied with its matching geometry, after this frame's validation.
	pass

func _update_road_preview_material(preview: Dictionary) -> void:
	if _road_preview_material == null:
		return
	_road_preview_material.set_shader_parameter("forward_lanes", fwd_lanes)
	_road_preview_material.set_shader_parameter("backward_lanes", bkw_lanes)
	var state := 1 if bool(preview.get("is_pending", false)) else (0 if bool(preview.get("is_valid", false)) else 2)
	_road_preview_material.set_shader_parameter("placement_state", state)

func _update_preview_measurement_label(points: PackedVector3Array, preview: Dictionary = {}, plan_state: String = "") -> void:
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

	var snap_str := " [zoning grid]" if (active and _zoning_snap_enabled()) else (" [snapped]" if _angle_snapped else "")
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
		# A grade above the design tier is shown, not refused (ROAD-68 question 37).
		if String(preview.get("warning_reason", "")) == "road_steep":
			_info_label.add_theme_color_override("font_color", Color(1.0, 0.76, 0.18, 0.98))
			_info_label.text += "  steep %.0f%% (design %.0f%%)" % [
				float(preview.get("max_grade", 0.0)) * 100.0,
				float(preview.get("warning_limit", 0.0)) * 100.0,
			]
		var state: String = plan_state if not plan_state.is_empty() else preview.get("plan_state", "")
		if not state.is_empty() and state != "ready":
			_info_label.add_theme_color_override("font_color", Color(1.0, 0.76, 0.18, 0.98))
			_info_label.text += " [checking road]"

	# Store the preview endpoint — projected to screen each frame in _process.
	var label_pos: Vector3 = points[points.size() - 1]
	label_pos.y += 2.0
	_label_world_pos = label_pos
	_info_label.visible = true

func _preview_invalid_text(preview: Dictionary) -> String:
	var reason := String(preview.get("invalid_reason", ""))
	match reason:
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
		"field_overlap":
			return "Road overlaps a farm field"
		"zoning_cell_overlap":
			return "Road overlaps painted zoning cells"
		"surface_geometry_invalid":
			return "Road surface compilation failed"
		"terrain_clip_missing_outer_boundary_owner":
			return "Road terrain boundary could not be built"
		"parcel_overlap":
			return "Road overlaps existing zoning parcels"
		"road_grade_change":
			return "Road bends too sharply up or down"
		"road_too_steep":
			return "Road too steep: %.0f%%, at most %.0f%%" % [
				float(preview.get("geometry_measured", 0.0)) * 100.0,
				float(preview.get("geometry_limit", 0.0)) * 100.0,
			]
		"road_tight_curve":
			return "Curve too tight: radius %.1f m, needs %.1f m" % [
				float(preview.get("geometry_measured", 0.0)),
				float(preview.get("geometry_limit", 0.0)),
			]
		"junction_arm_angle":
			return "Roads meet at %.0f°; junction arms need %.0f°" % [
				float(preview.get("geometry_measured", 0.0)),
				float(preview.get("geometry_limit", 0.0)),
			]
		"junction_corner_does_not_fit":
			return "Junction corner does not fit; lengthen the roads between junctions"
		"junction_core_not_simple":
			return "Junction roads overlap; widen the angle between them"
		"junction_band_mismatch":
			return "These road types cannot meet at a junction yet"
		"junction_arm_too_short":
			return "Road too short for its junction"
		"junction_profile_mismatch":
			return "Junction heights do not fit; adjust the road grade"
		"junction_layout_invalid":
			return "Junction could not be laid out"
		"junction_degree":
			return "Too many roads at one junction (%d, at most %d)" % [
				int(preview.get("geometry_measured", 0.0)),
				int(preview.get("geometry_limit", 0.0)),
			]
		"same_node_connection":
			return "Road endpoints connect to the same junction"
		"too_short":
			return "Road too short"
		"validation_unavailable":
			return "Road check unavailable"
		"road_commit_rejected":
			return "Road could not be built; adjust the road and try again"
		_:
			return "Road cannot be placed"

func _commit_segment(end_pos: Vector3) -> bool:
	if _commit_request_id > 0:
		return false
	var total_start_us := Time.get_ticks_usec()
	_last_world_mouse_pos = end_pos
	_has_last_world_mouse_pos = true

	var baked_start_us := Time.get_ticks_usec()
	var points := _refresh_preview_curve()
	var raw_points := current_path.curve.get_baked_points() if current_path else PackedVector3Array()
	var baked_ms := float(Time.get_ticks_usec() - baked_start_us) / 1000.0
	var preview_start_us := Time.get_ticks_usec()
	# The click submits the preview of exactly this stroke; Rust attaches it to the command.
	var validation := _preview_road(points) if points.size() > 1 else {"is_valid": false, "invalid_reason": "too_short"}
	var preview_ms := float(Time.get_ticks_usec() - preview_start_us) / 1000.0
	var queued := false
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
		_show_preview(points)
		# Rust logs the full report on events: why, where and the exact stroke for replay.
		simulation_node.log_road_tool_rejection(points, fwd_lanes, bkw_lanes, _snap_to_roads_enabled(), validation)
	if points.size() > 1 and preview_valid:
		var add_road_start_us := Time.get_ticks_usec()
		_commit_request_id = simulation_node.add_road_with_snap(points, fwd_lanes, bkw_lanes, _snap_to_roads_enabled())
		# The commit adopts this epoch's graded tiles; they show until its edit installs.
		_graded_commit_epoch = simulation_node.release_road_preview_terrain()
		_graded_pending = 0
		_commit_path_id = current_path.get_instance_id()
		_commit_points = points
		_commit_validation = validation
		_commit_end_dir = _stroke_end_dir
		# Ending on an existing road or junction ends the chain, as it does in other city builders.
		_commit_ends_on_network = (
			int(_sticky_network_snap.get("snap_node", -1)) >= 0
			or int(_sticky_network_snap.get("snap_edge", -1)) >= 0
		)
		add_road_ms = float(Time.get_ticks_usec() - add_road_start_us) / 1000.0
		# Do NOT trigger the terrain/network visual refresh here — the road
		# is queued to the sim thread and is not in the graph yet.  _process polls
		# is_network_dirty() and NetworkRenderer rebuilds the visuals once the road lands.

		var bookkeeping_start_us := Time.get_ticks_usec()
		# Queue border check — must run AFTER the road is in the graph (nodes exist).
		# NetworkRenderer drains _pending_border_checks when network_dirty fires.
		_commit_border_check = [start_pos, end_pos]
		_pending_border_checks.push_back(_commit_border_check)
		bookkeeping_ms = float(Time.get_ticks_usec() - bookkeeping_start_us) / 1000.0
		queued = true
		_update_preview_measurement_label(validation.get("prepared_points", points), {"is_pending": true})

	if _road_debug_enabled:
		MetrumDebug.log(
			"road",
			"commit_segment_detail raw_points=%d points=%d preview_empty=%s preview_valid=%s invalid_reason=%s max_grade=%.3f allowed_grade=%.3f span=(%.3f,%.3f) run=%.3f dy=%.3f span_y=(%.3f,%.3f) span_terrain=(%.3f,%.3f) span_delta=(%.3f,%.3f) endpoint_snap=(%d,%d) endpoint_delta=(%.3f,%.3f) queued=%s preview_ms=%.3f baked_ms=%.3f add_road_ms=%.3f bookkeeping_ms=%.3f cancel_ms=%.3f total_ms=%.3f"
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
				str(queued),
				preview_ms,
				baked_ms,
				add_road_ms,
				bookkeeping_ms,
				cancel_ms,
				float(Time.get_ticks_usec() - total_start_us) / 1000.0,
			]
		)
	return queued

func _poll_road_commit_result() -> void:
	if _commit_request_id == 0:
		return
	var result: Variant = simulation_node.get_road_commit_result(_commit_request_id)
	if not result is Dictionary:
		return
	var committed := bool(result.get("committed", false))
	if _road_debug_enabled:
		MetrumDebug.log("road", "road_commit_result request_id=%d committed=%s detail=%s" % [_commit_request_id, committed, result.get("detail", "")])
	_commit_request_id = 0
	if not committed:
		_pending_border_checks.erase(_commit_border_check)
		var graded = _graded_renderer()
		if graded != null and _graded_commit_epoch >= 0:
			graded.release_preview(_graded_commit_epoch, _graded_commit_epoch)
	else:
		# The preview shows until the commit's own chunks are installed (`ROAD-68-E`).
		_hold_preview_until(int(result.get("generation", 0)))
	# Cancellation or a new drawing gesture must not be undone by an older completion.
	if current_path != null and current_path.get_instance_id() == _commit_path_id:
		if committed:
			var end_pos := _commit_points[_commit_points.size() - 1] if _commit_points.size() > 0 else Vector3.ZERO
			var chain := _commit_points.size() > 1 and not _commit_ends_on_network and not _commit_reaches_border()
			cancel_road()
			if chain and active:
				_begin_stroke(end_pos, 1, _commit_end_dir)
		else:
			# The rejection stays on the stroke until the pointer or the context moves.
			var rejected := _commit_validation.duplicate(false)
			rejected["is_valid"] = false
			rejected["is_pending"] = false
			rejected["invalid_reason"] = "road_commit_rejected"
			# Its graded tiles were handed over at the click.
			for key in ["graded_tiles", "graded_removed", "graded_pending"]:
				rejected.erase(key)
			rejected["surface_generation"] = simulation_node.get_road_tool_surface_generation()
			rejected["sequence"] = _preview_sequence + 1
			_preview_sequence += 1
			_preview = rejected
			_preview_points = _commit_points
			_show_preview(_commit_points)
	_commit_points = PackedVector3Array()
	_commit_validation = {}
	_commit_border_check = []
	_graded_commit_epoch = -1

# A road touching the map edge asks about an external connection; the chain ends there so the
# dialog never sits over a stroke still following the pointer off the map.
func _commit_reaches_border() -> bool:
	for pos in _commit_border_check:
		if simulation_node.check_border_candidate(pos) >= 0:
			return true
	return false

func cancel_road():
	current_state = State.IDLE
	_start_dir = Vector3.ZERO
	_has_road_tangent = false
	_angle_snapped = false
	_clear_sticky_network_snap()
	_clear_preview_visual()
	if current_path:
		current_path.queue_free()
	current_path = null
	_preview_update_pending = false
	_clear_preview_cache()
	if _info_label:
		_info_label.visible = false

func reset_main_mesh_chunks() -> void:
	_clear_preview_visual()
	_preview_display.reset()
	if _swap_display != null:
		_swap_display.reset()
		_swap_display = null
		_swap_generation = -1
	_clear_preview_cache()
	super.reset_main_mesh_chunks()
	_queue_preview_update()

func mark_network_topology_dirty() -> void:
	mark_network_nodes_dirty()
	_clear_sticky_network_snap()
	_queue_preview_update()

## Called by NetworkRenderer after the road is confirmed in the graph.
## Drains _pending_border_checks and shows the border-connection dialog if relevant.
func drain_pending_border_checks() -> void:
	var total_start_us := Time.get_ticks_usec()
	var pending_count := _pending_border_checks.size()
	var candidate_ms := 0.0
	var prompt_ms := 0.0
	var candidate_calls := 0
	var prompt_count := 0
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
		MetrumDebug.log(
			"road",
			"border_checks_detail pending=%d candidate_calls=%d candidate_ms=%.3f prompts=%d prompt_ms=%.3f total_ms=%.3f"
			% [
				pending_count,
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
		_zoning_snap_enabled(),
		fwd_lanes,
		bkw_lanes,
		MAP_BORDER_SNAP_DIST_M,
		_sticky_network_snap,
		ROAD_NETWORK_SNAP_RELEASE_DIST_M,
		_angle_snap()
	)
	if pos_variant == null:
		if _has_last_world_mouse_pos:
			return _last_world_mouse_pos
		is_valid = false
		return Vector3.ZERO
	is_valid = true
	var pos: Vector3 = pos_variant["position"]
	_angle_snapped = bool(pos_variant.get("angle_snapped", false))
	if not _snap_to_roads_enabled() or not _can_stick_to_network_snap(pos):
		_clear_sticky_network_snap()
		return pos
	_sticky_network_snap = pos_variant
	return pos

# Straight strokes snap every 45° from the start direction; Shift draws free.
func _angle_snap() -> Vector3:
	if draw_mode != 0 or not _has_road_tangent or Input.is_key_pressed(KEY_SHIFT):
		return Vector3.ZERO
	return _start_dir

func _can_stick_to_network_snap(pos: Vector3) -> bool:
	if current_state == State.SETTING_END:
		if _xz_distance(pos, start_pos) < ROAD_SELF_SNAP_DIST_M:
			return false
		if draw_mode != 0 and _xz_distance(pos, control_pos) < ROAD_SELF_SNAP_DIST_M:
			return false
	return true

func _clear_sticky_network_snap() -> void:
	_sticky_network_snap.clear()

func _xz_distance(a: Vector3, b: Vector3) -> float:
	var dx := a.x - b.x
	var dz := a.z - b.z
	return sqrt(dx * dx + dz * dz)


func _clear_preview_cache() -> void:
	_preview = {}
	_preview_points = PackedVector3Array()
	_preview_fwd_lanes = -1
	_preview_bkw_lanes = -1
	_preview_drawn_sequence = 0

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
	path_ms: float,
	commit_ms: float,
	total_start_us: int
) -> void:
	if not _road_debug_enabled:
		return
	MetrumDebug.log(
		"road",
		"road_click_detail state=%d valid=%s mouse_ms=%.3f tangent_ms=%.3f path_ms=%.3f commit_ms=%.3f total_ms=%.3f"
		% [
			state_before,
			str(valid_after),
			mouse_ms,
			tangent_ms,
			path_ms,
			commit_ms,
			float(Time.get_ticks_usec() - total_start_us) / 1000.0,
		]
	)
