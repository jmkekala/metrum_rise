# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_weather_source.gd
#  script_path: scripts/core/engine_weather_source.gd
#  module_name: engine_weather_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Weather at any point and moment, straight off the engine's
#           weather node: temperature, wind, humidity, pressure, and
#           cloud attenuation, one seed, batched for the boundary like
#           everything else. The game asks what the sky is doing and the
#           engine's circulation answers; nothing is scripted.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/weather_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [weather-conditions, batched, deterministic]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## What the sky is doing, per position, per moment.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Weather := preload("res://addons/2.5D_engine/evaluator/weather_node.gd")

## The world's one weather seed; the shell can thread the save's seed
## through when it wires the tick.
const WEATHER_SEED := 0x5EA50

static var _cfg: Dictionary = {}

# =========================================================================
# CONFIG
# =========================================================================
static func _config() -> Dictionary:
	if _cfg.is_empty():
		_cfg = Weather.default_config()
	return _cfg


# =========================================================================
# CONDITIONS
# =========================================================================
## Everything the game asks the sky, in one call.
static func conditions(x: float, z: float, t: float) -> Dictionary:
	var cfg := _config()
	return {
		"temp_c": Weather.temperature_at(x, z, t, WEATHER_SEED, cfg),
		"wind": Weather.wind_at(x, z, t, WEATHER_SEED, cfg),
		"humidity": Weather.humidity_at(x, z, t, WEATHER_SEED, cfg),
		"pressure": Weather.pressure_at(x, z, t, WEATHER_SEED, cfg),
		"cloud": Weather.cloud_attenuation(x, z, t, WEATHER_SEED, cfg),
	}


# =========================================================================
# SAMPLE
# =========================================================================
## The boundary's batched form: one array of positions, one array of
## condition dictionaries, once per tick.
static func sample(positions: PackedVector3Array, t: float) -> Array[Dictionary]:
	var out: Array[Dictionary] = []
	for p in positions:
		out.append(conditions(p.x, p.z, t))
	return out
