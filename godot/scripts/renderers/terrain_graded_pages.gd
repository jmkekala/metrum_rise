# SPDX-License-Identifier: GPL-2.0-only

## Graded terrain pages (TERRAIN-07-G, docs/terrain.md "Pages and the row atlas"): the page
## textures of terrain_graded_tiles.gd. A page is two Texture2DArrays, its slot layers (one per
## tile copy: the lattice and a header line) and its row atlas (a run of whole 256-texel layers per
## copy's rows). The allocator is Rust's (GradedTileBook, `RENDER-32-I`): it creates pages within
## the graded page budget, pools freed pages by level and size and hands out a page's slots and
## first-fit atlas runs; this script makes, keeps and drops each page's arrays as the book's page
## events say and holds each level's layer shapes. What the arrays hold is the book's too
## (`RENDER-32-K` part 2): it writes each copy from its bake and copies a moved block's page
## through the stream backend, by the arrays' RIDs, and on Compatibility keeps the written
## layers' bytes, which a growth copies (TERRAIN-22). Which page a block of patches uses, the
## tiles' copies, a block's growth and the materials' binding are terrain_graded_tiles.gd's.
## Nothing here allocates per frame: pages are pooled.
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

# A block moved (the book's effect): page `source_id`'s published contents are copied into page
# `target_id` before the block's materials rebind (the book laid the copies out). No rebake is
# involved: edits, unlisted commit tiles and previews all keep their exact bytes. The book copies
# (GradedTileBook.copy_page): O(source slots + source atlas layers), with device-to-device copies
# on Forward+/Mobile and the CPU copy's written layers on Compatibility.
func copied(source_id: int, target_id: int) -> void:
	if trace != null:
		trace.out("page_copy %d %d" % [source_id, target_id])
	book.copy_page(source_id, target_id)

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
# Page id -> {id, code, block, texels, atlas (Texture2DArray), slots, layers, bytes}: the book's
# live pages, the pooled ones included.
var by_id: Dictionary = {}
# Compatibility only (no RenderingDevice, a real renderer; TERRAIN-22): the book's backend also
# keeps, per slot and atlas layer of each page, the bytes last uploaded there since the page was
# created or pooled (none: never written, still zero), which a move copies instead of reading the
# page back (the same rule as the backend's). `mirror_bytes` is their total: the written layers of
# the pages in use, at most their page bytes; a pooled free page keeps none.
var mirrored := RenderingServer.get_rendering_device() == null and DisplayServer.get_name() != "headless"
var mirror_bytes: int:
	get:
		return book.page_mirror_bytes()

# The device's array layer limit (docs/terrain.md "Layer limit"): the RenderingDevice's, else
# what Vulkan and GLES3 both guarantee, never above the half-float slot channel's 2,048.
static func device_layer_limit() -> int:
	var device := RenderingServer.get_rendering_device()
	if device == null:
		return MIN_LAYER_LIMIT
	return clampi(int(device.limit_get(RenderingDevice.LIMIT_MAX_TEXTURE_ARRAY_LAYERS)), MIN_LAYER_LIMIT, MAX_LAYER_LIMIT)

# Adds the next level's layer shapes: the slot layer (side x (side + 1), its last line the
# header), the zero layers new pages start from, `cap` the most slots a page holds, and the most
# atlas layers a tile's rows take.
func add_level(level: Dictionary, cap: int) -> void:
	var side := int(level["side"])
	counters["images_created"] += 2
	levels.append({
		"side": side,
		"cap": cap,
		"max_layers": int(ceil(float(level["max_row_texels"]) / ATLAS_WIDTH)),
		"zero_texels": Image.create_empty(side, side + 1, false, Image.FORMAT_RGBAF),
		"zero_atlas": Image.create_empty(ATLAS_WIDTH, 1, false, Image.FORMAT_RGBAF),
	})

# Drops every level and page, as the renderer sets up (the book makes its pages again).
func clear_levels() -> void:
	levels.clear()
	by_id.clear()

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
	by_id[page_id] = {"id": page_id, "code": code, "block": block, "slots": slots, "layers": layers, "texels": texels,
		"atlas": atlas, "bytes": slots * side * (side + 1) * 16 + layers * ATLAS_LAYER_BYTES}

# A page holding no copy was released: pooled (`kept`, its CPU copies dropped) or freed.
func released(page_id: int, kept: bool) -> void:
	if trace != null:
		trace.out("page_release %d %d" % [page_id, int(kept)])
	book.forget_page(page_id)
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

# The bytes layer `layer` of `page`'s slot layers (`texels`) or row atlas holds, for the tests'
# byte checks: on Compatibility its CPU copy (an unwritten layer is zero), where the readback
# fails (TERRAIN-22), else the device's readback. Not for the frame path: it reads back.
func layer_data(page: Dictionary, texels: bool, layer: int) -> PackedByteArray:
	if not mirrored:
		return (page["texels" if texels else "atlas"] as TextureLayered).get_layer_data(layer).get_data()
	var data: PackedByteArray = book.page_layer(int(page["id"]), texels, layer)
	if data.is_empty():
		return (levels[int(page["code"])]["zero_texels" if texels else "zero_atlas"] as Image).get_data()
	return data
