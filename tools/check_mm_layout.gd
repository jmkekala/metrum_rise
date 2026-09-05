# SPDX-License-Identifier: GPL-2.0-only

extends SceneTree

func _init():
	var mm = MultiMesh.new()
	mm.transform_format = MultiMesh.TRANSFORM_3D
	mm.instance_count = 1
	var tf = Transform3D(Basis(Vector3(1, 4, 7), Vector3(2, 5, 8), Vector3(3, 6, 9)), Vector3(10, 11, 12))
	mm.set_instance_transform(0, tf)
	var buffer = mm.buffer
	print("Godot 4 MultiMesh TRANSFORM_3D Buffer Layout: ", buffer)
	quit()

