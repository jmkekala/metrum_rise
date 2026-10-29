# SPDX-License-Identifier: GPL-2.0-only

## Reversible render-only road preview for the road tool's synchronous preview (`ROAD-68-E`),
## below the chunk since `ROAD-68-H` round 3 (`docs/roads.md`). Rust sends, for each chunk a
## stroke changes, an overlay of the rebuilt pieces and the runs of the resident chunk they
## replace. The resident chunk stays drawn with those runs collapsed in its vertex buffer (three
## equal positions draw nothing), the overlays draw the rebuilt pieces, and a run shown again gets
## its published positions back. Everything is staged before anything visible changes. Replaced
## overlay nodes and their ArrayMeshes are recycled as detached spares.
extends RefCounted

# A surface's positions are their own f32 stream ahead of its normals (Godot 4.7, checked on the
# GPU): a run's positions are 12 bytes a vertex from 12 * start.
const POSITION_BYTES := 12

# Preview sequence of the displayed overlays; 0 when none is shown.
var sequence: int = 0
var _instances: Array[MeshInstance3D] = []
# The chunk keys the displayed preview changes.
var _keys: Array[Vector2i] = []
# Collapsed runs by chunk key: {"instance": the resident MeshInstance3D, "runs":
# {Vector3i(layer, start, end): its published positions as bytes}}.
var _collapsed: Dictionary = {}
# The latest preview while it indexes resident chunks not installed yet, and its sequence; it is
# shown when they are (`resident_installed`).
var _pending: Dictionary = {}
var _pending_sequence: int = 0
# The shown preview, re-shown when an install replaces the resident chunks it indexes.
var _shown: Dictionary = {}
# Never displayed. Staging fills only these, so a failed batch cannot touch the visible one.
# Trimmed to the displayed count on install: the pool is one staging set, not cursor history.
var _spare: Array[MeshInstance3D] = []

func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE:
		# Detached spares have no SceneTree owner; free them with the helper.
		for instance in _spare:
			if is_instance_valid(instance) and instance.get_parent() == null:
				instance.free()

## Shows a preview's `road_chunks` over the resident chunks with the same keys. False, with the
## previous display unchanged, when the payload does not fit the resident chunks. A preview of a
## road mesh generation not installed yet shows no road until `resident_installed`.
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
	# The hidden runs index the resident chunks of exactly this generation.
	if int(preview.get("road_mesh_generation", -1)) != tool._road_mesh_generation:
		clear()
		_pending = preview
		_pending_sequence = preview_sequence
		return true
	var keys: Array[Vector2i] = []
	var staged: Array[MeshInstance3D] = []
	var targets := {}
	for chunk in chunks:
		if not chunk is Dictionary or not chunk.has("chunk_x") or not chunk.has("chunk_z"):
			_recycle(staged)
			return false
		var key := Vector2i(chunk["chunk_x"], chunk["chunk_z"])
		if key in keys:
			_recycle(staged)
			return false
		keys.append(key)
		var target = _resident_target(tool, key, chunk)
		if target == null:
			_recycle(staged)
			return false
		if not target.is_empty():
			targets[key] = target
		if not _has_overlay(chunk):
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
	# Everything is staged before the visible state changes: the runs collapse and restore and the
	# overlays swap in this call, so no frame shows a replaced run beside its rebuilt piece or
	# neither.
	_pending = {}
	_apply_runs(targets)
	_recycle(_instances)
	for instance in staged:
		tool.road_mesh_root.add_child(instance)
	_instances = staged
	_keys = keys
	_shown = preview
	sequence = preview_sequence
	while _spare.size() > _instances.size():
		_spare.pop_back().free()
	return true

## After the road tool installed resident chunks: shows the preview that waited for them, or
## re-shows the shown one when the install replaced chunks it indexes.
func resident_installed(tool: Node3D) -> void:
	var preview := _pending
	var preview_sequence := _pending_sequence
	if preview.is_empty() and sequence > 0 and int(_shown.get("road_mesh_generation", -1)) != tool._road_mesh_generation:
		preview = _shown
		preview_sequence = sequence
	if preview.is_empty():
		return
	clear()
	if not show(tool, preview, preview_sequence):
		clear()

## Restores every collapsed run and removes the overlays.
func clear() -> void:
	_apply_runs({})
	_keys.clear()
	_recycle(_instances)
	_pending = {}
	_pending_sequence = 0
	_shown = {}
	sequence = 0

