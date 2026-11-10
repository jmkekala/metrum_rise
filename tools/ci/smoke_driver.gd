# SPDX-License-Identifier: GPL-2.0-only

## Release smoke-test driver. `tools/ci/smoke.sh` injects it into an exported game as an autoload
## through `override.cfg`; it is never part of the game itself. Once the launch router has seeded
## user data, it starts a new game on `SMOKE_WORLD` the way the main menu does, runs
## `SMOKE_FRAMES` frames, and quits with 0 only when the gameplay scene holds a real
## `SimulationNode` (a failed extension class leaves a placeholder) and the request was consumed.
extends Node

func _ready() -> void:
	_run.call_deferred()

func _frames(count: int) -> void:
	for i in count:
		await get_tree().process_frame

func _run() -> void:
	# The router seeds user data and moves to the main menu within the first frames.
	await _frames(5)
	var launch := get_node("/root/LaunchState")
	launch.queue_new_game(ProjectSettings.globalize_path(OS.get_environment("SMOKE_WORLD")), {})
	get_tree().change_scene_to_file("res://scenes/Main.tscn")
	await _frames(int(OS.get_environment("SMOKE_FRAMES")))
	var scene := get_tree().current_scene
	var simulation: Node = scene.get_node_or_null("SimulationNode") if scene else null
	var ok: bool = (
		scene != null
		and scene.name == "Main"
		and simulation != null
		and simulation.get_class() == "SimulationNode"
		and not launch.has_pending_gameplay_request()
	)
	print("SMOKE result=%s scene=%s simulation=%s" % [
		"ok" if ok else "fail",
		scene.name if scene else "<none>",
		simulation.get_class() if simulation else "<none>",
	])
	get_tree().quit(0 if ok else 1)
