## Base class for all road-network editing tools (RoadTool, MoveTool, CulDeSacTool).
##
## Rust methods called: add_road(), get_closest_network_point(), get_closest_node(),
##   get_road_mesh_data(full_snapshot), get_network_nodes(), get_node_pos(), get_world_surface_height(),
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

var road_mesh_root: Node3D # Final road chunks owned only by RoadTool
var _road_chunk_instances: Dictionary = {}
var _road_mesh_generation: int = -1
var _road_chunk_span_m: float = 0.0
var _road_chunk_origin_x_m: float = 0.0
var _road_chunk_origin_z_m: float = 0.0
var _staged_road_mesh_update: Dictionary = {}
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
var _scripted_pointer_enabled: bool = false
var _scripted_pointer_position: Vector3 = Vector3.ZERO
var _node_visuals_dirty: bool = true
var _node_visuals_visible: bool = false

const SURFACE_DEBUG_REFRESH_SEC := 0.2
const SURFACE_PROBE_REFRESH_SEC := 0.25
const CHUNK_COORD_MIN := -2147483648
const CHUNK_COORD_MAX := 2147483647

func _ready():
	if name == "RoadTool":
		WorldMaterials.prewarm_road_materials()
	_surface_debug_enabled = _is_surface_debug_enabled()
	_surface_probe_enabled = _is_surface_probe_enabled()
	_setup_visuals()

func _setup_visuals():
	# Final mesh container (ONLY RoadTool owns the final mesh)
	if name == "RoadTool":
		road_mesh_root = Node3D.new()
		road_mesh_root.name = "RoadMeshChunks"
		add_child(road_mesh_root)
	
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

static var _marking_mat: StandardMaterial3D = null
static var _earthwork_mat: StandardMaterial3D = null

func update_main_mesh(expected_generation: int = -1, stage_only: bool = false) -> int:
	if name != "RoadTool":
		mark_network_nodes_dirty()
		var road_tool = get_node_or_null("../RoadTool")
		if road_tool:
			return road_tool.update_main_mesh(expected_generation, stage_only)
		return -1

	discard_staged_main_mesh_update()
	# Dirty revisions are coordinated by NetworkRenderer so terrain and roads swap together.
	if expected_generation < 0 and simulation_node.is_network_dirty():
		return -1
	var data = simulation_node.get_road_mesh_data(_road_mesh_generation < 0)
	if typeof(data) != TYPE_DICTIONARY:
		return -1
	if (
		not data.has("surface_generation")
		or typeof(data["surface_generation"]) != TYPE_INT
		or not data.has("full_replace")
		or typeof(data["full_replace"]) != TYPE_BOOL
		or not data.has("chunk_span_m")
		or (
			typeof(data["chunk_span_m"]) != TYPE_FLOAT
			and typeof(data["chunk_span_m"]) != TYPE_INT
		)
		or not data.has("chunk_origin_x_m")
		or (
			typeof(data["chunk_origin_x_m"]) != TYPE_FLOAT
			and typeof(data["chunk_origin_x_m"]) != TYPE_INT
		)
		or not data.has("chunk_origin_z_m")
		or (
			typeof(data["chunk_origin_z_m"]) != TYPE_FLOAT
			and typeof(data["chunk_origin_z_m"]) != TYPE_INT
		)
		or not data.has("chunks")
		or typeof(data["chunks"]) != TYPE_ARRAY
	):
		return -1
	var surface_generation: int = data["surface_generation"]
	if surface_generation < 0:
		return -1
	if expected_generation >= 0 and surface_generation != expected_generation:
		return -1
	if surface_generation < _road_mesh_generation:
		return -1
	var full_replace: bool = data["full_replace"]
	if _road_mesh_generation < 0 and not full_replace:
		return -1
	var chunk_span_m := float(data["chunk_span_m"])
	if not is_finite(chunk_span_m) or chunk_span_m <= 0.0:
		return -1
	var chunk_origin_x_m := float(data["chunk_origin_x_m"])
	var chunk_origin_z_m := float(data["chunk_origin_z_m"])
	if not is_finite(chunk_origin_x_m) or not is_finite(chunk_origin_z_m):
		return -1
	if not full_replace and chunk_span_m != _road_chunk_span_m:
		return -1
	if (
		not full_replace
		and (
			chunk_origin_x_m != _road_chunk_origin_x_m
			or chunk_origin_z_m != _road_chunk_origin_z_m
		)
	):
		return -1
	var chunk_updates: Array = data["chunks"]
	var validated_updates: Array[Dictionary] = []
	var seen_keys: Dictionary = {}
	for chunk_variant in chunk_updates:
		if typeof(chunk_variant) != TYPE_DICTIONARY:
			return -1
		var chunk_data: Dictionary = chunk_variant
		if (
			not chunk_data.has("chunk_x")
			or typeof(chunk_data["chunk_x"]) != TYPE_INT
			or not chunk_data.has("chunk_z")
			or typeof(chunk_data["chunk_z"]) != TYPE_INT
			or not chunk_data.has("removed")
			or typeof(chunk_data["removed"]) != TYPE_BOOL
		):
			return -1
		if full_replace and chunk_data["removed"]:
			return -1
		var chunk_x: int = chunk_data["chunk_x"]
		var chunk_z: int = chunk_data["chunk_z"]
		if (
			chunk_x < CHUNK_COORD_MIN
			or chunk_x > CHUNK_COORD_MAX
			or chunk_z < CHUNK_COORD_MIN
			or chunk_z > CHUNK_COORD_MAX
		):
			return -1
		var key := Vector2i(chunk_x, chunk_z)
		if seen_keys.has(key):
			return -1
		seen_keys[key] = true
		validated_updates.append(chunk_data)

	var staged_instances: Dictionary = {}
	for chunk_data in validated_updates:
		if chunk_data["removed"]:
			continue
		var key := Vector2i(chunk_data["chunk_x"], chunk_data["chunk_z"])
		var instance := _build_road_chunk_instance(
			chunk_data,
			key,
			chunk_span_m,
			chunk_origin_x_m,
			chunk_origin_z_m
		)
		if instance == null:
			_discard_staged_road_chunks(staged_instances)
			return -1
		staged_instances[key] = instance

	# Building ArrayMeshes can span frames under a debugger. Never commit a stale batch.
	if int(simulation_node.get_network_render_generation()) != surface_generation:
		_discard_staged_road_chunks(staged_instances)
		return -1

	if stage_only:
		_staged_road_mesh_update = {
			"surface_generation": surface_generation,
			"full_replace": full_replace,
			"chunk_span_m": chunk_span_m,
			"chunk_origin_x_m": chunk_origin_x_m,
			"chunk_origin_z_m": chunk_origin_z_m,
			"validated_updates": validated_updates,
			"staged_instances": staged_instances,
		}
		return surface_generation
	return _commit_road_mesh_update(
		surface_generation,
		full_replace,
		chunk_span_m,
		chunk_origin_x_m,
		chunk_origin_z_m,
		validated_updates,
		staged_instances
	)

