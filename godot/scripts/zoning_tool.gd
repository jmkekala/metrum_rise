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

var preview_mesh: MeshInstance3D
var frontage_mesh: MeshInstance3D
var global_frontages_mesh: MeshInstance3D
var handles_mesh: MeshInstance3D

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
	
	handles_mesh = MeshInstance3D.new()
	var h_mat = StandardMaterial3D.new()
	h_mat.albedo_color = Color(1, 0.5, 0, 1) # Orange
	h_mat.emission_enabled = true
	h_mat.emission = Color(1, 0.5, 0, 1)
	handles_mesh.material_override = h_mat
	add_child(handles_mesh)

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
		
	elif state == 3 and world_pos != null:
		var props = simulation_node.get_polygon_properties(editing_poly_id)
		var edge_idx = int(props.x)
		var depth_amt = props.y
		
		var snapped = simulation_node.get_closest_point_on_edge(edge_idx, world_pos.x, world_pos.y)
		current_mouse_pos = snapped
		
		# Update the polygon real-time
		var handle_pts = simulation_node.get_zoning_frontage_points()
		var ids = simulation_node.get_zoning_polygon_ids()
		var p_idx = -1
		for i in range(ids.size()):
			if ids[i] == editing_poly_id:
				p_idx = i
				break
		
		if p_idx != -1:
			var p0 = handle_pts[p_idx * 2]
			var p1 = handle_pts[p_idx * 2 + 1]
			
			if editing_handle_idx == 0: p0 = current_mouse_pos
			else: p1 = current_mouse_pos
			
			if p0.distance_to(p1) < 2.0:
				simulation_node.delete_zoning_polygon(editing_poly_id)
				state = 0
				editing_poly_id = -1
			else:
				var fronts = simulation_node.get_curved_frontage(edge_idx, p0, p1)
				if fronts.size() >= 2:
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
					
					simulation_node.update_zoning_polygon(editing_poly_id, packed, fronts.size())

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
				var handle_pts = simulation_node.get_zoning_frontage_points()
				var ids = simulation_node.get_zoning_polygon_ids()
				for i in range(ids.size()):
					if world_pos.distance_to(handle_pts[i*2]) < 2.0:
						editing_poly_id = ids[i]
						editing_handle_idx = 0
						state = 3
						return
					if world_pos.distance_to(handle_pts[i*2+1]) < 2.0:
						editing_poly_id = ids[i]
						editing_handle_idx = 1
						state = 3
						return
						
				if attached_edge_idx != -1:
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
			if state == 3:
				state = 0
				editing_poly_id = -1
			
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
		
	var pts: Array[Vector2] = []
	for p in fronts: pts.append(p)
	for i in range(fronts.size() - 1, -1, -1):
		var local_tan = Vector2()
		if i > 0 and i < fronts.size() - 1:
			local_tan = (fronts[i+1] - fronts[i-1]).normalized()
		elif i == 0:
			local_tan = (fronts[1] - fronts[0]).normalized()
		else:
			local_tan = (fronts[-1] - fronts[-2]).normalized()
		var local_n = Vector2(-local_tan.y, local_tan.x)
		pts.append(fronts[i] + local_n * depth_amt)
	
	var st = SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	
	var indices = Geometry2D.triangulate_polygon(pts)
	if indices.size() > 0:
		for idx in indices:
			var v = pts[idx]
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
	
	simulation_node.add_zoning_polygon(attached_edge_idx, current_zone_type, packed, depth_amt, fronts.size())

func draw_all_frontages():
	var fronts = simulation_node.get_zoning_frontage_points()
	if fronts.size() < 2:
		global_frontages_mesh.mesh = null
		handles_mesh.mesh = null
		return
		
	var st = SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_LINES)
	
	for idx in range(0, fronts.size(), 2):
		var v0 = fronts[idx]
		var v1 = fronts[idx+1]
		var y0 = simulation_node.get_height_at(v0) + 0.8
		var y1 = simulation_node.get_height_at(v1) + 0.8
		st.add_vertex(Vector3(v0.x, y0, v0.y))
		st.add_vertex(Vector3(v1.x, y1, v1.y))
		
	global_frontages_mesh.mesh = st.commit()
	
	# Draw Handles
	var handle_pts = fronts # Reuse the points from get_zoning_frontage_points
	var hst = SurfaceTool.new()
	hst.begin(Mesh.PRIMITIVE_TRIANGLES)
	for p in handle_pts:
		var y = simulation_node.get_height_at(p) + 1.0
		var r = 0.4
		hst.add_vertex(Vector3(p.x-r, y, p.y-r))
		hst.add_vertex(Vector3(p.x+r, y, p.y-r))
		hst.add_vertex(Vector3(p.x, y, p.y+r))
	handles_mesh.mesh = hst.commit()
