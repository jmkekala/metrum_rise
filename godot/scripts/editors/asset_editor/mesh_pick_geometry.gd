# SPDX-License-Identifier: GPL-2.0-only

## Cached native triangle BVHs for static imported preview geometry, without physics bodies.
## Build on import, query on pointer/camera changes; transforms never rebuild triangle data.
extends RefCounted

class Proxy:
	var node: MeshInstance3D
	var triangles: TriangleMesh
	var bounds: AABB

var proxies: Array[Proxy] = []

func build(root: Node) -> void:
	if root is MeshInstance3D and root.mesh != null:
		var triangles: TriangleMesh = root.mesh.generate_triangle_mesh()
		if triangles != null:
			var proxy := Proxy.new()
			proxy.node = root
			proxy.triangles = triangles
			proxy.bounds = root.mesh.get_aabb()
			proxies.append(proxy)
	for child in root.get_children():
		build(child)

func intersect(begin: Vector3, end: Vector3) -> Dictionary:
	var closest := {}
	var distance := INF
	for proxy in proxies:
		if not proxy.node.is_visible_in_tree():
			continue
		var transform := proxy.node.global_transform
		if is_zero_approx(transform.basis.determinant()):
			continue
		var inverse := transform.affine_inverse()
		var local_begin := inverse * begin
		var local_end := inverse * end
		if proxy.bounds.intersects_segment(local_begin, local_end) == null:
			continue
		var hit := proxy.triangles.intersect_segment(local_begin, local_end)
		if hit.is_empty():
			continue
		var position: Vector3 = transform * hit["position"]
		var depth := begin.distance_squared_to(position)
		if depth < distance:
			distance = depth
			closest = {"position": position, "distance": sqrt(depth)}
	return closest
