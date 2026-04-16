# Demand

This document owns the city's coarse growth pressure and the demand-driven growth system:
household admission and removal, private building spawn and despawn pressure, and building upgrade
or downgrade pressure. The baseline `v0.1` ownership path is now live, while later extensions in
this document remain work in progress.

It answers questions like:

- should the city admit more households
- should the city lose households
- is there pressure for more residential, commercial, or industrial capacity
- should private buildings appear, disappear, upgrade, or downgrade
- is the city healthy enough to support future private development

It does not own household data layout, freight movement, or door-to-door travel.

## Cross-Doc Implementation Order

This document is intended to be implemented after the profile-based zoning and asset-editor
foundation in [`zoning.md`](zoning.md), and before the deeper economy-side integration work in
[`economy.md`](economy.md).

Cross-doc order:

1. implement the profile-based zoning and asset-editor changes from [`zoning.md`](zoning.md)
2. implement the demand-owned growth, admission, removal, and building-change rules from this
   document
3. finish the deeper economy-side signal production, viability gates, and runtime handoff work in
   [`economy.md`](economy.md)

This keeps legality and authoring stable first, then moves city-growth ownership into demand, and
only after that finishes the economy-side runtime details that demand consumes.

## Current Runtime Snapshot

The live game now has a larger Rust-side `DemandSystem`. The baseline `v0.1` ownership path
described here is implemented, while broader extensions described later in this document remain
future work.

Current live behavior:

- `DemandSystem` loads and validates the shipped `demand/growth_profiles.toml` tuning file at
  startup
- it computes the baseline city-level `ResidentialGrowth`, `CommercialGrowth`, and
  `IndustrialGrowth` channels from the daily post-settlement snapshot
- it computes and persists `households_to_admit_today`,
  `households_to_remove_today`, and the carried household-action credits used by the admission or
  removal thresholds
- it also computes and persists the carried building-action credits for spawn, upgrade, downgrade,
  and despawn budgets across residential, commercial, and industrial use families
- the live runtime now executes ordinary household admission from the demand-owned
  `households_to_admit_today` output instead of recomputing immigration pressure inside the
  allocator
- the live runtime now executes ordinary household removal from the demand-owned
  `households_to_remove_today` output using the deterministic selection order owned by
  [`economy.md`](economy.md)
- the live runtime now reads household affordability, relocation, eviction, and `unhoused`
  outcomes from the settled daily economy pass before computing the next demand snapshot
- the live runtime now executes private building spawn, despawn, upgrade, and downgrade actions
  from demand-owned daily building-action plans instead of allocator-owned heuristics
- those building-action plans now also pass through economy-side viability gates backed by the
  authored `runtime_tuning` block in `economy/profiles.toml`
- industrial building actions now read explicit input-coverage and output-headroom signals from the
  live resource-typed building inventory state instead of treating industrial viability as pure
  staffing-plus-buffer approximation
- fresh-map startup now uses purely organic demand signals — the pioneer demand floor has been
  removed; the unemployment benefit system in [`economy.md`](economy.md) provides early-city
  solvency instead

Current derived inputs:

- housing capacity and vacancy
- housed-resident presence
- open and filled job capacity
- household affordability and stock stability
- connected external-border availability
- economy-side residential and non-residential viability gates from `economy/profiles.toml`
- industrial input coverage plus industrial output headroom from the live starter inventory slice

Short version:

- the current system now owns household-admission and removal pressure plus the daily admitted or
  removed-household counts
- it also now owns the city's ordinary private building-change decisions through daily action plans
- baseline fresh-map startup now flows through purely organic demand signals; no hidden
  allocator-owned founding path or pioneer floor remains

## Terminology Conventions

This document uses the following terms consistently:

- `current DemandSystem`: the live Rust system that now computes baseline `DemandChannel`s plus
  demand-owned daily household-admission or removal outputs plus ordinary private building-action
  plans, including the shipped startup-support path for fresh maps
- `target demand layer`: the full authoritative growth controller described by this document,
  including later extensions that are not all live yet
- `GrowthProfile`: authored tuning data consumed by the target demand layer; it is not a runtime output generated by demand
- `build site`: one candidate legal location where a private building could appear, disappear, upgrade, downgrade, or be replaced. The concrete frontage-attached runtime shape of a build site is owned by [`building_allocator.md`](building_allocator.md); this remains a gameplay term, not a cadastral parcel system
- `pioneer household growth`: historical term for the now-removed static demand floor that
  was applied to fresh maps; replaced by the unemployment benefit in
  [`economy.md`](economy.md), which provides early-city solvency through real economic activity

## Current Replacement Targets

Several important growth behaviors still live outside the current `DemandSystem`. These are
temporary ownership violations, not behaviors to preserve. The demand redesign should replace the
decision-making parts cleanly and remove the old paths rather than keep compatibility shims.

Current replacement targets:

- no additional demand-owned replacement targets are currently tracked at the top level; the
  remaining follow-up work is the transport-oriented admission and displacement cleanup documented
  later in this file

Replacement-first rule:

- once a demand-owned output exists for one of these behaviors, the old allocator- or
  transport-owned decision path should be deleted rather than left behind as a fallback

## Target Scope

The target demand layer is the sole owner of ongoing city-growth decisions.

Target responsibilities:

- immigration and emigration pressure at whole-household granularity
- bounded daily outputs such as `households_to_admit_today` and `households_to_remove_today`
- baseline `v0.1` private residential, commercial, and industrial spawn pressure
- later other private-use growth families only if they are added with explicit formulas and shipped
  profile data at the same time
- private building despawn, abandonment, downgrade, recovery, or replacement pressure where those systems exist
- building upgrade and downgrade pressure derived from sustained conditions rather than one-frame spikes

The intended rule is:

- demand decides whether growth or decline should happen
- economy creates and updates the household or building runtime state once that decision is made
- transport may visualize movement, but transport does not decide city growth
- no other runtime subsystem should independently decide household admission or removal, or
  private-building spawn, despawn, upgrade, or downgrade, except for explicit scenario/startup
  setup and allocator-owned integrity cleanup

Important scope guard:

- demand owns ongoing city-growth decisions
- explicit one-time scenario setup such as founding placement does not need to become a permanent
  demand feature; it only needs to stop being a hidden allocator-owned exception
- invalid geometry cleanup, broken road attachment cleanup, and other placement-integrity repair are
  allocator concerns, not demand outputs

## Growth Profiles

Growth evaluation data belongs in the demand layer, not in zoning structure.

`ZoneProfile` in [`zoning.md`](zoning.md) stays focused on what is legally allowed at a build
site. When a zoning choice needs custom growth behavior, it references a demand-owned
`GrowthProfile` through `growth_profile_id`.

Deterministic `v0.1` rule:

- `GrowthProfile` tunes one fixed private-building growth evaluator
- `GrowthProfile` does not define its own formula language or custom aggregation logic
- whole-city household admission and removal remain demand-owned daily outputs, not `GrowthProfile`
  outputs
- the shipped `GrowthProfile` set is intentionally small and closed in `v0.1`
- `v0.1` ships exactly one default `GrowthProfile` per shipped baseline private-use family and
  density
- baseline `v0.1` demand does not ship dedicated `OfficeGrowth` or `MixedGrowth` channels
- many `ZoneProfile`s may later reuse the same `GrowthProfile`
- adding a new `ZoneProfile` should normally reuse an existing `GrowthProfile`
- adding a brand-new `GrowthProfile` is a larger design change and is expected to remain rare

Canonical `v0.1` shape:

```text
DemandChannel
  = ResidentialGrowth | CommercialGrowth | IndustrialGrowth

GrowthProfile
  - id
  - demand_channel
  - cadence_days
  - base_pressure_weight
  - local_modifier_scale
  - local_modifier_weights   # optional in v0.1
  - spawn_threshold
  - despawn_threshold
  - upgrade_threshold
  - downgrade_threshold
  - hysteresis_margin
```

Interpretation:

- `demand_channel` selects exactly one city-level growth-pressure input for the profile
- `cadence_days` controls how often this profile is re-evaluated
- `base_pressure_weight` controls how strongly the chosen city-level demand channel contributes to
  the final growth score
- `local_modifier_scale` controls how strongly local desirability changes the final growth score
- `local_modifier_weights` optionally tells the demand system how strongly each local condition
  contributes to local desirability
- thresholds convert the final normalized score into spawn, despawn, upgrade, or downgrade
  eligibility
- `hysteresis_margin` keeps state changes stable and stops one-frame spikes from causing churn

Implemented `v0.1` local-modifier rule:

- shipped `v0.1` `GrowthProfile`s are allowed to omit `local_modifier_weights` entirely
- if a shipped `GrowthProfile` omits `local_modifier_weights`, then `local_desirability = 0.5`
  neutral and `local_modifier_scale` must be `0.0`
- this keeps baseline demand implementable without requiring local-modifier systems that are not yet
  trustworthy or complete

Local-modifier input rule:

- every local modifier consumed by a `GrowthProfile` must be normalized to `0.0..1.0`
- higher values must always mean "more favorable for private building growth" by the time the
  demand layer reads them
- if another subsystem stores a raw harmful quantity such as pollution or crime, that subsystem or
  the demand adapter layer must invert or remap it before it is fed into the fixed evaluator
- local modifiers are a later extension in `v0.1`, not a required dependency for baseline demand

Future local-modifier families may include:

- pollution
- noise
- crime
- education
- parks and public-space quality
- transit access
- utility or service stability
- commute burden
- broader neighborhood attractiveness

Deterministic fixed evaluator:

1. Read the city-level `demand_channel` pressure as `city_pressure` in `0.0..1.0`.
2. If `local_modifier_weights` is present and non-empty, read each referenced local modifier in
   `0.0..1.0` and compute `local_desirability` as the weighted average of those referenced
   modifiers.
3. Otherwise set `local_desirability = 0.5`.
4. Compute the final normalized score:

```text
final_growth_score =
    clamp(
        city_pressure * base_pressure_weight
      + (local_desirability - 0.5) * local_modifier_scale,
        0.0,
        1.0
    )
```

5. Compare `final_growth_score` against the profile thresholds:
   - empty legal site: `spawn_threshold`
   - existing building: `upgrade_threshold`, `downgrade_threshold`, `despawn_threshold`
6. Apply one fixed hysteresis rule:
   - once a state change becomes eligible, it should not flip back until the score crosses the same
     threshold by more than `hysteresis_margin`

Important boundary:

- crossing a spawn, despawn, upgrade, or downgrade threshold makes that state change eligible from
  the demand side
- it does not bypass zoning legality or the economy-side viability gate described in
  [`economy.md`](economy.md)

Important ownership rule:

- the underlying modifier values come from their own simulation systems
- the demand system reads those values
- `GrowthProfile` stores authored tuning for how demand uses them
- demand consumes `GrowthProfile`; it does not generate the profile itself at runtime

### GrowthProfile Data And Loading

`GrowthProfile`s are data-authored in shipped TOML files bundled with the game.

Canonical `v0.1` data shape:

```text
demand/
  growth_profiles.toml
  growth_profiles.index.bin   # optional derived cache
```

Canonical source-of-truth rules:

- `demand/growth_profiles.toml` is the authoritative authored data
- any compiled cache or index file is optional derived data only
- the base-game growth-profile set ships with the game and does not live in the user directory
- a dedicated growth-profile editor is not required for the first implementation
- hand-authored TOML is acceptable while the shipped profile set remains small and closed
- baseline `v0.1` does not load extra growth profiles from zoning files, scenario files, mods, or
  the user directory
- baseline `v0.1` does not generate runtime-only synthetic `GrowthProfile`s

Closed shipped `v0.1` profile set:

- `residential_low_default`
- `residential_medium_default`
- `residential_high_default`
- `commercial_low_default`
- `commercial_medium_default`
- `commercial_high_default`
- `industrial_low_default`
- `industrial_medium_default`
- `industrial_high_default`

Deterministic `v0.1` identity rule:

- the shipped base-game set contains exactly those 9 profile ids and no others
- every shipped id follows the stable naming rule `<zone_type>_<density>_default`
- there is exactly one shipped `GrowthProfile` for each shipped baseline `(zone_type, density)`
  pair
- there is no separate compiled numeric `GrowthProfile` id in baseline `v0.1`; the stable authored
  string id is the authoritative identity key

`v0.1` mapping rule:

- each shipped `ZoneProfile` from [`zoning/profiles.toml`](../zoning/profiles.toml) must reference
  the one default `GrowthProfile` that matches its shipped baseline `(zone_type, density)`
- baseline `v0.1` does not ship zoning-specific custom `GrowthProfile`s beyond that closed set
- baseline `v0.1` also does not ship office or mixed zoning profiles; those remain later explicit
  extensions if the design reintroduces them

Canonical `growth_profiles.toml` shape:

```toml
[signal_normalization]
resident_presence_saturation_residents = 500
household_affordability_target_reserve_days = 7.0
household_stock_stability_target_days = 3.0

[action_budget]
max_households_per_day = 48

[household_action]
base_inflow = 1.0
admission_threshold = 0.10
removal_threshold = 0.55

[action_budget.spawn_batch_fraction_by_use]
residential = 0.50
commercial = 0.40
industrial = 0.40

[action_budget.upgrade_batch_fraction_by_use]
residential = 0.25
commercial = 0.25
industrial = 0.25

[action_budget.downgrade_batch_fraction_by_use]
residential = 0.25
commercial = 0.25
industrial = 0.25

[action_budget.despawn_batch_fraction_by_use]
residential = 0.50
commercial = 0.50
industrial = 0.50

[[profiles]]
id = "residential_low_default"
demand_channel = "ResidentialGrowth"
cadence_days = 1
base_pressure_weight = 1.0
local_modifier_scale = 0.0
spawn_threshold = 0.55
despawn_threshold = 0.20
upgrade_threshold = 0.80
downgrade_threshold = 0.30
hysteresis_margin = 0.05
```

Deterministic validation rules:

- `signal_normalization.resident_presence_saturation_residents` must be finite and `> 0`
- `signal_normalization.household_affordability_target_reserve_days` must be finite and `> 0.0`
- `signal_normalization.household_stock_stability_target_days` must be finite and `> 0.0`
- `action_budget.max_households_per_day` must be a finite integer `>= 0`
- `household_action.base_inflow` must be finite and in `0.0..1.0`
- `household_action.admission_threshold` must be finite and in `0.0..1.0`
- `household_action.removal_threshold` must be finite and in `0.0..1.0`
- `action_budget.spawn_batch_fraction_by_use` must contain exactly `residential`, `commercial`,
  and `industrial`
- every `action_budget.spawn_batch_fraction_by_use.*` value must be finite and in `0.0..1.0`
- `action_budget.upgrade_batch_fraction_by_use` must contain exactly `residential`, `commercial`,
  and `industrial`
- every `action_budget.upgrade_batch_fraction_by_use.*` value must be finite and in `0.0..1.0`
- `action_budget.downgrade_batch_fraction_by_use` must contain exactly `residential`,
  `commercial`, and `industrial`
- every `action_budget.downgrade_batch_fraction_by_use.*` value must be finite and in `0.0..1.0`
- `action_budget.despawn_batch_fraction_by_use` must contain exactly `residential`, `commercial`,
  and `industrial`
