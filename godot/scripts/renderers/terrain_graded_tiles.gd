# SPDX-License-Identifier: GPL-2.0-only

## Graded terrain tiles (TERRAIN-07-D, docs/terrain.md "Godot tile renderer"): residency of the
## graded 64 m tiles of the resident patches by camera distance, the pages (docs/terrain.md "Pages
## and the row atlas": per level several, each a Texture2DArray of slot layers and one of row atlas
## layers, a block of patches binding one page per level, a tile held in each page its patches'
## blocks use), in-place uploads from pooled Images per level, and per patch one MultiMesh per
## cell mesh (the base cells and GL0-GL3) whose instances carry the tile's place in the patch, slot
## and stitching codes, each graded level with its own material binding its block's page (the
## pages' allocator and row atlas uploads are terrain_graded_pages.gd's). It does no
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
const GRADED_BEHIND_SHADER := preload("res://assets/materials/terrain_graded_behind.gdshader")
const WALL_SHADER := preload("res://assets/materials/terrain_graded_walls.gdshader")
const GradedPages := preload("res://scripts/renderers/terrain_graded_pages.gd")
const TILE_M := 64.0
# Level distances with their hysteresis (T5, TD10): GL0 to 64 m, GL1 to 128 m, GL2 to 2 km,
# GL3 beyond, at the patch LOD rule's steps (2 to 5 km, 4 to 12 km, 8 beyond). Tuned by the GPU
# idle frame in TERRAIN-07-D round 8 (256 and 512 m drew GL0 and GL1 cells down to 1.8 px).
const LEVEL_END_M := [64.0, 128.0, 2000.0]
const GL3_STEP_END_M := [[2, 5000.0], [4, 12000.0], [8, INF]]
const LEVEL_HYSTERESIS_M := 32.0
# Pages (docs/terrain.md "Pages and the row atlas", T54 (a) and T50 (b), TERRAIN-07-G round 8).
# GL0 and GL1 have one page each for the world, sized by their bands' bound (the memory table);
# GL2 and GL3 one per block of patches and level, sized for the block's listed tiles.
const WORLD_PAGE_SLOTS := [64, 192]
# Atlas layers a block page holds per slot: the census's mean rows (455-620 texels) take two or
# three layers.
const PAGE_LAYERS_PER_SLOT := 3
# Patch cells: a patch meets at most (ceil(span / 64 m) + 1)^2 tiles (9 x 9 on a 510 m patch),
# the instances of each cell MultiMesh (_cells_per_patch). A tile's place in its patch is
# column + PATCH_TILE_ROW * row in one instance channel (terrain_cells.gdshaderinc).
const PATCH_TILE_ROW := 32
const REQUESTS_PER_FRAME := 24
const INSTALL_BUDGET_MS := 1.0
const POLL_BATCH := 4
# The cell meshes of a patch: GL0-GL3 (0-3), the base cells (CELL_BASE) and GL0-GL3 of the tiles
# whose bake flags them behind the road (CELL_BEHIND + level, T49 (c)): their cells draw the
# behind-the-road variant, which writes depth, so every other cell keeps the early depth test.
# Every set holds the first CELL_PLAIN; the variant's four meshes and materials only a set whose
# patch has shown a flagged tile (_add_variant, `TERRAIN-07-G` round 10).
const CELL_MESHES := 9
const CELL_BASE := 4
const CELL_BEHIND := 5
const CELL_PLAIN := 5
# Three transform rows followed by custom data, as MultiMesh.TRANSFORM_3D packs them.
const CELL_INSTANCE_FLOATS := 16
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
# The pages (terrain_graded_pages.gd): each level's layer shapes, every page by id, its slots and
# atlas runs, the budget and the free pool.
var page_store := GradedPages.new()
# The device's array layer limit (L) and the block side (B) in patches at GL2 and GL3.
var layer_limit := GradedPages.MIN_LAYER_LIMIT
var block_patches := 1
# The patch grid of the world (patches per axis).
var _patch_grid := Vector2i.ZERO
# Per level code: block -> the page its patches' sets bind; block -> the larger page a growing
# block moves to (its tiles fill it, then the sets bind it); block -> tiles that found the
# bound page full and wait for the move.
var _block_page: Array[Dictionary] = [{}, {}, {}, {}]
var _block_target: Array[Dictionary] = [{}, {}, {}, {}]
var _block_waiting: Array[Dictionary] = [{}, {}, {}, {}]
# Growing blocks whose target took a copy since the last check, as Vector3i(code, block x, z),
# and the tiles waiting for a move -> the same.
var _growth_dirty: Dictionary = {}
var _growth_waits: Dictionary = {}
# A block page not made yet because a resident patch of the block waits for its list.
const PAGE_PENDING := -2
# Tile key -> {code, level, step, token, walls, behind, copies}: what each installed tile shows;
# `copies` maps a page id to its copy there, Vector3i(slot, first atlas layer, layers).
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
# The road preview's tiles (TERRAIN-07-E): tile key -> {code, level, step, token (the preview
# epoch), walls, behind, copies}, drawn in place of the installed tile. Their copies are their own.
var preview: Dictionary = {}
# The highest preview epoch the edits in the batch release when it installs (-1: none).
var _edit_release := -1
# The serial of the last edit taken (Rust hands them over in serial order), and the one the
# last edit batch installed up to: installed_edit_serial() (T61 (2)).
var _edit_serial_taken := 0
var _edit_serial_batched := 0
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
# Tiles whose level's pool had no free slot (an exhausted pool, the typed error the pool sizes are
# meant to rule out, T23): tile key -> [level code, request key, token, asked again]. Counted and
# logged once, drawn as a refused tile is (its base cell) instead of baked again every frame
# (`TERRAIN-07-G` round 7: the 100k city's saved pose wants more `GL2` and `GL3` tiles than the
# pools' 2,048 slots each, and the re-asked bakes starved every farther tile, T54). A freed slot
# asks one of its level again (_free_copy), which stays recorded, so its patches draw as they did,
# until it installs or finds the pool exhausted again. Per level, the tiles in the order they were
# recorded (entries no longer recorded so are skipped as they come up).
var _exhausted: Dictionary = {}
var _exhausted_order: Array[Array] = [[], [], [], []]
# Patch key -> {root: Node3D (the set's nodes), nodes: Array[MultiMeshInstance3D],
# materials: Array[ShaderMaterial] (GL0-GL3), watched: Array}.
var patch_cells: Dictionary = {}
var _cell_mask_revision := 0
var _cells_pool: Array[Dictionary] = []
var _cells_per_patch := 81
var _meshes: Dictionary = {}
var _wall_mesh: ArrayMesh = null
var _wall_material: ShaderMaterial = null
var _dirty_patches: Dictionary = {}
# Patches whose wall instances must be rewritten (a tile rewritten in place changed its faces).
var _dirty_walls: Dictionary = {}
# Cell sets built off the frame paths (`TERRAIN-07-E` round 6): every resident patch gets its set,
# kept current by the per-tile updates and hidden (its root under the patch's node but not shown,
# no material bound) until one of its tiles shows graded, so a patch's first graded tile is a
# per-tile update and the set's flip (_apply_shown), never a whole build in a preview's, an
# edit's or a stream's frame. Patch key ->
# true in arrival order; `_builds_urgent` holds the patches a shown tile waits for (it draws once
# the set is built), taken first (_advance_builds).
var _builds: Dictionary = {}
var _builds_urgent: Dictionary = {}
# The behind-the-road variant (T49 (c)) on demand: patch key -> true for the sets the build path
# adds it to, first (_advance_builds), and the flagged tiles waiting for it in a set of theirs, so
# no set draws a flagged tile without its variant: tile key -> its bake result (installed once
# its sets hold the variant) and tile key -> its preview tile (shown then: the preview's lag,
# T27 (d)). An edit batch with a flagged tile waits whole (_install_edit_batch).
var _variants_wanted: Dictionary = {}
var _variant_waits: Dictionary = {}
var _preview_waits: Dictionary = {}
# Patches whose set may start or stop drawing.
var _dirty_shown: Dictionary = {}
# What a resident patch draws (_draw_state, T53 (a)): its PlaneMesh, its set or nothing.
const DRAW_PLANE := 0
const DRAW_SET := 1
const DRAW_NONE := 2
# Patch key -> the patch node whose PlaneMesh this renderer collapsed (_set_plane).
var _collapsed: Dictionary = {}
# Resident patches drawing nothing (DRAW_NONE) -> Vector2i(msec, process frame) since when.
var _unready: Dictionary = {}
## The view's frustum (terrain.gd sets it each frame from its camera, Camera3D.get_frustum()): an
## unready patch whose culling box meets it counts in `unready_in_view` (empty: none is counted).
var view_planes: Array[Plane] = []
## While a world load waits for the render to settle (the load flow holds the game until
## terrain.gd's is_render_settled()), the graded work takes the whole frame, not the streaming
## install budget's spare time.
var unbudgeted := false
## Whether terrain.gd's patch residency still has pending work this frame (its payloads,
## acknowledgements, LOD refreshes): while a load settles, a block's page waits for it
## (_block_lists_pending).
var residency_settling := false
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
	# Uploads that rewrote the tile's existing pool slot at the same level.
	"same_level_uploads": 0,
	# Resident patch mask revisions advanced after their graded cell payload changed.
	"mask_revision_bumps": 0,
	# Texel and row bytes written into the pools' layers.
	"upload_bytes": 0,
	# Polled results (edits, lists, failures, tiles) a frame left to the next one, its budget
	# spent.
	"results_deferred": 0,
	"evictions": 0,
	"pool_exhausted": 0,
	# Preview tiles a full page left to a later pose while its block grows (T27 (d)'s lag).
	"preview_page_waits": 0,
	# Pages created (not reused from the free pool), blocks moved to a larger page and moves
	# started (docs/terrain.md "Pages and the row atlas").
	"pages_created": 0,
	"page_moves": 0,
	"page_growths": 0,
	"walls_truncated": 0,
	"bake_failures": 0,
	"edits": 0,
	"edit_batches": 0,
	"edit_tiles_last": 0,
	# Committed tiles that took over the preview's slot instead of an upload (_adopt_preview).
	"edit_adopted": 0,
	"edit_install_ms_last": 0.0,
	"edit_install_ms_max": 0.0,
	# The road preview's installs (show_preview), the most one frame took; the frames that
	# changed the preview, and the last such frame's milliseconds and tiles.
	"preview_install_ms_max": 0.0,
	"preview_frames": 0,
	"preview_install_ms_last": 0.0,
	"preview_tiles_last": 0,
	"preview_parts": {},
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
	# Cell sets built (_build_tiles), and the patches waiting for one.
	"cell_builds": 0,
	"cell_prewarm_ms": 0.0,
	# Sets given the behind-the-road variant (_add_variant), and flagged tiles that waited for it.
	"cell_variants_created": 0,
	"variant_waits": 0,
	# T53 (a): the most milliseconds and frames a resident patch drew nothing before it was
	# ready, and the unready patches within the view distance, summed over frames and the most in
	# one frame (the residency margin's acceptance: 0 while the camera pans).
	"ready_ms_max": 0,
	"ready_frames_max": 0,
	"unready_in_view": 0,
	"unready_in_view_max": 0,
}
# Per-frame accumulators of the parts the edit frame reports.
var _frame_sync_us := 0
var _frame_walls_us := 0
var _frame_lists := 0
var _frame_lists_same := 0
var _frame_relevelled := 0
var _frame_cell_tiles := 0
# Sets that started or stopped drawing, and the time it took, since the counter's reader reset it.
var _frame_flips := 0
var _frame_flip_us := 0
# The per-tile updates' instance customs computed, instances written (moved between meshes among
# them) and the time each part took.
var _frame_custom_us := 0
var _frame_write_us := 0
var _frame_written := 0
var _frame_moved := 0

## Reads the slot layout, creates the world pages of GL0 and GL1 and prewarms the cell meshes.
## `pool_slots` overrides the per-level most slots a page holds (tests use small pages).
func setup(simulation: Node, cell_m: float, world_size: Vector2, span_m: float, pool_slots: Array = []) -> void:
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
	layer_limit = GradedPages.device_layer_limit()
	# B: the largest block of patches whose tiles fit a quarter of the layers (docs/terrain.md
	# "Blocks"), so a full block's slots and its tiles' mean atlas runs fit one page.
	block_patches = 1
	while _block_tiles(block_patches + 1) <= layer_limit / 4:
		block_patches += 1
	page_store.counters = counters
	page_store.levels.clear()
	for code in levels.size():
		var cap: int = WORLD_PAGE_SLOTS[code] if code < 2 else _block_tiles(block_patches)
		if code < pool_slots.size():
			cap = int(pool_slots[code])
		page_store.add_level(levels[code], mini(cap, layer_limit))
	_reset_pages()
	_build_meshes()
	_wall_material = ShaderMaterial.new()
	_wall_material.shader = WALL_SHADER
	counters["materials_created"] += 1
	# One cell set per patch of the world's grid, the most patches that can draw cells at once
	# (every resident patch may hold a graded tile), so neither streaming nor an edit creates one.
	var tiles_per_side := int(ceil(span_m / TILE_M)) + 1
	_cells_per_patch = tiles_per_side * tiles_per_side
	var grid := Vector2i(int(ceil(world_size.x / span_m)), int(ceil(world_size.y / span_m)))
	_patch_grid = grid
	_cells_pool.clear()
	var prewarm_us := Time.get_ticks_usec()
	for _patch in grid.x * grid.y:
		_cells_pool.append(_new_cells())
	counters["cell_sets_prewarmed"] = grid.x * grid.y
	counters["cell_prewarm_ms"] = float(Time.get_ticks_usec() - prewarm_us) / 1000.0

