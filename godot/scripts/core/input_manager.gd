# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: input_manager.gd
#  script_path: godot/scripts/core/input_manager.gd
#  module_name: input_manager
#  version: 0.1.0
#  description: Central input routing: which tool is active, and where a
#           key or click goes. Owns the lazy construction of the panels
#           it opens, including the building inspector and the traffic
#           report.
#  kind: core
#  spec: none
#  internal_dependencies: []
#  external_dependencies: [Godot 4.x]
#  features: [tool-activation, key-routing, panel-toggles]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-28
# =========================================================================

## Centralized input orchestrator — owns tool activation state and global keyboard/mouse routing.
##
## Routes input events to the active tool node (RoadTool, ZoningTool,
## MoveTool, LaneTool, CulDeSacTool, ServiceBuildingTool, IndustryBuildingTool, BulldozeTool), calls SimulationNode directly for global undo/save/load/sim-speed actions,
## and refreshes the thin Godot render nodes after world mutations.
##
## The Building Inspector helper is always present in the scene and can be
## opened by the active selection workflow.
extends Node

# =========================================================================
# THE DECLARATIONS
# =========================================================================

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"
@onready var zoning_overlay = $"../ZoningOverlay"
@onready var water_node = $"../Water"
@onready var road_tool = $"../RoadTool"
@onready var zoning_tool = $"../ZoningTool"
@onready var move_tool = $"../MoveTool"
var cul_de_sac_tool: Node3D
var service_building_tool: Node3D
var industry_building_tool: Node3D
var bulldoze_tool: Node3D
@onready var main_ui = $"../MainUI"
@onready var agents_node = $"../Agents"
@onready var buildings_node = $"../Buildings"
var select_tool: Node3D
var building_inspector: Node
var traffic_report: Node

enum Tool { NONE, ROAD, WALKWAY, ZONING, SERVICES, INDUSTRY, MOVE, AGENT, SCULPT, CUL_DE_SAC, SELECT, BULLDOZE }
var current_tool: Tool = Tool.NONE
const SIM_SPEED_STEPS := [0.0, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0, 32]
const DEPOSITS_OVERLAY_MODE := 4
const SAVES_DIR := "user://saves"
const WORLD_CAMERA_NEAR_CLIP_M := 0.5
const WORLD_CAMERA_MIN_FAR_M := 9000.0
const WORLD_CAMERA_FAR_MARGIN_M := 1000.0
const WORLD_CAMERA_PIVOT_CLEARANCE_M := 0.25
const WORLD_CAMERA_CLEARANCE_M := 1.5

var _current_save_path := ""
var _simulation_speed: float = 0.0
var _selected_industry_resource_id := ""
var _industry_deposits_overlay_forced := false
var _industry_previous_overlay_mode := 0

# =========================================================================
# READY
# =========================================================================
func _ready():
	if not has_node("../CulDeSacTool"):
		var rt = Node3D.new()
		rt.name = "CulDeSacTool"
		rt.set_script(load("res://scripts/tools/cul_de_sac_tool.gd"))
		get_parent().call_deferred("add_child", rt)
		cul_de_sac_tool = rt
	
	if not has_node("../SelectTool"):
		var st = Node3D.new()
		st.name = "SelectTool"
		st.set_script(load("res://scripts/tools/select_tool.gd"))
		get_parent().call_deferred("add_child", st)
		select_tool = st

	if not has_node("../ServiceBuildingTool"):
		var sbt = Node3D.new()
		sbt.name = "ServiceBuildingTool"
		sbt.set_script(load("res://scripts/tools/service_building_tool.gd"))
		get_parent().call_deferred("add_child", sbt)
		service_building_tool = sbt
	else:
		service_building_tool = get_node("../ServiceBuildingTool")

	if not has_node("../IndustryBuildingTool"):
		var ibt = Node3D.new()
		ibt.name = "IndustryBuildingTool"
		ibt.set_script(load("res://scripts/tools/industry_building_tool.gd"))
		get_parent().call_deferred("add_child", ibt)
		industry_building_tool = ibt
	else:
		industry_building_tool = get_node("../IndustryBuildingTool")

	if not has_node("../BulldozeTool"):
		var bt = Node3D.new()
		bt.name = "BulldozeTool"
		bt.set_script(load("res://scripts/tools/bulldoze_tool.gd"))
		get_parent().call_deferred("add_child", bt)
		bulldoze_tool = bt
	else:
		bulldoze_tool = get_node("../BulldozeTool")

	if has_node("../BuildingInspector"):
		building_inspector = get_node("../BuildingInspector")
	else:
		var inspector := Node.new()
		inspector.name = "BuildingInspector"
		inspector.set_script(load("res://scripts/ui/building_inspector.gd"))
		get_parent().call_deferred("add_child", inspector)
		building_inspector = inspector

	if has_node("../TrafficReport"):
		traffic_report = get_node("../TrafficReport")
	else:
		var report := Node.new()
		report.name = "TrafficReport"
		report.set_script(load("res://scripts/ui/traffic_report.gd"))
		get_parent().call_deferred("add_child", report)
		traffic_report = report

	# Hide overlay mesh if exists in cul-de-sac tool
	if cul_de_sac_tool and cul_de_sac_tool.has_node("PreviewMesh"):
		cul_de_sac_tool.get_node("PreviewMesh").visible = false
	# Removed old continuous sculpting polling
	call_deferred("_configure_world_camera_policy")

