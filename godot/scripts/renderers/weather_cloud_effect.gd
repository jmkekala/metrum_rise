# SPDX-License-Identifier: GPL-2.0-only

## Volumetric clouds as a compositor pass over the sky.
##
## Godot's sky shader keeps nothing between frames, so a sky-shader march must take every step
## every frame; with too few steps its clouds read as stacked sheets. This pass marches a
## low-resolution buffer with a fresh jitter each frame and blends it into the reprojected
## history, then lays the result over the pixels where the sky is visible. A second, smaller
## march fills a hemisphere panorama around the camera, published as the
## `weather_cloud_reflection` global for the water. A third pass bakes the clouds' sun
## transmission over a window of the deck around the camera, published as
## `weather_cloud_shadow_map`, so every lit surface reads one texel instead of evaluating the
## weather map; trees did that per vertex, at up to 0.7 ms a frame in a wooded town. The bake is
## read the frame after it is made: a bake before the opaque pass splits the frame's render pass,
## which cost more than it saved. [SceneLighting] therefore publishes each window a frame late,
## with the bake made from it, and the only lag is a frame of cloud drift.
##
## [SceneLighting] writes the weather and palette fields below every frame. All GPU work runs on
## the render thread.
extends CompositorEffect

const NOISE_SHADER := preload("res://scripts/shaders/weather_cloud_noise.glsl")
const MARCH_SHADER := preload("res://scripts/shaders/weather_cloud_march.glsl")
const COMPOSITE_SHADER := preload("res://scripts/shaders/weather_cloud_composite.glsl")
const SHADOW_SHADER := preload("res://scripts/shaders/weather_cloud_shadow_map.glsl")
const WeatherCloudShape := preload("res://scripts/renderers/weather_cloud_shape.gd")

const NOISE_TEXELS := 128
# The cloud buffer is this fraction of the render size on each axis.
const VIEW_DIVISOR := 4
const PANORAMA_TEXELS := 256
const SHADOW_TEXELS := 1024
const GROUP := 8

## xy: world offset, z: coverage, w: optical depth, as the `weather_clouds` global.
var clouds := Vector4(0.0, 0.0, 0.45, 1.2)
## xyz: towards the sun, w: 1 by day, as the `weather_sun` global.
var sun := Vector4(0.0, 1.0, 0.0, 1.0)
var shade := Color(0.52, 0.58, 0.64)
var light := Color(0.88, 0.91, 0.94)
var horizon := Color(0.7, 0.8, 0.9)
## High cloud layer: world X/Z offset of its lookup in xy, cover in z.
var high_clouds := Vector4.ZERO
## Rain intensity in [0, 1]: a raining deck's base darkens.
var rain := 0.0
## The hemisphere the water reflects. Valid once the pass has run.
var reflection_texture := Texture2DRD.new()
## Window of the deck to bake the sun transmission over this frame: xy its corner's world X/Z,
## z metres per texel (0 skips the bake), w the separation of the reference receiver below the
## shadow-casting height. Written by [SceneLighting].
var shadow_window := Vector4.ZERO
## The baked transmission, read by every receiver the frame after it is made.
var shadow_texture := Texture2DRD.new()

var _rd: RenderingDevice
var _noise_shader: RID
var _march_shader: RID
var _composite_shader: RID
var _shadow_shader: RID
var _shadow_pipeline: RID
var _shadow_rd: RID
var _noise_pipeline: RID
var _march_pipeline: RID
var _composite_pipeline: RID
var _noise_texture: RID
var _linear_repeat: RID
var _linear_clamp: RID
var _nearest_clamp: RID
var _view_params: RID
var _panorama_params: RID
var _view_bytes := PackedByteArray()
var _panorama_bytes := PackedByteArray()
var _composite_push := PackedByteArray()
var _shadow_push := PackedByteArray()
var _view_uniforms: Array[Array] = []
var _panorama_uniforms: Array[Array] = []
var _composite_uniforms: Array[Array] = []
var _composite_colors: Array[RID] = []
var _composite_depths: Array[RID] = []
var _shadow_uniforms: Array[RDUniform] = []
var _shape_rd: RID
# Ping-pong pairs: one is written each frame, the other read as history.
var _view_textures: Array[RID] = [RID(), RID()]
var _panorama_textures: Array[RID] = [RID(), RID()]
var _reflection_rd: RID
var _view_size := Vector2i.ZERO
var _frame := 0
var _view_history_valid := false
var _panorama_history_valid := false
var _previous_view_projection := Projection.IDENTITY

