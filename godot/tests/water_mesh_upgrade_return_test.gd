# SPDX-License-Identifier: GPL-3.0-or-later

## An upgrade mesh sent back for want of room never overwrites the step its patch moved to
## (`TERRAIN-41` L1 landing): a wet patch holding its step 1 mesh asks for step 4, the step 4 mesh
## waits in the apply queue, the patch moves on to step 8, and the room runs out. Returning the
## step 4 mesh as a request used to replace the patch's request for step 8, which the next submit
## then dropped as stale: the patch kept its step 1 mesh with nothing in flight (the stream
## benchmark's water mesh check found 2 such patches at the shipped Kuopio's far zoom-out settle).
## Headless; the water renderer's books only, no simulation.
extends SceneTree

const WaterRenderer := preload("res://scripts/renderers/water.gd")

var _failures := 0

func _initialize() -> void:
	var water: Node = WaterRenderer.new()
	var key := Vector2i(3, 5)
	water.patches[key] = {
		"lod_step": 1, "depth_nonzero_count": 10, "cover_generation": 2, "depth_signature": 7,
		"held_mesh": PackedInt64Array([1, 2, 7]), "material": ShaderMaterial.new(),
	}
	water.resident_patch_lookup[key] = true
	water._set_patch_mesh_lod(key, 4)
	_expect(int(water.mesh_refresh_requested_lod.get(key, 0)) == 4, "a new step asks for its mesh")
	# Submitted and polled: the step 4 mesh waits in the apply queue.
	water.mesh_refresh_requested_lod.erase(key)
	water.mesh_refresh_upgrade.erase(key)
	water.mesh_apply_queue.append(_mesh(key, 4))
	water._set_patch_mesh_lod(key, 8)
	_expect(int(water.mesh_refresh_requested_lod.get(key, 0)) == 8, "the next step asks for its mesh")
	_expect(water._return_upgrade_meshes() == 0, "a mesh of a step its patch left is not asked again")
	_expect(int(water.mesh_refresh_requested_lod.get(key, 0)) == 8,
		"the patch's request for its own step must survive the return")
	_expect(water.mesh_apply_queue.is_empty(), "the stale mesh leaves the apply queue")
	# A mesh still of its patch's step goes back as that step's request.
	water.mesh_refresh_requested_lod.erase(key)
	water.mesh_refresh_upgrade.erase(key)
	water.mesh_apply_queue.append(_mesh(key, 8))
	_expect(water._return_upgrade_meshes() == 1, "an upgrade of the patch's step goes back")
	_expect(int(water.mesh_refresh_requested_lod.get(key, 0)) == 8, "and is asked for again")
	water.free()
	print("Water mesh upgrade return tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

func _mesh(key: Vector2i, step: int) -> Dictionary:
	return {"patch_x": key.x, "patch_z": key.y, "lod_step": step, "cover_generation": 2,
		"depth_signature": 7}

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)
