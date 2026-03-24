## Cul-de-sac placement tool — toggles a circular dead-end cap on a selected road node.
##
## Extends NetworkTool.
## Rust methods called: get_closest_node(), has_cul_de_sac(), set_node_cul_de_sac()
## Displays a cylinder preview mesh at the hovered node; click to commit or remove the cap.
extends "res://scripts/network_tool.gd"

var preview_mesh: MeshInstance3D

func _ready():
	super._ready()
	preview_mesh = MeshInstance3D.new()
	preview_mesh.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	var mat = StandardMaterial3D.new()
	mat.albedo_color = Color(0.1, 1.0, 0.2, 0.4)
	mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
	preview_mesh.material_override = mat
	add_child(preview_mesh)
	
	# Create a simple cylinder mesh for the cul-de-sac preview
	var cylinder = CylinderMesh.new()
	cylinder.top_radius = 8.0
	cylinder.bottom_radius = 8.0
	cylinder.height = 0.2
	preview_mesh.mesh = cylinder

func _unhandled_input(event):
	if active:
		if event is InputEventMouseMotion:
			_update_preview()
		
		if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT and event.pressed:
			_handle_click()

func _handle_click():
	var pos = get_world_mouse_pos()
	
	# Check if clicking an existing road endpoint
	var node_id = simulation_node.get_closest_node(pos, 5.0)
	if node_id != -1:
		var conn_count = simulation_node.get_node_connection_count(node_id)
		# Only allow Cul-de-Sacs on dead ends (1 connection)
		if conn_count == 1:
			var enabled = !simulation_node.has_cul_de_sac(node_id)
			# Standard Cul-De-Sac radius is 8.0m unless specified
			simulation_node.set_node_cul_de_sac(node_id, enabled, 8.0)
			update_main_mesh()
		else:
			print("Cul-De-Sac can only be placed on a road end (1 connection).")

func _update_preview():
	var mouse_pos = get_world_mouse_pos()
	
	if not active:
		preview_mesh.visible = false
		return
		
	preview_mesh.visible = true
	preview_mesh.position = mouse_pos
	
	# Feedback for node clicking
	var node_id = simulation_node.get_closest_node(mouse_pos, 5.0)
	if node_id != -1 and simulation_node.get_node_connection_count(node_id) == 1:
		preview_mesh.position = simulation_node.get_node_pos(node_id)
		preview_mesh.material_override.albedo_color = Color(0.1, 1.0, 0.2, 0.5)
	else:
		preview_mesh.position = mouse_pos
		preview_mesh.material_override.albedo_color = Color(1.0, 0.1, 0.2, 0.3) # Red for invalid

func _process(delta):
	super._process(delta)
	if not active:
		if preview_mesh: preview_mesh.visible = false
		return
	
func cancel():
	preview_mesh.visible = false
