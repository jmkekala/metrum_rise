# SPDX-License-Identifier: GPL-2.0-only

## Agent inspector (`DEBUG-05`): click an agent to see its trip and follow it with the camera.
##
## With no tool active, `input_manager` offers world clicks here before the building inspector.
## A hit makes Rust select the agent (`pick_debug_agent`). While one is inspected, each frame
## reads `get_agent_inspection` (O(1)) and puts the camera pivot on the agent's drawn position;
## zoom and orbit still work. The route mesh (`get_agent_route_lines`) is rebuilt ten times a
## second, and at once when the agent changes lane or path. Panning the camera (WASD) stops the
## follow, and a home, work or destination link stops it, frames that building and asks for its
## Building Inspector (`building_requested`); the inspection stays, and Follow resumes. Esc, the window's close button, a
## click that hits no agent, a world load or the agent's removal end the inspection, and the
## camera stays where it is. With nothing inspected it does no work.
extends Node3D

## A building link was pressed: open the Building Inspector for the building at `world_pos`,
## its window placed beside `screen_pos`.
signal building_requested(world_pos: Vector3, screen_pos: Vector2)

const AgentInspectorPanel := preload("res://scripts/ui/agent_inspector_panel.gd")
const RouteRibbonShader := preload("res://scripts/shaders/route_ribbon.gdshader")

const ROUTE_REFRESH_S := 0.1
const PANEL_REFRESH_S := 0.25
# Pick radius per metre of camera distance (about 12 px at 1080p), clamped.
const PICK_RADIUS_PER_M := 0.015
const PICK_RADIUS_MIN_M := 3.0
const PICK_RADIUS_MAX_M := 15.0
# The route keeps these widths at any zoom: a dark outline under the colored line, so it reads
# from far away and over bright ground.
const ROUTE_WIDTH_PX := 6.0
const ROUTE_OUTLINE_WIDTH_PX := 10.0
# Framing radius when a link takes the camera to a building.
const BUILDING_FOCUS_RADIUS_M := 40.0

var simulation_node: Node
## Render ID of the inspected agent, or `-1`.
var render_id := -1
## From the last route build: `lanes`, `remaining_m` and `end`.
var last_route := {}
## Vertices in the drawn route ribbon: six per line segment.
var vertex_count := 0

var _panel: AgentInspectorPanel
var _mesh := ArrayMesh.new()
var _route_key := ""
var _route_elapsed_s := 0.0
var _panel_elapsed_s := 0.0
# False after the player panned or a building link moved the camera away, until Follow.
var _following := true
# True once the camera follows this inspection, so a camera that stopped was panned away.
var _camera_engaged := false

func _ready() -> void:
	simulation_node = get_node_or_null("../SimulationNode")
	# The shader ignores depth, so buildings and terrain never hide the route.
	_add_route_pass("RouteOutline", ROUTE_OUTLINE_WIDTH_PX, true, 1)
	_add_route_pass("Route", ROUTE_WIDTH_PX, false, 2)
	_panel = AgentInspectorPanel.new()
	_panel.name = "AgentInspectorPanel"
	_panel.close_pressed.connect(func() -> void: end_inspection())
	_panel.follow_pressed.connect(_set_following.bind(true))
	_panel.building_pressed.connect(_show_building)
	add_child(_panel)
	set_process(false)

## Inspects the agent nearest `world_pos`; a newly opened window goes beside `screen_pos`.
## Returns `true` on a hit, so the click is consumed; a miss ends any inspection and leaves
## the click to the caller.
func try_inspect_at(world_pos: Vector3, screen_pos := Vector2.ZERO) -> bool:
	if simulation_node == null:
		return false
	var picked: int = simulation_node.pick_debug_agent(world_pos, _pick_radius())
	if picked < 0:
		end_inspection()
		return false
	render_id = picked
	_route_key = ""
	_route_elapsed_s = ROUTE_REFRESH_S
	_panel_elapsed_s = PANEL_REFRESH_S
	set_process(true)
	_set_following(true)
	# Switching agents leaves the window where the player put it.
	if not _panel.visible:
		_panel.open_near(screen_pos)
	_refresh(0.0)
	var details: Dictionary = simulation_node.get_agent_inspection(render_id)
	if details.get("active", false):
		MetrumDebug.log("events", "agent inspection: agent %d, %s, %s, home %s, work %s, destination %s" % [
			render_id,
			details["mode"],
			details["state"],
			AgentInspectorPanel.building_text(details["home"]),
			AgentInspectorPanel.building_text(details["work"]),
			AgentInspectorPanel.building_text(details["destination"]),
		])
	return true

