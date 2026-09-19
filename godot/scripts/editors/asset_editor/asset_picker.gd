# SPDX-License-Identifier: GPL-2.0-only

## Deterministic editor-local hit collection. No selection, document or UI mutation.
## Meshes use cached native BVHs; site polygons and pixel handles use authored heights.
extends RefCounted

const ANCHOR_RADIUS := 12.0
const VERTEX_RADIUS := 8.0
const EDGE_RADIUS := 8.0
const REFERENCE_RADIUS := 10.0
const DEPTH_EPSILON_M := 0.01

var editor: Node
var queries := 0
var visibility_queries := 0
var _point_visibility: Dictionary = {}
var _visibility_revision := -1
var _visibility_transform := Transform3D.IDENTITY
var _visibility_projection := Projection()

## Cache fixed handle visibility until camera/projection or preview geometry changes.
func point_visible(camera: Camera3D, point: Vector3) -> bool:
	if not in_depth_range(camera, point):
		return false
	var transform := camera.get_camera_transform()
	var projection := camera.get_camera_projection()
	var revision: int = editor._preview.visibility_revision
	if revision != _visibility_revision or transform != _visibility_transform or projection != _visibility_projection:
		_point_visibility.clear()
		_visibility_revision = revision
		_visibility_transform = transform
		_visibility_projection = projection
	if not _point_visibility.has(point):
		visibility_queries += 1
		var begin := camera.project_position(camera.unproject_position(point), camera.near)
		var end := point - begin.direction_to(point) * DEPTH_EPSILON_M
		_point_visibility[point] = editor._preview.pick_mesh_parts(begin, end).is_empty()
	return _point_visibility[point]

func _init(owner: Node) -> void:
	editor = owner

func collect(mouse: Vector2, camera: Camera3D, filter: String) -> Array[Dictionary]:
	queries += 1
	var begin := camera.project_position(mouse, camera.near)
	var end := camera.project_position(mouse, camera.far)
	# Mesh depth is needed even when a selection filter excludes meshes.
	var mesh_hits: Array[Dictionary] = editor._preview.pick_mesh_parts(begin, end)
	var mesh_distance := INF
	for hit in mesh_hits:
		mesh_distance = minf(mesh_distance, hit["distance"])
	var hits: Array[Dictionary] = []
	if filter in ["mesh", "all"]:
		hits = mesh_hits
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
				var hit_pos: Vector3 = point if inside else _billboard_point(mouse, camera, position)
				if label_hit:
					hit_pos = _billboard_point(mouse, camera, editor._preview.site_label("anchor", index).global_position)
				var hit := _hit("anchor", index, hit_pos, begin)
				hit["label"] = label_hit
				if not inside and not label_hit:
					hit["occluded"] = not point_visible(camera, position)
				hits.append(hit)
	if filter in ["surface", "all"]:
		for index in editor._site_surfaces_data.size():
			var point = plane_hit(begin, end, surface_height(index))
			var label_hit := _label_contains("surface", index, mouse, camera)
			if label_hit:
				point = _billboard_point(mouse, camera, editor._preview.site_label("surface", index).global_position)
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
					hit["occluded"] = not point_visible(camera, point)
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
	for hit in hits:
		# The reference's deliberate manipulation handle is the only x-ray exception.
		if not hit.get("handle", false):
			hit["occluded"] = hit.get("occluded", false) or hit["distance"] > mesh_distance + DEPTH_EPSILON_M
	hits.sort_custom(_before)
	return hits

func _hit(kind: String, index: int, position: Vector3, begin: Vector3) -> Dictionary:
	return {"kind": kind, "index": index, "position": position, "distance": begin.distance_to(position)}

func _before(a: Dictionary, b: Dictionary) -> bool:
	# Hidden targets remain available to Alt+click but never outrank visible geometry.
	if a.get("occluded", false) != b.get("occluded", false):
		return not a.get("occluded", false)
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

func _billboard_point(mouse: Vector2, camera: Camera3D, position: Vector3) -> Vector3:
	return camera.project_position(mouse, -(camera.get_camera_transform().affine_inverse() * position).z)

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
			var begin := camera.project_position(screen_point, camera.near)
			if not editor._preview.pick_mesh_parts(begin, point - begin.direction_to(point) * DEPTH_EPSILON_M).is_empty():
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
