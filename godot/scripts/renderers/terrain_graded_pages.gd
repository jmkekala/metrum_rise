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
		# Every layer starts from one shared zero image.
		var texel_layers: Array[Image] = []
		texel_layers.resize(slots)
		texel_layers.fill(level["zero_texels"])
		var atlas_layers: Array[Image] = []
		atlas_layers.resize(layers)
		atlas_layers.fill(level["zero_atlas"])
		var texels := Texture2DArray.new()
		texels.create_from_images(texel_layers)
		var atlas := Texture2DArray.new()
		atlas.create_from_images(atlas_layers)
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
	(page["texels"] as Texture2DArray).update_layer(image, copy.x)
	var rows: PackedByteArray = tile["rows"]
	var count := rows.size() / ATLAS_LAYER_BYTES
	if count > 0:
		var run: Image = level["run_image"]
		run.set_data(ATLAS_WIDTH, count, false, Image.FORMAT_RGBAF, rows)
		var layer: Image = level["atlas_image"]
		for index in count:
			layer.blit_rect(run, Rect2i(0, index, ATLAS_WIDTH, 1), Vector2i.ZERO)
			(page["atlas"] as Texture2DArray).update_layer(layer, copy.y + index)
	counters["upload_ms_max"] = max(float(counters["upload_ms_max"]), float(Time.get_ticks_usec() - upload_start_us) / 1000.0)
	counters["uploads"] += 1
	counters["upload_bytes"] += (tile["texels"] as PackedByteArray).size() + rows.size()
