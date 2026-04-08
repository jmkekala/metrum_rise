# Demand

This document owns the city's coarse growth pressure (WIP)

It answers questions like:

- should the city admit more households
- should the city lose households
- is there pressure for more residential, commercial, or industrial capacity
- is the city healthy enough to support future private development

It does not own household data layout, freight movement, or door-to-door travel.

## Ownership

The intended ownership split is:

- `docs/demand.md`: city-level pressure, immigration and emigration pressure, and future building-growth pressure
- `docs/economy.md`: household/company/building economy state once those entities already exist
- `docs/entrance_and_exit.md`: transport semantics only, if a household move is visualized as a border-origin trip

So immigration should not be primarily owned by the entrance/exit redesign, and it should not be primarily owned by the building-entry system either.

## Immigration And Emigration

Immigration and emigration are demand-layer decisions.

For `v0.1`, the decision unit is the household, not the individual resident.

This document owns:

- whether the city should admit any new households today
- how many households may be admitted today
- whether household outflow pressure should rise
- which coarse pressure signals drive those outcomes

This document does not own:

- the exact `Household` runtime record
- the exact home-claim procedure once a household is admitted
- whether arrival is visualized through a border-origin transport trip

## Core Rule

Demand decides whether immigration should happen.

Economy creates and owns the admitted household record.

Buildings and vacancy rules decide whether a real home can be claimed.

Transport may optionally visualize arrival, but transport does not decide city growth.

## Inputs

The immigration and emigration pressure model should be derived from coarse city signals such as:

- vacant resident capacity
- job availability
- household cost pressure
- household stock stability
- commute burden
- service quality
- broader city stability
- existence of at least one external connection when external immigration is required

These are city-level signals, not per-agent trip decisions.

## Outputs

The demand layer should produce bounded daily outputs such as:

- `households_to_admit_today`
- `households_to_remove_today` or equivalent emigration pressure
- residential growth pressure for future building creation systems

Those outputs are then consumed by economy and building systems.

## Deterministic `v0.1` Rules

For `v0.1`, the immigration rules should stay simple and deterministic:

- evaluate immigration and emigration on a coarse daily cadence
- if there is no valid housing capacity, admit `0` households
- if there is no required external connection, admit `0` households
- admit whole households only
- the number admitted per day must be bounded
- the result should come from coarse pressure signals, not from a hidden bootstrap path or a transport-state side effect

If a household is admitted:

- economy creates the household record
- housing/vacancy logic claims a real home
- transport may either instantiate the household directly at home or visualize a border-origin arrival

Both outcomes are valid, but they are downstream of the demand decision.

## Relationship To Building Growth

Residential growth and immigration should stay coupled through demand, not through hidden automatic spawning.

That means:

- immigration should not ignore housing limits
- future residential building creation should not ignore household demand
- zoning alone is not enough to force either households or buildings to appear

The long-term clean model is:

- demand produces residential pressure
- that pressure can justify both household admission and future private residential development
- buildings, economy, and transport each consume that pressure in their own layer

## Legacy Cleanup Targets

The current codebase still has some transitional immigration logic outside the demand layer.

These paths should be removed, narrowed, or rewritten so immigration ownership matches this document.

- `rust/src/simulation/buildings/allocator/lifecycle.rs::spawn_immigrants()` still owns the live immigration pressure formula, daily household count, and admission gating. The coarse decision logic currently lives there through:
  - `IMMIGRATION_BASE_INFLOW`
  - `MAX_IMMIGRANT_HOUSEHOLDS_PER_DAY`
  - `PLAYER_STARTUP_POPULATION_TARGET`
  - the in-function `housing_factor`, `job_factor`, and `city_stability_factor` calculation
  - this should move behind a demand-owned output such as `households_to_admit_today`
- The connected-border requirement is currently enforced directly inside `spawn_immigrants()` by scanning `NodeType::Border`. That requirement is a city-growth admission rule and should ultimately be expressed through demand-owned immigration gating rather than hidden allocator-local policy.
- `rust/src/simulation/economy/agents/data.rs::spawn_agent()` still defaults new agents to `TRANSIT_IMMIGRATING`. That default is transport-oriented legacy behavior and should not be the generic constructor path for ordinary household admission.
- The generic agent-spawn API still mixes ordinary housed admission, border-origin transport bootstrap, and test/helper setup into one constructor shape. That should be split into explicit paths such as:
  - ordinary housed-agent creation for demand-driven household admission
  - optional border-origin transport visualization
  - test/helper spawning
  - ordinary demand-driven household admission must not depend on a constructor that defaults to `TRANSIT_IMMIGRATING`
- `rust/src/simulation/economy/agents/tick.rs::plan_immigration_trip()` and the `TRANSIT_IMMIGRATING` branch are acceptable only as optional transport-layer visualization for exceptional/manual border-origin arrivals. They must not remain a hidden prerequisite for normal city growth.
- The coarse immigration decision should move fully into a demand-owned daily output. The allocator/economy layer should consume something like `households_to_admit_today` rather than recomputing immigration pressure locally during `spawn_immigrants()`.
- Building-loss displacement currently has no dedicated demand/economy ownership boundary. `AgentSystem::evict_building()` still forces some agents into `TRANSIT_ACCESS_INGRESS` as a fake rubble/street fallback. That should be replaced by an explicit rehousing, homelessness, disaster, or removal contract rather than by reusing ordinary entrance-travel semantics.
- Debug logging and tooling should stop implying that immigration is fundamentally a border-spawn FSM process when the real source of truth is the demand-layer household-admission decision.

The target end state is:

- demand computes whether immigration should happen and how many households may be admitted
- economy creates the admitted household records
- housing/vacancy logic claims real homes
- transport either visualizes the move or does nothing, but it does not decide growth
