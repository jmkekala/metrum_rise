# SPDX-License-Identifier: GPL-3.0-or-later

## The backdrop's hand-over as the renderer sees it (`TERRAIN-41` D3, docs/terrain.md "The backdrop
## drawn"): on a fresh 20 km blank world, through a jump, a far zoom-out and a zoom back in, after
## every frame each patch of the map is drawn by exactly one of its patch node and its backdrop
## cell. A patch node draws when it is visible in the tree, has a mesh and its PlaneMesh is not
## collapsed by the graded tile book (which collapses a resident patch's PlaneMesh while its list of
## graded tiles is not known: T53 (a), "nothing until then"); a cell draws when the stream node's
## whole-map probe reports it covered. The resident patches drawing nothing must be covered by their
## cells, and the run must meet some. Then the same world under water (`TERRAIN-58`: an open water
## fill over the whole map, then a jump and the zoom again): a terrain patch's node never draws while
## its wet water patch does not draw (no water node, hidden, no mesh of its cover and depth, or bound to
## another height texture than its ground's, `TERRAIN-66`): its
## cell draws instead, and the run must meet such patches. One patch's water payload is refused at
## the start: its ground must not wait for it (drawn as a patch once resident). Headless; the game
## scene as the load flow builds it, without its input and camera processing.
extends SceneTree

const SETTLE_TIMEOUT_MS := 90000
const EYE := Vector2(-6000, -6000)
const JUMP := Vector2(6000, 5000)
# The wet phase's jump, and the patch under it whose water payload is refused (40 x 40 patches of
# 512 m on the 20,480 m map).
const WET_JUMP := Vector2(-2000, 7000)
const REFUSED := Vector2i(16, 33)
const WET_FRAMES := 600
# A collapsed PlaneMesh's visibility range (the graded book's HIDDEN_RANGE_M).
const COLLAPSED_RANGE_M := 0.01

