# SPDX-License-Identifier: GPL-2.0-only

## Graded terrain tiles (TERRAIN-07-D, docs/terrain.md "Godot tile renderer"): the executor of the
## graded book. The graded 64 m tiles of the resident patches by camera distance, their pages
## (docs/terrain.md "Pages and the row atlas"), the uploads, per patch one MultiMesh per cell mesh
## (the base cells and GL0-GL3) and a pooled wall MultiMesh, the edits' batches and the road
## preview's tiles (TERRAIN-07-E) are Rust's: GradedTileBook (TERRAIN-13, `RENDER-32-B` to `-K`)
## holds the state, runs the frame (the camera's re-level, the edit batch, the result path, the
## changed patches, the cell set builds, the lists and the hand-over to the bake queue, the cell
## and wall rewrites; `RENDER-32-K` part 7) and the preview frames, makes the cells' objects
## through the stream backend, applies the draw states to the patch nodes and keeps every counter
## and the readiness latency. The resident patches are the streaming core's (part 6); terrain.gd
## lends each one's node, material and culling box (set_patch_node). It does no geometry:
## positions, stitching, clamping and the mask are terrain_cells.gdshaderinc's, the values Rust's.
## Nothing is created per edit; the counters in get_pool_counters() prove it.
##
## What stays here, and why: the Godot objects the book draws with that it does not make (the
## lattice and wall meshes, the wall material, the shaders' uniform list, the page textures
## through terrain_graded_pages.gd); _apply_effects, which applies the effects that need them
## (page arrays, cancels through the simulation) and hands the rest back; the record and replay
## wrappers around every simulation call (`_sim_*`, the traced hand-over), which the book calls
## back while the traces are written in GDScript (terrain_graded_trace.gd); and one-line readers
## for terrain.gd, the tests and the probe.
##
## Rust methods called: get_graded_tile_layout(), request_graded_patch_tiles(),
##   hand_over_graded_tiles(), cancel_graded_tile(), cancel_graded_tile_requests(),
##   poll_graded_tile_results() (traced), set_road_preview_terrain_view(); GradedTileBook
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
# Patch cells: a patch meets at most (ceil(span / 64 m) + 1)^2 tiles (9 x 9 on a 510 m patch),
# the instances of each cell MultiMesh. A tile's place in its patch is column + 32 * row in one
# instance channel (terrain_cells.gdshaderinc; the book writes it).
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
# Floats per wall face in a tile's `walls` (the book grows a patch's wall MultiMesh as it needs).
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
# The pages' textures (terrain_graded_pages.gd): each level's layer shapes and every page by id.
var page_store := GradedPages.new()
# The device's array layer limit (L) and the block side (B) in patches at GL2 and GL3.
var layer_limit := GradedPages.MIN_LAYER_LIMIT
var block_patches := 1
# The graded state, frame and counters in Rust (GradedTileBook): each call's effects that need
# this script are applied in order (_apply_effects).
var _book: GradedTileBook = null
# A traced hand-over's answers (per stale cancel, then per request) and the next cancel's.
var _hand_over := PackedInt64Array()
var _hand_over_at := 0
# The level rule's ends the road preview's view takes (GradedTileBook's, read once).
var _level_end_m := PackedFloat64Array()
var _gl3_end_m := PackedFloat64Array()
var _meshes: Dictionary = {}
var _wall_mesh: ArrayMesh = null
var _wall_material: ShaderMaterial = null

## For tests and the probe: tile key -> {code, level, step, token, walls, behind, copies} of every
## installed tile (`copies` maps a page id to its copy there, Vector3i(slot, first atlas layer,
## layers)), the book's, built on each read. The renderer itself reads none.
var installed: Dictionary:
	get:
		return _book.entries_view(false) if _book != null else {}
## For tests and the probe: the road preview's tiles (TERRAIN-07-E) as `installed` gives them
## (the token is the preview epoch), drawn in place of the installed tile from copies of their own.
var preview: Dictionary:
	get:
		return _book.entries_view(true) if _book != null else {}
## The tiles the last edit batch installed, for tests and the probe (the book's).
var last_edit_batch: Array:
	get:
		return _book.last_edit_batch_tiles()
## For tests: patch key -> its cell set as the book holds it (`GradedTileBook.cell_objects`, with
## `set`, `variant` and `rect`), built on each read. Every resident patch gets its set (prewarmed
## in setup, one per patch of the grid), kept current by the per-tile updates and hidden until one
## of its tiles shows graded.
var patch_cells: Dictionary:
	get:
		return _book.patch_cells_view() if _book != null else {}
## The renderer's counters and the pools' (get_pool_counters), the book's, built on each read.
var counters: Dictionary:
	get:
		return _book.counters()
