# SPDX-License-Identifier: GPL-2.0-only

## Graded terrain tiles (TERRAIN-07-D, docs/terrain.md "Godot tile renderer"): residency of the
## graded 64 m tiles of the resident patches by camera distance, the slot pools (per level one
## Texture2DArray of RGBAF layers for the texels and one for the rows, prewarmed), in-place
## uploads from one pooled Image per level, and per patch one MultiMesh per cell mesh (the base
## cells and GL0-GL3) whose instances carry the tile key, slot and stitching codes. It does no
## geometry: positions, stitching, clamping and the mask are terrain_cells.gdshaderinc's, the
## values Rust's. Nothing is created per edit; the counters in get_pool_counters() prove it.
##
## Rust methods called: get_graded_tile_layout(), request_graded_patch_tiles(),
##   request_graded_tiles(), poll_graded_tile_results()
extends RefCounted

const GRADED_SHADER := preload("res://assets/materials/terrain_graded.gdshader")
const TILE_M := 64.0
# Level distances with their hysteresis (T5, TD10): GL0 to 256 m, GL1 to 512 m, GL2 to 2 km,
# GL3 beyond, at the patch LOD rule's steps (2 to 5 km, 4 to 12 km, 8 beyond).
const LEVEL_END_M := [256.0, 512.0, 2000.0]
const GL3_STEP_END_M := [[2, 5000.0], [4, 12000.0], [8, INF]]
const LEVEL_HYSTERESIS_M := 32.0
# Slots per level. GL0 and GL1 follow the memory table; GL2 and GL3 stop at 2,048 layers, the
# smallest Texture2DArray depth Vulkan guarantees (TERRAIN-07 open question T23).
const DEFAULT_POOL_SLOTS := [64, 192, 2048, 2048]
# Patch cells: one 510 m patch meets at most 9 x 9 tiles.
const MAX_CELLS_PER_PATCH := 100
const REQUESTS_PER_FRAME := 24
const INSTALL_BUDGET_MS := 1.0
const POLL_BATCH := 4
const CELL_MESHES := 5
const CELL_BASE := 4

var simulation_node: Node
var terrain_cell_m := 10.0
# The patch grid: patches of patch_span_m from the world's lower corner.
var patch_span_m := 510.0
var world_min := Vector2.ZERO
var lattice_anchor_hm := Vector2i.ZERO
var levels: Array[Dictionary] = []
# Per level: texels/rows Texture2DArray, the pooled upload Images, the free slots.
var pools: Array[Dictionary] = []
# Tile key -> {code, level, step, slot, token}: what each installed tile shows.
var installed: Dictionary = {}
# "x,z,level,step" -> token of a bake in flight.
var requested: Dictionary = {}
# Patch key -> {token, tiles: Array[Vector2i]}; patch key -> token of a list in flight.
var patch_lists: Dictionary = {}
var patch_lists_requested: Dictionary = {}
# Patch key -> {nodes: Array[MultiMeshInstance3D], material: ShaderMaterial, watched: Array}.
var patch_cells: Dictionary = {}
var _cells_pool: Array[Dictionary] = []
var _meshes: Dictionary = {}
var _dirty_patches: Dictionary = {}
var _uniform_names: PackedStringArray = PackedStringArray()
# The desired levels are recomputed when the camera moved or the lists or residency changed.
var _desired: Dictionary = {}
var _desired_camera := Vector3(INF, INF, INF)
var _desired_signature := -1
var _desired_dirty := true
const DESIRED_REFRESH_MOVE_M := 8.0
var counters := {
	"textures_created": 0,
	"images_created": 0,
	"meshes_created": 0,
	"materials_created": 0,
	"multimeshes_created": 0,
	"uploads": 0,
	"evictions": 0,
	"pool_exhausted": 0,
	"bake_failures": 0,
	"install_ms_last": 0.0,
	"install_ms_max": 0.0,
	"poll_ms_max": 0.0,
	"upload_ms_max": 0.0,
}

