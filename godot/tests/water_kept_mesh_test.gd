# SPDX-License-Identifier: GPL-3.0-or-later

## Kept water meshes (`TERRAIN-63`, docs/terrain.md "Kept water meshes"). Part one drives the water
## renderer's books with a recording simulation and stream node, deterministically: a kept request
## queued, submitted and applied, each met by a show, a release, a drop and a changed depth; after
## each the script's queues and the recorded mesh jobs agree, a shown kept patch is never a hole, a
## submitted request makes its release busy, and a stale mesh goes in the frame its depth changed.
## Part two runs the game scene on a 20 km blank world with lakes beyond the view: after the
## hand-over every hidden wet patch gets a kept mesh of the coarsest step, a still camera then asks,
## applies and drops nothing, a water re-solve under a kept patch drops its stale mesh in that frame
## and a new one comes when it is wet again, a zoom-out over them meets no hole and draws every
## patch built hidden on its ground's height texture from its show's frame on (`TERRAIN-66`), a tight target drops kept meshes and
## releases patches, no mesh job state outlives its patch, and a terrain edit under a shown water
## patch binds the water to its ground's new height texture in the commit's frame. Part three (round W2) loads that
## world as a player does: the preload hands over only once every hidden wet patch holds its kept
## mesh (the units at `units_total - 1` exactly then), a still camera after it asks, applies and
## drops nothing; at a target with room for only a few kept meshes the load waits for those and
## hands over; and a player's load is cancelled while the preload waits for kept meshes, back at
## the main menu. Headless.
extends SceneTree

const LoadingScreen := preload("res://scripts/ui/loading_screen.gd")

const WaterRenderer := preload("res://scripts/renderers/water.gd")
const SETTLE_TIMEOUT_MS := 90000
const EYE := Vector2(-6000, -6000)
# Basins lowered and filled beyond the view's square at EYE (its far side ends near 3.6 km).
const LAKES: Array[Vector2] = [Vector2(7000, 7000), Vector2(8500, -6000), Vector2(-6000, 8500), Vector2(6500, 1200)]
const LAKE_RADIUS_M := 650.0
const LAKE_DEPTH_M := 40.0
const LAKE_LEVEL_M := -8.0
const KEPT_FRAMES := 3000
const RESOLVE_FRAMES := 240
const SAVE_PATH := "user://water_kept_mesh_test/lakes.sqlite"

# The mesh jobs as the water renderer sees them: requests by key, what a poll hands back.
class RecordingSimulation extends Node:
	var requested := {}
	var finished: Array[Dictionary] = []
	var forgotten: Array[Vector2i] = []

	func request_water_patch_meshes(flat: PackedInt32Array) -> Dictionary:
		for at in range(0, flat.size(), 3):
			requested[Vector2i(flat[at], flat[at + 1])] = flat[at + 2]
		return {"accepted_count": flat.size() / 3}

	func poll_ready_water_patch_meshes(budget: int) -> Dictionary:
		var meshes := []
		while not finished.is_empty() and meshes.size() < budget:
			var mesh: Dictionary = finished.pop_front()
			requested.erase(Vector2i(mesh["patch_x"], mesh["patch_z"]))
			meshes.append(mesh)
		return {"meshes": meshes, "requested_after_count": requested.size()}

	func clear_water_patch_mesh_cache(flat: PackedInt32Array) -> void:
		for at in range(0, flat.size(), 2):
			var key := Vector2i(flat[at], flat[at + 1])
			requested.erase(key)
			forgotten.append(key)

