# SPDX-License-Identifier: GPL-2.0-only

## Graded terrain tiles (TERRAIN-07-D, docs/terrain.md "Godot tile renderer"): residency of the
## graded 64 m tiles of the resident patches by camera distance, the slot pools (per level one
## Texture2DArray of RGBAF layers for the texels and one for the rows, prewarmed), in-place
## uploads from one pooled Image per level, and per patch one MultiMesh per cell mesh (the base
## cells and GL0-GL3) whose instances carry the tile's place in the patch, slot and stitching
## codes, each graded level with its own material binding that level's pools. It does no
## geometry: positions, stitching, clamping and the mask are terrain_cells.gdshaderinc's, the
## values Rust's. Nothing is created per edit; the counters in get_pool_counters() prove it.
## An edit (item 5) dirties the tiles Rust selects for it at the level each is held at: they
## rebake with the edit's serial as their token and install together in one frame, each into
## its own slot when its level is unchanged. The tiles' wall faces (conflict walls, abutments,
## headwalls) draw as instances of one quad in a pooled MultiMesh per patch, placed by
## terrain_graded_walls.gdshader from the values in their instance transforms.
##
## Rust methods called: get_graded_tile_layout(), request_graded_patch_tiles(),
##   request_graded_tiles(), poll_graded_tile_results()
extends RefCounted

