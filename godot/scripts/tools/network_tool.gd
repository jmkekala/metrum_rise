## Base class for all road-network editing tools (RoadTool, MoveTool, CulDeSacTool).
##
## Rust methods called: add_road(), get_closest_network_point(), get_closest_node(),
##   get_road_mesh_data(), get_network_nodes(), get_node_pos(), get_world_surface_height(),
##   get_road_ghost_line_data(), get_road_ghost_snap(), intersect_world_surface(),
##   get_road_tool_cursor_pos(), get_road_surface_debug_data()
## Owns the shared preview mesh, blueprint spline, and node snapping MultiMesh.
## Subclasses override _handle_input() and _commit() for their specific editing behaviour.
extends Node3D
class_name NetworkTool

const WorldMaterials = preload("res://scripts/renderers/world_materials.gd")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")

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
var surface_debug_mesh: MeshInstance3D # Compiled roadbed debug overlay
var _surface_debug_enabled: bool = false
var _surface_probe_enabled: bool = false
var _surface_debug_refresh_elapsed: float = 0.0
var _surface_probe_refresh_elapsed: float = 0.0
var _last_world_mouse_pos: Vector3 = Vector3.ZERO
var _has_last_world_mouse_pos: bool = false
var _node_visuals_dirty: bool = true
var _node_visuals_visible: bool = false

const SURFACE_DEBUG_REFRESH_SEC := 0.2
const SURFACE_PROBE_REFRESH_SEC := 0.25

func _ready():
	_surface_debug_enabled = _is_surface_debug_enabled()
	_surface_probe_enabled = _is_surface_probe_enabled()
	_setup_visuals()

func _setup_visuals():
	# Final mesh container (ONLY RoadTool owns the final mesh)
	if name == "RoadTool":
		mesh_instance = MeshInstance3D.new()
		SceneLightingConfig.apply_shadow_policy(
			mesh_instance,
			SceneLightingConfig.SHADOW_RECEIVER_ONLY,
			"roads"
		)
		add_child(mesh_instance)
	
	# Blueprint mesh container
	blueprint_mesh = MeshInstance3D.new()
	SceneLightingConfig.apply_shadow_policy(
		blueprint_mesh,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"roads"
	)
	add_child(blueprint_mesh)
	
	blueprint_mat = StandardMaterial3D.new()
	blueprint_mat.albedo_color = blue_color
	blueprint_mat.emission_enabled = true
	blueprint_mat.emission = Color(0.0, 0.5, 0.5)
	blueprint_mat.transparency = StandardMaterial3D.TRANSPARENCY_ALPHA
	blueprint_mat.cull_mode = StandardMaterial3D.CULL_DISABLED
	blueprint_mat.shading_mode = StandardMaterial3D.SHADING_MODE_UNSHADED
	blueprint_mat.no_depth_test = false
	blueprint_mat.render_priority = 5
	blueprint_mesh.material_override = blueprint_mat
	
	# Node Snapping Highlights
	node_multimesh = MultiMeshInstance3D.new()
	SceneLightingConfig.apply_shadow_policy(
		node_multimesh,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"roads"
	)
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
	SceneLightingConfig.apply_shadow_policy(
		cursor_mesh,
		SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
		"roads"
	)
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
		SceneLightingConfig.apply_shadow_policy(
			ghost_mesh,
			SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
			"roads"
		)
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

	if _surface_debug_enabled:
		surface_debug_mesh = MeshInstance3D.new()
		SceneLightingConfig.apply_shadow_policy(
			surface_debug_mesh,
			SceneLightingConfig.SHADOW_DEBUG_OVERLAY,
			"roads"
		)
		var sm := StandardMaterial3D.new()
		sm.vertex_color_use_as_albedo = true
		sm.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		sm.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		sm.no_depth_test = true
		sm.cull_mode = BaseMaterial3D.CULL_DISABLED
		sm.render_priority = 10
		surface_debug_mesh.material_override = sm
		add_child(surface_debug_mesh)

