# SPDX-License-Identifier: GPL-2.0-only

## Immutable input/context for one asynchronous editor request; never simulation authority.
## Pointer position may advance while it runs, but mode, source and snap-target changes invalidate it.
extends RefCounted

var points: PackedVector3Array
var id: int
var _path_id: int
var _start: Vector3
var _control: Vector3
var _mode: int
var _state: int
var _forward: int
var _backward: int
var _snap: bool
var _target: Vector2i
var _generation: int
var _zoning_revision: int

func _init(tool: Node3D, input_points: PackedVector3Array, request_id: int) -> void:
	points = input_points
	id = request_id
	_path_id = tool.current_path.get_instance_id() if tool.current_path != null else 0
	_start = tool.start_pos
	_control = tool.control_pos
	_mode = tool.draw_mode
	_state = tool.current_state
	_forward = tool.fwd_lanes
	_backward = tool.bkw_lanes
	_snap = tool._snap_to_roads_enabled()
	_target = Vector2i(tool._sticky_network_snap.get("snap_edge", -1), tool._sticky_network_snap.get("snap_node", -1))
	_generation = tool.simulation_node.get_road_tool_surface_generation()
	_zoning_revision = tool.simulation_node.get_zoning_overlay_revision()

func matches_context(tool: Node3D) -> bool:
	return (
		_path_id == (tool.current_path.get_instance_id() if tool.current_path != null else 0)
		and _start == tool.start_pos and _control == tool.control_pos
		and _mode == tool.draw_mode and _state == tool.current_state
		and _forward == tool.fwd_lanes and _backward == tool.bkw_lanes
		and _snap == tool._snap_to_roads_enabled()
		and _target.x == int(tool._sticky_network_snap.get("snap_edge", -1))
		and _target.y == int(tool._sticky_network_snap.get("snap_node", -1))
		and _generation == tool.simulation_node.get_road_tool_surface_generation()
		and _zoning_revision == tool.simulation_node.get_zoning_overlay_revision()
	)
