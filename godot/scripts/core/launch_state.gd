# SPDX-License-Identifier: GPL-2.0-only

## Autoload launch handoff state for scene-to-scene gameplay startup.
## Stores the next requested world/save selection so gameplay only opens after
## the player explicitly chooses content from the main menu.
extends Node

var pending_world_definition_path := ""
var pending_save_path := ""

func queue_new_game(world_definition_path: String) -> void:
	pending_world_definition_path = world_definition_path
	pending_save_path = ""

func queue_load_game(save_path: String) -> void:
	pending_save_path = save_path
	pending_world_definition_path = ""

func clear_pending_gameplay_request() -> void:
	pending_world_definition_path = ""
	pending_save_path = ""

func has_pending_gameplay_request() -> bool:
	return not pending_world_definition_path.is_empty() or not pending_save_path.is_empty()

func consume_pending_gameplay_request() -> Dictionary:
	var payload := {
		"world_definition_path": pending_world_definition_path,
		"save_path": pending_save_path,
	}
	clear_pending_gameplay_request()
	return payload
