# SPDX-License-Identifier: GPL-2.0-only

## Seed 6807 reproduces the full-page migration deadlock at stroke attempt 52.
## Drives the production road tool and renderers; checks meshes, materials, cell references and
## GPU bytes against fresh native bakes, then repeats the comparison after save/load.
## Use -- --extended for 100 attempts (Metal); the 64-attempt gate fits headless
## device limits. No authored assets or player saves are needed.
extends SceneTree

const RoadTool := preload("res://scripts/tools/road_tool.gd")
const Terrain := preload("res://scripts/renderers/terrain.gd")
const Network := preload("res://scripts/renderers/network_renderer.gd")
const LAYERS := ["skirt", "curb", "raised_step", "sidewalk", "road", "marking", "concrete", "lining", "median_grass"]
var sim: SimulationNode
var terrain
var tool
var network
var camera: Camera3D
var failures := 0

func _initialize():
	call_deferred("_run")

func check(ok: bool, message: String):
	if not ok:
		failures += 1
		push_error(message)

func _run():
	var host := Node3D.new()
	root.add_child(host)
	sim = SimulationNode.new()
	sim.name = "SimulationNode"
	host.add_child(sim)
	check(sim.create_blank_world(500.0, 500.0, 10.0, 512.0, 0.0), "blank world")
	sim.set_simulation_speed(0.0)
	camera = Camera3D.new()
	host.add_child(camera)
	camera.position = Vector3(0, 35, 40)
	camera.look_at(Vector3.ZERO)
	camera.current = true
	var light := DirectionalLight3D.new()
	host.add_child(light)
	light.rotation_degrees = Vector3(-50, -30, 0)
	var water := Node3D.new()
	water.name = "Water"
	host.add_child(water)
	terrain = Terrain.new()
	terrain.name = "Terrain"
	host.add_child(terrain)
	terrain.set_process(false)
	# Detached tool avoids unrelated input/UI; its production mesh root is in the scene.
	tool = RoadTool.new()
	tool.name = "RoadTool"
	tool.simulation_node = sim
	tool.terrain_node = terrain
	tool.road_mesh_root = Node3D.new()
	host.add_child(tool.road_mesh_root)
	tool.blueprint_mesh = MeshInstance3D.new()
	tool.add_child(tool.blueprint_mesh)
	tool._info_label = Label.new()
	tool.add_child(tool._info_label)
	tool._node_visuals_visible = true
	network = Network.new()
	network.simulation_node = sim
	network.terrain = terrain
	network.road_tool = tool
	if not await settle("initial"):
		quit(1)
		return
	var rng := RandomNumberGenerator.new()
	rng.seed = 6807
	var strokes := []
	for i in (100 if "--extended" in OS.get_cmdline_user_args() else 64):
		var x := rng.randf_range(-210, 210)
		var z := rng.randf_range(-210, 210)
		strokes.append([Vector3(x,0,z), Vector3(clampf(x + rng.randf_range(-210,210),-240,240),0,clampf(z + rng.randf_range(-210,210),-240,240))])
	for i in strokes.size():
		var a: Vector3 = strokes[i][0]
		var b: Vector3 = strokes[i][1]
		camera.position = b + Vector3(0, 25 if i % 3 != 0 else 150, 30)
		camera.look_at(b)
		tool.active = true
		tool._node_visuals_dirty = false
		tool._begin_stroke(a, 0, Vector3.ZERO)
		tool.set_scripted_pointer(true, b)
		for frame in 5:
			tool.set_scripted_pointer(true,b + Vector3((4-frame)*8,0,0))
			tool._queue_preview_update()
			tool._process(1.0/60)
			terrain._process(1.0/60)
			network._process(1.0/60)
			await process_frame
		if not tool._commit_segment(b):
			tool.cancel_road()
			continue
		tool.active = false
		if not await settle("stroke %d" % i):
			break
		await audit(i)
		if failures > 0:
			break
	tool.cancel_road()
	if failures == 0:
		check(await settle("cancel"), "final preview releases")
		var path := OS.get_user_data_dir().path_join("road_render_publication.sqlite")
		check(sim.save_game(path), "fixture saves")
		check(sim.load_game(path), "fixture reloads")
		sim.set_simulation_speed(0)
		await audit(strokes.size())
		DirAccess.remove_absolute(path)
	print("Road render publication: seed=6807 attempts=%d generation=%d failures=%d" % [strokes.size(), tool._road_mesh_generation, failures])
	tool.free()
	network.free()
	host.free()
	quit(0 if failures == 0 else 1)

