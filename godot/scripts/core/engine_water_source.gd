# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_water_source.gd
#  script_path: scripts/core/engine_water_source.gd
#  module_name: engine_water_source
#  version: 0.2.1
#  author: [BantedHam]
#  description: Water's evaluated depth source: depth is what the water
#           level leaves above the SAME evaluated ground the terrain
#           source draws, so water sits in the field's own basins rather
#           than in authored fills. Byte-compatible with the renderer's
#           RF depth upload, seam-exact by the same endpoint-inclusive
#           convention, behind the same workflow toggle.
#  kind: module
#  spec: none
#  internal_dependencies: [scripts/core/engine_terrain_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [evaluated-depth, basin-coherent, toggle]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Depth = max(0, level - ground), over the evaluated ground.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const EngineTerrainSource := preload("res://scripts/core/engine_terrain_source.gd")

## Where the water table sits in the field's own units. The relief runs
## roughly plus-minus the terrain amplitude around zero, so a slightly
## negative level floods only the deeper basins.
const WATER_LEVEL_M := -1.5

const EngineTideSource := preload("res://scripts/core/engine_tide_source.gd")

## The tide's clock, advanced by engine_tick. Zero means still water.
static var tide_time := 0.0

# =========================================================================
# ENABLED
# =========================================================================
static func enabled() -> bool:
	return EngineTerrainSource.enabled()


# =========================================================================
# LEVEL M
# =========================================================================
## The level the world floods to right now: the still level breathing
## with the tide.
static func level_m() -> float:
	return WATER_LEVEL_M + EngineTideSource.level_offset_m(0.0, tide_time)


# =========================================================================
# DEPTH BYTES
# =========================================================================
## RF float32 depth bytes for one patch: the terrain source's own heights,
## inverted into depth below the level. Identical sampling, identical
## seams, one field for both renderers. When the patch carries the sim's
## ground testimony, the depth composites sculpts and earthworks exactly
## as the terrain draws them, so the shoreline follows a dug shore.
static func depth_bytes(patch_data: Dictionary) -> PackedByteArray:
	var testimony: PackedByteArray = patch_data.get("ground_bytes", PackedByteArray())
	var ground: PackedByteArray
	if testimony.is_empty():
		ground = EngineTerrainSource.height_bytes(patch_data)
	else:
		ground = EngineTerrainSource.composite_bytes(patch_data, testimony)
	if ground.is_empty():
		return ground
	var floats := ground.to_float32_array()
	# Ground bytes are stored units; the level joins them there.
	var level := level_m() / EngineTerrainSource.RENDER_HEIGHT_SCALE
	for i in floats.size():
		floats[i] = maxf(0.0, level - floats[i])
	return floats.to_byte_array()