# The tiles a block of `patches` x `patches` patches meets at most: T(B).
func _block_tiles(patches: int) -> int:
	var side := int(ceil(float(patches) * patch_span_m / TILE_M)) + 1
	return side * side

# Drops every page and creates the world pages of GL0 and GL1, sized by their bands' bound and
# the most rows a tile can hold (at most the layer limit), so they never fill.
func _reset_pages() -> void:
	page_store.reset()
	for code in 4:
		_block_page[code].clear()
		_block_target[code].clear()
		_block_waiting[code].clear()
	_growth_waits.clear()
	_growth_dirty.clear()
	for code in mini(2, page_store.levels.size()):
		var size := _page_size(code, Vector2i.ZERO, Vector2i.ZERO)
		_block_page[code][Vector2i.ZERO] = page_store.create(code, Vector2i.ZERO, size.x, size.y)

# --- Pages (docs/terrain.md "Pages and the row atlas"). ---

# The block of patch `key` at level `code`: the world at GL0 and GL1, else B x B patches.
func _block_of(key: Vector2i, code: int) -> Vector2i:
	return Vector2i.ZERO if code < 2 else key / block_patches

# The page patch `key`'s sets bind at level `code` (-1: none yet).
func _patch_page(key: Vector2i, code: int) -> int:
	return int(_block_page[code].get(_block_of(key, code), -1))

# The patch keys of block `block` at level `code` whose sets exist.
func _block_sets(code: int, block: Vector2i) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	if code < 2:
		keys.assign(patch_cells.keys())
		return keys
	for z in block_patches:
		for x in block_patches:
			var key := block * block_patches + Vector2i(x, z)
			if patch_cells.has(key):
				keys.append(key)
	return keys

# The tiles block `block`'s resident patches' lists name: what the block can need at a level
# while the lists stand, but for the border tiles only a neighbour's list names (the page's
# headroom). Read from the lists themselves, not from the desired set's counts, which a list's
# arrival queues over frames (round 8: sized from the counts, the 100k load moved 44 blocks).
# O(the block's listed tiles).
func _block_listed(block: Vector2i) -> int:
	var seen := {}
	for z in block_patches:
		for x in block_patches:
			var key := block * block_patches + Vector2i(x, z)
			if not _resident.has(key):
				continue
			for tile: Vector2i in patch_lists.get(key, {}).get("tiles", []):
				seen[tile] = true
	return seen.size()

# The size of block `block`'s page at level `code`: its listed tiles plus an eighth and 8,
# rounded up to 32 slots, at most the level's cap, and three atlas layers a slot, at most the
# layer limit (docs/terrain.md "Page size"); at least `least` (slots, layers). The world pages of
# GL0 and GL1 keep their band's slots and the most rows a tile can hold, and grow only their
# layers. O(the block's tiles).
func _page_size(code: int, block: Vector2i, least: Vector2i) -> Vector2i:
	var cap := int(page_store.levels[code]["cap"])
	if code < 2:
		return Vector2i(cap, mini(layer_limit, maxi(least.y, cap * int(page_store.levels[code]["max_layers"]))))
	var listed := _block_listed(block)
	var slots := mini(cap, maxi(least.x, ((listed + listed / 8 + 8 + 31) / 32) * 32))
	return Vector2i(slots, mini(layer_limit, maxi(least.y, slots * PAGE_LAYERS_PER_SLOT)))

# Block `block`'s first page at level `code`, which its sets bind. Returns its id (-1: refused by
# the budget).
func _first_page(code: int, block: Vector2i) -> int:
	var size := _page_size(code, block, Vector2i.ZERO)
	var page_id := page_store.create(code, block, size.x, size.y)
	if page_id >= 0:
		_block_page[code][block] = page_id
		_bind_block(code, block, false)
	return page_id

# The pages a copy of `tile` at level `code` goes into, as [page id, block]: per block of the
# resident patches drawing it, its page (made as the block first needs one, once every resident
# patch of the block has its list, so the page is sized for them: PAGE_PENDING until then; -1
# where the budget refuses it), and the larger page a growing block moves to (its own page keeps
# taking every copy until the move, so its sets show every installed tile meanwhile). O(patches
# drawing it, at most 4).
func _copy_pages(tile: Vector2i, code: int) -> Array[Array]:
	var pages: Array[Array] = []
	var origin := Vector2(tile) * TILE_M - world_min
	var first := Vector2i((origin / patch_span_m).floor())
	var last := Vector2i(((origin + Vector2.ONE * TILE_M) / patch_span_m).ceil()) - Vector2i.ONE
	for z in range(first.y, last.y + 1):
		for x in range(first.x, last.x + 1):
			var key := Vector2i(x, z)
			if not _resident.has(key):
				continue
			var block := _block_of(key, code)
			var target := int(_block_target[code].get(block, -1))
			var bound := int(_block_page[code].get(block, -1))
			if bound < 0 and target < 0:
				if _block_lists_pending(code, block):
					pages.append([PAGE_PENDING, block])
					continue
				bound = _first_page(code, block)
				if bound < 0:
					pages.append([-1, block])
					continue
			if target >= 0 and not _has_page(pages, target):
				pages.append([target, block])
			if bound >= 0 and not _has_page(pages, bound):
				pages.append([bound, block])
	return pages

# Whether block `block`'s page (GL2 and GL3) must wait: a resident patch of the block still
# waits for its list, or, while a world load settles (`unbudgeted`) and its patch residency is
# still growing (`residency_settling`), a patch of the block within the world is not resident
# yet (the load makes the patches near the camera resident over its frames; a page sized before
# them moves its block once they list their tiles: the 100k load moved 44 blocks, about 20 s of
# rebakes). Once the residency stops growing, a block straddling its edge takes its resident
# patches' tiles.
func _block_lists_pending(code: int, block: Vector2i) -> bool:
	if code < 2:
		return false
	for z in block_patches:
		for x in block_patches:
			var key := block * block_patches + Vector2i(x, z)
			if not _resident.has(key):
				if unbudgeted and residency_settling and key.x < _patch_grid.x and key.y < _patch_grid.y:
					return true
				continue
			if not patch_lists.has(key) and not patch_lists_refused.has(key):
				return true
	return false

# Whether the placement that found page `page_id` of `block` unable to take a copy waits for
# the block's move (the page is the one the block's sets bind while it moves to a larger one) or
# else starts or restarts a move (_grow). Returns whether the tile waits.
func _wait_or_grow(code: int, block: Vector2i, page_id: int, layers: int) -> bool:
	var target := int(_block_target[code].get(block, -1))
	if target >= 0 and page_id != target:
		return true
	return _grow(code, block, layers)

static func _has_page(pages: Array[Array], page_id: int) -> bool:
	for entry: Array in pages:
		if int(entry[0]) == page_id:
			return true
	return false

# Whether installed `entry` of `tile` has a copy in every page _copy_pages places it in.
func _tile_complete(tile: Vector2i, entry: Dictionary) -> bool:
	var copies: Dictionary = entry["copies"]
	for pair: Array in _copy_pages(tile, int(entry["level"])):
		if not copies.has(int(pair[0])):
			return false
	return true

# Places `tile`'s copies at level `code` in every page _copy_pages names, all or none, and writes
# them (a held copy of this level and token, `fresh`, is kept unwritten); the held copies no page
# wants any more are freed. Returns the copies by page id. Where a page cannot take it,
# _place_failed is set and nothing changes: that page's block grows (_place_waits, the tile
# waiting for the move unless `is_preview`), or, at its cap or the budget, the caller records
# the tile exhausted.
var _place_failed := false
var _place_waits := false
func _place_copies(key: Vector2i, code: int, tile: Dictionary, held: Dictionary, fresh: bool, is_preview: bool) -> Dictionary:
	_place_failed = false
	_place_waits = false
	var layers := (tile["rows"] as PackedByteArray).size() / GradedPages.ATLAS_LAYER_BYTES
	var wanted := _copy_pages(key, code)
	for pair: Array in wanted:
		var page_id := int(pair[0])
		var page: Dictionary = page_store.by_id.get(page_id, {})
		if not page.is_empty() and GradedPages.can_place(page, held.get(page_id, GradedPages.NO_COPY), layers):
			continue
		_place_failed = true
		if page_id == PAGE_PENDING:
			# Asked again once the block's lists are in.
			_place_waits = true
			_wanted[key] = true
			return {}
		_place_waits = not page.is_empty() and _wait_or_grow(code, pair[1], page_id, layers)
		if _place_waits and not is_preview:
			_wait_for_growth(key, code, pair[1])
		return {}
	var copies := {}
	for pair: Array in wanted:
		var page_id := int(pair[0])
		var page: Dictionary = page_store.by_id[page_id]
		var before: Vector3i = held.get(page_id, GradedPages.NO_COPY)
		var copy := GradedPages.place(page, before, layers)
		if not (fresh and copy == before):
			page_store.write_copy(page, copy, tile)
		copies[page_id] = copy
		var holders: Dictionary = page["previews"] if is_preview else page["tiles"]
		holders[key] = true
		if int(_block_target[code].get(page["block"], -1)) == page_id:
			_growth_dirty[Vector3i(code, pair[1].x, pair[1].y)] = true
	for page_id: int in held:
		if not copies.has(page_id):
			_free_copy(key, page_id, held[page_id], is_preview)
	return copies

func _wait_for_growth(key: Vector2i, code: int, block: Vector2i) -> void:
	if not _block_waiting[code].has(block):
		_block_waiting[code][block] = {}
	_block_waiting[code][block][key] = true
	_growth_waits[key] = Vector3i(code, block.x, block.y)
	_wanted.erase(key)

# Frees `tile`'s copy in page `page_id`: its slot and run, and with its last copy the page (a
# block's own page, or a growing block's target; the world pages stay). A freed slot asks one
# tile that found the level exhausted again (it stays recorded until it installs or finds no
# slot again). O(the page's free runs).
func _free_copy(tile: Vector2i, page_id: int, copy: Vector3i, is_preview: bool) -> void:
	var page: Dictionary = page_store.by_id.get(page_id, {})
	if page.is_empty():
		return
	var code := int(page["code"])
	GradedPages.vacate(page, copy)
	(page["previews"] if is_preview else page["tiles"]).erase(tile)
	var block: Vector2i = page["block"]
	if int(_block_target[code].get(block, -1)) >= 0:
		_growth_dirty[Vector3i(code, block.x, block.y)] = true
	if code >= 2 and (page["tiles"] as Dictionary).is_empty() and (page["previews"] as Dictionary).is_empty():
		_page_emptied(code, block, page_id)
	var order: Array = _exhausted_order[code]
	while not order.is_empty():
		var waiting: Vector2i = order.pop_back()
		var record: Array = _exhausted.get(waiting, [])
		if record.is_empty() or int(record[0]) != code or bool(record[3]):
			continue
		record[3] = true
		_wanted[waiting] = true
		return

# A block page left without copies: a block's own page goes (its sets unbind it) unless the
# block is moving, whose move then completes; an empty target goes with its move, unless the
# block's own page still holds copies.
func _page_emptied(code: int, block: Vector2i, page_id: int) -> void:
	var target := int(_block_target[code].get(block, -1))
	if page_id == target:
		var bound: Dictionary = page_store.by_id.get(int(_block_page[code].get(block, -1)), {})
		if not bound.is_empty() and not (bound["tiles"] as Dictionary).is_empty():
			return
		_block_target[code].erase(block)
		page_store.release(page_id)
		_end_waits(code, block)
		return
	if target >= 0:
		return
	_block_page[code].erase(block)
	page_store.release(page_id)
	_bind_block(code, block, false)

# Frees every copy of `entry` (installed or preview tile `tile`).
func _free_copies(tile: Vector2i, entry: Dictionary, is_preview: bool) -> void:
	var copies: Dictionary = entry["copies"]
	for page_id: int in copies.keys():
		_free_copy(tile, page_id, copies[page_id], is_preview)
	copies.clear()

# Block `block`'s page at level `code` cannot place a copy of `layers` atlas layers: it moves to a
# larger page (docs/terrain.md "Growth"), sized for its listed tiles now and at least twice the
# dimension that ran out, which its tiles fill by rebakes while its sets keep binding their page.
# Returns false where no larger page can be made (the level's cap and the layer limit, or the
# budget). O(the block's tiles).
func _grow(code: int, block: Vector2i, layers: int) -> bool:
	var target := int(_block_target[code].get(block, -1))
	var from_id := target if target >= 0 else int(_block_page[code].get(block, -1))
	var from: Dictionary = page_store.by_id.get(from_id, {})
	if from.is_empty():
		return false
	var slots := int(from["slots"])
	var atlas := int(from["layers"])
	var least := Vector2i(
		slots * 2 if (from["free"] as Array).is_empty() else slots,
		atlas * 2 if GradedPages.find_run(from["runs"], maxi(layers, 1)) < 0 else atlas)
	var size := _page_size(code, block, least)
	if size.x <= slots and size.y <= atlas:
		return false
	return _move_block(code, block, Vector2i(maxi(size.x, slots), maxi(size.y, atlas)))

