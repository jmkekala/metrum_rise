# SPDX-License-Identifier: GPL-2.0-only

## Deterministic editor-local hit collection. No selection, document or UI mutation.
## Meshes use cached native BVHs; site polygons and pixel handles use authored heights.
extends RefCounted

const ANCHOR_RADIUS := 12.0
const VERTEX_RADIUS := 8.0
const EDGE_RADIUS := 8.0
const REFERENCE_RADIUS := 10.0

var editor: Node
var queries := 0

func _init(owner: Node) -> void:
	editor = owner

func collect(mouse: Vector2, camera: Camera3D, filter: String) -> Array[Dictionary]:
	queries += 1
	var begin := camera.project_position(mouse, camera.near)
	var end := camera.project_position(mouse, camera.far)
	var hits: Array[Dictionary] = []
	if filter in ["mesh", "all"]:
		hits = editor._preview.pick_mesh_parts(begin, end)
	if filter in ["anchor", "all"]:
		for index in editor._site_anchors_data.size():
			var anchor: Dictionary = editor._site_anchors_data[index]
			var position := anchor_position(index)
			if not in_depth_range(camera, position):
				continue
			var point = plane_hit(begin, end, position.y)
			var inside := false
			if point != null:
				var forward: Vector3 = editor._anchor_forward(anchor)
				var side := Vector3(-forward.z, 0, forward.x)
				var rel: Vector3 = point - position
				var width: float = editor._anchor_number(anchor, "width_m", 2.0)
				var length := 0.0
				if anchor.get("anchor_type") in ["parking", "loading_bay"]:
					length = maxf(0.5, editor._anchor_number(anchor, "length_m", 0.0))
				elif anchor.get("anchor_type") == "driveway":
					length = maxf(1.5, width * 1.4)
				inside = length > 0 and absf(rel.dot(side)) <= width * 0.5 and rel.dot(forward) >= 0 and rel.dot(forward) <= length
			var label_hit := _label_contains("anchor", index, mouse, camera)
			if inside or label_hit or mouse.distance_to(camera.unproject_position(position)) <= ANCHOR_RADIUS:
				var hit_pos: Vector3 = point if inside else position
				var hit := _hit("anchor", index, hit_pos, begin)
				hit["label"] = label_hit
				hits.append(hit)
	if filter in ["surface", "all"]:
		for index in editor._site_surfaces_data.size():
			var point = plane_hit(begin, end, surface_height(index))
			var label_hit := _label_contains("surface", index, mouse, camera)
			if point != null and (label_hit or editor._site_surface_contains_world_xz(index, point)):
				var hit := _hit("surface", index, point, begin)
				hit["label"] = label_hit
				hits.append(hit)
		var selected: int = editor._selected_site_surface_index
		if selected >= 0:
			var points := surface_points(selected)
			for vertex in points.size():
				var point := points[vertex]
				if in_depth_range(camera, point) and mouse.distance_to(camera.unproject_position(point)) <= VERTEX_RADIUS:
					var hit := _hit("vertex", selected, point, begin)
					hit["vertex"] = vertex
					hit["screen_distance"] = mouse.distance_squared_to(camera.unproject_position(point))
					hits.append(hit)
	if filter == "all":
		if editor._preview.has_scale_reference():
			var centre: Vector3 = editor._preview.scale_reference_world_position()
			var reference: Dictionary = editor._preview.pick_scale_reference(begin, end)
			# The visible overlay handle remains reachable even when the small figure is occluded.
			var handle := in_depth_range(camera, centre) and mouse.distance_to(camera.unproject_position(centre)) <= REFERENCE_RADIUS
			if handle or not reference.is_empty():
				if handle:
					reference = _hit("reference", 0, centre, begin)
				reference.merge({"kind": "reference", "index": 0, "handle": handle})
				hits.append(reference)
		var ghost: Dictionary = editor._preview.pick_ghost(begin, end)
		if not ghost.is_empty():
			ghost.merge({"kind": "ghost", "index": 0})
			hits.append(ghost)
	hits.sort_custom(_before)
	return hits

