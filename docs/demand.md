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
| Acceptance: second-mix year, 10k time budget, no housing overbuild | Done: all three pass on one build (`01888e88`) ([status](#status-2026-10-11-one-working-tree-build-on-47c5a053)) ([status](#status-2026-10-11-one-working-tree-build-on-47c5a053)) |
| 100k-scale growth costs (late-day household hour, one residual census) | Open (`ECON-13`) |

### Exit criteria

`DEM-02` is done when all three checks pass on one build. Populated-city scaling is not part of
this acceptance and is tracked under `ECON-13`: execution under changing frontage prices,
local-market scaling, the census per link of a profitability-failing supply chain, and the 100k
costs.

1. **Second-mix year (`DEM-02-A`).** A second `GrowthScenario` constructor builds a
   producer-led town modelled on a pre-zoned player city: residential, commercial and industrial
   paint plus a placed farm, coal mine and coal power plant, with no households at the start. A
   test in `rust/tests/growth_scenario.rs` runs it for 365 days and asserts, as the starter-town
   test does:
   - two concurrent runs record identical days and accounting;
   - city, business and household cash residuals stay below 0.1;
   - residents and employment rise above day 1, and day 365 is not below day 275 on either;
   - no business that opens goes bankrupt or needs replacement capital, and every opened profile
     has positive operating cash over the last 90 days;
   - over the last 90 days: no empty-household days, no unpaid household transfers and a
     positive recurring city balance.

   The mine is sized within what the `producer_balance_matrix` found to stay supplied for a year.
   Mine depletion and contraction are outside `DEM-02`.
2. **10k time budget (`DEM-02-B`).** The [`ECON-13` recipe](economy.md#econ-13-economy-tick-baseline)
   (`cargo bench --bench economy_tick_benchmark -- /10k`, `RAYON_NUM_THREADS=8`, pinned CPUs,
   unprofiled release) measures at most **40 ms** per mean hour and **10 ms** per mean day. The
   2026-10-10 baseline is 34.45 ms and 8.73 ms; the budget leaves about 15% for noise.
3. **No housing overbuild (`DEM-02-C`).** One home per net open job overbuilt housing, because
   admission charges every adult of an arriving family against those jobs. A player's 7-day
   pre-zoned save showed it: 379 households with about 1.47 employed adults each, no open jobs,
   no departures, and 79 vacant homes against a 38-home reserve, left vacant because industrial
   land ran out while residential land remained.
   - **Change (implemented 2026-10-11):** [incoming need](#residential) walks admission's
     applicant stream and counts accepted households until their adults cover the net open jobs.
     The [workforce calendar](#workforce-calendar) credits each future place with its
     applicant's adults instead of one worker. Both cost O(offers or places walked), with no
     measured city ratio.
   - **Check:** `GrowthDayRecord` records end-of-day vacant homes, the vacancy reserve,
     unhoused households, net open jobs and cumulative departures. Both growth scenarios assert
     that on every day with no net open jobs, no unhoused households and no departures so far,
     vacant homes ≤ vacancy reserve + the largest household capacity among the installed
     residential assets. Planner tests cover two-adult applicants (40 open jobs need 20 homes,
     single-adult ones 40, income-refused ones none) and calendar places offered to a couple or
     an elder.

   Vacancy above the reserve remains legitimate after departures or job losses, because healthy
   vacant buildings are never cleared. A per-pass "need plus one building" bound is not enough:
   every pass in the save above stayed within it.

#### Status (2026-10-11, one working-tree build on `47c5a053`)

Fresh results; release build.

- **`DEM-02-A`: passes.** `GrowthScenario::producer_town` places a 4 ha farm, a 1 ha mine and a
  coal power plant funded for two workers, paid from the treasury. Over 365 days both concurrent
  runs match; all residuals stay below 0.1; the town reaches 83 households, 153 residents and
  126/134 employed by day 4. Farm, mine, food processor, machinery factory, grocery, barber and
  pharmacy all stay open with no bankruptcy or replacement capital and positive operating cash
  over the last 90 days. The last 90 days have no empty-household day, no unpaid transfer and a
  positive recurring balance. Residential land fills by day 4, so the year tests stability
  rather than continued growth.
- **`DEM-02-B`: passes.** The `ECON-13` recipe, run as four alternating unprofiled runs of each
  build (`RAYON_NUM_THREADS=8`, `taskset -c 0,2,4,6,8,10,12,14`, `$BENCH_BINARY --bench /10k`),
  measures **34.32 ms per mean hour and 8.71 ms per mean day** on the final build (per-run hours
  34.27, 33.99, 34.67 and 34.36), against 34.48 / 8.70 ms for `47c5a053` (34.49, 34.43, 34.55
  and 34.45). Exact permutation tests give p = 0.37 (hour) and p = 0.91 (day): no significant
  change. Build-time and day-15 `GrowthDayRecord`s match on every shared field in all runs; the
  fixture builds every zoned lot up front, so the new need has no site to act on.
- **`DEM-02-C`: passes.** Vacancy stays within the bound in the producer town and over the
  starter town's two years; the planner tests pass. Copies of the player's saves, replayed for
  14 days on the final build:
  - the 7-day save keeps its 79 vacant homes (reserve 38) and places no further building (306
    before and after). The change stops further overbuild but cannot remove homes that already
    stand, because healthy vacant buildings are never cleared;
  - the empty pre-zoned save fills its residential land by day 3 and ends at 218 households with
    2 vacant homes against a reserve of 22.

  An ignored probe with ample residential land and one short industrial frontage
  (`balance_cases::residential_rich_town_housing_balance`) shows the effect of each part:

  | Build | Vacant homes / reserve, day 61 |
  | --- | --- |
  | Need walk only, one worker per calendar place | 17 / 5 |
  | Need walk and applicant adults in the calendar | 6 / 6 |
  | Final: adults net of store labor, store crews capped to supply | 8 / 6 (bound 7) |

  The final build is one home over the probe's bound, accepted on review (2026-10-11). Its stores are fully staffed (grocery 7,
  barber 1, pharmacy 1), and rounding store labor once over all hours changes nothing. The excess
  comes from the calendar's applicants, which are not income-filtered: they bring fewer adults
  than the families admission accepts (1.44 per household here). Filtering them needs the wage
  of jobs that do not exist yet. The probe is not one of the criterion's two scenarios.

  **Why the calendar needed two more rules.** Applicant adults alone:
  - left the starter town without a grocery. On day 1 industry claimed every worker the land
    could house. The grocery's crew is sized to its whole market and grew from 2 to 11 workers
    as residents arrived, while spare supply ended at 9, so all 79 households had empty
    pantries for two years;
  - left a new food processor in `many_new_industrial_lots_…` at 80% of its reserved staffing.
    The 20 workers arriving for 20 new jobs were partly taken by the incumbent grocery, whose
    crew grew with the new residents.

  Both arise because arrivals bring shoppers as well as workers. The calendar now credits
  arrivals with their adults net of the store workers their members' consumption needs, and a
  store short only of workers opens with the crew the calendar can supply (see
  [Workforce calendar](#workforce-calendar) and
  [Commercial and industrial](#commercial-and-industrial)). Each rule alone leaves one of the
  two cases failing; together all library and growth tests pass, and the starter town reaches
  83 households with a profitable grocery. Tried and reverted: keeping later industrial
  proposals off a waiting store's crew held industry at 110 jobs and still left the grocery
  2 workers short.

## Growth loop

Each hourly demand pass does the following:

1. Reads the settled operational-hour economy: households, homes, vacancies, the funding census
   and border connectivity.
2. Reserves every committed project's claims first: live firms, construction sites and the
   construction queue.
3. Plans new homes, businesses and level changes against what remains, paced by authored
   fractions.
4. Decides household admission, proves the level changes and queues new construction one start
   per minute; request queues then create the admitted households, apply the level changes and
   remove the cleared buildings (`SIM-02-J`).

```text
settled economy ─► snapshot ─► reserve committed ─► plan R/C/I ─► admit ─► execute / queue
                    (census)    (live, building,     (claims are     (finite income
                                 queued projects)     shared)         per family)
```

A proposal that fails any check leaves every shared claim and its pacing credit untouched, so a
later viable proposal is not hidden.

## Cadence and handoff

This section describes the current cadence. Its replacement is planned in
[`simulation.md`](simulation.md#sim-02-scheduler-refactor) (`SIM-02`).

- **Hourly demand planning:** four slots of every hour, 00:00 included, a game minute apart, each
  in two halves of its minute (`SIM-02-I` part 4, `SIM-02-K` part 5): a setup at the minute's
  first slot counts the live workforce and, beside the count, scans the zone's free lots and
  existing buildings (a workforce reset passes over the buildings once, and the site feasibility
  cache is split by parcel), and the plan eight slots later decides on it. Minutes 0, 1 and 2
  (plans at slots 8, 24 and 40) plan
  residential, commercial and industrial projects into a saved hour plan, each on the
  published census books; minute 3 (the actions slot, plan at slot 56) plans downgrades,
  decides the hour's admissions,
  queues the hour's spawns in the single pass's order (homes, claimed optional homes, shops,
  industry), proves the queue's permits and the level changes, and queues the level changes,
  admissions and clearances for their request queues (`SIM-02-J`). Its proposals are provisional: the live
  economic books it plans on are restored before it returns. A save between a setup and its plan
  keeps the setup. It plans on the cell lots served
  so far: dirty lot regions are regenerated by a per-slot queue, one region per slot outside the
  hour's queue-free slots (two in the slot after one), not by the
  pass (`SIM-02-H`, [`simulation.md`](simulation.md#request-queues)).
- **Household exits:** each household checks its own exit eligibility at its daily phase, after
  its settlement, and claims its share of the day's exit budget
  ([Household removal](#household-removal)). The `departures` queue removes the claimed
  households, one a slot, in slots 65–479 of the hour (slot 65 also serving slot 64's share;
  `SIM-02-J`). There is no daily pass
  (`SIM-02-G`).
- **Construction queue:** new construction is queued and started one project per authored minute,
  each start a timed event (`SIM-02-C`); the hour's queued spawns start one a minute from minute 4
  (from minute 1 before `SIM-02-K` part 5), each at its minute's first slot, or its second when the
  first is one of the hour's queue-free slots, as minute 4's is (slot 65;
  [`simulation.md`](simulation.md#request-queues)). A site completes at its own slot, its duration
  after the
  start (`SIM-02-D`). The workforce calendar counts every site, placed, queued or proposed, as
  ready at the first hour boundary at or after its completion.
- **Frozen inputs:** a pass's inputs are fixed. Its changes affect the next operational hour and
  each entity's next hourly phase. The pass runs before the phases of its slot, so it sees every
  building and household exactly one hour on (`SIM-02-F`).
- **Snapshot:** the pass plans on the demand snapshot the previous hour's rolling sweep built,
  plus the buildings and households created since that sweep began, folded at the hour's first
  slot before that slot's job requests hire them (`SIM-02-I`, `SIM-02-J`,
  [`simulation.md`](simulation.md#snapshots)):
  each entity as it stood at its chunk of that hour, read with the funded jobs and activity the
  published census copied onto each building. The workforce (housed unemployed adults,
  displaced households' labour, and the open funded jobs, their wages and the worker
  capacities) is counted on the live city at the pass, with the hires the per-slot job queue
  has made so far.
  The pass lends the published census books, and the construction epoch at the same slot proves
  the queue's permits on them (`SIM-02-I` part 3).
  A household admitted by an hourly pass cannot claim an exit: exits need at least two unhoused
  days.

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
one refused family never blocks the stream. Housing need and the workforce calendar read the
same applicant stream (`DEM-02-C`). For each physically launchable offer:

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

**Arrival.** An accepted family claims its home at once and waits in the `admissions` queue
(`SIM-02-J`). The actions slot (slot 56) decides the whole batch; the queue creates the households
over the following hour, paced by a credit that starts at one whole admission, the first at slot 57
(the actions slot
itself is queue-free), and launches each
family's arrival carrier from the border to its home. The carrier waits at the border for a
departure slot paced from its creation (`SIM-02-E`). A family whose home left before its turn is
dropped; one whose border has lost its car road releases its home. The border arrival sweeps are kept
while the routing hierarchy, its metric and the graph's node roles are unchanged (`SIM-02-K`
part 5). Families still in the queue count
as pending arrivals. Its approved adult, elder and child composition travels with the carrier,
survives arrival reordering and save/load, and materializes when the carrier reaches its home.
Pending adults count as labor supply in the next forecast; each hour recounts them from the carriers
still on their way.

The **empty-town bootstrap** offers the first home when the city has a border and no households.

## Household removal

`demand/system.rs::claim_household_exit` and `households/removal.rs`. Departure eligibility belongs to
each household (`HouseholdExitPolicy`). A household may leave only while it has no home, and
then only if one of these holds:

- it has been unhoused for at least `persistent_exit_max_unhoused_days` (7); or
- it has been unhoused for at least `persistent_exit_destitute_unhoused_days` (2), its pantry is
  at most 0.25 days and its reserve is under 1 day.

Housed households never qualify, and citywide crisis pressure plays no part. At its daily phase
an eligible household adds 0.25 (`persistent_exit_daily_fraction`) to the exit credit; each whole
unit of credit releases the household that completed it, at most 48 per calendar day. The
credit lapses at a calendar close when no household was eligible that day. Each claim is O(1)
(`SIM-02-G`). The `departures` queue (`SIM-02-J`) removes the released households in release
order, one a slot in slots 65–479 of each hour (slot 65 also serving slot 64's share), through the
existing lifecycle, after rechecking
the same policy against live state. The window is the one in which a swap-remove neither
restarts the census nor invalidates the published books
([`simulation.md`](simulation.md#request-queues)).

The next operational hour removes the released households in release order through the
existing lifecycle, after rechecking the same policy against live state, so a household that
recovered stays. Departures therefore follow the households' phases, not lowest reserve first.
Unused releases are never passed to other households.

## Private construction

`demand/building_actions.rs`, `spawn_need.rs`, `workforce.rs` and `supply.rs`.

### Residential

Homes are built to cover a concrete household need:

```text
missing_slots = unhoused households + incoming need + vacancy reserve
              − (vacant homes + homes under construction)
incoming need  = arriving households whose adults staff the net open jobs (below),
                 at least 1 for an empty connected town
vacancy reserve = ceil(10% of households)
```

**Incoming need (`DEM-02-C`).** Admission charges every adult of an arriving family against the
open jobs, so one home per job overbuilt housing. The planner instead walks the applicant stream
admission examines this hour: offer `k` goes to vacancy position `(seed + k) mod vacancies` with
household seed `seed + k`, where `seed = day × 24 + hour`. It runs each offer through a copy of
admission's income budget, skips the refused ones, and stops once the accepted families' adults
cover the net open jobs. The accepted offers are the incoming need. Homes do not limit the walk;
it is bounded by admission's own `max(16, 4 × net open jobs)` offers. Before any home is vacant,
this pass's legal home sites, in attachment order, stand in for the homes the offers would
reach (keyed by parcel ID). The walk skips admission's border-route check. Cost:
O(offers walked) per pass, with no allocation (`spawn_need.rs::incoming_household_need`,
`HouseholdAdmissionBudget::households_to_staff`).

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
   A store (`Store` or `ServiceStore`) that passes every other check but cannot get its full
   crew is assessed again with the crew the calendar can supply. It claims only that crew's
   share of its output from the customer book, so the remaining customers stay available to
   another store. Queued revalidation applies the same rule, so no extra queue state is saved
   (`DEM-02-C`). The retry costs one more probe of the same site.

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

A future place supplies the adults of the applicant the shared stream offers it
(`households::applicant_offer`), keyed by the building's stable applicant seed (the queued
spawn's id, kept once placed, or a city counter's value for other buildings), by parcel ID while
only proposed, and by place ordinal; an elder applicant supplies none (`DEM-02-C`). Arrivals also shop: each
hour's arrival adults are reduced by `ceil(members × store labor per resident)`. Store labor per
resident is the authored consumption rate of each resource households buy from stores, times the
fewest workers per daily unit among the stores selling it (`workforce::resident_store_labor`).
Individual future homes take whole store workers from a running total in matching order. Ready places are also counted
by those adults. "Unreserved ready homes", which non-worker arrivals and residential
downgrades may use, are the places left after claimed workers fill single-adult places first.
That keeps as many homes reserved as the claims could need. A downgrade retires the
calendar workers of the places it removes. Counting adults costs O(free places) in a parallel
pass over the buildings, which a planning slot runs beside its site preparation (`SIM-02-K`), and
O(1) per claim.

Projects cannot share workers or count homes that finish too late. Immediate level changes may
claim only unemployed adults and already-ready homes.

### Construction queue and execution

Accepted new construction is appended to a minute queue (one start per minute). Queued projects
keep their claims and their prepared door.

Prepared supplier/buyer variants are rows owned by a live building or an available parcel.
Between passes only the owners named by the allocator's building log and zoning's occupancy log
are regenerated; other rows move unchanged, and parcel rows are renumbered when the building
count changes. Registry, zoning-overlay, parcel-layout, lane and CCH changes regenerate every row.
Regenerated rows still reuse entrance geometry only when exact door coordinates, frontage
edge/side, lane revision and CCH generation match. Rows keep the full rebuild's owner/key order,
so candidate order, financing and job-lock expiry are unchanged. Each census's book is merged
and laid out again in four parts in minute 59 of the hour before the one it serves (the pass rows
and the supplier rows' factory at slot 936, the supplier layout at 944, the buyer rows at 953, and
at 957 the business book's finance masks, which the census's financing publication marks for
every buyer, so the residential slot's first business claim refreshes only buyers changed since;
a second half that runs later refreshes them itself), once the
census is done and while the zone index is current, so the residential slot finds its rows
current unless a building or occupancy changed in that last minute. A lending of the same book keeps the live operation and
site rows of the previous one and derives again only the rows of buildings changed or appended
since and those the previous pass overwrote (`SIM-02-K` part 5).

Each planning slot plans directly on the live customer, business and input books of the census
published at the hour's first slot. A later slot first offers the residential slot's homes to
its workforce calendar again and re-proves the earlier slots' business spawns and upgrades in
the single pass's order; one whose re-proof fails is dropped from the hour plan. A slot whose
published books no longer fit the live city plans nothing for its zone and is counted; a
skipped actions slot queues the hour's spawns with voided permits and drops the business level
changes. It opens one first-touch transaction per book, nests each proposal's
ordinary proof inside it, and restores every changed row, link, port mapping and bucket entry when
it closes. Temporary project owners and appended rows are truncated. Buildings appended since the
census's copy join the lent business book as buyers: their input ports are laid out after the
census's live ports and the live port index is rebuilt with them for the pass; the census's own
index returns with the rollback. There is no planner census: under the slot clock the published
census is current (test builds assert it, and `planner_censuses` counts any pass that would not
be). Test builds compare full book fingerprints after every slot.

After the actions slot queues the hour's spawns, it opens one construction epoch on the same
published books, under the same journal: the planner's snapshot (rolling front, live workforce,
queue capacity) and a replay of every constructing building and queued project in queue order,
counting queued homes' applicants as the planner does. A building the census did not see operating
still counts as under construction. Each queued commercial or industrial spawn gets a permit when
its proof funds and staffs it: a digest of its door, with its profile and level. Permits are saved
with the queue. The hour's business level changes are proved in the same epoch and queued with a
permit (their building's asset, level, profile and door); the `level changes` queue applies them
from slot 65, whose share includes slot 64's, a game minute after the actions slot's setup, while
the
building still matches, and the hour's
clearances wait for the `building removals` queue (`SIM-02-J`).

At its start minute a business spawn places and spends its owner equity only with a permit for
its current door, profile and level; otherwise it is rejected and its queue entry retires.
Nothing is proved at the start, and a missing permit always rejects. A home the actions slot
counted that fails to place voids the queue's granted business permits; those spawns wait, in
queue order, until the next hour's actions slot proves them again. Queued homes' applicants are seeded
by the spawn id each keeps once placed (`Building::applicant_seed`); buildings placed outside
the queue take a city counter's next value, so planning and the permits count what a home draws
once placed. Residential spawns need no permit. Construction itself is owned by
[`economy.md`](economy.md#private-construction).

## Level changes

`demand/viability.rs` and `demand/execution/level_change.rs`. Upgrades and downgrades are instant
replacements by the asset one level up or down. Planned at the hour's slots, they are applied by
the `level changes` queue (quota 8, slots 65–959 but the census book's preparation slots 936, 944
and 953, each skipped share
served in the next slot) under a permit (`SIM-02-J`); a change whose
building no longer has the asset, level, profile or (for a business) door it was proved for is
rejected. Level is a wealth tier; a target may hold more
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
company and its cash do not return. Planned clearances are removed by the `building removals`
queue in slots 64–479 of the hour, after checking the site may still be cleared (`SIM-02-J`); the
zone index and entrance cache are repaired once after a slot of clearances.

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
- `households_to_admit_today` and `households_to_remove_today` (departures released today);
- the household exit credit and whether any household was eligible to leave today;
- the households released to leave (the `departures` queue) and queued for a housing review, in
  order, by handle (`household_requests`);
- the level changes (with their permits), building removals and admissions waiting for their
  queues, and the admission credit and rate (save format 104, `SIM-02-J`);
- the upgrade, downgrade and clearance credits per use;
- the cheat flag;
- the pending construction queue;
- the hour plan between its slots (`demand_hour_plan`: upgrades, despawns, optional homes and
  spawns by parcel and building key); a key whose road edge the save drops is skipped;
- the last admission and removal diagnostics (`demand_diagnostics`, save format 104), because a
  load cannot recount what the actions slot saw;
- the setup a planning slot holds between its two halves (`demand_planning_setup`, save format
  106): the workforce, the spawn sites and the building candidates, so a load between a setup and
  its plan plans as the unloaded game does.

Reservation books, the workforce calendar, execution proofs and the other diagnostics are derived.
They are rebuilt after load, never saved.

## Determinism and complexity

- **Ordering:** candidates use deterministic attachment order with stable parcel and building ID
  tie-breaks. Parallel reductions merge in input order, and the applicant stream depends only on
  simulation time.
- **Costs:**
  - snapshots and candidate collection are O(B) parallel passes (the actions slot collects every
    zone's downgrade candidates in one O(B/p) pass, `SIM-02-K` part 5);
  - residential selection is O(candidates);
  - each project check is bounded by its recipe ports, home-calendar size and indexed customer
    and supplier work.
- **No city scans:** there is no per-project city scan or per-agent search. Reservation scratch
  is reused across passes. It is prepared at load and, for the census storage, on a copy of the
  live books made at load, so no planning slot builds it from cold. Row storage that follows the
  city keeps spare capacity, so an appended building does not reallocate it in a planning slot
  (`SIM-02-K` part 5). A pass's appended owner rows are filled on the pool when they number
  16,384 or more. Its
  warm-start estimates count new projects on free lots only; a shortfall replays the proposal
  after growing storage exactly, so estimates never change a result.
