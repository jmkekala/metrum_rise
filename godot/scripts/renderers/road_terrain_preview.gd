# SPDX-License-Identifier: GPL-2.0-only

## Reversible plan-owned terrain display. Rust supplies geometry and planned ownership.
## Reuses terrain validation/mesh builders; never changes payload caches, samples or acknowledgments.
extends RefCounted

var request_id: int = 0
var _patches: Dictionary = {}
var _renderer: Node3D
var _invalidated: Callable

func stage(terrain: Node3D, payloads: Variant, generation: int) -> Array:
	if not is_instance_valid(terrain) or not terrain.has_signal("patch_render_will_change") or not payloads is Array or payloads.is_empty():
		return []
	var staged: Array = []
	var seen := {}
	for data in payloads:
		if not data is Dictionary or not data.has("patch_x") or not data.has("patch_z"):
			discard(staged)
			return []
		var key := Vector2i(data["patch_x"], data["patch_z"])
		if seen.has(key) or not terrain._terrain_patch_payload_is_stageable(key, data, generation, int(data.get("render_step_mm", 0)), true):
			discard(staged)
			return []
		seen[key] = true
		var patch: Dictionary = terrain.patches[key]
		var original: MeshInstance3D = patch["node"]
		var center := Vector3(float(data["world_origin_x"]) + float(data["world_size_x"]) * 0.5, 0.0, float(data["world_origin_z"]) + float(data["world_size_z"]) * 0.5)
		if original.position != center or patch["world_size_x"] != data["world_size_x"] or patch["world_size_z"] != data["world_size_z"]:
			discard(staged)
			return []
		var mesh: Mesh = terrain._terrain_patch_mesh_from_data(data, int(patch.get("lod_step", 1)), int(patch.get("subdivision_factor", 1)))
		if mesh == null:
			discard(staged)
			return []
		var node := MeshInstance3D.new()
		node.name = "RoadTerrainPreview_%d_%d" % [key.x, key.y]
		node.mesh = mesh
		node.extra_cull_margin = original.extra_cull_margin
		node.cast_shadow = original.cast_shadow
		var material: ShaderMaterial = patch["material"].duplicate()
		var image := Image.create_from_data(int(data["texture_width"]), int(data["texture_height"]), false, Image.FORMAT_RF, terrain._terrain_patch_height_bytes(data))
		material.set_shader_parameter("heightmap", ImageTexture.create_from_image(image))
		material.set_shader_parameter("height_is_baked", terrain._terrain_patch_mesh_is_baked(data))
		node.material_override = material
		var walls := MeshInstance3D.new()
		walls.mesh = terrain._retaining_wall_patch_mesh(data)
		walls.material_override = terrain._retaining_wall_material()
		walls.cast_shadow = original.cast_shadow
		walls.extra_cull_margin = original.extra_cull_margin
		node.add_child(walls)
		staged.append({"key": key, "node": node})
	return staged

func commit(terrain: Node3D, staged: Array, id: int, invalidated: Callable) -> void:
	# Called synchronously after the road batch stages successfully, with no await in between.
	clear()
	_invalidated = invalidated
	if not staged.is_empty():
		_renderer = terrain
		_renderer.patch_render_will_change.connect(_patch_will_change)
		_renderer.patches_will_reset.connect(_reset_will_change)
	for entry in staged:
		var patch: Dictionary = terrain.patches[entry["key"]]
		var original: MeshInstance3D = patch["node"]
		var walls: MeshInstance3D = patch["retaining_wall_node"]
		entry["original"] = original
		entry["original_mesh"] = original.mesh
		entry["walls"] = walls
		entry["walls_mesh"] = walls.mesh
		# Keep the resident parent and its culling/visibility, textures and metadata untouched.
		# Only its draw meshes are substituted; the replacement inherits residency visibility.
		original.mesh = null
		walls.mesh = null
		original.add_child(entry["node"])
		_patches[entry["key"]] = entry
	request_id = id

func clear() -> void:
	if is_instance_valid(_renderer):
		_renderer.patch_render_will_change.disconnect(_patch_will_change)
		_renderer.patches_will_reset.disconnect(_reset_will_change)
	for entry in _patches.values():
		if is_instance_valid(entry["original"]):
			entry["original"].mesh = entry["original_mesh"]
		if is_instance_valid(entry["walls"]):
			entry["walls"].mesh = entry["walls_mesh"]
		var node: MeshInstance3D = entry["node"]
		if is_instance_valid(node):
			node.free()
	_patches.clear()
	_renderer = null
	_invalidated = Callable()
	request_id = 0

func discard(staged: Array) -> void:
	for entry in staged:
		entry["node"].free()
	staged.clear()

func _patch_will_change(key: Vector2i) -> void:
	if _patches.has(key) and _invalidated.is_valid():
		_invalidated.call()

func _reset_will_change() -> void:
	if _invalidated.is_valid():
		_invalidated.call()
