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

	var resident := _resident_patches(host, layout, world_size, 1)
	var camera := Vector3(0.0, 120.0, 10.0)
	_expect(await _settle(graded, camera, resident), "the junction's tiles must install")
	var counters: Dictionary = graded.get_pool_counters()
	_expect(int(counters["installed"]) > 0, "graded tiles must be installed: %s" % counters)
	_expect(int(counters["bake_failures"]) == 0, "no bake may fail: %s" % counters)
	_expect(int(counters["pool_exhausted"]) == 0, "the pools must not run out: %s" % counters)
	# The tiles under the junction are graded and, 120 m above it, at GL0.
	var junction := Vector2i(0, 0)
	_expect(graded.installed.has(junction), "the junction's tile must be graded")
	if graded.installed.has(junction):
		_expect(int(graded.installed[junction]["level"]) == 0, "the junction's tile must be at GL0")
	_expect(graded.patch_cells.size() > 0, "patches with graded tiles must draw cells")
	for key: Vector2i in graded.patch_cells.keys():
		_check_cells(graded, key, resident)

	# A populated edit: another road beside the junction. Rust selects the commit's dirty tiles;
	# the installed ones among them rebake and install together in one frame, each into its own
	# slot, while the patch lists refresh with the patches' new generations.
	var before_edit: Dictionary = graded.installed.duplicate(true)
	var uploads_before := int(graded.get_pool_counters()["uploads"])
	await _add_road(simulation, [Vector3(-120.0, 0.0, 70.0), Vector3(-10.0, 0.0, 70.0)])
	for key: Vector2i in resident.keys():
		resident[key]["generation"] = int(resident[key]["generation"]) + 1
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
	var after := _creation_counters(graded)
	for name: String in created.keys():
		if name == "materials_created" or name == "multimeshes_created":
			continue
		_expect(after[name] == created[name], "an edit must create no %s: %s -> %s" % [name, created[name], after[name]])
	# Each pooled cell set holds five cell MultiMeshes, the wall MultiMesh and the four graded
	# levels' materials; the wall material is one, made at setup.
	_expect((int(after["materials_created"]) - 1) * 6 == int(after["multimeshes_created"]) * 4, "cell materials are pooled with their cells: %s" % after)
	print("graded tile install ms max %.3f (poll %.3f, upload %.3f), uploads %d, installed %d, pool bytes %d, edit %d tiles in %.3f ms" % [
		float(counters["install_ms_max"]), float(counters["poll_ms_max"]), float(counters["upload_ms_max"]),
		int(counters["uploads"]), int(counters["installed"]), int(counters["pool_bytes"]),
		int(counters["edit_tiles_last"]), float(counters["edit_install_ms_last"])])

	# Far away every tile is evicted and every slot freed.
	var far := _resident_patches(host, layout, world_size, 1)
	for key: Vector2i in far.keys():
		far[key]["generation"] = int(resident[key]["generation"])
	for frame in 4:
		graded.process(Vector3(0.0, 120.0, 10.0), {})
		await process_frame
	counters = graded.get_pool_counters()
	_expect(int(counters["installed"]) == 0, "no tile may stay installed without resident patches")
	for code in 4:
		_expect(int(counters["free_%d" % code]) == int(counters["slots_%d" % code]), "level %d must free its slots" % code)
	graded.clear()
	await _check_production_renderer(host, simulation)
	host.queue_free()
	await process_frame
	print("Graded tile renderer tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

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

func _settle(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> bool:
	var stable := 0
	for frame in TIMEOUT_FRAMES:
		graded.process(camera, resident)
		await process_frame
		var idle: bool = graded.requested.is_empty() and graded.patch_lists_requested.is_empty()
		stable = stable + 1 if idle and graded.installed.size() > 0 else 0
		if stable >= 3:
			return true
	return false

# Waits for the edit batch's install, then for the residency to settle.
func _settle_edit(graded: GradedTiles, camera: Vector3, resident: Dictionary) -> bool:
	for frame in TIMEOUT_FRAMES:
		graded.process(camera, resident)
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
	var instances: Array = cells["instances"]
	for index in nodes.size():
		var multimesh: MultiMesh = (nodes[index] as MultiMeshInstance3D).multimesh
		var written: PackedColorArray = instances[index]
		_expect(multimesh.visible_instance_count == written.size(), "visible instances match the written ones")
		total += written.size()
		for data in written:
			# The tile's place in the patch (column + 32 row) and the codes in two channels.
			var place := int(data.r)
			var tile: Vector2i = tiles[0] + Vector2i(place % GradedTiles.PATCH_TILE_ROW, place / GradedTiles.PATCH_TILE_ROW)
			var codes := int(data.b) | (int(data.a) << 9)
			_expect(data.r < 2048.0 and data.b < 2048.0 and data.a < 2048.0, "custom channels are exact as half floats")
			var own := codes & 7
			var entry: Dictionary = graded.installed.get(tile, {})
			if index < 4:
				_expect(not entry.is_empty() and int(data.g) == int(entry["slot"]), "a graded instance names its slot")
			else:
				_expect(entry.is_empty() and int(data.g) == -1, "a base cell has no slot")
			for side in 4:
				var side_code := (codes >> (3 + side * 3)) & 7
				_expect(side_code >= own, "a side stitches to its own or a coarser lattice")
	_expect(total == tiles.size(), "patch %s: every tile is one cell (%d of %d)" % [key, total, tiles.size()])

func _add_road(simulation: Node, points: Array) -> void:
	var request: int = simulation.add_road_with_snap(PackedVector3Array(points), 1, 1, true)
	for frame in TIMEOUT_FRAMES:
		var result: Variant = simulation.get_road_commit_result(request)
		if result != null:
			_expect(result.get("committed", false), "fixture road must commit: %s" % result)
			return
		await process_frame
	_expect(false, "fixture road commit timed out")