# The stream node as the water renderer calls it: reports recorded, the ledger always has room.
class RecordingStream extends Node:
	var reports: Array[Vector3i] = []
	var kept_bytes := -1

	func water_patch_state(key: Vector2i, state: int) -> void:
		reports.append(Vector3i(key.x, key.y, state))
	func charge_install(_owner: int, _us: float) -> void:
		pass
	func get_install_room_us() -> float:
		return 1000.0
	func set_patch_extras(_layer: int, _bytes: int, _pooled: int) -> void:
		pass
	func set_kept_water_mesh_bytes(bytes: int) -> void:
		kept_bytes = bytes
	func is_terrain_preloading() -> bool:
		return false
	func has_water_pending_work() -> bool:
		return false

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	_test_books()
	await _test_world()
	await _test_preload()
	print("Water kept mesh tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# ---------------------------------------------------------------------------------------------
# Part one: the water renderer's books.

func _books() -> Array:
	var water: Node = WaterRenderer.new()
	var sim := RecordingSimulation.new()
	var stream := RecordingStream.new()
	water.simulation_node = sim
	water._stream = stream
	return [water, sim, stream]

# A built patch at `step`, hidden, wet without a mesh of its cover and depth (signature `depth`).
func _hidden_wet(water: Node, key: Vector2i, step: int, depth: int) -> void:
	var node := MeshInstance3D.new()
	node.mesh = water._empty_water_mesh_resource()
	water.add_child(node)
	water.patches[key] = {"node": node, "material": ShaderMaterial.new(), "lod_step": step,
		"depth_nonzero_count": 10, "cover_generation": 2, "depth_signature": depth,
		"held_mesh": PackedInt64Array(), "texture_width": 4, "texture_height": 4}

func _mesh(key: Vector2i, step: int, depth: int) -> Dictionary:
	return {"patch_x": key.x, "patch_z": key.y, "lod_step": step, "cover_generation": 2,
		"depth_signature": depth, "vertices": PackedVector3Array([Vector3.ZERO, Vector3.RIGHT, Vector3.BACK]),
		"normals": PackedVector3Array([Vector3.UP, Vector3.UP, Vector3.UP]),
		"uvs": PackedVector2Array([Vector2.ZERO, Vector2.RIGHT, Vector2.DOWN]),
		"indices": PackedInt32Array([0, 1, 2]), "mesh_estimated_bytes": 1000}

# One pass of the mesh pipeline (poll, apply, submit).
func _pipeline(water: Node) -> void:
	water._process_mesh_refresh_queue(water.WATER_PATCH_MESH_REQUEST_BUDGET_PER_FRAME)

# Puts `key`'s kept request in `stage`: "queued" (asked, not submitted), "pending" (submitted,
# its job running) or "applied" (its node holds the kept mesh).
func _to_stage(water: Node, sim: RecordingSimulation, key: Vector2i, stage: String) -> void:
	_expect(water.stream_request_meshes(PackedInt32Array([key.x, key.y])).is_empty(), "%s: the request is taken" % stage)
	if stage == "queued":
		return
	_pipeline(water)
	_expect(int(sim.requested.get(key, 0)) == water.KEPT_WATER_STEP and water.mesh_pending_kept.has(key),
		"%s: submitted at the kept step" % stage)
	if stage == "pending":
		return
	sim.finished.append(_mesh(key, water.KEPT_WATER_STEP, int(water.patches[key]["depth_signature"])))
	_pipeline(water)
	_expect(bool(water.patches[key].get("kept_mesh", false)) and water._water_state(water.patches[key]) == 1,
		"applied: the hidden node holds its kept mesh")

# The script's kept books hold nothing for `key`, and the recorded jobs no request.
func _expect_no_kept_work(water: Node, sim: RecordingSimulation, key: Vector2i, what: String) -> void:
	_expect(not water.mesh_kept_requested.has(key) and not water.mesh_pending_kept.has(key),
		"%s: no kept request left" % what)
	_expect(water._kept_mesh_bytes == _kept_bytes(water), "%s: the kept bytes are the kept meshes'" % what)

func _kept_bytes(water: Node) -> int:
	var total := 0
	for patch: Dictionary in water.patches.values():
		if bool(patch.get("kept_mesh", false)):
			total += int(patch.get("mesh_bytes", 0))
	return total

func _test_books() -> void:
	var key := Vector2i(3, 4)
	for stage: String in ["queued", "pending", "applied"]:
		# Shown: the request ends; a kept mesh draws at once (no hole), a finer step is an upgrade.
		for step: int in [8, 4]:
			var parts := _books()
			var water: Node = parts[0]
			var sim: RecordingSimulation = parts[1]
			_hidden_wet(water, key, step, 7)
			_to_stage(water, sim, key, stage)
			var bytes_before: int = water._mesh_bytes
			var answer: Vector2i = water.stream_activate(key, step)
			_expect_no_kept_work(water, sim, key, "%s, shown at step %d" % [stage, step])
			var need: int = water._mesh_need(water.patches[key])
			if stage == "applied":
				_expect(answer.y == 1, "a kept patch shown draws its water at once (state %d)" % answer.y)
				_expect(need == (water.MESH_NEED_NONE if step == 8 else water.MESH_NEED_UPGRADE),
					"shown at step %d: never a hole (need %d)" % [step, need])
				_expect(not bool(water.patches[key].get("kept_mesh", false)) and water._kept_mesh_bytes == 0
					and water._mesh_bytes == bytes_before, "shown, its mesh is an ordinary one")
				_expect(water.mesh_refresh_queue.is_empty(), "no hole queued")
			else:
				_expect(need == water.MESH_NEED_HOLE and water.mesh_refresh_requested_lod.has(key),
					"%s: shown without a mesh it asks for its own step" % stage)
			# Polled after the show: a kept job's mesh is no longer this patch's.
			if stage == "pending":
				sim.finished.append(_mesh(key, water.KEPT_WATER_STEP, 7))
				_pipeline(water)
				_expect(not bool(water.patches[key].get("kept_mesh", false)), "a kept mesh polled after the show is dropped")
			_free_books(parts)
		# Released: a submitted job makes the patch busy; a queued request or an applied mesh goes.
		var parts := _books()
		var water: Node = parts[0]
		var sim: RecordingSimulation = parts[1]
		_hidden_wet(water, key, 8, 7)
		_to_stage(water, sim, key, stage)
		var released: int = water.stream_release(key)
		if stage == "pending":
			_expect(released == 3, "a release while the kept job runs is busy (answered %d)" % released)
			sim.finished.append(_mesh(key, water.KEPT_WATER_STEP, 7))
			_pipeline(water)
			released = water.stream_release(key)
		_expect(released == 0 and not water.patches.has(key), "%s: released" % stage)
		_expect(sim.forgotten.has(key), "%s: the released patch's mesh jobs are forgotten" % stage)
		_expect_no_kept_work(water, sim, key, "%s, released" % stage)
		_expect(water._mesh_bytes == 0 and water._kept_mesh_bytes == 0, "%s, released: no bytes left" % stage)
		_free_books(parts)
		# Dropped: only a kept mesh goes; the node draws nothing and its bytes are freed.
		parts = _books()
		water = parts[0]
		sim = parts[1]
		_hidden_wet(water, key, 8, 7)
		_to_stage(water, sim, key, stage)
		var freed: int = water.stream_drop_mesh(key)
		if stage == "applied":
			_expect(freed == 1000 and water._water_state(water.patches[key]) == 2, "a kept mesh dropped frees its bytes")
		else:
			_expect(freed == -1, "%s: nothing to drop" % stage)
		_expect(water._mesh_bytes == 0 or stage != "applied", "dropped: no mesh bytes left")
		_free_books(parts)
		# The depth changed (a terrain edit, a water re-solve): the request and any stale mesh go in
		# that call, and the state is reported even when it did not change.
		for wet_after: bool in [true, false]:
			parts = _books()
			water = parts[0]
			sim = parts[1]
			var stream: RecordingStream = parts[2]
			_hidden_wet(water, key, 8, 7)
			_to_stage(water, sim, key, stage)
			stream.reports.clear()
			water.patch_payload_ready[key] = {"texture_width": 4, "texture_height": 4, "sample_width": 2,
				"sample_height": 2, "inner_offset_x": 1, "inner_offset_z": 1, "world_size_x": 512.0,
				"world_size_z": 512.0, "world_origin_x": 0.0, "world_origin_z": 0.0, "cover_generation": 2,
				"depth_signature": 9, "depth_nonzero_count": 10 if wet_after else 0,
				"depth_bytes": PackedFloat32Array([1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
					1.0, 1.0, 1.0, 1.0]).to_byte_array()}
			water.patches[key]["depth_image"] = Image.create(4, 4, false, Image.FORMAT_RF)
			water.patches[key]["depth_texture"] = ImageTexture.create_from_image(water.patches[key]["depth_image"])
			_expect(water._upload_patch(key, true), "the changed payload uploads")
			var what := "%s, depth changed (%s)" % [stage, "wet" if wet_after else "dry"]
			_expect_no_kept_work(water, sim, key, what)
			_expect(water.patches[key]["held_mesh"].is_empty() and int(water.patches[key].get("mesh_bytes", 0)) == 0,
				"%s: no stale mesh left on the hidden node" % what)
			_expect(stream.reports.size() == 1 and stream.reports[0] == Vector3i(key.x, key.y, 2 if wet_after else 0),
				"%s: the state reported once (%s)" % [what, stream.reports])
			# A job of the old depth that still comes back is dropped.
			if stage == "pending":
				sim.finished.append(_mesh(key, water.KEPT_WATER_STEP, 7))
				_pipeline(water)
				_expect(water.patches[key]["held_mesh"].is_empty(), "%s: the old job's mesh is dropped" % what)
			_free_books(parts)
	# A request the script cannot take is answered with the patch's state.
	var parts := _books()
	var water: Node = parts[0]
	_hidden_wet(water, key, 8, 7)
	water.resident_patch_lookup[key] = true
	_expect(water.stream_request_meshes(PackedInt32Array([key.x, key.y, 9, 9])) == PackedInt32Array([key.x, key.y, 2, 9, 9, -1]),
		"a shown or unbuilt patch's request is refused with its state")
	_free_books(parts)

func _free_books(parts: Array) -> void:
	(parts[0] as Node).free()
	(parts[1] as Node).free()
	(parts[2] as Node).free()

# ---------------------------------------------------------------------------------------------
# Part two: the game scene.

func _test_world() -> void:
	var main: Node = await _lake_world()
	var sim: Node = main.get_node("SimulationNode")
	var input: Node = main.get_node("InputManager")
	var camera: Node = main.get_node("CameraNode")
	# Saved for part three, which loads it as a player does.
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(SAVE_PATH.get_base_dir()))
	_expect(sim.save_game(ProjectSettings.globalize_path(SAVE_PATH)), "the lakes' world must save")
	var stream: Node = main.get_node("StreamNode")
	var terrain: Node = main.get_node("Terrain")
	var water: Node = main.get_node("Water")
	input._refresh_after_world_load()
	# No vegetation: on this headless blank world its products never settle, and the budget's fill
	# and the kept meshes' requests both wait for a vegetation the view waits for.
	stream.set_vegetation_enabled(false)
	camera.focus_on(Vector3(EYE.x, sim.get_world_surface_height(EYE), EYE.y), 120.0)
	await _frames_until_settled(camera, terrain, "the load")
	terrain.set_load_settling(false)
	await _frames_until_settled(camera, terrain, "the hand-over")
	# The background fill of kept meshes.
	var frames := await _frames_until_kept(camera, stream, water)
	var kept := _kept_keys(water)
	print("[water_kept_mesh] kept meshes %d after %d frames; hidden wet %d; %s" % [kept.size(), frames,
		_hidden_wet_count(water), _budget_line(stream)])
	_expect(kept.size() >= LAKES.size(), "the lakes beyond the view hold kept meshes (%d)" % kept.size())
	_expect(kept.size() == _hidden_wet_count(water), "every hidden wet patch holds a kept mesh")
	_check_books(stream, water, sim, "after the kept fill")
	# A still camera asks, applies and drops nothing for 120 frames.
	var budget: Dictionary = stream.get_terrain_budget()
	var bytes: int = water._kept_mesh_bytes
	for frame in 120:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
	var still: Dictionary = stream.get_terrain_budget()
	for field: String in ["kept_asked_total", "kept_dropped_total", "kept_water_meshes", "kept_water_mesh_bytes"]:
		_expect(still.get(field) == budget.get(field), "a still camera leaves %s alone (%s, %s)" % [field, budget.get(field), still.get(field)])
	_expect(water._kept_mesh_bytes == bytes, "a still camera applies nothing")
	# A target that leaves the places as they are but not the kept meshes' room: kept meshes are
	# dropped, farthest first, until they fit; nothing is released. Every patch fitting again, they
	# are asked for again.
	await _frames_until_kept(camera, stream, water)
	var before: Dictionary = stream.get_terrain_budget()
	var held_bytes := int(before.get("kept_water_mesh_bytes", 0))
	var target := int(before.get("counted_bytes", 0)) + held_bytes / 2
	stream.set_terrain_budget_mb(float(target) / 1e6)
	for frame in 30:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
	var squeezed: Dictionary = stream.get_terrain_budget()
	print("[water_kept_mesh] squeezed to %d bytes: %s" % [target, _budget_line(stream)])
	_expect(int(squeezed.get("kept_dropped_total", 0)) > int(before.get("kept_dropped_total", 0)), "kept meshes are dropped over the target")
	_expect(int(squeezed.get("counted_bytes", 0)) + int(squeezed.get("kept_water_mesh_bytes", 0)) <= int(squeezed.get("budget_bytes", 0)),
		"the kept meshes fit the room the places leave")
	_expect(squeezed.get("places") == before.get("places") or int(squeezed.get("ring", 0)) == int(before.get("ring", 0)),
		"the places' ring is the budget's own (%s, %s)" % [before.get("ring"), squeezed.get("ring")])
	_expect(int(squeezed.get("water_releases_total", 0)) == int(before.get("water_releases_total", 0)), "no patch is released for a kept mesh")
	_check_books(stream, water, sim, "squeezed")
	# The farthest went: every patch still holding a kept mesh is no farther than every dropped one.
	var eye_3d: Vector3 = main.get_viewport().get_camera_3d().global_position
	var eye := Vector2(eye_3d.x, eye_3d.z)
	var nearest_dropped := INF
	var farthest_kept := 0.0
	for key: Vector2i in kept:
		var centre := Vector2((key.x + 0.5) * 512.0 - 10240.0, (key.y + 0.5) * 512.0 - 10240.0)
		var distance := centre.distance_squared_to(eye)
		if bool(water.patches.get(key, {}).get("kept_mesh", false)):
			farthest_kept = maxf(farthest_kept, distance)
		elif int(water.patches.get(key, {}).get("depth_nonzero_count", 0)) > 0:
			nearest_dropped = minf(nearest_dropped, distance)
	_expect(farthest_kept <= nearest_dropped, "the farthest kept meshes go first")
	stream.set_terrain_budget_mb(0.0)
	await _frames_until_kept(camera, stream, water)
	_expect(_kept_keys(water).size() == _hidden_wet_count(water), "every patch fitting again, every hidden wet patch keeps a mesh")
	# A zoom-out over the kept patches: none of them is a hole when shown; then back in.
	var view: PackedFloat32Array = camera.get_view_state()
	view[5] = 16000.0
	_expect(camera.set_view_state(view), "the view state must apply")
	var holes := 0
	var shown := 0
	var kept_now := _kept_keys(water)
	# The patches built hidden before the zoom (`TERRAIN-66`): in the frame that shows one, its
	# material reads its terrain patch's height texture, not the zero fallback it may have been
	# built with. Checked at the next frame's start, before any node processes, so it reads what the
	# show's frame drew (headless runs draw nothing, so there is no frame_post_draw).
	var built_hidden: Array[Vector2i] = []
	for key: Vector2i in water.patches:
		if not water.resident_patch_lookup.has(key):
			built_hidden.append(key)
	var first_shows := {}
	var unbound := 0
	var start := Time.get_ticks_msec()
	var stable := 0
	while stable < 10 and Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		for key: Vector2i in kept_now:
			if water.resident_patch_lookup.has(key):
				shown += 1
				holes += int(water._mesh_need(water.patches[key]) == water.MESH_NEED_HOLE)
		for key: Vector2i in built_hidden:
			if water.resident_patch_lookup.has(key) and not first_shows.has(key):
				first_shows[key] = true
				var bound: Variant = (water.patches[key]["material"] as ShaderMaterial).get_shader_parameter("heightmap")
				unbound += int(bound != terrain.get_patch_height_texture(key) or bound == water.fallback_height_texture)
		stable = stable + 1 if terrain.is_render_settled() else 0
	print("[water_kept_mesh] zoom-out: kept patches shown %d patch-frames, holes %d; hidden-built patches shown %d, on another height texture than their ground's in their show's frame %d"
		% [shown, holes, first_shows.size(), unbound])
	_expect(shown > 0, "the zoom-out shows kept patches")
	_expect(holes == 0, "a kept patch shown is never a hole (%d)" % holes)
	_expect(first_shows.size() > 0, "the zoom-out shows patches built hidden")
	_expect(unbound == 0, "a hidden-built patch draws its show's frame on its ground's height texture (%d of %d did not)"
		% [unbound, first_shows.size()])
	view[5] = 400.0
	_expect(camera.set_view_state(view), "the view state must apply")
	await _frames_until_settled(camera, terrain, "the zoom back in")
	await _frames_until_kept(camera, stream, water)
	_check_books(stream, water, sim, "after the zoom")
	# A tight target: kept meshes are dropped (farthest first) and far patches released; no mesh
	# job state outlives a released patch.
	stream.set_terrain_budget_mb(30.0)
	await _frames_until_settled(camera, terrain, "the tight target")
	for frame in 300:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
	var tight: Dictionary = stream.get_terrain_budget()
	print("[water_kept_mesh] tight target: %s" % _budget_line(stream))
	_expect(int(tight.get("water_releases_total", 0)) > 0, "the tight target releases water patches")
	_expect(int(tight.get("counted_bytes", 0)) + int(tight.get("kept_water_mesh_bytes", 0)) <= int(tight.get("budget_bytes", 0))
		or int(tight.get("kept_water_meshes", 0)) == 0, "kept meshes fit the room the places leave")
	_check_books(stream, water, sim, "at the tight target")
	# Every patch fitting again: the fill and the kept meshes come back. Then a water re-solve under
	# a kept hidden patch (last: a whole-map re-solve keeps the water's uploads busy for a while).
	stream.set_terrain_budget_mb(0.0)
	await _frames_until_kept(camera, stream, water)
	await _edit_under_kept(camera, stream, water, sim, _kept_keys(water))
	await _edit_under_shown(camera, terrain, water, sim)
	main.queue_free()
	await process_frame

