# SPDX-License-Identifier: GPL-2.0-only

## The red "this goes" highlight shared by the bulldoze tool's hover and the road tool's
## demolition preview (`ZONE-09`): filled, outlined polygons on an unshaded, see-through material
## drawn over the ground. Rust supplies the points, already lifted above the surface.
extends RefCounted

const FILL := Color(1.0, 0.08, 0.03, 0.34)
const LINE := Color(1.0, 0.16, 0.08, 0.90)

## One filled, outlined polygon fanned from its first point; null for fewer than three points.
static func polygon(points: PackedVector3Array) -> Mesh:
	return polygons(points, points.size())

## Filled, outlined polygons of `corners` points each, packed one after another (as Rust packs
## lot outlines); null when there is not one whole polygon.
static func polygons(points: PackedVector3Array, corners: int) -> Mesh:
	if corners < 3 or points.size() < corners:
		return null
	var count := points.size() / corners
	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	im.surface_set_color(FILL)
	for polygon_index in count:
		var first := polygon_index * corners
		for i in range(1, corners - 1):
			im.surface_add_vertex(points[first])
			im.surface_add_vertex(points[first + i])
			im.surface_add_vertex(points[first + i + 1])
	im.surface_end()

	im.surface_begin(Mesh.PRIMITIVE_LINES)
	im.surface_set_color(LINE)
	for polygon_index in count:
		var first := polygon_index * corners
		for i in corners:
			im.surface_add_vertex(points[first + i])
			im.surface_add_vertex(points[first + (i + 1) % corners])
	im.surface_end()
	return im

## The highlight's material: unshaded (it marks, it is not lit), vertex-coloured, see-through,
## drawn after the ground.
static func material() -> StandardMaterial3D:
	var mat := StandardMaterial3D.new()
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	mat.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	mat.cull_mode = BaseMaterial3D.CULL_DISABLED
	mat.vertex_color_use_as_albedo = true
	mat.no_depth_test = false
	mat.render_priority = 8
	return mat

## A shadowless, world-placed instance drawing the highlight, hidden until it has a mesh.
static func instance() -> MeshInstance3D:
	var mesh := MeshInstance3D.new()
	mesh.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	mesh.top_level = true
	mesh.visible = false
	mesh.material_override = material()
	return mesh
