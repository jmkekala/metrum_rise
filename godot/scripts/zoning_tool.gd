extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var current_zone_type: int = 1 # 1: Res, 2: Com, 3: Ind, 4: Mix
var state: int = 0 # 0: Idle, 1: Drawing Frontage, 2: Pulling Depth
var active: bool = false
var attached_edge_idx: int = -1


var frontage_start: Vector2
var frontage_end: Vector2
var current_mouse_pos: Vector2

var editing_poly_id: int = -1
var editing_handle_idx: int = -1 # 0: start, 1: end
var editing_poly_verts: PackedVector2Array

var preview_mesh: MeshInstance3D
var frontage_mesh: MeshInstance3D
var global_frontages_mesh: MeshInstance3D
var handles_mesh: MultiMeshInstance3D

func _ready():
	preview_mesh = MeshInstance3D.new()
	var mat = StandardMaterial3D.new()
	mat.albedo_color = Color(0.2, 0.8, 0.9, 0.5)
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	preview_mesh.material_override = mat
	add_child(preview_mesh)
	
	frontage_mesh = MeshInstance3D.new()
	var line_mat = StandardMaterial3D.new()
	line_mat.albedo_color = Color(1, 1, 1, 1) # Bold White
	line_mat.emission_enabled = true
	line_mat.emission = Color(1, 1, 1, 1)
	line_mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	frontage_mesh.material_override = line_mat
	add_child(frontage_mesh)
	
	global_frontages_mesh = MeshInstance3D.new()
	var gline_mat = StandardMaterial3D.new()
	gline_mat.albedo_color = Color(1, 1, 1, 1) # Lean White
	gline_mat.emission_enabled = true
	gline_mat.emission = Color(1, 1, 1, 1)
	gline_mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	global_frontages_mesh.material_override = gline_mat
	add_child(global_frontages_mesh)
	
	handles_mesh = MultiMeshInstance3D.new()
	var hmm = MultiMesh.new()
	hmm.transform_format = MultiMesh.TRANSFORM_3D
	var sm = SphereMesh.new()
	sm.radius = 0.5
	sm.height = 1.0
	sm.radial_segments = 8
	sm.rings = 4
	hmm.mesh = sm
	
	var h_mat = StandardMaterial3D.new()
	h_mat.albedo_color = Color(0.8, 0.95, 1.0) # White Blue
	h_mat.emission_enabled = true
	h_mat.emission = Color(0.2, 0.5, 1.0)
	h_mat.emission_energy_multiplier = 2.0
	sm.surface_set_material(0, h_mat)
	
	handles_mesh.multimesh = hmm
	add_child(handles_mesh)
	
	# ENSURE ALL VISUAL MESHES OPERATE IN GLOBAL WORLD SPACE REGARDLESS OF PARENT OFFSET
	preview_mesh.top_level = true
	frontage_mesh.top_level = true
	global_frontages_mesh.top_level = true
	handles_mesh.top_level = true

func _process(delta):
	# Key handling moved to InputManager.gd
	
	
	if terrain_node and not terrain_node.show_global_zoning:
		if state != 0:
			state = 0
			preview_mesh.mesh = null
			frontage_mesh.mesh = null
		global_frontages_mesh.mesh = null
		return

	if Engine.get_frames_drawn() % 15 == 0:
		draw_all_frontages()

	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	
	var world_pos = null
	if intersection != null:
		world_pos = Vector2(intersection.x, intersection.z)

	if state == 0 and world_pos != null:
		var edge_idx = simulation_node.get_hovered_edge(world_pos.x, world_pos.y)
		if edge_idx != -1:
			var snapped = simulation_node.get_closest_point_on_edge(edge_idx, world_pos.x, world_pos.y)
			current_mouse_pos = snapped
			attached_edge_idx = edge_idx
			
			var st = SurfaceTool.new()
			st.begin(Mesh.PRIMITIVE_TRIANGLES)
			
			# Check if edge is available for new zoning
			var is_available = simulation_node.get_zoning_for_edge(edge_idx) == -1
			if is_available:
				st.set_color(Color(1, 1, 0, 0.7)) # Yellow/Green
			else:
				st.set_color(Color(1, 0, 0, 0.7)) # Red (Blocked)
				
			var y0 = simulation_node.get_height_at(current_mouse_pos) + 0.5
			var r = 0.5
			st.add_vertex(Vector3(current_mouse_pos.x-r, y0, current_mouse_pos.y-r))
			st.add_vertex(Vector3(current_mouse_pos.x+r, y0, current_mouse_pos.y-r))
			st.add_vertex(Vector3(current_mouse_pos.x, y0, current_mouse_pos.y+r))
			preview_mesh.mesh = st.commit()
		else:
			preview_mesh.mesh = null
			attached_edge_idx = -1

	elif state == 1 and world_pos != null:
		var snapped = simulation_node.get_closest_point_on_edge(attached_edge_idx, world_pos.x, world_pos.y)
		current_mouse_pos = snapped
		if current_mouse_pos.distance_to(frontage_start) > 0.5:
			update_preview_2_points()
			
	elif state == 2 and world_pos != null:
		current_mouse_pos = world_pos
		update_preview_4_points()
		
	elif state == 4 and world_pos != null:
		var edge_idx = attached_edge_idx
			
		var snapped = world_pos
		# ONLY snap vertices 0 and 1 (Frontage handles) to the road edge!
		# Vertices 2 and 3 are back-corners and should move freely in world space.
		if edge_idx != -1 and editing_handle_idx < 2:
			snapped = simulation_node.get_closest_point_on_edge(edge_idx, world_pos.x, world_pos.y)
			
		current_mouse_pos = snapped
		
		# real-time editing
		var modified = PackedVector2Array()
		for v in editing_poly_verts: modified.push_back(v)
		if editing_handle_idx >= 0 and editing_handle_idx < modified.size():
			modified[editing_handle_idx] = current_mouse_pos
			
			# Correctly extract frontage from the base primitive (handles 0 and 1 define the edge)
			var target_fronts = PackedVector2Array()
			target_fronts.push_back(modified[0]) 
			target_fronts.push_back(modified[1])
			
			var clipped = get_clipped_polygon(modified, target_fronts, editing_poly_id, edge_idx)
			var final_packed = clipped["poly"]
			var final_fronts_count = clipped["fronts_count"]
			if final_packed.size() >= 3:
				simulation_node.update_zoning_polygon(editing_poly_id, final_packed, final_fronts_count, modified)

func _unhandled_input(event):
	if not active: return
	
	if terrain_node and not terrain_node.show_global_zoning:
		return
		
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		var mouse_pos = get_viewport().get_mouse_position()
		var camera = get_viewport().get_camera_3d()
		var ray_origin = camera.project_ray_origin(mouse_pos)
		var ray_dir = camera.project_ray_normal(mouse_pos)
		var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
		if intersection == null: return
		var world_pos = Vector2(intersection.x, intersection.z)
		
		if event.pressed:
			if state == 0:
				# Check for handles first
				var handles_data = simulation_node.get_zoning_polygons_handles()
				var offset = 0
				var found_handle = false
				while offset < handles_data.size():
					var p_id = int(handles_data[offset])
					var v_count = int(handles_data[offset+1])
					offset += 2
					for k in range(v_count):
						var vx = handles_data[offset + k*2]
						var vy = handles_data[offset + k*2 + 1]
						if world_pos.distance_to(Vector2(vx, vy)) < 3.0:
							editing_poly_id = p_id
							editing_handle_idx = k
							editing_poly_verts = simulation_node.get_polygon_base_vertices(p_id)
							
							# Sync attached edge for road snapping from the selected polygon
							var props = simulation_node.get_polygon_properties(p_id)
							attached_edge_idx = int(props.x)
							
							state = 4
							found_handle = true
							break
					if found_handle: break
					offset += v_count * 2
				
				if found_handle: return
						
				if attached_edge_idx != -1:
					var clicked_poly = simulation_node.get_polygon_at(world_pos.x, world_pos.y)
					if clicked_poly != -1:
						simulation_node.add_frontage_to_polygon(clicked_poly, attached_edge_idx)
					else:
						# Prevent STARTING a new zone if this edge is already fronting another zone
						if simulation_node.get_zoning_for_edge(attached_edge_idx) != -1:
							return
							
						frontage_start = current_mouse_pos
						state = 1
			elif state == 1:
				if current_mouse_pos.distance_to(frontage_start) > 0.5:
					frontage_end = current_mouse_pos
					state = 2
			elif state == 2:
				commit_polygon()
				state = 0
				preview_mesh.mesh = null
				frontage_mesh.mesh = null
		else: # Released
			if state == 3 or state == 4:
				state = 0
				editing_poly_id = -1
				editing_poly_verts = PackedVector2Array()
			
	elif event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		if state != 0:
			state = 0
			preview_mesh.mesh = null
			frontage_mesh.mesh = null
			editing_poly_id = -1

func update_preview_2_points():
	var fronts = simulation_node.get_curved_frontage(attached_edge_idx, frontage_start, current_mouse_pos)
	if fronts.size() < 2: return
	
	var st = SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_LINE_STRIP)
	for i in range(fronts.size()):
		var y0 = simulation_node.get_height_at(fronts[i]) + 0.5
		st.add_vertex(Vector3(fronts[i].x, y0, fronts[i].y))
	
	frontage_mesh.mesh = st.commit()
	preview_mesh.mesh = null

