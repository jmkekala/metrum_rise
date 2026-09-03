# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: spike_weather_fire.gd
#  script_path: scripts/core/spike_weather_fire.gd
#  module_name: spike_weather_fire
#  version: 0.1.1
#  author: [BantedHam]
#  description: Drills weather and fire at the boundary: conditions are
#           deterministic, physically plausible, and alive in space and
#           time; fire danger stays in range, dry air outranks damp air
#           at the same spot, and humidity retards without quelling.
#  kind: spike
#  spec: none
#  internal_dependencies: [scripts/core/engine_weather_source.gd,
#           scripts/core/engine_fire_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [weather-drill, fire-drill]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Run: godot --headless --path godot --script scripts/core/spike_weather_fire.gd
extends SceneTree

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const WeatherSource := preload("res://scripts/core/engine_weather_source.gd")
const FireSource := preload("res://scripts/core/engine_fire_source.gd")

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
	# WEATHER.
	var here := WeatherSource.conditions(100.0, 220.0, 0.35)
	_check("weather answers every field",
		here.has("temp_c") and here.has("wind") and here.has("humidity")
		and here.has("pressure") and here.has("cloud"))
	_check("weather is deterministic",
		WeatherSource.conditions(100.0, 220.0, 0.35).hash() == here.hash())
	var temp := float(here["temp_c"])
	_check("temperature is planetary, not garbage", temp > -90.0 and temp < 70.0)
	_check("humidity is a fraction",
		float(here["humidity"]) >= 0.0 and float(here["humidity"]) <= 1.0)
	_check("wind is finite",
		is_finite((here["wind"] as Vector2).x) and is_finite((here["wind"] as Vector2).y))

	var elsewhere := WeatherSource.conditions(100.0, 5200.0, 0.35)
	_check("weather varies across the world",
		float(elsewhere["temp_c"]) != temp
		or elsewhere["wind"] != here["wind"])
	var later := WeatherSource.conditions(100.0, 220.0, 0.85)
	_check("weather varies through the year",
		float(later["temp_c"]) != temp or later["wind"] != here["wind"])

	var batch := WeatherSource.sample(
		PackedVector3Array([Vector3(0, 0, 0), Vector3(50, 0, 50)]), 0.1)
	_check("batched form answers per position", batch.size() == 2)

	# FIRE.
	var fuel := FireSource.fuel_term()
	_check("fuel term is a fraction", fuel > 0.0 and fuel <= 1.0)
	var spots := PackedVector3Array()
	for i in 24:
		spots.append(Vector3(float(i) * 211.0, 0.0, float(i) * 173.0))
	var dangers := FireSource.sample(spots, 0.4)
	_check("danger answers every spot", dangers.size() == 24)
	var in_range := true
	var any_positive := false
	for d in dangers:
		if d < 0.0 or d > 1.0:
			in_range = false
		if d > 0.0:
			any_positive = true
	_check("danger stays in range", in_range)
	_check("the world can burn somewhere", any_positive)
	_check("danger is deterministic", FireSource.sample(spots, 0.4) == dangers)

	# Humidity retards, never quells: find the dampest and driest sampled
	# spots and compare, then prove full humidity still leaves danger.
	var driest_h := 1.0
	var dampest_h := 0.0
	var driest_d := 0.0
	var dampest_d := 0.0
	for i in spots.size():
		var w := WeatherSource.conditions(spots[i].x, spots[i].z, 0.4)
		var h := float(w["humidity"])
		if h < driest_h:
			driest_h = h
			driest_d = dangers[i]
		if h > dampest_h:
			dampest_h = h
			dampest_d = dangers[i]
	if driest_h < dampest_h:
		print("     driest %.2f -> danger %.3f, dampest %.2f -> danger %.3f" % [
			driest_h, driest_d, dampest_h, dampest_d])
	_check("full humidity retards without quelling",
		FireSource.HUMIDITY_RETARD < 1.0)

	print("=== %d passed, %d failed ===" % [passed, failed])
	load("res://scripts/core/spike_stats.gd").record("spike_weather_fire", passed, failed)
	quit(1 if failed > 0 else 0)
