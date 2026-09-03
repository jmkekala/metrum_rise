# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_map_source.gd
#  script_path: scripts/core/engine_map_source.gd
#  module_name: engine_map_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: The map view's projection, from the cartography node:
#           world positions project through a named real projection and
#           unproject back, with its area distortion published so the
#           overview can say what it stretches. The minimap draws on a
#           cartographer's math, not an eyeballed scale.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/cartography_node.gd,
#           addons/2.5D_engine/evaluator/surface_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [projection, round-trip, area-scale]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## The overview's cartographer.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Cartography := preload("res://addons/2.5D_engine/evaluator/cartography_node.gd")

## The projection the overview uses; any name from projections() works.
static var projection := "equirectangular"

# =========================================================================
# PROJECTIONS
# =========================================================================
static func projections() -> Array:
	return Cartography.projections()


# =========================================================================
# A WORLD LAT/LON ONTO THE MAP PLANE
# =========================================================================
## A world lat/lon onto the map plane.
static func to_map(lat_rad: float, lon_rad: float) -> Vector2:
	return Cartography.project(projection, lat_rad, lon_rad)


# =========================================================================
# A MAP POINT BACK TO LAT/LON
# =========================================================================
## A map point back to lat/lon.
static func from_map(p: Vector2) -> Vector2:
	return Cartography.unproject(projection, p)


# =========================================================================
# AREA SCALE
# =========================================================================
## How much the map stretches area at a latitude, for honest legends.
static func area_scale(lat_rad: float) -> float:
	return Cartography.area_scale(projection, lat_rad)