# =========================================================================
# CONFIGURE WORLD CAMERA POLICY
# =========================================================================
func _configure_world_camera_policy() -> void:
	var camera = get_parent().find_child("CameraNode", true, false)
	var debug_under_terrain: bool = _debug_camera_can_go_under_terrain()
	if camera and camera.has_method("set_clip_policy"):
		camera.set_clip_policy(
			WORLD_CAMERA_NEAR_CLIP_M,
			WORLD_CAMERA_MIN_FAR_M,
			WORLD_CAMERA_FAR_MARGIN_M
		)
	if camera and camera.has_method("set_terrain_clearance_policy"):
		camera.set_terrain_clearance_policy(
			not debug_under_terrain,
			WORLD_CAMERA_PIVOT_CLEARANCE_M,
			WORLD_CAMERA_CLEARANCE_M
		)
	if camera and camera.has_method("set_debug_under_terrain_enabled"):
		camera.set_debug_under_terrain_enabled(debug_under_terrain)

# =========================================================================
# DEBUG CAMERA CAN GO UNDER TERRAIN
# =========================================================================
func _debug_camera_can_go_under_terrain() -> bool:
	var debug_value: String = OS.get_environment("METRUM_DEBUG").strip_edges()
	return not debug_value.is_empty() and debug_value != "0"

# =========================================================================
# PROCESS
# =========================================================================
func _process(delta):
	if _ui_captures_keyboard_input():
		return
	_handle_camera_controls(delta)

# =========================================================================
# INPUT
# =========================================================================
func _input(event):
	if _ui_has_modal_popup():
		return
	if event is InputEventMouseButton:
		_handle_zoom_wheel(event)

# =========================================================================
# HANDLE CAMERA CONTROLS
# =========================================================================
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
			camera.orbit(mouse_vel * delta)

# =========================================================================
# UNHANDLED INPUT
# =========================================================================
func _unhandled_input(event):
	if _ui_captures_keyboard_input():
		return
	if event is InputEventKey and event.pressed and not event.echo:
		match event.keycode:
			KEY_M: _toggle_tool(Tool.MOVE)
			KEY_R: _toggle_tool(Tool.ROAD)
			KEY_X: _toggle_tool(Tool.WALKWAY)
			KEY_Y: _toggle_tool(Tool.SCULPT)
			KEY_C: _toggle_tool(Tool.CUL_DE_SAC) # Cul-De-Sac (C = Circle/CulDeSac)
			KEY_B: _toggle_tool(Tool.BULLDOZE)
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
			KEY_T: _toggle_traffic_report()
			KEY_SPACE:
				_toggle_pause()
				get_viewport().set_input_as_handled()
			KEY_L:
				if event.ctrl_pressed:
					_handle_load_game()
			KEY_F12:
				_handle_money_and_demand_cheat()
			
			# Lane Adjustments (Forward)
			KEY_BRACKETRIGHT, KEY_UP: _handle_lane_adjust(1, 0)
			KEY_BRACKETLEFT, KEY_DOWN: _handle_lane_adjust(-1, 0)
			# Lane Adjustments (Backward)
			KEY_PERIOD: _handle_lane_adjust(0, 1)
			KEY_COMMA: _handle_lane_adjust(0, -1)
			
			# Overlay Modes
			KEY_7, KEY_8, KEY_9, KEY_0, KEY_MINUS:
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

