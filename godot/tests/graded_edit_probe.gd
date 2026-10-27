# SPDX-License-Identifier: GPL-2.0-only

## Graded tile probe (TERRAIN-07-D item 5; opt-in, not part of run.sh, needs a rendering device):
## loads a save and renders its terrain through the production renderer at one camera, the exact
## cut and then the graded tiles in one session, recording per mode the time until the terrain
## settles and the video memory; for the graded tiles also the slots each level uses, then a road
## committed beside the camera and the frame that installs the commit's dirty tiles
## (edit_install_ms), with the creation counters before and after (nothing may be created).
## Env: METRUM_GRADED_PROBE_SAVE (save path), METRUM_GRADED_PROBE_MODS (asset pack directory,
## optional), METRUM_GRADED_PROBE_AT ("x,z", the camera's focus), METRUM_GRADED_PROBE_ROAD
## ("x0,z0,x1,z1", the edit), METRUM_GRADED_PROBE_HEIGHT (camera height over the focus, m;
## default 150), METRUM_GRADED_PROBE_OUT (the JSON report's path).
extends SceneTree

const TerrainScript := preload("res://scripts/renderers/terrain.gd")
const SETTLE_TIMEOUT_MS := 600000
const CREATED := ["textures_created", "images_created", "meshes_created", "materials_created", "multimeshes_created"]

var simulation: SimulationNode
var _failures := 0
var _report := {}

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _numbers(name: String, count: int) -> PackedFloat64Array:
	var out := PackedFloat64Array()
	for part in OS.get_environment(name).split(",", false):
		out.append(float(part))
	_expect(out.size() == count, "%s needs %d numbers" % [name, count])
	return out

# The production terrain uploads nothing while the network generation is unacknowledged, which
# network_renderer.gd does in the game once its road chunks are up.
func _acknowledge_network() -> void:
	var deadline := Time.get_ticks_msec() + 30000
	while simulation.is_network_dirty() and Time.get_ticks_msec() < deadline:
		simulation.acknowledge_network_render(simulation.get_network_render_generation())
		await process_frame
	_expect(not simulation.is_network_dirty(), "the network render must be acknowledged")

func _surface(point: Vector2) -> Vector3:
	return Vector3(point.x, float(simulation.get_world_surface_height(point)), point.y)

func _memory() -> Dictionary:
	return {
		"video_mem_bytes": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_VIDEO_MEM_USED),
		"texture_mem_bytes": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_TEXTURE_MEM_USED),
		"buffer_mem_bytes": RenderingServer.get_rendering_info(RenderingServer.RENDERING_INFO_BUFFER_MEM_USED),
	}