func _init() -> void:
	# Writes the clouds into the colour texture after the sky. The project renders without
	# MSAA; with MSAA the opaque pass would keep drawing into the multisampled target and its
	# final resolve would overwrite this composite, so enabling MSAA needs another composite path.
	effect_callback_type = EFFECT_CALLBACK_TYPE_POST_SKY
	access_resolved_depth = true
	RenderingServer.call_on_render_thread(_initialize)

func _notification(what: int) -> void:
	if what != NOTIFICATION_PREDELETE or _rd == null:
		return
	reflection_texture.texture_rd_rid = RID()
	shadow_texture.texture_rd_rid = RID()
	_free_on_render_thread(_rd, _view_textures + _panorama_textures + [_reflection_rd, _shadow_rd,
		_noise_texture, _linear_repeat, _linear_clamp, _nearest_clamp, _view_params,
		_panorama_params, _noise_shader, _march_shader, _composite_shader, _shadow_shader])

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
	_noise_shader = _rd.shader_create_from_spirv(NOISE_SHADER.get_spirv())
	_march_shader = _rd.shader_create_from_spirv(MARCH_SHADER.get_spirv())
	_composite_shader = _rd.shader_create_from_spirv(COMPOSITE_SHADER.get_spirv())
	_noise_pipeline = _rd.compute_pipeline_create(_noise_shader)
	_march_pipeline = _rd.compute_pipeline_create(_march_shader)
	_composite_pipeline = _rd.compute_pipeline_create(_composite_shader)
	_shadow_shader = _rd.shader_create_from_spirv(SHADOW_SHADER.get_spirv())
	_shadow_pipeline = _rd.compute_pipeline_create(_shadow_shader)
	_linear_repeat = _create_sampler(RenderingDevice.SAMPLER_REPEAT_MODE_REPEAT, true)
	_linear_clamp = _create_sampler(RenderingDevice.SAMPLER_REPEAT_MODE_CLAMP_TO_EDGE, true)
	_nearest_clamp = _create_sampler(RenderingDevice.SAMPLER_REPEAT_MODE_CLAMP_TO_EDGE, false)
	_view_bytes.resize(320)
	_panorama_bytes.resize(320)
	_composite_push.resize(16)
	_shadow_push.resize(48)
	_view_params = _rd.uniform_buffer_create(_view_bytes.size())
	_panorama_params = _rd.uniform_buffer_create(_panorama_bytes.size())
	_bake_noise()
	for i in 2:
		_panorama_textures[i] = _create_cloud_texture(Vector2i(PANORAMA_TEXELS, PANORAMA_TEXELS))
	_reflection_rd = _create_cloud_texture(Vector2i(PANORAMA_TEXELS, PANORAMA_TEXELS))
	reflection_texture.texture_rd_rid = _reflection_rd
	var shadow_format := RDTextureFormat.new()
	shadow_format.format = RenderingDevice.DATA_FORMAT_R16_SFLOAT
	shadow_format.width = SHADOW_TEXELS
	shadow_format.height = SHADOW_TEXELS
	shadow_format.usage_bits = (RenderingDevice.TEXTURE_USAGE_SAMPLING_BIT
		| RenderingDevice.TEXTURE_USAGE_STORAGE_BIT
		| RenderingDevice.TEXTURE_USAGE_CAN_UPDATE_BIT
		| RenderingDevice.TEXTURE_USAGE_CAN_COPY_TO_BIT)
	_shadow_rd = _rd.texture_create(shadow_format, RDTextureView.new())
	# No cloud until the first bake.
	_rd.texture_clear(_shadow_rd, Color(1.0, 1.0, 1.0, 1.0), 0, 1, 0, 1)
	shadow_texture.texture_rd_rid = _shadow_rd
	for i in 2:
		_view_uniforms.append(_march_uniforms(_view_params, RID(), RID()))
		_panorama_uniforms.append(_march_uniforms(_panorama_params, _panorama_textures[1 - i],
			_panorama_textures[i]))
	_shadow_uniforms = [
		_sampler_uniform(0, _linear_repeat, RID()),
		_image_uniform(1, _shadow_rd),
	]