# =========================================================================
# HANDLE ZOOM WHEEL
# =========================================================================
func _handle_zoom_wheel(event: InputEventMouseButton) -> void:
	if not event.pressed:
		return

	var zoom_delta := 0.0
	if event.button_index == MOUSE_BUTTON_WHEEL_UP:
		zoom_delta = 1.0
	elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
		zoom_delta = -1.0
	else:
		return

	var camera = get_viewport().get_camera_3d()
	if camera and camera.has_method("zoom"):
		camera.zoom(zoom_delta)

# =========================================================================
# UI HAS MODAL POPUP
# =========================================================================
func _ui_has_modal_popup() -> bool:
	var viewport := get_viewport()
	var window := viewport as Window
	return (
		window != null
		and window.has_method("has_visible_popup")
		and window.has_visible_popup()
	)

# =========================================================================
# UI CAPTURES KEYBOARD INPUT
# =========================================================================
func _ui_captures_keyboard_input() -> bool:
	var viewport := get_viewport()
	var focus_owner := viewport.gui_get_focus_owner()
	var editing_focus := (
		focus_owner is SpinBox
		or focus_owner is LineEdit
		or focus_owner is TextEdit
		or focus_owner is CodeEdit
	)
	return _ui_has_modal_popup() or editing_focus

# =========================================================================
# HANDLE ESCAPE
# =========================================================================
func _handle_escape():
	if current_tool != Tool.NONE:
		_cancel_active_tool()

# =========================================================================
# HANDLE LANE ADJUST
# =========================================================================
func _handle_lane_adjust(fwd, bkw):
	if current_tool == Tool.ROAD and road_tool:
		road_tool.adjust_lanes(fwd, bkw)

# =========================================================================
# TOGGLE TRAFFIC REPORT
# =========================================================================
func _toggle_traffic_report():
	if traffic_report and traffic_report.has_method("toggle"):
		traffic_report.toggle()

# =========================================================================
# TOGGLE AGENT PATHS
# =========================================================================
func _toggle_agent_paths():
	if agents_node:
		agents_node.show_paths = not agents_node.show_paths
		if not agents_node.show_paths:
			if agents_node.has_method("clear_debug_overlay"):
				agents_node.clear_debug_overlay()
			else:
				agents_node.debug_mesh.clear_surfaces()
		print("Agent Path Debug: ", agents_node.show_paths)

# --- Logic Hub ---

# =========================================================================
# TOGGLE TOOL
# =========================================================================
func _toggle_tool(tool_type: Tool):
	if current_tool == tool_type:
		_cancel_active_tool()
	else:
		_cancel_active_tool()
		current_tool = tool_type
		_activate_tool_logic(current_tool, true)
		print("Tool Switched to: ", Tool.keys()[current_tool])

# =========================================================================
# CANCEL ACTIVE TOOL
# =========================================================================
func _cancel_active_tool():
	if current_tool != Tool.NONE:
		_activate_tool_logic(current_tool, false)
		current_tool = Tool.NONE

# =========================================================================
# ACTIVATE TOOL LOGIC
# =========================================================================
func _activate_tool_logic(tool_type: Tool, enabled: bool):
	# Close the building inspector whenever any dedicated tool activates.
	if enabled and building_inspector:
		building_inspector.close_window()
	match tool_type:
		Tool.MOVE: if move_tool: move_tool.active = enabled
		Tool.ROAD:
			if road_tool:
				if not enabled:
					road_tool.cancel_road()
				road_tool.active = enabled
				if enabled:
					road_tool.fwd_lanes = 1
					road_tool.bkw_lanes = 1
					road_tool._update_lanes_label()
					road_tool.mark_network_topology_dirty()
		Tool.WALKWAY: 
			if road_tool: 
				if not enabled:
					road_tool.cancel_road()
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
		Tool.SERVICES:
			if service_building_tool: service_building_tool.active = enabled
			if zoning_overlay: zoning_overlay.set_tool_active(enabled)
		Tool.INDUSTRY:
			if industry_building_tool: industry_building_tool.active = enabled
			if zoning_overlay: zoning_overlay.set_tool_active(enabled)
			if enabled:
				_sync_industry_deposits_overlay()
			else:
				_restore_industry_deposits_overlay()
				_selected_industry_resource_id = ""
		Tool.SELECT:
			if select_tool: select_tool.active = enabled
		Tool.BULLDOZE:
			if bulldoze_tool: bulldoze_tool.active = enabled