# Starts block `block`'s move at level `code` to a new page of `size` (slots, atlas layers): it
# becomes the block's target (a target that ran out too is dropped), and the tiles the block's
# page holds are asked again for their copies there. Returns false where the budget refuses the
# page. O(the block's tiles).
func _move_block(code: int, block: Vector2i, size: Vector2i) -> bool:
	var target := int(_block_target[code].get(block, -1))
	var page_id := page_store.create(code, block, size.x, size.y)
	if page_id < 0:
		return false
	if target >= 0:
		# A move to a target that ran out too starts again at the larger page.
		var old: Dictionary = page_store.by_id[target]
		for tile: Vector2i in (old["tiles"] as Dictionary).keys():
			(installed[tile]["copies"] as Dictionary).erase(target)
		for tile: Vector2i in (old["previews"] as Dictionary).keys():
			(preview[tile]["copies"] as Dictionary).erase(target)
		(old["tiles"] as Dictionary).clear()
		(old["previews"] as Dictionary).clear()
		page_store.release(target)
	_block_target[code][block] = page_id
	counters["page_growths"] += 1
	MetrumDebug.log("terrain", "graded level %d block %s grows to %d slots, %d layers" % [code, str(block), size.x, size.y])
	# Its tiles are asked again for the target's copies (nothing is kept on the CPU).
	var bound: Dictionary = page_store.by_id.get(int(_block_page[code].get(block, -1)), {})
	if not bound.is_empty():
		for tile: Vector2i in (bound["tiles"] as Dictionary).keys():
			_wanted[tile] = true
	_growth_dirty[Vector3i(code, block.x, block.y)] = true
	return true

# The growing blocks whose target took a copy or whose page lost one: a target holding a copy of
# every installed tile the block's page holds becomes the page the block's sets bind
# (_switch_block). O(those blocks' tiles).
func _advance_growth() -> void:
	if _growth_dirty.is_empty():
		return
	for marked: Vector3i in _growth_dirty.keys():
		var code := marked.x
		var block := Vector2i(marked.y, marked.z)
		var target := int(_block_target[code].get(block, -1))
		if target < 0:
			continue
		var bound: Dictionary = page_store.by_id.get(int(_block_page[code].get(block, -1)), {})
		var complete := true
		if not bound.is_empty():
			for tile: Vector2i in bound["tiles"]:
				# A tile refused or exhausted at its level since (an error, drawn as its base cell)
				# is not waited for: it loses this block's copy with the old page.
				if not (installed[tile]["copies"] as Dictionary).has(target) and not _refused_at_level(tile):
					complete = false
					break
		if complete:
			_switch_block(code, block)
	_growth_dirty.clear()

# Block `block` moves to its target at level `code`: its sets bind the target in this frame (each
# of its patches' tiles rewritten with the target's slots), the old page's copies go with it, and
# the tiles that waited for the move are asked again.
func _switch_block(code: int, block: Vector2i) -> void:
	var target := int(_block_target[code][block])
	var old_id := int(_block_page[code].get(block, -1))
	_block_target[code].erase(block)
	var moved: Dictionary = page_store.by_id[target]
	if code >= 2 and (moved["tiles"] as Dictionary).is_empty() and (moved["previews"] as Dictionary).is_empty():
		# Every tile left the block meanwhile: neither page holds a copy it draws.
		page_store.release(target)
		target = -1
		_block_page[code].erase(block)
	else:
		_block_page[code][block] = target
	if old_id >= 0:
		var old: Dictionary = page_store.by_id[old_id]
		for tile: Vector2i in (old["tiles"] as Dictionary).keys():
			(installed[tile]["copies"] as Dictionary).erase(old_id)
		for tile: Vector2i in (old["previews"] as Dictionary).keys():
			(preview[tile]["copies"] as Dictionary).erase(old_id)
		(old["tiles"] as Dictionary).clear()
		(old["previews"] as Dictionary).clear()
		page_store.release(old_id)
	_bind_block(code, block, true)
	_end_waits(code, block)
	counters["page_moves"] += 1

func _end_waits(code: int, block: Vector2i) -> void:
	for tile: Vector2i in _block_waiting[code].get(block, {}):
		_growth_waits.erase(tile)
		_wanted[tile] = true
	_block_waiting[code].erase(block)

# Block `block`'s sets bind its page at level `code` (_bind_page); with `rewrite` (a move: the
# tiles' slots are the new page's) each of their tiles' instances is rewritten. A block's first
# page and an emptied page change no instance: no tile shows from either.
func _bind_block(code: int, block: Vector2i, rewrite: bool) -> void:
	for key: Vector2i in _block_sets(code, block):
		var cells: Dictionary = patch_cells[key]
		if _bind_page(cells, key) and rewrite:
			for tile: Vector2i in cells["tiles"]:
				_dirty_cell_tiles[tile] = true
			_dirty_shown[key] = true

# A set's graded materials of each level take the page its patch's block uses there (none: no
# texture, and no instance draws at that level); the grass mask mirrors follow the set's mask
# revision. Returns whether a binding changed. O(levels).
func _bind_page(cells: Dictionary, key: Vector2i) -> bool:
	var bound: Array = cells["pages"]
	var changed := false
	for code in page_store.levels.size():
		var page_id := _patch_page(key, code)
		if int(bound[code]) == page_id:
			continue
		bound[code] = page_id
		changed = true
		var page: Dictionary = page_store.by_id.get(page_id, {})
		for index in [code, code + CELL_BEHIND]:
			if index >= (cells["materials"] as Array).size():
				continue
			var material: ShaderMaterial = cells["materials"][index]
			material.set_shader_parameter("graded_texels", page.get("texels", null))
			material.set_shader_parameter("graded_atlas", page.get("atlas", null))
	if changed:
		_cell_mask_revision += 1
		counters["mask_revision_bumps"] += 1
		cells["mask_revision"] = _cell_mask_revision
	return changed

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

# The cell sets' nodes are not reference counted: they go with the renderer (terrain.gd makes a
# new one per world load and disposes of the last). Before round 6 each load leaked the previous
# renderer's 1,296 Kuopio sets; until `TERRAIN-07-G` round 10 dispose() freed each set's nodes but
# not their root (round 9C: one orphan per set per load). A renderer dropped without dispose()
# frees its sets here, each through its root, once; after dispose() there are none left. A held
# set's root under a patch node the scene already freed went with it. O(sets).
func _notification(what: int) -> void:
	if what != NOTIFICATION_PREDELETE:
		return
	for cells: Dictionary in _cells_pool + patch_cells.values():
		var root: Variant = cells["root"]
		if not is_instance_valid(root):
			continue
		if (root as Node).is_inside_tree():
			(root as Node).queue_free()
		else:
			(root as Node).free()

## Frees every slot and cell resource, as the renderer resets.
func clear() -> void:
	for key: Vector2i in installed.keys():
		_free_copies(key, installed[key], false)
	installed.clear()
	for key: Vector2i in preview.keys():
		_free_copies(key, preview[key], true)
	preview.clear()
	# Every block page is empty now: the moves end with them.
	for code in 4:
		for block: Vector2i in _block_target[code].keys():
			page_store.release(int(_block_target[code][block]))
		_block_target[code].clear()
		_block_waiting[code].clear()
	_growth_waits.clear()
	_growth_dirty.clear()
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
	_exhausted.clear()
	for order: Array in _exhausted_order:
		order.clear()
	_dirty_walls.clear()
	_dirty_cell_tiles.clear()
	_patch_steps.clear()
	for key in patch_cells.keys():
		release_patch(key, null)
	for key: Vector2i in _collapsed.keys():
		_set_plane(key, null, true)
	_unready.clear()
	_builds.clear()
	_builds_urgent.clear()
	_variants_wanted.clear()
	_variant_waits.clear()
	_preview_waits.clear()
	_dirty_shown.clear()
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

## Frees the cell sets, which are Nodes detached while pooled (no scene owner frees them).
## Renderer replacement and shutdown call it before dropping the renderer; clear() first returns
## every set to the pool.
func dispose() -> void:
	clear()
	for cells: Dictionary in _cells_pool:
		(cells["root"] as Node).free()
	_cells_pool.clear()

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
	_frame_flips = 0
	_frame_flip_us = 0
	_frame_custom_us = 0
	_frame_write_us = 0
	_frame_written = 0
	_frame_moved = 0
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
	_advance_builds(start_us, resident_patches)
	if not (_variant_waits.is_empty() and _preview_waits.is_empty()):
		_take_variant_waits(resident_patches)
	if not _lists_wanted.is_empty():
		_request_patch_lists(resident_patches)
	if not _wanted.is_empty():
		_request_tiles()
	var residency_us := Time.get_ticks_usec()
	# A patch's cells follow its material's parameters (a changed patch's LOD step in
	# _patch_changed). O(changed materials).
	for key: Vector2i in changed_materials.keys():
		if not patch_cells.has(key) or not resident_patches.has(key) or not bool(patch_cells[key]["shown"]):
			continue
		var material: ShaderMaterial = resident_patches[key]["material"]
		var cells: Dictionary = patch_cells[key]
		if _watched_parameters(material) != cells["watched"]:
			_sync_material(cells, material)
	var watched_us := Time.get_ticks_usec()
	_advance_growth()
	var cell_patches := _dirty_patches.size()
	_apply_cell_changes(resident_patches)
	var cells_us := Time.get_ticks_usec()
	var wall_patches := _dirty_walls.size()
	_apply_wall_changes(resident_patches)
	_count_unready_in_view(resident_patches)
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
			"flips": _frame_flips, "flip_ms": float(_frame_flip_us) / 1000.0,
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
				for index in (cells["nodes"] as Array).size():
					var node: MultiMeshInstance3D = cells["nodes"][index]
					if node.is_visible_in_tree():
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

# The residency margin's check (T53 (a)): the unready patches whose culling box (the one Godot
# culls the patch by) is not wholly outside a plane of the view's frustum. O(unready patches):
# none once the residency has caught up.
func _count_unready_in_view(resident_patches: Dictionary) -> void:
	if view_planes.is_empty() or _unready.is_empty():
		return
	var in_view := 0
	for key: Vector2i in _unready.keys():
		var patch: Dictionary = resident_patches.get(key, {})
		var node: Node3D = patch.get("node", null)
		if node == null or not node.is_inside_tree():
			continue
		var box: AABB = node.global_transform * (patch["aabb"] as AABB)
		var outside := false
		for plane: Plane in view_planes:
			# The box corner farthest along the plane's inward side: outside if even it is over.
			var corner := box.position + Vector3(
				box.size.x if plane.normal.x < 0.0 else 0.0,
				box.size.y if plane.normal.y < 0.0 else 0.0,
				box.size.z if plane.normal.z < 0.0 else 0.0)
			if plane.is_point_over(corner):
				outside = true
				break
		if not outside:
			in_view += 1
	counters["unready_in_view"] += in_view
	counters["unready_in_view_max"] = max(int(counters["unready_in_view_max"]), in_view)

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
	for key: Vector2i in _dirty_shown.keys():
		_apply_shown(key, resident_patches)
	_dirty_shown.clear()
	# The cells across the sides of the sets that started or stopped drawing stitch to what those
	# patches draw now, in this frame; rewriting them changes no patch's draw state (_draw_state
	# reads no side code).
	if not _dirty_cell_tiles.is_empty():
		_update_cell_tiles(resident_patches)
		_dirty_shown.clear()

func _apply_wall_changes(resident_patches: Dictionary) -> void:
	for key: Vector2i in _dirty_walls.keys():
		if resident_patches.has(key) and patch_cells.has(key) and _built(patch_cells[key]):
			_rebuild_patch_walls(patch_cells[key], resident_patches[key])
	_dirty_walls.clear()

## The serial of the last graded edit whose tiles draw (docs/terrain.md T61 (2)): every edit
## taken once no batch waits, else those up to the last installed batch. The road tool holds an
## install's replaced road chunks until this reaches the journal's serial it read then. A batch
## waiting on its variant or results keeps it back; an edit no held tile takes leaves nothing to
## wait for. O(1).
func installed_edit_serial() -> int:
	return _edit_serial_taken if _edit_batch.is_empty() and _edit_release < 0 else _edit_serial_batched

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
	var tiles_us := Time.get_ticks_usec()
	var cell_patches := _dirty_patches.size()
	var cell_tiles := _frame_cell_tiles
	var flips := _frame_flips
	var flip_us := _frame_flip_us
	var marks := [_frame_custom_us, _frame_write_us, _frame_written, _frame_moved]
	_apply_cell_changes(_resident)
	var cells_us := Time.get_ticks_usec()
	var wall_patches := _dirty_walls.size()
	_apply_wall_changes(_resident)
	var end_us := Time.get_ticks_usec()
	var ms := float(end_us - start_us) / 1000.0
	counters["preview_install_ms_max"] = max(float(counters["preview_install_ms_max"]), ms)
	counters["preview_frames"] += 1
	counters["preview_install_ms_last"] = ms
	counters["preview_tiles_last"] = tiles.size()
	# The frame's slot writes and drops, then the cell and wall rewrites they caused.
	counters["preview_parts"] = {
		"tiles_ms": float(tiles_us - start_us) / 1000.0, "cells_ms": float(cells_us - tiles_us) / 1000.0,
		"walls_ms": float(end_us - cells_us) / 1000.0, "cell_patches": cell_patches, "wall_patches": wall_patches,
		"cell_tiles": _frame_cell_tiles - cell_tiles, "flips": _frame_flips - flips,
		"flip_ms": float(_frame_flip_us - flip_us) / 1000.0,
		"custom_ms": float(_frame_custom_us - int(marks[0])) / 1000.0, "write_ms": float(_frame_write_us - int(marks[1])) / 1000.0,
		"written": _frame_written - int(marks[2]), "moved": _frame_moved - int(marks[3]),
	}
	return ms