func _run() -> void:
	var at := _numbers("METRUM_GRADED_PROBE_AT", 2)
	var road := _numbers("METRUM_GRADED_PROBE_ROAD", 4)
	var height_text := OS.get_environment("METRUM_GRADED_PROBE_HEIGHT")
	var height := float(height_text) if not height_text.is_empty() else 150.0
	_expect(DisplayServer.get_name() != "headless", "the probe needs a rendering device")
	if _failures > 0:
		quit(1)
		return
	var scene := Node3D.new()
	root.add_child(scene)
	simulation = SimulationNode.new()
	simulation.name = "SimulationNode"
	scene.add_child(simulation)
	var mods := OS.get_environment("METRUM_GRADED_PROBE_MODS")
	if not mods.is_empty():
		simulation.load_all_asset_packs(mods)
	var load_start_us := Time.get_ticks_usec()
	_expect(simulation.load_game(OS.get_environment("METRUM_GRADED_PROBE_SAVE")), "the save must load")
	_report["load_game_ms"] = float(Time.get_ticks_usec() - load_start_us) / 1000.0
	simulation.set_simulation_speed(0.0)
	await _acknowledge_network()
	var water := Node3D.new()
	water.name = "Water"
	scene.add_child(water)
	var camera := Camera3D.new()
	scene.add_child(camera)
	camera.fov = 70.0
	camera.far = 20000.0
	var focus := _surface(Vector2(at[0], at[1]))
	camera.position = focus + Vector3(0.0, height, height * 0.6)
	camera.look_at(focus, Vector3.UP)
	camera.current = true
	_report["focus"] = [focus.x, focus.y, focus.z]
	_report["empty"] = _memory()
	for graded in [false, true]:
		var mode := "graded" if graded else "exact"
		var start_us := Time.get_ticks_usec()
		var terrain := TerrainScript.new()
		terrain.name = "Terrain"
		terrain.graded_tiles_enabled = graded
		scene.add_child(terrain)
		var entry := {"settled": await _settle(terrain), "settle_ms": float(Time.get_ticks_usec() - start_us) / 1000.0}
		entry.merge(_memory())
		if graded:
			entry["pools"] = terrain.graded_tiles.get_pool_counters()
			entry["edit"] = await _edit(terrain, road)
		_report[mode] = entry
		print("graded_edit_probe %s: %s" % [mode, JSON.stringify(entry)])
		terrain.free()
		for frame in 4:
			await process_frame
	var out := OS.get_environment("METRUM_GRADED_PROBE_OUT")
	if not out.is_empty():
		var file := FileAccess.open(out, FileAccess.WRITE)
		_expect(file != null, "the report must open")
		if file != null:
			file.store_string(JSON.stringify(_report, "\t"))
	print("graded_edit_probe: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# Until the terrain has no pending patch work and, graded, no tile list or bake in flight and no
# count queued, for three frames in a row.
func _settle(terrain) -> bool:
	var stable := 0
	var deadline := Time.get_ticks_msec() + SETTLE_TIMEOUT_MS
	while stable < 3 and Time.get_ticks_msec() < deadline:
		await process_frame
		var graded = terrain.graded_tiles
		var idle: bool = not terrain.has_pending_render_work(false)
		if graded != null:
			idle = idle and graded.requested.is_empty() and graded.patch_lists_requested.is_empty() and graded._counts.is_empty()
		stable = stable + 1 if idle else 0
	_expect(stable >= 3, "the terrain must settle")
	return stable >= 3

# Commits the road, then waits for the edit batch's install frame and the residency to settle.
func _edit(terrain, road: PackedFloat64Array) -> Dictionary:
	var graded = terrain.graded_tiles
	var before: Dictionary = graded.get_pool_counters()
	var points := PackedVector3Array([_surface(Vector2(road[0], road[1])), _surface(Vector2(road[2], road[3]))])
	var commit_start_us := Time.get_ticks_usec()
	var request: int = simulation.add_road_with_snap(points, 1, 1, true)
	var committed := false
	var frames := 0
	while frames < 3000:
		await process_frame
		frames += 1
		var result: Variant = simulation.get_road_commit_result(request)
		if result is Dictionary:
			committed = bool(result.get("committed", false))
			_expect(committed, "the probe's road must commit: %s" % result)
			break
	var commit_ms := float(Time.get_ticks_usec() - commit_start_us) / 1000.0
	await _acknowledge_network()
	var install_frames := 0
	while committed and int(graded.counters["edit_batches"]) == int(before["edit_batches"]) and install_frames < 3000:
		await process_frame
		install_frames += 1
	await _settle(terrain)
	var after: Dictionary = graded.get_pool_counters()
	var created := {}
	for name: String in CREATED:
		created[name] = int(after[name]) - int(before[name])
		_expect(created[name] == 0, "the edit created %d %s" % [created[name], name])
	_expect(int(after["edit_batches"]) > int(before["edit_batches"]), "the edit installs as a batch")
	return {"committed": committed, "commit_ms": commit_ms, "frames_to_batch": install_frames,
		"edit_tiles": int(after["edit_tiles_last"]), "edit_install_ms": float(after["edit_install_ms_last"]),
		"uploads": int(after["uploads"]) - int(before["uploads"]), "created": created, "pools_after": after}
