# Metrum Rise — Simulation Engine

This file owns the simulation engine contract: the game clock, the scheduler that decides when
each system runs, entity handles, timed events, snapshot rules, the sim-frame budget and
simulation tiers. Economy and growth rules stay in [`economy.md`](economy.md) and
[`demand.md`](demand.md); traffic movement stays in [`traffic.md`](traffic.md).

**Status:** design target for `SIM-02` (decided 2026-10-15, not implemented). Until each
migration step lands, the current cadence in
[economy.md § Time and cadence](economy.md#time-and-cadence) and
[demand.md § Cadence and handoff](demand.md#cadence-and-handoff) remains authoritative for the
systems it describes.

## SIM-02 scheduler refactor

### Problem

`SimCore::step_authored_minute` runs the whole operational hour at every :00 and the daily tick
plus a second demand pass at 00:00, all inside one `step_frame` while the sim thread holds the
core mutex. Rendering does not block, but no snapshot is published, so agents freeze; missed
frame deadlines are dropped rather than caught up, so the game clock slips as well. The
[economy performance contract](economy.md#performance-contract-perf-01perf-05) measures means
only: at 30k residents the mean hour is about 52 ms (76 ms under fiscal stress) and the mean
daily tick about 35 ms (73 ms), so midnight holds the lock for roughly 85–150 ms. A complete
100k candidate run averaged about 161 ms per hour and 328 ms per daily tick: a half-second
freeze every game midnight.

The same boundaries make the city visibly pulse. Freight carriers, household arrival carriers,
shopper returns and service-visit returns all depart on the hour; construction completes on the
hour; bankruptcies, wages, evictions, job changes and departures all happen at 00:00. Work
commutes (`stable_minute_in_window`) and construction starts (one per minute) are already spread.

Spread evenly, the current 30k work is about 1.3 s of CPU per game day, roughly 0.5 ms per frame
even at 32×. The cost problem is the peak, not the total, at today's sizes.

### Goals

1. No work whose cost grows with the city runs at a fixed calendar boundary.
2. Worst sim frame within the [`PERF-07` budget](#frame-budget-perf-07).
3. Determinism as before: the same save, inputs and frame sequence give the same result,
   independent of Rayon worker count, wall-clock timing and hardware.
4. Scale headroom: the scheduler supports [simulation tiers](#simulation-tiers), so the coarse
   distant-area tier is added as new systems, not another engine refactor.

### Decisions (2026-10-15)

- **Saves break.** The first `SIM-02` save change raises the minimum loadable version to the new
  `SAVE_VERSION` and deletes every legacy `version >=` load branch. Later layout changes bump the
  version and the minimum together; no load branches are added during `SIM-02`.
- **Gameplay changes are accepted.** City income posts continuously instead of at midnight; each
  building and household settles on its own 24-hour window; city-wide aggregates are read up to
  one period old.
- **Generational handles** for buildings, households, agents and shipments.
- **GPU compute is deferred**; see [GPU compute](#gpu-compute).

## Time model

- **Movement** keeps its per-frame step: every sim frame moves agents by `FRAME_DT_S × speed`.
  Agent movement is not scheduled through slots.
- **Slots** are the smallest schedulable unit of operational time: `SLOTS_PER_MINUTE = 16`, so a
  day has 23,040 slots and an hour 960. Slot boundaries are crossed in operational time exactly
  as minutes are today; `TimeSystem` reports crossed slots and the frame runs each in order.
  At 1× a slot is 62.5 ms of real time (about every fourth frame); at 32× one frame crosses about
  8.5 slots. The constant may only grow, and systems never assume its value.
- **Absolute slot** is a `u64` counted from day 1 00:00. All deadlines, events and queue stamps
  use absolute slots.
- **No wall-clock budgets.** No system decides how much to do from elapsed real time. Work per
  slot is fixed by game state alone.

## Entity handles

- A handle is `(index: u32, generation: u32)`. Each entity kind keeps a handle table: per handle
  slot its generation and current dense index, plus a dense-to-handle array. A swap-remove updates
  two entries in O(1). A stale handle resolves to `None`. Freed handle slots are reused in a
  deterministic order, and the free list is saved.
- **Dense indices are valid within one tick.** Anything stored across ticks — queues, phase
  buckets, snapshots, pending requests, cross-entity references and saved references — stores
  handles.
- Per-frame hot loops may keep dense cross-reference fields where the
  [`PERF-06` agent-tick contract](traffic.md#short-agent-tick-contract-perf-06) rejects the extra
  indirection. Each such field keeps the existing remap path and is listed here when decided.
- Saves store handles and the handle tables, so a loaded game resolves the same handles to the
  same entities.

## Scheduler

Every system registers a declaration. The dispatcher runs declarations in one fixed order per
slot; systems never test the minute, hour or day themselves.

A declaration names:

- **Kind**, one of:
  - `EntityPhase { period }` — runs for entities whose phase falls in this slot (hourly or daily).
  - `FixedSlot { period, offset }` — one call at a fixed slot of each period. Allowed only for
    work with a stated O(1), O(zones) or O(dirty) bound, and for re-hosted legacy passes during
    migration.
  - `Events` — consumes this slot's [timed events](#deadlines-and-timed-events).
  - `Queue { quota }` — serves up to `quota` [requests](#request-queues) per slot.
  - `Rolling { period, chunks }` — advances one chunk of a [rolling snapshot](#snapshots).
  - `PureJob { latency }` — see [pure jobs](#pure-jobs).
- **Reads and writes**: the state it touches, so ordering conflicts are visible and independent
  systems can later run together.
- **Cost bound** per slot, in entities or cells.
- **Tier**: which [simulation tier](#simulation-tiers) it serves.

Per-slot order:

1. Timed events due in this slot.
2. `EntityPhase` systems, hourly before daily.
3. `Queue` matchers.
4. `Rolling` chunks, then swaps for periods that ended.
5. `PureJob` launches and due results.
6. On a day boundary only: the calendar close (budget report window), O(zones).

Slices above a size threshold use Rayon with fixed chunks and ordered merges; smaller slices run
serially and give the same result.

### Phases

- Each building and household has a daily phase `d` in `0..23,040`, a stable hash of its seed:
  `parcel_id` and `build_generation` for buildings, the admission seed already used for
  `replenishment_offset_hours` for households. Agents use `schedule_seed`. The hourly phase is
  `d mod 960`.
- Phases are derived, not saved: load recomputes them, as load does for the `ECON-19` offsets.
- `PhaseBuckets` holds handles per daily phase and is maintained by the existing lifecycle
  helpers in O(1) per spawn or removal, like `vacancy_index`. An hourly slot visits the 24 daily
  buckets with the same hourly phase.
- Entities in a slot are processed in ascending handle order, so the order depends on the set,
  not on removal history.
- Cost per slot: O(N/960) for hourly work and O(N/23,040) for daily work. At 1M residents that is
  about 1,000 and 43 households.

### Deadlines and timed events

- **Passive deadlines** replace countdown fields that are only checked when their entity is
  visited: a `*_until_slot: u64` field compared on read. No queue entry, no decrement scan.
- **Timed events** are things that must happen at a given slot: construction start and
  completion, trip departures, carrier arrivals, freight dispatch, shopper and service returns.
  They live in one hierarchical timing wheel keyed by absolute slot, holding `(handle, event)`
  with O(1) amortized insert and pop. Events in one slot fire in insertion-sequence order; the
  sequence counter is saved with the wheel.
- An event whose handle no longer resolves is dropped.

### Snapshots

City-wide values are one of:

- **Incremental counters**, updated by lifecycle helpers and mutators: household and worker
  counts, vacancies, total road length and similar sums. Nothing rebuilds them on a cadence.
- **Rolling snapshots**: a front buffer that readers use and a back buffer being built. Chunk `k`
  of a period folds handle indices `[k·H/C, (k+1)·H/C)`, where `H` is the handle capacity frozen
  at the period start and `C` the chunk count; entities created later are counted next period.
  The back buffer swaps in when the period ends.

Readers only ever see a completed snapshot. This replaces the rule that demand never reads
half-updated economy state. Front buffer, back buffer and cursor are saved, so a loaded game
continues bit-identically.

### Request queues

Order-dependent matching against shared capacity runs as queues: job search, freight requests,
housing, household admission and departure, building level changes and clearances. An entity
enqueues `(enqueue slot, handle, kind)` when its phase or deadline comes up; the matcher serves
requests oldest first, ties broken by handle, up to its quota per slot, against the live indices.
Rate caps such as the daily departure limit accrue per slot through the existing credit helpers
(`demand/credits.rs`). The construction queue (`pending_demand_spawns`) is the precedent and
moves onto the timing wheel.

### Pure jobs

A `PureJob` is a pure function of an immutable snapshot taken at slot `s`; its result is applied
at slot `s + latency`. It runs off the core mutex in a dedicated Rayon pool, so long jobs cannot
starve per-frame movement (the `ROAD-38` lesson), and it never touches `SimCore` (the `SIM-01`
lesson). If the job has not finished by its apply slot, the sim thread waits: results never
depend on completion time. Mutating passes are never pure jobs. Candidates: distance tables,
field diffusion and other read-only precomputation.

### Simulation tiers

Every declaration names its tier. Only `Full` (per-agent state machines) exists today. A coarse
distant-area tier adds its own systems over district aggregates; moving an area between tiers
moves its entities between the tiers' phase buckets.

## Frame budget (`PERF-07`)

- **Metric:** core-mutex hold time per sim frame (`lock_held_ms` in the sim thread), including
  movement, slot work and snapshot build. Reported as max, p99, p99.9, and the count of frames
  over 8 ms and over 16.7 ms, across one full game day.
- **Workload:** the populated-city fixture used by `benchmark_populated_hour_boundaries` and
  `benchmark_populated_midnight_boundaries`, extended to record every frame of the day, at 10k,
  30k and 100k residents, at 1× and 32×.
- **Target:** max ≤ 8 ms at 30k and 100k, at both speeds. 32× at 100k is the binding case: one
  complete 100k candidate run in `economy.md` averaged about 161 ms per hour and 328 ms per daily
  tick, about 4.2 s of CPU per game day, or about 1.6 ms per frame at 32× once spread.
- Matched, unprofiled release runs, recorded here, as for the economy contract. The
  [`PERF-01`–`PERF-05`](economy.md#performance-contract-perf-01perf-05) mean limits still apply.

## GPU compute

Deferred (2026-10-15). The deciding facts were not render contention:

- The expensive work is serial, order-dependent matching over shared capacity — funding census
  claims, wages, transfers, housing, freight reservations — which does not map to GPU threads.
- The road graph is small: about 2,200 nodes at 100k residents and 21,000 at 1M. Batched routing
  already takes 0.2–1 ms on the CPU ([`NET-05`](entrance_and_exit.md#cch-routing-performance-track-net-02net-05)),
  comparable to GPU dispatch and readback.
- WGSL does not guarantee bit-identical floating-point results across devices and drivers. The
  CCH metric is `f32`, and `NET-05` already rejected RPHAST for changing f32 grouping.
- An integrated GPU shares memory bandwidth with the CPU.

A GPU backend may later implement a `PureJob` if it uses integer or fixed-point arithmetic, has a
CPU reference with bit-identical output (used by tests and machines without a usable GPU), and
has enough regular work per dispatch to beat that reference after transfer. Revisit with
`PERF-07` data: candidates are full-tier agent movement at very large in-view populations, and
high-resolution or continuously updated environment fields.

## Migration

Each step lands separately on `main`. The game stays playable after every step.

**Gate for every step:** determinism checks pass (repeat run, save/reload continuation,
different worker counts); economy invariants pass; the behaviour envelope stays in tolerance or
the change is explained; `PERF-07` worst frame is no worse; `PERF-01` means stay in their limits;
`cargo clippy --all-targets` is clean for changed code. Steps marked *identical* must reproduce the
previous step's results exactly.

| Step | Content |
| --- | --- |
| `SIM-02-A` | Guards before code. `PERF-07` benchmark. Determinism checks. Economy invariants: money conservation across treasury, budgets, households and outside-world flows; occupancy, job and reference consistency. The [behaviour envelope](#behaviour-envelope-sim-02-a), recorded on the pre-refactor build. |
| `SIM-02-B` | Save cutover and generational handles; durable references move to handles. *Identical.* |
| `SIM-02-C` | Slot clock, declarations, timing wheel, phases and `PhaseBuckets`. Today's operational hour, hourly demand pass and daily tick re-hosted as `FixedSlot` systems at their current slots; construction starts on the wheel. *Identical.* |
| `SIM-02-D` | Countdowns become passive deadlines, aligned to current boundaries (*identical*); then construction completes at its own slot. |
| `SIM-02-E` | Trips become timed events: arrival departures, freight dispatch, shopper and service returns on completion, slot-granular replenishment phase. Removes the visible bursts. |
| `SIM-02-F` | Per-entity hourly economy moves to `EntityPhase`. |
| `SIM-02-G` | Daily settlement moves to `EntityPhase`; income posts continuously; midnight keeps only the calendar close. |
| `SIM-02-H` | Environment grids become rolling sweeps; cell-lot regeneration becomes a dirty-region queue. |
| `SIM-02-I` | Census and demand become incremental counters and rolling snapshots. The `PERF-03` and `PERF-04` scaling fixes land here, since this step rewrites the census. |
| `SIM-02-J` | Matching becomes request queues. |
| `SIM-02-K` | Legacy `FixedSlot` hour and day systems, `simulate_tick_internal`, `simulate_operational_hour_internal` and every minute-0 path are deleted. `PERF-07` acceptance at 30k and 100k. The cadence sections in `economy.md` and `demand.md` point here. |

### Behaviour envelope (`SIM-02-A`)

Decided 2026-10-15. The envelope defines "the economy still works" while gameplay changes are
allowed: a step that leaves it is investigated before it lands.

- **Run:** the populated-city fixture at 10k and 30k, baseline and fiscal-stress workloads,
  14 game days after its existing two-day warm-up, sampled once per game day.
- **Metrics:** the fixture's `GrowthDayRecord` (households, residents, employed, jobs,
  buildings, treasury, vacant homes, unhoused households, departures), plus bankruptcies, city
  net income, freight deliveries, households out of stock and trip failures.
- **Tolerance:** per metric, the spread across a few fixture variants on the pre-refactor build.
  Trip failures must not rise.
- **Burst limits:** within each game hour, the busiest game minute may hold at most 5% of that
  hour's freight departures, household arrivals or shopping departures, each kind checked
  separately in hours with at least 40 departures of that kind; a perfectly even spread is 1.7%.
  Within each game day, the busiest game hour may hold at most 10% of that day's building and
  household settlements; a perfectly even spread is 4.2%. Before `SIM-02` both shares are 100%.
  Work commutes are excluded, because their morning and evening peaks are authored.
- **Later:** traffic jams, commute times and RCI demand exist in rough form and join the envelope
  when they mature.

### Phase inventory

Every phase of today's operational hour, hourly demand pass and daily tick, and where it goes.

| Today | Cost today | Target | Step |
| --- | --- | --- | --- |
| `publish_frontage_prices` | O(lanes), par | `FixedSlot` hourly | C |
| `advance_construction_hour` | O(buildings); a completion rebuilds the zone index | Completion event; incremental zone-index update | D |
| Farm and mine `produce_hourly` | O(sites) | `EntityPhase` hourly | F |
| `electricity_funding_by_building` | O(buildings) | Rolling snapshot | I |
| `materialize_arrived_household_carriers` | O(agents) scan | Carrier-arrival event | E |
| `rebuild_household_and_worker_counts` | O(agents + households + buildings) | Incremental counters | I |
| `rehouse_unhoused_before_growth` | O(households) when any unhoused | Housing queue | J |
| `run_building_economy` production, `refresh_commercial_activity_floor` | O(buildings) | `EntityPhase` hourly | F |
| `settle_hourly_service_store_sales`, `charge_households_for_aggregate_service_sales` | O(households + buildings), global pool | Pool as rolling snapshot; each household charged at its phase | F, I |
| `run_electricity_hour` | O(households + buildings), one city-wide coverage | Coverage as rolling snapshot; billing at phase | F, I |
| `decrement_building_cooldowns` | O(buildings) | Passive deadline | D |
| `FreightPlanningContext::build` | O(nodes + edges + buildings), rebuilt hourly | Rolling snapshot; component indices cached by routing revision | I |
| `create_profile_input_shipments`, `create_profile_output_exports` | O(buildings), serial reservations | Freight request queue at buyer phase | J |
| `progress_shipments`, `dispatch_ready_carriers` | O(open shipments); all trucks leave on the hour | Dispatch and arrival events | E |
| `progress_households_for_operational_hour` | O(households) | `EntityPhase` hourly | F |
| `apply_starvation_happiness_loss` | O(agents) | At household phase | F |
| `process_active_household_shopping` | O(households) scan | Shopper events | E |
| `process_visible_service_visit_returns` | O(agents) scan | Visit-end event | E |
| `plan_and_apply_household_replenishment` | Indices rebuilt hourly when needed | Restock queue; indices from rolling snapshots | J |
| `plan_and_apply_visible_service_visits` | Capped at 64 per hour | Visit queue with per-slot quota | J |
| Funding census (`job_funding.refresh`) | Superlinear (`PERF-03`/`PERF-04`) | Incremental totals plus rolling snapshot | I |
| Job search | Gated by `job_lock_days` | Job queue on lock deadline | J |
| `sync_agent_money_from_households` | O(agents + households) | At phase, or remove the duplicated field | F |
| `prepare_cell_lots_internal` | O(dirty regions), can spike | Dirty-region queue with quota | H |
| Demand snapshot (`from_runtime_with_funding`) | O(households + buildings + nodes) hourly | Rolling snapshot | I |
| Demand planning (`plan_private_building_actions`) | Scans all parcels hourly | Open question 1 | I |
| Household admission | Bounded; all carriers leave on the hour | Admission queue with credit; departure events | E, J |
| Construction queue | One start per minute | Timed events | C |
| `execute_building_actions` | Plan-bounded | Level-change queue with quota | J |
| `allocator.maintain`, `cleanup_stale_buildings` | O(buildings), serial | Compatibility check at building phase; removals queued | G, J |
| Pollution, noise, desirability | O(grid) plus O(buildings + edges) emission | Rolling grid sweeps | H |
| Daily ledger begin and finish, `reset_daily_ledgers`, `reset_daily_input_accumulators` | O(households), O(buildings) | Each entity rolls its own window at its phase | G |
| `job_lock_days` decrement | O(agents) | Passive deadline | D |
| Bankruptcy, wages, property tax, utilities, profit tax | Serial over buildings and agents | Per building at its daily phase, same internal order | G |
| `pay_household_transfers` | Serial over households until the treasury runs out | Per household at its phase; slot order decides who is paid | G |
| `resolve_household_housing` | Serial over households | Check at phase enqueues housing requests | G, J |
| `workplace_route_cache.clear` | Daily clear | Entries expire by slot stamp | G |
| Road upkeep sum | O(edges) | Incremental road length; upkeep posted per slot | G |
| `run_daily_pass`, `execute_demand_household_removal` | O(households) | Exit check at phase; departures through a credit-paced queue | G, J |
| `finalize_daily_tax_window`, `record_daily_budget_ledger` | Serial over ledgers | Calendar close over running sums, O(zones) | G |
| `agents.daily_update` | O(agents), par | `EntityPhase` daily | G |

### Open questions

1. **Demand planning slicing.** The planner works directly on the live books and rolls them back
   through journals. Options: separate slots per zone type, or per-district planning against the
   rolling snapshot. Decide in `SIM-02-I` with `PERF-07` data.
2. **Hot dense references.** Which agent fields keep dense indices; decide in `SIM-02-B` from the
   `PERF-06` contract.
3. **Pure-job latency and pool size** per job.
4. **Behaviour envelope.** Resolved 2026-10-15; see
   [Behaviour envelope](#behaviour-envelope-sim-02-a).
5. **Electricity stability.** Dispatch against coverage one period old may oscillate; check in
   `SIM-02-F`.