## Drops the preview tiles of preview epochs `first_epoch` to `last_epoch`: the installed tiles
## draw again. A cancelled preview drops its own epoch; an edit batch drops every epoch up to the
## one its commit adopted.
func release_preview(first_epoch: int, last_epoch: int) -> void:
	for key: Vector2i in preview.keys():
		var token := int(preview[key]["token"])
		if token >= first_epoch and token <= last_epoch:
			_drop_preview(key)
	_drop_preview_waits(first_epoch, last_epoch)
	_apply_cell_changes(_resident)
	_apply_wall_changes(_resident)

func _show_preview_tile(tile: Dictionary) -> void:
	var key := Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))
	# A flagged tile shows once its sets hold the variant (the preview's lag, T27 (d)); the tile
	# shows what it showed meanwhile.
	_preview_waits.erase(key)
	if bool(tile["behind"]) and _variant_missing(key):
		counters["variant_waits"] += 1
		_preview_waits[key] = tile
		return
	var code := int(tile["level"])
	var current: Dictionary = preview.get(key, {})
	var same_level := not current.is_empty() and int(current["level"]) == code
	var before := _shown(key)
	var copies := _place_copies(key, code, tile, current["copies"] if same_level else {}, false, true)
	if _place_failed:
		# The preview's lag (T27 (d)): a page that is full draws the tile once a later pose finds
		# room (its block grows meanwhile). A page that cannot grow (its cap or the budget) is the
		# level's pool exhausted, as the install path records it (_exhaust).
		if _place_waits:
			counters["preview_page_waits"] += 1
			MetrumDebug.log("terrain", "graded level %d page full at preview tile %s" % [code, str(key)])
		else:
			counters["pool_exhausted"] += 1
			MetrumDebug.log("terrain", "graded pool %d exhausted at preview tile %s" % [code, str(key)])
		return
	if not current.is_empty() and not same_level:
		_free_copies(key, current, true)
	var step := int(tile["step"])
	var walls: PackedFloat32Array = (tile["walls"] as PackedByteArray).to_float32_array()
	preview[key] = {"code": code if code < 3 else base_code(step), "level": code, "step": step, "token": int(tile["token"]), "walls": walls, "behind": bool(tile["behind"]), "copies": copies}
	_shown_changed(key, before)

# Drops the preview tiles of epochs `first_epoch` to `last_epoch` waiting for a set's variant.
func _drop_preview_waits(first_epoch: int, last_epoch: int) -> void:
	for key: Vector2i in _preview_waits.keys():
		var token := int(_preview_waits[key]["token"])
		if token >= first_epoch and token <= last_epoch:
			_preview_waits.erase(key)

func _drop_preview(key: Vector2i) -> void:
	_preview_waits.erase(key)
	var entry: Dictionary = preview.get(key, {})
	if entry.is_empty():
		return
	preview.erase(key)
	_free_copies(key, entry, true)
	_shown_changed(key, entry)

# What `key` draws changed from `before`: its instance and its neighbours' sides, the walls when
# its faces changed, and the sets of the patches meeting it (_await_cells).
func _shown_changed(key: Vector2i, before: Dictionary) -> void:
	var after := _shown(key)
	_cell_changed(key, before.is_empty() or after.is_empty() or int(before["code"]) != int(after["code"]))
	if after.get("walls", PackedFloat32Array()) != before.get("walls", PackedFloat32Array()):
		_walls_changed(key)
	_await_cells(key, _resident)

# Tile `key` shows something new: each resident patch meeting it starts or stops drawing its set,
# or, without a built set yet, builds one first (urgent); the tile draws there once it is built.
# O(1).
func _await_cells(key: Vector2i, resident_patches: Dictionary) -> void:
	for patch_key in _patches_meeting(Rect2(Vector2(key) * TILE_M, Vector2.ONE * TILE_M)):
		if not resident_patches.has(patch_key):
			continue
		if patch_cells.has(patch_key) and _built(patch_cells[patch_key]):
			_dirty_shown[patch_key] = true
		else:
			_builds_urgent[patch_key] = true

# What a tile draws: its preview tile while the road preview shows one, else its installed tile.
func _shown(tile: Vector2i) -> Dictionary:
	var entry: Dictionary = preview.get(tile, {})
	return entry if not entry.is_empty() else installed.get(tile, {})

# What `tile` draws in patch `key`'s cells: its preview tile where the page of the patch's block
# at the preview's level holds its copy, else its installed tile where that page holds one, else
# nothing (a base cell). O(1).
func _shown_in(tile: Vector2i, key: Vector2i) -> Dictionary:
	var entry: Dictionary = preview.get(tile, {})
	if not entry.is_empty() and (entry["copies"] as Dictionary).has(_patch_page(key, int(entry["level"]))):
		return entry
	entry = installed.get(tile, {})
	if not entry.is_empty() and (entry["copies"] as Dictionary).has(_patch_page(key, int(entry["level"]))):
		return entry
	return {}

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
	# The cells show the installed tiles of the patch's rectangle, not its list: a list that
	# empties or fills only starts or stops the set's drawing (_apply_shown); listed tiles install
	# or evict on their own.
	if not patch_cells.has(key):
		_builds[key] = true
	# What the patch draws follows its list (_draw_state): its PlaneMesh, or its set once the set
	# shows the listed tiles.
	_dirty_shown[key] = true
	patch_lists[key] = {"token": int(list["token"]), "tiles": tiles}
	_recount(key, resident_patches)
	# A list baked for an older generation is asked again.
	_lists_wanted[key] = true

func _over_budget(start_us: int) -> bool:
	return not unbudgeted and float(Time.get_ticks_usec() - start_us) / 1000.0 >= INSTALL_BUDGET_MS

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
			_dirty_shown[key] = true
		_lists_wanted[key] = true
	else:
		var tile := Vector2i(keys[0], keys[1])
		var request_key := "%d,%d,%d,%d" % [keys[0], keys[1], keys[2], keys[3]]
		if int(requested.get(request_key, -1)) == token:
			requested.erase(request_key)
		if not retry:
			refused[request_key] = token
			# The tile stays as it was; the edit installs without it, and its patches' sets draw
			# without it (_draw_state).
			_edit_batch.erase(tile)
			_edit_staged.erase(tile)
			for patch_key in _patches_meeting(Rect2(Vector2(tile) * TILE_M, Vector2.ONE * TILE_M)):
				_dirty_shown[patch_key] = true
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
	var flagged := bool(tile["behind"])
	if int(_edit_batch.get(key, -1)) == token:
		# Part of an edit: it waits for the batch's other tiles (and, flagged, its sets' variant).
		requested.erase(request_key)
		if flagged:
			_variant_missing(key)
		_edit_staged[key] = tile
		return
	# A later result of the tile supersedes one that waited (an older level's or token's).
	_drop_variant_wait(key)
	if flagged and _variant_missing(key):
		# Still requested while it waits (_take_variant_waits).
		counters["variant_waits"] += 1
		_variant_waits[key] = tile
		return
	requested.erase(request_key)
	_upload(key, tile, resident_patches)

# Drops the bake result of `key` waiting for its sets' variant, and its request.
func _drop_variant_wait(key: Vector2i) -> void:
	var tile: Dictionary = _variant_waits.get(key, {})
	if tile.is_empty():
		return
	_variant_waits.erase(key)
	var request_key := "%d,%d,%d,%d" % [key.x, key.y, int(tile["level"]), int(tile["step"])]
	if int(requested.get(request_key, -1)) == int(tile["token"]):
		requested.erase(request_key)

func _upload(key: Vector2i, tile: Dictionary, resident_patches: Dictionary) -> void:
	var code := int(tile["level"])
	var step := int(tile["step"])
	var token := int(tile["token"])
	var current: Dictionary = installed.get(key, {})
	var same_level := not current.is_empty() and int(current["level"]) == code
	# Into every page its patches' blocks use; at the same level each copy keeps its slot and is
	# rewritten in place, before this frame draws.
	var held: Dictionary = current["copies"] if same_level else {}
	# A rebake of the installed level, step and token (asked again for a page that lacks its copy)
	# leaves the copies it already has unwritten; a GL3 step change rewrites them.
	var fresh := same_level and int(current["step"]) == step and int(current["token"]) == token
	var copies := _place_copies(key, code, tile, held, fresh, false)
	if _place_failed:
		# The tile stays as it was: waiting for its block's larger page (_wait_for_growth), or
		# recorded (_exhausted) until a slot of the level frees.
		if not _place_waits:
			_exhaust(key, code, "%d,%d,%d,%d" % [key.x, key.y, code, step], token)
		return
	if same_level:
		counters["same_level_uploads"] += 1
	_exhausted.erase(key)
	var cell_code := code if code < 3 else base_code(step)
	var pages_changed := current.is_empty() or (current["copies"] as Dictionary).size() != copies.size()
	if not pages_changed:
		for page_id: int in copies:
			pages_changed = pages_changed or not (current["copies"] as Dictionary).has(page_id)
	# A tile moving between levels frees its copies in the old level's pages.
	if not current.is_empty() and not same_level:
		_free_copies(key, current, false)
	var walls: PackedFloat32Array = (tile["walls"] as PackedByteArray).to_float32_array()
	var behind := bool(tile["behind"])
	installed[key] = {"code": cell_code, "level": code, "step": step, "token": token, "walls": walls, "behind": behind, "copies": copies}
	# A same-level upload keeps its cell instance and slot, but the grass mask reads the slot's
	# texels. Invalidate the private mask even when no cell geometry changed.
	if not current.is_empty() and int(current["level"]) == code and not preview.has(key):
		_mask_changed(key, resident_patches)
	# The installed level moves the hysteresis: the tile is levelled again at once. A bake of an
	# older level or token (the camera moved, an edit dirtied it) still wants one.
	var want: Dictionary = _desired.get(key, {})
	if not want.is_empty():
		_level_tile(key, want)
		if want["level"] != Vector2i(code, step) or int(want["token"]) != token:
			_wanted[key] = true
	# Rewritten in place, the tile keeps its instances; a new slot or lattice rewrites its own
	# instances and its neighbours' sides, and changed faces the walls of the patches that draw them.
	if (current.is_empty() or int(current["level"]) != code or int(current["code"]) != cell_code
			or bool(current["behind"]) != behind or pages_changed):
		# Under a preview tile nothing drawn changes until the preview drops it (_drop_preview).
		if not preview.has(key):
			_cell_changed(key, current.is_empty() or int(current["code"]) != cell_code)
		_await_cells(key, resident_patches)
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
	# A flagged tile's sets take the variant first (queued as it staged): the batch installs whole
	# once none lacks it.
	var waiting := false
	for key: Vector2i in _edit_staged:
		if bool(_edit_staged[key]["behind"]) and _variant_missing(key):
			waiting = true
	if waiting:
		counters["variant_waits"] += 1
		return 0
	for key: Vector2i in _edit_staged.keys():
		if not _adopt_preview(key, _edit_staged[key]):
			_upload(key, _edit_staged[key], resident_patches)
	last_edit_batch = _edit_staged.keys()
	var count := _edit_staged.size()
	_edit_batch.clear()
	_edit_staged.clear()
	_edit_serial_batched = _edit_serial_taken
	# The committed tiles replace the preview's in the same frame.
	if _edit_release >= 0:
		for key: Vector2i in preview.keys():
			if int(preview[key]["token"]) <= _edit_release:
				_drop_preview(key)
		_drop_preview_waits(0, _edit_release)
		_edit_release = -1
	return count

# A committed tile that is the very payload the preview shows (Rust's `adopted`, the preview tile
# of the commit's epoch at the same level) takes over the preview's slot as the installed tile
# (`TERRAIN-07-E` round 6): nothing is uploaded and nothing drawn changes, so neither its instance
# nor its walls are rewritten. Returns whether it did. O(1).
func _adopt_preview(key: Vector2i, tile: Dictionary) -> bool:
	var shown: Dictionary = preview.get(key, {})
	if (not bool(tile.get("adopted", false)) or shown.is_empty()
			or int(shown["token"]) != int(tile["preview_epoch"])
			or int(shown["level"]) != int(tile["level"]) or int(shown["step"]) != int(tile["step"])):
		return false
	var current: Dictionary = installed.get(key, {})
	if not current.is_empty():
		_free_copies(key, current, false)
	var token := int(tile["token"])
	installed[key] = {"code": shown["code"], "level": shown["level"], "step": shown["step"], "token": token, "walls": shown["walls"], "behind": shown["behind"], "copies": shown["copies"]}
	for page_id: int in shown["copies"]:
		var page: Dictionary = page_store.by_id[page_id]
		(page["previews"] as Dictionary).erase(key)
		(page["tiles"] as Dictionary)[key] = true
	preview.erase(key)
	counters["edit_adopted"] += 1
	# As _upload: the installed level moves the hysteresis, and a bake of another level or token
	# is still wanted.
	var want: Dictionary = _desired.get(key, {})
	if not want.is_empty():
		_level_tile(key, want)
		if want["level"] != Vector2i(int(tile["level"]), int(tile["step"])) or int(want["token"]) != token:
			_wanted[key] = true
	return true

# Rebakes the tiles an edit dirtied at the level each is held at, installed or in flight.
func _apply_edit(edit: Dictionary) -> void:
	counters["edits"] += 1
	var serial := int(edit["serial"])
	_edit_serial_taken = serial
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
		tile["preview_epoch"] = int(edit.get("preview_epoch", -1))
		if bool(tile["behind"]):
			_variant_missing(key)
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

