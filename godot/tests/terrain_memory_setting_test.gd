# SPDX-License-Identifier: GPL-3.0-or-later

## The Terrain memory setting (`TERRAIN-41` B2): its stored choice round trips through the settings
## file and the graphics options (pending, applied, reset), an unknown value falls back to
## Automatic, each choice resolves to the budget the stream node takes (Automatic by the adapter
## type: 256 MB headless, where the renderer reports no discrete adapter), and Options' apply
## reaches the loaded world's stream node. Then a run-time change on a fresh 20 km blank world,
## paused: from every patch fitting to a budget below the built set and back, the releases and
## the fill move the built count to the new places through their units, no frame's predicted
## install ledger goes over its budget, and the debug console's `terrain` command reports it.
## With `-- --benchmark` it checks instead that a benchmark run reads the default whatever is
## stored. The settings file is restored. Headless; the game scene as the load flow builds it,
## without its input and camera processing.
extends SceneTree

const Settings = preload("res://scripts/core/game_settings.gd")
const SETTLE_TIMEOUT_MS := 90000
# Below the built set of the 20 km world, above its wanted square and release ring at EYE (near
# a corner) with the 256 pooled sets counted as places: about 800 of the 1,600 patches released.
const SMALL_BUDGET_MB := 45.0
const EYE := Vector2(-9000, -9000)

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var had_config := FileAccess.file_exists(Settings.CFG_PATH)
	var original := FileAccess.get_file_as_bytes(Settings.CFG_PATH) if had_config else PackedByteArray()
	if "--benchmark" in OS.get_cmdline_user_args():
		Settings.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_MEMORY, 1024)
		_expect(Settings.get_terrain_memory() == Settings.DEFAULT_TERRAIN_MEMORY,
			"a benchmark run reads the default Terrain memory whatever is stored")
	else:
		_test_choices()
		_test_options_panel()
		await _test_run_time_change()
	if had_config:
		var file := FileAccess.open(Settings.CFG_PATH, FileAccess.WRITE)
		file.store_buffer(original)
		file.close()
	else:
		DirAccess.remove_absolute(ProjectSettings.globalize_path(Settings.CFG_PATH))
	print("Terrain memory setting tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)

func _test_choices() -> void:
	DirAccess.remove_absolute(ProjectSettings.globalize_path(Settings.CFG_PATH))
	Settings.seed_default_config_if_missing()
	_expect(Settings.get_terrain_memory() == Settings.TERRAIN_MEMORY_AUTOMATIC, "a new settings file holds Automatic")
	var discrete := RenderingServer.get_video_adapter_type() == RenderingDevice.DEVICE_TYPE_DISCRETE_GPU
	_expect(Settings.automatic_terrain_memory_mb() == (512 if discrete else 256),
		"Automatic is 512 MB on a discrete adapter and 256 MB otherwise")
	if DisplayServer.get_name() == "headless":
		_expect(Settings.automatic_terrain_memory_mb() == 256, "headless reports no discrete adapter: 256 MB")
	for pair: Array in [[0, float(Settings.automatic_terrain_memory_mb())], [256, 256.0], [512, 512.0],
			[1024, 1024.0], [-1, 0.0], [300, float(Settings.automatic_terrain_memory_mb())]]:
		Settings.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_MEMORY, pair[0])
		_expect(is_equal_approx(Settings.get_terrain_memory_budget_mb(), pair[1]),
			"Terrain memory %d resolves to %.0f MB" % [pair[0], pair[1]])
	_expect(Settings.get_terrain_memory() == Settings.TERRAIN_MEMORY_AUTOMATIC, "an unknown value falls back to Automatic")

func _test_options_panel() -> void:
	Settings.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_MEMORY, Settings.TERRAIN_MEMORY_AUTOMATIC)
	var panel := preload("res://scripts/ui/graphics_options.gd").new()
	root.add_child(panel)
	var cycler: Variant = panel._choices[Settings.KEY_TERRAIN_MEMORY]
	_expect(cycler.selected == 0, "Options shows the stored Automatic")
	cycler.item_selected.emit(Settings.TERRAIN_MEMORY_CHOICES.find(1024))
	_expect(panel.has_pending_changes() and Settings.get_terrain_memory() == 0, "a pending choice is not stored")
	_expect(panel.apply_changes() == OK and Settings.get_terrain_memory() == 1024, "Apply stores 1,024 MB")
	panel.refresh()
	_expect(cycler.selected == Settings.TERRAIN_MEMORY_CHOICES.find(1024), "Options shows the stored 1,024 MB")
	cycler.item_selected.emit(Settings.TERRAIN_MEMORY_CHOICES.find(Settings.TERRAIN_MEMORY_WHOLE_MAP))
	_expect(panel.apply_changes() == OK and Settings.get_terrain_memory() == Settings.TERRAIN_MEMORY_WHOLE_MAP,
		"Apply stores Whole map")
	panel.reset_defaults()
	_expect(panel.has_pending_changes() and cycler.selected == 0, "Reset proposes Automatic without applying it")
	panel.free()

