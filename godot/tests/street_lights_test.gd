# SPDX-License-Identifier: GPL-2.0-only

## Headless lamp grid coverage, packing, deterministic overflow, far-map splat and chunk-edit
## regression through the Rust `StreetLightBook` (`RENDER-32-H`), and the chunk batches against
## the transforms GDScript builds, drawn on the book's slots and freed by a reset (`RENDER-52`
## part 2), each pole at the level Godot's visibility range gave its 64 m cell (`RENDER-46`); no
## scene.
extends SceneTree

const StreetLights := preload("res://scripts/renderers/street_lights.gd")
const SceneLighting := preload("res://scripts/core/scene_lighting.gd")
const SIDE := StreetLightBook.SIDE
const CAPACITY := StreetLightBook.CAPACITY
const CELL_M := 16.0
var _failures := 0

func _initialize() -> void:
	_test_coverage()
	_test_overflow()
	_test_far_splat()
	_test_far_edits()
	# The root enters the tree after `_initialize`: the drawn batches need its world.
	await process_frame
	_test_batches()
	quit(1 if _failures > 0 else 0)

func _expect(condition: bool, message: String) -> void:
	if not condition:
		_failures += 1
		push_error(message)

func _test_coverage() -> void:
	var origin := Vector2(-1024.0, -2048.0)
	# Edge, corner, exact tangent, negative coordinates, and a head just outside the window.
	var heads := PackedVector3Array([
		Vector3(-1025.0, 8.0, -2020.0), Vector3(-992.0, 11.15, -2016.0),
		Vector3(-1008.0, 4.4, -2032.0), Vector3(1030.0, 8.0, -8.0)])
	var reaches := PackedFloat32Array([28.0, 31.15, 16.0, 28.0])
	# Distinct kinds identify the heads in the slots.
	var kinds := PackedByteArray([0, 1, 2, 3])
	var slots := StreetLightBook.pack_cells(heads, kinds, reaches, origin)
	_expect(slots.size() == SIDE * SIDE * CAPACITY * 4, "RGBA cell texture size")
	for z in SIDE:
		for x in SIDE:
			var low := origin + Vector2(x, z) * CELL_M
			var high := low + Vector2.ONE * CELL_M
			var offset := (z * SIDE + x) * CAPACITY
			for i in heads.size():
				var point := Vector2(heads[i].x, heads[i].z)
				var dx := maxf(maxf(low.x - point.x, point.x - high.x), 0.0)
				var dz := maxf(maxf(low.y - point.y, point.y - high.y), 0.0)
				var overlaps := dx * dx + dz * dz <= reaches[i] * reaches[i]
				var found := false
				for slot in CAPACITY:
					found = found or int(slots[(offset + slot) * 4 + 3]) == i + 1
				_expect(found == overlaps, "head %d coverage at (%d, %d)" % [i, x, z])
	_expect(StreetLightBook.pack_cells(PackedVector3Array(), PackedByteArray(), PackedFloat32Array(), origin).count(0.0)
		== slots.size(), "empty grid must contain only terminators")

func _test_overflow() -> void:
	var heads := PackedVector3Array()
	var reaches := PackedFloat32Array()
	var kinds := PackedByteArray()
	for i in CAPACITY + 5:
		heads.append(Vector3(8.0, float(i), 8.0))
		reaches.append(28.0)
		kinds.append(i)
	var slots := StreetLightBook.pack_cells(heads, kinds, reaches, Vector2.ZERO)
	_expect(slots == StreetLightBook.pack_cells(heads, kinds, reaches, Vector2.ZERO), "packing must be repeatable")
	for slot in CAPACITY:
		_expect(int(slots[slot * 4 + 3]) == slot + 1, "overflow keeps the first K stable-priority heads")
	for offset in range(0, slots.size() / 4, CAPACITY):
		var seen := {}
		var ended := false
		for slot in CAPACITY:
			var id := int(slots[(offset + slot) * 4 + 3])
			if id == 0:
				ended = true
			else:
				_expect(not ended and not seen.has(id) and id <= CAPACITY,
					"cell slots must be contiguous, unique and capped")
				seen[id] = true

