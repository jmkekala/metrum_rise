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
## tunnel row and trough walls) draw as instances of one quad in a pooled MultiMesh per patch, placed by
## terrain_graded_walls.gdshader from the values in their instance transforms.
## The road preview's graded tiles (TERRAIN-07-E) draw over the installed ones from slots of
## their own (show_preview); a commit's or undo's edit carries the tiles it staged, which install
## with the edit's batch instead of rebaking, and the batch releases the preview tiles of the
## epoch the commit adopted, in the same frame.
##
## The graded state is Rust's (GradedTileBook, TERRAIN-13, `RENDER-32-B` and `-I`): the tiles'
## levels and travel buckets, the pages and a block's pages and growth, every installed and
## preview tile's copies, the requests, refusals and exhaustion, the edit batch's tokens and the
## hand-over to the bake queue, which starts the bakes nearest the camera first within its
## in-flight bound. This script calls it where it did that work and applies its effects
## (_apply_effects): page arrays and uploads, cancels, and the cells' Godot calls. The cell sets,
## walls and draw states are the book's too (`RENDER-32-J`): every set's instance records and CPU
## buffers, the per-tile updates, the builds' order, the wall faces and what each resident patch
## draws. Their Godot objects are the book's too (`RENDER-32-K`): per set a `RenderingServer`
## `MultiMesh` and instance per cell mesh and for the walls, and its graded materials, made through
## the stream backend, with the material sync from the patch's material; this script hands the
## book the meshes, shaders, page textures and the patches' nodes, materials and culling boxes it
## draws with, and applies the draw states to the patch nodes.
##
## Rust methods called: get_graded_tile_layout(), request_graded_patch_tiles(),
##   request_graded_tiles(), cancel_graded_tile(), cancel_graded_tile_requests(),
##   poll_graded_tile_results(), set_road_preview_terrain_view(); GradedTileBook
extends RefCounted

const PerfDebug := preload("res://scripts/core/perf_debug.gd")
const GRADED_SHADER := preload("res://assets/materials/terrain_graded.gdshader")
const GRADED_BEHIND_SHADER := preload("res://assets/materials/terrain_graded_behind.gdshader")
const WALL_SHADER := preload("res://assets/materials/terrain_graded_walls.gdshader")
# The patches' shader: a set's graded materials take its parameters.
const PATCH_SHADER := preload("res://assets/materials/terrain.gdshader")
const GradedPages := preload("res://scripts/renderers/terrain_graded_pages.gd")
const GradedTrace := preload("res://scripts/renderers/terrain_graded_trace.gd")
const TILE_M := 64.0
# Pages (docs/terrain.md "Pages and the row atlas", T54 (a) and T50 (b), TERRAIN-07-G round 8).
# GL0 and GL1 have one page each for the world, sized by their bands' bound (the memory table);
# GL2 and GL3 one per block of patches and level, sized for the block's listed tiles.
const WORLD_PAGE_SLOTS := [64, 192]
# Atlas layers a block page holds per slot: the census's mean rows (455-620 texels) take two or
# three layers.
const PAGE_LAYERS_PER_SLOT := 3
# Patch cells: a patch meets at most (ceil(span / 64 m) + 1)^2 tiles (9 x 9 on a 510 m patch),
# the instances of each cell MultiMesh (_cells_per_patch). A tile's place in its patch is
# column + 32 * row in one instance channel (terrain_cells.gdshaderinc; the book writes it).
const INSTALL_BUDGET_MS := 1.0
const POLL_BATCH := 4
# The cell meshes of a patch: GL0-GL3 (0-3), the base cells (CELL_BASE) and GL0-GL3 of the tiles
# whose bake flags them behind the road (CELL_BEHIND + level, T49 (c)): their cells draw the
# behind-the-road variant, which writes depth, so every other cell keeps the early depth test.
# Every set holds the first CELL_PLAIN; the variant's four meshes and materials only a set whose
# patch has shown a flagged tile (the book's variant, `TERRAIN-07-G` round 10).
const CELL_MESHES := 9
const CELL_BASE := 4
const CELL_BEHIND := 5
const CELL_PLAIN := 5
# Wall face instances per patch, prewarmed with its cells: at most 42 faces per patch on the
# Kuopio reference save and none on the populated save (TERRAIN-07-D round 6 census, GL0). A
# tunnel approach's trough walls (`ROAD-61` round R8e) take about 256 faces per side per 64 m at
# GL0, so a patch that needs more grows its MultiMesh to the next power of two (the book's wall rebuild).
const MAX_WALLS_PER_PATCH := 256
# A visibility range no camera is within (metres): a patch's PlaneMesh while its cells draw it.
const HIDDEN_RANGE_M := 0.001
const WALL_FACE_FLOATS := 8
# Every (level code, GL3 step) a tile can be requested at, in the order of request_graded_tiles'
# replaced-request codes.
const LEVEL_STEPS := [Vector2i(0, 0), Vector2i(1, 0), Vector2i(2, 0), Vector2i(3, 2), Vector2i(3, 4), Vector2i(3, 8)]

var simulation_node: Node
## Record and replay (terrain_graded_trace.gd): null unless METRUM_GRADED_TRACE names a file to
## record into (one file per renderer: `<path>.<n>`), or a replay sets it before setup().
var trace: RefCounted = null
static var _traces := 0
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
# The graded state in Rust (GradedTileBook, `RENDER-32-I`): the pages and a block's pages and
# growth, every installed and preview tile's copies, the requests, refusals and exhaustion, the
# desired levels and counts, the edit batch's tokens and the hand-over. Each call's effects are
# applied in order (_apply_effects).
var _book: GradedTileBook = null
## For tests and the probe: tile key -> {code, level, step, token, walls, behind, copies} of every
## installed tile, built from the book on each read (`copies` maps a page id to its copy there,
## Vector3i(slot, first atlas layer, layers)). The renderer itself reads none.
var installed: Dictionary:
	get:
		return _entries(false)
# The edit batch's staged results (the book keeps the batch's tokens): tile key -> its bake.
var _edit_staged: Dictionary = {}
# The bake result an upload's write effects write.
var _uploading: Dictionary = {}
# The views `installed` and `preview` built since the book last changed (null: none).
var _views: Array = [null, null]
# A traced hand-over's answers (per stale cancel, then per request) and the next cancel's.
var _hand_over := PackedInt64Array()
var _hand_over_at := 0
# Whether the book's resident set follows the live dictionary (from the first process()).
var _resident_synced := false
# Results polled after the frame's install budget ran out, in arrival order, as [kind, result]:
# the next frame takes them first (Rust hands a result over once).
var _deferred: Array[Array] = []
# A poll's result kinds, in the order a poll's results are taken.
const RESULT_KINDS := ["edits", "patches", "failed", "tiles"]
# The tiles the last edit batch installed, for tests and the probe.
var last_edit_batch: Array = []
## For tests and the probe: the road preview's tiles (TERRAIN-07-E) as `installed` gives them
## (the token is the preview epoch), drawn in place of the installed tile from copies of their own.
var preview: Dictionary:
	get:
		return _entries(true)
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
# Patch lists Rust refused by contract (retry false): patch key -> the token refused, never asked
# again at that token. (Refused tiles and exhausted pools are the book's.)
var patch_lists_refused: Dictionary = {}
## For tests: patch key -> its cell set as the book holds it (`GradedTileBook.cell_objects`, with
## `set`, `variant` and `rect`), built on each read. Cell sets are built off the frame paths
## (`TERRAIN-07-E` round 6): every resident patch gets its set (prewarmed in setup, one per patch of
## the grid), kept current by the per-tile updates and hidden until one of its tiles shows graded
## (the book's build queues, the patches a shown tile waits for first).
var patch_cells: Dictionary:
	get:
		var out := {}
		if _book == null:
			return out
		for key: Vector2i in _resident.keys():
			var set := _book.patch_set(key)
			if set < 0:
				continue
			var view: Dictionary = _book.cell_objects(set)
			view["set"] = set
			view["variant"] = (view["multimeshes"] as Array).size() == CELL_MESHES
			view["rect"] = _resident[key]["rect"]
			out[key] = view
		return out
