# SPDX-License-Identifier: GPL-2.0-only

## Base class for the road-network editing tools (RoadTool, RoundaboutTool).
##
## Rust methods called: add_road(), get_closest_network_point(), get_closest_node(),
##   get_network_nodes(), get_node_pos(), get_world_surface_height(),
##   intersect_world_surface(), hover_world_surface(),
##   get_road_surface_debug_data()
## Owns the shared blueprint mesh (the roundabout tool's ring; the road tool draws its preview as
## road chunks and its gizmos itself) and the node snapping MultiMesh. The committed
## network's chunks are drawn by `road_chunk_renderer.gd` (`RENDER-52`), the road tool's `chunks`.
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

var blueprint_mesh: MeshInstance3D # The preview line/spline
var blueprint_mat: StandardMaterial3D
var node_multimesh: MultiMeshInstance3D # Holographic snapping points
var cursor_mesh: MeshInstance3D # Active hovered snap cursor
var surface_debug_mesh: MeshInstance3D # Compiled roadbed debug overlay
var _surface_debug_enabled: bool = false
var _surface_probe_enabled: bool = false
var _surface_debug_refresh_elapsed: float = 0.0
var _surface_probe_refresh_elapsed: float = 0.0
var _last_world_mouse_pos: Vector3 = Vector3.ZERO
var _has_last_world_mouse_pos: bool = false
var _scripted_pointer_enabled: bool = false
var _scripted_pointer_position: Vector3 = Vector3.ZERO
var _node_visuals_dirty: bool = true
var _node_visuals_visible: bool = false

const SURFACE_DEBUG_REFRESH_SEC := 0.2
const SURFACE_PROBE_REFRESH_SEC := 0.25

func _ready():
	_surface_debug_enabled = MetrumDebug.startup_mode("road_surface")
	_surface_probe_enabled = MetrumDebug.startup_mode("road_probe")
	_setup_visuals()

func _setup_visuals():
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
		var pos = _scripted_pointer_position if _scripted_pointer_enabled else get_world_mouse_pos()
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

func set_scripted_pointer(enabled: bool, world_position: Vector3 = Vector3.ZERO) -> void:
	_scripted_pointer_enabled = enabled
	_scripted_pointer_position = world_position
	if enabled:
		_last_world_mouse_pos = world_position
		_has_last_world_mouse_pos = true

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


# The world point under the pointer, null for none. A click passes `click` and waits for the
# core (`CODE-25`); a `hover` query answers false while the simulation thread holds it.
func get_terrain_interaction(click := false, hover := false):
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return null
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	
	# Combined world-surface raycast: compiled roadbed where owned, otherwise visual terrain.
	if hover:
		return simulation_node.hover_world_surface(ray_origin, ray_dir)
	return simulation_node.intersect_world_surface(ray_origin, ray_dir, click)

func get_world_mouse_pos() -> Vector3:
	var pos_variant = get_terrain_interaction(false, true)
	# The simulation thread holds the core: the cursor keeps its last position.
	if pos_variant is bool and _has_last_world_mouse_pos:
		return _last_world_mouse_pos
	if pos_variant == null or pos_variant is bool:
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
		MetrumDebug.log("road", dump)
	_surface_probe_refresh_elapsed = SURFACE_PROBE_REFRESH_SEC

func _append_debug_lines(immediate: ImmediateMesh, points: PackedVector3Array, color: Color) -> void:
	if points.size() < 2:
		return
	immediate.surface_begin(Mesh.PRIMITIVE_LINES)
	for point in points:
		immediate.surface_set_color(color)
		immediate.surface_add_vertex(point)
	immediate.surface_end()
