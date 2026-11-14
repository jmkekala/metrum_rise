# SPDX-License-Identifier: GPL-2.0-only

## The road tool's gizmos (`ROAD-112` rounds D and V): what `road_tool_frame` sends, drawn into
## one reused `ArrayMesh` over the terrain and the roads (no depth test), so the stroke's path
## stays visible where the preview terrain's grass covers the preview road. Up to five line
## surfaces of quads Rust built (`gizmo_stroke_*`, `gizmo_guide_*`, `gizmo_zoning_out_*`,
## `gizmo_zoning_in_*`, `gizmo_curb_*`; none for a style without quads), widened to a constant on-screen width by `road_tool_line.gdshader`: the
## stroke's centreline, solid and thick; every guide line in one look (alignment guides, the
## angle ray, tangent legs, control polygons and handle stems), dashed and thinner; the
## zoning-depth guide's outside (and the grid snap's curbs) solid and strong, its inside solid and
## faint (round V item 9); the grid snap's curbs solid in their own colour (item 8). Handles (`gizmo_handles`) draw as
## filled round points of a constant size, one surface per kind: a placed end, a control point,
## the chain's start, the live end (or the grabbed freeform grip), a joint and a handle. The
## palette is defined once (`PALETTE`), from the UI theme's colours (`ui_style.gd`). Rust
## decides every point; this script only uploads them, and only when they changed: in place
## while the surfaces' layout holds (Rust pads the quads to power-of-two counts), rebuilt when it
## changes.
extends MeshInstance3D

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const LineShader := preload("res://scripts/shaders/road_tool_line.gdshader")
const HandleShader := preload("res://scripts/shaders/road_tool_handle.gdshader")

## The gizmos' one palette: the stroke's centreline, every guide line (one colour, width and
## dash), and the handles by kind (placed end, control point, chain start, live end, joint,
## handle).
const PALETTE := {
	"stroke": Color(1.0, 1.0, 1.0, 0.82),
	"stroke_width_px": 4.0,
	"guide": Color(0.72, 0.74, 1.0, 0.72),
	"guide_width_px": 2.0,
	"guide_dash_px": 9.0,
	"guide_gap_px": 6.0,
	# The zoning-depth guide (round V item 9): its outside (and the grid curbs) solid and strong,
	# its inside solid and faint.
	"zoning_out": Color(0.72, 0.74, 1.0, 0.9),
	"zoning_out_width_px": 3.0,
	"zoning_in": Color(0.72, 0.74, 1.0, 0.4),
	"zoning_in_width_px": 2.0,
	# The grid snap's curbs (item 8): solid, in a colour that stands out on the preview road's
	# light edges.
	"curb": Color(0.35, 0.82, 1.0, 0.95),
	"curb_width_px": 3.0,
	"handles": [
		Color(1.0, 1.0, 1.0, 0.95),
		Color(0.65, 0.65, 0.90, 0.95),
		Color(0.35, 0.95, 0.55, 0.95),
		Color(1.0, 1.0, 1.0, 0.95),
		Color(0.65, 0.65, 0.90, 0.95),
		Color(0.82, 0.82, 0.98, 0.95),
	],
	"handle_sizes_px": [14.0, 13.0, 14.0, 16.0, 12.0, 10.0],
}
# The line surfaces' frame keys, in surface order: the stroke, the guides, the zoning guide's
# outside and inside, the grid curbs. A style without quads has no surface.
const LINE_KEYS := ["gizmo_stroke", "gizmo_guide", "gizmo_zoning_out", "gizmo_zoning_in", "gizmo_curb"]

var _mesh := ArrayMesh.new()
var _line_materials: Array[ShaderMaterial] = []
var _handle_materials: Array[ShaderMaterial] = []
# The arrays last uploaded: an unchanged frame uploads nothing.
var _line_points := PackedVector3Array()
var _line_classes := PackedInt32Array()
var _handles := PackedVector3Array()
var _handle_kinds := PackedInt32Array()
# The surfaces' layout (each line surface's vertex count, each handle kind's count): while it
# holds, a new pose rewrites the buffers in place instead of rebuilding the surfaces.
var _layout := PackedInt32Array()
# Each handle kind's points of this frame.
var _kind_points: Array[PackedVector3Array] = []
# Quad index buffers by vertex count (two triangles a quad), made once per count.
var _quad_indices := {}

