# SPDX-License-Identifier: GPL-2.0-only

## Bakes the tiling cloud shape once and publishes it as the `weather_cloud_shape_map` shader
## global, which the sky and every cloud-shadow receiver sample.
##
## The shape depends on nothing but the fixed noise field, so one GPU pass at startup replaces
## 7 value noises and 3 Worley evaluations per lookup. The sky samples it at distances where one
## pixel spans many texels, so the bake is read back once to build a mip chain; a viewport
## texture has none and would alias into streaks near the horizon.
extends Node

const BAKE_SHADER := preload("res://scripts/shaders/weather_cloud_shape_bake.gdshader")
# One texel is 8 m of an 8192 m tile, finer than the smallest 128 m Worley lobe.
const TEXELS := 1024

static var _shape_cache: ImageTexture

func _ready() -> void:
	if _shape_cache != null:
		RenderingServer.global_shader_parameter_set("weather_cloud_shape_map", _shape_cache)
		return
	_bake.call_deferred()

func _bake() -> void:
	var viewport := SubViewport.new()
	viewport.name = "CloudShapeBake"
	viewport.disable_3d = true
	viewport.size = Vector2i(TEXELS, TEXELS)
	viewport.render_target_update_mode = SubViewport.UPDATE_ONCE
	var rect := ColorRect.new()
	rect.size = Vector2(TEXELS, TEXELS)
	var material := ShaderMaterial.new()
	material.shader = BAKE_SHADER
	rect.material = material
	viewport.add_child(rect)
	add_child(viewport)
	await RenderingServer.frame_post_draw
	var image := viewport.get_texture().get_image()
	viewport.queue_free()
	image.convert(Image.FORMAT_RGBA8)
	image.generate_mipmaps()
	_shape_cache = ImageTexture.create_from_image(image)
	RenderingServer.global_shader_parameter_set("weather_cloud_shape_map", _shape_cache)

## The baked shape, or null until the first bake has finished.
static func shape_texture() -> ImageTexture:
	return _shape_cache
