# SPDX-License-Identifier: GPL-2.0-only

## The road tool's gizmos (`ROAD-112` round D): what `road_tool_frame` sends as packed arrays,
## drawn into one reused `ArrayMesh`. Line strips (`gizmo_line_points`, each strip starting at
## its `gizmo_line_starts` entry) draw one surface each in their class's colour: the draft's
## centreline, a guide line, the angle ray, a curve's tangent legs (and round N's S-curve control
## polygon and freeform handle stems). Handles (`gizmo_handles`) draw as screen-sized points, one
## surface per kind: a placed end, a control point, the chain's start, the live end (or the
## grabbed freeform grip), a joint (the S-curve's, a freeform inner node) and a handle (an
## S-curve arc's control point, a freeform Bezier handle). Plain defaults: unshaded, drawn over the road, colours from the tool's
## existing palette (white, the HUD's amber, the cursor's green, the node markers' blue). Rust
## decides every point; this script only uploads them, and only when they changed: in place
## while the surfaces' layout holds (a drag), rebuilt when it changes.
extends MeshInstance3D

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
# By line class: draft centreline, guide line, angle ray, tangent leg.
const LINE_COLORS := [
	Color(1.0, 1.0, 1.0, 0.9),
	Color(0.26, 0.92, 0.95, 0.85),
	Color(0.3, 0.8, 1.0, 0.85),
	Color(1.0, 0.76, 0.18, 0.95),
]
# By handle kind: placed end, control point, chain start, live end, joint, handle.
const HANDLE_COLORS := [
	Color(1.0, 1.0, 1.0, 0.95),
	Color(1.0, 0.76, 0.18, 0.95),
	Color(0.0, 1.0, 0.5, 0.95),
	Color(0.3, 0.8, 1.0, 0.95),
	Color(1.0, 0.42, 0.82, 0.95),
	Color(1.0, 0.9, 0.55, 0.95),
]
const HANDLE_SIZE_PX := 10.0

var _mesh := ArrayMesh.new()
var _line_materials: Array[StandardMaterial3D] = []
var _handle_materials: Array[StandardMaterial3D] = []
# The arrays last uploaded: an unchanged frame uploads nothing.
var _line_points := PackedVector3Array()
var _handles := PackedVector3Array()
var _handle_kinds := PackedInt32Array()
var _line_classes := PackedInt32Array()
# The surfaces' layout (each strip's start and class, each handle kind's count): while it holds,
# a new pose rewrites the vertex buffers in place instead of rebuilding the surfaces.
var _layout := PackedInt32Array()
# Each handle kind's points of this frame.
var _kind_points: Array[PackedVector3Array] = []

func _init() -> void:
	mesh = _mesh
	# Vertices move in place without the mesh's bounds following: never cull the gizmos.
	custom_aabb = AABB(Vector3(-1.0e6, -1.0e4, -1.0e6), Vector3(2.0e6, 2.0e4, 2.0e6))
	for kind in HANDLE_COLORS.size():
		_kind_points.append(PackedVector3Array())
	SceneLightingConfig.apply_shadow_policy(self, SceneLightingConfig.SHADOW_DEBUG_OVERLAY, "roads")
	for color in LINE_COLORS:
		_line_materials.append(_material(color, false))
	for color in HANDLE_COLORS:
		_handle_materials.append(_material(color, true))

## Uploads the frame's gizmos when they differ from the last ones; returns the milliseconds the
## upload took (0 when nothing changed).
func show_frame(frame: Dictionary) -> float:
	var line_points: PackedVector3Array = frame.get("gizmo_line_points", PackedVector3Array())
	var line_classes: PackedInt32Array = frame.get("gizmo_line_classes", PackedInt32Array())
	var handles: PackedVector3Array = frame.get("gizmo_handles", PackedVector3Array())
	var handle_kinds: PackedInt32Array = frame.get("gizmo_handle_kinds", PackedInt32Array())
	if line_points == _line_points and handles == _handles and line_classes == _line_classes and handle_kinds == _handle_kinds:
		return 0.0
	var start_us := Time.get_ticks_usec()
	_line_points = line_points
	_line_classes = line_classes
	_handles = handles
	_handle_kinds = handle_kinds
	var starts: PackedInt32Array = frame.get("gizmo_line_starts", PackedInt32Array())
	# One point surface per handle kind (a freeform draft has up to 16 nodes and 30 handles).
	for kind in HANDLE_COLORS.size():
		var points := PackedVector3Array()
		for index in handles.size():
			if handle_kinds[index] == kind:
				points.append(handles[index])
		_kind_points[kind] = points
	var layout := starts.duplicate()
	layout.append(line_points.size())
	layout.append_array(line_classes)
	for kind in HANDLE_COLORS.size():
		layout.append(_kind_points[kind].size())
	if layout == _layout and _mesh.get_surface_count() > 0:
		_update_in_place(line_points, starts)
	else:
		_layout = layout
		_rebuild(line_points, starts, line_classes)
	visible = _mesh.get_surface_count() > 0
	return float(Time.get_ticks_usec() - start_us) / 1000.0

