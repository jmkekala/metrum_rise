# SPDX-License-Identifier: GPL-2.0-only

## Checks command-line arguments and loads the appropriate scene.
## This is the project's main scene entry point.
## Normal launch → MainMenu.tscn. --asset-editor → AssetEditor.tscn.
## --economy-editor → EconomyEditor.tscn. --world-editor → WorldEditor.tscn.
## --gameplay-road-benchmark → Main.tscn with the deterministic Kuopio harness.
extends Node

const UserDataBootstrap = preload("res://scripts/core/user_data_bootstrap.gd")

func _ready() -> void:
	UserDataBootstrap.run()
	var args := OS.get_cmdline_user_args()
	if "--asset-editor" in args:
		get_tree().change_scene_to_file.call_deferred("res://scenes/AssetEditor.tscn")
	elif "--economy-editor" in args:
		get_tree().change_scene_to_file.call_deferred("res://scenes/EconomyEditor.tscn")
	elif "--world-editor" in args:
		get_tree().change_scene_to_file.call_deferred("res://scenes/WorldEditor.tscn")
	elif (
		"--benchmark" in args
		or "--generate-benchmark" in args
		or "--huge-map" in args
		or "--gameplay-road-benchmark" in args
	):
		get_tree().change_scene_to_file.call_deferred("res://scenes/Main.tscn")
	else:
		get_tree().change_scene_to_file.call_deferred("res://scenes/MainMenu.tscn")
