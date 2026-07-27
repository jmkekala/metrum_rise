## Shared world/editor material factory and texture cache.
## Rendering tools should request common asphalt/concrete materials here instead
## of rebuilding shader materials or reloading texture sets locally.
## Site-surface materials are shared by the asset editor and live building-site clients.
extends RefCounted
class_name WorldMaterials

const SceneLightingConfig := preload("res://scripts/core/scene_lighting.gd")

const MATERIAL_ASPHALT := "asphalt"
const MATERIAL_CONCRETE := "concrete"

const GRASS_ALBEDO := "res://assets/textures/general/grass/Grass002_2K_Runtime/grass002_2k_albedo.jpg"
const GRASS_HEIGHT := "res://assets/textures/general/grass/Grass002_2K_Runtime/grass002_2k_height.jpg"

const ROAD_ASPHALT_DIFF := "res://assets/textures/road/clean_asphalt/clean_asphalt_diff_4k.jpg"
const ROAD_ASPHALT_NORMAL := "res://assets/textures/road/clean_asphalt/clean_asphalt_nor_gl_4k.png"
const ROAD_ASPHALT_ROUGH := "res://assets/textures/road/clean_asphalt/clean_asphalt_rough_4k.png"
const ROAD_ASPHALT_DISP := "res://assets/textures/road/clean_asphalt/clean_asphalt_disp_4k.png"

const SIDEWALK_ASPHALT_DIFF := "res://assets/textures/road/asphalt_04/asphalt_04_diff_2k.jpg"
const SIDEWALK_ASPHALT_NORMAL := "res://assets/textures/road/asphalt_04/asphalt_04_nor_gl_2k.png"
const SIDEWALK_ASPHALT_ROUGH := "res://assets/textures/road/asphalt_04/asphalt_04_rough_2k.png"
const SIDEWALK_ASPHALT_DISP := "res://assets/textures/road/asphalt_04/asphalt_04_disp_2k.png"
const SIDEWALK_ASPHALT_BRIGHTNESS := 1.16
const SIDEWALK_ASPHALT_FLOOR := Vector3(0.38, 0.36, 0.32)
const SIDEWALK_ASPHALT_FLOOR_INFLUENCE := 0.28

const CONCRETE_DIFF := "res://assets/textures/general/concrete_layers/concrete_layers_02_diff_4k.jpg"
const CONCRETE_NORMAL := "res://assets/textures/general/concrete_layers/concrete_layers_02_nor_gl_4k.png"
const CONCRETE_ROUGH := "res://assets/textures/general/concrete_layers/concrete_layers_02_rough_4k.png"
const CONCRETE_DISP := "res://assets/textures/general/concrete_layers/concrete_layers_02_disp_4k.png"

const ROAD_SHADER := "res://assets/materials/road.gdshader"
const ROAD_FACE_SHADER := "res://scripts/shaders/road_sidewalk_face.gdshader"
const CONCRETE_SHADER := "res://assets/materials/concrete.gdshader"
const SITE_SURFACE_SHADER := "res://scripts/shaders/site_surface.gdshader"
const SITE_GROUND_SHADER := "res://scripts/shaders/site_ground.gdshader"

static var _texture_cache = {}
static var _shader_cache = {}
static var _road_asphalt_material: ShaderMaterial
static var _road_sidewalk_material: ShaderMaterial
static var _road_sidewalk_face_material: ShaderMaterial
static var _road_concrete_material: ShaderMaterial
static var _site_ground_material: ShaderMaterial
static var _site_asphalt_material: ShaderMaterial
static var _site_concrete_material: ShaderMaterial

static func prewarm_road_materials() -> void:
	road_asphalt_material()
	road_sidewalk_material()
	road_sidewalk_face_material()
	road_concrete_material()

static func road_asphalt_material() -> ShaderMaterial:
	if _road_asphalt_material == null:
		_road_asphalt_material = ShaderMaterial.new()
		_road_asphalt_material.shader = _load_shader(ROAD_SHADER)
		_apply_pbr_textures(
			_road_asphalt_material,
			ROAD_ASPHALT_DIFF,
			ROAD_ASPHALT_NORMAL,
			ROAD_ASPHALT_ROUGH,
			ROAD_ASPHALT_DISP
		)
		_road_asphalt_material.set_shader_parameter("uv_scale", 0.05)
		_road_asphalt_material.set_shader_parameter("macro_uv_scale", 0.007)
		_road_asphalt_material.set_shader_parameter("macro_influence", 0.4)
	return _road_asphalt_material