## Reads the slot layout and prewarms the pools and cell meshes. `pool_slots` overrides the
## per-level slot counts (tests use small pools).
func setup(simulation: Node, cell_m: float, world_size: Vector2, span_m: float, pool_slots: Array = DEFAULT_POOL_SLOTS) -> void:
	simulation_node = simulation
	terrain_cell_m = cell_m
	patch_span_m = span_m
	world_min = -world_size * 0.5
	# The base lattice starts at the negative half extents (BaseGrid), in half-metres.
	lattice_anchor_hm = Vector2i(int(round(-world_size.x)), int(round(-world_size.y)))
	var layout: Dictionary = simulation_node.get_graded_tile_layout()
	levels.clear()
	for level: Dictionary in layout.get("levels", []):
		levels.append(level)
	pools.clear()
	for code in levels.size():
		pools.append(_new_pool(levels[code], int(pool_slots[code])))
	_build_meshes()

func _new_pool(level: Dictionary, slots: int) -> Dictionary:
	var side := int(level["side"])
	var row_texels := int(level["row_texels"])
	var texel_layers: Array[Image] = []
	var row_layers: Array[Image] = []
	for _slot in slots:
		texel_layers.append(Image.create_empty(side, side, false, Image.FORMAT_RGBAF))
		row_layers.append(Image.create_empty(row_texels, 1, false, Image.FORMAT_RGBAF))
	var texels := Texture2DArray.new()
	texels.create_from_images(texel_layers)
	var rows := Texture2DArray.new()
	rows.create_from_images(row_layers)
	counters["textures_created"] += 2
	counters["images_created"] += 2 * slots + 2
	var free: Array[int] = []
	for slot in range(slots - 1, -1, -1):
		free.append(slot)
	return {
		"side": side,
		"row_texels": row_texels,
		"texels": texels,
		"rows": rows,
		"texel_image": Image.create_empty(side, side, false, Image.FORMAT_RGBAF),
		"row_image": Image.create_empty(row_texels, 1, false, Image.FORMAT_RGBAF),
		"free": free,
		"slots": slots,
	}

# Cell mesh per lattice code: vertex k per axis carries its lattice index in VERTEX.xz (the
# shader places it); each lattice cell is split along its (i, j)-(i + 1, j + 1) diagonal, as the
# graded queries interpolate.
func _build_meshes() -> void:
	_meshes.clear()
	var cell_hm := int(round(terrain_cell_m * 2.0))
	for code in 7:
		var spacing_hm: int = (1 << code) if code < 3 else (cell_hm << (code - 3))
		var n: int = (128 / spacing_hm + 1) if code < 3 else (int(ceil(128.0 / float(spacing_hm))) + 2)
		var vertices := PackedVector3Array()
		vertices.resize(n * n)
		for j in n:
			for i in n:
				vertices[j * n + i] = Vector3(i, 0.0, j)
		var indices := PackedInt32Array()
		indices.resize((n - 1) * (n - 1) * 6)
		var at := 0
		for j in n - 1:
			for i in n - 1:
				var a := j * n + i
				for index in [a, a + 1, a + n + 1, a, a + n + 1, a + n]:
					indices[at] = index
					at += 1
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = vertices
		arrays[Mesh.ARRAY_INDEX] = indices
		var mesh := ArrayMesh.new()
		mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
		_meshes[code] = mesh
		counters["meshes_created"] += 1

## The lattice code of the base cells and GL3 at a patch LOD step: 3 + log2(step).
static func base_code(step: int) -> int:
	return 3 + int(round(log(float(max(step, 1))) / log(2.0)))

## The level code (0-3) and GL3 step a tile takes at `distance_m`, with hysteresis around its
## current level.
func desired_level(distance_m: float, current_code: int) -> Vector2i:
	var code := 3
	for index in LEVEL_END_M.size():
		var end_m: float = LEVEL_END_M[index]
		if current_code >= 0 and current_code <= index:
			end_m += LEVEL_HYSTERESIS_M
		if distance_m < end_m:
			code = index
			break
	if code < 3:
		return Vector2i(code, 0)
	for entry: Array in GL3_STEP_END_M:
		if distance_m < float(entry[1]):
			return Vector2i(3, int(entry[0]))
	return Vector2i(3, 8)

## Frees every slot and cell resource, as the renderer resets.
func clear() -> void:
	for key in installed.keys():
		_free_slot(installed[key])
	installed.clear()
	requested.clear()
	patch_lists.clear()
	patch_lists_requested.clear()
	for key in patch_cells.keys():
		release_patch(key, null)

