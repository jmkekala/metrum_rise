## Road node move tool — click to select a node, drag to reposition it on the terrain.
##
## Extends NetworkTool.
## Rust methods called: get_closest_node(), get_node_pos(), move_network_node()
## PgUp/PgDown adjusts height; Esc or right-click cancels and reverts to last committed position.
extends "res://scripts/tools/network_tool.gd"

enum State { IDLE, SELECTED, DRAGGING }
var current_state = State.IDLE

var selected_node_id: int = -1
var selected_node_pos: Vector3

func _ready():
	super._ready()
	# Optional label for instructions
	var label = Label.new()
	label.position = Vector2(20, 160)
	label.add_theme_color_override("font_color", Color(0, 1, 0))
	label.text = "Move Tool Active\n[Click] Select Node\n[Drag] Move Planar\n[PgUp/PgDown] Adjust Height\n[Esc/RClick] Drop/Cancel"
	label.visible = false
	add_child(label)

func _process(delta):
	super._process(delta)
	
	if get_child_count() > 0 and get_child(get_child_count() - 1) is Label:
		get_child(get_child_count() - 1).visible = active
		
	if active and current_state == State.SELECTED:
		# Process keyboard height adjustments every frame for smooth nudging
		var y_shift = 0.0
		if Input.is_key_pressed(KEY_PAGEUP): y_shift += 2.0 * delta
		if Input.is_key_pressed(KEY_PAGEDOWN): y_shift -= 2.0 * delta
		
		if y_shift != 0.0:
			selected_node_pos.y += y_shift
			_apply_move(selected_node_pos)
			
	if active and current_state == State.DRAGGING:
		# Lock dragging to terrain X-Z strictly, preserving customized Y offset unless terrain drops drastically
		var ground_pos = get_terrain_interaction()
		if ground_pos != null:
			# Automatically float 0.001m above target terrain unless explicitly raised
			var terrain_h = simulation_node.get_world_surface_height(Vector2(ground_pos.x, ground_pos.z))
			# Keep the relative height difference, but drag planar!
			# Or simpler: Just set it exactly at terra level if dragging planarly!
			var target_pos = Vector3(ground_pos.x, terrain_h + 0.001, ground_pos.z)
			if (target_pos - selected_node_pos).length() > 0.1:
				selected_node_pos = target_pos
				_apply_move(selected_node_pos)

func _input(event):
	if not active: return

	if event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_LEFT:
			if event.pressed:
				_handle_left_click()
			else:
				if current_state == State.DRAGGING:
					current_state = State.SELECTED # Drop but keep selected

					
		if event.button_index == MOUSE_BUTTON_RIGHT and event.pressed:
			cancel_move()

func _handle_left_click():
	var pos_variant = get_terrain_interaction()
	if pos_variant == null: return
	var pos: Vector3 = pos_variant
	
	match current_state:
		State.IDLE:
			# Look for a node near the mouse to select
			var closest_pos = simulation_node.get_closest_network_point(pos, 5.0)
			if closest_pos != null:
				selected_node_id = simulation_node.get_closest_node(closest_pos, 5.0)
				if selected_node_id != -1:
					selected_node_pos = closest_pos
					current_state = State.DRAGGING
					
		State.SELECTED:
			# Re-click to drag
			var closest_pos = simulation_node.get_closest_network_point(pos, 5.0)
			if closest_pos != null:
				var clicked_id = simulation_node.get_closest_node(closest_pos, 5.0)
				if clicked_id == selected_node_id:
					current_state = State.DRAGGING
				else:
					# Clicked another node
					selected_node_id = clicked_id
					selected_node_pos = closest_pos
					current_state = State.DRAGGING

func _apply_move(pos: Vector3):
	if selected_node_id != -1:
		simulation_node.move_network_node(selected_node_id, pos)
		
		# Force mesh rebuilding globally
		var road_tool = get_node("../RoadTool")
		if road_tool:
			road_tool.update_main_mesh()
		terrain_node.update_terrain_visuals()

func cancel_move():
	active = false
	current_state = State.IDLE
	selected_node_id = -1

# Visual overrides
func _update_preview():
	pass

func _update_node_visuals():
	super._update_node_visuals()
	
	# Highlight the selected node
	if selected_node_id != -1 and current_state != State.IDLE:
		if cursor_mesh:
			cursor_mesh.global_position = selected_node_pos
			cursor_mesh.global_position.y += 0.8
			cursor_mesh.visible = true
