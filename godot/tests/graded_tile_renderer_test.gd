# SPDX-License-Identifier: GPL-2.0-only

## Graded tile renderer (TERRAIN-07-D): residency requests and async bakes through the bridge,
## installs into the prewarmed slot pools within the install budget, the cell instances and
## their stitching codes, eviction, and a populated edit's reinstall creating no texture, image,
## material, mesh or MultiMesh (the pool counters): only the installed tiles Rust selects for the
## commit rebake, all in one frame, each into its own slot (item 5). Then the production renderer
## (terrain.gd) with graded tiles on settles with every patch installed, the road's included.
extends SceneTree

const GradedTiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const TerrainRenderer := preload("res://scripts/renderers/terrain.gd")
const TIMEOUT_FRAMES := 900
const WORLD_M := 1020.0
const CELL_M := 10.0

var _failures := 0
# The resident patch keys changed since the renderer's last frame (terrain.gd's): every key of a
# rewritten or dropped resident dictionary.
var _changed := {}

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var host := Node3D.new()
	root.add_child(host)
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
	_expect(int(created["textures_created"]) == 8, "the pools are 4 levels x (texels, rows): %s" % created)
	_expect(int(created["multimeshes_created"]) == (GradedTiles.CELL_MESHES + 1) * 4, "one cell set per patch of the 2 x 2 grid: %s" % created)

	var resident := _resident_patches(host, layout, world_size, 1)
	_mark_changed(resident)
	var camera := Vector3(0.0, 40.0, 10.0)
	_expect(await _settle(graded, camera, resident), "the junction's tiles must install")
	var counters: Dictionary = graded.get_pool_counters()
	_expect(int(counters["installed"]) > 0, "graded tiles must be installed: %s" % counters)
	_expect(int(counters["bake_failures"]) == 0 and int(counters["refused"]) == 0, "no bake may fail: %s" % counters)
	_expect(int(counters["pool_exhausted"]) == 0, "the pools must not run out: %s" % counters)
	# The tiles under the junction are graded and, 40 m above it, at GL0 (to 64 m).
	var junction := Vector2i(0, 0)
	_expect(graded.installed.has(junction), "the junction's tile must be graded")
	if graded.installed.has(junction):
		_expect(int(graded.installed[junction]["level"]) == 0, "the junction's tile must be at GL0")
	_expect(graded.patch_cells.size() > 0, "patches with graded tiles must draw cells")
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
	var uploads_before := int(graded.get_pool_counters()["uploads"])
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
			_expect(int(now["slot"]) == int(was["slot"]), "tile %s rebakes into its own slot" % tile)
	_expect(rebaked > 0, "the edit rebakes installed tiles")
	_expect(batch.size() < graded.installed.size(), "the edit rebakes only its dirty tiles (%d of %d installed)" % [batch.size(), graded.installed.size()])
	# Nothing is created after setup, neither while streaming nor by the edit: the cell sets are
	# prewarmed, one per patch of the grid (round 5's populated edit created 16 materials and 24
	# MultiMeshes for patches newly drawing cells).
	var after := _creation_counters(graded)
	for name: String in created.keys():
		_expect(after[name] == created[name], "nothing may be created after setup (%s): %s -> %s" % [name, created[name], after[name]])
	# Each pooled cell set holds five cell MultiMeshes, the wall MultiMesh and the four graded
	# levels' materials; the wall material is one, made at setup.
	_expect((int(after["materials_created"]) - 1) * (GradedTiles.CELL_MESHES + 1) == int(after["multimeshes_created"]) * 4, "cell materials are pooled with their cells: %s" % after)
	print("graded tile install ms max %.3f (poll %.3f, upload %.3f), uploads %d, installed %d, pool bytes %d, edit %d tiles in %.3f ms" % [
		float(counters["install_ms_max"]), float(counters["poll_ms_max"]), float(counters["upload_ms_max"]),
		int(counters["uploads"]), int(counters["installed"]), int(counters["pool_bytes"]),
		int(counters["edit_tiles_last"]), float(counters["edit_install_ms_last"])])

	await _check_road_preview(graded, simulation, camera, resident)
	await _check_moving_camera(graded, resident)

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

