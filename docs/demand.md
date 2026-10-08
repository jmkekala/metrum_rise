# Demand

This document owns household admission and removal, private construction and failed-site clearance,
and building upgrades and downgrades. Growth uses concrete household and project decisions;
the obsolete citywide R/C/I pressure calculation and saved values are removed (format 83). The baseline `v0.1` ownership path is now live, while later extensions in
this document remain work in progress.

It answers questions like:

- should the city admit more households
- should the city lose households
- which residential, commercial, or industrial projects have supportable customers and workers
- should private buildings appear, disappear, upgrade, or downgrade
- is the city healthy enough to support future private development

It does not own household data layout, freight movement, or door-to-door travel.

The [healthy-growth plan](economy.md#growth-redesign-econ-11econ-14-dem-02) is being implemented
under `DEM-02`. The [implementation contract and evidence](#implementation-contract-and-progress)
own the current redesign status. Earlier formulas below document the original pressure system;
their hypothetical-family runway, marginal shop jobs, regional household cutoff and admission
distress penalties have been removed. They are historical reference, not current runtime rules.
Economic viability and demand simplification are coordinated work, with 10k performance checks
and the 1M run deferred.

The [village failure report](economy.md#village-failure-timeline-econ-14) identified a recovery
defect under `DEM-02` / `ECON-14`: positive demand stranded 32 deserted commercial buildings.
The 2026-10-06 repair makes deserted, empty commercial and industrial sites eligible for clearance
regardless of demand. It retains the existing bounded action credits, deterministic order and
occupancy safeguards. The format-78 cleanup extends this to every private use: only deserted,
empty sites clear; healthy vacant buildings remain available for recovery.

Recovery demolition evacuates remaining physical occupants through the existing street-handoff
and route-home lifecycle before removing the entrance and remapping building indices. Empty
employment slots alone do not mean that all former workers have left the building.

Service-store candidates must also pass a first-worker break-even check: remaining unreserved
resource demand, capped to one worker's throughput, must pay that worker, imported recipe inputs,
current-policy property tax and imported utility overhead. At the current 1.25× import price,
level-one personal and health services need at least 47 and 52 residents respectively after
the 2026-10-06 village-service tuning (previously 156/166), if all
their spending is available to the candidate. This is a necessary market-size check, not proof
of household affordability or guaranteed receipts. The existing capacity reservation prevents
several candidates from claiming the same customers; the explicit max-demand cheat bypasses it.

## Machinery and production-chain demand (`ECON-09`)

Machinery upkeep creates upstream industrial demand through existing input/output ports. OWA
imports fulfill orders while leaving local production opportunities visible. Industrial spawn
counts use resource-specific unmet demand against matching asset output, rather than dividing an
aggregate currency deficit by unrelated factory output. Selected candidates immediately reserve
net output for the remaining selection pass, preventing repeated factories for the same demand.
Depleted mines clear productive area, so they contribute neither future output capacity nor
Machinery input demand; their saved geometry and remaining saleable stock are retained.
Factories under construction reserve their output minus their own same-resource upkeep; live
factories count gross output and actual upkeep demand. Input demand uses the same active staffing
cap as production, including farm jobs reduced by available customers. No producer asset means no
corresponding spawn; the HUD identifies the missing asset rather than displaying import pressure.

Initial rates, material recipes and authoring details are owned by
[`economy.md`](economy.md#machinery-upkeep-econ-09).

## Cross-Doc Implementation Order

The original integration established zoning/asset legality, demand-owned decisions, and the
economy handoff. Those foundations are live. The current order is the
[healthy-growth delivery plan](economy.md#delivery-plan): define viable budgets and customers,
repair startup, extend economic activity, and coordinate `DEM-02` with `ECON-14`. Reuse the
existing zoning, household, service and freight systems; do not repeat their initial integration.

## Current Runtime Snapshot

The live game now has a larger Rust-side `DemandSystem`. The baseline `v0.1` ownership path
described here is implemented, while broader extensions described later in this document remain
future work.

Current live behavior:

- `DemandSystem` loads and validates the shipped `demand/growth_profiles.toml` tuning file at
  startup
- it evaluates concrete projects from each completed operational-hour snapshot; daily departures
  reuse the household census without rebuilding the business market
- it computes and persists `households_to_admit_today` and `households_to_remove_today`;
  admission capacity refreshes hourly, while removal retains its daily action credits.
  Format 83 stores concrete counts and pacing state, without R/C/I pressure values
- it also computes and persists the carried building-action credits for upgrade, downgrade,
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
- selected existing-building actions use current parcel ownership and revalidate the full selection
  key before mutation; lookup does not copy the whole city (`AUDIT-01-B2`, measured in
  [`building_allocator.md`](building_allocator.md))
- those building-action plans now also pass through economy-side viability gates backed by the
  authored `runtime_tuning` block in `economy/profiles.toml`
- industrial building actions now read explicit input-coverage and output-headroom signals from the
  live resource-typed building inventory state instead of treating industrial viability as pure
  staffing-plus-buffer approximation
- household admission tries a bounded set of actual applicants. Live revenue-backed jobs,
  available homes and recurring income determine acceptance. Existing residents and pending
  arrivals claim wages and transfer coverage first; one rejected household cannot block the batch
- there is no hypothetical-family preview, search-runway calculation, regional household cutoff,
  or repeated citywide distress penalty. Empty-town housing can start with one offered home;
  funded construction can pair with homes using completion-time workforce reservations
- each physically launchable household also claims the batch's remaining recurring-income
  capacity. Its approved adult/elder counts travel with the carrier and survive arrival reordering
  and save/load; incoming adults are included in the next hourly labor forecast
- gameplay cheat mode is the explicit exception to the ordinary organic demand model: once enabled,
  it persists an explicit viability bypass in saves and lets each
  baseline private-use family with legal candidates plan at least one spawn per demand pass without
  the ordinary non-residential output-absorption gate

Current derived inputs:

- housing capacity and vacancy
- housed-resident presence
- open and filled job capacity
- actual applicant income, affordable consumer spending and individual exit eligibility
- connected external-border availability
- economy-side residential and non-residential viability gates from `economy/profiles.toml`
- industrial input coverage plus industrial output headroom from the live starter inventory slice

Short version:

- the current system owns hourly household-admission and daily household-removal decisions
- it also now owns the city's ordinary private building-change decisions through hourly action plans
- baseline fresh-map startup and later household growth now flow through explicit demand-owned
  signals; no hidden allocator-owned founding path or static R/C/I pioneer floor remains

## Terminology Conventions

This document uses the following terms consistently:

- `current DemandSystem`: the live Rust system that computes hourly household-admission, daily
  household-removal outputs and private building-action plans with shared reservations
- `target demand layer`: the full authoritative growth controller described by this document,
  including later extensions that are not all live yet
- `GrowthProfile`: retired per-density threshold configuration; current demand uses direct project checks
- `build site`: one candidate legal location where a private building could appear, disappear, upgrade, downgrade, or be replaced. The concrete frontage-attached runtime shape of a build site is owned by [`building_allocator.md`](building_allocator.md); this remains a gameplay term, not a cadastral parcel system
- `pioneer household growth`: historical term for the now-removed static demand floor that was
  applied to fresh maps; replaced by household transfers in [`economy.md`](economy.md), which
  provide early-city solvency through real economic activity

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

The old per-density `GrowthProfile` threshold registry is removed. Project viability and shared
reservations decide eligibility; explicit action fractions pace accepted opportunities. Zoning
profiles describe legal use, density, asset tags and presentation, without `growth_profile_id`.
Format 81 removes the last saved lifecycle hysteresis flags.

[`demand/growth_profiles.toml`](../demand/growth_profiles.toml) retains demand-owned pacing,
individual household-exit eligibility and diagnostic normalization. Its filename is unchanged;
it no longer contains a `[[profiles]]` registry. Economy-owned operating costs, household
composition and tier viability remain in [`economy/profiles.toml`](../economy/profiles.toml).

The demand loader validates these tables once and shares the immutable result:

- diagnostic affordability/stock targets and benefit coverage must be finite and positive;
- the destitute pantry threshold must be finite and in `0..365` days;
- individual exit durations must be at least one day;
- departure and building-action fractions must be finite and in `0..1`;
- each building-action table must provide residential, commercial and industrial fractions;
- the household departure cap is a nonnegative integer, and unknown demand fields are rejected.

Construction opportunities do not need citywide percentages. Upgrades use local viability and
shared claims. Business downsizing must repair an unsupported operation with a supportable
replacement; residential downsizing may retire only surplus ready homes after workplace claims.
Rejected proposals preserve action credit. Cheat mode explicitly bypasses ordinary growth
viability checks and disables downsizing, while retaining site/asset legality.

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
- `city_treasury_balance`
- `candidate_household_size`
- `candidate_child_count`
- `candidate_adult_count`
- `candidate_elder_count`
- `candidate_daily_essential_cost`
- `immigrant_starter_savings_per_household`
- `unemployment_daily_benefit_per_adult`
- `pension_daily_benefit_per_elder`
- `child_support_daily_benefit_per_child`
- `existing_unemployed_adult_count`
- `existing_child_count`
- `existing_elder_count`
- `open_job_slots`
- `marginal_commercial_job_household_pull` — forecast-only household pull from budget-backed
  commercial store worker-equivalent demand if one candidate household's demand were included; it
  is continuous below integer staffing thresholds, capped to one candidate household, net of
  existing unemployed adults after current open jobs are consumed, and does not hire workers before
  that household exists
- `marginal_commercial_job_slots` — integer forecast-only commercial job slots that would become
  active if one candidate household's demand crossed a staffing threshold
- `marginal_commercial_job_equivalent_slots` — fractional commercial worker-equivalent progress
  caused by one candidate household, with integer threshold crossings promoted to at least the
  unlocked slot count
- `move_in_job_slots` — physical current or forecast-unlocked job slots remaining for the
  candidate household after existing unemployed adults are counted against job demand
- `move_in_job_equivalent_slots` — remaining current open job slots plus remaining fractional
  forecast-only marginal commercial worker-equivalents used for move-in viability and expected
  income
- `average_move_in_job_wage_per_day`
- `regional_growth_household_pull` — an authored outside-world household attraction signal after
  external connection, city-health damping, and the soft household target are applied
- `industrial_input_capacity_deficit` — fraction of business input value not covered by live
  local industrial output capacity; this is one `IndustrialGrowth` pressure source
- `business_input_need_value` — daily value of active business and utility input demand
- `local_industrial_input_capacity_value` — daily value of live local industrial output capacity
  for resources consumed by business and utility profiles
- `industrial_missing_input_value` — daily business input value still missing after local
  industrial capacity is subtracted resource-by-resource
- `business_owa_dependency` — fraction of business input value sourced from OWA imports rather
  than local industrial; this is the actual-throughput import-substitution signal for
  `IndustrialGrowth`

Baseline ownership rule:

- `housing_availability` comes from housing capacity and vacancy state owned by economy/building
  systems
- `household_affordability` comes from household budgets and essential-cost state owned by economy
- `zero_budget_household_ratio` is derived from all active household records, including unhoused
  households, so failed households cannot be hidden by affluent housed survivors
- `household_stock_stability` comes from household stock buffers owned by economy
- `commercial_capacity_deficit` is derived by the demand snapshot from catalog demand-sink input
  resources that a `store` or `service_store` commercial profile can produce, comparing
  housed-resident per-resource demand against live non-deserted commercial output capacity
- `external_connection_available` comes from network-border connectivity owned by the road/network
  layer
- `regional_growth_household_pull` is derived by demand from authored demand tuning plus the
  current household count and normalized city-health signals. It is not an allocator-owned spawn
  exception and it does not bypass admission, vacancy, or move-in viability gates.
- `city_treasury_balance` is read from the city treasury owned by the fiscal ledger
- `candidate_household_size`, `candidate_child_count`, `candidate_adult_count`, and
  `candidate_elder_count` are the exact next claimable starter immigrant composition from the
  residential vacancy index. They come from the same deterministic starter mix used by actual
  household admission, may describe a one-person household, and are capped by the authored
  flat-size capacity so a home's maximum fit is not treated as the requested household size.
  When current budget-backed open jobs remain after existing unemployed adults are counted, the
  preview prefers a claimable household with at least one adult worker before falling back to the
  deterministic smallest-vacancy order.
  The seed includes households still travelling into town; materialization uses the approved
  counts rather than resampling from its eventual household index.
- `candidate_effective_workers` is `candidate_adult_count`; children and elders contribute no
  worker capacity
- `candidate_daily_essential_cost` combines household demand-sink resource prices, including
  aggregate service-store demand resources, and imported utility costs for that candidate size.
  Electricity reserves use authored residential kWh rates and include one conservative home base
  load per candidate household, separately from per-resident costs;
  household power arrears reduce the cash available for housing reserves
- `immigrant_starter_savings_per_household` comes from economy-owned household starter tuning
- transfer amounts come from live economy-owned `CityFiscalPolicy`
- `existing_unemployed_adult_count`, `existing_child_count`, `existing_elder_count`, and
  `open_job_slots` come from the settled household and job-offering building state. Existing
  unemployed adults include pending adult arrivals; pending children and elders also contribute
  to forecast transfer claims. Children and elders are not work-eligible.
  Open jobs count commercial, industrial, and utility/service profile slots with a positive
  authored wage and current operating budget
- `move_in_job_slots`, `move_in_job_equivalent_slots`, and
  `average_move_in_job_wage_per_day` combine current budget-backed open jobs with forecast-only
  marginal commercial jobs for candidate household runway checks. Integer marginal commercial
  slots describe physical jobs that would unlock after admission; fractional marginal commercial
  worker-equivalents describe expected partial employment/income from the candidate's own demand.
  Admission prices that income from the worker-equivalents only: rounding up a physical vacancy
  must not promote fractional recurring sales into a full wage.
  Neither form hires a live worker before the household exists.
- `business_input_need_value`, `local_industrial_input_capacity_value`,
  `industrial_missing_input_value`, and `industrial_input_capacity_deficit` are derived by the
  demand snapshot from compiled economy-profile ports. Market-scaled commercial inputs plus staffed industry, farm, mine and utility inputs define the target;
  live non-deserted industrial and area-producer outputs for those same resources define local capacity; missing
  value is computed resource-by-resource before being summed.
- `business_owa_dependency` is derived by the demand snapshot from daily per-building
  `daily_owa_input_value` and `daily_local_input_value` accumulators, reset after each snapshot:
  `total_owa / max(total_owa + total_local, total_expected_business_input_value)` across active
  business and utility buildings, `0.0` when none have transacted yet. This
  raises industrial pressure when the city actually relies on outside inputs, but it does not
  directly set industrial spawn count.

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

Household-slot totals include the single home provided by each operational farm. Its field area
scales employment independently; the farm stays outside residential zoning/growth candidates.
The normal vacancy and admission rules fill its home (`ECON-08`; see
[`economy.md`](economy.md#farm-households-econ-08)).

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

regional_growth_household_pull =
    external_connection_available
    * regional_growth_household_pull_config
    * clamp(1.0 - total_household_count / regional_growth_soft_households, 0.0, 1.0)
    * min(household_affordability, household_stock_stability)
    * clamp(1.0 - max(unhoused_household_ratio, zero_budget_household_ratio), 0.0, 1.0)
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
- fiscal state affects household admission only through the transfer-backed move-in acceptance
  formula below; demand reads the current policy values and treasury balance but does not own
  transfer disbursement
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

Industrial local customer value includes operating business input needs. Committed output,
including construction and candidates selected in the current pass, reserves those customers.
Outside buyers have no quantity cap and are never converted into infinite unmet output units.
With a connected freight gateway and positive export bid, demand grants at most one export-led
opening at a time: existing zoned industry must be live and employing workers, show paid revenue,
and hold more operating cash than its original seven-day payroll
plus input-buffer startup grant. Construction, deserted/broken industry and unrecovered startup
cash block that opportunity. A bulk-input payment can make one day cash-negative without erasing
earned reserves; factories need not synchronize profitable days. Local unmet demand remains
eligible independently.

Industrial openings also require either unemployed adults beyond existing job vacancies or room
for additional households: vacant/unfinished homes, residential placement candidates, or eligible
home upgrades. The building-action pass reuses its existing residential candidates for this O(1)
gate. Outside customers alone cannot justify an unstaffable startup after housing is saturated.
Max-demand cheat mode bypasses this gate along with the other ordinary opening checks.

A producer/processor candidate must cover full payroll, imported external inputs, property tax,
utilities and the minimum possible dispatch cost. Prospective sites have no freight route quote;
this is a necessary lower-bound check, not a promise of profitability. Actual route fees affect
cash recovery before the next export-led opening. Same-resource upkeep is netted from saleable
output. Selecting an industrial candidate consumes the export opening for the current pass;
ordinary construction then reserves it in later snapshots. Need is local missing-building count,
or one export opening, never a fictitious volume of outside customers. The readiness boolean
reuses the existing deterministic parallel building reduction, with O(1) fixed-profile work per
building and no additional route searches.

Employment now lets unemployed adults accept reachable funded jobs even when household reserves
are healthy, matching the local labor pool subtracted from admission opportunities. Existing
workers retain their job-switch ranking and locks.

Admission forecasts grocery jobs from recurring consumption, excluding temporary pantry refill
work. Candidate essential bills include demand-sink consumption with household VAT, imported
utility costs and property tax at the candidate home's level. Starter cash and unemployment
benefits still describe search runway, but only wages and funded ongoing benefits can cover a
permanent household. This is an initial viability constraint; broader pressure simplification
remains `DEM-02` work.

Batch execution must not reuse the first candidate's wages for every vacant home. Demand keeps
one scalar budget of remaining worker-equivalents, net entry wage, transfer claims and per-member
essential costs from the hourly snapshot. For each legal border-to-home arrival, it checks the
actual family and home-level property tax, then reserves that family's adults and transfer
claims. A refusal changes neither the budget nor housing occupancy. The marginal commercial
forecast is credited once to the batch, not again per household. This is O(1) arithmetic per
candidate inside the existing bounded launch loop; the existing vacancy index and border routing
are reused. Prepopulated benchmark fixtures explicitly bypass demand admission.

Pending counts are accumulated during the existing carrier-materialization pass, adding no scan
or allocation. Arrival turns those counts into ordinary residents before the next forecast;
invalidated carriers cease claiming capacity. This reserves aggregate forecast capacity, not a
specific employer contract: the existing reachable-work search still owns hiring. The model does
not promise employment under later business closures, disconnected roads or policy changes.
The outside applicant seed advances with simulation hours, independently of the number admitted:
rejecting an unaffordable family cannot freeze the stream. Preview and launch use the same seed;
the approved composition is stored on the carrier rather than recomputed from arrival order.
The hour seed is derived again from the saved clock on load.
Save format 72 stores two u16 composition counts per agent; inactive carrier counts are ignored.
Pre-72 pending carriers lacked an approved mix, so loading chooses it once from their saved ids;
existing residents retain their saved ages.

```text
goods_shortage   = 1.0 - household_stock_stability
commercial_need  = max(goods_shortage, commercial_capacity_deficit)
industrial_input_capacity_deficit =
    if export_expansion_available then 1.0
    else if business_input_need_value <= EPSILON then 0.0
    else clamp(
        industrial_missing_input_value / business_input_need_value,
        0.0,
        1.0
    )
industrial_import_substitution_pressure = business_owa_dependency
industrial_need =
    max(industrial_input_capacity_deficit, industrial_import_substitution_pressure)
household_purchase_power =
    clamp(
        household_affordability * household_affordability_target_reserve_days,
        0.0,
        1.0,
    )

candidate_effective_workers =
    candidate_adult_count
net_open_job_slots =
    max(open_job_slots - existing_unemployed_adult_count, 0)
existing_unemployed_after_open_jobs =
    max(existing_unemployed_adult_count - open_job_slots, 0)
net_marginal_commercial_job_equivalent_slots =
    max(
        marginal_commercial_job_equivalent_slots - existing_unemployed_after_open_jobs,
        0.0
    )
move_in_job_equivalent_slots =
    net_open_job_slots + net_marginal_commercial_job_equivalent_slots
expected_employed_members =
    min(candidate_effective_workers, move_in_job_equivalent_slots)
expected_unemployed_members =
    max(candidate_effective_workers - expected_employed_members, 0.0)
expected_wage_income_per_day =
    expected_employed_members * average_move_in_job_wage_per_day

existing_transfer_claim_per_day =
      existing_unemployed_adult_count * unemployment_daily_benefit_per_adult
    + existing_elder_count * pension_daily_benefit_per_elder
    + existing_child_count * child_support_daily_benefit_per_child
candidate_unemployment_claim_per_day =
    expected_unemployed_members * unemployment_daily_benefit_per_adult
candidate_pension_claim_per_day =
    candidate_elder_count * pension_daily_benefit_per_elder
candidate_child_support_claim_per_day =
    candidate_child_count * child_support_daily_benefit_per_child
candidate_transfer_claim_per_day =
      candidate_unemployment_claim_per_day
    + candidate_pension_claim_per_day
    + candidate_child_support_claim_per_day
total_transfer_claim_per_day =
    existing_transfer_claim_per_day + candidate_transfer_claim_per_day
transfer_reliability =
    if total_transfer_claim_per_day <= EPSILON then 1.0
    else clamp(
        max(city_treasury_balance, 0.0)
        / (total_transfer_claim_per_day * move_in_benefit_treasury_coverage_days),
        0.0,
        1.0
    )
expected_transfer_income_per_day =
    candidate_transfer_claim_per_day * transfer_reliability
expected_daily_income =
    expected_wage_income_per_day + expected_transfer_income_per_day
recurring_income_per_day =
    expected_wage_income_per_day
    + (candidate_pension_claim_per_day + candidate_child_support_claim_per_day)
      * transfer_reliability
daily_deficit =
    max(candidate_daily_essential_cost - expected_daily_income, 0.0)
move_in_search_runway_days =
    if daily_deficit <= EPSILON then move_in_target_search_runway_days
    else immigrant_starter_savings_per_household / daily_deficit
move_in_runway_factor =
    if recurring_income_per_day + EPSILON < candidate_daily_essential_cost then 0.0
    else clamp(
        (move_in_search_runway_days - move_in_min_search_runway_days)
        / (move_in_target_search_runway_days - move_in_min_search_runway_days),
        0.0,
        1.0
    )
move_in_acceptance = move_in_runway_factor

admission_unhoused_factor =
    1.0 - admission_unhoused_ratio_penalty * unhoused_household_ratio
admission_zero_budget_factor =
    1.0 - admission_zero_budget_penalty * zero_budget_household_ratio
admission_recent_failure_factor =
    1.0 - admission_recent_failure_penalty * recent_household_failure_pressure
admission_failure_factor =
    admission_unhoused_factor
    * admission_zero_budget_factor
    * admission_recent_failure_factor
residential_construction_viability =
    move_in_acceptance * admission_failure_factor
open_job_household_pull =
    if candidate_effective_workers <= EPSILON then 0.0
    else net_open_job_slots / candidate_effective_workers
marginal_commercial_job_household_pull =
    if candidate_effective_workers <= EPSILON then 0.0
    else min(net_marginal_commercial_job_equivalent_slots / candidate_effective_workers, 1.0)
bootstrap_household_pull =
    if total_household_count == 0 then 1.0 else 0.0
incoming_household_need =
    max(
        open_job_household_pull
            + marginal_commercial_job_household_pull
            + regional_growth_household_pull,
        bootstrap_household_pull
    )
incoming_household_pressure =
    clamp(incoming_household_need, 0.0, 1.0)
```

Evaluation order:

1. Read every baseline city-level signal and clamp it to `0.0..1.0`.
2. Compute the helper terms above.
3. Compute the residential intermediate terms:

```text
// Desire for new residential capacity: high when incoming household pull exists and
// move-in economics are viable. Existing vacancies satisfy this pressure, but do not
// decide whether households want to enter the city.
inflow_desire =
    clamp(
        external_connection_available
        * incoming_household_pressure
        * residential_construction_viability,
        0.0,
        1.0
    )

// Admission pressure is incoming household pull after viability and failure damping.
// Vacant household slots cap execution later; they do not lower this pressure.
admission_pressure =
    clamp(
        external_connection_available
        * incoming_household_pressure
        * move_in_acceptance
        * admission_failure_factor,
        0.0,
        1.0
    )

// Removal pressure: households leave when they have no home.
// Future: an evacuation system will extend this signal.
removal_pressure = unhoused_household_ratio

// Net migration balance: positive when the city wants to grow, negative when it wants
// to shrink. Rescaled to 0.0..1.0 with 0.5 as exact equilibrium for legacy diagnostics only.
net_residential_demand =
    clamp(inflow_desire - removal_pressure, -1.0, 1.0)
```

4. Compute legacy city-level pressure telemetry in this order; these values do not gate actions:

```text
ResidentialGrowth =
    clamp(net_residential_demand * 0.5 + 0.5, 0.0, 1.0)

CommercialGrowth =
    clamp(commercial_need * household_purchase_power * external_connection_available, 0.0, 1.0)

IndustrialGrowth =
    clamp(industrial_need * external_connection_available, 0.0, 1.0)
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
- `incoming_household_need` is the deterministic household pull from current budget-backed open
  jobs that remain after existing unemployed adults are counted first, forecast-only marginal
  commercial worker-equivalents that remain after that same unemployed pool is counted, and regional
  migration pressure, with a one-household bootstrap pull when the city has no households yet
- `admission_pressure` and `inflow_desire` deliberately read the same incoming household pressure:
  vacant homes satisfy that pressure and cap actual admission, but they do not create or remove the
  desire for households to enter the city
- both formulas read deterministic move-in viability and the existing household failure state;
  this keeps residential construction and household admission coupled to the same city-health
  signal without merging them into one action
- failing move-in viability lowers both admission and residential construction pressure
- `ResidentialSpawnLimit = 1.0` is safe because residential spawn need is computed from incoming
  household need and available vacancies; zoning candidates only cap placement
- `CommercialGrowth` rises when a real resident/customer base exists, either household stock is
  unstable or commercial output capacity is missing, households have enough short-run buying power
  for essential purchases, and the city is connected enough to support more commerce
- `IndustrialGrowth` is driven by the stronger of local industrial input-capacity deficit and
  actual business/utility OWA input dependency. Commercial activity and staffed industrial, farm, mine
  and utility operation define input need; industrial and area producers define local output capacity, and the missing value
  ratio captures paper capacity shortage. `business_owa_dependency` captures actual outside-input
  reliance when customer buildings bought inputs from OWA instead of local industry. Industrial
  spawn quantity still uses committed missing input capacity, so existing or under-construction
  local factories prevent duplicate factory spawns even when the pressure signal exposes a failing
  local supply chain.

### Construction Pipeline

Demand starts fresh private construction; economy owns the construction lifecycle once a site is
created. Under-construction buildings are not live housing, job, production, shopping, utility, or
inventory capacity. They do, however, count as committed pipeline capacity for spawn-volume and
absorption decisions so pressure does not repeatedly start duplicate construction while already
claimed sites are still being built.

Rules:

- spawn candidates consume legal parcel/build-site capacity immediately when a construction site is
  created
- residential spawn need subtracts under-construction household slots from the missing-slot target,
  but admission and move-in execution may use only completed vacant homes
- commercial and industrial spawn need may subtract under-construction compatible output capacity
  from committed capacity, but live shortage, jobs, revenue, and logistics remain unavailable until
  completion
- non-residential absorption gates treat under-construction output as committed capacity to avoid
  oversupply from repeated hourly construction actions
- upgrades, downgrades, and despawns stay instant in the first implementation; a later
  redevelopment model must specify how existing households, workers, inventory, and budgets survive
  occupied-building construction
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

- incoming household pull from bootstrap entry and budget-backed open jobs
- housing capacity and vacancy as the execution cap for household admission and as satisfied
  capacity for residential spawn need
- resident presence (commercial/industrial demand gating)
- household affordability (commercial demand and soft household-admission damping)
- zero-budget household ratio (soft household-admission damping)
- household stock stability (commercial demand)
- industrial input-capacity deficit for active business and utility inputs (industrial spawn pressure)
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

Admission directly tests a bounded set of actual applicants against ready homes and recurring
income, as specified in the growth-redesign contract. One rejection does not consume capacity.
There is no household pressure threshold or admission credit.

Daily departures use the shared `HouseholdExitPolicy` after relocation and eviction. With the
shipped settings, a household must be unhoused for two days with less than one day of essential
cash reserves and at most 0.25 pantry days, or remain unhoused for seven days regardless of savings.
Housed households and newly displaced households are ineligible. Citywide homelessness, poverty
or treasury ratios do not change an individual household's eligibility.

```text
if eligible_households == 0:
    exit_credit = 0
else:
    exit_credit += eligible_households * persistent_exit_daily_fraction
    households_to_remove_today = min(floor(exit_credit), eligible_households,
                                    max_household_removals_per_day)
    exit_credit -= households_to_remove_today
```

The shipped fraction is 0.25/day and the execution cap is 48/day. These pace eligible departures;
they do not make additional households eligible. Execution rechecks the same policy, ranks only
eligible households by reserve days, pantry days and ID, and removes at most the planned count.
Recovery cancels a departure; unused slots never spill into housed or newly displaced households.
Format 79 removes the former citywide crisis-removal credit.

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
- selected projects immediately reserve customers, workers, homes and parcels within the pass;
  live placements affect the next hourly snapshot
- upgrade, downgrade and despawn still use carried action credits; spawning has no credit or
  percentage threshold
- there is no separate allocator-owned arbitrary cap on top of these demand-owned budgets

Hourly economy diagnostics emit one `building action diagnostics` line per use family. These lines
record the same decisions and constraints summarized by the R/C/I growth HUD:

- `spawn_candidates` is the frozen legal, site-feasible empty-site count for that use before
  economic gates. Each compatible initial asset passes the shared site-support solver before
  deterministic selection; one rejected variant cannot starve a lot that supports another.
- `housing_needed_slots` counts missing household places after ready and committed homes
- `spawn_plan` and `spawn_selected` count projects selected after customer, income and workforce checks
- `spawn_reject_absorption`, `spawn_reject_viability` and `spawn_reject_workforce` identify the
  actual economic constraint after viable alternatives on the same lot have been tried
- `spawn_not_needed` counts residential offers left unused after housing needs and viable
  workplace reservations are covered
- the matching `upgrade_*`, `downgrade_*`, and `despawn_*` fields expose the same candidate,
  normalized-pressure, budget, credit, planned, and selected counts for existing-building actions

Allocator execution emits a separate `building action execution` line when selected spawn actions
are submitted to final placement. This line is the first place to check when `spawn_selected` is
positive but no building appears:

Unchanged successful and rejected site solutions are cached; local road, terrain, neighboring-site
and asset changes trigger re-evaluation. This avoids repeating expensive grading checks every hour.
The existing parcel/asset discovery still runs in Rayon. Execution revalidates queued placements
against current geometry before insertion, so a later invalidating edit can still reject an action.

- `spawn_attempted` is the selected spawn count submitted to allocator placement
- `spawn_placed` is the count that committed a building
- `spawn_failed` is the total final placement rejection count
- `spawn_failed_geometry` counts geometry/world-surface rejections rather than stale asset or
  parcel references
- `fail_driveway_surface`, `fail_driveway_height`, `fail_driveway_connection`,
  `fail_frontage_surface`, and `fail_neighbor_height` identify the flat-site/road-surface reason
  for geometry failures
- `fail_asset`, `fail_parcel`, and `fail_slot` identify stale selected candidates whose authored
  asset, zoning parcel, or legal placement slot no longer exists by execution time

The hourly summary line uses `planned_spawns=(R/C/I)` for demand-selected spawn actions and
`placed_spawns=(R/C/I)` for the buildings actually committed by allocator placement.

Deterministic selection rule for building actions:

Each hourly pass freezes legal, site-feasible candidates. Residential selection counts the actual
household capacity of each candidate in deterministic attachment order:

```text
incoming_places = ceil(incoming_household_need)
reserve_places = ceil(total_household_count * 0.10)
missing_places = max(unhoused_household_count + incoming_places + reserve_places
                    - vacant_household_slots - committed_household_slots, 0)

for each eligible home while missing_places > 0:
    select home
    missing_places = max(missing_places - home's actual household capacity, 0)
```

Existing unhoused households need replacement places even without open jobs or an outside
connection. Ready vacancies and committed homes offset that need once, together with incoming
households and the vacancy reserve. Committed homes include construction already started and
reserved queued projects. There is no average building-size estimate, accumulated spawn credit,
pressure multiplier or hourly building quota. The last indivisible home can exceed the remaining
need. Remaining legal home offers stay
available to the workforce calendar: a viable business can select additional homes only if their
workers can be housed before its construction finishes. Construction jobs drive housing planning;
actual arrivals still claim ready homes and live, revenue-backed jobs.

The 2026-10-08 displacement regression reproduced missing housing targets of 1 instead of 11
and 0 instead of 25. Including the existing unhoused census fixes both without new scans or
allocations; the target remains O(1). The hourly planner selects two six-place homes for ten
displaced households with no incoming workers, with and without a border, then requests no more
places once those homes are queued. Fresh release verification passes 2,186 library tests and all
three growth integration tests, including both 30-day cash continuations. Only the pre-existing
utility-machinery quantity assertion fails. Year, two-year, mass-zoning and business-recovery
reports match the preceding build exactly. Logs: `/tmp/metrum-rehousing-before-tests.log` and
`/tmp/metrum-rehousing-tests.log`; reports: `/tmp/metrum-rehousing-{year,two-years,expansion,recovery}.txt`.
This verifies housing selection and committed capacity. Existing residents now also attempt
affordable rehousing every operational hour, after construction completion and the membership
census, before jobs and arrivals can claim ready homes. The existing admission index adds a
level/building ordering maintained with vacancy claims, releases and redevelopment changes;
unaffordable homes remain available. Future-home reservations now match displaced households
before crediting prospective arrivals. Homes are processed by completion time, then level and
stable site order; each home claims the first eligible household at its actual relocation
threshold. Thresholds need not increase with level. A matched household supplies only its
currently unemployed adults, without a starter-family size cap; unmatched places supply one
prospective adult each when arrivals are possible. Optional homes claimed by displaced residents
are materialized even when those residents supply no workers, keeping physical places distinct
from worker credits. Failed/bulldozed supporting homes remain subject to execution revalidation.

The household snapshot now sums housed households' unemployed adults instead of subtracting
citywide jobs from housed adults. The existing post-employment money-sync pass refreshes employed
counts directly from agent assignments using reused census counters; it does not follow member
links that intervening freight operations may have invalidated. This retains O(A + H) parallel
work and no per-agent allocation. Future matching reuses scratch and a maximum-reserve tree in
household order: O(F log F + U log U + F log T) preparation for F housing proposals, U displaced
households and T completion coordinates, followed by the existing calendar claims. With no
displaced households, committed homes retain the parallel hourly histogram path. Execution
updates placed homes incrementally in that common case; displacement triggers one lazy rematch
at the next business check after a home batch.

Fresh future-home verification (2026-10-08) passes **2,195 library tests and all three growth
integration tests**, including both 30-day save/reload continuations. The only failure remains
the pre-existing utility-machinery quantity assertion (`540` versus `96`). Six added regressions
cover physical factory rejection before owner funding when displaced nonworkers claim supporting
homes, affordability and nonmonotone thresholds, completion deadlines, large existing households,
optional-home materialization, disconnected towns, and current household employment accounting.
All four growth reports and the saved-town year match the preceding build byte-for-byte. The
saved town retains 80 residents, 66 employed and all 15 operating-cash-positive firms. Commands
use the release library/growth suite above and `diagnose_saved_city_growth --ignored --nocapture
--test-threads=1` with eight Rayon workers, `/tmp/metrum-consumer-budget-input.sqlite`, the user's
mods read-only, and `METRUM_DIAG_DAYS=365`. Logs:
`/tmp/metrum-future-homes-all-tests.log`, `/tmp/metrum-future-homes-saved-city-365-tests.log`;
reports: `/tmp/metrum-future-homes-{year,two-years,expansion,recovery,saved-city-365}.txt`.

The isolated release `benchmark_future_home_matching --ignored --nocapture --test-threads=1`
uses eight Rayon workers on CPUs `0,2,4,6,8,10,12,14`, three warmups and eleven samples. With 128
displaced nonworker households and 1,024/8,192/65,536 homes, matching takes
**0.0207/0.2751/2.4229 ms**; 512 subsequent finite worker claims take
**0.0212/0.0217/0.0207 ms**. Fixture construction and scratch restoration are outside timing.
This measures calendar matching and prepared claims, not successful physical-placement throughput.
Log: `/tmp/metrum-future-homes-scaling.log`; retained test executable and identity:
`/tmp/metrum-future-homes-tests`, `/tmp/metrum-future-homes-test-identity.txt`.

Matched unprofiled release timing uses the retained `/tmp/metrum-ready-rehousing-bench` baseline
and `/tmp/metrum-future-homes-bench`, unchanged configs, eight Rayon workers and the same CPUs.
The first sequential before/after pair is **2.6727/3.5226 ms hourly** (+31.8%, p<0.01) and
**192.58/189.94 ms daily** (p=0.98). Reversing execution order gives **2.6745/2.6641 ms hourly**
(p=0.87) and **198.02/187.90 ms daily** (p=0.93), expressed before/after. The initial hourly
increase does not reproduce. All four initial/final city records match, including treasury:
10,000 residents, 5,003 households, 6,791 employed, 13,881 jobs and 7,396 buildings.
Commands use `--bench /10k` with `--save-baseline`/`--baseline` under
`CRITERION_HOME=/tmp/metrum-future-homes-perf`. Drivers:
`/tmp/metrum-future-homes-perf.py`, `/tmp/metrum-future-homes-reverse-perf.py`; logs:
`/tmp/metrum-future-homes-{baseline,after}-perf.log`,
`/tmp/metrum-future-homes-reverse-{baseline,after}-perf.log`; build/config/source identities:
`/tmp/metrum-future-homes-identities.txt`. These are incremental checks, not acceptance of
finite local suppliers, reachable customers, successful placement throughput or the full redesign.

The housing-target change's incremental performance uses unchanged configs, eight Rayon workers and CPUs
`0,2,4,6,8,10,12,14`, running retained unprofiled release executables sequentially with
`--bench /10k`. Before/after hourly estimates in four balanced-order pairs are
2.4756/2.8907, 2.6529/2.5777, 2.5554/2.5858 and 2.8029/2.6522 ms. The first pair's 16.8%
increase does not reproduce in the three subsequent comparisons (p=0.26, 0.61, 0.15);
daily timing shows no significant difference in any pair. All eight runs have identical initial
and final city records: 10,000 residents, 5,003 households, 6,791 employed, 13,881 jobs and
7,396 buildings, including identical treasury. This measures the existing fully built 10k
fixture, not displacement recovery throughput. Build:
`cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
Executables: `/tmp/metrum-input-quotes-bench` and `/tmp/metrum-rehousing-bench`; hashes:
`/tmp/metrum-rehousing-identities.txt`; driver: `/tmp/metrum-rehousing-perf.py`.
Logs: `/tmp/metrum-rehousing-{baseline,after}-perf.log`,
`/tmp/metrum-rehousing-reverse-{baseline,after}-perf.log` and
`/tmp/metrum-rehousing-pair{3,4}-{baseline,after}-perf.log`.

Ready-home priority verification (2026-10-08) passes 2,189 library tests and all three growth
integration tests, including both 30-day reload continuations; the sole failure is the existing
utility-machinery quantity assertion. The new checks cover construction completing before
arrivals, individual affordability, unchanged daily clocks, 2,000 vacancy-index mutations and
release of an old broken home's occupancy. The latter also repairs the daily relocation path:
an unusable home must not retain a phantom household after its residents move elsewhere.
All four growth reports and the saved-town year match the preceding build exactly; the saved town
retains 80 residents, 66 employed residents and all 15 operating-cash-positive firms.
Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`; log:
`/tmp/metrum-ready-rehousing-verified-tests.log`; growth reports:
`/tmp/metrum-ready-rehousing-verified-{year,two-years,expansion,recovery}.txt`;
saved-town report: `/tmp/metrum-ready-rehousing-saved-city-365.txt`.

Its matched sequential release comparison uses the same eight workers, pinned CPUs and unchanged
configs as above. Hourly cost is 2.5857/2.5078 ms (p=0.22); daily cost is 189.14/191.73 ms
(p=0.98), with identical initial/final city records. No significant timing change is detected.
The isolated `benchmark_hourly_rehousing_priority --ignored --nocapture --test-threads=1`
holds 128 displaced households and 128 affordable homes fixed while unrelated, unaffordable
vacancies grow to 1,024/8,192/65,536. Median rehousing time is 0.023/0.021/0.022 ms; the existing
census is measured separately at 0.042/0.051/0.090 ms. Two warmups precede nine samples; fixture
construction and vacancy restoration are outside timing. This establishes the indexed vacancy
lookup's locality, not scaling with a large displaced population or future-home reservations.
Driver: `/tmp/metrum-ready-rehousing-perf.py`; logs:
`/tmp/metrum-ready-rehousing-{baseline,after}-perf.log` and
`/tmp/metrum-ready-rehousing-scaling.log`; executable/config identities:
`/tmp/metrum-ready-rehousing-identities.txt`, `/tmp/metrum-ready-rehousing-test-identity.txt`.

Businesses are considered independently by category and candidate profile, reserving their
customers and workforce immediately upon selection. Local recurring consumption and business
input needs provide customers; pantry recovery and one-time imports do not establish recurring
sales. Existing and committed output reserve the market by resource. Export opportunities are
independent of incumbents' recovery of startup capital, but every new operation must cover costs
and claim available labour. Rejected candidates expose viable asset alternatives on the same lot.
The minute queue paces physical starts and revalidates claims before owner funding.

Nonresidential level changes check the target operation's recurring income and working capital,
and reserve its extra ready workers and customers in the shared planning pass. The incumbent's own customers,
assigned staff and funded vacancies are credited once. Future housing cannot staff an instant
level change. A downgrade must also repair a current operation that cannot cover recurring costs
or working capital. A cash payment, empty stock or low staffing cannot downsize a supportable firm.
Rejected proposals keep their pacing credit, and the planner continues to later candidates.

Upgrade and downgrade pacing use eligible candidate counts, use-family batch fractions, carried
credit and `cadence_fraction = 1/24`, independently of citywide pressure. A zero budget or zero
eligible candidates clears that action's credit. Economic or reservation rejection refunds unused
slots. Downgrades run after growth claims, with business replacements before residential downsizing.
Homes promised to workers, the normal vacancy reserve, unhoused households and the first incoming
household remain protected. Multiple downsizes consume the same surplus; future homes cannot
replace ready capacity that is being removed now.

For upgrade:

```text
upgrade_action_credit[use] +=
    eligible_upgrade_count[use]
  * upgrade_batch_fraction_by_use[use]
  * cadence_fraction

upgrades_this_pass[use] =
    min(eligible_upgrade_count[use], floor(upgrade_action_credit[use]))

upgrade_action_credit[use] -= upgrades_this_pass[use]
```

For downgrade:

```text
downgrade_action_credit[use] +=
    eligible_downgrade_count[use]
  * downgrade_batch_fraction_by_use[use]
  * cadence_fraction

downgrades_this_pass[use] =
    min(eligible_downgrade_count[use], floor(downgrade_action_credit[use]))

downgrade_action_credit[use] -= downgrades_this_pass[use]
```

For failed-site clearance:

```text
eligible_failed_sites = deserted private buildings with no occupants or workers,
                        excluding broken, economy-broken, constructing and rezoning sites

despawn_action_credit[use] +=
    count(eligible_failed_sites) * despawn_batch_fraction_by_use[use] * cadence_fraction

despawns_this_pass[use] = min(count(eligible_failed_sites), floor(despawn_action_credit[use]))
despawn_action_credit[use] -= despawns_this_pass[use]
```

Healthy vacant homes and unstaffed workplaces are retained. Clearance has no citywide pressure
threshold or hysteresis. At the shipped `0.5` daily fraction, one eligible failed site clears in
approximately two days, regardless of market pressure. The executor rechecks desertion, zero
occupants/workers and construction/redevelopment state before removing it; a stale plan cannot
clear recovered or newly occupied capacity. Replacement follows ordinary site and project checks.

Authoring rule:

- `upgrade_batch_fraction_by_use = action_budget.upgrade_batch_fraction_by_use`
- `downgrade_batch_fraction_by_use = action_budget.downgrade_batch_fraction_by_use`
- `despawn_batch_fraction_by_use = action_budget.despawn_batch_fraction_by_use`
- all of those tables are authored in [`demand/growth_profiles.toml`](../demand/growth_profiles.toml)
- spawn has no authored batch fraction in baseline `v0.1`; its action count is derived from the
  deterministic missing-building need above

Deterministic execution order:

- hourly demand telemetry refreshes `ResidentialGrowth`, `CommercialGrowth`, and
  `IndustrialGrowth` from the latest operational-hour economy state
- hourly household admission advances `admission_action_credit` at `1/24` of the authored daily
  household cap, then consumes `households_to_admit_today` immediately by launching household
  arrival carriers for available reserved homes
- hourly private spawn actions advance their carried action credits by demand-derived missing
  building units; upgrade, downgrade, and despawn actions advance their carried action credits at
  `1/24` of their authored daily building-action budget, then execute the selected private building
  plan immediately
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
   - `commercial_capacity_deficit`
   - `industrial_input_capacity_deficit`
   - `business_owa_dependency`
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

For `v0.1`, fresh-city bootstrapping and later household growth are demand-owned through explicit
city-level signals. The pioneer demand floor has been removed; household transfers in
[`economy.md`](economy.md) keep early-city households solvent through real economic activity, while
authored regional migration pressure provides continuing household attraction after the first
settlement.

Rules:

- empty-city bootstrap may admit the first household when external connection, move-in viability,
  and vacant execution capacity exist
- after bootstrap, budget-backed open jobs, forecast-only marginal commercial worker-equivalents,
  and authored regional migration pressure define incoming household pull; vacant homes are
  execution capacity only
- job-driven admission uses the same future household id as materialization and prefers an
  adult-capable claimable household over a workerless front candidate; if no adult-capable
  candidate exists, deterministic vacancy order remains the fallback
- immigration pressure is driven by coarse city signals only, not by hidden magic or static floors
- no hard job requirement for the first settlement: people may move to a new city before jobs exist
  when starter savings and reliable transfer income provide the authored search runway

Two distinct pressure formulas drive the residential system. They share the same economic
signals but serve different purposes.

**`admission_pressure`** — measures incoming household pull. Used only for the household-action
`households_to_admit_today` output. High when bootstrap/open-job pull exists and the existing
household economy plus candidate move-in terms are healthy enough to absorb new arrivals:

```text
admission_pressure =
    clamp(
        external_connection_available
        * incoming_household_pressure
        * move_in_acceptance
        * admission_failure_factor,
        0.0,
        1.0
    )
```

`move_in_acceptance` is deterministic candidate-side viability. It estimates the household that
would enter the city, then asks whether starter savings plus expected wage income and
treasury-backed transfers provide at least the authored search runway. The candidate is not an
averaged household-size proxy: demand previews the exact child/adult/elder composition that would
be materialized for the next vacant home, or the exact first compatible construction fallback when
no vacant home exists yet. Jobs count only paid, budget-backed commercial or industrial open slots,
and only candidate adults can fill them. Transfer support is reliable only to the extent that the
current treasury can cover the existing unemployed-adult, elder, and child support claim plus the
candidate's exact unemployment, pension, and child-support claim for
`move_in_benefit_treasury_coverage_days`.

Consequences:

- a single elder candidate may be viable from pension income without inventing an unemployed adult
  worker
- a child-heavy household receives child-support support but still contributes no worker capacity
  from those children
- adult-only or mixed adult households use exact adult count for both job pull and unemployment
  claim

**`inflow_desire`** — drives new residential building capacity. Used only in `ResidentialGrowth`.
High when incoming household pull exists and the city is welcoming:

```text
inflow_desire =
    clamp(
        external_connection_available
        * incoming_household_pressure
        * residential_construction_viability,
        0.0,
        1.0
    )
```

Relationship: `admission_pressure` and `inflow_desire` are complementary by design.
Incoming household pull raises both admission pressure and residential construction pressure.
Vacant household slots cap how many households can actually arrive on an hourly pass and subtract
from residential spawn need, but they do not lower the incoming pull itself. If the city already has
broke or unhoused households, failure damping still reduces both admission and construction
pressure. If no vacant household slots exist yet, the move-in estimate derives its candidate
household size from registered level-1 residential assets, so a healthy empty city can still build
first homes without requiring an already-vacant home.

`recent_household_failure_pressure` is demand-owned carried state in `0.0..1.0`. Each daily
demand pass first decays the previous value by `recent_failure_daily_decay`, then raises it to at
least that settled day's `failure_pressure`. After actual demand-owned household removal executes,
the same memory is raised to at least `removed_households / total_household_count` for the
pre-removal settled snapshot. The value is persisted with `demand_state`, because it affects
admission behavior after save/load.

**Household departures** follow the individual eligibility and pacing rules in [Outputs](#outputs).
The former homelessness-ratio threshold and crisis credit have been removed. The settled snapshot
counts eligible households using the same policy execution uses to recheck their current state.
Diagnostics report raw household counts, eligible count, pacing credit and planned/actual exits.
The remaining residential level-change pressure still reads the homelessness ratio; it does not
control arrivals, departures, new construction or deserted-site clearance.

Interpretation:

- ordinary low money does not directly remove housed households from the city in baseline `v0.1`
- household affordability failure first flows through the economy-owned relocation, eviction, and
  `unhoused` rules described in [`economy.md`](economy.md)
- once the settled snapshot contains persistent `unhoused` households, demand may convert that
  explicit state into whole-household city removal
- baseline `v0.1` household outflow therefore comes from sustained failed living conditions, not
  from one bad hourly dip or one direct poverty-to-deletion shortcut

## Deterministic `v0.1` Rules

Floating-point city activity and demand totals use fixed input chunks, parallel work within the
collection, and chunk results merged in input order. Changing the Rayon worker count must not
change the resulting totals. The shared economy reduction costs O(N) work and O(N / chunk size)
temporary accumulators; it does not allocate per resident or sort chunk results.

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
- household size and child/adult/elder composition are selected from the shared deterministic
  starter mix and capped by the claimed home's `flat_size_m2`; pressure telemetry previews the
  same next claimable candidate that execution will launch
- economy creates the actual household record and resident agents when the carrier reaches the
  claimed home
- if no required external connection or no legal border-to-home car route exists, admission waits
  instead of silently instantiating the household inside the home

The `economy` debug log emits a compact household-admission diagnostic line for each hourly pass.
It reports the raw admission pressure, base incoming-household pressure, vacancy telemetry,
incoming household need, open-job household pull, household affordability telemetry, candidate
move-in acceptance, search-runway days, candidate household size, candidate adult-worker count,
budget-backed open jobs, expected employed and unemployed adult workers, expected wage income,
candidate child/adult/elder composition, transfer reliability and unemployment/pension/child-support
claims, starter savings, daily essential cost, daily deficit, construction-side move-in acceptance,
construction runway, residential construction viability, individual household-failure damping
factors, the recent-failure memory and factor, threshold, normalized action pressure, carried
admission credit, planned households, and actually launched arrival carriers.

The daily `city flow diagnostics` line summarizes the settled city state after daily household
settlement, removal execution, and the midnight demand pass. It reports:

- `net_households`, `admitted_since_daily`, and `removed_today` for the household flow since the
  previous daily diagnostic
- housed, unhoused, zero-budget, stock-empty, and low-stock household counts
- resident agents, child/adult/elder resident counts, pending household carriers, employed adult
  residents, unemployed adult residents, theoretical commercial or industrial job capacity,
  budget-backed active job capacity, and active open jobs
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

Admission claims ready housing and live funded income. Private development claims customers,
workers and housing that will be ready by workplace completion. Upgrades and business downsizes
share those reservations; instant changes cannot use future housing. Zoning establishes which
assets are legal, while the target operation establishes whether the site can sustain them.

A family can offer both upgrade and downgrade candidates. A selected upgrade excludes the same
building from downsizing in that pass; a rejected downgrade cannot hide a viable upgrade.
Residential downsizing retains every occupant and all housing promised to workers. Household
poverty remains a relocation/eviction concern, not a direct whole-building downgrade trigger.

Crime, noise, parks and similar neighborhood conditions are later extensions, not baseline
percentage vetoes on viable development. Citywide RCI pressure remains diagnostic telemetry only.

### Deterministic building spawn rule

A private building spawns only when all owning layers agree that the spawn is allowed.

Deterministic `v0.1` spawn rule:

1. Demand freezes legal site candidates in deterministic allocator attachment order.
2. Homes cover the actual missing household capacity. Additional homes can be selected for
   viable workplaces whose workers can be housed by construction completion.
3. Businesses claim customers, income and workforce independently. The minute queue paces starts;
   there is no spawn-pressure threshold, hysteresis, accumulated credit or `1/24` spawn quota.
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

Commercial and industrial buildings spawning into a neighbourhood with no consumers to absorb their
output is the root cause of zombie businesses and oversupply. Baseline `v0.1` therefore applies a
deterministic output-absorption gate before a non-residential spawn is allowed to execute.

This gate applies **after** the budget (`spawns_this_pass[use]`) has been computed and **during**
final candidate selection. Candidates that fail it are removed from the selected set; the remaining
budget is not redistributed to other candidates on the same hourly pass.

#### Workforce Is A Pull Signal

Workplace capacity must not be a hard spawn prerequisite. A commercial or industrial building may
spawn with fewer current residents than its full `worker_capacity` when demand and output
absorption justify the building. The new building's budget-backed open jobs then feed
`incoming_household_need` on the next demand snapshot after existing unemployed adults are counted
against those jobs, which raises household admission and residential construction pressure only for
the remaining workforce shortfall.

Deterministic rule:

```text
required_workers = worker_capacity of the candidate's bound economy profile

non_residential_spawn_preexisting_worker_requirement = 0
```

Where:

- `required_workers` still matters for job capacity, wages, startup budget, staffing factor,
  production throughput, and household admission pull
- `required_workers` does **not** reject the spawn candidate during final selection

Interpretation:

- a starter city with one 5-person household may place a 15-worker grocery if household demand is
  unmet and the output-absorption gate passes
- that grocery's open jobs become deterministic incoming-household pull instead of creating a
  chicken-and-egg deadlock
- business viability is still handled by operational systems: partial staffing lowers throughput,
  wages require operating budget, and persistent failure can later make the building eligible for
  downgrade/despawn

#### Output-Absorption Gate

A non-residential spawn candidate is rejected if the city's consumer base cannot absorb its
output.

Deterministic rule:

```text
output_capacity_already_placed =
    for each output resource in the candidate's bound economy profile:
        sum that resource's net daily output, with explicit work-area scaling,
        from existing non-broken buildings whose profile outputs the same resource

total_consumer_demand =
    for each output resource in the candidate's bound economy profile:
        resident demand for that resource
        + existing business and utility input need for that resource

output_absorption_gate_passes =
    any candidate resource with positive net output and:
        output_capacity_already_placed_for_resource < total_consumer_demand_for_resource

If the candidate asset has no resolvable economy profile binding, the gate fails safe and rejects
the spawn candidate. Existing buildings with unresolved runtime profiles contribute no placed
capacity.
```

Where:

- resident demand uses the authored per-resident basket capped by each housed household's
  recurring disposable income, including VAT and funded permanent transfers
- business input need includes market-scaled commercial ports and staffed industry, farm, mine and
  utility ports, with area scaling and output headroom. Input shortages do not suppress demand;
  unstaffed producers and full output buffers do. A candidate cannot create its own customers
- `output_capacity_already_placed` sums full net saleable output with explicit work-area scaling;
  current staffing, stock and demand-responsive activity do not release customer reservations.
  Self-produced maintenance is excluded from both saleable output and external input demand;
  pending construction and selected candidates reserve net output. Broken, economy-broken and
  deserted buildings are excluded
- if the candidate profile has no declared outputs, or none of its output resources have resident
  demand or business input need, the gate fails safe
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
```

The `satisfaction_ratio` for each resource is authored in `RuntimeEconomyTuning` (see
[`economy.md § Commercial Tiers`](economy.md#commercial-tiers-and-multi-resource-household-consumption)).
For all current primary resources (`household_supplies`, `packaged_food`, etc.) the ratio is
implicitly `1.0` and no change in gate behavior occurs.

#### Gate Interaction

The output-absorption gate is evaluated for each non-residential candidate. A candidate that fails
is excluded from that day's selected spawn set.

This gate does not affect residential spawns, upgrades, downgrades, or despawns.

Residential note:

- `households_to_admit_today` and residential building spawn are related but not identical outputs
- household admission decides whether new households enter the city
- residential building spawn decides whether new housing capacity appears on the map
- both should respond to residential demand, but each still uses its own owning-layer contract

## Building Desertion

A Commercial or Industrial building enters the **deserted** state when it has been
economically dead for long enough that no self-rescue is possible. Deserted buildings are
inert in all simulation systems, visually distinct, and remain on the map occupying their
land until the player demolishes them or the paced failed-site clearance removes them.

Residential buildings are excluded. Household emigration handles the equivalent residential
lifecycle event.

### Desertion Trigger

Desertion is evaluated once per daily settlement for every non-broken, non-economy-broken
commercial, industrial, or utility building that participates in budget distress. Residential
buildings are handled by household relocation and removal instead.

Deterministic rule:

```text
Step 1, before today's wages and utility costs:
    if budget_distress == true AND operating_budget < 0:
        is_deserted = true

Step 4, after today's utility cost and forced OWA liquidation:
    if operating_budget < 0:
        budget_distress = true
    else:
        budget_distress = false
```

Where:

- `budget_distress` is a persisted one-day memory bit set when the previous daily settlement ended
  negative after forced OWA liquidation tried to sell unreserved output inventory
- a building gets one full distress day before desertion: the first negative day sets
  `budget_distress`, and only the next daily Step 1 can set `is_deserted`
- if liquidation or later revenue brings `operating_budget` back to zero or positive before the
  next Step 1, the building survives and Step 4 clears `budget_distress`
- under-construction, broken, economy-broken, and already-deserted buildings are skipped by this
  sequence

Interpretation:

- a newly placed building receives startup operating budget once at spawn/construction creation;
  there is no daily refill or repeated rescue mechanism
- negative budget is not instantly fatal; forced OWA liquidation can rescue a building by selling
  unreserved inventory before the next bankruptcy check
- two consecutive daily observations are required: the building must end one day negative and still
  be negative at the start of the next daily settlement
- `is_deserted` is a one-way latch; the business cannot resume operation in place, but bounded
  site clearance allows a subsequent new business

### Insolvent Employer Filter (Job Search)

Employment and household admission now read the same prepared live-job capacities from
`economy/hiring.rs`. A private operation needs forecast recurring sales covering payroll,
purchased inputs, utilities, property and freight, plus working capital for replenishment and
settlement. Cash, stock and unsettled freight assets supply capital; owner grants do not count
as revenue. The largest supportable integer workforce supplies the hiring ceiling. Construction,
broken/deserted state and missing site access exclude an employer before that calculation.
Municipal services use their explicit funding limit.

This replaces both the proposed `revenue == 0 && cash < one wage` filter and the separate
cash-per-wage cap. Existing employment priority and unpaid-wage handling remain owned by the
household system. Actual payroll still pays from cash; the forecast assesses the normal interval
between replenishment, delivered sales and settlement. See the
[current implementation and market limitations](#implementation-contract-and-progress).

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

No utility charge is applied, and deserted utility providers do not receive distributed local
utility revenue.

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
    sum matching output-resource base_rate_units_per_day for all existing buildings where:
        NOT broken
        AND NOT economy_broken
        AND NOT is_deserted                 // ← added exclusion
        AND output resource overlaps one candidate output resource
```

A deserted building is removed from `placed_capacity`. This directly unblocks the spawn of a
replacement building when the only existing producer for a given resource has become deserted.

**Despawn candidate priority:**

```text
despawn_candidate_eligible(building) =
    true    if is_deserted AND worker_count == 0 AND occupancy == 0
    false   otherwise (also reject broken, constructing or rezoning sites)

despawn_candidate_order:
    eligible deserted sites sorted by deterministic attachment order within each zone family
```

Only deserted buildings enter demand-driven clearance. Current market pressure neither strands
failed sites nor turns a healthy empty building into a demolition candidate.

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

A deserted business does not resume operation in place: `is_deserted` is a one-way latch.
The demand system can clear its site and subsequently place a new business.

A deserted building:

- occupies its parcel claim, blocking new spawns at that location
- is ineligible for upgrade or downgrade consideration
- is not counted in building-level demand snapshot signals (e.g. `business_owa_dependency`)
- is removed when the player demolishes it, or when the demand system's bounded despawn budget
  selects it (deserted buildings are always first in the despawn queue). Empty commercial and
  industrial sites with no workers are eligible even at positive demand; deserted residential
  buildings still require negative demand.

### Data Model

| Field | Type | Default | Notes |
|---|---|---|---|
| `is_deserted: bool` | bool | false | One-way latch. Set when the two-day `budget_distress` bankruptcy check fails. |
| `budget_distress: bool` | bool | false | Persisted one-day memory that the previous daily settlement ended with negative operating budget after forced liquidation. |

Both fields are persisted in the current save schema. Old save compatibility is not required during
the cleanup phase, so schema changes should be direct version bumps rather than compatibility
shims.

All construction sites that build a `Building` literal must initialise both fields:
`is_deserted: false, budget_distress: false`.

Current durable construction path: `allocator/placement.rs`. Tests and internal helpers that build
`Building` literals must initialise the same fields explicitly.

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
- shipped fresh-map startup and continuing household growth use demand-owned bootstrap and
  regional-migration signals; any future special founding-placement scenario rule must stay outside
  allocator maintenance
- economy creates the admitted household records
- building systems execute legal placement, removal, and level changes once demand has already decided the pressure outcome
- housing/vacancy logic claims real homes
- transport either visualizes the move or does nothing, but it does not decide growth

## Commercial need cleanup measurements (`AUDIT-01-D5`)

The snapshot keeps one resource-specific committed-shortage vector. The obsolete aggregate scalar
and fallback averaging function are removed; tests now populate the same representation as live
snapshots. Staffing remains an operational outcome after spawning, and the retired always-zero
labour rejection counter is removed from diagnostics. Selected spawn count comes from the result
vector instead of a second counter. Candidate ordering, resource averaging and immediate absorption
reservations are unchanged. Empty shortage vectors still return in O(1); nonempty work keeps the
existing candidate/output/catalog-resource bound, independent of population outside snapshot input.

Five alternating unprofiled release pairs ran the ignored `benchmark_commercial_spawn_need` on
CPU 0 with one Rayon worker, three warmups and 21 batches of 32 queries. Fixtures use a 200-unit/day
grocery profile and either no shortage or 401 missing units; every result remains exactly 0 or 3.
Candidate and snapshot setup are excluded. Median milliseconds per nonempty query:

| Eligible candidates | Before | After |
| --- | ---: | ---: |
| 16 | 0.000511 | 0.000505 |
| 256 | 0.007023 | 0.007028 |
| 4,096 | 0.109900 | 0.108741 |

Empty queries measured roughly 1.9–2.2 ns, below useful resolution for a speedup claim. Three
alternating eight-core pairs also ran the existing `benchmark_business_demand_snapshot` fixture
on CPUs 0,2,4,6,8,10,12,14: 1,024/8,192/65,536 factories measured 0.030/0.046/0.294 ms before and
0.028/0.044/0.276 ms after. This industrial fixture checks general snapshot cost; its timing loop
has no output checksum and does not exercise a commercial shortage. Correctness comes from the
separate full release suite and resource-by-resource determinism regression, not those timings.
The cleanup does not establish a general performance improvement.

Builds used `cargo test --offline --manifest-path rust/Cargo.toml --release --lib` and Rust
1.98.1 (48a229cea 2026-09-01). Runs use `METRUM_DEBUG=0`, `--exact`, `--ignored`, `--nocapture` and
`--test-threads=1`; `match_demand_legacy.py` records exact commands, affinity and worker counts.
Before/after test executable SHA-256 values are
`163c0d9acf10345cbaf03e5b5a9ef20640db91715885a808a26090848230e940` and
`ce6bfc820b9e28188c6cb67d3473b4139af3ec99ed4ac3619c15e124b28bc948`.
Sources and binaries remained fixed during timings, with no competing builds/tests. Source hashes,
raw logs and matched summaries are under `/tmp/metrum-full-audit/demand-legacy-*`.

## Remaining Follow-Up Limitations

### Growth redesign (`DEM-02`)

**In progress, revised 2026-10-07.** Replace stacked pressure, spawn-need threshold and admission
penalties with one explainable signal per zone, derived from explicit economic quantities:

- residential: households that can support themselves and need homes, with existing job seekers
  considered before incoming workers;
- commercial: affordable spending and unmet goods/service capacity by category;
- industrial: local input requirements and viable regional sales opportunities by resource.

Keep resource-specific accounting and existing/pending/selected capacity reservations beneath
the display signal. Make land, assets, access, vacant homes, construction and execution cadence
named constraints. Do not repeatedly discount the same distress through separate pressure layers.
Regional customers must enter both industrial demand and the absorption/spawn calculation.

**Saved-town diagnosis, 2026-10-07.** The user's `economy.sqlite` (format 73, day 3 at 05:41)
contains seven residents, all employed: two farm workers, four machinery workers and one grocer.
There are three vacant homes and ten industrial placement candidates, but industrial pressure
is 0.827 while household admission is zero. A read-only snapshot replay with the user's Kenney
and Kuopio packs stays at seven residents through day 32 and reaches twelve on day 33, following
a second machinery factory. No cash, land, demand or staffing was changed for the replay.

The existing `unsettled_industry` reduction prevents export expansion until **every** zoned
factory has paid revenue and cash above its original startup grant. In this town the machinery
startup threshold is 6,920, versus 1,853.81 cash at the saved point. This is an artificial
dependency between independent companies, introduced to restrain repeated startup capital; it
must be removed in the redesign. Imported steel/metals still produce industrial pressure even
though no matching local producer is available, and the alternate export opportunity is blocked.
No additional recurring jobs means prospective households fail affordability despite empty homes.
The diagnostic's nominal job count (21 initially) includes slots that current sales cannot fund;
the effective staffed capacity is seven.

The replacement should use one opportunity calculation for decisions and their explanation:

1. Evaluate each proposed business on its own recurring local/export sales, recipe inputs,
   profile-based electricity, other utilities, wages, taxes and freight. A rival's cash balance
   is not an opening condition. Reuse economy formulas and cached transport information.
2. Reserve shared local customers, labor and housing for existing, constructing and selected
   projects. Export sales have no artificial shared buyer quota; expansion is paced by explicit
   project execution capacity and finance, not competitors recovering their startup grants.
3. Admit households against available housing and their own recurring affordable income,
   after reserving work for existing residents and already approved arrivals. Derive residential
   construction from the resulting shortage of homes.
4. Publish the same feasible opportunities and named blockers in the demand UI. Missing producer
   assets, insufficient margin, access, labor, housing and construction timing must be distinct.
   Remove the extra normalized pressure/threshold/penalty layers that can veto a valid opportunity.

Startup money remains separately reported as owner equity. Failure and replacement capital must
stay visible in diagnostics; they must not be hidden by growth or used to grant rival permission.
Before replacing execution, add regressions where a cash-poor rival cannot block a viable entrant,
two projects cannot claim the same local buyers/workers, and an unprofitable entrant is rejected.
Replay this save alongside the existing long-run growth cases, then measure only the 10k tier.

Reproduction uses the opt-in `diagnose_saved_city_growth` test in `scenario/balance_cases.rs`:

```bash
METRUM_DIAG_SAVE=/tmp/metrum-demand-economy-input.sqlite \
METRUM_DIAG_MODS="$HOME/.local/share/godot/app_userdata/Metrum Rise/mods" \
METRUM_DIAG_REPORT=/tmp/metrum-demand-economy-30-days.txt METRUM_DIAG_DAYS=30 \
RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib \
  diagnose_saved_city_growth -- --ignored --nocapture --test-threads=1
```

The input is a SQLite backup copied read-only from the user's save; the test never saves its
simulated state. Runtime: `6f159238` plus the diagnostic runner, eight Rayon workers. Snapshot
SHA-256: `e17c42e823caf561f0a333f22f0fa2175ffdadbfa0678bc4e2cce673764f2948`.
Log: `/tmp/metrum-demand-economy-replay-final.log`. This is a diagnostic continuation,
not full accounting acceptance: the initial partial-day observer lacks some earlier flows.
No live demand behavior has been changed by this investigation.

The current `regional_growth_soft_households = 600` eliminates only regional pull, not job-driven
immigration. Removing that arbitrary cutoff cannot repair early business losses on its own.
`ECON-14` owns household budgets, viable startup, additional activities and regional trade;
`DEM-02` consumes those economic inputs. Sector choices and numerical formulas remain design
work. Both share healthy-growth scenario acceptance and matched 10k performance checks; the 1M
run is deferred. The [plan and acceptance](economy.md#growth-redesign-econ-11econ-14-dem-02)
are owned by `economy.md`.

#### Critical demand review (2026-10-07)

This records the pre-implementation source review under `DEM-02`.
The existing saved-town diagnosis above is reproduced by a fresh run below.

- **Independent industry is coupled citywide.** `snapshot.rs::absorb_building` ORs every
  industrial site's unfinished, failed, unstaffed or unrecovered-capital state into
  `unsettled_industry`. One such site closes export-led entry for every other proposal, even
  one producing a different resource. Local demand can still permit entry. Remove this shared
  veto when candidate viability and finite project/labor/housing reservations replace it;
  simply deleting it would expose repeated owner-equity openings without that pacing.
- **Commercial shortage mixes incompatible quantities and suppresses whole categories.**
  `snapshot.rs::from_runtime_with_catalog` adds kg of household supplies to service equivalents
  before dividing unmet by total demand. With full pantries, full purchase power and all other
  categories supplied, missing personal services produce `0.10 / 1.26 = 0.079365` pressure;
  missing health essentials produce `0.16 / 1.26 = 0.126984`. Neither reaches an inactive
  medium-density threshold of `0.15` or high-density threshold of `0.20`, regardless of city
  population. These are calculations from shipped TOML and the source formula, not scenario
  measurements. Decide entry per category using recurring sales and costs; changing the unit
  used to describe groceries must not decide whether a barber can open.
- **Household distress is counted repeatedly.** Regional pull already includes affordability,
  pantry stability and the maximum of homelessness/cashlessness. `pressure.rs` then multiplies
  incoming pull by separate homelessness, cashlessness and recent-failure penalties. The same
  product controls residential construction, from which homelessness is subtracted again.
  Positive need still has to clear density thresholds and accumulate credits. This can suppress
  homes needed for recovery even when an individual household can afford to live in the city.
  Preserve candidate income checks and existing-resident priority; remove citywide repeated
  discounts as separate vetoes on a feasible proposal.
- **The search-runway calculation no longer provides a meaningful gradual signal.**
  `pressure.rs::compute_move_in_acceptance` rejects recurring income below essential costs.
  When recurring income covers those costs, expected income (which additionally includes
  unemployment support) also covers them, so the runway calculation returns its target and
  acceptance becomes one, apart from numerical tolerances. The authored two/six-day runway
  knobs obscure an effectively binary affordability check. Consolidate this forecast with the
  actual per-family recurring-income claim in `admission.rs`; keep claims that prevent multiple
  arrivals spending the same wages or transfer capacity.
- **One preview applicant can veto the whole hour.** `candidate_household_preview` chooses one
  household/home pair. Failed affordability sets admission pressure to zero and
  `advance_household_action_credit` clears accumulated credit before any actual family claim.
  Execution also breaks on a rejected income claim in `allocator/lifecycle.rs`. Selection
  already prefers small worker-capable households when there are net open jobs, so an arbitrary
  large family ahead of an available single worker is not the general selection rule; nevertheless
  selection does not search for affordability. The existing rejected-applicant test proves a
  later *hour* can pass, not that other affordable applicants are considered in the same hour.
  Replace the preview veto with bounded deterministic candidate evaluation and shared claims.
- **Advertised work reflects immediate liquidity, not recurring earning capacity.** Private
  open slots in `snapshot.rs::absorb_building` are capped by
  `floor(operating_budget / average_daily_wage)` before subtracting filled workers. Bulk purchases
  or payroll can therefore hide vacancies at a profitable firm. Actual hiring uses the same cap
  in `households/employment.rs::effective_job_capacity`: repair the common forecast/working-capital
  contract rather than letting demand promise jobs that employment will refuse. Keep the real
  obligation to fund payroll; profitability alone does not supply cash for the next payment.
- **Committed workplaces are absent from the housing forecast.** Under-construction buildings
  reserve their output capacity, then return before job accounting. Their future workforce
  creates no household pull. When spare housing and other housing pull are absent, factories
  finish before their associated homes can start, serializing two construction stages. This is
  conditional, not an extra cycle for every expansion. Count funded committed jobs when planning
  homes, while actual move-ins still require ready housing and supportable income.
- **Regional pull is not an independent economic base.** It cannot admit a working household
  whose wages and permanent transfers fail the recurring-income check. It is redundant for
  admission when job pull already saturates the clamped signal, but can affect sub-saturation
  pacing, pension-supported households and un-clamped housing-need arithmetic. Removing the
  600-household cutoff alone does not repair an unaffordable applicant or the saved-town job gap.
- **Pacing is mixed with economic need.** Incoming pull is clamped to one before admission
  credit accrues, whose maximum sustained rate is 48 households/day at every city size.
  Regional pull separately vanishes at 600 households (job-driven immigration does not).
  Building planning multiplies missing capacity by averaged density-normalized pressure before
  checking the selected business's market viability. Replace these layers with explicit,
  deterministic execution capacity separate from the count of viable opportunities. Rejected
  proposals should not consume the opportunity budget of unrelated viable candidates.
- **The bars do not report the executable decision.** `pressure.rs::net_pressure_for` applies
  low-density thresholds and hysteresis to aggregate channels, without candidate profitability,
  available producer assets, construction commitments or placement results. Positive industrial
  pressure can therefore describe imports for which no local asset exists. Preserve unmet need
  in diagnostics, but label missing assets/access/capital/labor/homes and construction timing
  alongside the feasible growth count instead of presenting a percentage as an explanation.

Implementation acceptance should add category-isolation and unit-invariance checks, a viable
entrant beside a cash-poor rival, recovery housing with distressed existing households, and a
no-double-claim check across existing, constructing, selected and arriving capacity. Make the
founding contract explicit: the current startup selection fixtures provide only residential and
commercial zoning, while the passing growth fixture also provides export industry. A town with
no recurring economic base needs an explicit founding source of income; a pressure bonus alone
cannot make its households sustainable. Test growth after additional zoning and employment
opportunities, not just population retention in one starter layout.

Reuse the existing deterministic snapshot reductions and allocator indices. Evaluate economic
quantities per profile/resource and bounded site candidate; do not add a city scan or route search
per proposal. Runtime changes require matched 10k performance acceptance; this review adds no
runtime code and ran no benchmarks.

Fresh verification at `6f159238` plus the existing saved-city diagnostic working-tree changes,
with eight Rayon workers and one test thread:

- `cargo test --manifest-path rust/Cargo.toml --release --lib demand`: **93 passed, 2 failed,
  3 ignored**. Both failures are the previously tracked residential family/variant startup
  fixtures, at `pioneer demand should place a residential building`, before selection assertions.
  Log: `/tmp/metrum-demand-audit-tests.log`.
- `cargo test --manifest-path rust/Cargo.toml --release --test growth_scenario`: **3 passed**.
  The two-year fixture grows from 102 residents / 76 employed at day 366 to 155 / 115 at day 731.
  Log: `/tmp/metrum-demand-audit-growth.log`; daily reports:
  `/tmp/metrum-demand-audit-year.txt` and `/tmp/metrum-demand-audit-two-years.txt`.
- The saved-city command above, with report `/tmp/metrum-demand-audit-saved-city.txt`, freshly
  reproduces seven residents through day 32 and twelve at day 33. Log:
  `/tmp/metrum-demand-audit-saved-city.log`. This diagnostic pass establishes reproducibility,
  not healthy-growth acceptance. No source save was written.
- Follow-up source review of preview admission, liquidity, construction and regional pull:
  `cargo test --manifest-path rust/Cargo.toml --release --lib simulation::economy::demand::`
  with the same worker settings freshly passes **74 tests, 2 ignored**. Log:
  `/tmp/metrum-demand-audit-followup-tests.log`. Existing coverage includes later-hour applicant
  recovery, recurring-income claims and pension acceptance; it does not establish same-hour
  candidate fallback or coordinated workplace/housing construction.

#### Implementation contract and progress

The full growth-loop replacement is in progress; passing a partial admission test does not
establish completion. User clarification adds these required invariants:

- A business must be independently viable **and** have a workforce available from existing
  unemployed adults plus incoming worker households with homes ready by its construction
  completion. Reserve that labor and the required homes per selected project, alongside shared
  local customers. Reserve existing and constructing projects first. Do not count the same
  unemployed adult or future home against multiple industrial lots.
- Under-construction jobs drive home construction, never admission income. Admission uses only
  live funded jobs after existing residents and pending arrivals have their claims. Advertised
  jobs must have recurring sales supporting payroll; the initial owner grant is not revenue.
  Demand and employment need the same funding calculation, including liquidity for normal
  payroll and replenishment timing.
- Expansion acceptance must paint many industrial lots at once while labor is scarce and inspect
  actual staffing, loss-making firms and total owner money spent. Zero bankruptcies alone cannot
  pass. Retain empty-town, saved-town, different-business-mix, recovery and long-run affordability
  cases, plus matched 10k performance checks and deterministic continuation.
- Cash comparisons use cent-level precision: sub-cent differences alone do not fail growth
  acceptance. Population, staffing, construction and project reservations must still agree
  exactly. This does not change internal currency storage or round each intermediate calculation.

The first admission edit removes the preview/pressure veto from execution. Per-hour execution
capacity is `min(vacant homes, max(1, ceil(live unclaimed jobs / 24)))`; each real applicant must
still claim affordable recurring income. Future spending by that applicant no longer supplies
fictional marginal wages. A batch tries at most `max(16, 4 * execution capacity)` offers, varying
the family composition/size and home deterministically. Rejections reserve neither wages nor a
home. Applicant income rejections are counted separately from launched carriers in diagnostics.
After workplace planning, admission also retains its claims on ready homes. A household without
adults can claim only a surplus ready slot; worker households consume reserved slots first.
Income rejection consumes neither type of slot, and housing-reservation rejections have a
separate diagnostic. This is O(1) per candidate using the reservation calendar's ready bin and
two batch counters. It prevents pension-only arrivals from consuming the promised labour supply;
a reserved home still supplies no income from an unfinished workplace.

Selection uses the existing vacancy array in O(1) per offer, with O(log V) vacancy-index updates
on successful claims and the existing cached border routing. A dirty index is rebuilt once before
the batch, never once per applicant. No new spatial index or per-offer allocation is introduced.

Completion-time workforce planning reserves housed unemployed adults, actual unemployed adults
from displaced households matched to affordable homes, and one prospective adult per remaining
unclaimed family slot. Existing vacancies and constructing workplaces reserve first;
new commercial and industrial projects must claim their workforce before output customers are
reserved. Shops reserve demand-responsive staffing; factories reserve the authored workforce used
by their operating-cost check. Rejected projects consume neither labour nor customers. Legal
residential candidates can be paired with a workplace only when their authored construction time
meets its completion deadline. Only the needed whole houses enter the construction plan. Pending
workplaces can therefore create homes without providing admission income.

The reusable reservation calendar aggregates existing buildings with Rayon. It uses sum trees
over operational hours, not a new spatial index. Authored/remaining durations retain a dense
bounded prefix; queued completion dates add distinct coordinates instead of allocating every hour
until a potentially distant saved date. Preparation is O(B + (R + Q + H) log(H + Q)), and claims
are amortized O((P + H + Q) log(H + Q)), where B is buildings, R residential candidates, Q queued
projects, P project claims and H the maximum remaining/authored duration (bounded by u16).
Per-project claims neither allocate nor scan the city. Tree/histogram storage is reused. Claims
consume unemployed adults, then committed homes, then optional homes; within each home category
the latest eligible capacity is consumed first.

Queued business placement now revalidates workforce and recurring sales before spending owner
equity. Failed or bulldozed supporting homes cannot supply workers; successful placements reserve
their workers and customers before the next project. A successfully placed home serves incumbent
labour deficits first. Pending arrivals are recorded immediately, so the execution snapshot sees
adults already admitted in the current hour. The derived execution calendar rebuilds once per
active queue hour or external building/access revision. Own successful placements update it
incrementally: warm checks cost O(ports² + log H + K log K + G), with K connected borders and
local entrance geometry G, without per-project city scans or allocations.
Active queue measurement (2026-10-08): isolated release fixtures now measure the numerical
queue path and actual `prepare_queued_spawn` checks. The fixtures deliberately keep physical
placement, rendering and agent simulation outside the timed path. The first uses real asset
capacities, distinct queued parcel IDs and one-minute placement deadlines, with a synthetic
background housing census. It rebuilds the demand snapshot, collects queued commitments,
reserves existing workers/customers and makes additional finite claims. Every measured batch
asserts exact queued home/workplace counts, zero arrival jobs from construction, one customer
reservation per project and exhaustion of worker supply without reuse.

| Background buildings | Queued projects | New claims | Census ms | Queue/calendar ms | New claims ms |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 2,048 | 1,024 | 512 | 0.043 | 0.143 | 0.027 |
| 16,384 | 1,024 | 512 | 0.080 | 0.536 | 0.021 |
| 131,072 | 1,024 | 512 | 0.617 | 3.606 | 0.021 |
| 16,384 | 8,192 | 4,096 | 0.197 | 0.978 | 0.233 |
| 131,072 | 65,536 | 32,768 | 0.744 | 11.161 | 2.791 |

The second fixture retains the same two candidate sites and real entrance/border route. It
separates the initial census/quote from 1,024 subsequent uncommitted permits, modeling failed
placement without spending owner equity or consuming reservations. Background census records
grow independently of the local query. Repeated checks remain independent of background size:

| Background buildings | First check including census ms | 1,024 prepared checks ms |
| ---: | ---: | ---: |
| 0 | 0.057 | 0.324 |
| 2,048 | 0.121 | 0.321 |
| 16,384 | 0.718 | 0.310 |
| 131,072 | 4.198 | 0.316 |

These results establish bounded numerical queue preparation and local repeated validation for
the measured fixtures. Full successful-placement throughput, mixed populated markets, larger
road networks and full-redesign performance acceptance remain open. No runtime behavior changes
in this measurement step. Source inspection still finds import-only input forecasts in
`operation.rs` and citywide customer totals in `hiring.rs`/`demand/spawn_need.rs`; local supplier
prices/routes and buyer connectivity require shared finite reservations across those consumers.

All timings are medians of nine samples after three warmups, unprofiled optimized release,
eight Rayon workers pinned to CPUs `0,2,4,6,8,10,12,14`, with sequential benchmark processes.
Build: `cargo test --manifest-path rust/Cargo.toml --release --lib --no-run`.
Driver: `python3 /tmp/metrum-active-queue-perf.py`; retained executable:
`/tmp/metrum-active-queue-tests` (SHA256
`8f0c25097c4c8467afd156018b9c74ca30f02ee2cf0353731109d2adee1361e5`).
Logs: `/tmp/metrum-active-queue-{reservations,validation}-perf.log`; executable/source/config
identities: `/tmp/metrum-active-queue-identities.txt`. Both ignored benchmark tests pass.
Fresh focused correctness runs pass 88 demand tests and 11 queue-related tests, including queued
30-day cash continuation. Logs: `/tmp/metrum-active-queue-demand-tests.log` and
`/tmp/metrum-active-queue-regressions.log`; commands use the release test executable with filters
`simulation::economy::demand::` and `queued`, `RAYON_NUM_THREADS=8`, `--test-threads=1`.
The preceding full-suite/10k results below remain evidence for their original build.

Per-resource supply quotation foundation (2026-10-08): `operation.rs` now separates a recipe's
net daily input, circulation buffer and legal replenishment batch from the prices of its sources.
`for_plan_with_input_quotes` accepts per-resource goods, amortized carrier and buffer costs;
`working_capital_assets_at_prices` values existing input stock at matching replacement prices.
Payroll, utilities, property tax and export freight stay in the same shared calculation. Quotes
are read-only and must cover each complete requirement; selecting and reserving finite supplier
capacity remains the caller's responsibility. Infinite input costs represent unavailable supply,
and input-free services need no supply quote. The core adds O(I) callback calls, no allocations
or city scans; existing import quotes retain O(I*O + (I+O)*log(resources)) cost.

The original quotation-boundary change below did not integrate local markets. Live hiring now
stages finite local input claims, values replacement stock at the staged source prices and commits
only accepted operations. The current source seed consists of import-funded, currently staffed
production; quantities are net daily output, not physical shipment stock. Missing outside routes
have no fabricated flat-fee quote. Directed routes to real and proposed doors use the same freight
geometry/cache, and dispatch now chooses a cheaper executable import over a local offer when
appropriate. Rejection restores prior quantities exactly, including a failure on a later input or
working-capital check. A real hiring regression funds one factory from a farm's 200 daily grain
units, rejects a second claim, and admits the second factory when the first fails its capital check.

New-entry planning now forks the live residual supply once, retaining warm index allocations.
Placed construction and queued projects claim inputs before new candidates; queued doors are
prepared with the queue census and claims follow execution order. Alternative selection requotes
its chosen winner before committing, rather than retaining the last inspected asset's claim.
Reconstruction first releases every pending project's output in a private market copy, then
replays constructing buildings in allocator order and queued projects in execution order.
Its resource vectors reuse execution scratch instead of allocating another market each pass.
After each input decision it restores that project's full output capacity, including failed
input checks. This prevents two unfinished shops from each seeing only the final leftover
customer demand and under-reserving inputs; the caller's full capacity commitments stay intact.
Execution refreshes live funding and constructing claims once per hour or external layout/metric
revision, stages the proposed input claims, and commits them only after physical placement.
Successful appends update the execution epoch without another city scan. Failed checks and dropped
permits cannot consume another project's supply. Warm project checks add local input quotation
work to the existing workforce/customer/door checks; preparation includes one O(B + ports + routes)
live funding/commitment pass. The final construction-input release run passes **2,204 library
tests** (103 ignored), all three growth integration scenarios and both save/reload continuations;
only the existing utility-machinery assertion fails (540 versus 96). Its year, two-year,
expansion and recovery reports exactly match the preceding live-supply build. Command:
`RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`, with the four growth-report variables
targeting `/tmp/metrum-project-input-final-{year,two-years,expansion,recovery}.txt`.
Full log: `/tmp/metrum-project-input-final-all-tests.log`. Retained test executable and complete
Rust source/config identities: `/tmp/metrum-project-input-final-tests` and
`/tmp/metrum-project-input-final-test-identities.txt`. The 99 focused demand tests also pass.
The fresh saved-town year is byte-for-byte identical to the preceding report: 80 residents,
66 employed, 45 households, 62 buildings and all 15 firms operating-cash positive. Report/log:
`/tmp/metrum-project-input-final-saved-city-365.txt` and
`/tmp/metrum-project-input-final-saved-city-365-tests.log`. The read-only diagnostic uses
`/tmp/metrum-consumer-budget-input.sqlite` and the user's existing mods.

The sequential unprofiled 10k comparison against the retained optimized live-supply build is
**5.6715 → 5.6546 ms hourly** (p = 0.82) and **379.29 → 384.92 ms daily**
(p = 0.96, wide intervals). Initial and final city records match exactly. This finds no
additional construction-integration regression; it does not resolve the earlier 2.69 → 5.79 ms
hourly increase or establish full-redesign acceptance. Both runs use eight Rayon workers and
CPUs 0,2,4,6,8,10,12,14. Driver, source/build identities and logs:
`/tmp/metrum-project-input-perf.py`, `/tmp/metrum-project-input-identities.txt`, and
`/tmp/metrum-project-input-{baseline,after}-perf.log`. Retained binaries:
`/tmp/metrum-local-supply-optimized-bench` and `/tmp/metrum-project-input-bench`.

Fresh pinned release placement measurements use the actual queued execution path, including
placement, index updates, reservation commits and owner funding. Each district has four
constructing six-household homes, two proposed food factories and a real outside road connection;
the benchmark asserts every placement and exact startup equity, with no workers or sales yet.
Five independent fixtures per size produce these medians:

| Successful factories | Setup ms | First check ms | Placement batch ms | Owner equity |
|---:|---:|---:|---:|---:|
| 2 | 6.142 | 0.126 | 0.082 | 26,200 |
| 16 | 11.820 | 0.108 | 0.575 | 209,600 |
| 128 | 74.011 | 0.315 | 5.565 | 1,676,800 |
| 1,024 | 721.076 | 6.484 | 113.358 | 13,414,400 |

Setup and the first census/quote are outside placement timing. Buildings, projects and borders
all increase here; this establishes successful import-backed placement throughput, not fixed
neighborhood locality or populated local-market acceptance. The separate 1,024 uncommitted-check
benchmark stays at 0.330–0.341 ms with 0–131,072 background buildings; its first census/check
grows from 0.025 to 8.441 ms. Commands use the retained test binary with
`benchmark_successful_project_placement` or `benchmark_queued_project_validation`, followed by
`--ignored --nocapture --test-threads=1`, with the same pinned eight-worker environment.
Logs: `/tmp/metrum-project-input-final-physical-placement.log` and
`/tmp/metrum-project-input-final-queue-scaling.log`. The sequential diagnostic/measurement driver
is `/tmp/metrum-project-input-final-checks.py`.

The subsequent live-service change reserves local inputs in the same building-order transaction
pass as private firms. Fiscal policy still determines public payroll; material delivery limits
the input-backed activity reserved in this book, not the city's employment policy. Service
operations use the existing treasury-backed invoice contract rather than a private sales/equity
test. Descending integer staffing probes reuse the same source/retry buffers and release failed
claims exactly: worst-case O(funded service workers * recipe ports² + source searches), with
serial commits required by shared quantities. No additional per-operation buffers are allocated.
The new water-plant regression reserves 0.5 daily machinery units from a 40-unit staffed source;
an unreachable service leaves the full 40 units available, with payroll and cash unchanged.

Fresh service-build checks pass **129 household tests**, **99 demand tests**, both reload
continuations, all three growth integrations, scarce-labour expansion and failure recovery.
The year, two-year, expansion, recovery and saved-town reports exactly match the construction
build. This is targeted validation after the preceding full suite, not a new full-suite count.
Driver/logs: `/tmp/metrum-service-input-checks.py`,
`/tmp/metrum-service-input-{households-tests,demand-tests,continuation-tests,expansion-tests,recovery-tests,growth-tests}.log`.
Reports: `/tmp/metrum-service-input-{year,two-years,expansion,recovery,saved-city-365}.txt`.
Retained binary/source identities: `/tmp/metrum-service-input-tests` and
`/tmp/metrum-service-input-test-identities.txt`.
The service follow-up's sequential unprofiled comparison uses the same eight workers, affinity
and 10k workload against `/tmp/metrum-project-input-bench`: **5.6725 → 5.6216 ms hourly**
(p = 0.52), **375.29 → 372.47 ms daily** (p = 0.98, wide intervals), with identical initial/final
city records. It detects no incremental regression in this workload; varied populated service
and local-buyer mixes still need acceptance. Driver/identities/logs:
`/tmp/metrum-service-input-perf.py`, `/tmp/metrum-service-input-identities.txt`, and
`/tmp/metrum-service-input-{baseline,after}-perf.log`; retained current benchmark:
`/tmp/metrum-service-input-bench`.

Replacement input integration now records accepted live claims by owner in reusable sparse
storage. The receipt bound is O(output ports + accepted input ports); owner lookup is O(1),
and releasing/restoring an incumbent costs O(its claims * log source bucket size). Upgrades and
downgrades quote the target's actual door after releasing only that owner's inputs. Rejection
restores prior quantities and index availability exactly; acceptance retires the old receipt
once. Retained input stock uses the same quoted prices as the target's required buffer.
Fresh replacement-input verification passes **2,207 library tests**, all three growth scenarios
and both reload continuations; only the known utility-machinery assertion fails (540 versus 96).
All five growth/saved-town reports match the service build. The owner-journal regression covers
cold/warm forks, repeated exact rollback and retiring only the replaced owner's claims; a real
planner regression permits only the incumbent holding sufficient grain to upgrade.
Sequential unprofiled 10k runs with eight workers on CPUs 0,2,4,6,8,10,12,14 measure
**5.5472 → 5.5806 ms hourly** (p = 0.67) and **369.74 → 371.58 ms daily**
(p = 0.98, wide intervals), with identical initial/final records. This detects no incremental
regression; it does not resolve the earlier hourly increase. The fresh physical run places
1,024 factories in **107.044 ms**, with setup **716.535 ms**, first check **6.441 ms** and
exact owner equity **13,414,400**; 1,024 warm queue checks take **0.328–0.334 ms** through
131,072 background buildings (first census/check **0.035–8.800 ms**).
Artifacts use `/tmp/metrum-replacement-input-`: `all-tests.log`, `checks.py`, `checks.log`,
`test-identities.txt`, `perf.py`, `identities.txt`, `{baseline,after}-perf.log`,
`physical-placement.log`, `queue-scaling.log` and the five named report `.txt` files.
Retained binaries: `/tmp/metrum-replacement-input-tests.gz` (lossless executable archive) and
`/tmp/metrum-replacement-input-bench`. Commands match the preceding stages.

Constructing explicit producers now quote their actual hectare-scaled recipe and physical
worker count through the same operating calculator, using current cash, usable stock and paid
cargo for capital. Insufficient capital releases all staged inputs. Pending services share the
same finite book using their fiscal staffing ceiling and treasury-backed material invoices;
a separate derived ceiling retains their future requirement without advertising live jobs.
Service ceilings refresh in the existing parallel building pass, adding O(B) reusable storage;
construction forks copy them and paid cargo once. Remapping moves both with their owners.
Placement and demolition undo register service ceilings from current fiscal policy before the
next hourly pass. Undo remaps grow prepared arrays for restored indices in O(mapping + new
slots), without allocating unprepared citywide views.
Explicit producer probes cost O(recipe ports² + source queries); service probes cost at most
O(funded workers * recipe ports² + source queries), using existing retry buffers and ordered
commits. Full output commitments remain reserved. Frontage-price revisions now invalidate
both hourly source-funding reuse and the queued execution epoch.
The mixed regression passes for a 10 ha farm (1 machinery/day), 1 ha mine (4/day) and public
water plant (0.5/day) sharing 40/day, including capital rejection, paid cargo and disconnected
service cases. Placement/undo tests cover current fiscal policy at 0%, 50% and 100%; funding
remap tests cover cycles, removal and restoring a last index after arrays have shrunk.

The final explicit-project/undo release run passes **2,211 library tests**, all three growth
scenarios and both reload continuations; the sole failure remains the known utility-machinery
assertion (540 versus 96), with 103 tests ignored. All five growth/saved-town reports exactly
match the replacement-input build, retaining 80 saved-town residents and 15 operating-cash-positive
firms. Driver, full log, source/binary hashes and reports:
`/tmp/metrum-service-undo-checks.py`, `/tmp/metrum-service-undo-all-tests.log`,
`/tmp/metrum-service-undo-test-identities.txt`, and
`/tmp/metrum-service-undo-{year,two-years,expansion,recovery,saved-city-365}.txt`.
The retained executable is losslessly archived at `/tmp/metrum-service-undo-tests.gz`;
its original and archive hashes are both recorded. Earlier explicit/replacement test executables
were also gzip-archived to relieve the temporary-file quota; decompressed hashes match their
original identity records.

Matched unprofiled 10k runs retain identical initial/final city records. The first explicit-project
comparison measured **5.5336 → 5.6779 ms hourly** (+2.61%, borderline p = 0.05) and
**367.69 → 391.12 ms daily** (p = 0.84, wide intervals). The final build, run first against the
retained replacement-input baseline, measured **5.6671 ms baseline / 5.6609 ms current hourly**
(p = 0.93) and **368.79 / 369.81 ms daily** (p = 0.99). The small increase did not reproduce;
the much larger earlier regression remains unresolved. Both runs use eight Rayon workers and
CPUs 0,2,4,6,8,10,12,14. Drivers, identity records and logs are
`/tmp/metrum-{explicit-project,service-undo}-perf.py`, corresponding `-identities.txt`, and
`-{baseline,after}-perf.log`. Retained current benchmark: `/tmp/metrum-service-undo-bench`.

The preceding explicit-project build also passed physical placement: 1,024 factories in
**107.412 ms**, with **721.324 ms** setup, **6.330 ms** first preparation and exact owner equity
**13,414,400**. Its 1,024 warm queue checks took **0.329–0.337 ms** through 131,072 background
buildings; first census/check **0.029–6.802 ms**. Commands match the construction measurements
above; logs are `/tmp/metrum-explicit-project-{physical-placement,queue-scaling}.log` and the
retained test archive is `/tmp/metrum-explicit-project-tests.gz`. These measurements precede
the final undo hook/remap fix. They hold the frontage revision fixed: changed-price execution
still refreshes the citywide market once per revision, so populated queue locality under
changing frontage prices needs acceptance alongside the broader market/scaling work.

Live funding now retains exact accepted recipe activity alongside integer job ceilings. The
construction snapshot uses that activity, capped by actual staff and scaled by explicit work
area, for private businesses' input purchases. Unstaffed, capital-rejected and revenue-rejected
businesses contribute no recurring input customers; full net output commitments remain reserved.
Public invoices instead use fiscal staffing ceilings, independently of material availability.
Initial recurring supplier quantities use exact import-funded activity: rounding 2.25 required
workers to three jobs cannot turn 30 funded units into 40 offered units. The new derived activity array
adds O(B) reusable storage; reads cost O(1), and refresh/remap reuse existing passes.
If local stock repricing cannot finance a previously validated import operation, funding keeps
that import alternative and releases all rejected local claims. This preserves the production
already promised by the initial source book, without another city scan or additional quote.
The initial live-hiring buyer aggregation still needs funded-customer closure and reachability;
filtering the construction snapshot alone does not establish either contract.

Fresh verification of this activity/import-alternative build passes **418 economy tests** and
**2,213 library tests**, all three growth integrations and both 30-day reload continuations.
The sole full-suite failure remains the existing utility-machinery assertion (540 versus 96);
103 tests are ignored. All five year/two-year/expansion/recovery/saved-town reports are byte-identical
to the service-undo build, retaining 80 saved-town residents and 15 operating-cash-positive firms.
The new tests cover fractional activity, staffing/capital/revenue rejection, fiscal input demand,
activity remapping and stock repricing that must preserve an import plan and release local claims.
Artifacts: `/tmp/metrum-funded-buyers-{economy-tests,all-tests,saved-city-365-tests}.log`,
`/tmp/metrum-funded-buyers-checks.py`, `-checks.log`, `-test-identities.txt` and the five report
`.txt` files. The retained test executable is `/tmp/metrum-funded-buyers-tests.gz`.

Matched sequential unprofiled 10k runs use eight Rayon workers on CPUs 0,2,4,6,8,10,12,14.
The first comparison measures **5.4853 ms baseline / 5.7023 ms current hourly** (about +4%,
p < 0.01) and **370.45 / 373.56 ms daily** (p = 0.98). Reversing run order measures
**5.6079 / 5.5031 ms hourly** (p = 0.14) and **372.24 / 372.16 ms daily** (p = 1.00).
The initial increase does not reproduce; both pairs retain identical initial/final city records.
This does not resolve the earlier full-redesign hourly regression. Drivers, identities and logs:
`/tmp/metrum-funded-buyers[-reverse]-perf.py`, corresponding `-identities.txt` and
`-{baseline,after}-perf.log`; retained current executable `/tmp/metrum-funded-buyers-bench.gz`
(decompressed SHA-256 verified against its identity record).
The fresh physical benchmark places **1,024 factories in 115.448 ms**, excluding **741.306 ms**
setup and **6.439 ms** first preparation, and verifies exact owner equity **13,414,400**.
Its **1,024 warm queue checks take 0.326–0.333 ms** through 131,072 background buildings;
first census/check costs **0.079–6.724 ms**. Commands match the preceding physical/queue runs;
logs: `/tmp/metrum-funded-buyers-{physical-placement,queue-scaling}.log`. These queue measurements
still hold frontage prices fixed and do not establish locality under price changes.

Recurring source selection now breaks equal-route ties by the supplier's actual goods price,
then stable building index, matching dispatch for the same batch. Cached incumbents remain
only search bounds: a later, cheaper source on an equal route must still win. Price lookups
run only on exact time ties, adding O(1) arithmetic per such candidate without new route
queries, allocations, indices or scans. An isolated authored-catalog regression checks live
and proposed doors, a cached expensive incumbent, mixed-price depletion, retry and rejection
against physical dispatch. Fresh release economy verification passes **419 tests** (30 ignored).
Matched unprofiled 10k timing against the retained funded-activity build measures
**5.7100 → 5.6692 ms hourly** (p = 0.53) and **369.49 → 380.31 ms daily** (p = 0.93,
wide intervals), with identical initial/final city records. The earlier version fetched prices
for every routed candidate and measured 5.6498 → 5.9050 ms hourly (p = 0.11, one outlier);
final code fetches prices only on ties. Neither run establishes full-redesign acceptance.
Both runs use eight Rayon workers, CPUs 0,2,4,6,8,10,12,14 and sequential release executables.
Artifacts use `/tmp/metrum-freight-price-ties-`: `economy-tests.log`, `perf.py`,
`perf-driver.log`, `identities.txt`, `{baseline,after}-perf.log` and `bench.gz` (verified against
its decompressed hash). Earlier eager-price results use `/tmp/metrum-freight-price-order-`.
The drivers execute the verified `/tmp/metrum-funded-buyers-bench.gz` baseline from an anonymous
memory file to avoid another large temporary executable; its hash is recorded. Full-suite,
growth/reload and physical-placement results above belong to the preceding funded-activity build.

Recurring input quotes now apply the receiving operation's current authored freight-window
multiplier to local and imported goods, buffer valuation and retained input stock. Carrier
charges stay unchanged. Explicit profile timing overrides zone defaults, using the same
lookup and adjustment as physical dispatch. Employment, planning and queued execution pass
the actual simulation minute; load and policy refresh use the restored/current clock. A
window boundary invalidates the funding view and execution market even within the same
execution hour. Price intervals cost O(authored windows) to identify; each operation's lookup
costs O(authored freight profiles + windows), with no city scan, allocation or route query.
These are current-price forecasts, like frontage prices, rather than averaged daily tariffs.
Demand snapshots and level changes now consume the funding view refreshed for the same
routes and price interval. The existing refresh and snapshot passes are reordered without
adding a city scan; moving the reservation book out temporarily lets the planner read matching
funding columns without copying those columns.

Fresh release verification passes **421 economy tests**, **2,216 library tests** (103 ignored),
all three growth integrations and both 30-day continuations. The sole failure remains the
unrelated utility-machinery assertion (540 versus 96). Year, two-year, scarce-labour expansion,
recovery and saved-town-year reports are byte-for-byte identical to the funded-activity build;
the saved town retains 80 residents and all 15 operating-cash-positive firms. Focused tests
cover local/import mixtures, exact dispatch prices, unchanged carrier fees, buffer/stock
valuation, rollback, the 07:59–08:00 boundary within one execution hour, and stale employment
funding versus the refreshed planning market. Admission fixtures now prepare actual funded
factory routes, and paid-cargo coverage shares the real logistics view with planning.
Commands match the full release and saved-town commands above. Artifacts use
`/tmp/metrum-freight-windows-`: `checks.py`, `checks.log`, `economy-tests.log`, `all-tests.log`,
`test-identities.txt`, `{year,two-years,expansion,recovery,saved-city-365}.txt` and
`saved-city-365-tests.log`. Source/build hashes are retained; no additional large test executable
was copied into the capacity-limited temporary filesystem.

Matched sequential unprofiled 10k release runs use eight Rayon workers on CPUs
0,2,4,6,8,10,12,14 and the retained price-tie build as baseline. The first pair measures
**5.6083 / 6.0569 ms hourly** (baseline/current, +8%, p < 0.01) and **375.34 / 376.93 ms
daily** (p = 0.98). Reversed order measures **5.6277 / 5.6565 ms hourly** (p = 0.73) and
**377.28 / 374.73 ms daily** (p = 0.98): the first increase does not reproduce. Initial records
and final discrete outcomes match, but final treasury is **54 currency lower** in the current
build in both comparisons. These are not identical outcome records or full-redesign acceptance;
same-build 30-day cash/discrete reload checks pass separately. Drivers, identities and logs use
`/tmp/metrum-freight-windows[-reverse]-{perf.py,identities.txt,baseline-perf.log,after-perf.log}`;
`perf-driver.log` records the outcome comparison. The current benchmark is retained as
`/tmp/metrum-freight-windows-bench.gz`, with its decompressed hash verified.

Fresh physical execution places **1,024 factories in 110.622 ms**, excluding **721.296 ms** setup
and **6.452 ms** first preparation, and verifies exact owner equity **13,414,400**. Its
**1,024 warm queue checks take 0.387–0.393 ms** through 131,072 background buildings, versus the
preceding build's historical 0.326–0.333 ms; first preparation costs **0.033–6.899 ms**. The new
authored-window phase check adds fixed work per proposal; these runs retain flat warm locality
but still hold frontage prices fixed. Commands run the release library executable's ignored
`benchmark_successful_project_placement` and `benchmark_queued_project_validation` tests with
`--ignored --nocapture --test-threads=1`, the same eight workers and CPU affinity. Logs use
`/tmp/metrum-freight-windows-{physical-placement,queue-scaling}.log`.

Accepted live private operations now publish their additional net production after local
inputs fund them. Publication adds only the increase over the import-funded seed, caps it by
actual staff, preserves existing claims, and enables the existing resource/component/chunk
index. It costs O(output ports + newly enabled ports * log bucket size), with no new routes,
spatial structure or per-operation allocation. Each source publishes once in the pass; receipt
storage is reserved for twice the source output ports plus accepted input ports, covering
exhaustion before and after publication. Seed and publication use identical production arithmetic.

Authored recipe dependencies now compile into supplier-before-buyer phases. Strongly connected
recipes share a phase; independent/sibling recipes at the same depth retain stable building
priority. Compilation is O(recipe ports² + authored dependency edges + profiles + resources),
independent of city size. A stable counting sort uses reusable O(buildings + phases) scratch
once per live funding pass. This allows downstream acyclic recipes to use validated local
production regardless of building index, without repeated city scans or supplier searches.
The derived phase metadata needs no save-format change.
Fresh focused release verification passes **424 economy tests** (30 ignored). The new chain
fixture checks both processor/shop building orders, exact finite depletion, removal of staff,
large inert stock, rejected producer capital and recovery. A separate publication regression
checks the delta after earlier claims, failed oversized quotes, owner release/retry and a
construction-book copy. The phase test covers a material cycle, downstream buyers and sibling
priority. Log: `/tmp/metrum-local-publication-economy-tests.log`.
Fresh full release verification passes **2,219 library tests** (103 ignored), all three growth
integrations and both 30-day save/reload continuations. The only failure remains the unrelated
utility-machinery assertion (540 versus 96). Year, two-year, expansion, recovery and saved-town
year reports are byte-for-byte identical to the freight-window build; the saved town retains
80 residents and all 15 operating-cash-positive firms. The reusable release/saved-town commands
run through `/tmp/metrum-local-publication-checks.py`; corresponding `checks.log`, `all-tests.log`,
`test-identities.txt`, `{year,two-years,expansion,recovery,saved-city-365}.txt` and
`saved-city-365-tests.log` retain results and source/build identity.

Matched sequential unprofiled 10k release runs against the retained freight-window build use
eight Rayon workers on CPUs 0,2,4,6,8,10,12,14. The first pair measures **5.7231 / 6.8628 ms
hourly** (baseline/current, +19.9%, p < 0.01) and **372.59 / 423.03 ms daily** (p = 0.70).
Reverse order measures **5.6632 / 5.5653 ms hourly** (p = 0.23) and **381.50 / 379.43 ms daily**
(p = 0.99): the initial increase does not reproduce. Initial records and final discrete counts
match; current final treasury is **54 currency higher** in both pairs. Outcome records are
therefore not identical, and these incremental runs do not establish full-redesign acceptance.
Artifacts use `/tmp/metrum-local-publication[-reverse]-`: `perf.py`, `perf-driver.log`,
`identities.txt`, `baseline-perf.log` and `after-perf.log`. The verified baseline archive is
`/tmp/metrum-freight-windows-bench.gz`, executed through an anonymous memory file.
The current benchmark archive is `rust/target/goal-evidence/metrum-local-publication-bench.gz`,
with its decompressed hash checked against the identity record; keeping it under the ignored
build directory avoids another large temporary-filesystem copy.
Fresh physical execution verifies **1,024 factories in 108.490 ms**, excluding **721.970 ms**
setup and **6.388 ms** first preparation, with exact owner equity **13,414,400**. Its **1,024
warm queue checks take 0.384–0.399 ms** through 131,072 background buildings; first preparation
costs **0.036–7.219 ms**. Commands and eight-worker CPU affinity match the preceding physical
and queue runs. Logs: `/tmp/metrum-local-publication-{physical-placement,queue-scaling}.log`.
These import-backed placement fixtures keep frontage prices fixed; they do not establish
full mixed-market scaling or publication performance at one million residents.

Live consumer-facing firms now claim finite quantities and spending from individual families.
The existing recurring-income visitor preserves finite benefit order, excludes savings and
temporary unemployment transfers, and subtracts taxes, utilities and property bills. Each
family's remaining gross spending is split among consumer resources by authored catalog basket
cost shares; desired units remain capped by actual members. Quotes spend that category's money
at the seller's price including VAT, while seller revenue excludes VAT. Neither another family's
income nor a cheap seller creates additional desired units. Repeated staged claims keep one
original receipt per account; rejection restores exact prior values and spatial eligibility.
An exhausted budget is retired exactly at its affordability bound, without residual roundoff buyers.

The same component/chunk bucket type indexes household shoppers separately by consumer category
and walking/car access. Weak components only prune candidates: the ordinary trip estimator
checks both directed trips, including pedestrian connectors, from actual or proposed entrances.
Children-only families have no eligible shopper. The existing agent census also records whether
an adult/elder owns a car; loading reconstructs that derived flag from persisted agents without
changing format 83 or relying on stale member links after freight-agent mutation.
Warm preparation costs O(households * consumer categories + building output ports), with Rayon
for independent accounts and category indices; access changes rebuild the existing buckets.
Quotes visit available bucket candidates and perform their directed route queries. Serial family
commits are finite dependencies. Accepted firms retain full reachable net-capacity quotas through
local-input staffing retries; rejected capital or operating-cost checks release their claims.
The trip estimator currently allocates network search/path storage; this integration is not
performance acceptance. Construction forks/owner receipts and replacement customer claims remain
to be connected before the family book governs every growth decision.

This remains intermediate integration: the live revenue forecast still counts staffed business
input demand before proving those buyers' financing. Business-to-business local revenue still
uses catalog resource prices, whereas physical local sales use the seller profile's price;
export revenue correctly uses the catalog export tariff. Funded business-buyer closure, closure
within cyclic recipe groups, replacement output publication/withdrawal and shared construction
customer reservations remain unfinished. Final focused validation and the remaining timing
regression are recorded in the continuation handoff below.
The earlier live-consumer release library verification passes **2,224 tests** (103 ignored), including both
30-day continuations and five new budget, access, rollback, car-census and live-funding cases.
The save round-trip regression additionally checks that car access is derived from loaded agents.
Only the pre-existing utility-machinery assertion fails (540 versus 96). Command:
`RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib -- --test-threads=1`;
log: `/tmp/metrum-consumer-live-library-tests.log`. All three growth integrations and the saved-town
continuation passed separately; this build predates the owner-receipt and walking-proof changes.

##### AI continuation handoff — 2026-10-08

**The overall goal is unfinished. `DEM-02` remains `in_progress`.** The user requested this
handoff after the current live-consumer work so another AI can finish the goal. Read the full
objective at `/home/jmkekala/.codex/attachments/0ff2799e-42be-40ba-bf04-d256bf7cc597/goal-objective.md`
and repository `CLAUDE.md` before continuing. Its original evidence predates the changes below.
Work in `/home/jmkekala/Documents/Projects/Hobbies/metrum_rise`. Preserve the extensive dirty
worktree, including essential untracked Rust modules. No commit, cleanup, deployment or subagent
work is authorized. User assets under `/home/jmkekala/.local/share/godot/app_userdata/Metrum Rise/mods`
are read-only. Keep tracking updates in the existing owning docs.

**Current implementation:** future-home priority for displaced households, actual available
workers and post-assignment income are implemented. Finite recurring input claims already cover
live firms, construction, queued/new projects and incumbent input replacement. Actual supplier
prices/routes, receiving windows, rollback, fiscal service ceilings, staffed production caps,
local-output publication and compiled supplier phases are present. Preserve these systems.
Live consumer firms now use `customers.rs` / `customers/routing.rs` through `hiring.rs`:
per-family recurring budgets, desired-quantity limits, seller-specific prices/VAT, directed
round trips, exact rejected-quote rollback and full reachable net-capacity retention during
staffing searches. They commit owner receipts with `ConsumerMarket::accept_for(index)`.

The book exposes tested `copy_reservations_from`, `release_owner`, `accept_for`, `accept` and
`reject` for construction/replacements, **but those construction call sites are not wired**.
The unused-method warnings reflect this unfinished integration; do not hide them with lint
allowances. `Household.shopping_has_car` is derived during the existing membership census and
rebuilt after SQLite load. Save format remains 83. Do not use linked household member scratch
after freight-agent mutation.

Reachability uses existing component/chunk reservation buckets. The first implementation
repeatedly built paths and regressed 10k setup from 13.8 to 272.3 seconds. The follow-up caches
positive proofs over actual directed pedestrian lane connectors in
`agents/tick/planning/reachability.rs`, reusing iterative strongly-connected-component traversal
extracted from recipe ordering into `pathing/components.rs`. Validated home endpoint groups are
cached, avoiding repeated door geometry work. Equal-node pairs retain the ordinary direct
connector check; unproven pairs retain the exact trip planner. Cold preparation is O(lanes +
connectors + household/category indexing); warm account preparation is O(households * consumer
categories + building output ports). Quotes visit remaining indexed candidates. Positive walking
proofs allocate nothing, but fallback route searches still allocate and need further work.

**Next implementation steps, in order:**

1. Connect the live residual customer book to `demand/supply.rs::SiteSupply` and its existing
   `ProjectSupply` transaction boundary. The hourly fork is in `demand/system.rs`; execution
   revalidation is in `demand/execution.rs`. Replay constructing/queued customer claims before
   new proposals, including input-free service shops. Replacement quotes must release only
   the incumbent's receipts, restoring everything if any subsequent check fails.
2. Carry the actual customer quote's supported worker count through selection, labour claims,
   queued/construction calendars and execution permits. `spawn_need.rs` still computes
   commercial workers/revenue from citywide `OutputMarket`; `building_actions.rs` still applies
   a citywide absorption gate and claims `candidate_workers`; `workforce.rs` still derives
   constructing/queued workers with `committed_workers`. Adding only a late reachability veto
   leaves those overlapping decisions and potentially wrong worker reservations in place.
   Retain full net output commitments through construction and staffing.
3. Close funded business-buyer demand. `hiring.rs` still aggregates raw staffed private input
   demand before proving the buyers' finances. Its initial source set is import-funded staffed
   production; accepted local operations publish increases in supplier-before-buyer phases.
   This handles acyclic seeded chains, but does not jointly establish all locally funded chains
   or closure inside recipe cycles. Neither raw staffing, stock nor owner grants proves buyers.
   Business-to-business forecasts still use catalog local sale prices while actual freight sales
   use seller prices. Exports correctly retain catalog export tariffs.
4. Implement replacement output withdrawal/publication and affected downstream commitments.
   Existing incumbent input receipts alone do not handle removal of its promised supply.
5. Finish performance and full acceptance: eliminate route-query hot allocations, address the
   still-open full-redesign regression (original hourly baseline about 2.69 ms), test populated
   varied local markets and supply/buyer connectivity, and include execution under changing
   frontage prices. Fixed-price import-backed 1,024-placement and warm queue results are prior
   limited evidence, not full local-market or million-resident acceptance. Keep startup, scarce
   labour/owner spending, recovery, saved-town solvency and exact discrete reload checks.

**Validation boundary:** `/tmp/metrum-consumer-live-library-tests.log` records 2,224 passes,
103 ignored and the known unrelated utility-machinery failure (actual 540, expected 96).
That run and all three growth integrations precede the later owner-receipt/lane-proof changes.
The one-year, two-year and saved-town reports for that build match the local-publication build
byte-for-byte. The latest current-source checks and timing use
`/tmp/metrum-consumer-final-checks.py`, `/tmp/metrum-consumer-final-perf.py` and their corresponding
`checks.log`, `*-tests.log`, `*-perf.log`, `test-identities.txt` and `identities.txt` artifacts.
Verify artifact existence and source hashes before reusing them; `/tmp` evidence can disappear.

**Final current-source results:** the sequential driver finished successfully. Release checks
pass 430 economy tests (30 ignored), 23 pathing tests (3 ignored), 25 save tests (3 ignored),
both 30-day save/reload continuations and the 365-day saved-town diagnostic. These focused
suites overlap; do not sum them as unique tests. The saved-town report is byte-identical to
`/tmp/metrum-local-publication-saved-city-365.txt`, retaining 80 residents and all 15 firms
operating-cash positive. All 956 test-identity records and 959 benchmark-identity records were
verified against the final files. The final full library and three growth integrations were
not rerun after the owner-receipt and walking-proof changes; the earlier results above are
historical evidence for their own build. The known utility assertion remains unresolved.

The final matched 10k hourly benchmark is **5.7515 → 6.4500 ms (+12.144%, p < 0.05)**;
setup is 13.2 → 13.8 seconds. This is an unresolved incremental regression, in addition to
the earlier full-redesign regression. There are 23 samples per executable, with the existing
fixture's short measurement configuration. Initial/final discrete counts match (10,000 residents,
5,003 households, 6,791 employed, 13,881 jobs, 7,396 buildings), but final treasury differs:
1,000,000,000,236,859.0 baseline versus 1,000,000,000,205,691.4 current. This workload does
not establish equal economic outcomes or million-resident acceptance. No final daily benchmark
was run. Commands and workload settings are in `/tmp/metrum-consumer-final-perf.py`; raw timing
is in `/tmp/metrum-consumer-final-{baseline,after}-perf.log`. No validation pipeline remains
running. Start the next session with construction integration, preserving this evidence boundary.

The retained pre-consumer benchmark is
`rust/target/goal-evidence/metrum-local-publication-bench.gz`, decompressed SHA-256
`446f4eefa013ce2d3220dd20b7c345d25e167b4960e3ab564570d53ddf1f03bd`.
The performance driver executes it through an anonymous memory file, with eight Rayon workers
and CPUs `0,2,4,6,8,10,12,14`, sequential unprofiled release runs. Benchmark population/staffing/
building counts match across the consumer change, but treasury does not; do not claim identical
economic outcome records. The earlier `/tmp/metrum-consumer-proof-perf-driver.log` assertion
failure was a report-count mistake after both measurements completed; the subsequent drivers
correct it. The deliberately stopped first consumer benchmark is not acceptance evidence.

Do not treat the source seed or existing citywide buyer
totals as full local-market acceptance. The existing resource/component/chunk index now supports
allocation-free depletion and rollback, with a per-bucket availability tree and an empty-component
fast exit. Static users retain flat iteration. Production arithmetic uses Rayon; finite claims are
serial dependencies. Source geometry is cached by building/entrance/CCH topology/metric revisions, while sparse
output-port quantities refresh in O(B + output ports + availability changes * log bucket size).
Exact source searches visit live nearby candidates and verify directed routes; broader query and
full-redesign scaling still require acceptance measurements.
Source searches now bound individual candidates by straight-line door distance at the fastest
road/driveway speed, and reprice the preceding supplier as an initial bound without changing
eligibility or route/price/index ordering. Staffing retries restore quantities and replay their exact
supplier order within that operation; accepted/rejected operations clear the replay. The existing
bucket counts also prove when a recipe has no remaining local offers, allowing reuse of its
parallel import forecast instead of a redundant serial calculation.

The preceding optimized live-supply build passes **2,200 library tests** (102 ignored), all three growth
scenarios and both 30-day save/reload continuations. The sole failure is still the existing
utility-machinery assertion (540 versus 96). The added dispatch regression preserves the local
option when the cheaper import gateway has no capacity. Year, two-year, expansion, recovery and
365-day saved-town reports are byte-for-byte identical to the preceding live-supply build.
Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`, with the four `METRUM_GROWTH_*REPORT`
variables targeting `/tmp/metrum-local-supply-optimized-{year,two-years,expansion,recovery}.txt`.
Full log: `/tmp/metrum-local-supply-optimized-all-tests.log`; saved-town report/log:
`/tmp/metrum-local-supply-optimized-saved-city-365.txt` and
`/tmp/metrum-local-supply-optimized-saved-city-365-tests.log`. Retained executable/identities:
`/tmp/metrum-local-supply-optimized-tests` and
`/tmp/metrum-local-supply-optimized-test-identities.txt`.

That stage's sequential unprofiled comparison against the retained future-home build records
**2.6914 → 5.7899 ms hourly** (+115.13%, p < 0.01) and **186.39 → 374.88 ms daily**
(p = 0.14, wide intervals). The hourly regression remains unresolved; the daily result does not
establish equivalence. Workers, CPU affinity and benchmark workload match the initial comparison
below. All four city records match the initial live-supply build, including its changed final
treasury. Driver/identities/logs: `/tmp/metrum-local-supply-optimized-perf.py`,
`/tmp/metrum-local-supply-optimized-identities.txt`, and
`/tmp/metrum-local-supply-optimized-{baseline,after}-perf.log`. Fresh pinned depleted-bucket
timings are 0.1015/0.1184/0.1349 ms for 4,096 one-survivor queries at
1,024/8,192/65,536 suppliers (`/tmp/metrum-local-supply-optimized-depletion-scaling.log`).
This establishes depletion traversal only; complete market/placement scaling remains open.

The initial live-supply release run passes 2,199 library tests (102 ignored), all three growth
integration scenarios and both 30-day continuations. Only the previously documented utility
machinery assertion fails (540 versus 96). Log: `/tmp/metrum-local-supply-all-tests.log`;
year/two-year/expansion/recovery reports: `/tmp/metrum-local-supply-*.txt`. Its saved-town year
retains 80 residents, 66 employed and all 15 operating-cash-positive firms; treasury is
726,289.9893540557. Report: `/tmp/metrum-local-supply-saved-city-365.txt`; log:
`/tmp/metrum-local-supply-saved-city-365-tests.log`. These results precede the search optimizations.

The initial matched, unprofiled 10k release comparison exposes a performance regression:
hourly 2.7328 → 13.295 ms, daily 187.57 → 388.12 ms (daily samples are highly variable).
Both runs use eight Rayon workers and CPUs 0,2,4,6,8,10,12,14, sequentially. Initial and final
population, household, job and building records match; final treasury differs by 1,024.90 after
the delivered-price dispatch change. Driver/identities/logs:
`/tmp/metrum-local-supply-perf.py`, `/tmp/metrum-local-supply-identities.txt`, and
`/tmp/metrum-local-supply-{baseline,after}-perf.log`. Individual distance pruning reduces the
hourly estimate to 6.0485 ms; retry replay alone adds no material improvement (5.9580 ms).
These are intermediate measurements, not acceptance. Logs:
`/tmp/metrum-local-supply-{pruned,retry}-perf.log`. The depleted-bucket benchmark performs
4,096 queries with one surviving supplier in 0.1787/0.2070/0.1358 ms at
1,024/8,192/65,536 suppliers, respectively (`/tmp/metrum-local-supply-depletion-scaling.log`).
That isolates availability traversal, not complete supply quotation or successful placement.

The original quotation-boundary tests exercise a mixed local-steel/imported-metals recipe whose
local quote pays operating costs while the import-only quote does not, separate export freight, unavailable input supply,
and input-stock valuation without double-counting self-produced output.

Fresh saved-town verification uses the same adapted input and user mods for 365 days. Its report
is byte-for-byte identical to the preceding pressure-removal build: 7 → 80 residents, 66 employed,
all 15 firms retained with positive operating cash, treasury 726,263.8699945625. Report/log:
`/tmp/metrum-input-quotes-saved-city-365.txt` and
`/tmp/metrum-input-quotes-saved-city-365-tests.log`. Command: `RAYON_NUM_THREADS=8
METRUM_DIAG_SAVE=/tmp/metrum-consumer-budget-input.sqlite METRUM_DIAG_DAYS=365` with the release
`diagnose_saved_city_growth --ignored --nocapture --test-threads=1` test and the same mods path.

Fresh release verification passes 2,184 library tests (99 ignored) and all three growth integration
scenarios. Only the pre-existing utility-machinery quantity assertion fails (540 versus 96).
Both 30-day cash continuations pass; year-one, year-two, scarce-labour expansion and grocery
recovery reports are byte-for-byte identical to the preceding pressure-removal build.
Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`;
log: `/tmp/metrum-input-quotes-tests.log`; reports:
`/tmp/metrum-input-quotes-{year,two-years,expansion,recovery}.txt`.
The four focused operating-cost tests also pass: `/tmp/metrum-input-quotes-operation-tests.log`.

Matched sequential unprofiled 10k release comparison uses unchanged configs and eight Rayon
workers pinned to CPUs `0,2,4,6,8,10,12,14`. Both builds match at 10,000 residents / 5,003
households / 6,791 employed / 13,881 jobs / 7,396 buildings, including starting and ending
treasury. Hourly estimates are 2.6982 / 2.5040 ms (−7.20%, p=0.03); daily estimates 191.87 /
183.72 ms show no significant change (p=0.94, wide intervals). No incremental regression is
observed in the existing import-priced path; this does not measure the future local-source
queries or establish full-redesign acceptance. Build: `cargo bench --manifest-path
rust/Cargo.toml --bench economy_tick_benchmark --no-run`; driver:
`python3 /tmp/metrum-input-quotes-perf.py`; logs:
`/tmp/metrum-input-quotes-{baseline,after}-perf.log`; executable/source/config hashes:
`/tmp/metrum-input-quotes-identities.txt`.

Outside freight pricing (2026-10-08): new businesses, alternative assets and level changes now
quote the proposed asset's actual door and frontage, using the same border sweeps and carrier
tariff as live hiring and dispatch. The flat dispatch fee alone no longer stands in for a route.
An unreachable outside route cannot fund imports/exports, while profiles that need no freight
still pay no carrier charge. Replacement checks quote both the incumbent and proposed door.
Queued execution requotes current frontage delay before owner equity is spent; quotes are derived
and are not saved. Rejected projects consume no workforce/customer claims or level-change credit.

Entrance derivation shares live building code and allocates no temporary building. Warm quotes
cost O(K log K + G), using the existing parcel index and border tables. Road topology/metric edits
invalidate network tables; building edits invalidate only indexed building quotes, preserving the
unchanged road sweeps across a construction queue. Border discovery reuses storage once per
hourly plan/execution snapshot. Local supplier prices/routes and affordable connected buyers still
need integration; this change repairs the outside-route component of project costing.

Fresh verification passes 14 focused freight tests (one ignored), then 2,181 library tests
(96 ignored) and all three growth integration scenarios. Only the pre-existing utility-machinery
quantity assertion fails. New regressions cover affordable/expensive physical routes, lost vehicle
access, a rejected lot leaving later opportunities available, replacement credit on freight-cost
rejection, and equality of proposed versus placed quotes. Both 30-day cash continuations pass.
Year one/two retain 19 individually operating-cash-positive firms; scarce-labour expansion adds
five firms above 99.806% lifetime staffing for 34,600 owner capital, with all 24 firms positive.
The original saved town reaches 80 residents / 66 employed after a year, with all 15 firms positive.
Commands: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib freight
-- --nocapture --test-threads=1`, then the full library/growth command below. Logs:
`/tmp/metrum-site-freight-tests.log`, `/tmp/metrum-site-freight-all-tests.log`; reports:
`/tmp/metrum-site-freight-{year,two-years,expansion,recovery,saved-city-365}.txt`.

The isolated warm entrance/quote benchmark takes 0.210 / 1.684 / 13.445 ms for 1,024 / 8,192 /
65,536 candidate quotes on two frontage lots and one gateway. Setup/sweeps are excluded; three
warmups precede nine samples. This measures the added quotation work, not a populated construction
queue or growth in network size. Run the release test executable with `RAYON_NUM_THREADS=8
taskset -c 0,2,4,6,8,10,12,14` and arguments `benchmark_demand_site_freight_quotes --ignored
--nocapture --test-threads=1`. Log: `/tmp/metrum-site-freight-active-perf.log`; executable/workload
hashes: `/tmp/metrum-site-freight-active-identities.txt`.

Matched 10k release performance uses unchanged demand/economy/zoning configuration, eight Rayon
workers and CPUs `0,2,4,6,8,10,12,14`, with sequential unprofiled executables. Both runs retain
10,036 residents / 5,023 households / 6,816 employed / 13,881 jobs / 7,396 buildings and identical
starting/ending treasury. Hourly estimates are 2.5918 / 2.6040 ms (p=0.86); daily estimates are
441.30 / 452.68 ms (p=0.91). No significant incremental timing change is detected. This fully
built workload does not establish active-queue or full-redesign performance acceptance.
Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
Driver: `python3 /tmp/metrum-site-freight-perf.py`; logs:
`/tmp/metrum-site-freight-{baseline,after}-perf.log`; build/config/source identities:
`/tmp/metrum-site-freight-identities.txt`.

Consumer purchasing forecast (2026-10-08): live hiring and project customer reservations now
share each household's recurring disposable income. Expected paid wages, net of income tax,
plus treasury-backed pensions and child support fund the authored consumer basket after housing
tax and utility costs. VAT consumes purchasing power without becoming business revenue. Each
family's demand is capped separately; wealth in one household cannot fund another's purchases.
Savings and time-limited unemployment benefits are not recurring customer revenue. Temporary
benefit claims still consume the shared treasury before later permanent claims, matching the
ordered transfer-settlement policy.

The income census reuses the existing member index: O(agents + households), deterministic member
order, parallel household sums, and retained scratch buffers. Market conversion is an
allocation-free O(households + profile ports) ordered pass because households share one finite
treasury. It runs once per hiring/planning snapshot, never per proposed project. Format 82 saves
the census at its observed phase; loading must not silently recompute wages from later assignments.
No compatibility fallback is added for old household rows. Buyer connectivity and local supplier
route/pricing remain separate unfinished work.

The bundled Kuopio terrain fixture's empty household table has the two new columns; all rows in
its 34 tables are unchanged. The older stalled-town diagnosis uses a temporary adapted copy,
`/tmp/metrum-consumer-budget-input.sqlite`, leaving the source save untouched. Its fresh year
reaches 80 residents / 66 employed, with 15 surviving firms individually operating-cash positive.
One extra `health_essentials_small` on parcel 34 nevertheless receives no staffing and closes
after 2,799 operating hours, losing its 3,465 owner capital. The capacity-reservation correction
below resolves this entry/hiring case; this earlier run is not healthy-growth acceptance. Evidence:
`/tmp/metrum-consumer-budget-saved-city-365.txt` and
`/tmp/metrum-consumer-budget-saved-city-365-tests.log`.

Fresh release verification passes 2,185 library tests (97 ignored) and all three growth scenarios;
only the pre-existing utility-machinery assertion fails (540 versus 96). Both 30-day framewise
reloads preserve sub-cent cash and exact discrete outcomes. Coverage checks cash-only versus
income-backed consumers in both hiring and construction, finite transfer claims, unpaid/inactive
assignments, and nonzero saved wage census fields. Year one/two retain all 19 individually
operating-cash-positive firms; scarce-labour expansion retains 24, and grocery recovery passes.
Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`. Log:
`/tmp/metrum-consumer-budget-final-tests.log`; reports:
`/tmp/metrum-consumer-budget-final-{year,two-years,expansion,recovery}.txt`.

Fresh isolated income-census/market timings are 0.027 / 0.087 / 0.609 ms for 1,024 / 8,192 /
65,536 households (two adults each), eight Rayon workers on CPUs `0,2,4,6,8,10,12,14`.
Minimal home/employer setup is excluded; three warmups precede nine samples. Command: the release
test executable with `benchmark_consumer_income_forecast --ignored --nocapture --test-threads=1`,
under the same worker/pinning settings. Log: `/tmp/metrum-consumer-budget-income-perf.log`.

The sequential unprofiled 10k comparison uses unchanged configuration and the same worker settings.
Before/after hourly estimates are 2.5234 / 2.8261 ms (+12.0%, p<0.05); daily estimates are
458.46 / 195.20 ms, with wide intervals. Outcomes differ: before has 10,036 residents / 5,023
households / 6,816 employed; after has 10,000 / 5,003 / 6,791. Buildings and physical jobs remain
7,396 / 13,881. Different admissions mean this does not establish matched-workload performance
acceptance, and the hourly increase remains open. Build: `cargo bench --manifest-path
rust/Cargo.toml --bench economy_tick_benchmark --no-run`; driver:
`python3 /tmp/metrum-consumer-budget-perf.py`; logs:
`/tmp/metrum-consumer-budget-{baseline,after}-perf.log`; executable/config/source hashes:
`/tmp/metrum-consumer-budget-identities.txt`. Full-redesign performance and active-queue scaling
remain unaccepted.

Pressure telemetry removal (2026-10-08): the remaining R/C/I floats, `pressure.rs`, unused
stock/cash normalization and import-dependency summaries are removed. They no longer influenced
admission, projects or the HUD. Format 83 drops the three saved pressure columns. Diagnostics
read concrete opportunities and eligibility directly; the cheat retains its explicit viability
bypass. Daily departures reuse the existing parallel household census and shared exit predicate,
O(H), without evaluating business markets or freight. Hourly snapshots also avoid household
reserve-day calculations used only by the deleted telemetry. No new per-agent allocation is added.
Seven tests asserting obsolete pressure values are removed; admission, reservation, exit,
project, save/load and worker-count determinism coverage remains.
Fresh 365-day saved-town replay is byte-for-byte identical to the preceding capacity-reservation
report: 7 → 80 residents, 66 employed, all 15 firms retained with positive operating cash, and
ending treasury 726,263.8699945625. It uses the same adapted diagnostic input and user mods.
Command: `RAYON_NUM_THREADS=8 METRUM_DIAG_SAVE=/tmp/metrum-consumer-budget-input.sqlite
METRUM_DIAG_DAYS=365` with `diagnose_saved_city_growth --ignored --nocapture --test-threads=1`;
report/log: `/tmp/metrum-pressure-removal-saved-city-365.txt` and
`/tmp/metrum-pressure-removal-saved-city-365-tests.log`.

Fresh final release verification passes 2,181 library tests (97 ignored) and all three growth
integration scenarios. Only the pre-existing utility-machinery quantity assertion fails
(540 versus 96). Both 30-day frame-by-frame cash continuations pass. Year-one, year-two,
scarce-labour expansion and grocery-recovery reports are byte-for-byte identical to the previous
build. Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release
--lib --test growth_scenario --no-fail-fast -- --test-threads=1`;
log: `/tmp/metrum-pressure-removal-final-tests.log`; reports:
`/tmp/metrum-pressure-removal-final-{year,two-years,expansion,recovery}.txt`.

Matched sequential unprofiled 10k release runs use eight Rayon workers pinned to CPUs
`0,2,4,6,8,10,12,14`. The baseline receives its two retired diagnostic constants; the driver
restores current configuration in a `finally` block. Both builds retain 10,000 residents /
5,003 households / 6,791 employed / 13,881 jobs / 7,396 buildings and identical starting/ending
treasury. Hourly estimates decrease from 2.8524 to 2.5685 ms (−9.95%, p<0.05); daily estimates
188.74 / 185.00 ms show no significant difference (p=0.98, wide intervals). This incremental
comparison does not establish active-queue scaling or full-redesign performance acceptance.
Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`;
driver: `python3 /tmp/metrum-pressure-removal-perf.py`; logs:
`/tmp/metrum-pressure-removal-{baseline,after}-perf.log`; executable/config/source hashes:
`/tmp/metrum-pressure-removal-identities.txt`.

Capacity reservation consistency (2026-10-08): hiring and construction now reserve the same
full net recipe capacity for a placed business. The previous construction snapshot reduced
service capacity to current throughput, briefly releasing all customers when construction
finished before the first employee arrived. The extra saved-town health shop was selected in
that gap; live hiring then divided its small market between both shops and could not fund the
new one's first worker. Healthy unstaffed shops retain their customers, while deserted/broken
shops release them for replacement. This is a per-business capacity correction, not a citywide
growth gate.

Construction, hiring and replacement credits share the existing physical work-area scale.
Replacement credits release net output rather than gross output, preserving neighbouring claims.
Self-produced maintenance no longer appears as external input demand after already being deducted
from saleable output. Snapshot aggregation remains O(B * P²), for buildings B and bounded recipe
ports P, within the existing parallel reduction; the changes add no scans or allocations per
project. Two unused activity fields are removed from the transient snapshot.

Fresh saved-town replay confirms the duplicate health shop is never built: the town still grows
from 7 to 80 residents / 66 employed, while all 15 observed firms remain open and individually
operating-cash positive over 365 days. The retained health shop is staffed for 8,710 of its 8,711
operating hours; the extra 3,465 owner grant is avoided. Report/log:
`/tmp/metrum-capacity-reservation-saved-city-365.txt` and
`/tmp/metrum-capacity-reservation-saved-city-365-tests.log`. This uses the same temporary adapted
input as the preceding run, without changing population, zoning, assets or balances.

Fresh final release verification passes 2,188 library tests (97 ignored) and all three growth
integration scenarios; only the pre-existing utility-machinery assertion fails. Both 30-day
cash continuations pass. The new regressions cover completion with no staff, partial/full
staffing, release after desertion, exact net replacement credit, and self-supplied maintenance.
Year one/two retain 19 individually operating-cash-positive firms. Forty-six added industrial
lots create five entrants above 99.84% lifetime staffing for 34,600 owner equity, with all 24
firms positive; grocery-failure recovery also passes. Command: `RAYON_NUM_THREADS=8 cargo test
--manifest-path rust/Cargo.toml --release --lib --test growth_scenario --no-fail-fast --
--test-threads=1`; log: `/tmp/metrum-capacity-reservation-final-tests.log`; reports:
`/tmp/metrum-capacity-reservation-final-{year,two-years,expansion,recovery}.txt`.

Matched sequential unprofiled 10k release runs use eight Rayon workers on CPUs
`0,2,4,6,8,10,12,14`, with unchanged configuration. Both builds retain 10,000 residents /
5,003 households / 6,791 employed / 13,881 jobs / 7,396 buildings and identical starting/ending
treasury. Before/after hourly estimates are 2.7324 / 2.7183 ms (p=0.85), daily 193.39 / 183.87 ms
(p=0.93, wide intervals). No significant incremental difference is detected. This does not close
the preceding income-forecast timing concern or establish active-queue/full-redesign scaling.
Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`;
driver: `python3 /tmp/metrum-capacity-reservation-perf.py`; logs:
`/tmp/metrum-capacity-reservation-{baseline,after}-perf.log`; executable/config/source hashes:
`/tmp/metrum-capacity-reservation-identities.txt`.

The global recovered-startup-cash gate is removed. Each factory can serve outside buyers while
another factory is still constructing or recovering its capital; workforce and local customers
remain finite shared claims. Commerce orders categories by unmet recurring sales value, avoiding
both unit-size bias and temporary pantry restocking that previously concealed small services.
Spawn percentages no longer multiply viable construction need; physical execution remains one
queued action per minute. Rejected preferred assets now fall back to viable alternatives on the
same lot, using current shared customers and workforce; rejected options reserve nothing. A filled
commercial category therefore leaves later lots available to other categories. Non-residential
selection evaluates every legal lot instead of applying the average-output estimate or the
one-project export quota. Prepared profile assets are shared by initial parallel lot collection
and fallback; commercial ranking checks each alternative once, preserving category priority and
deterministic family/variant hashes. With P lots, A assets per profile, C commercial categories,
K resource ports and H reservation-calendar size, fallback is bounded by
O(P*A*(C*K + K² + log H)), plus existing indexed site checks, without a city scan or a copied
alternative list per lot. Reservation-changing selection remains serial because projects share
customers and labor.
Still open: remaining building pressure/credit state and UI explanations from actual decisions.
Home capacity remains a prospective worker opportunity;
actual arrivals require affordable live wages. Full redesign acceptance remains pending.

Alternative-selection verification (2026-10-07): the release library run with eight Rayon workers
and one test thread passes 2,165 tests, 95 ignored. All four new regressions pass: an unprofitable
priority category does not hide a viable service; reserved customers release later lots to another
category; a four-worker factory replaces an infeasible ten-worker proposal on the same lot; and
separate export projects consume separate worker claims without a one-project quota. The two
cheat-mode checks now expect every eligible lot rather than the deleted quota. The existing
utility-machinery quantity failure and queued cash-continuation failure occurred in that run
(the latter first exceeded cent tolerance at day 5); the continuation repair below supersedes
that failure. No other library failures occurred. All three
growth integration tests pass. The starter town sustains 154 residents / 115 employed through
days 366 and 731, with positive cumulative operating cash and no bankruptcy/removal at each of
its 19 firms. Adding 46 industrial lots with scarce labor creates five machinery firms; minimum
entrant lifetime staffing is 99.8067%, and all 24 observed firms remain operating-cash positive.
Owner equity is 34,600, below 38,060 from 22 observed additional available workers. Reports:
`/tmp/metrum-demand-alternatives-{year,two-years,expansion}.txt`; full command/log:
`RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`,
`/tmp/metrum-demand-alternatives-all-tests.log`. Report paths were supplied using
`METRUM_GROWTH_REPORT`, `METRUM_GROWTH_LONG_REPORT` and `METRUM_GROWTH_EXPANSION_REPORT`.

Selection-revision performance (2026-10-07): sequential unprofiled release executables,
`RAYON_NUM_THREADS=8`, CPUs `0,2,4,6,8,10,12,14`, unchanged demand/economy config. Both runs
match at 10,036 residents / 5,023 households / 6,816 employed / 13,881 jobs / 7,396 buildings,
including starting/ending treasury. With `CRITERION_HOME=/tmp/metrum-demand-alternatives-perf`,
run `/tmp/metrum-demand-cleanup-bench --bench /10k --save-baseline cleanup`, then
`/tmp/metrum-demand-alternatives-bench --bench /10k --baseline cleanup`. Hourly estimates are
2.4479 / 2.4643 ms (p=0.76); daily 453.97 / 462.69 ms (p=0.93). Neither change is statistically
significant. This fully built fixture does not measure a large set of active construction options
or establish full-redesign performance. Driver: `/tmp/metrum-demand-alternatives-perf.py`;
logs: `/tmp/metrum-demand-alternatives-{baseline,after}-perf.log`; executable/config/source hashes:
`/tmp/metrum-demand-alternatives-identities.txt`.

The admission cleanup removes the hypothetical household preview, search-runway calculation,
marginal shop job forecast, regional migration multiplier and 600-household cutoff, repeated
distress penalties, and their unused configuration. The remaining daily household cap is named
`max_household_removals_per_day`; it never limits admission. Actual applicants alone claim recurring
income. Housing need uses remaining live vacancies at one incoming adult per home; the workforce
calendar separately pairs homes with funded constructing/proposed workplaces. Admission reporting
now contains real capacity, examined applicants, income/housing rejections and launched carriers.
There is no admission pressure percentage or saved admission credit. Save format 74 removes
`admission_action_credit` and `recent_household_failure_pressure`; older saves' extra columns do
not enter the explicit read query. The deleted marginal forecast also removes an O(B*ports)
commercial-building scan from each snapshot; no new per-applicant allocation or city scan is added.
Remaining building upgrade/downgrade/clearance pressure and display thresholds are separate work.

Admission-cleanup verification (2026-10-07): `RAYON_NUM_THREADS=8`, release build and one test
thread. The focused `simulation::economy::demand::` run passes 80 tests, 2 ignored. The full
library run passes 2,161 tests, 95 ignored, with the same pre-existing utility-machinery quantity
failure. The smaller test count removes assertions of the deleted marginal/pressure calculator;
replacement coverage checks actual batch claims, pension coverage, current resident transfer
claims, rejection without reservation, and affordable entry independent of city size/distress.
The scarce-labour expansion remains unchanged at 46 new lots / three profitable, staffed entrants /
26,940 equity. Logs: `/tmp/metrum-demand-cleanup-demand-tests.log`,
`/tmp/metrum-demand-cleanup-all-tests.log`; expansion report:
`/tmp/metrum-demand-cleanup-expansion.txt`. All three growth scenarios also pass. A separate
365-day replay of the unchanged saved town retains 85 residents / 69 employed and positive
cumulative operating cash at all 14 firms; report/log:
`/tmp/metrum-demand-cleanup-saved-city-365.txt`,
`/tmp/metrum-demand-cleanup-saved-city-365-tests.log`.

Upgrade reservations (2026-10-08): instant nonresidential level changes previously bypassed the
workforce and customer reservations shared by new construction. Before removing their remaining
pressure gates, selection now checks the target operation against its own market and claims any
extra workforce from ready supply. Assigned workers and already funded incumbent vacancies remain
reserved for that employer; they are not claimed twice. Homes still under construction cannot
staff an immediate upgrade. A rejected proposal leaves both customers and workers unchanged, keeps
its pacing credit, and does not prevent a later viable candidate from being examined.

A read-only replacement-market view releases only the incumbent's exact snapshot output contribution
while pricing the target. It reuses the same operating-cost calculation as independent entry,
including the target level's property bill and at least the wages of all retained staff. Selected
replacements update shared resource commitments before the next project. Rejected quotes do not
subtract and restore floating-point totals. Snapshot activity factors reuse an already computed
resource vector; no per-proposal allocation or additional city scan is introduced. Incremental
work is O(P² + log H) per candidate for bounded profile ports P and the existing workforce calendar H.
The focused full-planner test covers competing upgrades, funded incumbent vacancies, assigned but
unfunded staff, future homes and credit retention; it passes. A separate regression verifies that
repeated rejected replacement quotes leave shared customer totals bit-for-bit unchanged and cannot
reuse a competitor's commitment.

Fresh release verification passes 2,173 library tests (94 ignored) and all three growth integration
tests. Both 30-day cash continuations, the two-year starter town, scarce-labour rezoning and grocery
recovery pass. The only failure remains the pre-existing utility-machinery quantity assertion.
Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`; log:
`/tmp/metrum-upgrade-reservations-all-tests.log`; reports:
`/tmp/metrum-upgrade-reservations-{year,two-years,expansion,recovery}.txt`.
A fresh 365-day replay of the original stalled save matches the preceding exit-cleanup build at
80 residents / 66 employed, with all 15 firms operating-cash positive and none bankrupt or removed.
Run the saved-town diagnostic command below with `METRUM_DIAG_REPORT=/tmp/metrum-upgrade-reservations-saved-city-365.txt`;
log: `/tmp/metrum-upgrade-reservations-saved-city-365-tests.log`.

Matched incremental performance compares the preceding clearance build against household-exit
cleanup plus upgrade reservations, sequentially in unprofiled release executables with eight Rayon
workers on CPUs `0,2,4,6,8,10,12,14`. Both fixtures match at 10,036 residents / 5,023 households /
6,816 employed / 13,881 jobs / 7,396 buildings, including initial and final treasury. Hourly estimates
are 2.5001 / 2.6207 ms (+4.8%, p=0.10); daily estimates are 445.58 / 445.24 ms (p=0.99).
Neither difference is statistically significant; the hourly estimate is not evidence of zero cost.
This fully built fixture does not establish active-upgrade or queued-project scaling, and does not
close the earlier HUD timing concern or full-redesign performance acceptance.

Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
With `RAYON_NUM_THREADS=8 CRITERION_HOME=/tmp/metrum-upgrade-reservations-perf`, run
`taskset -c 0,2,4,6,8,10,12,14 /tmp/metrum-clearance-bench --bench /10k
--save-baseline before_exits_and_upgrade_reservations`, then
`taskset -c 0,2,4,6,8,10,12,14 /tmp/metrum-upgrade-reservations-bench --bench /10k
--baseline before_exits_and_upgrade_reservations`. Driver:
`/tmp/metrum-upgrade-reservations-perf.py`; logs:
`/tmp/metrum-upgrade-reservations-{baseline,after}-perf.log`; executable/config/source hashes:
`/tmp/metrum-upgrade-reservations-identities.txt`. Each executable uses its corresponding registry
schema; the driver restores the current registry in `finally` and verifies restoration.

Upgrade pressure removal (2026-10-08): viable upgrades no longer need citywide pressure to cross
a threshold. Eligible candidates earn pacing credit directly; rejected economic proposals retain
that credit. Format 80 removes saved upgrade hysteresis and the registry removes its thresholds.
Downgrades still use pressure and remain separate cleanup work.

Target operations must cover recurring costs and working capital. Upgrades and live hiring now
share the same cash, usable inventory and paid-cargo valuation, counting input/output overlap only
once. Paid cargo is credited only when the recipe is unchanged. The old four/eight-day cash-only
buffer is removed: a firm that already bought its inputs should not need their value again in cash.
Stock valuation visits bounded recipe ports, O(P²), without allocating or scanning shipments per
candidate. The existing indexed hiring forecast supplies paid cargo in O(1). Regressions cover
zero citywide pressure, no capital, cash funding, purchased input buffers and in-flight cargo;
capital cannot replace recurring income or shared workforce/customer reservations.

Fresh release verification passes 2,174 library tests (94 ignored) and all three growth integration
tests. Both 30-day frame-by-frame cash continuations pass, with less than 0.01 cash difference and
exact discrete outcomes. The sole failure is the existing utility-machinery quantity assertion.
Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`; log:
`/tmp/metrum-upgrade-pressure-all-tests.log`; reports:
`/tmp/metrum-upgrade-pressure-{year,two-years,expansion,recovery}.txt`.
Scarce-labour rezoning retains five staffed, individually profitable entrants and 34,600 owner
equity within the 38,060 workforce bound. A fresh 365-day replay of the original stalled town
retains 80 residents / 66 employed and all 15 firms with positive operating cash, none bankrupt or
removed. Diagnostic report/log: `/tmp/metrum-upgrade-pressure-saved-city-365.txt` and
`/tmp/metrum-upgrade-pressure-saved-city-365-tests.log`.

Incremental performance uses matched, sequential unprofiled release executables, eight Rayon workers
and CPUs `0,2,4,6,8,10,12,14`. Both builds retain identical initial/final treasury and 10,036 residents /
5,023 households / 6,816 employed / 13,881 jobs / 7,396 buildings. The first pair reports hourly
2.4601 / 4.3259 ms (+75.8%, p<0.05), daily 447.05 / 461.08 ms (p=0.89). Repeating in reversed
execution order gives before/after hourly 2.4968 / 2.5151 ms (p=0.77), daily 442.99 / 459.40 ms
(p=0.88), neither significant. The first slowdown is not reproduced; both measurements remain
recorded. This fully built fixture does not establish active-upgrade or queued-project scaling,
and full-redesign performance acceptance remains open.

Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
Drivers: `python3 /tmp/metrum-upgrade-pressure-perf.py` and
`python3 /tmp/metrum-upgrade-pressure-reverse-perf.py`; each runs pinned executables with
`--bench /10k` and a separate Criterion baseline under `/tmp`. Baseline executable:
`/tmp/metrum-upgrade-reservations-bench`; current: `/tmp/metrum-upgrade-pressure-bench`.
Logs: `/tmp/metrum-upgrade-pressure-{baseline,after}-perf.log` and
`/tmp/metrum-upgrade-pressure-reverse-{baseline,after}-perf.log`. Build/source/config hashes:
`/tmp/metrum-upgrade-pressure-identities.txt` and
`/tmp/metrum-upgrade-pressure-reverse-identities.txt`. Both drivers supply each executable's
demand/economy registry schema and restore the current files in `finally`, checking restoration.

Business downgrade viability (2026-10-08): low cash, staffing or stock previously selected a
smaller tier without checking the replacement's economics. Selection now shares the upgrade
replacement quote and reservation path. A supported incumbent remains unchanged; an unsupported
one may downsize only if the replacement covers recurring costs and working capital, retains
assigned staff, and claims any added ready workforce and customers. Rejection spends neither
claims nor pacing credit and does not hide later candidates. The extra current-operation quote is
O(P²) over bounded profile ports; successful worker claims retain O(log H), without per-quote
allocation or a new city scan. Utility prices remain prepared once per planning pass.

The focused full-planner regression passes eight cases: supported incumbents, paid input stock,
recovery through lower tier costs, shared workers, future homes, missing working capital, missing
customers, and competing recipe changes claiming the same customers. Command:
`RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
business_downgrades_require_recovery -- --nocapture --test-threads=1`; log:
`/tmp/metrum-downgrade-viability-focused.log`.

Fresh release verification passes 2,175 library tests (94 ignored) and all three growth integration
tests, including both 30-day cash continuations. The sole failure remains the pre-existing
utility-machinery quantity assertion (540 versus 96). Two years retain 154 residents / 115 employed
and all 19 firms with positive operating cash; scarce-labour expansion retains 24 individually
positive firms. Grocery-failure recovery also passes. Command: `RAYON_NUM_THREADS=8 cargo test
--manifest-path rust/Cargo.toml --release --lib --test growth_scenario --no-fail-fast --
--test-threads=1`; log: `/tmp/metrum-downgrade-viability-all-tests.log`; reports:
`/tmp/metrum-downgrade-viability-{year,two-years,expansion,recovery}.txt`.
The fresh saved-town year retains 80 residents / 66 employed and all 15 firms with positive
operating cash, none bankrupt or removed. Report/log:
`/tmp/metrum-downgrade-viability-saved-city-365.txt` and
`/tmp/metrum-downgrade-viability-saved-city-365-tests.log`.

Matched performance uses sequential unprofiled release executables, eight Rayon workers, CPUs
`0,2,4,6,8,10,12,14`, and unchanged demand/economy configuration. Both builds match at 10,036
residents / 5,023 households / 6,816 employed / 13,881 jobs / 7,396 buildings, including initial and
final treasury. The first comparison records hourly 2.5620 / 2.9808 ms (+16.3%, p<0.05), daily
450.41 / 452.65 ms (p=0.98). Reversing execution order records before/after hourly 2.3946 / 2.4999 ms
(+4.4%, p=0.14), daily 455.95 / 448.63 ms (p=0.94). The larger increase is not reproduced, but the
hourly concern remains open. The fixture has no level-change families and does not measure active
downgrade or upgrade scaling; it checks the surrounding hourly/daily path only.

Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
Drivers: `python3 /tmp/metrum-downgrade-viability-perf.py` and
`python3 /tmp/metrum-downgrade-viability-reverse-perf.py`; each runs pinned `--bench /10k`
executables against separate Criterion baselines. Before: `/tmp/metrum-upgrade-pressure-bench`;
after: `/tmp/metrum-downgrade-viability-bench`. Logs:
`/tmp/metrum-downgrade-viability-{baseline,after}-perf.log` and
`/tmp/metrum-downgrade-viability-reverse-{baseline,after}-perf.log`; build/source/config hashes:
`/tmp/metrum-downgrade-viability-identities.txt` and
`/tmp/metrum-downgrade-viability-reverse-identities.txt`.

Downgrade pressure removal (2026-10-08): all level changes now use direct project checks and
explicit pacing. The per-density threshold registry, zoning growth-profile references, transient
cash/stock/staffing downgrade triggers and saved hysteresis are removed (format 81). Legacy RCI
pressure remains diagnostic telemetry only. A compatible lower tier no longer hides a viable
upgrade, and a selected upgrade excludes that building from downsizing in the same pass.

Residential downsizing runs after new workplaces, upgrades and business replacements have claimed
workers. It retires only unreserved ready homes, protecting the shared vacancy reserve, unhoused
households and the first incoming household. Multiple selected homes debit the same remaining
surplus. Local unemployed adults and unfinished homes cannot substitute for ready housing being
removed. Independent discovery remains a parallel O(B) pass, returning at most two level candidates
per building. Shared selection remains ordered: O(P² + log H + log U) per business proposal and
O(log H + log U) per residential proposal, with bounded profile ports P, completion bins H and
selected upgrades U. There is no new spatial scan or per-reservation allocation. Cheat growth
explicitly disables downsizing instead of relying on a maximum pressure value to do so.

Fresh verification passes 90 focused demand tests (one ignored), then 2,178 library tests (94
ignored) and all three growth scenarios. Both 30-day cash continuations and the zoning registry
checks pass; only the pre-existing utility-machinery quantity assertion fails. New planner coverage
protects housing for live jobs, queued/active construction and newly selected workplaces, including
multiple surplus claims, unhoused residents and disconnected towns. It checks zero and maximum
citywide pressure and a family with both lower and higher tiers.
Commands: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
simulation::economy::demand:: -- --nocapture --test-threads=1`, then the full release command above.
Logs: `/tmp/metrum-downgrade-pressure-demand-tests.log`,
`/tmp/metrum-downgrade-pressure-all-tests.log`; reports:
`/tmp/metrum-downgrade-pressure-{year,two-years,expansion,recovery}.txt`.
The original saved town again reaches 80 residents / 66 employed after a year; report/log:
`/tmp/metrum-downgrade-pressure-saved-city-365.txt` and
`/tmp/metrum-downgrade-pressure-saved-city-365-tests.log`.

Fresh downgrade-cleanup performance uses sequential unprofiled release runs, eight Rayon workers
on CPUs `0,2,4,6,8,10,12,14`. Both 10k runs retain identical population, staffing, buildings and
treasury. Hourly estimates are 2.5315 / 2.5052 ms (p=0.71), and daily estimates are 453.82 /
439.55 ms (p=0.89); this incremental comparison detects no significant change. Build with
`cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`, then run
`python3 /tmp/metrum-downgrade-pressure-perf.py`. The driver temporarily supplies each build's
authored demand/economy/zoning registries and verifies restoration of all three current files.
Logs: `/tmp/metrum-downgrade-pressure-{baseline,after}-perf.log`; executable, config and source
hashes: `/tmp/metrum-downgrade-pressure-identities.txt`.

A separate active-level-change fixture, added after the full correctness run, passes with every
eligible business and home selected. Median planning times are 0.577 / 4.173 / 30.727 ms for
2,048 / 16,384 / 131,072 buildings. Setup is outside timing; each size uses three warmups and
nine samples of three passes, including action-buffer reset and shared reservation checks.
This measures a forced simultaneous downsizing burst, not normal pacing or active construction
queues, and has no matched pre-change timing. Build with `RAYON_NUM_THREADS=8 cargo test
--manifest-path rust/Cargo.toml --release --lib benchmark_active_level_change_planning --no-run`,
then run the resulting test executable with `RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14`
and arguments `benchmark_active_level_change_planning --ignored --nocapture --test-threads=1`.
Log: `/tmp/metrum-downgrade-pressure-active-perf.log`; workload/build identities:
`/tmp/metrum-downgrade-pressure-active-identities.txt`.

Actual site freight quotes, local buyer affordability/connectivity, legacy telemetry cleanup and
full-redesign performance acceptance remain open.

Household-exit cleanup (2026-10-07): planning previously counted long-unhoused eligible households,
but execution ranked every household and could remove a newly displaced or housed family instead.
A separate citywide crisis ratio bypassed the individual grace period. Both now use one shared
`HouseholdExitPolicy`, with live execution revalidation and the existing explicit daily pacing.
Format 79 drops crisis-removal credit and the registry drops its threshold. The parallel O(H)
eligibility pass reuses candidate storage and sorts only E eligible households in O(E log E),
replacing multiple allocated lists and ranking of the entire population. Existing swap-remove,
shopping, freight-carrier and survivor remapping paths remain the execution boundary. Remaining
upgrade/downgrade pressure is still open.

Verification: the full release run passes 2,170 library tests (94 ignored), both cash continuations
and all three growth scenarios. Its failures were the existing utility quantity assertion and a new
fixture assertion that confused reserve days with currency. After correcting only that fixture,
all 17 housing tests pass (one benchmark ignored), including eligibility rechecks, housed-family
protection, batched ledger/member repair and freight-carrier remapping. The city-size-invariance
and newly-displaced-household regressions also pass in the full run. Production code did not change
after that run. Commands: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release
--lib --test growth_scenario --no-fail-fast -- --test-threads=1`, then `RAYON_NUM_THREADS=8 cargo test
--manifest-path rust/Cargo.toml --release --lib simulation::economy::households::tests::housing
-- --test-threads=1`; logs: `/tmp/metrum-exits-tests.log`, `/tmp/metrum-exits-housing-tests.log`.
Year one/two retain 154 residents and 115 employed. Scarce-labour expansion retains all 24 firms with
positive individual operating cash; the 180-day grocery-recovery regression also passes. Reports:
`/tmp/metrum-exits-{year,two-years,expansion,recovery}.txt`.

Matched departure measurements use sequential unprofiled release executables, eight Rayon workers,
CPUs `0,2,4,6,8,10,12,14`, seven samples per size, one agent per household and 256 departures. Setup
is excluded. The new fixture marks every unhoused household's grace period expired so both versions
remove the same pool; the old executor ignored that age. Final assertions verify the actual removed,
remaining-household and remaining-agent counts outside timing. Median before/after milliseconds:
1,024 households 0.088 / 0.061; 8,192 households 0.514 / 0.158; 65,536 households 3.688 / 0.862.
This measures eligible selection, removal and reference repair with no freight orders; it does not
establish full-city or active-construction scaling. Commands: `RAYON_NUM_THREADS=8 taskset -c
0,2,4,6,8,10,12,14 /tmp/metrum-exits-baseline-tests benchmark_household_removal --ignored --nocapture
--test-threads=1`, then the same command with `/tmp/metrum-exits-after-tests`. Logs:
`/tmp/metrum-exits-{baseline,after}-removal.log`; executable/config/source hashes:
`/tmp/metrum-exits-identities.txt`.

A fresh 365-day continuation of the unchanged original stalled save grows from 7 to 80 residents,
with 66 employed and all 15 observed firms earning positive cumulative operating cash; none is
bankrupt or removed. This supersedes the earlier build's 85-resident/14-firm result as evidence for
the current build. The diagnostic resumes the saved clock without changing zoning, balances or
staffing. Command: `RAYON_NUM_THREADS=8 METRUM_DIAG_SAVE=/tmp/metrum-demand-economy-input.sqlite
METRUM_DIAG_MODS="$HOME/.local/share/godot/app_userdata/Metrum Rise/mods"
METRUM_DIAG_REPORT=/tmp/metrum-exits-saved-city-365.txt METRUM_DIAG_DAYS=365
rust/target/release/deps/metrum_rise-73d155d1f861cb8b diagnose_saved_city_growth --ignored
--nocapture --test-threads=1`; log: `/tmp/metrum-exits-saved-city-365-tests.log`.
This establishes growth and individual firm cash for that town, not full-redesign acceptance.

Failed-site clearance (2026-10-07): the remaining contraction path could remove a healthy empty
building at low pressure while stranding a deserted home at high pressure. Clearance now checks
actual abandonment and empty occupancy/staffing for every private use, independently of citywide
signals. Growth-profile lookup no longer gates failed-site recovery. The existing explicit
half-per-day clearance cadence is retained; it paces reclamation instead of approving it. Execution
rechecks the same lifecycle state before removal. Format 78 removes despawn hysteresis, and the
shipped registry no longer contains despawn thresholds. The existing parallel building pass and
attachment sort retain their complexity; an unnecessary normalized-pressure reduction is removed.
Upgrade/downgrade pressure and per-household exit cleanup remain separate work.

The grocery-failure scenario exposed a second reservation defect: an employer losing job funding
made its still-assigned workers appear unemployed. With 12 grocery staff still employed, the next
hour reported 13 unemployed adults instead of one and selected three factories. The grocery still
reserved its customers until daily bankruptcy, so a replacement could not compete for that labour;
180 days later no grocery had reopened. Trace: `/tmp/metrum-clearance-recovery-trace.txt`; failing
report: `/tmp/metrum-clearance-recovery.txt`. Snapshot employment now counts actual assignments
independently of funded capacity. Funding still limits advertised vacancies and admission income;
it does not release a worker before employment ends. This changes only the existing building
accumulator, with no new pass, per-worker allocation or forecast. The temporary trace hook was
removed after diagnosis; the 180-day recovery assertion remains a permanent regression.

The recovery scenario now passes: the original grocery goes bankrupt and clears; one grocery
opens on another already-zoned lot within 31 hours of the loss. Over 180 observed days it records
4,289 operating hours, 51,468 / 53,611 staffed/capacity hours (96.0%), 117,223.30 operating cash earned
and 18,450 owner equity. The town retains 154 residents and 115 employed, with no rezoning or player
intervention. Report: `/tmp/metrum-clearance-recovery-fixed.txt`. The unit regression separately
checks that funding falling below an assigned workforce changes vacancies but never invents
unemployed adults; clearance covers all three zones at low, neutral and high pressure, and execution
rejects recovered, occupied, staffed or constructing sites.

Fresh release verification passes 2,171 library tests (94 ignored) and all three growth integration
tests, including both 30-day save continuations. The only failure remains the pre-existing
utility-machinery quantity assertion (540 versus 96). Command: `RAYON_NUM_THREADS=8 cargo test
--manifest-path rust/Cargo.toml --release --lib --test growth_scenario --no-fail-fast
-- --test-threads=1`; log: `/tmp/metrum-clearance-fixed-tests.log`; reports:
`/tmp/metrum-clearance-fixed-{year,two-years,expansion}.txt`.

The combined HUD/clearance revision was compared with the pre-HUD capacity-cleanup executable,
sequentially in unprofiled release builds, using eight Rayon workers on CPUs
`0,2,4,6,8,10,12,14`. Both runs match at 10,036 residents / 5,023 households / 6,816 employed /
13,881 jobs / 7,396 buildings, including initial and final treasury. Hourly estimates are
2.4115 / 2.4981 ms (p=0.22); daily estimates are 443.57 / 444.35 ms (p=1.00).
This run detects no significant difference; it does not establish that the earlier smaller HUD
regression is eliminated, or validate active-queue scaling and full-redesign performance.

Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
With `RAYON_NUM_THREADS=8 CRITERION_HOME=/tmp/metrum-clearance-perf`, run
`taskset -c 0,2,4,6,8,10,12,14 /tmp/metrum-housing-capacity-bench --bench /10k
--save-baseline before_clearance_and_display`, then
`taskset -c 0,2,4,6,8,10,12,14 /tmp/metrum-clearance-bench --bench /10k
--baseline before_clearance_and_display`. Driver: `/tmp/metrum-clearance-perf.py`; logs:
`/tmp/metrum-clearance-{baseline,after}-perf.log`; build/config/source hashes:
`/tmp/metrum-clearance-identities.txt`. The driver supplies each executable its corresponding
registry schema and restores the current registry afterward.

Growth HUD and spawn-pressure removal (2026-10-07): `get_growth_overview` now exposes the last
hourly assessment's exact selected/queued/constructing counts and concrete reasons. The HUD uses
those counts on a shared bar scale and provides per-zone tooltips for homes needed, ready housing,
live funded jobs, missing assets, free/unsuitable lots, and customer/income/workforce constraints.
Rejection counts describe the preferred proposal after no viable alternative was found; they do
not claim that all asset alternatives failed for the same reason. Creation/load explicitly waits
for the first hourly assessment. No additional simulation decision is performed by the UI.

Availability counters join the existing Rayon parcel discovery/reduction; construction counters
join the existing building snapshot and queue reservation walk. Complexity remains O(P) discovery
plus its existing candidate sort and O(B + Q) snapshot/reservation work. The bridge formats three
fixed-size summaries, O(1) in city size. Removed spawn thresholds/hysteresis no longer require the
per-candidate density string allocation or normalized-pressure pass. Format 77 removes saved spawn
hysteresis. Remaining lifecycle pressure governs upgrades, downgrades, clearance and exits; it is
no longer presented as independent construction demand. That lifecycle simplification, actual-site
freight/customer evaluation and full-redesign acceptance remain open.

Fresh display-cleanup verification passes 2,167 library tests (94 ignored) and all three growth
scenarios, with only the pre-existing utility-machinery quantity failure. Both save/reload
continuations pass. The year-one/year-two outcomes remain 154 residents / 115 employed and 19
individually operating-cash-positive firms. Scarce-labour rezoning still adds five factories above
99.806% lifetime staffing, spends 34,600 owner equity, and leaves all 24 observed firms with positive
operating cash. Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release
--lib --test growth_scenario --no-fail-fast -- --test-threads=1`; log:
`/tmp/metrum-growth-display-tests.log`; reports: `/tmp/metrum-growth-display-{year,two-years,expansion}.txt`.

`cargo check --tests`, `cargo build --release` and the benchmark build pass. An isolated Godot
project runs `--headless --script res://tests/growth_meter_test.gd`: no failures. The same test
renders successfully under Xvfb/OpenGL compatibility and the meter image was inspected. It checks
the bridge, exact counts, proportional bars, zero visibility, constraint tooltips and compact
layout. `--check-only --script` passes for main_ui, top_menu and input_manager. Logs:
`/tmp/metrum-growth-display-{check,build,godot,render,gdcheck}.log`; image:
`/tmp/metrum-growth-display.png`; isolated project:
`/tmp/metrum-growth-display-godot-6dta4qck/project`. Rendering required running outside the sandbox
because Xvfb could not bind a display socket inside it. The player profile and deployed extension
were not changed; this is an isolated widget/bridge check, not full-game visual acceptance.

Display-cleanup performance remains under investigation. Sequential unprofiled release runs
use eight Rayon workers and CPUs `0,2,4,6,8,10,12,14`. Both executables match at 10,036 residents /
5,023 households / 6,816 employed / 13,881 jobs / 7,396 buildings and identical initial/final
treasury. The first old/new pair measures 2.5525 / 3.2587 ms hourly (+27.7%, p=0.01, three high
outliers) and 457.77 / 474.93 ms daily (p=0.88). A reversed-order repeat measures 2.3594 / 2.4854 ms
hourly in old/new order: +0.126 ms, about +5.3%. The repeat's Criterion comparison reports the old
binary 5.07% faster than the new (p=0.01). Daily is 453.98 / 461.08 ms (p=0.95). The larger spike
is not reproduced at that magnitude, but a smaller hourly cost remains; this is not a claim of
performance equivalence. Full acceptance and active-candidate/queue scaling remain open.

Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
Drivers `/tmp/metrum-growth-display-perf.py` and `/tmp/metrum-growth-display-repeat-perf.py` invoke
`taskset -c 0,2,4,6,8,10,12,14 <executable> --bench /10k` with `--save-baseline before_growth_display`
for the first executable and `--baseline before_growth_display` for the second. Executables are
`/tmp/metrum-housing-capacity-bench` (old) and `/tmp/metrum-growth-display-bench` (new). They use
separate Criterion directories and the old binary's exact preceding demand config, restoring the
current config in `finally` and checking it afterward; only the deleted spawn thresholds differ.
Logs: `/tmp/metrum-growth-display-{baseline,after}-perf.log` and
`/tmp/metrum-growth-display-repeat-{baseline,after}-perf.log`. Executable/config/source identities:
`/tmp/metrum-growth-display-identities.txt` and `/tmp/metrum-growth-display-repeat-identities.txt`.

Housing selection cleanup (2026-10-07): missing household places are covered by the actual
capacity of each selected home, in attachment order. The average-size estimate could select too
few small homes or unnecessary large ones when candidate sizes differed. There is no accumulated
spawn credit or business building-count estimate. Unselected homes remain offers for viable
workplaces; already queued/constructing capacity is deducted before selection. The selection pass
is O(candidates), with buffers reserved before iteration and no new city scan. Format 76 removes
the three saved spawn-credit columns. Scenario reports now name actual customer, income, workforce
and placement constraints, rather than obsolete pressure thresholds or spawn-credit gates.

Fresh capacity-cleanup verification passes 2,165 library tests (94 ignored) and all three growth
integration tests; the sole failure remains the pre-existing utility-machinery quantity assertion.
Mixed home sizes cover both under-selection and over-selection, while pending-capacity, pantry
recovery and resource reservations now assert actual decisions instead of the deleted estimates.
Both 30-day save continuations pass. Year-one and year-two runs retain 154 residents / 115 employed
and all 19 firms with positive individual operating cash. Forty-six new industrial lots produce
five entrants above 99.806% lifetime staffing; all 24 observed firms remain operating-cash positive,
and owner equity is 34,600 against the 38,060 workforce bound. Command: `RAYON_NUM_THREADS=8
cargo test --manifest-path rust/Cargo.toml --release --lib --test growth_scenario --no-fail-fast
-- --test-threads=1`; log: `/tmp/metrum-housing-capacity-tests.log`; reports:
`/tmp/metrum-housing-capacity-{year,two-years,expansion}.txt`. Four fewer active tests and one fewer
ignored benchmark remove coverage of obsolete estimators/credits, with direct selection coverage
added in their place.

Capacity-cleanup performance: matched sequential, unprofiled release runs use eight Rayon
workers on CPUs `0,2,4,6,8,10,12,14`, with unchanged demand/economy configs. Both match at 10,036
residents / 5,023 households / 6,816 employed / 13,881 jobs / 7,396 buildings and identical initial
and final treasury. Hourly estimates are 2.6171 / 2.4723 ms (p=0.03, within the fixture's noise
threshold); daily estimates are 464.16 / 451.23 ms (p=0.90). No regression is detected. This fully
built fixture does not establish active-construction or optional-home scaling.

Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
With `RAYON_NUM_THREADS=8 CRITERION_HOME=/tmp/metrum-housing-capacity-perf`, run
`taskset -c 0,2,4,6,8,10,12,14 /tmp/metrum-reload-cached-bench --bench /10k
--save-baseline before_housing_capacity`, then
`taskset -c 0,2,4,6,8,10,12,14 /tmp/metrum-housing-capacity-bench --bench /10k
--baseline before_housing_capacity`. Driver: `/tmp/metrum-housing-capacity-perf.py`; logs:
`/tmp/metrum-housing-capacity-{baseline,after}-perf.log`; executable/config/source hashes:
`/tmp/metrum-housing-capacity-identities.txt`.

Save-continuation repair (2026-10-07): the above-cent difference was caused by missing simulation
history and inconsistent route caches, not a need for finer currency precision. Format 75 saves
smoothed lane delays and the fractional one-second update clock. CCH rebuild/customization now
uses free-flow base costs, as required by the entrance-routing contract; it no longer samples
current congestion into whole-network costs. Freight retains network legs across traffic changes;
complete quotes are reused only while their sampled traffic revision still matches. Changed
frontage delays trigger lazy repricing without clearing or reallocating the complete quote cache.
Hiring quotes refresh when those delays change, and network cost tables refresh on CCH metric edits.

A developed-town replay exposed a further reset: speed, planned target/activity, lane-change and
overtaking state, replan failures and employment counters were initialized again on load. Two
shoppers then completed different trips, changing first-day VAT by 13.33. Format 75 preserves
these fields, remapping lane and building references through the existing snapshot maps. See
[the continuation contract](entrance_and_exit.md#exact-frontage-congestion-penalty-cache).

Fresh focused release verification passes 35 save-related checks, five ignored, including both
30-day frame-by-frame continuations: queued construction and a ten-day developed town with live
freight and traffic delays. Treasury, each firm's cash and each household's cash must differ by
less than 0.01; population, staffing, construction, inventory, shopping state and queued projects
remain exact. Lane-compaction coverage checks delay and active-agent state remapping. Command:
`RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib save --
--nocapture --test-threads=1`; log: `/tmp/metrum-reload-agents-tests.log`. Investigation logs:
`/tmp/metrum-reload-fee-trace.log`, `/tmp/metrum-reload-busy-trace.log`. The earlier failing logs
remain historical evidence; cash continuation is resolved for newly written format-75 saves.

The final release run passes 2,169 library tests and all three growth integration tests, with 95
library tests ignored. The sole failure is the pre-existing utility-machinery quantity expectation
(`budget_counts_machinery_for_all_utilities_without_treating_it_as_coal`). Both continuation tests,
the live-frontage/base-cost cache regression and the free-flow CCH rebuild regression pass.
Command: `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib
--test growth_scenario --no-fail-fast -- --test-threads=1`; log:
`/tmp/metrum-reload-cached-final-tests.log`. This final run includes revision-guarded quote reuse.

Reload-fix performance (2026-10-07): matched sequential, unprofiled release executables with
`RAYON_NUM_THREADS=8`, CPUs `0,2,4,6,8,10,12,14`, and unchanged demand/economy configuration.
Both runs match at 10,036 residents / 5,023 households / 6,816 employed / 13,881 jobs /
7,396 buildings, including starting and ending treasury. Hourly estimates are 2.4241 / 2.4730 ms
(p=0.48); daily 448.93 / 458.70 ms (p=0.92): neither change is statistically significant.
Unconditional frontage repricing was rejected after the initial comparison rose from 2.5024 to
4.3915 ms hourly. Traffic-revision tags retain complete quotes until their inputs change, avoiding
that repeated geometry work without retaining stale prices. The fixture measures fully built
economic ticks; it does not establish full traffic or active-frontage-invalidation scaling.

Build: `cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run`.
With `CRITERION_HOME=/tmp/metrum-reload-cached-perf`, run
`taskset -c 0,2,4,6,8,10,12,14 /tmp/metrum-demand-alternatives-bench --bench /10k
--save-baseline before_reload_fix`, then
`taskset -c 0,2,4,6,8,10,12,14 /tmp/metrum-reload-cached-bench --bench /10k
--baseline before_reload_fix`, both with eight Rayon workers. Driver:
`/tmp/metrum-reload-cached-perf.py`; logs: `/tmp/metrum-reload-cached-{baseline,after}-perf.log`;
executable/config/source hashes: `/tmp/metrum-reload-cached-identities.txt`. The rejected initial
comparison is retained in `/tmp/metrum-reload-{baseline,after}-perf.log`.

Cleanup performance (2026-10-07): sequential unprofiled release runs, eight Rayon workers on
CPUs `0,2,4,6,8,10,12,14`, match at 10,036 residents / 5,023 households / 6,816 employed /
13,881 jobs / 7,396 buildings, including starting/ending treasury. With
`CRITERION_HOME=/tmp/metrum-demand-cleanup-perf`, run
`/tmp/metrum-demand-entry-bench --bench /10k --save-baseline entry`, then
`/tmp/metrum-demand-cleanup-bench --bench /10k --baseline entry`. The older executable needs its
preceding demand config; the driver restores the cleanup config before running the new executable
and verifies restoration. Economy prices and assets are unchanged. Hourly estimates are
2.4397 / 2.4188 ms (p=0.74), daily 454.16 / 453.35 ms (p=0.99), with no significant timing change.
This measures the incremental cleanup on a fully built city, not active queues or the original
pre-redesign baseline. Driver: `/tmp/metrum-demand-cleanup-perf.py`; logs:
`/tmp/metrum-demand-cleanup-{baseline,after}-perf.log`; executable, config and save hashes:
`/tmp/metrum-demand-cleanup-identities.txt`.

Queue reservations now reach each hourly planner before project selection. Numeric scratch
reserves queued output customers, workforce, homes and parcels, with historical duplicate parcels
counted once in execution order. Collection/sorting costs O(Q log Q); checking each candidate's
parcel costs O(log Q). No asset-name clones or new spatial index are added. Queue timestamps
include one execution slot per preceding action, even a stale action that will fail placement.
Construction completion follows the actual boundary order: placement at minute 60 gets its first
construction tick immediately, while placement at minute 61 waits until minute 120. New proposals
use the earliest possible start after the existing queue; paired new homes are queued before all
new workplaces. This conservative deadline cannot count an existing late home prematurely, though
later projects within a large new batch may have additional feasible supply not yet credited.
Future workplaces still supply no admission jobs or wages. Queue reservations are derived again
from the saved queue; no reservation calendar is serialized. Execution checks only actually placed supporting homes,
including their completion dates. A rejected project spends no equity and can be reconsidered
by the next hourly planner. Planning is not disabled merely because projects remain queued.

Independent-entry verification (2026-10-07): the full eight-worker release run
`cargo test --manifest-path rust/Cargo.toml --release --lib --test growth_scenario --no-fail-fast -- --test-threads=1`
passes **2,164 library tests and all three growth scenarios**. Two library checks failed: the
pre-existing utility-machinery quantity expectation and the new expansion test's initial owner
capital bound. That bound incorrectly limited 270 days of actual arrivals to one adult per future
home. The corrected test counts actual supplied adults and freshly passes separately; production
code is unchanged after the full run. Logs: `/tmp/metrum-demand-entry-categories-tests.log`,
`/tmp/metrum-demand-entry-final-expansion-tests.log`. The starter scenario reaches 157 residents /
123 employed at day 366 and retains them at day 731, with all five business profiles operating,
positive per-firm operating cash changes and no bankruptcies. Reports:
`/tmp/metrum-demand-entry-categories-{year,two-years}.txt`.

The expansion regression establishes a town for 365 days, adds 46 industrial lots and limited
residential zoning, then observes 270 days. Initial prospective workforce is 15; actual new
adult supply reaches 20. Three factories open, each exceeds 95% lifetime staffing and has positive
operating cash change; every incumbent industrial firm also remains profitable. Entrant owner
equity is exactly the authored grants, 26,940 total, below the 34,600 workforce-derived bound.
The test retains failed/removed firms and checks each separately. New entrants need not occupy
the newly painted street: existing spare lots remain eligible. Report:
`/tmp/metrum-demand-entry-final-expansion.txt`. Targeted execution regressions cover failed and
bulldozed supporting housing, no reused labour, and simultaneous independently supported factories.

The saved-town replay freshly passes for 30 days with eight workers, using the unchanged source
`/tmp/metrum-demand-economy-input.sqlite`: 7 residents / 7 employed at day 3 becomes 85 / 69 at
day 33. This establishes recovery from the prior growth stall, not long-run profitability: some
new factories still have negative cumulative operating cash at 30 days. Command uses
`METRUM_DIAG_SAVE`, `METRUM_DIAG_MODS`, `METRUM_DIAG_REPORT`, `METRUM_DIAG_DAYS=30` with
`cargo test --manifest-path rust/Cargo.toml --release --lib diagnose_saved_city_growth -- --ignored --nocapture --test-threads=1`.
Report: `/tmp/metrum-demand-entry-saved-city.txt`; test log:
`/tmp/metrum-demand-entry-saved-city-tests.log`.

The same saved-town replay extended to `METRUM_DIAG_DAYS=365` also passes. At day 368 it retains
85 residents / 69 employed, with treasury 741,874.70. Inspection of all 14 individual firm records
finds positive cumulative operating cash change and no bankruptcy/removal: the early negative
balances recover. Report: `/tmp/metrum-demand-entry-saved-city-365.txt`; log:
`/tmp/metrum-demand-entry-saved-city-365-tests.log`. This diagnostic observes the unmodified saved
world; it does not replace the automated varied-town, failure-recovery or save-continuation checks.

Independent-entry performance (2026-10-07): sequential unprofiled release runs, eight Rayon
workers pinned to CPUs `0,2,4,6,8,10,12,14`, match exactly at 10,036 residents / 5,023 households /
6,816 employed / 13,881 jobs / 7,396 buildings, including starting and ending treasury. With
`CRITERION_HOME=/tmp/metrum-demand-entry-perf`, run
`/tmp/metrum-demand-worker-housing-bench --bench /10k --save-baseline worker-housing`, then
`/tmp/metrum-demand-entry-bench --bench /10k --baseline worker-housing`. Hourly estimates are
2.4348 / 2.4244 ms (p=0.85), daily 448.29 / 451.01 ms (p=0.98): no significant timing change.
This is an incremental comparison with the preceding worker-home guard, not the original
pre-redesign build or an active construction queue. Logs:
`/tmp/metrum-demand-entry-{baseline,after}-perf.log`; executable and save hashes:
`/tmp/metrum-demand-entry-identities.txt`. Catalog identity is unchanged from the queue comparison.

Queue-stage verification (2026-10-07): `RAYON_NUM_THREADS=8`, one test thread,
`cargo test --manifest-path rust/Cargo.toml --release --lib --test growth_scenario --no-fail-fast -- --test-threads=1`
freshly passes **2,150 library tests and all three growth scenarios**, with the same four remaining
library failures: utility-machinery quantity, the mine bankruptcy-count expectation, and the two
startup residential-selection fixtures. The five added regressions cover hourly boundary timing,
queued housing deadlines/finite claims, queued customer reservations without admission wages,
queued factory/home coordination, and duplicate-parcel prevention across hourly passes. Existing
queue save/reload and erased-lot tests also pass. The year/two-year scenarios still reach 123/153
residents and 88/111 employed, with no business bankruptcies. Logs/reports:
`/tmp/metrum-demand-queue-all-tests.log`, `/tmp/metrum-demand-queue-year.txt`,
`/tmp/metrum-demand-queue-two-years.txt`. These are queue-reservation checks, not the still-required
scarce-labour many-industrial-lots acceptance case.

Queue-stage performance (2026-10-07): sequential unprofiled release comparisons use eight Rayon
workers pinned to CPUs `0,2,4,6,8,10,12,14`. Both binaries start/end with 10,082 residents,
5,048 households, 6,851 employed, 13,881 jobs and 7,396 buildings; ending treasury also matches.
With `CRITERION_HOME=/tmp/metrum-demand-queue-perf`, commands are
`/tmp/metrum-demand-operation-bench --bench /10k --save-baseline operation` then
`/tmp/metrum-demand-queue-bench --bench /10k --baseline operation`.
Hourly estimates are 2.2644/2.4074 ms (+6.32%, p=0.02); daily estimates 710.34/720.20 ms
(p=0.93). Because the hourly result flags regression, a reversed-order comparison uses
`CRITERION_HOME=/tmp/metrum-demand-queue-perf-reverse`, the queue binary with
`--save-baseline queue`, then the operation binary with `--baseline queue`. This time queue/old
hourly estimates are 2.3603/2.4422 ms (old +3.47%, p=0.17), and daily 706.75/719.28 ms (p=0.92).
The first slowdown does not reproduce; both comparisons are retained rather than claiming a
performance improvement. The fully built fixture does not measure a large construction queue,
and comparison against the original pre-admission fixture remains open. Logs:
`/tmp/metrum-demand-queue-{baseline,after}-perf.log`,
`/tmp/metrum-demand-queue-reverse-{new,old}-perf.log`; executable/catalog hashes:
`/tmp/metrum-demand-queue-identities.txt`.

Business entry now uses `economy/operation.rs` for gross payroll, net purchased inputs, authored
electricity/area utility demand, property tax and amortized freight batches. Grocery stores as
well as services must cover their demand-responsive operating workforce. Owner equity is absent
from the recurring-revenue comparison. The forecast reuses the logistics import/export rounding
and the profile's input buffer targets, including the two-batch minimum for slow maintenance
inputs. Self-produced maintenance does not become a recurring import expense, but its initial
circulation inventory still requires working capital. Per-resource export allocation avoids mixing
output quantities to derive a shared export ratio.

Live employment and admission now share `economy/hiring.rs` instead of separate cash-only
limits. The forecast compares recurring local/export sales with the operating calculator and
requires working capital for replenishment and settlement. Cash, stocked inputs/outputs, paid
inbound goods and dispatched sales receivables bridge timing; owner equity never enters sales.
Freight assets exclude carrier charges and closed orders, and undeparted outbound cargo is not
counted twice. Constructing, broken and deserted employers supply no live jobs. City services
retain their explicit funding limit. Forecast capacities are rebuilt on employment passes,
policy changes and load, and consumed in O(1) by job assignment and demand.

Live exporters use the existing cached border route sweeps and actual carrier charge. Gateway
role changes invalidate quotes even without a CCH rebuild. Private staffing checks the largest
operation first, then smaller integer workforces: cheap exports must not erase a profitable
smaller local operation. Resource aggregation uses fixed Rayon chunks and ordered f64 merges;
storage is reused. Cost is O(B*ports + ceil(B/256)*resources + households + shipments + nodes),
plus route-quote refresh on revisions. Staffing commonly evaluates one operation per business;
worst-case work is O(total physical jobs * ports²), parallel across buildings, without per-job
allocation. This worst case still needs scale coverage beyond the ordinary 10k fixture.

Project forecasts still receive only the dispatch-fee lower bound; actual candidate-site quotes
remain open. Local market aggregation is citywide and counts current resident consumption and
staffed business inputs; it does not yet bound buyers by their affordability or connected local
market. Full shared economic acceptance therefore remains pending. Proposed factory workforce
remains the full authored operation until entry and live hiring use the same site/market inputs.

Shared-hiring verification (2026-10-07): with eight Rayon workers and one test thread, the full
`--release --lib --test growth_scenario --no-fail-fast` run passes **2,158 library tests and all
three growth scenarios**, with 95 ignored library tests. The remaining failure is the pre-existing
utility-machinery fixture (96 expected versus 540 actual kg for 1,200 kWh). The two startup
family/variant fixtures now supply funded factory construction as their housing opportunity;
their original selection assertions pass. The oversized mine check now asserts individual cash
loss, incomplete staffing and no further owner equity instead of requiring bankruptcy by day 90:
withdrawing unfunded jobs can extend its runway. Other no-bankruptcy requirements are retained.
New tests cover paid inbound inventory, post-payroll liquidity, grants without profitable sales,
construction, gateway changes without CCH rebuilding, and a viable smaller local operation when
full exports lose money. Freight tests exclude closed orders and prevent cargo/receivable
double-counting. Log: `/tmp/metrum-demand-hiring-verified-tests.log`; reports:
`/tmp/metrum-demand-hiring-verified-{year,two-years}.txt`. The starter town reaches 97 residents /
76 employed at day 366 and 156 / 123 at day 731. These runs precede the subsequent worker-home
admission guard and do not replace the required scarce-labour zoning expansion scenario.

The worker-home guard's final run uses the same full command and worker settings and passes
**2,160 library tests and all three growth scenarios**, with 95 ignored tests and the same
utility-machinery failure. Its 84 demand tests include preservation of future-worker homes,
continued pension admission into surplus homes, and rejection without consuming either wage or
home reservations. Final log: `/tmp/metrum-demand-worker-housing-all-tests.log`; reports:
`/tmp/metrum-demand-worker-housing-{year,two-years}.txt`. Population/employment and individual
firm outcomes match the preceding hiring run. Year one records 12 firms, 116,775 owner equity,
and two firms with cumulative operating losses (both younger than 2,160 observed operating hours).
Year two records 18 firms, 176,835 owner equity and no individually loss-making firm. These are
starter-town observations, not the scarce-labour many-lot expansion acceptance test.

Hiring-stage 10k measurements use sequential unprofiled release runs, eight Rayon workers and
affinity `0,2,4,6,8,10,12,14`. With `CRITERION_HOME=/tmp/metrum-demand-hiring-perf`, run
`/tmp/metrum-demand-queue-bench --bench /10k --save-baseline queue` then
`/tmp/metrum-demand-hiring-bench --bench /10k --baseline queue`. Hourly estimates are
2.3880/2.5461 ms and daily estimates 707.77/457.46 ms. **This comparison is not matched and is
not performance acceptance:** the old fixture has 10,082 residents / 5,048 households / 6,851
employed, versus 10,036 / 5,023 / 6,816 in the hiring build. Both retain 7,396 buildings and
13,881 physical jobs. Fixture seeding advances normal demand between explicit admission batches,
so changes in hiring affect automatic arrivals and the final population. A common seeded state
is still needed for comparative acceptance. Logs:
`/tmp/metrum-demand-hiring-{baseline,after}-perf.log`; binary hashes:
`/tmp/metrum-demand-hiring-identities.txt` (same catalog hashes as the queue comparison).

The subsequent worker-home guard has a matched 10k comparison against that hiring build, using
the same affinity/workers and sequential unprofiled release execution. With
`CRITERION_HOME=/tmp/metrum-demand-worker-housing-perf`, run
`/tmp/metrum-demand-hiring-bench --bench /10k --save-baseline hiring` then
`/tmp/metrum-demand-worker-housing-bench --bench /10k --baseline hiring`. Both start/end with
10,036 residents, 5,023 households, 6,816 employed, 7,396 buildings and 13,881 jobs; starting and
ending treasury match exactly. Hourly estimates are 2.4712/2.3984 ms (p=0.22), daily
457.09/452.15 ms (p=0.96), with no significant timing change. This establishes the incremental
guard cost only, not performance acceptance for the broader hiring redesign. Logs:
`/tmp/metrum-demand-worker-housing-{baseline,after}-perf.log`; hashes:
`/tmp/metrum-demand-worker-housing-identities.txt`.

Fresh operating-cost-stage verification (2026-10-07), eight Rayon workers and one test thread:
`cargo test --manifest-path rust/Cargo.toml --release --lib simulation::economy::` passes
**368 tests, 24 ignored** after adopting the dispatcher quantization helpers. The full
`--release --lib --test growth_scenario --no-fail-fast` run passes **2,145 library tests and all
three growth tests**, with the same four outstanding library failures recorded below. Logs:
`/tmp/metrum-demand-operation-quantized-tests.log`, `/tmp/metrum-demand-operation-final-tests.log`.
Reports: `/tmp/metrum-demand-operation-final-year.txt`,
`/tmp/metrum-demand-operation-final-two-years.txt`. Year one has 123 residents / 88 employed and
13 firms; year two has 153 / 111 and 15 firms. Owner startup capital totals 129,875 / 156,075,
compared with 121,920 / 161,220 in the workforce-only run. Year one has one young firm with a
cumulative operating loss, but none among firms with at least 2,160 observed operating hours;
the earlier revision had four losses in that older group. All firms have positive cumulative
operating cash change at year two. This remains the starter scenario, not the required scarce-
labour many-lot expansion test.

The operating-cost stage also has a fresh matched 10k comparison against the workforce-only
build: eight Rayon workers, fixed affinity `0,2,4,6,8,10,12,14`, sequential unprofiled release runs.
Use `CRITERION_HOME=/tmp/metrum-demand-operation-perf` with
`/tmp/metrum-demand-workforce-bench --bench /10k --save-baseline workforce`, followed by
`/tmp/metrum-demand-operation-bench --bench /10k --baseline workforce`. Both builds retain the
same 10,082 residents / 5,048 households / 6,851 employed / 13,881 jobs / 7,396 buildings and ending
treasury. Hourly estimates are **2.2130 / 2.2728 ms** (+2.70%, p=0.14); daily estimates are
**719.00 / 700.61 ms** (p=0.88). Neither difference is statistically significant. This benchmark
uses a fully built city, so it does not establish the cost of a large queued construction batch.
Logs: `/tmp/metrum-demand-operation-baseline-perf.log`,
`/tmp/metrum-demand-operation-after-perf.log`; identities:
`/tmp/metrum-demand-operation-identities.txt`. The original pre-admission workload mismatch
remains a separate full-redesign acceptance issue.

Fresh workforce-stage verification (2026-10-07): eight Rayon workers, one test thread,
`cargo test --manifest-path rust/Cargo.toml --release --lib --test growth_scenario --no-fail-fast
-- --test-threads=1` passes **2,143 library tests and all three growth integration tests**. The same
four library failures listed below remain. The two-year no-bankruptcy assertion now passes without
weakening it; the town has 101 residents / 78 employed at day 366 and 151 / 112 at day 731. Targeted
tests cover insufficient homes, late homes, construction creating housing without admission wages,
rejection without consuming supply, and shared reservations. This does not yet establish the
many-industrial-lots acceptance case. Log: `/tmp/metrum-demand-workforce-all-tests.log`; reports:
`/tmp/metrum-demand-workforce-year.txt`, `/tmp/metrum-demand-workforce-two-years.txt`.

The opt-in growth report now retains each firm's operating cash change separately from opening
cash and new owner capital, sampled staffing/capacity hours, and bankruptcy/removal status. Site
and build-generation identities survive allocator swap-removal. The three growth tests freshly
pass with reconciliation of individual balances against profile totals, including mid-run observer
attachment and concurrent deterministic runs. Log: `/tmp/metrum-demand-workforce-firms-tests.log`;
reports: `/tmp/metrum-demand-workforce-firms-year.txt` and
`/tmp/metrum-demand-workforce-firms-two-years.txt`. Four service firms have cumulative losses at
year one despite zero bankruptcies, then positive cumulative operating cash at year two. This is
exactly why a future expansion test must inspect each firm's losses and capital, not just closures.

Matched, sequential unprofiled 10k checks for the **workforce stage relative to the admission-only
stage** use eight Rayon workers and affinity `0,2,4,6,8,10,12,14`. Both builds start and end with
10,082 residents, 5,048 households, 6,851 employed, 13,881 jobs and 7,396 buildings; ending treasury
also matches. With `CRITERION_HOME=/tmp/metrum-demand-workforce-perf`, run
`/tmp/metrum-demand-admission-bench --bench /10k --save-baseline admission`, then
`/tmp/metrum-demand-workforce-bench --bench /10k --baseline admission`. Hourly estimates:
**2.3355 / 2.3479 ms** (change +0.53%, p=0.88); daily: **717.40 / 703.99 ms** (p=0.91).
Neither comparison detects a significant change. Logs:
`/tmp/metrum-demand-workforce-baseline-perf.log`, `/tmp/metrum-demand-workforce-after-perf.log`;
binary/catalog identities: `/tmp/metrum-demand-workforce-identities.txt`. This validates the
added reservation work at the matched 10k workload, not the full redesign against the original
pre-admission build; that earlier fixture mismatch remains unresolved.

Earlier admission-only intermediate verification (2026-10-07; not acceptance):

- `RAYON_NUM_THREADS=8 cargo test --manifest-path rust/Cargo.toml --release --lib --test
  growth_scenario --no-fail-fast -- --test-threads=1`: **2,139 library tests pass, four fail,
  95 ignored; two growth tests pass, one fails**. Same-hour rejection recovery, finite search,
  unchanged occupancy after rejection, preserved arrival composition and exclusion of marginal
  jobs pass. The library failures are the two tracked residential startup cases, the 100 ha mine
  bankruptcy-timing assertion, and a coal ledger fixture expecting 96 kg where the current recipe
  reports 540 kg for 1,200 kWh (that fixture and recipe were not edited here).
- The deterministic year and read-only-observer tests pass. The two-year fixture reaches
  157 residents / 124 employed, but a food processor closes on day 664. Its profile's positive
  aggregate cash flow does not excuse an individual failed firm. Do not weaken the zero-failure
  or replacement-capital checks to accept this intermediate state. Full log:
  `/tmp/metrum-demand-admission-all-tests.log`; daily reports:
  `/tmp/metrum-demand-admission-year.txt`, `/tmp/metrum-demand-admission-two-years.txt`.
- The fresh saved-town replay ends at 13 residents / 11 employed on day 33, with the global
  industry gate still present. Report: `/tmp/metrum-demand-admission-saved-city.txt`; log:
  `/tmp/metrum-demand-admission-saved-city.log`. This is a diagnostic, not acceptance.
- Rustdoc completes with no missing-doc warnings; it reports the existing unresolved
  `VegetationGenerator` link in `nodes/simulation_node/vegetation_api.rs:155`, outside this change.
  Log: `/tmp/metrum-demand-admission-doc.log`.
- Sequential unprofiled 10k benchmark runs use eight Rayon workers, affinity
  `0,2,4,6,8,10,12,14`, the same authored economy/demand files, and release builds from the project
  root. Baseline executable `/tmp/metrum-demand-before-bench` uses `--bench /10k --save-baseline
  before`; `/tmp/metrum-demand-admission-bench` uses `--bench /10k --baseline before`, with
  `CRITERION_HOME=/tmp/metrum-demand-growth-perf`. Hourly estimates are 2.3070 / 2.2847 ms; daily
  estimates 778.07 / 732.06 ms. **These are not a matched-workload acceptance comparison:** the
  fixture currently shares admission selection, so its seeded population changed from
  11,120 residents / 5,597 households to 10,082 / 5,048, with 7,396 buildings in both.
  Restore a fixed prepopulation workload before claiming performance parity. Logs:
  `/tmp/metrum-demand-before-perf.log`, `/tmp/metrum-demand-admission-perf.log`; binary/catalog
  SHA-256 identities: `/tmp/metrum-demand-admission-identities.txt`.

### Open Spec Gaps

- **Building Desertion** — spec complete and implemented; see `§ Building Desertion` above

### Intentional `v0.1` Deferrals

- Zoning-specific growth behavior, if later required, must use explicit project inputs and shared
  reservations rather than reintroducing blanket citywide thresholds.
- `Office` and `Mixed` remain outside baseline private growth. Any later extension needs authored
  operations, customer/workforce reservations and matching zoning legality.
- local-modifier support beyond neutral defaults should be added only when the underlying source
  systems are implemented well enough to be trustworthy demand inputs.
