## Centralized input orchestrator — owns tool activation state and global keyboard/mouse routing.
##
## Routes input events to the active tool node (RoadTool, ZoningTool,
## MoveTool, LaneTool, CulDeSacTool), calls SimulationNode directly for global undo/save/load/sim-speed actions,
## and refreshes the thin Godot render nodes after world mutations.
##
## When no tool is active (Tool.NONE) left-clicking any building opens the
## building stats panel via InspectTool, which is always present in the scene.
extends Node

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"
@onready var zoning_overlay = $"../ZoningOverlay"
@onready var water_node = $"../Water"
@onready var road_tool = $"../RoadTool"
@onready var zoning_tool = $"../ZoningTool"
@onready var move_tool = $"../MoveTool"
var cul_de_sac_tool: Node3D
@onready var main_ui = $"../MainUI"
@onready var agents_node = $"../Agents"
@onready var buildings_node = $"../Buildings"
var select_tool: Node3D
var inspect_tool: Node

enum Tool { NONE, ROAD, WALKWAY, ZONING, MOVE, AGENT, SCULPT, WATER, CUL_DE_SAC, SELECT }
var current_tool: Tool = Tool.NONE
const SIM_SPEED_STEPS := [0.0, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32]

func _ready():
	if not has_node("../CulDeSacTool"):
		var rt = Node3D.new()
		rt.name = "CulDeSacTool"
		rt.set_script(load("res://scripts/cul_de_sac_tool.gd"))
		get_parent().call_deferred("add_child", rt)
		cul_de_sac_tool = rt
	
	if not has_node("../SelectTool"):
		var st = Node3D.new()
		st.name = "SelectTool"
		st.set_script(load("res://scripts/select_tool.gd"))
		get_parent().call_deferred("add_child", st)
		select_tool = st

	if not has_node("../InspectTool"):
		var it := Node.new()
		it.name = "InspectTool"
		it.set_script(load("res://scripts/inspect_tool.gd"))
		get_parent().call_deferred("add_child", it)
		inspect_tool = it

	# Hide overlay mesh if exists in cul-de-sac tool
	if cul_de_sac_tool and cul_de_sac_tool.has_node("PreviewMesh"):
		cul_de_sac_tool.get_node("PreviewMesh").visible = false
	# Removed old continuous sculpting polling
	pass

func _process(delta):
	_handle_camera_controls(delta)

func _handle_camera_controls(delta):
	var camera = get_viewport().get_camera_3d()
	if not camera: return
	
	# WASD Panning
	var pan_dir = Vector3.ZERO
	if Input.is_key_pressed(KEY_W): pan_dir.z -= 1.0
	if Input.is_key_pressed(KEY_S): pan_dir.z += 1.0
	if Input.is_key_pressed(KEY_A): pan_dir.x -= 1.0
	if Input.is_key_pressed(KEY_D): pan_dir.x += 1.0
	
	if pan_dir.length() > 0.0 and camera.has_method("pan"):
		camera.pan(pan_dir, 1.0, delta)
		
	# MMB Orbit
	if Input.is_mouse_button_pressed(MOUSE_BUTTON_MIDDLE) and camera.has_method("orbit"):
		var mouse_vel = Input.get_last_mouse_velocity()
		if mouse_vel.length() > 0.1:
			camera.orbit(mouse_vel, delta)

