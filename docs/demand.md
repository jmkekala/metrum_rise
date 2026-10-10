# Demand

This is the current specification of city growth: which households arrive and leave, which private
projects start, when buildings change level, and when failed sites are cleared. Earlier pressure
formulas, the `DEM-02` implementation log and its measurements up to 2026-10-10 are preserved in
[`archive/demand_history_2026-10-10.md`](archive/demand_history_2026-10-10.md).

## Ownership

| Concern | Owner |
| --- | --- |
| Household admission and removal decisions; private construction, upgrades, downgrades and failed-site clearance; growth diagnostics | This document (`simulation/economy/demand/`) |
| Household records, money, housing affordability, relocation and eviction; the funding census; level-change viability | [`economy.md`](economy.md) |
| Legal zones, densities and level families | [`zoning.md`](zoning.md) |
| Build-site discovery, frontage attachment, placement and removal execution | [`building_allocator.md`](building_allocator.md) |
| Arrival trips and household carriers | [`entrance_and_exit.md`](entrance_and_exit.md) |

Rules:

- Demand decides; economy creates and updates the household or building record; transport only
  visualizes movement.
- No other subsystem decides ongoing growth. Scenario setup and allocator integrity cleanup are
  the only exceptions.
- City-owned facilities are placed by the player and are not demand-owned growth.

## Growth redesign (`DEM-02`)

Status, 2026-10-10. Growth uses concrete decisions about actual households and projects. The
former citywide R/C/I pressure signals, per-density growth thresholds, hypothetical-family
admission runway, marginal shop-job forecast, regional household cutoff, distress penalties,
spawn credits and saved hysteresis are all removed (save formats 74–83).

| Component | State |
| --- | --- |
| Independent business entry with shared customer, input and workforce reservations | Live |
| Completion-time workforce calendar pairing workplaces with homes | Live |
| Household admission from actual applicants' recurring income | Live |
| Individual household exit policy | Live |
| Level changes through economic viability and shared claims | Live |
| Paced failed-site clearance | Live |
| Growth HUD explanations from recorded decisions | Live |
| Second zoning/service mix, long-run affordability and broader expansion acceptance | Open (`DEM-02`, `ECON-14`) |
| 100k-scale growth costs (late-day household hour, one residual census) | Open (`ECON-13`) |

## Growth loop

Each hourly demand pass does the following:

1. Reads the settled operational-hour economy: households, homes, vacancies, the funding census
   and border connectivity.
2. Reserves every committed project's claims first: live firms, construction sites and the
   construction queue.
3. Plans new homes, businesses and level changes against what remains, paced by authored
   fractions.
4. Executes household admission and immediate building actions, and queues new construction one
   start per minute.

```text
settled economy ─► snapshot ─► reserve committed ─► plan R/C/I ─► admit ─► execute / queue
                    (census)    (live, building,     (claims are     (finite income
                                 queued projects)     shared)         per family)
```

A proposal that fails any check leaves every shared claim and its pacing credit untouched, so a
later viable proposal is not hidden.

## Cadence and handoff

- **Hourly demand pass:** runs after each completed operational hour at minutes other than 0
  (`SimCore::execute_hourly_demand_pass`). It plans projects, admits households, executes level
  changes and clearances, and queues new construction.
- **Daily pass:** runs at minute 0, after the daily economy settlement. It plans household
  departures from the settled snapshot; removals execute at once. The minute-0 hourly demand pass
  then runs, so each day has 24 hourly slices without reading pre-settlement state.
- **Construction queue:** new construction is queued and started one project per authored minute
  (`execute_pending_demand_spawns_for_minute`).
- **Frozen inputs:** a pass's inputs are fixed. Its changes affect the next operational hour.
  Hourly admissions are never eligible for removals already selected from that day's settlement.

## Authored tuning

`demand/growth_profiles.toml` holds only pacing and household-exit eligibility. Economic costs,
composition and viability thresholds belong to `economy/profiles.toml`.

| Key | Value | Meaning |
| --- | ---: | --- |
| `action_budget.max_household_removals_per_day` | 48 | Daily departure cap |
| `household_action.persistent_exit_daily_fraction` | 0.25 | Share of eligible households that may leave per day |
| `household_action.persistent_exit_destitute_stock_days` | 0.25 | Pantry coverage for early destitute exit |
| `household_action.persistent_exit_destitute_unhoused_days` | 2 | Days unhoused before a destitute exit |
| `household_action.persistent_exit_max_unhoused_days` | 7 | Days unhoused before any exit |
| `household_action.move_in_benefit_treasury_coverage_days` | 7 | Treasury days that make transfers count as reliable income |
| `upgrade_batch_fraction_by_use` | 0.25 R/C/I | Share of viable upgrades per day |
| `downgrade_batch_fraction_by_use` | 0.25 R/C/I | Share of viable downgrades per day |
| `despawn_batch_fraction_by_use` | 0.50 R/C/I | Share of clearable failed sites per day |

