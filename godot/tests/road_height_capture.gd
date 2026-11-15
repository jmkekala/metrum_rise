# SPDX-License-Identifier: GPL-2.0-only

## Opt-in stills of the road tool's height display (`ROAD-112` rounds I and H2) on flat ground: each
## case from above and from a low angle. A draft whose end is raised 6.5 m (a bridge: the end's
## ground marker and "+6.5 m bridge" chip), a draft whose end is lowered 10 m (a cut ramp then the
## bore: "-10.0 m tunnel"), that draft built and the chain's draft back up from its buried end
## built (the entry and the exit portal, each close from its own side), an end raised over a
## lower road (the marker stands on the road), and a bridge then a tunnel in one draft (a batch).
## `METRUM_ROAD_HEIGHT_CAPTURE_SET=hue` writes round HU's stills of the draft's footprint hue
## instead (`_capture_hue`). Uses a fresh blank world and an isolated test profile; never loads or
## writes a gameplay save.
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
	if OS.get_environment("METRUM_ROAD_HEIGHT_CAPTURE_SET") == "hue":
		await _capture_hue()
		_main.queue_free()
		await process_frame
		print("road_height_capture: %d failures" % _failures)
		quit(0 if _failures == 0 else 1)
		return

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

# Round HU: the draft's footprint hue (valid blue, checking amber, refused red and hatched; on the
# ground under a bridge, outlined over a tunnel, through the hill in front of a road). Each case
# logs its draft's runs and the hue's state.
func _capture_hue() -> void:
	# A deep cut on flat ground: the end lowered 4.5 m, held ground in a cut.
	_tool._begin_stroke(Vector3(-60.0, 0.0, 0.0))
	_tool.set_scripted_pointer(true, Vector3(60.0, 0.0, 0.0))
	await _frames(2)
	_tool.step_height(-9)
	await _settle()
	_log_hue("cut")
	_expect(_profiles() == [1], "the lowered draft is one held cut, got %s" % [_profiles()])
	await _capture_pair("hue_cut_flat", Vector3(20.0, -2.0, 0.0), Vector3(85.0, 4.0, 35.0))
	_tool.cancel_road()
	_tool.step_height(9)

	# A road behind a hill: a 30 m hill between the camera and a draft on the flat behind it,
	# running away from the camera.
	_raise(Vector2(0.0, 200.0), 80.0, 30.0)
	_tool._begin_stroke(Vector3(0.0, 0.0, 290.0))
	_tool.set_scripted_pointer(true, Vector3(10.0, 0.0, 420.0))
	await _settle()
	_log_hue("behind_hill")
	_look(Vector3(0.0, 0.0, 350.0), Vector3(-15.0, 42.0, 60.0))
	await _capture("hue_behind_hill")
	_tool.cancel_road()

	# A bridge: straight grade across a 12 m valley, ground, a bridge, ground; the bridge's hue on
	# the valley floor.
	_raise(Vector2(0.0, 450.0), 70.0, -12.0)
	_tool.set_straight_grade(true)
	_tool._begin_stroke(Vector3(-150.0, 0.0, 450.0))
	_tool.set_scripted_pointer(true, Vector3(150.0, 0.0, 450.0))
	await _settle()
	_log_hue("bridge")
	_expect(_profiles().has(2), "the level draft has a bridge, got %s" % [_profiles()])
	await _capture_pair("hue_bridge", Vector3(0.0, -8.0, 450.0), Vector3(-130.0, 8.0, 540.0))
	_main.get_node("SceneLighting").pin_hour_of_day(0.5)
	await _capture("hue_bridge_night")
	_main.get_node("SceneLighting").pin_hour_of_day(12.0)
	_tool.cancel_road()
	_tool.set_straight_grade(false)

	# A bridge then a tunnel in one draft (start raised 5 m, end lowered 8 m): the bridge's hue
	# on the ground, the tunnel's outline.
	_tool._begin_stroke(Vector3(-60.0, 0.0, 650.0))
	_tool.set_scripted_pointer(true, Vector3(60.0, 0.0, 650.0))
	await _frames(2)
	await _drag_start(Vector3(-60.0, 0.0, 650.0), 5.0)
	_tool.step_height(-16)
	await _settle()
	_log_hue("bridge_then_tunnel")
	_expect(_profiles() == [2, 3], "a bridge then a tunnel, got %s" % [_profiles()])
	await _capture_pair("hue_bridge_then_tunnel", Vector3(0.0, 0.0, 650.0), Vector3(-40.0, 30.0, 740.0))
	_tool.cancel_road()
	_tool.step_height(16)

	# A junction onto an existing road: one hue over the draft's footprint, none on the road.
	_tool._begin_stroke(Vector3(-80.0, 0.0, 850.0))
	_tool.set_scripted_pointer(true, Vector3(80.0, 0.0, 850.0))
	await _frames(3)
	_expect(_tool._commit_segment(Vector3(80.0, 0.0, 850.0)), "the existing road commits")
	await _ack()
	_tool.cancel_road()
	_tool._begin_stroke(Vector3(-30.0, 0.0, 930.0))
	_tool.set_scripted_pointer(true, Vector3(0.0, 0.0, 850.0))
	await _settle()
	_log_hue("junction")
	await _capture_pair("hue_junction", Vector3(-10.0, 0.0, 875.0), Vector3(-60.0, 18.0, 950.0))
	_tool.cancel_road()

	# A refusal: a draft joining the existing road at about 6 degrees (`junction_arm_angle`).
	_tool._begin_stroke(Vector3(-60.0, 0.0, 858.0))
	_tool.set_scripted_pointer(true, Vector3(20.0, 0.0, 850.0))
	await _settle()
	_log_hue("refused")
	_expect(int(_simulation.road_tool_hue().get("state", -1)) == 2, "the refused draft's hue is refused")
	await _capture_pair("hue_refused", Vector3(-20.0, 0.0, 854.0), Vector3(-60.0, 14.0, 900.0))
	_tool.cancel_road()

	# Round HU2: a short draft from the existing road (its junction corners and end cap under the
	# hue), then a long draft on a slope beside an existing road's graded ground.
	_tool._begin_stroke(Vector3(50.0, 0.0, 885.0))
	_tool.set_scripted_pointer(true, Vector3(50.0, 0.0, 850.0))
	await _settle()
	_log_hue("short_junction")
	await _capture_pair("hue_short_junction", Vector3(50.0, 0.0, 862.0), Vector3(20.0, 14.0, 900.0))
	_tool.cancel_road()
	_raise(Vector2(-400.0, 300.0), 110.0, 12.0)
	_tool._begin_stroke(Vector3(-490.0, 0.0, 300.0))
	_tool.set_scripted_pointer(true, Vector3(-330.0, 0.0, 300.0))
	await _frames(3)
	_expect(_tool._commit_segment(Vector3(-330.0, 0.0, 300.0)), "the sloped road commits")
	await _ack()
	_tool.cancel_road()
	_tool._begin_stroke(Vector3(-490.0, 0.0, 316.0))
	_tool.set_scripted_pointer(true, Vector3(-330.0, 0.0, 316.0))
	await _settle()
	_log_hue("beside_graded")
	await _capture_pair("hue_beside_graded", Vector3(-440.0, 6.0, 312.0), Vector3(-470.0, 26.0, 360.0))
	_tool.cancel_road()

	# Across water: a basin 8 m deep filled to 2 m below the ground, crossed level (a bridge).
	_raise(Vector2(450.0, 0.0), 70.0, -8.0)
	var lake: Dictionary = _simulation.begin_world_lake_fill_preview(Vector2(450.0, 0.0), -2.0)
	_expect(_simulation.commit_world_lake_fill_preview(), "the lake fills: %s" % [lake])
	_tool.set_straight_grade(true)
	_tool._begin_stroke(Vector3(300.0, 0.0, 0.0))
	_tool.set_scripted_pointer(true, Vector3(600.0, 0.0, 0.0))
	await _settle()
	_log_hue("across_water")
	await _capture_pair("hue_across_water", Vector3(450.0, -3.0, 0.0), Vector3(420.0, 6.0, 90.0))
	_tool.cancel_road()
	_tool.set_straight_grade(false)

	# Round HU3: a road on flat ground with its start raised 1 m (a held fill) and its end
	# lowered 4.5 m (a held cut), then a draft on the ground beside it and a draft crossing its
	# fill: the hue over the graded slopes of the cut and the fill, not under them.
	_tool._begin_stroke(Vector3(-540.0, 0.0, -450.0))
	_tool.set_scripted_pointer(true, Vector3(-240.0, 0.0, -450.0))
	await _frames(2)
	await _drag_start(Vector3(-540.0, 0.0, -450.0), 1.0)
	_tool.step_height(-9)
	await _settle()
	_log_hue("fill_then_cut_road")
	_expect(_profiles() == [1], "the road is one held fill then cut, got %s" % [_profiles()])
	_expect(_tool._commit_segment(Vector3(-240.0, 0.0, -450.0)), "the fill-then-cut road commits")
	await _ack()
	_tool.cancel_road()
	_tool.step_height(9)
	_tool._begin_stroke(Vector3(-540.0, 0.0, -436.0))
	_tool.set_scripted_pointer(true, Vector3(-240.0, 0.0, -436.0))
	await _settle()
	_log_hue("beside_cut_fill")
	await _capture_pair("hue_beside_cut", Vector3(-270.0, -2.0, -440.0), Vector3(-250.0, 6.0, -400.0))
	_look(Vector3(-520.0, 0.5, -443.0), Vector3(-500.0, 6.0, -405.0))
	await _capture("hue_beside_fill_low")
	_tool.cancel_road()
	_tool._begin_stroke(Vector3(-510.0, 0.0, -500.0))
	_tool.set_scripted_pointer(true, Vector3(-510.0, 0.0, -400.0))
	await _settle()
	_log_hue("across_fill")
	await _capture_pair("hue_across_fill", Vector3(-510.0, 0.5, -450.0), Vector3(-475.0, 6.0, -425.0))
	_tool.cancel_road()

