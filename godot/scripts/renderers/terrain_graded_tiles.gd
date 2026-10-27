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
## The road preview's graded tiles (TERRAIN-07-E) draw over the installed ones from slots of
## their own (show_preview); a commit's or undo's edit carries the tiles it staged, which install
## with the edit's batch instead of rebaking, and the batch releases the preview tiles of the
## epoch the commit adopted, in the same frame.
##
## Rust methods called: get_graded_tile_layout(), request_graded_patch_tiles(),
##   request_graded_tiles(), poll_graded_tile_results(), set_road_preview_terrain_view()
extends RefCounted

const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const GRADED_SHADER := preload("res://assets/materials/terrain_graded.gdshader")
const WALL_SHADER := preload("res://assets/materials/terrain_graded_walls.gdshader")
const TILE_M := 64.0
# Level distances with their hysteresis (T5, TD10): GL0 to 64 m, GL1 to 128 m, GL2 to 2 km,
# GL3 beyond, at the patch LOD rule's steps (2 to 5 km, 4 to 12 km, 8 beyond). Tuned by the GPU
# idle frame in TERRAIN-07-D round 8 (256 and 512 m drew GL0 and GL1 cells down to 1.8 px).
const LEVEL_END_M := [64.0, 128.0, 2000.0]
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
# The cell meshes of a patch: GL0-GL3 (0-3) and the base cells (CELL_BASE).
const CELL_MESHES := 5
const CELL_BASE := 4
# Wall face instances per patch, prewarmed with its cells: at most 42 faces per patch on the
# Kuopio reference save and none on the populated save (TERRAIN-07-D round 6 census, GL0). A
# patch with more draws the first ones and counts and logs the rest.
const MAX_WALLS_PER_PATCH := 256
# A visibility range no camera is within (metres): a patch's PlaneMesh while its cells draw it.
const HIDDEN_RANGE_M := 0.001
const WALL_FACE_FLOATS := 8
const SIDE_OFFSETS := [Vector2i(-1, 0), Vector2i(1, 0), Vector2i(0, -1), Vector2i(0, 1)]
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
# Tile key -> {code, level, step, slot, token, walls}: what each installed tile shows.
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
# Results polled after the frame's install budget ran out, in arrival order, as [kind, result]:
# the next frame takes them first (Rust hands a result over once).
var _deferred: Array[Array] = []
# A poll's result kinds, in the order a poll's results are taken.
const RESULT_KINDS := ["edits", "patches", "failed", "tiles"]
# The tiles the last edit batch installed, for tests and the probe.
var last_edit_batch: Array = []
# The road preview's tiles (TERRAIN-07-E): tile key -> {code, level, step, slot, token (the
# preview epoch), walls}, drawn in place of the installed tile. Their slots are their own.
var preview: Dictionary = {}
# The highest preview epoch the edits in the batch release when it installs (-1: none).
var _edit_release := -1
# The resident patches of the last process() call, for preview changes between frames.
var _resident: Dictionary = {}
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
# Tile key -> {level: Vector2i(code, step), token, bucket}: every tile a resident patch's list
# names. It follows the changes: a patch entering, leaving or getting a new list adds and
# removes its own tiles (each tile counts the lists naming it, in _tile_lists, and the patch's
# counted tiles are kept in _counted), and a camera move re-levels only the tiles due (T26).
var _desired: Dictionary = {}
var _tile_lists: Dictionary = {}
var _counted: Dictionary = {}
# The counts _recount queued that are not applied yet, in order, as (x, z, +1 or -1): the poll
# applies them one tile at a time under the install budget, before the next result (a list
# counted at once took up to 0.35 ms past the budget on the populated save, TERRAIN-07-D round
# 10). _counted already holds what they count to.
var _counts: Array[Vector3i] = []
var _counts_next := 0
# The camera (XZ, with its height above the ground as y) and the distance it has travelled since
# setup. A tile's distance changes by at most the camera's travel, so a tile levelled at travel t
# with its distance m from the nearest distance where its level can change keeps its level until
# the travel reaches t + m: it waits in bucket floor((t + m) / LEVEL_BUCKET_M) (bucket -> {tile:
# true}), and a frame re-levels the buckets the travel has reached since the last, its own
# included. Exact: every desired tile has its distance's level in every frame.
var _camera := Vector3.ZERO
var _camera_set := false
var _travel := 0.0
var _buckets: Dictionary = {}
var _bucket_first := 0
const LEVEL_BUCKET_M := 8.0
# The distances are float64 from float32 positions; their rounding is far below this.
const LEVEL_MARGIN_SLACK_M := 1e-6
# A patch's cells change per tile (T26): tiles whose instance or a side's code may have changed
# this frame (a tile installed at a new slot or lattice or evicted, with its four neighbours; the
# tiles along a patch whose residency or step changed).
var _dirty_cell_tiles: Dictionary = {}
# Patch key -> the LOD step of each resident patch, to tell a step change from other changes.
var _patch_steps: Dictionary = {}
# The tiles and patch lists that may need a request: a tile whose desire, level or token
# changed, whose request failed or did not go out (the per-frame budget or Rust's in-flight
# bound); a patch that changed or whose list arrived, failed or was not accepted. An idle frame
# has none.
var _wanted: Dictionary = {}
var _lists_wanted: Dictionary = {}
# Patch list requests per frame: Rust holds at most 32 in flight.
const LIST_REQUESTS_PER_FRAME := 32
var counters := {
	"textures_created": 0,
	"images_created": 0,
	"meshes_created": 0,
	"materials_created": 0,
	"multimeshes_created": 0,
	"uploads": 0,
	# Polled results (edits, lists, failures, tiles) a frame left to the next one, its budget
	# spent.
	"results_deferred": 0,
	"evictions": 0,
	"pool_exhausted": 0,
	"walls_truncated": 0,
	"bake_failures": 0,
	"edits": 0,
	"edit_batches": 0,
	"edit_tiles_last": 0,
	"edit_install_ms_last": 0.0,
	"edit_install_ms_max": 0.0,
	# The road preview's installs (show_preview), the most one frame took.
	"preview_install_ms_max": 0.0,
	"install_ms_last": 0.0,
	"install_ms_max": 0.0,
	# The streaming installs' own part of a frame (the polls and their installs), without the
	# re-levelling and the edit batch before them.
	"stream_install_ms_max": 0.0,
	"poll_ms_max": 0.0,
	"upload_ms_max": 0.0,
	# The last edit frame's parts (ms) and the patches whose cells or walls it rewrote.
	"edit_frame_parts": {},
	# Every frame's parts while the perf debug channel is on (terrain.gd reports them).
	"frame_parts": {},
	# The cell sets prewarmed at setup and the time it took (T25's load cost).
	"cell_sets_prewarmed": 0,
	"cell_prewarm_ms": 0.0,
}
# Per-frame accumulators of the parts the edit frame reports.
var _frame_sync_us := 0
var _frame_walls_us := 0
var _frame_lists := 0
var _frame_lists_same := 0
var _frame_relevelled := 0
var _frame_cell_tiles := 0

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
	var prewarm_us := Time.get_ticks_usec()
	for _patch in grid.x * grid.y:
		_cells_pool.append(_new_cells())
	counters["cell_sets_prewarmed"] = grid.x * grid.y
	counters["cell_prewarm_ms"] = float(Time.get_ticks_usec() - prewarm_us) / 1000.0

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
	for key in preview.keys():
		_free_slot(preview[key])
	preview.clear()
	_edit_release = -1
	requested.clear()
	tile_tokens.clear()
	_edit_batch.clear()
	_edit_staged.clear()
	_deferred.clear()
	patch_lists.clear()
	patch_lists_requested.clear()
	refused.clear()
	patch_lists_refused.clear()
	_dirty_walls.clear()
	_dirty_cell_tiles.clear()
	_patch_steps.clear()
	for key in patch_cells.keys():
		release_patch(key, null)
	_desired.clear()
	_tile_lists.clear()
	_counted.clear()
	_counts.clear()
	_counts_next = 0
	_camera_set = false
	_travel = 0.0
	_buckets.clear()
	_bucket_first = 0
	_wanted.clear()
	_lists_wanted.clear()