## One frame: polls finished work and installs it within the install budget, keeps the tiles of
## `resident_patches` (patch key -> {node, material, rect: Rect2, step, generation}) at their
## distance's level, evicts the rest and rewrites the instances of the patches that changed.
func process(camera_position: Vector3, resident_patches: Dictionary) -> void:
	var start_us := Time.get_ticks_usec()
	_poll_and_install(start_us, resident_patches)
	counters["install_ms_last"] = float(Time.get_ticks_usec() - start_us) / 1000.0
	counters["install_ms_max"] = max(float(counters["install_ms_max"]), float(counters["install_ms_last"]))
	_request_patch_lists(resident_patches)
	var signature := resident_patches.size()
	for key: Vector2i in resident_patches.keys():
		signature = signature * 31 + int(resident_patches[key]["generation"]) + key.x * 7 + key.y * 13
	if (
		_desired_dirty
		or signature != _desired_signature
		or camera_position.distance_to(_desired_camera) >= DESIRED_REFRESH_MOVE_M
	):
		_desired = _desired_tiles(camera_position, resident_patches)
		_desired_camera = camera_position
		_desired_signature = signature
		_desired_dirty = false
	_request_tiles(_desired)
	_evict(_desired)
	for key: Vector2i in resident_patches.keys():
		var patch: Dictionary = resident_patches[key]
		var cells: Dictionary = patch_cells.get(key, {})
		if cells.is_empty():
			continue
		if int(cells.get("step", -1)) != int(patch["step"]):
			_dirty_patches[key] = true
		elif _watched_parameters(patch["material"]) != cells["watched"]:
			_sync_material(cells, patch["material"])
	for key: Vector2i in _dirty_patches.keys():
		if resident_patches.has(key):
			_rebuild_patch_cells(key, resident_patches)
	_dirty_patches.clear()

func _poll_and_install(start_us: int, resident_patches: Dictionary) -> void:
	while true:
		var poll_start_us := Time.get_ticks_usec()
		var result: Dictionary = simulation_node.poll_graded_tile_results(POLL_BATCH)
		counters["poll_ms_max"] = max(float(counters["poll_ms_max"]), float(Time.get_ticks_usec() - poll_start_us) / 1000.0)
		for list: Dictionary in result.get("patches", []):
			var key := Vector2i(int(list["patch_x"]), int(list["patch_z"]))
			patch_lists_requested.erase(key)
			var tiles: Array[Vector2i] = []
			var flat: PackedInt32Array = list["tiles"]
			for index in range(0, flat.size(), 2):
				tiles.append(Vector2i(flat[index], flat[index + 1]))
			patch_lists[key] = {"token": int(list["token"]), "tiles": tiles}
			_dirty_patches[key] = true
			_desired_dirty = true
		for failure: Dictionary in result.get("failed", []):
			var keys: PackedInt64Array = failure["keys"]
			if String(failure["kind"]) == "patch":
				patch_lists_requested.erase(Vector2i(keys[0], keys[1]))
			else:
				requested.erase("%d,%d,%d,%d" % [keys[0], keys[1], keys[2], keys[3]])
			if not bool(failure["retry"]):
				counters["bake_failures"] += 1
				MetrumDebug.log("terrain", "graded %s %s failed: %s" % [failure["kind"], str(keys), failure["reason"]])
		var tiles: Array = result.get("tiles", [])
		for tile: Dictionary in tiles:
			_install(tile, resident_patches)
		if tiles.size() < POLL_BATCH:
			return
		if float(Time.get_ticks_usec() - start_us) / 1000.0 >= INSTALL_BUDGET_MS:
			return

