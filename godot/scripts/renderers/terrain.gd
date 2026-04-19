## Terrain mesh renderer — displays the heightmap and optionally handles gameplay sculpt input.
##
## Rust methods called: get_heightmap_size(), get_terrain_world_size(), get_heightmap_data(),
##   sculpt_terrain(), flatten_terrain_for_roads(), is_terrain_dirty(), clear_terrain_dirty(),
##   get_pollution_image_data(), get_noise_image_data(), get_desirability_image_data()
## The heightmap arrives as a flat PackedFloat32Array in row-major order (width × height f32 values).
## Overlay textures (pollution/noise/desirability) arrive as RGBA8 PackedByteArray and are
## uploaded to a shader texture each frame when the active overlay mode is non-zero.
## Terrain hillshade is generated procedurally in the shader from the same live heightmap.
extends MeshInstance3D

const HILLSHADE_AZIMUTH_DEG := 315.0
const HILLSHADE_ALTITUDE_DEG := 38.0
const HILLSHADE_STRENGTH := 0.58
const HILLSHADE_AMBIENT := 0.24
const HILLSHADE_CONTRAST := 1.35
const HILLSHADE_SHADOW_TINT := Color(0.62, 0.71, 0.77)
const HILLSHADE_LIGHT_TINT := Color(0.97, 0.99, 0.95)
const TERRAIN_MACRO_VARIATION_STRENGTH := 0.10
const TERRAIN_ROCK_SLOPE_START := 0.15
const TERRAIN_ROCK_SLOPE_END := 0.34
const TERRAIN_SHORE_BLEND_STRENGTH := 0.28
const TERRAIN_SHORE_LOOKUP_RADIUS_TEXELS := 1.0
const CLIFF_SLOPE_START := 0.26
const CLIFF_SLOPE_END := 0.44
const CLIFF_RELIEF_START_M := 4.0
const CLIFF_RELIEF_END_M := 14.0
const CLIFF_SAMPLE_RADIUS_TEXELS := 2.25
const CLIFF_LATERAL_SMOOTHING_TEXELS := 1.2
const CLIFF_FACE_STRENGTH := 0.28
const CLIFF_EDGE_STRENGTH := 0.46
const CLIFF_CONTOUR_FADE := 0.78
const CLIFF_FACE_COLOR := Color(0.35, 0.35, 0.32)
const CLIFF_TOP_EDGE_COLOR := Color(0.27, 0.28, 0.22)
const CLIFF_TOE_EDGE_COLOR := Color(0.19, 0.20, 0.18)
const CONTOUR_MINOR_INTERVAL_M := 5.0
const CONTOUR_MAJOR_INTERVAL_M := 25.0
const CONTOUR_MINOR_THICKNESS := 0.95
const CONTOUR_MAJOR_THICKNESS := 1.25
const CONTOUR_MINOR_STRENGTH := 0.14
const CONTOUR_MAJOR_STRENGTH := 0.34

@onready var simulation_node = $"../SimulationNode"
var texture: ImageTexture
var height_image: Image

var overlay_texture: ImageTexture
var overlay_image: Image

var parcel_texture: ImageTexture
var parcel_image: Image
var water_proxy_texture: ImageTexture
var shared_water_texture: Texture2D

var overlay_mode: int = 0 # 0=None, 1=Pollution, 2=Noise, 3=Desirability
var sim_speed: float = 0.0

var cached_overlay_state: bool = false
var cached_overlay_mode: int = -1

func _ready():
	# Large displaced terrain self-shadowing is unstable at close zoom on the coarse grid.
	# Keep terrain lit by procedural hillshade/cliff shading and let other scene objects cast.
	cast_shadow = GeometryInstance3D.SHADOW_CASTING_SETTING_OFF
	rebuild_from_simulation_state()

