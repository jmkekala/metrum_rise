# SPDX-License-Identifier: GPL-3.0-or-later

## The backdrop's drawing resources (`TERRAIN-41` D2, docs/terrain.md "The backdrop drawn"): the
## cell mesh and one material a block of the terrain shader's backdrop variant, handed to the
## StreamNode at a world's setup. The StreamNode binds each block's textures to its material and
## decides which patches draw a cell (every patch of the map not drawn as a patch); this script
## only builds the resources and keeps the materials alive.
extends RefCounted

const BACKDROP_SHADER := preload("res://assets/materials/terrain_backdrop.gdshader")
# The real water's constants: the backdrop's flat water takes its deep colour and Fresnel terms
# from them (`TERRAIN-41` D3), so the two cannot drift apart.
const WaterRenderer := preload("res://scripts/renderers/water.gd")

## The blocks' materials, row-major as the backdrop's blocks.
var materials: Array[ShaderMaterial] = []
var _mesh: ArrayMesh
var _lattice := 0

## Builds the mesh and the materials for a map of `cols` x `rows` patches of `interval_cells`
## cells of `cell_m` metres over `world_size`, sets each material's shared parameters through
## `configure` (the patch materials' own), and hands them to `stream`. The backdrop's layout
## (block size, lattice and land cover steps) is the StreamNode's (`get_backdrop()`).
func setup(stream: Node, scenario: RID, world_size: Vector2, cols: int, rows: int,
		interval_cells: int, cell_m: float, configure: Callable) -> void:
	var layout: Dictionary = stream.get_backdrop()
	var block_patches := int(layout.get("block_patches", 0))
	var step_cells := int(layout.get("height_step_cells", 0))
	var cover_cells := int(layout.get("cover_step_cells", 0))
	if (block_patches <= 0 or step_cells <= 0 or cover_cells <= 0 or cols <= 0 or rows <= 0
			or interval_cells % step_cells != 0 or interval_cells % cover_cells != 0):
		return
	var lattice := interval_cells / step_cells
	if _mesh == null or _lattice != lattice:
		_mesh = _cell_mesh(lattice)
		_lattice = lattice
	var span := cell_m * interval_cells
	var block_m := span * block_patches
	# The land cover texture: one texel a cover step, cell-centred, plus the next block's first.
	var cover_m := cell_m * cover_cells
	var cover_side := float(block_patches * (interval_cells / cover_cells) + 1) * cover_m
	var made: Array[ShaderMaterial] = []
	var rids: Array[RID] = []
	for block_z in ceili(rows / float(block_patches)):
		for block_x in ceili(cols / float(block_patches)):
			var material := ShaderMaterial.new()
			material.shader = BACKDROP_SHADER
			configure.call(material)
			var origin := -world_size * 0.5 + Vector2(block_x, block_z) * block_m
			material.set_shader_parameter("terrain_cell_m", cell_m * step_cells)
			material.set_shader_parameter("patch_world_size_m", Vector2(span, span))
			material.set_shader_parameter(
				"land_cover_world_bounds", Vector4(origin.x, origin.y, cover_side, cover_side)
			)
			material.set_shader_parameter("terrain_debug_lod_step", float(step_cells))
			material.set_shader_parameter("backdrop_water_color", WaterRenderer.DEEP_WATER_COLOR)
			material.set_shader_parameter("backdrop_water_fresnel",
				Vector2(WaterRenderer.WATER_FRESNEL_POWER, WaterRenderer.WATER_FRESNEL_STRENGTH))
			made.append(material)
			rids.append(material.get_rid())
	# Handed over before the old materials are dropped, so no cell names a freed material.
	stream.set_backdrop_drawing(scenario, _mesh.get_rid(), rids)
	materials = made

## A cell: (n + 1)^2 lattice vertices (x, z = the lattice index, y = 0) and a skirt along the four
## edges (y = -1), which the shader drops below the edge. No UVs or normals: the shader makes them.
static func _cell_mesh(n: int) -> ArrayMesh:
	var vertices := PackedVector3Array()
	var normals := PackedVector3Array()
	var indices := PackedInt32Array()
	for z in n + 1:
		for x in n + 1:
			vertices.append(Vector3(x, 0, z))
	for z in n:
		for x in n:
			var a := z * (n + 1) + x
			indices.append_array([a, a + 1, a + n + 1, a + 1, a + n + 2, a + n + 1])
	# The border's vertices in order around the cell, each with a skirt vertex below it.
	var ring: Array[Vector2i] = []
	for x in n:
		ring.append(Vector2i(x, 0))
	for z in n:
		ring.append(Vector2i(n, z))
	for x in range(n, 0, -1):
		ring.append(Vector2i(x, n))
	for z in range(n, 0, -1):
		ring.append(Vector2i(0, z))
	var skirt := vertices.size()
	for at in ring:
		vertices.append(Vector3(at.x, -1, at.y))
	for i in ring.size():
		var j := (i + 1) % ring.size()
		var top_i := ring[i].y * (n + 1) + ring[i].x
		var top_j := ring[j].y * (n + 1) + ring[j].x
		indices.append_array([top_i, top_j, skirt + i, top_j, skirt + j, skirt + i])
	normals.resize(vertices.size())
	normals.fill(Vector3.UP)
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_INDEX] = indices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh
