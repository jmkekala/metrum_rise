# SPDX-License-Identifier: GPL-2.0-only

## Bakes all stars and the Milky Way once in the celestial frame for water reflections.
## The cached RGB9E5 octahedral map keeps linear HDR radiance and a full mip chain; turning
## the sky only changes the lookup rotation. The temporary viewport is freed after readback.
extends Node

const BAKE_SHADER := preload("res://scripts/shaders/night_sky_bake.gdshader")
# Must match NIGHT_SKY_TEXELS in night_sky.gdshaderinc.
const TEXELS := 2048

static var _sky_cache: ImageTexture

func _ready() -> void:
	if _sky_cache != null:
		RenderingServer.global_shader_parameter_set("night_sky_map", _sky_cache)
		return
	_bake.call_deferred()

func _bake() -> void:
	var viewport := SubViewport.new()
	viewport.name = "NightSkyBake"
	viewport.disable_3d = true
	viewport.use_hdr_2d = true
	# Keep the readback RGBAH rather than converting the target to RGBH.
	viewport.transparent_bg = true
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
	# RD reads the half-float target without a transfer conversion. Stored as shared-exponent
	# RGB9E5, which keeps the linear HDR range at half RGBAH's size (21 MiB with mips, against
	# 43): the stars need the full 2048 texels to stay points, and the alpha was unused. Its mip
	# averages are linear too (Image::average_4_rgbe9995).
	image.convert(Image.FORMAT_RGBAH)
	image.convert(Image.FORMAT_RGBE9995)
	image.generate_mipmaps()
	_sky_cache = ImageTexture.create_from_image(image)
	RenderingServer.global_shader_parameter_set("night_sky_map", _sky_cache)
