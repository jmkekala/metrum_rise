# SPDX-License-Identifier: GPL-2.0-only

## The road tool's gizmos (`ROAD-112` rounds D and V): what Rust's tick built, drawn into
## one reused `ArrayMesh` over the terrain and the roads (no depth test), so the stroke's path
## stays visible where the preview terrain's grass covers the preview road. One line surface of
## quads Rust built (`gizmo_line_*`, each vertex carrying its look, round V2), widened to a
## constant on-screen width by `road_tool_line.gdshader`: the
## stroke's centreline, solid and thick; the alignment guide that took the cursor and the
## angle ray dashed and strong, a guide near the cursor that has not taken it dashed and faint
## (round UI1); tangent legs, control polygons and handle stems in one guide look, dashed and
## thinner; an end's corner arc solid (round UI1; at a right angle its square, round UI2); the
## zoning-depth guide's outside (and the grid snap's curbs) solid and strong, its inside solid and
## faint (round V item 9); the grid snap's curbs solid in their own colour (item 8). Handles (`gizmo_handles`) draw as
## filled round points of a constant size, all in one surface whose per-point kind (round V2)
## picks the colour and size: a placed end, a control point, the chain's start, the live end (or
## the grabbed freeform grip), a joint and a handle (a height grip too). The
## palette is defined once (`PALETTE`), from the UI theme's colours (`ui_style.gd`). Rust
## decides every point and builds the buffers in the server's layout; this script only uploads
## them, and only on the tick's gizmo flag (round R2-2: no compare here): in place while the
## surfaces' layout holds (Rust pads the quads to power-of-two counts), the surfaces rebuilt with
## placeholders of the new layout when it changes. The batch ghost (rounds G1 and V2) draws in the same palette and line look.
extends MeshInstance3D

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const LineShader := preload("res://scripts/shaders/road_tool_line.gdshader")
const LineDepthShader := preload("res://scripts/shaders/road_tool_line_depth.gdshader")
const HandleShader := preload("res://scripts/shaders/road_tool_handle.gdshader")