func _create_sampler(repeat: int, linear: bool) -> RID:
	var state := RDSamplerState.new()
	var filter := RenderingDevice.SAMPLER_FILTER_LINEAR if linear else RenderingDevice.SAMPLER_FILTER_NEAREST
	state.min_filter = filter
	state.mag_filter = filter
	state.mip_filter = filter
	state.repeat_u = repeat
	state.repeat_v = repeat
	state.repeat_w = repeat
	return _rd.sampler_create(state)

func _create_cloud_texture(size: Vector2i) -> RID:
	var format := RDTextureFormat.new()
	format.format = RenderingDevice.DATA_FORMAT_R16G16B16A16_SFLOAT
	format.width = size.x
	format.height = size.y
	format.usage_bits = (RenderingDevice.TEXTURE_USAGE_SAMPLING_BIT
		| RenderingDevice.TEXTURE_USAGE_STORAGE_BIT
		| RenderingDevice.TEXTURE_USAGE_CAN_UPDATE_BIT
		| RenderingDevice.TEXTURE_USAGE_CAN_COPY_FROM_BIT
		| RenderingDevice.TEXTURE_USAGE_CAN_COPY_TO_BIT)
	var texture := _rd.texture_create(format, RDTextureView.new())
	# Clear sky: no scattered light, full transmittance.
	_rd.texture_clear(texture, Color(0.0, 0.0, 0.0, 1.0), 0, 1, 0, 1)
	return texture

func _bake_noise() -> void:
	var format := RDTextureFormat.new()
	format.texture_type = RenderingDevice.TEXTURE_TYPE_3D
	format.format = RenderingDevice.DATA_FORMAT_R8G8B8A8_UNORM
	format.width = NOISE_TEXELS
	format.height = NOISE_TEXELS
	format.depth = NOISE_TEXELS
	format.usage_bits = (RenderingDevice.TEXTURE_USAGE_SAMPLING_BIT
		| RenderingDevice.TEXTURE_USAGE_STORAGE_BIT)
	_noise_texture = _rd.texture_create(format, RDTextureView.new())
	var uniform_set := UniformSetCacheRD.get_cache(_noise_shader, 0, [_image_uniform(0, _noise_texture)])
	var list := _rd.compute_list_begin()
	_rd.compute_list_bind_compute_pipeline(list, _noise_pipeline)
	_rd.compute_list_bind_uniform_set(list, uniform_set, 0)
	var groups := NOISE_TEXELS / 4
	_rd.compute_list_dispatch(list, groups, groups, groups)
	_rd.compute_list_end()

