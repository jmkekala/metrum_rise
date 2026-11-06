# SPDX-License-Identifier: GPL-2.0-only

## Sparse resident ground masks and analytic rows. Layer zero of rows is the world chunk table;
## its R/G channels select the committed/preview layers. Empty chunks allocate no mask layer.
## Textures grow geometrically, retaining their layers between edits; hidden zoning samples none.
extends RefCounted

const SIDE := 256
var entries: Dictionary = {}
var _slots: Dictionary = {}
var _capacity := 0
var _row_height := 0
var _masks: Texture2DArray
var _rows: Texture2DArray
var _lookup: Image
var _bounds := Rect2i()
var upload_bytes := 0
var uploads := 0
var _lookup_dirty := false

func setup(world_size: Vector2) -> void:
	var first := Vector2i((-world_size * 0.5 / 512.0).floor())
	var last := Vector2i((world_size * 0.5 / 512.0).ceil())
	_bounds = Rect2i(first, last - first)
	RenderingServer.global_shader_parameter_set("zoning_mask_bounds", Vector4(first.x, first.y, _bounds.size.x, _bounds.size.y))

func clear() -> void:
	entries.clear()
	_slots.clear()
	_capacity = 0
	_row_height = 0
	RenderingServer.global_shader_parameter_set("zoning_masks", null)
	RenderingServer.global_shader_parameter_set("zoning_rows", null)
	_masks = null
	_rows = null
	_lookup = null
	RenderingServer.global_shader_parameter_set("zoning_mask_enabled", false)

func remove(key: Vector3i) -> void:
	if not entries.has(key):
		return
	_slots.erase(int(entries[key].slot))
	entries.erase(key)
	_set_lookup(key, 0)

func replace(key: Vector3i, payload: Dictionary) -> void:
	if int(payload.get("rectangle_count", 0)) == 0:
		remove(key)
		return
	var slot := 1
	if entries.has(key):
		slot = int(entries[key].slot)
	else:
		while _slots.has(slot):
			slot += 1
	var previous: Dictionary = entries.get(key, {})
	if not previous.is_empty() and previous.payload.mask == payload.mask and previous.payload.rows == payload.rows:
		return
	entries[key] = {"slot": slot, "payload": payload}
	_slots[slot] = key
	var row_bytes: PackedByteArray = payload.rows
	var needed_height := maxi(ceili(float(row_bytes.size()) / (SIDE * 16)), ceili(float(_bounds.size.x * _bounds.size.y) / SIDE))
	if slot >= _capacity or needed_height > _row_height:
		_reallocate(slot + 1, needed_height)
	else:
		# Paint changes analytic styles only; the conservative index mask remains bit-identical.
		if previous.is_empty() or previous.payload.mask != payload.mask:
			_masks.update_layer(Image.create_from_data(SIDE, SIDE, false, Image.FORMAT_RGBA8, payload.mask), slot)
			upload_bytes += SIDE * SIDE * 4
		_rows.update_layer(_row_image(payload), slot)
		upload_bytes += SIDE * _row_height * 16
		uploads += 1
		_set_lookup(key, slot)

func _row_image(payload: Dictionary) -> Image:
	var bytes: PackedByteArray = payload.rows
	bytes.resize(SIDE * _row_height * 16)
	return Image.create_from_data(SIDE, _row_height, false, Image.FORMAT_RGBAF, bytes)

func _set_lookup(key: Vector3i, slot: int) -> void:
	var chunk := Vector2i(key.x, key.y) - _bounds.position
	if _lookup == null or not Rect2i(Vector2i.ZERO, _bounds.size).has_point(chunk):
		return
	var index := chunk.y * _bounds.size.x + chunk.x
	var value := _lookup.get_pixel(index % SIDE, index / SIDE)
	if (key.z == 0 and int(value.r) == slot) or (key.z == 1 and int(value.g) == slot):
		return
	if key.z == 0:
		value.r = slot
	else:
		value.g = slot
	_lookup.set_pixel(index % SIDE, index / SIDE, value)
	_lookup_dirty = true

func publish() -> void:
	if _lookup != null and _lookup_dirty:
		_rows.update_layer(_lookup, 0)
		upload_bytes += SIDE * _row_height * 16
		_lookup_dirty = false

func _reallocate(layers: int, height: int) -> void:
	# Only residency growth takes this path. Exact capacity is device-bounded; never silently
	# omit a mask on exhaustion. The visible-chunk renderer keeps residency local to the view.
	_capacity = maxi(2, _capacity)
	while _capacity < layers:
		_capacity *= 2
	_row_height = maxi(1, _row_height)
	while _row_height < height:
		_row_height *= 2
	var mask_images: Array[Image] = []
	var row_images: Array[Image] = []
	mask_images.resize(_capacity)
	row_images.resize(_capacity)
	mask_images.fill(Image.create_empty(SIDE, SIDE, false, Image.FORMAT_RGBA8))
	row_images.fill(Image.create_empty(SIDE, _row_height, false, Image.FORMAT_RGBAF))
	_lookup = Image.create_empty(SIDE, _row_height, false, Image.FORMAT_RGBAF)
	for key: Vector3i in entries:
		var entry: Dictionary = entries[key]
		var slot := int(entry.slot)
		mask_images[slot] = Image.create_from_data(SIDE, SIDE, false, Image.FORMAT_RGBA8, entry.payload.mask)
		row_images[slot] = _row_image(entry.payload)
		_set_lookup(key, slot)
	row_images[0] = _lookup
	_masks = Texture2DArray.new()
	_rows = Texture2DArray.new()
	var mask_error := _masks.create_from_images(mask_images)
	var row_error := _rows.create_from_images(row_images)
	if mask_error != OK or row_error != OK:
		push_error("Zoning ground texture allocation failed")
		clear()
		return
	RenderingServer.global_shader_parameter_set("zoning_masks", _masks)
	RenderingServer.global_shader_parameter_set("zoning_rows", _rows)
	upload_bytes += resident_bytes()
	uploads += 1

func resident_bytes() -> int:
	return _capacity * (SIDE * SIDE * 4 + SIDE * _row_height * 16)
