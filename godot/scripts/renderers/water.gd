## Water surface renderer — visualises visible water depth plus dynamic flow velocity.
##
## Rust methods called: get_heightmap_size(), get_terrain_world_size(), get_water_data(),
##   get_water_velocity_data(), add_water_source(), is_water_dirty(), clear_water_dirty()
## Water depth arrives as a flat PackedFloat32Array (same layout as the heightmap) representing
## visible total water depth above terrain after baseline-water plus dynamic-water composition.
## Velocity data is a parallel PackedFloat32Array for dynamic water only and drives foam/current
## shader effects.
extends MeshInstance3D

const HEIGHT_SCALE := 20.0
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
const WATER_BORDER_MIN_DEPTH_M := 0.02

@onready var simulation_node = $"../SimulationNode"
@onready var terrain_node = $"../Terrain"
var texture: ImageTexture
var water_image: Image
var velocity_texture: ImageTexture
var velocity_image: Image
var height_texture: ImageTexture
var water_border_instance: MeshInstance3D
var water_border_material: ShaderMaterial

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
	height_texture = terrain_node.texture
	_ensure_water_border_visual()
	
	var material = ShaderMaterial.new()
	material.shader = load("res://assets/materials/water.gdshader")
	material.set_shader_parameter("heightmap", height_texture)
	material.set_shader_parameter("watermap", texture)
	material.set_shader_parameter("velocity_map", velocity_texture)
	material.set_shader_parameter("height_scale", HEIGHT_SCALE)
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
	_rebuild_water_border(depth_data, int(dims.x), int(dims.y))
	
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

func _ensure_water_border_visual() -> void:
	if water_border_instance == null:
		water_border_instance = MeshInstance3D.new()
		water_border_instance.name = "WaterBorderCurtain"
		water_border_instance.cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
		add_child(water_border_instance)
	if water_border_material == null:
		water_border_material = ShaderMaterial.new()
		water_border_material.shader = load("res://scripts/renderers/water_border.gdshader")
		water_border_material.set_shader_parameter("shallow_water_color", SHALLOW_WATER_COLOR)
		water_border_material.set_shader_parameter("deep_water_color", DEEP_WATER_COLOR)
	water_border_instance.material_override = water_border_material

func _rebuild_water_border(depth_data: PackedFloat32Array, w: int, h: int) -> void:
	_ensure_water_border_visual()
	if terrain_node == null or terrain_node.height_image == null or depth_data.size() < w * h:
		water_border_instance.mesh = null
		return

	var world_size: Vector2 = simulation_node.get_terrain_world_size()
	if world_size == Vector2.ZERO or w < 2 or h < 2:
		water_border_instance.mesh = null
		return

	var st := SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)
	var segment_count := 0
	for x in range(w - 1):
		segment_count += _add_water_border_segment(st, x, 0, x + 1, 0, w, h, world_size, depth_data)
	for z in range(h - 1):
		segment_count += _add_water_border_segment(st, w - 1, z, w - 1, z + 1, w, h, world_size, depth_data)
	for x in range(w - 1, 0, -1):
		segment_count += _add_water_border_segment(st, x, h - 1, x - 1, h - 1, w, h, world_size, depth_data)
	for z in range(h - 1, 0, -1):
		segment_count += _add_water_border_segment(st, 0, z, 0, z - 1, w, h, world_size, depth_data)

	if segment_count == 0:
		water_border_instance.mesh = null
		return

	water_border_instance.mesh = st.commit()
	water_border_instance.material_override = water_border_material

func _add_water_border_segment(
	st: SurfaceTool,
	x0: int,
	z0: int,
	x1: int,
	z1: int,
	w: int,
	h: int,
	world_size: Vector2,
	depth_data: PackedFloat32Array
) -> int:
	var depth0: float = depth_data[z0 * w + x0]
	var depth1: float = depth_data[z1 * w + x1]
	if depth0 <= WATER_BORDER_MIN_DEPTH_M and depth1 <= WATER_BORDER_MIN_DEPTH_M:
		return 0

	var terrain0: float = terrain_node.height_image.get_pixel(x0, z0).r * HEIGHT_SCALE
	var terrain1: float = terrain_node.height_image.get_pixel(x1, z1).r * HEIGHT_SCALE
	var top0 := _water_border_position(x0, z0, world_size, w, h, terrain0 + depth0 + 0.02)
	var top1 := _water_border_position(x1, z1, world_size, w, h, terrain1 + depth1 + 0.02)
	var bottom0 := _water_border_position(x0, z0, world_size, w, h, terrain0)
	var bottom1 := _water_border_position(x1, z1, world_size, w, h, terrain1)
	var max_depth: float = max(depth0, depth1)
	_add_water_border_quad(st, top0, top1, bottom1, bottom0, depth0, depth1, max_depth)
	return 1

func _water_border_position(
	x_idx: int,
	z_idx: int,
	world_size: Vector2,
	w: int,
	h: int,
	y: float
) -> Vector3:
	var x_t := float(x_idx) / float(max(1, w - 1))
	var z_t := float(z_idx) / float(max(1, h - 1))
	var x: float = lerp(-world_size.x * 0.5, world_size.x * 0.5, x_t)
	var z: float = lerp(-world_size.y * 0.5, world_size.y * 0.5, z_t)
	return Vector3(x, y, z)

func _add_water_border_quad(
	st: SurfaceTool,
	top0: Vector3,
	top1: Vector3,
	bottom1: Vector3,
	bottom0: Vector3,
	depth0: float,
	depth1: float,
	max_depth: float
) -> void:
	var normal := (top1 - top0).cross(bottom0 - top0).normalized()
	_add_water_border_vertex(st, top0, normal, Vector2(0.0, 0.0), depth0, max_depth)
	_add_water_border_vertex(st, top1, normal, Vector2(1.0, 0.0), depth1, max_depth)
	_add_water_border_vertex(st, bottom1, normal, Vector2(1.0, 1.0), depth1, max_depth)
	_add_water_border_vertex(st, top0, normal, Vector2(0.0, 0.0), depth0, max_depth)
	_add_water_border_vertex(st, bottom1, normal, Vector2(1.0, 1.0), depth1, max_depth)
	_add_water_border_vertex(st, bottom0, normal, Vector2(0.0, 1.0), depth0, max_depth)

func _add_water_border_vertex(
	st: SurfaceTool,
	position: Vector3,
	normal: Vector3,
	uv: Vector2,
	local_depth_m: float,
	max_depth_m: float
) -> void:
	var encoded_depth := 0.0
	if max_depth_m > 0.001:
		encoded_depth = clamp(local_depth_m / max_depth_m, 0.0, 1.0)
	st.set_normal(normal)
	st.set_uv(uv)
	st.set_color(Color(encoded_depth, 0.0, 0.0, clamp(local_depth_m / 10.0, 0.0, 1.0)))
	st.add_vertex(position)
