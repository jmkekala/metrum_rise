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
  `IndustrialGrowth` channels from completed operational-hour snapshots and the daily
  post-settlement snapshot
- it computes and persists `households_to_admit_today`,
  `households_to_remove_today`, and the carried household-action credits used by the admission or
  removal thresholds; `households_to_admit_today` is refreshed on the hourly admission cadence,
  while removal remains a daily settled-snapshot action
- it also computes and persists the carried building-action credits for spawn, upgrade, downgrade,
  and despawn budgets across residential, commercial, and industrial use families
- the live runtime now executes ordinary household admission hourly from the demand-owned
  `households_to_admit_today` output instead of recomputing immigration pressure inside the
  allocator
- the live runtime now executes ordinary household removal from the demand-owned
  `households_to_remove_today` output using the deterministic selection order owned by
  [`economy.md`](economy.md)
- the live runtime now reads household affordability, relocation, eviction, and `unhoused`
  outcomes from the settled daily economy pass before computing the next demand snapshot
- the live runtime now executes private building spawn, despawn, upgrade, and downgrade actions
  hourly from demand-owned building-action plans instead of allocator-owned heuristics
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

- the current system now owns household-admission and removal pressure plus the hourly admitted
  and daily removed household counts
- it also now owns the city's ordinary private building-change decisions through hourly action plans
- baseline fresh-map startup now flows through purely organic demand signals; no hidden
  allocator-owned founding path or pioneer floor remains

## Terminology Conventions

This document uses the following terms consistently:

- `current DemandSystem`: the live Rust system that now computes baseline `DemandChannel`s plus
  demand-owned hourly household-admission, daily household-removal outputs, and private building-action
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
- bounded cadence outputs such as hourly `households_to_admit_today` and daily
  `households_to_remove_today`
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
- whole-city household admission and removal remain demand-owned outputs, not `GrowthProfile`
  outputs; admission runs hourly and removal runs daily
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
  - spawn_threshold
  - despawn_threshold
  - upgrade_threshold
  - downgrade_threshold
  - hysteresis_margin
```

Interpretation:

- `demand_channel` selects exactly one city-level growth-pressure input for the profile
- thresholds convert the channel value into spawn, despawn, upgrade, or downgrade eligibility
- `hysteresis_margin` keeps state changes stable and stops one-frame spikes from causing churn

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

1. Read the city-level `demand_channel` pressure in `0.0..1.0` as the growth score.
2. Compare the score against the profile thresholds:
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
household_affordability_target_reserve_days = 7.0
household_stock_stability_target_days = 3.0

[action_budget]
max_households_per_day = 48

[household_action]
admission_threshold = 0.10
admission_affordability_floor = 0.25
admission_unhoused_ratio_penalty = 0.75
admission_zero_budget_penalty = 0.75
admission_negative_treasury_factor = 0.60
admission_recent_failure_penalty = 0.85
recent_failure_daily_decay = 0.50
removal_threshold = 0.55
persistent_exit_destitute_stock_days = 0.25
persistent_exit_destitute_unhoused_days = 2
persistent_exit_max_unhoused_days = 7
persistent_exit_daily_fraction = 0.25

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
spawn_threshold = 0.55     # fires when net inflow desire > 10%  (net_residential_demand > +0.10)
despawn_threshold = 0.45   # fires when net outflow desire > 10% (net_residential_demand < −0.10)
upgrade_threshold = 0.80
downgrade_threshold = 0.30
hysteresis_margin = 0.05
```

Deterministic validation rules:

- `signal_normalization.household_affordability_target_reserve_days` must be finite and `> 0.0`
- `signal_normalization.household_stock_stability_target_days` must be finite and `> 0.0`
- `action_budget.max_households_per_day` must be a finite integer `>= 0`
- `household_action.admission_threshold` must be finite and in `0.0..1.0`
- `household_action.admission_affordability_floor` must be finite and in `0.0..1.0`
- `household_action.admission_unhoused_ratio_penalty` must be finite and in `0.0..1.0`
- `household_action.admission_zero_budget_penalty` must be finite and in `0.0..1.0`
- `household_action.admission_negative_treasury_factor` must be finite and in `0.0..1.0`
- `household_action.admission_recent_failure_penalty` must be finite and in `0.0..1.0`
- `household_action.recent_failure_daily_decay` must be finite and in `0.0..1.0`
- `household_action.removal_threshold` must be finite and in `0.0..1.0`
- `household_action.persistent_exit_destitute_stock_days` must be finite and in `0.0..365.0`
- `household_action.persistent_exit_destitute_unhoused_days` must be an integer `>= 1`
- `household_action.persistent_exit_max_unhoused_days` must be an integer `>= 1`
- `household_action.persistent_exit_daily_fraction` must be finite and in `0.0..1.0`
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
- every threshold and `hysteresis_margin` must be finite values in `0.0..1.0`
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
- `household_affordability`
- `zero_budget_household_ratio`
- `household_stock_stability`
- `commercial_capacity_deficit` — fraction of housed-resident demand-sink consumption that is not
  covered by existing live commercial output capacity
- `external_connection_available`
- `city_treasury_negative`
- `commercial_owa_dependency` — fraction of commercial input value sourced from OWA imports rather
  than local industrial; computed from daily shipment costs accumulated per building, giving a
  smooth 0..1 signal that reflects actual throughput coverage

Baseline ownership rule:

- `housing_availability` comes from housing capacity and vacancy state owned by economy/building
  systems
- `household_affordability` comes from household budgets and essential-cost state owned by economy
- `zero_budget_household_ratio` is derived from all active household records, including unhoused
  households, so failed households cannot be hidden by affluent housed survivors
- `household_stock_stability` comes from household stock buffers owned by economy
- `commercial_capacity_deficit` is derived by the demand snapshot from catalog demand-sink input
  resources that a store-style commercial profile can produce, comparing housed-resident
  per-resource demand against live non-deserted commercial output capacity
- `external_connection_available` comes from network-border connectivity owned by the road/network
  layer
- `city_treasury_negative` is derived from the city treasury owned by the fiscal ledger
- `commercial_owa_dependency` is derived by the demand snapshot from daily per-building
  `daily_owa_input_value` and `daily_local_input_value` accumulators, reset after each snapshot:
  `total_owa / (total_owa + total_local)` across active commercial buildings, `0.0` when no
  commercial buildings exist or none have transacted yet

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
```

Signal formulas:

```text
housing_availability =
    if total_household_slots == 0 then 0.0
    else clamp(vacant_household_slots / total_household_slots, 0.0, 1.0)

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

zero_budget_household_ratio =
    if total_household_count == 0 then 0.0
    else clamp(zero_budget_household_count / total_household_count, 0.0, 1.0)

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

commercial_capacity_deficit =
    if total_commercial_consumer_demand == 0 then 0.0
    else clamp(
        unmet_commercial_consumer_demand
        / total_commercial_consumer_demand,
        0.0,
        1.0
    )

external_connection_available =
    if connected_border_count > 0 then 1.0 else 0.0

city_treasury_negative =
    if city_treasury_balance < 0.0 then 1.0 else 0.0