The loader validates the file once and rejects unknown fields:
- fractions must be finite and in `0..1`;
- durations must be at least one day;
- the pantry threshold must be in `0..365`;
- the removal cap must be a non-negative integer.

Fractions pace accepted opportunities; they never create demand. An hourly pass adds `1/24` of
each daily budget to a credit, and acts on the whole number of actions accumulated.

## Household admission

`demand/admission.rs`, executed by `BuildingAllocator::execute_demand_household_admission_with_income`.

**Execution capacity.** With at least one connected border:

```text
households_to_admit = min(vacant_homes, max(1, ceil(net_open_jobs / 24)))
net_open_jobs = funded open jobs − existing unemployed adults (including pending arrivals)
```

This is execution capacity, not approval. Without a border, nobody is admitted.

**Admission budget.** The pass reserves existing residents' and pending arrivals' transfer claims
first, then keeps one finite budget for the batch:
- remaining job-equivalents and their average net wage;
- per-member essential cost, including VAT and imported utilities;
- one home electricity allowance per household;
- the treasury coverage of transfers.

**Offers.** The batch tries at most `max(16, 4 × capacity)` deterministic offers. Each varies
family composition, size and home from an applicant seed that advances with simulation hours, so
one refused family never blocks the stream. For each physically launchable offer:

- Employed adults are `min(adults, remaining job-equivalents)`.
- Recurring income is employed adults × net wage, plus pension and child support scaled by
  transfer reliability. Unemployment benefit and savings never count as recurring income.
- The family is accepted only if recurring income covers its members' essentials, the home
  electricity allowance and property tax at the home's level.
- Acceptance consumes all of the family's adults from the job budget and adds its transfer claims.
  A refusal changes nothing.

**Worker homes.** Ready homes claimed by workplace planning stay available only to families with
an adult. Families without workers use only unreserved ready homes, so they cannot take housing
promised to a new employer's staff.

**Arrival.** An accepted family launches an arrival carrier from the border to its home. Its
approved adult, elder and child composition travels with the carrier, survives arrival
reordering and save/load, and materializes on arrival. Pending adults count as labor supply in
the next forecast.

The **empty-town bootstrap** offers the first home when the city has a border and no households.

## Household removal

`demand/system.rs::run_daily_pass` and `households/removal.rs`. Departure eligibility belongs to
each household (`HouseholdExitPolicy`). A household may leave only while it has no home, and
then only if one of these holds:

- it has been unhoused for at least `persistent_exit_max_unhoused_days` (7); or
- it has been unhoused for at least `persistent_exit_destitute_unhoused_days` (2), its pantry is
  at most 0.25 days and its reserve is under 1 day.

Housed households never qualify, and citywide crisis pressure plays no part. The daily credit
adds `eligible × 0.25`, capped by eligibility and by 48 per day.

Execution rechecks the same policy against live state, so a household that recovered stays. It
then ranks the remaining candidates by lower reserve days, lower pantry days and lower household
ID, and removes them in descending ID order through the existing lifecycle. Unused slots are
never filled with other households. Eligibility is an O(H) parallel pass; ranking is O(E log E).

## Private construction

`demand/building_actions.rs`, `spawn_need.rs`, `workforce.rs` and `supply.rs`.

### Residential

Homes are built to cover a concrete household need:

```text
missing_slots = unhoused households + incoming need + vacancy reserve
              − (vacant homes + homes under construction)
incoming need  = net open jobs (one incoming adult per job), or 1 for an empty connected town
vacancy reserve = ceil(10% of households)
```

Legal home sites are selected in attachment order until their actual household capacity covers
the need. The remaining legal sites become optional homes. An optional home is built only if
workforce planning pairs it with a workplace that needs its future residents.

### Commercial and industrial

Every legal site is assessed as a proposed business. Commercial categories are ranked by
unreserved recurring sales value, in currency rather than units. A proposal must pass every check
before any claim is committed:

1. **Customers.** The proposal needs unreserved recurring customers for its output. These are
   households' funded spending from the shared customer book, staffed business buyers' financed
   inputs, or executable exports at the outside bid. Output selected earlier in the pass is
   already reserved.
2. **Operating income.** Recurring sales must cover operating costs at the proposed door
   (`economy/operation.rs`): payroll, inputs at quoted local or import prices, utilities,
   property tax, and freight priced from that door's actual route. Working capital is checked
   separately from owner equity.
3. **Inputs.** The proposal's inputs are reserved from residual local supply in recipe order,
   with local-first sourcing.
4. **Workforce.** The calendar must supply the supported workers by construction completion.

If the preferred asset fails, other legal assets on the same lot are tried, for example a smaller
factory or another service category. Cheat mode bypasses these economic checks but keeps site and
asset legality.

### Workforce calendar