## One frame: polls finished work and installs it within the install budget, keeps the tiles of
## `resident_patches` (patch key -> {node, material, rect: Rect2, step, generation}) at their
## distance's level, evicts the rest and rewrites the instances of the patches that changed.
## `camera_position` is the camera's XZ with, as `y`, its height above the ground under it: a
## tile's distance is the camera's to the tile's centre on that ground.
## `changed_patches` holds the keys whose entry was added, rewritten or removed since the last
## call and `changed_materials` the keys whose material parameters may have changed (the caller
## clears both); with neither, nor a camera move, nor finished work, the frame only polls.
## O(changes) per frame: a changed patch or an arriving list costs its own tiles.
func process(camera_position: Vector3, resident_patches: Dictionary, changed_patches: Dictionary, changed_materials: Dictionary) -> void:
	var start_us := Time.get_ticks_usec()
	_resident = resident_patches
	_frame_sync_us = 0
	_frame_walls_us = 0
	_frame_lists = 0
	_frame_lists_same = 0
	_frame_relevelled = 0
	_frame_cell_tiles = 0
	var uploads_before := int(counters["uploads"])
	var requests_before := requested.size()
	# A camera move re-levels the tiles due first, so the tiles this frame adds take the camera's
	# position too. O(tiles due + buckets passed).
	_move_camera(camera_position)
	var relevel_us := Time.get_ticks_usec()
	# An edit's batch installs in a frame of its own: the poll that completes it leaves it for
	# the next frame, which installs it and polls nothing.
	var batch_tiles := _install_edit_batch(resident_patches)
	var batch_us := Time.get_ticks_usec()
	if batch_tiles == 0:
		_poll_and_install(start_us, resident_patches)
	var polled_us := Time.get_ticks_usec()
	counters["stream_install_ms_max"] = max(float(counters["stream_install_ms_max"]), float(polled_us - batch_us) / 1000.0)
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
	_apply_cell_changes(resident_patches)
	var cells_us := Time.get_ticks_usec()
	var wall_patches := _dirty_walls.size()
	_apply_wall_changes(resident_patches)
	var perf := PerfDebug.is_enabled()
	if batch_tiles > 0 or perf:
		var end_us := Time.get_ticks_usec()
		var ms := func(from_us: int, to_us: int) -> float: return float(to_us - from_us) / 1000.0
		var parts := {
			"relevel_ms": ms.call(start_us, relevel_us),
			"batch_ms": ms.call(relevel_us, batch_us), "poll_ms": ms.call(batch_us, polled_us),
			"residency_ms": ms.call(polled_us, residency_us), "watched_ms": ms.call(residency_us, watched_us),
			"cells_ms": ms.call(watched_us, cells_us), "cells_sync_ms": float(_frame_sync_us) / 1000.0,
			"cells_walls_ms": float(_frame_walls_us) / 1000.0, "walls_ms": ms.call(cells_us, end_us),
			"total_ms": ms.call(start_us, end_us),
			"cell_patches": cell_patches, "wall_patches": wall_patches,
			"cell_tiles": _frame_cell_tiles, "relevelled": _frame_relevelled,
			"lists": _frame_lists, "lists_unchanged": _frame_lists_same,
			"uploads": int(counters["uploads"]) - uploads_before,
			"requested": requested.size() - requests_before, "wanted": _wanted.size(),
			"desired": _desired.size(), "installed": installed.size(),
		}
		if perf:
			# What the cells submit (before culling): instances and triangles per cell mesh.
			for index in CELL_MESHES:
				parts["cells_%d" % index] = 0
				parts["cell_triangles_%d" % index] = 0
			for cells: Dictionary in patch_cells.values():
				for index in CELL_MESHES:
					var node: MultiMeshInstance3D = cells["nodes"][index]
					if node.visible and node.is_inside_tree():
						var count := node.multimesh.visible_instance_count
						parts["cells_%d" % index] += count
						parts["cell_triangles_%d" % index] += count * node.multimesh.mesh.surface_get_array_index_len(0) / 3
			counters["frame_parts"] = parts
		if batch_tiles > 0:
			counters["edit_frame_parts"] = parts
	if batch_tiles > 0:
		# The edit's frame: its uploads, the cell rewrites they caused and the frame's changes.
		counters["edit_batches"] += 1
		counters["edit_tiles_last"] = batch_tiles
		counters["edit_install_ms_last"] = float(Time.get_ticks_usec() - start_us) / 1000.0
		counters["edit_install_ms_max"] = max(float(counters["edit_install_ms_max"]), float(counters["edit_install_ms_last"]))