```

Interpretation and source rule:

- `housing_availability` uses settled household-slot capacity after the daily economy pass
- `household_affordability` uses settled economy-owned `household_reserve_days` values from
  [`economy.md`](economy.md)
- `zero_budget_household_ratio` counts active households with `budget <= EPSILON`; unlike
  `household_affordability`, it includes unhoused households
- `household_stock_stability` uses settled economy-owned `household_stock_days` values
- `commercial_capacity_deficit` uses settled commercial building output capacity and settled
  housed-resident demand-sink consumption rates from the compiled economy catalog
- `external_connection_available` is a hard gate derived from settled network-border connectivity
- `city_treasury_negative` is the only fiscal input consumed by demand in baseline `v0.1`; demand
  does not inspect the magnitude of the deficit
- `household_affordability_target_reserve_days` and `household_stock_stability_target_days` are
  authored in the `signal_normalization` table in
  [`demand/growth_profiles.toml`](../demand/growth_profiles.toml)

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
commercial_need  = max(goods_shortage, commercial_capacity_deficit)
household_purchase_power =
    clamp(
        household_affordability * household_affordability_target_reserve_days,
        0.0,
        1.0,
    )
admission_affordability_factor =
    admission_affordability_floor
    + (1.0 - admission_affordability_floor) * household_affordability
admission_unhoused_factor =
    1.0 - admission_unhoused_ratio_penalty * unhoused_household_ratio
admission_zero_budget_factor =
    1.0 - admission_zero_budget_penalty * zero_budget_household_ratio
admission_treasury_factor =
    1.0 - city_treasury_negative
        * (1.0 - admission_negative_treasury_factor)
admission_recent_failure_factor =
    1.0 - admission_recent_failure_penalty * recent_household_failure_pressure
```

Evaluation order:

1. Read every baseline city-level signal and clamp it to `0.0..1.0`.
2. Compute the helper terms above.
3. Compute the residential intermediate terms:

```text
// Desire for new residential capacity: high when the city attracts settlers and buildings
// are nearly full; low when buildings are mostly empty or the city is unwelcoming.
// Uses housing_shortage (not housing_availability) to measure unmet demand for new slots,
// not just ease of filling vacancies that already exist.
inflow_desire =
    clamp(external_connection_available * housing_shortage, 0.0, 1.0)

// Admission pressure fills existing vacancies, then soft-damps new arrivals when
// the existing household economy is already failing.
admission_pressure =
    clamp(
        external_connection_available
        * housing_availability
        * admission_affordability_factor
        * admission_unhoused_factor
        * admission_zero_budget_factor
        * admission_treasury_factor
        * admission_recent_failure_factor,
        0.0,
        1.0
    )

// Removal pressure: households leave when they have no home.
// Future: an evacuation system will extend this signal.
removal_pressure = unhoused_household_ratio

// Net migration balance: positive when the city wants to grow, negative when it wants
// to shrink. Rescaled to 0.0..1.0 with 0.5 as exact equilibrium so that the existing
// GrowthProfile threshold infrastructure applies without modification.
net_residential_demand =
    clamp(inflow_desire - removal_pressure, -1.0, 1.0)
```

4. Compute the city-level `DemandChannel` values consumed by `GrowthProfile`s in this exact order:

```text
ResidentialGrowth =
    clamp(net_residential_demand * 0.5 + 0.5, 0.0, 1.0)

CommercialGrowth =
    clamp(commercial_need * household_purchase_power * external_connection_available, 0.0, 1.0)

IndustrialGrowth =
    clamp(commercial_owa_dependency * external_connection_available, 0.0, 1.0)
```

5. Compute the action-limit gate for building spawns. All use families are uncapped so they can
   bootstrap before the city is large:

```text
ResidentialSpawnLimit    = 1.0
NonResidentialSpawnLimit = 1.0
```

Interpretation:

- `ResidentialGrowth = 0.5` is the equilibrium: the city is neither growing nor shrinking
- `ResidentialGrowth > 0.5` means net inflow desire — more people want in than out, and
  buildings are filling up; spawn threshold fires somewhere above 0.5
- `ResidentialGrowth < 0.5` means net outflow desire — more people are leaving than arriving,
  or existing buildings are mostly vacant; despawn threshold fires somewhere below 0.5
- `inflow_desire` and `admission_pressure` (the household-action signal) are deliberately
  different formulas: `admission_pressure` uses `housing_availability` to fill existing vacancies,
  but is soft-damped by household affordability, existing unhoused households, zero-budget
  households, and negative treasury; `inflow_desire` uses `housing_shortage` to measure unmet
  demand for new capacity — the two naturally complement each other: high healthy vacancy raises
  admission pressure while lowering inflow_desire, so the system fills existing buildings before
  building new ones