## Ends the inspection; `reason` is logged on `events` when given. Returns `true` if an agent
## was inspected. The camera stays where it is.
func end_inspection(reason := "") -> bool:
	if render_id < 0:
		return false
	if not reason.is_empty():
		MetrumDebug.log("events", "agent inspection ended: agent %d %s" % [render_id, reason])
	if simulation_node != null:
		simulation_node.clear_agent_inspection()
	render_id = -1
	last_route = {}
	vertex_count = 0
	_mesh.clear_surfaces()
	_panel.close()
	set_process(false)
	var camera := _camera()
	if camera != null:
		camera.stop_follow()
	return true

## Returns `true` while an agent is inspected.
func is_active() -> bool:
	return render_id >= 0

## The panel, for tests.
func panel() -> AgentInspectorPanel:
	return _panel

func _process(delta: float) -> void:
	_refresh(delta)

func _refresh(delta: float) -> void:
	var details: Dictionary = simulation_node.get_agent_inspection(render_id)
	# While the simulation core is busy only a car's drawn position comes back: keep following
	# it, keep the rest of the view and try again next frame.
	if not details.has("active"):
		if details.has("drawn_position"):
			_follow(details["drawn_position"])
		return
	if not details["active"]:
		end_inspection(String(details.get("reason", "")))
		return
	_follow(details.get("drawn_position", details["position"]))
	_route_elapsed_s += delta
	_panel_elapsed_s += delta
	var key := "%d %d %d %d" % [details["lane"], details["path_index"], details["path_length"], details["destination"].get("id", -1)]
	if key != _route_key or _route_elapsed_s >= ROUTE_REFRESH_S:
		_rebuild_route(key)
	if _panel_elapsed_s >= PANEL_REFRESH_S:
		_panel_elapsed_s = 0.0
		_panel.show_agent(details)
		_panel.show_route(details, last_route)

func _rebuild_route(key: String) -> void:
	var data: Dictionary = simulation_node.get_agent_route_lines(render_id)
	# Empty while the core is busy: keep the last mesh and retry next frame.
	if data.is_empty():
		return
	_route_key = key
	_route_elapsed_s = 0.0
	var vertices: PackedVector3Array = data["ribbon_vertices"]
	_mesh.clear_surfaces()
	vertex_count = vertices.size()
	if vertex_count > 0:
		var arrays := []
		arrays.resize(Mesh.ARRAY_MAX)
		arrays[Mesh.ARRAY_VERTEX] = vertices
		arrays[Mesh.ARRAY_TEX_UV] = data["ribbon_uv"]
		arrays[Mesh.ARRAY_TEX_UV2] = data["ribbon_uv2"]
		arrays[Mesh.ARRAY_COLOR] = data["ribbon_colors"]
		_mesh.add_surface_from_arrays(Mesh.PRIMITIVE_TRIANGLES, arrays)
	for array in ["points", "colors", "ribbon_vertices", "ribbon_uv", "ribbon_uv2", "ribbon_colors"]:
		data.erase(array)
	last_route = data

# One draw of the route ribbon; `priority` orders the outline under the colored line.
func _add_route_pass(pass_name: String, width_px: float, outline: bool, priority: int) -> void:
	var material := ShaderMaterial.new()
	material.shader = RouteRibbonShader
	material.set_shader_parameter("width_px", width_px)
	material.set_shader_parameter("use_tint", outline)
	material.render_priority = priority
	var instance := MeshInstance3D.new()
	instance.name = pass_name
	instance.mesh = _mesh
	instance.material_override = material
	instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	add_child(instance)

# Puts the camera on `target` while following. A camera that stopped following since the last
# frame was panned by the player, which ends the follow but not the inspection.
func _follow(target: Vector3) -> void:
	var camera := _camera()
	if camera == null or not _following:
		return
	if _camera_engaged and not camera.is_following():
		_set_following(false)
		return
	camera.follow(target)
	_camera_engaged = true

func _set_following(following: bool) -> void:
	_following = following
	_camera_engaged = false
	_panel.set_following(following)

# A building link: stop following, frame the building and open its inspector beside the window.
func _show_building(building: Dictionary) -> void:
	if building.is_empty():
		return
	_set_following(false)
	var target: Vector3 = building["position"]
	var camera := _camera()
	if camera != null:
		camera.stop_follow()
		camera.focus_on(target, BUILDING_FOCUS_RADIUS_M)
	building_requested.emit(target, Vector2(_panel.position.x + _panel.size.x, _panel.position.y + 24.0))

func _pick_radius() -> float:
	var camera := get_viewport().get_camera_3d()
	if camera == null or not camera.has_method("get_focus_position"):
		return PICK_RADIUS_MIN_M
	var distance := camera.global_position.distance_to(camera.get_focus_position())
	return clampf(distance * PICK_RADIUS_PER_M, PICK_RADIUS_MIN_M, PICK_RADIUS_MAX_M)

func _camera() -> CameraNode:
	return get_viewport().get_camera_3d() as CameraNode