# The game scene on a 20 km blank world with LAKES dug and filled beyond the view before the
# renderers are built (as a world that comes with them); the edits' road publication is
# acknowledged, as no renderer drew the world yet. The test drives the camera and the load's hold
# itself (the input manager's and the camera's processing off).
func _lake_world() -> Node:
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	for node: Node in [main.get_node("InputManager"), main.get_node("CameraNode")]:
		node.set_process(false)
		node.set_process_input(false)
		node.set_process_unhandled_input(false)
	var sim: Node = main.get_node("SimulationNode")
	_expect(sim.create_blank_world(20480.0, 20480.0, 8.0, 0.0), "the 20 km blank world must load")
	sim.set_simulation_speed(0.0)
	for lake: Vector2 in LAKES:
		sim.sculpt_terrain(lake, LAKE_RADIUS_M, -LAKE_DEPTH_M)
		sim.begin_world_lake_fill_preview(lake, LAKE_LEVEL_M)
		_expect(sim.commit_world_lake_fill_preview(), "the lake at %s must commit" % lake)
	for frame in 600:
		if not sim.is_network_dirty():
			break
		sim.acknowledge_network_render(sim.get_road_tool_surface_generation())
		await process_frame
	_expect(not sim.is_network_dirty(), "the edits' publication acknowledged")
	return main

