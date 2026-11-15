# SPDX-License-Identifier: GPL-2.0-only

## Opt-in stills of the road tool's modes, gizmos and HUD (`ROAD-112` round D) and the gizmo
## upload time per frame. Uses a fresh blank world and an isolated test profile; never loads or
## writes a gameplay save. `METRUM_ROAD_TOOL_CAPTURE_DIR` names the output directory; needs a
## rendering device (gamescope's headless backend). Writes `<mode>.png` stills and
## `gizmo_upload.json` (milliseconds per upload while a curve is dragged).
## `METRUM_ROAD_TOOL_CAPTURE_SET=snapping` writes round E's stills instead: the options panel's
## Snapping and Steps rows, a guide line and the angle lock taking the cursor, then the zoning
## grid holding it beside the dead end (the grid snap's curbs, round V2), then on the dead end's
## centreline and a branch drawn from there (round GS).
## `METRUM_ROAD_TOOL_CAPTURE_SET=scurve_freeform` writes round N's: an S-curve mid-draw, a
## freeform draft in its edit phase, the committed freeform road, and
## `gizmo_upload_scurve_freeform.json` (uploads while an S-curve's end and a freeform handle are
## dragged).
## `METRUM_ROAD_TOOL_CAPTURE_SET=restyle` writes round V's extras: a stroke climbing steeply, the
## same straight stroke at a far zoom (the lines' and chips' size stays constant on screen), and
## four road types snapped on one zoning grid with their curb lines.
## `METRUM_ROAD_TOOL_CAPTURE_SET=batch_ghost` writes round V2's: a level draft across a sculpted
## valley, which segments into ground, a bridge and ground, as tier 1's batch ghost.
## `METRUM_ROAD_TOOL_CAPTURE_SET=scurve_steps` writes round SC's: an S-curve after its first,
## second and third click (the road previewed with the step's hint chip) and finished.
## `METRUM_ROAD_TOOL_CAPTURE_SET=continuous_chains` writes round CT's: with Continuous on, a built
## S-curve and the chained one previewed from its end, then a built freeform draft and the
## chained one in its edit phase.
extends SceneTree

const RoadToolScript := preload("res://scripts/tools/road_tool.gd")

var _output := ""
var _main: Node3D
var _camera: Camera3D
var _simulation: Node
var _tool
var _failures := 0
var _upload_order := PackedFloat32Array()
# The last drag's parts (`_parts`).
var _last_parts := {}

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
	var capture_set := OS.get_environment("METRUM_ROAD_TOOL_CAPTURE_SET")
	if capture_set in ["snapping", "scurve_freeform", "restyle", "batch_ghost", "scurve_steps", "continuous_chains"]:
		if capture_set == "snapping":
			await _capture_snapping()
		elif capture_set == "continuous_chains":
			await _capture_continuous_chains()
		elif capture_set == "scurve_steps":
			await _capture_scurve_steps()
		elif capture_set == "restyle":
			await _capture_restyle()
		elif capture_set == "batch_ghost":
			await _capture_batch_ghost()
		else:
			await _capture_scurve_freeform()
		_main.queue_free()
		await process_frame
		quit(0 if _failures == 0 else 1)
		return
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
	for key in _tool.gizmo_part_ms:
		_tool.gizmo_part_ms[key] = PackedFloat32Array()
	for index in 120:
		var turn := float(index) / 120.0 * TAU
		_tool.set_scripted_pointer(true, Vector3(40.0 + 12.0 * cos(turn), 0.0, 10.0 + 12.0 * sin(turn)))
		await process_frame
	_upload_order = _tool.gizmo_upload_ms.duplicate()
	_last_parts = _parts()
	# The drag rewrote the vertex buffers in place: the server's copy is this frame's draft.
	var gizmo_mesh: ArrayMesh = _tool.gizmos.mesh
	var drawn: Array = RenderingServer.mesh_surface_get_arrays(gizmo_mesh.get_rid(), 0)
	var expected: PackedVector3Array = _simulation.road_tool_frame()["gizmo_line_vertices"]
	_expect(not drawn.is_empty() and PackedVector3Array(drawn[Mesh.ARRAY_VERTEX]) == expected, "the in-place gizmo upload leaves the lines' quads on the server")
	# The interleaved attributes went into the attribute buffer as UV, UV2 and the look (rounds V,
	# V2: five floats a vertex).
	var attributes: PackedFloat32Array = _simulation.road_tool_frame()["gizmo_line_attributes"]
	var uv: PackedVector2Array = drawn[Mesh.ARRAY_TEX_UV]
	var uv2: PackedVector2Array = drawn[Mesh.ARRAY_TEX_UV2]
	var looks: PackedFloat32Array = drawn[Mesh.ARRAY_CUSTOM0]
	_expect(uv.size() * 5 == attributes.size() and uv[1] == Vector2(attributes[5], attributes[6]) and uv2[1] == Vector2(attributes[7], attributes[8]) and looks[1] == attributes[9], "the lines' attributes are their other ends, sides and looks on the server")
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
		"parts": _last_parts,
		"failures": _failures,
	}
	var file := FileAccess.open(_output.path_join("gizmo_upload.json"), FileAccess.WRITE)
	file.store_string(JSON.stringify(report, "\t"))
	file.close()
	print("ROAD_TOOL_CAPTURE %s" % JSON.stringify(report))
	_main.queue_free()
	await process_frame
	quit(0 if _failures == 0 else 1)

