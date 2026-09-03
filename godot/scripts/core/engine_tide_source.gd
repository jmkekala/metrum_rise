# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_tide_source.gd
#  script_path: scripts/core/engine_tide_source.gd
#  module_name: engine_tide_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Tides from the engine's orbital machinery and published
#           physics: a moon on a circular orbit built with orbital_node,
#           its period from circular_period, and the equilibrium tidal
#           bulge H = (3/4) * (Mm/Mp) * (Rp/d)^3 * Rp from standard
#           tidal theory, swept semidiurnally by the moon's angle. The
#           sea level the water source floods to breathes with it.
#  kind: module
#  spec: equilibrium tide over orbital_node circular orbits
#  internal_dependencies: [addons/2.5D_engine/evaluator/orbital_node.gd,
#           addons/2.5D_engine/evaluator/sealevel_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [tides, semidiurnal, orbital-period]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## The shoreline's clock.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Orbital := preload("res://addons/2.5D_engine/evaluator/orbital_node.gd")
const Sealevel := preload("res://addons/2.5D_engine/evaluator/sealevel_node.gd")

## The world's bodies, Earthlike by default; the save can rewrite them.
const PLANET_MASS_KG := 5.972e24
const PLANET_RADIUS_M := 6.371e6
const MOON_MASS_KG := 7.342e22
const MOON_DISTANCE_M := 3.844e8

## How many world units of shoreline rise one metre of equilibrium bulge
## drives. The bulge is ~0.54 m for these bodies; coastal amplification
## makes the visible tide, and this is that one honest dial.
const COASTAL_GAIN := 2.5


# =========================================================================
# BULGE M
# =========================================================================
## The equilibrium bulge height for the configured bodies, in metres:
## (3/4) * (Mm/Mp) * (Rp/d)^3 * Rp, standard tidal theory.
static func bulge_m() -> float:
	var ratio := MOON_MASS_KG / PLANET_MASS_KG
	var cubed := pow(PLANET_RADIUS_M / MOON_DISTANCE_M, 3.0)
	return 0.75 * ratio * cubed * PLANET_RADIUS_M


# =========================================================================
# PERIOD S
# =========================================================================
## The moon's period around these bodies, straight from the engine.
static func period_s() -> float:
	return Orbital.circular_period(PLANET_MASS_KG, MOON_DISTANCE_M)


# =========================================================================
# LEVEL OFFSET M
# =========================================================================
## The tide's offset to sea level at a longitude angle and moment:
## semidiurnal, two bulges opposite each other, swept by the moon.
static func level_offset_m(lon_rad: float, t_s: float) -> float:
	var moon_angle := TAU * fmod(t_s / period_s(), 1.0)
	return bulge_m() * COASTAL_GAIN * cos(2.0 * (lon_rad - moon_angle))


# =========================================================================
# SEA LEVEL M
# =========================================================================
## Sea level with the tide on it, riding sealevel_node's own budget for
## the still-water part.
static func sea_level_m(lon_rad: float, t_s: float, ice_mass: float = 0.0,
		ocean_temp_k: float = 288.0) -> float:
	return Sealevel.sea_level_m(ice_mass, ocean_temp_k) \
		+ level_offset_m(lon_rad, t_s)