# The last mask revision the book gave a set (grass mirrors a set's cells by it: mask_revision()).
var _cell_mask_revision: int:
	get:
		return _book.mask_revision_last() if _book != null else 0
var _cells_per_patch := 81
var _meshes: Dictionary = {}
# Materials this script made (the wall material; the graded ones are the book's).
var _materials_made := 0
var _wall_mesh: ArrayMesh = null
var _wall_material: ShaderMaterial = null
# The behind-the-road variant (T49 (c)) on demand (the book queues the sets the build path adds it
# to): the flagged tiles waiting for it in a set of theirs, so no set draws a flagged tile without
# its variant: tile key -> its bake result (installed once its sets hold the variant) and tile key
# -> its preview tile (shown then: the preview's lag, T27 (d)). An edit batch with a flagged tile
# waits whole (_install_edit_batch).
var _variant_waits: Dictionary = {}
var _preview_waits: Dictionary = {}
# What a resident patch draws (the book's draw state, T53 (a)): its PlaneMesh, its set or nothing.
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
var unbudgeted := false:
	set(value):
		unbudgeted = value
		if _book != null:
			_book.set_settle(unbudgeted, residency_settling)
## Whether terrain.gd's patch residency still has pending work this frame (its payloads,
## acknowledgements, LOD refreshes): while a load settles, a block's page waits for its patches.
var residency_settling := false:
	set(value):
		residency_settling = value
		if _book != null:
			_book.set_settle(unbudgeted, residency_settling)
# The desired tiles are the book's: every tile a resident patch's list names, at its distance's
# level (GL0 to 64 m, GL1 to 128 m, GL2 to 2 km, GL3 beyond at steps 2, 4, 8; T5, TD10) with
# hysteresis around the level it is installed at, re-levelled by travel bucket (T26). A list's
# tiles are counted in one at a time under the install budget (_poll_and_install), before the
# next result (a list counted at once took up to 0.35 ms past the budget, TERRAIN-07-D round 10).
# The last camera (XZ, with its height above the ground as y), and the level rule's ends the
# road preview's view takes (GradedTileLevels').
var _camera := Vector3.ZERO
var _camera_set := false
var _level_end_m := PackedFloat64Array()
var _gl3_end_m := PackedFloat64Array()
# The patch lists that may need a request: a patch that changed or whose list arrived, failed or
# was not accepted. (The tiles that may need one are the book's.)
var _lists_wanted: Dictionary = {}
# Tile bakes waiting in Rust's queue, as of the last poll.
var _tiles_pending := 0
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
	# Cell sets built (the book's builds), and the patches waiting for one.
	"cell_builds": 0,
	"cell_prewarm_ms": 0.0,
	# Sets given the behind-the-road variant (the book's), and flagged tiles that waited for it.
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
var _frame_lists := 0
var _frame_lists_same := 0
var _frame_relevelled := 0
# The wanted tiles _request_tiles took this frame and those it handed to Rust.
var _frame_considered := 0
var _frame_handed := 0

## Reads the slot layout, creates the world pages of GL0 and GL1 and prewarms the cell meshes.
## `pool_slots` overrides the per-level most slots a page holds (tests use small pages).
func setup(simulation: Node, cell_m: float, world_size: Vector2, span_m: float, pool_slots: Array = []) -> void:
	simulation_node = simulation
	if trace == null and not OS.get_environment("METRUM_GRADED_TRACE").is_empty():
		_traces += 1
		trace = GradedTrace.record("%s.%d" % [OS.get_environment("METRUM_GRADED_TRACE"), _traces])
	if trace != null:
		trace.entry("setup %s %s" % [GradedTrace.floats(PackedFloat32Array([cell_m, world_size.x, world_size.y, span_m])), GradedTrace.ints(pool_slots)])
	terrain_cell_m = cell_m
	patch_span_m = span_m
	world_min = -world_size * 0.5
	# The base lattice starts at the negative half extents (BaseGrid), in half-metres.
	lattice_anchor_hm = Vector2i(int(round(-world_size.x)), int(round(-world_size.y)))
	var layout: Dictionary = _sim_layout()
	_level_end_m = GradedTileLevels.level_end_m()
	_gl3_end_m = GradedTileLevels.gl3_end_m()
	levels.clear()
	for level: Dictionary in layout.get("levels", []):
		levels.append(level)
	layer_limit = GradedPages.device_layer_limit()
	if trace != null:
		layer_limit = int(trace.value("layer_limit", str(layer_limit)))
	# B: the largest block of patches whose tiles fit a quarter of the layers (docs/terrain.md
	# "Blocks"), so a full block's slots and its tiles' mean atlas runs fit one page.
	block_patches = 1
	while _block_tiles(block_patches + 1) <= layer_limit / 4:
		block_patches += 1
	page_store.counters = counters
	page_store.trace = trace
	page_store.clear_levels()
	_book = page_store.book
	_book.set_trace(trace)
	var shapes := PackedInt64Array()
	for code in levels.size():
		var cap: int = WORLD_PAGE_SLOTS[code] if code < 2 else _block_tiles(block_patches)
		if code < pool_slots.size():
			cap = int(pool_slots[code])
		page_store.add_level(levels[code], mini(cap, layer_limit))
		shapes.append_array(PackedInt64Array([int(levels[code]["side"]), mini(cap, layer_limit), int(levels[code]["max_row_texels"])]))
	_patch_grid = Vector2i(int(ceil(world_size.x / span_m)), int(ceil(world_size.y / span_m)))
	# The book drops every page and makes the world pages of GL0 and GL1, sized by their bands'
	# bound and the most rows a tile can hold (at most the layer limit), so they never fill.
	_book.setup(world_min, patch_span_m, _patch_grid, block_patches, layer_limit, shapes)
	_book.set_markers(trace != null)
	_book.set_settle(unbudgeted, residency_settling)
	_apply_effects()
	_build_meshes()
	_wall_material = ShaderMaterial.new()
	_wall_material.shader = WALL_SHADER
	_materials_made = 1
	# One cell set per patch of the world's grid, the most patches that can draw cells at once
	# (every resident patch may hold a graded tile), so neither streaming nor an edit creates one.
	var tiles_per_side := int(ceil(span_m / TILE_M)) + 1
	_cells_per_patch = tiles_per_side * tiles_per_side
	var grid := Vector2i(int(ceil(world_size.x / span_m)), int(ceil(world_size.y / span_m)))
	_patch_grid = grid
	var prewarm_us := Time.get_ticks_usec()
	# The book's records and CPU buffers of the sets, by index, all free, and their objects.
	_book.cells_setup(_cells_per_patch, grid.x * grid.y)
	_attach_cells()
	counters["cell_sets_prewarmed"] = grid.x * grid.y
	counters["cell_prewarm_ms"] = float(Time.get_ticks_usec() - prewarm_us) / 1000.0
	_sync_cell_counters()

# Hands the book what the cells draw with (`RENDER-32-K`): the main world, the lattice meshes and
# the wall quad, the graded shaders, the wall material, the lattice anchor, the patch material's
# parameters a set's materials take (all but the graded and cell ones), the meshes' triangles.
func _attach_cells() -> void:
	var meshes: Array[RID] = []
	var triangles := PackedInt64Array()
	for code in 7:
		meshes.append((_meshes[code] as Mesh).get_rid())
		triangles.append((_meshes[code] as ArrayMesh).surface_get_array_index_len(0) / 3)
	meshes.append(_wall_mesh.get_rid())
	var uniforms := PackedStringArray()
	for uniform: Dictionary in PATCH_SHADER.get_shader_uniform_list():
		var name := String(uniform["name"])
		if not name.begins_with("graded_") and not name.begins_with("cell_"):
			uniforms.append(name)
	var scene := Engine.get_main_loop() as SceneTree
	_book.cells_attach(scene.root.find_world_3d().scenario, meshes, GRADED_SHADER.get_rid(), GRADED_BEHIND_SHADER.get_rid(),
		_wall_material.get_rid(), lattice_anchor_hm, uniforms, triangles)
	# The pages made before (setup's world pages) bind by their textures.
	for id: int in page_store.by_id:
		_page_textures(id)