# Records that `tile` found level `code`'s pool exhausted at `request_key` and `token`: counted,
# logged, not asked again until a slot of the level frees, and its patches' sets draw without it
# (_draw_state). O(1).
func _exhaust(tile: Vector2i, code: int, request_key: String, token: int) -> void:
	_wanted.erase(tile)
	_exhausted[tile] = [code, request_key, token, false]
	var order: Array = _exhausted_order[code]
	order.append(tile)
	# A tile recorded again leaves its earlier entry behind: past twice the records, the order is
	# rebuilt from them (amortized O(1)), so it stays bounded however long the pools stay exhausted.
	if order.size() > 2 * _exhausted.size() + 64:
		order.clear()
		for waiting: Vector2i in _exhausted.keys():
			if int(_exhausted[waiting][0]) == code and not bool(_exhausted[waiting][3]):
				order.append(waiting)
	counters["pool_exhausted"] += 1
	MetrumDebug.log("terrain", "graded pool %d exhausted at tile %s" % [code, str(tile)])
	for patch_key in _patches_meeting(Rect2(Vector2(tile) * TILE_M, Vector2.ONE * TILE_M)):
		_dirty_shown[patch_key] = true

# The patch keys whose rectangle meets `rect`, from the patch grid. O(patches met).
func _patches_meeting(rect: Rect2) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	var first := ((rect.position - world_min) / patch_span_m).floor()
	var last := ((rect.end - world_min) / patch_span_m).floor()
	for z in range(int(first.y), int(last.y) + 1):
		for x in range(int(first.x), int(last.x) + 1):
			keys.append(Vector2i(x, z))
	return keys

# Tile `key` was installed at a new slot or lattice, or evicted: its own instance changes, and with
# `sides` (its lattice code changed) the sides of its four neighbours too (the cell code is all a
# side reads of its neighbour), each in the patches that draw it. O(1).
func _cell_changed(key: Vector2i, sides := true) -> void:
	_dirty_cell_tiles[key] = true
	if not sides:
		return
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
	# From residency on, the patch draws what _draw_state says: nothing until its list arrives.
	_dirty_shown[key] = true
	# The installed tiles it draws want copies in its blocks' pages (_tile_complete).
	for tile: Vector2i in _patch_tiles(resident_patches[key]["rect"]):
		if installed.has(tile):
			_wanted[tile] = true
	# Every resident patch builds its set; a drawing set follows a new step in this frame (the
	# camera moved), a hidden one is built again off the frame.
	var cells: Dictionary = patch_cells.get(key, {})
	if not cells.is_empty() and bool(cells["shown"]) and cells["node_parent"] != resident_patches[key]["node"]:
		_hide_cells(cells)
		_dirty_shown[key] = true
	if not cells.is_empty():
		_park(cells, resident_patches[key]["node"])
	if cells.is_empty():
		_builds[key] = true
	elif int(cells["step"]) != int(resident_patches[key]["step"]):
		if bool(cells["shown"]):
			_dirty_patches[key] = true
		else:
			cells["built"] = -1
			_builds[key] = true

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
	if _growth_waits.has(tile):
		var waiting: Vector3i = _growth_waits[tile]
		(_block_waiting[waiting.x].get(Vector2i(waiting.y, waiting.z), {}) as Dictionary).erase(tile)
		_growth_waits.erase(tile)
	if installed.has(tile):
		var entry: Dictionary = installed[tile]
		installed.erase(tile)
		_free_copies(tile, entry, false)
		counters["evictions"] += 1
		_cell_changed(tile)
		if not (entry["walls"] as PackedFloat32Array).is_empty():
			_walls_changed(tile)
	_edit_batch.erase(tile)
	_edit_staged.erase(tile)
	_exhausted.erase(tile)
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
			and _tile_complete(tile, current)
		):
			_wanted.erase(tile)
			continue
		# A tile waiting for its block's larger page is asked again when the block moves.
		if _growth_waits.has(tile):
			_wanted.erase(tile)
			continue
		var request_key := "%d,%d,%d,%d" % [tile.x, tile.y, level.x, level.y]
		if int(requested.get(request_key, -1)) == token or int(refused.get(request_key, -1)) == token:
			_wanted.erase(tile)
			continue
		var exhausted: Array = _exhausted.get(tile, [])
		if not exhausted.is_empty() and String(exhausted[1]) == request_key and int(exhausted[2]) == token:
			if not bool(exhausted[3]):
				_wanted.erase(tile)
				continue
		else:
			_exhausted.erase(tile)
		# A new copy needs a free slot in its page (a rebake at the installed level rewrites its
		# own): a full page grows (the tile waits for the move), or the tile is exhausted.
		var same_level := not current.is_empty() and int(current["level"]) == level.x
		var held: Dictionary = current["copies"] if same_level else {}
		var blocked := false
		for pair: Array in _copy_pages(tile, level.x):
			var page_id := int(pair[0])
			var page: Dictionary = page_store.by_id.get(page_id, {})
			if not page.is_empty() and (held.has(page_id) or not (page["free"] as Array).is_empty()):
				continue
			blocked = true
			if page_id == PAGE_PENDING:
				# Its block's lists are not all in: it stays wanted for a later frame.
				pass
			elif not page.is_empty() and _wait_or_grow(level.x, pair[1], page_id, 0):
				_wait_for_growth(tile, level.x, pair[1])
			else:
				_exhaust(tile, level.x, request_key, token)
			break
		if blocked:
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

# A cell's lattice code in patch `key`: its shown level's there, else the patch's base cells'.
func _cell_code(tile: Vector2i, step: int, key: Vector2i) -> int:
	var entry := _shown_in(tile, key)
	return int(entry["code"]) if not entry.is_empty() else base_code(step)

# The code a cell's side stitches to: the coarser of its own and its neighbour's, where the
# neighbour shares the tile side (inside the patch, or across the patch border at a tile side).
func _side_code(own: int, neighbour: Vector2i, side_m: float, patch: Dictionary, resident_patches: Dictionary, axis: int, own_key: Vector2i) -> int:
	var rect: Rect2 = patch["rect"]
	var lo := rect.position[axis]
	var hi := rect.end[axis]
	if side_m > lo and side_m < hi:
		return max(own, _cell_code(neighbour, int(patch["step"]), own_key))
	if side_m != lo and side_m != hi:
		# The cell is clamped to the patch here: its side is not the tile's.
		return own
	# Across the patch border: what the neighbour patch draws (T53 (a)), its set's cell, else its
	# PlaneMesh at its own step (or nothing, which no side meets). The set's flip rewrites the
	# cells across its sides (_apply_shown).
	var probe := (Vector2(neighbour) + Vector2(0.5, 0.5)) * TILE_M
	if side_m == lo:
		probe[axis] = lo - 0.5
	else:
		probe[axis] = hi + 0.5
	for key in _patches_meeting(Rect2(probe, Vector2.ZERO)):
		if resident_patches.has(key):
			var step := int(resident_patches[key]["step"])
			if bool(patch_cells.get(key, {}).get("shown", false)):
				return max(own, _cell_code(neighbour, step, key))
			return max(own, base_code(step))
	return own

# The cell MultiMesh a tile draws in, in patch `key`: its shown level's there (the behind-the-road
# variant's where its bake flags it), else the base cells'.
func _cell_mesh(tile: Vector2i, key: Vector2i) -> int:
	var entry := _shown_in(tile, key)
	if entry.is_empty():
		return CELL_BASE
	return int(entry["level"]) + (CELL_BEHIND if bool(entry["behind"]) else 0)

# Instances in a set's graded cell meshes (every one but the base cells').
static func _graded_cells(counts: Array[int]) -> int:
	var total := 0
	for index in CELL_MESHES:
		if index != CELL_BASE:
			total += counts[index]
	return total

# A tile's instance custom data in patch `key`'s cells, for the patch's rewrite and the per-tile
# updates alike: its place in the patch (column + PATCH_TILE_ROW * row from `first`, the tile
# holding the patch's lower corner), its slot in the page of the patch's block (-1 for a base
# cell), its own lattice code and each side's stitching code. Every channel an integer below
# 2048: the Compatibility renderer keeps instance custom data as half floats.
func _cell_custom(tile: Vector2i, first: Vector2i, patch: Dictionary, resident_patches: Dictionary, key: Vector2i) -> Color:
	var own := _cell_code(tile, int(patch["step"]), key)
	var origin := Vector2(tile) * TILE_M
	var codes := own
	codes |= _side_code(own, tile + Vector2i(-1, 0), origin.x, patch, resident_patches, 0, key) << 3
	codes |= _side_code(own, tile + Vector2i(1, 0), origin.x + TILE_M, patch, resident_patches, 0, key) << 6
	codes |= _side_code(own, tile + Vector2i(0, -1), origin.y, patch, resident_patches, 1, key) << 9
	codes |= _side_code(own, tile + Vector2i(0, 1), origin.y + TILE_M, patch, resident_patches, 1, key) << 12
	var entry := _shown_in(tile, key)
	var slot := -1
	if not entry.is_empty():
		slot = (entry["copies"][_patch_page(key, int(entry["level"]))] as Vector3i).x
	var place := tile - first
	return Color(place.x + PATCH_TILE_ROW * place.y, slot, codes & 511, codes >> 9)

# A tile's index in its cell set's per-place record (`where`), row-major over the patch's tiles.
static func _cell_place(cells: Dictionary, tile: Vector2i) -> int:
	var first: Vector2i = cells["first"]
	var last: Vector2i = cells["last"]
	return (tile.x - first.x) + (last.x - first.x + 1) * (tile.y - first.y)

# Rewrites the queued tiles' instances in every patch's set that holds them (the sets rewritten
# whole this frame excepted, a set being built only where its build has written them); a set
# whose instances changed may start or stop drawing. O(queued tiles).
func _update_cell_tiles(resident_patches: Dictionary) -> void:
	var touched := {}
	for tile: Vector2i in _dirty_cell_tiles.keys():
		_mask_changed(tile, resident_patches)
		for key in _patches_meeting(Rect2(Vector2(tile) * TILE_M, Vector2.ONE * TILE_M)):
			if _dirty_patches.has(key) or not patch_cells.has(key) or not resident_patches.has(key):
				continue
			if _update_tile_cell(patch_cells[key], tile, resident_patches[key], resident_patches):
				touched[key] = true
	_dirty_cell_tiles.clear()
	for key: Vector2i in touched.keys():
		_flush_cell_buffers(patch_cells[key])
		_dirty_shown[key] = true

# The grading payload shown by `tile` changed. Grass mirrors the cell buffers, so each resident
# patch drawing that tile must render its private mask again even when the cell stayed in place.
func _mask_changed(tile: Vector2i, resident_patches: Dictionary) -> void:
	for key in _patches_meeting(Rect2(Vector2(tile) * TILE_M, Vector2.ONE * TILE_M)):
		if patch_cells.has(key) and resident_patches.has(key):
			_cell_mask_revision += 1
			counters["mask_revision_bumps"] += 1
			patch_cells[key]["mask_revision"] = _cell_mask_revision

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
	if at.x < 0:
		# Not written yet: its build writes it as it stands then.
		return false
	var custom_us := Time.get_ticks_usec()
	var mesh_index := _cell_mesh(tile, cells["key"])
	var custom := _cell_custom(tile, first, patch, resident_patches, cells["key"])
	var records: Array = cells["custom"]
	var nodes: Array = cells["nodes"]
	# What the instance writes cost apart from computing them (the RenderingServer's part).
	var write_us := Time.get_ticks_usec()
	_frame_custom_us += write_us - custom_us
	if at.x == mesh_index:
		if records[at.x][at.y] == custom:
			return false
		records[at.x][at.y] = custom
		_write_cell(cells, at.x, at.y, custom)
		_frame_write_us += Time.get_ticks_usec() - write_us
		_frame_written += 1
		return true
	var counts: Array[int] = cells["counts"]
	var tile_at: Array = cells["tile_at"]
	var end := counts[at.x] - 1
	if at.y != end:
		var moved: Vector2i = tile_at[at.x][end]
		records[at.x][at.y] = records[at.x][end]
		tile_at[at.x][at.y] = moved
		where[_cell_place(cells, moved)] = at
		_write_cell(cells, at.x, at.y, records[at.x][at.y])
	_set_cell_count(nodes[at.x], counts, at.x, end)
	var index := counts[mesh_index]
	records[mesh_index][index] = custom
	tile_at[mesh_index][index] = tile
	where[place] = Vector2i(mesh_index, index)
	_write_cell(cells, mesh_index, index, custom)
	_set_cell_count(nodes[mesh_index], counts, mesh_index, index + 1)
	_frame_write_us += Time.get_ticks_usec() - write_us
	_frame_written += 1
	_frame_moved += 1
	return true

# Edit the authoritative CPU buffer, never Godot's partial per-instance cache (TERRAIN-10).
# Its motion-vector buffer switches can lose both transforms and unchanged custom data after
# hidden writes or count changes. All identity transforms are already in this buffer. O(1).
static func _write_cell(cells: Dictionary, mesh_index: int, index: int, custom: Color) -> void:
	var buffer: PackedFloat32Array = cells["buffers"][mesh_index]
	var offset := index * CELL_INSTANCE_FLOATS + 12
	buffer[offset] = custom.r
	buffer[offset + 1] = custom.g
	buffer[offset + 2] = custom.b
	buffer[offset + 3] = custom.a
	cells["buffer_dirty"][mesh_index] = 1

