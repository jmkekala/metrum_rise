# SPDX-License-Identifier: GPL-2.0-only

## Visible cell-grid chunks from Rust; retained meshes upload only after a chunk changes.
## Generation, reservations, cell ownership and paint remain in the simulation.
extends Node3D

var simulation_node: Node
var _chunks: Dictionary = {}
var _versions: Dictionary = {}
var _material: StandardMaterial3D
var _span: float
const VIEW_DISTANCE_M := 2000.0
const UPLOADS_PER_FRAME := 2

func full_refresh() -> void:
	_versions.clear()

func _ready() -> void:
	_span = simulation_node.get_zoning_cell_chunk_span()
	_material = StandardMaterial3D.new()
	_material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	_material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	_material.cull_mode = BaseMaterial3D.CULL_DISABLED
	_material.vertex_color_use_as_albedo = true
	_material.no_depth_test = true
	top_level = true

func _process(_delta: float) -> void:
	visible = get_parent().is_overlay_requested()
	if not visible:
		return
	var keys := _visible_chunks()
	var requested: Dictionary = {}
	var packed := PackedInt32Array()
	for key in keys:
		requested[key] = true
		packed.append(key.x)
		packed.append(key.y)
	for key in _chunks.keys():
		if not requested.has(key):
			_chunks[key].queue_free()
			_chunks.erase(key)
			_versions.erase(key)
	var metadata: Dictionary = simulation_node.try_get_zoning_cell_chunk_states(packed)
	if bool(metadata.get("busy", true)):
		return
	var states: PackedInt64Array = metadata.get("states", PackedInt64Array())
	if states.size() != keys.size() * 5:
		return
	var uploads := 0
	for i in range(keys.size()):
		var key: Vector2i = keys[i]
		var version := PackedInt64Array([states[i * 5], states[i * 5 + 2], states[i * 5 + 3], states[i * 5 + 4]])
		var known: PackedInt64Array = _versions.get(key, PackedInt64Array())
		if states[i * 5 + 1] != 0 and known == version:
			continue
		var payload: Dictionary = simulation_node.try_get_zoning_cell_chunk_packed(key.x, key.y, known)
		if bool(payload.get("busy", true)):
			break
		if bool(payload.get("outside", false)):
			continue
		_versions[key] = payload.get("version", PackedInt64Array())
		if not bool(payload.get("unchanged", false)):
			_replace_chunk(key, payload)
		uploads += 1
		if uploads >= UPLOADS_PER_FRAME:
			break

func _replace_chunk(key: Vector2i, payload: Dictionary) -> void:
	var instance: MeshInstance3D = _chunks.get(key)
	if instance == null:
		instance = MeshInstance3D.new()
		instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		instance.material_override = _material
		add_child(instance)
		_chunks[key] = instance
	var vertices: PackedVector3Array = payload.get("vertices", PackedVector3Array())
	var colors: PackedColorArray = payload.get("colors", PackedColorArray())
	if vertices.is_empty():
		instance.mesh = null
		return
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_COLOR] = colors
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	var lines: PackedVector3Array = payload.get("lines", PackedVector3Array())
	if not lines.is_empty():
		arrays[Mesh.ARRAY_VERTEX] = lines
		arrays[Mesh.ARRAY_COLOR] = payload.get("line_colors", PackedColorArray())
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_LINES, arrays)
	instance.mesh = mesh

func _visible_chunks() -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	var camera := get_viewport().get_camera_3d()
	if camera == null:
		return keys
	var size := get_viewport().get_visible_rect().size
	var lower := Vector2(INF, INF)
	var upper := Vector2(-INF, -INF)
	for screen in [Vector2.ZERO, Vector2(size.x, 0), size, Vector2(0, size.y)]:
		var origin := camera.project_ray_origin(screen)
		var direction := camera.project_ray_normal(screen)
		var distance := VIEW_DISTANCE_M
		if direction.y < -0.001:
			distance = clampf(-origin.y / direction.y, 0.0, VIEW_DISTANCE_M)
		var point := origin + direction * distance
		lower = lower.min(Vector2(point.x, point.z))
		upper = upper.max(Vector2(point.x, point.z))
	# One extra chunk covers squares crossing ownership boundaries and terrain-height variation.
	var half_world: Vector2 = simulation_node.get_terrain_world_size() * 0.5
	lower = (lower - Vector2.ONE * _span).max(-half_world)
	upper = (upper + Vector2.ONE * _span).min(half_world)
	for x in range(floori(lower.x / _span), floori(upper.x / _span) + 1):
		for z in range(floori(lower.y / _span), floori(upper.y / _span) + 1):
			keys.append(Vector2i(x, z))
	var center := (lower + upper) * 0.5 / _span
	keys.sort_custom(func(a: Vector2i, b: Vector2i):
		return Vector2(a).distance_squared_to(center) < Vector2(b).distance_squared_to(center))
	return keys