func _render_callback(callback_type: int, render_data: RenderData) -> void:
	if callback_type != EFFECT_CALLBACK_TYPE_POST_SKY or _rd == null:
		return
	var shape := WeatherCloudShape.shape_texture()
	if shape == null:
		return
	var shape_rd := RenderingServer.texture_get_rd_texture(shape.get_rid())
	var buffers := render_data.get_render_scene_buffers() as RenderSceneBuffersRD
	var scene := render_data.get_render_scene_data()
	if buffers == null or scene == null or not shape_rd.is_valid():
		return
	var size := buffers.get_internal_size()
	if size.x <= 0 or size.y <= 0:
		return
	var view_size := Vector2i(maxi(size.x / VIEW_DIVISOR, 1), maxi(size.y / VIEW_DIVISOR, 1))
	if view_size != _view_size:
		for i in 2:
			if _view_textures[i].is_valid():
				_rd.free_rid(_view_textures[i])
			_view_textures[i] = _create_cloud_texture(view_size)
		for i in 2:
			_set_texture(_view_uniforms[i][3], _view_textures[1 - i], _linear_clamp)
			_set_texture(_view_uniforms[i][4], _view_textures[i])
		for i in _composite_uniforms.size():
			_set_texture(_composite_uniforms[i][2], _view_textures[i & 1], _linear_clamp)
		_view_size = view_size
		_view_history_valid = false
	if shape_rd != _shape_rd:
		for i in 2:
			_set_texture(_view_uniforms[i][2], shape_rd, _linear_repeat)
			_set_texture(_panorama_uniforms[i][2], shape_rd, _linear_repeat)
		_set_texture(_shadow_uniforms[0], shape_rd, _linear_repeat)
		_shape_rd = shape_rd

	var projection := scene.get_cam_projection()
	var camera := scene.get_cam_transform()
	var rotation := Projection(Transform3D(camera.basis, Vector3.ZERO))
	var write := _frame & 1
	var inv_projection := projection.inverse()

	_pack_params(_view_bytes, inv_projection, rotation, camera.origin,
		Vector2(view_size), _view_history_valid, 0.0)
	_rd.buffer_update(_view_params, 0, _view_bytes.size(), _view_bytes)
	_pack_params(_panorama_bytes, inv_projection, rotation, camera.origin,
		Vector2(PANORAMA_TEXELS, PANORAMA_TEXELS), _panorama_history_valid, 1.0)
	_rd.buffer_update(_panorama_params, 0, _panorama_bytes.size(), _panorama_bytes)

	var list := _rd.compute_list_begin()
	_rd.compute_list_bind_compute_pipeline(list, _march_pipeline)
	_dispatch_march(list, _view_uniforms[write], view_size)
	_dispatch_march(list, _panorama_uniforms[write], Vector2i(PANORAMA_TEXELS, PANORAMA_TEXELS))
	_rd.compute_list_add_barrier(list)
	_rd.compute_list_bind_compute_pipeline(list, _composite_pipeline)
	_pack_vec4(_composite_push, 0, size.x, size.y, 0.0, 0.0)
	for view in buffers.get_view_count():
		var color_rd := buffers.get_color_layer(view)
		var depth_rd := buffers.get_depth_layer(view)
		if view >= _composite_colors.size():
			_composite_colors.append(color_rd)
			_composite_depths.append(depth_rd)
			for i in 2:
				var uniforms: Array[RDUniform] = [
					_image_uniform(0, color_rd),
					_sampler_uniform(1, _nearest_clamp, depth_rd),
					_sampler_uniform(2, _linear_clamp, _view_textures[i]),
				]
				_composite_uniforms.append(uniforms)
		if color_rd != _composite_colors[view]:
			for i in 2:
				_set_texture(_composite_uniforms[view * 2 + i][0], color_rd)
			_composite_colors[view] = color_rd
		if depth_rd != _composite_depths[view]:
			for i in 2:
				_set_texture(_composite_uniforms[view * 2 + i][1], depth_rd, _nearest_clamp)
			_composite_depths[view] = depth_rd
		var composite_set := UniformSetCacheRD.get_cache(_composite_shader, 0,
			_composite_uniforms[view * 2 + write])
		_rd.compute_list_bind_uniform_set(list, composite_set, 0)
		_rd.compute_list_set_push_constant(list, _composite_push, _composite_push.size())
		_rd.compute_list_dispatch(list, ceili(size.x / float(GROUP)), ceili(size.y / float(GROUP)), 1)
	if shadow_window.z > 0.0:
		_dispatch_shadow_bake(list)
	_rd.compute_list_end()

	# The global keeps one texture; rebinding it to the ping-pong pair every frame would
	# recreate its texture each time.
	var panorama_size := Vector3(PANORAMA_TEXELS, PANORAMA_TEXELS, 1)
	_rd.texture_copy(_panorama_textures[write], _reflection_rd, Vector3.ZERO, Vector3.ZERO,
		panorama_size, 0, 0, 0, 0)
	var view_rotation := Projection(Transform3D(camera.basis.inverse(), Vector3.ZERO))
	_previous_view_projection = projection * view_rotation
	_view_history_valid = true
	_panorama_history_valid = true
	_frame += 1

