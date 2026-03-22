extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var current_zone_type: int = 1 # 1: Res, 2: Com, 3: Ind, 4: Office, 5: Mixed
var state: int = 0 # 0: Idle, 1: Painting
var active: bool = false
var hovered_edge_idx: int = -1

var grid_mesh: MultiMeshInstance3D
var paint_mesh: MultiMeshInstance3D

func _ready():
	grid_mesh = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = true
	mm.use_custom_data = true
	
	var box = BoxMesh.new()
	box.size = Vector3(7.5, 0.1, 7.5) # cell size 8.0, leave a gap
	mm.mesh = box
	grid_mesh.multimesh = mm
	
	var mat = StandardMaterial3D.new()
	mat.albedo_color = Color(1, 1, 1, 0.3)
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.vertex_color_use_as_albedo = true
	box.surface_set_material(0, mat)
	
	add_child(grid_mesh)
	grid_mesh.top_level = true
	
	paint_mesh = MultiMeshInstance3D.new()
	var mm2 = MultiMesh.new()
	mm2.transform_format = MultiMesh.TRANSFORM_3D
	mm2.use_colors = true
	mm2.use_custom_data = true
	mm2.mesh = box
	paint_mesh.multimesh = mm2
	paint_mesh.material_override = mat # share material
	add_child(paint_mesh)
	paint_mesh.top_level = true

func _process(delta):
	if not active:
		grid_mesh.visible = false
		return

	_update_visuals()
	_handle_painting()

func _update_visuals():
	if not simulation_node: return

	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	
	if intersection != null:
		hovered_edge_idx = simulation_node.get_hovered_edge(intersection.x, intersection.z)
	else:
		hovered_edge_idx = -1
	
	var data = simulation_node.get_all_zoning_preview_data()
	var count = data.size() / 4
	
	grid_mesh.multimesh.instance_count = count
	for i in range(count):
		var x = data[i*4+0]
		var z = data[i*4+1]
		var tx = data[i*4+2]
		var ty = data[i*4+3]
		
		var t = Transform3D()
		t.origin = Vector3(x, 0.1, z)
		
		# Build rotation from tangent
		var basis = Basis()
		basis.x = Vector3(tx, 0, ty) # Tangent
		basis.y = Vector3(0, 1, 0)
		basis.z = Vector3(-ty, 0, tx) # Normal
		t.basis = basis
		
		grid_mesh.multimesh.set_instance_transform(i, t)
		grid_mesh.multimesh.set_instance_color(i, Color(0.1, 0.8, 0.1, 0.3)) # Holographic green
	
	grid_mesh.visible = true
	_draw_all_painted()

func _draw_all_painted():
	var data = simulation_node.get_zoning_grid_data()
	# data: [edge_idx, side, x, y, type]
	var count = data.size() / 5
	paint_mesh.multimesh.instance_count = count
	
	for i in range(count):
		var edge_idx = int(data[i*5])
		var side = int(data[i*5+1])
		var x = int(data[i*5+2])
		var y = int(data[i*5+3])
		var z_type = int(data[i*5+4])
		
		if simulation_node.is_zoning_cell_obstructed(edge_idx, side, x, y): continue
		
		var edge_geom = simulation_node.get_edge_geometry(edge_idx)
		var edge_width = simulation_node.get_edge_width(edge_idx)
		var cell_size = 8.0 
		
		var t = (x as float + 0.5) * cell_size / _get_edge_length(edge_geom)
		var pos_on_edge = _get_pos_on_edge(edge_geom, t)
		var tangent = _get_tangent_on_edge(edge_geom, t)
		var normal = Vector2(-tangent.y, tangent.x) * side
		
		var depth_offset = (y as float + 0.5) * cell_size
		var center_2d = pos_on_edge + normal * (edge_width * 0.5 + depth_offset)
		var world_y = simulation_node.get_height_at(center_2d) + 0.4
		
		var tr = Transform3D()
		var angle = atan2(-tangent.y, tangent.x)
		tr = tr.rotated(Vector3.UP, angle)
		tr.origin = Vector3(center_2d.x, world_y, center_2d.y)
		paint_mesh.multimesh.set_instance_transform(i, tr)
		
		var color = _get_zone_color(z_type)
		paint_mesh.multimesh.set_instance_color(i, color)

