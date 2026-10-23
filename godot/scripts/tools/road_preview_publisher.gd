# SPDX-License-Identifier: GPL-2.0-only

## Runs one late road-preview publication pass after ordinary scene processing.
## The owning RoadTool controls activation; this node never samples input or dispatches work.
extends Node

var _road_tool: Node

func _ready() -> void:
	# Godot auto-enables callbacks declared by a script when it enters the tree.
	set_process(false)

func bind(road_tool: Node, late_process_priority: int) -> void:
	_road_tool = road_tool
	process_priority = late_process_priority
	process_mode = Node.PROCESS_MODE_ALWAYS
	set_process(false)

func request_publication() -> void:
	set_process(true)

func cancel_publication() -> void:
	set_process(false)

func _process(_delta: float) -> void:
	if not is_instance_valid(_road_tool):
		set_process(false)
		return
	_road_tool._publish_preview_result_late()