func _hit(kind: String, index: int, position: Vector3, begin: Vector3) -> Dictionary:
	return {"kind": kind, "index": index, "position": position, "distance": begin.distance_to(position)}

func _before(a: Dictionary, b: Dictionary) -> bool:
	# Site guides/labels render without depth testing. Pick their visible layer before geometry.
	var a_layer := _pick_layer(a)
	var b_layer := _pick_layer(b)
	if a_layer != b_layer:
		return a_layer < b_layer
	if a["kind"] == "vertex" and a["screen_distance"] != b["screen_distance"]:
		return a["screen_distance"] < b["screen_distance"]
	if a["distance"] != b["distance"]:
		return a["distance"] < b["distance"]
	if a["kind"] != b["kind"]:
		return a["kind"] < b["kind"]
	if a["index"] != b["index"]:
		return a["index"] < b["index"]
	return a.get("vertex", -1) < b.get("vertex", -1)

func _pick_layer(hit: Dictionary) -> int:
	if hit.get("handle", false):
		return -1
	if hit["kind"] == "vertex":
		return 0
	if hit.get("label", false):
		return 1
	return 2 if hit["kind"] == "anchor" else 3

func _label_contains(kind: String, index: int, mouse: Vector2, camera: Camera3D) -> bool:
	var label: Label3D = editor._preview.site_label(kind, index)
	if label == null or not label.is_visible_in_tree() or not in_depth_range(camera, label.global_position):
		return false
	var bounds := label.get_aabb()
	if bounds.size.x <= 0 or bounds.size.y <= 0:
		return false
	# Billboard labels use camera right/up, not their unrotated Node3D basis.
	var basis := camera.get_camera_transform().basis
	var a := camera.unproject_position(label.global_position + basis.x * bounds.position.x + basis.y * bounds.position.y)
	var b := camera.unproject_position(label.global_position + basis.x * bounds.end.x + basis.y * bounds.end.y)
	return Rect2(a, b - a).abs().grow(2.0).has_point(mouse)

func surface_height(index: int) -> float:
	return editor._anchor_number(editor._site_surfaces_data[index], "y_m", 0.01) + editor._preview.SITE_SURFACE_FILL_Y

func surface_points(index: int) -> PackedVector3Array:
	var points := PackedVector3Array()
	var y := surface_height(index)
	for vertex in editor._site_surface_vertices(editor._site_surfaces_data[index]):
		points.append(Vector3(vertex.x, y, vertex.y))
	return points

func anchor_position(index: int) -> Vector3:
	return editor._anchor_position(editor._site_anchors_data[index]) + Vector3(0, 0.12, 0)

func surface_edge(mouse: Vector2, camera: Camera3D) -> Dictionary:
	var selected: int = editor._selected_site_surface_index
	if selected < 0:
		return {}
	var points := surface_points(selected)
	var closest := {}
	var distance := EDGE_RADIUS * EDGE_RADIUS
	for index in points.size():
		var next := (index + 1) % points.size()
		if not in_depth_range(camera, points[index]) or not in_depth_range(camera, points[next]):
			continue
		var a := camera.unproject_position(points[index])
		var b := camera.unproject_position(points[next])
		var screen_point := Geometry2D.get_closest_point_to_segment(mouse, a, b)
		var screen_distance := mouse.distance_squared_to(screen_point)
		if screen_distance <= distance:
			var point = plane_hit(camera.project_position(screen_point, camera.near), camera.project_position(screen_point, camera.far), points[index].y)
			if point == null:
				continue
			distance = screen_distance
			closest = {"surface": selected, "edge": index, "point": Vector2(point.x, point.z)}
	return closest

static func in_depth_range(camera: Camera3D, point: Vector3) -> bool:
	var depth := -(camera.get_camera_transform().affine_inverse() * point).z
	return depth >= camera.near and depth <= camera.far

static func plane_hit(begin: Vector3, end: Vector3, height: float):
	var direction := end - begin
	if absf(direction.y) < 0.000001:
		return null
	var t := (height - begin.y) / direction.y
	return begin + direction * t if t >= 0 and t <= 1 else null