# A patch-wide change rewrites the patch's cells; every other change rewrites its own tiles'
# instances in the patches that draw them. O(changed patches' tiles + changed tiles).
func _apply_cell_changes(resident_patches: Dictionary) -> void:
	for key: Vector2i in _dirty_patches.keys():
		if resident_patches.has(key):
			_rebuild_patch_cells(key, resident_patches)
		_dirty_walls.erase(key)
	if not _dirty_cell_tiles.is_empty():
		_update_cell_tiles(resident_patches)
	_dirty_patches.clear()

func _apply_wall_changes(resident_patches: Dictionary) -> void:
	for key: Vector2i in _dirty_walls.keys():
		if resident_patches.has(key) and patch_cells.has(key):
			_rebuild_patch_walls(patch_cells[key], resident_patches[key])
	_dirty_walls.clear()

## Shows one road preview frame's graded tiles (TERRAIN-07-E, docs/terrain.md "Preview, commit,
## undo and load"): `changes` holds `graded_tiles` (packed as poll_graded_tile_results packs its
## tiles, the token the preview epoch) and `graded_removed` (flat (x, z) pairs), as preview_road
## and advance_road_preview_terrain return them. A shown tile takes a free slot of its level's
## pool and draws in place of the installed tile from this frame; a removed one returns to the
## installed tile. Returns the milliseconds it took. O(changed tiles x slot bytes).
func show_preview(changes: Dictionary) -> float:
	var start_us := Time.get_ticks_usec()
	var removed: PackedInt32Array = changes.get("graded_removed", PackedInt32Array())
	var tiles: Array = changes.get("graded_tiles", [])
	if removed.is_empty() and tiles.is_empty():
		return 0.0
	for index in range(0, removed.size(), 2):
		_drop_preview(Vector2i(removed[index], removed[index + 1]))
	for tile: Dictionary in tiles:
		_show_preview_tile(tile)
	_apply_cell_changes(_resident)
	_apply_wall_changes(_resident)
	var ms := float(Time.get_ticks_usec() - start_us) / 1000.0
	counters["preview_install_ms_max"] = max(float(counters["preview_install_ms_max"]), ms)
	return ms

## Drops the preview tiles of preview epochs `first_epoch` to `last_epoch`: the installed tiles
## draw again. A cancelled preview drops its own epoch; an edit batch drops every epoch up to the
## one its commit adopted.
func release_preview(first_epoch: int, last_epoch: int) -> void:
	for key: Vector2i in preview.keys():
		var token := int(preview[key]["token"])
		if token >= first_epoch and token <= last_epoch:
			_drop_preview(key)
	_apply_cell_changes(_resident)
	_apply_wall_changes(_resident)

func _show_preview_tile(tile: Dictionary) -> void:
	var key := Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))
	var code := int(tile["level"])
	var current: Dictionary = preview.get(key, {})
	var slot := -1
	if not current.is_empty() and int(current["level"]) == code:
		slot = int(current["slot"])
	else:
		var free: Array[int] = pools[code]["free"]
		if free.is_empty():
			counters["pool_exhausted"] += 1
			MetrumDebug.log("terrain", "graded pool %d exhausted at preview tile %s" % [code, str(key)])
			return
		if not current.is_empty():
			_free_slot(current)
		slot = free.pop_back()
	var before := _shown(key)
	_write_slot(code, slot, tile)
	var step := int(tile["step"])
	var walls: PackedFloat32Array = (tile["walls"] as PackedByteArray).to_float32_array()
	preview[key] = {"code": code if code < 3 else base_code(step), "level": code, "step": step, "slot": slot, "token": int(tile["token"]), "walls": walls}
	_shown_changed(key, before)

