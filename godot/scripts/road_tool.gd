extends "res://scripts/network_tool.gd"

enum State { IDLE, SETTING_CONTROL, SETTING_END }
var current_state = State.IDLE

var start_pos: Vector3
var control_pos: Vector3

var current_path: Path3D

var fwd_lanes: int = 1
var bkw_lanes: int = 1
var lanes_label: Label

var draw_mode: int = 0 # 0: straight, 1: spline

func _update_lanes_label():
	if active and current_path != null:
		_draw_blueprint()

func adjust_lanes(fwd_delta: int, bkw_delta: int):
	if fwd_delta != 0: fwd_lanes = clamp(fwd_lanes + fwd_delta, 0, 4)
	if bkw_delta != 0: bkw_lanes = clamp(bkw_lanes + bkw_delta, 0, 4)
	# Allow 0,0 for walkways
	_update_lanes_label()

func _unhandled_input(event):
	if active and event is InputEventMouseMotion:
		_update_preview()
	
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
	var preview_verts = current_path.curve.get_baked_points()
	if preview_verts.size() > 1:
		var slope_too_steep = false
		
		# HARMONIC CONFORMANCE PREVIEW (Laplacian Smoothing)
		var start_h = simulation_node.get_height_at(Vector2(preview_verts[0].x, preview_verts[0].z))
		var end_h = simulation_node.get_height_at(Vector2(preview_verts[-1].x, preview_verts[-1].z))
		
		# 1. Sample raw terrain for all points
		for i in range(preview_verts.size()):
			if i == 0:
				preview_verts[i].y = start_h
			elif i == preview_verts.size() - 1:
				preview_verts[i].y = end_h
			else:
				preview_verts[i].y = simulation_node.get_height_at(Vector2(preview_verts[i].x, preview_verts[i].z))
				
		# 2. Taubin Smoothing (Iron out bumps without volume shrinkage)
		var iters = 50
		var num_verts = preview_verts.size()
		if num_verts > 2:
			var temp_h = []
			temp_h.resize(num_verts)
			var lambda_val = 0.5
			var mu_val = -0.53
			for it in range(iters):
				# Positive Pass (Shrink/Smooth)
				for i in range(1, num_verts - 1):
					var laplacian = 0.5 * (preview_verts[i-1].y + preview_verts[i+1].y) - preview_verts[i].y
					temp_h[i] = preview_verts[i].y + lambda_val * laplacian
				for i in range(1, num_verts - 1):
					preview_verts[i].y = temp_h[i]
					
				# Negative Pass (Inflate/Restore Volume)
				for i in range(1, num_verts - 1):
					var laplacian = 0.5 * (preview_verts[i-1].y + preview_verts[i+1].y) - preview_verts[i].y
					temp_h[i] = preview_verts[i].y + mu_val * laplacian
				for i in range(1, num_verts - 1):
					preview_verts[i].y = temp_h[i]
		
		# 3. Apply vertical offset and slope checks
		for i in range(preview_verts.size()):
			var p = preview_verts[i]
			var check_y = p.y
			p.y += 0.05 # purely for visibility in preview
			
			if i > 0:
				var prev_p = preview_verts[i-1]
				# We calculate slope based on the true geometry, not the visually shifted one
				var prev_check_y = prev_p.y - 0.05
				var dist = Vector2(p.x, p.z).distance_to(Vector2(prev_p.x, prev_p.z))
				if dist > 0.01:
					var slope = abs(check_y - prev_check_y) / dist
					if slope > 0.41:
						slope_too_steep = true
			
			preview_verts[i] = p
		
		if slope_too_steep:
			is_valid = false
			
		var arr_mesh = ArrayMesh.new()
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		
		var road_width = max(2.0, (fwd_lanes + bkw_lanes) * simulation_node.get_lane_width())
		var half_w = road_width * 0.5
		
		var ribbon_verts = PackedVector3Array()
		for i in range(preview_verts.size()):
			var p = preview_verts[i]
			var tangent
			if i < preview_verts.size() - 1:
				tangent = (preview_verts[i+1] - p).normalized()
			else:
				tangent = (p - preview_verts[i-1]).normalized()
				
			if tangent.length() < 0.001:
				tangent = Vector3(0, 0, 1)
				
			var normal = Vector3(-tangent.z, 0, tangent.x)
			
			ribbon_verts.push_back(p - normal * half_w)
			ribbon_verts.push_back(p + normal * half_w)
		
		arrays[Mesh.ARRAY_VERTEX] = ribbon_verts
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLE_STRIP, arrays)
		blueprint_mesh.mesh = arr_mesh

func _commit_segment(end_pos):
	if not is_valid: return
	
	var points = current_path.curve.get_baked_points()
	if points.size() > 1:
		var main_ui = get_node("../MainUI")
		var z_left = main_ui.road_zoning_left_btn.button_pressed
		var z_right = main_ui.road_zoning_right_btn.button_pressed
		simulation_node.add_road(points, fwd_lanes, bkw_lanes, z_left, z_right)
		simulation_node.flatten_terrain_for_roads()
		update_main_mesh()
		terrain_node.update_terrain_visuals()
	
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


# Override to add Self-Snapping and Angle Snapping
func get_world_mouse_pos() -> Vector3:
	var pos = super.get_world_mouse_pos()
	
	if active and Input.is_key_pressed(KEY_SHIFT):
		var ref_pos = start_pos if current_state == State.SETTING_CONTROL else control_pos
		var dir = pos - ref_pos
		var length = dir.length()
		if length > 0.1:
			var angle = atan2(dir.z, dir.x)
			var snap_rad = PI / 12.0 # 15 degrees
			angle = round(angle / snap_rad) * snap_rad
			pos = ref_pos + Vector3(cos(angle), 0, sin(angle)) * length
	
	# Self-snapping (to Start or Control points)
	if active:
		if pos.distance_to(start_pos) < 2.5:
			return start_pos
		
		if current_state == State.SETTING_END:
			if pos.distance_to(control_pos) < 2.5:
				return control_pos
				
	return pos