func update_preview_4_points():
	var fronts = simulation_node.get_curved_frontage(attached_edge_idx, frontage_start, frontage_end)
	if fronts.size() < 2: return
	
	var front_vec = frontage_end - frontage_start
	var tangent = front_vec.normalized()
	if tangent.length() < 0.1: tangent = (fronts[1] - fronts[0]).normalized()
	var normal = Vector2(-tangent.y, tangent.x) 
	
	var to_mouse = current_mouse_pos - frontage_start
	var depth_amt = to_mouse.dot(normal)
	
	if depth_amt < 0:
		depth_amt = min(-0.5, depth_amt)
	else:
		depth_amt = max(0.5, depth_amt)
		
	var pts = PackedVector2Array()
	for p in fronts: pts.push_back(p)
	for i in range(fronts.size() - 1, -1, -1):
		var local_tan = Vector2()
		if i > 0 and i < fronts.size() - 1:
			local_tan = (fronts[i+1] - fronts[i-1]).normalized()
		elif i == 0:
			local_tan = (fronts[1] - fronts[0]).normalized()
		else:
			local_tan = (fronts[-1] - fronts[-2]).normalized()
		var local_n = Vector2(-local_tan.y, local_tan.x)
		pts.push_back(fronts[i] + local_n * depth_amt)
	
	var clipped = get_clipped_polygon(pts, fronts, -1, attached_edge_idx)
	var final_pts = clipped["poly"]
	if final_pts.size() < 3:
		preview_mesh.mesh = null
		return
	
	var st = SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	
	var indices = Geometry2D.triangulate_polygon(final_pts)
	if indices.size() > 0:
		for idx in indices:
			var v = final_pts[idx]
			var y = simulation_node.get_height_at(v) + 0.5
			st.add_vertex(Vector3(v.x, y, v.y))
		st.generate_normals()
		preview_mesh.mesh = st.commit()

func commit_polygon():
	var fronts = simulation_node.get_curved_frontage(attached_edge_idx, frontage_start, frontage_end)
	if fronts.size() < 2: return
	
	var front_vec = frontage_end - frontage_start
	var tangent = front_vec.normalized()
	if tangent.length() < 0.1: tangent = (fronts[1] - fronts[0]).normalized()
	var normal = Vector2(-tangent.y, tangent.x) 
	
	var to_mouse = current_mouse_pos - frontage_start
	var depth_amt = to_mouse.dot(normal)
	
	if depth_amt < 0:
		depth_amt = min(-0.5, depth_amt)
	else:
		depth_amt = max(0.5, depth_amt)
		
	var front_pos_end = fronts[-1]
	var back_pos_end = front_pos_end + normal * depth_amt
	var back_pos_start = fronts[0] + normal * depth_amt

	var packed = PackedVector2Array()
	for p in fronts: packed.push_back(p)
	for i in range(fronts.size() - 1, -1, -1):
		var local_tan = Vector2()
		if i > 0 and i < fronts.size() - 1:
			local_tan = (fronts[i+1] - fronts[i-1]).normalized()
		elif i == 0:
			local_tan = (fronts[1] - fronts[0]).normalized()
		else:
			local_tan = (fronts[-1] - fronts[-2]).normalized()
		var local_n = Vector2(-local_tan.y, local_tan.x)
		packed.push_back(fronts[i] + local_n * depth_amt)
	
	var clipped = get_clipped_polygon(packed, fronts, -1, attached_edge_idx)
	var final_packed = clipped["poly"]
	var final_fronts_count = clipped["fronts_count"]
	
	if final_packed.size() >= 3 and final_fronts_count > 0:
		# Create a clean 4-corner base primitive for editing handles
		var base_corners = PackedVector2Array()
		base_corners.push_back(fronts[0])
		base_corners.push_back(front_pos_end)
		base_corners.push_back(back_pos_end)
		base_corners.push_back(back_pos_start)
		simulation_node.add_zoning_polygon(attached_edge_idx, current_zone_type, final_packed, depth_amt, final_fronts_count, base_corners)