# =========================================================================
# TOGGLE ZONING OVERLAY
# =========================================================================
func _toggle_zoning_overlay():
	_toggle_tool(Tool.ZONING)

# =========================================================================
# HANDLE UNDO
# =========================================================================
func _handle_undo():
	# Zoning tool maintains its own undo stack for zone paint operations.
	if current_tool == Tool.ZONING and zoning_tool:
		zoning_tool.undo()
		return
	if simulation_node.undo_action():
		print("Undo Queued Globally")

# =========================================================================
# HANDLE MONEY AND DEMAND CHEAT
# =========================================================================
func _handle_money_and_demand_cheat() -> void:
	if not simulation_node or not simulation_node.has_method("apply_money_and_max_demand_cheat"):
		return
	var balance: float = simulation_node.apply_money_and_max_demand_cheat()
	print("Cheat applied: +1000000 money, R/C/I demand locked at 100%. Treasury: ", balance)
	get_viewport().set_input_as_handled()

# =========================================================================
# DEFAULT SAVE NAME
# =========================================================================
func _default_save_name() -> String:
	if not _current_save_path.is_empty():
		return _current_save_path.get_file()
	return "savegame.sqlite"

# =========================================================================
# ENSURE SAVES DIR
# =========================================================================
func _ensure_saves_dir() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(SAVES_DIR))

# =========================================================================
# REFRESH AFTER WORLD LOAD
# =========================================================================
func _refresh_after_world_load():
	var engine_tick := get_node_or_null("/root/EngineTick")  # [C]
	if engine_tick != null and engine_tick.has_method("reapply_ground"):  # [C]
		engine_tick.reapply_ground()  # [C] a loaded world's untouched cells re-derive
	if road_tool and road_tool.current_state != 0:
		road_tool.cancel_road()
	if move_tool and move_tool.current_state != 0:
		move_tool.cancel_move()
	_cancel_active_tool()
	# Never leave the previous world's road chunks visible while the new terrain is rebuilt.
	if road_tool:
		road_tool.reset_main_mesh_chunks()
	if terrain_node:
		terrain_node.rebuild_from_simulation_state()
	if water_node:
		water_node.rebuild_from_simulation_state()
	if buildings_node:
		buildings_node.reload_asset_packs()
		buildings_node.update_all_buildings()
	if zoning_overlay: zoning_overlay.full_refresh()
	if agents_node:
		agents_node.update_swarm()

# =========================================================================
# HANDLE SAVE GAME
# =========================================================================
func _handle_save_game():
	_ensure_saves_dir()
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_SAVE_FILE
	dialog.exclusive = true
	dialog.filters = PackedStringArray(["*.sqlite ; Save Files"])
	dialog.current_dir = ProjectSettings.globalize_path(SAVES_DIR)
	dialog.current_file = _default_save_name()
	dialog.file_selected.connect(func(path: String): _on_save_game_selected(path, dialog))
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(880, 620))

# =========================================================================
# HANDLE LOAD GAME
# =========================================================================
func _handle_load_game():
	_ensure_saves_dir()
	var dialog := FileDialog.new()
	dialog.access = FileDialog.ACCESS_FILESYSTEM
	dialog.file_mode = FileDialog.FILE_MODE_OPEN_FILE
	dialog.exclusive = true
	dialog.filters = PackedStringArray(["*.sqlite ; Save Files"])
	dialog.current_dir = ProjectSettings.globalize_path(SAVES_DIR)
	dialog.file_selected.connect(func(path: String): _on_load_game_selected(path, dialog))
	dialog.canceled.connect(dialog.queue_free)
	add_child(dialog)
	dialog.popup_centered(Vector2i(880, 620))

