# SPDX-License-Identifier: GPL-2.0-only

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

const SIDEWALK_ASPHALT_DIFF := "res://assets/textures/road/asphalt_04/asphalt_04_diff_2k.jpg"
const SIDEWALK_ASPHALT_NORMAL := "res://assets/textures/road/asphalt_04/asphalt_04_nor_gl_2k.png"
const SIDEWALK_ASPHALT_ROUGH := "res://assets/textures/road/asphalt_04/asphalt_04_rough_2k.png"
const SIDEWALK_ASPHALT_BRIGHTNESS := 1.16
const SIDEWALK_ASPHALT_FLOOR := Vector3(0.38, 0.36, 0.32)
const SIDEWALK_ASPHALT_FLOOR_INFLUENCE := 0.28

const CONCRETE_DIFF := "res://assets/textures/general/concrete_layers/concrete_layers_02_diff_4k.jpg"
const CONCRETE_NORMAL := "res://assets/textures/general/concrete_layers/concrete_layers_02_nor_gl_4k.png"
const CONCRETE_ROUGH := "res://assets/textures/general/concrete_layers/concrete_layers_02_rough_4k.png"

const ROAD_SHADER := "res://assets/materials/road.gdshader"
const ROAD_PREVIEW_SHADER := "res://scripts/shaders/road_preview.gdshader"
const ROAD_FACE_SHADER := "res://scripts/shaders/road_sidewalk_face.gdshader"
const CONCRETE_SHADER := "res://assets/materials/concrete.gdshader"
const SITE_SURFACE_SHADER := "res://scripts/shaders/site_surface.gdshader"
const SITE_GROUND_SHADER := "res://scripts/shaders/site_ground.gdshader"
const TERRAIN_SHADER := "res://assets/materials/terrain.gdshader"
const FIELD_OVERLAY_SHADER := "res://scripts/shaders/field_overlay.gdshader"

static var _texture_cache = {}
static var _shader_cache = {}
static var _road_asphalt_material: ShaderMaterial
static var _road_sidewalk_material: ShaderMaterial
static var _road_sidewalk_face_material: ShaderMaterial
static var _road_concrete_material: ShaderMaterial
static var _site_ground_material: ShaderMaterial
static var _flat_terrain_material: ShaderMaterial
static var _site_asphalt_material: ShaderMaterial
static var _site_concrete_material: ShaderMaterial

static func prewarm_road_materials() -> void:
	road_asphalt_material()
	road_sidewalk_material()
	road_sidewalk_face_material()
	road_concrete_material()

static func road_preview_material() -> ShaderMaterial:
	# Per-tool status/lane uniforms; the large texture resources remain shared with committed roads.
	var material := ShaderMaterial.new()
	material.shader = _load_shader(ROAD_PREVIEW_SHADER)
	material.render_priority = 5
	material.set_shader_parameter("asphalt_tex", load_texture(ROAD_ASPHALT_DIFF))
	material.set_shader_parameter("sidewalk_tex", load_texture(SIDEWALK_ASPHALT_DIFF))
	return material

static func road_asphalt_material() -> ShaderMaterial:
	if _road_asphalt_material == null:
		_road_asphalt_material = _new_paving_material(
			ROAD_SHADER, ROAD_ASPHALT_DIFF, ROAD_ASPHALT_NORMAL, ROAD_ASPHALT_ROUGH,
			0.05, 0.007, 0.4
		)
	return _road_asphalt_material

static func road_sidewalk_material() -> ShaderMaterial:
	if _road_sidewalk_material == null:
		_road_sidewalk_material = _new_sidewalk_asphalt(ROAD_SHADER)
		_road_sidewalk_material.resource_name = "road_sidewalk_asphalt_04"
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
			SIDEWALK_ASPHALT_ROUGH
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
			"", # This shader uses geometric normals.
			CONCRETE_ROUGH
		)
		_road_concrete_material.set_shader_parameter("uv_scale", 0.1)
	return _road_concrete_material

static func site_surface_material(material: String) -> ShaderMaterial:
	match material:
		MATERIAL_CONCRETE:
			return site_concrete_material()
		_:
			return site_asphalt_material()

static func field_overlay_material(
	albedo_path: String,
	fallback_color: Color,
	texture_tile_m: float,
	render_priority: int
) -> ShaderMaterial:
	var material := ShaderMaterial.new()
	material.resource_name = "field_overlay_grain"
	material.shader = _load_shader(FIELD_OVERLAY_SHADER)
	material.render_priority = render_priority
	material.set_shader_parameter("albedo_tex", load_texture_or_solid(albedo_path, fallback_color))
	material.set_shader_parameter("uv_scale", 1.0 / max(texture_tile_m, 0.001))
	return material

## Preview terrain uses the game's shader/textures with constant zero height and no water.
static func flat_terrain_material() -> ShaderMaterial:
	if _flat_terrain_material == null:
		_flat_terrain_material = ShaderMaterial.new()
		_flat_terrain_material.resource_name = "flat_terrain_grass"
		_flat_terrain_material.shader = _load_shader(TERRAIN_SHADER)
		var flat_image := Image.create(2, 2, false, Image.FORMAT_RF)
		flat_image.fill(Color.BLACK)
		var flat_texture := ImageTexture.create_from_image(flat_image)
		_flat_terrain_material.set_shader_parameter("heightmap", flat_texture)
		_flat_terrain_material.set_shader_parameter("watermap", flat_texture)
		_flat_terrain_material.set_shader_parameter("height_is_baked", true)
		_flat_terrain_material.set_shader_parameter("heightmap_texture_size", Vector2(2.0, 2.0))
		_flat_terrain_material.set_shader_parameter("inner_sample_offset_texels", Vector2.ZERO)
		_flat_terrain_material.set_shader_parameter("terrain_grass_albedo", load_texture(GRASS_ALBEDO))
		_flat_terrain_material.set_shader_parameter("terrain_grass_height", load_texture(GRASS_HEIGHT))
		SceneLightingConfig.apply_ground_shadow_parameters(_flat_terrain_material)
	return _flat_terrain_material

