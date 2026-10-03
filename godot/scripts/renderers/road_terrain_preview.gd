# SPDX-License-Identifier: GPL-2.0-only

## Reversible plan-owned terrain display. Rust supplies geometry and planned ownership.
## Reuses terrain validation/mesh builders; never changes payload caches, samples or acknowledgments.
## Preview-owned nodes, materials, images and textures are recycled through detached spare slots.
## Each filled slot keeps its Rust display revision; an `unchanged` payload reuses that slot.
extends RefCounted

var metrics: Dictionary = {}
var request_id: int = 0
# Set when an `unchanged` payload names a revision no slot still holds. The caller must fetch
# the same result again with `revisions()`, which then yields complete payloads.
var missing_revision := false
var _patches: Dictionary = {}
# Uniforms each slot sets from its own heights instead of copying from the resident material.
const SLOT_OWNED_UNIFORMS := [
	"heightmap", "height_is_baked", "terrain_masks", "terrain_mask_layer", "terrain_mask_uv_scale",
]
# Detached slots {node, walls, material, image, texture, texture_size, masks, key, revision, lod}.
# Staging writes only these, never the displayed slots, so a failed batch leaves the previous
# display intact. Trimmed to the displayed count on commit: one staging set, not cursor history.
var _spares: Array[Dictionary] = []
var _renderer: Node3D
var _invalidated: Callable
var _uniform_shader: Shader
var _uniform_names: Array[StringName] = []

func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE:
		# Own methods are unavailable during RefCounted teardown; free detached spares inline.
		for slot in _spares:
			if is_instance_valid(slot["node"]) and slot["node"].get_parent() == null:
				slot["node"].free()

# Returns revisions whose display this helper can reproduce without payload data: displayed
# slots and detached slots whose content survived since they were filled. O(slots).
func revisions() -> PackedInt64Array:
	var held := PackedInt64Array()
	for entry in _patches.values():
		if int(entry["slot"].get("revision", 0)) > 0 and is_instance_valid(entry["slot"]["node"]):
			held.append(entry["slot"]["revision"])
	for slot in _spares:
		if int(slot.get("revision", 0)) > 0 and is_instance_valid(slot["node"]):
			held.append(slot["revision"])
	return held

