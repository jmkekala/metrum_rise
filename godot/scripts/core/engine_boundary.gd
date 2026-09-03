# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_boundary.gd
#  script_path: scripts/core/engine_boundary.gd
#  module_name: engine_boundary
#  version: 0.1.0
#  author: [BantedHam]
#  description: The simulation boundary between the Rust economy and the
#           2.5D engine. Down: batched field sampling through a kernel's
#           static evaluate, one array in and one array out per tick.
#           Up: per-cell deposit buffers written as the engine's
#           converted-grid format (signed 16-bit metres plus a JSON
#           sidecar), which heightmap_node opens as measured ground
#           truth. The coupling closes with one tick of lag; nothing
#           ever calls per-agent.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/fbm_node.gd,
#           addons/2.5D_engine/evaluator/heightmap_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [batched-sampling, deposit-grids, fixtures]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Two arrays and a tick boundary: the whole interface between the
## discrete economy and the continuous engine.
@tool
extends RefCounted

const Fbm := preload("res://addons/2.5D_engine/evaluator/fbm_node.gd")
const Heightmap := preload("res://addons/2.5D_engine/evaluator/heightmap_node.gd")


## Sample one engine field at every position, batched. The economy calls
## this once per tick with everything it wants to know, and per-agent
## chatter never crosses the boundary. Sixty-four bit throughout: the
## engine's kernels and goldens are f64, and a 32-bit boundary truncated
## every sample, which the spike caught on its first run.
static func sample_field(positions: PackedVector3Array, footprint: float,
		t: float, field_seed: int) -> PackedFloat64Array:
	var out := PackedFloat64Array()
	out.resize(positions.size())
	for i in positions.size():
		var p := positions[i]
		out[i] = Fbm.evaluate(p.x, p.y, p.z, footprint, t, field_seed)
	return out


## Write one deposit buffer in the engine's converted-grid format: row-major
## signed 16-bit values and the sidecar heightmap_node reads. Values clamp
## to the format's range; the engine treats the result as measured data.
static func write_deposit(grid: PackedFloat32Array, width: int, height: int,
		origin_lon: float, origin_lat: float, pixel_deg: float,
		raw_path: String) -> bool:
	if width <= 0 or height <= 0 or grid.size() != width * height:
		return false
	var f := FileAccess.open(raw_path, FileAccess.WRITE)
	if f == null:
		return false
	for v in grid:
		var q := clampi(int(roundf(v)), -32768, 32767)
		f.store_16(q & 0xFFFF)
	f.close()
	var meta := {
		"width": width, "height": height,
		"origin_lon": origin_lon, "origin_lat": origin_lat,
		"pixel_deg_lon": pixel_deg, "pixel_deg_lat": pixel_deg,
	}
	var m := FileAccess.open(raw_path.get_basename() + ".json", FileAccess.WRITE)
	if m == null:
		return false
	m.store_string(JSON.stringify(meta, "\t"))
	m.close()
	return true


## Open a deposit the engine's own way. What comes back answers
## `data_elevation_m(lat, lon)` from the written cells alone.
static func open_deposit(raw_path: String) -> RefCounted:
	return Heightmap.new(raw_path)
