# SPDX-License-Identifier: GPL-2.0-only

## Morning mist over the lakes, as a compositor pass after the transparent pass.
##
## [SceneLighting] uploads the Rust mist map (where mist can lie and the level of the water it
## lies on) when the water or terrain changes, and sets the density, colour and sun each frame.
## The pass is disabled whenever the density is zero, which is most of the day, so it costs
## nothing outside the misty morning hours.
extends CompositorEffect

const SHADER := preload("res://scripts/shaders/weather_mist.glsl")
const WeatherCloudShape := preload("res://scripts/renderers/weather_cloud_shape.gd")
const GROUP := 8

## Extinction per metre at the water surface over open water; zero disables the pass.
var density := 0.0
## Height in metres over which the density falls by e.
var scale_height := 12.0
## Height of the mist's ceiling in metres, above every lake's surface.
var ceiling := 0.0
## Lit mist colour.
var color := Color(0.8, 0.82, 0.85)
## Distance the wind has carried the air, as `weather_wind.zw`: the mist banks drift with it.
var wind_offset := Vector2.ZERO
## Direction towards the sun, its light colour, and how strongly mist glows towards it.
var sun := Vector3.UP
var sun_color := Color.WHITE
var sun_scatter := 0.0
## The mist map as an RG float texture with mipmaps: presence, presence x water level.
var map_texture: Texture2D
## World X/Z of map cell (0, 0)'s centre, metres per cell and the map size in cells.
var map_origin := Vector2.ZERO
var map_cell_m := 32.0
var map_size := Vector2i.ONE

var _rd: RenderingDevice
var _shader: RID
var _pipeline: RID
var _params: RID
var _nearest: RID
var _linear: RID
var _repeat: RID
var _params_data := PackedByteArray()
var _view_uniforms: Array[Array] = []
var _view_colors: Array[RID] = []
var _view_depths: Array[RID] = []
var _map_rd: RID
var _shape_rd: RID

func _init() -> void:
	effect_callback_type = EFFECT_CALLBACK_TYPE_POST_TRANSPARENT
	enabled = false
	RenderingServer.call_on_render_thread(_initialize)

func _notification(what: int) -> void:
	if what != NOTIFICATION_PREDELETE or _rd == null:
		return
	_free_on_render_thread(_rd, [_params, _nearest, _linear, _repeat, _shader])

# Frees RenderingDevice resources on the render thread, which RenderingDevice requires. A static
# function on the script, so a free queued from a destroyed effect does not call into it.
static func _free_rids(rd: RenderingDevice, rids: Array) -> void:
	for rid: RID in rids:
		if rid.is_valid():
			rd.free_rid(rid)

static func _free_on_render_thread(rd: RenderingDevice, rids: Array) -> void:
	if RenderingServer.is_on_render_thread():
		_free_rids(rd, rids)
	else:
		RenderingServer.call_on_render_thread(_free_rids.bind(rd, rids))

func _initialize() -> void:
	_rd = RenderingServer.get_rendering_device()
	if _rd == null:
		return
	_shader = _rd.shader_create_from_spirv(SHADER.get_spirv())
	_pipeline = _rd.compute_pipeline_create(_shader)
	_params_data.resize(224)
	_params = _rd.uniform_buffer_create(_params_data.size())
	_nearest = _sampler(RenderingDevice.SAMPLER_FILTER_NEAREST, RenderingDevice.SAMPLER_REPEAT_MODE_CLAMP_TO_EDGE)
	_linear = _sampler(RenderingDevice.SAMPLER_FILTER_LINEAR, RenderingDevice.SAMPLER_REPEAT_MODE_CLAMP_TO_EDGE)
	_repeat = _sampler(RenderingDevice.SAMPLER_FILTER_LINEAR, RenderingDevice.SAMPLER_REPEAT_MODE_REPEAT)

func _sampler(filter: int, repeat: int) -> RID:
	var state := RDSamplerState.new()
	state.min_filter = filter
	state.mag_filter = filter
	state.mip_filter = filter
	state.repeat_u = repeat
	state.repeat_v = repeat
	return _rd.sampler_create(state)

