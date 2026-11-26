# SPDX-License-Identifier: GPL-3.0-or-later

## Main gameplay scene root.
##
## Attaches shared scene-level UI such as the top menu and exposes gameplay
## menu actions that delegate to the existing InputManager.
extends Node3D

const TopMenu = preload("res://scripts/ui/top_menu.gd")
const NewGameDialog = preload("res://scripts/ui/new_game_dialog.gd")
const SaveBrowser = preload("res://scripts/ui/save_browser.gd")
const GameplayRoadBenchmark = preload("res://scripts/benchmarks/gameplay_road_benchmark.gd")
const WORLDS_DIR := "user://worlds"

var _new_game_dialog: Window = null

@onready var input_manager = $InputManager

func _ready() -> void:
	if _handle_pending_launch_request():
		return
	_attach_top_menu()
	if _gameplay_road_benchmark_requested():
		var benchmark := GameplayRoadBenchmark.new()
		benchmark.name = "GameplayRoadBenchmark"
		add_child(benchmark)
		benchmark.call_deferred("run")

func _attach_top_menu() -> void:
	if has_node("TopMenu"):
		return
	var top_menu := TopMenu.new()
	top_menu.name = "TopMenu"
	top_menu.scene_kind = TopMenu.SCENE_GAMEPLAY
	add_child(top_menu)

func menu_new_game() -> void:
	SaveBrowser.open(_overlay_parent(), SaveBrowser.MODE_WORLD, WORLDS_DIR, _open_new_game_options)

# The save browser lives under TopMenu, which reports it as an open overlay to the camera.
func _overlay_parent() -> Node:
	var menu := get_node_or_null("TopMenu")
	return menu if menu else self

func menu_save() -> void:
	if input_manager:
		input_manager.menu_save_game()

func menu_main_menu() -> void:
	get_tree().change_scene_to_file("res://scenes/MainMenu.tscn")

func menu_load() -> void:
	if input_manager:
		input_manager.menu_load_game()

func menu_set_overlay_mode(mode: int) -> void:
	if input_manager:
		input_manager.menu_set_overlay_mode(mode)

func menu_toggle_zoning_overlay() -> void:
	if input_manager:
		input_manager.menu_toggle_zoning_overlay()

## Holds the rendered daylight at one hour of day, or follows the simulation clock again when
## `hour` is negative. The clock itself never stops: only the lighting is pinned.
func menu_set_time_of_day(hour: float) -> void:
	var lighting := get_node_or_null("SceneLighting")
	if lighting:
		lighting.pin_hour_of_day(hour)

## Sets the weather to scenario `index` of [constant TopMenu.WEATHER_SCENARIOS] at once and
## holds it there: the weather schedule stops.
func menu_set_weather(index: int) -> void:
	var simulation := get_node_or_null("SimulationNode")
	if simulation:
		simulation.set_weather_scenario(index, 0.0)

## Hands the weather to the seeded schedule, or takes it away.
func menu_set_weather_automatic(automatic: bool) -> void:
	var simulation := get_node_or_null("SimulationNode")
	if simulation:
		simulation.set_weather_automatic(automatic)

## Whether the seeded schedule moves the weather.
func menu_weather_automatic() -> bool:
	var simulation := get_node_or_null("SimulationNode")
	return simulation != null and simulation.get_weather_automatic()

func menu_open_asset_editor() -> void:
	_spawn_project_instance(["--asset-editor"])

func _spawn_project_instance(arguments: PackedStringArray) -> void:
	var launch_args := PackedStringArray()
	if not arguments.is_empty():
		launch_args.append("--")
		launch_args.append_array(arguments)
	var pid := OS.create_instance(launch_args)
	if pid == -1:
		push_error("Failed to launch a new project instance.")

func _open_new_game_options(path: String) -> void:
	if _new_game_dialog == null:
		_new_game_dialog = NewGameDialog.new()
		_new_game_dialog.start_requested.connect(_finish_new_game_world_selection)
		add_child(_new_game_dialog)
	_new_game_dialog.popup_for_world(path)

func _finish_new_game_world_selection(path: String, vegetation: Dictionary) -> void:
	# A player's new game preloads its terrain behind the loading screen (`TERRAIN-59`, `UI-15`).
	if input_manager and not await input_manager.player_load(input_manager.LoadKind.NEW_GAME, path, vegetation):
		# The running city stays; the world browser opens again saying why the world was refused.
		SaveBrowser.open(_overlay_parent(), SaveBrowser.MODE_WORLD, WORLDS_DIR,
			_open_new_game_options).show_refusal(input_manager.world_load_error())

func _handle_pending_launch_request() -> bool:
	if _should_allow_direct_gameplay_boot():
		return false
	if not LaunchState.has_pending_gameplay_request():
		get_tree().change_scene_to_file.call_deferred("res://scenes/MainMenu.tscn")
		return true

	var request: Dictionary = LaunchState.consume_pending_gameplay_request()
	_attach_top_menu()
	call_deferred("_apply_launch_request", request)
	return true

func _apply_launch_request(request: Dictionary) -> void:
	var save_path := str(request.get("save_path", ""))
	if not save_path.is_empty():
		if input_manager:
			# A player's load from the menu preloads its terrain behind the loading screen the
			# menu opened (`TERRAIN-59`, `UI-15`); a refusal leaves it up for the menu to close.
			if not await input_manager.player_load(input_manager.LoadKind.SAVE, save_path):
				LaunchState.note_refusal("save", input_manager.world_load_error())
				get_tree().change_scene_to_file("res://scenes/MainMenu.tscn")
		else:
			get_tree().change_scene_to_file("res://scenes/MainMenu.tscn")
		return

	var world_path := str(request.get("world_definition_path", ""))
	if not world_path.is_empty():
		if input_manager:
			var vegetation: Dictionary = request.get("vegetation", {})
			if not await input_manager.player_load(input_manager.LoadKind.NEW_GAME, world_path, vegetation):
				LaunchState.note_refusal("world", input_manager.world_load_error())
				get_tree().change_scene_to_file("res://scenes/MainMenu.tscn")
		else:
			get_tree().change_scene_to_file("res://scenes/MainMenu.tscn")
		return

	get_tree().change_scene_to_file("res://scenes/MainMenu.tscn")

func _should_allow_direct_gameplay_boot() -> bool:
	var args := OS.get_cmdline_user_args()
	return (
		"--benchmark" in args
		or "--generate-benchmark" in args
		or "--huge-map" in args
		or "--gameplay-road-benchmark" in args
	)

func _gameplay_road_benchmark_requested() -> bool:
	return "--gameplay-road-benchmark" in OS.get_cmdline_user_args()