func _drop_preview(key: Vector2i) -> void:
	var entry: Dictionary = preview.get(key, {})
	if entry.is_empty():
		return
	_free_slot(entry)
	preview.erase(key)
	_shown_changed(key, entry)

# What `key` draws changed from `before`: its instance and its neighbours' sides, the walls when
# its faces changed, and a resident patch meeting it without cells is built whole (one left
# without a graded cell returns to its PlaneMesh in _update_cell_tiles).
func _shown_changed(key: Vector2i, before: Dictionary) -> void:
	_cell_changed(key)
	var after := _shown(key)
	if after.get("walls", PackedFloat32Array()) != before.get("walls", PackedFloat32Array()):
		_walls_changed(key)
	for patch_key in _patches_meeting(Rect2(Vector2(key) * TILE_M, Vector2.ONE * TILE_M)):
		if _resident.has(patch_key) and not patch_cells.has(patch_key):
			_dirty_patches[patch_key] = true

# What a tile draws: its preview tile while the road preview shows one, else its installed tile.
func _shown(tile: Vector2i) -> Dictionary:
	var entry: Dictionary = preview.get(tile, {})
	return entry if not entry.is_empty() else installed.get(tile, {})

# Writes a polled or previewed tile's texels and rows into `slot` of level `code`'s pool.
func _write_slot(code: int, slot: int, tile: Dictionary) -> void:
	var pool: Dictionary = pools[code]
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

# Takes finished work (edits, patch lists, failures, tile bakes, in that order per poll) until the
# frame's install budget is spent, checked before each result and before each tile a list or a
# patch change counts: a frame ends within the budget plus one result's or one count's work
# (round 7 checked it after each poll, and a poll hands over every finished list at once: 17
# lists took 2.1 ms in one frame on the populated save; round 10 checked it before each result,
# and one list counted up to 0.35 ms past it). Each result is taken after the counts the one
# before it queued, so the order of the changes is the arrival order whatever the frame breaks.
# The frame's first count or result is taken unconditionally, so the work progresses whatever
# the frame's other work cost. O(results and counts taken + one poll's results).
func _poll_and_install(start_us: int, resident_patches: Dictionary) -> void:
	var progressed := false
	while true:
		while not _counts.is_empty():
			if progressed and _over_budget(start_us):
				return
			_apply_count()
			progressed = true
		if _deferred.is_empty():
			break
		if progressed and _over_budget(start_us):
			return
		var item: Array = _deferred.pop_front()
		_take_result(item[0], item[1], resident_patches)
		progressed = true
	while true:
		if progressed and _over_budget(start_us):
			return
		var poll_start_us := Time.get_ticks_usec()
		var result: Dictionary = simulation_node.poll_graded_tile_results(POLL_BATCH)
		counters["poll_ms_max"] = max(float(counters["poll_ms_max"]), float(Time.get_ticks_usec() - poll_start_us) / 1000.0)
		for kind: String in RESULT_KINDS:
			for entry: Dictionary in result.get(kind, []):
				if progressed and _over_budget(start_us):
					_deferred.append([kind, entry])
					counters["results_deferred"] += 1
					continue
				_take_result(kind, entry, resident_patches)
				progressed = true
				# The budget spent, the rest of the counts and every later result wait.
				while not _counts.is_empty() and not _over_budget(start_us):
					_apply_count()
		if result.get("tiles", []).size() < POLL_BATCH:
			return

func _take_result(kind: String, entry: Dictionary, resident_patches: Dictionary) -> void:
	match kind:
		"edits":
			_apply_edit(entry)
		"patches":
			_take_list(entry, resident_patches)
		"failed":
			_failed(entry)
		"tiles":
			_install(entry, resident_patches)

# A patch's graded tile list from Rust: recorded, its tiles counted in the desired set.
func _take_list(list: Dictionary, resident_patches: Dictionary) -> void:
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

static func _over_budget(start_us: int) -> bool:
	return float(Time.get_ticks_usec() - start_us) / 1000.0 >= INSTALL_BUDGET_MS

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
	_write_slot(code, slot, tile)
	var cell_code := code if code < 3 else base_code(step)
	# A tile moving between levels frees its slot in the old level's pool (slot numbers repeat
	# across pools: round 7 compared the numbers and kept a GL0 slot 0 moving to GL1 slot 0).
	if not current.is_empty() and int(current["level"]) != code:
		_free_slot(current)
	var walls: PackedFloat32Array = (tile["walls"] as PackedByteArray).to_float32_array()
	installed[key] = {"code": cell_code, "level": code, "step": step, "slot": slot, "token": token, "walls": walls}
	# The installed level moves the hysteresis: the tile is levelled again at once. A bake of an
	# older level or token (the camera moved, an edit dirtied it) still wants one.
	var want: Dictionary = _desired.get(key, {})
	if not want.is_empty():
		_level_tile(key, want)
		if want["level"] != Vector2i(code, step) or int(want["token"]) != token:
			_wanted[key] = true
	# Rewritten in place, the tile keeps its instances; a new slot or lattice rewrites its own
	# instances and its neighbours' sides, and changed faces the walls of the patches that draw them.
	if current.is_empty() or int(current["level"]) != code or int(current["code"]) != cell_code:
		_cell_changed(key)
		# A resident patch meeting the tile without cells yet is built whole.
		for patch_key in _patches_meeting(Rect2(Vector2(key) * TILE_M, Vector2.ONE * TILE_M)):
			if resident_patches.has(patch_key) and not patch_cells.has(patch_key):
				_dirty_patches[patch_key] = true
	if walls != current.get("walls", PackedFloat32Array()):
		_walls_changed(key)

