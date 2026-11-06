# SPDX-License-Identifier: GPL-2.0-only

## Opt-in production-ground fixture, screenshots and hidden/shown frame timings. Uses a fresh
## blank world and an isolated test profile; never loads or writes a gameplay save.
extends "res://tests/zoning_reference_test.gd"

var _main: Node3D
var _camera: Camera3D
var _timings: Array[Dictionary] = []

func _run() -> void:
	_output = OS.get_environment("METRUM_ZONE_MASK_DIR")
	_expect(not _output.is_empty() and DisplayServer.get_name() != "headless", "Set METRUM_ZONE_MASK_DIR and use a rendering device")
	if _failures != 0:
		quit(1)
		return
	DirAccess.make_dir_recursive_absolute(_output)
	_main = load("res://scenes/Main.tscn").instantiate()
	_main.set_script(null)
	root.add_child(_main)
	current_scene = _main
	await process_frame
	simulation = _main.get_node("SimulationNode")
	_expect(simulation.create_blank_world(2048.0, 2048.0, 2.0, 512.0, 0.0), "Capture world loads")
	simulation.set_simulation_speed(0.0)
	simulation.sculpt_terrain(Vector2(-80, 35), 50.0, 0.2)
	simulation.slope_terrain(Vector2(150, 0), 100.0, Vector2(70, 0), 0.0, Vector2(230, 0), 8.0, 1.0)
	if not await _commit(_path([Vector2(-280, 0), Vector2(280, 0)])):
		quit(1)
		return
	if not await _commit(_path([Vector2(-180, 0), Vector2(-180, -160)])):
		quit(1)
		return
	if not await _commit(_path([Vector2(280, 0), Vector2(310, 25), Vector2(320, 60), Vector2(305, 100), Vector2(270, 125)])):
		quit(1)
		return
	_paint_reference()
	var planted := 0
	for point in [Vector2(-80, 25), Vector2(-35, 30), Vector2(20, 30), Vector2(55, 45), Vector2(140, -35), Vector2(-210, 40)]:
		planted += int(simulation.add_vegetation_at(point, 3))
	_expect(planted >= 3, "Fixture adds at least three trees")
	var input: Node = _main.get_node("InputManager")
	input._refresh_after_world_load()
	_place_fixture_building()
	input.set_process(false)
	input.set_process_input(false)
	input.set_process_unhandled_input(false)
	_main.get_node("ZoningTool").set_process(false)
	_main.get_node("RoadTool").set_process(false)
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
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0
	root.set_disable_input(true)
	RenderingServer.viewport_set_measure_render_time(root.get_viewport_rid(), true)
	var overlay: Node = _main.get_node("ZoningOverlay")
	overlay.set_tool_active(true)
	for pose in [
		["close", Vector3(-25, 100, 135), Vector3(-25, 0, 0)],
		["wide", Vector3(0, 360, 480), Vector3(0, 0, 0)],
		["bump_slope", Vector3(100, 55, 110), Vector3(100, 0, 0)],
		["under_trees", Vector3(-40, 22, 65), Vector3(-40, 2, 20)],
	]:
		_camera.position = pose[1]
		_camera.look_at(pose[2])
		await _settle_scene(overlay)
		for hour in [12.0, 23.0]:
			_main.get_node("SceneLighting").pin_hour_of_day(hour)
			await _capture_mask(str(pose[0]) + ("_day" if hour == 12.0 else "_night"))
		if pose[0] == "close":
			_main.get_node("SceneLighting").pin_hour_of_day(12.0)
			var start := Time.get_ticks_usec()
			var preview := simulation.get_zoning_cells_preview_packed(1, PackedVector2Array([Vector2(-65, 12), Vector2(75, 50)]), 20.0, 4)
			overlay.set_cell_preview(preview)
			_timings.append({"preview_cells": preview.cell_count, "preview_build_upload_us": Time.get_ticks_usec() - start})
			await _capture_mask("mid_drag")
			overlay.set_cell_preview({})
			for enabled in [false, true, false, true]:
				overlay.set_tool_active(enabled)
				for frame in 30:
					await process_frame
				await _measure_ground(enabled)
	var pool = overlay._cell_chunks._masks
	_timings.append({"resident_bytes": pool.resident_bytes(), "capacity": pool._capacity, "row_height": pool._row_height, "resident_masks": pool.entries.size(), "upload_bytes": pool.upload_bytes})
	var file := FileAccess.open(_output.path_join("mask_measurements.json"), FileAccess.WRITE)
	file.store_string(JSON.stringify({"engine": Engine.get_version_info(), "adapter": RenderingServer.get_video_adapter_name(), "timings": _timings, "failures": _failures}, "\t"))
	_main.queue_free()
	await process_frame
	quit(0 if _failures == 0 else 1)

