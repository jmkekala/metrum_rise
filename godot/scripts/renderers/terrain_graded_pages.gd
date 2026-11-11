# SPDX-License-Identifier: GPL-2.0-only

## Graded terrain pages (TERRAIN-07-G, docs/terrain.md "Pages and the row atlas"): the page
## textures and the row atlas uploads of terrain_graded_tiles.gd. A page is two Texture2DArrays,
## its slot layers (one per tile copy: the lattice and a header line) and its row atlas (a run of
## whole 256-texel layers per copy's rows). The allocator is Rust's (GradedTileBook, `RENDER-32-I`):
## it creates pages within the graded page budget, pools freed pages by level and size and hands
## out a page's slots and first-fit atlas runs; this script makes, keeps and drops each page's
## arrays as the book's page events say, holds each level's layer shapes and pooled upload Images,
## and uploads a copy. Which page a block of patches uses, the tiles' copies, a block's growth and
## the materials' binding are terrain_graded_tiles.gd's. Nothing here allocates per frame: pages
## are pooled, and every upload goes through the level's pooled Images. On Compatibility each page
## also keeps its uploaded layers' bytes, which a growth copies (TERRAIN-22).
extends RefCounted

# Texels of a row atlas layer (residency.rs ATLAS_WIDTH, checked against the layout at setup).
const ATLAS_WIDTH := 256
const ATLAS_LAYER_BYTES := ATLAS_WIDTH * 16
# The layer limit without a RenderingDevice (Compatibility, headless): what Vulkan and GLES3 both
# guarantee. Never above 2,048: a slot reaches the shader in a half-float instance channel.
const MIN_LAYER_LIMIT := 256
const MAX_LAYER_LIMIT := 2048
# No copy in a page: Vector3i(slot, first atlas layer, layers) with no slot.
const NO_COPY := Vector3i(-1, 0, 0)

# Godot's ImageTextureLayered arrays lack CAN_COPY_TO, so their growth cannot preserve GPU
# contents without a readback. These arrays keep the same TextureLayered shader interface and
# add that usage bit. All device work stays on the rendering thread, in submission order.
# Queued calls retain their owner until execution, including when a world resets meanwhile.
class CopyableArray extends Texture2DArrayRD:
	func create(width: int, height: int, count: int) -> void:
		RenderingServer.call_on_render_thread(_create.bind(width, height, count, self))

	func _create(width: int, height: int, count: int, _owner: RefCounted) -> void:
		var device := RenderingServer.get_rendering_device()
		var format := RDTextureFormat.new()
		format.texture_type = RenderingDevice.TEXTURE_TYPE_2D_ARRAY
		format.format = RenderingDevice.DATA_FORMAT_R32G32B32A32_SFLOAT
		format.width = width
		format.height = height
		format.array_layers = count
		format.usage_bits = (RenderingDevice.TEXTURE_USAGE_SAMPLING_BIT
			| RenderingDevice.TEXTURE_USAGE_CAN_UPDATE_BIT
			| RenderingDevice.TEXTURE_USAGE_CAN_COPY_FROM_BIT
			| RenderingDevice.TEXTURE_USAGE_CAN_COPY_TO_BIT)
		var handle := device.texture_create(format, RDTextureView.new())
		device.texture_clear(handle, Color(0, 0, 0, 0), 0, 1, 0, count)
		texture_rd_rid = handle

	func update_layer(image: Image, layer: int) -> void:
		# Packed bytes are copy-on-write. Queue their snapshot, never the mutable pooled Image.
		RenderingServer.call_on_render_thread(_update.bind(image.get_data(), layer, self))

	func _update(data: PackedByteArray, layer: int, _owner: RefCounted) -> void:
		RenderingServer.get_rendering_device().texture_update(texture_rd_rid, layer, data)

	func copy_from(source: TextureLayered) -> void:
		RenderingServer.call_on_render_thread(_copy.bind(source, self))

	func _copy(source: TextureLayered, _owner: RefCounted) -> void:
		var device := RenderingServer.get_rendering_device()
		var from := RenderingServer.texture_get_rd_texture(source.get_rid())
		var size := Vector3(source.get_width(), source.get_height(), 1)
		for layer in source.get_layers():
			device.texture_copy(from, texture_rd_rid, Vector3.ZERO, Vector3.ZERO, size, 0, 0, layer, layer)

	func _notification(what: int) -> void:
		if what == NOTIFICATION_PREDELETE and texture_rd_rid.is_valid():
			# TextureLayeredRD owns its RenderingServer wrapper, not the device texture.
			RenderingServer.call_on_render_thread(_free.bind(texture_rd_rid))

	static func _free(handle: RID) -> void:
		RenderingServer.get_rendering_device().free_rid(handle)

