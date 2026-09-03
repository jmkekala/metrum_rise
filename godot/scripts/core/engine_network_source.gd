# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_network_source.gd
#  script_path: scripts/core/engine_network_source.gd
#  module_name: engine_network_source
#  version: 0.2.0
#  author: [BantedHam]
#  description: Roads on the evaluated ground: chunk vertices from the
#           Rust road mesh re-heighted to the SAME field the terrain and
#           water sources evaluate, plus a constant deck offset, so
#           roads lie on the evaluated surface instead of the payload
#           heightmap. Same workflow toggle, Rust heights untouched as
#           the fallback.
#  kind: module
#  spec: none
#  internal_dependencies: [scripts/core/engine_terrain_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [conformed-roads, toggle]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Road vertices conformed to the evaluated field.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const EngineTerrainSource := preload("res://scripts/core/engine_terrain_source.gd")
const Fbm := preload("res://addons/2.5D_engine/evaluator/fbm_node.gd")

## How far the deck rides above the ground it follows.
const DECK_OFFSET_M := 0.15

# =========================================================================
# ENABLED
# =========================================================================
static func enabled() -> bool:
	return EngineTerrainSource.enabled()


# =========================================================================
# GROUND HEIGHT
# =========================================================================
## The terrain source's one ground function, one position at a time, so
## the scale can never fork between the deck and the ground it rides.
static func ground_height(wx: float, wz: float) -> float:
	return EngineTerrainSource.ground_m(wx, wz)


# =========================================================================
# CONFORM HEIGHTS
# =========================================================================
## Re-height every vertex onto the evaluated ground. X and Z stay the
## road's own; only the ride height changes hands.
static func conform_heights(vertices: PackedVector3Array) -> PackedVector3Array:
	var out := PackedVector3Array()
	out.resize(vertices.size())
	for i in vertices.size():
		var v := vertices[i]
		out[i] = Vector3(v.x, ground_height(v.x, v.z) + DECK_OFFSET_M, v.z)
	return out
