# SPDX-License-Identifier: GPL-3.0-or-later

## The pieces of a pipeline prewarm template (`RENDER-43`, `pipeline_prewarm.gd`). A pipeline
## depends on the shader, the vertex format, the primitive and the pass, never on a material's
## parameters or a mesh's size, so a renderer's template is its shader on one triangle of its real
## mesh's vertex format, with no model, scene or texture loaded. Kept apart from
## `pipeline_prewarm.gd`, which preloads the renderers that use these.
extends RefCounted

## One triangle with the arrays `format`'s `Mesh.ARRAY_FORMAT_*` bits name (vertex, normal,
## tangent, colour, UV, UV2, CUSTOM0 as RGBA float), its attributes compressed when `compressed`.
static func format_mesh(format: int, compressed := false) -> ArrayMesh:
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = PackedVector3Array([Vector3.ZERO, Vector3.RIGHT, Vector3.BACK])
	var flags := Mesh.ARRAY_FLAG_COMPRESS_ATTRIBUTES if compressed else 0
	if format & Mesh.ARRAY_FORMAT_NORMAL:
		arrays[Mesh.ARRAY_NORMAL] = PackedVector3Array([Vector3.UP, Vector3.UP, Vector3.UP])
	if format & Mesh.ARRAY_FORMAT_TANGENT:
		arrays[Mesh.ARRAY_TANGENT] = PackedFloat32Array([1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1])
	if format & Mesh.ARRAY_FORMAT_COLOR:
		arrays[Mesh.ARRAY_COLOR] = PackedColorArray([Color.WHITE, Color.WHITE, Color.WHITE])
	if format & Mesh.ARRAY_FORMAT_TEX_UV:
		arrays[Mesh.ARRAY_TEX_UV] = PackedVector2Array([Vector2.ZERO, Vector2.RIGHT, Vector2.DOWN])
	if format & Mesh.ARRAY_FORMAT_TEX_UV2:
		arrays[Mesh.ARRAY_TEX_UV2] = PackedVector2Array([Vector2.ZERO, Vector2.RIGHT, Vector2.DOWN])
	if format & Mesh.ARRAY_FORMAT_CUSTOM0:
		var custom := PackedFloat32Array()
		custom.resize(12)
		arrays[Mesh.ARRAY_CUSTOM0] = custom
		flags |= Mesh.ARRAY_CUSTOM_RGBA_FLOAT << Mesh.ARRAY_FORMAT_CUSTOM0_SHIFT
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays, [], {}, flags)
	return mesh

## A material with `shader` and nothing else.
static func bare(shader: Shader) -> ShaderMaterial:
	var material := ShaderMaterial.new()
	material.shader = shader
	return material