func commit_staged_main_mesh_update(expected_generation: int) -> int:
	if _staged_road_mesh_update.is_empty():
		return -1
	var staged := _staged_road_mesh_update
	_staged_road_mesh_update = {}
	var surface_generation := int(staged.get("surface_generation", -1))
	if surface_generation != expected_generation or surface_generation < _road_mesh_generation:
		_discard_staged_road_chunks(staged.get("staged_instances", {}) as Dictionary)
		return -1
	# A newer sim revision may arrive after staging. Committing this complete older road/terrain
	# pair is safe; its exact acknowledgement will fail and the newer pair remains dirty.
	return _commit_road_mesh_update(
		surface_generation,
		bool(staged["full_replace"]),
		float(staged["chunk_span_m"]),
		float(staged["chunk_origin_x_m"]),
		float(staged["chunk_origin_z_m"]),
		staged["validated_updates"] as Array,
		staged["staged_instances"] as Dictionary,
		false
	)

func discard_staged_main_mesh_update() -> void:
	if _staged_road_mesh_update.is_empty():
		return
	_discard_staged_road_chunks(
		_staged_road_mesh_update.get("staged_instances", {}) as Dictionary
	)
	_staged_road_mesh_update = {}

func _commit_road_mesh_update(
	surface_generation: int,
	full_replace: bool,
	chunk_span_m: float,
	chunk_origin_x_m: float,
	chunk_origin_z_m: float,
	validated_updates: Array,
	staged_instances: Dictionary,
	invalidate_nodes: bool = true
) -> int:
	if invalidate_nodes:
		mark_network_nodes_dirty()
	if full_replace:
		for key in _road_chunk_instances.keys():
			_remove_road_chunk(key)
	for chunk_data in validated_updates:
		var key := Vector2i(chunk_data["chunk_x"], chunk_data["chunk_z"])
		if chunk_data["removed"]:
			_remove_road_chunk(key)
	for key in staged_instances:
		_remove_road_chunk(key)
		var instance: MeshInstance3D = staged_instances[key]
		road_mesh_root.add_child(instance)
		_road_chunk_instances[key] = instance
	_road_mesh_generation = surface_generation
	_road_chunk_span_m = chunk_span_m
	_road_chunk_origin_x_m = chunk_origin_x_m
	_road_chunk_origin_z_m = chunk_origin_z_m
	return surface_generation

func reset_main_mesh_chunks() -> void:
	discard_staged_main_mesh_update()
	for key in _road_chunk_instances.keys():
		_remove_road_chunk(key)
	_road_mesh_generation = -1
	_road_chunk_span_m = 0.0
	_road_chunk_origin_x_m = 0.0
	_road_chunk_origin_z_m = 0.0

func needs_main_mesh_hydration() -> bool:
	return name == "RoadTool" and _road_mesh_generation < 0

func _discard_staged_road_chunks(staged_instances: Dictionary) -> void:
	for instance in staged_instances.values():
		instance.queue_free()

func _remove_road_chunk(key: Vector2i) -> void:
	if not _road_chunk_instances.has(key):
		return
	var instance: MeshInstance3D = _road_chunk_instances[key]
	_road_chunk_instances.erase(key)
	if is_instance_valid(instance):
		if instance.get_parent() != null:
			instance.get_parent().remove_child(instance)
		instance.queue_free()

func _build_road_chunk_instance(
	chunk_data: Dictionary,
	key: Vector2i,
	chunk_span_m: float,
	chunk_origin_x_m: float,
	chunk_origin_z_m: float
) -> MeshInstance3D:
	_ensure_road_mesh_materials()
	var arr_mesh := ArrayMesh.new()
	if not _append_road_surface(arr_mesh, chunk_data, "earthwork", _earthwork_mat):
		return null
	if not _append_road_surface(
		arr_mesh,
		chunk_data,
		"curb",
		WorldMaterials.road_sidewalk_face_material()
	):
		return null
	if not _append_road_surface(
		arr_mesh,
		chunk_data,
		"raised_step",
		WorldMaterials.road_sidewalk_face_material()
	):
		return null
	if not _append_road_surface(
		arr_mesh,
		chunk_data,
		"sidewalk",
		WorldMaterials.road_sidewalk_material()
	):
		return null
	if not _append_road_surface(
		arr_mesh,
		chunk_data,
		"road",
		WorldMaterials.road_asphalt_material()
	):
		return null
	if not _append_road_surface(arr_mesh, chunk_data, "marking", _marking_mat):
		return null
	if not _append_road_surface(
		arr_mesh,
		chunk_data,
		"concrete",
		WorldMaterials.road_concrete_material()
	):
		return null
	if arr_mesh.get_surface_count() == 0:
		return null

	var instance := MeshInstance3D.new()
	instance.name = "RoadChunk_%d_%d" % [key.x, key.y]
	instance.position = Vector3(
		chunk_origin_x_m + float(key.x) * chunk_span_m,
		0.0,
		chunk_origin_z_m + float(key.y) * chunk_span_m
	)
	instance.mesh = arr_mesh
	SceneLightingConfig.apply_shadow_policy(
		instance,
		SceneLightingConfig.SHADOW_RECEIVER_ONLY,
		"roads"
	)
	return instance

func _append_road_surface(
	arr_mesh: ArrayMesh,
	chunk_data: Dictionary,
	layer: String,
	material: Material
) -> bool:
	var vertices_key := layer + "_vertices"
	var normals_key := layer + "_normals"
	var colors_key := layer + "_colors"
	var uvs_key := layer + "_uvs"
	if (
		not chunk_data.has(vertices_key)
		or typeof(chunk_data[vertices_key]) != TYPE_PACKED_VECTOR3_ARRAY
		or not chunk_data.has(normals_key)
		or typeof(chunk_data[normals_key]) != TYPE_PACKED_VECTOR3_ARRAY
		or not chunk_data.has(colors_key)
		or typeof(chunk_data[colors_key]) != TYPE_PACKED_COLOR_ARRAY
		or not chunk_data.has(uvs_key)
		or typeof(chunk_data[uvs_key]) != TYPE_PACKED_VECTOR2_ARRAY
	):
		return false
	var vertices: PackedVector3Array = chunk_data[vertices_key]
	var normals: PackedVector3Array = chunk_data[normals_key]
	var colors: PackedColorArray = chunk_data[colors_key]
	var uvs: PackedVector2Array = chunk_data[uvs_key]
	if (
		vertices.size() % 3 != 0
		or normals.size() != vertices.size()
		or colors.size() != vertices.size()
		or uvs.size() != vertices.size()
	):
		return false
	if vertices.is_empty():
		return true
	if material == null:
		return false
	for index in vertices.size():
		var vertex := vertices[index]
		var normal := normals[index]
		var uv := uvs[index]
		var color := colors[index]
		if (
			not is_finite(vertex.x)
			or not is_finite(vertex.y)
			or not is_finite(vertex.z)
			or not is_finite(normal.x)
			or not is_finite(normal.y)
			or not is_finite(normal.z)
			or not is_finite(uv.x)
			or not is_finite(uv.y)
			or not is_finite(color.r)
			or not is_finite(color.g)
			or not is_finite(color.b)
			or not is_finite(color.a)
		):
			return false
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_COLOR] = colors
	arrays[Mesh.ARRAY_TEX_UV] = uvs
	var previous_surface_count := arr_mesh.get_surface_count()
	arr_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	if arr_mesh.get_surface_count() != previous_surface_count + 1:
		return false
	arr_mesh.surface_set_material(previous_surface_count, material)
	return true

func _ensure_road_mesh_materials() -> void:
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
