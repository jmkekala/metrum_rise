# SPDX-License-Identifier: GPL-2.0-only

## Shared window material factory for gameplay batches and asset previews. Imports remain
## immutable; feature-compatible shader programs and materials are reused across instances.
extends RefCounted

const SOURCE := preload("res://scripts/shaders/building_windows.gdshader")
const REFERENCE_COLOR := Color(1.0, 0.76, 0.52)
const DEFAULT_BRIGHTNESS := 3.0
static var _shaders: Dictionary = {}

static func channel(index: int) -> Vector4:
	if index == BaseMaterial3D.TEXTURE_CHANNEL_GRAYSCALE:
		return Vector4(1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0, 0.0)
	var result := Vector4.ZERO
	result[clampi(index, 0, 3)] = 1.0
	return result

static func create(source: BaseMaterial3D, preview: bool = false, brightness: float = DEFAULT_BRIGHTNESS) -> ShaderMaterial:
	var flags := ""
	if source.normal_enabled: flags += "#define WINDOW_NORMAL\n"
	if source.ao_enabled: flags += "#define WINDOW_AO\n"
	if source.vertex_color_use_as_albedo: flags += "#define WINDOW_VERTEX_COLOR\n"
	if source.cull_mode == BaseMaterial3D.CULL_DISABLED: flags += "#define WINDOW_TWO_SIDED\n"
	if source.transparency != BaseMaterial3D.TRANSPARENCY_DISABLED: flags += "#define WINDOW_ALPHA\n"
	if source.transparency == BaseMaterial3D.TRANSPARENCY_ALPHA_SCISSOR: flags += "#define WINDOW_CUTOUT\n"
	if source.transparency == BaseMaterial3D.TRANSPARENCY_ALPHA_HASH: flags += "#define WINDOW_HASH\n"
	var cull: String = ["cull_back", "cull_front", "cull_disabled"][source.cull_mode]
	var code := SOURCE.code.replace("cull_back", cull)
	var filtering: String = ["filter_nearest", "filter_linear", "filter_nearest_mipmap",
		"filter_linear_mipmap", "filter_nearest_mipmap_anisotropic", "filter_linear_mipmap_anisotropic"][source.texture_filter]
	code = code.replace("filter_linear_mipmap", filtering + (", repeat_enable" if source.texture_repeat else ", repeat_disable"))
	if source.no_depth_test:
		code = code.replace("depth_draw_opaque", "depth_draw_opaque, depth_test_disabled")
	if source.transparency == BaseMaterial3D.TRANSPARENCY_ALPHA_DEPTH_PRE_PASS:
		code = code.replace("depth_draw_opaque", "depth_draw_opaque, depth_prepass_alpha")
	if source.shading_mode == BaseMaterial3D.SHADING_MODE_UNSHADED:
		code = code.replace("diffuse_burley", "unshaded, diffuse_burley")
	code = code.replace("shader_type spatial;", "shader_type spatial;\n" + flags)
	if not _shaders.has(code):
		var shader := Shader.new()
		shader.code = code
		_shaders[code] = shader
	var material := ShaderMaterial.new()
	material.shader = _shaders[code]
	material.render_priority = source.render_priority
	material.next_pass = source.next_pass
	material.resource_name = source.resource_name
	var values := {
		"window_strength": brightness,
		"window_preview": preview, "albedo": source.albedo_color,
		"texture_albedo": source.albedo_texture, "texture_emission": source.emission_texture,
		"texture_normal": source.normal_texture, "normal_scale": source.normal_scale,
		"roughness": source.roughness, "metallic": source.metallic, "specular": source.metallic_specular,
		"uv1_scale": source.uv1_scale, "uv1_offset": source.uv1_offset,
		"uv2_scale": source.uv2_scale, "uv2_offset": source.uv2_offset,
		"emission_uv2": source.emission_on_uv2, "ao_uv2": source.ao_on_uv2,
		"ao_light_affect": source.ao_light_affect, "alpha_scissor": source.alpha_scissor_threshold,
	}
	if source is ORMMaterial3D:
		# ORMMaterial3D reads roughness/metallic directly from G/B, without scalar factors.
		values["roughness"] = 1.0
		values["metallic"] = 1.0
		values["specular"] = 0.5
		values["ao_uv2"] = false
		values.merge({"texture_ao": source.orm_texture, "texture_roughness": source.orm_texture,
			"texture_metallic": source.orm_texture, "ao_channel": channel(0),
			"roughness_channel": channel(1), "metallic_channel": channel(2)})
	else:
		values.merge({"texture_ao": source.ao_texture, "texture_roughness": source.roughness_texture,
			"texture_metallic": source.metallic_texture, "ao_channel": channel(source.ao_texture_channel),
			"roughness_channel": channel(source.roughness_texture_channel),
			"metallic_channel": channel(source.metallic_texture_channel)})
	for name: String in values:
		material.set_shader_parameter(name, values[name])
	return material
