# SPDX-License-Identifier: GPL-2.0-only

## Main gameplay scene root.
##
## Attaches shared scene-level UI such as the top menu and exposes gameplay
## menu actions that delegate to the existing InputManager.
extends Node3D

const TopMenu = preload("res://scripts/ui/top_menu.gd")
const GameplayRoadBenchmark = preload("res://scripts/benchmarks/gameplay_road_benchmark.gd")
const WORLDS_DIR := "user://worlds"

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
	_ensure_worlds_dir()
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.filters = PackedStringArray(["*.sqlite ; WorldDefinition Files"])
	dialog.current_dir = ProjectSettings.globalize_path(WORLDS_DIR)
	dialog.file_selected.connect(func(path: String): _on_new_game_world_selected(path, dialog))
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(880, 620))

func menu_save() -> void:
	if input_manager:
		input_manager.menu_save_game()

func menu_load() -> void:
	if input_manager:
		input_manager.menu_load_game()

func menu_set_overlay_mode(mode: int) -> void:
	if input_manager:
		input_manager.menu_set_overlay_mode(mode)

func menu_toggle_zoning_overlay() -> void:
	if input_manager:
		input_manager.menu_toggle_zoning_overlay()

func menu_open_asset_editor() -> void:
	_spawn_project_instance(["--asset-editor"])

func menu_open_economy_editor() -> void:
	_spawn_project_instance(["--economy-editor"])

func _spawn_project_instance(arguments: PackedStringArray) -> void:
	var launch_args := PackedStringArray()
	if not arguments.is_empty():
		launch_args.append("--")
		launch_args.append_array(arguments)
	var pid := OS.create_instance(launch_args)
	if pid == -1:
		push_error("Failed to launch a new project instance.")

func _on_new_game_world_selected(path: String, dialog: FileDialog) -> void:
	dialog.hide()
	dialog.call_deferred("queue_free")
	call_deferred("_finish_new_game_world_selection", path)

func _finish_new_game_world_selection(path: String) -> void:
	if input_manager:
		input_manager.menu_load_world_definition(path)

func _ensure_worlds_dir() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(WORLDS_DIR))

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
			if not input_manager.menu_load_game_from_path(save_path):
				get_tree().change_scene_to_file("res://scenes/MainMenu.tscn")
		else:
			get_tree().change_scene_to_file("res://scenes/MainMenu.tscn")
		return

	var world_path := str(request.get("world_definition_path", ""))
	if not world_path.is_empty():
		if input_manager:
			if not input_manager.menu_load_world_definition(world_path):
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