# Installs the edit batch once every tile of it has its result, all in this frame. Returns how
# many tiles it installed.
func _install_edit_batch(resident_patches: Dictionary) -> int:
	if _edit_batch.is_empty() and _edit_release >= 0:
		# A commit whose staged tiles no held tile takes: its preview ends with the edit.
		release_preview(0, _edit_release)
		_edit_release = -1
	if _edit_batch.is_empty() or _edit_staged.size() < _edit_batch.size():
		return 0
	for key: Vector2i in _edit_staged.keys():
		_upload(key, _edit_staged[key], resident_patches)
	last_edit_batch = _edit_staged.keys()
	var count := _edit_staged.size()
	_edit_batch.clear()
	_edit_staged.clear()
	# The committed tiles replace the preview's in the same frame.
	if _edit_release >= 0:
		for key: Vector2i in preview.keys():
			if int(preview[key]["token"]) <= _edit_release:
				_drop_preview(key)
		_edit_release = -1
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
	# A commit's or undo's own tiles (TERRAIN-07-E): bitwise the rebakes of the edit's inputs, so
	# they install with the batch instead of rebaking. A tile neither held nor wanted installs
	# only where the preview shows it graded, so it draws on in the batch's frame; its patch's
	# list, refreshed with the edit's patch generations, then wants it. Any other tile waits for
	# that list, as a newly graded tile does.
	for tile: Dictionary in edit.get("staged", []):
		var key := Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))
		var held := installed.has(key) or _desired.has(key)
		if not held and not (preview.has(key) and bool(tile["graded"])):
			continue
		_dirty_tile(key, serial)
		_edit_batch[key] = int(tile_tokens[key])
		tile["token"] = int(tile_tokens[key])
		_edit_staged[key] = tile
	_edit_release = max(_edit_release, int(edit.get("preview_epoch", -1)))

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

# Tile `key` was installed at a new slot or lattice, or evicted: its own instance and the sides
# of its four neighbours change (the cell code is all a side reads of its neighbour), each in
# the patches that draw it. O(1).
func _cell_changed(key: Vector2i) -> void:
	_dirty_cell_tiles[key] = true
	for offset: Vector2i in SIDE_OFFSETS:
		_dirty_cell_tiles[key + offset] = true

# Patch `key`'s residency or step changed: the tiles just outside its sides that lie on tile
# lines stitch across them to its step (_side_code), so their instances are rewritten. O(the
# patch's side tiles).
func _border_changed(key: Vector2i) -> void:
	var origin := world_min + Vector2(key) * patch_span_m
	var rect := Rect2(origin, (-world_min - origin).min(Vector2.ONE * patch_span_m))
	var first := Vector2i((rect.position / TILE_M).floor()) - Vector2i.ONE
	var last := Vector2i((rect.end / TILE_M).ceil())
	for axis in 2:
		var across := 1 - axis
		for side: float in [rect.position[axis], rect.end[axis]]:
			if fposmod(side, TILE_M) != 0.0:
				continue
			# The tile on the far side of the line from the patch.
			var column := int(side / TILE_M) - (1 if side == rect.position[axis] else 0)
			for row in range(first[across], last[across] + 1):
				var tile := Vector2i.ZERO
				tile[axis] = column
				tile[across] = row
				_dirty_cell_tiles[tile] = true

# Tile `key`'s wall faces changed: the patches that may draw one (a face draws in the patch
# holding its midpoint) rewrite their wall instances.
func _walls_changed(key: Vector2i) -> void:
	for patch_key in _patches_meeting(Rect2(Vector2(key) * TILE_M, Vector2.ONE * TILE_M)):
		_dirty_walls[patch_key] = true

# A resident patch entry was added, rewritten or removed: its counted tiles follow its residency,
# its list is asked again if its generation moved, and its cells follow its LOD step.
func _patch_changed(key: Vector2i, resident_patches: Dictionary) -> void:
	_recount(key, resident_patches)
	# The cells across its sides stitch to its step, or to their own while it is not resident.
	var step := int(resident_patches[key]["step"]) if resident_patches.has(key) else -1
	if step != int(_patch_steps.get(key, -1)):
		_border_changed(key)
		if step < 0:
			_patch_steps.erase(key)
		else:
			_patch_steps[key] = step
	if not resident_patches.has(key):
		_lists_wanted.erase(key)
		return
	_lists_wanted[key] = true
	var cells: Dictionary = patch_cells.get(key, {})
	if not cells.is_empty() and int(cells.get("step", -1)) != int(resident_patches[key]["step"]):
		_dirty_patches[key] = true