const GRADED_SHADER := preload("res://assets/materials/terrain_graded.gdshader")
const WALL_SHADER := preload("res://assets/materials/terrain_graded_walls.gdshader")
const TILE_M := 64.0
# Level distances with their hysteresis (T5, TD10): GL0 to 256 m, GL1 to 512 m, GL2 to 2 km,
# GL3 beyond, at the patch LOD rule's steps (2 to 5 km, 4 to 12 km, 8 beyond).
const LEVEL_END_M := [256.0, 512.0, 2000.0]
const GL3_STEP_END_M := [[2, 5000.0], [4, 12000.0], [8, INF]]
const LEVEL_HYSTERESIS_M := 32.0
# Slots per level. GL0 and GL1 follow the memory table; GL2 and GL3 stop at 2,048 layers, the
# smallest Texture2DArray depth Vulkan guarantees (TERRAIN-07 open question T23).
const DEFAULT_POOL_SLOTS := [64, 192, 2048, 2048]
# Patch cells: a patch meets at most (ceil(span / 64 m) + 1)^2 tiles (9 x 9 on a 510 m patch),
# the instances of each cell MultiMesh (_cells_per_patch). A tile's place in its patch is
# column + PATCH_TILE_ROW * row in one instance channel (terrain_cells.gdshaderinc).
const PATCH_TILE_ROW := 32
const REQUESTS_PER_FRAME := 24
const INSTALL_BUDGET_MS := 1.0
const POLL_BATCH := 4
const CELL_MESHES := 5
const CELL_BASE := 4
# Wall face instances per patch, prewarmed with its cells: at most 42 faces per patch on the
# Kuopio reference save and none on the populated save (TERRAIN-07-D round 6 census, GL0). A
# patch with more draws the first ones and counts and logs the rest.
const MAX_WALLS_PER_PATCH := 256
# A visibility range no camera is within (metres): a patch's PlaneMesh while its cells draw it.
const HIDDEN_RANGE_M := 0.001
const WALL_FACE_FLOATS := 8
# Every (level code, GL3 step) a tile can be requested at.
const LEVEL_STEPS := [Vector2i(0, 0), Vector2i(1, 0), Vector2i(2, 0), Vector2i(3, 2), Vector2i(3, 4), Vector2i(3, 8)]

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
# Tile key -> its token: the serial of the last edit that dirtied it while it was installed or
# in flight (0 before any). Only held tiles have one.
var tile_tokens: Dictionary = {}
# The edit batch: tile key -> the token its rebake must carry. Results wait in _edit_staged
# until every tile of the batch has one, then install in the same frame.
var _edit_batch: Dictionary = {}
var _edit_staged: Dictionary = {}
# The tiles the last edit batch installed, for tests and the probe.
var last_edit_batch: Array = []
# Patch key -> {token, tiles: Array[Vector2i]}; patch key -> token of a list in flight.
var patch_lists: Dictionary = {}
var patch_lists_requested: Dictionary = {}
# Requests Rust refused by contract (retry false), by request key ("x,z,level,step" for a tile,
# the patch key for a list) -> the token refused: never asked again at that token, so a refusal
# is counted and logged once instead of being re-asked every frame (round 5's populated save).
var refused: Dictionary = {}
var patch_lists_refused: Dictionary = {}
# Patch key -> {nodes: Array[MultiMeshInstance3D], materials: Array[ShaderMaterial] (GL0-GL3),
# watched: Array}.
var patch_cells: Dictionary = {}
var _cells_pool: Array[Dictionary] = []
var _cells_per_patch := 81
# The cell MultiMeshes' initial buffer (identity transforms, zero custom data), shared.
var _identity_cells := PackedFloat32Array()
var _meshes: Dictionary = {}
var _wall_mesh: ArrayMesh = null
var _wall_material: ShaderMaterial = null
var _dirty_patches: Dictionary = {}
# Patches whose wall instances must be rewritten (a tile rewritten in place changed its faces).
var _dirty_walls: Dictionary = {}
var _uniform_names: PackedStringArray = PackedStringArray()
# Tile key -> {level: Vector2i(code, step), token, distance}: every tile a resident patch's list
# names. It follows the changes: a patch entering, leaving or getting a new list adds and
# removes its own tiles (each tile counts the lists naming it, in _tile_lists, and the patch's
# counted tiles are kept in _counted), and a camera move of 8 m re-levels the desired tiles.
var _desired: Dictionary = {}
var _tile_lists: Dictionary = {}
var _counted: Dictionary = {}
var _desired_camera := Vector3(INF, INF, INF)
# The tiles and patch lists that may need a request: a tile whose desire, level or token
# changed, whose request failed or did not go out (the per-frame budget or Rust's in-flight
# bound); a patch that changed or whose list arrived, failed or was not accepted. An idle frame
# has none.
var _wanted: Dictionary = {}
var _lists_wanted: Dictionary = {}
const DESIRED_REFRESH_MOVE_M := 8.0
# Patch list requests per frame: Rust holds at most 32 in flight.
const LIST_REQUESTS_PER_FRAME := 32
var counters := {
	"textures_created": 0,
	"images_created": 0,
	"meshes_created": 0,
	"materials_created": 0,
	"multimeshes_created": 0,
	"uploads": 0,
	"evictions": 0,
	"pool_exhausted": 0,
	"walls_truncated": 0,
	"bake_failures": 0,
	"edits": 0,
	"edit_batches": 0,
	"edit_tiles_last": 0,
	"edit_install_ms_last": 0.0,
	"edit_install_ms_max": 0.0,
	"install_ms_last": 0.0,
	"install_ms_max": 0.0,
	"poll_ms_max": 0.0,
	"upload_ms_max": 0.0,
	# The last edit frame's parts (ms) and the patches whose cells or walls it rewrote.
	"edit_frame_parts": {},
}
# Per-frame accumulators of the parts the edit frame reports.
var _frame_sync_us := 0
var _frame_walls_us := 0
var _frame_lists := 0
var _frame_lists_same := 0

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
	_wall_material = ShaderMaterial.new()
	_wall_material.shader = WALL_SHADER
	counters["materials_created"] += 1
	# One cell set per patch of the world's grid, the most patches that can draw cells at once
	# (every resident patch may hold a graded tile), so neither streaming nor an edit creates one.
	var tiles_per_side := int(ceil(span_m / TILE_M)) + 1
	_cells_per_patch = tiles_per_side * tiles_per_side
	_identity_cells = PackedFloat32Array()
	var grid := Vector2i(int(ceil(world_size.x / span_m)), int(ceil(world_size.y / span_m)))
	_cells_pool.clear()
	for _patch in grid.x * grid.y:
		_cells_pool.append(_new_cells())

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
	# The wall quad: x 0 at a face's end a and 1 at b, y 0 at its bottom and 1 at its top.
	var quad := []
	quad.resize(Mesh.ARRAY_MAX)
	quad[Mesh.ARRAY_VERTEX] = PackedVector3Array([Vector3(0, 0, 0), Vector3(1, 0, 0), Vector3(0, 1, 0), Vector3(1, 1, 0)])
	quad[Mesh.ARRAY_INDEX] = PackedInt32Array([0, 1, 3, 0, 3, 2])
	_wall_mesh = ArrayMesh.new()
	_wall_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, quad)
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
	tile_tokens.clear()
	_edit_batch.clear()
	_edit_staged.clear()
	patch_lists.clear()
	patch_lists_requested.clear()
	refused.clear()
	patch_lists_refused.clear()
	_dirty_walls.clear()
	for key in patch_cells.keys():
		release_patch(key, null)
	_desired.clear()
	_tile_lists.clear()
	_counted.clear()
	_desired_camera = Vector3(INF, INF, INF)
	_wanted.clear()
	_lists_wanted.clear()

