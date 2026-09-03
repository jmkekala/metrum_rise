# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_mind_source.gd
#  script_path: scripts/core/engine_mind_source.gd
#  module_name: engine_mind_source
#  version: 0.2.0
#  author: [BantedHam]
#  description: Living minds from the engine's mind node: one instance
#           per steered body, ticked on the node's own fixed step, each
#           carrying a certified creature, its drives, and its decision
#           log. The policy table this source once cached was removed
#           upstream with the layer it certified; wanting is utility and
#           connectome dynamics now, and this roster speaks that
#           contract. The actor path is the consumer that mounts bodies
#           on these minds when it lands.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/mind_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [mind-roster, fixed-step, decision-logs]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-31
# =========================================================================

## Living minds, one instance per steered body.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Mind := preload("res://addons/2.5D_engine/evaluator/mind_node.gd")

## Real seconds per mind tick belong to the node, not to this roster.
## Minds run on real time; how world-time scaling maps onto biological
## clocks is the actor path's decision when it mounts bodies.
static var _minds: Dictionary = {}
static var _accum := 0.0

## A stalled frame never spirals the fixed-step loop: at most this many
## mind ticks catch up per call, the rest of the debt is dropped.
const MAX_STEPS_PER_CALL := 240


# =========================================================================
# SPAWN
# =========================================================================
## Add one mind to the roster. Config keys are the node's own
## (thirst_rate, drink_height, connectome, ...); empty means defaults.
static func spawn(id: int, mind_seed: int, at: Vector2, config: Dictionary = {}) -> void:
	_minds[id] = Mind.new(mind_seed, at, config)

# =========================================================================
# DESPAWN
# =========================================================================
static func despawn(id: int) -> void:
	_minds.erase(id)

# =========================================================================
# CLEAR
# =========================================================================
static func clear() -> void:
	_minds.clear()
	_accum = 0.0

# =========================================================================
# COUNT
# =========================================================================
static func count() -> int:
	return _minds.size()


# =========================================================================
# TICK ALL
# =========================================================================
## Advance every mind on the node's fixed step; real time accumulates
## here so callers can hand in frame deltas. Returns ticks advanced.
static func tick_all(delta: float) -> int:
	if _minds.is_empty():
		_accum = 0.0
		return 0
	_accum += delta
	var steps := 0
	while _accum >= Mind.DT and steps < MAX_STEPS_PER_CALL:
		_accum -= Mind.DT
		steps += 1
		for id in _minds:
			_minds[id].tick()
	if steps >= MAX_STEPS_PER_CALL:
		_accum = 0.0
	return steps


# =========================================================================
# STATE
# =========================================================================
## One mind's readable state: where its body is, what it is doing, and
## what it wants. Empty when the id is unknown.
static func state(id: int) -> Dictionary:
	var m = _minds.get(id)
	if m == null:
		return {}
	return {
		"pos": m.creature.pos,
		"behavior": m.behavior,
		"thirst": m.thirst,
		"fatigue": m.fatigue,
		"hunger": m.hunger(),
		"decisions": m.decision_log.size(),
	}
