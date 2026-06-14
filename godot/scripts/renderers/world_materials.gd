## Shared world-rendering material factory and texture cache.
## Rendering tools should request common asphalt/concrete materials here instead
## of rebuilding shader materials or reloading texture sets locally.
extends RefCounted
class_name WorldMaterials

const MATERIAL_ASPHALT := "asphalt"
const MATERIAL_CONCRETE := "concrete"

const ASPHALT_DIFF := "res://assets/textures/road/clean_asphalt/clean_asphalt_diff_4k.jpg"
const ASPHALT_NORMAL := "res://assets/textures/road/clean_asphalt/clean_asphalt_nor_gl_4k.png"
const ASPHALT_ROUGH := "res://assets/textures/road/clean_asphalt/clean_asphalt_rough_4k.png"
const ASPHALT_DISP := "res://assets/textures/road/clean_asphalt/clean_asphalt_disp_4k.png"

const CONCRETE_DIFF := "res://assets/textures/general/concrete_layers/concrete_layers_02_diff_4k.jpg"
const CONCRETE_NORMAL := "res://assets/textures/general/concrete_layers/concrete_layers_02_nor_gl_4k.png"
const CONCRETE_ROUGH := "res://assets/textures/general/concrete_layers/concrete_layers_02_rough_4k.png"
const CONCRETE_DISP := "res://assets/textures/general/concrete_layers/concrete_layers_02_disp_4k.png"

const ROAD_SHADER := "res://assets/materials/road.gdshader"
const CONCRETE_SHADER := "res://assets/materials/concrete.gdshader"
const SITE_SURFACE_SHADER := "res://scripts/shaders/site_surface.gdshader"

static var _texture_cache = {}
static var _shader_cache = {}
static var _road_asphalt_material: ShaderMaterial
static var _road_concrete_material: ShaderMaterial
static var _site_asphalt_material: ShaderMaterial
static var _site_concrete_material: ShaderMaterial

static func road_asphalt_material() -> ShaderMaterial:
	if _road_asphalt_material == null:
		_road_asphalt_material = ShaderMaterial.new()
		_road_asphalt_material.shader = _load_shader(ROAD_SHADER)
		_apply_pbr_textures(_road_asphalt_material, ASPHALT_DIFF, ASPHALT_NORMAL, ASPHALT_ROUGH, ASPHALT_DISP)
		_road_asphalt_material.set_shader_parameter("uv_scale", 0.05)
		_road_asphalt_material.set_shader_parameter("macro_uv_scale", 0.007)
		_road_asphalt_material.set_shader_parameter("macro_influence", 0.4)
	return _road_asphalt_material

static func road_concrete_material() -> ShaderMaterial:
	if _road_concrete_material == null:
		_road_concrete_material = ShaderMaterial.new()
		_road_concrete_material.shader = _load_shader(CONCRETE_SHADER)
		_apply_pbr_textures(_road_concrete_material, CONCRETE_DIFF, CONCRETE_NORMAL, CONCRETE_ROUGH, CONCRETE_DISP)
		_road_concrete_material.set_shader_parameter("uv_scale", 0.1)
	return _road_concrete_material

static func site_surface_material(material: String) -> ShaderMaterial:
	match material:
		MATERIAL_CONCRETE:
			return site_concrete_material()
		_:
			return site_asphalt_material()

static func site_asphalt_material() -> ShaderMaterial:
	if _site_asphalt_material == null:
		_site_asphalt_material = ShaderMaterial.new()
		_site_asphalt_material.shader = _load_shader(SITE_SURFACE_SHADER)
		_apply_pbr_textures(_site_asphalt_material, ASPHALT_DIFF, ASPHALT_NORMAL, ASPHALT_ROUGH, ASPHALT_DISP)
		_site_asphalt_material.set_shader_parameter("tint", Color(1.0, 1.0, 1.0, 1.0))
		_site_asphalt_material.set_shader_parameter("uv_scale", 0.05)
		_site_asphalt_material.set_shader_parameter("macro_uv_scale", 0.007)
		_site_asphalt_material.set_shader_parameter("macro_influence", 0.18)
		_site_asphalt_material.set_shader_parameter("brightness", 1.45)
		_site_asphalt_material.set_shader_parameter("albedo_floor", Vector3(0.24, 0.25, 0.24))
		_site_asphalt_material.set_shader_parameter("floor_influence", 0.85)
	return _site_asphalt_material

static func site_concrete_material() -> ShaderMaterial:
	if _site_concrete_material == null:
		_site_concrete_material = ShaderMaterial.new()
		_site_concrete_material.shader = _load_shader(SITE_SURFACE_SHADER)
		_apply_pbr_textures(_site_concrete_material, CONCRETE_DIFF, CONCRETE_NORMAL, CONCRETE_ROUGH, CONCRETE_DISP)
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
	if ResourceLoader.exists(path):
		tex = load(path)
	else:
		var abs_path := ProjectSettings.globalize_path(path)
		var image := Image.load_from_file(abs_path)
		if image:
			image.generate_mipmaps()
			tex = ImageTexture.create_from_image(image)

	_texture_cache[path] = tex
	return tex