# The tiles patch `key` adds to the desired set: its list while it is resident (a patch whose
# list is being refreshed keeps its last one until the new one arrives, so an edit's patch shows
# its tiles meanwhile), queued as counts (_apply_count). O(the patch's old and new tiles).
func _recount(key: Vector2i, resident_patches: Dictionary) -> void:
	var tiles: Array = []
	if resident_patches.has(key):
		tiles = patch_lists.get(key, {}).get("tiles", [])
	var old: Array = _counted.get(key, [])
	if old == tiles:
		return
	# The new list counts first, so a tile on both lists never leaves.
	for tile: Vector2i in tiles:
		_counts.append(Vector3i(tile.x, tile.y, 1))
	for tile: Vector2i in old:
		_counts.append(Vector3i(tile.x, tile.y, -1))
	if tiles.is_empty():
		_counted.erase(key)
	else:
		_counted[key] = tiles

# Applies the next queued count: a tile enters the desired set with the first list naming it and
# leaves with the last.
func _apply_count() -> void:
	var counted: Vector3i = _counts[_counts_next]
	_counts_next += 1
	if _counts_next == _counts.size():
		_counts.clear()
		_counts_next = 0
	var tile := Vector2i(counted.x, counted.y)
	var count := int(_tile_lists.get(tile, 0)) + counted.z
	if count > 0:
		_tile_lists[tile] = count
		if count == 1 and counted.z > 0:
			_desire(tile)
		return
	_tile_lists.erase(tile)
	_undesire(tile)

# A tile some resident patch lists: its level at the camera's distance, with hysteresis around
# the level it is installed at.
func _desire(tile: Vector2i) -> void:
	var want := {"level": Vector2i(-1, -1), "token": int(tile_tokens.get(tile, 0)), "bucket": -1}
	_desired[tile] = want
	_level_tile(tile, want)
	_wanted[tile] = true

# A tile no resident patch lists any more: its slot is freed and its requests, token and
# refusals dropped (only held tiles keep a token).
func _undesire(tile: Vector2i) -> void:
	_unbucket(tile, int(_desired[tile]["bucket"]))
	_desired.erase(tile)
	_wanted.erase(tile)
	if installed.has(tile):
		var entry: Dictionary = installed[tile]
		_free_slot(entry)
		installed.erase(tile)
		counters["evictions"] += 1
		_cell_changed(tile)
		if not (entry["walls"] as PackedFloat32Array).is_empty():
			_walls_changed(tile)
	_edit_batch.erase(tile)
	_edit_staged.erase(tile)
	for level: Vector2i in LEVEL_STEPS:
		var request_key := "%d,%d,%d,%d" % [tile.x, tile.y, level.x, level.y]
		requested.erase(request_key)
		refused.erase(request_key)
	tile_tokens.erase(tile)

# The camera's distance to a tile (docs/terrain.md "LOD and distant tiles"): to the tile's centre
# on the ground under the camera, from the camera's height above that ground, so a camera high
# over a tile takes the level of its distance, not of its offset along the ground. In float64.
func _tile_distance(tile: Vector2i) -> float:
	var dx := (float(tile.x) + 0.5) * TILE_M - _camera.x
	var dz := (float(tile.y) + 0.5) * TILE_M - _camera.z
	return sqrt(dx * dx + _camera.y * _camera.y + dz * dz)

# The camera moved: its travel grows by the move and the buckets it reached since the last move
# are levelled again, the last one included (a tile due within it waits there). O(tiles due +
# buckets passed).
func _move_camera(camera_position: Vector3) -> void:
	if not _camera_set or camera_position != _camera:
		# The road preview stages its tiles at this rule's levels, without the hysteresis (T42).
		var gl3_end_m := PackedFloat64Array()
		for entry: Array in GL3_STEP_END_M:
			gl3_end_m.append(float(entry[1]))
		simulation_node.set_road_preview_terrain_view(camera_position, PackedFloat64Array(LEVEL_END_M), gl3_end_m)
	if not _camera_set:
		_camera = camera_position
		_camera_set = true
		return
	var dx := float(camera_position.x) - _camera.x
	var dy := float(camera_position.y) - _camera.y
	var dz := float(camera_position.z) - _camera.z
	var moved := sqrt(dx * dx + dy * dy + dz * dz)
	if moved == 0.0:
		return
	_camera = camera_position
	_travel += moved
	var last := int(floor(_travel / LEVEL_BUCKET_M))
	for bucket in range(_bucket_first, last + 1):
		if not _buckets.has(bucket):
			continue
		var tiles: Dictionary = _buckets[bucket]
		_buckets.erase(bucket)
		for tile: Vector2i in tiles.keys():
			var want: Dictionary = _desired[tile]
			want["bucket"] = -1
			_level_tile(tile, want)
			_frame_relevelled += 1
	_bucket_first = last

# A desired tile's level at the camera's distance, with hysteresis around its installed level (a
# tile whose level changed wants a bake at it), and its bucket: the travel at which its distance
# can first reach a distance where that level changes.
func _level_tile(tile: Vector2i, want: Dictionary) -> void:
	var distance := _tile_distance(tile)
	var current_code := int(installed.get(tile, {}).get("level", -1))
	var level := desired_level(distance, current_code)
	if level != want["level"]:
		want["level"] = level
		_wanted[tile] = true
	var margin: float = max(_level_margin(distance, current_code) - LEVEL_MARGIN_SLACK_M, 0.0)
	var bucket := int(floor((_travel + margin) / LEVEL_BUCKET_M))
	if bucket == int(want["bucket"]):
		return
	_unbucket(tile, int(want["bucket"]))
	if not _buckets.has(bucket):
		_buckets[bucket] = {}
	_buckets[bucket][tile] = true
	want["bucket"] = bucket

