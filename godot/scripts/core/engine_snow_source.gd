# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_snow_source.gd
#  script_path: scripts/core/engine_snow_source.gd
#  module_name: engine_snow_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Winters from the snowpack node under the same weather
#           seed the sky already uses: the pack state at any spot and
#           moment, batched for the boundary, so roads slow and roofs
#           whiten where and when the model says snow lies.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/snowpack_node.gd,
#           scripts/core/engine_weather_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [snowpack, batched]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Where the snow lies.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Snowpack := preload("res://addons/2.5D_engine/evaluator/snowpack_node.gd")
const EngineWeatherSource := preload("res://scripts/core/engine_weather_source.gd")


# =========================================================================
# PACK AT
# =========================================================================
## The pack at one spot and moment, on the weather's own seed.
static func pack_at(x: float, z: float, t: float) -> Dictionary:
	return Snowpack.pack_state(x, z, t, EngineWeatherSource.WEATHER_SEED)


# =========================================================================
# DENSITIES
# =========================================================================
## Batched pack densities, the boundary's way: the node states the pack
## as density, crusts, and wetness rather than a single depth, and
## density in kilograms per cubic metre is the load a roof and a road
## actually care about.
static func densities(positions: PackedVector3Array, t: float) -> PackedFloat64Array:
	var out := PackedFloat64Array()
	out.resize(positions.size())
	for i in positions.size():
		var pack := pack_at(positions[i].x, positions[i].z, t)
		out[i] = float(pack.get("density", 0.0))
	return out