# New surfaces for a new layout: one per strip of two or more points, one per handle kind.
func _rebuild(line_points: PackedVector3Array, starts: PackedInt32Array, line_classes: PackedInt32Array) -> void:
	_mesh.clear_surfaces()
	for strip in starts.size():
		var end := starts[strip + 1] if strip + 1 < starts.size() else line_points.size()
		if end - starts[strip] >= 2:
			_add_surface(Mesh.PRIMITIVE_LINE_STRIP, line_points.slice(starts[strip], end), _line_materials[line_classes[strip]])
	for kind in HANDLE_COLORS.size():
		if not _kind_points[kind].is_empty():
			_add_surface(Mesh.PRIMITIVE_POINTS, _kind_points[kind], _handle_materials[kind])

# The same layout: each surface's positions (12 bytes a vertex, the only attribute) rewritten in
# its existing vertex buffer, in the order `_rebuild` made the surfaces.
func _update_in_place(line_points: PackedVector3Array, starts: PackedInt32Array) -> void:
	var rid := _mesh.get_rid()
	var surface := 0
	for strip in starts.size():
		var end := starts[strip + 1] if strip + 1 < starts.size() else line_points.size()
		if end - starts[strip] >= 2:
			RenderingServer.mesh_surface_update_vertex_region(rid, surface, 0, line_points.slice(starts[strip], end).to_byte_array())
			surface += 1
	for kind in HANDLE_COLORS.size():
		if not _kind_points[kind].is_empty():
			RenderingServer.mesh_surface_update_vertex_region(rid, surface, 0, _kind_points[kind].to_byte_array())
			surface += 1

## Removes every gizmo.
func clear() -> void:
	if _mesh.get_surface_count() > 0:
		_mesh.clear_surfaces()
	_line_points = PackedVector3Array()
	_handles = PackedVector3Array()
	_layout = PackedInt32Array()
	visible = false

func _add_surface(primitive: int, points: PackedVector3Array, material: StandardMaterial3D) -> void:
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = points
	_mesh.add_surface_from_arrays(primitive, arrays)
	_mesh.surface_set_material(_mesh.get_surface_count() - 1, material)

# Unshaded, over the road and the terrain (an overlay, not a lit world material).
static func _material(color: Color, points: bool) -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.no_depth_test = true
	material.render_priority = 10
	if points:
		material.use_point_size = true
		material.point_size = HANDLE_SIZE_PX
	return material

# Batch preview ghost (`ROAD-112` round G1): tier 1's ribbons of a draft's strokes, drawn by its
# own child mesh so the gizmos' in-place layout above never sees it. Valid strokes in the
# centreline's white, refused ones in the HUD's red, both translucent; the batch's exact preview
# replaces it through the road preview display once tier 2 accepts.
const GHOST_COLORS := [Color(1.0, 1.0, 1.0, 0.35), Color(1.0, 0.3, 0.25, 0.45)]

var _ghost: MeshInstance3D
var _ghost_materials: Array[StandardMaterial3D] = []

## Draws tier 1's ghost of a batch pose (`preview_road_draft`'s `ghost`, `ghost_starts` and
## `valid`): one triangle-list surface per verdict. Returns the upload's milliseconds.
func show_batch_ghost(result: Dictionary) -> float:
	var start_us := Time.get_ticks_usec()
	if _ghost == null:
		_ghost = MeshInstance3D.new()
		_ghost.mesh = ArrayMesh.new()
		_ghost.custom_aabb = custom_aabb
		SceneLightingConfig.apply_shadow_policy(_ghost, SceneLightingConfig.SHADOW_DEBUG_OVERLAY, "roads")
		for color in GHOST_COLORS:
			var material := _material(color, false)
			material.cull_mode = BaseMaterial3D.CULL_DISABLED
			_ghost_materials.append(material)
		add_child(_ghost)
	var ghost: PackedVector3Array = result.get("ghost", PackedVector3Array())
	var starts: PackedInt32Array = result.get("ghost_starts", PackedInt32Array())
	var valid: PackedInt32Array = result.get("valid", PackedInt32Array())
	# Each stroke's vertices run from its start to the next stroke's (or the end).
	var by_verdict: Array[PackedVector3Array] = [PackedVector3Array(), PackedVector3Array()]
	for stroke in starts.size():
		var end := starts[stroke + 1] if stroke + 1 < starts.size() else ghost.size()
		var refused := 0 if stroke < valid.size() and valid[stroke] != 0 else 1
		by_verdict[refused].append_array(ghost.slice(starts[stroke], end))
	var mesh: ArrayMesh = _ghost.mesh
	mesh.clear_surfaces()
	for verdict in by_verdict.size():
		if by_verdict[verdict].is_empty():
			continue
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = by_verdict[verdict]
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		mesh.surface_set_material(mesh.get_surface_count() - 1, _ghost_materials[verdict])
	_ghost.visible = mesh.get_surface_count() > 0
	return float(Time.get_ticks_usec() - start_us) / 1000.0

## Removes the batch ghost.
func clear_batch_ghost() -> void:
	if _ghost != null:
		(_ghost.mesh as ArrayMesh).clear_surfaces()
		_ghost.visible = false

## The batch ghost's surface count (tests).
func batch_ghost_surfaces() -> int:
	return 0 if _ghost == null else (_ghost.mesh as ArrayMesh).get_surface_count()