func _install(tile: Dictionary, resident_patches: Dictionary) -> void:
	var key := Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))
	var code := int(tile["level"])
	var step := int(tile["step"])
	var request_key := "%d,%d,%d,%d" % [key.x, key.y, code, step]
	var token := int(tile["token"])
	if int(requested.get(request_key, -1)) != token:
		return
	requested.erase(request_key)
	var pool: Dictionary = pools[code]
	var free: Array[int] = pool["free"]
	if free.is_empty():
		# The pool sizes make this impossible within the distance rules; counted, the tile stays
		# as it was.
		counters["pool_exhausted"] += 1
		MetrumDebug.log("terrain", "graded pool %d exhausted at tile %s" % [code, str(key)])
		return
	var slot: int = free.pop_back()
	var upload_start_us := Time.get_ticks_usec()
	var side := int(pool["side"])
	var texel_image: Image = pool["texel_image"]
	texel_image.set_data(side, side, false, Image.FORMAT_RGBAF, tile["texels"])
	(pool["texels"] as Texture2DArray).update_layer(texel_image, slot)
	var row_image: Image = pool["row_image"]
	row_image.set_data(int(pool["row_texels"]), 1, false, Image.FORMAT_RGBAF, tile["rows"])
	(pool["rows"] as Texture2DArray).update_layer(row_image, slot)
	counters["upload_ms_max"] = max(float(counters["upload_ms_max"]), float(Time.get_ticks_usec() - upload_start_us) / 1000.0)
	counters["uploads"] += 1
	if installed.has(key):
		_free_slot(installed[key])
	var cell_code := code if code < 3 else base_code(step)
	installed[key] = {"code": cell_code, "level": code, "step": step, "slot": slot, "token": token}
	_mark_tile_patches(key, resident_patches)

func _free_slot(entry: Dictionary) -> void:
	var pool: Dictionary = pools[int(entry["level"])]
	var free: Array[int] = pool["free"]
	free.append(int(entry["slot"]))

# The patch keys whose rectangle meets `rect`, from the patch grid. O(patches met).
func _patches_meeting(rect: Rect2) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	var first := ((rect.position - world_min) / patch_span_m).floor()
	var last := ((rect.end - world_min) / patch_span_m).floor()
	for z in range(int(first.y), int(last.y) + 1):
		for x in range(int(first.x), int(last.x) + 1):
			keys.append(Vector2i(x, z))
	return keys

# The patches whose cells show `key` or stitch to it.
func _mark_tile_patches(key: Vector2i, resident_patches: Dictionary) -> void:
	var rect := Rect2(Vector2(key) * TILE_M - Vector2.ONE * TILE_M, Vector2.ONE * TILE_M * 3.0)
	for patch_key in _patches_meeting(rect):
		if resident_patches.has(patch_key):
			_dirty_patches[patch_key] = true

func _request_patch_lists(resident_patches: Dictionary) -> void:
	var flat := PackedInt64Array()
	for key: Vector2i in resident_patches.keys():
		var generation := int(resident_patches[key]["generation"])
		var list: Dictionary = patch_lists.get(key, {})
		if not list.is_empty() and int(list["token"]) == generation:
			continue
		if int(patch_lists_requested.get(key, -2)) == generation:
			continue
		patch_lists_requested[key] = generation
		flat.append_array(PackedInt64Array([key.x, key.y, generation]))
	if flat.is_empty():
		return
	var accepted: PackedByteArray = simulation_node.request_graded_patch_tiles(flat)
	for index in accepted.size():
		if accepted[index] == 0:
			patch_lists_requested.erase(Vector2i(flat[index * 3], flat[index * 3 + 1]))

# Tile key -> {level: Vector2i(code, step), token, distance} for every listed tile of a resident
# patch whose list is current.
func _desired_tiles(camera_position: Vector3, resident_patches: Dictionary) -> Dictionary:
	var desired := {}
	var camera_xz := Vector2(camera_position.x, camera_position.z)
	for key: Vector2i in resident_patches.keys():
		var list: Dictionary = patch_lists.get(key, {})
		if list.is_empty() or int(list["token"]) != int(resident_patches[key]["generation"]):
			continue
		for tile: Vector2i in list["tiles"]:
			var token := _tile_token(tile, resident_patches)
			if desired.has(tile):
				continue
			var centre := (Vector2(tile) + Vector2(0.5, 0.5)) * TILE_M
			var distance := centre.distance_to(camera_xz)
			var current: Dictionary = installed.get(tile, {})
			var current_code := int(current.get("level", -1))
			desired[tile] = {"level": desired_level(distance, current_code), "token": token, "distance": distance}
	return desired

# A tile's token: the sum of its resident patches' payload generations, which grows whenever one
# of them does.
func _tile_token(tile: Vector2i, resident_patches: Dictionary) -> int:
	# The tile's interior: a patch that only touches its side shows none of it.
	var rect := Rect2(Vector2(tile) * TILE_M + Vector2.ONE * 0.01, Vector2.ONE * (TILE_M - 0.02))
	var token := 0
	for key in _patches_meeting(rect):
		if resident_patches.has(key):
			token += int(resident_patches[key]["generation"])
	return token

