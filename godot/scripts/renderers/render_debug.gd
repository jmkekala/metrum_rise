# SPDX-License-Identifier: GPL-2.0-only

## Shared mesh and buffer diagnostics for terrain and water renderers.
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