- every `action_budget.despawn_batch_fraction_by_use.*` value must be finite and in `0.0..1.0`
- every `id` must be globally unique
- every shipped base-game `id` must belong to the closed 9-id `v0.1` set listed above
- every `demand_channel` must decode to a known `DemandChannel`
- every `cadence_days` must be an integer `>= 1`
- `base_pressure_weight`, `local_modifier_scale`, every threshold, and `hysteresis_margin` must be
  finite values in `0.0..1.0`
- shipped base-game `v0.1` profiles must set `local_modifier_scale = 0.0`
- shipped base-game `v0.1` profiles must omit `profiles.local_modifier_weights`
- if a future extension re-enables `profiles.local_modifier_weights`, the explicit supported key set
  must be documented here at the same time
- `upgrade_threshold` must be `>= downgrade_threshold`
- invalid shipped base-game growth profiles must fail validation explicitly rather than being
  silently dropped or rewritten

Deterministic runtime loading rules:

1. Read the shipped `demand/growth_profiles.toml` file during startup.
2. Validate the full file before creating the live growth-profile registry.
3. Validate that the shipped base-game file contains exactly one valid entry for each required
   shipped baseline `(zone_type, density)` pair and no extra base-game entries.
4. Build the runtime registry keyed by stable `GrowthProfile.id`.
5. Build the deterministic ordered profile list in this exact order:
   `residential_low_default`, `residential_medium_default`, `residential_high_default`,
   `commercial_low_default`, `commercial_medium_default`, `commercial_high_default`,
   `industrial_low_default`, `industrial_medium_default`, `industrial_high_default`.
6. Use that deterministic ordered list whenever the demand layer iterates over all shipped
   `GrowthProfile`s.
7. Validate every `ZoneProfile.growth_profile_id` reference from
   [`zoning/profiles.toml`](../zoning/profiles.toml) against that loaded registry.
8. Load and validate the one shipped `signal_normalization` table before any demand formula is
   evaluated.
9. Load and validate the one shipped `household_action` table before any household admission or
   removal formula is evaluated.
10. Load and validate the one shipped `action_budget` table before any household-action or
    building-action budget formula is evaluated.
11. In `v0.1`, enforce the one-default-profile-per-`(zone_type, density)` mapping rule for the
   shipped base-game profile set.

## Modifiers And Signal Sources

The demand layer should consume normalized signals from other systems. It should not become the
source of truth for those underlying systems.

Deterministic `v0.1` rule:

- shipped `v0.1` `GrowthProfile`s do not use local modifiers
- baseline `v0.1` building-growth evaluation therefore reads only city-level demand channels plus
  zoning, allocator, and economy viability gates
- local modifiers are a later extension and are not required for baseline demand behavior

Baseline `v0.1` city-level signal families:

- `housing_availability`
- `resident_presence`
- `job_availability`
- `household_affordability`
- `household_stock_stability`
- `utility_service_stability`
- `external_connection_available`
- `commercial_input_deficit` — fraction of active commercial buildings without a corresponding
  local industrial (farm/input) supplier; drives industrial spawn pressure independently of
  household `goods_shortage` (which OWA fallback suppresses)

Baseline ownership rule:

- `housing_availability` comes from housing capacity and vacancy state owned by economy/building
  systems
- `resident_presence` comes from occupied housing or admitted-household presence state owned by
  economy/building systems
- `job_availability` comes from labor demand and open-job state owned by economy/building systems
- `household_affordability` comes from household budgets and essential-cost state owned by economy
- `household_stock_stability` comes from household stock buffers owned by economy
- `utility_service_stability` comes from utility-service resolution owned by economy/utility systems
- `external_connection_available` comes from network-border connectivity owned by the road/network
  layer
- `commercial_input_deficit` is derived by the demand snapshot from the live building inventory:
  `clamp(1.0 - active_industrial_count / active_commercial_count, 0.0, 1.0)` when
  `active_commercial_count > 0`, otherwise `0.0`

Normalization rule:

- every signal consumed by demand must be normalized to `0.0..1.0` before it enters a demand
  formula
- higher values must always mean "more favorable for growth" by the time demand reads the signal
- if another system stores a harmful raw quantity such as pollution, noise, or crime, that source
  system or a thin adapter layer must invert or remap it before demand consumes it

Deterministic `v0.1` signal-normalization rules:

The baseline city-level signals are derived from the frozen post-settlement economy snapshot using
the following exact formulas.

Baseline derived economy values:

```text
vacant_household_slots =
    max(total_household_slots - occupied_household_slots, 0)

total_reachable_job_slots =
    occupied_reachable_job_slots + open_reachable_job_slots
```

Signal formulas:

```text
housing_availability =
    if total_household_slots == 0 then 0.0
    else clamp(vacant_household_slots / total_household_slots, 0.0, 1.0)

resident_presence =
    clamp(
        housed_resident_count / resident_presence_saturation_residents,
        0.0,
        1.0
    )

job_availability =
    if total_reachable_job_slots == 0 then 0.0
    else clamp(open_reachable_job_slots / total_reachable_job_slots, 0.0, 1.0)

household_affordability =
    if housed_household_count == 0 then 1.0
    else average_over_housed_households(
        clamp(
            household_reserve_days
            / household_affordability_target_reserve_days,
            0.0,
            1.0
        )
    )

household_stock_stability =
    if housed_household_count == 0 then 1.0
    else average_over_housed_households(
        clamp(
            household_stock_days
            / household_stock_stability_target_days,
            0.0,
            1.0
        )
    )

utility_service_stability =
    min(power_service_ratio, water_service_ratio, sewage_service_ratio)

external_connection_available =
    if connected_border_count > 0 then 1.0 else 0.0
```

Interpretation and source rule:

- `housing_availability` uses settled household-slot capacity after the daily economy pass
- `resident_presence` uses housed residents, not raw map population targets
- `job_availability` uses open reachable jobs after the settled daily viability and staffing state
- `household_affordability` uses settled economy-owned `household_reserve_days` values from
  [`economy.md`](economy.md)
- `household_stock_stability` uses settled economy-owned `household_stock_days` values
- `utility_service_stability` uses settled service-satisfaction ratios in `0.0..1.0`; if a
  specific utility dimension is not implemented yet in baseline `v0.1`, that dimension contributes
  a neutral `1.0`
- the current live runtime uses the settled building-level `utility_service_available` outcomes as
  its baseline approximation for those service-satisfaction ratios; finer per-utility breakdown can
  replace that approximation later without changing the surrounding demand formulas
- `external_connection_available` is a hard gate derived from settled network-border connectivity
- `resident_presence_saturation_residents`,
  `household_affordability_target_reserve_days`, and
  `household_stock_stability_target_days` are authored in the top-level
  `signal_normalization` table in [`demand/growth_profiles.toml`](../demand/growth_profiles.toml)

Future local-modifier families may include:

- pollution
- noise
- crime
- education
- parks and public-space quality
- transit access
- commute burden
- broader neighborhood or city attractiveness

Future-extension rule:

- those later local modifiers remain owned by their source systems
- demand may only consume normalized summaries of them
- any future expansion of shipped `GrowthProfile`s to use local modifiers must update the supported
  key set and validation rules in this document at the same time

Deterministic `v0.1` `DemandChannel` formulas:

Baseline helper terms:

```text
housing_shortage = 1.0 - housing_availability
goods_shortage   = 1.0 - household_stock_stability
service_gate     = utility_service_stability * external_connection_available
```

Evaluation order:

1. Read every baseline city-level signal and clamp it to `0.0..1.0`.
2. Compute the helper terms above.
3. Compute the city-level `DemandChannel` values consumed by `GrowthProfile`s in this exact order:

```text
ResidentialGrowth =
    clamp(
        housing_shortage
      * household_affordability
      * service_gate,
        0.0,
        1.0
    )

CommercialGrowth =
    clamp(
        resident_presence
      * goods_shortage
      * household_affordability
      * service_gate,
        0.0,
        1.0
    )

IndustrialGrowth =
    clamp(
        resident_presence
      * commercial_input_deficit
      * service_gate,
        0.0,
        1.0
    )
```

4. Compute the action-limit gates for building spawns. Residential spawns use a quadratic
   shortage curve to prevent runaway while vacant houses exist. Non-residential spawns are
   uncapped so commercial and industrial can bootstrap before the city is large:

```text
ResidentialSpawnLimit = housing_shortage ^ 2
NonResidentialSpawnLimit = 1.0
```

Interpretation:

- `ResidentialGrowth` rises when the city has a housing shortage, households can still afford
  to settle, and utility or border-service support is healthy
- `job_availability` is intentionally excluded from `ResidentialGrowth`: people can move to a
  new city before jobs exist; the unemployment benefit keeps them solvent during that window
- `CommercialGrowth` rises when a real resident/customer base exists, household stock is
  unstable, households can still spend, and the city is healthy enough to support more commerce
- `IndustrialGrowth` is driven by `commercial_input_deficit` — the fraction of commercial
  buildings that lack a local industrial supplier — rather than `goods_shortage`; this decouples
  farm spawning from the OWA fallback that suppresses `goods_shortage` when imports are available
- `NonResidentialSpawnLimit = 1.0` so commercial and industrial buildings can bootstrap without
  waiting for a large population; gating them on `resident_presence` created a circular deadlock
  where the first grocery store took 20+ days to accumulate a single spawn credit
- baseline `v0.1` intentionally does not define `OfficeGrowth` or `MixedGrowth`; office and mixed
  private growth remain later explicit extensions if they are reintroduced with fully specified
  formulas and matching shipped profile data

## Ownership

The intended ownership split is:

- `docs/demand.md`: city-level pressure, immigration and emigration pressure, private building growth or decline pressure, building upgrade or downgrade eligibility, redevelopment pressure, and `GrowthProfile` evaluation rules
- `docs/economy.md`: household, company, and building economy state once those entities already exist, including household affordability, relocation, eviction, and economy-side viability gates
- `docs/zoning.md`: player-facing zoning model, `ZoneType`, `ZoneProfile`, and legal placement or upgrade envelopes
- `docs/building_allocator.md`: concrete build-site discovery, frontage attachment, geometric fit, placement, and removal execution
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
- whether arrival or departure is visualized through a border-origin or later outside-gateway
  transport trip
- which outside gateway is chosen when transport visualizes household arrival or departure
- the exact freight or utility-service flow that later changes migration pressure

## Core Rule

Demand decides whether immigration should happen.

Economy creates and owns the admitted household record.

Buildings and vacancy rules decide whether a real home can be claimed.

Transport may optionally visualize arrival, but transport does not decide city growth.

The same ownership pattern should later apply to private buildings:

- demand decides whether pressure exists for spawn, despawn, upgrade, or downgrade
- building and economy systems execute the resulting runtime change
- zoning and placement rules still decide whether a legal site exists

## Inputs

Baseline `v0.1` immigration and emigration pressure is derived from coarse city signals such
as:

- vacant resident capacity
- resident presence
- job availability (emigration pressure only)
- household affordability (commercial/industrial demand; not an admission gate)
- household stock stability
- utility or service stability
- existence of at least one external connection when external immigration is required
- commercial input deficit (industrial spawn pressure)

These are city-level signals, not per-agent trip decisions.

Later extensions may add more city-level pressure inputs such as commute burden, broader service
quality, or other wider city-stability summaries, but baseline `v0.1` must stay aligned with the
fixed signal families defined in `Modifiers And Signal Sources`.

## Outputs

The demand layer should produce three distinct output families:

- concrete daily household-action outputs
- concrete daily building-action budgets
- ongoing building-growth pressure outputs

Concrete daily household-action outputs:

- `households_to_admit_today`
- `households_to_remove_today`

Interpretation:

- these are bounded whole-household counts
- they are direct daily city-growth actions, not vague pressure scores
- economy, household, and vacancy systems consume them to create or remove real households

Deterministic pressure-to-action rule for household outputs:

- household admission and removal start from normalized whole-city pressure values in `0.0..1.0`
- each action has a fixed threshold in `0.0..1.0`
- pressure below threshold produces `0` action today
- pressure above threshold produces a bounded daily household count derived from the excess above
  threshold

Deterministic conversion rule:

```text
normalized_action_pressure =
    clamp((pressure - action_threshold) / (1.0 - action_threshold), 0.0, 1.0)

daily_action_credit += normalized_action_pressure * max_households_per_day

households_to_act_today = floor(daily_action_credit)
daily_action_credit -= households_to_act_today
```

Authoring rule:

- `admission_threshold = household_action.admission_threshold`
- `removal_threshold = household_action.removal_threshold`
- `max_households_per_day = action_budget.max_households_per_day` from
  [`demand/growth_profiles.toml`](../demand/growth_profiles.toml)
- admission uses `action_threshold = admission_threshold`
- removal uses `action_threshold = removal_threshold`
- baseline `v0.1` uses one shared authored daily household-action cap for both admission and
  removal instead of hard-coded runtime constants

Interpretation:

- the farther pressure is above the threshold, the faster household action accumulates
- weak but persistent pressure still produces deterministic action over multiple days
- stronger pressure produces larger daily household counts, but never above the bounded daily cap

Concrete daily building-action budgets:

- `residential_spawns_today`
- `commercial_spawns_today`
- `industrial_spawns_today`
- `residential_upgrades_today`
- `commercial_upgrades_today`
- `industrial_upgrades_today`
- `residential_downgrades_today`
- `commercial_downgrades_today`
- `industrial_downgrades_today`
- `residential_despawns_today`
- `commercial_despawns_today`
- `industrial_despawns_today`

Interpretation:

- these are bounded whole-building or whole-site counts, not vague pressure scores
- demand computes them once from one frozen daily city snapshot and one frozen eligible-candidate
  snapshot
- buildings placed, upgraded, downgraded, or removed during that pass do not change the same-day
  budgets; they affect the next daily demand pass
- there is no separate allocator-owned arbitrary cap on top of these demand-owned daily budgets

Deterministic budget rule for building actions:

For each use family `use` and action type `action`, demand first builds the eligible candidate list
from the frozen daily snapshot. It then computes the bounded daily budget from the relevant
normalized action pressure, the eligible candidate count, and a carried action-credit buffer.

Implementation note for mixed-density candidate sets:

- when one use family aggregates eligible candidates from multiple density profiles with different
  thresholds, the runtime sums each candidate's normalized pressure contribution before applying the
  shared use-family batch fraction
- when all candidates share the same threshold, this reduces to the simpler `eligible_count *
  normalized_action_pressure` form shown below

For spawn:

```text
normalized_spawn_pressure =
    clamp((growth_pressure - spawn_threshold) / (1.0 - spawn_threshold), 0.0, 1.0)

spawn_action_credit[use] +=
    normalized_spawn_pressure
  * (eligible_spawn_count[use] * spawn_batch_fraction_by_use[use])
  * spawn_limit[use]

spawns_today[use] =
    min(eligible_spawn_count[use], floor(spawn_action_credit[use]))

spawn_action_credit[use] -= spawns_today[use]
```

For upgrade:

```text
normalized_upgrade_pressure =
    clamp((growth_pressure - upgrade_threshold) / (1.0 - upgrade_threshold), 0.0, 1.0)

upgrade_action_credit[use] +=
    normalized_upgrade_pressure
  * eligible_upgrade_count[use]
  * upgrade_batch_fraction_by_use[use]

upgrades_today[use] =
    min(eligible_upgrade_count[use], floor(upgrade_action_credit[use]))

upgrade_action_credit[use] -= upgrades_today[use]
```

For downgrade:

```text
normalized_downgrade_pressure =
    clamp(
        (downgrade_threshold - growth_pressure) / max(downgrade_threshold, epsilon),
        0.0,
        1.0
    )

downgrade_action_credit[use] +=
    normalized_downgrade_pressure
  * eligible_downgrade_count[use]
  * downgrade_batch_fraction_by_use[use]

downgrades_today[use] =
    min(eligible_downgrade_count[use], floor(downgrade_action_credit[use]))

downgrade_action_credit[use] -= downgrades_today[use]
```

For despawn:

```text
normalized_despawn_pressure =
    clamp(
        (despawn_threshold - growth_pressure) / max(despawn_threshold, epsilon),
        0.0,
        1.0
    )

despawn_action_credit[use] +=
    normalized_despawn_pressure
  * eligible_despawn_count[use]
  * despawn_batch_fraction_by_use[use]

despawns_today[use] =
    min(eligible_despawn_count[use], floor(despawn_action_credit[use]))

despawn_action_credit[use] -= despawns_today[use]
```

Authoring rule:

- `spawn_batch_fraction_by_use = action_budget.spawn_batch_fraction_by_use`
- `upgrade_batch_fraction_by_use = action_budget.upgrade_batch_fraction_by_use`
- `downgrade_batch_fraction_by_use = action_budget.downgrade_batch_fraction_by_use`
- `despawn_batch_fraction_by_use = action_budget.despawn_batch_fraction_by_use`
- all of those tables are authored in [`demand/growth_profiles.toml`](../demand/growth_profiles.toml)
- baseline `v0.1` does not hard-code these fractions in runtime code

Deterministic execution order:

- daily demand outputs consume in this exact order at the midnight boundary:
  `households_to_remove_today`, then all private building actions, then
  `households_to_admit_today`, then one lightweight post-admission workplace-assignment pass for
  the newly admitted households
- `households_to_remove_today` uses only the deterministic settled-snapshot candidate order owned
  by [`economy.md`](economy.md); same-boundary admissions must not enter that candidate set
- household admission claims vacancy only after the same-boundary building actions complete, so
  fresh residential spawns may be filled immediately on that same midnight boundary
- demand builds all eligible candidate lists once per daily pass before any building change is
  executed
- use families iterate in this exact order for the daily pass: `residential`, then `commercial`,
  then `industrial`
- spawn candidates sort by the allocator's deterministic build-site order:
  `(edge_idx, side_order, cell_x, width_cells, depth_cells, zone_profile_id)` where
  `side_order = [1, -1]`
- existing-building candidates for upgrade, downgrade, and despawn sort by the building's
  attachment order:
  `(edge_idx, side_order, cell_x, width_cells, depth_cells, level, asset_id)`
- for each use family, execution order is `despawn`, then `downgrade`, then `upgrade`, then
  `spawn`
- a building or empty site may be affected at most once per daily demand pass

Interpretation:

- large legally zoned estates under strong pressure can fill in visible daily batches rather than
  one building at a time
- weak but persistent pressure still produces deterministic building change over multiple days
- startup support can intentionally make early-city spawn batches larger so the city forms a real
  starter neighborhood instead of stalling at tiny-village scale
- newly admitted households may receive workplaces before the next daytime trip window, but that
  post-admission assignment does not rerun the daily demand pass or rewrite the frozen daily
  demand snapshot

Ongoing building-growth pressure outputs:

- `residential_growth_pressure`
- `commercial_growth_pressure`
- `industrial_growth_pressure`
- later, other use-family growth pressures if new top-level zoning categories are added
- building upgrade pressure summaries
- building downgrade pressure summaries
- optional local build-site scores for site-level private development later on

Interpretation:

- these outputs say how strongly the city wants more, less, better, or worse private building
  capacity of a given type
- they are not direct spawn commands and they are not direct household counts
- economy, zoning, and allocator/building systems consume them together with legal-site
  availability to decide whether a real building spawn, despawn, upgrade, downgrade, or
  replacement may happen

Pressure-dynamics rule:

- these growth-pressure outputs are recalculated from current city conditions; they do not
  monotonically increase over time
- a growth-pressure value may rise, plateau, or fall as housing, jobs, stock stability,
  utility-service stability, and other documented current inputs change
- categories the city does not structurally support should naturally remain low instead of drifting
  upward just because time has passed
- startup support, when present, is a separate temporary input layered onto the ordinary pressure
  calculation; it is not a permanent "pressure always ticks upward" rule

Those outputs are then consumed by economy and building systems in their own layers.

## Demand Cadence And Economy Handoff

Demand runs from the operational clock, but it does not sample the economy continuously.

Deterministic `v0.1` day-boundary rule:

1. Finish the final sub-daily economy step for the current operational day.
2. Run one daily economy settlement pass owned by [`economy.md`](economy.md).
3. Freeze the post-settlement city snapshot produced by that pass.
4. Derive every baseline demand input from that frozen snapshot:
   - `housing_availability`
   - `resident_presence`
   - `job_availability`
   - `household_affordability`
   - `household_stock_stability`
   - `utility_service_stability`
   - `external_connection_available`
5. Compute the city-level `DemandChannel` values from that same frozen snapshot.
6. Evaluate every active `GrowthProfile` whose cadence matches that day boundary.
7. Compute `households_to_admit_today`, `households_to_remove_today`, and all building-action
   budgets from that same frozen snapshot.
8. Execute the resulting demand-owned daily actions before the next operational day's sub-daily
   economy steps begin.

Deterministic cadence rule:

- the demand layer runs exactly once per operational day boundary
- a `GrowthProfile` with `cadence_days = N` is evaluated only on day boundaries where
  the current operational `day_index % N == 0`
- household-action outputs and building-action budgets are rebuilt on every daily demand pass from
  the current frozen post-settlement snapshot

Interpretation:

- demand does not read half-settled hourly economy state
- demand does not recompute after each spawned, removed, upgraded, or downgraded building inside
  the same day-boundary pass
- demand-owned actions at one day boundary change the city's runtime state for the next operational
  day and therefore affect the next daily demand pass
- the economy layer owns the sub-daily work, stock, utility, wage, and affordability updates that
  produce the settled snapshot; demand owns the once-per-day growth decision taken from that
  snapshot

## Startup Household Growth

An empty map needs an early growth bias or the city deadlocks before the local economy can form.

For `v0.1`, fresh-city bootstrapping is demand-owned through purely organic signals. The pioneer
demand floor has been removed; the unemployment benefit in [`economy.md`](economy.md) keeps
early-city households solvent through real economic activity, which in turn generates real spawn
pressure on commercial and industrial buildings.

Rules:

- early game should favor agent admission while vacant housing and external connections exist
- immigration pressure is driven by coarse city signals only, not by hidden magic or static floors
- no job requirement for initial settlement: people move to a new city before jobs exist; the
  unemployment benefit sustains them during the bootstrap window

Deterministic `v0.1` admission-pressure rule:

```text
base_inflow = household_action.base_inflow * external_connection_available

admission_pressure =
    base_inflow
  * housing_availability
  * utility_service_stability
```

Where:

- all factors are clamped to `0.0..1.0`
- `utility_service_stability` acts as a city-health gate: a city with no services cannot admit more
- this keeps early growth positive while letting immigration slow automatically as housing fills or
  city stability deteriorates