func _request_tiles(desired: Dictionary) -> void:
	var wanted: Array = []
	for tile: Vector2i in desired.keys():
		var want: Dictionary = desired[tile]
		var level: Vector2i = want["level"]
		var current: Dictionary = installed.get(tile, {})
		if (
			not current.is_empty()
			and int(current["level"]) == level.x
			and int(current["step"]) == level.y
			and int(current["token"]) == int(want["token"])
		):
			continue
		var request_key := "%d,%d,%d,%d" % [tile.x, tile.y, level.x, level.y]
		if int(requested.get(request_key, -1)) == int(want["token"]):
			continue
		wanted.append([float(want["distance"]), tile, level, int(want["token"]), request_key])
	# Closest first, within the per-frame request budget.
	wanted.sort_custom(func(a: Array, b: Array) -> bool: return a[0] < b[0])
	var flat := PackedInt64Array()
	for entry: Array in wanted.slice(0, REQUESTS_PER_FRAME):
		var tile: Vector2i = entry[1]
		var level: Vector2i = entry[2]
		flat.append_array(PackedInt64Array([tile.x, tile.y, level.x, level.y, entry[3]]))
	if flat.is_empty():
		return
	# Rust queues what fits its in-flight budget; the rest is asked again on a later frame.
	var accepted: PackedByteArray = simulation_node.request_graded_tiles(flat)
	var sent := wanted.slice(0, REQUESTS_PER_FRAME)
	for index in sent.size():
		if index < accepted.size() and accepted[index] != 0:
			requested[sent[index][4]] = sent[index][3]

func _evict(desired: Dictionary) -> void:
	for tile: Vector2i in installed.keys():
		if desired.has(tile):
			continue
		_free_slot(installed[tile])
		installed.erase(tile)
		counters["evictions"] += 1
		_dirty_patches_around(tile)
	for request_key: String in requested.keys():
		var parts := request_key.split(",")
		if not desired.has(Vector2i(int(parts[0]), int(parts[1]))):
			requested.erase(request_key)

func _dirty_patches_around(tile: Vector2i) -> void:
	var rect := Rect2(Vector2(tile) * TILE_M - Vector2.ONE * TILE_M, Vector2.ONE * TILE_M * 3.0)
	for key in _patches_meeting(rect):
		if patch_cells.has(key):
			_dirty_patches[key] = true

# The tiles whose interior meets a patch rectangle.
static func _patch_tiles(rect: Rect2) -> Array[Vector2i]:
	var tiles: Array[Vector2i] = []
	var first := Vector2i(int(floor(rect.position.x / TILE_M)), int(floor(rect.position.y / TILE_M)))
	var last := Vector2i(int(ceil(rect.end.x / TILE_M)) - 1, int(ceil(rect.end.y / TILE_M)) - 1)
	for z in range(first.y, last.y + 1):
		for x in range(first.x, last.x + 1):
			tiles.append(Vector2i(x, z))
	return tiles

# A cell's lattice code: its installed level's, else the patch's base cells'.
func _cell_code(tile: Vector2i, step: int) -> int:
	var entry: Dictionary = installed.get(tile, {})
	return int(entry["code"]) if not entry.is_empty() else base_code(step)

# The code a cell's side stitches to: the coarser of its own and its neighbour's, where the
# neighbour shares the tile side (inside the patch, or across the patch border at a tile side).
func _side_code(own: int, _tile: Vector2i, neighbour: Vector2i, side_m: float, patch: Dictionary, resident_patches: Dictionary, axis: int) -> int:
	var rect: Rect2 = patch["rect"]
	var lo := rect.position[axis]
	var hi := rect.end[axis]
	if side_m > lo and side_m < hi:
		return max(own, _cell_code(neighbour, int(patch["step"])))
	if side_m != lo and side_m != hi:
		# The cell is clamped to the patch here: its side is not the tile's.
		return own
	# Across the patch border: the neighbour patch's cell (or PlaneMesh) at its own step.
	var probe := (Vector2(neighbour) + Vector2(0.5, 0.5)) * TILE_M
	if side_m == lo:
		probe[axis] = lo - 0.5
	else:
		probe[axis] = hi + 0.5
	for key in _patches_meeting(Rect2(probe, Vector2.ZERO)):
		if resident_patches.has(key):
			return max(own, _cell_code(neighbour, int(resident_patches[key]["step"])))
	return own