## One frame: polls finished work and installs it within the install budget, keeps the tiles of
## `resident_patches` (patch key -> {node, material, rect: Rect2, step, generation}) at their
## distance's level, evicts the rest and rewrites the instances of the patches that changed.
## `changed_patches` holds the keys whose entry was added, rewritten or removed since the last
## call and `changed_materials` the keys whose material parameters may have changed (the caller
## clears both); with neither, nor a camera move, nor finished work, the frame only polls.
## O(changes) per frame: a changed patch or an arriving list costs its own tiles.
func process(camera_position: Vector3, resident_patches: Dictionary, changed_patches: Dictionary, changed_materials: Dictionary) -> void:
	var start_us := Time.get_ticks_usec()
	_frame_sync_us = 0
	_frame_walls_us = 0
	_frame_lists = 0
	_frame_lists_same = 0
	# A camera move re-levels the desired tiles first, so the tiles this frame adds take the
	# camera's position too. O(desired tiles) per 8 m.
	if camera_position.distance_to(_desired_camera) >= DESIRED_REFRESH_MOVE_M:
		_desired_camera = camera_position
		_relevel_desired()
	# An edit's batch installs in a frame of its own: the poll that completes it leaves it for
	# the next frame, which installs it and polls nothing.
	var batch_tiles := _install_edit_batch(resident_patches)
	var batch_us := Time.get_ticks_usec()
	if batch_tiles == 0:
		_poll_and_install(start_us, resident_patches)
	var polled_us := Time.get_ticks_usec()
	counters["install_ms_last"] = float(Time.get_ticks_usec() - start_us) / 1000.0
	counters["install_ms_max"] = max(float(counters["install_ms_max"]), float(counters["install_ms_last"]))
	for key: Vector2i in changed_patches.keys():
		_patch_changed(key, resident_patches)
	if not _lists_wanted.is_empty():
		_request_patch_lists(resident_patches)
	if not _wanted.is_empty():
		_request_tiles()
	var residency_us := Time.get_ticks_usec()
	# A patch's cells follow its material's parameters (a changed patch's LOD step in
	# _patch_changed). O(changed materials).
	for key: Vector2i in changed_materials.keys():
		if not patch_cells.has(key) or not resident_patches.has(key):
			continue
		var material: ShaderMaterial = resident_patches[key]["material"]
		var cells: Dictionary = patch_cells[key]
		if _watched_parameters(material) != cells["watched"]:
			_sync_material(cells, material)
	var watched_us := Time.get_ticks_usec()
	var cell_patches := _dirty_patches.size()
	for key: Vector2i in _dirty_patches.keys():
		if resident_patches.has(key):
			_rebuild_patch_cells(key, resident_patches)
		_dirty_walls.erase(key)
	_dirty_patches.clear()
	var cells_us := Time.get_ticks_usec()
	var wall_patches := _dirty_walls.size()
	for key: Vector2i in _dirty_walls.keys():
		if resident_patches.has(key) and patch_cells.has(key):
			_rebuild_patch_walls(patch_cells[key], resident_patches[key])
	_dirty_walls.clear()
	if batch_tiles > 0:
		var end_us := Time.get_ticks_usec()
		var ms := func(from_us: int, to_us: int) -> float: return float(to_us - from_us) / 1000.0
		counters["edit_frame_parts"] = {
			"batch_ms": ms.call(start_us, batch_us), "poll_ms": ms.call(batch_us, polled_us),
			"residency_ms": ms.call(polled_us, residency_us), "watched_ms": ms.call(residency_us, watched_us),
			"cells_ms": ms.call(watched_us, cells_us), "cells_sync_ms": float(_frame_sync_us) / 1000.0,
			"cells_walls_ms": float(_frame_walls_us) / 1000.0, "walls_ms": ms.call(cells_us, end_us),
			"cell_patches": cell_patches, "wall_patches": wall_patches,
			"lists": _frame_lists, "lists_unchanged": _frame_lists_same,
		}
		# The edit's frame: its uploads, the cell rewrites they caused and the frame's changes.
		counters["edit_batches"] += 1
		counters["edit_tiles_last"] = batch_tiles
		counters["edit_install_ms_last"] = float(Time.get_ticks_usec() - start_us) / 1000.0
		counters["edit_install_ms_max"] = max(float(counters["edit_install_ms_max"]), float(counters["edit_install_ms_last"]))