func rebuild_from_simulation_state():
	var dims = simulation_node.get_heightmap_size()
	var world_size = simulation_node.get_terrain_world_size()
	var w = int(dims.x)
	var h = int(dims.y)
	
	# Setup mesh from world extent while keeping one vertex per terrain sample.
	var plane_mesh = PlaneMesh.new()
	plane_mesh.size = world_size
	plane_mesh.subdivide_width = max(0, w - 2)
	plane_mesh.subdivide_depth = max(0, h - 2)
	self.mesh = plane_mesh
	
	# Setup texture
	height_image = Image.create(w, h, false, Image.FORMAT_RF)
	texture = ImageTexture.create_from_image(height_image)
	
	# Overlay Texture (RGBA8 for heatmaps)
	overlay_image = Image.create(w, h, false, Image.FORMAT_RGBA8)
	overlay_texture = ImageTexture.create_from_image(overlay_image)
	
	parcel_image = Image.create(w, h, false, Image.FORMAT_RGBAF)
	parcel_texture = ImageTexture.create_from_image(parcel_image)

	var water_proxy_image := Image.create(w, h, false, Image.FORMAT_RF)
	water_proxy_texture = ImageTexture.create_from_image(water_proxy_image)
	
	var material = ShaderMaterial.new()
	material.shader = load("res://assets/materials/terrain.gdshader")
	material.set_shader_parameter("heightmap", texture)
	material.set_shader_parameter("overlay_texture", overlay_texture)
	material.set_shader_parameter("parcel_texture", parcel_texture)
	material.set_shader_parameter("watermap", water_proxy_texture)
	material.set_shader_parameter("height_scale", 20.0)
	material.set_shader_parameter("mesh_size", world_size)
	material.set_shader_parameter("heightmap_texture_size", Vector2(float(w), float(h)))
	material.set_shader_parameter(
		"terrain_cell_m",
		world_size.x / float(max(1, w - 1))
	)
	material.set_shader_parameter("hillshade_azimuth_deg", HILLSHADE_AZIMUTH_DEG)
	material.set_shader_parameter("hillshade_altitude_deg", HILLSHADE_ALTITUDE_DEG)
	material.set_shader_parameter("hillshade_strength", HILLSHADE_STRENGTH)
	material.set_shader_parameter("hillshade_ambient", HILLSHADE_AMBIENT)
	material.set_shader_parameter("hillshade_contrast", HILLSHADE_CONTRAST)
	material.set_shader_parameter("hillshade_shadow_tint", HILLSHADE_SHADOW_TINT)
	material.set_shader_parameter("hillshade_light_tint", HILLSHADE_LIGHT_TINT)
	material.set_shader_parameter("terrain_macro_variation_strength", TERRAIN_MACRO_VARIATION_STRENGTH)
	material.set_shader_parameter("terrain_rock_slope_start", TERRAIN_ROCK_SLOPE_START)
	material.set_shader_parameter("terrain_rock_slope_end", TERRAIN_ROCK_SLOPE_END)
	material.set_shader_parameter("terrain_shore_blend_strength", TERRAIN_SHORE_BLEND_STRENGTH)
	material.set_shader_parameter("terrain_shore_lookup_radius_texels", TERRAIN_SHORE_LOOKUP_RADIUS_TEXELS)
	material.set_shader_parameter("cliff_slope_start", CLIFF_SLOPE_START)
	material.set_shader_parameter("cliff_slope_end", CLIFF_SLOPE_END)
	material.set_shader_parameter("cliff_relief_start_m", CLIFF_RELIEF_START_M)
	material.set_shader_parameter("cliff_relief_end_m", CLIFF_RELIEF_END_M)
	material.set_shader_parameter("cliff_sample_radius_texels", CLIFF_SAMPLE_RADIUS_TEXELS)
	material.set_shader_parameter("cliff_lateral_smoothing_texels", CLIFF_LATERAL_SMOOTHING_TEXELS)
	material.set_shader_parameter("cliff_face_strength", CLIFF_FACE_STRENGTH)
	material.set_shader_parameter("cliff_edge_strength", CLIFF_EDGE_STRENGTH)
	material.set_shader_parameter("cliff_contour_fade", CLIFF_CONTOUR_FADE)
	material.set_shader_parameter("cliff_face_color", CLIFF_FACE_COLOR)
	material.set_shader_parameter("cliff_top_edge_color", CLIFF_TOP_EDGE_COLOR)
	material.set_shader_parameter("cliff_toe_edge_color", CLIFF_TOE_EDGE_COLOR)
	material.set_shader_parameter("contour_minor_interval_m", CONTOUR_MINOR_INTERVAL_M)
	material.set_shader_parameter("contour_major_interval_m", CONTOUR_MAJOR_INTERVAL_M)
	material.set_shader_parameter("contour_minor_thickness", CONTOUR_MINOR_THICKNESS)
	material.set_shader_parameter("contour_major_thickness", CONTOUR_MAJOR_THICKNESS)
	material.set_shader_parameter("contour_minor_strength", CONTOUR_MINOR_STRENGTH)
	material.set_shader_parameter("contour_major_strength", CONTOUR_MAJOR_STRENGTH)
	self.material_override = material
	_sync_water_texture()
	update_terrain_visuals()