func _render_callback(callback_type: int, render_data: RenderData) -> void:
	if (callback_type != EFFECT_CALLBACK_TYPE_POST_TRANSPARENT or _rd == null or density <= 0.0
			or map_texture == null):
		return
	var map_rd := RenderingServer.texture_get_rd_texture(map_texture.get_rid())
	var shape := WeatherCloudShape.shape_texture()
	if shape == null:
		return
	var shape_rd := RenderingServer.texture_get_rd_texture(shape.get_rid())
	var buffers := render_data.get_render_scene_buffers() as RenderSceneBuffersRD
	var scene := render_data.get_render_scene_data()
	if buffers == null or scene == null or not map_rd.is_valid() or not shape_rd.is_valid():
		return
	var size := buffers.get_internal_size()
	if size.x <= 0 or size.y <= 0:
		return
	if map_rd != _map_rd:
		for uniforms in _view_uniforms:
			_set_texture(uniforms[2], map_rd, _linear)
		_map_rd = map_rd
	if shape_rd != _shape_rd:
		for uniforms in _view_uniforms:
			_set_texture(uniforms[4], shape_rd, _repeat)
		_shape_rd = shape_rd
	_pack_params(scene.get_cam_projection().inverse(), scene.get_cam_transform(), size)
	_rd.buffer_update(_params, 0, _params_data.size(), _params_data)
	var list := _rd.compute_list_begin()
	_rd.compute_list_bind_compute_pipeline(list, _pipeline)
	for view in buffers.get_view_count():
		var color_rd := buffers.get_color_layer(view)
		var depth_rd := buffers.get_depth_layer(view)
		if view >= _view_uniforms.size():
			var uniforms: Array[RDUniform] = [
				_uniform(RenderingDevice.UNIFORM_TYPE_IMAGE, 0, [color_rd]),
				_uniform(RenderingDevice.UNIFORM_TYPE_SAMPLER_WITH_TEXTURE, 1, [_nearest, depth_rd]),
				_uniform(RenderingDevice.UNIFORM_TYPE_SAMPLER_WITH_TEXTURE, 2, [_linear, map_rd]),
				_uniform(RenderingDevice.UNIFORM_TYPE_UNIFORM_BUFFER, 3, [_params]),
				_uniform(RenderingDevice.UNIFORM_TYPE_SAMPLER_WITH_TEXTURE, 4, [_repeat, shape_rd]),
			]
			_view_uniforms.append(uniforms)
			_view_colors.append(color_rd)
			_view_depths.append(depth_rd)
		if color_rd != _view_colors[view]:
			_set_texture(_view_uniforms[view][0], color_rd)
			_view_colors[view] = color_rd
		if depth_rd != _view_depths[view]:
			_set_texture(_view_uniforms[view][1], depth_rd, _nearest)
			_view_depths[view] = depth_rd
		var uniform_set := UniformSetCacheRD.get_cache(_shader, 0, _view_uniforms[view])
		_rd.compute_list_bind_uniform_set(list, uniform_set, 0)
		_rd.compute_list_dispatch(list, ceili(size.x / float(GROUP)), ceili(size.y / float(GROUP)), 1)
	_rd.compute_list_end()

# std140 layout of the shader's Params block: two mat4, then six vec4.
func _pack_params(inv_projection: Projection, camera: Transform3D, size: Vector2i) -> void:
	_pack_matrix(0, inv_projection)
	_pack_matrix(64, Projection(camera))
	var levels := 0.0
	if map_texture != null:
		levels = floorf(log(float(maxi(map_size.x, map_size.y))) / log(2.0))
	_pack_vec4(128, map_origin.x, map_origin.y, map_cell_m, levels)
	_pack_vec4(144, map_size.x, map_size.y, density, scale_height)
	_pack_vec4(160, color.r, color.g, color.b, ceiling)
	_pack_vec4(176, sun.x, sun.y, sun.z, sun_scatter)
	_pack_vec4(192, sun_color.r, sun_color.g, sun_color.b, 0.0)
	_pack_vec4(208, size.x, size.y, wind_offset.x, wind_offset.y)

func _pack_matrix(offset: int, matrix: Projection) -> void:
	for i in 4:
		var column: Vector4 = matrix[i]
		_pack_vec4(offset + i * 16, column.x, column.y, column.z, column.w)

func _pack_vec4(offset: int, x: float, y: float, z: float, w: float) -> void:
	_params_data.encode_float(offset, x)
	_params_data.encode_float(offset + 4, y)
	_params_data.encode_float(offset + 8, z)
	_params_data.encode_float(offset + 12, w)

# Only called when a texture RID changes; uniform sets stay owned by UniformSetCacheRD.
func _set_texture(uniform: RDUniform, texture: RID, sampler: RID = RID()) -> void:
	uniform.clear_ids()
	if sampler.is_valid():
		uniform.add_id(sampler)
	uniform.add_id(texture)

func _uniform(type: int, binding: int, ids: Array) -> RDUniform:
	var uniform := RDUniform.new()
	uniform.uniform_type = type
	uniform.binding = binding
	for id in ids:
		uniform.add_id(id)
	return uniform
