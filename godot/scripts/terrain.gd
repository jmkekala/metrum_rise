extends MeshInstance3D

@onready var simulation_node = $"../SimulationNode"
var texture: ImageTexture
var height_image: Image

var zoning_texture: ImageTexture
var zoning_image: Image

var parcel_texture: ImageTexture
var parcel_image: Image

var overlay_mode: int = 0 # 0=Zoning, 1=Pollution, 2=Noise, 3=Desirability
var show_global_zoning: bool = false
var sim_speed: float = 0.0

var cached_overlay_state: bool = false
var cached_overlay_mode: int = -1

func _ready():
	var size = simulation_node.get_heightmap_size()
	var w = int(size.x)
	var h = int(size.y)
	
	# Setup mesh (Perfect 1.0m spacing)
	var plane_mesh = PlaneMesh.new()
	plane_mesh.size = size - Vector2(1,1) # 255x255m for 256 vertices
	plane_mesh.subdivide_depth = w - 2 # 254 divisions = 255 segments
	plane_mesh.subdivide_width = h - 2
	self.mesh = plane_mesh
	
	# Setup texture
	height_image = Image.create(w, h, false, Image.FORMAT_RF)
	texture = ImageTexture.create_from_image(height_image)
	
	# Zoning texture (RGBA8 for colored zones)
	zoning_image = Image.create(w, h, false, Image.FORMAT_RGBA8)
	zoning_texture = ImageTexture.create_from_image(zoning_image)
	
	parcel_image = Image.create(w, h, false, Image.FORMAT_RGBAF)
	parcel_texture = ImageTexture.create_from_image(parcel_image)
	
	var material = ShaderMaterial.new()
	material.shader = load("res://assets/materials/terrain.gdshader")
	material.set_shader_parameter("heightmap", texture)
	material.set_shader_parameter("zoning_texture", zoning_texture)
	material.set_shader_parameter("parcel_texture", parcel_texture)
	material.set_shader_parameter("height_scale", 20.0)
	material.set_shader_parameter("mesh_size", size)
	self.material_override = material

func _process(delta):
	if simulation_node.is_terrain_dirty():
		update_terrain_visuals()
		simulation_node.clear_terrain_dirty()
	
	var input_manager = get_node("../InputManager")
	if input_manager and input_manager.current_tool == input_manager.Tool.SCULPT:
		if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) or Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT):
			sculpt_at_mouse(delta)
	
	var material = self.material_override as ShaderMaterial
	if material != null:
		material.set_shader_parameter("show_global_zoning", show_global_zoning)

func update_terrain_visuals():
	var data = simulation_node.get_heightmap_data()
	var size = simulation_node.get_heightmap_size()
	
	# Convert PackedFloat32Array to byte array for Image
	# RF format is 4 bytes per pixel (float32)
	var byte_data = data.to_byte_array()
	height_image.set_data(int(size.x), int(size.y), false, Image.FORMAT_RF, byte_data)
	texture.update(height_image)
	
	# Update Zoning / Overlay Mode
	var material = self.material_override as ShaderMaterial
	if material != null:
		material.set_shader_parameter("overlay_mode", overlay_mode)
		
	var zone_bytes: PackedByteArray
	if overlay_mode == 0:
		pass # Painted zones transitioned to purely native geometric objects
	elif overlay_mode == 1:
		zone_bytes = simulation_node.get_pollution_image_data()
	elif overlay_mode == 2:
		zone_bytes = simulation_node.get_noise_image_data()
	elif overlay_mode == 3:
		zone_bytes = simulation_node.get_desirability_image_data()
		
	if zone_bytes.size() > 0:
		zoning_image.set_data(int(size.x), int(size.y), false, Image.FORMAT_RGBA8, zone_bytes)
		zoning_texture.update(zoning_image)

	# Grid-based zoning is now managed by ZoningTool.gd and SimulationNode direct rendering.

func sculpt_at_mouse(delta):
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if intersection != null:
		var size = simulation_node.get_heightmap_size()
		var local_pos = Vector2(
			intersection.x + (size.x - 1.0) * 0.5,
			intersection.z + (size.y - 1.0) * 0.5
		)
		
		var strength = 2.0 * delta
		if Input.is_key_pressed(KEY_CTRL) or Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT):
			strength = -2.0 * delta
			
		simulation_node.sculpt_terrain(local_pos, 15.0, strength)
		
		var road_tool = get_node("../RoadTool")
		if road_tool:
			road_tool.update_main_mesh()

func export_heightmap(path: String):
	print("Exporting heightmap to: ", ProjectSettings.globalize_path(path))
	# height_image is FORMAT_RF (float32). We need normalized 8-bit or 16-bit for PNG.
	var export_img = Image.create(height_image.get_width(), height_image.get_height(), false, Image.FORMAT_L8)
	
	var data = simulation_node.get_heightmap_data()
	for i in range(data.size()):
		var val = clamp(data[i] / 5.0, 0.0, 1.0) # Normalize 0-5m to 0-1
		var y = i / height_image.get_width()
		var x = i % height_image.get_width()
		export_img.set_pixel(x, y, Color(val, val, val, 1.0))
	
	export_img.save_png(path)

func import_heightmap(path: String):
	if not FileAccess.file_exists(path):
		print("Export file not found!")
		return
		
	print("Importing heightmap from: ", ProjectSettings.globalize_path(path))
	var img = Image.load_from_file(path)
	img.convert(Image.FORMAT_L8)
	
	var w = img.get_width()
	var h = img.get_height()
	var size = simulation_node.get_heightmap_size()
	
	if w != int(size.x) or h != int(size.y):
		print("Import size mismatch!")
		return
		
	var data = PackedFloat32Array()
	data.resize(w * h)
	
	for y in range(h):
		for x in range(w):
			var pixel = img.get_pixel(x, y)
			data[y * w + x] = pixel.r * 5.0 # Denormalize
			
	simulation_node.load_heightmap_data(data)
	
	var road_tool = get_node("../RoadTool")
	if road_tool:
		road_tool.update_main_mesh()