func _page_textures(id: int) -> void:
	var page: Dictionary = page_store.by_id[id]
	_book.set_page_textures(id, (page["texels"] as Texture).get_rid(), (page["atlas"] as Texture).get_rid())

# The book's object counters into `counters` (the creation counters prove nothing is made per
# edit).
func _sync_cell_counters() -> void:
	var cell: PackedInt64Array = _book.cell_counters()
	counters["walls_truncated"] = cell[3]
	counters["cell_variants_created"] = cell[4]
	counters["multimeshes_created"] = cell[5]
	counters["materials_created"] = _materials_made + cell[6]
	counters["cell_builds"] = cell[7]

# The tiles a block of `patches` x `patches` patches meets at most: T(B).
func _block_tiles(patches: int) -> int:
	var side := int(ceil(float(patches) * patch_span_m / TILE_M)) + 1
	return side * side

# --- Pages (docs/terrain.md "Pages and the row atlas"). ---

# Whether installed `tile` has a copy in every page that draws it (the book's). For tests.
func _tile_complete(tile: Vector2i, _entry: Dictionary = {}) -> bool:
	return _book.tile_complete(tile)

# Starts block `block`'s move at level `code` to a new page of `size` (slots, atlas layers): it
# becomes the block's target (a target that ran out too is dropped), preserving all installed
# and preview copies. Returns false where the budget refuses the page. O(page capacity).
func _move_block(code: int, block: Vector2i, size: Vector2i) -> bool:
	var moved := _book.move_block(code, block, size)
	_apply_effects()
	return moved

# The growing blocks whose target took a copy or whose page lost one: a target holding a copy of
# every installed tile the block's page holds becomes the page the block's sets bind
# (_switch_block). O(those blocks' tiles).
func _advance_growth() -> void:
	_book.advance_growth()
	_apply_effects()

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
	# The wall quad: x 0 at a face's end a and 1 at b, y 0 at its bottom and 1 at its top. Its
	# triangles are clockwise seen from +Z, the side terrain_graded_walls.gdshader's normal points
	# to (the face's lower side), so that side is the front face: with cull_disabled Godot
	# negates a back face's normal, and the other winding lit every wall seen from the road as if
	# it faced the hill (ROAD-61 round R8f).
	var quad := []
	quad.resize(Mesh.ARRAY_MAX)
	quad[Mesh.ARRAY_VERTEX] = PackedVector3Array([Vector3(0, 0, 0), Vector3(1, 0, 0), Vector3(0, 1, 0), Vector3(1, 1, 0)])
	quad[Mesh.ARRAY_INDEX] = PackedInt32Array([0, 3, 1, 0, 2, 3])
	_wall_mesh = ArrayMesh.new()
	_wall_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, quad)
	counters["meshes_created"] += 1

## The lattice code of the base cells and GL3 at a patch LOD step: 3 + log2(step).
static func base_code(step: int) -> int:
	return 3 + int(round(log(float(max(step, 1))) / log(2.0)))

# The cell sets' `RenderingServer` objects are not reference counted: they go with the renderer
# (terrain.gd makes a new one per world load and disposes of the last). A renderer dropped without
# dispose() frees them here; after dispose() there are none left. O(sets).
func _notification(what: int) -> void:
	if what == NOTIFICATION_PREDELETE and _book != null:
		_book.cells_detach()

## Frees every slot and cell resource, as the renderer resets.
func clear() -> void:
	if trace != null:
		trace.entry("clear")
	# Every copy freed (block pages go with their last; the moves end with them), and the book's
	# requests, refusals, tokens, lists, counts and desires dropped.
	if _book != null:
		_book.clear()
		_apply_effects()
	_edit_release = -1
	_edit_staged.clear()
	_deferred.clear()
	patch_lists.clear()
	patch_lists_requested.clear()
	patch_lists_refused.clear()
	for key: Vector2i in _resident.keys():
		if _book.patch_set(key) >= 0:
			release_patch(key, null)
	for key: Vector2i in _collapsed.keys():
		_set_plane(key, null, true)
	_unready.clear()
	_variant_waits.clear()
	_preview_waits.clear()
	if _book != null:
		_book.clear_cells()
	if is_instance_valid(simulation_node) or (trace != null and trace.replaying):
		_sim_cancel_all()
	_camera_set = false
	_lists_wanted.clear()
	_tiles_pending = 0

## Frees the cell sets' objects. Renderer replacement and shutdown call it before dropping the
## renderer; clear() first returns every set to the pool.
func dispose() -> void:
	clear()
	if _book != null:
		_book.cells_detach()

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
	if trace != null:
		trace.entry("process %s %d %d %s %s" % [GradedTrace.floats(PackedFloat32Array([camera_position.x, camera_position.y, camera_position.z])),
			int(unbudgeted), int(residency_settling), GradedTrace.keys(changed_patches), GradedTrace.keys(changed_materials)])
	var start_us := Time.get_ticks_usec()
	if not _resident_synced or not is_same(_resident, resident_patches):
		# From the first frame on the book's resident set follows this dictionary's mutations
		# (patch_mutated); before it the renderer saw none.
		_book.clear_resident()
		for key: Vector2i in resident_patches:
			_book_patch(key, resident_patches[key])
		_resident_synced = true
	_resident = resident_patches
	# A caller that changes the dictionary without patch_mutated (tests) names the keys here.
	for key: Vector2i in changed_patches:
		_book_patch(key, resident_patches.get(key, {}))
	_frame_lists = 0
	_frame_lists_same = 0
	_frame_relevelled = 0
	_frame_considered = 0
	_frame_handed = 0
	_book.reset_frame_counters()
	var uploads_before := int(counters["uploads"])
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
	var changes_us := Time.get_ticks_usec()
	_advance_builds(start_us, resident_patches)
	var builds_us := Time.get_ticks_usec()
	if not (_variant_waits.is_empty() and _preview_waits.is_empty()):
		_take_variant_waits(resident_patches)
	if not _lists_wanted.is_empty():
		_request_patch_lists(resident_patches)
	# The settle flags changed: parked blocks whose lists are in release their tiles.
	_book.release_parked()
	if _book.has_wanted():
		_request_tiles()
	var residency_us := Time.get_ticks_usec()
	# A patch's cells follow its material's parameters (a changed patch's LOD step in
	# _patch_changed). O(changed materials).
	for key: Vector2i in changed_materials.keys():
		_book.material_changed(key)
	var watched_us := Time.get_ticks_usec()
	_advance_growth()
	var cell_patches := _book.dirty_counts().x
	_apply_cell_changes()
	var cells_us := Time.get_ticks_usec()
	var wall_patches := _book.dirty_counts().y
	_apply_wall_changes()
	_count_unready_in_view(resident_patches)
	var perf := PerfDebug.is_enabled()
	if batch_tiles > 0 or perf:
		var end_us := Time.get_ticks_usec()
		var frame: PackedInt64Array = _book.frame_counters()
		var cell: PackedInt64Array = _book.cell_counters()
		var ms := func(from_us: int, to_us: int) -> float: return float(to_us - from_us) / 1000.0
		var parts := {
			"relevel_ms": ms.call(start_us, relevel_us),
			"batch_ms": ms.call(relevel_us, batch_us), "poll_ms": ms.call(batch_us, polled_us),
			"residency_ms": ms.call(polled_us, residency_us), "watched_ms": ms.call(residency_us, watched_us),
			# residency_ms's parts: the changed patches, the cell set builds and the requests
			# (variant waits, lists, parked tiles, tile requests).
			"patch_changes_ms": ms.call(polled_us, changes_us), "builds_ms": ms.call(changes_us, builds_us),
			"requests_ms": ms.call(builds_us, residency_us),
			"cells_ms": ms.call(watched_us, cells_us), "cells_sync_ms": float(cell[0]) / 1000.0,
			"cells_walls_ms": float(cell[1]) / 1000.0, "walls_ms": ms.call(cells_us, end_us),
			"total_ms": ms.call(start_us, end_us),
			"cell_patches": cell_patches, "wall_patches": wall_patches,
			"cell_tiles": frame[0], "relevelled": _frame_relevelled,
			"considered": _frame_considered, "handed": _frame_handed,
			"flips": frame[1], "flip_ms": float(cell[2]) / 1000.0,
			"lists": _frame_lists, "lists_unchanged": _frame_lists_same,
			"uploads": int(counters["uploads"]) - uploads_before,
			"wanted": wanted_count(), "installed": _book.entry_count(false),
		}
		if perf:
			counters["frame_parts"] = parts
		if batch_tiles > 0:
			counters["edit_frame_parts"] = parts
	if batch_tiles > 0:
		# The edit's frame: its uploads, the cell rewrites they caused and the frame's changes.
		counters["edit_batches"] += 1
		counters["edit_tiles_last"] = batch_tiles
		counters["edit_install_ms_last"] = float(Time.get_ticks_usec() - start_us) / 1000.0
		counters["edit_install_ms_max"] = max(float(counters["edit_install_ms_max"]), float(counters["edit_install_ms_last"]))