# One complete publication per changed mesh in this batch. Never read MultiMesh.buffer back
# or mix bulk and per-instance writes: that would stall on GPU read-back. O(dirty meshes x the
# bounded patch capacity), the same whole dirty region Godot uploaded for partial writes.
func _flush_cell_buffers(cells: Dictionary) -> void:
	var start_us := Time.get_ticks_usec()
	var dirty: PackedByteArray = cells["buffer_dirty"]
	var nodes: Array = cells["nodes"]
	for index in nodes.size():
		if dirty[index] == 0:
			continue
		(nodes[index] as MultiMeshInstance3D).multimesh.buffer = cells["buffers"][index]
		dirty[index] = 0
	_frame_write_us += Time.get_ticks_usec() - start_us

static func _set_cell_count(node: MultiMeshInstance3D, counts: Array[int], index: int, count: int) -> void:
	counts[index] = count
	node.multimesh.visible_instance_count = count
	node.visible = count > 0

# Builds patch `key`'s set whole in this frame: a drawing set whose patch changed step (the
# camera moved), and the tests' reference for the per-tile updates. O(the patch's tiles).
func _rebuild_patch_cells(key: Vector2i, resident_patches: Dictionary) -> void:
	var patch: Dictionary = resident_patches[key]
	var cells := _begin_build(key, patch)
	_build_tiles(cells, patch, resident_patches, -1, 0)
	if bool(cells["shown"]):
		(cells["nodes"][CELL_BASE] as GeometryInstance3D).material_override = patch["material"]
		var sync_us := Time.get_ticks_usec()
		_sync_material(cells, patch["material"])
		_frame_sync_us += Time.get_ticks_usec() - sync_us
	_dirty_shown[key] = true

# Whether a set's build has written every tile of its patch.
static func _built(cells: Dictionary) -> bool:
	return int(cells["built"]) >= 0 and int(cells["built"]) == (cells["tiles"] as Array).size()

# Starts (or restarts) patch `key`'s set for its patch as it stands: a pooled set taken if it has
# none, its rectangle, step and meshes set, no tile written. O(1) and the set's records.
func _begin_build(key: Vector2i, patch: Dictionary) -> Dictionary:
	var cells: Dictionary = patch_cells.get(key, {})
	if cells.is_empty():
		cells = _acquire_cells(patch)
		patch_cells[key] = cells
	cells["key"] = key
	_bind_page(cells, key)
	_park(cells, patch["node"])
	_cell_mask_revision += 1
	counters["mask_revision_bumps"] += 1
	cells["mask_revision"] = _cell_mask_revision
	cells["rect"] = patch["rect"]
	var step := int(patch["step"])
	cells["step"] = step
	var rect: Rect2 = patch["rect"]
	var rect_hm := Vector4i(
		int(round(rect.position.x * 2.0)), int(round(rect.position.y * 2.0)),
		int(round(rect.end.x * 2.0)), int(round(rect.end.y * 2.0))
	)
	var tiles := _patch_tiles(rect)
	cells["tiles"] = tiles
	# The tile holding the patch rectangle's lower corner, as the shader finds it, and the last.
	cells["first"] = tiles[0] if not tiles.is_empty() else Vector2i.ZERO
	cells["last"] = tiles[-1] if not tiles.is_empty() else Vector2i(-1, -1)
	cells["built"] = 0
	cells["synced"] = false
	cells["complete"] = false
	# What each MultiMesh holds, kept beside it (the per-tile updates move instances by it); a
	# place not written yet is (-1, -1).
	var counts: Array[int] = cells["counts"]
	counts.fill(0)
	var where: Array[Vector2i] = cells["where"]
	where.fill(Vector2i(-1, -1))
	cells["rect_hm"] = rect_hm
	# The build path gives the set the variant its patch's flagged tiles draw in.
	if not bool(cells["variant"]) and _patch_flagged(cells):
		_add_variant(cells)
	for node: MultiMeshInstance3D in cells["nodes"]:
		node.multimesh.visible_instance_count = 0
		node.visible = false
	if bool(cells["shown"]):
		_set_cell_meshes(cells)
	return cells

# A drawing set's meshes and instance uniforms for its patch's step and rectangle. Only a set
# starting to draw (or rewritten while drawing) takes them: a node with instance uniforms set holds
# Godot instance-uniform slots (16 each, `TERRAIN-07-D` round 7) whatever its material, so a set
# built off the frame paths that never drew holds none (`TERRAIN-07-E` round 6: setting them on
# every prebuilt set overflowed the buffer). From cell mesh `first` on (a variant added to a drawing
# set, _add_variant). O(1).
func _set_cell_meshes(cells: Dictionary, first := 0) -> void:
	var step := int(cells["step"])
	var nodes: Array = cells["nodes"]
	for index in range(first, nodes.size()):
		var node: MultiMeshInstance3D = nodes[index]
		# GL3 cells take the finest GL3 mesh (step 2): a coarser instance clamps the extra
		# vertices onto its sides.
		var level := index - CELL_BEHIND if index >= CELL_BEHIND else index
		var code := level if level < 3 else (base_code(step) if index == CELL_BASE else base_code(2))
		node.multimesh.mesh = _meshes[min(code, 6)]
		node.set_instance_shader_parameter("cell_patch_rect_hm", cells["rect_hm"])
		node.set_instance_shader_parameter("cell_lattice_anchor_hm", lattice_anchor_hm)
	if first <= CELL_BASE:
		(nodes[CELL_BASE] as GeometryInstance3D).set_instance_shader_parameter("cell_mode", 2.0)

# Writes the set's next tiles, each as it stands (its shown level's cell, else a base cell); with
# `start_us` >= 0 it stops once the frame's install budget is spent and at least `minimum` tiles
# were written. The last tile completes the set: its counts, walls and build counter. Returns
# whether the set is built. O(tiles written).
func _build_tiles(cells: Dictionary, patch: Dictionary, resident_patches: Dictionary, start_us: int, minimum: int) -> bool:
	var tiles: Array[Vector2i] = cells["tiles"]
	var first: Vector2i = cells["first"]
	var nodes: Array = cells["nodes"]
	var counts: Array[int] = cells["counts"]
	var records: Array = cells["custom"]
	var tile_at: Array = cells["tile_at"]
	var where: Array[Vector2i] = cells["where"]
	var built := int(cells["built"])
	var written := 0
	while built < tiles.size():
		if start_us >= 0 and written >= minimum and _over_budget(start_us):
			_flush_cell_buffers(cells)
			cells["built"] = built
			return false
		var tile := tiles[built]
		var mesh_index := _cell_mesh(tile, cells["key"])
		var custom := _cell_custom(tile, first, patch, resident_patches, cells["key"])
		var index := counts[mesh_index]
		_write_cell(cells, mesh_index, index, custom)
		records[mesh_index][index] = custom
		tile_at[mesh_index][index] = tile
		where[_cell_place(cells, tile)] = Vector2i(mesh_index, index)
		counts[mesh_index] = index + 1
		built += 1
		written += 1
	cells["built"] = built
	_flush_cell_buffers(cells)
	for index in nodes.size():
		_set_cell_count(nodes[index], counts, index, counts[index])
	var walls_us := Time.get_ticks_usec()
	_rebuild_patch_walls(cells, patch)
	_frame_walls_us += Time.get_ticks_usec() - walls_us
	counters["cell_builds"] += 1
	return true

# Builds the queued sets off the frame paths: the patches a shown tile waits for first, at least
# one tile of the first of them in every frame, then the rest while the frame's install budget
# lasts. A set first syncs its material, then writes its tiles; a built set starts drawing if a
# tile of it shows graded. O(tiles written this frame + uniforms synced).
func _advance_builds(start_us: int, resident_patches: Dictionary) -> void:
	_add_wanted_variants(start_us)
	var minimum := 1
	while not (_builds_urgent.is_empty() and _builds.is_empty()):
		var urgent := not _builds_urgent.is_empty()
		var queue: Dictionary = _builds_urgent if urgent else _builds
		if not urgent:
			minimum = 0
		if minimum == 0 and _over_budget(start_us):
			return
		var key := Vector2i.ZERO
		for queued: Vector2i in queue:
			key = queued
			break
		if not resident_patches.has(key):
			_builds.erase(key)
			_builds_urgent.erase(key)
			continue
		var patch: Dictionary = resident_patches[key]
		var cells: Dictionary = patch_cells.get(key, {})
		if cells.is_empty() or int(cells["built"]) < 0:
			cells = _begin_build(key, patch)
		var done := _built(cells)
		if not done and not bool(cells["synced"]):
			cells["synced"] = _sync_material(cells, patch["material"], start_us)
		if not done and bool(cells["synced"]):
			done = _build_tiles(cells, patch, resident_patches, start_us, minimum)
		minimum = 0
		if not done:
			return
		_builds.erase(key)
		_builds_urgent.erase(key)
		_dirty_shown[key] = true

# The sets a flagged tile waits for take the variant, first in the build path: the first of them
# in every frame, then the rest while the frame's install budget lasts. O(sets given it x
# _add_variant).
func _add_wanted_variants(start_us: int) -> void:
	var added := 0
	while not _variants_wanted.is_empty():
		if added > 0 and _over_budget(start_us):
			return
		var key := Vector2i.ZERO
		for queued: Vector2i in _variants_wanted:
			key = queued
			break
		_variants_wanted.erase(key)
		var cells: Dictionary = patch_cells.get(key, {})
		if not cells.is_empty() and not bool(cells["variant"]):
			_add_variant(cells)
			added += 1

# The flagged tiles whose sets all hold the variant now: a bake result installs, a preview tile
# shows (where its preview still wants it). O(waiting tiles).
func _take_variant_waits(resident_patches: Dictionary) -> void:
	for key: Vector2i in _variant_waits.keys():
		if not _variant_missing(key):
			var tile: Dictionary = _variant_waits[key]
			_variant_waits.erase(key)
			_install(tile, resident_patches)
	for key: Vector2i in _preview_waits.keys():
		if not _variant_missing(key):
			_show_preview_tile(_preview_waits[key])

# What resident patch `key` draws (T53 (a), decided by the user, 2026-10-28): its PlaneMesh only
# while its list is known to name no tile and no preview tile lies in it; otherwise its set once
# the set is built and shows every tile its list names, and nothing until then, so no frame shows
# the base ground under a graded road or a set with listed tiles missing. A listed tile that cannot
# show (refused by contract, or its level's pool exhausted: counted, logged errors) draws its base
# cell, and a patch none of whose listed tiles can show draws its PlaneMesh. A patch whose list has
# not arrived draws nothing (the residency margin keeps such patches out of the view). A set that
# drew once in the patch's residency (`complete`) keeps drawing while its list is refreshed: an
# edit installs its own tiles in its batch's frame, and a tile the new list adds that the edit did
# not select renders the base it rendered before. A list being refreshed (of an older generation)
# counts the set's installed tiles with it, so a commit's tiles draw before the new list arrives.
# O(the patch's list) while the set is not complete, else O(1), and the preview's tiles.
func _draw_state(key: Vector2i, patch: Dictionary, cells: Dictionary) -> int:
	var list: Dictionary = patch_lists.get(key, {})
	if list.is_empty() and not patch_lists_refused.has(key):
		return DRAW_NONE
	var tiles: Array = list.get("tiles", [])
	var built := not cells.is_empty() and _built(cells)
	var graded := built and _graded_cells(cells["counts"]) > 0
	var refreshing := not list.is_empty() and int(list["token"]) != int(patch["generation"])
	var previews := _previews_in(patch["rect"])
	if tiles.is_empty() and not (refreshing and graded) and not previews:
		return DRAW_PLANE
	if graded and bool(cells["complete"]):
		return DRAW_SET
	# What the set would show: previews, a refreshing list's installed tiles, the listed tiles.
	var showing := previews or (refreshing and graded)
	for tile: Vector2i in tiles:
		if not _shown_in(tile, key).is_empty():
			showing = true
		elif not _refused_at_level(tile):
			return DRAW_NONE
	# Every listed tile shows (its set draws once built), or none can (refused, or its page
	# exhausted: errors, drawn as the base their cells would draw, with or without the set).
	if not showing:
		return DRAW_PLANE
	if not (graded and built):
		return DRAW_NONE
	# And every installed tile the patch draws is copied into its block's page: a border tile's
	# second copy may arrive frames after the first, and the two halves must not differ.
	for tile: Vector2i in cells["tiles"]:
		if installed.has(tile) and _shown_in(tile, key).is_empty() and not _refused_at_level(tile):
			return DRAW_NONE
	return DRAW_SET

# Whether `tile` cannot show at its desired level and token: Rust refused it by contract (T7) or
# its level's pool is exhausted (T23). Its patch's set draws without it (a base cell; both are
# counted and logged errors).
func _refused_at_level(tile: Vector2i) -> bool:
	var want: Dictionary = _desired.get(tile, {})
	if want.is_empty():
		return false
	var level: Vector2i = want["level"]
	var request_key := "%d,%d,%d,%d" % [tile.x, tile.y, level.x, level.y]
	var exhausted: Array = _exhausted.get(tile, [])
	if not exhausted.is_empty() and String(exhausted[1]) == request_key and int(exhausted[2]) == int(want["token"]):
		return true
	return int(refused.get(request_key, -1)) == int(want["token"])