func _rebuild_patch_cells(key: Vector2i, resident_patches: Dictionary) -> void:
	var patch: Dictionary = resident_patches[key]
	var list: Dictionary = patch_lists.get(key, {})
	var tiles := _patch_tiles(patch["rect"])
	var any_graded := false
	for tile in tiles:
		if installed.has(tile):
			any_graded = true
			break
	var patch_node: GeometryInstance3D = patch["node"]
	if not any_graded or list.is_empty():
		if patch_cells.has(key):
			release_patch(key, patch_node)
		return
	var cells: Dictionary = patch_cells.get(key, {})
	if cells.is_empty():
		cells = _acquire_cells(patch)
		patch_cells[key] = cells
	cells["rect"] = patch["rect"]
	var step := int(patch["step"])
	cells["step"] = step
	var rect: Rect2 = patch["rect"]
	var rect_hm := Vector4i(
		int(round(rect.position.x * 2.0)), int(round(rect.position.y * 2.0)),
		int(round(rect.end.x * 2.0)), int(round(rect.end.y * 2.0))
	)
	var counts := PackedInt32Array()
	counts.resize(CELL_MESHES)
	var nodes: Array = cells["nodes"]
	# What each MultiMesh holds, kept beside it: per mesh the instance custom data written.
	var instances: Array[PackedColorArray] = []
	for index in CELL_MESHES:
		instances.append(PackedColorArray())
	for tile in tiles:
		var own := _cell_code(tile, step)
		var entry: Dictionary = installed.get(tile, {})
		var mesh_index := int(entry["level"]) if not entry.is_empty() else CELL_BASE
		var origin := Vector2(tile) * TILE_M
		var codes := own
		codes |= _side_code(own, tile, tile + Vector2i(-1, 0), origin.x, patch, resident_patches, 0) << 3
		codes |= _side_code(own, tile, tile + Vector2i(1, 0), origin.x + TILE_M, patch, resident_patches, 0) << 6
		codes |= _side_code(own, tile, tile + Vector2i(0, -1), origin.y, patch, resident_patches, 1) << 9
		codes |= _side_code(own, tile, tile + Vector2i(0, 1), origin.y + TILE_M, patch, resident_patches, 1) << 12
		var slot := int(entry["slot"]) if not entry.is_empty() else -1
		var multimesh: MultiMesh = (nodes[mesh_index] as MultiMeshInstance3D).multimesh
		var custom := Color(tile.x, slot, tile.y, codes)
		multimesh.set_instance_custom_data(counts[mesh_index], custom)
		instances[mesh_index].append(custom)
		counts[mesh_index] += 1
	cells["instances"] = instances
	for index in CELL_MESHES:
		var node: MultiMeshInstance3D = nodes[index]
		var multimesh := node.multimesh
		# GL3 cells take the finest GL3 mesh (step 2): a coarser instance clamps the extra
		# vertices onto its sides.
		var code := index if index < 3 else (base_code(step) if index == CELL_BASE else base_code(2))
		multimesh.mesh = _meshes[min(code, 6)]
		multimesh.visible_instance_count = counts[index]
		node.visible = counts[index] > 0
		node.set_instance_shader_parameter("cell_patch_rect_hm", rect_hm)
		node.set_instance_shader_parameter("cell_lattice_anchor_hm", lattice_anchor_hm)
	(nodes[CELL_BASE] as GeometryInstance3D).material_override = patch["material"]
	(nodes[CELL_BASE] as GeometryInstance3D).set_instance_shader_parameter("cell_mode", 2.0)
	if cells.get("node_parent", null) != patch_node:
		for node: Node in nodes:
			if node.get_parent() != null:
				node.get_parent().remove_child(node)
			patch_node.add_child(node)
		cells["node_parent"] = patch_node
	_sync_material(cells, patch["material"])
	# The PlaneMesh collapses while the cells draw the patch.
	patch_node.set_instance_shader_parameter("cell_mode", 1.0)

