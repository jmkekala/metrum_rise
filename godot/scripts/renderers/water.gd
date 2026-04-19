## Water surface renderer — visualises visible water depth plus dynamic flow velocity.
##
## Rust methods called: get_heightmap_size(), get_terrain_world_size(), get_water_data(),
##   get_water_velocity_data(), add_water_source(), is_water_dirty(), clear_water_dirty()
## Water depth arrives as a flat PackedFloat32Array (same layout as the heightmap) representing
## visible total water depth above terrain after baseline-water plus dynamic-water composition.
## Velocity data is a parallel PackedFloat32Array for dynamic water only and drives foam/current
## shader effects.
extends MeshInstance3D

const SHORE_SOFTNESS_M := 0.5
const SHORE_FOAM_BAND_M := 0.5
const SHALLOW_WATER_COLOR := Color(0.20, 0.37, 0.40, 0.58)
const DEEP_WATER_COLOR := Color(0.05, 0.16, 0.29, 0.86)
const WATER_FRESNEL_STRENGTH := 0.24
const WATER_FRESNEL_POWER := 4.0
const WATER_WAVE_COLOR_STRENGTH := 0.025
const WATER_WAVE_ROUGHNESS_STRENGTH := 0.010
const WATER_DISPLAY_SURFACE_SMOOTHING := 0.94
const WATER_DISPLAY_SURFACE_BLEND_RADIUS_TEXELS := 1.0

@onready var simulation_node = $"../SimulationNode"
var texture: ImageTexture
var water_image: Image
var velocity_texture: ImageTexture
var velocity_image: Image
var height_texture: ImageTexture

func _ready():
	rebuild_from_simulation_state()

func rebuild_from_simulation_state():
	var dims = simulation_node.get_heightmap_size()
	var world_size = simulation_node.get_terrain_world_size()
	var w = int(dims.x)
	var h = int(dims.y)
	
	# Setup mesh (same as terrain but as a separate layer)
	var plane_mesh = PlaneMesh.new()
	plane_mesh.size = world_size
	plane_mesh.subdivide_width = max(0, w - 2)
	plane_mesh.subdivide_depth = max(0, h - 2)
	self.mesh = plane_mesh
	
	# Setup texture for water depth
	water_image = Image.create(w, h, false, Image.FORMAT_RF)
	texture = ImageTexture.create_from_image(water_image)
	
	# Setup texture for water velocity
	velocity_image = Image.create(w, h, false, Image.FORMAT_RF)
	velocity_texture = ImageTexture.create_from_image(velocity_image)
	
	# We also need the heightmap texture for alignment
	var terrain_node = $"../Terrain"
	height_texture = terrain_node.texture
	
	var material = ShaderMaterial.new()
	material.shader = load("res://assets/materials/water.gdshader")
	material.set_shader_parameter("heightmap", height_texture)
	material.set_shader_parameter("watermap", texture)
	material.set_shader_parameter("velocity_map", velocity_texture)
	material.set_shader_parameter("height_scale", 20.0)
	material.set_shader_parameter("shore_softness_m", SHORE_SOFTNESS_M)
	material.set_shader_parameter("shore_foam_band_m", SHORE_FOAM_BAND_M)
	material.set_shader_parameter("shallow_water_color", SHALLOW_WATER_COLOR)
	material.set_shader_parameter("deep_water_color", DEEP_WATER_COLOR)
	material.set_shader_parameter("water_fresnel_strength", WATER_FRESNEL_STRENGTH)
	material.set_shader_parameter("water_fresnel_power", WATER_FRESNEL_POWER)
	material.set_shader_parameter("water_wave_color_strength", WATER_WAVE_COLOR_STRENGTH)
	material.set_shader_parameter("water_wave_roughness_strength", WATER_WAVE_ROUGHNESS_STRENGTH)
	material.set_shader_parameter("water_surface_smoothing", WATER_DISPLAY_SURFACE_SMOOTHING)
	material.set_shader_parameter("water_surface_blend_radius_texels", WATER_DISPLAY_SURFACE_BLEND_RADIUS_TEXELS)
	material.set_shader_parameter("watermap_texture_size", Vector2(float(w), float(h)))
	self.material_override = material
	update_water_visuals()

func _process(delta):
	if simulation_node.is_water_dirty():
		update_water_visuals()
		simulation_node.clear_water_dirty()
	handle_water_input(delta)

func update_water_visuals():
	var dims = simulation_node.get_heightmap_size()
	
	# Update Depth
	var depth_data = simulation_node.get_water_data()
	water_image.set_data(int(dims.x), int(dims.y), false, Image.FORMAT_RF, depth_data.to_byte_array())
	texture.update(water_image)
	
	# Update Velocity
	var velocity_data = simulation_node.get_water_velocity_data()
	velocity_image.set_data(int(dims.x), int(dims.y), false, Image.FORMAT_RF, velocity_data.to_byte_array())
	velocity_texture.update(velocity_image)

func handle_water_input(delta):
	var input_manager = get_node_or_null("../InputManager")
	if input_manager and input_manager.current_tool == input_manager.Tool.WATER:
		if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT):
			var mouse_pos = get_viewport().get_mouse_position()
			var camera = get_viewport().get_camera_3d()
			
			var ray_origin = camera.project_ray_origin(mouse_pos)
			var ray_dir = camera.project_ray_normal(mouse_pos)
			
			var plane = Plane(Vector3.UP, 0)
			var intersection = plane.intersects_ray(ray_origin, ray_dir)
			
			if intersection != null:
				# Add a source with increasing strength
				simulation_node.add_water_source(Vector2(intersection.x, intersection.z), 0.5 * delta)
