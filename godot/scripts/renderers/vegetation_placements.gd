# SPDX-License-Identifier: GPL-3.0-or-later

## Draws a handful of Rust placements (6 floats each: x, y, z, yaw, scale, species | biased
## variant << 2, as `yard_preview_plants` and `get_vegetation_preset_sample` pack them) with the
## game's own meshes, one MultiMeshInstance3D per batch. Rust builds the seed, variant,
## proportions and tint of every plant exactly as a vegetation patch does, here at the origin with
## open stands. For previews of a few dozen plants (the asset editor's yard, the terrain menu's
## thumbnails), not the world.
extends RefCounted

const Vegetation = preload("res://scripts/renderers/vegetation.gd")

## A node holding `placements` drawn with `meshes` (`TreeSpecies.build_meshes()`), the nearest
## level of each.
static func instances(placements: PackedFloat32Array, meshes: Array) -> Node3D:
	var root := Node3D.new()
	# This helper never enters the tree, so it starts no world streaming or processing.
	var appearance := Vegetation.new()
	appearance.meshes = meshes
	var product: Dictionary = SimulationNode.build_vegetation_product(placements, {},
		PackedFloat32Array(), Vector2.ZERO, true, false, appearance._product_settings(),
		appearance._product_catalogue())
	appearance.free()
	var batches: PackedInt32Array = product.get("batches", PackedInt32Array())
	var buffers: Array = product.get("buffers", [])
	for at in range(0, batches.size(), 8):
		# Only the near level; the distant impostors are for a patch seen from afar.
		if batches[at + 1] != 0:
			continue
		var mm := MultiMesh.new()
		mm.transform_format = MultiMesh.TRANSFORM_3D
		mm.use_colors = true
		mm.use_custom_data = batches[at + 6] == 1
		mm.mesh = meshes[batches[at]][batches[at + 2]][0]
		mm.instance_count = batches[at + 4]
		mm.buffer = buffers[batches[at + 7]]
		var instance := MultiMeshInstance3D.new()
		instance.multimesh = mm
		root.add_child(instance)
	return root
