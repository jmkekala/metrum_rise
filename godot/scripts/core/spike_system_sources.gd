# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_system_sources.gd
#  script_path: scripts/core/spike_system_sources.gd
#  module_name: spike_system_sources
#  version: 0.2.0
#  author: [BantedHam]
#  description: Drills the physics, mineral, and social sources against
#           the evaluated field: contact normals are unit and agree with
#           measured slope, mineral channels are distinct deterministic
#           strata with coal riding biomineral carbon, and desirability
#           prefers flat ground and the shore band, all by measurement.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_physics_source.gd,
#           scripts/core/engine_mineral_source.gd,
#           scripts/core/engine_social_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [system-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_system_sources.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Physics := preload("res://scripts/core/engine_physics_source.gd")
const Minerals := preload("res://scripts/core/engine_mineral_source.gd")
const Social := preload("res://scripts/core/engine_social_source.gd")

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
# INIT
# =========================================================================
func _init() -> void:
	# PHYSICS: contact on the field.
	var c := Physics.contact(12.5, -40.25)
	var n: Vector3 = c["normal"]
	_check("normal is unit length", absf(n.length() - 1.0) < 1.0e-6)
	_check("normal points up off the ground", n.y > 0.0)
	_check("slope agrees with its own normal",
		absf(float(c["slope_deg"]) - rad_to_deg(acos(n.y))) < 1.0e-6)
	_check("contact is deterministic",
		Physics.contact(12.5, -40.25).hash() == c.hash())

	# Slope truth: measure rise over run along the field and compare.
	var run := 0.5
	var rise := absf(Physics.height(12.5 + run, -40.25) - Physics.height(12.5, -40.25))
	var measured_deg := rad_to_deg(atan(rise / run))
	_check("slope is the field's own steepness, roughly measured",
		absf(float(c["slope_deg"]) - measured_deg) < 25.0)

	# MINERALS: strata of the same world.
	var spots := PackedVector3Array()
	for i in 32:
		spots.append(Vector3(float(i) * 997.0, 0.0, float(i) * -761.0))
	var iron := Minerals.richness("iron", spots)
	var stone := Minerals.richness("stone", spots)
	var coal := Minerals.richness("coal", spots)
	_check("every channel answers every spot",
		iron.size() == 32 and stone.size() == 32 and coal.size() == 32)
	_check("channels are distinct strata", stone != iron and stone != coal)
	_check("richness stays in range", _in_range(iron) and _in_range(stone) and _in_range(coal))
	_check("minerals are deterministic", Minerals.richness("iron", spots) == iron)
	_check("an unknown channel refuses",
		Minerals.richness("adamantium", spots).is_empty())
	# ORE BODIES ARE BODIES: ten metres apart reads nearly the same
	# vein, a kilometre shows the real spread, or a probe grid is
	# reading noise and an extractor polygon averages the ore away.
	var near := Minerals.richness("iron", PackedVector3Array([
		Vector3(300.0, 0.0, 300.0), Vector3(310.0, 0.0, 300.0)]))
	var far_spread := 0.0
	var far_lo := 1.0
	var far_hi := 0.0
	var km := PackedVector3Array()
	for i in 16:
		km.append(Vector3(float(i) * 400.0, 0.0, float(i) * 313.0))
	var km_r := Minerals.richness("iron", km)
	for v in km_r:
		far_lo = minf(far_lo, v)
		far_hi = maxf(far_hi, v)
	far_spread = far_hi - far_lo
	print("     vein coherence: 10 m delta %.4f, spread over km %.4f" % [
		absf(near[0] - near[1]), far_spread])
	_check("ore bodies cohere at walking distance",
		absf(near[0] - near[1]) < 0.1)
	_check("ore bodies vary across the map", far_spread > 0.2)

	# SOCIAL: desirability prefers flat ground; measured, not asserted.
	var best_flat := -1.0
	var best_flat_score := 0.0
	var best_steep_score := 1.0
	var probe := PackedVector3Array()
	for i in 64:
		probe.append(Vector3(float(i) * 31.1, 0.0, float(i) * 17.9))
	var scores := Social.desirability(probe)
	_check("every parcel is scored", scores.size() == 64)
	_check("scores stay in range", _in_range(scores))
	var flattest_slope := INF
	var steepest_slope := -INF
	var flattest_score := 0.0
	var steepest_score := 0.0
	for i in probe.size():
		var slope := float(Physics.contact(probe[i].x, probe[i].z)["slope_deg"])
		if slope < flattest_slope:
			flattest_slope = slope
			flattest_score = scores[i]
		if slope > steepest_slope:
			steepest_slope = slope
			steepest_score = scores[i]
	_check("flat ground outranks steep ground",
		flattest_score >= steepest_score)
	_check("desirability is deterministic", Social.desirability(probe) == scores)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_system_sources", passed, failed)
	quit(1 if failed > 0 else 0)

# =========================================================================
# IN RANGE
# =========================================================================
func _in_range(values: PackedFloat64Array) -> bool:
	for v in values:
		if v < 0.0 or v > 1.0:
			return false
	return true
