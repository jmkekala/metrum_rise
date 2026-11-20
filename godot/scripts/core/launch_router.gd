# SPDX-License-Identifier: GPL-3.0-or-later

## Checks command-line arguments and loads the appropriate scene.
## This is the project's main scene entry point.
## Normal launch → MainMenu.tscn. --asset-editor → AssetEditor.tscn.
## --world-editor → WorldEditor.tscn.
## --gameplay-road-benchmark → Main.tscn with the deterministic Kuopio harness.
extends Node

const UserDataBootstrap = preload("res://scripts/core/user_data_bootstrap.gd")

func _ready() -> void:
	UserDataBootstrap.run()
	var args := OS.get_cmdline_user_args()
	if "--asset-editor" in args:
		get_tree().change_scene_to_file.call_deferred("res://scenes/AssetEditor.tscn")
	elif "--world-editor" in args:
		get_tree().change_scene_to_file.call_deferred("res://scenes/WorldEditor.tscn")
	elif (
		"--benchmark" in args
		or "--generate-benchmark" in args
		or "--huge-map" in args
		or "--gameplay-road-benchmark" in args
	):
		# A benchmark may pass through the menu for a dwell first (`RENDER-43`, the menu's
		# pipeline prewarm); the menu then opens the world scene.
		var dwell := OS.get_environment("METRUM_STREAM_MENU_DWELL_S")
		if dwell.is_valid_float():
			LaunchState.benchmark_menu_dwell_s = float(dwell)
			get_tree().change_scene_to_file.call_deferred("res://scenes/MainMenu.tscn")
		else:
			get_tree().change_scene_to_file.call_deferred("res://scenes/Main.tscn")
	else:
		get_tree().change_scene_to_file.call_deferred("res://scenes/MainMenu.tscn")
