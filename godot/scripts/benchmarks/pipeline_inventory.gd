# SPDX-License-Identifier: GPL-3.0-or-later

## The stream benchmark's pipeline inventory (`RENDER-43`, METRUM_STREAM_INVENTORY=1, untimed): what
## a pipeline depends on (the shader, the vertex format and the primitive) for every surface the
## world draws, and which of those the menu's prewarm templates (`PipelinePrewarm.templates()`)
## do not hold. A template of the wrong vertex format compiles pipelines no surface uses and says
## nothing; this lists what the templates missed. Sources: the tree's geometry nodes, the building
## batches' looks (`building_lods.gd`), the window shader's variants made so far, and the stream
## core's slots (by vertex format only: their materials sit on RenderingServer instances).
extends RefCounted

const PipelinePrewarm := preload("res://scripts/renderers/pipeline_prewarm.gd")
const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")

# The vertex format bits a pipeline's vertex layout follows: the attributes but the index, their
# custom formats, compression and 2D vertices (the format version bits above them are not).
const FORMAT_MASK := (((1 << 25) - 1) & ~Mesh.ARRAY_FORMAT_INDEX) | Mesh.ARRAY_FLAG_USE_2D_VERTICES \
	| Mesh.ARRAY_FLAG_COMPRESS_ATTRIBUTES

## The inventory of `main`'s world: `drawn` (key to its node count with up to four node paths,
## and its building looks' count),
## `untemplated` (the drawn keys no template holds), `window_variants` (every variant of the
## window shader made, by code hash, with whether a template holds it), `slots` (each stream
## layer's vertex formats with whether a template has the format) and `templates` (the count).
static func collect(main: Node) -> Dictionary:
	var drawn := {}
	_walk(main, drawn)
	var buildings := main.get_node_or_null("Buildings")
	var lods: Object = buildings.get("lod_renderer") if buildings != null else null
	var looks = lods.get("_looks") if lods != null else null
	if looks is Dictionary:
		for look: Array in (looks as Dictionary).values():
			var mesh := look[0] as Mesh
			if mesh != null:
				for surface in mesh.get_surface_count():
					_count(drawn, surface_key(mesh, surface, look[1] as Material), "", true)
	var templated := {}
	var formats := {}
	for pair: Array in PipelinePrewarm.templates():
		var mesh := pair[0] as Mesh
		if mesh == null:
			continue
		for surface in mesh.get_surface_count():
			templated[surface_key(mesh, surface, pair[1] as Material)] = true
			formats[_format(mesh, surface)] = true
	var untemplated := []
	for key: String in drawn:
		if not templated.has(key):
			untemplated.append(key)
	untemplated.sort()
	var variants := {}
	for code: String in WindowMaterials._shaders:
		var label := code.substr(0, code.find("render_mode")).replace("shader_type spatial;", "").strip_edges()
		variants["code:%d" % code.hash()] = {"defines": label.replace("\n", " "),
			"templated": templated.keys().any(func(key: String) -> bool: return key.begins_with("code:%d " % code.hash()))}
	var slots := {}
	var stream := main.get_node_or_null("StreamNode")
	var simulation := main.get_node_or_null("SimulationNode")
	for layer: String in ["vegetation", "parked", "agents", "buildings"]:
		var census: Dictionary = {}
		if layer == "buildings":
			census = simulation.get_building_draw_census() if simulation != null else {}
		elif stream != null:
			census = stream.get_draw_census(layer)
		var seen := {}
		var by_format := {}
		for multimesh: int in census.get("multimesh", []):
			var mesh := RenderingServer.multimesh_get_mesh(rid_from_int64(multimesh))
			if not mesh.is_valid() or seen.has(mesh):
				continue
			seen[mesh] = true
			for index in RenderingServer.mesh_get_surface_count(mesh):
				var surface: Dictionary = RenderingServer.mesh_get_surface(mesh, index)
				var format := "%d/%d" % [int(surface.get("format", 0)) & FORMAT_MASK, int(surface.get("primitive", -1))]
				by_format[format] = {"templated": formats.has(format)}
		slots[layer] = by_format
	return {"drawn": drawn, "untemplated": untemplated, "window_variants": variants, "slots": slots,
		"templates": templated.size()}

## What `mesh`'s `surface` drawn with `material` (null: its own) compiles by: the shader (its path,
## its code's hash when it has none, or a base material's class), the masked vertex format and the
## primitive, as "<shader> <format>/<primitive>".
static func surface_key(mesh: Mesh, surface: int, material: Material) -> String:
	if material == null:
		material = mesh.surface_get_material(surface)
	return "%s %s" % [_shader_name(material), _format(mesh, surface)]

static func _shader_name(material: Material) -> String:
	if material is ShaderMaterial and (material as ShaderMaterial).shader != null:
		var shader := (material as ShaderMaterial).shader
		return shader.resource_path if not shader.resource_path.is_empty() else "code:%d" % shader.code.hash()
	return material.get_class() if material != null else "none"

static func _format(mesh: Mesh, surface: int) -> String:
	# The RenderingServer's copy, so a PrimitiveMesh reports its generated format too.
	var data: Dictionary = RenderingServer.mesh_get_surface(mesh.get_rid(), surface)
	return "%d/%d" % [int(data.get("format", 0)) & FORMAT_MASK, int(data.get("primitive", -1))]

static func _walk(node: Node, drawn: Dictionary) -> void:
	var geometry := node as GeometryInstance3D
	if geometry != null:
		var mesh: Mesh = null
		if node is MeshInstance3D:
			mesh = (node as MeshInstance3D).mesh
		elif node is MultiMeshInstance3D and (node as MultiMeshInstance3D).multimesh != null:
			mesh = (node as MultiMeshInstance3D).multimesh.mesh
		if mesh != null:
			for surface in mesh.get_surface_count():
				var material: Material = geometry.material_override
				if material == null and node is MeshInstance3D:
					material = (node as MeshInstance3D).get_active_material(surface)
				_count(drawn, surface_key(mesh, surface, material), str(node.get_path()))
	for child in node.get_children(true):
		_walk(child, drawn)

# A building look counts in `looks`, a node in `count` with its path in `at` (the first four).
static func _count(drawn: Dictionary, key: String, where: String, look := false) -> void:
	var entry: Dictionary = drawn.get(key, {"count": 0, "looks": 0, "at": []})
	if look:
		entry.looks += 1
	else:
		entry.count += 1
		if (entry.at as Array).size() < 4:
			(entry.at as Array).append(where)
	drawn[key] = entry