static func _array(zero: Image, count: int) -> TextureLayered:
	# Texture2DArrayRD requires at least two layers; one-layer test pools use the regular array.
	if count > 1 and RenderingServer.get_rendering_device() != null:
		var texture := CopyableArray.new()
		texture.create(zero.get_width(), zero.get_height(), count)
		return texture
	var images: Array[Image] = []
	images.resize(count)
	images.fill(zero)
	var texture := Texture2DArray.new()
	texture.create_from_images(images)
	return texture

static func _copy_array(source: TextureLayered, target: TextureLayered) -> void:
	if target is CopyableArray:
		target.copy_from(source)
	elif RenderingServer.get_rendering_device() != null:
		# A one-layer page on Forward+/Mobile: the RenderingDevice reads it back exactly. The
		# dummy renderer (headless) stores no texels, and Compatibility copies its CPU copy.
		for layer in source.get_layers():
			target.update_layer(source.get_layer_data(layer), layer)

# Compatibility (TERRAIN-22): uploads the source page's CPU copy of every layer written since it
# was created or pooled into the same layers of `target` (a page just created or taken from the
# pool, holding no copy), which shares those bytes. Godot 4.7.2's GLES3 `get_layer_data` returns
# an RGBAF layer as layer 0, clamped to 0-1 at 8 bits, so nothing is read back. O(written bytes).
func _copy_written(source: Dictionary, target: Dictionary, key: String, image: Image) -> void:
	var from: Array = source[key]
	var into: Array = target[key]
	var texture: TextureLayered = target["texels" if key == "texel_bytes" else "atlas"]
	for layer in from.size():
		var data: PackedByteArray = from[layer]
		if data.is_empty():
			continue
		# set_data shares the bytes: the next write_copy replaces or copies them, never edits them.
		image.set_data(image.get_width(), image.get_height(), false, Image.FORMAT_RGBAF, data)
		texture.update_layer(image, layer)
		_keep_written(into, layer, data)

# Records `data` as the CPU copy of layer `layer` of a page array (Compatibility only).
func _keep_written(written: Array, layer: int, data: PackedByteArray) -> void:
	mirror_bytes += data.size() - (written[layer] as PackedByteArray).size()
	written[layer] = data

# Drops a page's CPU copies (Compatibility only): it holds no copy any more. O(slots + layers).
func _drop_written(page: Dictionary) -> void:
	if not mirrored:
		return
	for key: String in ["texel_bytes", "atlas_bytes"]:
		for data: PackedByteArray in page[key]:
			mirror_bytes -= data.size()
		(page[key] as Array).fill(PackedByteArray())

# A block moved (the book's effect): page `source_id`'s published contents are copied into page
# `target_id` before the block's materials rebind (the book laid the copies out). No rebake is
# involved: edits, unlisted commit tiles and previews all keep their exact bytes. O(source slots +
# source atlas layers), with device-to-device copies on Forward+/Mobile and the CPU copy's written
# layers on Compatibility.
func copied(source_id: int, target_id: int) -> void:
	if trace != null:
		trace.out("page_copy %d %d" % [source_id, target_id])
	var source: Dictionary = by_id[source_id]
	var target: Dictionary = by_id[target_id]
	if mirrored:
		var level: Dictionary = levels[int(source["code"])]
		_copy_written(source, target, "texel_bytes", level["texel_image"])
		_copy_written(source, target, "atlas_bytes", level["atlas_image"])
	else:
		_copy_array(source["texels"], target["texels"])
		_copy_array(source["atlas"], target["atlas"])

