# SPDX-License-Identifier: GPL-3.0-or-later

## Deterministic editor-local hit collection. No selection, document or UI mutation.
## Meshes use cached native BVHs; site polygons and pixel handles use authored heights. The
## selected loading bay's authored waypoints are pixel handles too; the derived points of its
## proven ways (gate connection, approach, stop) are not.
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
			var area := 0.0
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
				area = width * length
				inside = length > 0 and absf(rel.dot(side)) <= width * 0.5 and rel.dot(forward) >= 0 and rel.dot(forward) <= length
			var near := mouse.distance_to(camera.unproject_position(position)) <= ANCHOR_RADIUS
			# A name only selects where no shape or handle is under the cursor (see _pick_layer).
			var label_hit := not inside and not near and _label_contains("anchor", index, mouse, camera)
			if inside or near or label_hit:
				var hit_pos: Vector3 = point if inside else _billboard_point(mouse, camera, position)
				if label_hit:
					hit_pos = _billboard_point(mouse, camera, editor._preview.site_label("anchor", index).global_position)
				var hit := _hit("anchor", index, hit_pos, begin)
				hit["label"] = label_hit
				hit["near"] = near
				if inside:
					hit["area"] = area
				if near:
					hit["occluded"] = not point_visible(camera, position)
				hits.append(hit)
		for handle in waypoints():
			var point: Vector3 = handle["position"]
			if in_depth_range(camera, point) and mouse.distance_to(camera.unproject_position(point)) <= VERTEX_RADIUS:
				var hit := _hit("waypoint", editor._selected_site_anchor_index, point, begin)
				hit["occluded"] = not point_visible(camera, point)
				hit["way"] = handle["way"]
				hit["waypoint"] = handle["waypoint"]
				hit["screen_distance"] = mouse.distance_squared_to(camera.unproject_position(point))
				hits.append(hit)
	if filter in ["surface", "all"]:
		for index in editor._site_surfaces_data.size():
			var point = plane_hit(begin, end, surface_height(index))
			var inside: bool = point != null and editor._site_surface_contains_world_xz(index, point)
			var label_hit := not inside and _label_contains("surface", index, mouse, camera)
			if label_hit:
				point = _billboard_point(mouse, camera, editor._preview.site_label("surface", index).global_position)
			if inside or label_hit:
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
	# Handles of one layer: the nearest on screen.
	if a.has("screen_distance") and b.has("screen_distance") and a["screen_distance"] != b["screen_distance"]:
		return a["screen_distance"] < b["screen_distance"]
	# Overlapping spots: the smaller footprint is the one the cursor most plausibly means.
	if a.get("area", 0.0) != b.get("area", 0.0):
		return a.get("area", 0.0) < b.get("area", 0.0)
	if a["distance"] != b["distance"]:
		return a["distance"] < b["distance"]
	if a["kind"] != b["kind"]:
		return a["kind"] < b["kind"]
	if a["index"] != b["index"]:
		return a["index"] < b["index"]
	if a.get("way", "") != b.get("way", ""):
		return a.get("way", "") < b.get("way", "")
	return a.get("vertex", a.get("waypoint", -1)) < b.get("vertex", b.get("waypoint", -1))

# Precise targets first: handles and vertices, an anchor's own circle, then anchor shapes.
# Floating names come next: they are wide and can cover another access point, so a name never
# wins over an access point the cursor is actually on, but it still beats the yard and the
# meshes behind it.
func _pick_layer(hit: Dictionary) -> int:
	if hit.get("handle", false):
		return -1
	if hit["kind"] in ["vertex", "waypoint"]:
		return 0
	if hit.get("near", false):
		return 1
	if hit["kind"] == "anchor" and not hit.get("label", false):
		return 2
	return 3 if hit.get("label", false) else 4

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

## Whether `hit` is a handle or an access point's own circle, which a pixel-sized line target such
## as a route leg never outranks.
func precise(hit: Dictionary) -> bool:
	return _pick_layer(hit) < 2