# The change at run time: the setting applied through Options' group call, then the budget moved
# below the built set and back with the stream node's own setter (the setting's presets do not bind
# on a 20 km world), every frame's predicted ledger within its budget.
func _test_run_time_change() -> void:
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
	Settings.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_MEMORY, Settings.TERRAIN_MEMORY_WHOLE_MAP)
	_expect(sim.create_blank_world(20480.0, 20480.0, 8.0, 0.0), "the 20 km blank world must load")
	sim.set_simulation_speed(0.0)
	input._refresh_after_world_load()
	_expect(int(stream.get_terrain_budget().get("budget_bytes", 0)) == -1, "Whole map: every patch fits")
	Settings.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_MEMORY, 512)
	call_group("graphics_settings", "apply_graphics_settings")
	_expect(int(stream.get_terrain_budget().get("budget_bytes", 0)) == 512000000, "Apply reaches the stream node")
	Settings.set_value(Settings.SECTION_GRAPHICS, Settings.KEY_TERRAIN_MEMORY, Settings.TERRAIN_MEMORY_WHOLE_MAP)
	call_group("graphics_settings", "apply_graphics_settings")
	camera.focus_on(Vector3(EYE.x, sim.get_world_surface_height(EYE), EYE.y), 120.0)
	# The load's hold runs the patch units without the frame budget; then the hand-over.
	var whole: Dictionary = await _settle(camera, stream, "the load, every patch fits", false)
	main.get_node("Terrain").set_load_settling(false)
	var grid := int(whole.get("terrain_built", 0))
	_expect(grid > 0 and int(whole.get("fill_left", -1)) == 0, "every patch fits: the fill builds the map: %s" % whole)
	# The backdrop's data (`TERRAIN-41` D1): built after the load and every block's three textures
	# uploaded (40 x 40 patches: 3 x 3 blocks of 199,177 bytes), nothing drawn; a block's land
	# cover is uploaded again as its patches' payloads land, so 27 uploads at least.
	var backdrop: Dictionary = await _backdrop_ready(camera, stream)
	_expect(bool(backdrop.get("ready", false)) and int(backdrop.get("blocks", 0)) == 9
		and int(backdrop.get("product_bytes", 0)) == 9 * 199177
		and int(backdrop.get("texture_bytes", 0)) == 9 * 199177
		and int(backdrop.get("pending", -1)) == 0
		and int(backdrop.get("uploads", 0)) >= 27, "the backdrop is built and uploaded: %s" % backdrop)
	stream.set_terrain_budget_mb(SMALL_BUDGET_MB)
	var small: Dictionary = await _settle(camera, stream, "%.0f MB" % SMALL_BUDGET_MB)
	var places := int(small.get("places", 0))
	_expect(int(small.get("terrain_built", 0)) < grid and int(small.get("terrain_built", 0)) <= places
		and int(small.get("water_built", 0)) <= places,
		"a smaller budget releases to its places: %s" % small)
	_expect(int(small.get("counted_bytes", 0)) <= int(small.get("budget_bytes", 0)), "settled within the target: %s" % small)
	_expect(int(small.get("terrain_releases_total", 0)) > 0, "the releases are counted")
	# The debug console's view of the same state (`terrain`).
	var console: Node = root.get_node_or_null("DebugConsole")
	if console != null:
		console._run_command("terrain")
		var text: String = console._view._status.text
		print("[terrain_memory_setting] console:\n%s" % text)
		_expect(text.contains("target 45 MB") and text.contains("Places %d" % places)
			and text.contains("water depth"), "the console's terrain command shows the budget: %s" % text)
	stream.set_terrain_budget_mb(0.0)
	var again: Dictionary = await _settle(camera, stream, "every patch fits again")
	_expect(int(again.get("terrain_built", 0)) == grid, "the fill builds the map again: %s" % again)
	main.queue_free()
	await process_frame

# Frames until the patches settle and the fill and releases are done; every frame's predicted
# install ledger within its budget. Answers the budget's last status.
func _settle(camera: Node, stream: Node, what: String, ledger := true) -> Dictionary:
	var start := Time.get_ticks_msec()
	var worst := 0.0
	var worst_measured := 0.0
	var status: Dictionary = {}
	while Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		status = stream.get_terrain_budget()
		worst = maxf(worst, float(status.get("ledger_predicted_us", 0.0)))
		worst_measured = maxf(worst_measured, float(status.get("ledger_us", 0.0)))
		var places := int(status.get("places", -1))
		var releases_left := places >= 0 and (int(status.get("terrain_built", 0)) > places
			or int(status.get("water_built", 0)) > places)
		if int(status.get("fill_left", 1)) == 0 and not releases_left \
				and _terrain_settled(stream):
			break
	print("[terrain_memory_setting] %s: built %d/%d places %s counted %.1f MB, ledger predicted at most %.0f us, measured at most %.0f us"
		% [what, int(status.get("terrain_built", 0)), int(status.get("water_built", 0)), str(status.get("places", "-")),
			float(status.get("counted_bytes", 0)) / 1e6, worst, worst_measured])
	_expect(Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS, "%s must settle" % what)
	if ledger:
		_expect(worst <= float(status.get("ledger_budget_us", 2000.0)),
			"%s: no frame's predicted ledger over its budget (%.0f us)" % [what, worst])
	return status

# Frames until the backdrop reports ready (or the settle timeout); answers its last status.
func _backdrop_ready(camera: Node, stream: Node) -> Dictionary:
	var start := Time.get_ticks_msec()
	var status: Dictionary = stream.get_backdrop()
	while not bool(status.get("ready", false)) and Time.get_ticks_msec() - start < SETTLE_TIMEOUT_MS:
		camera.advance_motion(1.0 / 60.0)
		await process_frame
		status = stream.get_backdrop()
	print("[terrain_memory_setting] backdrop: %s" % status)
	return status

func _terrain_settled(stream: Node) -> bool:
	var scene := current_scene
	var terrain: Node = scene.get_node("Terrain")
	var water: Node = scene.get_node("Water")
	return terrain.is_render_settled() and not water.has_pending_render_work(false) \
		and not stream.has_vegetation_pending_work()