# Sculpts `centre` by `height_m` (a hill up, a valley down) with the sculpt tool's brush.
func _raise(centre: Vector2, radius: float, height_m: float) -> void:
	for step in 120:
		var now: float = _simulation.get_world_surface_height(centre)
		if (height_m > 0.0 and now > height_m) or (height_m < 0.0 and now < height_m):
			break
		_simulation.sculpt_terrain(centre, radius, signf(height_m))
	_expect(absf(_simulation.get_world_surface_height(centre)) >= absf(height_m), "the sculpt reaches %.1f m" % height_m)

func _log_hue(label: String) -> void:
	var hue: Dictionary = _simulation.road_tool_hue()
	var heights: Dictionary = _simulation.road_tool_heights()
	print("hue %s: state %d, fill %d vertices, outline %d, runs %s, draft reason '%s', preview reason '%s'" % [label, int(hue.get("state", -1)), int(hue.get("fill_vertices", 0)), PackedVector3Array(hue.get("outline_vertices", PackedVector3Array())).size(), _profiles(), heights.get("draft_reason", ""), _tool._preview.get("invalid_reason", "")])

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
	var water: Node = _main.get_node("Water")
	var deadline := Time.get_ticks_msec() + 20000
	while (_simulation.is_network_dirty() or _simulation.is_terrain_dirty() or terrain.unready_patches_near_camera(400.0) > 0 or water.has_pending_render_work(false)) and Time.get_ticks_msec() < deadline:
		await process_frame
	await _frames(20)
	await RenderingServer.frame_post_draw
	_expect(root.get_texture().get_image().save_png(_output.path_join(label + ".png")) == OK, "Capture %s saves" % label)

func _expect(condition: bool, message: String) -> void:
	if condition:
		return
	_failures += 1
	push_error(message)
