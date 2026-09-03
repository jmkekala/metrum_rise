# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_flora_source.gd
#  script_path: scripts/core/engine_flora_source.gd
#  module_name: engine_flora_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Grown plants for agriculture and flora: the plant node's
#           own growth from a seed at an origin, cached by position and
#           seed so a field of crops is one growth per plot, ready for
#           the freeze-and-instance path the buildings already ride.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/plant_node.gd,
#           scripts/core/engine_terrain_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [grown-plants, cached]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## What grows where.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Plant := preload("res://addons/2.5D_engine/evaluator/plant_node.gd")
const EngineTerrainSource := preload("res://scripts/core/engine_terrain_source.gd")

static var _cache: Dictionary = {}


# =========================================================================
# GROWN AT
# =========================================================================
## The plant grown at a plot, the node's own growth, cached per plot.
static func grown_at(origin: Vector3, salt: int = 0) -> Dictionary:
	var key := "%d|%d|%d|%d" % [int(origin.x), int(origin.y), int(origin.z), salt]
	if _cache.has(key):
		return _cache[key]
	var p_seed := EngineTerrainSource.FIELD_SEED ^ (key.hash() & 0x7FFFFFFF)
	var grown := Plant.grow(origin, p_seed)
	_cache[key] = grown
	return grown
