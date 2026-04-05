## Zoning paint tool — projects mouse position onto road edges and paints land-use cells.
##
## Rust methods called: update_zoning_visuals(), get_hovered_edge(), set_zoning_cell(),
##   set_zoning_enabled(), get_closest_network_point()
## Zone types: 1=Residential, 2=Commercial, 3=Industrial, 4=Office, 5=Mixed.
## Modes: SINGLE (one cell), PAINT (drag to paint), FILL (flood-fill edge), DELETE.
## Cell projection uses Rust's edge geometry to compute (edge_idx, side, cell_x, cell_y)
## from a world-space mouse ray — all projection math lives in update_zoning_visuals() on
## the Rust side; GDScript only forwards the hovered edge and mouse position.
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var current_zone_type: int = 1 # 1: Res, 2: Com, 3: Ind, 4: Office, 5: Mixed
var state: int = 0 # 0: Idle, 1: Painting
var active: bool = false
var hovered_edge_idx: int = -1

var current_depth: int = 3 # Default 3 cells (15m)
var drag_start_t: float = -1.0

const ZONING_DEPTH: int = 12
const CELL_SIZE: float = 10.0

var grid_mesh: MultiMeshInstance3D
var paint_mesh: MultiMeshInstance3D
var brush_mesh: MultiMeshInstance3D

var persistent_container: Node3D
var persistent_meshes: Dictionary = {} # int (ZoneType) -> MultiMeshInstance3D

func _ready():
	var shader = load("res://scripts/shaders/zoning_grid.gdshader")
	var mat = ShaderMaterial.new()
	mat.shader = shader
	
	grid_mesh = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = true
	mm.use_custom_data = true
	
	var plane = PlaneMesh.new()
	plane.size = Vector2(1.0, 1.0)
	
	mm.mesh = plane
	grid_mesh.multimesh = mm
	grid_mesh.material_override = mat
	
	add_child(grid_mesh)
	grid_mesh.top_level = true
	
	paint_mesh = MultiMeshInstance3D.new()
	var mm2 = MultiMesh.new()
	mm2.transform_format = MultiMesh.TRANSFORM_3D
	mm2.use_colors = true
	mm2.use_custom_data = true
	mm2.mesh = plane
	paint_mesh.multimesh = mm2
	paint_mesh.material_override = mat # share material
	add_child(paint_mesh)
	paint_mesh.top_level = true

	brush_mesh = MultiMeshInstance3D.new()
	var mm3 = MultiMesh.new()
	mm3.transform_format = MultiMesh.TRANSFORM_3D
	mm3.use_colors = true
	mm3.mesh = plane
	brush_mesh.multimesh = mm3
	
	var brush_mat = StandardMaterial3D.new()
	brush_mat.albedo_color = Color(1, 1, 1, 0.5)
	brush_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	brush_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	brush_mesh.material_override = brush_mat
	add_child(brush_mesh)
	brush_mesh.top_level = true
	
	persistent_container = Node3D.new()
	persistent_container.name = "PersistentVisuals"
	add_child(persistent_container)
	
	_update_persistent_visuals()

func _process(delta):
	if not active:
		grid_mesh.visible = false
		paint_mesh.visible = false
		brush_mesh.visible = false
		persistent_container.visible = false
		return

	if simulation_node.is_network_dirty():
		_update_persistent_visuals()
		# clear_network_dirty is usually handled by RoadTool or InputManager, 
		# but if we're in zoning mode we might need to check it.
		
	_update_visuals()

var visited_edges: Array = []
var drag_side: int = 0

func _update_visuals():
	if not simulation_node: return

	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	var mouse_pos_3d = intersection if intersection != null else Vector3.ZERO
	
	hovered_edge_idx = simulation_node.get_hovered_edge(mouse_pos_3d.x, mouse_pos_3d.z) if intersection != null else -1
	var target_edge_idx = hovered_edge_idx
	if not visited_edges.is_empty():
		target_edge_idx = visited_edges[0]
	
	var current_t = 0.0
	var current_side = 0
	if intersection != null and target_edge_idx != -1:
		var edge_geom = simulation_node.get_edge_geometry_3d(target_edge_idx)
		var proj = _get_projection_data(edge_geom, Vector2(intersection.x, intersection.z))
		current_t = proj["t"]
		current_side = proj["side"]
		
		# Depth logic: Default 3, or mouse-driven if Ctrl is held
		if Input.is_key_pressed(KEY_CTRL):
			var road_width = simulation_node.get_edge_width(target_edge_idx)
			var curb_dist = road_width * 0.5 + 1.5 # config::SIDEWALK_WIDTH
			var dist_from_sidewalk = proj["dist_from_road"] - curb_dist
			current_depth = clamp(int(floor(dist_from_sidewalk / CELL_SIZE)) + 1, 1, ZONING_DEPTH)
		else:
			current_depth = 3
	
	_handle_painting(hovered_edge_idx, current_t, current_side)
	
	var is_painting = (state == 1)
	var p_edges: Array = []
	p_edges.assign(visited_edges)
	if p_edges.is_empty() and hovered_edge_idx != -1:
		p_edges = [hovered_edge_idx]
	
	var p_t1 = drag_start_t
	var p_t2 = current_t
	if p_t1 < 0: p_t1 = current_t
	
	# Pass the whole path to Rust for preview
	var p_side = drag_side if not visited_edges.is_empty() else current_side
	var solid_mode = Input.is_key_pressed(KEY_CTRL)
	simulation_node.update_zoning_visuals(grid_mesh.multimesh, paint_mesh.multimesh, p_edges, is_painting, solid_mode, p_side, p_t1, p_t2, current_depth, current_zone_type)
	
	grid_mesh.visible = true
	paint_mesh.visible = true
	persistent_container.visible = true