# Patch `key` draws what _draw_state says: its set shown (its meshes and instance uniforms set, its
# materials bound and synced where its watched parameters moved) or hidden, waiting kept current
# under the patch's node with no material bound (so a set that never drew holds no instance
# uniform slot), and its PlaneMesh drawn only in DRAW_PLANE. A set starting or stopping to draw
# changes what the cells across the patch's sides stitch to (_side_code), so they are rewritten in
# the same frame (_apply_cell_changes). O(_draw_state), and the material's parameters when the set
# starts drawing.
func _apply_shown(key: Vector2i, resident_patches: Dictionary) -> void:
	if not resident_patches.has(key):
		return
	var patch: Dictionary = resident_patches[key]
	var cells: Dictionary = patch_cells.get(key, {})
	var state := _draw_state(key, patch, cells)
	_set_ready(key, state != DRAW_NONE)
	# A patch drawing nothing whose list names a tile waits for its set as a shown tile does: its
	# build goes first (one whose list has not arrived may need none).
	if state == DRAW_NONE and (cells.is_empty() or not _built(cells)) and not (patch_lists.get(key, {}).get("tiles", []) as Array).is_empty():
		_builds_urgent[key] = true
	var patch_node: GeometryInstance3D = patch["node"]
	_set_plane(key, patch_node, state == DRAW_PLANE)
	var shown := not cells.is_empty() and bool(cells["shown"])
	if state == DRAW_PLANE and not cells.is_empty():
		cells["complete"] = false
	if (state == DRAW_SET) == shown:
		return
	var flip_us := Time.get_ticks_usec()
	_frame_flips += 1
	_border_changed(key)
	if shown:
		_hide_cells(cells)
		_frame_flip_us += Time.get_ticks_usec() - flip_us
		return
	cells["shown"] = true
	cells["complete"] = true
	_set_cell_meshes(cells)
	_bind_materials(cells, true)
	(cells["nodes"][CELL_BASE] as GeometryInstance3D).material_override = patch["material"]
	# The set synced its material as it was built; a drawing set follows its patch's material
	# changes (process), a hidden one catches up here, as the watched parameters tell.
	var sync_us := Time.get_ticks_usec()
	if _watched_parameters(patch["material"]) != cells["watched"]:
		_sync_material(cells, patch["material"])
	_frame_sync_us += Time.get_ticks_usec() - sync_us
	_park(cells, patch_node)
	(cells["root"] as Node3D).visible = true
	cells["node_parent"] = patch_node
	_frame_flip_us += Time.get_ticks_usec() - flip_us

# Patch `key`'s PlaneMesh draws (`drawn`) or not. Collapsed, it is not drawn at all: its
# visibility range ends before any camera, so its vertices do not run (TERRAIN-07-D round 5). It
# stays visible, since grass and the water reflection read the patch node's visibility. Drawing
# again restores the node it collapsed, whatever node the patch holds now: terrain.gd pools its
# patch nodes, and a pooled node left collapsed draws nothing for the next patch it takes
# (`TERRAIN-07-E` round 6). O(1).
func _set_plane(key: Vector2i, patch_node: GeometryInstance3D, drawn: bool) -> void:
	var held: Variant = _collapsed.get(key)
	if held != null and (drawn or held != patch_node):
		_collapsed.erase(key)
		if is_instance_valid(held):
			(held as GeometryInstance3D).set_instance_shader_parameter("cell_mode", 0.0)
			(held as GeometryInstance3D).visibility_range_end = 0.0
		held = null
	if drawn or held != null or patch_node == null:
		return
	patch_node.set_instance_shader_parameter("cell_mode", 1.0)
	patch_node.visibility_range_end = HIDDEN_RANGE_M
	_collapsed[key] = patch_node

# Patch `key` is ready (draws its set or its PlaneMesh) or not (draws nothing). The time and
# frames from its residency to ready are the residency margin's latency (counters); an unready
# patch within the view distance counts in each frame (process). O(1).
func _set_ready(key: Vector2i, ready: bool) -> void:
	if not ready:
		if not _unready.has(key):
			_unready[key] = Vector2i(Time.get_ticks_msec(), Engine.get_process_frames())
		return
	var since: Variant = _unready.get(key)
	if since == null:
		return
	_unready.erase(key)
	counters["ready_ms_max"] = max(int(counters["ready_ms_max"]), Time.get_ticks_msec() - (since as Vector2i).x)
	counters["ready_frames_max"] = max(int(counters["ready_frames_max"]), Engine.get_process_frames() - (since as Vector2i).y)

# Whether a preview tile meets the patch rectangle `rect`. O(preview tiles).
func _previews_in(rect: Rect2) -> bool:
	if preview.is_empty():
		return false
	var first := Vector2i((rect.position / TILE_M).floor())
	var last := Vector2i((rect.end / TILE_M).ceil()) - Vector2i.ONE
	for tile: Vector2i in preview:
		if tile.x >= first.x and tile.y >= first.y and tile.x <= last.x and tile.y <= last.y:
			return true
	return false

# The set stops drawing: its root hidden (still under the patch's node while the patch is
# resident) and its materials unbound; the patch's PlaneMesh is _set_plane's.
func _hide_cells(cells: Dictionary) -> void:
	cells["shown"] = false
	(cells["root"] as Node3D).visible = false
	cells["node_parent"] = null
	_bind_materials(cells, false)

# Puts the set's root under `patch_node` (null: out of the tree), its visibility unchanged. A built
# set waits under its patch's node, hidden, so starting to draw only shows it (`TERRAIN-07-E` round
# 7: moving the nodes into the tree was most of a flip on the pose frames); terrain.gd may free a
# released patch node, so release_patch takes the root out first. O(1).
func _park(cells: Dictionary, patch_node: Node) -> void:
	var root: Node3D = cells["root"]
	var parent := root.get_parent()
	if parent == patch_node:
		return
	if parent != null:
		parent.remove_child(root)
	if patch_node != null:
		patch_node.add_child(root)

## Cell set builds still queued (built off the frame paths: a world's sets after its load, or
## hidden sets whose patch took a new step). They take the streaming budget's spare time each
## frame, so a benchmark of the idle frame waits for none to be left. O(1).
func cell_builds_pending() -> int:
	return _builds.size() + _builds_urgent.size()

## Whether queued work will change what the graded cells draw without a new input: tile or list
## requests in flight or waiting for Rust's in-flight bound, counts, results deferred past a
## frame's budget, an edit batch waiting for its tiles, a resident patch drawing nothing (T53
## (a)), and the cell set builds of patches a shown tile waits for or whose list names a tile (a
## set whose patch lists none stays hidden when built, so its build changes nothing drawn).
## terrain.gd's is_render_settled includes it: a capture of the settled terrain waits on that one
## signal, and so does a world load's hand-over (`TERRAIN-07-G` round 6: the reference test's own
## copy of it left out the set builds). O(queued builds).
func has_pending_work() -> bool:
	if not (
		requested.is_empty() and patch_lists_requested.is_empty() and _lists_wanted.is_empty()
		and _counts.is_empty() and _wanted.is_empty() and _deferred.is_empty()
		and _edit_batch.is_empty() and _builds_urgent.is_empty() and _unready.is_empty()
		and _growth_waits.is_empty() and _variants_wanted.is_empty() and _variant_waits.is_empty()
		and _preview_waits.is_empty()
	):
		return true
	for code in 4:
		if not _block_target[code].is_empty():
			return true
	for key: Vector2i in _builds:
		if not (patch_lists.get(key, {}).get("tiles", []) as Array).is_empty():
			return true
	return false

## The world's patches within `radius_m` of the ground point `centre` (XZ, to the nearest point of
## each patch's rectangle) that do not yet draw what they draw once settled: not resident, their
## list not arrived (nor refused), or drawing nothing (T53 (a)). As of the last process() call.
## O(patches within the radius).
func unready_near(centre: Vector2, radius_m: float) -> int:
	var first := ((centre - Vector2.ONE * radius_m - world_min) / patch_span_m).floor()
	var last := ((centre + Vector2.ONE * radius_m - world_min) / patch_span_m).floor()
	var count := 0
	for z in range(maxi(int(first.y), 0), mini(int(last.y), _patch_grid.y - 1) + 1):
		for x in range(maxi(int(first.x), 0), mini(int(last.x), _patch_grid.x - 1) + 1):
			var key := Vector2i(x, z)
			var origin := world_min + Vector2(key) * patch_span_m
			if centre.clamp(origin, origin + Vector2.ONE * patch_span_m).distance_to(centre) > radius_m:
				continue
			if not _resident.has(key) or _unready.has(key) or not (patch_lists.has(key) or patch_lists_refused.has(key)):
				count += 1
	return count

## Resident patches drawing their PlaneMesh where T53 (a) forbids it: their list has not arrived
## or names a tile that can show (not refused, its pool not exhausted), or a preview tile meets
## them. Read from the patch nodes, for tests, which check it in every frame: always 0.
## O(resident patches' lists + preview tiles).
func plane_violations() -> int:
	var violations := 0
	for key: Vector2i in _resident.keys():
		var node: GeometryInstance3D = _resident[key]["node"]
		# The node keeps the range as a 32-bit float (hole_patches).
		if node == null or is_equal_approx(node.visibility_range_end, HIDDEN_RANGE_M):
			continue
		var list: Dictionary = patch_lists.get(key, {})
		var drawable := list.is_empty() and not patch_lists_refused.has(key)
		for tile: Vector2i in list.get("tiles", []):
			drawable = drawable or not _refused_at_level(tile)
		if drawable or _previews_in(_resident[key]["rect"]):
			violations += 1
	return violations

## Drawing sets whose list names a tile they do not show (not installed, not previewed and not
## refused): a partial set. For tests of a streaming run without edits (an edit's refreshed list
## may name a tile its edit did not select, drawn as the base it was until it installs,
## _draw_state): always 0 there. O(drawing sets' lists).
func partial_sets() -> int:
	var partial := 0
	for key: Vector2i in _resident.keys():
		if not bool(patch_cells.get(key, {}).get("shown", false)):
			continue
		for tile: Vector2i in patch_lists.get(key, {}).get("tiles", []):
			if _shown_in(tile, key).is_empty() and not _refused_at_level(tile):
				partial += 1
				break
	return partial

## Tiles of drawing sets that draw other than what they show (`TERRAIN-12`): per drawing set of a
## resident patch, each of its tiles must be exactly one live instance, in the cell mesh of what
## it shows there now (its base cell, or the graded level of its installed or preview tile,
## _cell_mesh), whose custom data in the MultiMesh itself is _cell_custom's now, and a graded
## instance's slot must hold that tile's copy in the page the set's material binds. Returned as
## {holes: tiles drawn from the wrong mesh, a vacated slot or an unbound page; stitches: tiles
## with only a stale side code}. For the probe and tests, every frame of a drag; O(drawing sets'
## tiles).
func tile_draw_violations() -> Dictionary:
	var out := {"holes": 0, "stitches": 0}
	var rendered := DisplayServer.get_name() != "headless"
	for key: Vector2i in _resident.keys():
		var cells: Dictionary = patch_cells.get(key, {})
		if cells.is_empty() or not bool(cells["shown"]) or not _built(cells):
			continue
		var patch: Dictionary = _resident[key]
		var where: Array[Vector2i] = cells["where"]
		var counts: Array[int] = cells["counts"]
		var tile_at: Array = cells["tile_at"]
		var records: Array = cells["custom"]
		var nodes: Array = cells["nodes"]
		var instances := 0
		for index in counts.size():
			if index < nodes.size():
				var node: MultiMeshInstance3D = nodes[index]
				if node.multimesh.visible_instance_count != counts[index] or node.visible != (counts[index] > 0):
					out["holes"] += 1
			instances += counts[index]
		if instances != (cells["tiles"] as Array).size():
			out["holes"] += 1
		for tile: Vector2i in cells["tiles"]:
			var at: Vector2i = where[_cell_place(cells, tile)]
			var mesh_index := _cell_mesh(tile, key)
			if at.x != mesh_index or at.y < 0 or at.y >= counts[at.x] or tile_at[at.x][at.y] != tile:
				out["holes"] += 1
				continue
			var custom: Color = records[at.x][at.y]
			# The MultiMesh itself where a rendering device keeps its data (headless keeps none).
			if rendered and (nodes[at.x] as MultiMeshInstance3D).multimesh.get_instance_custom_data(at.y) != custom:
				out["holes"] += 1
				continue
			var want := _cell_custom(tile, cells["first"], patch, _resident, key)
			if mesh_index != CELL_BASE:
				var entry := _shown_in(tile, key)
				var level := int(entry["level"])
				var page_id := _patch_page(key, level)
				var page: Dictionary = page_store.by_id.get(page_id, {})
				var holders: Dictionary = page.get("previews" if preview.get(tile, {}) == entry else "tiles", {})
				if int(cells["pages"][level]) != page_id or not holders.has(tile) or int(custom.g) != int(want.g):
					out["holes"] += 1
					continue
			if custom != want:
				out["holes" if int(custom.r) != int(want.r) or int(custom.b) & 7 != int(want.b) & 7 else "stitches"] += 1
	return out

## Resident patches drawing nothing: their PlaneMesh collapsed with no drawing set of theirs under
## it (a hole showing the sky). For tests and the probe; O(resident patches).
func hole_patches() -> int:
	var holes := 0
	for key: Vector2i in _resident.keys():
		var node: GeometryInstance3D = _resident[key]["node"]
		# The node keeps the range as a 32-bit float: the value read back is HIDDEN_RANGE_M
		# rounded to it, never equal to the 64-bit constant (round 6's check counted nothing).
		if node == null or not is_equal_approx(node.visibility_range_end, HIDDEN_RANGE_M):
			continue
		var cells: Dictionary = patch_cells.get(key, {})
		if cells.is_empty() or not bool(cells["shown"]) or cells["node_parent"] != node:
			holes += 1
	return holes

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
		var entry := _shown_in(tile, cells["key"])
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
## recycles (leaves the residency or the renderer resets).
func release_patch(key: Vector2i, patch_node: GeometryInstance3D) -> void:
	var cells: Dictionary = patch_cells.get(key, {})
	patch_cells.erase(key)
	_builds.erase(key)
	_builds_urgent.erase(key)
	_dirty_shown.erase(key)
	_unready.erase(key)
	_set_plane(key, patch_node, true)
	if cells.is_empty():
		return
	_hide_cells(cells)
	_park(cells, null)
	for node: MultiMeshInstance3D in cells["nodes"] + [cells["walls"]]:
		node.visible = false
		node.multimesh.visible_instance_count = 0
	cells["built"] = -1
	_cells_pool.append(cells)