# Per cell mesh, the sorted custom data of its instances, with the visible counts.
func _cell_record(cells: Dictionary) -> Array:
	var out := []
	var counts: Array[int] = cells["counts"]
	for index in GradedTiles.CELL_MESHES:
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
	camera.position = Vector3(0.0, 120.0, 60.0)
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
	_expect(graded.patch_cells.has(junction_patch), "the junction's patch draws cells")

	var edge: int = simulation.get_hovered_edge(0.0, 100.0)
	_expect(edge >= 0, "the side road has an edge at (0, 100)")
	simulation.set_edge_class(edge, 1)
	simulation.sculpt_terrain(Vector2(0.0, 42.0), 26.0, -0.25)
	await _acknowledge_network(simulation)
	_expect(await _settle_production(terrain, simulation), "the bridge's terrain must settle: %s" % terrain.get_pending_render_work_counts())
	var faces := 0
	for tile: Vector2i in graded.installed.keys():
		faces += (graded.installed[tile]["walls"] as PackedFloat32Array).size() / GradedTiles.WALL_FACE_FLOATS
	var instances := 0
	for key: Vector2i in graded.patch_cells.keys():
		instances += ((graded.patch_cells[key]["walls"] as MultiMeshInstance3D).multimesh).visible_instance_count
	_expect(faces > 0, "the bridge's mouth has wall faces")
	_expect(instances == faces, "every wall face draws once: %d instances of %d faces" % [instances, faces])
	_expect(int(graded.counters["walls_truncated"]) == 0, "no patch runs out of wall instances")
	_expect(terrain.patch_payload_refused.is_empty(), "no payload request is refused after the bridge: %s" % terrain.patch_payload_refused)
	print("graded walls: %d faces over %d installed tiles" % [faces, graded.installed.size()])
	terrain.free()
	camera.free()
	water.free()

# terrain.gd uploads nothing while the network generation is unacknowledged
# (network_renderer.gd acknowledges it in the game).
func _acknowledge_network(simulation: SimulationNode) -> void:
	for frame in 60:
		if not simulation.is_network_dirty():
			return
		simulation.acknowledge_network_render(simulation.get_network_render_generation())
		await process_frame

# No pending patch work and no tile list or bake in flight for three frames in a row.
func _settle_production(terrain, simulation: SimulationNode) -> bool:
	var graded: GradedTiles = terrain.graded_tiles
	var stable := 0
	for frame in TIMEOUT_FRAMES:
		await process_frame
		if simulation.is_network_dirty():
			simulation.acknowledge_network_render(simulation.get_network_render_generation())
		var idle: bool = (
			not terrain.has_pending_render_work(false)
			and not simulation.is_network_dirty()
			and not simulation.is_terrain_dirty()
			and graded.requested.is_empty()
			and graded.patch_lists_requested.is_empty()
			and graded._counts.is_empty()
			and graded._edit_batch.is_empty()
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

func _settle(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> bool:
	var stable := 0
	for frame in TIMEOUT_FRAMES:
		_frame(graded, camera, resident)
		await process_frame
		var idle: bool = graded.requested.is_empty() and graded.patch_lists_requested.is_empty() and graded._counts.is_empty()
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
	for index in nodes.size():
		_expect((nodes[index] as GeometryInstance3D).material_override != null, "a set drawing a patch binds its materials")
		var multimesh: MultiMesh = (nodes[index] as MultiMeshInstance3D).multimesh
		var written: Array = (cells["custom"][index] as Array).slice(0, counts[index])
		_expect(multimesh.visible_instance_count == written.size(), "visible instances match the written ones")
		total += written.size()
		for data: Color in written:
			# The tile's place in the patch (column + 32 row) and the codes in two channels.
			var place := int(data.r)
			var tile: Vector2i = tiles[0] + Vector2i(place % GradedTiles.PATCH_TILE_ROW, place / GradedTiles.PATCH_TILE_ROW)
			var codes := int(data.b) | (int(data.a) << 9)
			_expect(data.r < 2048.0 and data.b < 2048.0 and data.a < 2048.0, "custom channels are exact as half floats")
			var own := codes & 7
			var entry: Dictionary = graded._shown(tile)
			if index != GradedTiles.CELL_BASE:
				_expect(not entry.is_empty() and int(data.g) == int(entry["slot"]), "a graded instance names its slot")
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
	# A T off the trunk. (A crossing here, (60, -50) to (60, 70), panics in the exact cut's
	# legacy road loops while the commit grades its margins: road_clip.rs, TERRAIN-07-G's.)
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
