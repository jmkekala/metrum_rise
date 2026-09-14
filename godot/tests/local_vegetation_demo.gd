# SPDX-License-Identifier: GPL-2.0-only

## Local interactive Kuopio vegetation preview. F9 toggles trees; normal camera controls work.
## Run: godot --path godot --script res://tests/local_vegetation_demo.gd
## Loads the bootstrap world without modifying a save. Simulation starts paused.
extends SceneTree

var main: Node

func _initialize() -> void:
	call_deferred("run")

func run() -> void:
	main = load("res://scenes/Main.tscn").instantiate()
	main.set_script(null)
	root.add_child(main)
	current_scene = main
	await process_frame
	var input_manager: Node = main.get_node("InputManager")
	if not input_manager.menu_load_world_definition(ProjectSettings.globalize_path(
		"res://bootstrap/worlds/kuopio_324km2_10m.sqlite")):
		push_error("Could not load Kuopio")
		quit(1)
		return
	input_manager.set_simulation_speed(0.0)
	var height: float = main.get_node("SimulationNode").get_world_surface_height(Vector2.ZERO)
	main.get_node("CameraNode").focus_on(Vector3(0, height, 0), 120.0)
	root.title = "Kuopio tree prototype — F9: trees on/off — reload after world edits"
	root.window_input.connect(_on_input)

func _on_input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed and not event.echo and event.keycode == KEY_F9:
		var vegetation: Node = main.get_node("Vegetation")
		vegetation.enabled = not vegetation.enabled
