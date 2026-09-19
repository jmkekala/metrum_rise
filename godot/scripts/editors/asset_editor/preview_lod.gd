# SPDX-License-Identifier: GPL-2.0-only

## Editor-only orchestration of the shared Rust LOD policy and preloaded preview tiers.
## Idle updates are O(1); changed camera/placement updates are O(parts in this document).
## This is not the city renderer: no simulation collection or asset manifest is mutated.
extends RefCounted

const MeshPart = preload("res://scripts/editors/asset_editor/mesh_part.gd")

class PartState:
	var paths: Array[String] = []
	var failed: Array[String] = []
	var forced := -1
	var active := -1
	var previous := -1
	var pixels := 0.0
	var triangles := 0

var policy := AssetLodPolicy.new()
var quality := 1
var states: Dictionary = {}
var evaluations := 0
var _parts: Array[MeshPart] = []
var _selected := -1
var _preview: Node3D
var _panel: Control
var _dirty := true
var _camera_transform := Transform3D.IDENTITY
var _projection := Projection()
var _render_size := Vector2.ZERO
var _revision := -1
var _preview_transform := Transform3D.IDENTITY

func configure(preview: Node3D, panel: Control) -> void:
	_preview = preview
	_panel = panel

## Synchronize only on selection/document changes; loading is deliberately outside update().
func sync(parts: Array[MeshPart], selected: int) -> void:
	_parts = parts
	_selected = selected
	var retained := {}
	for index in parts.size():
		var part := parts[index]
		var state: PartState = states[part] if states.has(part) else PartState.new()
		var paths: Array[String] = []
		for lod in part.lods:
			paths.append(lod.source_path)
		if state.paths != paths:
			state.paths = paths
			state.failed = _preview.prepare_mesh_part_lods(index, paths)
			state.active = paths.find(_preview.mesh_part_lod_path(index))
			state.triangles = _preview.mesh_part_triangles(index)
			state.previous = -1
			state.forced = mini(state.forced, paths.size() - 1)
		retained[part] = state
	states = retained
	if selected >= 0 and selected < parts.size():
		_panel.set_part(parts[selected], states[parts[selected]].forced)
	else:
		_panel.set_part(null)
	_dirty = true

func set_mode(index: int) -> void:
	if _selected < 0 or _selected >= _parts.size():
		return
	var state: PartState = states[_parts[_selected]]
	state.forced = clampi(index, -1, state.paths.size() - 1)
	state.previous = -1
	_panel.set_part(_parts[_selected], state.forced)
	_dirty = true

func set_quality(index: int) -> void:
	quality = clampi(index, 0, 2)
	for state: PartState in states.values():
		state.previous = -1
	_dirty = true

func update(camera: Camera3D) -> void:
	if _preview == null or _parts.is_empty():
		return
	var transform := camera.get_camera_transform()
	var projection := camera.get_camera_projection()
	var viewport := camera.get_viewport()
	var render_size := Vector2(viewport.get_texture().get_size()) * viewport.scaling_3d_scale
	if not _dirty and transform == _camera_transform and projection == _projection and render_size == _render_size and _revision == _preview.lod_revision and _preview_transform == _preview.global_transform:
		return
	_dirty = false
	_camera_transform = transform
	_projection = projection
	_render_size = render_size
	_revision = _preview.lod_revision
	_preview_transform = _preview.global_transform
	var view := transform.affine_inverse()
	for index in _parts.size():
		var part := _parts[index]
		var state: PartState = states[part]
		var model: Transform3D = _preview.mesh_part_transform(index)
		state.pixels = policy.projected_size_pixels(part.aabb, projection * Projection(view * model), render_size)
		evaluations += 1
		var target := state.forced
		if target < 0:
			target = policy.select_lod(state.pixels, state.paths.size(), state.previous, quality)
		var error := ""
		if target >= 0:
			if state.failed.has(state.paths[target]):
				error = "LOD%d failed to load; previous mesh retained. Reimport the tier to retry." % target
			elif _preview.set_mesh_part_lod(index, state.paths[target]):
				state.active = target
				state.previous = target if state.forced < 0 else -1
				state.triangles = _preview.mesh_part_triangles(index)
		if index == _selected:
			if error.is_empty() and not state.failed.is_empty():
				error = "%d tier(s) failed to load; this chain cannot be fully previewed." % state.failed.size()
			_panel.show_lod_state(state.active, state.pixels, state.triangles, error)