func _zeros(side: int) -> PackedFloat32Array:
	var texels := PackedFloat32Array()
	texels.resize(side * side * 4)
	texels.fill(0.0)
	return texels

# Every head inside the window adds exactly its light, spread over the four texels around it.
func _test_far_splat() -> void:
	var light := PackedColorArray([Color(1.0, 0.5, 0.25), Color(2.0, 2.0, 2.0)])
	var origin := Vector2(-64.0, -64.0)
	var heads := PackedVector3Array([Vector3(-38.0, 8.0, -35.0), Vector3(0.0, 11.0, 13.0),
		Vector3(500.0, 8.0, 0.0)])
	var kinds := PackedByteArray([0, 1, 0])
	var texels := StreetLightBook.splat_far(_zeros(8), heads, kinds, light, origin, 16.0, 8, 1.0)
	_expect(texels.size() == 8 * 8 * 4, "far map is side x side RGBA")
	var sum := Vector3.ZERO
	var touched := 0
	for i in range(0, texels.size(), 4):
		sum += Vector3(texels[i], texels[i + 1], texels[i + 2])
		touched += 1 if texels[i] > 0.0 else 0
		_expect(texels[i + 3] == 0.0, "far map alpha stays zero")
	_expect(sum.is_equal_approx(Vector3(3.0, 2.5, 2.25)), "splat conserves each head's light: %s" % sum)
	_expect(touched == 8, "a head inside the window touches four texels")
	# Exact texel centre: all of it in one texel.
	var centred := StreetLightBook.splat_far(_zeros(8), PackedVector3Array([Vector3(-56.0, 8.0, -56.0)]),
		PackedByteArray([1]), light, origin, 16.0, 8, 1.0)
	_expect(is_equal_approx(centred[0], 2.0) and centred.count(0.0) == centred.size() - 3,
		"a head on a texel centre lands in that texel only")
	var removed := StreetLightBook.splat_far(texels, heads, kinds, light, origin, 16.0, 8, -1.0)
	var residue := 0.0
	for value in removed:
		residue = maxf(residue, absf(value))
	_expect(residue < 1e-5, "taking the same heads back out empties the map: %f" % residue)
	var flux := StreetLightBook.ground_flux(8.0, 28.0, cos(deg_to_rad(80.0)))
	# Without window and cone, cos/d over the disc where d < reach integrates to pi h ln(reach²/h²).
	_expect(flux > 0.0 and flux < PI * 8.0 * log(28.0 * 28.0 / 64.0),
		"ground flux below the windowless, coneless bound: %f" % flux)
	_expect(flux > StreetLightBook.ground_flux(11.15, 31.15, cos(deg_to_rad(80.0))) * 0.5,
		"higher heads spread a similar flux")

func _far_texels(lights: Node3D) -> PackedFloat32Array:
	return (lights.get("_book") as StreetLightBook).far_data().to_float32_array()

# Road edits patch the far map per chunk; the result must match a full rebuild, and removing every
# chunk must leave it empty.
func _test_far_edits() -> void:
	var lights: Node3D = StreetLights.new()
	# Outside the tree: no real light pool.
	lights.call("_upload_kinds")
	var book: StreetLightBook = lights.get("_book")
	lights.configure_chunks(512.0, Vector2(-1024.0, -1024.0))
	lights.set_chunk(Vector2i(2, 2), Vector3(0.0, 0.0, 0.0), PackedFloat32Array([10.0, 0.0, 20.0, 0.0, 0.0]))
	book.rebuild_far_at(Vector2.ZERO)
	lights.set_chunk(Vector2i(2, 2), Vector3(0.0, 0.0, 0.0),
		PackedFloat32Array([30.0, 0.0, 5.0, 1.0, 2.0, 90.0, 0.0, 40.0, 0.0, 4.0]))
	lights.set_chunk(Vector2i(3, 2), Vector3(512.0, 0.0, 0.0), PackedFloat32Array([7.0, 0.0, 7.0, 0.0, 1.0]))
	var patched := _far_texels(lights)
	book.rebuild_far_at(Vector2.ZERO)
	var rebuilt := _far_texels(lights)
	var worst := 0.0
	for i in patched.size():
		worst = maxf(worst, absf(patched[i] - rebuilt[i]))
	_expect(worst < 1e-4 and patched.count(0.0) < patched.size(), "edited far map matches a rebuild: %f" % worst)
	lights.clear()
	worst = 0.0
	for value in _far_texels(lights):
		worst = maxf(worst, absf(value))
	_expect(worst < 1e-4, "removing every chunk empties the far map: %f" % worst)
	lights.free()