# The last mask revision the book gave a set (grass mirrors a set's cells by it: mask_revision()).
var _cell_mask_revision: int:
	get:
		return _book.mask_revision_last() if _book != null else 0
## The view's frustum (terrain.gd sets it each frame from its camera, Camera3D.get_frustum()): an
## unready patch whose culling box meets it counts in `unready_in_view` (empty: none is counted).
var view_planes: Array[Plane] = []:
	set(value):
		view_planes = value
		if _book != null:
			_book.set_view_planes(value)
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
	_level_end_m = GradedTileBook.level_end_m()
	_gl3_end_m = GradedTileBook.gl3_end_m()
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
	var grid := Vector2i(int(ceil(world_size.x / span_m)), int(ceil(world_size.y / span_m)))
	# The book drops every page and makes the world pages of GL0 and GL1, sized by their bands'
	# bound and the most rows a tile can hold (at most the layer limit), so they never fill.
	_book.setup(world_min, patch_span_m, grid, block_patches, layer_limit, shapes)
	_book.set_markers(trace != null)
	_book.set_settle(unbudgeted, residency_settling)
	_book.set_view_planes(view_planes)
	_apply_effects()
	_build_meshes()
	_wall_material = ShaderMaterial.new()
	_wall_material.shader = WALL_SHADER
	# One cell set per patch of the world's grid, the most patches that can draw cells at once
	# (every resident patch may hold a graded tile), so neither streaming nor an edit creates one.
	var tiles_per_side := int(ceil(span_m / TILE_M)) + 1
	var prewarm_us := Time.get_ticks_usec()
	# The book's records and CPU buffers of the sets, by index, all free, and their objects.
	_book.cells_setup(tiles_per_side * tiles_per_side, grid.x * grid.y)
	_attach_cells()
	_book.note_prewarm(Time.get_ticks_usec() - prewarm_us)

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

# The tiles a block of `patches` x `patches` patches meets at most: T(B).
func _block_tiles(patches: int) -> int:
	var side := int(ceil(float(patches) * patch_span_m / TILE_M)) + 1
	return side * side

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
	if _book != null:
		# Every copy freed (block pages go with their last; the moves end with them), and the
		# book's requests, refusals, tokens, lists, counts and desires dropped; every set back in
		# the pool, every collapsed PlaneMesh drawn again.
		_book.clear()
		_apply_effects()
		for key: Vector2i in _book.patch_node_keys():
			if _book.patch_set(key) >= 0:
				release_patch(key, null)
		_book.restore_planes()
		_book.clear_cells()
	if is_instance_valid(simulation_node) or (trace != null and trace.replaying):
		_sim_cancel_all()

## Frees the cell sets' objects. Renderer replacement and shutdown call it before dropping the
## renderer; clear() first returns every set to the pool.
func dispose() -> void:
	clear()
	if _book != null:
		_book.cells_detach()

## One frame (the book's, GradedTileBook.frame): polls finished work and installs it within the
## install budget, keeps the tiles of the resident patches at their distance's level, evicts the
## rest and rewrites the instances of the patches that changed. The resident patches are the
## book's, set by the streaming core as the terrain's patches come, change step or commit and
## leave (`RENDER-32-K` part 6); the patches it changed are the frame's news. `camera_position` is
## the camera's XZ with, as `y`, its height above the ground under it: a tile's distance is the
## camera's to the tile's centre on that ground. `changed_materials` holds the keys whose material
## parameters may have changed (the caller clears it). O(changes) per frame.
func process(camera_position: Vector3, changed_materials: Dictionary) -> void:
	var news: PackedInt32Array = _book.take_patch_news() if _book.has_patch_news() else PackedInt32Array()
	if trace != null:
		trace.entry("frame %s %d %d %s %s" % [GradedTrace.floats(PackedFloat32Array([camera_position.x, camera_position.y, camera_position.z])),
			int(unbudgeted), int(residency_settling), GradedTrace.ints(news), GradedTrace.keys(changed_materials)])
	_run_frame(camera_position, news, changed_materials)

# The frame itself, the changed resident patches given flat (x, z) (process; a replay of a trace
# recorded before the core's hand-off gives its recorded keys).
func _run_frame(camera_position: Vector3, news: PackedInt32Array, changed_materials: Dictionary) -> void:
	var simulation: SimulationNode = simulation_node as SimulationNode if trace == null else null
	GradedTileBook.frame(_book, self, simulation, camera_position, news, changed_materials, PerfDebug.is_enabled())