## Perf diagnostics: what the shown cell sets submit (before culling), instances and triangles per
## cell mesh, into `parts` as cells_<mesh> and cell_triangles_<mesh>. O(resident patches' cell
## meshes) a call; the perf channel calls it after its timed span, so it costs no renderer time.
func count_submitted_cells(parts: Dictionary) -> void:
	var submitted: PackedInt64Array = _book.submitted_cells()
	for index in CELL_MESHES:
		parts["cells_%d" % index] = submitted[index]
		parts["cell_triangles_%d" % index] = submitted[CELL_MESHES + index]

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

# The book's queued cell changes: a patch-wide change rewrites the patch's set, every other change
# its own tiles' instances in the sets drawing them; then the sets that may start or stop drawing
# and the cells across their sides. O(changed patches' tiles + changed tiles).
func _apply_cell_changes() -> void:
	_book.apply_cell_changes()
	_apply_effects()

func _apply_wall_changes() -> void:
	_book.apply_wall_changes()
	_apply_effects()

## The serial of the last graded edit whose tiles draw (docs/terrain.md T61 (2)): every edit
## taken once no batch waits, else those up to the last installed batch. The road tool holds an
## install's replaced road chunks until this reaches the journal's serial it read then. A batch
## waiting on its variant or results keeps it back; an edit no held tile takes leaves nothing to
## wait for. O(1).
func installed_edit_serial() -> int:
	return _edit_serial_taken if _book.edit_batch_len() == 0 and _edit_release < 0 else _edit_serial_batched

## Shows one road preview frame's graded tiles (TERRAIN-07-E, docs/terrain.md "Preview, commit,
## undo and load"): `changes` holds `graded_tiles` (packed as poll_graded_tile_results packs its
## tiles, the token the preview epoch) and `graded_removed` (flat (x, z) pairs), as preview_road
## and advance_road_preview_terrain return them. A shown tile takes a free slot of its level's
## pool and draws in place of the installed tile from this frame; a removed one returns to the
## installed tile. Returns the milliseconds it took. O(changed tiles x slot bytes).
func show_preview(changes: Dictionary) -> float:
	if trace != null:
		trace.entry(GradedTrace.preview(changes))
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
	var cell_patches := _book.dirty_counts().x
	var marks: PackedInt64Array = _book.frame_counters()
	var flip_us: int = _book.cell_counters()[2]
	_apply_cell_changes()
	var cells_us := Time.get_ticks_usec()
	var wall_patches := _book.dirty_counts().y
	_apply_wall_changes()
	var frame: PackedInt64Array = _book.frame_counters()
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
		"cell_tiles": frame[0] - marks[0], "flips": frame[1] - marks[1],
		"flip_ms": float(_book.cell_counters()[2] - flip_us) / 1000.0,
		"written": frame[2] - marks[2], "moved": frame[3] - marks[3],
	}
	return ms

## Drops the preview tiles of preview epochs `first_epoch` to `last_epoch`: the installed tiles
## draw again. A cancelled preview drops its own epoch; an edit batch drops every epoch up to the
## one its commit adopted.
func release_preview(first_epoch: int, last_epoch: int) -> void:
	if trace != null:
		trace.entry("release_preview %d %d" % [first_epoch, last_epoch])
	var tiles: PackedInt32Array = _book.preview_tiles(first_epoch, last_epoch)
	for index in range(0, tiles.size(), 2):
		_drop_preview(Vector2i(tiles[index], tiles[index + 1]))
	_drop_preview_waits(first_epoch, last_epoch)
	_apply_cell_changes()
	_apply_wall_changes()

func _show_preview_tile(tile: Dictionary) -> void:
	var key := Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))
	# A flagged tile shows once its sets hold the variant (the preview's lag, T27 (d)); the tile
	# shows what it showed meanwhile.
	_preview_waits.erase(key)
	if bool(tile["behind"]) and _book.variant_missing(key):
		counters["variant_waits"] += 1
		_preview_waits[key] = tile
		return
	var code := int(tile["level"])
	var layers := (tile["rows"] as PackedByteArray).size() / GradedPages.ATLAS_LAYER_BYTES
	_uploading = tile
	# Placed, what the tile draws changed: the book marks its cells, walls and sets.
	var placed: int = _book.place_preview(key, Vector4i(code, int(tile["step"]), int(tile["token"]), int(bool(tile["behind"]))), layers,
		(tile["walls"] as PackedByteArray).to_float32_array())
	_apply_effects()
	_uploading = {}
	if placed != 0:
		# The preview's lag (T27 (d)): a page that is full draws the tile once a later pose finds
		# room (its block grows meanwhile). A page that cannot grow (its cap or the budget) is the
		# level's pool exhausted, as the install path records it.
		if placed == 1:
			counters["preview_page_waits"] += 1
			MetrumDebug.log("terrain", "graded level %d page full at preview tile %s" % [code, str(key)])
		else:
			counters["pool_exhausted"] += 1
			MetrumDebug.log("terrain", "graded pool %d exhausted at preview tile %s" % [code, str(key)])

# Drops the preview tiles of epochs `first_epoch` to `last_epoch` waiting for a set's variant.
func _drop_preview_waits(first_epoch: int, last_epoch: int) -> void:
	for key: Vector2i in _preview_waits.keys():
		var token := int(_preview_waits[key]["token"])
		if token >= first_epoch and token <= last_epoch:
			_preview_waits.erase(key)

func _drop_preview(key: Vector2i) -> void:
	_preview_waits.erase(key)
	# The installed tile draws again: the book marks its cells, walls and sets.
	_book.drop_preview(key)
	_apply_effects()

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
		while _book.has_counts():
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
		var result: Dictionary = _sim_poll()
		counters["poll_ms_max"] = max(float(counters["poll_ms_max"]), float(Time.get_ticks_usec() - poll_start_us) / 1000.0)
		_tiles_pending = int(result.get("tiles_pending", 0))
		for kind: String in RESULT_KINDS:
			for entry: Dictionary in result.get(kind, []):
				if progressed and _over_budget(start_us):
					_deferred.append([kind, entry])
					counters["results_deferred"] += 1
					continue
				_take_result(kind, entry, resident_patches)
				progressed = true
				# The budget spent, the rest of the counts and every later result wait.
				while _book.has_counts() and not _over_budget(start_us):
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
	# empties or fills only starts or stops the set's drawing (the book's draw state); listed
	# tiles install or evict on their own. A patch without a set builds one.
	_book.list_cells(key, true)
	patch_lists[key] = {"token": int(list["token"]), "tiles": tiles}
	# Counted into the desired set; the parked tiles of its blocks released once their lists are in.
	_book.take_list(key, int(list["token"]), flat)
	_apply_effects()
	# A list baked for an older generation is asked again.
	_lists_wanted[key] = true