func _unbucket(tile: Vector2i, bucket: int) -> void:
	if not _buckets.has(bucket):
		return
	var tiles: Dictionary = _buckets[bucket]
	tiles.erase(tile)
	if tiles.is_empty():
		_buckets.erase(bucket)

# The distance from `distance_m` to the nearest distance at which desired_level(_, current_code)
# changes value: each level's end, with the hysteresis desired_level adds around the installed
# level, and the GL3 steps' ends.
func _level_margin(distance_m: float, current_code: int) -> float:
	var margin := INF
	for index in LEVEL_END_M.size():
		var end_m: float = LEVEL_END_M[index]
		if current_code >= 0 and current_code <= index:
			end_m += LEVEL_HYSTERESIS_M
		margin = min(margin, abs(distance_m - end_m))
	for entry: Array in GL3_STEP_END_M:
		margin = min(margin, abs(distance_m - float(entry[1])))
	return margin

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
		wanted.append([_tile_distance(tile), tile, level, token, request_key])
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
	var entry := _shown(tile)
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

# The cell MultiMesh a tile draws in: its installed level's, else the base cells'.
func _cell_mesh(tile: Vector2i) -> int:
	var entry := _shown(tile)
	return int(entry["level"]) if not entry.is_empty() else CELL_BASE

# A tile's instance custom data in a patch's cells, for the patch's rewrite and the per-tile
# updates alike: its place in the patch (column + PATCH_TILE_ROW * row from `first`, the tile
# holding the patch's lower corner), its slot (-1 for a base cell), its own lattice code and
# each side's stitching code. Every channel an integer below 2048: the Compatibility renderer
# keeps instance custom data as half floats.
func _cell_custom(tile: Vector2i, first: Vector2i, patch: Dictionary, resident_patches: Dictionary) -> Color:
	var own := _cell_code(tile, int(patch["step"]))
	var origin := Vector2(tile) * TILE_M
	var codes := own
	codes |= _side_code(own, tile, tile + Vector2i(-1, 0), origin.x, patch, resident_patches, 0) << 3
	codes |= _side_code(own, tile, tile + Vector2i(1, 0), origin.x + TILE_M, patch, resident_patches, 0) << 6
	codes |= _side_code(own, tile, tile + Vector2i(0, -1), origin.y, patch, resident_patches, 1) << 9
	codes |= _side_code(own, tile, tile + Vector2i(0, 1), origin.y + TILE_M, patch, resident_patches, 1) << 12
	var entry := _shown(tile)
	var slot := int(entry["slot"]) if not entry.is_empty() else -1
	var place := tile - first
	return Color(place.x + PATCH_TILE_ROW * place.y, slot, codes & 511, codes >> 9)

# A tile's index in its cell set's per-place record (`where`), row-major over the patch's tiles.
static func _cell_place(cells: Dictionary, tile: Vector2i) -> int:
	var first: Vector2i = cells["first"]
	var last: Vector2i = cells["last"]
	return (tile.x - first.x) + (last.x - first.x + 1) * (tile.y - first.y)

# Rewrites the queued tiles' instances in every patch drawing cells that holds them (the patches
# rewritten whole this frame excepted), and returns a patch left without a graded cell to its
# PlaneMesh. O(queued tiles).
func _update_cell_tiles(resident_patches: Dictionary) -> void:
	var touched := {}
	for tile: Vector2i in _dirty_cell_tiles.keys():
		for key in _patches_meeting(Rect2(Vector2(tile) * TILE_M, Vector2.ONE * TILE_M)):
			if _dirty_patches.has(key) or not patch_cells.has(key) or not resident_patches.has(key):
				continue
			if _update_tile_cell(patch_cells[key], tile, resident_patches[key], resident_patches):
				touched[key] = true
	_dirty_cell_tiles.clear()
	for key: Vector2i in touched.keys():
		var counts: Array[int] = patch_cells[key]["counts"]
		if counts[0] + counts[1] + counts[2] + counts[3] == 0:
			release_patch(key, resident_patches[key]["node"])

# One tile's instance in a patch's cells (T26): its custom data rewritten in place while its
# mesh is unchanged; otherwise it leaves the old mesh's MultiMesh by a swap-remove (the last
# instance takes its index) and is appended to the new one's. Returns whether anything changed.
# O(1).
func _update_tile_cell(cells: Dictionary, tile: Vector2i, patch: Dictionary, resident_patches: Dictionary) -> bool:
	var first: Vector2i = cells["first"]
	var last: Vector2i = cells["last"]
	if tile.x < first.x or tile.y < first.y or tile.x > last.x or tile.y > last.y:
		return false
	_frame_cell_tiles += 1
	var where: Array[Vector2i] = cells["where"]
	var place := _cell_place(cells, tile)
	var at: Vector2i = where[place]
	var mesh_index := _cell_mesh(tile)
	var custom := _cell_custom(tile, first, patch, resident_patches)
	var records: Array = cells["custom"]
	var nodes: Array = cells["nodes"]
	if at.x == mesh_index:
		if records[at.x][at.y] == custom:
			return false
		records[at.x][at.y] = custom
		(nodes[at.x] as MultiMeshInstance3D).multimesh.set_instance_custom_data(at.y, custom)
		return true
	var counts: Array[int] = cells["counts"]
	var tile_at: Array = cells["tile_at"]
	var end := counts[at.x] - 1
	if at.y != end:
		var moved: Vector2i = tile_at[at.x][end]
		records[at.x][at.y] = records[at.x][end]
		tile_at[at.x][at.y] = moved
		where[_cell_place(cells, moved)] = at
		(nodes[at.x] as MultiMeshInstance3D).multimesh.set_instance_custom_data(at.y, records[at.x][at.y])
	_set_cell_count(nodes[at.x], counts, at.x, end)
	var index := counts[mesh_index]
	records[mesh_index][index] = custom
	tile_at[mesh_index][index] = tile
	where[place] = Vector2i(mesh_index, index)
	(nodes[mesh_index] as MultiMeshInstance3D).multimesh.set_instance_custom_data(index, custom)
	_set_cell_count(nodes[mesh_index], counts, mesh_index, index + 1)
	return true