## Patch `key`'s Godot objects (terrain.gd, as its node draws the patch): its node, material and
## culling box; a null node: none (the patch left or has no payload). The cell sets draw with
## them; a trace records them. O(1).
func set_patch_node(key: Vector2i, node: GeometryInstance3D, material: Material, aabb: AABB) -> void:
	var id := node.get_instance_id() if node != null else 0
	if not _book.lend_patch_node(key, id, material.get_rid() if node != null else RID(), aabb):
		return
	if trace == null or trace.replaying:
		return
	if node == null:
		trace.entry("patch_node_erase %d %d" % [key.x, key.y])
		return
	trace.entry("patch_node %d %d %d %d %s" % [key.x, key.y, id, material.get_instance_id(),
		GradedTrace.floats(PackedFloat32Array([aabb.position.x, aabb.position.y, aabb.position.z, aabb.size.x, aabb.size.y, aabb.size.z]))])

## Has the streaming core hand the terrain's resident entries to this renderer's book.
func attach_stream(stream: Node) -> void:
	stream.attach_graded_book(_book)

## The resident patches' keys, in the order they became resident (terrain.gd's overlay sync).
func resident_keys() -> Array:
	return _book.patch_node_keys()

## Forgets every resident patch (terrain.gd cleared its patches; the core starts over with the
## world). A trace records it as `patches_clear`.
func forget_patches() -> void:
	if trace != null and not trace.replaying:
		trace.entry("patches_clear")
	if _book != null:
		_book.forget_patches()

## Returns a patch to its PlaneMesh and its cell resources to the pool. Called when the patch
## recycles (leaves the residency or the renderer resets).
func release_patch(key: Vector2i, patch_node: GeometryInstance3D) -> void:
	if trace != null:
		trace.entry("release_patch %d %d %d" % [key.x, key.y, trace.object_id(patch_node)])
	_book.release_patch_node(key, patch_node.get_instance_id() if patch_node != null else 0)

## Shows one road preview frame's graded tiles (TERRAIN-07-E, docs/terrain.md "Preview, commit,
## undo and load"): `changes` holds `graded_tiles` (packed as poll_graded_tile_results packs its
## tiles, the token the preview epoch) and `graded_removed` (flat (x, z) pairs), as preview_road
## and advance_road_preview_terrain return them. A shown tile takes a free slot of its level's
## pool and draws in place of the installed tile from this frame; a removed one returns to the
## installed tile. Returns the milliseconds it took. O(changed tiles x slot bytes).
func show_preview(changes: Dictionary) -> float:
	if trace != null:
		trace.entry(GradedTrace.preview(changes))
	return GradedTileBook.preview_changes(_book, self, changes.get("graded_removed", PackedInt32Array()), changes.get("graded_tiles", []))

## Drops the preview tiles of preview epochs `first_epoch` to `last_epoch`: the installed tiles
## draw again. A cancelled preview drops its own epoch; an edit batch drops every epoch up to the
## one its commit adopted.
func release_preview(first_epoch: int, last_epoch: int) -> void:
	if trace != null:
		trace.entry("release_preview %d %d" % [first_epoch, last_epoch])
	GradedTileBook.release_preview_frame(_book, self, first_epoch, last_epoch)

# Applies the book's effects of the last calls, in order: page arrays (made, reused, released,
# dropped, copied, refused), the runs of slot uploads the book writes from the result being
# placed (48), Rust queue cancels, the runs of cell effects the book applies to its objects (47)
# and the draw states (40, the book's on the patch nodes). The book counts every effect as it
# hands it over.
func _apply_effects() -> void:
	var effects: PackedInt64Array = _book.take_effects()
	for at in range(0, effects.size(), 7):
		var kind := int(effects[at])
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
			48:
				_book.apply_writes(effects[at + 1], effects[at + 2])
			12:
				var tile := Vector2i(int(effects[at + 1]), int(effects[at + 2]))
				if int(effects[at + 3]) == 1:
					# A traced hand-over's stale cancel, recorded in its place (_request_tiles).
					trace.out("cancel %d %d" % [tile.x, tile.y])
					_hand_over[_hand_over_at] = int(trace.value("cancel", str(_hand_over[_hand_over_at])))
					_hand_over_at += 1
				else:
					_sim_cancel(tile)
			47:
				_book.apply_cells(effects[at + 1], effects[at + 2])
			40:
				_book.apply_draw(Vector2i(int(effects[at + 1]), int(effects[at + 2])), int(effects[at + 3]))

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

# The camera moved (the book's frame calls it before it re-levels): the road preview stages its
# tiles at the same rule's levels, without the hysteresis (T42).
func _sim_view(camera_position: Vector3) -> void:
	if trace != null:
		trace.out("view " + GradedTrace.floats(PackedFloat32Array([camera_position.x, camera_position.y, camera_position.z])))
		if trace.replaying:
			return
	simulation_node.set_road_preview_terrain_view(camera_position, _level_end_m, _gl3_end_m)

