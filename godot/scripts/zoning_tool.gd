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

enum Mode { SINGLE, PAINT, FILL, DELETE }
var current_mode = Mode.SINGLE

const ZONING_DEPTH: int = 10
const CELL_SIZE: float = 10.0

var grid_mesh: MultiMeshInstance3D
var paint_mesh: MultiMeshInstance3D
var brush_mesh: MultiMeshInstance3D

func _ready():
	grid_mesh = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.use_colors = true
	mm.use_custom_data = false
	
	var box = BoxMesh.new()
	box.size = Vector3(CELL_SIZE - 0.5, 0.1, CELL_SIZE - 0.5) # cell size 10.0, leave a gap
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
	mm2.use_custom_data = false
	mm2.mesh = box
	paint_mesh.multimesh = mm2
	paint_mesh.material_override = mat # share material
	add_child(paint_mesh)
	paint_mesh.top_level = true

	brush_mesh = MultiMeshInstance3D.new()
	var mm3 = MultiMesh.new()
	mm3.transform_format = MultiMesh.TRANSFORM_3D
	mm3.use_colors = true
	mm3.mesh = box
	brush_mesh.multimesh = mm3
	
	var brush_mat = StandardMaterial3D.new()
	brush_mat.albedo_color = Color(1, 1, 1, 0.5)
	brush_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	brush_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	brush_mesh.material_override = brush_mat
	add_child(brush_mesh)
	brush_mesh.top_level = true

func _process(delta):
	if not active:
		grid_mesh.visible = false
		paint_mesh.visible = false
		brush_mesh.visible = false
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
	var mouse_pos_3d = intersection if intersection != null else Vector3.ZERO
	var h_edge = simulation_node.get_hovered_edge(mouse_pos_3d.x, mouse_pos_3d.z) if intersection != null else -1
	hovered_edge_idx = h_edge
	
	simulation_node.update_zoning_visuals(grid_mesh.multimesh, paint_mesh.multimesh, h_edge, current_mode, mouse_pos_3d)
	
	grid_mesh.visible = true
	paint_mesh.visible = true

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
		
		var cell_x = floor(t * l / CELL_SIZE)
		var cell_y = floor((dist_from_edge - edge_width * 0.5) / CELL_SIZE)
		
		if cell_y >= 0 and cell_y < ZONING_DEPTH:
			match current_mode:
				Mode.SINGLE:
					simulation_node.set_zoning_cell(hovered_edge_idx, side, cell_x, cell_y, current_zone_type)
				Mode.PAINT:
					# 3x3 brush
					for dx in range(-1, 2):
						for dy in range(-1, 2):
							var bx = cell_x + dx
							var by = cell_y + dy
							if by >= 0 and by < ZONING_DEPTH:
								simulation_node.set_zoning_cell(hovered_edge_idx, side, bx, by, current_zone_type)
				Mode.DELETE:
					# This mode is handled in _handle_click or by setting enabled=false
					simulation_node.set_zoning_enabled(hovered_edge_idx, side, false)
					state = 0 # Single click tool

func _unhandled_input(event):
	if not active: return
	
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			if (current_mode == Mode.FILL or current_mode == Mode.DELETE) and hovered_edge_idx != -1:
				if current_mode == Mode.FILL: _handle_fill()
				else: _handle_side_delete()
			else:
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

func _handle_fill():
	if hovered_edge_idx == -1: return
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if intersection == null: return
	var world_pos = Vector2(intersection.x, intersection.z)
	var edge_geom = simulation_node.get_edge_geometry(hovered_edge_idx)
	var proj = _get_projection_data(edge_geom, world_pos)
	var side = proj["side"]
	
	# Fill all ZONING_DEPTH rows along the whole edge
	var l = _get_edge_length(edge_geom)
	var cells_long = floor(l / CELL_SIZE)
	for x in range(cells_long):
		for y in range(ZONING_DEPTH):
			simulation_node.set_zoning_cell(hovered_edge_idx, side, x, y, current_zone_type)

func _handle_side_delete():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if intersection == null: return
	var world_pos = Vector2(intersection.x, intersection.z)
	var edge_geom = simulation_node.get_edge_geometry(hovered_edge_idx)
	var proj = _get_projection_data(edge_geom, world_pos)
	simulation_node.set_zoning_enabled(hovered_edge_idx, proj["side"], false)

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
			var normal = Vector2(tangent.y, -tangent.x)
			var to_pt = p - proj
			best_side = 1 if to_pt.dot(normal) > 0 else -1
			best_dist_from_road = sqrt(dist_sq)
			
		curr_l += sqrt(l2)
		
	return {"t": best_t, "side": best_side, "dist_from_road": best_dist_from_road}