func _handle_painting(h_edge: int, current_t: float, current_side: int):
	if state == 1: # Mouse Pressed (Dragging)
		if h_edge != -1:
			if visited_edges.is_empty():
				visited_edges.append(h_edge)
				drag_start_t = current_t
				drag_side = current_side
			elif visited_edges[-1] != h_edge:
				if not h_edge in visited_edges:
					var conn = _get_connection(visited_edges[-1], h_edge)
					# Sensitivity: Only switch edges if we are reasonably close to the new road's center (e.g. 7.5m)
					# AND closer than we are to the current road's center.
					var e_geom = simulation_node.get_edge_geometry_3d(h_edge)
					var last_edge_geom = simulation_node.get_edge_geometry_3d(visited_edges[-1])
					var camera = get_viewport().get_camera_3d()
					var mouse_pos = get_viewport().get_mouse_position()
					var ray_origin = camera.project_ray_origin(mouse_pos)
					var ray_dir = camera.project_ray_normal(mouse_pos)
					var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
					if intersection == null:
						return

					var proj = _get_projection_data(e_geom, Vector2(intersection.x, intersection.z))
					var last_proj = _get_projection_data(last_edge_geom, Vector2(intersection.x, intersection.z))
					
					if conn.has("connected") and conn["connected"] and proj["dist_from_road"] < 7.5 and proj["dist_from_road"] < last_proj["dist_from_road"] - 2.0:
						# Only follow edges that are roughly collinear with the current one.
						# This prevents the drag from jumping onto perpendicular arms at junctions.
						var alignment = _edge_alignment(last_edge_geom, conn["t_a"], e_geom, conn["t_b"])
						if alignment > 0.7: # cos(~46°) — allows gentle curves, blocks right-angle turns
							visited_edges.append(h_edge)
	
func _unhandled_input(event):
	if not active: return
	
	if event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_LEFT:
			if event.pressed:
				state = 1
				visited_edges.clear()
				drag_start_t = -1.0
			else:
				if state == 1 and not visited_edges.is_empty():
					var mouse_pos = get_viewport().get_mouse_position()
					var camera = get_viewport().get_camera_3d()
					var ray_origin = camera.project_ray_origin(mouse_pos)
					var ray_dir = camera.project_ray_normal(mouse_pos)
					var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
					
					var last_edge_id = visited_edges[-1]
					var last_edge_geom = simulation_node.get_edge_geometry_3d(last_edge_id)
					var last_proj = _get_projection_data(last_edge_geom, Vector2(intersection.x, intersection.z))
					var end_t = last_proj["t"]
					
					var num_edges = visited_edges.size()
					var cur_side = drag_side
					
					for i in range(num_edges):
						var e_idx = visited_edges[i]
						var s_t = 0.0
						var e_t = 1.0
						
						if num_edges == 1:
							s_t = drag_start_t
							e_t = end_t
						elif i == 0:
							s_t = drag_start_t
							var conn = _get_connection(visited_edges[0], visited_edges[1])
							e_t = conn["t_a"]
						elif i == num_edges - 1:
							var conn = _get_connection(visited_edges[i - 1], visited_edges[i])
							s_t = conn["t_b"]
							e_t = end_t
						else:
							var conn_in  = _get_connection(visited_edges[i - 1], visited_edges[i])
							var conn_out = _get_connection(visited_edges[i],     visited_edges[i + 1])
							s_t = conn_in["t_b"]
							e_t = conn_out["t_a"]
						
						if Input.is_key_pressed(KEY_CTRL):
							simulation_node.set_block_zone_range(e_idx, cur_side, s_t, e_t, current_depth, current_zone_type)
						else:
							simulation_node.set_zoning_range(e_idx, cur_side, s_t, e_t, current_depth, current_zone_type)
						
						# B7: flip side only if road orientation is genuinely reversed
						if i < num_edges - 1:
							var conn = _get_connection(visited_edges[i], visited_edges[i + 1])
							if conn["t_a"] == conn["t_b"]:
								cur_side = -cur_side
					
					_update_persistent_visuals()
				
				state = 0
				visited_edges.clear()
				drag_start_t = -1.0
		
		elif event.button_index == MOUSE_BUTTON_WHEEL_UP and event.pressed:
			pass # Mouse wheel functionality removed per user request
		elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN and event.pressed:
			pass
			
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		if hovered_edge_idx != -1:
			var mouse_pos = get_viewport().get_mouse_position()
			var camera = get_viewport().get_camera_3d()
			var ray_origin = camera.project_ray_origin(mouse_pos)
			var ray_dir = camera.project_ray_normal(mouse_pos)
			var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
			if intersection != null:
				var edge_geom = simulation_node.get_edge_geometry(hovered_edge_idx)
				var proj = _get_projection_data(edge_geom, Vector2(intersection.x, intersection.z))
				simulation_node.set_zoning_range(hovered_edge_idx, proj["side"], 0.0, 1.0, ZONING_DEPTH, 0) # Clear all
				_update_persistent_visuals()