func _poll_and_install(start_us: int, resident_patches: Dictionary) -> void:
	while true:
		var poll_start_us := Time.get_ticks_usec()
		var result: Dictionary = simulation_node.poll_graded_tile_results(POLL_BATCH)
		counters["poll_ms_max"] = max(float(counters["poll_ms_max"]), float(Time.get_ticks_usec() - poll_start_us) / 1000.0)
		for edit: Dictionary in result.get("edits", []):
			_apply_edit(edit)
		for list: Dictionary in result.get("patches", []):
			var key := Vector2i(int(list["patch_x"]), int(list["patch_z"]))
			patch_lists_requested.erase(key)
			var tiles: Array[Vector2i] = []
			var flat: PackedInt32Array = list["tiles"]
			for index in range(0, flat.size(), 2):
				tiles.append(Vector2i(flat[index], flat[index + 1]))
			_frame_lists += 1
			var previous: Array = patch_lists.get(key, {}).get("tiles", [])
			if patch_lists.has(key) and previous == tiles:
				_frame_lists_same += 1
			# The cells show the installed tiles of the patch's rectangle, not its list: a list
			# rewrites them only when the patch has none yet or the list empties or fills
			# (_rebuild_patch_cells); listed tiles install or evict on their own.
			if not patch_cells.has(key) or previous.is_empty() != tiles.is_empty():
				_dirty_patches[key] = true
			patch_lists[key] = {"token": int(list["token"]), "tiles": tiles}
			_recount(key, resident_patches)
			# A list baked for an older generation is asked again.
			_lists_wanted[key] = true
		for failure: Dictionary in result.get("failed", []):
			_failed(failure)
		var tiles: Array = result.get("tiles", [])
		for tile: Dictionary in tiles:
			_install(tile, resident_patches)
		if tiles.size() < POLL_BATCH:
			return
		if float(Time.get_ticks_usec() - start_us) / 1000.0 >= INSTALL_BUDGET_MS:
			return

# A failed request: a transient one (busy core, unready site index) is asked again; one Rust
# refuses by contract is recorded with its token, counted and logged once, and not asked again
# until its token changes (an edit, a new patch generation).
func _failed(failure: Dictionary) -> void:
	var keys: PackedInt64Array = failure["keys"]
	var token := int(failure["token"])
	var retry := bool(failure["retry"])
	if String(failure["kind"]) == "patch":
		var key := Vector2i(keys[0], keys[1])
		if int(patch_lists_requested.get(key, -2)) == token:
			patch_lists_requested.erase(key)
		if not retry:
			patch_lists_refused[key] = token
		_lists_wanted[key] = true
	else:
		var tile := Vector2i(keys[0], keys[1])
		var request_key := "%d,%d,%d,%d" % [keys[0], keys[1], keys[2], keys[3]]
		if int(requested.get(request_key, -1)) == token:
			requested.erase(request_key)
		if not retry:
			refused[request_key] = token
			# The tile stays as it was; the edit installs without it.
			_edit_batch.erase(tile)
			_edit_staged.erase(tile)
		_wanted[tile] = true
	if not retry:
		counters["bake_failures"] += 1
		MetrumDebug.log("terrain", "graded %s %s token %d refused: %s" % [failure["kind"], str(keys), token, failure["reason"]])

func _install(tile: Dictionary, resident_patches: Dictionary) -> void:
	var key := Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))
	var code := int(tile["level"])
	var step := int(tile["step"])
	var request_key := "%d,%d,%d,%d" % [key.x, key.y, code, step]
	var token := int(tile["token"])
	if int(requested.get(request_key, -1)) != token:
		return
	requested.erase(request_key)
	if int(_edit_batch.get(key, -1)) == token:
		# Part of an edit: it waits for the batch's other tiles.
		_edit_staged[key] = tile
		return
	_upload(key, tile, resident_patches)

func _upload(key: Vector2i, tile: Dictionary, resident_patches: Dictionary) -> void:
	var code := int(tile["level"])
	var step := int(tile["step"])
	var token := int(tile["token"])
	var pool: Dictionary = pools[code]
	var current: Dictionary = installed.get(key, {})
	var slot := -1
	if not current.is_empty() and int(current["level"]) == code:
		# The same pool: the slot is rewritten in place, before this frame draws.
		slot = int(current["slot"])
	else:
		var free: Array[int] = pool["free"]
		if free.is_empty():
			# The pool sizes make this impossible within the distance rules; counted, the tile
			# stays as it was.
			counters["pool_exhausted"] += 1
			_wanted[key] = true
			MetrumDebug.log("terrain", "graded pool %d exhausted at tile %s" % [code, str(key)])
			return
		slot = free.pop_back()
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
	var cell_code := code if code < 3 else base_code(step)
	if not current.is_empty() and int(current["slot"]) != slot:
		_free_slot(current)
	var walls: PackedFloat32Array = (tile["walls"] as PackedByteArray).to_float32_array()
	installed[key] = {"code": cell_code, "level": code, "step": step, "slot": slot, "token": token, "walls": walls}
	# A bake of an older level or token (the camera moved, an edit dirtied it) still wants one.
	var want: Dictionary = _desired.get(key, {})
	if not want.is_empty() and (want["level"] != Vector2i(code, step) or int(want["token"]) != token):
		_wanted[key] = true
	# Rewritten in place, the tile keeps its instances; a new slot or lattice rewrites the cells
	# that show it or stitch to it, and changed faces the walls of the patches that draw them.
	if current.is_empty() or int(current["slot"]) != slot or int(current["code"]) != cell_code:
		_mark_tile_patches(key, resident_patches)
	elif walls != current["walls"]:
		for patch_key in _patches_meeting(Rect2(Vector2(key) * TILE_M, Vector2.ONE * TILE_M)):
			_dirty_walls[patch_key] = true