## The gizmos' one palette: the line looks in the order Rust numbers them (`LineStyle`: the
## stroke's centreline; the guide look of tangent legs, control polygons and stems; the zoning
## guide's outside and inside; the grid curbs; the batch ghost's edges, accepted and refused; an
## end's ground marker; the footprint hue's tunnel outlines by state; the alignment guide that
## took the cursor, a faint guide near it, an end's corner arc), the footprint hue, the batch
## ghost's fills, and the handles by kind (placed end, control point, chain start, live end,
## joint, handle).
const PALETTE := {
	# Colour, width in pixels, dash and gap in pixels (0 dash: solid) per line look.
	"lines": [
		[Color(1.0, 1.0, 1.0, 0.82), 4.0, 0.0, 0.0],
		[Color(0.72, 0.74, 1.0, 0.72), 2.0, 9.0, 6.0],
		# The zoning-depth guide (round V item 9): its outside solid and strong, its inside solid
		# and fainter. Round UI1: thicker and more opaque, so both read on grass.
		[Color(0.72, 0.74, 1.0, 1.0), 5.0, 0.0, 0.0],
		[Color(0.72, 0.74, 1.0, 0.65), 3.0, 0.0, 0.0],
		# The grid snap's curbs (item 8): solid, in a colour that stands out on the preview road's
		# light edges.
		[Color(0.35, 0.82, 1.0, 0.95), 3.0, 0.0, 0.0],
		# The batch ghost's edges (round V2): the stroke's white when tier 1 accepts the stroke,
		# the HUD's refusal red when not.
		[Color(1.0, 1.0, 1.0, 0.82), 2.0, 0.0, 0.0],
		[Color(1.0, 0.40, 0.30, 0.95), 2.0, 0.0, 0.0],
		# An end's ground marker (round H2): its vertical line to the ground and its ring there,
		# solid, in the stroke's white.
		[Color(1.0, 1.0, 1.0, 0.9), 2.0, 0.0, 0.0],
		# The footprint hue's tunnel outlines (round HU), solid and depth-tested on the ground
		# over the bore, in the hue's colour by state: valid, checking, refused.
		[Color(0.35, 0.65, 1.0, 0.85), 2.0, 0.0, 0.0],
		[Color(1.0, 0.76, 0.18, 0.85), 2.0, 0.0, 0.0],
		[Color(1.0, 0.40, 0.30, 0.9), 2.0, 0.0, 0.0],
		# Round UI1: the alignment guide that took the cursor and the angle lock's ray, thicker and
		# more opaque than the shared guide look; a guide near the cursor that has not taken it,
		# fainter than that look; an end's corner arc or square (UI2), solid in the stroke's white.
		[Color(0.72, 0.74, 1.0, 0.95), 3.5, 12.0, 5.0],
		[Color(0.72, 0.74, 1.0, 0.45), 2.0, 9.0, 6.0],
		[Color(1.0, 1.0, 1.0, 0.9), 2.5, 0.0, 0.0],
	],
	# The footprint hue's fill (round HU, `road_tool_hue.gd`) by state: valid blue, checking the
	# chips' warning amber, refused the HUD's refusal red (also hatched); one alpha, depth-tested.
	"hue": [Color(0.35, 0.65, 1.0), Color(1.0, 0.76, 0.18), Color(1.0, 0.40, 0.30)],
	"hue_alpha": 0.30,
	# The batch ghost's fills (accepted, refused): each stroke's ribbon at its road's width.
	"ghost_fills": [Color(1.0, 1.0, 1.0, 0.18), Color(1.0, 0.40, 0.30, 0.28)],
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
# The line surface's custom attribute: one float, the look.
const LINE_FORMAT := Mesh.ARRAY_CUSTOM_R_FLOAT << Mesh.ARRAY_FORMAT_CUSTOM0_SHIFT

var _mesh := ArrayMesh.new()
# Every line look in one material (round V2), shared with the batch ghost's edges.
var _line_material := ShaderMaterial.new()
var _handle_material := ShaderMaterial.new()
# `depth_line_material`, made on first use.
var _depth_line_material: ShaderMaterial = null
# The surfaces' layout (the line surface's vertex count, the handles' count): while it holds, a
# new pose rewrites the buffers in place instead of rebuilding the surfaces.
var _layout := Vector2i(-1, -1)
# Quad index buffers by vertex count (two triangles a quad), made once per count.
var _quad_indices := {}

func _init() -> void:
	mesh = _mesh
	# Vertices move in place without the mesh's bounds following: never cull the gizmos.
	custom_aabb = AABB(Vector3(-1.0e6, -1.0e4, -1.0e6), Vector3(2.0e6, 2.0e4, 2.0e6))
	SceneLightingConfig.apply_shadow_policy(self, SceneLightingConfig.SHADOW_DEBUG_OVERLAY, "roads")
	var colors := PackedColorArray()
	var widths := PackedFloat32Array()
	var dashes := PackedFloat32Array()
	var gaps := PackedFloat32Array()
	for look in PALETTE["lines"]:
		colors.append(look[0])
		widths.append(look[1])
		dashes.append(look[2])
		gaps.append(look[3])
	_line_material.shader = LineShader
	_line_material.set_shader_parameter("colors", colors)
	_line_material.set_shader_parameter("widths_px", widths)
	_line_material.set_shader_parameter("dashes_px", dashes)
	_line_material.set_shader_parameter("gaps_px", gaps)
	_line_material.render_priority = 10
	# One handle surface: each point's kind (its UV) picks the colour and size (round V2).
	_handle_material.shader = HandleShader
	_handle_material.set_shader_parameter("colors", PackedColorArray(PALETTE["handles"]))
	_handle_material.set_shader_parameter("sizes_px", PackedFloat32Array(PALETTE["handle_sizes_px"]))
	_handle_material.render_priority = 11

## Uploads the gizmos `simulation` built in its last tick (`road_tool_gizmo_layout`,
## `road_tool_gizmo_bytes`), on the tick's gizmo flag.
func show_gizmos(simulation: Object) -> void:
	var layout: Vector2i = simulation.road_tool_gizmo_layout()
	# The dashes keep their length on screen at the cursor's depth.
	_line_material.set_shader_parameter("dash_focus", simulation.road_tool_gizmo_focus())
	if layout != _layout or _mesh.get_surface_count() == 0:
		_layout = layout
		_mesh.clear_surfaces()
		if layout.x > 0:
			_add_line_placeholder(_mesh, layout.x)
		if layout.y > 0:
			var arrays := []
			arrays.resize(Mesh.ARRAY_MAX)
			var points := PackedVector3Array()
			points.resize(layout.y)
			arrays[Mesh.ARRAY_VERTEX] = points
			# A placeholder of the right size: the kinds go in below in one write.
			var uv := PackedVector2Array()
			uv.resize(layout.y)
			arrays[Mesh.ARRAY_TEX_UV] = uv
			_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_POINTS, arrays)
			_mesh.surface_set_material(_mesh.get_surface_count() - 1, _handle_material)
	# Positions (12 bytes a vertex) and attributes in place, the line surface first; a handle
	# can change kind in place (the live end becoming a placed end).
	var rid := _mesh.get_rid()
	var surface := 0
	if layout.x > 0:
		RenderingServer.mesh_surface_update_vertex_region(rid, 0, 0, simulation.road_tool_gizmo_bytes(0))
		RenderingServer.mesh_surface_update_attribute_region(rid, 0, 0, simulation.road_tool_gizmo_bytes(1))
		surface = 1
	if layout.y > 0:
		RenderingServer.mesh_surface_update_vertex_region(rid, surface, 0, simulation.road_tool_gizmo_bytes(2))
		RenderingServer.mesh_surface_update_attribute_region(rid, surface, 0, simulation.road_tool_gizmo_bytes(3))
	visible = _mesh.get_surface_count() > 0