func stage(terrain: Node3D, payloads: Variant, generation: int) -> Array:
	missing_revision = false
	if not is_instance_valid(terrain) or not terrain.has_signal("patch_render_will_change") or not payloads is Array or payloads.is_empty():
		return []
	var validation_us := Time.get_ticks_usec() if not metrics.is_empty() else 0
	# Check residency before constructing any GPU resources. Streaming may need another frame.
	# Unchanged payloads carry metadata only; their geometry was validated when first staged.
	var held := {}
	for data in payloads:
		if not data is Dictionary or not data.has("patch_x") or not data.has("patch_z"):
			return []
		var key := Vector2i(data["patch_x"], data["patch_z"])
		if data.get("unchanged", false):
			if not _metadata_is_stageable(terrain, key, data, generation) or held.has(key):
				return []
			var slot := _held_slot(terrain, key, int(data["terrain_revision"]))
			if slot.is_empty():
				missing_revision = true
				return []
			held[key] = slot
		elif not terrain._terrain_patch_payload_is_stageable(key, data, generation, int(data.get("render_step_mm", 0)), true):
			return []
	if not metrics.is_empty():
		metrics["terrain_preflight_ms"] = float(Time.get_ticks_usec() - validation_us) / 1000.0
	var staged: Array = []
	var seen := {}
	# Claim held slots before fresh payloads take spares, so no fill overwrites a reused slot.
	for data in payloads:
		var key := Vector2i(data["patch_x"], data["patch_z"])
		if not held.has(key):
			continue
		seen[key] = true
		if not _matches_resident(terrain.patches[key], data):
			discard(staged)
			return []
		var slot: Dictionary = held[key]
		var carried: bool = _patches.has(key) and is_same(_patches[key]["slot"], slot)
		if not carried:
			# A detached slot is not displayed, so it can be brought up to date now. A displayed
			# slot is only resynchronized in commit, after the whole batch has staged.
			_spares.erase(slot)
			_mirror_resident(terrain, slot, terrain.patches[key])
		staged.append({"key": key, "slot": slot, "carried": carried})
		if not metrics.is_empty():
			metrics["terrain_patches_reused"] = metrics.get("terrain_patches_reused", 0) + 1
	for data in payloads:
		if not data is Dictionary or not data.has("patch_x") or not data.has("patch_z"):
			discard(staged)
			return []
		var key := Vector2i(data["patch_x"], data["patch_z"])
		if held.has(key):
			continue
		if seen.has(key) or not terrain._terrain_patch_payload_is_stageable(key, data, generation, int(data.get("render_step_mm", 0)), true):
			discard(staged)
			return []
		seen[key] = true
		var patch: Dictionary = terrain.patches[key]
		if not _matches_resident(patch, data):
			discard(staged)
			return []
		var cached_meshes_before: int = terrain.patch_mesh_cache.size() if not metrics.is_empty() else 0
		var mesh_us := Time.get_ticks_usec() if not metrics.is_empty() else 0
		var mesh: Mesh = terrain._terrain_patch_mesh_from_data(data, int(patch.get("lod_step", 1)), int(patch.get("subdivision_factor", 1)))
		if mesh == null:
			discard(staged)
			return []
		if not metrics.is_empty():
			metrics["terrain_mesh_ms"] = metrics.get("terrain_mesh_ms", 0.0) + float(Time.get_ticks_usec() - mesh_us) / 1000.0
		var texture_us := Time.get_ticks_usec() if not metrics.is_empty() else 0
		var slot := _take_slot()
		var reused := slot.has("image")
		var node: MeshInstance3D = slot["node"]
		node.name = "RoadTerrainPreview_%d_%d" % [key.x, key.y]
		node.mesh = mesh
		var material: ShaderMaterial = slot["material"]
		# Mirror the resident material as a fresh duplicate would; only the heights differ.
		_mirror_resident(terrain, slot, patch)
		var size := Vector2i(int(data["texture_width"]), int(data["texture_height"]))
		var image: Image = slot.get("image", Image.new())
		image.set_data(size.x, size.y, false, Image.FORMAT_RF, terrain._terrain_patch_height_bytes(data))
		slot["image"] = image
		# Matching dimensions/format/mipmaps update the existing allocation in place.
		var texture_reused: bool = slot.get("texture") != null and slot["texture_size"] == size
		if texture_reused:
			slot["texture"].update(image)
		else:
			slot["texture"] = ImageTexture.create_from_image(image)
			slot["texture_size"] = size
			material.set_shader_parameter("heightmap", slot["texture"])
		material.set_shader_parameter("height_is_baked", terrain._terrain_patch_mesh_is_baked(data))
		_fill_slot_masks(slot, terrain._terrain_patch_mask_image(data), size)
		var walls: MeshInstance3D = slot["walls"]
		walls.mesh = terrain._retaining_wall_patch_mesh(data)
		# The filled content now reproduces this revision at this resident level of detail.
		slot["key"] = key
		slot["revision"] = int(data.get("terrain_revision", 0))
		slot["lod"] = _resident_lod(patch)
		staged.append({"key": key, "slot": slot})
		if not metrics.is_empty():
			metrics["terrain_resources_ms"] = metrics.get("terrain_resources_ms", 0.0) + float(Time.get_ticks_usec() - texture_us) / 1000.0
			metrics["terrain_patches_created"] = metrics.get("terrain_patches_created", 0) + 1
			# Baked ArrayMeshes are fresh; regular PlaneMeshes come from the existing cache.
			var created_meshes := int(mesh is ArrayMesh) + int(terrain.patch_mesh_cache.size()) - cached_meshes_before
			metrics["terrain_meshes_created"] = metrics.get("terrain_meshes_created", 0) + created_meshes + int(walls.mesh != null)
			metrics["terrain_meshes_reused"] = metrics.get("terrain_meshes_reused", 0) + 1 - created_meshes
			var resources := "reused" if reused else "created"
			metrics["terrain_nodes_" + resources] = metrics.get("terrain_nodes_" + resources, 0) + 2
			metrics["terrain_materials_" + resources] = metrics.get("terrain_materials_" + resources, 0) + 1
			metrics["terrain_images_" + resources] = metrics.get("terrain_images_" + resources, 0) + 1
			var textures := "terrain_textures_reused" if texture_reused else "terrain_textures_created"
			metrics[textures] = metrics.get(textures, 0) + 1
	return staged