Deterministic `v0.1` removal-pressure rule:

```text
unhoused_household_ratio =
    if total_household_count == 0 then 0.0
    else clamp(unhoused_household_count / total_household_count, 0.0, 1.0)

job_failure =
    1.0 - job_availability

stock_shortage =
    1.0 - household_stock_stability

removal_pressure =
    clamp(
        unhoused_household_ratio * 0.50
      + job_failure * 0.25
      + stock_shortage * 0.25,
        0.0,
        1.0
    )
```

Where:

- all derived terms are clamped to `0.0..1.0`
- `total_household_count = housed_household_count + unhoused_household_count`
- `unhoused_household_count` is read from the settled economy snapshot after relocation and
  eviction have already run for that operational day
- `job_availability` and `household_stock_stability` come from the same frozen post-settlement
  snapshot used by the rest of the daily demand pass
- `removal_threshold = 0.55` is intentionally above the bare-startup value of
  `job_failure×0.25 + stock_shortage×0.25 = 0.50` so that a city without any supply chain yet
  does not immediately expel its first households; removal only fires once the economy is
  genuinely failing (unhoused households push the total above 0.55)
- the final `households_to_remove_today` output is derived deterministically from this pressure
  through the generic household pressure-to-action conversion rule documented earlier in this file

Interpretation:

- ordinary low money does not directly remove households from the city in baseline `v0.1`
- household affordability failure first flows through the economy-owned relocation, eviction, and
  `unhoused` rules described in [`economy.md`](economy.md)
- once the settled snapshot contains persistent `unhoused` households, sustained job failure, or
  persistent stock shortage, demand may convert that state into whole-household city removal
- baseline `v0.1` household outflow therefore comes from sustained failed living conditions, not
  from one bad hourly dip or one direct poverty-to-deletion shortcut

## Deterministic `v0.1` Rules

For `v0.1`, the immigration rules are simple and deterministic:

- evaluate immigration and emigration on a coarse daily cadence
- if there is no valid housing capacity, admit `0` households
- if there is no required external connection, admit `0` households
- admit whole households only
- the number admitted per day must be bounded
- the result should come from coarse pressure signals, not from a hidden startup path or a transport-state side effect

If a household is admitted:

- economy creates the household record
- housing/vacancy logic claims a real home
- transport may either instantiate the household directly at home or visualize a border-origin arrival

If a household is removed:

- demand still owns the whole-household removal count
- economy still owns the household-side bad-state reasons that made removal pressure rise
- transport may later either remove the household immediately or visualize a trip to an external
  gateway
- any future external-gateway arrival or departure visualization remains downstream of the demand
  decision and belongs to [`docs/entrance_and_exit.md`](entrance_and_exit.md)

Both outcomes are valid, but they are downstream of the demand decision.

## Relationship To Building Growth

Residential growth and immigration stay coupled through demand, not through hidden automatic
spawning.

That means:

- immigration should not ignore housing limits
- future residential building creation should not ignore household demand
- future commercial, industrial, and any later private-use building creation should not ignore
  their own demand pressure
- future building upgrades should not ignore demand pressure and the documented zoning and economy
  gates that make higher levels legal and viable
- zoning alone is not enough to force either households or buildings to appear

The long-term clean model is:

- demand produces residential pressure
- demand produces non-residential growth pressure
- demand produces upgrade or downgrade pressure
- that pressure can justify both household admission and future private residential development
- buildings, economy, and transport each consume that pressure in their own layer

Important residential boundary:

- household affordability failure, relocation between vacant homes, and eviction from the current
  home are economy-owned household rules described in [`economy.md`](economy.md)
- residential downgrade or redevelopment pressure is a building-side demand output, not a direct
  "one poor household makes the building downgrade" rule
- in practice, household poverty should first create move-out, relocation, or unhoused outcomes;
  only later vacancy and weak building-side conditions may justify building-level decline or
  redevelopment

Important zone-type boundary:

- demand upgrade and downgrade pressure is intentionally broad and use-family-level
- the final upgrade or downgrade decision still passes through zone-type-specific economy viability
  gates described in [`economy.md`](economy.md)
- in `v0.1`, those economy gates should rely on relatively mature signals such as occupancy,
  affordability, staffing, stock, input coverage, utility status, and operating-buffer days
- crime, noise, parks, education, and similar neighborhood conditions are later local modifiers to
  demand pressure, not required baseline hard blockers for building level changes

### Deterministic building spawn rule

A private building spawns only when all owning layers agree that the spawn is allowed.

Deterministic `v0.1` spawn rule:

1. Demand reads the relevant city-level growth pressure for the target use family.
2. The relevant `GrowthProfile` evaluates that pressure and makes spawn eligible only if
   `final_growth_score >= spawn_threshold`.
3. Demand builds the frozen eligible spawn-candidate list, sorts it in deterministic allocator
   build-site order, and computes `spawns_today[use]` from the daily batch-budget rule described
   above.
4. Zoning and allocator rules must then find at least one legal build site for the active
   `ZoneProfile`.
5. Economy viability must allow the spawn for that zone type:
   - `Residential`: enough demand plus a legal site and ordinary residential viability
   - `Commercial`, `Industrial`: enough demand plus a legal site and the relevant business-side
     viability gate from [`economy.md`](economy.md)
6. The allocator chooses up to `spawns_today[use]` concrete legal build sites from the sorted
   candidate list.
7. Zoning-side asset selection chooses one legal `level = 1` asset deterministically from the
   allowed family pool.

Interpretation:

- city-level growth pressure alone does not create a building
- zoning alone does not create a building
- economy viability alone does not create a building
- spawn happens only when demand, zoning, economy, and allocator all pass, and only up to the
  bounded daily spawn budget for that use family
- baseline `v0.1` fresh spawn stays at `level = 1`; higher-level direct spawn is a later explicit
  extension

### Non-Residential Spawn Gates

Commercial and industrial buildings spawning into a neighbourhood with no workers to staff them,
or no consumers to absorb their output, is the root cause of zombie businesses and oversupply.
Two deterministic per-candidate gates enforce economic readiness before a non-residential spawn
is allowed to execute.

These gates apply **after** the budget (`spawns_today[use]`) has been computed and **during** final
candidate selection. Candidates that fail either gate are removed from the selected set; the
remaining budget is not redistributed to other candidates on the same day.

#### Labour Gate

A non-residential spawn candidate is rejected if the city cannot plausibly staff it.

Deterministic rule:

```text
available_unemployed =
    open_reachable_job_slots   # from the frozen daily snapshot

required_workers = worker_capacity of the candidate's bound economy profile
                   (0 if the profile has no workers, e.g. utility nodes)

labour_gate_passes =
    required_workers == 0
    OR available_unemployed >= required_workers
```

Where:

- `open_reachable_job_slots` is the same settled snapshot value used in `job_availability`
- `required_workers` is read from the compiled economy catalog for the candidate's bound profile
- a candidate with `required_workers == 0` always passes (utility buildings, warehouses)
- if the economy profile binding is missing or the catalog cannot be read, the gate fails safe
  (spawn is rejected)
- the gate consumes `required_workers` from the running `available_unemployed` count as each
  candidate passes, so a single daily pass does not spawn more buildings than there are workers

Interpretation:

- a completely empty city can spawn commercial or industrial buildings without a prior population
  because `NonResidentialSpawnLimit = 1.0`; the labour gate then checks the actual settled
  open-job count from the snapshot
- once the city's real workers fill open jobs the gate naturally relaxes again for new spawns

#### Output-Absorption Gate

A non-residential spawn candidate is rejected if the city's consumer base cannot absorb its
output.

Deterministic rule:

```text
output_capacity_already_placed =
    sum of base_rate_units_per_day for all existing non-broken buildings
    whose bound economy profile has any output overlapping
    the candidate profile's outputs

total_consumer_demand =
    housed_resident_count * consumption_rate_per_resident
    summed over all demand-sink profiles reachable from the candidate's output resources

output_absorption_gate_passes =
    output_capacity_already_placed < total_consumer_demand
    OR total_consumer_demand == 0   # no demand-sink in catalog → gate not applicable
```

Where:

- `total_consumer_demand` is computed from the settled snapshot's `housed_resident_count` and the
  per-resident consumption rates declared in the compiled economy catalog for connected
  demand-sink profiles
- `output_capacity_already_placed` sums `base_rate_units_per_day` over all live buildings with
  matching output resources; broken or economy-broken buildings are excluded
- if the candidate profile has no declared outputs the gate passes (e.g. pure service buildings)
- if the economy profile binding cannot be resolved the gate fails safe (spawn is rejected)

Interpretation:

- this prevents the city from placing a second grocery before the first one's output capacity is
  fully absorbed by the resident population
- it also prevents industrial over-placement when the downstream commercial or household demand
  cannot consume additional raw output
- the gate is not a hard per-building quota; it measures total placed capacity against total
  consumer demand, so a single large building covering 80% of demand still allows a second smaller
  one once demand grows enough

#### Partial-Substitute Resource Extension

When a candidate building produces a **partial-substitute resource** (e.g. `convenience_goods`
produced by a Grocery Kiosk), the gate uses an adjusted consumer demand rather than the raw
resident count. This prevents kiosk capacity from blocking grocery spawning while still
limiting kiosk over-supply:

```text
effective_consumer_demand =
    housed_resident_count
    * consumption_rate_per_resident
    * satisfaction_ratio   // 1.0 for primary resources, < 1.0 for partial substitutes

output_absorption_gate_passes =
    output_capacity_already_placed < effective_consumer_demand
    OR effective_consumer_demand == 0
```