## Returns a patch to its PlaneMesh and its cell resources to the pool. Called when the patch
## recycles or loses its last graded tile.
func release_patch(key: Vector2i, patch_node: GeometryInstance3D) -> void:
	var cells: Dictionary = patch_cells.get(key, {})
	patch_cells.erase(key)
	if patch_node != null:
		patch_node.set_instance_shader_parameter("cell_mode", 0.0)
	if cells.is_empty():
		return
	for node: MultiMeshInstance3D in cells["nodes"]:
		node.visible = false
		node.multimesh.visible_instance_count = 0
		if node.get_parent() != null:
			node.get_parent().remove_child(node)
	cells["node_parent"] = null
	_cells_pool.append(cells)

func _acquire_cells(patch: Dictionary) -> Dictionary:
	if not _cells_pool.is_empty():
		return _cells_pool.pop_back()
	var nodes: Array = []
	var aabb: AABB = patch["aabb"]
	for index in CELL_MESHES:
		var multimesh := MultiMesh.new()
		multimesh.transform_format = MultiMesh.TRANSFORM_3D
		multimesh.use_custom_data = true
		multimesh.instance_count = MAX_CELLS_PER_PATCH
		# New instances start with a zero transform; the shader places the vertices, so every
		# instance keeps the identity.
		for instance in MAX_CELLS_PER_PATCH:
			multimesh.set_instance_transform(instance, Transform3D.IDENTITY)
		multimesh.custom_aabb = aabb
		multimesh.visible_instance_count = 0
		counters["multimeshes_created"] += 1
		var node := MultiMeshInstance3D.new()
		node.name = "GradedCells%d" % index
		node.multimesh = multimesh
		node.custom_aabb = aabb
		node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		node.visible = false
		nodes.append(node)
	var material := ShaderMaterial.new()
	material.shader = GRADED_SHADER
	counters["materials_created"] += 1
	for index in CELL_MESHES:
		if index != CELL_BASE:
			(nodes[index] as GeometryInstance3D).material_override = material
	for code in pools.size():
		material.set_shader_parameter("graded_texels_%d" % code, pools[code]["texels"])
		material.set_shader_parameter("graded_rows_%d" % code, pools[code]["rows"])
	material.set_shader_parameter("graded_seam_capacity", Vector4i(
		int(levels[0]["seam_capacity"]), int(levels[1]["seam_capacity"]),
		int(levels[2]["seam_capacity"]), int(levels[3]["seam_capacity"])
	))
	return {"nodes": nodes, "material": material, "watched": [], "node_parent": null}

# The patch material parameters whose change means the graded material must be synced: the
# textures and per-patch values terrain.gd swaps on commits, masks, water and overlays.
func _watched_parameters(material: ShaderMaterial) -> Array:
	return [
		material.get_shader_parameter("heightmap"),
		material.get_shader_parameter("terrain_mask_layer"),
		material.get_shader_parameter("terrain_masks"),
		material.get_shader_parameter("watermap"),
		material.get_shader_parameter("land_cover_texture"),
		material.get_shader_parameter("overlay_mode"),
		material.get_shader_parameter("overlay_texture"),
		material.get_shader_parameter("terrain_debug_lod_step"),
	]

func _sync_material(cells: Dictionary, source: ShaderMaterial) -> void:
	var material: ShaderMaterial = cells["material"]
	if _uniform_names.is_empty():
		for uniform: Dictionary in source.shader.get_shader_uniform_list():
			var name := String(uniform["name"])
			if not name.begins_with("graded_") and not name.begins_with("cell_"):
				_uniform_names.append(name)
	for name in _uniform_names:
		material.set_shader_parameter(name, source.get_shader_parameter(name))
	cells["watched"] = _watched_parameters(source)

## Pool sizes, use and creation counters, for tests and the memory record.
func get_pool_counters() -> Dictionary:
	var out := counters.duplicate()
	var bytes := 0
	for code in pools.size():
		var pool: Dictionary = pools[code]
		out["slots_%d" % code] = int(pool["slots"])
		var free: Array[int] = pool["free"]
		out["free_%d" % code] = free.size()
		bytes += int(pool["slots"]) * (int(pool["side"]) * int(pool["side"]) + int(pool["row_texels"])) * 16
	out["pool_bytes"] = bytes
	out["installed"] = installed.size()
	out["patches_with_cells"] = patch_cells.size()
	return out
