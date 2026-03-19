extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var current_zone_type: int = 1 # 1: Res, 2: Com, 3: Ind, 4: Mix
var state: int = 0 # 0: Idle, 1: Drawing, 2: Dragging Vertex
var drawn_vertices: Array[Vector2] = []
var current_mouse_pos: Vector2
var attached_edge_idx: int = -1

var grabbed_poly_id: int = -1
var grabbed_vertex_idx: int = -1
var hovered_poly_id: int = -1
var hovered_vertex_idx: int = -1

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
	hmm.instance_count = 0
	var hmesh = BoxMesh.new()
	hmesh.size = Vector3(2.0, 2.0, 2.0)
	var hm = StandardMaterial3D.new()
	hm.albedo_color = Color(1.0, 0.6, 0.1) # Bright Orange
	hm.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	hmesh.material = hm
	hmm.mesh = hmesh
	handles_mesh.multimesh = hmm
	add_child(handles_mesh)

func _process(delta):
	# ... Keyboard Inputs
	if Input.is_key_pressed(KEY_1): 
		if current_zone_type != 1: print("Zoning Mode: Residential (Green)")
		current_zone_type = 1
	if Input.is_key_pressed(KEY_2): 
		if current_zone_type != 2: print("Zoning Mode: Commercial (Blue)")
		current_zone_type = 2
	if Input.is_key_pressed(KEY_3): 
		if current_zone_type != 3: print("Zoning Mode: Industrial (Yellow)")
		current_zone_type = 3
	if Input.is_key_pressed(KEY_4): 
		if current_zone_type != 4: print("Zoning Mode: Mixed (Purple)")
		current_zone_type = 4
	
	if terrain_node and not terrain_node.show_global_zoning:
		if state != 0:
			state = 0
			drawn_vertices.clear()
			preview_mesh.mesh = null
			frontage_mesh.mesh = null
		global_frontages_mesh.mesh = null
		if handles_mesh: handles_mesh.multimesh.instance_count = 0
		return

	if Engine.get_frames_drawn() % 15 == 0:
		draw_all_frontages()

	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	
	var local_pos = null
	var size = simulation_node.get_heightmap_size()
	if intersection != null:
		local_pos = Vector2(
			intersection.x + (size.x - 1.0) * 0.5,
			intersection.z + (size.y - 1.0) * 0.5
		)

	if state == 0:
		var ids = simulation_node.get_zoning_polygon_ids()
		var hit_id = -1
		var hit_idx = -1
		var best_dist = 4.0
		
		var mm_buffer = PackedFloat32Array()
		
		for i in range(ids.size()):
			var pid = ids[i]
			var verts = simulation_node.get_zoning_polygon_vertices(pid)
			for j in range(2, verts.size()):
				var v = verts[j]
				var wy = simulation_node.get_height_at(v)
				
				var scale = 1.0
				if local_pos != null:
					var d = local_pos.distance_to(v)
					if d < best_dist:
						best_dist = d
						hit_id = pid
						hit_idx = j
				
				if hit_id == pid and hit_idx == j:
					scale = 1.5 # Hover highlight implicitly
				
				var gX = v.x - (size.x - 1.0) * 0.5
				var gZ = v.y - (size.y - 1.0) * 0.5
				# Row major transformation array bindings natively!
				mm_buffer.push_back(scale); mm_buffer.push_back(0.0); mm_buffer.push_back(0.0); mm_buffer.push_back(gX)
				mm_buffer.push_back(0.0); mm_buffer.push_back(scale); mm_buffer.push_back(0.0); mm_buffer.push_back(wy + 1.0)
				mm_buffer.push_back(0.0); mm_buffer.push_back(0.0); mm_buffer.push_back(scale); mm_buffer.push_back(gZ)
				
		var count = mm_buffer.size() / 12
		handles_mesh.multimesh.instance_count = count
		if count > 0:
			handles_mesh.multimesh.buffer = mm_buffer
			
		hovered_poly_id = hit_id
		hovered_vertex_idx = hit_idx

	elif state == 1 and local_pos != null:
		handles_mesh.multimesh.instance_count = 0
		# Snap visually to origin to close loop
		if drawn_vertices.size() >= 3 and local_pos.distance_to(drawn_vertices[0]) < 8.0:
			current_mouse_pos = drawn_vertices[0]
		else:
			current_mouse_pos = local_pos
			
		update_preview()
		
	elif state == 2 and local_pos != null:
		handles_mesh.multimesh.instance_count = 0
		simulation_node.update_zoning_polygon_vertex(grabbed_poly_id, grabbed_vertex_idx, local_pos.x, local_pos.y)

