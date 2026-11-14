# SPDX-License-Identifier: GPL-2.0-only

## The road tool's footprint hue (`ROAD-112` round HU): a translucent mark of the ground a
## draft's road takes, so a draft reads as a preview where the preview terrain's grass still
## covers the preview road (T27 (d)). A stopgap: it marks the footprint and does not fix the lag
## (`TERRAIN-14`). Rust builds the mesh and decides its state (`road_tool_hue`: the fill on the
## road, a bridge's on the ground under it; a tunnel's outline as line quads); this node only
## uploads it, in place while the padded fill keeps its size. The fill draws once, unshaded and
## translucent without writing depth, with the normal depth test (the user, 2026-11-14): Rust
## lays it on the higher of the road and the ground, so hills, trees and buildings in front hide
## it; above the water, below the tool's lines and chips.
## Colours and alphas come from the gizmos' one palette; a refusal is hatched as well as red.
extends MeshInstance3D

const RoadToolGizmos := preload("res://scripts/renderers/road_tool_gizmos.gd")
const HueShader := preload("res://scripts/shaders/road_tool_hue.gdshader")
# Above the water (0), below the batch ghost's fill (9) and the tool's lines (10).
const RENDER_PRIORITY := 8

var _mesh := ArrayMesh.new()
var _fill := ShaderMaterial.new()
# The gizmos (`road_tool_gizmos.gd`), whose depth-tested line material draws the tunnel outlines.
var _gizmos: Node
# The fill's vertex count while its surface is the mesh's only one: a pose of the same count
# rewrites it in place.
var _fill_size := -1
# The state the materials show (-1 none yet).
var _state := -1

func _init(gizmos: Node) -> void:
	_gizmos = gizmos
	mesh = _mesh
	# Vertices move in place without the mesh's bounds following: never cull the hue.
	custom_aabb = AABB(Vector3(-1.0e6, -1.0e4, -1.0e6), Vector3(2.0e6, 2.0e4, 2.0e6))
	cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	_fill.shader = HueShader
	_fill.set_shader_parameter("hue_alpha", RoadToolGizmos.PALETTE["hue_alpha"])
	_fill.render_priority = RENDER_PRIORITY
	visible = false

## Draws `hue` (`road_tool_hue`'s dictionary); returns the upload's milliseconds.
func show_hue(hue: Dictionary) -> float:
	var start_us := Time.get_ticks_usec()
	var state := int(hue.get("state", -1))
	if state < 0:
		clear()
		return float(Time.get_ticks_usec() - start_us) / 1000.0
	if state != _state:
		_state = state
		var color: Color = RoadToolGizmos.PALETTE["hue"][state]
		_fill.set_shader_parameter("hue_color", color)
		_fill.set_shader_parameter("hatched", state == 2)
	var count := int(hue.get("fill_vertices", 0))
	var fill: PackedByteArray = hue.get("fill_bytes", PackedByteArray())
	var outline: PackedVector3Array = hue.get("outline_vertices", PackedVector3Array())
	if outline.is_empty() and count == _fill_size and _mesh.get_surface_count() == 1:
		RenderingServer.mesh_surface_update_vertex_region(_mesh.get_rid(), 0, 0, fill)
	else:
		_mesh.clear_surfaces()
		_fill_size = -1
		if count > 0:
			# A placeholder of the right size, then Rust's bytes in one write.
			var placeholder := PackedVector3Array()
			placeholder.resize(count)
			var arrays := []
			arrays.resize(Mesh.ARRAY_MAX)
			arrays[Mesh.ARRAY_VERTEX] = placeholder
			_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
			_mesh.surface_set_material(0, _fill)
			RenderingServer.mesh_surface_update_vertex_region(_mesh.get_rid(), 0, 0, fill)
			if outline.is_empty():
				_fill_size = count
		if not outline.is_empty():
			_gizmos.add_line_surface(_mesh, outline, hue.get("outline_attributes", PackedFloat32Array()))
			_mesh.surface_set_material(_mesh.get_surface_count() - 1, _gizmos.depth_line_material())
	visible = _mesh.get_surface_count() > 0
	return float(Time.get_ticks_usec() - start_us) / 1000.0

## Hides the hue.
func clear() -> void:
	if _mesh.get_surface_count() > 0:
		_mesh.clear_surfaces()
	_fill_size = -1
	visible = false