- `ResidentialSpawnLimit = 1.0` is safe because `housing_shortage` is already embedded in
  `inflow_desire`; when vacancy is high, `inflow_desire` falls and `ResidentialGrowth` drops
  toward 0.5 or below, which stops spawning without a separate quadratic throttle
- `CommercialGrowth` rises when a real resident/customer base exists, either household stock is
  unstable or commercial output capacity is missing, households have enough short-run buying power
  for essential purchases, and the city is connected enough to support more commerce
- `IndustrialGrowth` is driven by `commercial_owa_dependency` — the fraction of commercial
  input value sourced from OWA imports rather than local industrial — computed from daily shipment
  costs accumulated per building; one farm that partially covers multiple shops produces a smooth
  intermediate signal rather than the binary 0/1 of a headcount ratio
- `NonResidentialSpawnLimit = 1.0` so commercial and industrial buildings can bootstrap without
  waiting for a large population
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

- housing capacity and vacancy (`housing_availability` for household admission; `housing_shortage`
  for residential building spawn)
- resident presence (commercial/industrial demand gating)
- household affordability (commercial demand and soft household-admission damping)
- zero-budget household ratio (soft household-admission damping)
- household stock stability (commercial demand)
- commercial OWA dependency (industrial spawn pressure)
- existence of at least one external connection (hard gate for admission and residential spawn)
- negative city treasury (soft household-admission damping)
- recent household failure/removal memory (soft household-admission damping)

These are city-level signals, not per-agent trip decisions.

Later extensions may add more city-level pressure inputs such as commute burden, broader service
quality, or other wider city-stability summaries, but baseline `v0.1` must stay aligned with the
fixed signal families defined in `Modifiers And Signal Sources`.

## Outputs

The demand layer should produce three distinct output families:

- concrete household-action outputs
- concrete building-action budgets
- ongoing building-growth pressure outputs

Concrete household-action outputs:

- `households_to_admit_today`
- `households_to_remove_today`

Interpretation:

- these are bounded whole-household counts
- they are direct city-growth actions, not vague pressure scores
- economy, household, and vacancy systems consume them to create or remove real households

Deterministic pressure-to-action rule for household outputs:

- household admission and removal start from normalized whole-city pressure values in `0.0..1.0`
- each action has a fixed threshold in `0.0..1.0`
- pressure below threshold produces `0` action on that cadence
- pressure above threshold produces a bounded household count derived from the excess above
  threshold and the active cadence fraction

Deterministic conversion rule:

```text
normalized_action_pressure =
    clamp((pressure - action_threshold) / (1.0 - action_threshold), 0.0, 1.0)

action_credit += normalized_action_pressure * max_households_per_day * cadence_fraction

households_to_act = floor(action_credit)
action_credit -= households_to_act
```

Authoring rule:

- `admission_threshold = household_action.admission_threshold`
- `removal_threshold = household_action.removal_threshold`
- `max_households_per_day = action_budget.max_households_per_day` from
  [`demand/growth_profiles.toml`](../demand/growth_profiles.toml)
- admission uses `action_threshold = admission_threshold`
- removal uses `action_threshold = removal_threshold`
- baseline `v0.1` uses one shared authored daily household-action cap for both admission and
  removal instead of hard-coded runtime constants; admission uses `cadence_fraction = 1/24`, while
  removal uses `cadence_fraction = 1.0`
- persistent exit uses `persistent_exit_daily_fraction` and
  `persistent_exit_eligible_household_count` after the crisis-removal budget has been computed:

```text
persistent_exit_credit +=
    persistent_exit_eligible_household_count * persistent_exit_daily_fraction

persistent_exit_removals =
    min(
        floor(persistent_exit_credit),
        persistent_exit_eligible_household_count,
        max_households_per_day - crisis_removals_this_day
    )

persistent_exit_credit -= persistent_exit_removals
```

Interpretation:

- the farther pressure is above the threshold, the faster household action accumulates
- weak but persistent pressure still produces deterministic action over multiple cadence steps
- stronger pressure produces larger household counts, but never above the bounded daily cap
- persistent exit prevents a small failed-unhoused tail from staying forever just below the
  citywide crisis-removal threshold

