# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_mesh_source.gd
#  script_path: scripts/core/engine_mesh_source.gd
#  module_name: engine_mesh_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Buildings' alternate mesh source: geometry frozen from the
#           2.5D engine's evaluated fields instead of loaded from asset
#           packs. Each part is sampled from the engine's own kernels,
#           keyed by a seed derived from the asset id, built once, and
#           cached by that key, which is the engine's freeze-and-instance
#           workflow consumed entirely through public evaluate calls.
#           Nothing engine-side is touched.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/fbm_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [frozen-meshes, seeded, cached, toggle]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Frozen evaluated geometry for building parts, cached by graph key.
@tool
extends RefCounted

const Fbm := preload("res://addons/2.5D_engine/evaluator/fbm_node.gd")

## The toggle: packs stay the default until the switch is thrown, and the
## fallback either way is the pack pipeline that already works.
const TOGGLE_PATH := "user://engine_meshes.cfg"

static var _cache: Dictionary = {}


static func enabled() -> bool:
	if not FileAccess.file_exists(TOGGLE_PATH):
		return false
	var cfg := ConfigFile.new()
	if cfg.load(TOGGLE_PATH) != OK:
		return false
	return bool(cfg.get_value("engine_meshes", "enabled", false))


static func set_enabled(on: bool) -> void:
	var cfg := ConfigFile.new()
	cfg.set_value("engine_meshes", "enabled", on)
	cfg.save(TOGGLE_PATH)


## The freeze key: one asset part is one graph identity, and identical
## identities share one frozen mesh, which is the instancing rule.
static func freeze_key(asset_id: String, part_index: int) -> int:
	return ("%s|%d" % [asset_id, part_index]).hash()


## A frozen mesh for one building part, evaluated from the engine's field
## and cached. The part is a shell whose upper surface carries the field's
## relief, sampled at a fixed grid: deterministic for its key, different
## between keys, and built from evaluate calls alone.
static func frozen_mesh_for(asset_id: String, part_index: int) -> Mesh:
	var key := freeze_key(asset_id, part_index)
	if _cache.has(key):
		return _cache[key]

	var grid := 8
	var size := 4.0
	var base_h := 3.0
	var st := SurfaceTool.new()
	st.begin(Mesh.PRIMITIVE_TRIANGLES)

	# The evaluated top: engine field relief over the part's footprint.
	var heights: Array[float] = []
	for row in grid + 1:
		for col in grid + 1:
			var u := float(col) / float(grid)
			var v := float(row) / float(grid)
			var relief := Fbm.evaluate(u * size, 0.0, v * size, 0.25, 0.0, key)
			heights.append(base_h + relief * 0.6)
	for row in grid:
		for col in grid:
			var i00 := row * (grid + 1) + col
			var i10 := i00 + 1
			var i01 := i00 + (grid + 1)
			var i11 := i01 + 1
			var p00 := Vector3((float(col) / grid - 0.5) * size, heights[i00], (float(row) / grid - 0.5) * size)
			var p10 := Vector3((float(col + 1) / grid - 0.5) * size, heights[i10], (float(row) / grid - 0.5) * size)
			var p01 := Vector3((float(col) / grid - 0.5) * size, heights[i01], (float(row + 1) / grid - 0.5) * size)
			var p11 := Vector3((float(col + 1) / grid - 0.5) * size, heights[i11], (float(row + 1) / grid - 0.5) * size)
			_tri(st, p00, p10, p11)
			_tri(st, p00, p11, p01)

	# Four walls and the floor close the shell.
	var half := size * 0.5
	var corners := [
		Vector3(-half, 0, -half), Vector3(half, 0, -half),
		Vector3(half, 0, half), Vector3(-half, 0, half),
	]
	var tops := [
		Vector3(-half, heights[0], -half),
		Vector3(half, heights[grid], -half),
		Vector3(half, heights[heights.size() - 1], half),
		Vector3(-half, heights[grid * (grid + 1)], half),
	]
	for i in 4:
		var j := (i + 1) % 4
		_tri(st, corners[i], corners[j], tops[j])
		_tri(st, corners[i], tops[j], tops[i])
	_tri(st, corners[0], corners[2], corners[1])
	_tri(st, corners[0], corners[3], corners[2])

	st.generate_normals()
	var mesh := st.commit()
	_cache[key] = mesh
	return mesh


static func _tri(st: SurfaceTool, a: Vector3, b: Vector3, c: Vector3) -> void:
	st.add_vertex(a)
	st.add_vertex(b)
	st.add_vertex(c)