func _unhandled_input(event):
	if event is InputEventMouseButton and event.pressed:
		var camera = get_viewport().get_camera_3d()
		if camera and camera.has_method("zoom"):
			if event.button_index == MOUSE_BUTTON_WHEEL_UP:
				camera.zoom(1.0)
			elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
				camera.zoom(-1.0)

	if event is InputEventKey and event.pressed and not event.echo:
		match event.keycode:
			KEY_M: _toggle_tool(Tool.MOVE)
			KEY_R: _toggle_tool(Tool.ROAD)
			KEY_X: _toggle_tool(Tool.WALKWAY)
			KEY_Y: _toggle_tool(Tool.SCULPT)
			KEY_K: _toggle_tool(Tool.WATER) # Moved from W to avoid WASD overlap
			KEY_C: _toggle_tool(Tool.CUL_DE_SAC) # Cul-De-Sac (C = Circle/CulDeSac)
			KEY_V: _toggle_tool(Tool.SELECT) # Moved from S to avoid WASD overlap
			KEY_Z: 
				if event.ctrl_pressed:
					_handle_undo()
				else:
					if main_ui: main_ui._on_zoning_main_pressed()
			KEY_S:
				if event.ctrl_pressed:
					_handle_save_game()
			KEY_P: _toggle_agent_paths()
			KEY_SPACE: _toggle_pause()
			KEY_ENTER: _handle_export()
			KEY_L:
				if event.ctrl_pressed:
					_handle_load_game()
				else:
					_handle_import()
			
			# Lane Adjustments (Forward)
			KEY_BRACKETRIGHT, KEY_UP: _handle_lane_adjust(1, 0)
			KEY_BRACKETLEFT, KEY_DOWN: _handle_lane_adjust(-1, 0)
			# Lane Adjustments (Backward)
			KEY_PERIOD: _handle_lane_adjust(0, 1)
			KEY_COMMA: _handle_lane_adjust(0, -1)
			
			# Overlay Modes
			KEY_7, KEY_8, KEY_9, KEY_0:
				_handle_overlay_mode(event.keycode)

			# Altitude Adjustments
			KEY_PAGEUP: _handle_altitude_adjust(2.5)
			KEY_PAGEDOWN: _handle_altitude_adjust(-2.5)

			# Zoning Selection
			KEY_1, KEY_2, KEY_3:
				_handle_zoning_selection(event.keycode)

			KEY_ESCAPE:
				_handle_escape()

	if event is InputEventMouseButton:
		_handle_mouse(event)

func _handle_escape():
	if current_tool != Tool.NONE:
		_cancel_active_tool()

func _handle_lane_adjust(fwd, bkw):
	if current_tool == Tool.ROAD and road_tool:
		road_tool.adjust_lanes(fwd, bkw)

func _toggle_agent_paths():
	if agents_node:
		agents_node.show_paths = not agents_node.show_paths
		if not agents_node.show_paths:
			agents_node.debug_mesh.clear_surfaces()
		print("Agent Path Debug: ", agents_node.show_paths)

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
	# Close the building inspector whenever any dedicated tool activates.
	if enabled and inspect_tool:
		inspect_tool.close_window()
	match tool_type:
		Tool.MOVE: if move_tool: move_tool.active = enabled
		Tool.ROAD:
			if road_tool:
				road_tool.active = enabled
				if enabled:
					road_tool.fwd_lanes = 1
					road_tool.bkw_lanes = 1
					road_tool._update_lanes_label()
					road_tool._ghost_guides_dirty = true
					road_tool._rebuild_ghost_lines()
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
			if zoning_overlay: zoning_overlay.set_tool_active(enabled)
		Tool.SELECT:
			if select_tool: select_tool.active = enabled

func _toggle_zoning_overlay():
	_toggle_tool(Tool.ZONING)

func _handle_undo():
	# Zoning tool maintains its own undo stack for zone paint operations.
	if current_tool == Tool.ZONING and zoning_tool:
		zoning_tool.undo()
		return
	if simulation_node.undo_action():
		print("Undo Executed Globally")
		if terrain_node: terrain_node.update_terrain_visuals()
		if road_tool:
			road_tool.update_main_mesh()
			road_tool._ghost_guides_dirty = true
			road_tool._rebuild_ghost_lines()

func _savegame_path() -> String:
	return ProjectSettings.globalize_path("user://savegame.sqlite")

