# SPDX-License-Identifier: GPL-2.0-only

## Bakes the gust lattice once and publishes it as the `weather_gust_map` shader global, which
## weather_gust() reads for every tree vertex, impostor and water pixel.
##
## The lattice is fixed, so one pass at startup replaces four integer hashes per noise layer
## with one filtered fetch: in a storm every tree vertex reads two layers, and hashing them cost
## about 0.8 ms in the town view.
extends Node

const BAKE_SHADER := preload("res://scripts/shaders/weather_gust_bake.gdshader")
# Must match WEATHER_GUST_CELLS in weather_wind.gdshaderinc.
const TEXELS := 256

static var _gust_cache: ImageTexture

func _ready() -> void:
	if _gust_cache != null:
		RenderingServer.global_shader_parameter_set("weather_gust_map", _gust_cache)
		return
	_bake.call_deferred()

func _bake() -> void:
	var viewport := SubViewport.new()
	viewport.name = "GustMapBake"
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
	image.convert(Image.FORMAT_R8)
	_gust_cache = ImageTexture.create_from_image(image)
	RenderingServer.global_shader_parameter_set("weather_gust_map", _gust_cache)
