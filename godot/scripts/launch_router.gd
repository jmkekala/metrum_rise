## Checks command-line arguments and loads the appropriate scene.
## This is the project's main scene entry point.
## Normal launch → Main.tscn. --asset-editor → AssetEditor.tscn.
extends Node

func _ready() -> void:
	var args := OS.get_cmdline_user_args()
	if "--asset-editor" in args:
		get_tree().change_scene_to_file.call_deferred("res://scenes/AssetEditor.tscn")
	else:
		get_tree().change_scene_to_file.call_deferred("res://scenes/Main.tscn")
