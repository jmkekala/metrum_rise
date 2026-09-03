# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_outbreak_source.gd
#  script_path: scripts/core/engine_outbreak_source.gd
#  module_name: engine_outbreak_source
#  version: 0.1.0
#  author: [BantedHam]
#  description: Disease for the health services, from the contagion
#           node's own epidemiology: reproduction numbers, the herd
#           immunity threshold, and the outbreak stepped day by day
#           through its SIR state. The city's sickness is a model, not
#           a random event.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/contagion_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [outbreak, sir-step, herd-immunity]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## The city's epidemiology.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Contagion := preload("res://addons/2.5D_engine/evaluator/contagion_node.gd")


# =========================================================================
# BEGIN
# =========================================================================
## A fresh outbreak: one seed case in a susceptible town, in the node's
## own state keys.
static func begin(population: int, seed_cases: int = 1) -> Dictionary:
	var s := maxi(population - seed_cases, 0)
	return {"susceptible": float(s), "infected": float(seed_cases),
		"recovered": 0.0}


# =========================================================================
# ADVANCE
# =========================================================================
## One tick of sickness, on the node's own step.
static func advance(state: Dictionary, days: float) -> Dictionary:
	return Contagion.step(state, days)


# =========================================================================
# FORECAST
# =========================================================================
## What the health service plans against.
static func forecast(state: Dictionary, population: int) -> Dictionary:
	var pop := maxf(float(population), 1.0)
	var susceptible := float(state.get("susceptible", 0.0)) / pop
	return {
		"r0": Contagion.basic_reproduction(),
		"r_effective": Contagion.effective_reproduction(susceptible),
		"herd_threshold": Contagion.herd_immunity_threshold(),
		"infected_fraction": float(state.get("infected", 0.0)) / pop,
	}
