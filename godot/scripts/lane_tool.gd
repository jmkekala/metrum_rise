extends Node3D

@onready var simulation_node = $"../SimulationNode"

var active: bool = false
var hovered_node: int = -1
var selected_node: int = -1

var lane_spheres = []
var connection_lines = []

var dragging = false
var drag_start_lane = null
var drag_from_edge = -1
var drag_from_lane = 0
var drag_is_incoming = false

var drag_line_mesh: MeshInstance3D

func _ready():
	drag_line_mesh = MeshInstance3D.new()
	add_child(drag_line_mesh)

func _process(delta):
	if Input.is_key_pressed(KEY_T):
		if not active:
			print("Traffic Lane Manager Activated (T)")
			active = true
	if Input.is_key_pressed(KEY_ESCAPE) or Input.is_key_pressed(KEY_Z) or Input.is_key_pressed(KEY_ALT):
		if active:
			print("Traffic Lane Manager Deactivated")
			active = false
			clear_visuals()
			selected_node = -1
	
	if not active: return
	
	if dragging:
		_update_drag_line()
	elif selected_node == -1:
		_highlight_closest_node()
		
func _input(event):
	if not active: return
	
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			if selected_node == -1 and hovered_node != -1:
				selected_node = hovered_node
				_build_node_visuals(selected_node)
			elif selected_node != -1:
				var clicked_lane = _get_hovered_lane_sphere()
				if clicked_lane != null and clicked_lane.is_incoming:
					dragging = true
					drag_start_lane = clicked_lane.pos
					drag_from_edge = clicked_lane.edge_id
					drag_from_lane = clicked_lane.lane_id
					drag_is_incoming = clicked_lane.is_incoming
		else:
			if dragging:
				dragging = false
				drag_line_mesh.mesh = null
				var target_lane = _get_hovered_lane_sphere()
				if target_lane != null:
					if target_lane.edge_id == drag_from_edge and target_lane.lane_id == drag_from_lane:
						simulation_node.clear_lane_source(selected_node, drag_from_edge, drag_from_lane)
						print("Cleared connections for Edge ", drag_from_edge, " Lane ", drag_from_lane)
					elif not target_lane.is_incoming:
						simulation_node.set_lane_connection(selected_node, drag_from_edge, drag_from_lane, target_lane.edge_id, target_lane.lane_id)
						print("Connected Edge ", drag_from_edge, " Lane ", drag_from_lane, " to Edge ", target_lane.edge_id, " Lane ", target_lane.lane_id)
					_build_node_visuals(selected_node)
					
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		if selected_node != -1:
			simulation_node.clear_lane_connections(selected_node)
			_build_node_visuals(selected_node)
			selected_node = -1

func _highlight_closest_node():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var pos = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if pos == null: return
	
	var nearest_node = simulation_node.get_closest_node(pos, 5.0)
	hovered_node = nearest_node
	# Could flash or draw a circle to indicate hover. It works silently for now.

func _build_node_visuals(node_id: int):
	clear_visuals()
	
	var lanes = simulation_node.get_node_lanes(node_id)
	for lane in lanes:
		var edge_id = lane["edge_id"]
		var lane_id = lane["lane_id"]
		var is_inc = lane["is_incoming"]
		var pos = lane["pos"]
		
		var mesh_inst = MeshInstance3D.new()
		add_child(mesh_inst)
		var sphere = SphereMesh.new()
		sphere.radius = 0.5
		sphere.height = 1.0
		mesh_inst.mesh = sphere
		mesh_inst.global_position = pos
		
		var mat = StandardMaterial3D.new()
		mat.albedo_color = Color(0.2, 1.0, 0.2) if is_inc else Color(1.0, 0.2, 0.2)
		mat.emission_enabled = true
		mat.emission = mat.albedo_color
		mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		mesh_inst.material_override = mat
		
		lane_spheres.push_back({
			"edge_id": edge_id,
			"lane_id": lane_id,
			"is_incoming": is_inc,
			"pos": pos,
			"mesh_inst": mesh_inst
		})
		
	var connections = simulation_node.get_lane_connections_array(node_id)
	for conn in connections:
		var f_e = int(conn["from_edge"])
		var f_l = int(conn["from_lane"])
		var t_e = int(conn["to_edge"])
		var t_l = int(conn["to_lane"])
		
		var p1 = null
		var p2 = null
		for s in lane_spheres:
			if int(s.edge_id) == f_e and int(s.lane_id) == f_l and s.is_incoming: p1 = s.pos
			if int(s.edge_id) == t_e and int(s.lane_id) == t_l and not s.is_incoming: p2 = s.pos
			
		if p1 != null and p2 != null:
			var line_inst = MeshInstance3D.new()
			add_child(line_inst)
			_draw_arch(line_inst, p1, p2, Color(1.0, 1.0, 0.0))
			connection_lines.push_back(line_inst)
		else:
			print("Could not find matching spheres! p1: ", str(p1), ", p2: ", str(p2))

func _get_hovered_lane_sphere():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return null
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var pos_variant = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if pos_variant == null: return null
	var pos: Vector3 = pos_variant
	
	var best_dist = 4.0
	var best_lane = null
	for s in lane_spheres:
		var d = pos.distance_to(s.pos)
		if d < best_dist:
			best_dist = d
			best_lane = s
	return best_lane
	
func _update_drag_line():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var pos_variant = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if pos_variant == null: return
	var drag_end: Vector3 = pos_variant
	drag_end.y += 0.5
	
	var target = _get_hovered_lane_sphere()
	if target != null and not target.is_incoming:
		drag_end = target.pos
	
	_draw_arch(drag_line_mesh, drag_start_lane, drag_end, Color(0.0, 1.0, 1.0))

func _draw_arch(mesh_inst: MeshInstance3D, p1: Vector3, p2: Vector3, color: Color):
	var arr_mesh = ArrayMesh.new()
	var arrays = []
	arrays.resize(Mesh.ARRAY_MAX)
	var ribbon_verts = PackedVector3Array()
	
	var segments = 10
	var dist = p1.distance_to(p2)
	for i in range(segments + 1):
		var t = float(i) / segments
		var p = p1.lerp(p2, t)
		p.y += sin(t * PI) * min(dist * 0.2, 2.0)
		
		# Simple 2D billboard thickness
		var camera = get_viewport().get_camera_3d()
		var view_dir = (camera.global_position - p).normalized()
		var fwd = (p2 - p1).normalized()
		var right = fwd.cross(view_dir).normalized()
		if right.length_squared() < 0.001: right = Vector3(1, 0, 0)
		
		ribbon_verts.push_back(p - right * 0.15)
		ribbon_verts.push_back(p + right * 0.15)
		
	arrays[Mesh.ARRAY_VERTEX] = ribbon_verts
	arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLE_STRIP, arrays)
	mesh_inst.mesh = arr_mesh
	
	var mat = StandardMaterial3D.new()
	mat.albedo_color = color
	mat.emission_enabled = true
	mat.emission = color
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mesh_inst.material_override = mat

func clear_visuals():
	for s in lane_spheres:
		s.mesh_inst.queue_free()
	lane_spheres.clear()
	for l in connection_lines:
		l.queue_free()
	connection_lines.clear()
	if drag_line_mesh: drag_line_mesh.mesh = null