# Installs the edit batch once every tile of it has its result, all in this frame. Returns how
# many tiles it installed.
func _install_edit_batch(resident_patches: Dictionary) -> int:
	if _edit_batch.is_empty() or _edit_staged.size() < _edit_batch.size():
		return 0
	for key: Vector2i in _edit_staged.keys():
		_upload(key, _edit_staged[key], resident_patches)
	last_edit_batch = _edit_staged.keys()
	var count := _edit_staged.size()
	_edit_batch.clear()
	_edit_staged.clear()
	return count

# Rebakes the tiles an edit dirtied at the level each is held at, installed or in flight.
func _apply_edit(edit: Dictionary) -> void:
	counters["edits"] += 1
	var serial := int(edit["serial"])
	if bool(edit["all"]):
		for tile: Vector2i in installed.keys():
			_dirty_tile(tile, serial)
		for request_key: String in requested.keys() + refused.keys():
			var parts := request_key.split(",")
			_dirty_tile(Vector2i(int(parts[0]), int(parts[1])), serial)
		return
	for entry: Dictionary in edit["levels"]:
		var code := int(entry["level"])
		var step := int(entry["step"])
		var flat: PackedInt32Array = entry["tiles"]
		for index in range(0, flat.size(), 2):
			var tile := Vector2i(flat[index], flat[index + 1])
			var current: Dictionary = installed.get(tile, {})
			var held := not current.is_empty() and int(current["level"]) == code and int(current["step"]) == step
			var request_key := "%d,%d,%d,%d" % [tile.x, tile.y, code, step]
			# A refused tile is held too: the edit may have changed what refused it.
			if held or requested.has(request_key) or refused.has(request_key):
				_dirty_tile(tile, serial)

# A dirty tile takes the edit's serial as its token, so a bake in flight with an older one is
# dropped and the tile is requested again, a refused one too; an installed tile joins the edit
# batch.
func _dirty_tile(tile: Vector2i, serial: int) -> void:
	var token: int = max(int(tile_tokens.get(tile, 0)), serial)
	tile_tokens[tile] = token
	for level: Vector2i in LEVEL_STEPS:
		var request_key := "%d,%d,%d,%d" % [tile.x, tile.y, level.x, level.y]
		requested.erase(request_key)
		refused.erase(request_key)
	_edit_staged.erase(tile)
	if installed.has(tile):
		_edit_batch[tile] = token
	if _desired.has(tile):
		_desired[tile]["token"] = token
		_wanted[tile] = true

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

# A resident patch entry was added, rewritten or removed: its counted tiles follow its residency,
# its list is asked again if its generation moved, and its cells follow its LOD step.
func _patch_changed(key: Vector2i, resident_patches: Dictionary) -> void:
	_recount(key, resident_patches)
	if not resident_patches.has(key):
		_lists_wanted.erase(key)
		return
	_lists_wanted[key] = true
	var cells: Dictionary = patch_cells.get(key, {})
	if not cells.is_empty() and int(cells.get("step", -1)) != int(resident_patches[key]["step"]):
		_dirty_patches[key] = true

