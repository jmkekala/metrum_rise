# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_mineral_source.gd
#  script_path: scripts/core/engine_mineral_source.gd
#  module_name: engine_mineral_source
#  version: 0.3.0
#  author: [BantedHam]
#  description: Mineral richness from the engine's own geology, replacing
#           the salted-noise stand-in the first version was: strata_node
#           says which depositional environment tops each spot, the
#           environment's rock formula says what it is made of (iron is
#           element 26 in basalt's own formula), density says how it
#           quarries, and biomineral's carbon accounting keeps owning
#           coal where burial preserves. Veins modulate within what the
#           geology allows; nothing is placed by hand.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/strata_node.gd,
#           addons/2.5D_engine/evaluator/biomineral_node.gd,
#           addons/2.5D_engine/evaluator/fbm_node.gd,
#           scripts/core/engine_terrain_source.gd]
#  external_dependencies: [Godot 4.x]
#  features: [strata-environments, formula-iron, density-stone,
#           biomineral-coal, veins]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## Richness in [0, 1] per channel, from the geology that is actually there.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Strata := preload("res://addons/2.5D_engine/evaluator/strata_node.gd")
const Biomineral := preload("res://addons/2.5D_engine/evaluator/biomineral_node.gd")
const Fbm := preload("res://addons/2.5D_engine/evaluator/fbm_node.gd")
const EngineTerrainSource := preload("res://scripts/core/engine_terrain_source.gd")

const CHANNELS := ["iron", "stone", "coal"]
## Atomic number of iron, which is how a rock formula names it.
const ELEMENT_FE := 26
## Environments where burial preserves organics, coal's precondition.
const BURIAL_ENVIRONMENTS := ["deep_marine", "fluvial"]
## Vein detail rides the world seed at a coarse footprint.
const VEIN_FOOTPRINT := 0.25
const VEIN_SALT := 0x7E1A
## Metres of world per unit of vein field: the size of an ore body,
## authored game taste like the settlement weights. Raw metres put a
## whole vein in every metre of rock, so a probe grid read noise and
## an extractor polygon averaged it away; at this scale a discovered
## body is worth walking an extractor over to.
const VEIN_SCALE_M := 250.0

static var _seed := EngineTerrainSource.FIELD_SEED


# =========================================================================
# ELEMENT FRACTION
# =========================================================================
## The fraction of a formula that is one element, by atom count.
static func _element_fraction(formula: Dictionary, element: int) -> float:
	var total := 0.0
	var hits := 0.0
	for k in formula:
		total += float(formula[k])
		if int(k) == element:
			hits += float(formula[k])
	return hits / total if total > 0.0 else 0.0

# =========================================================================
# VEIN
# =========================================================================
static func _vein(p: Vector3) -> float:
	var v := Fbm.evaluate(p.x / VEIN_SCALE_M, p.y / VEIN_SCALE_M,
		p.z / VEIN_SCALE_M, VEIN_FOOTPRINT / VEIN_SCALE_M, 0.0,
		_seed ^ VEIN_SALT)
	return clampf(v * 0.5 + 0.5, 0.0, 1.0)

# =========================================================================
# RICHNESS
# =========================================================================
static func richness(channel: String, positions: PackedVector3Array) -> PackedFloat64Array:
	var out := PackedFloat64Array()
	if channel not in CHANNELS:
		return out
	var envs := Strata.environments()
	out.resize(positions.size())
	for i in positions.size():
		var p := positions[i]
		var env_name := Strata.top_environment(p.x, p.z, _seed)
		var env: Dictionary = envs.get(env_name, {})
		var vein := _vein(p)
		var r := 0.0
		match channel:
			"iron":
				# What the rock is made of decides what it yields: basalt's
				# own formula carries iron; a limestone yields traces.
				var fe := _element_fraction(env.get("formula", {}), ELEMENT_FE)
				r = clampf(fe * 8.0, 0.0, 1.0) * vein
			"stone":
				# Denser rock quarries better; the densities are the
				# environments' own.
				var density := float(env.get("density", 0.0))
				r = clampf(density / 2900.0, 0.0, 1.0) * (0.5 + 0.5 * vein)
			"coal":
				# Coal is buried carbon: only where burial preserves, and
				# scaled by biomineral's own carbon accounting.
				if env_name in BURIAL_ENVIRONMENTS:
					var locked := Biomineral.carbon_locked_kg(vein * 1000.0)
					r = clampf(locked / 120.0, 0.0, 1.0)
		out[i] = r
	return out
