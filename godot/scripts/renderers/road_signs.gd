# SPDX-License-Identifier: GPL-2.0-only

## Cosmetic Finnish road signs. Rust supplies five floats per pose: chunk-local pole-base
## x, y, z, yaw about +Y, model index. This renderer only batches those transforms.
extends Node3D

const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const MODEL_DIR := "res://assets/models/road_signs/"
# Indexed by Rust RoadSignKind. B1/B6 are assets for later, never placed here. B5-D2 is B5.glb
# with the D2 disc added under its triangle (`_add_roundabout_disc`); E1-B5-D2 is that with
# E1-1.glb's plate raised above the triangle on the same pole (`_append_glb`).
const MODELS: Array[String] = [
	"E1-1", "E1-2", "C32-30", "C32-40", "C32-50", "C32-60", "C32-80",
	"A1.1", "A1.2", "A2.1", "A2.2",
	# Descents (A3.2: the car points down the slope), then ascents (A3.1).
	"A3.2-8", "A3.2-10", "A3.2-12", "A3.2-14", "A3.2-16", "A3.2-18",
	"A3.1-8", "A3.1-10", "A3.1-12", "A3.1-14", "A3.1-16", "A3.1-18", "A9", "B5-D2", "E1-B5-D2",
]
# E1-B5-D2: E1-1's plate (2.0-2.6 m in its GLB) is raised to stand 6 cm above B5's triangle top
# (2.789 m), and E1-1's pole base moves up to B5's pole top (2.671 m), so one pole runs through.
const CROSSING_ON_RING_LIFT_M := 0.85
const B5_POLE_TOP_M := 2.671
const D2_BLUE_RADIUS_FRACTION := 0.95
# D2's 0.30 m disc hangs 5 cm below B5's triangle tip (2.0 m in B5.glb), not over it.
const D2_CENTRE_Y_M := 1.65
# The atlas signs' own sign blue (Väylä's #005EB8), back grey and finish, so D2 matches E1 beside it.
const SIGN_BLUE := Color8(0, 94, 184)
const SIGN_BACK_GREY := Color8(154, 159, 160)
const SIGN_METALLIC := 0.18
const SIGN_ROUGHNESS := 0.46
# GLB contract (validation.json/build.log and export_validation.json): metres, ground-origin
# pole base, +Y up, +Z is the printed front. One mesh/surface and one embedded atlas material
# per model, no authored LODs (E1: 72 triangles, C32: 216). No three-LOD light-model lookup.
# The shared atlas UVs and imported mesh transform are retained. Published dimensions are
# look-dev assumptions (dimensions_verified=false); runtime import was not tested upstream.
static var _meshes: Array[ArrayMesh] = []
var _chunks: Dictionary = {}

func set_chunk(key: Vector2i, origin: Vector3, signs: PackedFloat32Array) -> void:
	remove_chunk(key)
	if signs.size() < 5:
		return
	_ensure_meshes()
	var buckets: Array = []
	buckets.resize(MODELS.size())
	for kind in MODELS.size():
		buckets[kind] = []
	for i in range(0, signs.size() - 4, 5):
		var kind := int(signs[i + 4])
		if kind < 0 or kind >= MODELS.size() or _meshes[kind] == null:
			continue
		var base := Vector3(signs[i], signs[i + 1], signs[i + 2])
		# +Z front rotated about +Y points to (sin yaw, 0, cos yaw), matching Rust.
		(buckets[kind] as Array).append(Transform3D(Basis(Vector3.UP, signs[i + 3]), base))
	var root := Node3D.new()
	root.name = "RoadSigns_%d_%d" % [key.x, key.y]
	root.position = origin
	for kind in MODELS.size():
		var transforms: Array = buckets[kind]
		if transforms.is_empty():
			continue
		var multimesh := MultiMesh.new()
		multimesh.transform_format = MultiMesh.TRANSFORM_3D
		multimesh.mesh = _meshes[kind]
		multimesh.instance_count = transforms.size()
		for i in transforms.size():
			multimesh.set_instance_transform(i, transforms[i])
		var instance := MultiMeshInstance3D.new()
		instance.name = MODELS[kind]
		instance.multimesh = multimesh
		SceneLightingConfig.apply_shadow_policy(instance, SceneLightingConfig.SHADOW_STATIC_CASTER, "road_signs")
		root.add_child(instance)
	add_child(root)
	_chunks[key] = root

func remove_chunk(key: Vector2i) -> void:
	if not _chunks.has(key):
		return
	var root: Node3D = _chunks[key]
	_chunks.erase(key)
	root.queue_free()

func clear() -> void:
	for key in _chunks.keys():
		remove_chunk(key)

static func _ensure_meshes() -> void:
	if not _meshes.is_empty():
		return
	for model_name in MODELS:
		var mesh := ArrayMesh.new()
		var ring := model_name.ends_with("B5-D2")
		var loaded := _append_glb(mesh, "B5" if ring else model_name)
		if loaded and ring:
			_add_roundabout_disc(mesh)
		if loaded and model_name == "E1-B5-D2":
			loaded = _append_glb(mesh, "E1-1", CROSSING_ON_RING_LIFT_M, B5_POLE_TOP_M)
		if not loaded:
			push_error("road_signs: %s lacks its sign mesh" % model_name)
			mesh = null
		_meshes.append(mesh)

