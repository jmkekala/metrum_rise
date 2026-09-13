# SPDX-License-Identifier: GPL-2.0-only

## Shared mesh, clip and buffer diagnostics for terrain and water renderers.
extends RefCounted

static func float_stats(values: PackedFloat32Array) -> Dictionary:
	if values.is_empty():
		return {
			"min": 0.0,
			"max": 0.0,
			"nonzero": 0,
			"sum": 0.0,
		}
	var min_value: float = values[0]
	var max_value: float = values[0]
	var nonzero_count: int = 0
	var sum_value: float = 0.0
	for value in values:
		min_value = minf(min_value, value)
		max_value = maxf(max_value, value)
		sum_value += value
		if absf(value) > 0.001:
			nonzero_count += 1
	return {
		"min": min_value,
		"max": max_value,
		"nonzero": nonzero_count,
		"sum": sum_value,
	}


static func clip_stats(loop_groups: Array) -> Dictionary:
	var has_bounds: bool = false
	var min_x: float = 0.0
	var max_x: float = 0.0
	var min_z: float = 0.0
	var max_z: float = 0.0
	var point_count: int = 0
	var total_area: float = 0.0
	var max_bbox_x: float = 0.0
	var max_bbox_z: float = 0.0
	var loop_count: int = 0
	for group_variant in loop_groups:
		var clip_group: Dictionary = group_variant
		var bounds: Rect2 = clip_group["bounds"]
		max_bbox_x = maxf(max_bbox_x, bounds.size.x)
		max_bbox_z = maxf(max_bbox_z, bounds.size.y)
		if not has_bounds:
			min_x = bounds.position.x
			max_x = bounds.position.x + bounds.size.x
			min_z = bounds.position.y
			max_z = bounds.position.y + bounds.size.y
			has_bounds = true
		else:
			min_x = minf(min_x, bounds.position.x)
			max_x = maxf(max_x, bounds.position.x + bounds.size.x)
			min_z = minf(min_z, bounds.position.y)
			max_z = maxf(max_z, bounds.position.y + bounds.size.y)
		var group_area: float = 0.0
		var outer_loops: Array = clip_group["outer_loops"]
		for outer_variant in outer_loops:
			var outer: Dictionary = outer_variant
			var outer_points: PackedVector2Array = outer["points"]
			point_count += outer_points.size()
			loop_count += 1
			group_area += absf(polygon_area(outer_points))
		var hole_loops: Array = clip_group["hole_loops"]
		for hole_variant in hole_loops:
			var hole: Dictionary = hole_variant
			var hole_points: PackedVector2Array = hole["points"]
			point_count += hole_points.size()
			loop_count += 1
			group_area -= absf(polygon_area(hole_points))
		total_area += maxf(0.0, group_area)
	return {
		"group_count": loop_groups.size(),
		"loop_count": loop_count,
		"point_count": point_count,
		"area": total_area,
		"has_bounds": has_bounds,
		"min_x": min_x,
		"max_x": max_x,
		"min_z": min_z,
		"max_z": max_z,
		"max_bbox_x": max_bbox_x,
		"max_bbox_z": max_bbox_z,
	}

static func polygon_area(points: PackedVector2Array) -> float:
	if points.size() < 3:
		return 0.0
	var area: float = 0.0
	for index in range(points.size()):
		var a: Vector2 = points[index]
		var b: Vector2 = points[(index + 1) % points.size()]
		area += a.x * b.y - b.x * a.y
	return area * 0.5

static func bounds_label(stats: Dictionary) -> String:
	if not bool(stats.get("has_bounds", false)):
		return "none"
	return "[(%.3f,%.3f)..(%.3f,%.3f)]" % [
		float(stats.get("min_x", 0.0)),
		float(stats.get("min_z", 0.0)),
		float(stats.get("max_x", 0.0)),
		float(stats.get("max_z", 0.0)),
	]

static func mesh_label(mesh: Mesh) -> String:
	if mesh == null:
		return "null"
	if mesh is ArrayMesh:
		var array_mesh: ArrayMesh = mesh as ArrayMesh
		var surface_count := array_mesh.get_surface_count()
		var vertex_count := 0
		for surface_index in range(surface_count):
			# Surface metadata already owns the vertex count.
			vertex_count += array_mesh.surface_get_array_len(surface_index)
		return "ArrayMesh surfaces=%d vertices=%d" % [surface_count, vertex_count]
	return mesh.get_class()
