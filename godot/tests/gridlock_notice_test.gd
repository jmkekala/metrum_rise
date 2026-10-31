# SPDX-License-Identifier: GPL-2.0-only

## The gridlock notice and junction highlight (`TRAFFIC-14`): the game scene shows nothing on an
## empty world, and on the locked ring fixture (`METRUM_GRIDLOCK_FIXTURE`, written by the Rust
## recorder `record_gridlock_saturation_fixture`) it names the four junctions Rust reports and
## rings each of them. METRUM_GRIDLOCK_SCREENSHOT, when set on a rendered run, saves the view.
extends SceneTree

# Real time the display is given to show the loaded report (the load may still hold the core).
const TIMEOUT_MS := 60000

var _failures := 0

func _initialize() -> void:
	call_deferred("_run")

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _run() -> void:
	var fixture := OS.get_environment("METRUM_GRIDLOCK_FIXTURE")
	_expect(FileAccess.file_exists(fixture), "METRUM_GRIDLOCK_FIXTURE must name the locked ring save, got '%s'" % fixture)
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	# Own scene startup without the menu launch flow.
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	await process_frame
	var display: Node = main.get_node_or_null("GridlockDisplay")
	_expect(display != null, "the game scene must hold the gridlock display")
	if display == null or _failures > 0:
		_finish()
		return
	display.refresh()
	_expect(display.notice_text() == "" and display.shown_junctions.is_empty(), "an empty world shows no gridlock")

	var input_manager: Node = main.get_node("InputManager")
	_expect(input_manager.menu_load_game_from_path(fixture), "the locked ring fixture must load")
	input_manager.set_simulation_speed(0.0)
	# The display polls on its own; wait for it rather than calling it.
	var start := Time.get_ticks_msec()
	while display.shown_junctions.is_empty() and Time.get_ticks_msec() - start < TIMEOUT_MS:
		await process_frame
	_expect(display.shown_junctions == PackedInt32Array([0, 1, 2, 3]), "the ring's four corners must be highlighted, got %s" % display.shown_junctions)
	_expect(display.notice_text().contains("4 junctions"), "the notice must name the four locked junctions, got '%s'" % display.notice_text())
	var report: Dictionary = main.get_node("SimulationNode").get_gridlock_report()
	_expect(int(report.get("saturated", 0)) == 1 and int(report.get("detours", -1)) == 0, "Rust must report one saturated cycle and no detour, got %s" % report)

	var screenshot := OS.get_environment("METRUM_GRIDLOCK_SCREENSHOT")
	if screenshot != "":
		# Give the terrain and road streaming a few seconds to fill the view.
		await create_timer(5.0).timeout
		await RenderingServer.frame_post_draw
		_expect(root.get_texture().get_image().save_png(screenshot) == OK, "the screenshot must save")
	_finish()

func _finish() -> void:
	print("Gridlock notice tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