func _handle_painting():
	if state == 1 and hovered_edge_idx != -1:
		var mouse_pos = get_viewport().get_mouse_position()
		var camera = get_viewport().get_camera_3d()
		var ray_origin = camera.project_ray_origin(mouse_pos)
		var ray_dir = camera.project_ray_normal(mouse_pos)
		var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
		if intersection == null: return
		
		var world_pos = Vector2(intersection.x, intersection.z)
		
		# Find which cell we are over
		var edge_geom = simulation_node.get_edge_geometry(hovered_edge_idx)
		var edge_width = simulation_node.get_edge_width(hovered_edge_idx)
		var l = _get_edge_length(edge_geom)
		if l < 0.1: return
		
		# Project world_pos onto edge to find side and T
		var proj = _get_projection_data(edge_geom, world_pos)
		var side = proj["side"]
		var t = proj["t"]
		var dist_from_edge = proj["dist_from_road"]
		
		var cell_x = floor(t * l / 8.0)
		var cell_y = floor((dist_from_edge - edge_width * 0.5) / 8.0)
		
		if cell_y >= 0 and cell_y < 4:
			simulation_node.set_zoning_cell(hovered_edge_idx, side, cell_x, cell_y, current_zone_type)

func _unhandled_input(event):
	if not active: return
	
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			state = 1
		else:
			state = 0
			
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		# right click to clear
		var old_type = current_zone_type
		current_zone_type = 0 # None
		state = 1
		await get_tree().create_timer(0.1).timeout
		state = 0
		current_zone_type = old_type

func _get_zone_color(z_type):
	match z_type:
		1: return Color(0, 1, 0, 0.6) # Res: Green
		2: return Color(0, 0.5, 1, 0.6) # Com: Blue
		3: return Color(1, 1, 0, 0.6) # Ind: Yellow
		4: return Color(0.1, 0.8, 0.8, 0.6) # Office: Cyan
		5: return Color(0.8, 0, 0.8, 0.6) # Mixed: Purple
		_: return Color(1, 1, 1, 0.1)

func _get_edge_length(geom: PackedVector2Array) -> float:
	var l = 0.0
	for i in range(geom.size() - 1):
		l += geom[i].distance_to(geom[i+1])
	return l

func _get_pos_on_edge(geom: PackedVector2Array, t: float) -> Vector2:
	var total_l = _get_edge_length(geom)
	var target_l = t * total_l
	var curr_l = 0.0
	for i in range(geom.size() - 1):
		var d = geom[i].distance_to(geom[i+1])
		if curr_l + d >= target_l:
			var local_t = (target_l - curr_l) / d
			return geom[i].lerp(geom[i+1], local_t)
		curr_l += d
	return geom[-1]

func _get_tangent_on_edge(geom: PackedVector2Array, t: float) -> Vector2:
	var total_l = _get_edge_length(geom)
	var target_l = t * total_l
	var curr_l = 0.0
	for i in range(geom.size() - 1):
		var d = geom[i].distance_to(geom[i+1])
		if curr_l + d >= target_l:
			return (geom[i+1] - geom[i]).normalized()
		curr_l += d
	return (geom[-1] - geom[-2]).normalized()

func _get_projection_data(geom: PackedVector2Array, p: Vector2) -> Dictionary:
	var best_dist_sq = 1e10
	var best_t = 0.0
	var best_side = 1
	var best_dist_from_road = 0.0
	
	var total_l = _get_edge_length(geom)
	var curr_l = 0.0
	
	for i in range(geom.size() - 1):
		var a = geom[i]
		var b = geom[i+1]
		var seg = b - a
		var l2 = seg.length_squared()
		if l2 < 0.001: continue
		
		var t_val = ((p.x - a.x) * seg.x + (p.y - a.y) * seg.y) / l2
		t_val = clamp(t_val, 0.0, 1.0)
		var proj = a + seg * t_val
		
		var dist_sq = p.distance_squared_to(proj)
		if dist_sq < best_dist_sq:
			best_dist_sq = dist_sq
			best_t = (curr_l + t_val * sqrt(l2)) / total_l
			var tangent = seg.normalized()
			var normal = Vector2(-tangent.y, tangent.x)
			var to_pt = p - proj
			best_side = 1 if to_pt.dot(normal) > 0 else -1
			best_dist_from_road = sqrt(dist_sq)
			
		curr_l += sqrt(l2)
		
	return {"t": best_t, "side": best_side, "dist_from_road": best_dist_from_road}