func _dispatch_shadow_bake(list: int) -> void:
	_rd.compute_list_bind_compute_pipeline(list, _shadow_pipeline)
	var uniform_set := UniformSetCacheRD.get_cache(_shadow_shader, 0, _shadow_uniforms)
	_rd.compute_list_bind_uniform_set(list, uniform_set, 0)
	_pack_vec4(_shadow_push, 0, shadow_window.x, shadow_window.y, shadow_window.z, shadow_window.w)
	_pack_vec4(_shadow_push, 16, clouds.x, clouds.y, clouds.z, clouds.w)
	_pack_vec4(_shadow_push, 32, sun.x, sun.y, sun.z, 0.0)
	_rd.compute_list_set_push_constant(list, _shadow_push, _shadow_push.size())
	_rd.compute_list_dispatch(list, SHADOW_TEXELS / GROUP, SHADOW_TEXELS / GROUP, 1)

func _dispatch_march(list: int, uniforms: Array[RDUniform], size: Vector2i) -> void:
	var uniform_set := UniformSetCacheRD.get_cache(_march_shader, 0, uniforms)
	_rd.compute_list_bind_uniform_set(list, uniform_set, 0)
	_rd.compute_list_dispatch(list, ceili(size.x / float(GROUP)), ceili(size.y / float(GROUP)), 1)

func _march_uniforms(params: RID, history: RID, output: RID) -> Array[RDUniform]:
	return [
		_buffer_uniform(0, params),
		_sampler_uniform(1, _linear_repeat, _noise_texture),
		_sampler_uniform(2, _linear_repeat, _shape_rd),
		_sampler_uniform(3, _linear_clamp, history),
		_image_uniform(4, output),
	]

# std140 layout of the march shader's Params block: three mat4, then eight vec4.
func _pack_params(data: PackedByteArray, inv_projection: Projection, rotation: Projection,
		origin: Vector3, target: Vector2, history_valid: bool, mode: float) -> void:
	_pack_matrix(data, 0, inv_projection)
	_pack_matrix(data, 64, rotation)
	_pack_matrix(data, 128, _previous_view_projection)
	_pack_vec4(data, 192, origin.x, origin.y, origin.z, float(_frame))
	_pack_vec4(data, 208, clouds.x, clouds.y, clouds.z, clouds.w)
	_pack_vec4(data, 224, sun.x, sun.y, sun.z, sun.w)
	_pack_vec4(data, 240, shade.r, shade.g, shade.b, 1.0)
	_pack_vec4(data, 256, light.r, light.g, light.b, 1.0)
	_pack_vec4(data, 272, horizon.r, horizon.g, horizon.b, 1.0)
	_pack_vec4(data, 288, target.x, target.y, 1.0 if history_valid else 0.0, mode)
	_pack_vec4(data, 304, high_clouds.x, high_clouds.y, high_clouds.z, rain)

func _pack_matrix(data: PackedByteArray, offset: int, matrix: Projection) -> void:
	for i in 4:
		var column: Vector4 = matrix[i]
		_pack_vec4(data, offset + i * 16, column.x, column.y, column.z, column.w)

func _pack_vec4(data: PackedByteArray, offset: int, x: float, y: float, z: float, w: float) -> void:
	data.encode_float(offset, x)
	data.encode_float(offset + 4, y)
	data.encode_float(offset + 8, z)
	data.encode_float(offset + 12, w)

# Only called when a texture RID changes; uniform sets stay owned by UniformSetCacheRD.
func _set_texture(uniform: RDUniform, texture: RID, sampler: RID = RID()) -> void:
	uniform.clear_ids()
	if sampler.is_valid():
		uniform.add_id(sampler)
	uniform.add_id(texture)

func _image_uniform(binding: int, texture: RID) -> RDUniform:
	var uniform := RDUniform.new()
	uniform.uniform_type = RenderingDevice.UNIFORM_TYPE_IMAGE
	uniform.binding = binding
	uniform.add_id(texture)
	return uniform

func _sampler_uniform(binding: int, sampler: RID, texture: RID) -> RDUniform:
	var uniform := RDUniform.new()
	uniform.uniform_type = RenderingDevice.UNIFORM_TYPE_SAMPLER_WITH_TEXTURE
	uniform.binding = binding
	uniform.add_id(sampler)
	uniform.add_id(texture)
	return uniform

func _buffer_uniform(binding: int, buffer: RID) -> RDUniform:
	var uniform := RDUniform.new()
	uniform.uniform_type = RenderingDevice.UNIFORM_TYPE_UNIFORM_BUFFER
	uniform.binding = binding
	uniform.add_id(buffer)
	return uniform