func commit(terrain: Node3D, staged: Array, id: int, invalidated: Callable) -> void:
	var install_us := Time.get_ticks_usec() if not metrics.is_empty() else 0
	# Called synchronously after the road batch stages successfully, with no await in between.
	# Carried slots stay attached; clear() restores only the patches this batch drops.
	var carried := {}
	for entry in staged:
		if entry.get("carried", false):
			carried[entry["key"]] = _patches[entry["key"]]
			_patches.erase(entry["key"])
	clear()
	_invalidated = invalidated
	if not staged.is_empty():
		_renderer = terrain
		_renderer.patch_render_will_change.connect(_patch_will_change)
		_renderer.patches_will_reset.connect(_reset_will_change)
	for entry in staged:
		if carried.has(entry["key"]):
			# The resident material or shadow settings may have changed since this slot was filled.
			_mirror_resident(terrain, entry["slot"], terrain.patches[entry["key"]])
			_patches[entry["key"]] = carried[entry["key"]]
			continue
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
		original.add_child(entry["slot"]["node"])
		_patches[entry["key"]] = entry
	request_id = id
	while _spares.size() > _patches.size():
		_free_slot(_spares.pop_back())
	if not metrics.is_empty():
		metrics["terrain_install_ms"] = float(Time.get_ticks_usec() - install_us) / 1000.0

func clear() -> void:
	if is_instance_valid(_renderer):
		_renderer.patch_render_will_change.disconnect(_patch_will_change)
		_renderer.patches_will_reset.disconnect(_reset_will_change)
	for entry in _patches.values():
		if is_instance_valid(entry["original"]):
			entry["original"].mesh = entry["original_mesh"]
		if is_instance_valid(entry["walls"]):
			entry["walls"].mesh = entry["walls_mesh"]
		# A freed resident parent frees the child slot too; _take_slot skips such slots.
		var node: MeshInstance3D = entry["slot"]["node"]
		if is_instance_valid(node) and node.get_parent() != null:
			node.get_parent().remove_child(node)
		_spares.append(entry["slot"])
	_patches.clear()
	_renderer = null
	_invalidated = Callable()
	request_id = 0

# Releases the display and every spare slot. Use when the preview session ends.
func reset() -> void:
	clear()
	_free_spares()

func discard(staged: Array) -> void:
	for entry in staged:
		# A carried slot is still displayed; only detached slots return to the spares.
		if not entry.get("carried", false):
			_spares.append(entry["slot"])
	staged.clear()

func _take_slot() -> Dictionary:
	while not _spares.is_empty():
		var slot: Dictionary = _spares.pop_back()
		if is_instance_valid(slot["node"]):
			# Its previous content is about to be overwritten.
			slot["revision"] = 0
			return slot
	var node := MeshInstance3D.new()
	var walls := MeshInstance3D.new()
	node.add_child(walls)
	node.material_override = ShaderMaterial.new()
	return {"node": node, "walls": walls, "material": node.material_override}

# Finds the displayed or detached slot holding this revision. Its mesh was built for one
# resident level of detail, and a displayed slot hangs under the resident node it replaced.
# A slot failing either condition forgets its revision, so revisions() stops offering it and
# the caller's refetch receives complete data. O(slots).
func _held_slot(terrain: Node3D, key: Vector2i, revision: int) -> Dictionary:
	var resident: Dictionary = terrain.patches[key]
	var displayed: Dictionary = _patches[key]["slot"] if _patches.has(key) else {}
	var candidates: Array = [displayed] + _spares if not displayed.is_empty() else _spares.duplicate()
	for slot in candidates:
		if revision <= 0 or slot.get("revision", 0) != revision or slot.get("key") != key or not is_instance_valid(slot["node"]):
			continue
		var attached_current := not is_same(slot, displayed) or is_same(_patches[key]["original"], resident["node"])
		if slot.get("lod") == _resident_lod(resident) and attached_current:
			return slot
		slot["revision"] = 0
	return {}

# Residency and generation checks for a metadata-only payload; its buffers are not re-sent.
func _metadata_is_stageable(terrain: Node3D, key: Vector2i, data: Dictionary, generation: int) -> bool:
	if not terrain.patches.has(key):
		return false
	var patch: Dictionary = terrain.patches[key]
	return (
		patch.get("node", null) is MeshInstance3D
		and patch.get("retaining_wall_node", null) is MeshInstance3D
		and patch.get("material", null) is ShaderMaterial
		and typeof(data.get("surface_generation", null)) == TYPE_INT
		and int(data["surface_generation"]) == generation
		and typeof(data.get("terrain_revision", null)) == TYPE_INT
		and terrain._terrain_numeric_field_is_finite(data, "world_origin_x")
		and terrain._terrain_numeric_field_is_finite(data, "world_origin_z")
		and terrain._terrain_numeric_field_is_finite(data, "world_size_x")
		and terrain._terrain_numeric_field_is_finite(data, "world_size_z")
	)