# The chunk's pole batches carry the transforms GDScript builds from the same lights, bit for bit,
# each pole at the level its own 64 m cell's batch drew under Godot's visibility range
# (`RENDER-46`): the cells' centres equal the engine's bounds of those batches on their pooled
# slots, and along a camera path in and out through every band the batches hold exactly the cells
# the engine's rule shows. The glow batch holds one quad a lamp head; the book draws them on its
# slots and frees every slot.
func _test_batches() -> void:
	var lights: Node3D = StreetLights.new()
	# In the tree: the book draws in its world.
	root.add_child(lights)
	var raw := PackedFloat32Array()
	for i in 40:
		raw.append_array([-250.0 + float(i) * 12.31, 3.7 + float(i) * 0.05, 240.0 - float(i) * 11.77,
			-3.1 + float(i) * 0.157, float(i % 5)])
	var key := Vector2i(1, -2)
	var origin := Vector3(768.0, 0.0, -768.0)
	lights.set_chunk(key, origin, raw)
	var drawing: StreetLightBook = lights.get("_book")
	var expected := {}
	for i in range(0, raw.size(), 5):
		var t := Transform3D(Basis(Vector3.UP, raw[i + 3]), Vector3(raw[i], raw[i + 1], raw[i + 2]))
		expected[PackedFloat32Array([t.basis.x.x, t.basis.y.x, t.basis.z.x, t.origin.x, t.basis.x.y,
			t.basis.y.y, t.basis.z.y, t.origin.y, t.basis.x.z, t.basis.y.z, t.basis.z.z, t.origin.z]).to_byte_array().hex_encode()] = true
	_expect(drawing.get_drawn_instances(key) == Vector2i(0, 48),
		"one glow quad a head, no pole before a frame: %s" % drawing.get_drawn_instances(key))
	# The 64 m cell batches the control drew, through a book of its own: per cell its kind and
	# transforms.
	var book := StreetLightBook.new()
	book.set_kinds(PackedColorArray(StreetLights.LAMP_COLORS), PackedColorArray(StreetLights.LAMP_COLORS),
		PackedByteArray([1, 1, 1, 1, 1]))
	var cells := book.set_chunk(key, origin, raw)
	var cell_kinds: PackedByteArray = cells["kinds"]
	var cell_rows: Array = cells["transforms"]
	var seen := 0
	for buffer: PackedFloat32Array in cell_rows:
		for index in range(0, buffer.size(), 12):
			_expect(expected.has(buffer.slice(index, index + 12).to_byte_array().hex_encode()), "a cell transform is a light's")
			seen += 1
	_expect(seen == 40, "every light in a cell batch: %d" % seen)
	# Per cell and level, the centre of the batch's bounds as the engine measured it: the mesh
	# bounds through each instance of the slot's buffer (padded to its capacity with the first),
	# merged in order, grown by the level's extra cull margin, in the chunk's frame.
	var lods := StreetLightBook.pole_lods()
	var margins: Array[float] = [SceneLighting.STATIC_CASTER_EXTRA_CULL_MARGIN_M,
		SceneLighting.STATIC_CASTER_EXTRA_CULL_MARGIN_M, SceneLighting.RECEIVER_EXTRA_CULL_MARGIN_M]
	var meshes: Array = lights.get("_meshes")
	var engine := PackedVector3Array()
	for cell in cell_kinds.size():
		var rows: PackedFloat32Array = cell_rows[cell]
		var capacity := StreetLightBook.slot_capacity(rows.size() / 12)
		for lod in lods:
			var mesh_aabb: AABB = (meshes[cell_kinds[cell]][lod] as ArrayMesh).get_aabb()
			var bounds := AABB()
			for instance in capacity:
				var at := (instance if instance < rows.size() / 12 else 0) * 12
				var t := Transform3D(Basis(Vector3(rows[at], rows[at + 4], rows[at + 8]),
					Vector3(rows[at + 1], rows[at + 5], rows[at + 9]),
					Vector3(rows[at + 2], rows[at + 6], rows[at + 10])), Vector3(rows[at + 3], rows[at + 7], rows[at + 11]))
				bounds = t * mesh_aabb if instance == 0 else bounds.merge(t * mesh_aabb)
			engine.append((Transform3D(Basis(), origin) * bounds.grow(margins[lod])).get_center())
	var poles := drawing.pole_cells(key)
	var centres: PackedVector3Array = poles["centres"]
	_expect(centres.size() == cell_kinds.size() * lods, "a centre per cell and level: %d" % centres.size())
	_expect(centres.to_byte_array() == engine.to_byte_array(), "cell centres are the engine's, bit for bit")
	# The slots' kinds: a kind's batches in its first cell's order.
	var slots := PackedByteArray()
	for kind in cell_kinds:
		if not slots.has(kind):
			slots.append(kind)
	_expect(poles["kinds"] == slots, "a batch per kind and level: %s" % poles["kinds"])
	# Camera path: from the chunk out to 1,600 m and back, 3 m steps, a little above the poles.
	var levels: Array[Vector2] = StreetLights.LOD_RANGES
	var shown := PackedByteArray()
	shown.resize(centres.size())
	var path: Array[float] = []
	for step in 534:
		path.append(float(step) * 3.0)
	for step in 534:
		path.append(1599.0 - float(step) * 3.0)
	var mismatches := 0
	var levels_seen := {}
	for distance in path:
		var eye := origin + Vector3(distance * 0.8, 6.5, distance * 0.6)
		drawing.draw_poles(eye)
		# The engine's rule (fade disabled): a shown batch keeps the range widened by its margins.
		var drawn := 0
		for index in centres.size():
			var lod := index % lods
			var band: Vector2 = levels[lod]
			var begin_margin := 10.0 if lod > 0 else 0.0
			var end_margin := 10.0
			var was := shown[index] != 0
			var begin_offset := -begin_margin if was else begin_margin
			var end_offset := end_margin if was else -end_margin
			var d := eye.distance_to(centres[index])
			var now := not (band.y > 0.0 and d > band.y + end_offset) and not (band.x > 0.0 and d < band.x + begin_offset)
			shown[index] = 1 if now else 0
			if now:
				levels_seen[lod] = true
		for slot in slots.size():
			for lod in lods:
				var want := PackedFloat32Array()
				for cell in cell_kinds.size():
					if cell_kinds[cell] == slots[slot] and shown[cell * lods + lod] != 0:
						want.append_array(cell_rows[cell])
				var held := drawing.pole_batch(key, slot, lod)
				if held != want:
					mismatches += 1
				drawn += held.size() / 12
		if drawing.get_drawn_instances(key).x != drawn:
			mismatches += 1
	_expect(mismatches == 0, "the batches hold the cells the engine's range shows: %d mismatches" % mismatches)
	_expect(levels_seen.size() == lods, "the path crosses every level: %s" % levels_seen)
	# A reset keeps the slots as spares; leaving the tree frees them all.
	lights.clear()
	var ledger: Dictionary = drawing.get_draw_ledger()
	_expect(int(ledger["made"]) > 0 and ledger["live"] == 0
		and int(ledger["made"]) - int(ledger["freed"]) == int(ledger["spares"]), "a reset keeps spares: %s" % ledger)
	root.remove_child(lights)
	_expect(drawing.get_draw_ledger()["made"] == 0, "leaving the tree lets the draw go: %s" % drawing.get_draw_ledger())
	lights.free()
