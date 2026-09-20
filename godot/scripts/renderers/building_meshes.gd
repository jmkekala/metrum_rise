# SPDX-License-Identifier: GPL-2.0-only

## Pack-generation mesh cache. Imports and root-space baking happen only at loading/reload,
## never while changing LODs. Authored surface materials, including emission, are preserved.
extends RefCounted

var _meshes: Dictionary = {}
var imports := 0

func load_mesh(path: String) -> ArrayMesh:
	if _meshes.has(path):
		return _meshes[path]
	var mesh := _import(path)
	_meshes[path] = mesh
	return mesh

func _import(path: String) -> ArrayMesh:
	imports += 1
	if not FileAccess.file_exists(path):
		return null
	var document: GLTFDocument = FBXDocument.new() if path.get_extension().to_lower() == "fbx" else GLTFDocument.new()
	var state: GLTFState = FBXState.new() if document is FBXDocument else GLTFState.new()
	if document.append_from_file(path, state) != OK:
		return null
	var scene := document.generate_scene(state)
	if scene == null:
		return null
	var mesh := ArrayMesh.new()
	_bake_node(scene, scene, Transform3D.IDENTITY, mesh)
	scene.free()
	return mesh if mesh.get_surface_count() > 0 else null

func _bake_node(node: Node, root: Node, parent_transform: Transform3D, result: ArrayMesh) -> void:
	var transform := parent_transform
	if node is Node3D and node != root:
		transform = parent_transform * (node as Node3D).transform
	if node is MeshInstance3D:
		var instance := node as MeshInstance3D
		if instance.mesh:
			for surface in instance.mesh.get_surface_count():
				var arrays := instance.mesh.surface_get_arrays(surface)
				var vertices: PackedVector3Array = arrays[Mesh.ARRAY_VERTEX]
				for index in vertices.size():
					vertices[index] = transform * vertices[index]
				arrays[Mesh.ARRAY_VERTEX] = vertices
				if arrays[Mesh.ARRAY_NORMAL] is PackedVector3Array:
					var normals: PackedVector3Array = arrays[Mesh.ARRAY_NORMAL]
					var normal_transform := transform.basis.inverse().transposed()
					for index in normals.size():
						normals[index] = (normal_transform * normals[index]).normalized()
					arrays[Mesh.ARRAY_NORMAL] = normals
				if arrays[Mesh.ARRAY_TANGENT] is PackedFloat32Array:
					var tangents: PackedFloat32Array = arrays[Mesh.ARRAY_TANGENT]
					for index in range(0, tangents.size(), 4):
						var tangent := (transform.basis * Vector3(tangents[index], tangents[index + 1], tangents[index + 2])).normalized()
						tangents[index] = tangent.x
						tangents[index + 1] = tangent.y
						tangents[index + 2] = tangent.z
						tangents[index + 3] *= signf(transform.basis.determinant())
					arrays[Mesh.ARRAY_TANGENT] = tangents
				result.add_surface_from_arrays(instance.mesh.surface_get_primitive_type(surface), arrays)
				result.surface_set_material(result.get_surface_count() - 1, instance.get_active_material(surface))
	for child in node.get_children():
		_bake_node(child, root, transform, result)