func _over_budget(start_us: int) -> bool:
	var over := not unbudgeted and float(Time.get_ticks_usec() - start_us) / 1000.0 >= INSTALL_BUDGET_MS
	if trace != null:
		over = trace.value("budget", str(int(over))) == "1"
	return over

# Applies the book's effects of the last calls, in order: page arrays (made, reused, released,
# dropped, copied, refused), slot uploads from the result being installed, Rust queue cancels,
# exhaustion, evictions and the staged results an eviction drops, the runs of cell effects the
# book applies to its objects (47) and the draw states.
func _apply_effects() -> void:
	_views = [null, null]
	var effects: PackedInt64Array = _book.take_effects()
	var cells := false
	for at in range(0, effects.size(), 7):
		var kind := int(effects[at])
		var tile := Vector2i(int(effects[at + 1]), int(effects[at + 2]))
		match kind:
			0, 1:
				page_store.made(int(effects[at + 1]), kind == 1, int(effects[at + 2]), Vector2i(int(effects[at + 3]), int(effects[at + 4])), int(effects[at + 5]), int(effects[at + 6]))
				if kind == 0:
					_page_textures(int(effects[at + 1]))
			2:
				page_store.released(int(effects[at + 1]), int(effects[at + 2]) == 1)
			3:
				page_store.dropped(int(effects[at + 1]))
			4:
				page_store.copied(int(effects[at + 1]), int(effects[at + 2]))
			5:
				page_store.refused(int(effects[at + 2]), Vector2i(int(effects[at + 3]), int(effects[at + 4])), int(effects[at + 5]), int(effects[at + 6]))
			10:
				page_store.write_copy(page_store.by_id[int(effects[at + 1])], Vector3i(int(effects[at + 2]), int(effects[at + 3]), int(effects[at + 4])), _uploading)
			12:
				if int(effects[at + 3]) == 1:
					# A traced hand-over's stale cancel, recorded in its place (_request_tiles).
					trace.out("cancel %d %d" % [tile.x, tile.y])
					_hand_over[_hand_over_at] = int(trace.value("cancel", str(_hand_over[_hand_over_at])))
					_hand_over_at += 1
				else:
					_sim_cancel(tile)
			13:
				counters["pool_exhausted"] += 1
				MetrumDebug.log("terrain", "graded pool %d exhausted at tile %s" % [int(effects[at + 3]), str(tile)])
			14:
				counters["evictions"] += 1
			15:
				_edit_staged.erase(tile)
			16:
				counters["same_level_uploads"] += 1
			21:
				counters["page_growths"] += 1
				MetrumDebug.log("terrain", "graded level %d block %s grows to %d slots, %d layers" % [int(effects[at + 1]), str(Vector2i(int(effects[at + 2]), int(effects[at + 3]))), int(effects[at + 4]), int(effects[at + 5])])
			22:
				counters["page_moves"] += 1
			47:
				_book.apply_cells(effects[at + 1], effects[at + 2])
				cells = true
			40:
				var key := Vector2i(int(effects[at + 1]), int(effects[at + 2]))
				var state := int(effects[at + 3])
				if trace != null:
					trace.out("draw %d %d %d" % [key.x, key.y, state])
				_set_ready(key, state != DRAW_NONE)
				_set_plane(key, _resident[key]["node"], state == DRAW_PLANE)
	if cells:
		_sync_cell_counters()

# --- Rust calls, through the trace when one records or replays (a replay calls no Rust). ---

func _sim_layout() -> Dictionary:
	if trace == null:
		return simulation_node.get_graded_tile_layout()
	var parts := PackedStringArray()
	if not trace.replaying:
		var layout: Dictionary = simulation_node.get_graded_tile_layout()
		for level: Dictionary in layout.get("levels", []):
			parts.append("%d,%d" % [int(level["side"]), int(level["max_row_texels"])])
	var levels := []
	for part in trace.value("layout", ";".join(parts)).split(";", false):
		levels.append({"side": int(part.get_slice(",", 0)), "max_row_texels": int(part.get_slice(",", 1))})
	return {"levels": levels}

func _sim_poll() -> Dictionary:
	if trace == null:
		return simulation_node.poll_graded_tile_results(POLL_BATCH)
	trace.out("poll %d" % POLL_BATCH)
	return trace.poll({} if trace.replaying else simulation_node.poll_graded_tile_results(POLL_BATCH))

func _sim_request_lists(flat: PackedInt64Array) -> PackedByteArray:
	if trace == null:
		return simulation_node.request_graded_patch_tiles(flat)
	trace.out("request_lists " + GradedTrace.ints(flat))
	var reply := "" if trace.replaying else GradedTrace.ints(simulation_node.request_graded_patch_tiles(flat))
	return PackedByteArray(Array(GradedTrace.ints_of(trace.value("request_lists", reply))))

func _sim_cancel(tile: Vector2i) -> int:
	if trace == null:
		return int(simulation_node.cancel_graded_tile(tile))
	trace.out("cancel %d %d" % [tile.x, tile.y])
	return int(trace.value("cancel", "" if trace.replaying else str(int(simulation_node.cancel_graded_tile(tile)))))

func _sim_cancel_all() -> void:
	if trace != null:
		trace.out("cancel_all")
		if trace.replaying:
			return
	simulation_node.cancel_graded_tile_requests()

func _sim_view(camera_position: Vector3) -> void:
	if trace != null:
		trace.out("view " + GradedTrace.floats(PackedFloat32Array([camera_position.x, camera_position.y, camera_position.z])))
		if trace.replaying:
			return
	simulation_node.set_road_preview_terrain_view(camera_position, _level_end_m, _gl3_end_m)

## terrain.gd's mutations of the resident patch dictionary, as it makes them: `entry` set for
## `key`, or with an empty `entry` erased; `clear_all` the whole dictionary cleared. The book's
## resident set follows them from the first process() on; a trace records them (a replay makes
## the same ones, so the dictionary's order is the same too).
func patch_mutated(key: Vector2i, entry: Dictionary, clear_all := false) -> void:
	if _book != null and _resident_synced:
		if clear_all:
			_book.clear_resident()
		else:
			_book_patch(key, entry)
	if trace == null or trace.replaying:
		return
	if clear_all:
		trace.entry("patches_clear")
	elif entry.is_empty():
		trace.entry("patch_erase %d %d" % [key.x, key.y])
	else:
		trace.entry(GradedTrace.patch(key, entry))

# The book's copy of a resident patch entry (its rectangle, step and generation, what its set
# draws with; empty: erased).
func _book_patch(key: Vector2i, entry: Dictionary) -> void:
	_book.set_patch(key, not entry.is_empty(), entry.get("rect", Rect2()), int(entry.get("step", 0)), int(entry.get("generation", 0)))
	if not entry.is_empty():
		# What its set draws with: the patch node (whose place it takes), material and culling box.
		_book.set_patch_draw(key, (entry["node"] as Object).get_instance_id(), (entry["material"] as Material).get_rid(), entry["aabb"])

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
			_book.list_cells(key, false)
			_book.list_refused(key, token)
			_apply_effects()
		_lists_wanted[key] = true
	else:
		var tile := Vector2i(keys[0], keys[1])
		# The book drops the request; a refusal by contract is recorded and leaves the edit batch.
		_book.failed_tile(tile, keys[2], keys[3], token, retry)
		if not retry:
			# The tile stays as it was; the edit installs without it, and its patches' sets draw
			# without it (the book's draw state).
			_edit_staged.erase(tile)
			_book.tile_refused(tile)
	if not retry:
		counters["bake_failures"] += 1
		MetrumDebug.log("terrain", "graded %s %s token %d refused: %s" % [failure["kind"], str(keys), token, failure["reason"]])