static func road_sidewalk_material() -> ShaderMaterial:
	if _road_sidewalk_material == null:
		_road_sidewalk_material = ShaderMaterial.new()
		_road_sidewalk_material.resource_name = "road_sidewalk_asphalt_04"
		_road_sidewalk_material.shader = _load_shader(ROAD_SHADER)
		_apply_pbr_textures(
			_road_sidewalk_material,
			SIDEWALK_ASPHALT_DIFF,
			SIDEWALK_ASPHALT_NORMAL,
			SIDEWALK_ASPHALT_ROUGH,
			SIDEWALK_ASPHALT_DISP
		)
		_road_sidewalk_material.set_shader_parameter("uv_scale", 0.12)
		_road_sidewalk_material.set_shader_parameter("macro_uv_scale", 0.018)
		_road_sidewalk_material.set_shader_parameter("macro_influence", 0.25)
		_apply_sidewalk_asphalt_tone(_road_sidewalk_material)
	return _road_sidewalk_material

static func road_sidewalk_face_material() -> ShaderMaterial:
	if _road_sidewalk_face_material == null:
		_road_sidewalk_face_material = ShaderMaterial.new()
		_road_sidewalk_face_material.resource_name = "road_sidewalk_face_asphalt_04"
		_road_sidewalk_face_material.shader = _load_shader(ROAD_FACE_SHADER)
		_apply_pbr_textures(
			_road_sidewalk_face_material,
			SIDEWALK_ASPHALT_DIFF,
			SIDEWALK_ASPHALT_NORMAL,
			SIDEWALK_ASPHALT_ROUGH,
			SIDEWALK_ASPHALT_DISP
		)
		_road_sidewalk_face_material.set_shader_parameter("uv_scale", Vector2(0.12, 0.12))
		_road_sidewalk_face_material.set_shader_parameter("vertical_uv_scale", Vector2(0.12, 0.12))
		_road_sidewalk_face_material.set_shader_parameter("vertical_normal_strength", 0.25)
		_road_sidewalk_face_material.set_shader_parameter("tint", Color(0.88, 0.87, 0.82, 1.0))
	return _road_sidewalk_face_material

static func road_concrete_material() -> ShaderMaterial:
	if _road_concrete_material == null:
		_road_concrete_material = ShaderMaterial.new()
		_road_concrete_material.shader = _load_shader(CONCRETE_SHADER)
		_apply_pbr_textures(
			_road_concrete_material,
			CONCRETE_DIFF,
			CONCRETE_NORMAL,
			CONCRETE_ROUGH,
			CONCRETE_DISP
		)
		_road_concrete_material.set_shader_parameter("uv_scale", 0.1)
	return _road_concrete_material

static func site_surface_material(material: String) -> ShaderMaterial:
	match material:
		MATERIAL_CONCRETE:
			return site_concrete_material()
		_:
			return site_asphalt_material()

static func site_ground_material() -> ShaderMaterial:
	if _site_ground_material == null:
		_site_ground_material = ShaderMaterial.new()
		_site_ground_material.resource_name = "site_ground_grass"
		_site_ground_material.shader = _load_shader(SITE_GROUND_SHADER)
		_apply_site_ground_grass_parameters(_site_ground_material)
	return _site_ground_material

static func site_asphalt_material() -> ShaderMaterial:
	if _site_asphalt_material == null:
		_site_asphalt_material = ShaderMaterial.new()
		_site_asphalt_material.resource_name = "site_asphalt_asphalt_04"
		_site_asphalt_material.shader = _load_shader(SITE_SURFACE_SHADER)
		_apply_pbr_textures(
			_site_asphalt_material,
			SIDEWALK_ASPHALT_DIFF,
			SIDEWALK_ASPHALT_NORMAL,
			SIDEWALK_ASPHALT_ROUGH,
			SIDEWALK_ASPHALT_DISP
		)
		_site_asphalt_material.set_shader_parameter("tint", Color(1.0, 1.0, 1.0, 1.0))
		_site_asphalt_material.set_shader_parameter("uv_scale", 0.12)
		_site_asphalt_material.set_shader_parameter("macro_uv_scale", 0.018)
		_site_asphalt_material.set_shader_parameter("macro_influence", 0.25)
		_apply_sidewalk_asphalt_tone(_site_asphalt_material)
	return _site_asphalt_material

static func site_concrete_material() -> ShaderMaterial:
	if _site_concrete_material == null:
		_site_concrete_material = ShaderMaterial.new()
		_site_concrete_material.shader = _load_shader(SITE_SURFACE_SHADER)
		_apply_pbr_textures(
			_site_concrete_material,
			CONCRETE_DIFF,
			CONCRETE_NORMAL,
			CONCRETE_ROUGH,
			CONCRETE_DISP
		)
		_site_concrete_material.set_shader_parameter("tint", Color(1.0, 1.0, 1.0, 1.0))
		_site_concrete_material.set_shader_parameter("uv_scale", 0.18)
		_site_concrete_material.set_shader_parameter("macro_uv_scale", 0.030)
		_site_concrete_material.set_shader_parameter("macro_influence", 0.12)
		_site_concrete_material.set_shader_parameter("brightness", 1.0)
		_site_concrete_material.set_shader_parameter("albedo_floor", Vector3(0.0, 0.0, 0.0))
		_site_concrete_material.set_shader_parameter("floor_influence", 0.0)
	return _site_concrete_material