# ---------------------------------------------------------------------------------------------
# Part three (`TERRAIN-63` round W2): the load's preload waits for the kept meshes.

func _test_preload() -> void:
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	var sim: Node = main.get_node("SimulationNode")
	var stream: Node = main.get_node("StreamNode")
	var water: Node = main.get_node("Water")
	var camera: Node = main.get_node("CameraNode")
	var input: Node = main.get_node("InputManager")
	var path := ProjectSettings.globalize_path(SAVE_PATH)
	# A player's load of the whole map: every hidden wet patch holds its kept mesh at the hand-over.
	_expect(await input.player_load(input.LoadKind.SAVE, path), "the lakes' save must load")
	camera.focus_on(Vector3(EYE.x, sim.get_world_surface_height(EYE), EYE.y), 120.0)
	var whole: Dictionary = await _preload_until_hand_over(main, "whole map")
	_expect(int(whole.get("kept_water_meshes", -1)) == _hidden_wet_count(water) and _hidden_wet_count(water) >= LAKES.size(),
		"whole map: every hidden wet patch holds a kept mesh at the hand-over (%s of %d)" % [whole.get("kept_water_meshes"), _hidden_wet_count(water)])
	_check_books(stream, water, sim, "the preload's hand-over")
	# A still camera: nothing asked, applied or dropped.
	for frame in 120:
		await process_frame
	var still: Dictionary = stream.get_terrain_budget()
	for field: String in ["kept_asked_total", "kept_dropped_total", "kept_water_meshes", "kept_water_mesh_bytes"]:
		_expect(still.get(field) == whole.get(field), "after the preload a still camera leaves %s alone (%s, %s)" % [field, whole.get(field), still.get(field)])
	# A target set at the load's start, 3 kept meshes above what the whole map counted: the load
	# hands over with nothing waited for that the target admits.
	var target := int(whole.get("counted_bytes", 0)) + 3 * int(whole.get("kept_mesh_max_bytes", 0))
	_expect(input.menu_load_game_from_path(path, true), "the lakes' save must load again")
	camera.focus_on(Vector3(EYE.x, sim.get_world_surface_height(EYE), EYE.y), 120.0)
	stream.set_terrain_budget_mb(float(target) / 1e6)
	var tight: Dictionary = await _preload_until_hand_over(main, "a target")
	# (The places' hysteresis leaves at least two rings of room below a target the view fits, so
	# every hidden wet patch usually fits; a full target is the book's Rust test's.)
	_expect(int(tight.get("counted_bytes", 0)) + int(tight.get("kept_water_mesh_bytes", 0)) <= int(tight.get("budget_bytes", 0)),
		"a target: the kept meshes fit the room the places leave")
	stream.set_terrain_budget_mb(0.0)
	# A player's load cancelled while the preload waits for kept meshes: back at the main menu.
	await input.player_load(input.LoadKind.SAVE, path)
	camera.focus_on(Vector3(EYE.x, sim.get_world_surface_height(EYE), EYE.y), 120.0)
	var cancelled := false
	var start := Time.get_ticks_msec()
	while not cancelled and not input.is_world_ready() and Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		await process_frame
		if bool(stream.get_terrain_preload().get("kept_waits", false)):
			cancelled = true
			LoadingScreen.current(self).cancel_requested.emit()
	_expect(cancelled, "the player's load waited for kept meshes under the screen")
	for frame in 30:
		await process_frame
		if current_scene != null and current_scene.scene_file_path == "res://scenes/MainMenu.tscn" \
				and LoadingScreen.current(self) == null:
			break
	_expect(current_scene != null and current_scene.scene_file_path == "res://scenes/MainMenu.tscn"
		and LoadingScreen.current(self) == null, "a cancel in the kept meshes' wait returns to the main menu, the screen gone")