func _unhandled_input(event):
	if terrain_node and not terrain_node.show_global_zoning:
		return
		
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		var mouse_pos = get_viewport().get_mouse_position()
		var camera = get_viewport().get_camera_3d()
		var ray_origin = camera.project_ray_origin(mouse_pos)
		var ray_dir = camera.project_ray_normal(mouse_pos)
		var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
		
		if intersection == null: return
		
		var size = simulation_node.get_heightmap_size()
		var local_pos = Vector2(
			intersection.x + (size.x - 1.0) * 0.5,
			intersection.z + (size.y - 1.0) * 0.5
		)
		
		if state == 0:
			if hovered_poly_id != -1:
				grabbed_poly_id = hovered_poly_id
				grabbed_vertex_idx = hovered_vertex_idx
				state = 2
				return
				
			var edge_idx = simulation_node.get_hovered_edge(intersection.x, intersection.z)
			if edge_idx != -1:
				attached_edge_idx = edge_idx
				drawn_vertices.clear()
				drawn_vertices.append(local_pos)
				current_mouse_pos = local_pos
				state = 1
		elif state == 1:
			# Check loop closure!
			if drawn_vertices.size() >= 3 and local_pos.distance_to(drawn_vertices[0]) < 8.0:
				commit_polygon()
				state = 0
				drawn_vertices.clear()
				preview_mesh.mesh = null
				frontage_mesh.mesh = null
			else:
				drawn_vertices.append(local_pos)
			
	elif event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and not event.pressed:
		if state == 2:
			state = 0
			grabbed_poly_id = -1
			grabbed_vertex_idx = -1
			
	elif event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		if state != 0:
			state = 0
			drawn_vertices.clear()
			preview_mesh.mesh = null
			frontage_mesh.mesh = null

func update_preview():
	var pts = drawn_vertices.duplicate()
	if current_mouse_pos != pts[0]: # If not snapped exactly to origin, preview current tracing point
		pts.append(current_mouse_pos)
		
	if pts.size() < 3:
		# Draw a simple line if only 2 points
		return
		
	var st = SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	
	var size = simulation_node.get_heightmap_size()
	var offset = Vector2((size.x - 1.0) * 0.5, (size.y - 1.0) * 0.5)
	
	# Basic Ear-Clipping Fan Triangulation from Point 0 (Convex assumption for zoning blocks)
	var v0 = pts[0]
	var g0 = v0 - offset
	var y0 = simulation_node.get_height_at(v0) + 0.5
	
	for i in range(1, pts.size() - 1):
		var v1 = pts[i]
		var v2 = pts[i+1]
		
		var g1 = v1 - offset
		var g2 = v2 - offset
		
		var y1 = simulation_node.get_height_at(v1) + 0.5
		var y2 = simulation_node.get_height_at(v2) + 0.5
		
		st.add_vertex(Vector3(g0.x, y0, g0.y))
		st.add_vertex(Vector3(g1.x, y1, g1.y))
		st.add_vertex(Vector3(g2.x, y2, g2.y))

	st.generate_normals()
	preview_mesh.mesh = st.commit()
	
	if pts.size() >= 2:
		var st_line = SurfaceTool.new()
		st_line.begin(Mesh.PRIMITIVE_TRIANGLES)
		var f_vec = (pts[1] - pts[0]).normalized()
		var f_norm = Vector2(f_vec.y, -f_vec.x) * 0.4 # 0.8m thinner hover trace
		var quad_h = 1.0 # Hover structurally over bounds
		
		var g1 = pts[1] - offset
		var y1 = simulation_node.get_height_at(pts[1]) + 0.5
		
		st_line.add_vertex(Vector3(g0.x + f_norm.x, y0 + quad_h, g0.y + f_norm.y))
		st_line.add_vertex(Vector3(g1.x + f_norm.x, y1 + quad_h, g1.y + f_norm.y))
		st_line.add_vertex(Vector3(g1.x - f_norm.x, y1 + quad_h, g1.y - f_norm.y))
		
		st_line.add_vertex(Vector3(g0.x + f_norm.x, y0 + quad_h, g0.y + f_norm.y))
		st_line.add_vertex(Vector3(g1.x - f_norm.x, y1 + quad_h, g1.y - f_norm.y))
		st_line.add_vertex(Vector3(g0.x - f_norm.x, y0 + quad_h, g0.y - f_norm.y))
		
		st_line.generate_normals()
		frontage_mesh.mesh = st_line.commit()
	else:
		frontage_mesh.mesh = null

