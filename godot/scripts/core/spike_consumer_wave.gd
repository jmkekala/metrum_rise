# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_consumer_wave.gd
#  script_path: scripts/core/spike_consumer_wave.gd
#  module_name: spike_consumer_wave
#  version: 0.1.1
#  author: [BantedHam]
#  description: Drills the outbreak, snow, map, and flora sources: the
#           epidemic grows while susceptibles last and passes herd
#           immunity honestly, snow answers deterministically and knows
#           seasons where the climate has them, projections round-trip
#           to the cartographer's precision, and a plot grows the same
#           plant twice.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_outbreak_source.gd,
#           scripts/core/engine_snow_source.gd,
#           scripts/core/engine_map_source.gd,
#           scripts/core/engine_flora_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [consumer-wave-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_consumer_wave.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Outbreak := preload("res://scripts/core/engine_outbreak_source.gd")
const Snow := preload("res://scripts/core/engine_snow_source.gd")
const MapSource := preload("res://scripts/core/engine_map_source.gd")
const Flora := preload("res://scripts/core/engine_flora_source.gd")

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
	# OUTBREAK.
	var town := Outbreak.begin(10000, 5)
	_check("the outbreak begins with the town standing",
		float(town["susceptible"]) == 9995.0 and float(town["infected"]) == 5.0)
	var day7 := town
	for i in 7:
		day7 = Outbreak.advance(day7, 1.0)
	_check("sickness spreads while susceptibles last",
		float(day7["infected"]) > float(town["infected"]))
	var f := Outbreak.forecast(day7, 10000)
	_check("the forecast answers the health service",
		float(f["r0"]) > 0.0 and float(f["herd_threshold"]) > 0.0
		and float(f["herd_threshold"]) < 1.0)
	var burned := day7
	for i in 400:
		burned = Outbreak.advance(burned, 1.0)
	_check("an epidemic ends", float(burned["infected"]) < float(day7["infected"]))
	print("     day 7 infected %.0f; after a year %.2f" % [
		day7["infected"], burned["infected"]])

	# SNOW.
	var spots := PackedVector3Array([Vector3(0, 0, 0), Vector3(0, 0, 8000)])
	var winter := Snow.densities(spots, 0.02)
	var summer := Snow.densities(spots, 0.52)
	_check("snow answers every spot", winter.size() == 2 and summer.size() == 2)
	_check("snow is deterministic", Snow.densities(spots, 0.02) == winter)
	_check("pack density is never negative",
		winter[0] >= 0.0 and winter[1] >= 0.0 and summer[0] >= 0.0 and summer[1] >= 0.0)
	var pack := Snow.pack_at(0.0, 8000.0, 0.02)
	_check("the pack states its climate",
		pack.has("mean_temp_k") and float(pack["mean_temp_k"]) > 150.0)

	# MAP.
	_check("the cartographer publishes projections",
		not MapSource.projections().is_empty())
	var lat := 0.62
	var lon := -1.9
	var on_map := MapSource.to_map(lat, lon)
	var back := MapSource.from_map(on_map)
	_check("the projection round-trips",
		absf(back.x - lat) < 1.0e-6 and absf(back.y - lon) < 1.0e-6)
	_check("area distortion is published", MapSource.area_scale(lat) > 0.0)

	# FLORA.
	var plot := Vector3(120, 0, -40)
	var a := Flora.grown_at(plot)
	_check("a plot grows a plant", not a.is_empty())
	_check("the same plot grows the same plant", Flora.grown_at(plot) == a)
	var b := Flora.grown_at(Vector3(900, 0, 900))
	_check("another plot grows its own plant", b != a)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_consumer_wave", passed, failed)
	quit(1 if failed > 0 else 0)