func _process(delta):
	if active:
		var pos = get_world_mouse_pos()
		_last_world_mouse_pos = pos
		_has_last_world_mouse_pos = true
		if cursor_mesh:
			cursor_mesh.global_position = pos
			cursor_mesh.global_position.y += 0.35 # Float above road geometries
			cursor_mesh.visible = is_valid

		_update_blueprint_visuals()
		_update_node_visuals()
		_update_surface_debug_overlay(delta)
		_update_surface_probe_debug(delta)
	else:
		_has_last_world_mouse_pos = false
		if cursor_mesh:
			cursor_mesh.visible = false
		if _node_visuals_visible and node_multimesh and node_multimesh.multimesh:
			node_multimesh.multimesh.instance_count = 0
			_node_visuals_visible = false
		if surface_debug_mesh:
			surface_debug_mesh.visible = false

func _update_node_visuals():
	if not simulation_node: return
	if not _node_visuals_dirty and _node_visuals_visible:
		return
	var nodes = simulation_node.get_network_nodes()
	if nodes == null: return
	var mm = node_multimesh.multimesh
	mm.instance_count = nodes.size()
	for i in range(nodes.size()):
		var t = Transform3D()
		t.origin = nodes[i]
		t.origin.y += 0.3 # Elevate above the 0.15m asphalt and 0.05m kerb
		mm.set_instance_transform(i, t)
	_node_visuals_dirty = false
	_node_visuals_visible = true

func mark_network_nodes_dirty() -> void:
	_node_visuals_dirty = true

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
	
	# Combined world-surface raycast: compiled roadbed where owned, otherwise visual terrain.
	return simulation_node.intersect_world_surface(ray_origin, ray_dir)

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

func _is_surface_debug_enabled() -> bool:
	var explicit_value := OS.get_environment("METRUM_DEBUG_SURFACE").strip_edges()
	if not explicit_value.is_empty():
		return explicit_value != "0"
	var filter_value := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	return filter_value.contains("road-surface") or filter_value.contains("surface-debug")

func _is_surface_probe_enabled() -> bool:
	var explicit_value := OS.get_environment("METRUM_DEBUG_ROAD_PROBE").strip_edges()
	if not explicit_value.is_empty():
		return explicit_value != "0"
	var filter_value := OS.get_environment("METRUM_DEBUG_FILTER").strip_edges().to_lower()
	return filter_value.contains("road-probe")

func _update_surface_debug_overlay(delta: float) -> void:
	if surface_debug_mesh == null:
		return
	_surface_debug_refresh_elapsed = max(_surface_debug_refresh_elapsed - delta, 0.0)
	if _surface_debug_refresh_elapsed > 0.0 and surface_debug_mesh.mesh != null:
		surface_debug_mesh.visible = true
		return

	var debug_data = simulation_node.get_road_surface_debug_data()
	if debug_data == null:
		surface_debug_mesh.visible = false
		return

	var immediate := ImmediateMesh.new()
	_append_debug_lines(immediate, debug_data.get("section_lines", PackedVector3Array()), Color(0.97, 0.84, 0.28, 0.90))
	_append_debug_lines(immediate, debug_data.get("band_lines", PackedVector3Array()), Color(0.26, 0.92, 0.95, 0.78))
	_append_debug_lines(immediate, debug_data.get("piece_boundary_lines", PackedVector3Array()), Color(1.0, 0.42, 0.38, 0.92))
	_append_debug_lines(immediate, debug_data.get("earthwork_chunk_lines", PackedVector3Array()), Color(0.56, 1.0, 0.46, 0.70))
	surface_debug_mesh.mesh = immediate
	surface_debug_mesh.visible = true
	_surface_debug_refresh_elapsed = SURFACE_DEBUG_REFRESH_SEC

func _update_surface_probe_debug(delta: float) -> void:
	if not _surface_probe_enabled or not _has_last_world_mouse_pos:
		return
	_surface_probe_refresh_elapsed = max(_surface_probe_refresh_elapsed - delta, 0.0)
	if _surface_probe_refresh_elapsed > 0.0:
		return
	var dump: String = simulation_node.get_road_surface_probe_debug(_last_world_mouse_pos)
	if not dump.is_empty():
		print("[DEBUG:road] " + dump)
	_surface_probe_refresh_elapsed = SURFACE_PROBE_REFRESH_SEC

func _append_debug_lines(immediate: ImmediateMesh, points: PackedVector3Array, color: Color) -> void:
	if points.size() < 2:
		return
	immediate.surface_begin(Mesh.PRIMITIVE_LINES)
	for point in points:
		immediate.surface_set_color(color)
		immediate.surface_add_vertex(point)
	immediate.surface_end()

