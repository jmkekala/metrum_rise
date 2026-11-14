# SPDX-License-Identifier: GPL-2.0-only

## Opt-in stills of the road tool's modes, gizmos and HUD (`ROAD-112` round D) and the gizmo
## upload time per frame. Uses a fresh blank world and an isolated test profile; never loads or
## writes a gameplay save. `METRUM_ROAD_TOOL_CAPTURE_DIR` names the output directory; needs a
## rendering device (gamescope's headless backend). Writes `<mode>.png` stills and
## `gizmo_upload.json` (milliseconds per upload while a curve is dragged).
extends SceneTree

const RoadToolScript := preload("res://scripts/tools/road_tool.gd")

var _output := ""
var _main: Node3D
var _camera: Camera3D
var _simulation: Node
var _tool
var _failures := 0
var _upload_order := PackedFloat32Array()

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
	_camera.position = Vector3(0.0, 150.0, 110.0)
	_camera.look_at(Vector3(0.0, 0.0, 10.0))
	for child in _main.get_children():
		if child is CanvasLayer:
			child.visible = false
	_main.get_node("SceneLighting").pin_hour_of_day(12.0)
	root.set_disable_input(true)
	_tool = _main.get_node("RoadTool")
	_tool.set_road_type(_tool.DEFAULT_ROAD_TYPE)
	_tool.active = true
	_tool.begin_preview_measurement()
	# An existing road for the strokes to start from.
	_tool.draw_mode = RoadToolScript.DrawMode.STRAIGHT
	_tool._begin_stroke(Vector3(-120.0, 0.0, 40.0))
	_tool.set_scripted_pointer(true, Vector3(-60.0, 0.0, 40.0))
	await _frames(3)
	_expect(_tool._commit_segment(Vector3(-60.0, 0.0, 40.0)), "the base road commits")
	await _ack()
	_tool.cancel_road()
	# Straight, mid-draw.
	_tool._begin_stroke(Vector3(-40.0, 0.0, 60.0))
	_tool.set_scripted_pointer(true, Vector3(30.0, 0.0, 20.0))
	await _capture("straight")
	# Simple curve, mid-draw: start, the control point, the end at the pointer.
	_tool.cancel_road()
	_tool.draw_mode = RoadToolScript.DrawMode.SIMPLE_CURVE
	_tool._begin_stroke(Vector3(-40.0, 0.0, 60.0))
	_tool._commit_segment(Vector3(20.0, 0.0, 60.0))
	_tool.set_scripted_pointer(true, Vector3(40.0, 0.0, 10.0))
	await _capture("curve_mid_draw")
	# The gizmo upload while the curve's end is dragged (every frame a new pose).
	_tool.gizmo_upload_ms = PackedFloat32Array()
	for index in 120:
		var turn := float(index) / 120.0 * TAU
		_tool.set_scripted_pointer(true, Vector3(40.0 + 12.0 * cos(turn), 0.0, 10.0 + 12.0 * sin(turn)))
		await process_frame
	_upload_order = _tool.gizmo_upload_ms.duplicate()
	# The drag rewrote the vertex buffers in place: the server's copy is this frame's draft.
	var gizmo_mesh: ArrayMesh = _tool.gizmos.mesh
	var drawn: Array = RenderingServer.mesh_surface_get_arrays(gizmo_mesh.get_rid(), 1)
	var expected: PackedVector3Array = _tool.gizmos._line_points.slice(int(_simulation.road_tool_frame()["gizmo_line_starts"][1]))
	_expect(not drawn.is_empty() and PackedVector3Array(drawn[Mesh.ARRAY_VERTEX]) == expected, "the in-place gizmo upload leaves the draft's vertices on the server")
	var uploads: Array = Array(_tool.gizmo_upload_ms)
	uploads.sort()
	_tool.set_scripted_pointer(true, Vector3(40.0, 0.0, 10.0))
	await _frames(3)
	# Committed: the curve is built and the chain's next stroke starts at its end.
	_expect(_tool._commit_segment(Vector3(40.0, 0.0, 10.0)), "the curve commits")
	await _ack()
	_tool.set_scripted_pointer(true, Vector3(60.0, 0.0, -30.0))
	await _capture("curve_committed")
	# Continuous chain: the next stroke leaves along the curve's end as an arc.
	_tool.cancel_road()
	_tool.continuous = true
	_tool.draw_mode = RoadToolScript.DrawMode.STRAIGHT
	# Started by a camera ray on the base road's dead end, so the session reads its direction.
	_tool.set_scripted_pointer(false)
	_simulation.road_tool_pointer(Vector3(-60.0, 100.0, 40.0), Vector3.DOWN, false)
	_tool._read_press(_simulation.road_tool_event("press"))
	_tool.set_scripted_pointer(true, Vector3(-20.0, 0.0, 0.0))
	await _capture("continuous_from_dead_end")
	_expect(_tool._commit_segment(Vector3(-20.0, 0.0, 0.0)), "the continuous stroke commits")
	await _ack()
	_tool.set_scripted_pointer(true, Vector3(30.0, 0.0, -20.0))
	await _capture("continuous_chain")
	_tool.continuous = false
	# HUD warnings: a 20 m radius designs for 20 km/h, under the road's 50; a 4 m radius is
	# refused before any preview.
	_tool.cancel_road()
	_tool.draw_mode = RoadToolScript.DrawMode.SIMPLE_CURVE
	_tool._begin_stroke(Vector3(-80.0, 0.0, -40.0))
	_tool._commit_segment(Vector3(-60.0, 0.0, -40.0))
	_tool.set_scripted_pointer(true, Vector3(-60.0, 0.0, -20.0))
	await _capture("hud_design_speed_warning")
	_tool.cancel_road()
	_tool._begin_stroke(Vector3(-80.0, 0.0, -40.0))
	_tool._commit_segment(Vector3(-76.0, 0.0, -40.0))
	_tool.set_scripted_pointer(true, Vector3(-76.0, 0.0, -36.0))
	await _capture("hud_tight_curve_refused")
	var p50: float = uploads[uploads.size() / 2] if not uploads.is_empty() else -1.0
	var p95: float = uploads[int(uploads.size() * 0.95)] if not uploads.is_empty() else -1.0
	var report := {
		"engine": Engine.get_version_info(),
		"adapter": RenderingServer.get_video_adapter_name(),
		"gizmo_uploads": uploads.size(),
		"gizmo_upload_ms_p50": p50,
		"gizmo_upload_ms_p95": p95,
		"gizmo_upload_ms_max": uploads[-1] if not uploads.is_empty() else -1.0,
		"gizmo_upload_ms_in_order": Array(_upload_order),
		"failures": _failures,
	}
	var file := FileAccess.open(_output.path_join("gizmo_upload.json"), FileAccess.WRITE)
	file.store_string(JSON.stringify(report, "\t"))
	file.close()
	print("ROAD_TOOL_CAPTURE %s" % JSON.stringify(report))
	_main.queue_free()
	await process_frame
	quit(0 if _failures == 0 else 1)

func _frames(count: int) -> void:
	for frame in count:
		await process_frame

# Waits for the sent stroke's acknowledgement and the committed chunks.
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
