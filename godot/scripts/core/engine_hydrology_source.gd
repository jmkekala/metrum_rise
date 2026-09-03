# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_hydrology_source.gd
#  script_path: scripts/core/engine_hydrology_source.gd
#  module_name: engine_hydrology_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Standing water and flooding from the engine's own
#           hydrology and flood nodes: where water stands, how deep,
#           whether it is ocean, and how a basin fills toward
#           equilibrium or overflows under an inflow. The economy reads
#           these; the nodes own what water does.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/hydrology_node.gd,
#           addons/2.5D_engine/evaluator/flood_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [standing-water, flood-dynamics, batched]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Where the water is and what it is doing.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Hydrology := preload("res://addons/2.5D_engine/evaluator/hydrology_node.gd")
const Flood := preload("res://addons/2.5D_engine/evaluator/flood_node.gd")
const EngineTerrainSource := preload("res://scripts/core/engine_terrain_source.gd")

## One water seed with the world's under it.
static var _seed := 0x4A0 ^ EngineTerrainSource.FIELD_SEED


# =========================================================================
# WATER AT
# =========================================================================
## Standing water at a spot: depth, whether any stands, whether ocean.
static func water_at(x: float, z: float) -> Dictionary:
	return {
		"depth_m": Hydrology.water_depth_at(x, z, _seed),
		"standing": Hydrology.has_standing_water(x, z, _seed),
		"ocean": Hydrology.is_ocean(x, z, _seed),
	}


# =========================================================================
# BATCHED, THE BOUNDARY'S WAY
# =========================================================================
## Batched, the boundary's way.
static func depths(positions: PackedVector3Array) -> PackedFloat64Array:
	var out := PackedFloat64Array()
	out.resize(positions.size())
	for i in positions.size():
		out[i] = Hydrology.water_depth_at(positions[i].x, positions[i].z, _seed)
	return out


# =========================================================================
# BASIN FORECAST
# =========================================================================
## A basin under an inflow: the level it settles at and whether it
## overflows, straight from the flood node's reservoir.
static func basin_forecast(inflow: float) -> Dictionary:
	return {
		"equilibrium_level": Flood.equilibrium_level(inflow),
		"overflows": Flood.overflows(inflow),
	}


# =========================================================================
# BASIN STEP
# =========================================================================
## One step of a filling basin, for the sim's own tick.
static func basin_step(volume: float, inflow: float, dt: float) -> float:
	return Flood.step(volume, inflow, dt)