# Asks for the lists of the wanted patches whose list is not of their generation, not in flight
# and not refused at it, within Rust's in-flight bound (the book's; its frame calls it); the rest
# stay wanted.
func _request_patch_lists() -> void:
	var flat: PackedInt64Array = _book.lists_to_request()
	if not flat.is_empty():
		_book.lists_accepted(flat, _sim_request_lists(flat))

# The hand-over (the book's, `RENDER-32-I`; its frame calls it): every wanted tile that is not
# installed at its level and token, not requested or refused at that token and not blocked on a
# page goes to Rust's queue, which starts the bakes nearest the camera first as its in-flight
# places free (TERRAIN-13 part 1); a complete tile's stale pending request is cancelled (the
# effects, whose answers go back). A newer request replaces the tile's pending one, never baked.
# A tile blocked on a page waits for the event that unblocks it: its block's move, a freed slot
# of its exhausted level or its block's lists. O(wanted tiles) in Rust, each once per change.
func _request_tiles() -> void:
	_book.request_tiles()
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

# --- Readers: terrain.gd, the perf channel, tests and the probe (the book's). ---

## The serial of the last graded edit whose tiles draw (docs/terrain.md T61 (2)): the road tool
## holds an install's replaced road chunks until this reaches the journal's serial it read then.
## O(1).
func installed_edit_serial() -> int:
	return _book.installed_edit_serial()

## The tiles that want a bake and have none started: wanted, parked on a page, or pending in
## Rust's queue (as of the last poll).
func wanted_count() -> int:
	return _book.wanted_total()

## Cell set builds still queued (built off the frame paths: a world's sets after its load, or
## hidden sets whose patch took a new step). They take the streaming budget's spare time each
## frame, so a benchmark of the idle frame waits for none to be left. O(1).
func cell_builds_pending() -> int:
	return _book.builds_queued()

## Whether queued work will change what the graded cells draw without a new input
## (GradedTileBook.render_pending). terrain.gd's is_render_settled includes it: a capture of the
## settled terrain waits on that one signal, and so does a world load's hand-over (`TERRAIN-07-G`
## round 6). A set whose patch lists no tile stays hidden when built, so its queued build is not
## counted (cell_builds_pending counts it). O(queued builds).
func has_pending_work() -> bool:
	return _book.render_pending()

## The world's patches within `radius_m` of the ground point `centre` (XZ) that do not yet draw
## what they draw once settled (GradedTileBook.unready_near). O(patches within the radius).
func unready_near(centre: Vector2, radius_m: float) -> int:
	return _book.unready_near(centre, radius_m)

## Resident patches drawing their PlaneMesh where T53 (a) forbids it (tests: always 0).
func plane_violations() -> int:
	return _book.plane_violations()

## Drawing sets whose list names a tile they do not show, for tests of a streaming run without
## edits: always 0 there. O(drawing sets' lists).
func partial_sets() -> int:
	return _book.partial_sets()

## Tiles of drawing sets that draw other than what they show (`TERRAIN-12`), {holes, stitches}.
## For the probe and tests, every frame of a drag; O(drawing sets' tiles).
func tile_draw_violations() -> Dictionary:
	return _book.tile_draw_violations()

## Resident patches drawing nothing (a hole showing the sky). For tests and the probe.
func hole_patches() -> int:
	return _book.hole_patches()

## Resident patches holding a cell set. O(resident patches).
func patches_with_cells() -> int:
	return _book.patches_with_cells()

## The page sizes, use and every counter (GradedTileBook.counters), for tests, the probe and the
## memory record. O(levels).
func get_pool_counters() -> Dictionary:
	return _book.counters()

## The perf channel's parts of the last frame (terrain.gd reports them while the channel is on).
func frame_parts() -> Dictionary:
	return _book.frame_parts()

## Perf diagnostics: what the shown cell sets submit (before culling), instances and triangles per
## cell mesh, into `parts` as cells_<mesh> and cell_triangles_<mesh>. O(resident patches' cell
## meshes) a call; the perf channel calls it after its timed span, so it costs no renderer time.
func count_submitted_cells(parts: Dictionary) -> void:
	var submitted: PackedInt64Array = _book.submitted_cells()
	for index in CELL_MESHES:
		parts["cells_%d" % index] = submitted[index]
		parts["cell_triangles_%d" % index] = submitted[CELL_MESHES + index]

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
	return _book.patch_rect(key) if _book != null else Rect2()

## Patch `key`'s cell set's grass mask revision (0 without a set): it moves whenever what the set
## draws changes, and grass renders the patch's mask again then. O(1).
func mask_revision(key: Vector2i) -> int:
	return _book.mask_revision(key)