## Appends a line surface of quads (`road_tool_line.gdshader`) to `mesh`: placeholders of the
## right size for UV, UV2 and the look, then Rust's interleaved attributes in one write (the
## batch ghost and the footprint hue's tunnel outlines draw their edges with it too).
func add_line_surface(mesh: ArrayMesh, vertices: PackedVector3Array, attributes: PackedFloat32Array) -> void:
	_add_line_surface_arrays(mesh, vertices)
	RenderingServer.mesh_surface_update_attribute_region(mesh.get_rid(), mesh.get_surface_count() - 1, 0, attributes.to_byte_array())

# A line surface of `count` vertices whose positions and attributes are written after it.
func _add_line_placeholder(mesh: ArrayMesh, count: int) -> void:
	var vertices := PackedVector3Array()
	vertices.resize(count)
	_add_line_surface_arrays(mesh, vertices)

# A line surface of quads at `vertices`, with placeholders for UV, UV2 and the look.
func _add_line_surface_arrays(mesh: ArrayMesh, vertices: PackedVector3Array) -> void:
	var count := vertices.size()
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	var uv := PackedVector2Array()
	uv.resize(count)
	arrays[Mesh.ARRAY_TEX_UV] = uv
	arrays[Mesh.ARRAY_TEX_UV2] = uv
	var looks := PackedFloat32Array()
	looks.resize(count)
	arrays[Mesh.ARRAY_CUSTOM0] = looks
	arrays[Mesh.ARRAY_INDEX] = _indices(count)
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays, [], {}, LINE_FORMAT)
	mesh.surface_set_material(mesh.get_surface_count() - 1, _line_material)

## The line material with the normal depth test (`road_tool_line_depth.gdshader`, the same
## palette): the footprint hue's tunnel outlines, which hills, trees and buildings hide.
func depth_line_material() -> ShaderMaterial:
	if _depth_line_material == null:
		_depth_line_material = _line_material.duplicate()
		_depth_line_material.shader = LineDepthShader
		_depth_line_material.render_priority = 8
	return _depth_line_material

## Removes every gizmo.
func clear() -> void:
	if _mesh.get_surface_count() > 0:
		_mesh.clear_surfaces()
	_layout = Vector2i(-1, -1)
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

# Batch preview ghost (`ROAD-112` rounds G1 and V2): tier 1's ribbons of a draft's strokes, drawn
# by its own child mesh so the gizmos' in-place layout above never sees it, in the gizmos' look:
# a faint fill over the terrain and the ribbon's edges as solid lines of `road_tool_line.gdshader`
# (quads Rust built), valid strokes in the stroke's white, refused ones in the refusal red. The
# batch's exact preview replaces it through the road preview display once tier 2 accepts.
var _ghost: MeshInstance3D
# By verdict (valid, refused): the fill's material (the edges share the gizmos' line material).
var _ghost_fills: Array[StandardMaterial3D] = []

## Draws tier 1's ghost of a batch pose (`preview_road_draft`'s `ghost`, `ghost_starts`, `valid`
## and the edges' `ghost_line_*` quads): per verdict one fill surface, and one edge surface.
func show_batch_ghost(result: Dictionary) -> void:
	if _ghost == null:
		_ghost = MeshInstance3D.new()
		_ghost.mesh = ArrayMesh.new()
		_ghost.custom_aabb = custom_aabb
		SceneLightingConfig.apply_shadow_policy(_ghost, SceneLightingConfig.SHADOW_DEBUG_OVERLAY, "roads")
		for color in PALETTE["ghost_fills"]:
			_ghost_fills.append(_fill_material(color))
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
		mesh.surface_set_material(mesh.get_surface_count() - 1, _ghost_fills[verdict])
	var edges: PackedVector3Array = result.get("ghost_line_vertices", PackedVector3Array())
	if not edges.is_empty():
		add_line_surface(mesh, edges, result.get("ghost_line_attributes", PackedFloat32Array()))
	_ghost.visible = mesh.get_surface_count() > 0

## Removes the batch ghost.
func clear_batch_ghost() -> void:
	if _ghost != null:
		(_ghost.mesh as ArrayMesh).clear_surfaces()
		_ghost.visible = false

## The batch ghost's surface count (tests).
func batch_ghost_surfaces() -> int:
	return 0 if _ghost == null else (_ghost.mesh as ArrayMesh).get_surface_count()

# The ghost's fill: unshaded, translucent, over the terrain like the lines (no depth test), just
# under them.
static func _fill_material(color: Color) -> StandardMaterial3D:
	var material := StandardMaterial3D.new()
	material.albedo_color = color
	material.shading_mode = BaseMaterial3D.SHADING_MODE_UNSHADED
	material.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	material.cull_mode = BaseMaterial3D.CULL_DISABLED
	material.no_depth_test = true
	material.render_priority = 9
	return material
