# SPDX-License-Identifier: GPL-2.0-only

## Pure packing of the camera's lamp-cell lists (input order is the stable overflow priority) and
## of the coarse far light map.
extends RefCounted

const CELL_M := 16.0
const SIDE := 128
# Two street sides at 25 m spacing plus junction approaches; excess heads keep stable priority.
# street_lamp_light.gdshaderinc reads the same count.
const CAPACITY := 16

## One RGBAF texel per slot, CAPACITY slots per cell along a row: the lamp head's world position
## and its kind plus one, so a zero ends a partial list.
static func pack_cells(heads: PackedVector3Array, kinds: PackedByteArray, ranges: PackedFloat32Array,
		origin: Vector2) -> PackedFloat32Array:
	var slots := PackedFloat32Array()
	slots.resize(SIDE * SIDE * CAPACITY * 4)
	slots.fill(0.0)
	var counts := PackedByteArray()
	counts.resize(SIDE * SIDE)
	counts.fill(0)
	for i in heads.size():
		var point := Vector2(heads[i].x, heads[i].z) - origin
		var reach := ranges[i]
		var first := (Vector2i(((point - Vector2.ONE * reach) / CELL_M).ceil()) - Vector2i.ONE).max(Vector2i.ZERO)
		var last := Vector2i(((point + Vector2.ONE * reach) / CELL_M).floor()).min(Vector2i(SIDE - 1, SIDE - 1))
		for z in range(first.y, last.y + 1):
			for x in range(first.x, last.x + 1):
				var low := Vector2(x, z) * CELL_M
				var closest := point.clamp(low, low + Vector2.ONE * CELL_M)
				if point.distance_squared_to(closest) > reach * reach:
					continue
				var cell := z * SIDE + x
				if counts[cell] < CAPACITY:
					var offset := (cell * CAPACITY + counts[cell]) * 4
					slots[offset] = heads[i].x
					slots[offset + 1] = heads[i].y
					slots[offset + 2] = heads[i].z
					slots[offset + 3] = float(kinds[i] + 1)
					counts[cell] += 1
	return slots

## Light a lamp puts on level ground below it, integrated over the ground (m² times the shader's
## irradiance with energy 1): the same range window, cone and Lambert as the exact lamp, summed
## on a 1 m grid. Dividing by a cell's area gives the cell's mean irradiance.
static func ground_flux(height: float, reach: float, cone_cos: float) -> float:
	var flux := 0.0
	var step := 1.0
	var steps := ceili(reach / step)
	for zi in range(-steps, steps):
		for xi in range(-steps, steps):
			var x := (xi + 0.5) * step
			var z := (zi + 0.5) * step
			var d := sqrt(x * x + z * z + height * height)
			if d >= reach:
				continue
			var nd := pow(d / reach, 4.0)
			var window := maxf(1.0 - nd, 0.0)
			var down := height / d
			var rim := (1.0 - maxf(down, cone_cos)) / (1.0 - cone_cos)
			flux += window * window * (1.0 - rim * rim) * down / d * step * step
	return flux

## Mean ground light per far cell, RGBAF rows of `side`: every head adds its kind's flux (linear
## colour times energy times ground_flux, per m²) times `scale` to the four texels around it with
## bilinear weights, so filtered sampling spreads it as a tent. A scale of -1 takes a chunk's heads
## back out on an edit. Adds into `texels` in place and returns it; O(heads).
static func splat_far(texels: PackedFloat32Array, heads: PackedVector3Array, kinds: PackedByteArray,
		kind_light: PackedColorArray, origin: Vector2, cell_m: float, side: int, scale := 1.0) -> PackedFloat32Array:
	for i in heads.size():
		var f := (Vector2(heads[i].x, heads[i].z) - origin) / cell_m - Vector2(0.5, 0.5)
		var base := Vector2i(f.floor())
		var t := f - Vector2(base)
		var light := kind_light[kinds[i]] * scale
		for corner in 4:
			var x := base.x + (corner & 1)
			var z := base.y + (corner >> 1)
			if x < 0 or z < 0 or x >= side or z >= side:
				continue
			var w := (t.x if corner & 1 else 1.0 - t.x) * (t.y if corner >> 1 else 1.0 - t.y)
			var offset := (z * side + x) * 4
			texels[offset] += light.r * w
			texels[offset + 1] += light.g * w
			texels[offset + 2] += light.b * w
	return texels
