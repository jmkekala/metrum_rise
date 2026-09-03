# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_social_source.gd
#  script_path: scripts/core/engine_social_source.gd
#  module_name: engine_social_source
#  version: 0.2.0
#  author: [BantedHam]
#  description: Desirability derived from the same evaluated ground
#           everything else reads: flat land scores, steep land costs,
#           and standing on the waterline earns the basin bonus. The
#           social simulation samples this batched at parcel positions
#           through the boundary; no parcel is scored by hand.
#  kind: module
#  spec: none
#  internal_dependencies: [scripts/core/engine_physics_source.gd,
#           scripts/core/engine_water_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [derived-desirability, batched]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Desirability in [0, 1], batched at parcel positions.
##
## The engine's habitability node is the authority on whether a place
## sustains life (its score reads moisture under the prevailing wind and
## thermal growth); buildability is the city's own concern layered on
## top, because a cliff can be habitable to goats and still impossible
## to found a street on.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Habitability := preload("res://addons/2.5D_engine/evaluator/habitability_node.gd")
const EnginePhysicsSource := preload("res://scripts/core/engine_physics_source.gd")
const EngineWeatherSource := preload("res://scripts/core/engine_weather_source.gd")
const EngineTerrainSource := preload("res://scripts/core/engine_terrain_source.gd")

## Slope where buildability reaches zero.
const SLOPE_CEILING_DEG := 30.0

## The city's settlement taste over the engine's biome suitabilities:
## authored game weights over engine facts. Grasslands and temperate
## lands found cities; deserts and rainforests resist them. The engine
## answers per-biome suitability and a winner; a scalar "score" key
## never existed, and reading one scored every parcel 0.0 until the
## windowed run printed the mean and said so.
const SETTLEMENT_WEIGHTS := {
	"grassland": 1.0, "temperate": 1.0, "wet_temperate": 0.8,
	"semi_arid": 0.5, "boreal": 0.4, "rainforest": 0.25, "arid": 0.15,
}

static var _seed := EngineTerrainSource.FIELD_SEED

# =========================================================================
# DESIRABILITY
# =========================================================================
static func desirability(positions: PackedVector3Array, t: float = 0.0) -> PackedFloat64Array:
	var out := PackedFloat64Array()
	out.resize(positions.size())
	for i in positions.size():
		var p := positions[i]
		var w := EngineWeatherSource.conditions(p.x, p.z, t)
		var score := Habitability.score_at(p.x, p.z, _seed, w["wind"] as Vector2)
		# Settlement suitability: the weighted mean of the biome
		# suitabilities, normalised by their own mass so a place that is
		# strongly one biome reads as that biome's welcome.
		var weighted := 0.0
		var mass := 0.0
		for biome in SETTLEMENT_WEIGHTS:
			var s := float(score.get(biome, 0.0))
			weighted += s * float(SETTLEMENT_WEIGHTS[biome])
			mass += s
		var habitable := clampf(weighted / mass, 0.0, 1.0) if mass > 0.0 else 0.0
		var c := EnginePhysicsSource.contact(p.x, p.z)
		var buildable := clampf(1.0 - float(c["slope_deg"]) / SLOPE_CEILING_DEG, 0.0, 1.0)
		out[i] = clampf(habitable * buildable, 0.0, 1.0)
	return out
