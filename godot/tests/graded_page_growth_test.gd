# SPDX-License-Identifier: GPL-2.0-only

## A graded page growth keeps every moved copy's exact bytes on the GPU (TERRAIN-22). Installed and
## preview tiles of distinct floats (negative, above 1, per texel) fill a GL2 block's page; the
## block moves to a larger page without a rebake or rewrite, one tile is edited there, and the
## block moves again. After each move the page's layers are drawn into an RGBA8 SubViewport, each
## channel's float bits as four bytes, and compared with the bytes uploaded: no `get_layer_data`,
## which on Compatibility (Godot 4.7.2) reads an RGBAF layer back as layer 0 clamped to 0-1 at 8
## bits. On Compatibility the pages' CPU copy must hold the same bytes. Rendered only: the gate
## runs it under Compatibility.
extends SceneTree

const Tiles := preload("res://scripts/renderers/terrain_graded_tiles.gd")
const Pages := preload("res://scripts/renderers/terrain_graded_pages.gd")
# One output pixel per texel channel: its float's bits as four bytes, low byte in red, so an
# output row's RGBA8 bytes are the layer row's RGBAF bytes. Blending off: alpha is data.
const DECODE := """
shader_type canvas_item;
render_mode blend_disabled, unshaded;
uniform sampler2DArray layers : filter_nearest;
uniform int first_layer;
uniform ivec2 texels;
uniform int count;
void fragment() {
	ivec2 pixel = ivec2(floor(UV * vec2(float(texels.x * 4), float(texels.y * count))));
	vec4 value = texelFetch(layers, ivec3(pixel.x / 4, pixel.y % texels.y, first_layer + pixel.y / texels.y), 0);
	int channel = pixel.x % 4;
	float picked = channel == 0 ? value.r : (channel == 1 ? value.g : (channel == 2 ? value.b : value.a));
	uint bits = floatBitsToUint(picked);
	COLOR = vec4(float(bits & 255u), float((bits >> 8u) & 255u), float((bits >> 16u) & 255u), float(bits >> 24u)) / 255.0;
}
"""
var failures := 0
var viewport: SubViewport
var rect: ColorRect

func _initialize() -> void:
	call_deferred("_run")

func expect(condition: bool, message: String) -> void:
	if not condition:
		failures += 1
		push_error(message)

# A GL2 tile whose texels and `layers` atlas layers differ per texel and per `token`.
func payload(graded: Tiles, key: Vector2i, token: int, layers: int) -> Dictionary:
	var side := int(graded.page_store.levels[2]["side"])
	var texels := Image.create(side, side + 1, false, Image.FORMAT_RGBAF)
	for y in side + 1:
		for x in side:
			texels.set_pixel(x, y, Color(token * 1000.0 + x * 0.5, -1.0 - y * 0.37, 2.5 + token, 100000.0 + x + y))
	texels.set_pixel(0, side, Color(0, 2, 3, 4))
	var atlas := Image.create(Pages.ATLAS_WIDTH, layers, false, Image.FORMAT_RGBAF)
	for y in layers:
		for x in Pages.ATLAS_WIDTH:
			atlas.set_pixel(x, y, Color(token + x / 256.0, -token - y, 7.5, x * 3.0 + y))
	return {"tile_x": key.x, "tile_z": key.y, "level": 2, "step": 0, "token": token,
		"texels": texels.get_data(), "rows": atlas.get_data(), "walls": PackedByteArray(), "behind": false}

# The bytes layers [first, first + count) of `array` draw, each `size` texels.
func draw(array: TextureLayered, size: Vector2i, first: int, count: int) -> PackedByteArray:
	var pixels := Vector2i(size.x * 4, size.y * count)
	viewport.size = pixels
	rect.size = Vector2(pixels)
	var material: ShaderMaterial = rect.material
	material.set_shader_parameter("layers", array)
	material.set_shader_parameter("first_layer", first)
	material.set_shader_parameter("texels", size)
	material.set_shader_parameter("count", count)
	await RenderingServer.frame_post_draw
	await RenderingServer.frame_post_draw
	# A viewport is at least two pixels high: a one-row atlas layer is cropped.
	var image := viewport.get_texture().get_image().get_region(Rect2i(Vector2i.ZERO, pixels))
	image.convert(Image.FORMAT_RGBA8)
	return image.get_data()

# Every copy in page `page_id` of `tiles` (key -> payload) draws its payload's bytes, the header's
# first channel its run's first layer; on Compatibility the page's CPU copy holds them too.
func check_page(graded: Tiles, page_id: int, tiles: Dictionary, label: String) -> void:
	var page: Dictionary = graded.page_store.by_id[page_id]
	var side := int(graded.page_store.levels[2]["side"])
	for key: Vector2i in tiles:
		var entry: Dictionary = graded.preview[key] if graded.preview.has(key) else graded.installed[key]
		var copy: Vector3i = entry["copies"].get(page_id, Pages.NO_COPY)
		expect(copy.x >= 0, "%s: tile %s has a copy in the page" % [label, key])
		if copy.x < 0:
			continue
		var data: Dictionary = tiles[key]
		var texels := Image.create_from_data(side, side + 1, false, Image.FORMAT_RGBAF, data["texels"])
		var header := texels.get_pixel(0, side)
		header.r = float(copy.y)
		texels.set_pixel(0, side, header)
		var layers := (data["rows"] as PackedByteArray).size() / Pages.ATLAS_LAYER_BYTES
		var drawn_texels: PackedByteArray = await draw(page["texels"], Vector2i(side, side + 1), copy.x, 1)
		var drawn_rows: PackedByteArray = await draw(page["atlas"], Vector2i(Pages.ATLAS_WIDTH, 1), copy.y, layers)
		expect(drawn_texels == texels.get_data(), "%s: tile %s draws its exact slot bytes" % [label, key])
		expect(drawn_rows == data["rows"], "%s: tile %s draws its exact atlas bytes" % [label, key])
		check_cpu_copy(graded, page, copy, texels.get_data(), data["rows"], "%s: tile %s" % [label, key])

func check_cpu_copy(graded: Tiles, page: Dictionary, copy: Vector3i, texels: PackedByteArray, rows: PackedByteArray, label: String) -> void:
	if not graded.page_store.mirrored:
		return
	expect(graded.page_store.layer_data(page, true, copy.x) == texels, "%s: the CPU copy holds the slot bytes" % label)
	var kept := PackedByteArray()
	for layer in rows.size() / Pages.ATLAS_LAYER_BYTES:
		kept.append_array(graded.page_store.layer_data(page, false, copy.y + layer))
	expect(kept == rows, "%s: the CPU copy holds the atlas bytes" % label)

# Moves the block to a page twice the size and switches to it: no rebake, no rewrite.
func grow(graded: Tiles, label: String) -> int:
	var old_id := int(graded._book.block_page(2, Vector2i.ZERO))
	var old: Dictionary = graded.page_store.by_id[old_id]
	var uploads := int(graded.counters["uploads"])
	expect(graded._move_block(2, Vector2i.ZERO, Vector2i(int(old["slots"]) * 2, int(old["layers"]) * 2)), "%s: the block moves" % label)
	graded._advance_growth()
	var page_id := int(graded._book.block_page(2, Vector2i.ZERO))
	expect(page_id != old_id and graded._book.block_target(2, Vector2i.ZERO) < 0, "%s: the block binds its larger page" % label)
	expect(int(graded.counters["uploads"]) == uploads, "%s: the move uploads no tile" % label)
	# A freed page leaves the book's texture map (AUDIT-stream 13); a pooled one stays in both.
	expect(int(graded._book.page_texture_count()) == graded.page_store.by_id.size(),
		"%s: the book holds the textures of the live and pooled pages only" % label)
	return page_id

func _run() -> void:
	expect(DisplayServer.get_name() != "headless", "the growth test needs a renderer")
	viewport = SubViewport.new()
	viewport.transparent_bg = true
	viewport.disable_3d = true
	viewport.render_target_update_mode = SubViewport.UPDATE_ALWAYS
	rect = ColorRect.new()
	var shader := Shader.new()
	shader.code = DECODE
	var material := ShaderMaterial.new()
	material.shader = shader
	rect.material = material
	viewport.add_child(rect)
	root.add_child(viewport)
	var sim := SimulationNode.new()
	root.add_child(sim)
	expect(sim.create_blank_world(500, 500, 10, 512, 0), "fixture world")
	sim.set_simulation_speed(0)
	var graded := Tiles.new()
	graded.setup(sim, 10, Vector2(500, 500), 512, [8, 8, 8, 8])
	graded._resident = {Vector2i.ZERO: {"rect": Rect2(-250, -250, 512, 512), "step": 1, "generation": 0}}
	graded._book.clear_resident()
	graded._book_patch(Vector2i.ZERO, graded._resident[Vector2i.ZERO])
	graded._resident_synced = true
	graded.patch_lists[Vector2i.ZERO] = {"tiles": [], "token": 0}
	graded._book.take_list(Vector2i.ZERO, 0, PackedInt32Array())
	var tiles := {}
	for index in 3:
		var key := Vector2i(index, 0)
		tiles[key] = payload(graded, key, index + 1, index + 2)
		graded._upload(key, tiles[key], graded._resident)
	var shown := Vector2i(3, 0)
	tiles[shown] = payload(graded, shown, 9, 3)
	graded._show_preview_tile(tiles[shown])
	await process_frame
	var first_page := int(graded._book.block_page(2, Vector2i.ZERO))
	await check_page(graded, first_page, tiles, "before the moves")
	var moved := grow(graded, "first move")
	await check_page(graded, moved, tiles, "first move")
	# An edit into the moved page (its run longer: a new run), then a second move from it.
	var edited := Vector2i(1, 0)
	tiles[edited] = payload(graded, edited, 21, 5)
	graded._upload(edited, tiles[edited], graded._resident)
	var grown := grow(graded, "second move")
	await check_page(graded, grown, tiles, "second move")
	if graded.page_store.mirrored:
		# Only the bound page holds copies; the released ones keep no CPU copy. Its written
		# layers: four slot layers and every run written into it since it was created.
		var page: Dictionary = graded.page_store.by_id[grown]
		var kept := 0
		for layer in int(page["slots"]):
			kept += graded._book.page_layer(grown, true, layer).size()
		for layer in int(page["layers"]):
			kept += graded._book.page_layer(grown, false, layer).size()
		expect(graded.page_store.mirror_bytes == kept, "the CPU copy is the bound page's written layers: %d of %d" % [graded.page_store.mirror_bytes, kept])
		expect(kept <= int(page["bytes"]), "the CPU copy is at most the page's bytes")
	graded.dispose()
	sim.free()
	print("Graded page growth tests (%s): %d failures" % [RenderingServer.get_current_rendering_method(), failures])
	quit(0 if failures == 0 else 1)