static func _apply_pbr_textures(
	material: ShaderMaterial,
	albedo_path: String,
	normal_path: String,
	roughness_path: String,
	displacement_path: String
) -> void:
	material.set_shader_parameter("albedo_tex", _load_texture(albedo_path))
	material.set_shader_parameter("normal_tex", _load_texture(normal_path))
	material.set_shader_parameter("roughness_tex", _load_texture(roughness_path))
	material.set_shader_parameter("displacement_tex", _load_texture(displacement_path))

static func _apply_sidewalk_asphalt_tone(material: ShaderMaterial) -> void:
	material.set_shader_parameter("brightness", SIDEWALK_ASPHALT_BRIGHTNESS)
	material.set_shader_parameter("albedo_floor", SIDEWALK_ASPHALT_FLOOR)
	material.set_shader_parameter("floor_influence", SIDEWALK_ASPHALT_FLOOR_INFLUENCE)

static func _apply_site_ground_grass_parameters(material: ShaderMaterial) -> void:
	material.set_shader_parameter("terrain_grass_albedo", _load_texture(GRASS_ALBEDO))
	material.set_shader_parameter("terrain_grass_height", _load_texture(GRASS_HEIGHT))
	material.set_shader_parameter("scene_sun_direction", SceneLightingConfig.sun_direction())
	material.set_shader_parameter("hillshade_azimuth_deg", 315.0)
	material.set_shader_parameter("hillshade_altitude_deg", 38.0)
	material.set_shader_parameter("hillshade_strength", 0.18)
	material.set_shader_parameter("hillshade_ambient", 0.70)
	material.set_shader_parameter("hillshade_contrast", 1.05)
	material.set_shader_parameter("hillshade_shadow_tint", Color(0.84, 0.90, 0.88))
	material.set_shader_parameter("hillshade_light_tint", Color(1.00, 0.99, 0.94))
	material.set_shader_parameter("terrain_macro_variation_strength", 0.10)
	material.set_shader_parameter("terrain_grass_tint", Color(0.22, 0.42, 0.16))
	material.set_shader_parameter("terrain_grass_tint_strength", 0.0)
	material.set_shader_parameter("terrain_grass_albedo_strength", 0.90)
	material.set_shader_parameter("terrain_grass_macro_scale", 0.018)
	material.set_shader_parameter("terrain_grass_mid_scale", 0.065)
	material.set_shader_parameter("terrain_grass_macro_strength", 0.58)
	material.set_shader_parameter("terrain_grass_mid_strength", 0.80)
	material.set_shader_parameter("terrain_grass_micro_strength", 0.50)
	material.set_shader_parameter("terrain_natural_variation_strength", 0.18)
	material.set_shader_parameter("terrain_meadow_mottle_strength", 0.08)
	material.set_shader_parameter("terrain_grass_detail_scale", 0.34)
	material.set_shader_parameter("terrain_grass_detail_strength", 0.58)
	material.set_shader_parameter("terrain_grass_height_detail_strength", 0.24)
	material.set_shader_parameter("terrain_grass_detail_fade_start", 0.08)
	material.set_shader_parameter("terrain_grass_detail_fade_end", 0.90)
	SceneLightingConfig.apply_ground_shadow_parameters(material)

static func _load_shader(path: String) -> Shader:
	if _shader_cache.has(path):
		return _shader_cache[path]
	var shader := load(path) as Shader
	_shader_cache[path] = shader
	return shader

static func _load_texture(path: String) -> Texture2D:
	if _texture_cache.has(path):
		return _texture_cache[path]

	var tex: Texture2D = null
	if ResourceLoader.exists(path) and _import_dest_files_exist(path):
		tex = load(path)
	if tex == null:
		var abs_path := ProjectSettings.globalize_path(path)
		var image := Image.load_from_file(abs_path)
		if image:
			image.generate_mipmaps()
			tex = ImageTexture.create_from_image(image)

	_texture_cache[path] = tex
	return tex

static func _import_dest_files_exist(path: String) -> bool:
	var import_path := path + ".import"
	if not FileAccess.file_exists(ProjectSettings.globalize_path(import_path)):
		return true

	var cfg := ConfigFile.new()
	if cfg.load(import_path) != OK:
		return true

	var dest_files = cfg.get_value("deps", "dest_files", [])
	if dest_files.is_empty():
		return true
	for dest_file in dest_files:
		if not FileAccess.file_exists(ProjectSettings.globalize_path(str(dest_file))):
			return false
	return true