var _failures := 0
var _frames := 0
var _neither := 0
var _both := 0
var _blank_covered := 0
var _first_failure := ""
# The wet phase (`TERRAIN-58`): patch-frames of a drawn terrain patch whose wet water does not
# draw, and of a held patch (its own draw waiting for its water; the check above has its cell
# drawing in its place).
var _wet := false
var _bed := 0
var _held_covered := 0
var _first_bed := ""

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	var input: Node = main.get_node("InputManager")
	input.set_process(false)
	input.set_process_input(false)
	input.set_process_unhandled_input(false)
	var camera: Node = main.get_node("CameraNode")
	camera.set_process(false)
	camera.set_process_input(false)
	camera.set_process_unhandled_input(false)
	var sim: Node = main.get_node("SimulationNode")
	var stream: Node = main.get_node("StreamNode")
	var terrain: Node = main.get_node("Terrain")
	_expect(sim.create_blank_world(20480.0, 20480.0, 8.0, 0.0), "the 20 km blank world must load")
	sim.set_simulation_speed(0.0)
	input._refresh_after_world_load()
	stream.water_payload(REFUSED, 2)
	camera.focus_on(Vector3(EYE.x, sim.get_world_surface_height(EYE), EYE.y), 120.0)
	await _frames_until_settled(camera, stream, terrain, "the load")
	terrain.set_load_settling(false)
	await _frames_until_settled(camera, stream, terrain, "the hand-over")
	# A jump: the near patches arrive at once, each without its graded list.
	camera.focus_on(Vector3(JUMP.x, sim.get_world_surface_height(JUMP), JUMP.y), 120.0)
	await _frames_until_settled(camera, stream, terrain, "the jump")
	# A far zoom-out and back: the square grows over the map, then shrinks.
	var view: PackedFloat32Array = camera.get_view_state()
	for distance: float in [12000.0, 400.0]:
		view[5] = distance
		_expect(camera.set_view_state(view), "the view state must apply")
		await _frames_until_settled(camera, stream, terrain, "the zoom to %.0f m" % distance)
	# The world under water, first left WET_FRAMES frames (a drawn patch that turns wet keeps its
	# ground while its new mesh comes), then a jump and the zoom with every patch's water checked.
	# A whole map flooded at once does not settle within the timeout on `dev` either (the water's
	# uploads keep the terrain's water texture syncs queued), so the wet phases run WET_FRAMES
	# frames each instead of waiting for the settle.
	sim.begin_world_open_water_fill_preview(Vector2.ZERO, 5.0)
	_expect(sim.commit_world_open_water_fill_preview(), "the open water fill must commit")
	await _wet_frames(camera, stream, terrain, "the fill")
	_wet = true
	camera.focus_on(Vector3(WET_JUMP.x, sim.get_world_surface_height(WET_JUMP), WET_JUMP.y), 120.0)
	await _wet_frames(camera, stream, terrain, "the wet jump")
	for distance: float in [12000.0, 400.0]:
		view = camera.get_view_state()
		view[5] = distance
		_expect(camera.set_view_state(view), "the view state must apply")
		await _wet_frames(camera, stream, terrain, "the wet zoom to %.0f m" % distance)
	var refused: Dictionary = terrain.patches.get(REFUSED, {})
	var refused_node := refused.get("node") as GeometryInstance3D
	_expect(terrain.resident_patch_lookup.has(REFUSED) and refused_node != null and refused_node.is_visible_in_tree()
		and not (refused_node.visibility_range_end > 0.0 and refused_node.visibility_range_end < COLLAPSED_RANGE_M),
		"the patch whose water payload was refused is drawn as a patch")
	var water: Node = main.get_node("Water")
	_expect(not water.patches.has(REFUSED), "the refused water payload was never built (the case ran)")
	print("[terrain_backdrop_handover] wet: lake beds drawn %d, held patches covered %d%s"
		% [_bed, _held_covered, (", first: " + _first_bed) if not _first_bed.is_empty() else ""])
	_expect(_bed == 0, "no terrain patch draws while its wet water does not (%d, first: %s)" % [_bed, _first_bed])
	_expect(_held_covered > 0, "the run met patches held for their water, covered by their cells")
	print("[terrain_backdrop_handover] frames %d, neither %d, both %d, resident patches drawing nothing covered %d%s"
		% [_frames, _neither, _both, _blank_covered, (", first: " + _first_failure) if not _first_failure.is_empty() else ""])
	_expect(_neither == 0, "no patch drawn by neither its node nor its cell (%d, first: %s)" % [_neither, _first_failure])
	_expect(_both == 0, "no patch drawn by both its node and its cell (%d, first: %s)" % [_both, _first_failure])
	_expect(_blank_covered > 0, "the run met resident patches drawing nothing, covered by their cells")
	main.queue_free()
	await process_frame
	print("Terrain backdrop hand-over tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# Frames until the terrain settles, each checked as drawn: a frame's check runs at the next frame's
# start, before any node processes, so it reads the state the frame drew.
func _frames_until_settled(camera: Node, stream: Node, terrain: Node, what: String) -> void:
	var start := Time.get_ticks_msec()
	var stable := 0
	while stable < 10 and Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		_check(stream, terrain, what)
		stable = stable + 1 if terrain.is_render_settled() else 0
	_expect(stable >= 10, "%s must settle" % what)

# WET_FRAMES frames, each checked as drawn (as above).
func _wet_frames(camera: Node, stream: Node, terrain: Node, what: String) -> void:
	for frame in WET_FRAMES:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		_check(stream, terrain, what)

func _check(stream: Node, terrain: Node, what: String) -> void:
	_frames += 1
	var patches: Dictionary = terrain.patches
	var water: Node = terrain.get_parent().get_node("Water")
	if _wet:
		_held_covered += int(stream.get_backdrop().get("held", 0))
	var map: PackedFloat32Array = stream.probe_stream_layer("terrain_map")
	for at in range(0, map.size(), 8):
		var key := Vector2i(int(map[at]), int(map[at + 1]))
		var cell := (int(map[at + 3]) & 2) != 0
		var node_draws := false
		var blank := false
		var patch: Dictionary = patches.get(key, {})
		var node := patch.get("node") as GeometryInstance3D
		if node != null and node.is_visible_in_tree() and (node as MeshInstance3D).mesh != null:
			var range_end := node.visibility_range_end
			blank = range_end > 0.0 and range_end < COLLAPSED_RANGE_M
			node_draws = not blank
		if blank and cell:
			_blank_covered += 1
		if _wet and node_draws and key != REFUSED:
			var lake: Dictionary = water.patches.get(key, {})
			var dry := not lake.is_empty() and int(lake.get("depth_nonzero_count", 0)) <= 0
			# A shown water bound to another height texture than its ground's draws its surface on
			# those heights (`TERRAIN-66`: the zero fallback put it under its bed).
			var unbound: bool = not lake.is_empty() and (lake["material"] as ShaderMaterial).get_shader_parameter("heightmap") \
				!= terrain.get_patch_height_texture(key)
			if not dry and (lake.is_empty() or not water.resident_patch_lookup.has(key) or water._mesh_need(lake) == 1 or unbound):
				_bed += 1
				if _first_bed.is_empty():
					_first_bed = "%s frame %d patch %s water %s" % [what, _frames, key,
						"none" if lake.is_empty() else ("hidden" if not water.resident_patch_lookup.has(key)
						else ("no mesh" if water._mesh_need(lake) == 1 else "unbound"))]

		if node_draws == cell:
			if node_draws:
				_both += 1
			else:
				_neither += 1
			if _first_failure.is_empty():
				_first_failure = "%s frame %d patch %s node %s cell %s collapsed %s" % [what, _frames, key, node_draws, cell, blank]
