# SPDX-License-Identifier: GPL-2.0-only

## Graded tile renderer (TERRAIN-07-D): residency requests and async bakes through the bridge,
## installs into the prewarmed slot pools within the install budget, the cell instances and
## their stitching codes, eviction, and a populated edit's reinstall creating no texture, image,
## material, mesh or MultiMesh (the pool counters): only the installed tiles Rust selects for the
## commit rebake, all in one frame, each into its own slot (item 5). Then the production renderer
## (terrain.gd) with graded tiles on settles with every patch installed, the road's included.
## In every frame no patch draws its PlaneMesh where its list names a graded tile or has not
## arrived, and while patches stream in no set draws with a listed tile missing (T53 (a)); a
## world load hands the game over only once the production renderer has settled.
## A rendered Forward+ run enables FSR 2 and checks the actual instance transforms and custom
## data after hidden builds, level moves and recycling (TERRAIN-10); headless keeps the CPU checks.
extends SceneTree

const GradedTiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const TerrainRenderer := preload("res://scripts/renderers/terrain.gd")
const InputManager := preload("res://scripts/core/input_manager.gd")
const GrassRenderer := preload("res://scripts/renderers/grass.gd")
const TIMEOUT_FRAMES := 900
const WORLD_M := 1020.0
const CELL_M := 10.0

