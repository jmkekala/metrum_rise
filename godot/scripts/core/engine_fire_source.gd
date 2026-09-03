# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_fire_source.gd
#  script_path: scripts/core/engine_fire_source.gd
#  module_name: engine_fire_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Fire danger from the engine's own Rothermel timings and
#           the weather source's wind and humidity: fast ignition, fast
#           spread, and dry air raise it; humidity RETARDS it and never
#           quells it, which is the engine's wet-retards law spoken at
#           the boundary. The economy reads danger; the engine's fire
#           model owns what fire IS.
#  kind: module
#  spec: Rothermel timings from fire_node, weather from weather_node
#  internal_dependencies: [addons/2.5D_engine/evaluator/fire_node.gd,
#           scripts/core/engine_weather_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [fire-danger, wind-driven, wet-retards]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Danger in [0, 1], per position, per moment.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Fire := preload("res://addons/2.5D_engine/evaluator/fire_node.gd")
const EngineWeatherSource := preload("res://scripts/core/engine_weather_source.gd")

## Wind speed that doubles the danger's wind term.
const WIND_DOUBLING_MPS := 8.0
## How hard full humidity retards. Never 1.0: wet retards a fire, it
## does not quell it.
const HUMIDITY_RETARD := 0.75

static var _cfg: Dictionary = {}

# =========================================================================
# CONFIG
# =========================================================================
static func _config() -> Dictionary:
	if _cfg.is_empty():
		_cfg = Fire.default_config()
	return _cfg


# =========================================================================
# FUEL TERM
# =========================================================================
## The fuel's own eagerness to burn, from the model's ignition and burn
## timings alone: quick to light and long to burn is dangerous fuel.
static func fuel_term() -> float:
	var cfg := _config()
	var ignition := maxf(Fire.ignition_time_s(cfg), 0.001)
	var burn := maxf(Fire.burn_time_s(cfg), 0.001)
	# Seconds-to-ignite against a minute: 10 s to light is danger 0.86;
	# ten minutes to light is danger 0.09.
	var quickness := 1.0 / (1.0 + ignition / 60.0)
	var endurance := clampf(burn / (burn + 300.0), 0.0, 1.0)
	return clampf(0.5 * quickness + 0.5 * endurance, 0.0, 1.0)


# =========================================================================
# DANGER
# =========================================================================
## Danger at a spot and moment: fuel, fanned by the weather's wind,
## retarded by its humidity.
static func danger(x: float, z: float, t: float) -> float:
	var w := EngineWeatherSource.conditions(x, z, t)
	var wind_speed := (w["wind"] as Vector2).length()
	var wind_term := 1.0 + wind_speed / WIND_DOUBLING_MPS
	var humidity := clampf(float(w["humidity"]), 0.0, 1.0)
	var retard := 1.0 - HUMIDITY_RETARD * humidity
	return clampf(fuel_term() * wind_term * retard * 0.5, 0.0, 1.0)


# =========================================================================
# THE BOUNDARY'S BATCHED FORM
# =========================================================================
## The boundary's batched form.
static func sample(positions: PackedVector3Array, t: float) -> PackedFloat64Array:
	var out := PackedFloat64Array()
	out.resize(positions.size())
	for i in positions.size():
		out[i] = danger(positions[i].x, positions[i].z, t)
	return out
