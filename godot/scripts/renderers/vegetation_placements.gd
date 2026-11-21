# SPDX-License-Identifier: GPL-3.0-or-later

## Draws a handful of Rust placements (6 floats each: x, y, z, yaw, scale, species | biased
## variant byte << 2, as `yard_preview_plants` and `get_vegetation_preset_sample` pack them) with the
## game's own meshes, one MultiMeshInstance3D per batch. Rust builds the seed, variant,
## proportions and tint of every plant exactly as a vegetation patch does, here at the origin with
## open stands. Used by the yard preview, terrain thumbnails and point planting ghost.
extends RefCounted

const Vegetation = preload("res://scripts/renderers/vegetation.gd")
# Matches vegetation::edits::FIXED_LOOK in the high bit of the packed variant byte.
const FIXED_LOOK := 1 << 7
const BLOCKED_TINT := Color(2.5, 0.15, 0.15, 1.0)

var _meshes: Array
var _settings: PackedFloat64Array
var _catalogue: PackedFloat32Array

func _init(meshes: Array) -> void:
	_meshes = meshes
	# The helper never enters the tree, so it starts no world streaming or processing.
	var appearance := Vegetation.new()
	appearance.meshes = meshes
	_settings = appearance._product_settings()
	_catalogue = appearance._product_catalogue()
	appearance.free()

## A node holding `placements` drawn with `meshes` (`TreeSpecies.build_meshes()`), the nearest
## level of each.
static func instances(placements: PackedFloat32Array, meshes: Array) -> Node3D:
	var root := Node3D.new()
	var drawing := new(meshes)
	var product: Dictionary = drawing._product(placements)
	var batches: PackedInt32Array = product.get("batches", PackedInt32Array())
	var buffers: Array = product.get("buffers", [])
	for at in range(0, batches.size(), 8):
		# Only the near level; the distant impostors are for a patch seen from afar.
		if batches[at + 1] != 0:
			continue
		var mm := MultiMesh.new()
		mm.transform_format = MultiMesh.TRANSFORM_3D
		mm.use_colors = true
		drawing._set_batch(mm, batches, at, buffers)
		var instance := MultiMeshInstance3D.new()
		instance.multimesh = mm
		root.add_child(instance)
	return root

## Biased mesh pin in a packed placement's last lane, with the fixed-look flag removed.
static func variant_pin(packed: int) -> int:
	return (packed >> Vegetation.SPECIES_BITS) & ~FIXED_LOOK

## Builds the held ghost's fixed look at its root and returns its unblocked tint.
func set_ghost_plant(mm: MultiMesh, placement: PackedFloat32Array) -> Color:
	# Keep the yaw, scale and packed variant. The optional seventh lane is a refusal code.
	var record := placement.slice(0, 6)
	record[0] = 0.0
	record[1] = 0.0
	record[2] = 0.0
	var product := _product(record)
	var batches: PackedInt32Array = product["batches"]
	for at in range(0, batches.size(), 8):
		if batches[at + 1] != 0:
			continue
		_set_batch(mm, batches, at, product["buffers"])
		return mm.get_instance_color(0)
	return Color.WHITE

func _product(placements: PackedFloat32Array) -> Dictionary:
	return SimulationNode.build_vegetation_product(placements, {},
		PackedFloat32Array(), Vector2.ZERO, true, false, _settings, _catalogue)

func _set_batch(mm: MultiMesh, batches: PackedInt32Array, at: int, buffers: Array) -> void:
	var custom := batches[at + 6] == 1
	if mm.use_custom_data != custom:
		mm.instance_count = 0
		mm.use_custom_data = custom
	var mesh: Mesh = _meshes[batches[at]][batches[at + 2]][0]
	if mm.mesh != mesh:
		mm.mesh = mesh
	if mm.instance_count != batches[at + 4]:
		mm.instance_count = batches[at + 4]
	mm.buffer = buffers[batches[at + 7]]
