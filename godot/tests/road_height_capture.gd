# SPDX-License-Identifier: GPL-2.0-only

## Opt-in stills of the road tool's height display (`ROAD-112` rounds I and H2) on flat ground: each
## case from above and from a low angle. A draft whose end is raised 6.5 m (a bridge: the end's
## ground marker and "+6.5 m bridge" chip), a draft whose end is lowered 10 m (a cut ramp then the
## bore: "-10.0 m tunnel"), that draft built and the chain's draft back up from its buried end
## built (the entry and the exit portal, each close from its own side), an end raised over a
## lower road (the marker stands on the road), and a bridge then a tunnel in one draft (a batch).
## Uses a fresh blank world and an isolated test profile; never loads or writes a gameplay save.
## `METRUM_ROAD_TOOL_CAPTURE_DIR` names the output directory; needs a rendering device
## (gamescope's headless backend).
extends SceneTree

# `height::handles::HANDLE_STEM_M`: the grip stands this far above its end.
const STEM_M := 4.0

var _output := ""
var _main: Node3D
var _camera: Camera3D
var _simulation: Node
var _tool
var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _run() -> void:
	_output = OS.get_environment("METRUM_ROAD_TOOL_CAPTURE_DIR")
	_expect(not _output.is_empty() and DisplayServer.get_name() != "headless", "Set METRUM_ROAD_TOOL_CAPTURE_DIR and use a rendering device")
	if _failures != 0:
		quit(1)
		return
	DirAccess.make_dir_recursive_absolute(_output)
	_main = load("res://scenes/Main.tscn").instantiate()
	_main.set_script(null)
	root.add_child(_main)
	current_scene = _main
	await process_frame
	_simulation = _main.get_node("SimulationNode")
	_expect(_simulation.create_blank_world(2048.0, 2048.0, 2.0, 512.0, 0.0), "Capture world loads")
	_simulation.set_simulation_speed(0.0)
	var input: Node = _main.get_node("InputManager")
	input._refresh_after_world_load()
	input.set_process(false)
	input.set_process_input(false)
	input.set_process_unhandled_input(false)
	var old_camera: Node = _main.get_node("CameraNode")
	old_camera.set_process(false)
	old_camera.set_process_input(false)
	_camera = Camera3D.new()
	_main.add_child(_camera)
	_camera.far = 3000.0
	_camera.current = true
	for child in _main.get_children():
		if child is CanvasLayer:
			child.visible = false
	_main.get_node("SceneLighting").pin_hour_of_day(12.0)
	root.set_disable_input(true)
	_tool = _main.get_node("RoadTool")
	_tool.set_road_type(_tool.DEFAULT_ROAD_TYPE)
	_tool.active = true

	# The end raised 6.5 m over flat ground (thirteen 0.5 m PageUp steps): one bridge, its end's
	# ground marker and height chip.
	var start := Vector3(-60.0, 0.0, 0.0)
	var end := Vector3(60.0, 0.0, 0.0)
	_tool._begin_stroke(start)
	_tool.set_scripted_pointer(true, end)
	await _frames(2)
	_tool.step_height(13)
	await _settle()
	_expect(_profiles() == [2], "the raised draft is one bridge, got %s" % [_profiles()])
	await _capture_pair("raised_end_bridge", Vector3(20.0, 3.0, 0.0), Vector3(80.0, 6.0, 40.0))

	# The end lowered 10 m: one tunnel stroke, a cut ramp from the start and the bore from where
	# the cover reaches 5 m.
	_tool.cancel_road()
	# A new start takes the live end's offset: back to the ground first, then the end down.
	_tool.step_height(-13)
	start = Vector3(-60.0, 0.0, 60.0)
	end = Vector3(60.0, 0.0, 60.0)
	_tool._begin_stroke(start)
	_tool.set_scripted_pointer(true, end)
	await _frames(1)
	_tool.step_height(-20)
	await _settle()
	_expect(_profiles() == [3], "the lowered draft is one tunnel, got %s (%s)" % [_profiles(), _simulation.road_tool_heights().get("draft_reason")])
	await _capture_pair("lowered_end_cut_then_tunnel", Vector3(20.0, -3.0, 60.0), Vector3(85.0, 3.0, 95.0))
	_expect(_tool._commit_segment(end), "the tunnel draft is sent")
	await _ack()
	# Built; the chain's next draft starts locked on the buried end (no stroke yet).
	_look(Vector3(-20.0, -2.0, 60.0), Vector3(-75.0, 25.0, 95.0))
	await _capture("tunnel_entry_built")
	# Back up to the ground from the buried end: the locked buried start grades the draft, a
	# tunnel again, the second portal.
	_tool.step_height(20)
	var up := Vector3(180.0, 0.0, 60.0)
	_tool.set_scripted_pointer(true, up)
	await _settle()
	_expect(_profiles() == [3], "the draft back up is one tunnel, got %s (%s)" % [_profiles(), _simulation.road_tool_heights().get("draft_reason")])
	_expect(_tool._commit_segment(up), "the draft back up is sent")
	await _ack()
	_tool.cancel_road()
	# The exit portal from the exit side: the road climbing out towards the camera, and close
	# in at the portal's mouth.
	_look(Vector3(110.0, -3.0, 60.0), Vector3(200.0, 20.0, 80.0))
	await _capture("tunnel_exit_built")
	_look(Vector3(105.0, -4.0, 60.0), Vector3(150.0, 4.0, 66.0))
	await _capture("tunnel_exit_portal_close")
	# The entry portal from the same pose mirrored about the buried joint (x = 60, z = 60).
	_look(Vector3(15.0, -4.0, 60.0), Vector3(-30.0, 4.0, 54.0))
	await _capture("tunnel_entry_portal_close")

	# An end raised 8 m over a lower road (snap off, so the end stays free over it): the
	# marker's foot and the chip's height are the road's carriageway, as the segmentation reads.
	_tool._begin_stroke(Vector3(-60.0, 0.0, 200.0))
	_tool.set_scripted_pointer(true, Vector3(60.0, 0.0, 200.0))
	await _frames(3)
	_expect(_tool._commit_segment(Vector3(60.0, 0.0, 200.0)), "the lower road commits")
	await _ack()
	_tool.cancel_road()
	_tool.network_snap_enabled = false
	_tool._begin_stroke(Vector3(0.0, 0.0, 140.0))
	_tool.set_scripted_pointer(true, Vector3(0.0, 0.0, 200.0))
	await _frames(1)
	_tool.step_height(16)
	await _settle()
	_expect(_profiles() == [2], "the draft over the road is one bridge, got %s" % [_profiles()])
	await _capture_pair("end_over_lower_road", Vector3(0.0, 3.0, 185.0), Vector3(45.0, 6.0, 230.0))
	_tool.cancel_road()
	_tool.step_height(-16)
	_tool.network_snap_enabled = true

	# A bridge then a tunnel in one draft: the start raised 5 m, the end lowered 8 m (a batch);
	# a height chip at each end.
	_tool._begin_stroke(Vector3(-60.0, 0.0, 300.0))
	_tool.set_scripted_pointer(true, Vector3(60.0, 0.0, 300.0))
	await _frames(2)
	await _drag_start(Vector3(-60.0, 0.0, 300.0), 5.0)
	_tool.step_height(-16)
	await _settle()
	_expect(_profiles() == [2, 3], "a bridge then a tunnel, got %s" % [_profiles()])
	_look(Vector3(0.0, 0.0, 300.0), Vector3(-40.0, 40.0, 400.0))
	await _capture("bridge_then_tunnel_batch")
	_tool.cancel_road()
	_main.queue_free()
	await process_frame
	print("road_height_capture: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

# Grabs the start's grip (a press with the scripted pointer on it), drags it `rise_m` up and lets
# go; the pointer goes back to the live end.
func _drag_start(start: Vector3, rise_m: float) -> void:
	var live: Vector3 = _tool._scripted_pointer_position
	var grip := start + Vector3(0.0, STEM_M, 0.0)
	_tool.set_scripted_pointer(true, grip)
	await _frames(1)
	_simulation.road_tool_event("press")
	_tool.set_scripted_pointer(true, grip + Vector3(0.0, rise_m, 0.0))
	await _frames(1)
	_simulation.road_tool_event("release")
	_tool.set_scripted_pointer(true, live)
	await _frames(1)

# One case from above (looking down on `target`) and from a low angle (from `low`).
func _capture_pair(label: String, target: Vector3, low: Vector3) -> void:
	_look(target, target + Vector3(0.0, 140.0, 30.0))
	await _capture(label + "_top")
	_look(target, low)
	await _capture(label + "_low")

func _profiles() -> Array:
	return Array(_simulation.road_tool_heights().get("draft_profiles", PackedInt32Array()))

# Frames until the preview stands: a stroke's at once, a batch's once tier 2 accepted or refused.
func _settle() -> void:
	for frame in 120:
		await process_frame
		var shown: Dictionary = _tool._preview
		if not shown.is_empty() and not shown.get("is_pending", false):
			break

func _look(target: Vector3, from: Vector3) -> void:
	_camera.position = from
	_camera.look_at(target)

func _frames(count: int) -> void:
	for frame in count:
		await process_frame

# Waits for the sent draft's acknowledgement and the committed chunks.
func _ack() -> void:
	var deadline := Time.get_ticks_msec() + 20000
	while (_tool._commit_request_id > 0 or _simulation.is_network_dirty()) and Time.get_ticks_msec() < deadline:
		await process_frame
	_expect(_tool._commit_request_id == 0, "the commit is acknowledged")
	await _frames(10)

func _capture(label: String) -> void:
	var terrain: Node = _main.get_node("Terrain")
	var deadline := Time.get_ticks_msec() + 20000
	while (_simulation.is_network_dirty() or _simulation.is_terrain_dirty() or terrain.unready_patches_near_camera(400.0) > 0) and Time.get_ticks_msec() < deadline:
		await process_frame
	await _frames(20)
	await RenderingServer.frame_post_draw
	_expect(root.get_texture().get_image().save_png(_output.path_join(label + ".png")) == OK, "Capture %s saves" % label)

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