# A pooled cell set, bounded to the patch's culling box. The pool holds one per patch of the
# grid (setup), so this creates none; an empty pool would mean more patches than the grid.
func _acquire_cells(patch: Dictionary) -> Dictionary:
	var cells: Dictionary = _cells_pool.pop_back() if not _cells_pool.is_empty() else _new_cells()
	var aabb: AABB = patch["aabb"]
	cells["aabb"] = aabb
	for node: MultiMeshInstance3D in cells["nodes"] + [cells["walls"]]:
		node.multimesh.custom_aabb = aabb
		node.custom_aabb = aabb
	return cells

# A set's graded materials are bound to its nodes only while it draws a patch (the base cells take
# the patch's material as it starts drawing, _apply_shown): Godot reserves a node's instance uniforms, 16 of
# its global buffer's slots, while the node's material declares them, so the pooled sets hold none
# (prewarmed bound, Kuopio's 1,296 sets held 5,184 nodes' slots, TERRAIN-07-D round 9). No
# material is created: the set's own are bound.
func _bind_materials(cells: Dictionary, bound: bool) -> void:
	var nodes: Array = cells["nodes"]
	var materials: Array[ShaderMaterial] = cells["materials"]
	for index in materials.size():
		if index != CELL_BASE:
			(nodes[index] as GeometryInstance3D).material_override = materials[index] if bound else null
	if not bound:
		(nodes[CELL_BASE] as GeometryInstance3D).material_override = null

# One patch's cell resources: the five plain cell MultiMeshes (GL0-GL3, the base cells), the graded
# levels' materials by cell mesh (none at CELL_BASE, which takes the patch's) and the wall
# MultiMesh; the behind-the-road variant's four come with its first flagged tile (_add_variant).
func _new_cells() -> Dictionary:
	var nodes: Array = []
	var materials: Array[ShaderMaterial] = []
	for index in CELL_PLAIN:
		_add_cell_mesh(nodes, materials, index)
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
	# The set's nodes stay under one root, which takes the set into a patch's node and out, and
	# shows it while it draws (_park, _apply_shown); each node's own visibility is whether it has
	# instances.
	var root := Node3D.new()
	root.name = "GradedCellSet"
	root.visible = false
	for node: Node in nodes + [walls]:
		root.add_child(node)
	# The record of what each cell MultiMesh holds (_rebuild_patch_cells, _update_tile_cell): per
	# mesh the custom data and tile of each instance (the variant's with it) and the instance
	# count, per tile place its (mesh, instance), all sized once here.
	var records := []
	var tile_at := []
	var buffers := []
	_add_cell_records(records, tile_at, buffers, CELL_PLAIN)
	var buffer_dirty := PackedByteArray()
	buffer_dirty.resize(CELL_MESHES)
	var where: Array[Vector2i] = []
	where.resize(_cells_per_patch)
	var counts: Array[int] = []
	counts.resize(CELL_MESHES)
	return {
		"root": root, "nodes": nodes, "walls": walls, "materials": materials, "watched": [], "node_parent": null,
		# Whether it holds the behind variant's meshes, and the culling box its nodes take.
		"variant": false, "aabb": AABB(),
		"custom": records, "tile_at": tile_at, "where": where, "counts": counts,
		"buffers": buffers, "buffer_dirty": buffer_dirty,
		"first": Vector2i.ZERO, "last": Vector2i(-1, -1), "tiles": [] as Array[Vector2i], "step": -1, "rect_hm": Vector4i.ZERO,
		# Its patch, and the page its materials bind per level (-1: none).
		"key": Vector2i.ZERO, "pages": [-1, -1, -1, -1],
		# The tiles its build has written (-1: not begun), whether it synced its material for
		# this build and whether it draws its patch.
		# and whether it drew complete in its patch's residency (_draw_state).
		"built": -1, "synced": false, "shown": false, "complete": false, "values": [], "sync_next": 0,
	}

# Cell mesh `index` of a set: its MultiMesh of the patch's tile places and its node, and its
# material (none for the base cells). The renderer owns the CPU buffers (_add_cell_records),
# writes cells there and publishes complete changed meshes (_flush_cell_buffers). No per-instance
# RenderingServer write follows a bulk upload, so no GPU read-back (TERRAIN-07-E round 6).
# One material per graded level and variant, so a draw binds only its
# page's two samplers (bound per patch, _bind_page).
func _add_cell_mesh(nodes: Array, materials: Array[ShaderMaterial], index: int) -> void:
	var multimesh := MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_3D
	multimesh.use_custom_data = true
	multimesh.instance_count = _cells_per_patch
	multimesh.visible_instance_count = 0
	counters["multimeshes_created"] += 1
	var node := MultiMeshInstance3D.new()
	node.name = "GradedCells%d" % index
	node.multimesh = multimesh
	node.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	node.visible = false
	nodes.append(node)
	if index == CELL_BASE:
		materials.append(null)
		return
	var material := ShaderMaterial.new()
	material.shader = GRADED_BEHIND_SHADER if index >= CELL_BEHIND else GRADED_SHADER
	counters["materials_created"] += 1
	materials.append(material)

# `meshes` more cell meshes' records and complete CPU buffers, allocated only in the build path.
func _add_cell_records(records: Array, tile_at: Array, buffers: Array, meshes: int) -> void:
	for index in meshes:
		var custom: Array[Color] = []
		custom.resize(_cells_per_patch)
		records.append(custom)
		var tiles: Array[Vector2i] = []
		tiles.resize(_cells_per_patch)
		tile_at.append(tiles)
		var buffer := PackedFloat32Array()
		buffer.resize(_cells_per_patch * CELL_INSTANCE_FLOATS)
		for instance in _cells_per_patch:
			var offset := instance * CELL_INSTANCE_FLOATS
			buffer[offset] = 1.0
			buffer[offset + 5] = 1.0
			buffer[offset + 10] = 1.0
		buffers.append(buffer)

# Gives a set the behind-the-road variant's four cell meshes and materials (T49 (c)), as its patch
# takes its first flagged tile: in the build path only (_begin_build, _advance_builds), never in a
# frame path. They take the set's state as it stands, so a set drawing its patch draws on: empty
# meshes under its root with its culling box, materials
# binding its pages and holding its synced parameters, the meshes and instance uniforms of a
# drawing set and its bound materials. A set keeps them through the pool (`TERRAIN-07-G` round
# 10): each set creates them at most once, so streaming creates and frees no node, and a set that
# never met a flagged tile holds none. O(the variant's instances + the patch material's uniforms).
func _add_variant(cells: Dictionary) -> void:
	var nodes: Array = cells["nodes"]
	var materials: Array[ShaderMaterial] = cells["materials"]
	var aabb: AABB = cells["aabb"]
	for index in range(CELL_BEHIND, CELL_MESHES):
		_add_cell_mesh(nodes, materials, index)
		var node: MultiMeshInstance3D = nodes[index]
		node.multimesh.custom_aabb = aabb
		node.custom_aabb = aabb
		(cells["root"] as Node).add_child(node)
	_add_cell_records(cells["custom"], cells["tile_at"], cells["buffers"], CELL_MESHES - CELL_PLAIN)
	cells["variant"] = true
	counters["cell_variants_created"] += 1
	var pages: Array = cells["pages"]
	var values: Array = cells["values"]
	for index in range(CELL_BEHIND, CELL_MESHES):
		var material := materials[index]
		var page: Dictionary = page_store.by_id.get(int(pages[index - CELL_BEHIND]), {})
		material.set_shader_parameter("graded_texels", page.get("texels", null))
		material.set_shader_parameter("graded_atlas", page.get("atlas", null))
		for at in values.size():
			if values[at] != null:
				material.set_shader_parameter(_uniform_names[at], values[at])
	if bool(cells["shown"]):
		_set_cell_meshes(cells, CELL_BEHIND)
		for index in range(CELL_BEHIND, CELL_MESHES):
			(nodes[index] as GeometryInstance3D).material_override = materials[index]
	# The grass mask mirrors the set's meshes.
	_cell_mask_revision += 1
	counters["mask_revision_bumps"] += 1
	cells["mask_revision"] = _cell_mask_revision

# Whether a tile shown or installed in the set's patch is flagged behind the road: the set must
# hold the variant (a preview dropped shows the installed tile again). O(the patch's tiles).
func _patch_flagged(cells: Dictionary) -> bool:
	for tile: Vector2i in cells["tiles"]:
		if bool(preview.get(tile, {}).get("behind", false)) or bool(installed.get(tile, {}).get("behind", false)):
			return true
	return false

# Whether a set of the patches meeting `tile` lacks the variant: each such set is queued for it
# (the build path adds it), and the flagged tile waits until none lacks it. O(1).
func _variant_missing(tile: Vector2i) -> bool:
	var missing := false
	for key in _patches_meeting(Rect2(Vector2(tile) * TILE_M, Vector2.ONE * TILE_M)):
		var cells: Dictionary = patch_cells.get(key, {})
		if cells.is_empty() or bool(cells["variant"]):
			continue
		var first: Vector2i = cells["first"]
		var last: Vector2i = cells["last"]
		if tile.x < first.x or tile.y < first.y or tile.x > last.x or tile.y > last.y:
			continue
		_variants_wanted[key] = true
		missing = true
	return missing

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

# Copies the patch material's parameters into the set's graded materials. With `start_us` >= 0 (a
# build off the frame paths) it goes on from where the last call stopped and stops once the frame's
# install budget is spent, after one parameter at least; returns whether it finished. Without, it
# goes over every parameter.
func _sync_material(cells: Dictionary, source: ShaderMaterial, start_us := -1) -> bool:
	if _uniform_names.is_empty():
		for uniform: Dictionary in source.shader.get_shader_uniform_list():
			var name := String(uniform["name"])
			if not name.begins_with("graded_") and not name.begins_with("cell_"):
				_uniform_names.append(name)
	# The values the cell set's materials hold, by uniform: only the parameters that differ are
	# copied into the graded materials (a payload commit changes a few of about a hundred). A set
	# keeps its values through the pool, so a reused set copies what differs from its last patch;
	# a new set holds none (null, an unset parameter's value, which needs no copy).
	var values: Array = cells["values"]
	if values.size() != _uniform_names.size():
		values.resize(_uniform_names.size())
	var index := int(cells["sync_next"]) if start_us >= 0 else 0
	var first := index
	# The watched parameters as the pass starts: one changing during a pass spread over frames
	# shows as a difference later, which syncs again.
	if index == 0:
		cells["watched"] = _watched_parameters(source)
	while index < _uniform_names.size():
		if start_us >= 0 and index > first and _over_budget(start_us):
			cells["sync_next"] = index
			return false
		var name := _uniform_names[index]
		var value: Variant = source.get_shader_parameter(name)
		index += 1
		if typeof(value) == typeof(values[index - 1]) and value == values[index - 1]:
			continue
		values[index - 1] = value
		for material: ShaderMaterial in cells["materials"]:
			if material != null:
				material.set_shader_parameter(name, value)
	cells["sync_next"] = 0
	return true

## Page sizes, use and creation counters, for tests and the memory record: per level code its
## pages (`pages_N`), their slots and free slots (`slots_N`, `free_N`, the pages its blocks bind
## or move to), their atlas layers and free layers; `pool_bytes`, every page's bytes (the free
## pooled ones included), and the layer limit and block side.
func get_pool_counters() -> Dictionary:
	var out := counters.duplicate()
	for code in page_store.levels.size():
		var pages := 0
		var slots := 0
		var free := 0
		var layers := 0
		var free_layers := 0
		for page_id: int in _block_page[code].values() + _block_target[code].values():
			var page: Dictionary = page_store.by_id[page_id]
			pages += 1
			slots += int(page["slots"])
			free += (page["free"] as Array).size()
			layers += int(page["layers"])
			for run: Vector2i in page["runs"]:
				free_layers += run.y
		out["pages_%d" % code] = pages
		out["slots_%d" % code] = slots
		out["free_%d" % code] = free
		out["layers_%d" % code] = layers
		out["free_layers_%d" % code] = free_layers
	out["pool_bytes"] = page_store.bytes
	out["layer_limit"] = layer_limit
	out["block_patches"] = block_patches
	out["growing"] = _growth_waits.size()
	out["installed"] = installed.size()
	out["preview"] = preview.size()
	out["patches_with_cells"] = patch_cells.size()
	out["cell_builds_queued"] = _builds.size() + _builds_urgent.size()
	out["cell_sets_free"] = _cells_pool.size()
	out["refused"] = refused.size() + patch_lists_refused.size()
	out["desired"] = _desired.size()
	# Tiles waiting for a slot of their level's exhausted pool (T23), and the desired tiles per
	# wanted level code (the pools' demand).
	out["exhausted"] = _exhausted.size()
	var demand := [0, 0, 0, 0]
	for want: Dictionary in _desired.values():
		var level: Vector2i = want["level"]
		if level.x >= 0:
			demand[level.x] += 1
	out["desired_by_level"] = demand
	return out