The `satisfaction_ratio` for each resource is authored in `RuntimeEconomyTuning` (see
[`economy.md § Commercial Tiers`](economy.md#commercial-tiers-and-multi-resource-household-consumption)).
For all current primary resources (`household_supplies`, `staple_food`, etc.) the ratio is
implicitly `1.0` and no change in gate behavior occurs.

#### Gate Interaction

Both gates are evaluated independently for each candidate. A candidate that fails **either** gate
is excluded from that day's selected spawn set. The labour gate is evaluated first.

These gates do not affect residential spawns, upgrades, downgrades, or despawns.

Residential note:

- `households_to_admit_today` and residential building spawn are related but not identical outputs
- household admission decides whether new households enter the city
- residential building spawn decides whether new housing capacity appears on the map
- both should respond to residential demand, but each still uses its own owning-layer contract

## Building Desertion

A Commercial or Industrial building enters the **deserted** state when it has been
economically dead for long enough that no self-rescue is possible. Deserted buildings are
inert in all simulation systems, visually distinct, and remain on the map occupying their
land until the player demolishes them or demand-system despawn pressure removes them.

Residential buildings are excluded. Household emigration handles the equivalent residential
lifecycle event.

### Desertion Trigger

Desertion is evaluated once per daily tick for every non-broken, non-economy-broken
Commercial or Industrial building.

Deterministic rule:

```text
insolvent =
    operating_budget < profile.average_daily_wage()   // cannot pay even one worker for one day
    OR (worker_count == 0 AND operating_budget < 13.0)
    // "OR" covers the no-worker case where average_daily_wage may be 0

deserted_conditions_hold =
    startup_reset_used == true          // one-time bootstrap rescue already consumed
    AND revenue == 0.0                  // no income this period
    AND insolvent                       // cannot sustain any workers at current budget

economy_dead_days update (daily, before is_deserted check):
    if is_deserted:
        no change                       // already terminal; counter frozen
    else if deserted_conditions_hold:
        economy_dead_days += 1
    else:
        economy_dead_days = 0           // any condition failing resets the streak

is_deserted becomes true when:
    economy_dead_days >= DESERTED_THRESHOLD_DAYS   // constant: 14
```

Where:

- `startup_reset_used` is set by `ensure_building_startup_float` in `economy/households.rs` after
  the one-time budget rescue fires; it is never reset
- `profile.average_daily_wage()` is the wage declared in the bound economy profile; buildings
  with no workers (utility nodes, warehouses) return 0.0 from this call, which is why the
  fallback `worker_count == 0 AND operating_budget < 13.0` branch covers those cases
- `revenue` is the building's lifetime gross-revenue field; it stays at `0.0` only for
  buildings that have never completed a transaction since spawn or last restart
- `DESERTED_THRESHOLD_DAYS = 14` — two weeks of unbroken economic death required before the
  state transitions; any revenue receipt resets the streak to zero
- `economy_dead_days` reuses the existing dead-code field `abandoned_timer: u32`, renamed;
  it is persisted in the save schema so the streak survives reloads

Interpretation:

- a building with 5/5 workers and `operating_budget = 0.0` satisfies `insolvent` even though
  `worker_count != 0`; workers present at a building that cannot pay them do not rescue it
  from the desertion streak
- a newly placed building that cannot attract workers exhausts its startup float, gets one
  rescue from `ensure_building_startup_float`, exhausts it again, and then accumulates the
  14-day streak before becoming deserted — total time from spawn to deserted is roughly
  `2 × startup_runway_days + 14` (≈ 28 days at default tuning)
- any revenue receipt (even a single unit sold) resets `economy_dead_days` to zero; the
  building must be continuously insolvent and earningless for the full threshold period
- `is_deserted` is a one-way latch; no in-simulation recovery path exists once set

### Insolvent Employer Filter (Job Search)

Workers must not seek employment at a building that demonstrably cannot pay wages. The
current job-candidate evaluation scores on commute, income pressure, and stock pressure but
has no solvency check — agents target zero-budget employers the same as healthy ones.

Deterministic rule applied inside `assign_jobs` for each non-residential building candidate:

```text
insolvent_employer(building) =
    revenue == 0.0
    AND operating_budget < profile.average_daily_wage()

job_candidate_eligible(building) =
    NOT insolvent_employer(building)   // added gate
    AND worker_capacity > 0
    AND open_slots > 0
    AND zone_type IN {Industrial, Commercial}
```

Where:

- the same `profile.average_daily_wage()` value used in the desertion trigger is used here;
  zero-wage buildings (utility nodes) are not filtered out by this check
- `revenue == 0.0` is required alongside the budget check so that a newly funded building
  with no revenue yet (days 1–7 of startup) is not incorrectly excluded during its wage
  runway — a building burning its startup float legitimately has `revenue == 0` but a
  positive `operating_budget`; the combined check allows new buildings through while
  excluding buildings that have exhausted all capital

Interaction with `JOB_UNPAID_ABANDON_DAYS`:

The existing per-agent `consecutive_unpaid_days` escape hatch (after 3 unpaid days an agent
may override their job lock) remains unchanged. The insolvent employer filter acts earlier —
at the point where an agent without a current job searches for one — so agents never take an
insolvent job in the first place. An agent already locked into an insolvent job still uses
the existing 3-day escape.

### Effects When Deserted

All effects below apply only when `is_deserted == true`. The `broken` and `economy_broken`
flags remain unmodified; desertion is a third orthogonal terminal state.

**Worker capacity:**

```text
worker_capacity(building) =
    0               if is_deserted
    registry value  otherwise
```

A deserted building reports zero capacity to every caller. Agents evaluating job candidates
see it as fully staffed and never target it for employment.

**Economy IO (production and consumption):**

```text
run_building_economy skips building if:
    broken OR economy_broken OR is_deserted
```

No inputs are consumed from inventory. No outputs are produced into inventory. The building's
inventory contents are frozen at whatever level they held when desertion was reached.

**Utility billing:**

```text
resolve_building_utilities skips building if:
    broken OR economy_broken OR is_deserted
```

No utility charge is applied. `utility_service_available` is set to `false` for deserted
buildings and is never flipped back.

**Freight and OWA export:**

```text
freight_supplier_eligible(building) =
    false   if broken OR economy_broken OR is_deserted
    ...     otherwise (existing checks)

owa_export_eligible(building) =
    false   if is_deserted (in addition to existing broken / zone / utility checks)
```

A deserted building does not appear in supplier searches and is not offered to OWA freight
runs. Inventory it holds is permanently stranded.

**Output-Absorption Gate (`nonresidential_passes_absorption_gate`):**

```text
output_capacity_already_placed =
    sum of base_rate_units_per_day for all existing buildings where:
        NOT broken
        AND NOT economy_broken
        AND NOT is_deserted                 // ← added exclusion
        AND outputs overlap candidate outputs
```

A deserted building is removed from `placed_capacity`. This directly unblocks the spawn of a
replacement building when the only existing producer for a given resource has become deserted.

**Despawn candidate priority:**

```text
despawn_candidate_eligible(building) =
    true    if is_deserted AND worker_count == 0 AND occupancy == 0
    ...     existing conditions otherwise

despawn_candidate_order:
    deserted buildings sorted before non-deserted buildings within each zone family
```

Deserted buildings are always first in line for demand-driven removal. When despawn pressure
is positive they clear before healthy buildings downgrade.

### Visual Representation

The render pipeline uses `MultiMesh.TRANSFORM_3D` (12-float per-instance transform). This
format carries no per-instance color channel, so deserted tinting requires a parallel
MultiMesh with a material override rather than a buffer-format change.

**Render group assignment (Rust, `nodes/sim/render/buildings.rs`):**

```text
render_group(building) =
    "broken:error"      if broken
    "deserted"          if is_deserted AND NOT broken
    asset_id            otherwise
```

Add `get_deserted_building_transforms_for_asset_internal(asset_id: &str)` to `SimCore`:

- iterates `allocator.buildings`
- includes building if `is_deserted && !broken && asset_id == b.asset_id`
- packs the same 12-float transform layout as the existing function

Modify `get_building_transforms_for_asset_internal` to additionally skip buildings where
`is_deserted == true` (deserted buildings must not appear in the live multimesh).

**Godot side (`buildings.gd`):**

```text
deserted_multimeshes: Dictionary   // asset_id → MultiMeshInstance3D
```

`_setup_multimesh_for_asset(asset_id)` also calls `_setup_deserted_multimesh_for_asset(asset_id)`:

- loads the same mesh as the live multimesh for this asset
- applies a `StandardMaterial3D` override:
    - `albedo_color = Color(0.45, 0.42, 0.38, 1.0)` — warm gray, slightly desaturated
    - `shading_mode = BaseMaterial3D.SHADING_MODE_PER_PIXEL`
    - no emission, no metallic boost
- stores in `deserted_multimeshes[asset_id]`

On each dirty frame, for every registered `asset_id`, also call
`_update_deserted_multimesh(asset_id)` which queries
`get_deserted_building_transforms_for_asset(asset_id)` and updates the deserted multimesh
instance count and buffer.

No new mesh assets are required. The same geometry renders with a gray material.

### Recovery

There is no in-simulation recovery path. `is_deserted` is a one-way latch.

A deserted building:

- occupies its land cells in the zoning grid, blocking new spawns at that location
- is ineligible for upgrade or downgrade consideration
- is not counted in `job_availability`, `utility_stab`, or `stock_stab` snapshots
- is removed when the player demolishes it, or when the demand system's despawn pressure
  selects it (deserted buildings are always first in the despawn queue)

### Data Model

| Field | Type | Default | Notes |
|---|---|---|---|
| `economy_dead_days: u32` | u32 | 0 | Renamed from dead-code `abandoned_timer`. Consecutive-day streak counter. Frozen once `is_deserted`. |
| `is_deserted: bool` | bool | false | One-way latch. Set when `economy_dead_days >= DESERTED_THRESHOLD_DAYS`. |

Both fields are persisted. Add to `save/world.rs` INSERT and SELECT with forward-compatible
`ALTER TABLE buildings ADD COLUMN ... DEFAULT 0` migrations so older saves load cleanly.

All construction sites that build a `Building` literal must initialise both fields:
`economy_dead_days: 0, is_deserted: false`.

Affected sites: `allocator/placement.rs`, `economy/demand.rs`, `economy/households.rs`,
`economy/logistics.rs`, `network/topology.rs`, `grid/pollution.rs`, `save/world.rs`.

## Code Removal And Replacement Targets

The current codebase still has transitional growth logic outside the demand layer.

These paths are listed so they can be replaced or deleted. The goal is one authoritative
demand-owned decision path per growth behavior, without long-term fallback logic that keeps the old
allocator- or transport-owned behavior alive.

- The coarse immigration decision has now moved behind the demand-owned `households_to_admit_today`
  output, and allocator execution consumes that count through
  `execute_demand_household_admission(...)` rather than recomputing pressure locally.
- Ordinary household admission now uses the explicit housed-admission path in
  `rust/src/simulation/economy/agents/data.rs::spawn_housed_agent()`. Optional border-origin
  transport visualization is separate in `spawn_border_arrival_agent()` and remains the only place
  where `TRANSIT_IMMIGRATING` is still appropriate.
- Ordinary private-building despawn, downgrade, upgrade, and spawn now execute from demand-owned
  daily action plans. Stale topology or zoning cleanup remains only as invalid-placement cleanup,
  not as the ordinary building-growth path.
- Building-loss displacement currently has no dedicated demand/economy ownership boundary. `AgentSystem::evict_building()` still forces some agents into `TRANSIT_ACCESS_INGRESS` as a fake rubble/street fallback. That should be replaced by an explicit rehousing, homelessness, disaster, or removal contract rather than by reusing ordinary entrance-travel semantics.
- Debug logging and tooling should stop implying that immigration is fundamentally a border-spawn FSM process when the real source of truth is the demand-layer household-admission decision.

The target end state is:

- demand computes whether immigration should happen and how many households may be admitted
- demand computes whether private buildings should appear, disappear, upgrade, or downgrade
- shipped fresh-map startup uses the organic pioneer-demand bootstrap contract; any future special
  founding-placement scenario rule must stay outside allocator tick
- economy creates the admitted household records
- building systems execute legal placement, removal, and level changes once demand has already decided the pressure outcome
- housing/vacancy logic claims real homes
- transport either visualizes the move or does nothing, but it does not decide growth

## Remaining Follow-Up Limitations

### Open Spec Gaps

- **Building Desertion** — spec complete and implemented; see `§ Building Desertion` above

### Intentional `v0.1` Deferrals

- `v0.1` intentionally keeps the shipped `GrowthProfile` set small and closed. If later gameplay
  truly needs zoning-specific demand behavior beyond one default profile per shipped baseline
  `zone_type + density`,
  that extension should be added deliberately rather than by making growth-profile creation as open
  ended as zoning-profile creation.
- `Office` and `Mixed` are intentionally outside the baseline `v0.1` demand-channel set. If later
  gameplay reintroduces them as ordinary private growth families, they should come back only with
  explicit `DemandChannel` formulas, shipped `GrowthProfile` data, and matching zoning-side
  profile rules added at the same time.
- local-modifier support beyond neutral defaults should be added only when the underlying source
  systems are implemented well enough to be trustworthy demand inputs.