# The tiles patch `key` adds to the desired set: its list while it is resident (a patch whose
# list is being refreshed keeps its last one until the new one arrives, so an edit's patch shows
# its tiles meanwhile). A tile enters the desired set with the first list naming it and leaves
# with the last. O(the patch's old and new tiles).
func _recount(key: Vector2i, resident_patches: Dictionary) -> void:
	var tiles: Array = []
	if resident_patches.has(key):
		tiles = patch_lists.get(key, {}).get("tiles", [])
	var old: Array = _counted.get(key, [])
	if old == tiles:
		return
	# The new list counts first, so a tile on both lists never leaves.
	for tile: Vector2i in tiles:
		var count := int(_tile_lists.get(tile, 0)) + 1
		_tile_lists[tile] = count
		if count == 1:
			_desire(tile)
	for tile: Vector2i in old:
		var count := int(_tile_lists[tile]) - 1
		if count > 0:
			_tile_lists[tile] = count
			continue
		_tile_lists.erase(tile)
		_undesire(tile)
	if tiles.is_empty():
		_counted.erase(key)
	else:
		_counted[key] = tiles

# A tile some resident patch lists: its level at the camera's distance, with hysteresis around
# the level it is installed at.
func _desire(tile: Vector2i) -> void:
	var distance := ((Vector2(tile) + Vector2(0.5, 0.5)) * TILE_M).distance_to(Vector2(_desired_camera.x, _desired_camera.z))
	var current_code := int(installed.get(tile, {}).get("level", -1))
	_desired[tile] = {"level": desired_level(distance, current_code), "token": int(tile_tokens.get(tile, 0)), "distance": distance}
	_wanted[tile] = true

# A tile no resident patch lists any more: its slot is freed and its requests, token and
# refusals dropped (only held tiles keep a token).
func _undesire(tile: Vector2i) -> void:
	_desired.erase(tile)
	_wanted.erase(tile)
	if installed.has(tile):
		_free_slot(installed[tile])
		installed.erase(tile)
		counters["evictions"] += 1
		_dirty_patches_around(tile)
	_edit_batch.erase(tile)
	_edit_staged.erase(tile)
	for level: Vector2i in LEVEL_STEPS:
		var request_key := "%d,%d,%d,%d" % [tile.x, tile.y, level.x, level.y]
		requested.erase(request_key)
		refused.erase(request_key)
	tile_tokens.erase(tile)

# Every desired tile's distance and level at the camera's new position; a tile whose level
# changed wants a bake at it. O(desired tiles).
func _relevel_desired() -> void:
	var camera_xz := Vector2(_desired_camera.x, _desired_camera.z)
	for tile: Vector2i in _desired.keys():
		var want: Dictionary = _desired[tile]
		var distance := ((Vector2(tile) + Vector2(0.5, 0.5)) * TILE_M).distance_to(camera_xz)
		var level := desired_level(distance, int(installed.get(tile, {}).get("level", -1)))
		want["distance"] = distance
		if level != want["level"]:
			want["level"] = level
			_wanted[tile] = true

# Asks for the lists of the wanted patches whose list is not of their generation, not in flight
# and not refused at it, within Rust's in-flight bound; the rest stay wanted.
func _request_patch_lists(resident_patches: Dictionary) -> void:
	var flat := PackedInt64Array()
	for key: Vector2i in _lists_wanted.keys():
		if flat.size() >= LIST_REQUESTS_PER_FRAME * 3:
			break
		_lists_wanted.erase(key)
		if not resident_patches.has(key):
			continue
		var generation := int(resident_patches[key]["generation"])
		var list: Dictionary = patch_lists.get(key, {})
		if not list.is_empty() and int(list["token"]) == generation:
			continue
		if int(patch_lists_requested.get(key, -2)) == generation or int(patch_lists_refused.get(key, -2)) == generation:
			continue
		patch_lists_requested[key] = generation
		flat.append_array(PackedInt64Array([key.x, key.y, generation]))
	if flat.is_empty():
		return
	var accepted: PackedByteArray = simulation_node.request_graded_patch_tiles(flat)
	for index in accepted.size():
		if accepted[index] == 0:
			var key := Vector2i(flat[index * 3], flat[index * 3 + 1])
			patch_lists_requested.erase(key)
			_lists_wanted[key] = true