func _matches_resident(patch: Dictionary, data: Dictionary) -> bool:
	var center := Vector3(float(data["world_origin_x"]) + float(data["world_size_x"]) * 0.5, 0.0, float(data["world_origin_z"]) + float(data["world_size_z"]) * 0.5)
	return patch["node"].position == center and patch["world_size_x"] == data["world_size_x"] and patch["world_size_z"] == data["world_size_z"]

func _resident_lod(patch: Dictionary) -> Vector2i:
	return Vector2i(int(patch.get("lod_step", 1)), int(patch.get("subdivision_factor", 1)))

# Copies resident draw settings and material values; the preview heights stay slot-owned.
func _mirror_resident(terrain: Node3D, slot: Dictionary, patch: Dictionary) -> void:
	var original: MeshInstance3D = patch["node"]
	var node: MeshInstance3D = slot["node"]
	var walls: MeshInstance3D = slot["walls"]
	node.extra_cull_margin = original.extra_cull_margin
	node.cast_shadow = original.cast_shadow
	walls.material_override = terrain._retaining_wall_material()
	walls.cast_shadow = original.cast_shadow
	walls.extra_cull_margin = original.extra_cull_margin
	if _sync_material(slot["material"], patch["material"]) and slot.get("texture") != null:
		slot["material"].set_shader_parameter("heightmap", slot["texture"])
		_bind_slot_masks(slot)

# The preview's own relief and cliff-reach masks, baked from its heights: the resident patch's
# describe the terrain it replaces. A one-layer array per slot, updated in place while the size
# holds; a preview shows a handful of patches, so these few textures cost nothing measurable.
func _fill_slot_masks(slot: Dictionary, image: Image, texture_size: Vector2i) -> void:
	slot["mask_uv_scale"] = Vector2.ONE
	if image == null:
		slot.erase("masks")
	else:
		var masks: Texture2DArray = slot.get("masks", null)
		if masks != null and Vector2i(masks.get_width(), masks.get_height()) == image.get_size():
			masks.update_layer(image, 0)
		else:
			masks = Texture2DArray.new()
			var layers: Array[Image] = [image]
			masks.create_from_images(layers)
			slot["masks"] = masks
		slot["mask_uv_scale"] = Vector2(texture_size) / Vector2(image.get_size())
	_bind_slot_masks(slot)

func _bind_slot_masks(slot: Dictionary) -> void:
	var material: ShaderMaterial = slot["material"]
	var masks: Texture2DArray = slot.get("masks", null)
	material.set_shader_parameter("terrain_masks", masks)
	material.set_shader_parameter("terrain_mask_layer", 0.0 if masks != null else -1.0)
	material.set_shader_parameter("terrain_mask_uv_scale", slot.get("mask_uv_scale", Vector2.ONE))

# O(shader uniforms) reads; writes only changed values so reused materials stay clean.
# Returns true when the shader changed and the heightmap must be assigned again.
func _sync_material(target: ShaderMaterial, source: ShaderMaterial) -> bool:
	var shader_changed := target.shader != source.shader
	if shader_changed:
		target.shader = source.shader
	if _uniform_shader != source.shader:
		_uniform_shader = source.shader
		_uniform_names.clear()
		if _uniform_shader != null:
			for uniform in _uniform_shader.get_shader_uniform_list():
				if not uniform["name"] in SLOT_OWNED_UNIFORMS:
					_uniform_names.append(StringName(uniform["name"]))
	for name in _uniform_names:
		var value = source.get_shader_parameter(name)
		var current = target.get_shader_parameter(name)
		if typeof(value) != typeof(current) or value != current:
			target.set_shader_parameter(name, value)
	return shader_changed

func _free_slot(slot: Dictionary) -> void:
	var node = slot["node"]
	if is_instance_valid(node):
		if node.get_parent() != null:
			node.get_parent().remove_child(node)
		node.free()

func _free_spares() -> void:
	for slot in _spares:
		_free_slot(slot)
	_spares.clear()

func _patch_will_change(key: Vector2i) -> void:
	if _patches.has(key) and _invalidated.is_valid():
		_invalidated.call()

func _reset_will_change() -> void:
	if _invalidated.is_valid():
		_invalidated.call()
