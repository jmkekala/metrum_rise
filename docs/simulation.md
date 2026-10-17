# Metrum Rise — Simulation Engine

This file owns the simulation engine contract: the game clock, the scheduler that decides when
each system runs, entity handles, timed events, snapshot rules, the sim-frame budget and
simulation tiers. Economy and growth rules stay in [`economy.md`](economy.md) and
[`demand.md`](demand.md); traffic movement stays in [`traffic.md`](traffic.md).

**Status:** `SIM-02` in progress. `SIM-02-A` (guards and pre-refactor baseline), `SIM-02-B`
(save cutover and handles), `SIM-02-C` (slot clock, scheduler, timing wheel and phases),
`SIM-02-D` (passive deadlines and construction completion), `SIM-02-E` (trips as timed
events), `SIM-02-F` (per-entity hourly economy) and `SIM-02-G` (per-entity daily settlement)
are done; see the
[`SIM-02-A` baseline](#sim-02-a-guards-and-pre-refactor-baseline) and the
[step results](#step-results). Until each migration step lands,
the current cadence in
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

**Implemented in `SIM-02-C`** (`simulation/core/scheduler.rs`, `simulation/core/time.rs`): the
slot in progress is `floor(time_elapsed × 16 / seconds_per_minute)` within the current minute,
so it is derived from the saved clock. `TimeSystem::process_delta` reports the crossed range
`start_slot + 1..=end_slot`; the slot in progress has already been processed. A minute's first
slot is crossed exactly when the minute is.

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

**Implemented in `SIM-02-B`** (`simulation/core/handles.rs`):

- `HandleTable<K>` lives beside each store: `BuildingAllocator::building_handles`,
  `HouseholdSystem::handles` and `AgentSystem::handles`. Lifecycle helpers update it on every
  push and swap-remove. A store filled directly (loading, tests) gets handles for its untracked
  trailing entities in dense order the next time the table is read or written, so the ~450
  direct pushes in tests need no change. Freed slots are reused last-freed first. Saves write
  every table to `entity_handle_slots` and `entity_handle_free`.
- **Shipments** keep their stable `u64` id as the handle. Ids are never reused and shipments
  are never swap-removed (`Vec` in id order, `retain` only), so a lookup by id is the
  generation check.
- **References moved to handles:** `Household::shopping_agent` and `Shipment::carrier`, the two
  references other systems hold into the agent store. They replace the dense index plus
  `schedule_seed` guard and the two manual remaps (`remap_shopper_agent_index`,
  `remap_carrier_agent_index`). Undo re-inserts a removed carrier at its old dense index with a
  new handle and re-binds its shipment.
- **Fields that keep dense indices** (open question 2), each with its existing remap path:
  - every agent SoA column (`home_building`, `work_building`, `current_building`,
    `target_building`, `planned_target_building`, `household_id`, the departure and schedule
    caches), because the per-frame movement and schedule passes read them (`PERF-06`);
  - building references held by households (`home_building_id`, `reserved_store_building_id`),
    shipments (endpoints, request-failure keys) and production sites. They move to handles
    when their owning system moves to phases and queues (`SIM-02-E`–`SIM-02-J`);
  - allocator indexes (`zone_index`, vacancy and admission order, chunks, entrances, sites)
    and caches keyed by dense index that are dropped or remapped on every building-reference
    revision (`JobFunding` books, route and workplace caches). These are the owning store's
    own derived state.

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
2. `FixedSlot` systems, in declaration order. Until `SIM-02-K` these include the re-hosted
   operational hour and daily settlement.
3. `EntityPhase` systems, hourly before daily.
4. `Queue` matchers.
5. `Rolling` chunks, then swaps for periods that ended.
6. `PureJob` launches and due results.
7. On a day boundary only: the calendar close (budget report window), O(zones).

Slices above a size threshold use Rayon with fixed chunks and ordered merges; smaller slices run
serially and give the same result.

**Implemented in `SIM-02-C`.** `Schedule` (`simulation/core/scheduler.rs`) holds the
declarations in dispatch order and rejects one that breaks the contract: a fixed-slot offset
outside its period, a fixed-slot system with neither a bounded cost nor legacy status, or an
events system that does not cost its events. The simulation's list and its per-slot dispatch
are in `nodes/sim/core/schedule.rs`: timed events; then the operational hour (`FixedSlot`,
hour, offset 0) and the daily settlement (`FixedSlot`, day, offset 0), both re-hosted
unchanged. The `sim` debug channel prints the list with costs, tiers, reads and writes.

**Implemented in `SIM-02-F`.** `EntityPhase { period }` systems cost `Phase`, the entities of
one phase, and the schedule rejects any other cost for them. Three hourly ones follow the
re-hosted hour in every slot (`nodes/sim/core/entity_phases.rs`): `building_hours`,
`household_hours` and `agent_hours`. A caller that steps a whole operational hour without the
slot clock (tests and tools) runs the pooled hour and then every phase in order.

**Implemented in `SIM-02-G`.** Three daily `EntityPhase` systems follow the hourly ones:
`building_days`, `household_days` and `agent_days` (`nodes/sim/core/entity_days.rs`). The
calendar close is a `FixedSlot` costing `Constant` (O(1) or O(zones), which the schedule now
accepts for fixed slots). It is declared after the operational hour, so in each day's slot 0 it
runs after the 00:00 hour, which still belongs to the closing day, and before any daily phase
of the new day: every entity has settled exactly once since the previous close. Pollution,
noise and desirability remain a legacy midnight pass (`environment grids`), declared before the
hour, until `SIM-02-H`. A `Rolling { period }` kind costs one `Chunk` per slot
and runs after the phases; the rolling road length is its first system. Tools and tests that
step a whole day run the environment grids, every entity's daily phase in order, then the
close.

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
- **Implemented in `SIM-02-C`** with one change: the seed of every entity is its handle. The
  daily phase is `stable_hash64(index << 32 | generation) mod 23,040`
  (`simulation/core/phases.rs`). Each `HandleTable` buckets its live handles, so the buckets
  change exactly when a handle is allocated or freed, including for entities a test or loader
  appended directly; a restored table rebuilds them.
- **Implemented in `SIM-02-F`:** slot `s` visits the handles of hourly phase `s mod 960`.
  - **Buildings:** recipe production and its input consumption, and farm and mine output.
  - **Households:** consumption, utilities and the hour's ledger entries, and the shopping-leg
    timeout.
  - **Agents:** the happiness cost of an empty household's pantry.
  - **Kept at the hour:** the passes that decide over a city-wide pool or census, which
    `SIM-02-I` turns into rolling snapshots. These are the funding census and the activity
    floor it sets, service-store sales, electricity dispatch, the employed-adult counts, and
    restock and visit planning (`SIM-02-J` queues).
  - **Pooled reads:** a pooled pass reads what the phases of the previous hour left. The hourly
    electricity pass bills each building's activity from its own phase, and production reads
    the activity floor the last hour set. Both values are now saved.
  - **Census first:** the funding census now opens the hour, right after the membership
    counts, so the activity floor, store sales, restock and visit planning and hiring all read
    a census of the state the previous hour's phases left. A census rebuilt on load therefore
    never feeds the simulation.
- **Implemented in `SIM-02-G`:** slot `s` also visits the handles of daily phase `s mod 23,040`.
  Each entity settles its own 24-hour window there instead of at midnight.
  - **Buildings:** the zoning check and rezoning grace; then, in the order of
    [`economy.md § Building Bankruptcy`](economy.md#building-bankruptcy), the bankruptcy check,
    property tax, water, sewage and electricity bills, distress liquidation and business profit
    tax. Its daily accumulators then roll into the `recent_*` values.
  - **Households:** transfers, residential property tax, water, sewage and electricity bills;
    the ledger window then closes and the next one opens. The household also queues a housing
    review and, if it may leave, claims its share of the day's exit budget.
  - **Agents:** the wage, paid while the workplace has paid fewer than its funded jobs in its
    own window, and the daily happiness update.
  - **Served by the next hour:** housing reviews and exits, which need the member links the
    hourly membership census builds (`SIM-02-J` turns them into per-slot queues).
  - **Reads one period old:** wages read the funded jobs the last hourly census copied onto
    each building. Water and sewage coverage is whether an operational provider settled in the
    last closed day, and providers' revenue shares come from that day's receipts.
  - **City totals:** every payment also posts into the calendar day's totals
    (`CityDayTotals`, `CityInputDay`). The close keeps them as the last day for the budget
    report in O(1), and daily road upkeep reads the rolling road length.

### Deadlines and timed events

- **Passive deadlines** replace countdown fields that are only checked when their entity is
  visited: a `*_until_slot: u64` field compared on read. No queue entry, no decrement scan.
- **Timed events** are things that must happen at a given slot: construction start and
  completion, trip departures, carrier arrivals, freight dispatch, shopper and service returns.
  They live in one hierarchical timing wheel keyed by absolute slot, holding `(handle, event)`
  with O(1) amortized insert and pop. Events in one slot fire in insertion-sequence order; the
  sequence counter is saved with the wheel.
- An event whose handle no longer resolves is dropped.
- **Implemented in `SIM-02-C`** (`simulation/core/timing_wheel.rs`): a ring with one bucket per
  slot for the next game day, plus an ordered overflow map for later events, which move into the
  ring as it advances. Saved as `timed_events (slot, seq, kind, a)` with `scheduler_state`
  (next slot, next sequence, next spawn id). The construction queue's events are
  `ConstructionStart { id }`. The queue keeps the spawn payloads in order, so demand planning
  still reads it, and each queued spawn carries the id its event names. A spawn dropped from the
  queue (a loading repair) leaves an event that fires and does nothing. When the queue is
  replaced wholesale (undo, a new world) or the clock is set directly (tests, tools), its events
  are rescheduled: each spawn at its due minute, at most one per minute after the current one,
  as the per-minute queue check started them. A save stores the wheel as the next frame will see
  it.
- **Implemented in `SIM-02-D`:**
  - Passive deadlines: a building's freight cooldown is `shipment_cooldown_until_tick`, compared
    with `ShipmentSystem::hour_ticks`, and an agent's job lock is `job_lock_until_day`, compared
    with `AgentSystem::settlement_days`. Each counter advances where the old decrement scan ran,
    so results are unchanged. Both counters are saved in `scheduler_state`. The fields become
    slots when their systems move to phases (`SIM-02-E`, `SIM-02-J`).
  - Construction completes at `construction_complete_slot`: its start slot plus its duration ×
    960. Placement stamps the deadline from `BuildingAllocator::now_slot` (the slot in progress)
    and lists `(handle, slot)`. The simulation turns that list into `ConstructionComplete
    { building }` events after each slot and before each frame; a save includes any not yet
    turned. The event finishes the building in place, with no zone-index rebuild: when the
    indices are clean, a sorted insert into its zone list, a vacancy push and one recorded
    production change. Undoing a bulldoze re-inserts the building with a new handle and
    re-lists its deadline.
  - The demand workforce calendar counts every site as ready at the first hour boundary at or
    after its completion, whether placed (`Building::construction_hours_left`), queued or
    proposed, so a project's promise matches the hour its building actually opens.
- **Implemented in `SIM-02-E`** (`nodes/sim/core/trips.rs`): trips are timed events. The hourly
  passes still choose who sets out: the logistics hour loads each new shipment's cargo, the
  household hour reserves purchases and service visits, and admission launches arrival carriers.
  Nobody leaves on the hour. Each pass holds its trips and lists them; at the end of the pass
  `SimCore::schedule_trip_departures` spaces every kind evenly over the following hour
  (`paced_slot`: the k-th of n leaves at `slot + 1 + k·959/n`, so no minute holds more than
  n/60 + 2). The events are `FreightDispatch`, `ShopperDeparture`, `VisitDeparture` and
  `ArrivalDeparture`, saved as kinds 2, 4, 6 and 8.
  - A held agent keeps its planned trip and waits behind the replan gate every departure
    already obeys (`next_replan_time` = `f32::MAX`), so neither the work schedule nor another
    planner takes it. Loading keeps the hold. A freight carrier is spawned only at its slot;
    its cargo already left the source on the hour (`Shipment::cargo_loaded`).
  - From departure on, a trip is checked until it arrives: a minute after it sets out, then
    after a quarter of the agent's time on the road, from one to eight minutes
    (`TRIP_CHECK_SLOTS`, `TRIP_CHECK_MAX_SLOTS`):
    `FreightCheck` delivers at the destination and finishes back at the source; `ShopperCheck`
    picks up at the store, collecting VAT then, and delivers at home; `ArrivalCheck` turns the
    carrier into a household. A service visit ends an hour after the visitor set out
    (`VisitReturn`), as visits that all left on the hour ended at the next one.
  - Removed hourly scans: `materialize_arrived_household_carriers` (every agent),
    `process_visible_service_visit_returns` (every agent) and the arrival half of
    `progress_shipments` and `process_active_household_shopping`. Pending arrivals are
    recounted from `AgentSystem::arrival_carriers`, O(carriers in flight).
  - Leg timeouts still count operational hours in the hourly passes; they move with the systems
    that own them (`SIM-02-F` for households, `SIM-02-J` for freight).
  - A wheel restarted because the clock was set directly keeps its trips: an overdue one fires
    at the next slot.

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
- **Benchmark:** `frame_budget::perf07_frame_budget`, run for every tier and speed by
  `tools/sim02_gate.py perf07`, pinned with `--cpus` to one thread per performance core; see
  [`SIM-02-A`](#sim-02-a-guards-and-pre-refactor-baseline).
  The maximum varies by about 20% between runs of one build, the p99 by about 10%, so "no
  worse" allows 25% on the maximum and 10% on the p99.

**Pre-refactor baseline** (2026-10-15, `SIM-02-A` release `$TEST_BINARY`,
`RAYON_NUM_THREADS=8`, one run each on an i9-12900K). Milliseconds; "boundary" frames ran an
operational hour; movement and snapshot are the largest single values of those phases:

| Tier | Speed | Max | p99 | p99.9 | Frames > 8 ms | > 16.7 ms | Mean | Boundary max | Other max | Movement max | Snapshot max |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 10k | 1× | 214.1 | 7.81 | 10.22 | 718 / 86,376 | 9 | 2.73 | 214.1 | 23.0 | 4.5 | 13.3 |
| 10k | 32× | 266.7 | 10.28 | 21.77 | 42 / 2,700 | 9 | 2.97 | 266.7 | 26.0 | 4.5 | 8.5 |
| 30k | 1× | 636.3 | 8.94 | 11.20 | 2,151 / 86,376 | 42 | 3.79 | 636.3 | 54.5 | 17.5 | 11.0 |
| 30k | 32× | 637.4 | 33.71 | 74.13 | 332 / 2,700 | 41 | 5.45 | 637.4 | 48.4 | 13.8 | 10.8 |
| 100k | 1× | 2,546.6 | 20.30 | 31.97 | 86,376 / 86,376 | 7,183 | 12.65 | 2,546.6 | 247.5 | 85.2 | 17.8 |
| 100k | 32× | 2,342.9 | 178.89 | 452.29 | 2,700 / 2,700 | 817 | 21.27 | 2,342.9 | 265.6 | 94.6 | 16.8 |

The largest frame of every run is the first operational hour after loading the snapshot (see
the per-frame rows of the [phase inventory](#phase-inventory)). At 100k even an ordinary frame
exceeds the budget: about 5 ms of movement plus about 9 ms of render snapshot at 1×. Meeting
`PERF-07` therefore needs the movement and snapshot work listed there, not only the scheduler.

**Pinned reference** (2026-10-16, the same `SIM-02-A` `$TEST_BINARY` run with
`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14`, one thread per performance core,
`RAYON_NUM_THREADS=8`). Unpinned, the mean frame of one behaviour at 10k 32× ranged from 2.89
to 3.22 ms between runs on this hybrid CPU, as threads move to efficiency cores; pinned runs of
three builds stayed within 2.60–2.75 ms. Steps from `SIM-02-D` on compare against this:

| Tier | Speed | Max | p99 | p99.9 | Mean |
| --- | --- | --- | --- | --- | --- |
| 10k | 1× | 200.2 | 6.74 | 8.70 | 2.36 |
| 10k | 32× | 197.7 | 9.95 | 19.44 | 2.69 |
| 30k | 1× | 514.8 | 7.42 | 9.35 | 3.19 |
| 30k | 32× | 507.8 | 29.13 | 57.03 | 4.49 |
| 100k | 1× | 2,019.2 | 16.91 | 30.00 | 11.49 |
| 100k | 32× | 2,020.5 | 174.06 | 418.32 | 19.59 |

`PERF-01` on the same build (`tools/perf_contract.py --binary "$TEST_BINARY"`, 70.6 s): mean
hour / day 22.13 / 7.96 ms at 10k and 78.04 / 62.12 ms at 30k; fiscal stress 41.84 / 23.55 ms
and 105.58 / 88.42 ms. Later steps compare against these.

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
  Work commutes are excluded, because their morning and evening peaks are authored. From
  `SIM-02-E` each departure counts in the minute its trip event ran, and an `arrivals`
  workload (30k, every tenth household leaves after the build) supplies the hours with 40
  arrivals that the full fixtures never reach.
- **Later:** traffic jams, commute times and RCI demand exist in rough form and join the envelope
  when they mature.

### `SIM-02-A` guards and pre-refactor baseline

Done 2026-10-15. Every later step reruns these checks; each run gets its own `$ARTIFACTS`
directory, so the `PERF-07` snapshots are regenerated by the engine under test:

```bash
cd rust && cargo test --release --lib --no-run          # prints $TEST_BINARY
RAYON_NUM_THREADS=8 python3 tools/sim02_gate.py envelope determinism perf07 \
    --binary "$TEST_BINARY" --artifacts "$ARTIFACTS"     # --identical on identical steps
./run.sh --perf-contract                                  # PERF-01 means
```

- **Envelope** (`populated_city::envelope::sim02_envelope`). Saves break during `SIM-02`, so
  every run builds a fresh city with the engine under test; the fiscal-stress workload sets its
  property taxes after the warm-up. Fourteen game days run frame by frame, with one `SIM02_DAY`
  sample per day: the `GrowthDayRecord`, net income (treasury change), desertions, freight
  deliveries, households with an empty pantry, trip failures, building placements and
  removals, recorded outside-world money flows, burst shares and a semantic fingerprint. The
  baseline, its tolerance bands, fingerprints and burst shares are in
  `tools/sim02_envelope_baseline.json`.
- **Tolerance bands.** Variants at 0.97×, 1.00× and 1.03× of each tier's residents. Metrics are
  normalised per resident, household or building. A band is the variant range widened by half
  the range, by at least 2% of the base value, and by an absolute floor for rare counts. Trip
  failures have only an upper bound.
- **Burst counters** (test-only `debug::work` counters, compiled out of the game): freight
  carriers dispatched, household arrival carriers launched, shoppers dispatched with a reserved
  purchase, and buildings and households passed through daily settlement. A count is attributed
  to the game minute its frame ends in.
- **Money conservation.** A test-only ledger (`debug::money`) records every flow across the city
  boundary: owner capital, immigrant cash, outside sales, imported inputs and carrier fees, the
  outside share of utility bills, construction, road upkeep, and the cash of removed buildings
  and departing households. Each game hour, the change of the treasury, business and household
  cash and the two escrows (a shopper's reserved purchase, a paid shipment travelling to a
  building) must equal the recorded flows. The treasury enters as an exact `f64` difference, so
  the fixture's large treasury does not hide small residuals.
- **Reference and occupancy invariants,** checked every game hour: every agent, household and
  shipment reference names a live building, household or agent; household homes match their
  members; a carrier and its shipment name each other; occupancy equals homed households plus
  pending arrivals and stays within capacity; worker counts equal assigned workers.
- **Determinism** (`sim02_determinism`, 10k): three days after the build, compared hourly by
  a semantic fingerprint (plain values only, independent of entity storage) and finally by an
  order-insensitive hash of every saved table. Checks: a repeat run, Rayon pools of 1 and 3
  threads, and a save at 13:37 between frames, reloaded and run 26 game hours beside the straight
  run.
- **`PERF-07`** (`frame_budget::perf07_frame_budget`). Loads the perf-contract snapshot (day 12,
  00:00), sets a 2 km × 2 km camera box at the city centre and times the step, stale cell
  preparation and render snapshot of every frame of one game day. Godot's upload
  acknowledgements run between frames, outside the timing.

**Envelope baseline** (fresh, release `$TEST_BINARY` of this step, four parallel runs with
`RAYON_NUM_THREADS=6`), 14-day means of the 1.00× variant:

| Workload | Employment | Households / resident target | Net income / resident / day | Desertions / 1k buildings / day | Deliveries / 1k buildings / day | Empty pantries | Trip failures / 1k residents / day |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Baseline 10k | 67.5% | 0.533 | 13.70 | 0.14 | 175.5 | 7.0% | 0 |
| Baseline 30k | 67.2% | 0.506 | 13.12 | 0.37 | 173.9 | 22.9% | 0 |
| Fiscal stress 10k | 5.2% | 0.531 | 28.72 | 14.7 | 14.4 | 77.0% | 0.04 |
| Fiscal stress 30k | 5.2% | 0.506 | 29.94 | 15.2 | 13.9 | 77.8% | 0.12 |

- **Bursts.** In every qualifying hour the busiest minute holds 100% of freight dispatches and
  of shopper dispatches, and the busiest settlement hour holds 100% of a day's settlements.
  These cities are full, so arrivals never reach 40 per hour and the arrival limit is not
  exercised; `SIM-02-E` adds an arrival workload.
- **Invariants.** No reference, occupancy or job violation. The money residual stays under 0.7
  per game hour, except in baseline 30k, which loses about 40 per day at the daily settlement:
  income, property and profit taxes are summed in `f32` (`FiscalRevenue`) before they reach the
  treasury, and at 30k (about 222,000 income tax a day, `f32` step 1/64) the rounding loses
  about 1e-4 of the tax. This predates `SIM-02`; `SIM-02-G` rewrites settlement. The gate
  limit is 1.5× the baseline residual of the same workload, plus 5.
- **Determinism.** All four checks identical.
- **Save fix.** The 100k snapshot's exact-continuation check found that loading cleared the
  next plan (`planned_target_building`) of a car saved at a node before its first edge, so the
  reloaded city diverged within an hour. The load repair now keeps the plan when the agent
  replans from the node, as the running game does.

### Step results

Each step's gate, run with the commands of [`SIM-02-A`](#sim-02-a-guards-and-pre-refactor-baseline)
on a fresh `$ARTIFACTS` directory.

#### `SIM-02-B` — save cutover and handles (2026-10-16)

- **Save format 87**, the only version that loads. Every legacy `version` branch and the
  version-57 lane restore are deleted. The one committed save fixture
  (`benchmarks/fixtures/kuopio-terrain`, version 59, roads and terrain only) was re-saved with
  the `SIM-02-A` build; its original road tables (nodes, edges, edge geometry, lane connections
  and saved lanes, with zero frontage delays as the old loader assumed) were copied back so
  loading still repairs its rejected junction, and the two renamed reference columns and the
  handle tables were added by hand. `placements.json` pins the new file hash.
- **Identical:** all four envelope workloads reproduce the `SIM-02-A` fingerprints for every
  one of the 14 days (`tools/sim02_gate.py envelope --identical`); determinism checks pass.
- **`PERF-07`** (release `$TEST_BINARY`, `RAYON_NUM_THREADS=8`, `--perf07-baseline` against
  `SIM-02-A`): every tier within noise. Max / p99 in ms, `SIM-02-A` in parentheses: 10k 1×
  264.8 / 7.59 (214.1 / 7.81), 10k 32× 264.6 / 10.92 (266.7 / 10.28), 30k 1× 540.0 / 8.93
  (636.3 / 8.94), 30k 32× 600.4 / 32.04 (637.4 / 33.71), 100k 1× 2,646 / 19.70 (2,547 / 20.30),
  100k 32× 2,043 / 178.51 (2,343 / 178.89).
- **`PERF-01`**, two alternating runs per build (hour / day means in ms, `SIM-02-A` first):
  baseline 10k 22.1–25.0 / 8.0–12.2 against 19.6–22.0 / 8.0–8.5; baseline 30k 68.5–78.0 /
  54.0–62.1 against 71.8–75.7 / 59.0–67.3; fiscal 10k 34.0–41.8 / 23.6–32.4 against
  39.0–41.7 / 27.2; fiscal 30k 85.6–105.6 / 70.2–88.4 against 94.0–102.0 / 73.6–77.3. One
  build varies by up to 50% between runs, so `SIM-02-B` is inside the `SIM-02-A` spread on
  every workload.
- **Tests:** 2,425 release library tests and `./run.sh --test` pass; clippy is clean. New: the
  handle table (stale handles, displacing undo inserts, save round trip) and a bulldoze undo
  that re-binds a restored carrier.

#### `SIM-02-C` — slot clock, scheduler, timing wheel and phases (2026-10-16)

- **Save format 88**: `scheduler_state`, `timed_events` and a `spawn_id` per queued spawn. The
  Kuopio fixture gained the empty tables (no queued spawns) and its hash in `placements.json`.
- **Identical:** all four envelope workloads reproduce the `SIM-02-A` fingerprints for every
  day; determinism checks pass.
- **`PERF-07`** against `SIM-02-A`, within noise on every tier. Max / p99 in ms: 10k 1× 263.1 /
  7.86, 10k 32× 215.1 / 11.67, 30k 1× 663.6 / 9.29, 30k 32× 581.9 / 30.05, 100k 1× 2,433 /
  19.87, 100k 32× 2,594 / 176.94. Iterating slots costs nothing measurable: the frames that
  dominate are unchanged.
- **`PERF-01`**, two runs (hour / day means in ms): baseline 10k 21.1–23.4 / 11.8–13.0,
  baseline 30k 71.9–74.1 / 62.0–62.8, fiscal 10k 34.5–37.3 / 29.3–37.9, fiscal 30k 96.1–106.3 /
  81.3–90.6. Two daily means lie slightly above the `SIM-02-A` spread in one run each; they time
  the daily settlement alone, which this step re-hosts without change.
- **Tests:** 2,430 release library tests and `./run.sh --test` pass; clippy is clean. New: the
  timing wheel (order across both levels, save round trip), phase buckets (each live handle
  once per day and per hour, in handle order), declarations (dispatch order) and the slot clock
  (contiguous slots, minute starts).

#### `SIM-02-D` — passive deadlines and construction completion (2026-10-16)

- **Part 1, identical:** freight cooldowns and job locks become passive deadlines (save format
  89). All four envelope workloads reproduce the `SIM-02-C` fingerprints every day;
  determinism checks pass.
- **Part 2:** construction completes at its own slot (save format 90; the Kuopio fixture gained
  the renamed column and its hash in `placements.json`).
- **Envelope** (`tools/sim02_gate.py envelope determinism --binary "$TEST_BINARY"`): within the
  bands on every workload. The largest shift is 0.07 of a half-band (30k final employment rate
  0.6522 → 0.6510); fiscal stress at 10k is unchanged. Money residuals are as before (30k
  baseline 39.8, the `f32` tax sums of `SIM-02-A`). The populated fixtures are full, so the
  measured days build few sites. Burst shares are unchanged: freight and shopping still leave
  in one minute and settlements fall in one hour, until `SIM-02-E` and `SIM-02-G`.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days).
- **`PERF-07`**, pinned (`--cpus 0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`) against the
  [pinned `SIM-02-A` reference](#frame-budget-perf-07). Max / p99 / mean in ms: 10k 1× 193.9 /
  6.90 / 2.42, 10k 32× 198.8 / 11.22 / 2.73, 30k 1× 501.6 / 7.45 / 3.19, 30k 32× 504.4 / 26.60
  / 4.53, 100k 1× 1,985.6 / 16.87 / 11.44, 100k 32× 2,008.2 / 158.80 / 19.99. Means are within
  3% of `SIM-02-A` on every tier, and every maximum is lower except 10k 32× (+0.6%). The first,
  unpinned run failed the 10k 32× p99; see
  [Decided while unattended](#decided-while-unattended).
- **`PERF-01`**, two pinned runs alternating with `SIM-02-A` (hour / day means in ms, `SIM-02-A`
  first): baseline 10k 16.9–17.1 / 7.6–8.2 against 17.3–17.4 / 9.0–9.3; baseline 30k
  62.1–62.6 / 47.3–48.2 against 62.2–62.2 / 43.3–43.6; fiscal 10k 27.6–28.0 / 22.3–22.9 against
  27.7–28.0 / 22.1–22.2; fiscal 30k 78.5–78.5 / 67.7–68.4 against 78.3–78.9 / 68.0–69.9. The
  same ten limits fail as on `SIM-02-A` and nothing new fails. The 10k baseline day rose 1.1 ms
  but stays inside its 10 ms limit: its fixture city, grown with the new construction timing,
  runs 48% more supplier searches in the funding census (`PERF-03`/`PERF-04`), while the 30k
  city runs fewer (day −9%).
- **Tests:** 2,429 release library tests, the integration tests and `./run.sh --test` pass;
  clippy is clean. New or changed: completion updates a clean index in place and matches a
  rebuild; completion records one production change; render progress follows the slot clock;
  queued deadlines count hour boundaries to the start; a completed home rehouses its residents
  in the next hour.

#### `SIM-02-E` — trips as timed events (2026-10-16)

- **Save format 91:** `shipments.cargo_loaded` and the trip event kinds 2–9. The Kuopio fixture
  gained the empty column and its hash in `placements.json`.
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts departures
  --binary "$TEST_BINARY"`): busiest-minute departure shares are 2.1–3.3% for freight, shopping
  and arrivals on every workload (limit 5%; 100% before). The arrivals workload has 2 days with
  qualifying arrival hours, worst 3.2%. Settlements still fall in one hour until `SIM-02-G`.
  Every band holds except one near-zero value: fiscal stress at 10k ends with 0.52% of residents
  employed against a band of 0.36–0.39%, about 55 instead of 40 people in the collapsed city.
  The largest in-band shift is the 30k baseline out-of-stock rate, 0.229 → 0.262 (+0.92 of a
  half-band). Money residuals are unchanged and no invariant is violated.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical.
- **`PERF-07`**, pinned against the pinned `SIM-02-A` reference. Max / p99 / mean in ms: 10k 1×
  199.3 / 6.13 / 2.42, 10k 32× 197.7 / 8.65 / 2.85, 30k 1× 495.0 / 7.31 / 3.26, 30k 32× 500.8 /
  24.57 / 4.97, 100k 1× 1,928.7 / 18.04 / 12.12, 100k 32× 1,944.7 / 173.57 / 22.09. Every
  maximum is at or below `SIM-02-A`, and the largest movement step halves at 30k and 100k
  (100k 32×: 86.7 → 38.9 ms) because departures no longer arrive together. The 32× means rose
  6–13%: the 100k fixture is gridlocked, and trip checks plus changed traffic timing touch most
  of the city (phase inventory). Checking every minute had cost 23.6 ms at 100k 32× before the
  back-off.
- **`PERF-01`**, two pinned runs alternating with `SIM-02-A` (hour / day means in ms, `SIM-02-A`
  first): baseline 10k 17.0–17.2 / 7.5–8.1 against 16.7–16.7 / 8.8–8.8; baseline 30k 61.8–62.3 /
  46.8–47.5 against 52.6–52.7 / 41.7–44.9; fiscal 10k 27.5–27.7 / 22.3–22.8 against 27.8–28.2 /
  22.2–23.0; fiscal 30k 78.6–78.9 / 68.2–68.6 against 79.1–80.1 / 74.6–75.0. Eight limits fail
  against ten on `SIM-02-A`, all of them failing before. The fiscal 30k day rose 6 ms. Its
  household settlement is unchanged (17.4 against 18.1 ms), and the minute-0 demand pass's
  cell-lot preparation took 13.8 against 8.6 ms. That work follows the regions dirtied by the
  day's building removals in the collapsing city, the spike `SIM-02-H` queues.
- **Tests:** 2,434 release library tests, the integration tests and `./run.sh --test` pass;
  clippy is clean. The first `./run.sh --test` run failed `road_junction_preview_test` on
  asynchronous preview refetching; the unchanged rerun passed. New: paced departure spread,
  trip event save round trip, departure spacing in the wheel, trips surviving a clock set
  directly, and departure holds surviving the load repair.

#### `SIM-02-F` — per-entity hourly economy (2026-10-16)

- **Save format 92:** the agent `money` column is gone, and buildings save
  `hourly_economy_activity` and `commercial_activity_floor_scale`. The Kuopio fixture gained
  the two columns (no buildings) and its hash in `placements.json`.
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts departures
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): busiest-minute departure shares are
  2.0–3.3% for freight, shopping and arrivals on every workload (limit 5%). The arrivals
  workload has 2 days with qualifying arrival hours, worst 3.2%. Settlements still fall in one
  hour until `SIM-02-G`. Every band holds except the one near-zero value `SIM-02-E` also left:
  fiscal stress at 10k ends with 0.24% of residents employed against a band of 0.36–0.39%,
  about 25 instead of 40 people in the collapsed city (55 on `SIM-02-E`). The values closest
  to a band edge are fiscal stress at 10k, deliveries 14.42 → 13.12 per 1,000 buildings a day
  (0.95 of the way to the lower edge; 0.81 on `SIM-02-E`), and the 30k baseline out-of-stock
  rate, 0.229 → 0.256 (0.50 of the way; 0.61 on `SIM-02-E`). Money residuals are unchanged
  (30k baseline 40.6) and no invariant is violated.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical
  (10k, three days), and the 100k snapshot continues identically after loading. Two reload
  divergences were found and fixed on the way; see
  [Decided while unattended](#decided-while-unattended).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`,
  `RAYON_NUM_THREADS=8`), pinned against the pinned `SIM-02-A` reference. Max / p99 / mean in
  ms: 10k 1× 196.1 / 6.09 / 2.38, 10k 32× 203.5 / 8.19 / 2.90, 30k 1× 527.4 / 7.73 / 3.34, 30k
  32× 532.6 / 27.47 / 5.22, 100k 1× 1,952.5 / 18.68 / 12.06, 100k 32× 1,932.6 / 181.94 / 23.35.
  Every maximum is still an hour boundary and within the gate's noise of `SIM-02-A` (30k 32×
  +4.9%); both 100k maxima are below it. The worst frame away from a boundary is the
  construction start one minute after it (30k 1×: 65.8 ms, 60.8 of it the placement; 48.5 on
  `SIM-02-A`), which `SIM-02-K` splits. The 32× means rose 1–6% over `SIM-02-E` (100k 22.09 →
  23.35): an entity's hour now runs in its own slot, about N / 960 entities, too few to split
  across workers, where the pooled passes ran in parallel. The 1× means are unchanged.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 17.1–17.5 / 7.6–8.2 against 16.6–17.3 / 7.4–8.0;
  baseline 30k 63.5–63.9 / 47.2–48.4 against 56.5–58.6 / 45.8–48.9; fiscal 10k 27.9–28.1 /
  22.9–23.7 against 27.4–30.0 / 22.3–25.3; fiscal 30k 80.0–80.3 / 69.2–70.9 against 78.1–80.2 /
  76.4–76.5. Eight limits fail against ten on `SIM-02-A`, all of them failing before. The
  fiscal 30k day stays about 6 ms above `SIM-02-A`, as on `SIM-02-E`.
- **Tests:** 2,435 release library tests, the integration tests and `./run.sh --test` pass;
  clippy is clean. New: each household's hour runs once per hour, at its own phase and no
  other. The save/reload continuation test covers the two new building columns.

#### `SIM-02-G` — per-entity daily settlement (2026-10-17)

- **Save format 93:** buildings save `paid_workers_in_window`, `funded_job_capacity` and
  `unpaid_cash_wages_in_window`;
  households save `paid_workers_in_window`; the electricity day state saves the running and last
  day totals, the coal stock and the city input day; `household_requests` holds the queued
  housing reviews and exits by handle; `scheduler_state` saves the rolling road-length sweep; the
  demand state saves whether a household was eligible to leave today; the workplace quote cache
  saves the pass that added each quote (`WRC2`). The Kuopio fixture gained the columns, an empty
  `WRC2` quote cache and its hash in `placements.json`.
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): busiest-hour
  settlement shares are 7.1–7.9% on every workload (limit 10%; 100% before). At 32× a frame
  crosses about 32 minutes and its settlements count in the hour it ends in, so an even spread
  reads up to about twice 1/24. Busiest-minute departure shares are 2.0–3.3% (limit 5%). Money
  residuals are at most 0.9 an hour (30k baseline 40.6 on `SIM-02-F`; see the fixture treasury
  decision) and no invariant is violated. Out of band:
  - Fiscal stress, employment rate 0.077 at 10k and 0.079 at 30k against about 0.052. The
    collapse after the tax rise now spreads over day 5 as each building settles at its own phase
    (3,291 employed at that day's sample at 10k against 10 on `SIM-02-F`), so one of the 14
    samples still counts a third of the jobs.
  - Fiscal stress, final employment rate 0.58% at 10k and 0.74% at 30k against 0.36–0.39% and
    0.15–0.34%: about 60 and 220 people still working in the collapsed cities instead of about 40
    and 60, the near-zero value earlier steps also left out of band.
  - Fiscal stress at 10k, deliveries 12.74 per 1,000 buildings a day against a low edge of 13.05
    (13.12 on `SIM-02-F`, 13.31 on `SIM-02-E`): the drift of earlier steps continues.
  - Three first-build differences were found and fixed (see
    [Decided while unattended](#decided-while-unattended)): unhoused households in fiscal
    stress (unemployment benefit now follows paid wages); businesses that never went bankrupt
    (baseline 30k: 6 bankruptcies against 113 on `SIM-02-F`, now 67; final employment 0.630,
    below its band, now back in it); and two households in the year-long producer town whose
    shopper was stranded in a store after an eviction.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`,
  `RAYON_NUM_THREADS=8`), pinned against the pinned `SIM-02-A` reference. Max / p99 / mean in
  ms: 10k 1× 192.2 / 6.29 / 2.43, 10k 32× 192.6 / 8.33 / 2.95, 30k 1× 498.6 / 7.37 / 3.34, 30k
  32× 500.4 / 26.85 / 5.28, 100k 1× 1,970.1 / 19.41 / 12.09, 100k 32× 1,948.5 / 185.93 / 21.90.
  Every maximum is below `SIM-02-A`. The 00:00 frame no longer stands out: at 30k it falls out of the eight worst
  (75.9 ms on `SIM-02-F`, 45.5 of it the daily tick), and the worst frame away from an hour
  boundary fell at every tier (30k 1× 65.8 → 43.6 ms, 100k 1× 268.6 → 200.9 ms). The worst
  frame at 30k and 100k is 01:00, the first hour after the snapshot loads, which rebuilds the
  derived economy caches (`SIM-02-K` in the inventory). The gate failed one p99, 100k 1× 19.41
  against a limit of 19.10. A paired rerun of that tier with `SIM-02-F` right after it gave
  18.06 against 17.59 (means 11.67 and 11.53): each build moved by more than the 0.47 ms
  between them from one run to the next, so the gap is noise. The 32× means are 10–18% above
  `SIM-02-A`: within 2% of `SIM-02-F` at 10k and 30k, and 6% below it at 100k.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 17.5–17.7 / 7.4–7.9 against 15.6 / 0.2; baseline
  30k 62.9–63.5 / 47.7–48.0 against 56.2–56.4 / 0.7; fiscal 10k 27.9–28.2 / 22.1–22.6 against
  28.6–28.7 / 0.2; fiscal 30k 79.4–81.7 / 69.5–70.0 against 86.6–86.7 / 0.7–0.8. The day is now
  the calendar close, so the three daily time limits `SIM-02-A` failed pass; eight limits fail
  against ten. One is new: baseline `buyer_sites_built` scaling (1,202 at 10k against 7,039 at
  30k, limit 3.42×). The 30k count is unchanged (7,076 on `SIM-02-A`); the 10k city rebuilds its
  buyer sites once a day instead of five times, because it has almost no failed businesses.
  Fiscal 30k's hour is 6 ms (7%) above `SIM-02-A`: its census takes 11.3 against 5.9 ms a call
  and its planning 24.7 against 18.9 ms, over the same parcel scan and fewer census buildings, in
  a collapsing city whose workers now leave at their own phases. The cause is not isolated;
  `SIM-02-I` rewrites both passes. Over a whole day fiscal 30k costs 5% more and baseline 30k
  14% less.
- **Build identity:** envelope, determinism and tests ran on the committed build. `PERF-07` and
  `PERF-01` ran on the build before the last fix, which changed only the road-length sweep's sum
  (whole millimetres) and its saved edge positions.
- **Tests:** 2,442 release library tests and the integration tests pass on the final build, and
  `./run.sh --test` (isolated profile) passed on the build before the insolvency rule, which
  touches no bridge code; clippy is clean. Its first run hung after `agent_inspector_test` lost
  its debug car once; the unchanged test passed in the rerun and ten more runs, and G adds no
  path that removes an agent outside a household. New: the rolling road length sums the network over one day; exits claimed at
  daily phases keep the daily fraction and limit; workplace quotes expire a day of passes after
  they were added; adults not paid a wage in the window draw unemployment benefit; an employer
  that cannot pay wages two windows running deserts; evicting a
  household hands its shopper at a store back to the schedule and rehousing sends it home; a
  road-length sweep resumes identically on the edges a save keeps. The
  save round trip covers the day totals, coal stock, city input day, request queues and window
  counts.

### Phase inventory

Every phase of today's operational hour, hourly demand pass and daily tick, and where it goes.

| Today | Cost today | Target | Step |
| --- | --- | --- | --- |
| `publish_frontage_prices` | O(lanes), par | `FixedSlot` hourly | C |
| `advance_construction_hour` | O(buildings); a completion rebuilds the zone index | Completion event; incremental zone-index update | D (done) |
| Farm and mine `produce_hourly` | O(sites) | `EntityPhase` hourly, at the site's building | F (done) |
| `electricity_funding_by_building` | O(buildings) | Rolling snapshot | I |
| `materialize_arrived_household_carriers` | O(agents) scan | Carrier-arrival event | E (done) |
| `rebuild_household_and_worker_counts` | O(agents + households + buildings) | Incremental counters | I |
| `sync_agent_money_from_households` | O(agents) | Agent `money` mirror deleted (never read); the employed-adult count it carried stays hourly | F (done), I |
| `rehouse_unhoused_before_growth` | O(households) when any unhoused | Housing queue | J |
| `run_building_economy` production, `refresh_commercial_activity_floor` | O(buildings) | Production at the building phase; the floor follows the funding census | F (done), I |
| `settle_hourly_service_store_sales`, `charge_households_for_aggregate_service_sales` | O(households + buildings), global pool | Pool as rolling snapshot; each household charged at its phase | I |
| `run_electricity_hour` | O(households + buildings), one city-wide coverage | Coverage as rolling snapshot; billing at phase. Since F it bills the activity of each building's phase | I |
| `decrement_building_cooldowns` | O(buildings) | Passive deadline | D (done) |
| `FreightPlanningContext::build` | O(nodes + edges + buildings), rebuilt hourly | Rolling snapshot; component indices cached by routing revision | I |
| `create_profile_input_shipments`, `create_profile_output_exports` | O(buildings), serial reservations | Freight request queue at buyer phase | J |
| `progress_shipments`, `dispatch_ready_carriers` | O(open shipments); all trucks leave on the hour | Dispatch and arrival events; hourly timeouts until J | E (done), J |
| `progress_households_for_operational_hour` | O(households) | `EntityPhase` hourly | F (done) |
| `apply_starvation_happiness_loss` | O(agents) | At the agent phase | F (done) |
| `process_active_household_shopping` | O(households) scan | Shopper events; leg timeouts at the household phase | E (done), F (done) |
| `process_visible_service_visit_returns` | O(agents) scan | Visit-end event | E (done) |
| `plan_and_apply_household_replenishment` | Indices rebuilt hourly when needed; since F a parallel O(households) scan finds whether any household can restock | Restock queue; indices from rolling snapshots | J |
| `plan_and_apply_visible_service_visits` | Capped at 64 per hour | Visit queue with per-slot quota | J |
| Funding census (`job_funding.refresh`) | Superlinear (`PERF-03`/`PERF-04`) | Incremental totals plus rolling snapshot | I |
| Job search | Gated by the job lock (`job_lock_until_day`) | Job queue on lock deadline | J |
| `prepare_cell_lots_internal` | O(dirty regions), can spike | Dirty-region queue with quota | H |
| Demand snapshot (`from_runtime_with_funding`) | O(households + buildings + nodes) hourly | Rolling snapshot | I |
| Demand planning (`plan_private_building_actions`) | Scans all parcels hourly | Open question 1 | I |
| Household admission | Bounded; all carriers leave on the hour | Admission queue with credit; departure events | E (departures done), J |
| Construction queue | One start per minute | Timed events | C (done) |
| `execute_building_actions` | Plan-bounded | Level-change queue with quota | J |
| `allocator.maintain`, `cleanup_stale_buildings` | O(buildings), serial | Compatibility check at building phase; removals queued | G (done), J |
| Pollution, noise, desirability | O(grid) plus O(buildings + edges) emission | Rolling grid sweeps | H |
| Daily ledger begin and finish, `reset_daily_ledgers`, `reset_daily_input_accumulators` | O(households), O(buildings) | Each entity rolls its own window at its phase | G (done) |
| `job_lock_days` decrement | O(agents) | Passive deadline | D (done) |
| Bankruptcy, wages, property tax, utilities, profit tax | Serial over buildings and agents | Per building at its daily phase, same internal order; wages at each worker's phase | G (done) |
| `pay_household_transfers` | Serial over households until the treasury runs out | Per household at its phase; slot order decides who is paid | G (done) |
| `resolve_household_housing` | Serial over households | Check at phase enqueues housing requests, served by the next hour | G (done), J |
| `workplace_route_cache.clear` | Daily clear | Entries expire 24 passes after they were added | G (done) |
| Road upkeep sum | O(edges) | Rolling road-length sum; upkeep charged at the calendar close | G (done) |
| `run_daily_pass`, `execute_demand_household_removal` | O(households) | Exit check at phase; departures through a credit-paced queue | G (done), J |
| `finalize_daily_tax_window`, `record_daily_budget_ledger` | Serial over ledgers | Calendar close over running sums, O(zones) | G (done) |
| `agents.daily_update` | O(agents), par | `EntityPhase` daily | G (done) |

Per-frame costs found by the `SIM-02-A` `PERF-07` baseline, outside the boundary passes:

| Today | Cost today | Target | Step |
| --- | --- | --- | --- |
| Render snapshot (`build_snapshot_reusing`) | 3–12 ms: a lane pose for every agent before camera culling, serial | Cull before posing; bucket in parallel | K |
| Agent movement in departure frames | Up to 4 ms at 10k and 17 ms at 30k (32×) | Departures spread by events; `PERF-06` limits | E, K |
| One queued construction start | 14–21 ms for a single placement | Placement split across slots | K |
| First boundary after loading | About 10× an ordinary hour: derived economy caches rebuild | Rebuilt during loading | K |
| 100k fixture traffic (found in `SIM-02-E`) | About 45,000 of 145,000 agents moving at any time (30,000 on lanes, 15,000 in junctions) and 15,000 shopping trips in flight around the clock, on `SIM-02-D` too: the network is saturated, so movement and trip checks run for most of the city | Find why trips stall at 100k (junction throughput, routing); movement within `PERF-06` limits | K |

### Open questions

1. **Demand planning slicing.** The planner works directly on the live books and rolls them back
   through journals. Options: separate slots per zone type, or per-district planning against the
   rolling snapshot. Decide in `SIM-02-I` with `PERF-07` data.
2. **Hot dense references.** Resolved in `SIM-02-B`; see
   [Entity handles](#entity-handles).
3. **Pure-job latency and pool size** per job.
4. **Behaviour envelope.** Resolved 2026-10-15; see
   [Behaviour envelope](#behaviour-envelope-sim-02-a).
5. **Electricity stability.** Dispatch against coverage one period old may oscillate. Checked in
   `SIM-02-F`: no loop exists yet. Coverage is still computed in one hourly pass from the
   activity of the previous hour's phases, and production never reads coverage, only billing
   and the daily meters do. The question returns in `SIM-02-I`, when coverage becomes a
   rolling snapshot.

### Decided while unattended

Decisions taken during the unattended `SIM-02` run (from 2026-10-15), each with the
alternatives and the evidence. The user reviews these on return.

- **`SIM-02-A` — `PERF-01` gate.** Ten `PERF-01` time and scaling limits already failed before
  `SIM-02` (latest report: 10k daily 10.99 ms against 10 ms, 30k daily 46.9 ms against 30 ms,
  six scaling counts). "Within limits" cannot be met by a refactor that does not touch those
  costs, so a step passes when it does not regress against the `SIM-02-A` `PERF-01` run beyond
  noise. Alternative: fix `PERF-03`–`PERF-05` first; rejected because the user parked them on
  2026-10-14 and `SIM-02-I` rewrites the census anyway.
- **`SIM-02-A` — envelope from fresh cities and size variants.** Saves break, so each step builds
  its own city. The tolerance comes from cities at 0.97×, 1.00× and 1.03× of the tier's
  residents: they differ in layout and phase, not in rules, which is the natural spread the
  envelope asked for. Alternatives: different start times (no layout spread), or repeated runs
  (identical by determinism).
- **`SIM-02-A` — money residual limit.** The pre-refactor build already loses about 40 per day at
  30k to `f32` tax sums. The limit is 1.5× the baseline residual of the same workload plus 5,
  instead of zero (always failing) or a fixed number (blind to a large leak at 10k).
- **`SIM-02-A` — arrival bursts.** The populated fixtures are full, so no hour has 40 arrivals.
  The arrival limit is checked from `SIM-02-E` on with a workload that admits households;
  adding it now would only record 100%.
- **`SIM-02-A` — `PERF-07` workload.** The day after the perf-contract snapshot (day 12), a 2 km
  street-level camera box, and Godot acknowledgements outside the timing. The first frames
  after loading are included: a player loads saves too.
- **`SIM-02-A` — determinism tier.** 10k, because the fixture cannot house 5,000 residents (its
  400 m cross-street spacing leaves too few lots).
- **`SIM-02-A` — save repair fix.** Fixed in this step rather than worked around: the `PERF-07`
  benchmark needs the 100k snapshot, and the determinism contract requires exact continuation.
- **`SIM-02-B` — which references became handles.** Converted: the shopper and carrier
  references, the only ones other systems keep into the agent store. They needed a seed guard
  and two hand-written remaps. Kept dense, with their remap paths: agent SoA columns (read
  by the per-frame movement pass, `PERF-06`), and the building references of households,
  shipments and sites. Alternative: convert those building references now (about 500 lines in
  50 files, half of them tests) at a step that must stay identical, with no behavioural gain;
  they move when `SIM-02-E`–`SIM-02-J` rewrite the systems that own them.
- **`SIM-02-B` — direct store pushes.** Tests push about 450 entities straight into the
  stores. The handle table registers untracked trailing entities in dense order instead of
  requiring every push to go through a helper; the order is the one the helpers would give.
- **`SIM-02-B` — fixture conversion.** The version-59 Kuopio terrain fixture was converted once
  rather than keeping any legacy loader: re-saved with the `SIM-02-A` build, with the original
  road tables restored (a plain re-save stores the load-time junction repair, which one test
  exists to exercise), two columns renamed and empty handle tables added. It has no buildings,
  households, agents or shipments.
- **`SIM-02-C` — phase seeds are handles.** The spec named `parcel_id` + `build_generation` for
  buildings and the admission seed for households. Explicit sites have no parcel (id 0), and
  the household admission seed is not saved, so both would need new fields or fallbacks. A
  handle is unique among live entities, lasts the entity's life and is saved, so one hash
  covers every kind. Agents may still use `schedule_seed` when they get phases.
- **`SIM-02-C` — wheel shape.** One ring bucket per slot for a game day plus an ordered overflow
  map, instead of several power-of-two levels: almost every event (trips, returns, dispatches)
  is due within a day; later ones (construction completion) are few, and moving them costs
  O(log F).
- **`SIM-02-C` — construction queue.** The spawn payloads stay in the ordered queue that
  demand planning reads; the wheel holds `ConstructionStart { id }`. Alternative: payloads in
  the wheel, which would make demand planning iterate the ring every hour.
- **`SIM-02-C` — clock set directly.** Many tests set `SimCore::time` by hand. The frame
  re-aligns the wheel when its next slot does not follow the clock, and a save stores the
  aligned wheel, which reproduces the per-minute queue check exactly. A live game never takes
  this path.
- **`SIM-02-D` — deadlines against boundary counters.** The spec names `*_until_slot` fields.
  The freight cooldown counts operational hours and the job lock counts settlements; tests
  and tools call the hour and day passes directly without moving the clock, so a slot
  deadline would change results at a step that must stay identical. The deadlines compare
  with counters that advance exactly where the decrement scans ran. They become slot
  deadlines when `SIM-02-E` and `SIM-02-J` move their systems off the boundaries.
- **`SIM-02-D` — a completion updates the indices in place.** Alternatives: rebuild the zone
  index on every completion (O(buildings) per event instead of once an hour), or mark it stale
  (the next construction start pays the rebuild, and the hourly demand pass reads candidate
  lots instead of vacancies while it is stale). The zone list takes a sorted insert, so it
  matches a rebuild; the vacancy list takes a push at the end, as a spawned building does.
- **`SIM-02-D` — one production change per completion** instead of discarding the change
  history, which forced every production-input consumer into a full rebuild.
- **`SIM-02-D` — workforce calendar basis.** Construction now takes exactly its duration. The
  hour-boundary countdown it replaces counted the start hour as a whole hour, so sites
  finished up to an hour early (half an hour on average). Queued and proposed projects kept
  the old start offset while placed sites read their true deadline, so the calendar dated
  them one hour earlier than the buildings they become. With that mismatch, one balance case
  (`many_new_industrial_lots_respect_scarce_labor_and_earn_their_owner_capital`) funded a
  factory whose fourth worker never arrived. Every site now counts as ready at the first hour
  boundary at or after its completion, and the case passes. Alternative: keep durations one hour shorter to reproduce the old timing;
  rejected because the authored durations are what the player sees.
- **`SIM-02-D` — bulldoze undo kept stale building handles** (latent since `SIM-02-B`, when
  nothing yet held a building handle across ticks). The restored building now takes a new
  handle through `insert_displacing`, as carriers already did, and re-lists its construction
  deadline.
- **`SIM-02-D` — `PERF-07` runs pinned to performance cores.** The unpinned `SIM-02-D` gate run
  failed one limit, the 10k 32× p99, at 12.20 ms against 11.81 ms. Alternating unpinned
  runs of `SIM-02-D` part 1 and part 2 gave 11.24–11.74 and 12.19–12.34 ms. A profile showed
  even load-only work 32% slower in one run, with samples on efficiency cores: on the hybrid
  i9-12900K the scheduler moves threads between core types. Pinned to one thread per
  performance core (`--cpus 0,2,4,6,8,10,12,14`), three alternating runs each gave
  9.58–9.88 ms for part 1, 10.23–10.80 ms for part 2 and 9.89–10.28 ms for `SIM-02-A`, with
  means within 4% of each other. `SIM-02-A` was re-measured pinned as the reference for
  later steps. Alternative: widen the p99 allowance; rejected, because it would hide real
  per-frame regressions.
- **`SIM-02-E` — departures paced, not phased.** The step names a slot-granular replenishment
  phase. Random per-entity phases do not meet the burst limit near its 40-departure floor: 50
  freight departures in an hour, hashed over 60 minutes, put 3 or 4 in the busiest minute
  (6–8%) in almost every hour. The hourly passes still decide
  who sets out; their trips leave at evenly spaced slots over the following hour, in the order
  the pass listed them. Measured busiest-minute shares: 2.1–3.3% for freight, shopping and
  arrivals. The per-household decision moves to phases with the rest of the hourly economy
  (`SIM-02-F`).
- **`SIM-02-E` — arrival found by checks that back off.** The movement pass works on raw slices
  under `PERF-06`; reporting arrivals from it would mean new `unsafe` code. Each trip is checked
  from its departure on, O(1) per check. Checking every minute cost too much at 100k, where the
  fixture is gridlocked: about 23,000 trips are in flight at any time, most of them stuck. That
  was 1,450 checks a slot and raised the 32× mean frame from 20.4 to 23.6 ms. The wait now grows
  with the agent's time on the road (a quarter of it, from one to eight minutes), so an on-time
  arrival settles within minutes and a stuck trip is checked every eight. Alternatives: a callback
  from the movement pass (new `unsafe`), or checks at the estimated arrival (congestion moves it,
  so it still needs polling).
- **`SIM-02-E` — timeouts stay hourly.** Freight `queued_hours` and the shopping leg allowance
  still count operational hours in the hourly passes, so timeouts and their tests keep their
  meaning. A trip that leaves later in the hour gets up to one hour more allowance than one that
  left on the hour.
- **`SIM-02-E` — a hold is the replan gate.** A held agent keeps its planned trip with
  `next_replan_time = f32::MAX`, which every departure already checks, so no new agent column
  is needed. Loading reset that gate for every agent inside a building, which released held
  trips early after a reload. The load repair now keeps a hold.
- **`SIM-02-E` — freight cargo loads on the hour.** The planning hour still takes the cargo from
  the source, as dispatch at the hour did; only the truck waits for its slot. Leaving the cargo
  at the source until the slot would let shoppers or production consume reserved stock first.
  "Dispatched" is now an explicit `cargo_loaded` flag instead of a set carrier handle.
- **`SIM-02-E` — a service visit ends an hour after departure.** Visits used to end at the next
  hourly scan after leaving on the hour; now they end at the same distance from their own
  departure.
- **`SIM-02-E` — pending arrivals from a carrier registry.** The hourly scan of every agent also
  rebuilt the pending-arrival counts. Those now come from the carriers launched and not yet
  arrived, recounted each hour in O(carriers in flight). Incremental counting was rejected:
  eviction and home removal clear a carrier's household on several paths.
- **`SIM-02-E` — starter-town lane heights.** With arrivals settled within a minute, the starter
  town's save/reload balance check diverged: its lanes were never fitted to the road surface,
  while loading fits them, which changed lane lengths and arrival times. This is the same
  incomplete fixture setup found earlier for the populated city. Both fixtures now call one
  `SimCore::fit_lanes_to_road_surface`. Live road commits fit lanes through render preparation
  on the simulation thread.
- **`SIM-02-E` — departures counted in their own minute.** The burst tracker attributed every
  counter change in a frame to the minute the frame ended in, so a frame crossing an hour moved
  the previous hour's last departures into the next hour's first minute (7.4% at 30k).
  Departure counters now record the minute the clock was processing.
- **`SIM-02-E` — arrival workload.** Every tenth household leaves after the build; at 30k the
  following day has 13 hours with at least 40 arrivals. At 10k no hour reaches 40, because
  admission spreads the newcomers over the day. The workload is checked for invariants, money
  conservation and bursts; it has no pre-refactor band.
- **`SIM-02-F` — pooled passes stay on the hour.** Service-store sales and electricity dispatch
  divide one city-wide supply among all buyers each hour. Splitting them by phase would sell
  or dispatch against a pool that later phases have not reached yet. They stay in the hourly
  pass until `SIM-02-I` gives them rolling snapshots. The same holds for the activity floor,
  which the funding census sets, and the employed-adult counts. Alternative: per-phase slices
  of each pool, which changes who is served first and belongs with the snapshot design.
- **`SIM-02-F` — the hour boundary keeps whole hours.** The pooled pass runs before phase 0
  in its slot, so at every :00 each entity has finished exactly one phase since the previous
  :00. The hourly demand pass therefore never sees one household two hours on and another
  none. Electricity at :00 bills the activity that production recorded at each building's phase
  in the hour before, instead of the activity of the same pass.
- **`SIM-02-F` — starvation at the agent phase.** The inventory names the household phase.
  Households hold no list of their members: the old pass scanned every agent. At the agent
  phase each agent reads its own household's pantry in O(1). Alternative: a member index on
  households, maintained on every join, leave, eviction and remap.
- **`SIM-02-F` — agent `money` deleted.** The hourly agent pass copied each household's budget
  share into an agent column that nothing read (save, clamp, envelope fingerprint only). The
  column and the copy are gone (save format 92); the pass now only counts employed adults, which
  the census still reads hourly. Alternative: move the copy to the agent phase, keeping
  O(agents) of dead work per hour.
- **`SIM-02-F` — the activity floor and hourly activity are saved.** Production at a phase
  reads the floor the last hour set, and the electricity pass reads the activity recorded at
  the phases. Both used to be set and read inside one pass, so loading reset them to zero. The
  save/reload continuation test caught the difference (a store's stock after reload). The
  service staffing policy also re-derived the floor from the census, and on load it runs
  before any census, so it set every floor to zero. Its own staffing limits never read the
  floor, and it no longer touches it. Alternative: recompute both on load; rejected
  because the load-time census is not the census that set them.
- **`SIM-02-F` — the funding census opens the hour.** The census ran late in the hour, and
  the next hour's floor refresh and its restock and visit planning (the customer book) read
  it before the next census. Loading rebuilds the census from the saved state, but the phases
  now change that state between the census and any save, so a reloaded city read different
  census results. The 100k `PERF-07` snapshot check caught it: a store's stock differed
  13 hours after reloading, and the treasury differed by 1.1. Moving the census to the start
  of the hour, before every reader, makes the reloaded and straight runs identical (a
  diagnostic run from reloads at an hour boundary and mid-hour matched for 1,200 frames). The floor
  production reads during an hour now comes from that hour's census, one hour fresher.
  Alternative: save the census results, including the customer book; rejected as a large
  new save surface for state that is derived each hour.
- **`SIM-02-G` — wages at each worker's phase.** The inventory names the building's phase, but
  buildings hold no list of their workers. Each worker is paid at its own daily phase while its
  workplace has paid fewer than its funded jobs since the workplace's own phase
  (`paid_workers_in_window`, saved). An overstaffed store or work area releases the workers
  whose phases come after its funded jobs were paid, where midnight kept the lowest agent IDs.
  Alternative: a worker index per building, maintained on every hire, quit, removal and remap.
- **`SIM-02-G` — wages read funded jobs copied onto the buildings.** The hourly census writes
  each building's funded jobs to `funded_job_capacity`, which is saved, because wages now read
  the census between hours and a load rebuilds the census from a later state (the `SIM-02-F`
  lesson). Alternative: save the census.
- **`SIM-02-G` — unemployment benefit for adults not paid in the window.** A household's
  unemployed adults are its adults minus the members paid a wage in its own 24-hour window
  (each member's wage falls once in any such window; the count is saved). Counting the employed
  adults of the latest hourly census instead, the first build left a worker of a failing employer
  without wage or benefit for one or two windows: on F the employer paid its last wages before
  the tax that sank it, and its workers drew benefit at the same midnight. In fiscal stress at 10k
  that cost 478 evictions over the two days after the collapse (unhoused rate 0.0266 against a
  base of 0.0018, and departures at 30k); paid-wage counting brought it to 80 evictions and an
  unhoused rate of 0.0014. When the treasury runs dry, slot order decides who is paid instead of
  household ID. Alternatives: count members' jobs live at the phase (needs a member index), or
  release workers at their first unpaid wage (loses the workers of a building that recovers).
- **`SIM-02-G` — water and sewage coverage one day old.** Coverage was whether any provider
  operated at midnight. It is now whether one settled operational during the last closed day,
  and providers' revenue shares are that day's receipts. Alternative: count providers
  incrementally on every staffing change.
- **`SIM-02-G` — housing reviews and exits served by the next hour.** Relocation and eviction
  move every member, and only the hourly membership census builds member links. A household's
  phase queues a review, and the next hour serves the queue in order with the census's links and
  one vacancy planner. An exit check at the phase adds `persistent_exit_daily_fraction` to the
  exit credit; a whole unit releases the household, at most 48 per calendar day, and the next
  hour removes it if it is still eligible. Departures therefore go in slot order, not
  lowest-reserve first. Alternative: a member index maintained on every membership change.
- **`SIM-02-G` — zoning grace at the building's phase.** The compatibility check and its
  three-day grace run per building at its phase. A removal there repairs the derived caches
  once for the slot, as the midnight maintenance did. Alternative: a timed event at the grace
  deadline, which would also need a check after every edit.
- **`SIM-02-G` — city totals post as they happen.** The budget report summed household ledgers
  and building accumulators at midnight. Each payment now also posts into the calendar day's
  totals: transfers, wages, utility and electricity bills, city input purchases at payment, coal
  burned in each hourly dispatch, and the coal stock after the latest dispatch. The close keeps
  them in O(1); the headless growth report reconciles to them.
- **`SIM-02-G` — road upkeep from a rolling sum.** Upkeep summed every edge at midnight. A
  rolling sweep folds 1/23,040 of the edges per slot, and the close charges the last finished
  sweep; a world without one measures its network once at its first close. Alternative: a
  maintained total, which every road mutation would have to update. The sweep sums whole
  millimetres and saves its frozen edge count and next edge in saved edge numbering (a save
  drops deleted edges and keeps the rest in order), so a game saved mid-sweep resumes on the
  same live edges with the same total; a float sum would round differently once a save regroups
  the remaining edges into slots. The `SIM-02-H` grid work found the problem; the 10k
  determinism city has no deleted roads, so its reload check could not.
- **`SIM-02-G` — workplace quotes expire per entry.** The midnight clear of the commute quote
  cache is replaced by expiry 24 passes (hours) after each quote was added, through an
  insertion-order queue, so the cache still holds about one day's searches. Stamps are saved.
- **`SIM-02-G` — 00:00 is an ordinary hour.** The daily demand pass and the second hiring round
  after settlement are gone; 00:00 runs the ordinary hour, its demand pass included. The hourly
  employed-adult count already covered the daily one.
- **`SIM-02-G` — fixture treasury 1e10.** The populated-city fixture started with a treasury of
  1e15 so that city services never run out. At that size an f64 holds only multiples of 0.125, so
  every treasury posting rounded: per-worker income tax and per-entity bills leaked about 360 a
  day at 10k, and money conservation failed at 10k and in fiscal stress (hourly residual up to 42
  against a limit near 6). A 100k city spends about 1e6 a day, so 1e10 still never runs out and
  resolves 2e-6; every treasury-dependent rule in the fixture (admission's transfer coverage,
  transfer cash limits) stays saturated. The 30k residual fell from 40.6 an hour on `SIM-02-F`
  to 0.9, so the `SIM-02-A` residual at 30k came from the same rounding in the midnight pass. Alternatives: batch treasury postings per slot
  (still rounds, and hides per-entity flows), or widen the residual limit (would hide real
  leaks).
- **`SIM-02-G` — an evicted shopper goes home.** With housing reviews at any time of day,
  evictions now catch shoppers away from home. The eviction cleared the shopper's planned return
  but left it in the shopping activity, which the schedule skips, so it stayed in the store for
  good and its household never shopped again (the year-long producer town had two such
  households). The eviction now hands the shopper back to the schedule, and rehousing gives a
  member away from home and work a trip to the new home after its current one, since the
  schedule only moves agents at home or at work. Alternative: defer reviews of households with a
  shopper out, which would leave the latent bug in place.
- **`SIM-02-G` — wages read the census of the worker's hour.** Workers above a building's funded
  jobs go unpaid and quit after two days. Each worker's wage now reads the funded jobs the census
  of its own hour copied onto the building, where the midnight payroll read one census for all.
  Fixing a window's funded jobs at its first wage instead brought baseline 30k final employment
  back into band (0.635 against 0.630) but put fiscal stress far out (employment rate 0.147
  against 0.052, deliveries 20.6 against 14.4 per 1,000 buildings a day, trip failures out of
  band): a failing employer kept paying jobs the census no longer funded. The hourly copy stays.
- **`SIM-02-G` — unpaid wages make a window insolvent.** On `SIM-02-F` the midnight payroll sold
  output to pay every worker just before the bills, so an employer that could not pay ended the
  day negative and deserted the next. With wages at each worker's phase and production refilling
  stock between them, the distress sale at the building's phase always covered the bills: at 30k
  baseline about 100 businesses were in distress each day, none had a negative budget, and
  bankruptcies fell from 113 to 6 over the 14 days (21 to 0 at 10k); insolvent businesses lived
  on by fire sales while their unpaid workers quit, and with no failed sites to rebuild, the
  10k and 30k performance snapshots had no building change in their next day. A window is now
  also insolvent when a wage went unpaid for lack of cash after selling output (counted per
  building and saved); two insolvent windows in a row desert the business. Bankruptcies are 67
  at 30k, 77 in the arrivals workload (116 on F) and 3 at 10k, and baseline 30k final
  employment returned into band (19,434 workers against 19,489 on F). Alternatives: pay all of
  a building's workers at its phase (needs a worker index per building), or bill and test
  distress at each wage (moves the bills off the building's window).
- **`SIM-02-G` — the snapshot's building-change check runs at 30k.** The performance snapshot
  accepts a cache only if its continuation day changes a building, so the reload check covers
  the incremental tables keyed by building revision. At 10k the healthy city now loses 3
  businesses in 14 days (21 on `SIM-02-F`): wages at each worker's phase let marginal
  businesses pay as revenue comes in. Its day-12 snapshot changed no building, so the check now
  runs at 30k only, where 12–15 businesses fail a day around that day. Alternatives: a later
  snapshot day (changes every `PERF-01` and `PERF-07` workload), or a building removal injected
  into the continuation check (exercises an edit, not the economy's own churn).
