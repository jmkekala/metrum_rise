# SPDX-License-Identifier: GPL-2.0-only

## The stranded-walker notice and junction highlight (`ROAD-74`): the game scene shows nothing on
## an empty world, and on the stranded block fixture (`METRUM_STRANDED_FIXTURE`, written by the
## Rust recorder `record_stranded_walkers_fixture`) the real SimulationNode's report names the
## two corners where five walkers wait, the display rings both and the notice counts them, with
## residents waiting for a home after an eviction as a category of their own (`TRAFFIC-19`).
## METRUM_STRANDED_SCREENSHOT, when set on a rendered run, saves the view.
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
	var fixture := OS.get_environment("METRUM_STRANDED_FIXTURE")
	_expect(FileAccess.file_exists(fixture), "METRUM_STRANDED_FIXTURE must name the stranded block save, got '%s'" % fixture)
	var main: Node = load("res://scenes/Main.tscn").instantiate()
	# Own scene startup without the menu launch flow.
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	await process_frame
	var display: Node = main.get_node_or_null("StrandedDisplay")
	_expect(display != null, "the game scene must hold the stranded-walker display")
	if display == null or _failures > 0:
		_finish()
		return
	display.refresh()
	_expect(display.notice_text() == "" and display.shown_junctions.is_empty(), "an empty world shows no stranded walkers")

	var input_manager: Node = main.get_node("InputManager")
	_expect(input_manager.menu_load_game_from_path(fixture), "the stranded block fixture must load")
	input_manager.set_simulation_speed(0.0)
	# The display polls on its own; wait for it rather than calling it.
	var start := Time.get_ticks_msec()
	while display.shown_junctions.is_empty() and Time.get_ticks_msec() - start < TIMEOUT_MS:
		await process_frame
	_expect(display.shown_junctions == PackedInt32Array([1, 2]), "the block's corners 1 and 2 must be highlighted, got %s" % display.shown_junctions)
	_expect(display.notice_text() == "5 pedestrians stranded at 2 junctions: no way home", "the notice must count the walkers, got '%s'" % display.notice_text())
	var report: Dictionary = main.get_node("SimulationNode").get_stranded_walkers_report()
	_expect(report.get("walkers", PackedInt32Array()) == PackedInt32Array([3, 2]), "Rust must report 3 and 2 walkers, got %s" % report)
	# Residents waiting for a home after an eviction are a category of their own (`TRAFFIC-19`).
	var waiting := {"junctions": PackedInt32Array(), "stranded": 0, "waiting_for_home": 3}
	_expect(display.notice_message(waiting) == "3 residents waiting for a home", "the notice must count residents waiting for a home, got '%s'" % display.notice_message(waiting))
	waiting["junctions"] = PackedInt32Array([1])
	waiting["stranded"] = 1
	waiting["waiting_for_home"] = 1
	_expect(display.notice_message(waiting) == "1 pedestrian stranded at 1 junction: no way home; 1 resident waiting for a home", "both categories share the notice, got '%s'" % display.notice_message(waiting))
	_expect(int(report.get("waiting_for_home", -1)) == 0, "Rust must report no resident waiting for a home on the block, got %s" % report)
	var gridlock: Node = main.get_node_or_null("GridlockDisplay")
	_expect(gridlock != null and gridlock.notice_text() == "", "no gridlock is reported alongside")

	var screenshot := OS.get_environment("METRUM_STRANDED_SCREENSHOT")
	if screenshot != "":
		# Give the terrain and road streaming a few seconds to fill the view.
		await create_timer(5.0).timeout
		await RenderingServer.frame_post_draw
		_expect(root.get_texture().get_image().save_png(screenshot) == OK, "the screenshot must save")
	_finish()

func _finish() -> void:
	print("Stranded notice tests: %d failures" % _failures)
	quit(0 if _failures == 0 else 1)
