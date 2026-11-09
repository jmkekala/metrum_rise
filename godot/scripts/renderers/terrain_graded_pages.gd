# SPDX-License-Identifier: GPL-2.0-only

## Graded terrain pages (TERRAIN-07-G, docs/terrain.md "Pages and the row atlas"): the page
## allocator and the row atlas of terrain_graded_tiles.gd. A page is two Texture2DArrays, its slot
## layers (one per tile copy: the lattice and a header line) and its row atlas (a run of whole
## 256-texel layers per copy's rows). This script holds each level's layer shapes and pooled
## upload Images, creates pages within the graded page budget, pools freed pages by level and
## size, hands out a page's slots and first-fit atlas runs, and uploads a copy. Which page a block
## of patches uses, the tiles' copies, a block's growth and the materials' binding are
## terrain_graded_tiles.gd's. Nothing here allocates per frame: pages are pooled, and every upload
## goes through the level's pooled Images.
extends RefCounted

# Texels of a row atlas layer (residency.rs ATLAS_WIDTH, checked against the layout at setup).
const ATLAS_WIDTH := 256
const ATLAS_LAYER_BYTES := ATLAS_WIDTH * 16
# The layer limit without a RenderingDevice (Compatibility, headless): what Vulkan and GLES3 both
# guarantee. Never above 2,048: a slot reaches the shader in a half-float instance channel.
const MIN_LAYER_LIMIT := 256
const MAX_LAYER_LIMIT := 2048
# The graded pages' memory budget (texel and atlas layers of every page, free pooled ones
# included): a page the budget refuses leaves its tiles exhausted (counted, T53 (a)'s rule).
const GRADED_PAGE_BUDGET_MB := 1024
# Free pages kept per level and size for reuse before a new one is created.
const FREE_PAGES_KEPT := 4
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
	elif DisplayServer.get_name() != "headless":
		# Compatibility has no RenderingDevice. Growth alone reads back its bounded old page;
		# ordinary uploads and idle frames never do. The dummy renderer stores no texels.
		for layer in source.get_layers():
			target.update_layer(source.get_layer_data(layer), layer)

# Preserve the page's published contents and allocation layout before rebinding its materials.
# No rebake is involved: edits, unlisted commit tiles and previews all keep their exact bytes.
# O(source slots + source atlas layers), with device-to-device copies on Forward+/Mobile.
func copy_page(source: Dictionary, target: Dictionary) -> void:
	_copy_array(source["texels"], target["texels"])
	_copy_array(source["atlas"], target["atlas"])
	var free: Array[int] = []
	for slot in range(int(target["slots"]) - 1, int(source["slots"]) - 1, -1):
		free.append(slot)
	free.append_array(source["free"])
	target["free"] = free
	target["runs"] = (source["runs"] as Array[Vector2i]).duplicate()
	if int(target["layers"]) > int(source["layers"]):
		_free_run(target, int(source["layers"]), int(target["layers"]) - int(source["layers"]))

# The renderer's counters (the same Dictionary): textures, images and pages created, uploads.
var counters: Dictionary = {}
# Per level code: the slot layer's side, the pooled upload Images, the zero layers new pages start
# from, the most slots a page of the level holds (`cap`) and the most atlas layers a tile's rows
# take (`max_layers`).
var levels: Array[Dictionary] = []
# Page id -> {id, code, block, texels, atlas (Texture2DArray), slots, layers, free (slots), runs
# (free atlas runs as Vector2i(first, count), sorted by first), tiles and previews (tile key ->
# true: the installed and preview copies it holds), bytes}.
var by_id: Dictionary = {}
# Every page's bytes, the free pooled ones included.
var bytes := 0
var _next_id := 0
# Free pages for reuse: "code,slots,layers" -> Array of page ids.
var _free: Dictionary = {}

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

# Drops every page, the free pooled ones included.
func reset() -> void:
	by_id.clear()
	_free.clear()
	bytes = 0

# A page of `slots` slot layers and `layers` atlas layers for `block` at level `code`: a pooled
# free one of that size, else created within the budget (-1: the budget refuses it). O(slots +
# layers).
func create(code: int, block: Vector2i, slots: int, layers: int) -> int:
	var size_key := "%d,%d,%d" % [code, slots, layers]
	var pooled: Array = _free.get(size_key, [])
	var page: Dictionary = {}
	if not pooled.is_empty():
		page = by_id[pooled.pop_back()]
	else:
		var level: Dictionary = levels[code]
		var side := int(level["side"])
		var page_bytes := slots * side * (side + 1) * 16 + layers * ATLAS_LAYER_BYTES
		while bytes + page_bytes > GRADED_PAGE_BUDGET_MB * 1048576 and _drop_free_page():
			pass
		if bytes + page_bytes > GRADED_PAGE_BUDGET_MB * 1048576:
			return -1
		var texels := _array(level["zero_texels"], slots)
		var atlas := _array(level["zero_atlas"], layers)
		counters["textures_created"] += 2
		counters["pages_created"] += 1
		page = {"id": _next_id, "code": code, "slots": slots, "layers": layers, "texels": texels, "atlas": atlas, "bytes": page_bytes}
		by_id[_next_id] = page
		_next_id += 1
		bytes += page_bytes
	var free: Array[int] = []
	for slot in range(slots - 1, -1, -1):
		free.append(slot)
	var runs: Array[Vector2i] = [Vector2i(0, layers)]
	page["block"] = block
	page["free"] = free
	page["runs"] = runs
	page["tiles"] = {}
	page["previews"] = {}
	return int(page["id"])

