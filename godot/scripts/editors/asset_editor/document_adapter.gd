# SPDX-License-Identifier: GPL-2.0-only

## Projects an authoring document into preview objects without repairing or normalizing it.
## Mesh sources are reloaded only when they change; metadata/history never owns scene resources.
extends RefCounted

const MeshPart = preload("res://scripts/editors/asset_editor/mesh_part.gd")
const PreviewGeometry = preload("res://scripts/editors/asset_editor/preview_geometry.gd")

var _editor: Node
var _sources: Array = []
var _rendered: Dictionary = {}
var _render_input: Dictionary = {}

func _init(editor: Node) -> void:
	_editor = editor

func render(state: Dictionary) -> void:
	var input := _geometry_input(state)
	if input == _render_input:
		return
	_render_input = input
	var params: Dictionary = state.get("params", {})
	var sources: Array = state.get("sources", [])
	var entries: Array = params.get("mesh_parts", [])
	var reload_meshes: bool = sources != _sources or entries.size() != _editor._parts.size()
	var selected: int = _editor._selected_part_index
	if reload_meshes:
		_editor._clear_mesh_parts()
		_sources = sources.duplicate(true)
		for index in entries.size():
			var entry: Dictionary = entries[index]
			var paths: Array = sources[index] if index < sources.size() else []
			var first := str(paths[0]) if not paths.is_empty() else ""
			var part := MeshPart.new()
			var count: int = _editor._preview.mesh_part_count()
			part.aabb = _editor._preview.add_mesh_part(first) if FileAccess.file_exists(first) else AABB()
			if _editor._preview.mesh_part_count() == count:
				_editor._preview.add_missing_mesh_part()
			_editor._parts.append(part)
	for index in entries.size():
		var entry: Dictionary = entries[index]
		var part: MeshPart = _editor._parts[index]
		var paths: Array = sources[index] if index < sources.size() else []
		part.name = str(entry.get("name", "Part %d" % [index + 1]))
		part.position = _editor._array_to_vector3(entry.get("position"), Vector3.ZERO)
		part.rotation_y = _editor._array_to_vector3(entry.get("rotation_degrees"), Vector3.ZERO).y
		part.scale = PreviewGeometry.number(entry, "scale", 1.0)
		part.pivot_offset = _editor._array_to_vector3(entry.get("pivot_offset"), Vector3.ZERO)
		part.lods.clear()
		var lods: Array = entry.get("lods", [])
		for tier in lods.size():
			part.lods.append(MeshPart.Lod.new(str(paths[tier]) if tier < paths.size() else "", lods[tier]))
		part.retain_manifest(entry)
		_editor._preview.set_mesh_part_transform(index, part.position, part.rotation_y, part.scale, part.pivot_offset)
	_editor._view._mesh_part_list.clear()
	for part: MeshPart in _editor._parts:
		_editor._view._mesh_part_list.add_item(part.name)
	_editor._site_anchors_data.assign(params.get("anchors", []).duplicate(true))
	_editor._site_surfaces_data.assign(params.get("site_surfaces", []).duplicate(true))
	_editor._frontage_fwd = _editor._array_to_vector3(params.get("frontage_forward"), Vector3.BACK)
	_editor._preview.set_frontage_forward(_editor._frontage_fwd)
	_editor._view._frontage_lbl.text = "Frontage direction: %s" % _editor._frontage_fwd
	_editor._preview.set_lot_size(int(PreviewGeometry.number(params, "lot_width_cells", 2)), int(PreviewGeometry.number(params, "lot_depth_cells", 2)))
	_editor._selected_site_anchor_indices = _editor._selected_site_anchor_indices.filter(func(index): return index < _editor._site_anchors_data.size())
	_editor._selected_site_anchor_index = mini(_editor._selected_site_anchor_index, _editor._site_anchors_data.size() - 1)
	_editor._selected_site_surface_index = mini(_editor._selected_site_surface_index, _editor._site_surfaces_data.size() - 1)
	_editor._refresh_site_anchor_list()
	_editor._refresh_site_surface_list()
	_editor._set_selected_mesh_parts([selected] if selected >= 0 and selected < entries.size() else [], selected)
	_editor._sync_preview_lods()
	_rendered = geometry()

func geometry() -> Dictionary:
	var parts: Array = []
	var sources: Array = []
	for part: MeshPart in _editor._parts:
		parts.append(part.to_manifest())
		var paths: Array = []
		for lod in part.lods:
			paths.append(lod.source_path)
		sources.append(paths)
	return {"mesh_parts": parts, "sources": sources,
		"anchors": _editor._site_anchors_data.duplicate(true), "site_surfaces": _editor._site_surfaces_data.duplicate(true),
		"frontage_forward": [_editor._frontage_fwd.x, _editor._frontage_fwd.y, _editor._frontage_fwd.z]}

## Merge only edited geometry; merely rendering defaults must not fill missing authored fields.
func capture(state: Dictionary) -> Dictionary:
	var next := state.duplicate(true)
	var current := geometry()
	for key in current:
		if current[key] != _rendered.get(key):
			if key == "sources":
				next[key] = current[key]
			else:
				next["params"][key] = current[key]
	_sources = current["sources"].duplicate(true)
	_rendered = current
	_render_input = _geometry_input(next)
	return next

func forget_sources() -> void:
	_sources = [null]
	_render_input = {}

func _geometry_input(state: Dictionary) -> Dictionary:
	var input := {"sources": state.get("sources", [])}
	var params: Dictionary = state.get("params", {})
	for key in ["mesh_parts", "anchors", "site_surfaces", "frontage_forward", "lot_width_cells", "lot_depth_cells"]:
		input[key] = params.get(key)
	return input