static func site_ground_material() -> ShaderMaterial:
	if _site_ground_material == null:
		_site_ground_material = ShaderMaterial.new()
		_site_ground_material.resource_name = "site_ground_grass"
		_site_ground_material.shader = _load_shader(SITE_GROUND_SHADER)
		_apply_site_ground_grass_parameters(_site_ground_material)
	return _site_ground_material

static func site_asphalt_material() -> ShaderMaterial:
	if _site_asphalt_material == null:
		_site_asphalt_material = _new_sidewalk_asphalt(SITE_SURFACE_SHADER)
		_site_asphalt_material.resource_name = "site_asphalt_asphalt_04"
	return _site_asphalt_material

static func site_concrete_material() -> ShaderMaterial:
	if _site_concrete_material == null:
		_site_concrete_material = _new_paving_material(
			SITE_SURFACE_SHADER, CONCRETE_DIFF, CONCRETE_NORMAL, CONCRETE_ROUGH,
			0.18, 0.030, 0.12
		)
	return _site_concrete_material

static func _new_paving_material(
	shader_path: String, albedo_path: String, normal_path: String, roughness_path: String,
	detail_scale: float, macro_scale: float, macro_influence: float
) -> ShaderMaterial:
	var material := ShaderMaterial.new()
	material.shader = _load_shader(shader_path)
	_apply_pbr_textures(material, albedo_path, normal_path, roughness_path)
	material.set_shader_parameter("uv_scale", detail_scale)
	material.set_shader_parameter("macro_uv_scale", macro_scale)
	material.set_shader_parameter("macro_influence", macro_influence)
	# Explicit values are also copied into the terrain's per-material paving uniforms.
	material.set_shader_parameter("brightness", 1.0)
	material.set_shader_parameter("albedo_floor", Vector3.ZERO)
	material.set_shader_parameter("floor_influence", 0.0)
	return material

static func _new_sidewalk_asphalt(shader_path: String) -> ShaderMaterial:
	var material := _new_paving_material(
		shader_path, SIDEWALK_ASPHALT_DIFF, SIDEWALK_ASPHALT_NORMAL, SIDEWALK_ASPHALT_ROUGH,
		0.12, 0.018, 0.25
	)
	_apply_sidewalk_asphalt_tone(material)
	return material

## Bind the same paving textures and tone to the terrain's tagged frontage faces.
static func apply_terrain_paving_parameters(material: ShaderMaterial) -> void:
	for kind: String in [MATERIAL_ASPHALT, MATERIAL_CONCRETE]:
		var paving := site_surface_material(kind)
		for parameter: String in ["albedo_tex", "normal_tex", "roughness_tex", "uv_scale", "macro_uv_scale", "macro_influence", "brightness", "albedo_floor", "floor_influence"]:
			material.set_shader_parameter("site_" + kind + "_" + parameter, paving.get_shader_parameter(parameter))

static func _apply_pbr_textures(
	material: ShaderMaterial,
	albedo_path: String,
	normal_path: String,
	roughness_path: String
) -> void:
	material.set_shader_parameter("albedo_tex", load_texture(albedo_path))
	if not normal_path.is_empty():
		material.set_shader_parameter("normal_tex", load_texture(normal_path))
	material.set_shader_parameter("roughness_tex", load_texture(roughness_path))

static func _apply_sidewalk_asphalt_tone(material: ShaderMaterial) -> void:
	material.set_shader_parameter("brightness", SIDEWALK_ASPHALT_BRIGHTNESS)
	material.set_shader_parameter("albedo_floor", SIDEWALK_ASPHALT_FLOOR)
	material.set_shader_parameter("floor_influence", SIDEWALK_ASPHALT_FLOOR_INFLUENCE)

static func _apply_site_ground_grass_parameters(material: ShaderMaterial) -> void:
	material.set_shader_parameter("terrain_grass_albedo", load_texture(GRASS_ALBEDO))
	material.set_shader_parameter("terrain_grass_height", load_texture(GRASS_HEIGHT))
	SceneLightingConfig.apply_ground_shadow_parameters(material)

static func _load_shader(path: String) -> Shader:
	if _shader_cache.has(path):
		return _shader_cache[path]
	var shader := load(path) as Shader
	_shader_cache[path] = shader
	return shader

static func load_texture(path: String) -> Texture2D:
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

static func load_texture_or_solid(path: String, fallback_color: Color) -> Texture2D:
	var tex := load_texture(path)
	if tex != null:
		return tex

	var cache_key := "%s|solid:%s" % [path, str(fallback_color)]
	if _texture_cache.has(cache_key):
		return _texture_cache[cache_key]

	var fallback_image := Image.create(1, 1, false, Image.FORMAT_RGBA8)
	fallback_image.fill(fallback_color)
	tex = ImageTexture.create_from_image(fallback_image)
	_texture_cache[cache_key] = tex
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
