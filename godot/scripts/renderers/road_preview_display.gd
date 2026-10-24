# SPDX-License-Identifier: GPL-2.0-only

## Reversible render-only road chunk replacements for the road tool's synchronous preview
## (`ROAD-68-E`). Rust builds every chunk a stroke changes whole, as the commit publishes it;
## this hides the resident originals of those chunks and shows the replacements, all staged
## before anything visible changes. Originals stay resident until the commit's own chunks
## replace them. Replaced preview nodes and their ArrayMeshes are recycled as detached spares.
extends RefCounted

# Preview sequence of the displayed chunks; 0 when none is shown.
var sequence: int = 0
var _instances: Array[MeshInstance3D] = []
var _hidden: Array[MeshInstance3D] = []
# The chunk keys the displayed preview replaces.
var _keys: Array[Vector2i] = []
# Never displayed. Staging fills only these, so a failed batch cannot touch the visible one.
# Trimmed to the displayed count on install: the pool is one staging set, not cursor history.
var _spare: Array[MeshInstance3D] = []

func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE:
		# Detached spares have no SceneTree owner; free them with the helper.
		for instance in _spare:
			if is_instance_valid(instance) and instance.get_parent() == null:
				instance.free()

## Shows a preview's `road_chunks` in place of the resident chunks with the same keys. False,
## with the previous display unchanged, when the payload does not fit the resident chunk grid.
func show(tool: Node3D, preview: Dictionary, preview_sequence: int) -> bool:
	if preview_sequence == sequence and sequence > 0:
		return true
	var span := float(preview.get("chunk_span_m", 0.0))
	var origin_x := float(preview.get("chunk_origin_x_m", 0.0))
	var origin_z := float(preview.get("chunk_origin_z_m", 0.0))
	if tool._road_mesh_generation >= 0 and (span != tool._road_chunk_span_m or origin_x != tool._road_chunk_origin_x_m or origin_z != tool._road_chunk_origin_z_m):
		return false
	var chunks: Variant = preview.get("road_chunks")
	if not chunks is Array or chunks.is_empty():
		return false
	var keys: Array[Vector2i] = []
	var staged: Array[MeshInstance3D] = []
	for chunk in chunks:
		if not chunk is Dictionary or not chunk.has("chunk_x") or not chunk.has("chunk_z"):
			_recycle(staged)
			return false
		var key := Vector2i(chunk["chunk_x"], chunk["chunk_z"])
		if key in keys:
			_recycle(staged)
			return false
		keys.append(key)
		# A chunk the stroke empties only hides its original.
		if chunk.get("removed", false):
			continue
		var instance: MeshInstance3D = _spare.pop_back() if not _spare.is_empty() else tool._new_road_chunk_node()
		# Spares are detached and hidden from the display, so clearing their surfaces is safe.
		instance.mesh.clear_surfaces()
		staged.append(instance)
		if not tool._fill_road_chunk_mesh(instance.mesh, chunk):
			_recycle(staged)
			return false
		instance.name = "RoadPreview_%d_%d" % [key.x, key.y]
		tool._place_road_chunk_instance(instance, key, span, origin_x, origin_z)
	# Everything is staged before the visible state changes: no frame shows a partial swap.
	clear()
	_keys = keys
	_hide_originals(tool)
	for instance in staged:
		tool.road_mesh_root.add_child(instance)
	_instances = staged
	sequence = preview_sequence
	while _spare.size() > _instances.size():
		_spare.pop_back().free()
	return true

## Hides the resident chunks installed since the preview was shown, such as a previous commit's,
## in place of the originals they replaced.
func refresh_originals(tool: Node3D) -> void:
	if sequence == 0:
		return
	_show_hidden()
	_hide_originals(tool)

## Hides the replacements and shows the originals again.
func clear() -> void:
	_show_hidden()
	_keys.clear()
	_recycle(_instances)
	sequence = 0

func _hide_originals(tool: Node3D) -> void:
	for key in _keys:
		if tool._road_chunk_instances.has(key):
			var original: MeshInstance3D = tool._road_chunk_instances[key]
			original.visible = false
			_hidden.append(original)

func _show_hidden() -> void:
	for instance in _hidden:
		if is_instance_valid(instance):
			instance.visible = true
	_hidden.clear()

## Releases every preview-owned node, including spares. Use when the preview session ends.
func reset() -> void:
	clear()
	for instance in _spare:
		if is_instance_valid(instance):
			instance.queue_free()
	_spare.clear()

# Returns replaced or failed staging nodes to the detached spare set.
func _recycle(instances: Array[MeshInstance3D]) -> void:
	for instance in instances:
		if is_instance_valid(instance):
			if instance.get_parent() != null:
				instance.get_parent().remove_child(instance)
			_spare.append(instance)
	instances.clear()
