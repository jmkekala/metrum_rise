# SPDX-License-Identifier: GPL-2.0-only

## Cosmetic Finnish road signs. Rust supplies five floats per pose: chunk-local pole-base
## x, y, z, yaw about +Y, model index. This renderer only batches those transforms.
extends Node3D

const WindowMaterials := preload("res://scripts/renderers/window_materials.gd")
const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")
const MODEL_DIR := "res://assets/models/road_signs/"
# Indexed by Rust RoadSignKind. B1/B5/B6 are assets for later, never placed here.
const MODELS: Array[String] = [
	"E1-1", "E1-2", "C32-30", "C32-40", "C32-50", "C32-60", "C32-80",
	"A1.1", "A1.2", "A2.1", "A2.2",
	# Descents (A3.2: the car points down the slope), then ascents (A3.1).
	"A3.2-8", "A3.2-10", "A3.2-12", "A3.2-14", "A3.2-16", "A3.2-18",
	"A3.1-8", "A3.1-10", "A3.1-12", "A3.1-14", "A3.1-16", "A3.1-18", "A9",
]
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
		var mesh: ArrayMesh
		var scene := load(MODEL_DIR + model_name + ".glb") as PackedScene
		if scene != null:
			var model := scene.instantiate()
			var node := model as MeshInstance3D
			if node == null:
				# Godot replaces "." in imported node names (A1.1 becomes A1_1).
				node = model.find_child(model_name.validate_node_name(), true, false) as MeshInstance3D
			if node != null:
				mesh = ArrayMesh.new()
				# Each GLB has one surface. Bake its node transform into the shared mesh once.
				for surface in node.mesh.get_surface_count():
					var arrays := node.mesh.surface_get_arrays(surface)
					var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
					var normals: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
					for i in vertices.size():
						vertices[i] = node.transform * vertices[i]
						if i < normals.size():
							normals[i] = (node.transform.basis * normals[i]).normalized()
					arrays[Mesh.ARRAY_VERTEX] = vertices
					arrays[Mesh.ARRAY_NORMAL] = normals
					mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
					var material := node.get_active_material(surface) as BaseMaterial3D
					if material != null:
						# Shared scene-material factory; brightness zero gives the atlas no emission.
						mesh.surface_set_material(surface, WindowMaterials.create(material, false, 0.0))
			model.free()
		if mesh == null:
			push_error("road_signs: %s.glb lacks its sign mesh" % model_name)
		_meshes.append(mesh)