# Drops one pooled free page (its textures go with their last reference). Returns whether one was.
func _drop_free_page() -> bool:
	for size_key: String in _free.keys():
		var pooled: Array = _free[size_key]
		if pooled.is_empty():
			continue
		var page: Dictionary = by_id[pooled.pop_back()]
		by_id.erase(int(page["id"]))
		bytes -= int(page["bytes"])
		return true
	return false

# Returns a page holding no copy to the free pool (dropped past FREE_PAGES_KEPT of its size).
func release(page_id: int) -> void:
	var page: Dictionary = by_id[page_id]
	var size_key := "%d,%d,%d" % [int(page["code"]), int(page["slots"]), int(page["layers"])]
	if not _free.has(size_key):
		_free[size_key] = []
	var pooled: Array = _free[size_key]
	if pooled.size() < FREE_PAGES_KEPT:
		pooled.append(page_id)
		return
	by_id.erase(page_id)
	bytes -= int(page["bytes"])

# The first of the page's free atlas runs at least `count` layers long (-1: none). O(runs).
static func find_run(runs: Array[Vector2i], count: int) -> int:
	for index in runs.size():
		if runs[index].y >= count:
			return index
	return -1

# Takes `count` layers from the page's first free run long enough (-1: none). O(runs).
static func _take_run(page: Dictionary, count: int) -> int:
	var runs: Array[Vector2i] = page["runs"]
	var index := find_run(runs, count)
	if index < 0:
		return -1
	var run := runs[index]
	if run.y == count:
		runs.remove_at(index)
	else:
		runs[index] = Vector2i(run.x + count, run.y - count)
	return run.x

# Returns layers [first, first + count) to the page's free runs, merged with its neighbours.
# O(runs).
static func _free_run(page: Dictionary, first: int, count: int) -> void:
	var runs: Array[Vector2i] = page["runs"]
	var index := 0
	while index < runs.size() and runs[index].x < first:
		index += 1
	runs.insert(index, Vector2i(first, count))
	if index + 1 < runs.size() and first + count == runs[index + 1].x:
		runs[index] = Vector2i(first, count + runs[index + 1].y)
		runs.remove_at(index + 1)
	if index > 0 and runs[index - 1].x + runs[index - 1].y == first:
		runs[index - 1] = Vector2i(runs[index - 1].x, runs[index - 1].y + runs[index].y)
		runs.remove_at(index)

# Whether `page` can take a copy of `layers` atlas layers, `held` its copy there (x < 0: none):
# a free slot for a new copy, and a run unless the held one is long enough.
static func can_place(page: Dictionary, held: Vector3i, layers: int) -> bool:
	if held.x < 0 and (page["free"] as Array).is_empty():
		return false
	if layers > 0 and (held.x < 0 or layers > held.z) and find_run(page["runs"], layers) < 0:
		return false
	return true

# Places a copy in `page` (can_place holds): the held slot kept, else a free one, and the held
# run kept while long enough, else a new one (the held run freed after). Returns the copy,
# Vector3i(slot, first atlas layer, layers).
static func place(page: Dictionary, held: Vector3i, layers: int) -> Vector3i:
	var copy := held
	if layers > 0 and (held.x < 0 or layers > held.z):
		var first := _take_run(page, layers)
		if held.x >= 0 and held.z > 0:
			_free_run(page, held.y, held.z)
		copy = Vector3i(held.x, first, layers)
	if copy.x < 0:
		copy.x = (page["free"] as Array).pop_back()
	return copy

# Returns a copy's slot and run to `page`. O(the page's free runs).
static func vacate(page: Dictionary, copy: Vector3i) -> void:
	(page["free"] as Array).append(copy.x)
	if copy.z > 0:
		_free_run(page, copy.y, copy.z)

# Writes a tile's slot layer, its header's first channel set to its run's first atlas layer
# (residency.rs `pack_slot`), and its run into its copy in `page`. O(slot texels + rows).
func write_copy(page: Dictionary, copy: Vector3i, tile: Dictionary) -> void:
	var level: Dictionary = levels[int(page["code"])]
	var upload_start_us := Time.get_ticks_usec()
	var side := int(level["side"])
	var image: Image = level["texel_image"]
	image.set_data(side, side + 1, false, Image.FORMAT_RGBAF, tile["texels"])
	var header := image.get_pixel(0, side)
	header.r = float(copy.y)
	image.set_pixel(0, side, header)
	page["texels"].update_layer(image, copy.x)
	var rows: PackedByteArray = tile["rows"]
	var count := rows.size() / ATLAS_LAYER_BYTES
	if count > 0:
		var run: Image = level["run_image"]
		run.set_data(ATLAS_WIDTH, count, false, Image.FORMAT_RGBAF, rows)
		var layer: Image = level["atlas_image"]
		for index in count:
			layer.blit_rect(run, Rect2i(0, index, ATLAS_WIDTH, 1), Vector2i.ZERO)
			page["atlas"].update_layer(layer, copy.y + index)
	counters["upload_ms_max"] = max(float(counters["upload_ms_max"]), float(Time.get_ticks_usec() - upload_start_us) / 1000.0)
	counters["uploads"] += 1
	counters["upload_bytes"] += (tile["texels"] as PackedByteArray).size() + rows.size()