# Frames of a load's preload until its hand-over, each checked: the loading screen's fraction never
# goes back and is 1 exactly from the hand-over; the units reach `units_total - 1` exactly when the
# preload is done, and done only with no kept mesh waited for; at the hand-over nothing is wanting
# within the target nor asked. Returns the budget's status at the hand-over.
func _preload_until_hand_over(main: Node, what: String) -> Dictionary:
	var input: Node = main.get_node("InputManager")
	var stream: Node = main.get_node("StreamNode")
	var start := Time.get_ticks_msec()
	var waited := 0
	var status: Dictionary = {}
	var fraction := 0.0
	while not input.is_world_ready() and Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		await process_frame
		var shown: float = input.load_fraction()
		_expect(shown >= fraction and (shown == 1.0) == input.is_world_ready(),
			"%s: the fraction %f after %f, world ready %s" % [what, shown, fraction, input.is_world_ready()])
		fraction = shown
		var state: Dictionary = stream.get_terrain_preload()
		status = stream.get_terrain_budget()
		var total := int(state.get("units_total", 0))
		if not stream.is_terrain_preloading() or total == 0:
			continue
		var done := bool(state.get("done", false))
		var kept_waits := bool(state.get("kept_waits", true))
		waited += int(kept_waits)
		_expect((int(state.get("units_done", -1)) == total - 1) == done, "%s: units %s of %d, done %s" % [what, state.get("units_done"), total, done])
		_expect(not (done and kept_waits), "%s: done while kept meshes are waited for" % what)
	_expect(input.is_world_ready(), "%s: the preload hands over" % what)
	print("[water_kept_mesh] preload %s: %d frames waited for kept meshes; at the hand-over %s" % [what, waited, _budget_line(stream)])
	_expect(waited > 0, "%s: the preload waited for kept meshes" % what)
	_expect(int(status.get("kept_asked", -1)) == 0, "%s: no kept request out at the hand-over" % what)
	var room := (int(status.get("budget_bytes", -1)) < 0 or int(status.get("counted_bytes", 0))
		+ int(status.get("kept_water_mesh_bytes", 0)) + int(status.get("kept_mesh_max_bytes", 0)) <= int(status.get("budget_bytes", 0)))
	_expect(int(status.get("kept_wanting", -1)) == 0 or not room, "%s: nothing wanting within the target at the hand-over (%s)" % [what, _budget_line(stream)])
	return status