# Appends a sign GLB's one mesh to `mesh` with its node transform baked in. A positive `lift`
# raises the sign by that much and moves its pole base (the vertices at the ground) to `pole_floor`.
static func _append_glb(mesh: ArrayMesh, source_name: String, lift := 0.0, pole_floor := 0.0) -> bool:
	var scene := load(MODEL_DIR + source_name + ".glb") as PackedScene
	if scene == null:
		return false
	var model := scene.instantiate()
	var node := model as MeshInstance3D
	if node == null:
		# Godot replaces "." in imported node names (A1.1 becomes A1_1).
		node = model.find_child(source_name.validate_node_name(), true, false) as MeshInstance3D
	if node == null:
		model.free()
		return false
	# Each GLB has one surface. Bake its node transform into the shared mesh once.
	for surface in node.mesh.get_surface_count():
		var arrays := node.mesh.surface_get_arrays(surface)
		var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
		var normals: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
		for i in vertices.size():
			vertices[i] = node.transform * vertices[i]
			if lift > 0.0:
				vertices[i].y = pole_floor if vertices[i].y < 0.001 else vertices[i].y + lift
			if i < normals.size():
				normals[i] = (node.transform.basis * normals[i]).normalized()
		arrays[Mesh.ARRAY_VERTEX] = vertices
		arrays[Mesh.ARRAY_NORMAL] = normals
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		var material := node.get_active_material(surface) as BaseMaterial3D
		if material != null:
			# Shared scene-material factory; brightness zero gives the atlas no emission.
			mesh.surface_set_material(mesh.get_surface_count() - 1, WindowMaterials.create(material, false, 0.0))
	model.free()
	return true

# D2 sits below the B5 triangle on the same pole: a blue disc inside a white rim, three white
# arrows running counterclockwise, on a grey plate that is its back from behind. Code-native geometry on the
# shared cloud-shadow material, as the atlas signs.
static func _add_roundabout_disc(mesh: ArrayMesh) -> void:
	var blue := SurfaceTool.new()
	var white := SurfaceTool.new()
	var grey := SurfaceTool.new()
	blue.begin(Mesh.PRIMITIVE_TRIANGLES)
	white.begin(Mesh.PRIMITIVE_TRIANGLES)
	grey.begin(Mesh.PRIMITIVE_TRIANGLES)
	var origin := Vector3(0, D2_CENTRE_Y_M, 0.012)
	var back := origin - Vector3(0, 0, 0.004)
	for step in 48:
		var a := TAU * float(step) / 48.0
		var b := TAU * float(step + 1) / 48.0
		var rim_a := Vector3(cos(a), sin(a), 0) * 0.30
		var rim_b := Vector3(cos(b), sin(b), 0) * 0.30
		# The white rim is 15 mm of the 600 mm disc, as on the printed sign.
		var inner_a := rim_a * D2_BLUE_RADIUS_FRACTION
		var inner_b := rim_b * D2_BLUE_RADIUS_FRACTION
		_sign_triangle(blue, origin, origin + inner_a, origin + inner_b)
		_sign_triangle(white, origin + inner_a, origin + rim_a, origin + rim_b)
		_sign_triangle(white, origin + inner_a, origin + rim_b, origin + inner_b)
		_sign_triangle(grey, back, back + rim_a, back + rim_b)
	for arrow in 3:
		var start := TAU * float(arrow) / 3.0
		for step in 12:
			var a := start + float(step) * 1.35 / 12.0
			var b := start + float(step + 1) * 1.35 / 12.0
			var p := origin + Vector3(0, 0, 0.004)
			var v0 := p + Vector3(cos(a), sin(a), 0) * 0.19
			var v1 := p + Vector3(cos(b), sin(b), 0) * 0.19
			var v2 := p + Vector3(cos(a), sin(a), 0) * 0.14
			var v3 := p + Vector3(cos(b), sin(b), 0) * 0.14
			_sign_triangle(white, v0, v1, v2)
			_sign_triangle(white, v2, v1, v3)
		var angle := start + 1.35
		var tip := origin + Vector3(cos(angle), sin(angle), 0) * 0.165 + Vector3(0, 0, 0.004)
		var tangent := Vector3(-sin(angle), cos(angle), 0)
		var radial := Vector3(cos(angle), sin(angle), 0)
		_sign_triangle(white, tip + tangent * 0.09, tip - radial * 0.075 - tangent * 0.025, tip + radial * 0.075 - tangent * 0.025)
	for entry: Array in [[blue, SIGN_BLUE], [white, Color.WHITE], [grey, SIGN_BACK_GREY]]:
		var material := StandardMaterial3D.new()
		material.albedo_color = entry[1]
		material.cull_mode = BaseMaterial3D.CULL_DISABLED
		material.metallic = SIGN_METALLIC
		material.roughness = SIGN_ROUGHNESS
		entry[0].set_material(WindowMaterials.create(material, false, 0.0))
		entry[0].commit(mesh)

static func _sign_triangle(surface: SurfaceTool, a: Vector3, b: Vector3, c: Vector3) -> void:
	for point: Vector3 in [a, b, c]:
		surface.set_normal(Vector3(0, 0, 1))
		surface.add_vertex(point)