`WorkforceReservations` is rebuilt each pass, without per-project allocation. It offers
**unemployed adults now**, plus the **adults of homes that will be ready by a given completion
hour**:

- ready vacancies;
- homes under construction or queued;
- newly selected and optional homes;
- homes of displaced residents who meet the relocation reserve thresholds.

Claims are made in this order:
1. Incumbent workplaces claim first, including their unmet hiring after completion.
2. Constructing and queued projects claim next.
3. New proposals claim last.

Projects cannot share workers or count homes that finish too late. Immediate level changes may
claim only unemployed adults and already-ready homes.

### Construction queue and execution

Accepted new construction is appended to a minute queue (one start per minute). Queued projects
keep their claims and their prepared door.

Before placing a project and spending its owner equity, the minute executor revalidates:
- placed housing and unemployed adults;
- customers;
- input commitments.

Validation runs once per execution epoch, plus incremental updates. A failed placement consumes
nothing and retires the queue entry, so later checks revalidate anything that depended on it.
Construction itself is owned by [`economy.md`](economy.md#private-construction).

## Level changes

`demand/viability.rs` and `demand/execution/level_change.rs`. Upgrades and downgrades are instant
replacements by the asset one level up or down. Level is a wealth tier; a target may hold more
households or workers but need not. The target comes from the asset registry
(`BuildingAllocator::{upgrade,downgrade}_target`):

- **Upgrade family set** (`asset_set`): the family member at the next level, and nothing else.
- **No family:** one zoned-private asset without a family with the same zone type, density and
  lot footprint at the next level. A stable hash of parcel, generation and zoning profile picks
  among several, so planning, supply projection and execution agree, and the choice survives
  level changes and save/load.

A target must also:
- fit the same lot;
- hold the current occupants and workers;
- bind a valid economy profile.

Neither path checks a zoning profile's `required_asset_tags`; no shipped profile requires tags.
When the current or target asset authors a yard hedge or planting, the replacement clears the old
yard and lays the target's (same parcel and generation key).
The renderer presents each level change with a short real-time rise of the new model
([gameplay building LODs](asset_editor.md#gameplay-building-lods--render-07)); the simulation
swap itself stays instant, and households and workers stay in place.

Candidates must pass the economic viability gates in
[`economy.md`](economy.md#level-change-viability). Selection then works as follows:

- **Business replacements** prove the target operation inside one transaction. This covers its
  own income and working capital, released and re-reserved customers and inputs, extra workers,
  and the knock-on financing of affected buyers and suppliers. A rejected proof restores
  everything.
- **Residential downgrades** run after all growth claims. They may retire only ready homes left
  after workplace claims, the 10% vacancy reserve and homes for unhoused households.
- **Conflicts:** when one building offers both directions, a selected upgrade wins the pass.
- **Cheat mode** disables downgrades.

## Failed-site clearance

A private site becomes clearable only when its business is deserted (see
[Building Bankruptcy](economy.md#building-bankruptcy)) and it has no occupants and no workers.
Healthy empty buildings are never cleared for being empty.

Clearance is paced at half of the eligible sites per day, in deterministic attachment order, and
is independent of any market signal. Recovery demolition evacuates remaining occupants through the
street handoff before removing the building. A cleared site can host a new business; the bankrupt
company and its cash do not return.

Deserted buildings render through a parallel gray MultiMesh (`nodes/sim/render/buildings.rs`,
`buildings.gd`). They block their parcel, are excluded from all economic flows and demand
capacity, and are ineligible for level changes.

## Growth diagnostics

`demand/diagnostics.rs` and `demand/display.rs`. Each hourly pass records, per use:
- spawn candidates and free lots, lots missing an eligible asset, and unusable lots;
- selected, queued and constructing projects;
- homes needed;
- rejections for customers, operating income and workforce;
- level-change and clearance candidates, credits and selections.

Admission records vacancies, live funded jobs, income rejections and worker-housing deferrals.
The growth HUD presents these recorded decisions in O(1); it never reruns economic checks.
Debug filters `economy` and `spawn` log the per-pass details.

## Saved state

The save stores:
- `households_to_admit_today` and `households_to_remove_today`;
- the household exit credit;
- the upgrade, downgrade and clearance credits per use;
- the cheat flag;
- the pending construction queue.

Reservation books, the workforce calendar, execution proofs and diagnostics are derived. They are
rebuilt after load, never saved.

## Determinism and complexity

- **Ordering:** candidates use deterministic attachment order with stable parcel and building ID
  tie-breaks. Parallel reductions merge in input order, and the applicant stream depends only on
  simulation time.
- **Costs:**
  - snapshots and candidate collection are O(B) parallel passes;
  - residential selection is O(candidates);
  - each project check is bounded by its recipe ports, home-calendar size and indexed customer
    and supplier work.
- **No city scans:** there is no per-project city scan or per-agent search. Reservation scratch
  is reused across passes.