func settle(label: String) -> bool:
	var deadline := Time.get_ticks_msec() + 20000
	var stable := 0
	while Time.get_ticks_msec() < deadline:
		tool._process(1.0/60)
		terrain._process(1.0/60)
		network._process(1.0/60)
		await process_frame
		var idle: bool = tool._commit_request_id == 0 and not sim.is_network_dirty() and not sim.is_terrain_dirty() and terrain.is_render_settled() and tool._held_serial < 0
		stable = stable + 1 if idle else 0
		if stable == 3:
			return true
	var g = terrain.graded_tiles
	check(false, "%s timeout held=%d serial=%d installed=%d batch=%s staged=%s wanted=%s requests=%s counts=%s" % [label, tool._held_serial, sim.get_graded_edit_serial(), g.installed_edit_serial(), g._book.edit_batch_len(), g._edit_staged.keys(), g.wanted_count(), g.get_pool_counters(), terrain.get_pending_render_work_counts()])
	return false

func audit(stroke: int):
	var full: Dictionary = sim.get_road_mesh_data(true)
	check(tool._road_chunk_instances.size() == full.chunks.size(), "chunk count %d" % stroke)
	for chunk: Dictionary in full.chunks:
		var key := Vector2i(chunk.chunk_x, chunk.chunk_z)
		check(tool._road_chunk_instances.has(key), "missing %s" % key)
		if not tool._road_chunk_instances.has(key):
			continue
		var instance: MeshInstance3D = tool._road_chunk_instances[key]
		check(instance.visible, "hidden %s" % key)
		var surfaces: PackedInt32Array = instance.mesh.get_meta("road_layer_surfaces")
		for layer in LAYERS.size():
			if surfaces[layer] < 0:
				continue
			var arrays: Array = instance.mesh.surface_get_arrays(surfaces[layer])
			check(arrays[Mesh.ARRAY_VERTEX] == chunk[LAYERS[layer] + "_vertices"], "mesh mismatch stroke=%d chunk=%s layer=%s" % [stroke, key, LAYERS[layer]])
			check(instance.mesh.surface_get_material(surfaces[layer]) != null, "missing material")
	var g = terrain.graded_tiles
	for key: Vector2i in g.patch_cells:
		var cells: Dictionary = g.patch_cells[key]
		for tile: Vector2i in cells.tiles:
			var at: Vector2i = cells.where[g._cell_place(cells,tile)]
			var expected: Color = g._cell_custom(tile,cells.first,g._resident[key],g._resident,key)
			check(at.x == g._cell_mesh(tile,key) and cells.custom[at.x][at.y] == expected, "cell mismatch stroke=%d tile=%s stored=%s wanted=%s" % [stroke,tile,cells.custom[at.x][at.y],expected])
	if DisplayServer.get_name() == "headless":
		return
	var graded = terrain.graded_tiles
	var pending: Dictionary = graded.installed.duplicate()
	var unsent := pending.keys()
	var deadline := Time.get_ticks_msec() + 30000
	while not pending.is_empty() and Time.get_ticks_msec() < deadline:
		if not unsent.is_empty():
			var requests := PackedInt64Array()
			var keys := unsent.slice(0, 32)
			for key: Vector2i in keys:
				var entry: Dictionary = pending[key]
				requests.append_array(PackedInt64Array([key.x, key.y, entry.level, entry.step, 100000 + stroke]))
			var accepted := sim.request_graded_tiles(requests)
			for i in accepted.size():
				if accepted[i] != 0:
					unsent.erase(keys[i])
		var result := sim.poll_graded_tile_results(100, true)
		for failed: Dictionary in result.failed:
			if failed.kind == "tile":
				check(failed.retry, "fresh bake refused: %s" % failed)
				var key := Vector2i(failed.keys[0], failed.keys[1])
				if not unsent.has(key):
					unsent.append(key)
		for tile: Dictionary in result.tiles:
			var key := Vector2i(tile.tile_x,tile.tile_z)
			if not pending.has(key):
				continue
			var entry: Dictionary = pending[key]
			for page_id in entry.copies:
				var page: Dictionary = graded.page_store.by_id[page_id]
				var copy: Vector3i = entry.copies[page_id]
				var side: int = graded.page_store.levels[entry.level].side
				var texels := Image.create_from_data(side, side + 1, false, Image.FORMAT_RGBAF, graded.page_store.layer_data(page, true, copy.x))
				var header := texels.get_pixel(0,side)
				header.r = 0
				texels.set_pixel(0,side,header)
				check(texels.get_data() == tile.texels, "GPU texels mismatch stroke=%d tile=%s" % [stroke,key])
				var rows := PackedByteArray()
				for layer in tile.rows.size() / 4096:
					rows.append_array(graded.page_store.layer_data(page, false, copy.y + layer))
				check(rows == tile.rows, "GPU atlas mismatch stroke=%d tile=%s" % [stroke,key])
			pending.erase(key)
		await process_frame

	check(pending.is_empty(), "audit timed out %s" % [pending.keys()])
