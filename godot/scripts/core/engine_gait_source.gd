# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_gait_source.gd
#  script_path: scripts/core/engine_gait_source.gd
#  module_name: engine_gait_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Walk phase from the gait node's own offsets: each walker
#           carries a phase in INSTANCE_CUSTOM already, and this derives
#           it from speed and the gait's published leg timing instead of
#           an authored cycle. First step of the actor path; the frozen
#           forms animate from it when the shell wires the per-instance
#           write.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/gait_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [gait-phase, per-instance]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## The walk's clock, from the gait's own timing.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Gait := preload("res://addons/2.5D_engine/evaluator/gait_node.gd")

## Stride length at walking speed, metres per cycle.
const STRIDE_M := 1.4


# =========================================================================
# OFFSETS
# =========================================================================
## The gait node's leg phase offsets, verbatim.
static func offsets() -> Dictionary:
	return Gait.gait_offsets()


# =========================================================================
# PHASE
# =========================================================================
## The walk phase in [0, 1) for a walker that has covered `distance_m`:
## one cycle per stride, which ties the feet to the ground covered
## rather than to a timer that slides.
static func phase(distance_m: float) -> float:
	return fposmod(distance_m / STRIDE_M, 1.0)


# =========================================================================
# LEG PHASE
# =========================================================================
## A leg's own phase, the walker's phase plus the gait's offset for it.
static func leg_phase(distance_m: float, gait_name: String, leg_index: int) -> float:
	var offs := offsets()
	var gait: Variant = offs.get(gait_name)
	if gait == null:
		return phase(distance_m)
	var legs = gait
	if legs is Array and leg_index >= 0 and leg_index < (legs as Array).size():
		return fposmod(phase(distance_m) + float((legs as Array)[leg_index]), 1.0)
	return phase(distance_m)
