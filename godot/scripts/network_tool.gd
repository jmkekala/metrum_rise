extends Node3D
class_name NetworkTool

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"

var active: bool = false
var is_valid: bool = true
var blue_color = Color(0.0, 1.0, 1.0, 0.6)
var red_color = Color(1.0, 0.1, 0.1, 0.6)

var mesh_instance: MeshInstance3D # The final road mesh
var blueprint_mesh: MeshInstance3D # The preview line/spline
var blueprint_mat: StandardMaterial3D
var node_multimesh: MultiMeshInstance3D # Holographic snapping points
var cursor_mesh: MeshInstance3D # Active hovered snap cursor

func _ready():
	_setup_visuals()

func _setup_visuals():
	# Final mesh container
	mesh_instance = MeshInstance3D.new()
	add_child(mesh_instance)
	
	# Blueprint mesh container
	blueprint_mesh = MeshInstance3D.new()
	add_child(blueprint_mesh)
	
	blueprint_mat = StandardMaterial3D.new()
	blueprint_mat.albedo_color = blue_color
	blueprint_mat.emission_enabled = true
	blueprint_mat.emission = Color(0.0, 0.5, 0.5)
	blueprint_mat.transparency = StandardMaterial3D.TRANSPARENCY_ALPHA
	blueprint_mat.cull_mode = StandardMaterial3D.CULL_DISABLED
	blueprint_mat.shading_mode = StandardMaterial3D.SHADING_MODE_UNSHADED
	blueprint_mesh.material_override = blueprint_mat
	
	# Node Snapping Highlights
	node_multimesh = MultiMeshInstance3D.new()
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.mesh = SphereMesh.new()
	mm.mesh.radius = 1.0
	mm.mesh.height = 0.2 # flat disc
	
	var mm_mat = StandardMaterial3D.new()
	mm_mat.albedo_color = Color(0.3, 0.8, 1.0, 0.5) # Light blue indicator
	mm_mat.emission_enabled = true
	mm_mat.emission = Color(0.1, 0.5, 0.8)
	mm_mat.transparency = StandardMaterial3D.TRANSPARENCY_ALPHA
	mm_mat.shading_mode = StandardMaterial3D.SHADING_MODE_UNSHADED
	mm_mat.cull_mode = StandardMaterial3D.CULL_DISABLED
	mm.mesh.surface_set_material(0, mm_mat)
	
	node_multimesh.multimesh = mm
	node_multimesh.position.y += 0.05 # Elevated slightly to prevent Asphalt Z-fighting
	add_child(node_multimesh)
	
	# Active hover cursor
	cursor_mesh = MeshInstance3D.new()
	cursor_mesh.mesh = SphereMesh.new()
	cursor_mesh.mesh.radius = 1.5
	cursor_mesh.mesh.height = 0.3
	var cm_mat = StandardMaterial3D.new()
	cm_mat.albedo_color = Color(0.0, 1.0, 0.5, 0.7) # Green targeting cursor
	cm_mat.emission_enabled = true
	cm_mat.emission = Color(0.0, 0.8, 0.2)
	cm_mat.transparency = StandardMaterial3D.TRANSPARENCY_ALPHA
	cm_mat.shading_mode = StandardMaterial3D.SHADING_MODE_UNSHADED
	cm_mat.cull_mode = StandardMaterial3D.CULL_DISABLED
	cursor_mesh.material_override = cm_mat
	add_child(cursor_mesh)

func _process(_delta):
	if active:
		var pos = get_world_mouse_pos()
		if cursor_mesh:
			cursor_mesh.global_position = pos
			cursor_mesh.global_position.y += 0.35 # Float above road geometries
			cursor_mesh.visible = is_valid
			
		_update_blueprint_visuals()
		_update_node_visuals()
	else:
		if cursor_mesh:
			cursor_mesh.visible = false
		if node_multimesh and node_multimesh.multimesh:
			node_multimesh.multimesh.instance_count = 0

func _update_node_visuals():
	if not simulation_node: return
	var nodes = simulation_node.get_network_nodes()
	if nodes == null: return
	var mm = node_multimesh.multimesh
	mm.instance_count = nodes.size()
	for i in range(nodes.size()):
		var t = Transform3D()
		t.origin = nodes[i]
		t.origin.y += 0.3 # Elevate above the 0.15m asphalt and 0.05m kerb
		mm.set_instance_transform(i, t)

func _update_blueprint_visuals():
	if is_valid:
		blueprint_mat.albedo_color = blue_color
		blueprint_mat.emission = Color(0.0, 0.5, 0.5)
	else:
		blueprint_mat.albedo_color = red_color
		blueprint_mat.emission = Color(0.5, 0.0, 0.0)


func get_terrain_interaction():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return null
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	
	# HIGH-PRECISION RUST RAYCAST
	return simulation_node.intersect_terrain(ray_origin, ray_dir)

func get_world_mouse_pos() -> Vector3:
	var pos_variant = get_terrain_interaction()
	if pos_variant == null: 
		is_valid = false
		return Vector3.ZERO
	
	var pos: Vector3 = pos_variant
	
	# 1. Snap to existing network (High priority)
	# Enlarged from 2.5m to 5.0m to visibly catch strokes hovering over the 3.5m asphalt mesh radius!
	var snapped_pos = simulation_node.get_closest_network_point(pos, 5.0)
	if snapped_pos != null:
		is_valid = true
		return snapped_pos
	
	# 2. Dead Zone Check (Too close but not snapped)
	var too_close_pos = simulation_node.get_closest_network_point(pos, 8.0)
	if too_close_pos != null:
		is_valid = false
		return pos
	
	is_valid = true
	return pos

func get_height_at(_world_pos: Vector3) -> float:
	# Deprecated: use simulation_node.get_height_at instead
	return 0.0

func update_main_mesh():
	var data = simulation_node.get_road_mesh_data()
	var verts = data["vertices"]
	if verts.size() == 0: return
	
	var arr_mesh = ArrayMesh.new()
	var arrays = []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = verts
	arrays[Mesh.ARRAY_NORMAL] = data["normals"]
	arrays[Mesh.ARRAY_TEX_UV] = data["uvs"]
	arrays[Mesh.ARRAY_COLOR] = data["colors"]
	
	# Surface 0: Asphalt Base
	arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	
	# Surface 0: Asphalt Base
	arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)

	mesh_instance.mesh = arr_mesh
	
	# Assign Material
	var asph_mat = ShaderMaterial.new()
	asph_mat.shader = load("res://assets/materials/road.gdshader")
	mesh_instance.set_surface_override_material(0, asph_mat)