func _refresh_after_world_load():
	if road_tool and road_tool.current_state != 0:
		road_tool.cancel_road()
	if move_tool and move_tool.current_state != 0:
		move_tool.cancel_move()
	_cancel_active_tool()
	if terrain_node:
		terrain_node.rebuild_from_simulation_state()
	if water_node:
		water_node.rebuild_from_simulation_state()
	if road_tool:
		road_tool.update_main_mesh()
	if buildings_node:
		buildings_node.reload_asset_packs()
		buildings_node.update_all_buildings()
	if zoning_overlay: zoning_overlay.full_refresh()
	if agents_node:
		agents_node.update_swarm()

func _handle_save_game():
	var path = _savegame_path()
	if simulation_node.save_game(path):
		print("Saved game to: ", path)
	else:
		push_error("Save failed: " + path)

func _handle_load_game():
	var path = _savegame_path()
	if simulation_node.load_game(path):
		_refresh_after_world_load()
		print("Loaded game from: ", path)
	else:
		push_error("Load failed: " + path)

func _toggle_pause():
	var speed: float = 0.0 if terrain_node.sim_speed > 0.0 else 1.0
	set_simulation_speed(speed)

func set_simulation_speed(speed: float):
	var clamped_speed: float = maxf(speed, 0.0)
	terrain_node.sim_speed = clamped_speed
	simulation_node.set_simulation_speed(clamped_speed)
	if main_ui and main_ui.has_method("set_sim_speed_display"):
		main_ui.set_sim_speed_display(clamped_speed)
	print("Sim speed set to: ", clamped_speed)

func step_simulation_speed(direction: int):
	var current_speed: float = terrain_node.sim_speed
	var target_index := 0
	if direction > 0:
		target_index = SIM_SPEED_STEPS.size() - 1
		for i in range(SIM_SPEED_STEPS.size()):
			if SIM_SPEED_STEPS[i] > current_speed + 0.001:
				target_index = i
				break
	else:
		target_index = 0
		for i in range(SIM_SPEED_STEPS.size() - 1, -1, -1):
			if SIM_SPEED_STEPS[i] < current_speed - 0.001:
				target_index = i
				break
	set_simulation_speed(SIM_SPEED_STEPS[target_index])

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
	if current_tool != Tool.ZONING:
		_toggle_tool(Tool.ZONING)
	if not zoning_tool:
		return
	match keycode:
		KEY_1: zoning_tool.select_profile_by_zone_type("residential")
		KEY_2: zoning_tool.select_profile_by_zone_type("commercial")
		KEY_3: zoning_tool.select_profile_by_zone_type("industrial")

func select_zone_profile(runtime_id: int) -> void:
	if current_tool != Tool.ZONING:
		_toggle_tool(Tool.ZONING)
	if zoning_tool:
		zoning_tool.select_profile(runtime_id)

func set_zoning_paint_mode(mode: String) -> void:
	if current_tool != Tool.ZONING:
		_toggle_tool(Tool.ZONING)
	if zoning_tool:
		zoning_tool.set_paint_mode(mode)

func _handle_mouse(event):
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		# In default (no-tool) mode a left-click tries to inspect a building.
		if current_tool == Tool.NONE and inspect_tool:
			_handle_inspect_click(event.position)
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		_handle_right_click()

func _handle_inspect_click(mouse_pos: Vector2) -> void:
	var camera := get_viewport().get_camera_3d()
	if not camera:
		return
	var pos = simulation_node.intersect_terrain(
		camera.project_ray_origin(mouse_pos),
		camera.project_ray_normal(mouse_pos)
	)
	if pos == null:
		return
	inspect_tool.try_inspect(pos)

func _handle_right_click():
	if (current_tool == Tool.ROAD or current_tool == Tool.WALKWAY) and road_tool.current_state != 0:
		road_tool.cancel_road()
	elif current_tool == Tool.MOVE and move_tool.current_state != 0:
		move_tool.cancel_move()
	elif current_tool == Tool.SCULPT:
		pass # Right click is used for lowering ground during sculpting

	else:
		_cancel_active_tool()

func _handle_altitude_adjust(delta):
	if (current_tool == Tool.ROAD or current_tool == Tool.WALKWAY) and road_tool:
		road_tool.adjust_altitude(delta)
