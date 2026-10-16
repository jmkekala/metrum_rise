# Metrum Rise — Simulation Engine

This file owns the simulation engine contract: the game clock, the scheduler that decides when
each system runs, entity handles, timed events, snapshot rules, the sim-frame budget and
simulation tiers. Economy and growth rules stay in [`economy.md`](economy.md) and
[`demand.md`](demand.md); traffic movement stays in [`traffic.md`](traffic.md).

**Status:** `SIM-02` in progress. `SIM-02-A` (guards and pre-refactor baseline), `SIM-02-B`
(save cutover and handles), `SIM-02-C` (slot clock, scheduler, timing wheel and phases) and
`SIM-02-D` (passive deadlines and construction completion) are done; see the
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
  appended directly; a restored table rebuilds them. Nothing reads them yet: the first
  `EntityPhase` system arrives in `SIM-02-F`.

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
  Work commutes are excluded, because their morning and evening peaks are authored.
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

### Phase inventory

Every phase of today's operational hour, hourly demand pass and daily tick, and where it goes.

| Today | Cost today | Target | Step |
| --- | --- | --- | --- |
| `publish_frontage_prices` | O(lanes), par | `FixedSlot` hourly | C |
| `advance_construction_hour` | O(buildings); a completion rebuilds the zone index | Completion event; incremental zone-index update | D (done) |
| Farm and mine `produce_hourly` | O(sites) | `EntityPhase` hourly | F |
| `electricity_funding_by_building` | O(buildings) | Rolling snapshot | I |
| `materialize_arrived_household_carriers` | O(agents) scan | Carrier-arrival event | E |
| `rebuild_household_and_worker_counts` | O(agents + households + buildings) | Incremental counters | I |
| `rehouse_unhoused_before_growth` | O(households) when any unhoused | Housing queue | J |
| `run_building_economy` production, `refresh_commercial_activity_floor` | O(buildings) | `EntityPhase` hourly | F |
| `settle_hourly_service_store_sales`, `charge_households_for_aggregate_service_sales` | O(households + buildings), global pool | Pool as rolling snapshot; each household charged at its phase | F, I |
| `run_electricity_hour` | O(households + buildings), one city-wide coverage | Coverage as rolling snapshot; billing at phase | F, I |
| `decrement_building_cooldowns` | O(buildings) | Passive deadline | D (done) |
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
| Job search | Gated by the job lock (`job_lock_until_day`) | Job queue on lock deadline | J |
| `sync_agent_money_from_households` | O(agents + households) | At phase, or remove the duplicated field | F |
| `prepare_cell_lots_internal` | O(dirty regions), can spike | Dirty-region queue with quota | H |
| Demand snapshot (`from_runtime_with_funding`) | O(households + buildings + nodes) hourly | Rolling snapshot | I |
| Demand planning (`plan_private_building_actions`) | Scans all parcels hourly | Open question 1 | I |
| Household admission | Bounded; all carriers leave on the hour | Admission queue with credit; departure events | E, J |
| Construction queue | One start per minute | Timed events | C (done) |
| `execute_building_actions` | Plan-bounded | Level-change queue with quota | J |
| `allocator.maintain`, `cleanup_stale_buildings` | O(buildings), serial | Compatibility check at building phase; removals queued | G, J |
| Pollution, noise, desirability | O(grid) plus O(buildings + edges) emission | Rolling grid sweeps | H |
| Daily ledger begin and finish, `reset_daily_ledgers`, `reset_daily_input_accumulators` | O(households), O(buildings) | Each entity rolls its own window at its phase | G |
| `job_lock_days` decrement | O(agents) | Passive deadline | D (done) |
| Bankruptcy, wages, property tax, utilities, profit tax | Serial over buildings and agents | Per building at its daily phase, same internal order | G |
| `pay_household_transfers` | Serial over households until the treasury runs out | Per household at its phase; slot order decides who is paid | G |
| `resolve_household_housing` | Serial over households | Check at phase enqueues housing requests | G, J |
| `workplace_route_cache.clear` | Daily clear | Entries expire by slot stamp | G |
| Road upkeep sum | O(edges) | Incremental road length; upkeep posted per slot | G |
| `run_daily_pass`, `execute_demand_household_removal` | O(households) | Exit check at phase; departures through a credit-paced queue | G, J |
| `finalize_daily_tax_window`, `record_daily_budget_ledger` | Serial over ledgers | Calendar close over running sums, O(zones) | G |
| `agents.daily_update` | O(agents), par | `EntityPhase` daily | G |

Per-frame costs found by the `SIM-02-A` `PERF-07` baseline, outside the boundary passes:

| Today | Cost today | Target | Step |
| --- | --- | --- | --- |
| Render snapshot (`build_snapshot_reusing`) | 3–12 ms: a lane pose for every agent before camera culling, serial | Cull before posing; bucket in parallel | K |
| Agent movement in departure frames | Up to 4 ms at 10k and 17 ms at 30k (32×) | Departures spread by events; `PERF-06` limits | E, K |
| One queued construction start | 14–21 ms for a single placement | Placement split across slots | K |
| First boundary after loading | About 10× an ordinary hour: derived economy caches rebuild | Rebuilt during loading | K |

### Open questions

1. **Demand planning slicing.** The planner works directly on the live books and rolls them back
   through journals. Options: separate slots per zone type, or per-district planning against the
   rolling snapshot. Decide in `SIM-02-I` with `PERF-07` data.
2. **Hot dense references.** Resolved in `SIM-02-B`; see
   [Entity handles](#entity-handles).
3. **Pure-job latency and pool size** per job.
4. **Behaviour envelope.** Resolved 2026-10-15; see
   [Behaviour envelope](#behaviour-envelope-sim-02-a).
5. **Electricity stability.** Dispatch against coverage one period old may oscillate; check in
   `SIM-02-F`.

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