func draw_all_frontages():
	var all_data = simulation_node.get_zoning_frontage_data()
	var counts = simulation_node.get_zoning_frontage_counts()
	
	if counts.size() == 0:
		global_frontages_mesh.mesh = null
		handles_mesh.multimesh.instance_count = 0
		return
		
	var st = SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_LINES)
	
	var all_handle_pts = []
	var offset = 0
	
	for count in counts:
		if count < 2:
			offset += count
			continue
			
		all_handle_pts.push_back(all_data[offset])
		all_handle_pts.push_back(all_data[offset + count - 1])
		
		for i in range(count - 1):
			var v0 = all_data[offset + i]
			var v1 = all_data[offset + i + 1]
			var y0 = simulation_node.get_height_at(v0) + 0.8
			var y1 = simulation_node.get_height_at(v1) + 0.8
			st.add_vertex(Vector3(v0.x, y0, v0.y))
			st.add_vertex(Vector3(v1.x, y1, v1.y))
		offset += count
		
	global_frontages_mesh.mesh = st.commit()
	
	# Draw Handles (Entire polygon bounds!)
	var handle_data = simulation_node.get_zoning_polygons_handles()
	var total_handles = 0
	var h_count_ptr = 1
	while h_count_ptr < handle_data.size():
		total_handles += int(handle_data[h_count_ptr])
		h_count_ptr += 2 + int(handle_data[h_count_ptr]) * 2
		
	handles_mesh.multimesh.instance_count = total_handles
	
	var h_offset = 0
	var instance_idx = 0
	while h_offset < handle_data.size():
		var p_id = int(handle_data[h_offset])
		var v_count = int(handle_data[h_offset+1])
		h_offset += 2
		for k in range(v_count):
			var p = Vector2(handle_data[h_offset + k*2], handle_data[h_offset + k*2 + 1])
			
			# Add subtle inward offset so handles don't overlap road exactly
			var center = Vector2(0,0)
			for j in range(v_count): center += Vector2(handle_data[h_offset + j*2], handle_data[h_offset + j*2 + 1])
			center /= v_count
			var to_center = (center - p).normalized()
			p += to_center * 0.5
			
			var y = simulation_node.get_height_at(p) + 1.2
			
			var t = Transform3D()
			# Make them slightly bigger when hovered/dragging? (Maybe later)
			var scale_factor = 1.0
			if p_id == editing_poly_id and k == editing_handle_idx: scale_factor = 1.5
			
			t = t.scaled(Vector3(scale_factor, scale_factor, scale_factor))
			t.origin = Vector3(p.x, y, p.y)
			handles_mesh.multimesh.set_instance_transform(instance_idx, t)
			instance_idx += 1
			
		h_offset += v_count * 2

func get_clipped_polygon(base_poly: PackedVector2Array, original_fronts: PackedVector2Array, edit_poly_id: int, attach_edge: int) -> Dictionary:
	var floats = simulation_node.get_obstacle_polygons_float_array(edit_poly_id, attach_edge)
	var obstacles = []
	if floats.size() > 0:
		var num_polys = int(floats[0])
		var i = 1
		for p in range(num_polys):
			var num_verts = int(floats[i])
			i += 1
			var poly = PackedVector2Array()
			for v in range(num_verts):
				poly.push_back(Vector2(floats[i], floats[i+1]))
				i += 2
			obstacles.append(poly)
			
	var current_polys = [base_poly]
	for obs in obstacles:
		var next_polys = []
		for p in current_polys:
			var res = Geometry2D.clip_polygons(p, obs)
			for r in res:
				next_polys.append(r)
		current_polys = next_polys
		
	if current_polys.size() == 0:
		return { "poly": PackedVector2Array(), "fronts_count": 0 }
		
	var best_poly = current_polys[0]
	var best_area = 0.0
	for p in current_polys:
		var area = abs(get_poly_area(p))
		if area > best_area:
			best_area = area
			best_poly = p
			
	var on_frontage = []
	for i in range(best_poly.size()):
		var p = best_poly[i]
		var is_on = false
		for j in range(original_fronts.size() - 1):
			var a = original_fronts[j]
			var b = original_fronts[j+1]
			var l2 = a.distance_squared_to(b)
			var dist = 0.0
			if l2 == 0.0:
				dist = p.distance_to(a)
			else:
				var t = ((p.x - a.x) * (b.x - a.x) + (p.y - a.y) * (b.y - a.y)) / l2
				t = clamp(t, 0.0, 1.0)
				var proj = Vector2(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y))
				dist = p.distance_to(proj)
			if dist < 0.5:
				is_on = true
				break
		on_frontage.append(is_on)
		
	var n = best_poly.size()
	var best_start = 0
	var best_len = 0
	for i in range(n):
		if on_frontage[i]:
			var current_len = 0
			for j in range(n):
				var idx = (i + j) % n
				if on_frontage[idx]:
					current_len += 1
				else:
					break
			if current_len > best_len:
				best_len = current_len
				best_start = i
				
	var aligned = PackedVector2Array()
	if best_len >= 2:
		for i in range(n):
			aligned.push_back(best_poly[(best_start + i) % n])
		return { "poly": aligned, "fronts_count": best_len }
	else:
		return { "poly": best_poly, "fronts_count": original_fronts.size() }

func get_poly_area(poly: PackedVector2Array) -> float:
	var area = 0.0
	var n = poly.size()
	for i in range(n):
		var j = (i + 1) % n
		area += poly[i].x * poly[j].y - poly[j].x * poly[i].y
	return area / 2.0
