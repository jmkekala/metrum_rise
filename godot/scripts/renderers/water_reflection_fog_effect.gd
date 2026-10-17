# SPDX-License-Identifier: GPL-2.0-only

## The planar water reflection pass's distance fade, as coverage rather than colour
## (water_reflection_fog.glsl). water_reflection.gd attaches one to each mirror camera with the
## environment's fog disabled, and copies the main environment's depth fog into it every frame.
class_name WaterReflectionFogEffect
extends CompositorEffect

const SHADER := preload("res://scripts/shaders/water_reflection_fog.glsl")
const GROUP := 8

## The main environment's depth fog: begin and end distance in metres, curve and density.
## A density of zero disables the pass.
var fade_begin_m := 0.0
var fade_end_m := 1.0
var fade_curve := 1.0
var fade_density := 0.0

var _rd: RenderingDevice
var _shader: RID
var _pipeline: RID
var _params: RID
var _nearest: RID
var _params_data := PackedByteArray()
var _uniforms: Array[RDUniform] = []
var _color: RID
var _depth: RID

func _init() -> void:
	effect_callback_type = EFFECT_CALLBACK_TYPE_POST_TRANSPARENT
	# The mirror uses MSAA; the compute pass reads/writes its resolved single-sample buffers.
	access_resolved_color = true
	access_resolved_depth = true
	RenderingServer.call_on_render_thread(_initialize)

func _notification(what: int) -> void:
	if what != NOTIFICATION_PREDELETE or _rd == null:
		return
	var rids := [_params, _nearest, _shader]
	if RenderingServer.is_on_render_thread():
		_free_rids(_rd, rids)
	else:
		RenderingServer.call_on_render_thread(_free_rids.bind(_rd, rids))

# Static, so a free queued from a destroyed effect does not call into it.
static func _free_rids(rd: RenderingDevice, rids: Array) -> void:
	for rid: RID in rids:
		if rid.is_valid():
			rd.free_rid(rid)

func _initialize() -> void:
	_rd = RenderingServer.get_rendering_device()
	if _rd == null:
		return
	_shader = _rd.shader_create_from_spirv(SHADER.get_spirv())
	_pipeline = _rd.compute_pipeline_create(_shader)
	_params_data.resize(96)
	_params = _rd.uniform_buffer_create(_params_data.size())
	var state := RDSamplerState.new()
	_nearest = _rd.sampler_create(state)

func _render_callback(callback_type: int, render_data: RenderData) -> void:
	if callback_type != EFFECT_CALLBACK_TYPE_POST_TRANSPARENT or _rd == null or fade_density <= 0.0:
		return
	var buffers := render_data.get_render_scene_buffers() as RenderSceneBuffersRD
	var scene := render_data.get_render_scene_data()
	if buffers == null or scene == null:
		return
	var size := buffers.get_internal_size()
	if size.x <= 0 or size.y <= 0:
		return
	var color := buffers.get_color_layer(0)
	var depth := buffers.get_depth_layer(0)
	if _uniforms.is_empty():
		_uniforms = [_uniform(RenderingDevice.UNIFORM_TYPE_IMAGE, 0, [color]),
			_uniform(RenderingDevice.UNIFORM_TYPE_SAMPLER_WITH_TEXTURE, 1, [_nearest, depth]),
			_uniform(RenderingDevice.UNIFORM_TYPE_UNIFORM_BUFFER, 2, [_params])]
	elif color != _color or depth != _depth:
		_uniforms[0] = _uniform(RenderingDevice.UNIFORM_TYPE_IMAGE, 0, [color])
		_uniforms[1] = _uniform(RenderingDevice.UNIFORM_TYPE_SAMPLER_WITH_TEXTURE, 1, [_nearest, depth])
	_color = color
	_depth = depth
	# std140: the inverse projection, then two vec4.
	var inv_projection := scene.get_cam_projection().inverse()
	for i in 4:
		var column: Vector4 = inv_projection[i]
		_pack_vec4(i * 16, column.x, column.y, column.z, column.w)
	_pack_vec4(64, fade_begin_m, fade_end_m, fade_curve, fade_density)
	_pack_vec4(80, size.x, size.y, 0.0, 0.0)
	_rd.buffer_update(_params, 0, _params_data.size(), _params_data)
	var list := _rd.compute_list_begin()
	_rd.compute_list_bind_compute_pipeline(list, _pipeline)
	_rd.compute_list_bind_uniform_set(list, UniformSetCacheRD.get_cache(_shader, 0, _uniforms), 0)
	_rd.compute_list_dispatch(list, ceili(size.x / float(GROUP)), ceili(size.y / float(GROUP)), 1)
	_rd.compute_list_end()

func _pack_vec4(offset: int, x: float, y: float, z: float, w: float) -> void:
	_params_data.encode_float(offset, x)
	_params_data.encode_float(offset + 4, y)
	_params_data.encode_float(offset + 8, z)
	_params_data.encode_float(offset + 12, w)

func _uniform(type: int, binding: int, ids: Array) -> RDUniform:
	var uniform := RDUniform.new()
	uniform.uniform_type = type
	uniform.binding = binding
	for id in ids:
		uniform.add_id(id)
	return uniform