Concrete building-action budgets:

- `residential_spawns_this_hour`
- `commercial_spawns_this_hour`
- `industrial_spawns_this_hour`
- `residential_upgrades_this_hour`
- `commercial_upgrades_this_hour`
- `industrial_upgrades_this_hour`
- `residential_downgrades_this_hour`
- `commercial_downgrades_this_hour`
- `industrial_downgrades_this_hour`
- `residential_despawns_this_hour`
- `commercial_despawns_this_hour`
- `industrial_despawns_this_hour`

Interpretation:

- these are bounded whole-building or whole-site counts, not vague pressure scores
- demand computes them once per hourly pass from one frozen hourly city snapshot and one frozen eligible-candidate
  snapshot
- buildings placed, upgraded, downgraded, or removed during that pass do not change the same-hour
  budgets; they affect the next hourly demand pass
- there is no separate allocator-owned arbitrary cap on top of these demand-owned budgets

Deterministic budget rule for building actions:

For each use family `use` and action type `action`, demand first builds the eligible candidate list
from the frozen hourly snapshot. It then computes the bounded budget from the relevant normalized
action pressure, the eligible candidate count, the carried action-credit buffer, and
`cadence_fraction = 1/24`.

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
  * cadence_fraction

spawns_this_pass[use] =
    min(eligible_spawn_count[use], floor(spawn_action_credit[use]))

spawn_action_credit[use] -= spawns_this_pass[use]
```

For upgrade:

```text
normalized_upgrade_pressure =
    clamp((growth_pressure - upgrade_threshold) / (1.0 - upgrade_threshold), 0.0, 1.0)

upgrade_action_credit[use] +=
    normalized_upgrade_pressure
  * eligible_upgrade_count[use]
  * upgrade_batch_fraction_by_use[use]
  * cadence_fraction

upgrades_this_pass[use] =
    min(eligible_upgrade_count[use], floor(upgrade_action_credit[use]))

upgrade_action_credit[use] -= upgrades_this_pass[use]
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
  * cadence_fraction

downgrades_this_pass[use] =
    min(eligible_downgrade_count[use], floor(downgrade_action_credit[use]))

downgrade_action_credit[use] -= downgrades_this_pass[use]
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
  * cadence_fraction

despawns_this_pass[use] =
    min(eligible_despawn_count[use], floor(despawn_action_credit[use]))

