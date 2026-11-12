# SPDX-License-Identifier: GPL-2.0-only

## Ground-mask chunks from Rust on the streaming core's 2 km ring about the eye (`RENDER-32-P`);
## retained textures upload only after local changes. Generation, reservations, cell ownership
## and paint remain in the simulation.
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
# The streaming core: a sibling, or the overlay's sibling in the game scene.
@onready var _stream: Node = get_node_or_null("../StreamNode") if has_node("../StreamNode") else get_node_or_null("../../StreamNode")
# The ring's resident chunks, nearest first, as of its revision.
var _ring: Array[Vector2i] = []
var _ring_revision := -1
var _world := Vector2.ZERO
const UPLOADS_PER_FRAME := 2

func full_refresh() -> void:
	_versions.clear()
	_chunks.clear()
	_preview_keys.clear()
	_preview_payload.clear()
	_preview_view.clear()
	_masks.clear()
	_masks.setup(simulation_node.get_terrain_world_size())
	_world = simulation_node.get_terrain_world_size()

func _ready() -> void:
	_span = simulation_node.get_zoning_cell_chunk_span()
	_world = simulation_node.get_terrain_world_size()
	_masks.setup(_world)
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

# The ring's chunks about the camera (the stream node moves it: nothing for a still eye).
func _visible_chunks() -> Array[Vector2i]:
	var camera := get_viewport().get_camera_3d()
	if _stream == null or camera == null:
		return _ring
	var revision: int = _stream.update_zoning_ring(camera.global_position, _span, _world)
	if revision != _ring_revision:
		_ring_revision = revision
		# A new array: the preview keeps the list it was uploaded for.
		_ring = []
		var pairs: PackedInt32Array = _stream.get_zoning_ring()
		for at in range(0, pairs.size(), 2):
			_ring.append(Vector2i(pairs[at], pairs[at + 1]))
	return _ring
