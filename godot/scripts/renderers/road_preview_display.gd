# SPDX-License-Identifier: GPL-2.0-only

## Reversible render-only road preview for the road tool's synchronous preview (`ROAD-68-E`),
## below the chunk since `ROAD-68-H` round 3 (`docs/roads.md`). For each chunk a stroke changes,
## Rust draws an overlay of the rebuilt pieces (`ROAD-112` round R2-4a: encoded in the preview's
## payload stage and added to pooled meshes by the tool's draw module, `road_tool_show_overlay`)
## and the preview names the runs of the resident chunk they replace. The resident chunk stays
## drawn with those runs collapsed in its vertex buffer (three equal positions draw nothing), and
## a run shown again gets its published positions back. The runs are checked against the resident
## chunks before anything visible changes; Rust stages and swaps its overlays in one call. A
## resident chunk's runs are read from its mesh's layer metas and written through the
## RenderingServer on the mesh's RID, so a chunk a hydration built in Rust (`RENDER-52` part 2)
## collapses as one built here. The runs' bookkeeping moves to Rust in round R2-4b.
extends RefCounted

# A surface's positions are their own f32 stream ahead of its normals (Godot 4.7, checked on the
# GPU): a run's positions are 12 bytes a vertex from 12 * start.
const POSITION_BYTES := 12

# Preview sequence of the displayed overlays; 0 when none is shown.
var sequence: int = 0
## An accepted commit's display, held until its chunks install: Rust holds its overlays with the
## commit's hue and releases them with it (`road_tool_release_held`), so clearing it leaves
## Rust's current overlays alone.
var held := false
# The simulation node whose overlays this display shows (set by the first `show`).
var _simulation: Object = null
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

## Shows a preview's chunks (`overlay_chunks`, `overlay_hidden`, `overlay_hidden_positions`, and
## Rust's overlays of `preview_sequence`) over the resident chunks with the same keys. False,
## with the previous display unchanged, when the runs do not fit the resident chunks or Rust has
## no overlays of that number. A preview of a road mesh generation not installed yet shows no road
## until `resident_installed`.
func show(renderer: Node3D, preview: Dictionary, preview_sequence: int) -> bool:
	if preview_sequence == sequence and sequence > 0:
		return true
	var span := float(preview.get("chunk_span_m", 0.0))
	var origin_x := float(preview.get("chunk_origin_x_m", 0.0))
	var origin_z := float(preview.get("chunk_origin_z_m", 0.0))
	if renderer.generation >= 0 and (span != renderer.span_m or origin_x != renderer.origin_x_m or origin_z != renderer.origin_z_m):
		return false
	# Four ints a chunk: x, z, the end of its triples in `hidden`, the end of its positions.
	var table: Variant = preview.get("overlay_chunks")
	var hidden: Variant = preview.get("overlay_hidden")
	var positions: Variant = preview.get("overlay_hidden_positions")
	if not table is PackedInt32Array or table.is_empty() or table.size() % 4 != 0 or not hidden is PackedInt32Array or not positions is PackedVector3Array:
		return false
	# The hidden runs index the resident chunks of exactly this generation.
	# Held chunks (T61 (2)) are not drawn yet: wait for them as for an install.
	if int(preview.get("road_mesh_generation", -1)) != renderer.generation or renderer.is_holding():
		clear()
		_pending = preview
		_pending_sequence = preview_sequence
		return true
	var keys: Array[Vector2i] = []
	var targets := {}
	var hidden_from := 0
	var positions_from := 0
	for at in range(0, table.size(), 4):
		var key := Vector2i(table[at], table[at + 1])
		if key in keys:
			return false
		keys.append(key)
		var target = _resident_target(renderer, key, hidden, Vector2i(hidden_from, table[at + 2]), positions, Vector2i(positions_from, table[at + 3]))
		if target == null:
			return false
		if not target.is_empty():
			targets[key] = target
		hidden_from = table[at + 2]
		positions_from = table[at + 3]
	if hidden_from != hidden.size() or positions_from != positions.size():
		return false
	# Rust stages the overlays and swaps them for the shown ones in this call, and the runs
	# collapse and restore in this call too, so no frame shows a replaced run beside its rebuilt
	# piece or neither.
	_simulation = renderer.simulation_node
	if not _simulation.road_tool_show_overlay(preview_sequence):
		return false
	_pending = {}
	_apply_runs(targets)
	_keys = keys
	_shown = preview
	sequence = preview_sequence
	return true

## After the chunk renderer installed resident chunks: shows the preview that waited for them, or
## re-shows the shown one when the install replaced chunks it indexes.
func resident_installed(renderer: Node3D) -> void:
	var preview := _pending
	var preview_sequence := _pending_sequence
	if preview.is_empty() and sequence > 0 and int(_shown.get("road_mesh_generation", -1)) != renderer.generation:
		preview = _shown
		preview_sequence = sequence
	if preview.is_empty():
		return
	clear()
	if not show(renderer, preview, preview_sequence):
		clear()

## Restores every collapsed run and hides the overlays (a held display's go with the commit's
## hue, `road_tool_release_held`).
func clear() -> void:
	_apply_runs({})
	_keys.clear()
	if sequence > 0 and not held and is_instance_valid(_simulation):
		_simulation.road_tool_clear_overlay()
	_pending = {}
	_pending_sequence = 0
	_shown = {}
	sequence = 0

## Ends the display. Use when the preview session ends.
func reset() -> void:
	clear()

# The resident runs a chunk's preview hides: {"instance", "positions", "runs": {Vector3i(layer,
# start, end): Vector2i(first position, count)}}; empty when it hides none; null when they do not
# fit the resident chunk. `span` and `position_span` are the chunk's [from, to) in the preview's
# flat `hidden` triples and `positions`.
func _resident_target(renderer: Node3D, key: Vector2i, hidden: PackedInt32Array, span: Vector2i, positions: PackedVector3Array, position_span: Vector2i) -> Variant:
	if span.y < span.x or span.y > hidden.size() or (span.y - span.x) % 3 != 0 or position_span.y < position_span.x or position_span.y > positions.size():
		return null
	if span.y == span.x:
		return {} if position_span.y == position_span.x else null
	if not renderer.chunks.has(key):
		return null
	var instance: MeshInstance3D = renderer.chunks[key]
	var mesh := instance.mesh as ArrayMesh
	var surfaces: Variant = mesh.get_meta("road_layer_surfaces", null) if mesh != null else null
	var counts: Variant = mesh.get_meta("road_layer_counts", null) if mesh != null else null
	var formats: Variant = mesh.get_meta("road_layer_formats", null) if mesh != null else null
	if not surfaces is PackedInt32Array or not counts is PackedInt32Array or not formats is PackedInt64Array:
		return null
	var runs := {}
	var first := position_span.x
	for index in range(span.x, span.y, 3):
		var layer: int = hidden[index]
		var start: int = hidden[index + 1]
		var end: int = hidden[index + 2]
		if layer < 0 or layer >= surfaces.size() or surfaces[layer] < 0 or start < 0 or end <= start:
			return null
		var count: int = counts[layer]
		if end > count or RenderingServer.mesh_surface_get_format_vertex_stride(formats[layer], count) != POSITION_BYTES:
			return null
		runs[Vector3i(layer, start, end)] = Vector2i(first, end - start)
		first += end - start
	if first != position_span.y:
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
	RenderingServer.mesh_surface_update_vertex_region(mesh.get_rid(), surfaces[run.x], run.y * POSITION_BYTES, bytes)
