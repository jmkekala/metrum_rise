extends Node

# Centralized Input Orchestrator
# Manages tool switching, system shortcuts, and global modal state.

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"
@onready var road_tool = $"../RoadTool"
@onready var zoning_tool = $"../ZoningTool"
@onready var move_tool = $"../MoveTool"
@onready var lane_tool = $"../LaneTool"
var cul_de_sac_tool: Node3D
@onready var main_ui = $"../MainUI"
@onready var agents_node = $"../Agents"

enum Tool { NONE, ROAD, WALKWAY, ZONING, MOVE, LANE, AGENT, SCULPT, WATER, CUL_DE_SAC }
var current_tool: Tool = Tool.NONE

func _ready():
	if not has_node("../CulDeSacTool"):
		var rt = Node3D.new()
		rt.name = "CulDeSacTool"
		rt.set_script(load("res://scripts/cul_de_sac_tool.gd"))
		get_parent().call_deferred("add_child", rt)
		cul_de_sac_tool = rt
	
	# Hide overlay mesh if exists in cul-de-sac tool
	if cul_de_sac_tool and cul_de_sac_tool.has_node("PreviewMesh"):
		cul_de_sac_tool.get_node("PreviewMesh").visible = false
	# Removed old continuous sculpting polling
	pass

func _unhandled_input(event):
	if event is InputEventKey and event.pressed and not event.echo:
		match event.keycode:
			KEY_M: _toggle_tool(Tool.MOVE)
			KEY_T: _toggle_tool(Tool.LANE)
			KEY_R: _toggle_tool(Tool.ROAD)
			KEY_X: _toggle_tool(Tool.WALKWAY)
			KEY_Y: _toggle_tool(Tool.SCULPT)
			KEY_K: _toggle_tool(Tool.WATER) # Moved from W to avoid WASD overlap
			KEY_C: _toggle_tool(Tool.CUL_DE_SAC) # Cul-De-Sac (C = Circle/CulDeSac)
			KEY_Z: 
				if event.ctrl_pressed:
					_handle_undo()
				else:
					if main_ui: main_ui._on_zoning_main_pressed()
			KEY_P: _toggle_agent_paths()
			KEY_SPACE: _toggle_pause()
			KEY_ENTER: _handle_export()
			KEY_L: _handle_import()
			
			# Lane Adjustments (Forward)
			KEY_BRACKETRIGHT, KEY_UP: _handle_lane_adjust(1, 0)
			KEY_BRACKETLEFT, KEY_DOWN: _handle_lane_adjust(-1, 0)
			# Lane Adjustments (Backward)
			KEY_PERIOD: _handle_lane_adjust(0, 1)
			KEY_COMMA: _handle_lane_adjust(0, -1)
			
			# Overlay Modes
			KEY_7, KEY_8, KEY_9, KEY_0:
				_handle_overlay_mode(event.keycode)

			# Zoning Selection
			KEY_1, KEY_2, KEY_3, KEY_4, KEY_5:
				_handle_zoning_selection(event.keycode)

			KEY_ESCAPE:
				_handle_escape()

	if event is InputEventMouseButton:
		_handle_mouse(event)

func _handle_escape():
	if current_tool != Tool.NONE:
		_cancel_active_tool()
	elif terrain_node and terrain_node.show_global_zoning:
		_toggle_zoning_overlay()

func _toggle_agent_paths():
	if agents_node:
		agents_node.show_paths = not agents_node.show_paths
		if not agents_node.show_paths:
			agents_node.debug_mesh.clear_surfaces()
		print("Agent Path Debug: ", agents_node.show_paths)

func _handle_lane_adjust(fwd, bkw):
	if current_tool == Tool.ROAD and road_tool:
		road_tool.adjust_lanes(fwd, bkw)

# --- Logic Hub ---

func _toggle_tool(tool_type: Tool):
	if current_tool == tool_type:
		_cancel_active_tool()
	else:
		_cancel_active_tool()
		current_tool = tool_type
		_activate_tool_logic(current_tool, true)
		print("Tool Switched to: ", Tool.keys()[current_tool])

func _cancel_active_tool():
	if current_tool != Tool.NONE:
		_activate_tool_logic(current_tool, false)
		current_tool = Tool.NONE

func _activate_tool_logic(tool_type: Tool, enabled: bool):
	match tool_type:
		Tool.MOVE: if move_tool: move_tool.active = enabled
		Tool.LANE: if lane_tool: lane_tool.active = enabled
		Tool.ROAD: 
			if road_tool: 
				road_tool.active = enabled
				if enabled:
					road_tool.fwd_lanes = 1
					road_tool.bkw_lanes = 1
					road_tool._update_lanes_label()
		Tool.WALKWAY: 
			if road_tool: 
				road_tool.active = enabled
				if enabled:
					road_tool.fwd_lanes = 0
					road_tool.bkw_lanes = 0
					road_tool._update_lanes_label()
		Tool.CUL_DE_SAC:
			if cul_de_sac_tool:
				cul_de_sac_tool.active = enabled
		Tool.AGENT: if agents_node: pass # Agents diag always available
		Tool.ZONING: 
			if zoning_tool: zoning_tool.active = enabled
			if terrain_node: terrain_node.show_global_zoning = enabled

func _toggle_zoning_overlay():
	if terrain_node:
		terrain_node.show_global_zoning = not terrain_node.show_global_zoning
		if terrain_node.show_global_zoning:
			_toggle_tool(Tool.ZONING)
		else:
			if current_tool == Tool.ZONING:
				_cancel_active_tool()

func _handle_undo():
	if simulation_node.undo_action():
		print("Undo Executed Globally")
		if terrain_node: terrain_node.update_terrain_visuals()
		if road_tool: road_tool.update_main_mesh()

func _toggle_pause():
	var speed = 0.0 if terrain_node.sim_speed > 0.0 else 1.0
	terrain_node.sim_speed = speed
	simulation_node.set_simulation_speed(speed)
	print("Sim speed set to: ", speed)

func _handle_export():
	if terrain_node: terrain_node.export_heightmap("user://map_export.png")

func _handle_import():
	if terrain_node: terrain_node.import_heightmap("user://map_export.png")

func _handle_overlay_mode(keycode):
	var mode = 0
	match keycode:
		KEY_7: mode = 0
		KEY_8: mode = 1
		KEY_9: mode = 2
		KEY_0: mode = 3
	if terrain_node: terrain_node.overlay_mode = mode

func _handle_zoning_selection(keycode):
	var type = keycode - KEY_0 # 1-5
	if current_tool != Tool.ZONING:
		_toggle_tool(Tool.ZONING)
	if zoning_tool: zoning_tool.current_zone_type = type

func _handle_mouse(event):
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		_handle_right_click()

func _handle_right_click():
	if (current_tool == Tool.ROAD or current_tool == Tool.WALKWAY) and road_tool.current_state != 0:
		road_tool.cancel_road()
	elif current_tool == Tool.MOVE and move_tool.current_state != 0:
		move_tool.cancel_move()
	elif current_tool == Tool.SCULPT:
		pass # Right click is used for lowering ground during sculpting

	else:
		_cancel_active_tool()