despawn_action_credit[use] -= despawns_this_pass[use]
```

Authoring rule:

- `spawn_batch_fraction_by_use = action_budget.spawn_batch_fraction_by_use`
- `upgrade_batch_fraction_by_use = action_budget.upgrade_batch_fraction_by_use`
- `downgrade_batch_fraction_by_use = action_budget.downgrade_batch_fraction_by_use`
- `despawn_batch_fraction_by_use = action_budget.despawn_batch_fraction_by_use`
- all of those tables are authored in [`demand/growth_profiles.toml`](../demand/growth_profiles.toml)
- baseline `v0.1` does not hard-code these fractions in runtime code

Deterministic execution order:

- hourly demand telemetry refreshes `ResidentialGrowth`, `CommercialGrowth`, and
  `IndustrialGrowth` from the latest operational-hour economy state
- hourly household admission advances `admission_action_credit` at `1/24` of the authored daily
  household cap, then consumes `households_to_admit_today` immediately by launching household
  arrival carriers for available reserved homes
- hourly private building actions advance their carried action credits at `1/24` of the authored
  daily building-action budget, then execute the selected private building plan immediately
- the `00:00` hourly demand pass runs after daily settlement and `households_to_remove_today`, so
  the system still executes 24 hourly `1/24` demand slices per operational day
- daily demand output consumption at the midnight boundary is limited to
  `households_to_remove_today`
- `households_to_remove_today` uses only the deterministic settled-snapshot candidate order owned
  by [`economy.md`](economy.md); hourly admissions must not enter that same daily removal candidate
  set
- household admission claims vacancy on the hourly cadence after residential capacity exists; fresh
  residential spawns from an hourly pass may be filled by the next hourly admission pass
- demand builds all eligible candidate lists once per hourly building-action pass before any building change is
  executed
- use families iterate in this exact order for the hourly building-action pass: `residential`, then `commercial`,
  then `industrial`
- spawn candidates sort by the allocator's deterministic build-site order:
  `(edge_idx, side_order, cell_x, width_cells, depth_cells, zone_profile_id)` where
  `side_order = [1, -1]`
- existing-building candidates for upgrade, downgrade, and despawn sort by the building's
  attachment order:
  `(edge_idx, side_order, cell_x, width_cells, depth_cells, level, asset_id)`
- for each use family, execution order is `despawn`, then `downgrade`, then `upgrade`, then
  `spawn`
- a building or empty site may be affected at most once per hourly demand pass

Interpretation:

- large legally zoned estates under strong pressure can fill in visible hourly batches rather than
  one building at a time
- weak but persistent pressure still produces deterministic building change over multiple days
- startup support can intentionally make early-city spawn batches larger so the city forms a real
  starter neighborhood instead of stalling at tiny-village scale
- newly arrived households may receive workplaces on a later economy tick, but hourly admission
  does not rerun daily settlement or daily removal selection

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
   - `household_affordability`
   - `household_stock_stability`
   - `external_connection_available`
   - `commercial_owa_dependency`
5. Compute the city-level `DemandChannel` values from that same frozen snapshot.
6. Compute `households_to_remove_today` from that same frozen snapshot.
7. Execute the resulting demand-owned removal action before the next operational day's sub-daily
   economy steps begin.

Deterministic cadence rule:

- demand refreshes RCI telemetry, household admission, and private building-action budgets once per
  completed operational hour
- hourly admission and building-action credits use `cadence_fraction = 1/24` against their authored
  daily caps
- household removal is rebuilt only on the daily demand pass from the frozen post-settlement
  snapshot

Interpretation:

- demand does not read half-settled hourly economy state
- demand does not recompute after each spawned, removed, upgraded, or downgraded building inside
  the same hourly pass
- demand-owned hourly actions change the city's runtime state for the next operational hour and
  therefore affect the next hourly demand pass
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

Two distinct pressure formulas drive the residential system. They share the same economic
signals but serve different purposes.

**`admission_pressure`** — fills existing vacancies. Used only for the household-action
`households_to_admit_today` output. High when housing is available and the existing household
economy is healthy enough to absorb new arrivals:

```text
admission_pressure =
    clamp(
        external_connection_available
        * housing_availability
        * admission_affordability_factor
        * admission_unhoused_factor
        * admission_zero_budget_factor
        * admission_treasury_factor
        * admission_recent_failure_factor,
        0.0,
        1.0
    )
```

**`inflow_desire`** — drives new residential building capacity. Used only in `ResidentialGrowth`.
High when buildings are nearly full and the city is welcoming (unmet demand for new slots):

```text
inflow_desire =
    clamp(external_connection_available * housing_shortage, 0.0, 1.0)
```

Relationship: `admission_pressure` and `inflow_desire` are complementary by design.
High vacancy → `housing_availability` is high → `admission_pressure` is high (admit people
fast) when the city is healthy, and `housing_shortage` is low → `inflow_desire` is low (no need
for new buildings). If the city already has broke or unhoused households, vacancy remains a real
signal but no longer overrides the soft health damping.

`recent_household_failure_pressure` is demand-owned carried state in `0.0..1.0`. Each daily
demand pass first decays the previous value by `recent_failure_daily_decay`, then raises it to at
least that settled day's `failure_pressure`. After actual demand-owned household removal executes,
the same memory is raised to at least `removed_households / total_household_count` for the
pre-removal settled snapshot. The value is persisted with `demand_state`, because it affects
admission behavior after save/load.

**`removal_pressure`** — drives household emigration and residential despawn. Shared between
the household-action `households_to_remove_today` output and the `net_residential_demand` term
inside `ResidentialGrowth`. Households leave when they have no home:

```text
unhoused_household_ratio =
    if total_household_count == 0 then 0.0
    else clamp(unhoused_household_count / total_household_count, 0.0, 1.0)