# A water re-solve under a kept hidden patch (`TERRAIN-38`'s path for a terrain edit too): its lake
# drained, then filled again. In the frame its depth changes its stale mesh is gone; wet again, a kept
# mesh of the new depth comes. (A terrain edit under hidden patches is not driven here: in this
# headless scene its road publication can stay unacknowledged and hold the water's payloads back;
# part one drives the same `_upload_patch` rule.)
func _edit_under_kept(camera: Node, stream: Node, water: Node, sim: Node, kept: Array[Vector2i]) -> void:
	var lake := LAKES[0]
	var key := Vector2i(floori((lake.x + 10240.0) / 512.0), floori((lake.y + 10240.0) / 512.0))
	_expect(kept.has(key), "the lake's centre patch %s holds a kept mesh" % key)
	for edit: String in ["drain", "refill"]:
		var signature := int(water.patches[key].get("depth_signature", 0))
		match edit:
			"drain":
				_expect(sim.remove_world_lake_fill_near(lake, 100.0), "the lake's fill is removed")
			"refill":
				sim.begin_world_lake_fill_preview(lake, LAKE_LEVEL_M)
				_expect(sim.commit_world_lake_fill_preview(), "the lake fills again")
		var changed := -1
		for frame in KEPT_FRAMES:
			camera.advance_motion(1.0 / 60.0)
			await process_frame
			var patch: Dictionary = water.patches.get(key, {})
			if patch.is_empty():
				continue
			if changed < 0 and int(patch.get("depth_signature", 0)) != signature:
				changed = frame
				var held: PackedInt64Array = patch.get("held_mesh", PackedInt64Array())
				_expect(held.is_empty() and int(patch.get("mesh_bytes", 0)) == 0,
					"%s: the stale mesh goes in the frame the depth changed" % edit)
			# Done once its water draws or, a whole-map re-solve keeping the frames' ledger busy,
			# RESOLVE_FRAMES after the change.
			if changed >= 0 and (water._water_state(patch) != 2 or frame - changed >= RESOLVE_FRAMES):
				break
		var patch: Dictionary = water.patches.get(key, {})
		_expect(changed >= 0, "%s: the patch's depth changed" % edit)
		var wet := int(patch.get("depth_nonzero_count", 0)) > 0
		var budget: Dictionary = stream.get_terrain_budget()
		print("[water_kept_mesh] %s: depth changed at frame %d, wet %s, state %d, water dirty %s; %s" % [edit, changed, wet,
			water._water_state(patch), sim.is_water_dirty(), _budget_line(stream)])
		_expect(wet == (edit == "refill"), "%s: the lake is %s" % [edit, "wet" if edit == "refill" else "dry"])
		if wet:
			# The book asks again: its mesh came, or it is wanted or asked for while the frames have
			# no install ledger room left (a whole-map re-solve re-uploads every water patch, and
			# a debug build's frames spend the ledger on it for thousands of frames).
			var came: bool = water._water_state(patch) == 1 and bool(patch.get("kept_mesh", false))
			var waits := ((int(budget.get("kept_wanting", 0)) > 0 or int(budget.get("kept_asked", 0)) > 0)
				and int(budget.get("kept_held", 0)) & 32 != 0)
			_expect(came or waits, "%s: a kept mesh of the new depth came, or the book wants it and waits for ledger room" % edit)
		_check_books(stream, water, sim, "after the %s" % edit)