static var _curb_mat: StandardMaterial3D = null
static var _raised_step_mat: StandardMaterial3D = null
static var _marking_mat: StandardMaterial3D = null
static var _earthwork_mat: StandardMaterial3D = null

func update_main_mesh():
	if name != "RoadTool":
		mark_network_nodes_dirty()
		var road_tool = get_node_or_null("../RoadTool")
		if road_tool:
			road_tool.update_main_mesh()
		return

	mark_network_nodes_dirty()
	var data = simulation_node.get_road_mesh_data()
	if not data: return
	var road_mat := WorldMaterials.road_asphalt_material()
	var concrete_mat := WorldMaterials.road_concrete_material()

	if _curb_mat == null:
		_curb_mat = StandardMaterial3D.new()
		_curb_mat.vertex_color_use_as_albedo = true
		_curb_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		_curb_mat.roughness = 1.0
		_curb_mat.metallic = 0.0
		_curb_mat.cull_mode = BaseMaterial3D.CULL_BACK

	if _raised_step_mat == null:
		_raised_step_mat = StandardMaterial3D.new()
		_raised_step_mat.vertex_color_use_as_albedo = true
		_raised_step_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		_raised_step_mat.roughness = 1.0
		_raised_step_mat.metallic = 0.0
		_raised_step_mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	
	if _marking_mat == null:
		_marking_mat = StandardMaterial3D.new()
		_marking_mat.vertex_color_use_as_albedo = true
		_marking_mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
		_marking_mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
		_marking_mat.albedo_color = Color(1.0, 1.0, 1.0, 0.35)
		_marking_mat.cull_mode = BaseMaterial3D.CULL_DISABLED

	if _earthwork_mat == null:
		_earthwork_mat = StandardMaterial3D.new()
		_earthwork_mat.vertex_color_use_as_albedo = true
		_earthwork_mat.roughness = 1.0
		_earthwork_mat.metallic = 0.0
		_earthwork_mat.cull_mode = BaseMaterial3D.CULL_BACK

	var arr_mesh = ArrayMesh.new()
	var surface_map = [] # To keep track of which material goes to which surface
	
	# Surface 0: Earthwork tie-in faces
	if data.has("earthwork_vertices") and data.earthwork_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.earthwork_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.earthwork_normals
		arrays[Mesh.ARRAY_COLOR] = data.earthwork_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.earthwork_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(_earthwork_mat)

	# Curb / shoulder transition faces
	if data.has("curb_vertices") and data.curb_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.curb_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.curb_normals
		arrays[Mesh.ARRAY_COLOR] = data.curb_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.curb_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(_curb_mat)

	# Explicit raised-step faces between solved owner-pair top surfaces
	if data.has("raised_step_vertices") and data.raised_step_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.raised_step_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.raised_step_normals
		arrays[Mesh.ARRAY_COLOR] = data.raised_step_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.raised_step_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(_raised_step_mat)

	# Sidewalk base
	if data.has("sidewalk_vertices") and data.sidewalk_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.sidewalk_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.sidewalk_normals
		arrays[Mesh.ARRAY_COLOR] = data.sidewalk_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.sidewalk_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(road_mat)

	# Asphalt & Junctions
	if data.has("road_vertices") and data.road_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.road_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.road_normals
		arrays[Mesh.ARRAY_COLOR] = data.road_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.road_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(road_mat)

	# Markings (lane lines + crosswalk stripes — unlit white, semi-transparent)
	if data.has("marking_vertices") and data.marking_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.marking_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.marking_normals
		arrays[Mesh.ARRAY_COLOR] = data.marking_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.marking_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(_marking_mat)

	# Concrete
	if data.has("concrete_vertices") and data.concrete_vertices.size() > 0:
		var arrays = []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = data.concrete_vertices
		arrays[Mesh.ARRAY_NORMAL] = data.concrete_normals
		arrays[Mesh.ARRAY_COLOR] = data.concrete_colors
		arrays[Mesh.ARRAY_TEX_UV] = data.concrete_uvs
		arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		surface_map.push_back(concrete_mat)
	
	mesh_instance.mesh = arr_mesh
	
	# Apply materials according to the mapped surface indices
	for i in range(surface_map.size()):
		mesh_instance.set_surface_override_material(i, surface_map[i])
