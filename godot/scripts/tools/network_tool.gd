## Base class for all road-network editing tools (RoadTool, MoveTool, CulDeSacTool).
##
## Rust methods called: add_road(), get_closest_network_point(), get_closest_node(),
##   get_road_mesh_data(), get_network_nodes(), get_node_pos(), get_height_at(),
##   get_road_ghost_guides()
## Owns the shared preview mesh, blueprint spline, and node snapping MultiMesh.
## Subclasses override _handle_input() and _commit() for their specific editing behaviour.
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
var ghost_mesh: MeshInstance3D # Ghost guide lines (SimCity-style grid overlay, road tool only)

func _ready():
	_setup_visuals()

func _setup_visuals():
	# Final mesh container (ONLY RoadTool owns the final mesh)
	if name == "RoadTool":
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

	# Ghost guide lines (RoadTool only — other tools leave this null)
	if name == "RoadTool":
		ghost_mesh = MeshInstance3D.new()
		ghost_mesh.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		ghost_mesh.visible = false
		var gm := StandardMaterial3D.new()
		gm.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		gm.albedo_color = Color(1.0, 1.0, 1.0, 1.0)  # alpha driven per-vertex
		gm.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		gm.vertex_color_use_as_albedo = true
		gm.no_depth_test = true
		gm.render_priority = 1
		ghost_mesh.material_override = gm
		add_child(ghost_mesh)

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

static var _texture_cache = {}

# Helper to load textures even if they haven't been imported yet by the editor
func _load_texture(path: String) -> Texture2D:
	if _texture_cache.has(path):
		return _texture_cache[path]
		
	var tex: Texture2D = null
	if ResourceLoader.exists(path):
		tex = load(path)
	else:
		# Fallback for non-imported files
		var abs_path = ProjectSettings.globalize_path(path)
		var image = Image.load_from_file(abs_path)
		if image:
			image.generate_mipmaps()
			tex = ImageTexture.create_from_image(image)
	
	_texture_cache[path] = tex
	return tex

static var _road_mat: ShaderMaterial = null
static var _concrete_mat: ShaderMaterial = null
static var _marking_mat: StandardMaterial3D = null

func update_main_mesh():
	if name != "RoadTool":
		var road_tool = get_node_or_null("../RoadTool")
		if road_tool:
			road_tool.update_main_mesh()
		return

	var data = simulation_node.get_road_mesh_data()
	if not data: return
	
	if _road_mat == null:
		_road_mat = ShaderMaterial.new()
		_road_mat.shader = load("res://assets/materials/road.gdshader")
		_road_mat.set_shader_parameter("albedo_tex", _load_texture("res://assets/textures/road/clean_asphalt/clean_asphalt_diff_4k.jpg"))
		_road_mat.set_shader_parameter("normal_tex", _load_texture("res://assets/textures/road/clean_asphalt/clean_asphalt_nor_gl_4k.png"))
		_road_mat.set_shader_parameter("roughness_tex", _load_texture("res://assets/textures/road/clean_asphalt/clean_asphalt_rough_4k.png"))
		_road_mat.set_shader_parameter("displacement_tex", _load_texture("res://assets/textures/road/clean_asphalt/clean_asphalt_disp_4k.png"))
	
	if _marking_mat == null:
		_marking_mat = StandardMaterial3D.new()
		_marking_mat.vertex_color_use_as_albedo = true
		_marking_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		_marking_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		_marking_mat.albedo_color = Color(1.0, 1.0, 1.0, 0.35)
		_marking_mat.cull_mode = BaseMaterial3D.CULL_DISABLED

	if _concrete_mat == null:
		_concrete_mat = ShaderMaterial.new()
		_concrete_mat.shader = load("res://assets/materials/concrete.gdshader")
		_concrete_mat.set_shader_parameter("albedo_tex", _load_texture("res://assets/textures/general/concrete_layers/concrete_layers_02_diff_4k.jpg"))
		_concrete_mat.set_shader_parameter("normal_tex", _load_texture("res://assets/textures/general/concrete_layers/concrete_layers_02_nor_gl_4k.png"))
		_concrete_mat.set_shader_parameter("roughness_tex", _load_texture("res://assets/textures/general/concrete_layers/concrete_layers_02_rough_4k.png"))
		_concrete_mat.set_shader_parameter("displacement_tex", _load_texture("res://assets/textures/general/concrete_layers/concrete_layers_02_disp_4k.png"))

	var arr_mesh = ArrayMesh.new()
	var surface_map = [] # To keep track of which material goes to which surface
	
	# Surface 0: Sidewalk base
	if data.has("sidewalk_vertices") and data.sidewalk_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.sidewalk_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.sidewalk_normals
		arrays[Mesh.ARRAY_COLOR] = data.sidewalk_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.sidewalk_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(_road_mat)

	# Surface 1: Asphalt & Junctions
	if data.has("road_vertices") and data.road_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.road_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.road_normals
		arrays[Mesh.ARRAY_COLOR] = data.road_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.road_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(_road_mat)

	# Surface 2: Markings (lane lines + crosswalk stripes — unlit white, semi-transparent)
	if data.has("marking_vertices") and data.marking_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.marking_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.marking_normals
		arrays[Mesh.ARRAY_COLOR] = data.marking_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.marking_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(_marking_mat)

	# Surface 3: Concrete
	if data.has("concrete_vertices") and data.concrete_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.concrete_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.concrete_normals
		arrays[Mesh.ARRAY_COLOR] = data.concrete_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.concrete_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(_concrete_mat)
	
	mesh_instance.mesh = arr_mesh
	
	# Apply materials according to the mapped surface indices
	for i in range(surface_map.size()):
		mesh_instance.set_surface_override_material(i, surface_map[i])
