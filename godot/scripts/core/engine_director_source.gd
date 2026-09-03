# =========================================================================
#  MANIFEST
# =========================================================================
#  script_name: engine_director_source.gd
#  script_path: scripts/core/engine_director_source.gd
#  module_name: engine_director_source
#  version: 0.2.0
#  author: [BantedHam]
#  description: The engine's Director paces the city: gateway traffic
#           feeds its intensity, its state machine steps build-up,
#           peak, and relax on the node's own rules with the seeded
#           variety walk (the chain moves only where the thresholds
#           leave a state free, so two seeds pace differently and a
#           replay is bit-identical), and what it hands back are the
#           multipliers the simulation consumes: how much population
#           pressure, how strong the next shock. The city's drama is
#           directed, not scripted.
#  kind: module
#  spec: none
#  internal_dependencies: [addons/2.5D_engine/evaluator/director_node.gd]
#  external_dependencies: [Godot 4.x]
#  features: [pacing, intensity, event-fed]
#  api_version: metrum-v1.0.0
#  last_updated: 2026-08-30
# =========================================================================

## The city's dramatist.
@tool
extends RefCounted

# =========================================================================
# THE DECLARATIONS
# =========================================================================

const Director := preload("res://addons/2.5D_engine/evaluator/director_node.gd")

static var intensity := 0.0
static var state := "relax"
static var seconds_in_state := 0.0
## The variety walk is a pure function of (seed, tick), like every
## field in the engine; the city's director walks under one seed.
const DIRECTOR_SEED := 0xD1EC
static var tick_index := 0


# =========================================================================
# STEP
# =========================================================================
## Feed one tick of observed events (the gateway's drained batch) and
## step the pacing, translated into the director's own dialect: an
## event's severity is damage taken, a crisis with a distance is a
## nearby death, and any traffic at all means the city is engaged so
## intensity holds instead of decaying. Quiet decays it. The dwell
## clock resets whenever the machine moves.
static func step(events: Array, dt: float) -> Dictionary:
	var damage := 0.0
	var deaths: Array = []
	for e in events:
		var ev := e as Dictionary
		if ev == null:
			continue
		var data := ev.get("data", {}) as Dictionary
		damage += float(data.get("severity", 0.1))
		if data.has("distance"):
			deaths.append(float(data["distance"]))
	var observed := {
		"damage_taken": damage,
		"nearby_deaths": deaths,
		"engaged": not events.is_empty(),
	}
	intensity = Director.step_intensity(intensity, observed, dt)
	# The varied step: thresholds rule, the seeded chain fills the
	# freedom they leave, so two cities pace differently by seed alone.
	tick_index += 1
	var next := Director.step_state_varied(state, intensity,
		seconds_in_state, DIRECTOR_SEED, tick_index)
	if next == state:
		seconds_in_state += dt
	else:
		state = next
		seconds_in_state = 0.0
	return status()


# =========================================================================
# STATUS
# =========================================================================
## What the simulation reads: the state, its intensity, and the node's
## own multipliers for population pressure and shock strength.
static func status(base_population: int = 100, base_strength: float = 1.0) -> Dictionary:
	return {
		"state": state,
		"intensity": intensity,
		"population": Director.population_for_state(state, base_population),
		"threat_strength": Director.threat_strength(state, base_strength),
	}

# =========================================================================
# RESET
# =========================================================================
static func reset() -> void:
	intensity = 0.0
	state = "relax"
	seconds_in_state = 0.0
	tick_index = 0