# A terrain edit under the eye's shown water patch (`TERRAIN-66`): the terrain's commit gives the
# patch another height texture (the old one becomes the spare its next stage writes into), and the
# water over it reads the new one from that frame on (checked at the next frame's start, as above).
func _edit_under_shown(camera: Node, terrain: Node, water: Node, sim: Node) -> void:
	var key := Vector2i(floori((EYE.x + 10240.0) / 512.0), floori((EYE.y + 10240.0) / 512.0))
	_expect(water.resident_patch_lookup.has(key), "the eye's water patch %s is shown" % key)
	var before: Texture2D = terrain.get_patch_height_texture(key)
	sim.sculpt_terrain(EYE, 60.0, 2.0)
	var replaced := -1
	for frame in KEPT_FRAMES:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		if terrain.get_patch_height_texture(key) != before:
			replaced = frame
			break
	var bound: Variant = (water.patches.get(key, {}).get("material") as ShaderMaterial).get_shader_parameter("heightmap") \
		if water.patches.has(key) else null
	print("[water_kept_mesh] edit under shown patch %s: height texture replaced at frame %d, the water on the new one %s"
		% [key, replaced, bound == terrain.get_patch_height_texture(key)])
	_expect(replaced >= 0, "the edit's commit replaces the eye's patch's height texture")
	_expect(water.resident_patch_lookup.has(key) and bound == terrain.get_patch_height_texture(key),
		"the shown water reads its ground's new height texture from the commit's frame on")