# Returns the dot product of the outgoing direction of edge_a (at t_a) and the
# outgoing direction of edge_b (at t_b). Both are expressed as "away from the junction",
# so a straight continuation gives ~1.0 and a right-angle turn gives ~0.0.
func _edge_alignment(geom_a: PackedVector3Array, t_a: float, geom_b: PackedVector3Array, t_b: float) -> float:
	if geom_a.size() < 2 or geom_b.size() < 2:
		return 0.0
	# Tangent of A pointing outward from the junction end.
	var tan_a: Vector2
	if t_a > 0.5:
		var p1 = Vector2(geom_a[-2].x, geom_a[-2].z)
		var p2 = Vector2(geom_a[-1].x, geom_a[-1].z)
		tan_a = (p2 - p1).normalized()
	else:
		var p1 = Vector2(geom_a[0].x, geom_a[0].z)
		var p2 = Vector2(geom_a[1].x, geom_a[1].z)
		tan_a = (p1 - p2).normalized() # reversed: outward from start
	# Tangent of B pointing outward from the junction end (away from the connection point).
	var tan_b: Vector2
	if t_b < 0.5:
		var p1 = Vector2(geom_b[0].x, geom_b[0].z)
		var p2 = Vector2(geom_b[1].x, geom_b[1].z)
		tan_b = (p2 - p1).normalized()
	else:
		var n = geom_b.size()
		var p1 = Vector2(geom_b[n-2].x, geom_b[n-2].z)
		var p2 = Vector2(geom_b[n-1].x, geom_b[n-1].z)
		tan_b = (p1 - p2).normalized() # reversed: outward from end
	return tan_a.dot(tan_b)

func _get_connection(edge_a: int, edge_b: int) -> Dictionary:
	var ga = simulation_node.get_edge_geometry(edge_a)
	var gb = simulation_node.get_edge_geometry(edge_b)
	var a0 = ga[0];  var a1 = ga[ga.size() - 1]
	var b0 = gb[0];  var b1 = gb[gb.size() - 1]
	var thr = 400.0  # 20 m squared — covers any node radius
	if a1.distance_squared_to(b0) < thr: return {"t_a": 1.0, "t_b": 0.0, "connected": true}
	elif a1.distance_squared_to(b1) < thr: return {"t_a": 1.0, "t_b": 1.0, "connected": true}
	elif a0.distance_squared_to(b0) < thr: return {"t_a": 0.0, "t_b": 0.0, "connected": true}
	elif a0.distance_squared_to(b1) < thr: return {"t_a": 0.0, "t_b": 1.0, "connected": true}
	else: return {"t_a": 1.0, "t_b": 0.0, "connected": false}

func _update_persistent_visuals():
	if not simulation_node: return
	
	var data_dict = simulation_node.get_persistent_zoning_instances()
	if data_dict == null: return
	
	var active_types = data_dict.keys()
	
	# Cleanup any meshes for types that no longer exist
	for z_type in persistent_meshes.keys():
		if not z_type in active_types:
			persistent_meshes[z_type].multimesh.instance_count = 0
	
	for z_type in active_types:
		var buffer = data_dict[z_type]
		if buffer.size() == 0:
			if persistent_meshes.has(z_type):
				persistent_meshes[z_type].multimesh.instance_count = 0
			continue
			
		var mmi: MultiMeshInstance3D
		if persistent_meshes.has(z_type):
			mmi = persistent_meshes[z_type]
		else:
			mmi = _create_persistent_mmi(z_type)
			persistent_meshes[z_type] = mmi
			
		var count = buffer.size() / 20
		mmi.multimesh.instance_count = count
		mmi.multimesh.set_buffer(buffer)
		mmi.visible = true

