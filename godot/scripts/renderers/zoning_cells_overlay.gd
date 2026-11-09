# SPDX-License-Identifier: GPL-2.0-only

## Visible ground-mask chunks from Rust; retained textures upload only after local changes.
## Generation, reservations, cell ownership and paint remain in the simulation.
extends Node3D

var simulation_node: Node
var _chunks: Dictionary = {}
var _versions: Dictionary = {}
var _masks := preload("res://scripts/renderers/zoning_ground_masks.gd").new()
var _preview_keys: Dictionary = {}
var _preview_payload: Dictionary = {}
var _preview_view: Array[Vector2i] = []
var _span: float
var _reported_visible := false
const VIEW_DISTANCE_M := 2000.0
const UPLOADS_PER_FRAME := 2

func full_refresh() -> void:
	_versions.clear()
	_chunks.clear()
	_preview_keys.clear()
	_preview_payload.clear()
	_preview_view.clear()
	_masks.clear()
	_masks.setup(simulation_node.get_terrain_world_size())

func _ready() -> void:
	_span = simulation_node.get_zoning_cell_chunk_span()
	_masks.setup(simulation_node.get_terrain_world_size())
	top_level = true

func _process(_delta: float) -> void:
	visible = get_parent().is_overlay_requested()
	# While shown, Rust regenerates edited chunks before publishing the edit's snapshot.
	if visible != _reported_visible:
		_reported_visible = visible
		simulation_node.set_zoning_cell_overlay_visible(visible)
	# Until a layer is resident the sampler globals are unset, and Godot binds its 4 x 4 white
	# array there: the lookup would name layer 1 and the shader fetch past the texture.
	RenderingServer.global_shader_parameter_set("zoning_mask_enabled", visible and _masks.has_layers())
	if not visible:
		return
	var keys := _visible_chunks()
	if not _preview_payload.is_empty() and keys != _preview_view:
		_upload_preview(keys)
	var requested: Dictionary = {}
	var packed := PackedInt32Array()
	for key in keys:
		requested[key] = true
		packed.append(key.x)
		packed.append(key.y)
	for key in _chunks.keys():
		if not requested.has(key):
			_masks.remove(Vector3i(key.x, key.y, 0))
			_chunks.erase(key)
			_versions.erase(key)
	_masks.publish()
	var metadata: Dictionary = simulation_node.try_get_zoning_cell_chunk_states(packed)
	if bool(metadata.get("busy", true)):
		return
	var states: PackedInt64Array = metadata.get("states", PackedInt64Array())
	if states.size() != keys.size() * 4:
		return
	var uploads := 0
	for i in range(keys.size()):
		var key: Vector2i = keys[i]
		# Per chunk: crossing-owner revision, generated flag, parcel revision, world epoch.
		var version := PackedInt64Array([states[i * 4], states[i * 4 + 2], states[i * 4 + 3]])
		var known: PackedInt64Array = _versions.get(key, PackedInt64Array())
		if states[i * 4 + 1] != 0 and known == version:
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
	_chunks[key] = payload
	_masks.replace(Vector3i(key.x, key.y, 0), payload)
	_masks.publish()

func set_preview(payload: Dictionary) -> void:
	if payload.is_empty() and _preview_payload.is_empty() and _preview_keys.is_empty():
		return
	_preview_payload = payload
	_upload_preview(_visible_chunks())

func _upload_preview(keys: Array[Vector2i]) -> void:
	_preview_view = keys
	var retained := {}
	var visible_keys := {}
	for key in keys:
		visible_keys[key] = true
	for chunk: Dictionary in _preview_payload.get("masks", []):
		var key: Vector2i = chunk.chunk
		if not visible_keys.has(key):
			continue
		var address := Vector3i(key.x, key.y, 1)
		retained[address] = true
		_masks.replace(address, chunk)
	for key: Vector3i in _preview_keys:
		if not retained.has(key):
			_masks.remove(key)
	_preview_keys = retained
	_masks.publish()

func _exit_tree() -> void:
	_masks.clear()

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