func _init() -> void:
	mesh = _mesh
	# Vertices move in place without the mesh's bounds following: never cull the gizmos.
	custom_aabb = AABB(Vector3(-1.0e6, -1.0e4, -1.0e6), Vector3(2.0e6, 2.0e4, 2.0e6))
	SceneLightingConfig.apply_shadow_policy(self, SceneLightingConfig.SHADOW_DEBUG_OVERLAY, "roads")
	_line_materials.append(_line_material(PALETTE["stroke"], PALETTE["stroke_width_px"], 0.0, 0.0))
	_line_materials.append(_line_material(PALETTE["guide"], PALETTE["guide_width_px"], PALETTE["guide_dash_px"], PALETTE["guide_gap_px"]))
	_line_materials.append(_line_material(PALETTE["zoning_out"], PALETTE["zoning_out_width_px"], 0.0, 0.0))
	_line_materials.append(_line_material(PALETTE["zoning_in"], PALETTE["zoning_in_width_px"], 0.0, 0.0))
	_line_materials.append(_line_material(PALETTE["curb"], PALETTE["curb_width_px"], 0.0, 0.0))
	var colors: Array = PALETTE["handles"]
	var sizes: Array = PALETTE["handle_sizes_px"]
	for kind in colors.size():
		_kind_points.append(PackedVector3Array())
		var material := ShaderMaterial.new()
		material.shader = HandleShader
		material.set_shader_parameter("color", colors[kind])
		material.set_shader_parameter("size_px", sizes[kind])
		material.render_priority = 11
		_handle_materials.append(material)

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
	# The dashes keep their length on screen at the cursor's depth.
	_line_materials[1].set_shader_parameter("dash_focus", frame.get("cursor", Vector3.ZERO))
	# One point surface per handle kind (a freeform draft has up to 16 nodes and 30 handles).
	for kind in _kind_points.size():
		_kind_points[kind] = PackedVector3Array()
	for index in handles.size():
		_kind_points[handle_kinds[index]].append(handles[index])
	var layout := PackedInt32Array()
	var has_lines := not line_points.is_empty()
	for key in LINE_KEYS:
		var vertices: PackedVector3Array = frame.get(key + "_vertices", PackedVector3Array())
		layout.append(vertices.size() if has_lines else 0)
	for kind in _kind_points.size():
		layout.append(_kind_points[kind].size())
	if layout == _layout and _mesh.get_surface_count() > 0:
		_update_in_place(frame, has_lines)
	else:
		_layout = layout
		_rebuild(frame, has_lines)
	visible = _mesh.get_surface_count() > 0
	return float(Time.get_ticks_usec() - start_us) / 1000.0

# New surfaces for a new layout: the two line surfaces (when there are lines), one per handle
# kind with points.
func _rebuild(frame: Dictionary, has_lines: bool) -> void:
	_mesh.clear_surfaces()
	if has_lines:
		for style in LINE_KEYS.size():
			var vertices: PackedVector3Array = frame.get(LINE_KEYS[style] + "_vertices", PackedVector3Array())
			var attributes: PackedFloat32Array = frame.get(LINE_KEYS[style] + "_attributes", PackedFloat32Array())
			var count := vertices.size()
			if count == 0:
				continue
			var arrays := []
			arrays.resize(Mesh.ARRAY_MAX)
			arrays[Mesh.ARRAY_VERTEX] = vertices
			# Placeholders of the right size: the interleaved attributes go in below in one write.
			var uv := PackedVector2Array()
			uv.resize(count)
			arrays[Mesh.ARRAY_TEX_UV] = uv
			arrays[Mesh.ARRAY_TEX_UV2] = uv
			arrays[Mesh.ARRAY_INDEX] = _indices(count)
			_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
			var surface := _mesh.get_surface_count() - 1
			_mesh.surface_set_material(surface, _line_materials[style])
			RenderingServer.mesh_surface_update_attribute_region(_mesh.get_rid(), surface, 0, attributes.to_byte_array())
	for kind in _kind_points.size():
		if not _kind_points[kind].is_empty():
			var arrays := []
			arrays.resize(Mesh.ARRAY_MAX)
			arrays[Mesh.ARRAY_VERTEX] = _kind_points[kind]
			_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_POINTS, arrays)
			_mesh.surface_set_material(_mesh.get_surface_count() - 1, _handle_materials[kind])

# The same layout: each surface's positions (12 bytes a vertex) and, for the lines, the
# interleaved attributes (16 bytes a vertex) rewritten in their existing buffers, in the order
# `_rebuild` made the surfaces.
func _update_in_place(frame: Dictionary, has_lines: bool) -> void:
	var rid := _mesh.get_rid()
	var surface := 0
	if has_lines:
		for key in LINE_KEYS:
			var vertices: PackedVector3Array = frame.get(key + "_vertices", PackedVector3Array())
			var attributes: PackedFloat32Array = frame.get(key + "_attributes", PackedFloat32Array())
			if vertices.is_empty():
				continue
			RenderingServer.mesh_surface_update_vertex_region(rid, surface, 0, vertices.to_byte_array())
			RenderingServer.mesh_surface_update_attribute_region(rid, surface, 0, attributes.to_byte_array())
			surface += 1
	for kind in _kind_points.size():
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

# Two triangles per quad of four vertices (`a+, a-, b+, b-`), made once per vertex count.
func _indices(count: int) -> PackedInt32Array:
	if _quad_indices.has(count):
		return _quad_indices[count]
	var indices := PackedInt32Array()
	indices.resize(count / 4 * 6)
	for quad in count / 4:
		var base := quad * 4
		var at := quad * 6
		indices[at] = base
		indices[at + 1] = base + 1
		indices[at + 2] = base + 2
		indices[at + 3] = base + 2
		indices[at + 4] = base + 1
		indices[at + 5] = base + 3
	_quad_indices[count] = indices
	return indices

static func _line_material(color: Color, width_px: float, dash_px: float, gap_px: float) -> ShaderMaterial:
	var material := ShaderMaterial.new()
	material.shader = LineShader
	material.set_shader_parameter("color", color)
	material.set_shader_parameter("width_px", width_px)
	material.set_shader_parameter("dash_px", dash_px)
	material.set_shader_parameter("gap_px", gap_px)
	material.render_priority = 10
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