func _install(tile: Dictionary, resident_patches: Dictionary) -> void:
	var key := Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))
	var code := int(tile["level"])
	var step := int(tile["step"])
	var token := int(tile["token"])
	if _book.requested_token(key, code, step) != token:
		return
	var flagged := bool(tile["behind"])
	if _book.edit_batch_token(key) == token:
		# Part of an edit: it waits for the batch's other tiles (and, flagged, its sets' variant).
		_book.requested_erase(key, code, step, token)
		if flagged:
			_book.variant_missing(key)
		_edit_staged[key] = tile
		return
	# A later result of the tile supersedes one that waited (an older level's or token's).
	_drop_variant_wait(key)
	if flagged and _book.variant_missing(key):
		# Still requested while it waits (_take_variant_waits).
		counters["variant_waits"] += 1
		_variant_waits[key] = tile
		return
	_book.requested_erase(key, code, step, token)
	_upload(key, tile, resident_patches)

# Drops the bake result of `key` waiting for its sets' variant, and its request.
func _drop_variant_wait(key: Vector2i) -> void:
	var tile: Dictionary = _variant_waits.get(key, {})
	if tile.is_empty():
		return
	_variant_waits.erase(key)
	_book.requested_erase(key, int(tile["level"]), int(tile["step"]), int(tile["token"]))

func _upload(key: Vector2i, tile: Dictionary, _resident_patches: Dictionary, reserved: Variant = null) -> void:
	var level := Vector4i(int(tile["level"]), int(tile["step"]), int(tile["token"]), int(bool(tile["behind"])))
	var layers := (tile["rows"] as PackedByteArray).size() / GradedPages.ATLAS_LAYER_BYTES
	var walls: PackedFloat32Array = (tile["walls"] as PackedByteArray).to_float32_array()
	# Into every page its patches' blocks use; at the same level each copy keeps its slot and is
	# rewritten in place, before this frame draws (the book places, the effects write). A tile
	# whose page cannot take it waits for its block's larger page or is recorded exhausted. Its
	# entry takes the bake's walls; changed faces rewrite its patches' walls.
	_uploading = tile
	if reserved == null:
		_book.upload(key, level, layers, walls)
	else:
		_book.upload_reserved(key, level, layers, reserved, walls)
	_apply_effects()
	_uploading = {}

# Installs the edit batch once every tile of it has its result, all in this frame. Returns how
# many tiles it installed.
func _install_edit_batch(resident_patches: Dictionary) -> int:
	var batch := int(_book.edit_batch_len())
	if batch == 0 and _edit_release >= 0:
		# A commit whose staged tiles no held tile takes: its preview ends with the edit.
		release_preview(0, _edit_release)
		_edit_release = -1
	if batch == 0 or _edit_staged.size() < batch:
		return 0
	# A flagged tile's sets take the variant first (queued as it staged): the batch installs whole
	# once none lacks it.
	var waiting := false
	for key: Vector2i in _edit_staged:
		if bool(_edit_staged[key]["behind"]) and _book.variant_missing(key):
			waiting = true
	if waiting:
		counters["variant_waits"] += 1
		return 0
	# Adoption changes ownership only: the same preview slots remain drawn. Free the replaced
	# installed copies before planning the batch's allocations, without changing visible data.
	for key: Vector2i in _edit_staged:
		_adopt_preview(key, _edit_staged[key])
	var reservation := _reserve_edit_copies()
	if reservation.is_empty():
		return 0
	# Every allocation fits before any published texture is overwritten (the book placed them and
	# registered every incoming holder before the uploads free old-level copies).
	for key: Vector2i in _edit_staged:
		_upload(key, _edit_staged[key], resident_patches, reservation[key])
	last_edit_batch = _edit_staged.keys()
	var count := _edit_staged.size()
	_book.edit_batch_clear()
	_edit_staged.clear()
	_edit_serial_batched = _edit_serial_taken
	# The committed tiles replace the preview's in the same frame.
	if _edit_release >= 0:
		var released: PackedInt32Array = _book.preview_tiles(-(1 << 62), _edit_release)
		for index in range(0, released.size(), 2):
			_drop_preview(Vector2i(released[index], released[index + 1]))
		_drop_preview_waits(0, _edit_release)
		_edit_release = -1
	return count

# The book places every copy of the batch, all or none (a failure puts its pages back), and
# registers their holders. Failure starts a page move and keeps the complete edit staged; it must
# not acknowledge an upload that never happened. Returns tile -> its copies (flat page, slot,
# first, layers), empty on a failure. O(batch copies + free runs of the touched pages),
# independent of world size; idle does none.
func _reserve_edit_copies() -> Dictionary:
	var flat := PackedInt64Array()
	for key: Vector2i in _edit_staged:
		var tile: Dictionary = _edit_staged[key]
		var layers := (tile["rows"] as PackedByteArray).size() / GradedPages.ATLAS_LAYER_BYTES
		flat.append_array(PackedInt64Array([key.x, key.y, int(tile["level"]), int(tile["step"]), int(tile["token"]), layers]))
	var out: PackedInt64Array = _book.reserve_edit(flat)
	_apply_effects()
	if out[0] == 2:
		# Its level is exhausted at the device or budget cap (the explicit pool-exhaustion
		# contract): it leaves the batch without an upload.
		_edit_staged.erase(Vector2i(out[1], out[2]))
	if out[0] != 0:
		return {}
	var reserved := {}
	var at := 1
	for key: Vector2i in _edit_staged:
		var count := int(out[at])
		reserved[key] = out.slice(at + 1, at + 1 + 4 * count)
		at += 1 + 4 * count
	return reserved

# A committed tile that is the very payload the preview shows (Rust's `adopted`, the preview tile
# of the commit's epoch at the same level) takes over the preview's slot as the installed tile
# (`TERRAIN-07-E` round 6): nothing is uploaded and nothing drawn changes, so neither its instance
# nor its walls are rewritten. Returns whether it did. O(1).
func _adopt_preview(key: Vector2i, tile: Dictionary) -> bool:
	if not bool(tile.get("adopted", false)):
		return false
	if not _book.adopt_preview(key, Vector3i(int(tile["level"]), int(tile["step"]), int(tile["token"])), int(tile["preview_epoch"])):
		return false
	_apply_effects()
	counters["edit_adopted"] += 1
	return true

# Rebakes the tiles an edit dirtied at the level each is held at, installed or in flight.
func _apply_edit(edit: Dictionary) -> void:
	counters["edits"] += 1
	var serial := int(edit["serial"])
	_edit_serial_taken = serial
	if bool(edit["all"]):
		# Every batched, installed, requested and refused tile, in that order.
		_staged_dirty(_book.dirty_all(serial))
		return
	for entry: Dictionary in edit["levels"]:
		# The tiles held, batched, requested or refused at the level (a refused tile is held too:
		# the edit may have changed what refused it).
		_staged_dirty(_book.dirty_level(int(entry["level"]), int(entry["step"]), entry["tiles"], serial))
	# A commit's or undo's own tiles (TERRAIN-07-E): bitwise the rebakes of the edit's inputs, so
	# they install with the batch instead of rebaking. A tile neither held nor wanted installs
	# only where the preview shows it graded, so it draws on in the batch's frame; its patch's
	# list, refreshed with the edit's patch generations, then wants it. Any other tile waits for
	# that list, as a newly graded tile does.
	for tile: Dictionary in edit.get("staged", []):
		var key := Vector2i(int(tile["tile_x"]), int(tile["tile_z"]))
		var token := int(_book.stage_edit_tile(key, bool(tile["graded"]), serial))
		_staged_dirty(PackedInt32Array([key.x, key.y]) if token >= 0 else PackedInt32Array())
		if token < 0:
			continue
		tile["token"] = token
		tile["preview_epoch"] = int(edit.get("preview_epoch", -1))
		if bool(tile["behind"]):
			_book.variant_missing(key)
		_edit_staged[key] = tile
	_edit_release = max(_edit_release, int(edit.get("preview_epoch", -1)))

