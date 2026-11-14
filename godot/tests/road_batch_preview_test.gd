# SPDX-License-Identifier: GPL-2.0-only

## Batch preview through the bridge (`ROAD-112` round G1): a level draft across a sculpted valley
## segments into held ground, a bridge and held ground (`preview_road_draft`). Every pose runs
## tier 1 (each stroke valid, a ghost ribbon per stroke, the costs summed); resting poses run
## tier 2 one stroke a frame, then its overlay over at most as many frames, until it accepts the
## batch with the exact road chunks; the gizmos
## draw the ghost; the commit of the same draft (`add_road_draft`) takes tier 2's plans and
## commits a bridge as one batch.
extends SceneTree

const TIMEOUT_MS := 60000
const VALLEY_DEPTH_M := 10.0
const RoadToolGizmos := preload("res://scripts/renderers/road_tool_gizmos.gd")

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	# Own scene startup without the menu launch flow.
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	await process_frame
	var simulation: Node = main.get_node("SimulationNode")
	var input_manager: Node = main.get_node("InputManager")
	_expect(simulation.create_blank_world(640.0, 640.0, 4.0, 128.0, 0.0), "the blank world must load")
	for step in 80:
		if simulation.get_world_surface_height(Vector2.ZERO) < -VALLEY_DEPTH_M:
			break
		simulation.sculpt_terrain(Vector2.ZERO, 70.0, -1.0)
	_expect(simulation.get_world_surface_height(Vector2.ZERO) < -VALLEY_DEPTH_M, "the sculpt tool must lower a valley")
	input_manager._refresh_after_world_load()
	input_manager.set_simulation_speed(0.0)
	var ready_deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while not input_manager._world_ready and Time.get_ticks_msec() < ready_deadline:
		await process_frame
	_expect(input_manager._world_ready, "the world must become ready")

	var path := PackedVector3Array([Vector3(-200.0, 0.0, 0.0), Vector3(200.0, 0.0, 0.0)])
	# Tier 1 on a moving pose.
	var pose: Dictionary = simulation.preview_road_draft(path, 0.0, 0.0, 1, 1, true, false)
	_expect(pose.get("reason", "missing") == "", "the draft must segment: %s" % pose.get("reason", "missing"))
	var count := int(pose.get("stroke_count", 0))
	var profiles: PackedInt32Array = pose.get("profiles", PackedInt32Array())
	_expect(count >= 3 and profiles.has(2), "a level draft over the valley is ground, a bridge and ground, got %s" % [profiles])
	var valid: PackedInt32Array = pose.get("valid", PackedInt32Array())
	_expect(valid.size() == count and not valid.has(0), "every stroke passes its hover check: %s" % [pose.get("reasons", [])])
	var ghost: PackedVector3Array = pose.get("ghost", PackedVector3Array())
	var starts: PackedInt32Array = pose.get("ghost_starts", PackedInt32Array())
	_expect(starts.size() == count and ghost.size() >= 6 * count, "a ribbon per stroke, got %d vertices" % ghost.size())
	_expect(float(pose.get("build_cost", 0.0)) > 0.0 and float(pose.get("length_m", 0.0)) > 390.0, "the batch is priced over its length")
	_expect(pose.get("tier2_state", "") == "running", "a moving pose runs no tier 2")
	var gizmos: MeshInstance3D = RoadToolGizmos.new()
	root.add_child(gizmos)
	gizmos.show_batch_ghost(pose)
	_expect(gizmos.batch_ghost_surfaces() == 1, "valid strokes draw one ghost surface")

	# Tier 2 while the pointer rests: one stroke a frame, then the exact overlay over at most as
	# many frames (round G1b), shown whole on the frame that completes it.
	var accepted := {}
	for frame in 2 * count + 1:
		var rest: Dictionary = simulation.preview_road_draft(path, 0.0, 0.0, 1, 1, true, true)
		_expect(int(rest.get("tier2_done", -1)) == mini(frame + 1, count), "tier 2 finishes one stroke a frame, got %s at frame %d" % [rest.get("tier2_done"), frame])
		if rest.get("tier2_state", "") != "running":
			accepted = rest
			break
		await process_frame
	_expect(accepted.get("tier2_state", "") == "accepted", "tier 2 must accept the batch: %s %s" % [accepted.get("tier2_reason", "missing"), accepted.get("tier2_stroke", -1)])
	_expect(accepted.has("road_chunks") and accepted.has("has_road_chunks"), "the accepting frame carries the exact preview")
	_expect(not (accepted.get("changed_chunks", PackedInt32Array()) as PackedInt32Array).is_empty(), "the batch changes road chunks")

	# The click on the same draft commits tier 2's plans as one batch.
	var sent: Dictionary = simulation.add_road_draft(path, 0.0, 0.0, 1, 1, true)
	_expect(bool(sent.get("previewed", false)), "the commit takes tier 2's plans")
	var request_id := int(sent.get("request_id", 0))
	_expect(request_id > 0, "the batch must be sent: %s" % sent.get("reason", "missing"))
	var ack: Variant = null
	var deadline := Time.get_ticks_msec() + TIMEOUT_MS
	while ack == null and Time.get_ticks_msec() < deadline:
		await process_frame
		ack = simulation.get_road_commit_result(request_id)
	_expect(ack != null and bool(ack.get("committed", false)), "the batch must commit: %s" % [ack])
	var classes := []
	for edge in int(simulation.get_road_benchmark_state()["edge_slots"]):
		classes.append(simulation.get_edge_class(edge))
	_expect(classes.has(1), "the batch commits a bridge, got %s" % [classes])
	gizmos.clear_batch_ghost()
	_expect(gizmos.batch_ghost_surfaces() == 0, "the ghost clears")
	gizmos.queue_free()
	_finish()

func _finish() -> void:
	if _failures == 0:
		print("road_batch_preview_test: PASS")
		quit(0)
	else:
		push_error("road_batch_preview_test: %d failures" % _failures)
		quit(1)
