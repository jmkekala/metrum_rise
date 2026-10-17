# SPDX-License-Identifier: GPL-2.0-only

## Vehicle lamps: appends the night-lighting surfaces to a car mesh built by agents.gd.
##
## Lamp centres are measured from the original vehicle GLBs, after the renderer turns them to
## face -Z and grounds their wheels. Adds two surfaces to every car instance of the MultiMesh:
##   - glow: one camera-facing quad per lamp (vehicle_lamp_glow.gdshader). All four vertices sit
##     at the lamp centre; UV holds the corner (-1..1), NORMAL the way the lamp faces, COLOR.rgb
##     its colour and COLOR.a its world radius.
##   - beam: a box round the car (vehicle_headlight_beam.gdshader) that lights whatever surface
##     the depth buffer holds inside it: a red tail glow behind, and low beams ahead for cars
##     beyond the real headlights of vehicle_headlights.gd.
## Both collapse in their vertex shaders by day, so they draw nothing then.
extends RefCounted

const GLOW_SHADER := preload("res://scripts/shaders/vehicle_lamp_glow.gdshader")
const BEAM_SHADER := preload("res://scripts/shaders/vehicle_headlight_beam.gdshader")
# Metadata on the mesh: body surface count and body AABB, before the lamp surfaces were added.
const BODY_SURFACES_META := "vehicle_body_surfaces"
const BODY_AABB_META := "vehicle_body_aabb"
const HEAD_LAMP_META := "vehicle_head_lamp"
# Right-hand lamp centres in grounded, -Z-facing mesh space, in metres. Measured from the
# GLBs' front/rear lens bounds (sports uses the corresponding untextured body panels).
# Each left-hand lamp mirrors X. These anchors are independent of atlas palette variants.
const MODEL_LAMPS := {
	"sedan": [Vector3(0.4125, 0.65, -1.15), Vector3(0.4125, 0.65, 1.25)],
	"sedan-sports": [Vector3(0.4125, 0.55, -1.15), Vector3(0.4125, 0.55, 1.25)],
	"suv": [Vector3(0.4125, 0.65, -1.25), Vector3(0.50, 0.60, 1.15)],
	"suv-luxury": [Vector3(0.4125, 0.6875, -1.2375), Vector3(0.45, 0.75, 1.40)],
	"delivery": [Vector3(0.4125, 0.65, -1.50), Vector3(0.55, 0.45, 1.60)],
}
const HEAD_COLOR := Color(1.0, 0.93, 0.8)
const TAIL_COLOR := Color(1.0, 0.06, 0.03)
const HEAD_GLOW_RADIUS_M := 0.22
const TAIL_GLOW_RADIUS_M := 0.14
# Extent of the lit volume: reach ahead of the headlights and behind the tail lights, half width,
# and depth below and height above the car, in metres. The headlight cone is spent by the real
# light's range (`vehicle_headlights.gd` RANGE_M, 60 m) and is tan(35°) × 60 m wide there, so
# neither face shows; the shader fades the cone out below the top face. The vertical span keeps a
# dipping or climbing road inside the box.
const BEAM_REACH_M := 60.0
const TAIL_REACH_M := 4.0
const BEAM_HALF_WIDTH_M := 42.0
const BEAM_BELOW_M := 4.0
const BEAM_ABOVE_M := 8.0

const GLOW_MATERIAL_KEY := "glow"