# A dirty tile takes the edit's serial as its token, so a bake in flight with an older one is
# dropped and the tile is requested again, a refused one too; an installed tile joins the edit
# batch.
func _dirty_tile(tile: Vector2i, serial: int) -> void:
	_book.dirty_tile(tile, serial)
	_staged_dirty(PackedInt32Array([tile.x, tile.y]))

# The book dirtied `tiles` (flat x, z; their requests cancelled): a staged result of one is
# dropped, its level kept for the rebake.
func _staged_dirty(tiles: PackedInt32Array) -> void:
	_apply_effects()
	for index in range(0, tiles.size(), 2):
		var tile := Vector2i(tiles[index], tiles[index + 1])
		if _edit_staged.has(tile):
			var staged: Dictionary = _edit_staged[tile]
			_book.set_edit_level(tile, int(staged["level"]), int(staged["step"]))
			_edit_staged.erase(tile)

# A resident patch entry was added, rewritten or removed: its counted tiles follow its residency,
# its list is asked again if its generation moved, and its cells follow its LOD step.
func _patch_changed(key: Vector2i, resident_patches: Dictionary) -> void:
	var present := resident_patches.has(key)
	# A drawing set whose patch now has another node hides; it draws again once the book says.
	var node_moved: bool = present and _book.cells_node_moved(key, (resident_patches[key]["node"] as Object).get_instance_id())
	# The book: the cells across its sides follow its step; its counted tiles follow its residency
	# and its blocks' parked tiles release; a patch that left evicts the commit-only tiles no other
	# resident patch meets, a resident one's installed tiles want copies in its blocks' pages; a
	# resident patch's set follows it (built, rewritten in this frame if drawing, or again off the
	# frame) and draws what its draw state says: nothing until its list arrives.
	_book.patch_changed(key, present, resident_patches[key]["rect"] if present else Rect2(), node_moved)
	_apply_effects()
	if not present:
		_lists_wanted.erase(key)
		return
	_lists_wanted[key] = true

# Applies the next queued count: a tile enters the desired set with the first list naming it and
# leaves with the last.
func _apply_count() -> void:
	_book.apply_count()
	_apply_effects()

# The camera moved: Rust re-levels the tiles due (its travel buckets, T26) and returns those whose
# level changed, which want a bake at it. The road preview stages its tiles at the same rule's
# levels, without the hysteresis (T42). A still camera costs nothing; a move O(tiles due + buckets
# passed) in Rust and O(changed tiles) here.
func _move_camera(camera_position: Vector3) -> void:
	_frame_relevelled = 0
	if _camera_set and camera_position == _camera:
		return
	_sim_view(camera_position)
	_camera = camera_position
	_camera_set = true
	_frame_relevelled = _book.move_camera(camera_position)

## The tiles that want a bake and have none started: wanted, parked on a page, or pending in
## Rust's queue (as of the last poll).
func wanted_count() -> int:
	return _book.wanted_count() + _tiles_pending

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
	var accepted: PackedByteArray = _sim_request_lists(flat)
	for index in accepted.size():
		if accepted[index] == 0:
			var key := Vector2i(flat[index * 3], flat[index * 3 + 1])
			patch_lists_requested.erase(key)
			_lists_wanted[key] = true

# The hand-over (the book's, `RENDER-32-I`): every wanted tile that is not installed at its level
# and token, not requested or refused at that token and not blocked on a page goes to Rust's
# queue, which starts the bakes nearest the camera first as its in-flight places free (TERRAIN-13
# part 1); a complete tile's stale pending request is cancelled (the effects, whose answers go
# back). A newer request replaces the tile's pending one, never baked. A tile blocked on a page
# waits for the event that unblocks it: its block's move, a freed slot of its exhausted level or
# its block's lists. O(wanted tiles) in Rust, each once per change; two calls here.
func _request_tiles() -> void:
	_frame_handed += int(_book.request_tiles())
	_frame_considered += _book.considered()
	if trace == null:
		# One call: Rust's queue takes the cancels and requests from the book and answers it.
		simulation_node.hand_over_graded_tiles(_book)
		_apply_effects()
		return
	# Traced: the same hand-over from flat arrays, its answers recorded op by op where the
	# renderer made them (each stale cancel marked among the effects, then the requests).
	var stale: PackedInt32Array = _book.hand_over_stale()
	var requests: PackedInt64Array = _book.hand_over_requests()
	var cancels := stale.size() / 2
	_hand_over = PackedInt64Array()
	if not trace.replaying and not (stale.is_empty() and requests.is_empty()):
		_hand_over = simulation_node.hand_over_graded_ops(stale, requests)
	_hand_over.resize(cancels + requests.size() / 5)
	_hand_over_at = 0
	_apply_effects()
	if not requests.is_empty():
		# 0: not queued (an unknown level); 1: queued; 2 + k: queued in place of the tile's
		# pending request at LEVEL_STEPS[k], which Rust drops unbaked.
		trace.out("request_tiles " + GradedTrace.ints(requests))
		var accepted := GradedTrace.ints_of(trace.value("request_tiles", GradedTrace.ints(_hand_over.slice(cancels))))
		for index in mini(accepted.size(), _hand_over.size() - cancels):
			_hand_over[cancels + index] = accepted[index]
	_book.hand_over_results(_hand_over)

# Builds patch `key`'s set whole in this frame: a drawing set whose patch changed step (the
# camera moved), and the tests' reference for the per-tile updates. O(the patch's tiles).
func _rebuild_patch_cells(key: Vector2i, _resident_patches: Dictionary) -> void:
	_book.rebuild_set(key)
	_apply_effects()

# Builds the queued sets off the frame paths: the patches a shown tile waits for first, at least
# one tile of the first of them in every frame, then the rest while the frame's install budget
# lasts. A set first syncs its materials (the book, from the patch's material), then writes its tiles; a built set starts drawing if a
# tile of it shows graded. O(tiles written this frame + uniforms synced).
func _advance_builds(start_us: int, resident_patches: Dictionary) -> void:
	_add_wanted_variants(start_us)
	var minimum := 1
	while true:
		var next: Vector3i = _book.next_build()
		if next.z < 0:
			return
		var key := Vector2i(next.x, next.y)
		if next.z == 0:
			minimum = 0
		if minimum == 0 and _over_budget(start_us):
			return
		if not resident_patches.has(key):
			_book.build_finished(key, false)
			continue
		var set := _book.begin_build(key)
		_apply_effects()
		var done := _book.set_built(set)
		if not done and _book.sync_build(set, start_us):
			done = _book.build_tiles(set, start_us, minimum)
			_apply_effects()
		minimum = 0
		if not done:
			return
		_book.build_finished(key, true)

# The sets a flagged tile waits for take the variant, first in the build path: the first of them
# in every frame, then the rest while the frame's install budget lasts. O(sets given it x the
# variant's meshes).
func _add_wanted_variants(start_us: int) -> void:
	var added := 0
	while _book.has_variants_wanted():
		if added > 0 and _over_budget(start_us):
			return
		if _book.add_next_variant() == 1:
			_apply_effects()
			added += 1

