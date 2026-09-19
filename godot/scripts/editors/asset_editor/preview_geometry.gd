# SPDX-License-Identifier: GPL-2.0-only

## Shared, read-only projection of authored values into preview geometry.
## Invalid metadata stays in the document; display defaults are never written back.
extends RefCounted

static func vector3(value: Variant, fallback: Vector3 = Vector3.ZERO) -> Vector3:
	if not value is Array or value.size() != 3:
		return fallback
	for component in value:
		if not _finite_number(component):
			return fallback
	return Vector3(value[0], value[1], value[2])

static func forward(anchor: Dictionary) -> Vector3:
	var direction := vector3(anchor.get("forward"), Vector3.FORWARD)
	direction.y = 0.0
	return direction.normalized() if direction.length_squared() > 0.001 else Vector3.FORWARD

static func number(data: Dictionary, key: String, fallback: float) -> float:
	var value = data.get(key)
	if value is String and value.strip_edges().is_valid_float():
		value = value.strip_edges().to_float()
	return float(value) if _finite_number(value) else fallback

static func _finite_number(value: Variant) -> bool:
	return (value is int or value is float) and is_finite(float(value))

static func surface_vertices(surface: Dictionary) -> Array[Vector2]:
	var vertices: Array[Vector2] = []
	var raw = surface.get("vertices", [])
	if raw is Array:
		for vertex in raw:
			if vertex is Array and vertex.size() >= 2 and _finite_number(vertex[0]) and _finite_number(vertex[1]):
				vertices.append(Vector2(float(vertex[0]), float(vertex[1])))
	return vertices
