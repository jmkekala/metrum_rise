# SPDX-License-Identifier: GPL-2.0-only

## Native coverage publication, cross-patch edit invalidation and terrain texture reuse.
extends SceneTree

const TerrainScript := preload("res://scripts/renderers/terrain.gd")
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _nonzero(data: PackedByteArray) -> bool:
	for value in data:
		if value > 0:
			return true
	return false

func _run() -> void:
	var simulation := SimulationNode.new()
	root.add_child(simulation)
	simulation.set_simulation_speed(0.0)
	_expect(simulation.create_blank_world(1020.0, 1020.0, 10.0, 510.0, 20.0), "create coverage fixture")
	simulation.remove_vegetation_at(Vector2.ZERO, 1024.0)
	# Exercise the real upload helpers without starting terrain residency or the game scene.
	var terrain := TerrainScript.new()
	terrain.simulation_node = simulation
	var keys: Array[Vector2i] = [Vector2i(0, 1), Vector2i(1, 1)]
	for key in keys:
		var material := ShaderMaterial.new()
		material.shader = TerrainScript.TERRAIN_SHADER
		var patch := {"material": material, "land_cover": {}, "spare_land_cover": {}}
		terrain.patches[key] = patch
		terrain._commit_patch_land_cover(patch, terrain._stage_patch_land_cover(key, patch))
		_expect(not _nonzero(patch["land_cover"]["image"].get_data()), "clear-cut must publish zero coverage")
		_expect(simulation.is_vegetation_land_cover_current(key, patch["land_cover"]["generations"]), "initial coverage must be current")
	var left: Dictionary = terrain.patches[keys[0]]
	var original_texture: ImageTexture = left["land_cover"]["texture"]
	var target := Vector2(0.25, 17.0)
	_expect(simulation.add_vegetation_at(target, 0), "plant boundary canopy")
	for key in keys:
		_expect(not simulation.is_vegetation_land_cover_current(key, terrain.patches[key]["land_cover"]["generations"]), "neighbor edit must invalidate both textures")
	var staged: Dictionary = terrain._stage_patch_land_cover(keys[0], left)
	_expect(_nonzero(staged["image"].get_data()), "accepted boundary tree must publish coverage")
	_expect(not _nonzero(left["land_cover"]["image"].get_data()), "staging must leave active image unchanged")
	_expect(left["material"].get_shader_parameter("land_cover_texture") == original_texture, "staging must leave active material unchanged")
	terrain._commit_patch_land_cover(left, staged)
	terrain._sync_land_cover()
	for key in keys:
		var cover: Dictionary = terrain.patches[key]["land_cover"]
		_expect(cover["image"].get_format() == Image.FORMAT_R8, "coverage must upload as R8")
		_expect(_nonzero(cover["image"].get_data()), "crown must contribute across the patch boundary")
		_expect(simulation.is_vegetation_land_cover_current(key, cover["generations"]), "both edited patches must refresh")
	_expect(simulation.remove_vegetation_at(target, 0.01) == 1, "remove boundary tree")
	terrain._sync_land_cover()
	terrain._sync_land_cover()
	_expect(left["land_cover"]["texture"] == original_texture, "third upload must reuse original ImageTexture")
	for key in keys:
		_expect(not _nonzero(terrain.patches[key]["land_cover"]["image"].get_data()), "removal must clear both textures")
	# Terrain revisions are independent of plant revisions and must also invalidate coverage.
	simulation.sculpt_terrain(target, 16.0, 1.0)
	for key in keys:
		_expect(not simulation.is_vegetation_land_cover_current(key, terrain.patches[key]["land_cover"]["generations"]), "terrain edit must invalidate both textures")
	terrain._sync_land_cover()
	terrain._sync_land_cover()
	for key in keys:
		_expect(simulation.is_vegetation_land_cover_current(key, terrain.patches[key]["land_cover"]["generations"]), "terrain edit must refresh both textures")
	terrain.free()
	simulation.queue_free()
	await process_frame
	if _failures == 0:
		print("PASS vegetation land cover publication, boundary edits and texture reuse")
	quit(1 if _failures else 0)