## The selected loading bay's authored waypoints, or a parking spot's own route's (`PARK-26`), as
## {"way", "waypoint", "position"} at the height the preview draws their markers; empty for any
## other selection.
func waypoints() -> Array[Dictionary]:
	var result: Array[Dictionary] = []
	var bay := _selected_bay()
	if bay.is_empty():
		return result
	var y: float = editor._anchor_position(bay).y + editor._preview.WAYPOINT_Y_M
	for way in ["in", "out"]:
		var points: Array = bay.get("route_" + way, [])
		for index in points.size():
			result.append({"way": way, "waypoint": index, "position": Vector3(float(points[index][0]), y, float(points[index][1]))})
	return result

## The leg of the selected loading bay's proven way within a few pixels of `mouse`, as {"way",
## "waypoint": the index a waypoint inserted there takes, "point": the leg's ground point under
## the cursor}, or {}. A proven way runs connection, approach, waypoints, stop (in) or stop,
## waypoints, approach, connection (out); the gate's own connection-approach stretch takes none.
## A parking spot's own route (`PARK-26`) runs connection, waypoints, stop: every leg takes one.
func route_leg(mouse: Vector2, camera: Camera3D) -> Dictionary:
	var bay := _selected_bay()
	if bay.is_empty():
		return {}
	var car := str(bay.get("anchor_type", "")) == "parking"
	if car and bay.get("route_in", []).is_empty():
		return {}
	var drawn: Dictionary = {"route_in": editor._preview.car_way(editor._selected_site_anchor_index)} if car \
		else editor._preview.dock_way(editor._selected_site_anchor_index)
	var y: float = editor._anchor_position(bay).y + editor._preview.DOCK_ROUTE_Y_M
	var closest := {}
	var distance := EDGE_RADIUS * EDGE_RADIUS
	for way in ["in", "out"]:
		var points: Array = drawn.get("route_" + way, [])
		var authored: int = bay.get("route_" + way, []).size()
		for leg in points.size() - 1:
			if not car and leg == (0 if way == "in" else points.size() - 2):
				continue
			var a := Vector3(float(points[leg][0]), y, float(points[leg][1]))
			var b := Vector3(float(points[leg + 1][0]), y, float(points[leg + 1][1]))
			if not in_depth_range(camera, a) or not in_depth_range(camera, b):
				continue
			var screen_point := Geometry2D.get_closest_point_to_segment(mouse, camera.unproject_position(a), camera.unproject_position(b))
			var screen_distance := mouse.distance_squared_to(screen_point)
			if screen_distance > distance:
				continue
			var begin := camera.project_position(screen_point, camera.near)
			var point = plane_hit(begin, camera.project_position(screen_point, camera.far), y)
			if point == null or not editor._preview.pick_mesh_parts(begin, point - begin.direction_to(point) * DEPTH_EPSILON_M).is_empty():
				continue
			distance = screen_distance
			var at: int = leg if car else (leg - 1 if way == "in" else leg)
			closest = {"way": way, "waypoint": clampi(at, 0, authored), "point": Vector2(point.x, point.z)}
	return closest

func _selected_bay() -> Dictionary:
	var index: int = editor._selected_site_anchor_index
	if index < 0 or index >= editor._site_anchors_data.size():
		return {}
	var anchor: Dictionary = editor._site_anchors_data[index]
	return anchor if anchor.get("anchor_type") in ["loading_bay", "parking"] else {}

static func in_depth_range(camera: Camera3D, point: Vector3) -> bool:
	var depth := -(camera.get_camera_transform().affine_inverse() * point).z
	return depth >= camera.near and depth <= camera.far

static func plane_hit(begin: Vector3, end: Vector3, height: float):
	var direction := end - begin
	if absf(direction.y) < 0.000001:
		return null
	var t := (height - begin.y) / direction.y
	return begin + direction * t if t >= 0 and t <= 1 else null