# Round E: the options panel's rows with the session's defaults, then a stroke from open ground
# whose cursor a dead end's extension takes (a guide line), then the 15 degree angle lock. The
# cursor goes by vertical camera rays (a scripted pointer takes no snap), the tool's own pointer
# stopped so the mouse does not move it.
func _capture_snapping() -> void:
	_tool.draw_mode = RoadToolScript.DrawMode.STRAIGHT
	_tool._begin_stroke(Vector3(200.0, 0.0, 0.0))
	_tool.set_scripted_pointer(true, Vector3(260.0, 0.0, 0.0))
	await _frames(3)
	_expect(_tool._commit_segment(Vector3(260.0, 0.0, 0.0)), "the dead-end road commits")
	await _ack()
	_tool.cancel_road()
	_camera.position = Vector3(290.0, 110.0, 110.0)
	_camera.look_at(Vector3(290.0, 0.0, 25.0))
	var ui: CanvasLayer = _main.get_node("MainUI")
	ui.visible = true
	ui.road_combined_hbox.visible = true
	ui.get_meta("options_panel").visible = true
	ui.get_meta("road_sep").visible = true
	_tool.set_process(false)
	_tool.set_scripted_pointer(false)
	_ray(Vector3(280.0, 0.0, 50.0))
	_tool._read_press(_simulation.road_tool_event("press"))
	_ray(Vector3(310.0, 0.0, 0.8))
	_expect(String(_simulation.road_tool_frame()["snap"]) == "guide", "the dead end's extension takes the cursor")
	_ray(Vector3(310.0, 0.0, 0.8))
	await _capture("snapping_guide_line")
	ui.angle_step_option.select(1)
	ui.angle_step_option.item_selected.emit(1)
	_ray(Vector3(340.0, 0.0, 35.0))
	_expect(String(_simulation.road_tool_frame()["snap"]) == "angle", "the 15 degree lock takes the cursor")
	_ray(Vector3(340.0, 0.0, 35.0))
	await _capture("snapping_angle_lock")
	ui.angle_step_option.select(0)
	ui.angle_step_option.item_selected.emit(0)
	# Round V2: the zoning-grid snap on, the cursor over the dead end's cells, so the grid holds it
	# and the road's curbs show on the cell lines.
	_tool.zoning_grid_snap_enabled = true
	_ray(Vector3(243.0, 0.0, 27.0))
	var grid_snap := String(_simulation.road_tool_frame()["snap"])
	_expect(grid_snap in ["grid", "angle_grid"], "the zoning grid takes the cursor (%s)" % grid_snap)
	_ray(Vector3(243.0, 0.0, 27.0))
	await _capture("snapping_grid_curbs")
	# Round GS: with the grid on, the cursor over the dead-end road takes its centreline at the
	# grid's position along it; a branch drawn from there shows its curbs on the cell lines.
	_tool.cancel_road()
	_ray(Vector3(231.3, 0.0, 0.9))
	var on_road := String(_simulation.road_tool_frame()["snap"])
	_expect(on_road == "grid_edge", "the grid holds the cursor on the road (%s)" % on_road)
	_tool._read_press(_simulation.road_tool_event("press"))
	_ray(Vector3(232.1, 0.0, 41.0))
	var branch := String(_simulation.road_tool_frame()["snap"])
	_expect(branch == "grid", "the grid takes the branch's end (%s)" % branch)
	_ray(Vector3(232.1, 0.0, 41.0))
	await _capture("snapping_grid_on_road")
	_tool.zoning_grid_snap_enabled = false
	_tool.cancel_road()
	_tool.set_process(true)

