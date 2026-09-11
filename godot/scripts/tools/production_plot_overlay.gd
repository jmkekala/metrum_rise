# SPDX-License-Identifier: GPL-2.0-only

## Displays Rust's farm lot and blocking building/yard boundary during field authoring.
## Geometry is built once per selected farm; this node has no per-frame simulation queries.
extends MeshInstance3D

const LEGEND := "Yellow: farm plot. Red: building / yard — keep the field outside."
const PLOT_COLOR := Color(1.0, 0.85, 0.15, 1.0)
const SITE_COLOR := Color(1.0, 0.25, 0.15, 1.0)
const LINE_HALF_WIDTH_M := 0.18

func _init() -> void:
	cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var material := StandardMaterial3D.new()
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.vertex_color_use_as_albedo = true
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.cull_mode = BaseMaterial3D.CULL_DISABLED
	material.no_depth_test = true
	material.render_priority = 20
	material_override = material
	hide()

func show_boundaries(details: Dictionary) -> void:
	var plot: PackedVector3Array = details.get("plot_corners", PackedVector3Array())
	var site: PackedVector3Array = details.get("building_site_corners", PackedVector3Array())
	var drawing := ImmediateMesh.new()
	if plot.size() < 3 and site.size() < 3:
		clear_boundaries()
		return
	drawing.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	# Rust derives the blocking site as a convex hull, so its fill is a linear triangle fan.
	if site.size() >= 3:
		drawing.surface_set_color(Color(SITE_COLOR, 0.18))
		for index in range(1, site.size() - 1):
			drawing.surface_add_vertex(site[0])
			drawing.surface_add_vertex(site[index])
			drawing.surface_add_vertex(site[index + 1])
	_append_outline(drawing, plot, PLOT_COLOR)
	_append_outline(drawing, site, SITE_COLOR)
	drawing.surface_end()
	mesh = drawing
	show()

func clear_boundaries() -> void:
	hide()
	mesh = null

func _append_outline(drawing: ImmediateMesh, polygon: PackedVector3Array, color: Color) -> void:
	if polygon.size() < 3:
		return
	drawing.surface_set_color(color)
	for index in polygon.size():
		var start := polygon[index]
		var end := polygon[(index + 1) % polygon.size()]
		var along := Vector3(end.x - start.x, 0.0, end.z - start.z).normalized()
		var across := Vector3(-along.z, 0.0, along.x) * LINE_HALF_WIDTH_M
		for point in [start - across, end - across, end + across, start - across, end + across, start + across]:
			drawing.surface_add_vertex(point)