## Releases every preview-owned node, including spares. Use when the preview session ends.
func reset() -> void:
	clear()
	for instance in _spare:
		if is_instance_valid(instance):
			instance.queue_free()
	_spare.clear()

# The resident runs a chunk's payload hides: {"instance", "positions", "runs": {Vector3i(layer,
# start, end): Vector2i(first position, count)}}; empty when it hides none; null when the payload
# does not fit the resident chunk.
func _resident_target(tool: Node3D, key: Vector2i, chunk: Dictionary) -> Variant:
	var hidden: Variant = chunk.get("hidden")
	var positions: Variant = chunk.get("hidden_positions")
	if not hidden is PackedInt32Array or hidden.size() % 3 != 0 or not positions is PackedVector3Array:
		return null
	if hidden.is_empty():
		return {}
	if not tool._road_chunk_instances.has(key):
		return null
	var instance: MeshInstance3D = tool._road_chunk_instances[key]
	var mesh := instance.mesh as ArrayMesh
	var surfaces: Variant = mesh.get_meta("road_layer_surfaces", null) if mesh != null else null
	if not surfaces is PackedInt32Array:
		return null
	var runs := {}
	var first := 0
	for index in range(0, hidden.size(), 3):
		var layer: int = hidden[index]
		var start: int = hidden[index + 1]
		var end: int = hidden[index + 2]
		if layer < 0 or layer >= surfaces.size() or surfaces[layer] < 0 or start < 0 or end <= start:
			return null
		var surface: int = surfaces[layer]
		var count := mesh.surface_get_array_len(surface)
		if end > count or RenderingServer.mesh_surface_get_format_vertex_stride(mesh.surface_get_format(surface), count) != POSITION_BYTES:
			return null
		runs[Vector3i(layer, start, end)] = Vector2i(first, end - start)
		first += end - start
	if first != positions.size():
		return null
	return {"instance": instance, "positions": positions, "runs": runs}

# Restores the collapsed runs `targets` no longer hides, then collapses the ones it newly hides,
# keeping their published positions. A resident instance an install replaced takes its runs
# with it.
func _apply_runs(targets: Dictionary) -> void:
	for key in _collapsed.keys():
		var held: Dictionary = _collapsed[key]
		var instance: MeshInstance3D = held["instance"]
		var alive := is_instance_valid(instance) and not instance.is_queued_for_deletion()
		var kept: Dictionary = targets[key]["runs"] if targets.has(key) and targets[key]["instance"] == instance else {}
		var runs: Dictionary = held["runs"]
		for run in runs.keys():
			if kept.has(run):
				continue
			if alive:
				_write_positions(instance.mesh, run, runs[run])
			runs.erase(run)
		if runs.is_empty():
			_collapsed.erase(key)
	for key in targets:
		var target: Dictionary = targets[key]
		var instance: MeshInstance3D = target["instance"]
		if not _collapsed.has(key):
			_collapsed[key] = {"instance": instance, "runs": {}}
		var runs: Dictionary = _collapsed[key]["runs"]
		var positions: PackedVector3Array = target["positions"]
		for run in target["runs"]:
			if runs.has(run):
				continue
			var span: Vector2i = target["runs"][run]
			runs[run] = positions.slice(span.x, span.x + span.y).to_byte_array()
			var collapsed := PackedByteArray()
			collapsed.resize(span.y * POSITION_BYTES)
			collapsed.fill(0)
			_write_positions(instance.mesh, run, collapsed)

func _write_positions(mesh: ArrayMesh, run: Vector3i, bytes: PackedByteArray) -> void:
	var surfaces: PackedInt32Array = mesh.get_meta("road_layer_surfaces")
	mesh.surface_update_vertex_region(surfaces[run.x], run.y * POSITION_BYTES, bytes)

# True when a chunk payload carries any overlay triangle.
static func _has_overlay(chunk: Dictionary) -> bool:
	for layer in ["skirt", "curb", "raised_step", "sidewalk", "road", "marking", "concrete"]:
		var vertices: Variant = chunk.get(layer + "_vertices")
		if vertices is PackedVector3Array and not vertices.is_empty():
			return true
	return false

# Returns replaced or failed staging nodes to the detached spare set.
func _recycle(instances: Array[MeshInstance3D]) -> void:
	for instance in instances:
		if is_instance_valid(instance):
			if instance.get_parent() != null:
				instance.get_parent().remove_child(instance)
			_spare.append(instance)
	instances.clear()