# Round SC: an S-curve step by step, its road previewed from the first click, then built.
func _capture_scurve_steps() -> void:
	_tool.draw_mode = RoadToolScript.DrawMode.COMPLEX_CURVE
	_tool._begin_stroke(Vector3(-70.0, 0.0, 60.0))
	_tool.set_scripted_pointer(true, Vector3(-30.0, 0.0, 55.0))
	await _capture("scurve_step1")
	_expect(_tool.stroke_preview_current(), "the first step is previewed")
	_tool._commit_segment(Vector3(-40.0, 0.0, 60.0))
	_tool.set_scripted_pointer(true, Vector3(25.0, 0.0, 15.0))
	await _capture("scurve_step2")
	_expect(_tool.stroke_preview_current(), "the second step is previewed")
	_tool._commit_segment(Vector3(30.0, 0.0, 10.0))
	_tool.set_scripted_pointer(true, Vector3(72.0, 0.0, 10.0))
	await _capture("scurve_step3")
	_expect(_tool._commit_segment(Vector3(72.0, 0.0, 10.0)), "the end click sends the S-curve")
	await _ack()
	_tool.cancel_road()
	await _capture("scurve_finished")

# Round CT: Continuous on; an S-curve built, the chained S-curve (its start direction carried)
# previewed to its end; a freeform draft built, the chained draft (its start handle locked to the
# carried direction) in its edit phase.
func _capture_continuous_chains() -> void:
	_tool.continuous = true
	_tool.draw_mode = RoadToolScript.DrawMode.COMPLEX_CURVE
	_tool._begin_stroke(Vector3(-90.0, 0.0, 40.0))
	_tool._commit_segment(Vector3(-70.0, 0.0, 40.0))
	_tool._commit_segment(Vector3(-45.0, 0.0, 20.0))
	_tool.set_scripted_pointer(true, Vector3(-30.0, 0.0, 20.0))
	await _frames(3)
	_expect(_tool._commit_segment(Vector3(-30.0, 0.0, 20.0)), "the first S-curve is sent")
	await _ack()
	_expect(int(_simulation.road_tool_frame()["placed"]) == 2, "the chained S-curve has its start direction")
	_tool._commit_segment(Vector3(0.0, 0.0, 40.0))
	_tool.set_scripted_pointer(true, Vector3(25.0, 0.0, 40.0))
	await _capture("chained_scurve_pair")
	_expect(_tool.stroke_preview_current(), "the chained S-curve is previewed")
	_expect(_tool._commit_segment(Vector3(25.0, 0.0, 40.0)), "the chained S-curve is sent")
	await _ack()
	_tool.cancel_road()
	_tool.draw_mode = RoadToolScript.DrawMode.FREEFORM
	_tool._begin_stroke(Vector3(-90.0, 0.0, -30.0))
	for node in [Vector3(-60.0, 0.0, -10.0), Vector3(-30.0, 0.0, -30.0)]:
		_tool._commit_segment(node)
	_tool._session_press("confirm")
	await _frames(3)
	_tool._session_press("confirm")
	_expect(_tool._commit_request_id > 0, "the first freeform draft is sent")
	await _ack()
	for node in [Vector3(0.0, 0.0, -40.0), Vector3(30.0, 0.0, -20.0)]:
		_tool._commit_segment(node)
	_tool._session_press("confirm")
	await _frames(3)
	_expect(int(_simulation.road_tool_frame()["phase"]) == 3, "the chained draft is in its edit phase")
	await _capture("chained_freeform_pair")
	_tool._session_press("confirm")
	await _ack()
	_tool.cancel_road()
	_tool.continuous = false

