## Selection tool — allows clicking roads to view and edit properties (Item 27).
##
## Rust methods called: get_hovered_edge(), set_edge_class(), recalculate_zoning_local()
extends Node3D

@onready var simulation_node = $"../SimulationNode"
@onready var main_ui = $"../MainUI"

var active: bool = false:
	set(value):
		active = value
		if not active:
			_hide_properties_panel()
var selected_edge: int = -1

func _process(_delta):
	if not active: return
	
	if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
		_handle_click()

func _handle_click():
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	if not camera: return
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	var pos = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if pos == null: return
	
	var edge_idx = simulation_node.get_hovered_edge(pos.x, pos.z)
	if edge_idx != -1:
		selected_edge = edge_idx
		_show_properties_panel()
	else:
		selected_edge = -1
		_hide_properties_panel()

func _show_properties_panel():
	if main_ui:
		main_ui.show_road_properties(selected_edge)

func _hide_properties_panel():
	if main_ui:
		main_ui.hide_road_properties()

func set_selected_edge_class(class_int: int):
	if selected_edge != -1:
		simulation_node.set_edge_class(selected_edge, class_int)
		# Refresh visuals
		if get_node_or_null("../RoadTool"):
			get_node("../RoadTool").update_main_mesh()