# Asks for the wanted tiles that are not installed at their level and token, not in flight and
# not refused at that token, closest first within the per-frame budget. O(wanted tiles).
func _request_tiles() -> void:
	var wanted: Array = []
	for tile: Vector2i in _wanted.keys():
		var want: Dictionary = _desired.get(tile, {})
		if want.is_empty():
			_wanted.erase(tile)
			continue
		var level: Vector2i = want["level"]
		var token := int(want["token"])
		var current: Dictionary = installed.get(tile, {})
		if (
			not current.is_empty()
			and int(current["level"]) == level.x
			and int(current["step"]) == level.y
			and int(current["token"]) == token
		):
			_wanted.erase(tile)
			continue
		var request_key := "%d,%d,%d,%d" % [tile.x, tile.y, level.x, level.y]
		if int(requested.get(request_key, -1)) == token or int(refused.get(request_key, -1)) == token:
			_wanted.erase(tile)
			continue
		wanted.append([float(want["distance"]), tile, level, token, request_key])
	if wanted.is_empty():
		return
	wanted.sort_custom(func(a: Array, b: Array) -> bool: return a[0] < b[0])
	var sent := wanted.slice(0, REQUESTS_PER_FRAME)
	var flat := PackedInt64Array()
	for entry: Array in sent:
		var tile: Vector2i = entry[1]
		var level: Vector2i = entry[2]
		flat.append_array(PackedInt64Array([tile.x, tile.y, level.x, level.y, entry[3]]))
	# Rust queues what fits its in-flight budget; the rest stays wanted for a later frame.
	var accepted: PackedByteArray = simulation_node.request_graded_tiles(flat)
	for index in sent.size():
		if index < accepted.size() and accepted[index] != 0:
			requested[sent[index][4]] = sent[index][3]
			_wanted.erase(sent[index][1])

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
	# The tile holding the patch rectangle's lower corner, as the shader finds it.
	var first: Vector2i = tiles[0] if not tiles.is_empty() else Vector2i.ZERO
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
		# Every channel an integer below 2048: the Compatibility renderer keeps instance custom
		# data as half floats.
		var place := tile - first
		var custom := Color(place.x + PATCH_TILE_ROW * place.y, slot, codes & 511, codes >> 9)
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
		for node: Node in nodes + [cells["walls"]]:
			if node.get_parent() != null:
				node.get_parent().remove_child(node)
			patch_node.add_child(node)
		cells["node_parent"] = patch_node
	var sync_us := Time.get_ticks_usec()
	_sync_material(cells, patch["material"])
	var walls_us := Time.get_ticks_usec()
	_rebuild_patch_walls(cells, patch)
	_frame_sync_us += walls_us - sync_us
	_frame_walls_us += Time.get_ticks_usec() - walls_us
	# The PlaneMesh collapses while the cells draw the patch, and is not drawn at all: its
	# visibility range ends before any camera, so its vertices do not run (TERRAIN-07-D round 5).
	# It stays visible, since grass and the water reflection read the patch node's visibility.
	patch_node.set_instance_shader_parameter("cell_mode", 1.0)
	patch_node.visibility_range_end = HIDDEN_RANGE_M

# The wall faces of the patch's installed tiles, each drawn by the patch holding its midpoint
# (a face of a tile two patches share draws once), as one instance whose transform carries
# the face (terrain_graded_walls.gdshader): basis column 0 (a.x, bottom at a, a.z), column 1
# (b.x, bottom at b, b.z), both from the patch centre, column 2 (top at a, top at b, 0).
# O(faces of the patch's tiles).
func _rebuild_patch_walls(cells: Dictionary, patch: Dictionary) -> void:
	var node: MultiMeshInstance3D = cells["walls"]
	var multimesh := node.multimesh
	var rect: Rect2 = patch["rect"]
	var centre := rect.get_center()
	var count := 0
	var truncated := 0
	for tile in _patch_tiles(rect):
		var entry: Dictionary = installed.get(tile, {})
		if entry.is_empty():
			continue
		var faces: PackedFloat32Array = entry["walls"]
		var tile_origin := Vector2(tile) * TILE_M
		var origin := tile_origin - centre
		for at in range(0, faces.size(), WALL_FACE_FLOATS):
			var middle := tile_origin + Vector2(faces[at] + faces[at + 2], faces[at + 1] + faces[at + 3]) * 0.5
			if not rect.has_point(middle):
				continue
			if count == MAX_WALLS_PER_PATCH:
				truncated += 1
				continue
			multimesh.set_instance_transform(count, Transform3D(Basis(
				Vector3(origin.x + faces[at], faces[at + 4], origin.y + faces[at + 1]),
				Vector3(origin.x + faces[at + 2], faces[at + 5], origin.y + faces[at + 3]),
				Vector3(faces[at + 6], faces[at + 7], 0.0)), Vector3.ZERO))
			count += 1
	if truncated > 0:
		counters["walls_truncated"] += truncated
		MetrumDebug.log("terrain", "graded walls: %d faces past the patch capacity %d" % [truncated, MAX_WALLS_PER_PATCH])
	multimesh.visible_instance_count = count
	node.visible = count > 0