# The renderer's counters (the same Dictionary): textures, images and pages created, uploads.
var counters: Dictionary = {}
# The renderer's record and replay (terrain_graded_trace.gd), null when off: page creations,
# releases, moves and slot uploads are its outputs.
var trace: RefCounted = null
# Per level code: the slot layer's side, the pooled upload Images, the zero layers new pages start
# from, the most slots a page of the level holds (`cap`) and the most atlas layers a tile's rows
# take (`max_layers`).
var levels: Array[Dictionary] = []
# The graded state in Rust (GradedTileBook, the renderer's): page ids, sizes, the pool, the
# budget, each page's free slots and atlas runs, and which copies each holds.
var book := GradedTileBook.new()
# Page id -> {id, code, block, texels, atlas (Texture2DArray), slots, layers, bytes, and on
# Compatibility texel_bytes and atlas_bytes (below)}: the book's live pages, the pooled ones
# included.
var by_id: Dictionary = {}
# Compatibility only (no RenderingDevice, a real renderer; TERRAIN-22): each page also keeps
# `texel_bytes` and `atlas_bytes`, per slot and atlas layer the bytes last uploaded there since
# the page was created or pooled (empty: never written, still zero), which a move copies instead
# of reading the page back. `mirror_bytes` is their total: the written layers of the pages in use,
# at most their page bytes; a pooled free page keeps none.
var mirrored := RenderingServer.get_rendering_device() == null and DisplayServer.get_name() != "headless"
var mirror_bytes := 0

# The device's array layer limit (docs/terrain.md "Layer limit"): the RenderingDevice's, else
# what Vulkan and GLES3 both guarantee, never above the half-float slot channel's 2,048.
static func device_layer_limit() -> int:
	var device := RenderingServer.get_rendering_device()
	if device == null:
		return MIN_LAYER_LIMIT
	return clampi(int(device.limit_get(RenderingDevice.LIMIT_MAX_TEXTURE_ARRAY_LAYERS)), MIN_LAYER_LIMIT, MAX_LAYER_LIMIT)

# Adds the next level's layer shapes: the slot layer (side x (side + 1), its last line the
# header), the pooled upload Images and the zero layers new pages start from, `cap` the most
# slots a page holds, and the most atlas layers a tile's rows take.
func add_level(level: Dictionary, cap: int) -> void:
	var side := int(level["side"])
	counters["images_created"] += 5
	levels.append({
		"side": side,
		"cap": cap,
		"max_layers": int(ceil(float(level["max_row_texels"]) / ATLAS_WIDTH)),
		"texel_image": Image.create_empty(side, side + 1, false, Image.FORMAT_RGBAF),
		"atlas_image": Image.create_empty(ATLAS_WIDTH, 1, false, Image.FORMAT_RGBAF),
		"run_image": Image.create_empty(ATLAS_WIDTH, 1, false, Image.FORMAT_RGBAF),
		"zero_texels": Image.create_empty(side, side + 1, false, Image.FORMAT_RGBAF),
		"zero_atlas": Image.create_empty(ATLAS_WIDTH, 1, false, Image.FORMAT_RGBAF),
	})

# Drops every level and page, as the renderer sets up (the book makes its pages again).
func clear_levels() -> void:
	levels.clear()
	by_id.clear()
	mirror_bytes = 0

# A page the book made (`reused`: from its pool, the arrays kept) for `block` at level `code`.
func made(page_id: int, reused: bool, code: int, block: Vector2i, slots: int, layers: int) -> void:
	if reused:
		if trace != null:
			trace.out("page_reuse %d %d %d,%d %d %d" % [page_id, code, block.x, block.y, slots, layers])
		by_id[page_id]["block"] = block
		return
	if trace != null:
		trace.out("page_new %d %d %d,%d %d %d" % [page_id, code, block.x, block.y, slots, layers])
	var level: Dictionary = levels[code]
	var side := int(level["side"])
	var texels := _array(level["zero_texels"], slots)
	var atlas := _array(level["zero_atlas"], layers)
	counters["textures_created"] += 2
	counters["pages_created"] += 1
	var page := {"id": page_id, "code": code, "block": block, "slots": slots, "layers": layers, "texels": texels,
		"atlas": atlas, "bytes": slots * side * (side + 1) * 16 + layers * ATLAS_LAYER_BYTES}
	if mirrored:
		var texel_bytes := []
		texel_bytes.resize(slots)
		texel_bytes.fill(PackedByteArray())
		var atlas_bytes := []
		atlas_bytes.resize(layers)
		atlas_bytes.fill(PackedByteArray())
		page["texel_bytes"] = texel_bytes
		page["atlas_bytes"] = atlas_bytes
	by_id[page_id] = page

