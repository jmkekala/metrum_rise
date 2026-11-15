# SPDX-License-Identifier: GPL-2.0-only

## A glowing band around a building's footprint, the road selection's look
## (`road_highlight.gdshader`'s ribbon: an outline at each edge with a glow between), in a tint
## the caller picks. The building inspector rings the building it shows with it. The band runs
## from just inside the walls to `OUTSIDE_M` beyond them, mitred at the corners so each outline
## stays one line. Its shader (`footprint_halo.gdshader`) is the road highlight's, depth-tested,
## so the building hides the far side of its own ring.
extends MeshInstance3D

const HaloShader = preload("res://assets/materials/footprint_halo.gdshader")

## How far the band reaches beyond the walls, and in under them, in metres.
const OUTSIDE_M := 2.0
const INSIDE_M := 0.2
# The steepest corner widening: a sharp corner is capped instead of spiking.
const MITRE_MIN_COS := 0.5

func _init() -> void:
	cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var mat := ShaderMaterial.new()
	mat.shader = HaloShader
	mat.set_shader_parameter("fill_alpha", 0.08)
	mat.set_shader_parameter("glow_alpha", 0.30)
	mat.set_shader_parameter("edge_alpha", 0.95)
	mat.set_shader_parameter("chevron_alpha", 0.0)
	mat.set_shader_parameter("width_m", OUTSIDE_M + INSIDE_M)
	material_override = mat
	visible = false

## Rings `footprint` (Rust's outline points, in order, at the height to draw at) in `tint`; fewer
## than three points hides the halo.
func show_footprint(footprint: PackedVector3Array, tint: Color) -> void:
	var count := footprint.size()
	if count < 3:
		hide_halo()
		return
	(material_override as ShaderMaterial).set_shader_parameter("tint", tint)
	# Twice the signed area in xz; its sign says which way the outline turns, so which side is out.
	var area := 0.0
	for i in count:
		var a := footprint[i]
		var b := footprint[(i + 1) % count]
		area += a.x * b.z - b.x * a.z
	var turn := 1.0 if area > 0.0 else -1.0
	var outward := PackedVector3Array()
	outward.resize(count)
	for i in count:
		var prev := footprint[(i + count - 1) % count]
		var here := footprint[i]
		var next := footprint[(i + 1) % count]
		var n_in := _edge_normal(prev, here, turn)
		var n_out := _edge_normal(here, next, turn)
		var n := (n_in + n_out).normalized()
		if n == Vector3.ZERO:
			n = n_out
		outward[i] = n / maxf(n.dot(n_out), MITRE_MIN_COS)
	var im := ImmediateMesh.new()
	im.surface_begin(Mesh.PRIMITIVE_TRIANGLES)
	im.surface_set_color(Color.BLACK)
	for i in count:
		var j := (i + 1) % count
		var a0 := footprint[i] + outward[i] * OUTSIDE_M
		var a1 := footprint[i] - outward[i] * INSIDE_M
		var b0 := footprint[j] + outward[j] * OUTSIDE_M
		var b1 := footprint[j] - outward[j] * INSIDE_M
		# UV.x 0 on the outer edge, 1 on the inner; UV.y unused (no chevrons).
		for v in [[a0, 0.0], [a1, 1.0], [b1, 1.0], [a0, 0.0], [b1, 1.0], [b0, 0.0]]:
			im.surface_set_uv(Vector2(v[1], 0.0))
			im.surface_add_vertex(v[0])
	im.surface_end()
	mesh = im
	visible = true

## Hides the halo and drops its mesh.
func hide_halo() -> void:
	visible = false
	mesh = null

# The outward unit normal (in xz) of the side from `a` to `b`.
static func _edge_normal(a: Vector3, b: Vector3, turn: float) -> Vector3:
	var d := Vector3(b.x - a.x, 0.0, b.z - a.z).normalized()
	return Vector3(d.z, 0.0, -d.x) * turn
