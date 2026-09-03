# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_terrain_source.gd
#  script_path: scripts/core/spike_terrain_source.gd
#  module_name: spike_terrain_source
#  version: 0.2.1
#  author: [BantedHam]
#  description: Drills the evaluated terrain source: RF byte layout the
#           uploader expects, determinism, real relief, and the seam
#           check that matters most, abutting patches evaluating their
#           shared edge to identical heights.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_terrain_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [terrain-source-drill, seam-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_terrain_source.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Source := preload("res://scripts/core/engine_terrain_source.gd")

var passed := 0
var failed := 0

# =========================================================================
# CHECK
# =========================================================================
func _check(name: String, ok: bool) -> void:
	if ok:
		passed += 1
		print("  ok   %s" % name)
	else:
		failed += 1
		print("  FAIL %s" % name)

# =========================================================================
# PATCH
# =========================================================================
func _patch(ox: float, oz: float) -> Dictionary:
	return {"texture_width": 17, "texture_height": 17,
		"world_origin_x": ox, "world_origin_z": oz,
		"world_size_x": 64.0, "world_size_z": 64.0}

# =========================================================================
# INIT
# =========================================================================
func _init() -> void:
	var a := Source.height_bytes(_patch(0.0, 0.0))
	_check("bytes are one RF float per texel", a.size() == 17 * 17 * 4)
	_check("deterministic", a == Source.height_bytes(_patch(0.0, 0.0)))

	var floats := a.to_float32_array()
	var lo := INF
	var hi := -INF
	for v in floats:
		lo = minf(lo, v)
		hi = maxf(hi, v)
	_check("the field put real relief in", hi - lo > 0.01)

	# The seam: a neighbour starting where this patch ends must evaluate
	# the shared edge to the identical heights.
	var east := Source.height_bytes(_patch(64.0, 0.0)).to_float32_array()
	var seam_ok := true
	for row in 17:
		var edge := floats[row * 17 + 16]
		var neighbour_edge := east[row * 17 + 0]
		if edge != neighbour_edge:
			seam_ok = false
	_check("abutting patches share the edge exactly", seam_ok)

	var degenerate := Source.height_bytes({"texture_width": 1, "texture_height": 1})
	_check("degenerate patch refuses instead of dividing by zero",
		degenerate.is_empty())

	# THE DEFORMATION COMPOSITE. An undeformed sim payload (fabricated
	# from the baseline itself) must return the pure field; a sculpted
	# texel must draw its deviation on top of the field and leave every
	# other texel untouched.
	var pd := _patch(0.0, 0.0)
	var baseline := Source.baseline_texels(0.0, 0.0, 64.0, 64.0, 17, 17)
	var pure := Source.height_bytes(pd)
	var flat_sim := baseline.to_byte_array()
	_check("undeformed sim payload draws the pure field",
		Source.composite_bytes(pd, flat_sim) == pure)
	var sculpted := baseline.duplicate()
	sculpted[8 * 17 + 8] += 5.0
	var comp := Source.composite_bytes(pd, sculpted.to_byte_array()).to_float32_array()
	var pure_f := pure.to_float32_array()
	_check("a sculpt draws its full depth where the sim holds it",
		absf((comp[8 * 17 + 8] - pure_f[8 * 17 + 8]) - 5.0) < 0.001)
	var untouched_ok := true
	for i in comp.size():
		if i != 8 * 17 + 8 and absf(comp[i] - pure_f[i]) > 0.0001:
			untouched_ok = false
	_check("the sculpt bleeds nowhere else", untouched_ok)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_terrain_source", passed, failed)
	quit(1 if failed > 0 else 0)