removal_pressure = unhoused_household_ratio
```

Where:

- `total_household_count = housed_household_count + unhoused_household_count`
- `unhoused_household_count` is read from the settled economy snapshot after relocation and
  eviction have already run for that operational day
- `removal_threshold = 0.55` means removal fires when more than 55% of households are unhoused

The ratio threshold is the **crisis outflow** rule, not the only household-exit rule. Demand also
computes a persistent-exit candidate count from explicit economy-owned household state:

```text
destitute_unhoused =
    home_is_none
    AND budget <= epsilon
    AND stock_days <= persistent_exit_destitute_stock_days

persistent_exit_eligible =
    home_is_none
    AND (
        destitute_unhoused
        AND unhoused_days_elapsed >= persistent_exit_destitute_unhoused_days
    OR
        unhoused_days_elapsed >= persistent_exit_max_unhoused_days
    )
```

With the shipped values, a household with no home, no money, and no meaningful stock becomes
persistent-exit eligible after 2 settled unhoused days. Any household still without a home becomes
persistent-exit eligible after 7 settled unhoused days, even if it briefly retained money or stock.
Persistent exit then removes a deterministic fraction of the eligible tail each day after crisis
removal has consumed its budget share.

Runtime diagnostics also expose a broader `failure_pressure` for analysis:

```text
failure_pressure = max(unhoused_household_ratio, zero_budget_household_ratio)
```

This is logged beside `removal_pressure`, raw household counts, persistent-exit eligible count,
the removal threshold, crisis-removal credit, persistent-exit credit, planned removals, and
actually removed households. `failure_pressure` still does not directly set the removal count; it
drives recent-failure admission damping and diagnostics.

Interpretation:

- ordinary low money does not directly remove housed households from the city in baseline `v0.1`
- household affordability failure first flows through the economy-owned relocation, eviction, and
  `unhoused` rules described in [`economy.md`](economy.md)
- once the settled snapshot contains persistent `unhoused` households, demand may convert that
  explicit state into whole-household city removal
- baseline `v0.1` household outflow therefore comes from sustained failed living conditions, not
  from one bad hourly dip or one direct poverty-to-deletion shortcut

## Deterministic `v0.1` Rules

For `v0.1`, the immigration rules are simple and deterministic:

- refresh RCI telemetry, household admission, and private building actions on a coarse hourly
  cadence
- evaluate emigration/removal on the daily settled-snapshot cadence
- if there is no valid housing capacity, admit `0` households
- if there is no required external connection, admit `0` households
- admit whole households only
- the number admitted per hour must come from the authored daily cap scaled by `1/24`
- the result should come from coarse pressure signals, not from a hidden startup path or a transport-state side effect

If a household is admitted:

- housing/vacancy logic claims or reserves one real home immediately, so no second pending
  household can consume the same slot
- transport visualizes baseline `v0.1` admission with one border-origin household carrier car,
  not one car per resident
- the carrier represents the whole pending household while it is in transit; it may carry the
  deterministic household size and later age-group composition needed to materialize the household
- economy creates the actual household record and resident agents when the carrier reaches the
  claimed home
- if no required external connection or no legal border-to-home car route exists, admission waits
  instead of silently instantiating the household inside the home

The `economy` debug log emits a compact household-admission diagnostic line for each hourly pass.
It reports the raw admission pressure, base vacancy pressure, individual soft-damping factors,
the recent-failure memory and factor, threshold, normalized action pressure, carried admission
credit, planned households, and actually launched arrival carriers.

The daily `city flow diagnostics` line summarizes the settled city state after daily household
settlement, removal execution, and the midnight demand pass. It reports:

- `net_households`, `admitted_since_daily`, and `removed_today` for the household flow since the
  previous daily diagnostic
- housed, unhoused, zero-budget, stock-empty, and low-stock household counts
- resident agents, pending household carriers, employed residents, unemployed residents, and open
  commercial or industrial job capacity
- occupied/vacant household slots and treasury balance

This line is diagnostic only. It must not become a new demand input or hidden repair path. Its
purpose is to distinguish healthy low-volume inflow from churn: in a crisis run, outflow plus
persistent exit should exceed or roughly balance the damped trickle of new admissions unless the
city has recovered enough jobs, household budgets, and stock stability to grow again.

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
   build-site order, and computes `spawns_this_hour[use]` from the hourly `1/24` batch-budget rule
   described above.
4. Zoning and allocator rules must then find at least one legal build site for the active
   `ZoneProfile`.
5. Economy viability must allow the spawn for that zone type:
   - `Residential`: enough demand plus a legal site and ordinary residential viability
   - `Commercial`, `Industrial`: enough demand plus a legal site and the relevant business-side
     viability gate from [`economy.md`](economy.md)
6. The allocator chooses up to `spawns_this_hour[use]` concrete legal build sites from the sorted
   candidate list.
7. Zoning-side asset selection chooses one legal `level = 1` asset deterministically from the
   allowed family pool.

Interpretation:

- city-level growth pressure alone does not create a building
- zoning alone does not create a building
- economy viability alone does not create a building
- spawn happens only when demand, zoning, economy, and allocator all pass, and only up to the
  bounded hourly spawn budget for that use family
- baseline `v0.1` fresh spawn stays at `level = 1`; higher-level direct spawn is a later explicit
  extension

### Non-Residential Spawn Gates

Commercial and industrial buildings spawning into a neighbourhood with no workers to staff them,
or no consumers to absorb their output, is the root cause of zombie businesses and oversupply.
Two deterministic per-candidate gates enforce economic readiness before a non-residential spawn
is allowed to execute.

These gates apply **after** the budget (`spawns_this_pass[use]`) has been computed and **during** final
candidate selection. Candidates that fail either gate are removed from the selected set; the
remaining budget is not redistributed to other candidates on the same hourly pass.

#### Labour Gate

A non-residential spawn candidate is rejected if the city cannot plausibly staff it.

Deterministic rule:

```text
available_unemployed =
    housed_resident_count   # from the frozen hourly snapshot