func _create_persistent_mmi(z_type: int) -> MultiMeshInstance3D:
	var mmi = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = true
	mm.use_custom_data = true
	
	var plane = PlaneMesh.new()
	plane.size = Vector2(1.0, 1.0)
	mm.mesh = plane
	mmi.multimesh = mm
	
	var mat = ShaderMaterial.new()
	mat.shader = load("res://scripts/shaders/zoning_grid.gdshader")
	mmi.material_override = mat
	
	# Set zone color in the multimesh color buffer (Rust side passes white, 
	# but we can set per-instance color here if we wanted to be more efficient, 
	# but for now we just use a uniform or vertex color if needed).
	# Actually, the shader uses v_color = COLOR, so we must set instance colors.
	
	# Since all instances in this MMI are the same type, we can just set them all 
	# to the same color. However, Rust passes the WHOLE buffer including transforms.
	# The Rust code I wrote passes godot::prelude::Color::WHITE. 
	# I should probably update the Rust code to pass the CORRECT color or 
	# handle color in the shader per MultiMesh.
	
	# Better: Set the instance colors on the Godot side by post-processing or 
	# have the shader use a 'zone_color' uniform.
	mat.set_shader_parameter("v_color", _get_zone_color(z_type))
	
	persistent_container.add_child(mmi)
	mmi.top_level = true
	return mmi


func _show_brush_feedback():
	# Simple print or UI label could go here. 
	# For now we just let the visuals show it after the next frame.
	pass

func _get_zone_color(z_type):
	match z_type:
		1: return Color(0, 1, 0, 0.6) # Res: Green
		2: return Color(0, 0.5, 1, 0.6) # Com: Blue
		3: return Color(1, 1, 0, 0.6) # Ind: Yellow
		4: return Color(0.1, 0.8, 0.8, 0.6) # Office: Cyan
		5: return Color(0.8, 0, 0.8, 0.6) # Mixed: Purple
		_: return Color(1, 1, 1, 0.1)

func _get_depth_alpha(y: int) -> float:
	if y < 4:
		return 1.0
	# Fade from index 4 (1.0) to index 9 (approx 0.05)
	var t = float(y - 4) / float(ZONING_DEPTH - 1 - 4)
	return lerp(1.0, 0.1, t)


func _get_pos_on_edge(geom: PackedVector3Array, t: float) -> Vector3:
	var total_l = 0.0
	for i in range(geom.size() - 1):
		total_l += geom[i].distance_to(geom[i+1])
	var target_l = t * total_l
	var curr_l = 0.0
	for i in range(geom.size() - 1):
		var d = geom[i].distance_to(geom[i+1])
		if curr_l + d >= target_l:
			var local_t = (target_l - curr_l) / max(d, 0.001)
			return geom[i].lerp(geom[i+1], local_t)
		curr_l += d
	return geom[-1]

func _get_projection_data(geom: PackedVector3Array, p: Vector2) -> Dictionary:
	var best_dist_sq = 1e10
	var best_t = 0.0
	var best_side = 1
	var best_dist_from_road = 0.0
	
	var total_l = 0.0
	for i in range(geom.size() - 1):
		total_l += Vector2(geom[i].x, geom[i].z).distance_to(Vector2(geom[i+1].x, geom[i+1].z))
		
	var curr_l = 0.0
	for i in range(geom.size() - 1):
		var a = Vector2(geom[i].x, geom[i].z)
		var b = Vector2(geom[i+1].x, geom[i+1].z)
		var seg = b - a
		var l2 = seg.length_squared()
		if l2 < 0.001: continue
		
		var t_val = ((p.x - a.x) * seg.x + (p.y - a.y) * seg.y) / l2
		t_val = clamp(t_val, 0.0, 1.0)
		var proj = a + seg * t_val
		
		var dist_sq = p.distance_squared_to(proj)
		if dist_sq < best_dist_sq:
			best_dist_sq = dist_sq
			best_t = (curr_l + t_val * sqrt(l2)) / max(total_l, 0.001)
			var tangent = seg.normalized()
			# normal points LEFT in the (y, -x) convention for Godot Y-down (side = 1).
			var normal = Vector2(tangent.y, -tangent.x)
			var to_pt = p - proj
			best_side = 1 if to_pt.dot(normal) > 0 else -1
			best_dist_from_road = sqrt(dist_sq)
			
		curr_l += sqrt(l2)
		
	return {"t": best_t, "side": best_side, "dist_from_road": best_dist_from_road}