# =========================================================================
# ON SAVE GAME SELECTED
# =========================================================================
func _on_save_game_selected(path: String, dialog: FileDialog) -> void:
	dialog.hide()
	dialog.call_deferred("queue_free")
	call_deferred("_finish_save_game_selection", path)

# =========================================================================
# ON LOAD GAME SELECTED
# =========================================================================
func _on_load_game_selected(path: String, dialog: FileDialog) -> void:
	dialog.hide()
	dialog.call_deferred("queue_free")
	call_deferred("_finish_load_game_selection", path)

# =========================================================================
# FINISH SAVE GAME SELECTION
# =========================================================================
func _finish_save_game_selection(path: String) -> void:
	if simulation_node.save_game(path):
		_current_save_path = path
		print("Saved game to: ", path)
	else:
		push_error("Save failed: " + path)

# =========================================================================
# FINISH LOAD GAME SELECTION
# =========================================================================
func _finish_load_game_selection(path: String) -> void:
	menu_load_game_from_path(path)

# =========================================================================
# TOGGLE PAUSE
# =========================================================================
func _toggle_pause():
	var speed: float = 0.0 if _simulation_speed > 0.0 else 1.0
	set_simulation_speed(speed)

# =========================================================================
# SET SIMULATION SPEED
# =========================================================================
func set_simulation_speed(speed: float):
	var clamped_speed: float = maxf(speed, 0.0)
	_simulation_speed = clamped_speed
	simulation_node.set_simulation_speed(clamped_speed)
	if main_ui and main_ui.has_method("set_sim_speed_display"):
		main_ui.set_sim_speed_display(clamped_speed)
	print("Sim speed set to: ", clamped_speed)

# =========================================================================
# STEP SIMULATION SPEED
# =========================================================================
func step_simulation_speed(direction: int):
	var current_speed: float = _simulation_speed
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

# =========================================================================
# HANDLE OVERLAY MODE
# =========================================================================
func _handle_overlay_mode(keycode):
	var mode = 0
	match keycode:
		KEY_7: mode = 0
		KEY_8: mode = 1
		KEY_9: mode = 2
		KEY_0: mode = 3
		KEY_MINUS: mode = DEPOSITS_OVERLAY_MODE
	_set_overlay_mode(mode)

# =========================================================================
# SET OVERLAY MODE
# =========================================================================
func _set_overlay_mode(mode: int) -> void:
	var clamped_mode := clampi(mode, 0, DEPOSITS_OVERLAY_MODE)
	if _industry_deposits_overlay_forced and current_tool == Tool.INDUSTRY:
		if clamped_mode != DEPOSITS_OVERLAY_MODE:
			_industry_previous_overlay_mode = clamped_mode
		if terrain_node:
			terrain_node.overlay_mode = DEPOSITS_OVERLAY_MODE
		return
	if terrain_node:
		terrain_node.overlay_mode = clamped_mode

# =========================================================================
# SYNC INDUSTRY DEPOSITS OVERLAY
# =========================================================================
func _sync_industry_deposits_overlay() -> void:
	if current_tool != Tool.INDUSTRY or _selected_industry_resource_id.is_empty():
		_restore_industry_deposits_overlay()
		return
	if _industry_deposits_overlay_forced:
		if terrain_node:
			terrain_node.overlay_mode = DEPOSITS_OVERLAY_MODE
		return
	_industry_previous_overlay_mode = int(terrain_node.overlay_mode) if terrain_node else 0
	_industry_deposits_overlay_forced = true
	if terrain_node:
		terrain_node.overlay_mode = DEPOSITS_OVERLAY_MODE

# =========================================================================
# RESTORE INDUSTRY DEPOSITS OVERLAY
# =========================================================================
func _restore_industry_deposits_overlay() -> void:
	if not _industry_deposits_overlay_forced:
		return
	_industry_deposits_overlay_forced = false
	if terrain_node:
		terrain_node.overlay_mode = clampi(_industry_previous_overlay_mode, 0, DEPOSITS_OVERLAY_MODE)

# =========================================================================
# HANDLE ZONING SELECTION
# =========================================================================
func _handle_zoning_selection(keycode):
	if current_tool != Tool.ZONING:
		_toggle_tool(Tool.ZONING)
	if not zoning_tool:
		return
	match keycode:
		KEY_1: zoning_tool.select_profile_by_zone_type("residential")
		KEY_2: zoning_tool.select_profile_by_zone_type("commercial")
		KEY_3: zoning_tool.select_profile_by_zone_type("industrial")