# Frames until the budget's fill has nothing left, the kept fill nothing wanting or asked and the
# script nothing queued.
func _frames_until_kept(camera: Node, stream: Node, water: Node) -> int:
	for frame in KEPT_FRAMES:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		var budget: Dictionary = stream.get_terrain_budget()
		if (int(budget.get("kept_wanting", 0)) == 0 and int(budget.get("kept_asked", 0)) == 0
				and int(budget.get("fill_left", 0)) == 0
				and water.mesh_kept_requested.is_empty() and water.mesh_pending_kept.is_empty()
				and water.mesh_kept_apply_queue.is_empty()):
			return frame
	_expect(false, "the kept fill must end within %d frames (%s)" % [KEPT_FRAMES, _budget_line(stream)])
	return KEPT_FRAMES

func _kept_keys(water: Node) -> Array[Vector2i]:
	var keys: Array[Vector2i] = []
	for key: Vector2i in water.patches:
		if bool(water.patches[key].get("kept_mesh", false)):
			keys.append(key)
	return keys

func _hidden_wet_count(water: Node) -> int:
	var count := 0
	for key: Vector2i in water.patches:
		if not water.resident_patch_lookup.has(key) and int(water.patches[key].get("depth_nonzero_count", 0)) > 0:
			count += 1
	return count

# The stream node's book, the script's queues and the Rust mesh jobs agree; every kept mesh is of
# the kept step and of its patch's cover and depth; no cached mesh belongs to an unbuilt patch.
func _check_books(stream: Node, water: Node, sim: Node, what: String) -> void:
	var budget: Dictionary = stream.get_terrain_budget()
	var asked := int(budget.get("kept_asked", 0))
	var script_asked: int = water.mesh_kept_requested.size() + water.mesh_pending_kept.size() + water.mesh_kept_apply_queue.size()
	_expect(asked == script_asked, "%s: the book's asked %d, the script's %d" % [what, asked, script_asked])
	_expect(int(budget.get("kept_water_meshes", 0)) == _kept_keys(water).size(),
		"%s: the book's kept %s, the script's %d" % [what, budget.get("kept_water_meshes"), _kept_keys(water).size()])
	_expect(int(budget.get("kept_water_mesh_bytes", 0)) == water._kept_mesh_bytes and water._kept_mesh_bytes == _kept_bytes(water),
		"%s: kept bytes %s, script %d, summed %d" % [what, budget.get("kept_water_mesh_bytes"), water._kept_mesh_bytes, _kept_bytes(water)])
	_expect(int(budget.get("kept_refused_total", 0)) == 0, "%s: no request refused (books in step)" % what)
	for key: Vector2i in _kept_keys(water):
		var patch: Dictionary = water.patches[key]
		var held: PackedInt64Array = patch.get("held_mesh", PackedInt64Array())
		_expect(not water.resident_patch_lookup.has(key) and held.size() == 3 and held[0] == water.KEPT_WATER_STEP
			and held[1] == int(patch.get("cover_generation", 0)) and held[2] == int(patch.get("depth_signature", 0)),
			"%s: kept patch %s holds the kept step's mesh of its cover and depth" % [what, key])
	var jobs: Dictionary = sim.get_water_mesh_job_counts()
	var cached: PackedInt32Array = jobs.get("cache_patches", PackedInt32Array())
	for at in range(0, cached.size(), 2):
		var key := Vector2i(cached[at], cached[at + 1])
		_expect(water.patches.has(key), "%s: a cached mesh of unbuilt patch %s (a leaked entry)" % [what, key])
	# Every job ends: none runs for a patch the script no longer waits on. (The jobs' `requested`
	# may keep a hidden patch's hole request the script dropped at its hide, as before.)
	if script_asked == 0 and water.mesh_pending_lod.is_empty():
		_expect(int(jobs.get("in_flight", -1)) == 0, "%s: no mesh job left running (%s)" % [what, jobs])
	print("[water_kept_mesh] %s: jobs in flight %d, requested %d, ready %d, cache %d (%d patches)" % [what,
		int(jobs.get("in_flight", 0)), int(jobs.get("requested", 0)), int(jobs.get("ready", 0)),
		int(jobs.get("cache", 0)), cached.size() / 2])

func _budget_line(stream: Node) -> String:
	var budget: Dictionary = stream.get_terrain_budget()
	return "held %d, counted %.1f MB, kept %d (%.2f MB), wanting %d, asked %d, asked total %d, dropped %d, places %s, ring %s, water built %d, releases %d" % [int(budget.get("kept_held", -1)),
		float(budget.get("counted_bytes", 0)) / 1e6, int(budget.get("kept_water_meshes", 0)),
		float(budget.get("kept_water_mesh_bytes", 0)) / 1e6, int(budget.get("kept_wanting", 0)),
		int(budget.get("kept_asked", 0)), int(budget.get("kept_asked_total", 0)), int(budget.get("kept_dropped_total", 0)),
		str(budget.get("places", "-")), str(budget.get("ring", "-")), int(budget.get("water_built", 0)),
		int(budget.get("water_releases_total", 0))]

func _frames_until_settled(camera: Node, terrain: Node, what: String) -> void:
	var start := Time.get_ticks_msec()
	var stable := 0
	while stable < 10 and Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		stable = stable + 1 if terrain.is_render_settled() else 0
	_expect(stable >= 10, "%s must settle" % what)
