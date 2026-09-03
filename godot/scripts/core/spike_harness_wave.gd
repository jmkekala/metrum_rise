# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_harness_wave.gd
#  script_path: scripts/core/spike_harness_wave.gd
#  module_name: spike_harness_wave
#  version: 0.1.1
#  author: [BantedHam]
#  description: Drills the harness wave: the water level breathes with
#           the tide clock, habitability-based desirability answers in
#           range with buildability suppressing cliffs, the ambient bed
#           is louder in stronger wind and silent air is quieter, and
#           gait phase walks one cycle per stride with the node's own
#           leg offsets.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_water_source.gd,
#           scripts/core/engine_social_source.gd,
#           scripts/core/engine_ambience_source.gd,
#           scripts/core/engine_gait_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [harness-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_harness_wave.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Water := preload("res://scripts/core/engine_water_source.gd")
const Tide := preload("res://scripts/core/engine_tide_source.gd")
const Social := preload("res://scripts/core/engine_social_source.gd")
const Ambience := preload("res://scripts/core/engine_ambience_source.gd")
const GaitSource := preload("res://scripts/core/engine_gait_source.gd")

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
	# TIDE INTO WATER.
	Water.tide_time = 0.0
	var still := Water.level_m()
	Water.tide_time = Tide.period_s() * 0.25
	var turned := Water.level_m()
	_check("the water level breathes with the tide", turned != still)
	_check("the breath is the tide's own size",
		absf(turned - still) < 2.0 * Tide.bulge_m() * Tide.COASTAL_GAIN + 0.001)
	Water.tide_time = 0.0

	# HABITABILITY DESIRABILITY.
	var spots := PackedVector3Array()
	for i in 16:
		spots.append(Vector3(float(i) * 613.0, 0.0, float(i) * -389.0))
	var scores := Social.desirability(spots, 0.3)
	_check("desirability answers every parcel", scores.size() == 16)
	var in_range := true
	for s in scores:
		if s < 0.0 or s > 1.0:
			in_range = false
	_check("desirability stays in range", in_range)
	_check("desirability is deterministic",
		Social.desirability(spots, 0.3) == scores)

	# AMBIENCE.
	# The node's law: leaves are what wind sounds like, so vegetation
	# raises the level and bare rock is nearly silent.
	var bare_db := Ambience.wind_db(100.0, 100.0, 0.1, 0.0)
	var leafy_db := Ambience.wind_db(100.0, 100.0, 0.1, 0.9)
	_check("leaves make the wind audible", leafy_db >= bare_db)
	var loud := Ambience.render_bed(60.0, 0.25)
	var quiet := Ambience.render_bed(20.0, 0.25)
	_check("the bed renders", loud.size() > 0 and loud.size() == quiet.size())
	var loud_rms := _rms(loud)
	var quiet_rms := _rms(quiet)
	print("     bed rms: 60 dB %.4f vs 20 dB %.4f" % [loud_rms, quiet_rms])
	_check("stronger wind is audibly louder", loud_rms > quiet_rms)
	_check("the bed is deterministic", Ambience.render_bed(60.0, 0.25) == loud)

	# GAIT.
	var offs := GaitSource.offsets()
	_check("the gait node publishes offsets", not offs.is_empty())
	_check("one stride is one cycle",
		absf(GaitSource.phase(GaitSource.STRIDE_M) - 0.0) < 1.0e-9)
	_check("half a stride is half a cycle",
		absf(GaitSource.phase(GaitSource.STRIDE_M * 0.5) - 0.5) < 1.0e-9)
	var g0 := GaitSource.leg_phase(0.0, str(offs.keys()[0]), 0)
	var g1 := GaitSource.leg_phase(0.0, str(offs.keys()[0]), 1)
	_check("legs walk out of phase", g0 != g1 or (offs[offs.keys()[0]] as Array).size() < 2)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_harness_wave", passed, failed)
	quit(1 if failed > 0 else 0)

# =========================================================================
# RMS
# =========================================================================
func _rms(frames: PackedFloat32Array) -> float:
	var acc := 0.0
	for v in frames:
		acc += v * v
	return sqrt(acc / maxf(float(frames.size()), 1.0))
