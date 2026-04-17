## Main gameplay scene root.
##
## Attaches shared scene-level UI such as the top menu and exposes gameplay
## menu actions that delegate to the existing InputManager.
extends Node3D

const TopMenu = preload("res://scripts/ui/top_menu.gd")

@onready var input_manager = $InputManager

func _ready() -> void:
	_attach_top_menu()

func _attach_top_menu() -> void:
	if has_node("TopMenu"):
		return
	var top_menu := TopMenu.new()
	top_menu.name = "TopMenu"
	top_menu.scene_kind = TopMenu.SCENE_GAMEPLAY
	add_child(top_menu)

func menu_new_game() -> void:
	get_tree().change_scene_to_file("res://scenes/Main.tscn")

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
	var err := OS.create_instance(arguments)
	if err != OK:
		push_error("Failed to launch a new project instance: %s" % err)