required_workers = worker_capacity of the candidate's bound economy profile
                   (0 if the profile has no workers, e.g. utility nodes)

labour_gate_passes =
    required_workers == 0
    OR available_unemployed >= required_workers
```

Where:

- `housed_resident_count` is taken from the frozen hourly economy snapshot
- `required_workers` is read from the compiled economy catalog for the candidate's bound profile
- a candidate with `required_workers == 0` always passes (utility buildings, warehouses)
- if the economy profile binding is missing or the catalog cannot be read, the gate fails safe
  (spawn is rejected)
- the gate consumes `required_workers` from the running `available_unemployed` count as each
  candidate passes, so a single hourly pass does not spawn more buildings than there are workers

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

- occupies its parcel claim, blocking new spawns at that location
- is ineligible for upgrade or downgrade consideration
- is not counted in building-level demand snapshot signals (e.g. `commercial_owa_dependency`)
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

- The coarse immigration decision has now moved behind the hourly demand-owned
  `households_to_admit_today` output, and allocator execution consumes that count through
  `execute_demand_household_admission(...)` rather than recomputing pressure locally.
- Ordinary household admission now launches one border-origin carrier through
  `rust/src/simulation/economy/agents/data.rs::spawn_household_arrival_carrier()`. That carrier is
  the only ordinary household-admission use of `TRANSIT_IMMIGRATING`; resident agents are
  materialized at the claimed home after the carrier arrives.
- Ordinary private-building despawn, downgrade, upgrade, and spawn now execute from demand-owned
  hourly action plans. Stale topology or zoning cleanup remains only as invalid-placement cleanup,
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