# The flagged tiles whose sets all hold the variant now: a bake result installs, a preview tile
# shows (where its preview still wants it). O(waiting tiles).
func _take_variant_waits(resident_patches: Dictionary) -> void:
	for key: Vector2i in _variant_waits.keys():
		if not _book.variant_missing(key):
			var tile: Dictionary = _variant_waits[key]
			_variant_waits.erase(key)
			_install(tile, resident_patches)
	for key: Vector2i in _preview_waits.keys():
		if not _book.variant_missing(key):
			_show_preview_tile(_preview_waits[key])

# Whether `tile` cannot show at its desired level and token: Rust refused it by contract (T7) or
# its level's pool is exhausted (T23). Its patch's set draws without it (a base cell; both are
# counted and logged errors).
func _refused_at_level(tile: Vector2i) -> bool:
	return _book.refused_at_level(tile)

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

## Cell set builds still queued (built off the frame paths: a world's sets after its load, or
## hidden sets whose patch took a new step). They take the streaming budget's spare time each
## frame, so a benchmark of the idle frame waits for none to be left. O(1).
func cell_builds_pending() -> int:
	return _book.builds_queued()

## Whether queued work will change what the graded cells draw without a new input: tile or list
## requests pending or in flight in Rust, tiles parked on a block's page, counts, results deferred past a
## frame's budget, an edit batch waiting for its tiles, a resident patch drawing nothing (T53
## (a)), and the cell set builds of patches a shown tile waits for or whose list names a tile (a
## set whose patch lists none stays hidden when built, so its build changes nothing drawn).
## terrain.gd's is_render_settled includes it: a capture of the settled terrain waits on that one
## signal, and so does a world load's hand-over (`TERRAIN-07-G` round 6: the reference test's own
## copy of it left out the set builds). O(queued builds).
func has_pending_work() -> bool:
	return _book.has_pending_work() or _book.cells_pending() or not (
		patch_lists_requested.is_empty() and _lists_wanted.is_empty() and _deferred.is_empty()
		and _unready.is_empty() and _variant_waits.is_empty() and _preview_waits.is_empty()
	)

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
		if drawable or _book.previews_in(_resident[key]["rect"]):
			violations += 1
	return violations

## Drawing sets whose list names a tile they do not show (not installed, not previewed and not
## refused): a partial set. For tests of a streaming run without edits (an edit's refreshed list
## may name a tile its edit did not select, drawn as the base it was until it installs,
## the book's draw state): always 0 there. O(drawing sets' lists).
func partial_sets() -> int:
	return _book.partial_sets()

## Tiles of drawing sets that draw other than what they show (`TERRAIN-12`): per drawing set of a
## resident patch, each of its tiles must be exactly one live instance, in the cell mesh of what
## it shows there now (its base cell, or the graded level of its installed or preview tile,
## _cell_mesh), whose custom data in the MultiMesh itself is _cell_custom's now, and a graded
## instance's slot must hold that tile's copy in the page the set's material binds. Returned as
## {holes: tiles drawn from the wrong mesh, a vacated slot or an unbound page; stitches: tiles
## with only a stale side code}. For the probe and tests, every frame of a drag; O(drawing sets'
## tiles).
func tile_draw_violations() -> Dictionary:
	var state: Vector2i = _book.draw_violations()
	var out := {"holes": state.x, "stitches": state.y}
	var rendered := DisplayServer.get_name() != "headless"
	for key: Vector2i in _resident.keys():
		var set := _book.patch_set(key)
		if set < 0 or not _book.set_built(set):
			continue
		var cells: Dictionary = _book.cell_objects(set)
		if not bool(cells["shown"]):
			continue
		var counts: PackedInt32Array = _book.set_counts(set)
		var sent: PackedInt32Array = cells["counts"]
		var has: PackedByteArray = cells["has"]
		for index in mini(counts.size(), sent.size()):
			if sent[index] != counts[index] or (has[index] == 1) != (counts[index] > 0):
				out["holes"] += 1
		if not rendered:
			continue
		# The MultiMesh itself where a rendering device keeps its data (headless keeps none): each
		# instance's custom data is the book's record of it.
		var multimeshes: Array = cells["multimeshes"]
		var instances: PackedFloat32Array = _book.set_instances(set)
		for at in range(0, instances.size(), 6):
			var custom := Color(instances[at + 2], instances[at + 3], instances[at + 4], instances[at + 5])
			if RenderingServer.multimesh_instance_get_custom_data(multimeshes[int(instances[at])], int(instances[at + 1])) != custom:
				out["holes"] += 1
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
		if not _book.cells_drawn_under(key, node.get_instance_id()):
			holes += 1
	return holes

## Returns a patch to its PlaneMesh and its cell resources to the pool. Called when the patch
## recycles (leaves the residency or the renderer resets).
func release_patch(key: Vector2i, patch_node: GeometryInstance3D) -> void:
	if trace != null:
		trace.entry("release_patch %d %d %d" % [key.x, key.y, trace.object_id(patch_node)])
	# The book hides its set, takes it out of the patch node and returns it to the pool.
	_book.release_set(key)
	_unready.erase(key)
	_set_plane(key, patch_node, true)

## Whether the cells draw at all (the terrain node's visibility; they are not its children).
func set_drawn(drawn: bool) -> void:
	if _book != null:
		_book.cells_set_drawn(drawn)

## Patch `key`'s cell meshes for grass's mask mirrors (GradedTileBook.mask_sources): per mesh its
## MultiMesh's RID id, whether it has instances and the page its material binds; empty without a
## set. O(the set's meshes).
func mask_sources(key: Vector2i) -> PackedInt64Array:
	return _book.mask_sources(key) if _book != null else PackedInt64Array()

## Resident patch `key`'s rectangle (empty when it is not resident).
func patch_rect(key: Vector2i) -> Rect2:
	return _resident.get(key, {}).get("rect", Rect2())

## Patch `key`'s cell set's grass mask revision (0 without a set): it moves whenever what the set
## draws changes, and grass renders the patch's mask again then. O(1).
func mask_revision(key: Vector2i) -> int:
	return _book.mask_revision(key)

## Page sizes, use and creation counters, for tests and the memory record: per level code its
## pages (`pages_N`), their slots and free slots (`slots_N`, `free_N`, the pages its blocks bind
## or move to), their atlas layers and free layers; `pool_bytes`, every page's bytes (the free
## pooled ones included), and the layer limit and block side.
func get_pool_counters() -> Dictionary:
	var out := counters.duplicate()
	var book := _book.pool_counters()
	for name: String in book:
		out[name] = book[name]
	out["refused"] = int(book["refused"]) + patch_lists_refused.size()
	out["desired_by_level"] = Array(book["desired_by_level"])
	out["layer_limit"] = layer_limit
	out["block_patches"] = block_patches
	var with_cells := 0
	for key: Vector2i in _resident.keys():
		with_cells += int(_book.patch_set(key) >= 0)
	out["patches_with_cells"] = with_cells
	out["cell_builds_queued"] = _book.builds_queued()
	out["cell_sets_free"] = int(counters["cell_sets_prewarmed"]) - with_cells
	return out

# The view `installed` and `preview` give tests and the probe: the book's entries, read whole once
# after each change (_apply_effects drops the views).
func _entries(is_preview: bool) -> Dictionary:
	if _views[int(is_preview)] != null:
		return _views[int(is_preview)]
	var out := {}
	if _book == null:
		return out
	var tiles: PackedInt32Array = _book.entry_tiles(is_preview)
	for index in range(0, tiles.size(), 2):
		var tile := Vector2i(tiles[index], tiles[index + 1])
		var fields: PackedInt64Array = _book.entry(tile, is_preview)
		var copies := {}
		for at in range(6, fields.size(), 4):
			copies[int(fields[at])] = Vector3i(int(fields[at + 1]), int(fields[at + 2]), int(fields[at + 3]))
		out[tile] = {"code": int(fields[0]), "level": int(fields[1]), "step": int(fields[2]), "token": int(fields[3]),
			"walls": _book.entry_walls(tile, is_preview), "behind": fields[4] == 1, "copies": copies}
	_views[int(is_preview)] = out
	return out
