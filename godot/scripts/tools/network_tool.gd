# SPDX-License-Identifier: GPL-3.0-or-later

## Base class for the road-network editing tools (RoadTool, RoundaboutTool).
##
## Rust methods called: add_road(), get_closest_node(),
##   get_node_pos(), get_world_surface_height(),
##   intersect_world_surface(),
##   get_road_surface_debug_data()
## Owns the shared blueprint mesh (the roundabout tool's ring; the road tool draws its preview as
## road chunks and its gizmos itself) and the hover cursor. The committed
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
		_update_pointer()
		_update_blueprint_visuals()
		_update_surface_debug_overlay(delta)
		_update_surface_probe_debug(delta)
	else:
		_has_last_world_mouse_pos = false
		if cursor_mesh:
			cursor_mesh.visible = false
		if surface_debug_mesh:
			surface_debug_mesh.visible = false

# Resolves this frame's pointer and places the hover cursor on it (the roundabout tool's; the road
# tool resolves its pointer in its own tick, `ROAD-112` round R2-2).
func _update_pointer() -> void:
	var pos = _scripted_pointer_position if _scripted_pointer_enabled else get_world_mouse_pos()
	_last_world_mouse_pos = pos
	_has_last_world_mouse_pos = true
	if cursor_mesh:
		cursor_mesh.global_position = pos
		cursor_mesh.global_position.y += 0.35 # Float above road geometries
		cursor_mesh.visible = is_valid

func set_scripted_pointer(enabled: bool, world_position: Vector3 = Vector3.ZERO) -> void:
	_scripted_pointer_enabled = enabled
	_scripted_pointer_position = world_position
	if enabled:
		_last_world_mouse_pos = world_position
		_has_last_world_mouse_pos = true

func _update_blueprint_visuals():
	if is_valid:
		blueprint_mat.albedo_color = blue_color
		blueprint_mat.emission = Color(0.0, 0.5, 0.5)
	else:
		blueprint_mat.albedo_color = red_color
		blueprint_mat.emission = Color(0.5, 0.0, 0.0)


# The world point under the pointer, null for none. A click passes `click` and waits for the
# core (`CODE-25`).
func get_terrain_interaction(click := false):
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return null
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	
	# Combined world-surface raycast: compiled roadbed where owned, otherwise visual terrain.
	return simulation_node.intersect_world_surface(ray_origin, ray_dir, click)

# The pointer's world position, resolved once per frame by `_update_pointer` (the roundabout tool
# resolves it on the ground; the road tool's tick resolves its own); a bare base keeps the last
# one.
func get_world_mouse_pos() -> Vector3:
	return _last_world_mouse_pos

# The pointer's last world point, null before one (the surface probe's). The road tool reads its
# cursor from Rust (`ROAD-112` round R2-10).
func _pointer_point() -> Variant:
	return _last_world_mouse_pos if _has_last_world_mouse_pos else null

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
	if not _surface_probe_enabled:
		return
	var point: Variant = _pointer_point()
	if point == null:
		return
	_surface_probe_refresh_elapsed = max(_surface_probe_refresh_elapsed - delta, 0.0)
	if _surface_probe_refresh_elapsed > 0.0:
		return
	var dump: String = simulation_node.get_road_surface_probe_debug(point)
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