# Round N: an S-curve mid-draw (its end dragged round a circle for the upload record), then a
# freeform draft placed, taken into its edit phase, one handle dragged (round a circle too), and
# built.
func _capture_scurve_freeform() -> void:
	var uploads := {}
	_tool.draw_mode = RoadToolScript.DrawMode.COMPLEX_CURVE
	_tool._begin_stroke(Vector3(-70.0, 0.0, 60.0))
	_tool._commit_segment(Vector3(-40.0, 0.0, 60.0))
	_tool._commit_segment(Vector3(30.0, 0.0, 10.0))
	# The process's first gizmo upload (new surfaces and materials) apart from the drag's.
	_tool.gizmo_upload_ms = PackedFloat32Array()
	_tool.set_scripted_pointer(true, Vector3(72.0, 0.0, 10.0))
	await _frames(3)
	var first_upload := Array(_tool.gizmo_upload_ms)
	uploads["scurve"] = await _drag_record(Vector3(60.0, 0.0, 10.0), func(at: Vector3): _tool.set_scripted_pointer(true, at))
	var parts := {"scurve": _last_parts}
	_tool.set_scripted_pointer(true, Vector3(60.0, 0.0, 10.0))
	await _capture("scurve_mid_draw")
	_tool.cancel_road()
	_tool.draw_mode = RoadToolScript.DrawMode.FREEFORM
	_tool._begin_stroke(Vector3(-90.0, 0.0, 0.0))
	for node in [Vector3(-40.0, 0.0, 30.0), Vector3(10.0, 0.0, 0.0)]:
		_expect(not _tool._commit_segment(node), "a freeform node commits nothing")
	_tool.set_scripted_pointer(true, Vector3(60.0, 0.0, 30.0))
	await _frames(3)
	_tool._commit_segment(Vector3(60.0, 0.0, 30.0))
	_tool._session_press("confirm")
	await _frames(3)
	_expect(int(_simulation.road_tool_frame()["phase"]) == 3, "the draft is in its edit phase")
	# Node 1's outward handle (handles in frame order: node 0 out, node 1 in, node 1 out, ...).
	var handles := PackedVector3Array()
	for index in _tool.gizmos._handles.size():
		if _tool.gizmos._handle_kinds[index] == 5:
			handles.append(_tool.gizmos._handles[index])
	_tool.set_scripted_pointer(true, handles[2])
	await _frames(2)
	_tool._session_press()
	var grip := handles[2] + Vector3(10.0, 0.0, 6.0)
	uploads["freeform_handle"] = await _drag_record(grip, func(at: Vector3): _tool.set_scripted_pointer(true, at))
	parts["freeform_handle"] = _last_parts
	_tool.set_scripted_pointer(true, grip)
	await _frames(2)
	_simulation.road_tool_event("release")
	await _capture("freeform_edit_phase")
	_tool._session_press("confirm")
	_expect(_tool._commit_request_id > 0, "the freeform draft is sent")
	await _ack()
	_tool.cancel_road()
	await _capture("freeform_committed")
	var report := {"engine": Engine.get_version_info(), "adapter": RenderingServer.get_video_adapter_name(), "failures": _failures, "first_uploads_ms": first_upload, "parts": parts}
	for key in uploads:
		var times: Array = uploads[key]
		var sorted := times.duplicate()
		sorted.sort()
		report[key] = {
			"uploads": sorted.size(),
			"p50": sorted[sorted.size() / 2] if not sorted.is_empty() else -1.0,
			"p95": sorted[int(sorted.size() * 0.95)] if not sorted.is_empty() else -1.0,
			"max": sorted[-1] if not sorted.is_empty() else -1.0,
			"in_order": times,
		}
	var file := FileAccess.open(_output.path_join("gizmo_upload_scurve_freeform.json"), FileAccess.WRITE)
	file.store_string(JSON.stringify(report, "\t"))
	file.close()
	print("ROAD_TOOL_CAPTURE %s" % JSON.stringify(report))