## Returns a patch to its PlaneMesh and its cell resources to the pool. Called when the patch
## recycles or loses its last graded tile.
func release_patch(key: Vector2i, patch_node: GeometryInstance3D) -> void:
	var cells: Dictionary = patch_cells.get(key, {})
	patch_cells.erase(key)
	if patch_node != null:
		patch_node.set_instance_shader_parameter("cell_mode", 0.0)
		patch_node.visibility_range_end = 0.0
	if cells.is_empty():
		return
	for node: MultiMeshInstance3D in cells["nodes"] + [cells["walls"]]:
		node.visible = false
		node.multimesh.visible_instance_count = 0
		if node.get_parent() != null:
			node.get_parent().remove_child(node)
	cells["node_parent"] = null
	_cells_pool.append(cells)

# A pooled cell set, bounded to the patch's culling box. The pool holds one per patch of the
# grid (setup), so this creates none; an empty pool would mean more patches than the grid.
func _acquire_cells(patch: Dictionary) -> Dictionary:
	var cells: Dictionary = _cells_pool.pop_back() if not _cells_pool.is_empty() else _new_cells()
	var aabb: AABB = patch["aabb"]
	for node: MultiMeshInstance3D in cells["nodes"] + [cells["walls"]]:
		node.multimesh.custom_aabb = aabb
		node.custom_aabb = aabb
	return cells

# One patch's cell resources: five cell MultiMeshes (GL0-GL3 and the base cells), the four
# graded levels' materials and the wall MultiMesh.
func _new_cells() -> Dictionary:
	var nodes: Array = []
	# New instances start with a zero transform; the shader places the vertices, so every
	# instance keeps the identity: per instance the transform's three rows, then custom data.
	if _identity_cells.is_empty():
		for instance in _cells_per_patch:
			_identity_cells.append_array(PackedFloat32Array([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0]))
	for index in CELL_MESHES:
		var multimesh := MultiMesh.new()
		multimesh.transform_format = MultiMesh.TRANSFORM_3D
		multimesh.use_custom_data = true
		multimesh.instance_count = _cells_per_patch
		multimesh.buffer = _identity_cells
		multimesh.visible_instance_count = 0
		counters["multimeshes_created"] += 1
		var node := MultiMeshInstance3D.new()
		node.name = "GradedCells%d" % index
		node.multimesh = multimesh
		node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		node.visible = false
		nodes.append(node)
	# One material per graded level, so a draw binds only its level's two pool samplers.
	var materials: Array[ShaderMaterial] = []
	for code in pools.size():
		var material := ShaderMaterial.new()
		material.shader = GRADED_SHADER
		counters["materials_created"] += 1
		material.set_shader_parameter("graded_texels", pools[code]["texels"])
		material.set_shader_parameter("graded_rows", pools[code]["rows"])
		material.set_shader_parameter("graded_seam_capacity", int(levels[code]["seam_capacity"]))
		material.set_shader_parameter("graded_edge_capacity", int(levels[code]["edge_capacity"]))
		(nodes[code] as GeometryInstance3D).material_override = material
		materials.append(material)
	var wall_multimesh := MultiMesh.new()
	wall_multimesh.transform_format = MultiMesh.TRANSFORM_3D
	wall_multimesh.mesh = _wall_mesh
	wall_multimesh.instance_count = MAX_WALLS_PER_PATCH
	wall_multimesh.visible_instance_count = 0
	counters["multimeshes_created"] += 1
	var walls := MultiMeshInstance3D.new()
	walls.name = "GradedWalls"
	walls.multimesh = wall_multimesh
	walls.material_override = _wall_material
	walls.visible = false
	return {"nodes": nodes, "walls": walls, "materials": materials, "watched": [], "node_parent": null}

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
	if _uniform_names.is_empty():
		for uniform: Dictionary in source.shader.get_shader_uniform_list():
			var name := String(uniform["name"])
			if not name.begins_with("graded_") and not name.begins_with("cell_"):
				_uniform_names.append(name)
	# The values the cell set's materials hold, by uniform: only the parameters that differ are
	# copied into the four materials (a payload commit changes a few of about a hundred). A set
	# keeps its values through the pool, so a reused set copies what differs from its last patch.
	var values: Array = cells.get("values", [])
	var all := values.size() != _uniform_names.size()
	if all:
		values.resize(_uniform_names.size())
		cells["values"] = values
	for index in _uniform_names.size():
		var name := _uniform_names[index]
		var value: Variant = source.get_shader_parameter(name)
		if not all and typeof(value) == typeof(values[index]) and value == values[index]:
			continue
		values[index] = value
		for material: ShaderMaterial in cells["materials"]:
			material.set_shader_parameter(name, value)
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
	out["cell_sets_free"] = _cells_pool.size()
	out["refused"] = refused.size() + patch_lists_refused.size()
	out["desired"] = _desired.size()
	return out
