# SPDX-License-Identifier: GPL-3.0-or-later

## The backdrop's hand-over as the renderer sees it (`TERRAIN-41` D3, docs/terrain.md "The backdrop
## drawn"): on a fresh 20 km blank world, through a jump, a far zoom-out and a zoom back in, after
## every frame each patch of the map is drawn by exactly one of its patch node and its backdrop
## cell. A patch node draws when it is visible in the tree, has a mesh and its PlaneMesh is not
## collapsed by the graded tile book (which collapses a resident patch's PlaneMesh while its list of
## graded tiles is not known: T53 (a), "nothing until then"); a cell draws when the stream node's
## whole-map probe reports it covered. The resident patches drawing nothing must be covered by their
## cells, and the run must meet some. Headless; the game scene as the load flow builds it, without
## its input and camera processing.
extends SceneTree

const SETTLE_TIMEOUT_MS := 90000
const EYE := Vector2(-6000, -6000)
const JUMP := Vector2(6000, 5000)
# A collapsed PlaneMesh's visibility range (the graded book's HIDDEN_RANGE_M).
const COLLAPSED_RANGE_M := 0.01

var _failures := 0
var _frames := 0
var _neither := 0
var _both := 0
var _blank_covered := 0
var _first_failure := ""

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

func _check(stream: Node, terrain: Node, what: String) -> void:
	_frames += 1
	var patches: Dictionary = terrain.patches
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
		if node_draws == cell:
			if node_draws:
				_both += 1
			else:
				_neither += 1
			if _first_failure.is_empty():
				_first_failure = "%s frame %d patch %s node %s cell %s collapsed %s" % [what, _frames, key, node_draws, cell, blank]