# Round V: a straight stroke climbing 9 m over 60 m (its grade chip and the steep notice), then
# a straight stroke seen from 900 m.
func _capture_restyle() -> void:
	_tool.draw_mode = RoadToolScript.DrawMode.STRAIGHT
	_tool._begin_stroke(Vector3(-40.0, 0.0, 50.0))
	_tool.set_scripted_pointer(true, Vector3(20.0, 9.0, 30.0))
	await _capture("steep_slope")
	_tool.set_scripted_pointer(true, Vector3(40.0, 0.0, 10.0))
	await _capture("straight_near")
	_camera.position = Vector3(0.0, 760.0, 560.0)
	_camera.look_at(Vector3(0.0, 0.0, 10.0))
	await _capture("straight_far_zoom")
	_tool.cancel_road()
	# Item 8: four road types snapped on one zoning grid (a base road's cells), each drawn by
	# vertical camera rays with the grid snap on; the curbs and their ticks show.
	_camera.position = Vector3(0.0, 150.0, 200.0)
	_camera.look_at(Vector3(0.0, 0.0, 100.0))
	_tool._begin_stroke(Vector3(-140.0, 0.0, 40.0))
	_tool.set_scripted_pointer(true, Vector3(140.0, 0.0, 40.0))
	await _frames(3)
	_expect(_tool._commit_segment(Vector3(140.0, 0.0, 40.0)), "the grid's base road commits")
	await _ack()
	_tool.cancel_road()
	_tool.zoning_grid_snap_enabled = true
	_tool.set_process(false)
	_tool.set_scripted_pointer(false)
	for type_id in ["builtin:road_2l", "builtin:road_4l", "builtin:walkway", "builtin:one_way_1l"]:
		_tool.set_road_type(type_id)
		_ray(Vector3(-43.0, 0.0, 103.0))
		_tool._read_press(_simulation.road_tool_event("press"))
		_ray(Vector3(47.0, 0.0, 104.0))
		var snap := String(_simulation.road_tool_frame()["snap"])
		_expect(snap == "grid", "%s: the grid takes the cursor (%s)" % [type_id, snap])
		_ray(Vector3(47.0, 0.0, 104.0))
		await _capture("grid_curbs_" + type_id.trim_prefix("builtin:"))
		_tool.cancel_road()
	_tool.zoning_grid_snap_enabled = false
	_tool.set_road_type(_tool.DEFAULT_ROAD_TYPE)
	_tool.set_process(true)

# Round V2: a valley sculpted 12 m deep under a level 400 m draft (as `road_batch_preview_test`),
# whose segmentation is ground, a bridge and ground; tier 1's ghost of that pose in the gizmos.
func _capture_batch_ghost() -> void:
	var centre := Vector2(0.0, 10.0)
	for step in 80:
		if _simulation.get_world_surface_height(centre) < -12.0:
			break
		_simulation.sculpt_terrain(centre, 70.0, -1.0)
	_expect(_simulation.get_world_surface_height(centre) < -12.0, "the sculpt tool lowers a valley")
	_camera.position = Vector3(-150.0, 70.0, 120.0)
	_camera.look_at(Vector3(-20.0, -6.0, 10.0))
	var path := PackedVector3Array([Vector3(-200.0, 0.0, 10.0), Vector3(200.0, 0.0, 10.0)])
	var pose: Dictionary = _simulation.preview_road_draft(path, 0.0, 0.0, 1, 1, true, false)
	_expect(int(pose.get("stroke_count", 0)) >= 3 and PackedInt32Array(pose.get("profiles", PackedInt32Array())).has(2), "the draft segments with a bridge: %s" % [pose.get("profiles", [])])
	if _tool.gizmos == null:
		_tool._show_gizmos({})
	_tool.gizmos.show_batch_ghost(pose)
	await _capture("batch_ghost")
	_tool.gizmos.clear_batch_ghost()

# The recorded frames' parts (gizmo upload, chip wording, chip placement): p50, p95 and max each.
func _parts() -> Dictionary:
	var parts := {}
	for key in _tool.gizmo_part_ms:
		var times := Array(_tool.gizmo_part_ms[key])
		times.sort()
		parts[key] = [times[times.size() / 2], times[int(times.size() * 0.95)], times[-1]] if not times.is_empty() else []
	return parts

# Moves the pointer round a 12 m circle about `centre` for 120 frames through `move` and returns
# the gizmo uploads those frames recorded (ms each).
func _drag_record(centre: Vector3, move: Callable) -> Array:
	_tool.gizmo_upload_ms = PackedFloat32Array()
	for key in _tool.gizmo_part_ms:
		_tool.gizmo_part_ms[key] = PackedFloat32Array()
	for key in _tool.gizmo_part_ms:
		_tool.gizmo_part_ms[key] = PackedFloat32Array()
	for index in 120:
		var turn := float(index) / 120.0 * TAU
		move.call(centre + Vector3(12.0 * cos(turn), 0.0, 12.0 * sin(turn)))
		await process_frame
	_last_parts = _parts()
	return Array(_tool.gizmo_upload_ms)

# The session's cursor for a vertical camera ray onto `at`, then one frame of the tool's drawing.
func _ray(at: Vector3) -> void:
	_simulation.road_tool_pointer(Vector3(at.x, 200.0, at.z), Vector3.DOWN, false)
	_tool._place_labels(_tool._session_frame(true))

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
