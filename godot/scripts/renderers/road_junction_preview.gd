# SPDX-License-Identifier: GPL-2.0-only

## Reversible render-only junction replacements; Rust owns all geometry and source selection.
## Originals stay resident; unchanged retained meshes survive successive pointer updates.
extends RefCounted

var generation: int = -1
var request_id: int = 0
var _terrain_coupled: bool = false
var retained_revision: int = 0
var _instances: Array[MeshInstance3D] = []
var _retained_instances: Array[MeshInstance3D] = []
var _retained_keys := PackedInt32Array()
var _retained_source_generation: int = -1
var _hidden: Array[MeshInstance3D] = []

func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE:
		# Detached cache nodes have no SceneTree owner. Final helper teardown must free them
		# synchronously, including tools destroyed outside the tree or during engine shutdown.
		for instance in _retained_instances:
			if is_instance_valid(instance) and instance.get_parent() == null:
				instance.free()

func show_preview(tool: Node3D, data: Dictionary, requested_id: int) -> bool:
	var source_generation := int(data.get("surface_generation", -1))
	# Water edits advance query validity without changing the resident road-mesh revision.
	var mesh_generation := int(data.get("source_mesh_generation", -1))
	if source_generation < 0 or mesh_generation != tool._road_mesh_generation or source_generation != tool.simulation_node.get_road_tool_surface_generation():
		return false
	var terrain_coupled := bool(data.get("terrain_coupled", false))
	if requested_id == request_id and generation == source_generation and terrain_coupled == _terrain_coupled:
		return true
	var keys: PackedInt32Array = data.get("replacement_keys", PackedInt32Array())
	if keys.is_empty() or keys.size() % 2 != 0:
		return false
	var key_set := {}
	for index in range(0, keys.size(), 2):
		var key := Vector2i(keys[index], keys[index + 1])
		if key_set.has(key):
			return false
		key_set[key] = true
	var span := float(data.get("chunk_span_m", 0.0))
	var origin_x := float(data.get("chunk_origin_x_m", 0.0))
	var origin_z := float(data.get("chunk_origin_z_m", 0.0))
	if span != tool._road_chunk_span_m or origin_x != tool._road_chunk_origin_x_m or origin_z != tool._road_chunk_origin_z_m:
		return false
	var revision := int(data.get("retained_revision", 0))
	if revision <= 0:
		return false
	var reuse_retained := revision == retained_revision and mesh_generation == _retained_source_generation
	if reuse_retained and keys != _retained_keys:
		return false
	var staged: Array[MeshInstance3D] = []
	var retained: Array[MeshInstance3D] = []
	if not _stage(tool, data.get("chunks"), key_set, span, origin_x, origin_z, staged):
		_discard(staged)
		return false
	if not reuse_retained and not _stage(tool, data.get("retained_chunks"), key_set, span, origin_x, origin_z, retained):
		_discard(staged)
		_discard(retained)
		return false
	# Stage everything before hiding originals. No frame can expose a half-built replacement.
	if staged.is_empty() or int(tool.simulation_node.get_road_tool_surface_generation()) != source_generation:
		_discard(staged)
		_discard(retained)
		return false
	if not reuse_retained:
		reset()
		_retained_instances = retained
		_retained_keys = keys
		_retained_source_generation = mesh_generation
		retained_revision = revision
	else:
		_discard(_instances)
	if generation < 0:
		for key in key_set:
			if tool._road_chunk_instances.has(key):
				var original: MeshInstance3D = tool._road_chunk_instances[key]
				original.visible = false
				_hidden.append(original)
		for instance in _retained_instances:
			tool.road_mesh_root.add_child(instance)
	for instance in staged:
		tool.road_mesh_root.add_child(instance)
	_instances = staged
	generation = source_generation
	request_id = requested_id
	_terrain_coupled = terrain_coupled
	return true

func clear() -> void:
	for instance in _hidden:
		if is_instance_valid(instance):
			instance.visible = true
	_hidden.clear()
	_discard(_instances)
	# Keep one detached retained revision during a drag. Re-entering a valid preview needs no
	# upload, and a delta payload remains usable after a temporary invalid candidate.
	for instance in _retained_instances:
		if instance.get_parent() != null:
			instance.get_parent().remove_child(instance)
	generation = -1
	request_id = 0

func reset() -> void:
	clear()
	_discard(_retained_instances)
	_retained_keys = PackedInt32Array()
	_retained_source_generation = -1
	retained_revision = 0

func _stage(tool: Node3D, chunks: Variant, keys: Dictionary, span: float, origin_x: float, origin_z: float, output: Array[MeshInstance3D]) -> bool:
	if not chunks is Array:
		return false
	var seen := {}
	for chunk in chunks:
		if not chunk is Dictionary or not chunk.has("chunk_x") or not chunk.has("chunk_z"):
			return false
		var key := Vector2i(chunk["chunk_x"], chunk["chunk_z"])
		if not keys.has(key) or seen.has(key) or chunk.get("removed", true):
			return false
		seen[key] = true
		var instance: MeshInstance3D = tool._build_road_chunk_instance(chunk, key, span, origin_x, origin_z)
		if instance == null:
			return false
		instance.name = "JunctionPreview_%d_%d" % [key.x, key.y]
		output.append(instance)
	return true

func _discard(instances: Array[MeshInstance3D]) -> void:
	for instance in instances:
		if is_instance_valid(instance):
			if instance.get_parent() != null:
				instance.get_parent().remove_child(instance)
			instance.queue_free()
	instances.clear()