# `materials` is the caller's cache, shared by every model it builds: the glow material under
# `GLOW_MATERIAL_KEY` and one beam material per `material_key` (it holds that model's lamps).
static func add_surfaces(mesh: ArrayMesh, material_key: String, materials: Dictionary) -> void:
	var body_surfaces := mesh.get_surface_count()
	mesh.set_meta(BODY_SURFACES_META, body_surfaces)
	mesh.set_meta(BODY_AABB_META, mesh.get_aabb())
	var bounds := mesh.get_aabb()
	var model_name := material_key.get_file().get_basename()
	var anchors: Array = MODEL_LAMPS.get(model_name, [
		Vector3(bounds.size.x * 0.35, bounds.size.y * 0.45, bounds.position.z),
		Vector3(bounds.size.x * 0.35, bounds.size.y * 0.45, bounds.end.z),
	])
	var head := [anchors[0] * Vector3(-1, 1, 1), anchors[0]]
	var tail := [anchors[1] * Vector3(-1, 1, 1), anchors[1]]
	# The right-hand headlight, for vehicle_headlights.gd.
	mesh.set_meta(HEAD_LAMP_META, head[1])
	if not materials.has(GLOW_MATERIAL_KEY):
		var glow := ShaderMaterial.new()
		glow.shader = GLOW_SHADER
		materials[GLOW_MATERIAL_KEY] = glow
	_add_glow_surface(mesh, head, tail, materials[GLOW_MATERIAL_KEY])
	if not materials.has(material_key):
		var beam := ShaderMaterial.new()
		beam.shader = BEAM_SHADER
		# The right-hand lamp of each pair; the shader mirrors it for the left.
		beam.set_shader_parameter("head_lamp", head[1])
		beam.set_shader_parameter("tail_lamp", tail[1])
		beam.set_shader_parameter("tail_reach_m", TAIL_REACH_M)
		beam.set_shader_parameter("beam_above_m", BEAM_ABOVE_M)
		beam.set_shader_parameter("beam_reach_m", BEAM_REACH_M)
		beam.set_shader_parameter("beam_half_width_m", BEAM_HALF_WIDTH_M)
		materials[material_key] = beam
	_add_beam_surface(mesh, head[1].z, tail[1].z, materials[material_key])


static func _add_glow_surface(mesh: ArrayMesh, head: Array, tail: Array, material: Material) -> void:
	var vertices := PackedVector3Array()
	var normals := PackedVector3Array()
	var colors := PackedColorArray()
	var uvs := PackedVector2Array()
	var indices := PackedInt32Array()
	var lamps := []
	for lamp in head:
		lamps.append([lamp, Vector3.FORWARD, Color(HEAD_COLOR, HEAD_GLOW_RADIUS_M)])
	for lamp in tail:
		lamps.append([lamp, Vector3.BACK, Color(TAIL_COLOR, TAIL_GLOW_RADIUS_M)])
	for lamp in lamps:
		var base := vertices.size()
		for corner in [Vector2(-1, -1), Vector2(1, -1), Vector2(1, 1), Vector2(-1, 1)]:
			vertices.append(lamp[0])
			normals.append(lamp[1])
			colors.append(lamp[2])
			uvs.append(corner)
		indices.append_array([base, base + 1, base + 2, base, base + 2, base + 3])
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_NORMAL] = normals
	arrays[Mesh.ARRAY_COLOR] = colors
	arrays[Mesh.ARRAY_TEX_UV] = uvs
	arrays[Mesh.ARRAY_INDEX] = indices
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	mesh.surface_set_material(mesh.get_surface_count() - 1, material)


static func _add_beam_surface(mesh: ArrayMesh, head_z: float, tail_z: float, material: Material) -> void:
	var front := head_z - BEAM_REACH_M
	var back := tail_z + TAIL_REACH_M
	var box := BoxMesh.new()
	box.size = Vector3(BEAM_HALF_WIDTH_M * 2.0, BEAM_BELOW_M + BEAM_ABOVE_M, back - front)
	var arrays := box.get_mesh_arrays()
	var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
	var center := Vector3(0.0, (BEAM_ABOVE_M - BEAM_BELOW_M) * 0.5, (front + back) * 0.5)
	for i in vertices.size():
		vertices[i] += center
	var out := []
	out.resize(Mesh.ARRAY_MAX)
	out[Mesh.ARRAY_VERTEX] = vertices
	out[Mesh.ARRAY_INDEX] = arrays[Mesh.ARRAY_INDEX]
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, out)
	mesh.surface_set_material(mesh.get_surface_count() - 1, material)