# A page holding no copy was released: pooled (`kept`, its CPU copies dropped) or freed.
func released(page_id: int, kept: bool) -> void:
	if trace != null:
		trace.out("page_release %d %d" % [page_id, int(kept)])
	_drop_written(by_id[page_id])
	if not kept:
		by_id.erase(page_id)

# A pooled page was freed to make room in the budget.
func dropped(page_id: int) -> void:
	if trace != null:
		trace.out("page_drop %d" % page_id)
	by_id.erase(page_id)

# The budget refused a page (the book records its tile exhausted).
func refused(code: int, block: Vector2i, slots: int, layers: int) -> void:
	if trace != null:
		trace.out("page_refused %d %d,%d %d %d" % [code, block.x, block.y, slots, layers])

# Writes a tile's slot layer, its header's first channel set to its run's first atlas layer
# (residency.rs `pack_slot`), and its run into its copy in `page`. O(slot texels + rows).
func write_copy(page: Dictionary, copy: Vector3i, tile: Dictionary) -> void:
	var level: Dictionary = levels[int(page["code"])]
	if trace != null:
		trace.out("write %d %d %d %d %d %d %d %d %d" % [int(page["id"]), copy.x, copy.y, copy.z,
			int(tile["tile_x"]), int(tile["tile_z"]), int(tile["level"]), int(tile["step"]), int(tile["token"])])
	var upload_start_us := Time.get_ticks_usec()
	var side := int(level["side"])
	var image: Image = level["texel_image"]
	image.set_data(side, side + 1, false, Image.FORMAT_RGBAF, tile["texels"])
	var header := image.get_pixel(0, side)
	header.r = float(copy.y)
	image.set_pixel(0, side, header)
	page["texels"].update_layer(image, copy.x)
	if mirrored:
		_keep_written(page["texel_bytes"], copy.x, image.get_data())
	var rows: PackedByteArray = tile["rows"]
	var count := rows.size() / ATLAS_LAYER_BYTES
	if count > 0:
		var run: Image = level["run_image"]
		run.set_data(ATLAS_WIDTH, count, false, Image.FORMAT_RGBAF, rows)
		var layer: Image = level["atlas_image"]
		for index in count:
			layer.blit_rect(run, Rect2i(0, index, ATLAS_WIDTH, 1), Vector2i.ZERO)
			page["atlas"].update_layer(layer, copy.y + index)
			if mirrored:
				# The next blit copies these shared bytes before writing (copy-on-write).
				_keep_written(page["atlas_bytes"], copy.y + index, layer.get_data())
	counters["upload_ms_max"] = max(float(counters["upload_ms_max"]), float(Time.get_ticks_usec() - upload_start_us) / 1000.0)
	counters["uploads"] += 1
	counters["upload_bytes"] += (tile["texels"] as PackedByteArray).size() + rows.size()

# The bytes layer `layer` of `page`'s slot layers (`texels`) or row atlas holds, for the tests'
# byte checks: on Compatibility its CPU copy (an unwritten layer is zero), where the readback
# fails (TERRAIN-22), else the device's readback. Not for the frame path: it reads back.
func layer_data(page: Dictionary, texels: bool, layer: int) -> PackedByteArray:
	if not mirrored:
		return (page["texels" if texels else "atlas"] as TextureLayered).get_layer_data(layer).get_data()
	var data: PackedByteArray = page["texel_bytes" if texels else "atlas_bytes"][layer]
	if data.is_empty():
		return (levels[int(page["code"])]["zero_texels" if texels else "zero_atlas"] as Image).get_data()
	return data