# =========================================================================
# SELECT ZONE PROFILE
# =========================================================================
func select_zone_profile(runtime_id: int) -> void:
	if current_tool != Tool.ZONING:
		_toggle_tool(Tool.ZONING)
	if zoning_tool:
		zoning_tool.select_profile(runtime_id)

# =========================================================================
# SELECT SERVICE ASSET
# =========================================================================
func select_service_asset(asset_id: String) -> void:
	if current_tool != Tool.SERVICES:
		_cancel_active_tool()
		current_tool = Tool.SERVICES
		_activate_tool_logic(current_tool, true)
	if service_building_tool:
		service_building_tool.select_asset(asset_id)

# =========================================================================
# SELECT INDUSTRY ASSET
# =========================================================================
func select_industry_asset(asset_id: String, resource_id: String = "") -> void:
	if current_tool != Tool.INDUSTRY:
		_cancel_active_tool()
		current_tool = Tool.INDUSTRY
		_activate_tool_logic(current_tool, true)
	if industry_building_tool:
		industry_building_tool.select_asset(asset_id)
	_selected_industry_resource_id = resource_id.strip_edges()
	_sync_industry_deposits_overlay()

# =========================================================================
# SET ZONING PARCEL OPTIONS
# =========================================================================
func set_zoning_parcel_options(width_cells: int, depth_cells: int, gap_m: float) -> void:
	if current_tool != Tool.ZONING:
		_toggle_tool(Tool.ZONING)
	if zoning_tool:
		zoning_tool.set_parcel_options(width_cells, depth_cells, gap_m)

# =========================================================================
# HANDLE MOUSE
# =========================================================================
func _handle_mouse(event):
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
		if current_tool == Tool.NONE and building_inspector:
			_handle_inspect_click(event.position)
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
		_handle_right_click()

# =========================================================================
# HANDLE INSPECT CLICK
# =========================================================================
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
	building_inspector.try_inspect(pos, mouse_pos)

# =========================================================================
# HANDLE RIGHT CLICK
# =========================================================================
func _handle_right_click():
	if (current_tool == Tool.ROAD or current_tool == Tool.WALKWAY) and road_tool.current_state != 0:
		road_tool.cancel_road()
	elif current_tool == Tool.MOVE and move_tool.current_state != 0:
		move_tool.cancel_move()
	elif current_tool == Tool.SCULPT:
		pass # Right click is used for lowering ground during sculpting

	else:
		_cancel_active_tool()

# =========================================================================
# HANDLE ALTITUDE ADJUST
# =========================================================================
func _handle_altitude_adjust(delta):
	if (current_tool == Tool.ROAD or current_tool == Tool.WALKWAY) and road_tool:
		road_tool.adjust_altitude(delta)

# =========================================================================
# MENU SAVE GAME
# =========================================================================
func menu_save_game() -> void:
	_handle_save_game()

# =========================================================================
# MENU LOAD GAME
# =========================================================================
func menu_load_game() -> void:
	_handle_load_game()

# =========================================================================
# MENU LOAD GAME FROM PATH
# =========================================================================
func menu_load_game_from_path(path: String) -> bool:
	if path.is_empty():
		return false
	if simulation_node.load_game(path):
		_current_save_path = path
		_refresh_after_world_load()
		print("Loaded game from: ", path)
		return true
	push_error("Load failed: " + path)
	return false

# =========================================================================
# MENU LOAD WORLD DEFINITION
# =========================================================================
func menu_load_world_definition(path: String) -> bool:
	if path.is_empty():
		return false
	if simulation_node.load_world_definition(path):
		_refresh_after_world_load()
		set_simulation_speed(0.0)
		print("Loaded world definition: ", path)
		return true
	push_error("Load world definition failed: " + path)
	return false

# =========================================================================
# MENU SET OVERLAY MODE
# =========================================================================
func menu_set_overlay_mode(mode: int) -> void:
	_set_overlay_mode(mode)

# =========================================================================
# MENU TOGGLE ZONING OVERLAY
# =========================================================================
func menu_toggle_zoning_overlay() -> void:
	_toggle_zoning_overlay()
