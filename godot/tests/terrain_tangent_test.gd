# SPDX-License-Identifier: GPL-2.0-only

## Rendered (Forward+): every terrain surface variant draws the graded renderer's tangent-less
## cell meshes without Godot's "requires tangents" warning (`ROAD-68-H` round 3). A shader that
## reads TANGENT makes Godot warn each time it binds a mesh without tangents, once a pose in a
## populated drag; the base variant's lamp light read the tangent basis it never uses.
extends SceneTree

const SHADERS := [
	"res://assets/materials/terrain.gdshader",
	"res://assets/materials/terrain_graded.gdshader",
	"res://assets/materials/terrain_graded_behind.gdshader",
	"res://assets/materials/terrain_graded_walls.gdshader",
]
const WARNING := "requires tangents"

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

# A lattice like the graded cell meshes (`terrain_graded_tiles.gd` _build_meshes): positions and
# indices only.
static func _lattice() -> ArrayMesh:
	var n := 5
	var vertices := PackedVector3Array()
	for j in n:
		for i in n:
			vertices.append(Vector3(i, 0.0, j))
	var indices := PackedInt32Array()
	for j in n - 1:
		for i in n - 1:
			var a := j * n + i
			indices.append_array(PackedInt32Array([a, a + 1, a + n + 1, a, a + n + 1, a + n]))
	var arrays := []
	arrays.resize(Mesh.ARRAY_MAX)
	arrays[Mesh.ARRAY_VERTEX] = vertices
	arrays[Mesh.ARRAY_INDEX] = indices
	var mesh := ArrayMesh.new()
	mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	return mesh

func _log_warnings(path: String) -> int:
	var text := FileAccess.get_file_as_string(path)
	return text.count(WARNING)

func _run() -> void:
	var log_path := ProjectSettings.globalize_path(str(ProjectSettings.get_setting("debug/file_logging/log_path")))
	_expect(FileAccess.file_exists(log_path), "the test reads Godot's file log: %s" % log_path)
	var camera := Camera3D.new()
	root.add_child(camera)
	camera.position = Vector3(2.0, 10.0, 2.0)
	camera.look_at(Vector3(2.0, 0.0, 2.01))
	var mesh := _lattice()
	_expect((mesh.surface_get_format(0) & Mesh.ARRAY_FORMAT_TANGENT) == 0, "the lattice has no tangents, as the cell meshes")
	var before := _log_warnings(log_path)
	for path in SHADERS:
		var material := ShaderMaterial.new()
		material.shader = load(path)
		var node := MultiMeshInstance3D.new()
		node.multimesh = MultiMesh.new()
		node.multimesh.transform_format = MultiMesh.TRANSFORM_3D
		node.multimesh.mesh = mesh
		node.multimesh.instance_count = 1
		node.multimesh.set_instance_transform(0, Transform3D())
		node.material_override = material
		root.add_child(node)
		for frame in 3:
			await process_frame
		await RenderingServer.frame_post_draw
		var warnings := _log_warnings(log_path) - before
		_expect(warnings == 0, "%s draws a tangent-less cell mesh without the tangent warning (%d)" % [path, warnings])
		before += warnings
		node.queue_free()
		await process_frame
	if _failures == 0:
		print("terrain_tangent_test: PASS")
	quit(_failures)