func _process(delta):
	_sync_water_texture()
	if simulation_node.is_terrain_dirty():
		update_terrain_visuals()
		simulation_node.clear_terrain_dirty()
	
	# Bug Fix B2: Update overlay even if terrain is not dirty
	if overlay_mode != cached_overlay_mode:
		update_terrain_visuals()
		cached_overlay_mode = overlay_mode
	
	var input_manager = get_node_or_null("../InputManager")
	if input_manager and input_manager.current_tool == input_manager.Tool.SCULPT:
		if Input.is_mouse_button_pressed(MOUSE_BUTTON_LEFT) or Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT):
			sculpt_at_mouse(delta)
	
	# Overlay logic handled in update_terrain_visuals
	pass

func update_terrain_visuals():
	var data = simulation_node.get_heightmap_data()
	var dims = simulation_node.get_heightmap_size()
	
	# Convert PackedFloat32Array to byte array for Image
	# RF format is 4 bytes per pixel (float32)
	var byte_data = data.to_byte_array()
	height_image.set_data(int(dims.x), int(dims.y), false, Image.FORMAT_RF, byte_data)
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
		overlay_image.set_data(int(dims.x), int(dims.y), false, Image.FORMAT_RGBA8, zone_bytes)
		overlay_texture.update(overlay_image)
	else:
		# Clear overlay if mode is 0 or no data
		overlay_image.fill(Color(0, 0, 0, 0))
		overlay_texture.update(overlay_image)

	# Grid-based zoning is now managed by ZoningTool.gd and SimulationNode direct rendering.

func _sync_water_texture() -> void:
	var material := self.material_override as ShaderMaterial
	if material == null:
		return
	var water_node = get_node_or_null("../Water")
	if water_node == null:
		return
	var next_texture: Texture2D = water_node.texture
	if next_texture == null or next_texture == shared_water_texture:
		return
	shared_water_texture = next_texture
	material.set_shader_parameter("watermap", shared_water_texture)

func sculpt_at_mouse(delta):
	var mouse_pos = get_viewport().get_mouse_position()
	var camera = get_viewport().get_camera_3d()
	
	var ray_origin = camera.project_ray_origin(mouse_pos)
	var ray_dir = camera.project_ray_normal(mouse_pos)
	
	var intersection = simulation_node.intersect_terrain(ray_origin, ray_dir)
	if intersection != null:
		var strength = 2.0 * delta
		if Input.is_key_pressed(KEY_CTRL) or Input.is_mouse_button_pressed(MOUSE_BUTTON_RIGHT):
			strength = -2.0 * delta
			
		simulation_node.sculpt_terrain(Vector2(intersection.x, intersection.z), 15.0, strength)
		
		var road_tool = get_node_or_null("../RoadTool")
		if road_tool:
			road_tool.update_main_mesh()