func draw_all_frontages():
	var frontages = simulation_node.get_zoning_frontages()
	if frontages.size() < 2:
		global_frontages_mesh.mesh = null
		return
		
	var st_line = SurfaceTool.new()
	st_line.begin(Mesh.PRIMITIVE_TRIANGLES)
	var size = simulation_node.get_heightmap_size()
	var offset = Vector2((size.x - 1.0) * 0.5, (size.y - 1.0) * 0.5)
	
	for i in range(0, frontages.size(), 2):
		var p0 = frontages[i]
		var p1 = frontages[i+1]
		var f_vec = (p1 - p0).normalized()
		var f_norm = Vector2(f_vec.y, -f_vec.x) * 0.4 # Lean continuous lines
		var quad_h = 0.5 # Deep below plotting tool traces
		
		var g0 = p0 - offset
		var y0 = simulation_node.get_height_at(p0) + 0.5
		var g1 = p1 - offset
		var y1 = simulation_node.get_height_at(p1) + 0.5
		
		st_line.add_vertex(Vector3(g0.x + f_norm.x, y0 + quad_h, g0.y + f_norm.y))
		st_line.add_vertex(Vector3(g1.x + f_norm.x, y1 + quad_h, g1.y + f_norm.y))
		st_line.add_vertex(Vector3(g1.x - f_norm.x, y1 + quad_h, g1.y - f_norm.y))
		
		st_line.add_vertex(Vector3(g0.x + f_norm.x, y0 + quad_h, g0.y + f_norm.y))
		st_line.add_vertex(Vector3(g1.x - f_norm.x, y1 + quad_h, g1.y - f_norm.y))
		st_line.add_vertex(Vector3(g0.x - f_norm.x, y0 + quad_h, g0.y - f_norm.y))
		
	st_line.generate_normals()
	global_frontages_mesh.mesh = st_line.commit()

func commit_polygon():
	if drawn_vertices.size() < 3: return
	
	# Determine logical frontage inward facing direction
	var front_vec = (drawn_vertices[1] - drawn_vertices[0]).normalized()
	var normal = Vector2(front_vec.y, -front_vec.x)
	
	var avg_depth = 0.0
	for i in range(2, drawn_vertices.size()):
		avg_depth += (drawn_vertices[i] - drawn_vertices[0]).dot(normal)
		
	var facing_dir = -normal 
	if avg_depth < 0:
		facing_dir = normal
	
	var vertices = PackedVector2Array()
	for v in drawn_vertices:
		vertices.push_back(v)
	
	simulation_node.add_zoning_polygon(attached_edge_idx, current_zone_type, vertices, facing_dir.x, facing_dir.y)