var _failures := 0
# The resident patch keys changed since the renderer's last frame (terrain.gd's): every key of a
# rewritten or dropped resident dictionary.
var _changed := {}
# T53 (a), over every frame: the most patches drawing their PlaneMesh where their list names a
# graded tile or has not arrived, the most sets drawing with a listed tile missing (checked while
# `_check_partial`: an edit's refreshed list may name a tile its edit did not select), and the most
# patches drawing nothing (the window the rule covers, which a streaming run must exercise).
var _violations := 0
var _partial := 0
var _check_partial := true
var _unready_seen := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
	if DisplayServer.get_name() != "headless" and RenderingServer.get_current_rendering_method() == "forward_plus":
		root.scaling_3d_mode = Viewport.SCALING_3D_MODE_FSR2
		root.scaling_3d_scale = 0.5
		var render_camera := Camera3D.new()
		host.add_child(render_camera)
		render_camera.position = Vector3(0.0, 80.0, 80.0)
		render_camera.look_at(Vector3.ZERO)
		render_camera.make_current()
		# Activate motion-vector buffers before the cell sets are created.
		for frame in 3:
			await process_frame
	var simulation := SimulationNode.new()
	simulation.name = "SimulationNode"
	host.add_child(simulation)
	_expect(simulation.create_blank_world(WORLD_M, WORLD_M, CELL_M, 510.0, 0.0), "fixture world must be created")
	await _add_road(simulation, [Vector3(-120.0, 0.0, 10.0), Vector3(120.0, 0.0, 10.0)])
	await _add_road(simulation, [Vector3(0.0, 0.0, 10.0), Vector3(0.0, 0.0, 130.0)])

	var layout: Dictionary = simulation.get_terrain_patch_layout()
	var world_size: Vector2 = simulation.get_terrain_world_size()
	var graded := GradedTiles.new()
	graded.setup(simulation, CELL_M, world_size, float(layout["patch_interval_cells"]) * CELL_M, [24, 24, 24, 24])
	var created := _creation_counters(graded)
	_expect(int(created["textures_created"]) == 4, "the world pages of GL0 and GL1 x (slots, atlas): %s" % created)
	# A prewarmed set holds the five plain cell meshes and the walls' (round 10: the behind-the-road
	# variant's four come with a flagged tile, _check_variant).
	_expect(int(created["multimeshes_created"]) == (GradedTiles.CELL_PLAIN + 1) * 4, "one cell set per patch of the 2 x 2 grid: %s" % created)

	var resident := _resident_patches(host, layout, world_size, 1)
	_mark_changed(resident)
	var camera := Vector3(0.0, 40.0, 10.0)
	_expect(await _settle(graded, camera, resident), "the junction's tiles must install")
	_expect_streamed("the first residency")
	var counters: Dictionary = graded.get_pool_counters()
	_expect(int(counters["installed"]) > 0, "graded tiles must be installed: %s" % counters)
	_expect(int(counters["bake_failures"]) == 0 and int(counters["refused"]) == 0, "no bake may fail: %s" % counters)
	_expect(int(counters["pool_exhausted"]) == 0, "the pools must not run out: %s" % counters)
	# The tiles under the junction are graded and, 40 m above it, at GL0 (to 64 m).
	var junction := Vector2i(0, 0)
	_expect(graded.installed.has(junction), "the junction's tile must be graded")
	if graded.installed.has(junction):
		_expect(int(graded.installed[junction]["level"]) == 0, "the junction's tile must be at GL0")
	_expect(graded.patch_cells.size() == resident.size(), "every resident patch has its cell set built: %d of %d" % [graded.patch_cells.size(), resident.size()])
	_expect(graded.hole_patches() == 0, "every resident patch draws its PlaneMesh or its set")
	for key: Vector2i in graded.patch_cells.keys():
		_check_cells(graded, key, resident)
	# A pooled cell set binds no material, so it holds none of Godot's instance uniform slots.
	for cells: Dictionary in graded._cells_pool:
		for node: GeometryInstance3D in cells["nodes"]:
			_expect(node.material_override == null, "a pooled cell set binds no material")

	# A populated edit: another road beside the junction. Rust selects the commit's dirty tiles;
	# the installed ones among them rebake and install together in one frame, each into its own
	# slot, while the patch lists refresh with the patches' new generations.
	var before_edit: Dictionary = graded.installed.duplicate(true)
	_check_partial = false
	var uploads_before := int(graded.get_pool_counters()["uploads"])
	var mask_revision_before := graded._cell_mask_revision
	await _add_road(simulation, [Vector3(-120.0, 0.0, 70.0), Vector3(-10.0, 0.0, 70.0)])
	for key: Vector2i in resident.keys():
		resident[key]["generation"] = int(resident[key]["generation"]) + 1
	_mark_changed(resident)
	graded.counters["install_ms_max"] = 0.0
	_expect(await _settle_edit(graded, camera, resident), "the edit's tiles must install again")
	counters = graded.get_pool_counters()
	_expect(int(counters["uploads"]) > uploads_before, "the edit must upload its tiles again")
	_expect(int(counters["edits"]) >= 1 and int(counters["edit_batches"]) == 1, "one edit installs as one batch: %s" % counters)
	# Every tile installed before the commit that rebaked did so in the batch, into its own slot.
	# The batch may also hold tiles the patch lists' refresh installed after the commit, before the
	# edit's selection arrived (they rebake once more): the two race, so the batch is compared
	# with the tiles it installed, not with the tiles before the commit.
	var batch: Array = graded.last_edit_batch
	_expect(batch.size() > 0 and batch.size() == int(counters["edit_tiles_last"]), "the batch is its installed tiles: %s" % [batch])
	var rebaked := 0
	for tile: Vector2i in before_edit.keys():
		if not graded.installed.has(tile):
			continue
		var was: Dictionary = before_edit[tile]
		var now: Dictionary = graded.installed[tile]
		if int(now["token"]) == int(was["token"]):
			continue
		rebaked += 1
		_expect(batch.has(tile), "tile %s rebakes in the batch" % tile)
		if int(now["level"]) == int(was["level"]):
			for page_id: int in was["copies"]:
				if (now["copies"] as Dictionary).has(page_id):
					_expect((now["copies"][page_id] as Vector3i).x == (was["copies"][page_id] as Vector3i).x, "tile %s rebakes into its own slot" % tile)
	_expect(rebaked > 0, "the edit rebakes installed tiles")
	_expect(graded._cell_mask_revision > mask_revision_before,
		"an in-place tile upload must invalidate the grass mask")
	_expect(batch.size() < graded.installed.size(), "the edit rebakes only its dirty tiles (%d of %d installed)" % [batch.size(), graded.installed.size()])
	# Nothing is created after setup but a block's pages (as its streaming first needs them) and a
	# set's behind-the-road variant (its first flagged tile, in the build path), neither by the
	# edit: the cell sets are prewarmed, one per patch of the grid (round 5's populated edit
	# created 16 materials and 24 MultiMeshes for patches newly drawing cells).
	var after := _creation_counters(graded)
	var variants := int(graded.counters["cell_variants_created"])
	for name: String in created.keys():
		if name == "textures_created":
			_expect(int(after[name]) - int(created[name]) == 2 * int(graded.counters["pages_created"]) - 4, "only pages create textures: %s" % after)
			continue
		var variant_made := 4 * variants if name in ["materials_created", "multimeshes_created"] else 0
		_expect(int(after[name]) - int(created[name]) == variant_made, "nothing may be created after setup (%s): %s -> %s" % [name, created[name], after[name]])
	# Each cell set holds five cell MultiMeshes, the wall MultiMesh and the graded levels' four
	# materials (every cell mesh but the base cells'), and a set given the variant four more of
	# each; the wall material is one, made at setup.
	_expect(int(after["materials_created"]) - 1 == 4 * (4 + variants) and int(after["multimeshes_created"]) == 4 * (GradedTiles.CELL_PLAIN + 1) + 4 * variants, "cell materials are pooled with their cells: %s" % after)
	print("graded tile install ms max %.3f (poll %.3f, upload %.3f), uploads %d, installed %d, pool bytes %d, edit %d tiles in %.3f ms" % [
		float(counters["install_ms_max"]), float(counters["poll_ms_max"]), float(counters["upload_ms_max"]),
		int(counters["uploads"]), int(counters["installed"]), int(counters["pool_bytes"]),
		int(counters["edit_tiles_last"]), float(counters["edit_install_ms_last"])])

	await _check_road_preview(graded, simulation, camera, resident)
	await _check_moving_camera(graded, resident)
	await _check_streaming(graded, camera, resident)
	await _check_page_growth(graded, camera, resident)
	await _check_gl3_step_change(graded, camera, resident)
	_expect(_violations == 0, "no frame draws a PlaneMesh where its patch's list names a graded tile: up to %d" % _violations)

	# Far away every tile is evicted and every slot freed.
	var far := _resident_patches(host, layout, world_size, 1)
	for key: Vector2i in far.keys():
		far[key]["generation"] = int(resident[key]["generation"])
	_mark_changed(resident)
	for frame in 4:
		_frame(graded, Vector3(0.0, 120.0, 10.0), {})
		await process_frame
	counters = graded.get_pool_counters()
	_expect(int(counters["installed"]) == 0, "no tile may stay installed without resident patches")
	for code in 4:
		_expect(int(counters["free_%d" % code]) == int(counters["slots_%d" % code]), "level %d must free its slots" % code)
	graded.clear()
	await _check_variant(simulation, host, layout, world_size)
	await _check_dispose_frees_every_set(simulation, host, layout, world_size)
	await _check_exhausted_pools(simulation, host, layout, world_size)
	await _check_refusals_are_asked_once(host, simulation, layout, world_size)
	await _check_production_renderer(host, simulation)
	host.queue_free()
	await process_frame
	print("Graded tile renderer tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# T26: the camera climbs away from the junction and back in 40 m steps. In every frame each
# desired tile has its distance's level with hysteresis around its installed level (the travel
# buckets re-level only the tiles due), and after the tiles settle at each end every patch's
# cells, moved per tile, hold what a rewrite of the whole patch writes.
func _check_moving_camera(graded: GradedTiles, resident: Dictionary) -> void:
	var levels_seen := {}
	var exact := true
	for target_height: float in [700.0, 40.0]:
		var camera := Vector3(0.0, 40.0 if target_height > 40.0 else 700.0, 10.0)
		while camera.y != target_height:
			camera.y = move_toward(camera.y, target_height, 40.0)
			_frame(graded, camera, resident)
			await process_frame
			for tile: Vector2i in graded._desired.keys():
				var installed: Dictionary = graded.installed.get(tile, {})
				var level: Vector2i = graded.desired_level(graded._tile_distance(tile), int(installed.get("level", -1)))
				if graded._desired[tile]["level"] != level:
					exact = false
		_expect(await _settle(graded, camera, resident), "the tiles settle at %.0f m" % target_height)
		for tile: Vector2i in graded.installed.keys():
			levels_seen[int(graded.installed[tile]["level"])] = true
		for key: Vector2i in graded.patch_cells.keys():
			var moved := _cell_record(graded.patch_cells[key])
			graded._rebuild_patch_cells(key, resident)
			_expect(moved == _cell_record(graded.patch_cells[key]), "patch %s: the per-tile cells equal the patch's rewrite" % key)
			_check_cells(graded, key, resident)
	_expect(exact, "every desired tile has its distance's level in every frame")
	_expect(levels_seen.has(0) and levels_seen.has(2), "the climb moves tiles between levels: %s" % levels_seen)

# T53 (a) in a streaming run: the patches leave the residency (terrain.gd releases them) and come
# back over the next frames, as a camera crossing them would bring them. Each draws nothing from
# its residency until its set shows every tile its list names, never its PlaneMesh over them and
# never a partial set; settled, every patch draws its set or its PlaneMesh.
func _check_streaming(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> void:
	_check_partial = true
	_unready_seen = 0
	var away := {}
	for key: Vector2i in resident.keys():
		graded.release_patch(key, resident[key]["node"])
		away[key] = resident[key]
		resident.erase(key)
		_changed[key] = true
	_frame(graded, camera, resident)
	await process_frame
	for key: Vector2i in away.keys():
		resident[key] = away[key]
		_changed[key] = true
		_frame(graded, camera, resident)
		await process_frame
	_expect(await _settle(graded, camera, resident), "the streamed patches settle")
	_expect_streamed("the streamed patches")
	for key: Vector2i in resident.keys():
		var listed := not (graded.patch_lists.get(key, {}).get("tiles", []) as Array).is_empty()
		var cells: Dictionary = graded.patch_cells.get(key, {})
		_expect(not listed or bool(cells.get("shown", false)), "streamed patch %s draws its set" % key)

# A block whose page cannot take a copy moves to a larger page (docs/terrain.md "Growth"): its
# tiles are asked again into the new page while its sets keep drawing the page they bind, then the
# sets bind the new page in one frame, every instance naming its slot there, and no frame shows a
# partial set or a PlaneMesh over listed tiles. Every block holding tiles moves here, the GL0 and
# GL1 world pages too.
func _check_page_growth(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> void:
	_check_partial = true
	var moves_before := int(graded.counters["page_moves"])
	var moving := {}
	for code in 4:
		for block: Vector2i in graded._block_page[code].keys():
			var page: Dictionary = graded.page_store.by_id[graded._block_page[code][block]]
			if (page["tiles"] as Dictionary).is_empty():
				continue
			_expect(graded._move_block(code, block, Vector2i(int(page["slots"]) * 2, int(page["layers"]))), "level %d block %s starts its move" % [code, block])
			moving[Vector3i(code, block.x, block.y)] = int(graded._block_target[code][block])
			# The new page hands out other slot numbers than the old (its slot 0 held back), so an
			# instance left naming its old slot shows.
			(graded.page_store.by_id[graded._block_target[code][block]]["free"] as Array).pop_back()
			# Until the move, a copy of the block's tiles goes into both pages: its sets keep
			# showing every installed tile from the page they bind.
			for tile: Vector2i in (page["tiles"] as Dictionary).keys():
				var pages: Array = graded._copy_pages(tile, code).map(func(pair: Array) -> int: return int(pair[0]))
				_expect(pages.has(int(page["id"])) and pages.has(int(graded._block_target[code][block])),
					"tile %s of moving block %s goes into both pages: %s" % [tile, block, pages])
	_expect(not moving.is_empty(), "some block holds tiles to move")
	# The camera climbs while the blocks move: tiles change level and install meanwhile, into the
	# pages the sets bind as well as the new ones, so no drawing set misses one (_partial).
	_expect(await _settle(graded, camera + Vector3(0.0, 260.0, 0.0), resident), "the moved blocks settle")
	_expect(await _settle(graded, camera, resident), "the tiles settle back")
	_expect(int(graded.counters["page_moves"]) - moves_before == moving.size(), "every block moved once: %d of %d" % [int(graded.counters["page_moves"]) - moves_before, moving.size()])
	for marked: Vector3i in moving.keys():
		var block := Vector2i(marked.y, marked.z)
		_expect(int(graded._block_page[marked.x].get(block, -1)) == int(moving[marked]), "level %d block %s binds its new page" % [marked.x, block])
	for tile: Vector2i in graded.installed.keys():
		_expect(graded._tile_complete(tile, graded.installed[tile]), "tile %s has a copy in every page that draws it" % tile)
	for key: Vector2i in resident.keys():
		_check_cells(graded, key, resident)
	_expect(_partial == 0 and _violations == 0, "no frame of the moves draws a partial set (%d) or a PlaneMesh over listed tiles (%d)" % [_partial, _violations])
	# The slots held back go back to their pages.
	for page_id: int in moving.values():
		if graded.page_store.by_id.has(page_id):
			(graded.page_store.by_id[page_id]["free"] as Array).append(0)

# A tile changing `GL3` step with the camera (2 to 5 km, then 4) keeps its token and level code but
# not its lattice: every one is rebaked and rewritten into its copies (round 8: a rebake of the
# same level and token skips the copies it holds, which must not include a step change).
func _check_gl3_step_change(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> void:
	_expect(await _settle(graded, Vector3(0.0, 3000.0, 10.0), resident), "the tiles settle at step 2")
	var stepped := {}
	for tile: Vector2i in graded.installed.keys():
		var entry: Dictionary = graded.installed[tile]
		if int(entry["level"]) == 3 and int(entry["step"]) == 2:
			stepped[tile] = int(entry["token"])
	_expect(not stepped.is_empty(), "some tile is at GL3 step 2")
	var uploads := int(graded.counters["uploads"])
	_expect(await _settle(graded, Vector3(0.0, 6000.0, 10.0), resident), "the tiles settle at step 4")
	var copies := 0
	for tile: Vector2i in stepped.keys():
		var entry: Dictionary = graded.installed.get(tile, {})
		_expect(not entry.is_empty() and int(entry["step"]) == 4 and int(entry["token"]) == int(stepped[tile]), "tile %s moves to step 4 with its token" % tile)
		copies += (entry.get("copies", {}) as Dictionary).size()
	_expect(int(graded.counters["uploads"]) - uploads >= copies, "every copy of a stepped tile is rewritten: %d uploads for %d copies" % [int(graded.counters["uploads"]) - uploads, copies])
	_expect(await _settle(graded, camera, resident), "the tiles settle back")

# A run from no residency to settled exercised T53 (a)'s window and kept its rule in every frame.
func _expect_streamed(label: String) -> void:
	_expect(_unready_seen > 0, "%s: patches draw nothing until they are ready" % label)
	_expect(_violations == 0, "%s: up to %d patches drew their PlaneMesh over listed graded tiles" % [label, _violations])
	_expect(_partial == 0, "%s: up to %d sets drew with a listed tile missing" % [label, _partial])

# Per cell mesh, the sorted custom data of its instances, with the visible counts.
func _cell_record(cells: Dictionary) -> Array:
	var out := []
	var counts: Array[int] = cells["counts"]
	for index in (cells["nodes"] as Array).size():
		var written: Array = []
		for data: Color in (cells["custom"][index] as Array).slice(0, counts[index]):
			written.append(str(data))
		written.sort()
		out.append([(cells["nodes"][index] as MultiMeshInstance3D).multimesh.visible_instance_count, written])
	return out

# A stand-in bridge that refuses every tile bake by contract (retry false), as Rust refused round
# 5's populated GL2 tiles: each patch lists tiles (0, 0) and (1, 0).
class RefusingBridge extends Node:
	var layout: Dictionary
	# Request key -> how many times it was asked.
	var asked: Dictionary = {}
	var _lists: Array = []
	var _failed: Array = []

	func get_graded_tile_layout() -> Dictionary:
		return layout

	func request_graded_patch_tiles(flat: PackedInt64Array) -> PackedByteArray:
		var accepted := PackedByteArray()
		for index in range(0, flat.size(), 3):
			_lists.append({"patch_x": flat[index], "patch_z": flat[index + 1], "token": flat[index + 2], "tiles": PackedInt32Array([0, 0, 1, 0])})
			accepted.append(1)
		return accepted

	func request_graded_tiles(flat: PackedInt64Array) -> PackedByteArray:
		var accepted := PackedByteArray()
		for index in range(0, flat.size(), 5):
			var keys := PackedInt64Array([flat[index], flat[index + 1], flat[index + 2], flat[index + 3]])
			var key := "%d,%d,%d,%d" % [keys[0], keys[1], keys[2], keys[3]]
			asked[key] = int(asked.get(key, 0)) + 1
			_failed.append({"kind": "tile", "keys": keys, "token": flat[index + 4], "reason": "edge_capacity", "retry": false})
			accepted.append(1)
		return accepted

	func set_road_preview_terrain_view(_camera: Vector3, _level_end_m: PackedFloat64Array, _gl3_end_m: PackedFloat64Array) -> void:
		pass

	func poll_graded_tile_results(_max_tiles: int) -> Dictionary:
		var out := {"edits": [], "patches": _lists, "tiles": [], "failed": _failed}
		_lists = []
		_failed = []
		return out

# The renderer with every bake result flagged behind the road from `flag_all` on (the fixture's
# flat roads flag none), as a hilly map's tiles are (T49 (c)).
class FlaggingTiles extends GradedTiles:
	var flag_all := false

	func _install(tile: Dictionary, resident_patches: Dictionary) -> void:
		if flag_all:
			tile["behind"] = true
		super(tile, resident_patches)

# `TERRAIN-07-G` round 10: a set holds the behind-the-road variant's meshes and materials only once
# its patch shows a flagged tile. Settled with no flag, every set holds the five plain cell meshes;
# then every rebake is flagged and the camera climbs, so drawing sets take flagged tiles: each
# such set gains the variant in the build path, the tiles waiting meanwhile, and in no frame does
# a drawing set show a flagged tile outside its variant's mesh (nor a partial set, a PlaneMesh
# over listed tiles or a hole). Settled, the per-tile cells equal a rewrite of the patch.
func _check_variant(simulation: SimulationNode, host: Node3D, layout: Dictionary, world_size: Vector2) -> void:
	var graded := FlaggingTiles.new()
	graded.setup(simulation, CELL_M, world_size, float(layout["patch_interval_cells"]) * CELL_M, [24, 24, 24, 24])
	var resident := _resident_patches(host, layout, world_size, 1)
	_mark_changed(resident)
	var camera := Vector3(0.0, 40.0, 10.0)
	_check_partial = true
	_expect(await _settle(graded, camera, resident), "the unflagged tiles settle")
	for key: Vector2i in graded.patch_cells.keys():
		_expect((graded.patch_cells[key]["nodes"] as Array).size() == GradedTiles.CELL_PLAIN, "patch %s with no flagged tile holds five cell meshes" % key)
		_check_cells(graded, key, resident)
	_expect(int(graded.counters["cell_variants_created"]) == 0, "no set holds the variant without a flagged tile")
	graded.flag_all = true
	var missing := 0
	var holes := 0
	for target_height: float in [300.0, 40.0]:
		var stable := 0
		for frame in TIMEOUT_FRAMES:
			camera.y = move_toward(camera.y, target_height, 40.0)
			_frame(graded, camera, resident)
			missing += _flagged_outside_variant(graded)
			holes = max(holes, graded.hole_patches())
			await process_frame
			stable = stable + 1 if camera.y == target_height and not graded.has_pending_work() else 0
			if stable >= 3:
				break
		_expect(stable >= 3, "the flagged tiles settle at %.0f m" % target_height)
	_expect(missing == 0, "no frame draws a flagged tile outside its set's variant: %d" % missing)
	_expect(holes == 0 and _violations == 0 and _partial == 0, "the variant's tiles change no patch's draw state: holes %d, planes %d, partial %d" % [holes, _violations, _partial])
	var flagged := 0
	for tile: Vector2i in graded.installed.keys():
		if bool(graded.installed[tile]["behind"]):
			flagged += 1
	_expect(flagged > 0 and int(graded.counters["cell_variants_created"]) > 0 and int(graded.counters["variant_waits"]) > 0,
		"flagged tiles waited for their sets' variant: %d flagged, %s" % [flagged, graded.counters])
	print("graded behind variant: %d flagged tiles, %d sets given the variant, %d waits" % [flagged, int(graded.counters["cell_variants_created"]), int(graded.counters["variant_waits"])])
	for key: Vector2i in graded.patch_cells.keys():
		var cells: Dictionary = graded.patch_cells[key]
		_expect(bool(cells["variant"]) == graded._patch_flagged(cells), "patch %s holds the variant as its flagged tiles need it" % key)
		var moved := _cell_record(cells)
		graded._rebuild_patch_cells(key, resident)
		_expect(moved == _cell_record(cells), "patch %s: the per-tile cells equal the patch's rewrite" % key)
		_check_cells(graded, key, resident)
	graded.dispose()

# Drawing sets showing a flagged tile outside the variant's mesh of its level.
func _flagged_outside_variant(graded: GradedTiles) -> int:
	var count := 0
	for key: Vector2i in graded.patch_cells.keys():
		var cells: Dictionary = graded.patch_cells[key]
		if not bool(cells["shown"]):
			continue
		for tile: Vector2i in cells["tiles"]:
			var entry: Dictionary = graded._shown_in(tile, key)
			if entry.is_empty() or not bool(entry["behind"]):
				continue
			var at: Vector2i = cells["where"][GradedTiles._cell_place(cells, tile)]
			if at.x != int(entry["level"]) + GradedTiles.CELL_BEHIND:
				count += 1
	return count

# A world's renderer is set up, loads, is replaced by the next load's and shut down, as terrain.gd
# does (dispose, then a new renderer): every cell set's nodes are freed, roots included, so the
# orphan node count returns to its value before the first setup (round 9C: each set's root
# leaked, one orphan per set per load).
func _check_dispose_frees_every_set(simulation: SimulationNode, host: Node3D, layout: Dictionary, world_size: Vector2) -> void:
	await process_frame
	var orphans := int(Performance.get_monitor(Performance.OBJECT_ORPHAN_NODE_COUNT))
	var camera := Vector3(0.0, 40.0, 10.0)
	var span := float(layout["patch_interval_cells"]) * CELL_M
	var resident := _resident_patches(host, layout, world_size, 1)
	for load in 2:
		var graded := GradedTiles.new()
		graded.setup(simulation, CELL_M, world_size, span, [24, 24, 24, 24])
		_expect(int(Performance.get_monitor(Performance.OBJECT_ORPHAN_NODE_COUNT)) > orphans, "the prewarmed sets are detached nodes")
		_mark_changed(resident)
		_expect(await _settle(graded, camera, resident), "load %d settles" % load)
		graded.dispose()
		# PREDELETE after dispose() finds no set left: dropping the renderer frees nothing twice.
		graded = null
	await process_frame
	var left := int(Performance.get_monitor(Performance.OBJECT_ORPHAN_NODE_COUNT)) - orphans
	_expect(left == 0, "setup, load, reload and shutdown leave no orphan node: %d" % left)

# A request Rust refuses by contract is asked once at its token and counted, never again every
# frame (round 5's populated save re-asked 827,447 refusals and never settled); an edit that
# dirties the tile asks it once more, at the edit's token.
func _check_refusals_are_asked_once(host: Node3D, simulation: SimulationNode, layout: Dictionary, world_size: Vector2) -> void:
	var bridge := RefusingBridge.new()
	bridge.layout = simulation.get_graded_tile_layout()
	host.add_child(bridge)
	var graded := GradedTiles.new()
	graded.setup(bridge, CELL_M, world_size, float(layout["patch_interval_cells"]) * CELL_M, [8, 8, 8, 8])
	var resident := _resident_patches(host, layout, world_size, 1)
	_mark_changed(resident)
	var camera := Vector3(0.0, 120.0, 10.0)
	for frame in 30:
		_frame(graded, camera, resident)
		await process_frame
	var once := bridge.asked.size() == 2
	for key: String in bridge.asked.keys():
		once = once and int(bridge.asked[key]) == 1
	_expect(once, "each refused tile is asked once: %s" % bridge.asked)
	_expect(int(graded.counters["bake_failures"]) == 2 and int(graded.get_pool_counters()["refused"]) == 2,
		"each refusal is counted once: %s" % graded.get_pool_counters())
	graded._apply_edit({"serial": 1, "all": true, "levels": []})
	for frame in 30:
		_frame(graded, camera, resident)
		await process_frame
	var twice := bridge.asked.size() == 2
	for key: String in bridge.asked.keys():
		twice = twice and int(bridge.asked[key]) == 2
	_expect(twice, "an edit asks each refused tile once more: %s" % bridge.asked)
	graded.clear()
	bridge.queue_free()

# An exhausted pool (T23): with one slot a level, the tiles that find no slot are recorded once,
# not baked again every frame, and the render settles with every patch drawing (a tile without a
# slot draws its base cell, a counted error); a freed slot asks one of them again
# (`TERRAIN-07-G` round 7: the 100k city's re-asked bakes never settled and starved every farther
# tile).
func _check_exhausted_pools(simulation: SimulationNode, host: Node3D, layout: Dictionary, world_size: Vector2) -> void:
	var graded := GradedTiles.new()
	graded.setup(simulation, CELL_M, world_size, float(layout["patch_interval_cells"]) * CELL_M, [1, 1, 1, 1])
	var resident := _resident_patches(host, layout, world_size, 1)
	_mark_changed(resident)
	var camera := Vector3(0.0, 40.0, 10.0)
	_expect(await _settle(graded, camera, resident), "the render settles with exhausted pools")
	var counters: Dictionary = graded.get_pool_counters()
	_expect(int(counters["exhausted"]) > 0, "tiles wait for a slot: %s" % counters)
	_expect(graded.hole_patches() == 0 and graded.plane_violations() == 0, "every patch draws its set or its PlaneMesh")
	var exhausted_events := int(counters["pool_exhausted"])
	var uploads := int(counters["uploads"])
	for frame in 30:
		_frame(graded, camera, resident)
		await process_frame
	counters = graded.get_pool_counters()
	_expect(int(counters["pool_exhausted"]) == exhausted_events and int(counters["uploads"]) == uploads,
		"an exhausted tile is not asked again while no slot frees: %s" % counters)
	# An installed tile of a level with waiting tiles leaves (as an eviction does): its freed slot
	# asks them again.
	var freed := Vector2i.ZERO
	var code := -1
	for tile: Vector2i in graded.installed.keys():
		for entry: Array in graded._exhausted.values():
			if int(entry[0]) == int(graded.installed[tile]["level"]):
				freed = tile
				code = int(entry[0])
	_expect(code >= 0, "a level holds an installed tile and waiting ones")
	if code >= 0:
		graded._undesire(freed)
		var asked := 0
		for tile: Vector2i in graded._exhausted.keys():
			if int(graded._exhausted[tile][0]) == code and bool(graded._exhausted[tile][3]):
				asked += 1
				_expect(graded._wanted.has(tile), "the tile asked again is wanted")
		_expect(asked == 1, "a freed slot asks one of its level's waiting tiles again: %d" % asked)
	graded.clear()
	for key: Vector2i in resident.keys():
		(resident[key]["node"] as Node).queue_free()

# The production renderer (terrain.gd, graded tiles on, as the game runs) over the same roads:
# every patch it makes resident installs its plain payload, the road's patches included (Rust
# refused those, so no patch under a road ever installed and the terrain never settled, round
# 4), and the junction's tile draws as a graded cell over its patch. Then the side road becomes a
# bridge over a hollow: its mouth's structure walls draw as wall instances, one per face.
func _check_production_renderer(host: Node3D, simulation: SimulationNode) -> void:
	var water := Node3D.new()
	water.name = "Water"
	host.add_child(water)
	var camera := Camera3D.new()
	host.add_child(camera)
	camera.position = Vector3(0.0, 40.0, 60.0)
	camera.rotation_degrees = Vector3(-60.0, 0.0, 0.0)
	camera.current = true
	await _acknowledge_network(simulation)
	var terrain := TerrainRenderer.new()
	terrain.name = "Terrain"
	host.add_child(terrain)
	var graded: GradedTiles = terrain.graded_tiles
	_expect(graded != null, "graded tiles are the renderer's default")
	if graded == null:
		terrain.free()
		return
	_expect(await _settle_production(terrain, simulation), "the production terrain must settle: %s" % terrain.get_pending_render_work_counts())
	var world_size: Vector2 = simulation.get_terrain_world_size()
	var junction_patch := Vector2i(((Vector2(0.0, 10.0) + world_size * 0.5) / terrain.patch_span_m).floor())
	_expect(terrain.patches.has(junction_patch) and terrain.resident_patch_lookup.has(junction_patch),
		"the junction's patch %s installs: %d patches" % [junction_patch, terrain.patches.size()])
	for key: Vector2i in terrain.patch_payload_requested.keys():
		_expect(false, "patch %s still waits for its payload" % key)
	# A request Rust refuses by contract is a bug, reported apart from the retries.
	_expect(terrain.patch_payload_refused.is_empty(), "no payload request is refused: %s" % terrain.patch_payload_refused)
	_expect(int(graded.get_pool_counters()["refused"]) == 0, "no graded request is refused: %s" % graded.refused)
	_expect(graded.installed.has(Vector2i(0, 0)), "the junction's tile installs graded")
	_expect(graded.patch_cells.has(junction_patch) and bool(graded.patch_cells[junction_patch]["shown"]), "the junction's patch draws cells")
	var grass := GrassRenderer.new()
	grass.name = "Grass"
	host.add_child(grass)
	for frame in 3:
		await process_frame
	var idle_before := graded.get_pool_counters()
	var mask_renders_before := grass.mask_renders
	var mask_syncs_before := grass.mask_mirror_syncs
	for frame in 120:
		await process_frame
	var idle_after := graded.get_pool_counters()
	_expect(int(idle_after["uploads"]) == int(idle_before["uploads"]),
		"a settled view uploads no tile over 120 frames")
	_expect(int(idle_after["mask_revision_bumps"]) == int(idle_before["mask_revision_bumps"]),
		"a settled view advances no mask revision over 120 frames")
	_expect(grass.mask_renders == mask_renders_before,
		"a settled view renders no grass mask over 120 frames")
	_expect(grass.mask_mirror_syncs == mask_syncs_before,
		"a settled view rewrites no grass-mask mirror over 120 frames")

	var edge: int = simulation.get_hovered_edge(0.0, 100.0)
	_expect(edge >= 0, "the side road has an edge at (0, 100)")
	var edit_mask_renders_before := grass.mask_renders
	var edit_mask_syncs_before := grass.mask_mirror_syncs
	simulation.set_edge_class(edge, 1)
	simulation.sculpt_terrain(Vector2(0.0, 42.0), 26.0, -0.25)
	await _acknowledge_network(simulation)
	_expect(await _settle_production(terrain, simulation), "the bridge's terrain must settle: %s" % terrain.get_pending_render_work_counts())
	_expect(grass.mask_renders > edit_mask_renders_before,
		"a changed payload refreshes the grass mask")
	_expect(grass.mask_mirror_syncs > edit_mask_syncs_before,
		"a changed payload refreshes the grass-mask mirrors")
	var faces := 0
	for tile: Vector2i in graded.installed.keys():
		faces += (graded.installed[tile]["walls"] as PackedFloat32Array).size() / GradedTiles.WALL_FACE_FLOATS
	var instances := 0
	for key: Vector2i in graded.patch_cells.keys():
		if bool(graded.patch_cells[key]["shown"]):
			instances += ((graded.patch_cells[key]["walls"] as MultiMeshInstance3D).multimesh).visible_instance_count
	_expect(faces > 0, "the bridge's mouth has wall faces")
	_expect(instances == faces, "every wall face draws once: %d instances of %d faces" % [instances, faces])
	_expect(int(graded.counters["walls_truncated"]) == 0, "no patch runs out of wall instances")
	_expect(terrain.patch_payload_refused.is_empty(), "no payload request is refused after the bridge: %s" % terrain.patch_payload_refused)
	print("graded walls: %d faces over %d installed tiles" % [faces, graded.installed.size()])
	grass.free()
	await _check_load_hand_over(simulation, terrain)
	terrain.free()
	camera.free()
	water.free()

# T53 (a), TERRAIN-15: a world load through the load flow (input_manager.gd) hands the game over
# once every patch within WORLD_READY_RADIUS_M of the camera draws its set or its PlaneMesh. Until
# then the clock refuses to run and the graded work takes the whole frame; after it the rest
# settles within the budget, no frame drawing a PlaneMesh where T53 (a) forbids it. The fixture's
# world (1,020 m) lies within the radius, so every patch draws at the hand-over here.
func _check_load_hand_over(simulation: SimulationNode, terrain) -> void:
	var path := OS.get_temp_dir().path_join("metrum_graded_load_%d.sqlite" % OS.get_process_id())
	_expect(simulation.save_game(path), "the fixture saves")
	var input_manager = load("res://scripts/core/input_manager.gd").new()
	# Only the dependencies the load flow uses, without gameplay tools.
	input_manager.simulation_node = simulation
	input_manager.terrain_node = terrain
	_expect(input_manager.menu_load_game_from_path(path), "the fixture loads through the load flow")
	_expect(not input_manager.is_world_ready(), "the load holds the game while the terrain renders")
	_expect(terrain.graded_tiles.unbudgeted, "the graded work takes the whole frame while the load waits")
	_expect(terrain.unready_patches_near_camera(InputManager.WORLD_READY_RADIUS_M) > 0,
		"the loaded world's patches near the camera are not ready at the load")
	var handed := -1
	var near_ready := false
	var held := true
	for frame in TIMEOUT_FRAMES:
		await process_frame
		if simulation.is_network_dirty():
			simulation.acknowledge_network_render(simulation.get_network_render_generation())
		# A clock started before the hand-over is refused.
		input_manager.set_simulation_speed(1.0)
		held = held and is_zero_approx(input_manager._simulation_speed)
		input_manager.set_simulation_speed(0.0)
		# The load flow's per-frame check (input_manager._process, before the renderers' frames).
		input_manager._await_world_ready()
		if input_manager.is_world_ready():
			handed = frame
			near_ready = terrain.unready_patches_near_camera(InputManager.WORLD_READY_RADIUS_M) == 0
			break
	_expect(handed > 0, "the load hands the game over once the terrain near the camera is ready (frame %d)" % handed)
	_expect(held, "the clock stays stopped until the hand-over")
	_expect(near_ready, "the game is handed over on a frame with every patch near the camera ready")
	_expect(terrain.graded_tiles.hole_patches() == 0 and terrain.graded_tiles.plane_violations() == 0,
		"at the hand-over every patch near the camera draws its set or its PlaneMesh")
	_expect(not terrain.graded_tiles.unbudgeted, "the hand-over returns the graded work to its budget")
	var settled := -1
	var violations := 0
	for frame in TIMEOUT_FRAMES:
		await process_frame
		if simulation.is_network_dirty():
			simulation.acknowledge_network_render(simulation.get_network_render_generation())
		violations += terrain.graded_tiles.plane_violations()
		if terrain.is_render_settled():
			settled = frame
			break
	_expect(settled >= 0, "the terrain settles within its budget after the hand-over")
	_expect(violations == 0, "no frame after the hand-over draws a PlaneMesh T53 (a) forbids: %d" % violations)
	input_manager.set_simulation_speed(1.0)
	_expect(is_equal_approx(input_manager._simulation_speed, 1.0), "the clock runs after the hand-over")
	input_manager.set_simulation_speed(0.0)
	input_manager.free()
	DirAccess.remove_absolute(path)

# terrain.gd uploads nothing while the network generation is unacknowledged
# (network_renderer.gd acknowledges it in the game).
func _acknowledge_network(simulation: SimulationNode) -> void:
	for frame in 60:
		if not simulation.is_network_dirty():
			return
		simulation.acknowledge_network_render(simulation.get_network_render_generation())
		await process_frame

# The terrain renderer settled (its patches and graded tiles) and the simulation clean, for three
# frames in a row.
func _settle_production(terrain, simulation: SimulationNode) -> bool:
	var stable := 0
	for frame in TIMEOUT_FRAMES:
		await process_frame
		if simulation.is_network_dirty():
			simulation.acknowledge_network_render(simulation.get_network_render_generation())
		var idle: bool = (
			terrain.is_render_settled()
			and not simulation.is_network_dirty()
			and not simulation.is_terrain_dirty()
		)
		stable = stable + 1 if idle else 0
		if stable >= 3:
			return true
	return false

func _creation_counters(graded: GradedTiles) -> Dictionary:
	var counters: Dictionary = graded.get_pool_counters()
	var out := {}
	for name in ["textures_created", "images_created", "meshes_created", "materials_created", "multimeshes_created"]:
		out[name] = int(counters[name])
	return out

# Patch nodes with a terrain material each, as terrain.gd hands them over.
func _resident_patches(host: Node3D, layout: Dictionary, world_size: Vector2, generation: int) -> Dictionary:
	var resident := {}
	var span := float(layout["patch_interval_cells"]) * CELL_M
	var shader := load("res://assets/materials/terrain.gdshader")
	for z in int(layout["patch_rows"]):
		for x in int(layout["patch_cols"]):
			var origin := -world_size * 0.5 + Vector2(x, z) * span
			var size := Vector2(min(span, world_size.x * 0.5 - origin.x), min(span, world_size.y * 0.5 - origin.y))
			var node := MeshInstance3D.new()
			var material := ShaderMaterial.new()
			material.shader = shader
			node.material_override = material
			node.position = Vector3(origin.x + size.x * 0.5, 0.0, origin.y + size.y * 0.5)
			host.add_child(node)
			resident[Vector2i(x, z)] = {
				"node": node,
				"material": material,
				"rect": Rect2(origin, size),
				"step": 1,
				"generation": generation,
				"aabb": TerrainRenderer.patch_cull_aabb(size),
			}
	return resident

func _mark_changed(resident: Dictionary) -> void:
	for key: Vector2i in resident.keys():
		_changed[key] = true

# One renderer frame with the changes since the last, as terrain.gd hands them over.
func _frame(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> void:
	graded.process(camera, resident, _changed, {})
	_changed.clear()
	_violations = max(_violations, graded.plane_violations())
	if _check_partial:
		_partial = max(_partial, graded.partial_sets())
	_unready_seen = max(_unready_seen, graded._unready.size())

func _settle(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> bool:
	var stable := 0
	for frame in TIMEOUT_FRAMES:
		_frame(graded, camera, resident)
		await process_frame
		var idle: bool = not graded.has_pending_work()
		stable = stable + 1 if idle and graded.installed.size() > 0 else 0
		if stable >= 3:
			return true
	return false

# Waits for the edit batch's install, then for the residency to settle.
func _settle_edit(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> bool:
	for frame in TIMEOUT_FRAMES:
		_frame(graded, camera, resident)
		await process_frame
		if int(graded.counters["edit_batches"]) > 0:
			return await _settle(graded, camera, resident)
	return false

# Every tile meeting the patch is one instance, graded where installed and a base cell
# elsewhere; a side shared with a coarser cell stitches to the coarser code.
func _check_cells(graded: GradedTiles, key: Vector2i, resident: Dictionary) -> void:
	var cells: Dictionary = graded.patch_cells[key]
	var patch: Dictionary = resident[key]
	var tiles: Array[Vector2i] = GradedTiles._patch_tiles(patch["rect"])
	var total := 0
	var nodes: Array = cells["nodes"]
	# The headless renderer keeps no MultiMesh custom data; the renderer's own record of what it
	# wrote stands for it.
	var counts: Array[int] = cells["counts"]
	var shown := bool(cells["shown"])
	# Its graded materials bind the pages its block uses (docs/terrain.md "Pages and the row atlas").
	_expect(nodes.size() == (GradedTiles.CELL_MESHES if bool(cells["variant"]) else GradedTiles.CELL_PLAIN)
		and (cells["materials"] as Array).size() == nodes.size(), "patch %s holds its variant's meshes with it" % key)
	for index in nodes.size():
		if index == GradedTiles.CELL_BASE:
			continue
		var code := index - GradedTiles.CELL_BEHIND if index >= GradedTiles.CELL_BEHIND else index
		var page: Dictionary = graded.page_store.by_id.get(graded._patch_page(key, code), {})
		var material: ShaderMaterial = cells["materials"][index]
		_expect(material.get_shader_parameter("graded_texels") == page.get("texels", null)
			and material.get_shader_parameter("graded_atlas") == page.get("atlas", null),
			"patch %s binds its block's level %d page" % [key, code])
	_expect(shown == (GradedTiles._graded_cells(counts) > 0), "patch %s: a set draws while a tile of it shows graded" % key)
	for index in nodes.size():
		_expect((nodes[index] as GeometryInstance3D).material_override != null if shown else (nodes[index] as GeometryInstance3D).material_override == null,
			"a set binds its materials while it draws its patch, and only then")
		var multimesh: MultiMesh = (nodes[index] as MultiMeshInstance3D).multimesh
		var written: Array = (cells["custom"][index] as Array).slice(0, counts[index])
		_expect(multimesh.visible_instance_count == written.size(), "visible instances match the written ones")
		if DisplayServer.get_name() != "headless":
			for instance in written.size():
				_expect(multimesh.get_instance_transform(instance).is_equal_approx(Transform3D.IDENTITY),
					"patch %s mesh %d cell %d keeps its identity transform" % [key, index, instance])
				_expect(multimesh.get_instance_custom_data(instance) == written[instance],
					"patch %s mesh %d cell %d keeps its tile data" % [key, index, instance])
		total += written.size()
		for data: Color in written:
			# The tile's place in the patch (column + 32 row) and the codes in two channels.
			var place := int(data.r)
			var tile: Vector2i = tiles[0] + Vector2i(place % GradedTiles.PATCH_TILE_ROW, place / GradedTiles.PATCH_TILE_ROW)
			var codes := int(data.b) | (int(data.a) << 9)
			_expect(data.r < 2048.0 and data.b < 2048.0 and data.a < 2048.0, "custom channels are exact as half floats")
			var own := codes & 7
			var entry: Dictionary = graded._shown_in(tile, key)
			if index != GradedTiles.CELL_BASE:
				var page := graded._patch_page(key, int(entry.get("level", 0)))
				_expect(not entry.is_empty() and int(data.g) == (entry["copies"][page] as Vector3i).x, "a graded instance names its slot in its block's page")
			else:
				_expect(entry.is_empty() and int(data.g) == -1, "a base cell has no slot")
			for side in 4:
				var side_code := (codes >> (3 + side * 3)) & 7
				_expect(side_code >= own, "a side stitches to its own or a coarser lattice")
	_expect(total == tiles.size(), "patch %s: every tile is one cell (%d of %d)" % [key, total, tiles.size()])

# TERRAIN-07-E: a road preview's graded tiles draw from slots of their own in place of the
# installed tiles, staged at the camera's levels, and leave the installed tiles as they were; the
# commit's edit installs the tiles it staged and releases the preview's in the same frame.
func _check_road_preview(graded: GradedTiles, simulation: SimulationNode, camera: Vector3, resident: Dictionary) -> void:
	var installed_before: Dictionary = graded.installed.duplicate(true)
	# Every resident patch's set is built off the frame paths (round 6): neither the preview's
	# frames nor its commit's builds one.
	var builds_before := int(graded.counters["cell_builds"])
	var adopted_before := int(graded.counters["edit_adopted"])
	# A T off the trunk. Keep it clear of the crossing fixture so the commit grades only its own
	# local margins.
	var points := PackedVector3Array([Vector3(60.0, 0.0, 10.0), Vector3(60.0, 0.0, 80.0)])
	var frame: Dictionary = simulation.preview_road(points, 1, 1, true)
	_expect(frame.get("is_valid", false), "the preview stroke must validate: %s" % frame.get("invalid_reason", ""))
	var install_ms := graded.show_preview(frame)
	var pending := int(frame.get("graded_pending", 0))
	var frames := 1
	while pending > 0 and frames < 64:
		_frame(graded, camera, resident)
		var step: Dictionary = simulation.advance_road_preview_terrain()
		install_ms = maxf(install_ms, graded.show_preview(step))
		pending = int(step.get("graded_pending", 0))
		frames += 1
	_expect(pending == 0 and graded.preview.size() > 0, "the preview's tiles converge and show (%d)" % graded.preview.size())
	var levels := {}
	for entry: Dictionary in graded.preview.values():
		levels[int(entry["level"])] = true
	_expect(levels.size() > 1, "the preview's tiles take the camera's levels: %s" % [levels.keys()])
	_expect(graded.installed == installed_before, "a preview leaves the installed tiles as they were")
	for key: Vector2i in graded.patch_cells.keys():
		_check_cells(graded, key, resident)
	var shown := graded.preview.keys()
	print("graded preview: %d tiles over %d frames, levels %s, install ms max %.3f (headless)" % [shown.size(), frames, levels.keys(), install_ms])
	# The click: the commit adopts the shown tiles; its edit installs them and drops the preview's.
	var batches := int(graded.counters["edit_batches"])
	var request: int = simulation.add_road_with_snap(points, 1, 1, true)
	simulation.release_road_preview_terrain()
	for wait in TIMEOUT_FRAMES:
		if simulation.get_road_commit_result(request) != null:
			break
		await process_frame
	var installed_frame := false
	for wait in TIMEOUT_FRAMES:
		_frame(graded, camera, resident)
		await process_frame
		if int(graded.counters["edit_batches"]) > batches:
			installed_frame = graded.preview.is_empty()
			break
	_expect(installed_frame, "the commit's edit releases the preview's tiles in its batch's frame")
	for key: Vector2i in shown:
		_expect(graded.last_edit_batch.has(key), "the batch installs the committed tile %s" % key)
	_expect(int(graded.counters["cell_builds"]) == builds_before, "the preview and its commit build no cell set")
	# The converged preview's tiles are the commit's: each takes over its preview slot, no upload.
	var adopted := int(graded.counters["edit_adopted"]) - adopted_before
	_expect(adopted == shown.size(), "the commit adopts every previewed tile's slot: %d of %d" % [adopted, shown.size()])
	_expect(graded.hole_patches() == 0, "after the commit every resident patch draws its PlaneMesh or its set")
	# The patches' new generations, as terrain.gd hands them over after the commit.
	for key: Vector2i in resident.keys():
		resident[key]["generation"] = int(resident[key]["generation"]) + 1
	_mark_changed(resident)
	_expect(await _settle(graded, camera, resident), "the residency settles after the commit")
	for key: Vector2i in shown:
		_expect(graded._desired.has(key), "the committed tile %s is listed" % key)
	for key: Vector2i in graded.patch_cells.keys():
		_check_cells(graded, key, resident)

func _add_road(simulation: Node, points: Array) -> void:
	var request: int = simulation.add_road_with_snap(PackedVector3Array(points), 1, 1, true)
	for frame in TIMEOUT_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result != null:
			_expect(result.get("committed", false), "fixture road must commit: %s" % result)
			return
		await process_frame
	_expect(false, "fixture road commit timed out")
