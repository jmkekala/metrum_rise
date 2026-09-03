# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_tide_mind_water.gd
#  script_path: scripts/core/spike_tide_mind_water.gd
#  module_name: spike_tide_mind_water
#  version: 0.2.1
#  author: [BantedHam]
#  description: Drills the tide, mind, and hydrology sources: the tide is
#           semidiurnal with the orbital period and a sane amplitude,
#           the mind roster runs living instances deterministically (two
#           same-seed minds walk identically) with a parched mind
#           choosing water and water resolving the thirst that chose
#           it, and standing water is deterministic with flood basins
#           settling to equilibrium without overflowing under small
#           inflows.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_tide_source.gd,
#           scripts/core/engine_mind_source.gd,
#           scripts/core/engine_hydrology_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [tide-drill, mind-drill, hydrology-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_tide_mind_water.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Tide := preload("res://scripts/core/engine_tide_source.gd")
const MindSource := preload("res://scripts/core/engine_mind_source.gd")
const Hydro := preload("res://scripts/core/engine_hydrology_source.gd")

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
	# TIDES.
	var period := Tide.period_s()
	_check("the moon has a real month", period > 20.0 * 86400.0 and period < 40.0 * 86400.0)
	var bulge := Tide.bulge_m()
	print("     bulge %.3f m, period %.1f days" % [bulge, period / 86400.0])
	_check("the bulge is tidal, not oceanic", bulge > 0.1 and bulge < 2.0)
	var high := Tide.level_offset_m(0.0, 0.0)
	var low := Tide.level_offset_m(TAU * 0.125, 0.0)
	_check("high tide at the bulge, lower off it", high > low)
	var opposite := Tide.level_offset_m(PI, 0.0)
	_check("semidiurnal: the far side is also high", absf(opposite - high) < 1.0e-9)
	var later := Tide.level_offset_m(0.0, period * 0.25)
	_check("the tide turns as the moon moves", later != high)
	_check("a full orbit comes home",
		absf(Tide.level_offset_m(0.0, period) - high) < 0.01)

	# MINDS: living instances on the finished contract. The policy
	# table is gone with the layer that produced it; wanting is utility
	# and connectome dynamics, so the drill asserts on lives, not rows.
	MindSource.clear()
	MindSource.spawn(1, 91, Vector2(4.0, 4.0))
	MindSource.spawn(2, 91, Vector2(4.0, 4.0))
	_check("the roster holds its minds", MindSource.count() == 2)
	for i in 3000:
		MindSource.tick_all(1.0 / 60.0)
	var s1 := MindSource.state(1)
	var s2 := MindSource.state(2)
	_check("a mind's state answers", not s1.is_empty()
		and s1.has("pos") and s1.has("behavior") and s1.has("hunger"))
	print("     mind 1: %s at %s, thirst %.4f, fatigue %.4f" % [
		String(s1["behavior"]), str(s1["pos"]),
		float(s1["thirst"]), float(s1["fatigue"])])
	_check("two same-seed minds walk identically",
		s1["pos"] == s2["pos"] and s1["thirst"] == s2["thirst"]
		and s1["behavior"] == s2["behavior"])
	# A parched mind chooses water and only water resolves thirst; the
	# thirst is set directly, exactly as the engine's own spike does,
	# because steering is a function of thirst, not of how long thirst
	# took to arrive.
	MindSource.spawn(3, 91, Vector2(4.0, 4.0))
	MindSource._minds[3].thirst = 0.95
	var before: int = MindSource._minds[3].decision_log.size()
	for i in 3000:
		MindSource.tick_all(1.0 / 60.0)
	var chose_water := false
	for d in MindSource._minds[3].decision_log.slice(before):
		if String(d[1]) == "seek_water":
			chose_water = true
	var s3 := MindSource.state(3)
	_check("a parched mind chooses water", chose_water)
	# Thirst only accumulates on its own; anything below where it was
	# set is drinking, and a resolved thirst is far below it.
	_check("water resolves the thirst that chose it", float(s3["thirst"]) < 0.5)
	MindSource.despawn(3)
	_check("despawn forgets the mind",
		MindSource.count() == 2 and MindSource.state(3).is_empty())
	MindSource.clear()

	# HYDROLOGY.
	var spots := PackedVector3Array([Vector3(0, 0, 0), Vector3(400, 0, -900),
		Vector3(-2500, 0, 1300)])
	var d1 := Hydro.depths(spots)
	_check("depths answer every spot", d1.size() == 3)
	_check("hydrology is deterministic", Hydro.depths(spots) == d1)
	var at := Hydro.water_at(400.0, -900.0)
	_check("standing agrees with depth",
		bool(at["standing"]) == (float(at["depth_m"]) > 0.0))

	# 0.01 is genuinely small against the node's drain (0.02 m2 at 0.6
	# discharge over a 0.15 m overflow); 0.1 already floods it.
	var calm := Hydro.basin_forecast(0.01)
	_check("a small inflow settles without overflowing",
		not bool(calm["overflows"]) and float(calm["equilibrium_level"]) >= 0.0)
	var v := 0.0
	for i in 200:
		v = Hydro.basin_step(v, 0.01, 1.0)
	_check("stepping approaches the forecast equilibrium",
		absf(Flood_level(v) - float(calm["equilibrium_level"])) <
		maxf(0.15 * float(calm["equilibrium_level"]), 0.05))

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_tide_mind_water", passed, failed)
	quit(1 if failed > 0 else 0)

# =========================================================================
# FLOOD LEVEL
# =========================================================================
func Flood_level(volume: float) -> float:
	return preload("res://addons/2.5D_engine/evaluator/flood_node.gd").level_of(volume)
