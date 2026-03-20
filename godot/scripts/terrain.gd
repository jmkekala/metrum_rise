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

var cached_polygon_data_size: int = -1
var cached_overlay_state: bool = false
var cached_overlay_mode: int = -1
var polygon_meshes: Array[MeshInstance3D] = []

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
	update_terrain_visuals()
	
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

	# Dynamic Vector Polygon Extrusion System!
	var poly_data = simulation_node.get_zoning_polygons_data()
	
	if poly_data.size() != cached_polygon_data_size or show_global_zoning != cached_overlay_state or overlay_mode != cached_overlay_mode:
		cached_polygon_data_size = poly_data.size()
		cached_overlay_state = show_global_zoning
		cached_overlay_mode = overlay_mode
		
		# Clear existing geometry
		for m in polygon_meshes:
			m.queue_free()
		polygon_meshes.clear()
		
		if show_global_zoning and overlay_mode == 0:
			var i = 0
			while i < poly_data.size():
				var num_verts = int(poly_data[i])
				var zone_type = int(poly_data[i+1])
				i += 2
				
				var verts: Array[Vector2] = []
				for v_idx in range(num_verts):
					verts.append(Vector2(poly_data[i], poly_data[i+1]))
					i += 2
					
				if verts.size() >= 3:
					var st = SurfaceTool.new()
					st.begin(Mesh.PRIMITIVE_TRIANGLES)
					
					var indices = Geometry2D.triangulate_polygon(verts)
					if indices.size() > 0:
						for idx in indices:
							var v = verts[idx]
							# Floating absolutely identically just slightly over the grass mapping natively!
							var y = simulation_node.get_height_at(v) + 0.17 
							st.add_vertex(Vector3(v.x, y, v.y))
							
						st.generate_normals()
						var mesh_inst = MeshInstance3D.new()
						mesh_inst.mesh = st.commit()
						
						var color = Color(0,0,0)
						if zone_type == 1: color = Color(0.13, 0.77, 0.36, 0.4) # Green
						elif zone_type == 2: color = Color(0.23, 0.51, 0.96, 0.4) # Blue
						elif zone_type == 3: color = Color(0.9, 0.7, 0.03, 0.4) # Yellow
						elif zone_type == 4: color = Color(0.65, 0.33, 0.96, 0.4) # Purple
						
						var mat = StandardMaterial3D.new()
						mat.albedo_color = color
						mat.transparency = BaseMaterial3D.TRANSPARENCY_ALPHA
						mat.cull_mode = BaseMaterial3D.CULL_DISABLED
						mesh_inst.material_override = mat
						
						add_child(mesh_inst)
						polygon_meshes.append(mesh_inst)

func sculpt_at_mouse(delta):
	if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
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
			if Input.is_key_pressed(KEY_CTRL):
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