static func _set_cell_count(node: MultiMeshInstance3D, counts: Array[int], index: int, count: int) -> void:
	counts[index] = count
	node.multimesh.visible_instance_count = count
	node.visible = count > 0

func _rebuild_patch_cells(key: Vector2i, resident_patches: Dictionary) -> void:
	var patch: Dictionary = resident_patches[key]
	var list: Dictionary = patch_lists.get(key, {})
	var tiles := _patch_tiles(patch["rect"])
	var any_graded := false
	var any_preview := false
	for tile in tiles:
		any_graded = any_graded or installed.has(tile)
		any_preview = any_preview or preview.has(tile)
	var patch_node: GeometryInstance3D = patch["node"]
	if not any_preview and (not any_graded or list.is_empty()):
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
	var nodes: Array = cells["nodes"]
	# The tile holding the patch rectangle's lower corner, as the shader finds it, and the last.
	var first: Vector2i = tiles[0] if not tiles.is_empty() else Vector2i.ZERO
	cells["first"] = first
	cells["last"] = tiles[-1] if not tiles.is_empty() else Vector2i(-1, -1)
	# What each MultiMesh holds, kept beside it (the per-tile updates move instances by it).
	var counts: Array[int] = cells["counts"]
	counts.fill(0)
	var records: Array = cells["custom"]
	var tile_at: Array = cells["tile_at"]
	var where: Array[Vector2i] = cells["where"]
	for tile in tiles:
		var mesh_index := _cell_mesh(tile)
		var custom := _cell_custom(tile, first, patch, resident_patches)
		var index := counts[mesh_index]
		(nodes[mesh_index] as MultiMeshInstance3D).multimesh.set_instance_custom_data(index, custom)
		records[mesh_index][index] = custom
		tile_at[mesh_index][index] = tile
		where[_cell_place(cells, tile)] = Vector2i(mesh_index, index)
		counts[mesh_index] = index + 1
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
		var entry := _shown(tile)
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
	_bind_materials(cells, false)
	_cells_pool.append(cells)

# A pooled cell set, bounded to the patch's culling box. The pool holds one per patch of the
# grid (setup), so this creates none; an empty pool would mean more patches than the grid.
func _acquire_cells(patch: Dictionary) -> Dictionary:
	var cells: Dictionary = _cells_pool.pop_back() if not _cells_pool.is_empty() else _new_cells()
	var aabb: AABB = patch["aabb"]
	for node: MultiMeshInstance3D in cells["nodes"] + [cells["walls"]]:
		node.multimesh.custom_aabb = aabb
		node.custom_aabb = aabb
	_bind_materials(cells, true)
	return cells

# A set's graded materials are bound to its nodes only while it draws a patch (the base cells take
# the patch's material in _rebuild_patch_cells): Godot reserves a node's instance uniforms, 16 of
# its global buffer's slots, while the node's material declares them, so the pooled sets hold none
# (prewarmed bound, Kuopio's 1,296 sets held 5,184 nodes' slots, TERRAIN-07-D round 9). No
# material is created: the set's own are bound.
func _bind_materials(cells: Dictionary, bound: bool) -> void:
	var nodes: Array = cells["nodes"]
	var materials: Array[ShaderMaterial] = cells["materials"]
	for code in materials.size():
		(nodes[code] as GeometryInstance3D).material_override = materials[code] if bound else null
	if not bound:
		(nodes[CELL_BASE] as GeometryInstance3D).material_override = null

# One patch's cell resources: five cell MultiMeshes (GL0-GL3 and the base cells), the four graded
# levels' materials and the wall MultiMesh.
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
	# The record of what each cell MultiMesh holds (_rebuild_patch_cells, _update_tile_cell): per
	# mesh the custom data and tile of each instance and the instance count, per tile place its
	# (mesh, instance), all sized once here.
	var records := []
	var tile_at := []
	for index in CELL_MESHES:
		var custom: Array[Color] = []
		custom.resize(_cells_per_patch)
		records.append(custom)
		var tiles: Array[Vector2i] = []
		tiles.resize(_cells_per_patch)
		tile_at.append(tiles)
	var where: Array[Vector2i] = []
	where.resize(_cells_per_patch)
	var counts: Array[int] = []
	counts.resize(CELL_MESHES)
	return {
		"nodes": nodes, "walls": walls, "materials": materials, "watched": [], "node_parent": null,
		"custom": records, "tile_at": tile_at, "where": where, "counts": counts,
		"first": Vector2i.ZERO, "last": Vector2i(-1, -1),
	}

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
	out["preview"] = preview.size()
	out["patches_with_cells"] = patch_cells.size()
	out["cell_sets_free"] = _cells_pool.size()
	out["refused"] = refused.size() + patch_lists_refused.size()
	out["desired"] = _desired.size()
	return out
