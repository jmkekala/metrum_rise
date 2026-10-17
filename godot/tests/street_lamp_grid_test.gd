# SPDX-License-Identifier: GPL-2.0-only

## Headless lamp grid coverage, packing, deterministic overflow and far-map splat regression; no
## scene or assets.
extends SceneTree

const Grid := preload("res://scripts/renderers/street_lamp_grid.gd")
const StreetLights := preload("res://scripts/renderers/street_lights.gd")
var _failures := 0

func _initialize() -> void:
	_test_coverage()
	_test_overflow()
	_test_far_splat()
	_test_far_edits()
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
	var slots := Grid.pack_cells(heads, kinds, reaches, origin)
	_expect(slots.size() == Grid.SIDE * Grid.SIDE * Grid.CAPACITY * 4, "RGBA cell texture size")
	for z in Grid.SIDE:
		for x in Grid.SIDE:
			var low := origin + Vector2(x, z) * Grid.CELL_M
			var high := low + Vector2.ONE * Grid.CELL_M
			var offset := (z * Grid.SIDE + x) * Grid.CAPACITY
			for i in heads.size():
				var point := Vector2(heads[i].x, heads[i].z)
				var dx := maxf(maxf(low.x - point.x, point.x - high.x), 0.0)
				var dz := maxf(maxf(low.y - point.y, point.y - high.y), 0.0)
				var overlaps := dx * dx + dz * dz <= reaches[i] * reaches[i]
				var found := false
				for slot in Grid.CAPACITY:
					found = found or int(slots[(offset + slot) * 4 + 3]) == i + 1
				_expect(found == overlaps, "head %d coverage at (%d, %d)" % [i, x, z])
	_expect(Grid.pack_cells(PackedVector3Array(), PackedByteArray(), PackedFloat32Array(), origin).count(0.0)
		== slots.size(), "empty grid must contain only terminators")

func _test_overflow() -> void:
	var heads := PackedVector3Array()
	var reaches := PackedFloat32Array()
	var kinds := PackedByteArray()
	for i in Grid.CAPACITY + 5:
		heads.append(Vector3(8.0, float(i), 8.0))
		reaches.append(28.0)
		kinds.append(i)
	var slots := Grid.pack_cells(heads, kinds, reaches, Vector2.ZERO)
	_expect(slots == Grid.pack_cells(heads, kinds, reaches, Vector2.ZERO), "packing must be repeatable")
	for slot in Grid.CAPACITY:
		_expect(int(slots[slot * 4 + 3]) == slot + 1, "overflow keeps the first K stable-priority heads")
	for offset in range(0, slots.size() / 4, Grid.CAPACITY):
		var seen := {}
		var ended := false
		for slot in Grid.CAPACITY:
			var id := int(slots[(offset + slot) * 4 + 3])
			if id == 0:
				ended = true
			else:
				_expect(not ended and not seen.has(id) and id <= Grid.CAPACITY,
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
	var texels := Grid.splat_far(_zeros(8), heads, kinds, light, origin, 16.0, 8)
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
	var centred := Grid.splat_far(_zeros(8), PackedVector3Array([Vector3(-56.0, 8.0, -56.0)]), PackedByteArray([1]),
		light, origin, 16.0, 8)
	_expect(is_equal_approx(centred[0], 2.0) and centred.count(0.0) == centred.size() - 3,
		"a head on a texel centre lands in that texel only")
	var removed := Grid.splat_far(texels, heads, kinds, light, origin, 16.0, 8, -1.0)
	var residue := 0.0
	for value in removed:
		residue = maxf(residue, absf(value))
	_expect(residue < 1e-5, "taking the same heads back out empties the map: %f" % residue)
	var flux := Grid.ground_flux(8.0, 28.0, cos(deg_to_rad(80.0)))
	# Without window and cone, cos/d over the disc where d < reach integrates to pi h ln(reach²/h²).
	_expect(flux > 0.0 and flux < PI * 8.0 * log(28.0 * 28.0 / 64.0),
		"ground flux below the windowless, coneless bound: %f" % flux)
	_expect(flux > Grid.ground_flux(11.15, 31.15, cos(deg_to_rad(80.0))) * 0.5,
		"higher heads spread a similar flux")

# Road edits patch the far map per chunk; the result must match a full rebuild, and removing every
# chunk must leave it empty.
func _test_far_edits() -> void:
	var lights: Node3D = StreetLights.new()
	# Outside the tree: the colour/flux table that _ready() builds, without the real light pool.
	lights.call("_upload_kinds")
	lights.configure_chunks(512.0, Vector2(-1024.0, -1024.0))
	lights.set_chunk(Vector2i(2, 2), Vector3(0.0, 0.0, 0.0), PackedFloat32Array([10.0, 0.0, 20.0, 0.0, 0.0]))
	lights.call("_rebuild_far", Vector2.ZERO)
	lights.set_chunk(Vector2i(2, 2), Vector3(0.0, 0.0, 0.0),
		PackedFloat32Array([30.0, 0.0, 5.0, 1.0, 2.0, 90.0, 0.0, 40.0, 0.0, 4.0]))
	lights.set_chunk(Vector2i(3, 2), Vector3(512.0, 0.0, 0.0), PackedFloat32Array([7.0, 0.0, 7.0, 0.0, 1.0]))
	var patched: PackedFloat32Array = lights.get("_far_texels").duplicate()
	lights.call("_rebuild_far", Vector2.ZERO)
	var rebuilt: PackedFloat32Array = lights.get("_far_texels")
	var worst := 0.0
	for i in patched.size():
		worst = maxf(worst, absf(patched[i] - rebuilt[i]))
	_expect(worst < 1e-4 and patched.count(0.0) < patched.size(), "edited far map matches a rebuild: %f" % worst)
	lights.clear()
	worst = 0.0
	for value in lights.get("_far_texels"):
		worst = maxf(worst, absf(value))
	_expect(worst < 1e-4, "removing every chunk empties the far map: %f" % worst)
	lights.free()