func _settle_scene(overlay: Node) -> void:
	await _settle_cell_overlay(overlay)
	var terrain: Node = _main.get_node("Terrain")
	var deadline := Time.get_ticks_msec() + 30000
	while (simulation.is_network_dirty() or simulation.is_terrain_dirty() or terrain.unready_patches_near_camera(700.0) > 0) and Time.get_ticks_msec() < deadline:
		await process_frame
	_expect(not simulation.is_network_dirty() and not simulation.is_terrain_dirty(), "Production road and graded terrain must settle")
	for frame in 60:
		await process_frame

func _capture_mask(label: String) -> void:
	for frame in 12:
		await process_frame
	await RenderingServer.frame_post_draw
	_expect(root.get_texture().get_image().save_png(_output.path_join("mask_" + label + ".png")) == OK, "Capture saves")

func _measure_ground(enabled: bool) -> void:
	var wall: Array = []
	var cpu: Array = []
	var gpu: Array = []
	for frame in 101:
		var start := Time.get_ticks_usec()
		await process_frame
		await RenderingServer.frame_post_draw
		wall.append((Time.get_ticks_usec() - start) / 1000.0)
		cpu.append(RenderingServer.viewport_get_measured_render_time_cpu(root.get_viewport_rid()))
		gpu.append(RenderingServer.viewport_get_measured_render_time_gpu(root.get_viewport_rid()))
	wall.sort()
	cpu.sort()
	gpu.sort()
	_timings.append({"shown": enabled, "frame_ms_p50": wall[50], "frame_ms_p95": wall[95], "render_cpu_ms_p50": cpu[50], "render_cpu_ms_p95": cpu[95], "gpu_ms_p50": gpu[50], "gpu_ms_p95": gpu[95]})

func _place_fixture_building() -> void:
	# Assets live in this capture's temporary directory, outside every installed mods folder.
	var directory := OS.get_temp_dir().path_join("metrum-mask-assets-%d" % OS.get_process_id())
	var pack := directory.path_join("fixture")
	DirAccess.make_dir_recursive_absolute(pack)
	var file := FileAccess.open(pack.path_join("pack.toml"), FileAccess.WRITE)
	file.store_string('pack_id = "mask-fixture"\nschema_version = 1\ndisplay_name = "Mask fixture"\nversion = "1.0.0"\nauthor = "Test"\nlicense = "CC0"\n')
	file.close()
	file = FileAccess.open(pack.path_join("asset.toml"), FileAccess.WRITE)
	file.store_string("""asset_id = "building.service.mask"
display_name = "Mask fixture service building"
[building]
placement_mode = "explicit"
lot_width_cells = 2
lot_depth_cells = 2
service_class = "water"
economy_profile = "water_plant_basic"
worker_capacity = 6
[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 8.0]
forward = [0.0, 0.0, 1.0]
[[mesh_parts]]
name = "main"
[[mesh_parts.lods]]
file = "model.glb"
distance_min_m = 0.0
""")
	file.close()
	var model := Node3D.new()
	var instance := MeshInstance3D.new()
	var mesh := BoxMesh.new()
	mesh.size = Vector3(15, 12, 15)
	instance.mesh = mesh
	instance.position.y = 6.0
	var material := StandardMaterial3D.new()
	material.albedo_color = Color(0.65, 0.58, 0.50)
	mesh.material = material
	model.add_child(instance)
	var document := GLTFDocument.new()
	var state := GLTFState.new()
	_expect(document.append_from_scene(model, state) == OK, "Fixture mesh exports")
	_expect(document.write_to_filesystem(state, pack.path_join("model.glb")) == OK, "Fixture GLB writes")
	model.free()
	_expect(simulation.load_all_asset_packs(directory).is_empty(), "Temporary fixture pack loads")
	var placement := simulation.place_service_building("mask-fixture:building.service.mask", -125.0, -20.0)
	_expect(placement.is_empty(), "Service building places: " + placement)
	_main.get_node("Buildings").lod_renderer.reload_resources()
	_main.get_node("Buildings").update_all_buildings()
