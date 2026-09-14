# SPDX-License-Identifier: GPL-2.0-only

## Local, opt-in capture of the day/night cycle. Writes screenshots only; saves no state.
## Run from repo root:
##   godot --path godot --windowed --resolution 1600x900 --script res://tests/day_cycle_capture.gd
## METRUM_DAYCYCLE_OUTPUT selects a fresh output directory.
## METRUM_DAYCYCLE_WORLD selects a world SQLite.
## METRUM_DAYCYCLE_HOURS overrides the captured hours as a comma separated list.
##
## The world is loaded and settled once per camera pose, then the clock is pinned hour by
## hour, so the whole sweep costs one world load rather than one per hour.
extends SceneTree

const DayCycleConfig := preload("res://scripts/core/day_cycle.gd")
const DEFAULT_HOURS := [2.0, 4.0, 4.6, 5.5, 7.0, 9.0, 12.0, 16.0, 18.5, 19.5, 20.2, 21.0, 22.5]
const VIEWS := {"wide": 900.0, "near": 140.0}

var main: Node
var camera: Node
var terrain: Node
var water: Node
var vegetation: Node
var lighting: Node
var output_dir: String
var pivot := Vector3.ZERO
var records: Array = []

func _initialize() -> void:
	call_deferred("run")

func run() -> void:
	output_dir = OS.get_environment("METRUM_DAYCYCLE_OUTPUT")
	if output_dir.is_empty():
		output_dir = ProjectSettings.globalize_path(
			"res://../benchmark-results/daycycle-%d" % Time.get_unix_time_from_system()
		)
	if DirAccess.dir_exists_absolute(output_dir):
		push_error("Output directory already exists: " + output_dir)
		quit(1)
		return
	DirAccess.make_dir_recursive_absolute(output_dir)

	main = load("res://scenes/Main.tscn").instantiate()
	# Own scene startup without attaching the road workload or changing menu launch state.
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame

	var input_manager: Node = main.get_node("InputManager")
	input_manager.set_process(false)
	input_manager.set_process_input(false)
	input_manager.set_process_unhandled_input(false)
	root.set_disable_input(true)
	var world := OS.get_environment("METRUM_DAYCYCLE_WORLD")
	if world.is_empty():
		world = ProjectSettings.globalize_path("res://bootstrap/worlds/kuopio_324km2_10m.sqlite")
	if not input_manager.menu_load_world_definition(world):
		push_error("Could not load world: " + world)
		quit(1)
		return
	input_manager.set_simulation_speed(0.0)

	camera = main.get_node("CameraNode")
	camera.set_process(false)
	camera.set_process_input(false)
	camera.set_process_unhandled_input(false)
	terrain = main.get_node("Terrain")
	water = main.get_node("Water")
	vegetation = main.get_node("Vegetation")
	lighting = main.get_node("SceneLighting")
	pivot.y = main.get_node("SimulationNode").get_world_surface_height(Vector2.ZERO)

	DisplayServer.window_set_mode(DisplayServer.WINDOW_MODE_WINDOWED)
	DisplayServer.window_set_size(Vector2i(1600, 900))
	DisplayServer.window_set_vsync_mode(DisplayServer.VSYNC_DISABLED)
	Engine.max_fps = 0

	var hours := _requested_hours()
	for view_name in VIEWS:
		camera.focus_on(pivot, float(VIEWS[view_name]))
		if not await _settle():
			return
		for hour in hours:
			await _capture(str(view_name), float(hour))

	var file := FileAccess.open(output_dir.path_join("palette.json"), FileAccess.WRITE)
	file.store_string(JSON.stringify(records, "  "))
	file.close()
	print("[daycycle] wrote %d captures to %s" % [records.size(), output_dir])
	quit(0)

func _requested_hours() -> Array:
	var requested := OS.get_environment("METRUM_DAYCYCLE_HOURS").strip_edges()
	if requested.is_empty():
		return DEFAULT_HOURS
	var hours: Array = []
	for token in requested.split(",", false):
		var text := String(token).strip_edges()
		if text.is_valid_float():
			hours.append(text.to_float())
	return hours if not hours.is_empty() else DEFAULT_HOURS

func _settle() -> bool:
	var start := Time.get_ticks_msec()
	var stable := 0
	while stable < 30 and Time.get_ticks_msec() - start < 180000:
		await process_frame
		if (
			not terrain.has_pending_render_work(false)
			and not water.has_pending_render_work(false)
			and not vegetation.has_pending_work()
		):
			stable += 1
		else:
			stable = 0
	if stable < 30:
		push_error("World did not settle within 180 seconds")
		quit(1)
		return false
	return true

func _capture(view_name: String, hour: float) -> void:
	lighting.pin_hour_of_day(hour)
	# The sky radiance refreshes incrementally, so the dome needs a few frames to catch up
	# with a clock that jumped hours in one step. Gameplay never makes a jump this large.
	for _frame in range(24):
		await process_frame
	await RenderingServer.frame_post_draw
	var label := "%s-%02d%02d" % [view_name, int(hour), int(fposmod(hour, 1.0) * 60.0)]
	root.get_texture().get_image().save_png(output_dir.path_join(label + ".png"))
	var sample = lighting.current_sample()
	records.append({
		"view": view_name,
		"hour": hour,
		"sun_elevation_deg": sample.sun_elevation_deg,
		"sun_azimuth_deg": sample.sun_azimuth_deg,
		"moonlit": sample.is_moonlit,
		"key_energy": sample.key_energy,
		"key_color": [sample.key_color.r, sample.key_color.g, sample.key_color.b],
		"sky_horizon": [sample.sky_horizon.r, sample.sky_horizon.g, sample.sky_horizon.b],
		"fog": [sample.fog_color.r, sample.fog_color.g, sample.fog_color.b],
		"ambient_energy": sample.ambient_energy,
		"ambient_scale": [
			sample.ambient_light_scale.r,
			sample.ambient_light_scale.g,
			sample.ambient_light_scale.b,
		],
	})
	print("[daycycle] %s elevation=%.2f key=%.3f" % [label, sample.sun_elevation_deg, sample.key_energy])
