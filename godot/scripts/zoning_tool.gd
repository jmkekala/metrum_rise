extends Node3D

@onready var simulation_node = $"../SimulationNode"
var current_zone_type: int = 1 # Residential by default
var brush_radius: float = 10.0

func _process(delta):
	if Input.is_key_pressed(KEY_1): 
		if current_zone_type != 1: print("Zoning Mode: Residential (Green)")
		current_zone_type = 1
	if Input.is_key_pressed(KEY_2): 
		if current_zone_type != 2: print("Zoning Mode: Commercial (Blue)")
		current_zone_type = 2
	if Input.is_key_pressed(KEY_3): 
		if current_zone_type != 3: print("Zoning Mode: Industrial (Yellow)")
		current_zone_type = 3
	if Input.is_key_pressed(KEY_4): 
		if current_zone_type != 4: print("Zoning Mode: Mixed (Purple)")
		current_zone_type = 4
	if Input.is_key_pressed(KEY_0): 
		if current_zone_type != 0: print("Zoning Mode: Erase")
		current_zone_type = 0
	
	# Hold Z to paint zones
	if Input.is_key_pressed(KEY_Z):
		if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
			print("Painting Zone ", current_zone_type, " at radius ", brush_radius)
			paint_zone(current_zone_type)
		elif Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT):
			print("Erasing Zone at radius ", brush_radius)
			paint_zone(0)

func paint_zone(zone_type: int):
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	
	if intersection != null:
		# Convert world pos to heightmap local/array pos (0 to 256)
		# The Rust array centers 0,0 at index 128,128
		var size = simulation_node.get_heightmap_size()
		var local_pos = Vector2(
			intersection.x + (size.x - 1.0) * 0.5,
			intersection.z + (size.y - 1.0) * 0.5
		)
		
		# Also, send delta to Rust so that when painting with a radius it maps perfectly
		simulation_node.paint_zone(local_pos, brush_radius, zone_type)
