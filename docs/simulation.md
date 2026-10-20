# Metrum Rise — Simulation Engine

This file owns the simulation engine contract: the game clock, the scheduler that decides when
each system runs, entity handles, timed events, snapshot rules, the sim-frame budget and
simulation tiers. Economy and growth rules stay in [`economy.md`](economy.md) and
[`demand.md`](demand.md); traffic movement stays in [`traffic.md`](traffic.md).

**Status:** `SIM-02` in progress. `SIM-02-A` (guards and pre-refactor baseline), `SIM-02-B`
(save cutover and handles), `SIM-02-C` (slot clock, scheduler, timing wheel and phases),
`SIM-02-D` (passive deadlines and construction completion), `SIM-02-E` (trips as timed
events), `SIM-02-F` (per-entity hourly economy), `SIM-02-G` (per-entity daily settlement),
`SIM-02-H` (rolling environment grids and the cell-lot queue) and `SIM-02-I` (rolling
snapshots and pure jobs, in four parts: counts, electricity, store sales, freight suppliers and
the demand snapshot; the funding census over slots on a rolling input copy; construction
permits on the published census; demand planning one zone type a slot) are done.
`SIM-02-J` (request queues) is done: freight requests (part 1); the housing, restock, visit and
job queues (part 2); the level-change, building-removal, departure and admission queues (part
3). `SIM-02-K` is in progress: parts 1 (legacy hour and day paths deleted) and 2 (render
snapshot) are done, and part 3's first commit (frame spikes: cells at load, entrance
cache, first-frame pathing, timing-run checks) is done. See the
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
    work with a stated O(1), O(zones) or O(dirty) bound. `Cost::Dirty` is work proportional to
    what changed or came due since the previous call. The census publication and the hour's four
    demand planning slots declare `Cost::CityScan`, which grows with the city (the planning slots
    O(B + H + P + Q log H) a call in buildings, households, parcels and queued projects): splits
    of the former whole-city pass, recorded as `SIM-02-K` debt for part 5 and the only
    `CityScan` users.
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
2. `FixedSlot` systems, in declaration order.
3. `EntityPhase` systems, hourly before daily.
4. `Queue` matchers.
5. `Rolling` chunks, then swaps for periods that ended.
6. `PureJob` launches and due results.
7. On a day boundary only: the calendar close (budget report window), O(zones).

Slices above a size threshold use Rayon with fixed chunks and ordered merges; smaller slices run
serially and give the same result.

**Implemented in `SIM-02-C`.** `Schedule` (`simulation/core/scheduler.rs`) holds the
declarations in dispatch order and rejects one that breaks the contract: a fixed-slot offset
outside its period, a fixed-slot system without a bounded cost, or an events system that does
not cost its events. The simulation's list and its per-slot dispatch are in
`nodes/sim/core/schedule.rs`: timed events; the census publication, the operational hour's
remainder, the residential, commercial and industrial planning slots and the actions slot (hour,
offsets 0, 0, 0, 1, 2, 3); and the calendar close (day, offset 0). The `sim` debug channel prints the list with costs, tiers, reads and writes.

**Implemented in `SIM-02-F`.** `EntityPhase { period }` systems cost `Phase`, the entities of
one phase, and the schedule rejects any other cost for them. Three hourly ones follow the
re-hosted hour in every slot (`nodes/sim/core/entity_phases.rs`): `building_hours`,
`household_hours` and `agent_hours`.

**Implemented in `SIM-02-G`.** Three daily `EntityPhase` systems follow the hourly ones:
`building_days`, `household_days` and `agent_days` (`nodes/sim/core/entity_days.rs`). The
calendar close is a `FixedSlot` costing `Constant` (O(1) or O(zones), which the schedule now
accepts for fixed slots). It is declared after the operational hour, so in each day's slot 0 it
runs after the 00:00 hour, which still belongs to the closing day, and before any daily phase
of the new day: every entity has settled exactly once since the previous close. Pollution,
noise and desirability remain a legacy midnight pass (`environment grids`), declared before the
hour, until `SIM-02-H`. A `Rolling { period }` kind costs one `Chunk` per slot
and runs after the phases; the rolling road length is its first system.

**Implemented in `SIM-02-H`.** The environment grids leave the fixed slots: `environment grids`
is the second `Rolling { period: Day }` system, one `Chunk` per slot after the road length (see
[Snapshots](#snapshots)).

**Implemented in `SIM-02-H`.** `Queue { quota }` systems cost `Quota`: at most their quota of
requests per slot, each bounded by its own size, and an O(1) check with nothing queued. The
schedule rejects any other cost and a zero quota. Queues run after the daily phases and before
the rolling chunks. The first is `cell lots`, quota 1 ([request queues](#request-queues)).

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
  - **Census at :00:** the funding census publishes at the hour's first slot, before the
    operational hour, from a copy of the city taken over the previous hour's first half
    (`SIM-02-I` part 2, [Pure jobs](#pure-jobs)). The activity floor, store sales, restock and
    visit planning and hiring read it. Its copy is 31–60 minutes old when it publishes, where
    the census used to read the live city at :00. A census rebuilt on load equals the one the
    uninterrupted game publishes.
- **Implemented in `SIM-02-I` (hourly passes):** the activity floor, service-store sales and
  electricity leave the hour.
  - **Buildings:** the activity floor before production, from the funded activity the hour's
    census copied onto the building (`funded_activity`, saved like `funded_job_capacity`, so a
    census rebuilt on load never feeds the hour's phases); after
    production, a service store draws its sales and every building meters its electricity, a
    power plant generating at the last hour's dispatch of its current capacity. The hour's
    pooled passes (logistics, visit planning, the demand snapshot) now read the floors the
    phases set from the previous hour's census.
  - **Households:** service purchases priced by the last hour's residents and store capacity,
    paid into the hour's pool, and the household's residential kWh in its ledger and its home's
    meter. The home's standby load splits equally among its household places
    (`Building::occupancy`), so the home's total is the old one while every place is housed.
  - **`city hour`** (`Rolling { period: Hour }`, after the phases): at the hour's last slot the
    totals the phases folded in phase order become the next hour's dispatch and coverage and
    service prices, the coal burned and the stock left go to the day's report, and the hour's
    collections join the pool, split by resource as they were priced. Undrawn money carries.
  - **Saved:** `city_hour_state` and `service_sales_resources` as `REAL` columns, so the floats
    continue bit for bit, and each store's recorded capacity and profile on its building row.
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
  - **Served by queues:** housing reviews and exits, which need the member links the
    hourly membership census builds (`SIM-02-J` turns them into per-slot queues). Since
    `SIM-02-J` part 2 the `housing requests` queue serves the reviews, and since part 3 the
    `departures` queue serves the exits ([Request queues](#request-queues)).
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
    so results are unchanged. Both counters are saved in `scheduler_state`. Since `SIM-02-J`
    the freight cooldown is a slot deadline, `shipment_cooldown_until_slot`, compared at the
    building's phase and ending at its next hourly phase
    ([Request queues](#request-queues)). The job lock stays a day deadline.
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
  - Since `SIM-02-J` the request queues choose who sets out and list their own trips, and
    `schedule_trip_departures` is gone: freight carriers and reserved shoppers leave near a
    target slot in the hour ahead, levelled per minute; a visible visitor leaves at the next
    slot; arrival carriers keep `paced_slot` from the hour's actions slot. Freight leg timeouts
    are checked at each trip check ([Request queues](#request-queues)).

### Snapshots

City-wide values are one of:

- **Incremental counters**, updated by lifecycle helpers and mutators: household and worker
  counts, vacancies, total road length and similar sums. Nothing rebuilds them on a cadence.
  An index whose order follows its history is saved in that order: claims and releases move
  homes within the vacancy lists, and admission, rehousing and the admission order read their
  positions. Loading rebuilds the lists, then restores the saved order when the same homes
  are vacant (a changed asset pack keeps the rebuild). A stale saved index is stale again after
  loading, and every reader of the vacancy lists or the admission order rebuilds a stale index
  first, so both games rebuild it at the same reader.
- **Rolling snapshots**: a front buffer that readers use and a back buffer being built. Chunk `k`
  of a period folds handle indices `[k·H/C, (k+1)·H/C)`, where `H` is the handle capacity frozen
  at the period start and `C` the chunk count; entities created later are counted next period.
  The back buffer swaps in when the period ends.

Readers only ever see a completed snapshot. This replaces the rule that demand never reads
half-updated economy state. Front buffer, back buffer and cursor are saved, so a loaded game
continues bit-identically.

**Implemented in `SIM-02-H`** (`simulation/grid/sweep.rs`): pollution, noise and desirability
are one rolling sweep per day. Each keeps its front grid, which every reader uses, and a back
grid. With `E` = 11,520, half the day's slots:

- **Start:** the day's first slot freezes the building handle capacity `Hb` and the road edge
  count `He`, and zeroes the pollution and noise backs. This memset of two grids is the sweep's
  only O(grid) step.
- **Emissions:** slot `k < E` folds building handle slots `[k·Hb/E, (k+1)·Hb/E)` and road edges
  `[k·He/E, (k+1)·He/E)` into the backs, with the old per-building and per-edge rules. A freed
  handle slot is skipped; a slot reused since the day's start emits its new building.
- **Diffusion:** slot `E + j` diffuses rows `[j·R/E, (j+1)·R/E)` of the `R`-row backs from the
  fronts, with the old weights and neighbour order, then composes desirability for the same rows
  from the pollution and noise backs.
- **Swap:** the day's last slot swaps all three backs in. Over unchanged buildings and roads one
  sweep equals one old whole-grid step bit for bit. Emissions are whole numbers, so their order
  changes neither the sums nor the saturation at 100.
- **Timing:** readers see the sweep that finished at the end of the previous day, built from the
  buildings and roads of that day's first 12 hours. The midnight pass ran at 00:00 from the
  state at midnight. A sweep starts only at a day's first slot, so a world started mid-day gets
  its first grids at the end of its first full day. A sweep that misses a slot, as when the clock
  is set directly, is abandoned.
- **Saved:** the pollution and noise fronts and backs, and in `scheduler_state` the frozen
  counts, the next chunk and the next edge. Loading rebuilds the desirability back from the other
  backs. Saving drops deleted edges, so the frozen edge count and the edge cursor are saved as
  indices among the saved edges.

**Implemented in `SIM-02-I`:** two hourly rolling snapshots (`Rolling { period: Hour }`, after the
environment grids), each folding building or household handle slots `[k·H/960, (k+1)·H/960)`
with the handle capacity frozen at the hour's first slot, and publishing at its last slot. A
sweep that misses a slot is abandoned.

- **Freight suppliers** (`logistics/supplier_index/roll.rs`): the index of local suppliers with
  a truck load to sell, as building handles in buckets by resource, freight component and 512 m
  chunk. Freight planning (the per-slot freight requests since `SIM-02-J`) reads the index the
  previous hour finished, built for the same logistics hour and component generation;
  otherwise (a world's first hour, after a road edit) it builds one at once. Supplier choice still checks live stock, so only eligibility lags.
  Saved: both buffers, the sweep's cursor and whether each buffer was built for the saved road
  layout (`freight_supplier_sweep`, `freight_supplier_entries`); a load relabels only the
  current ones, so a game saved between a road edit and the next slot rebuilds where the
  running game does (part 3). The border list, freight components and fastest freight speed
  are cached by road-graph revision, and the shipments' reserved amounts are kept up to date
  by every shipment change as exact integer sums (2^-30 kg units), so they never depend on the
  order of changes and a load's rebuild equals them.
- **Demand snapshot** (`demand/snapshot/rolling.rs`): the household and building totals, the
  commercial activity signal, the treasury-ordered count of affordable consumers, the displaced
  workforce and the residential occupants that the hourly demand pass composes its snapshot
  from. The pass reads the previous hour's sweep, first folding the entities created in new
  handle slots since that sweep began, so a site placed mid-hour is never planned twice; the
  operational hour folds them before its job search (since `SIM-02-J`, before the slot's job
  requests), so a household hired that hour is not counted as unemployed while its workplace
  still shows the job open (part 2). The workforce fields are counted on the live city at the
  pass, after job search (since `SIM-02-J`, with the hires the job queue has made so far), from
  the buildings' published funded jobs, in a parallel O(H/p + B/p) scan that `SIM-02-K`
  replaces with maintained totals. The
  treasury the consumer count draws on is frozen at the sweep's first slot, and benefit claims
  are walked in handle order. The construction epoch at the planning slot reads the same
  snapshot, with the workforce the pass counted on the live city (part 3). Saved: front, back and cursor as binary blobs in `scheduler_state`. A handle slot
  freed and reused after its chunk keeps the old entity's contribution until the next sweep.

**Implemented in `SIM-02-J`:** two more hourly rolling snapshots of building handles, folded by
handle slot over the hour's slots and published at its last slot for the next hour's
[request queues](#request-queues).

- **Household destinations** (`households/destinations.rs`, after the demand snapshot): the
  stores selling household supplies and the service stores hosting visits, in buckets by foot
  and car component and 512 m chunk. Restock and visit requests resolve each handle and check
  the store again (sold out, closed), so only eligibility lags; a store restocked since its
  chunk waits for the next sweep. A store that left or sold out since its chunk counts against
  its own cap, the size of the live inspection cap, so a scan inspects at most 384 live stores
  (visits 128) and passes over at most as many others. A reader without a front for its hour
  and the current CCH build builds one at once, O(B). Walking proofs are prepared per
  inspected candidate instead of per store per hour; they are exact either way. Foot and car
  components are cached by CCH build generation (`HouseholdTopology`). Saved: both buffers, the
  sweep cursor and, for the front and the sweep, whether they were folded with the saved game's
  CCH build and no road change waits for the next rebuild (`household_queue_state`,
  `household_destinations`); a load labels only those current, as part 3 does for the freight
  supplier index.
- **Job board** (`households/job_board.rs`): the buildings with an open funded job at their
  slot, in handle order. A search reads each member's live funded jobs and workers, so a job
  filled since its chunk is no option. Membership lags the census by one hour: the board
  folded during hour h−1 reads h−1's published funded jobs, so a job the hour's census opens at
  a building not on the board waits for the next board; an hour without a board builds one at
  once. Its network side (buckets, arrival ends, prepared targets) is rebuilt per board hour
  and CCH, metric and lane revision, and when a member's own door changes
  (`BuildingEntranceKey` per entry), not on any building's door; top walking and car speeds are
  cached by CCH build and metric revision. Saved: front, back and cursor
  (`job_board_members.buffer`, `household_queue_state.job_*`).

### Request queues

Order-dependent matching against shared capacity runs as queues: job search, freight requests,
housing, household admission and departure, building level changes and clearances. An entity
enqueues `(enqueue slot, handle, kind)` when its phase or deadline comes up; the matcher serves
requests oldest first, ties broken by handle, up to its quota per slot, against the live indices.
Rate caps such as the daily departure limit accrue per slot through the existing credit helpers
(`demand/credits.rs`). The construction queue (`pending_demand_spawns`) is the precedent and
moves onto the timing wheel.

**Implemented in `SIM-02-H`:** dirty cell-lot regions (`zoning/system/cell_lots/dirty.rs`,
`nodes/sim/zoning.rs`).

- A region is one 512 m chunk, marked as today. Each slot the `cell lots` queue regenerates
  the generated cells and lots of at most one region, oldest first, and none while a road edit
  is staged. The hourly demand pass no longer drains the regions; it plans on the lots served
  so far, so a region marked late in an hour waits for the queue rather than for the hour.
- A chunk marked again while pending keeps its place. The chunks marked between two services
  join the queue together in coordinate order, so the order never depends on hash iteration.
- A catalog refresh (an asset reload, a dense-terrain undo, or a load whose live registry
  yields other lot sizes than the save recorded) re-derives every painted chunk. Unless an
  editing path drains them at once, those chunks are served only in slots with nothing else
  queued, so a load's refresh never delays the regions its save queued.
- Editing paths (road commits, undo, asset reload, entrance rebuilds) still regenerate every
  pending region synchronously, in coordinate order.
- Saves keep the queue in order (`zoning_cell_lot_queue`), flagging the chunks marked since
  the last service. Load replaces the regions that restoring parcels marked, since the saved
  lots already hold their derivation. Saves also record the asset footprint sizes the lots
  were derived with (`zoning_cell_lot_catalog`), unless a catalog refresh is still pending. A
  load whose live catalog yields the same sizes keeps the saved lots and invalidates only the
  generated cells, which saves leave out and the overlay rebuilds on demand; the sizes are all
  the derivation reads from the catalog.
- Cost per slot: O(1) with nothing queued. A served region costs O(region) with the cached
  asset sizes, plus repairing the building-site index or road surface if an earlier system in
  the slot left them stale, a repair the frame's render snapshot otherwise makes.

**Implemented in `SIM-02-J`** (part 1, freight; part 2, households;
`simulation/core/request_queue.rs`, `logistics/requests.rs`, `households/requests.rs`,
`nodes/sim/core/household_requests.rs`): five more queues follow `cell lots`, in this order:
`freight requests`, `housing requests`, `restock requests`, `visit requests` and
`job requests`.

- **Order and marks:** an entity queues `(slot, handle)` at its phase, a building also the
  request's kind. Phases visit their handles in ascending order, so each queue's insertion
  order is already oldest first with ties by handle. A mark per handle slot, holding the queued
  handle's generation, keeps one request per entity and queue (freight: per kind); a pop
  clears the mark only for the popped request's generation, so a stale request never clears
  the mark of a newer entity in the same slot. Marks are derived: a load rebuilds them from the
  saved requests. A request whose handle no longer resolves is dropped when served; one whose
  entity changed is served on the entity as it is then. With nothing queued a queue costs
  O(1).
- **Freight** (quota 8): at its hourly phase, after production and off its cooldown, a building
  queues an inbound restock request when an input has fallen to its reorder point and an export
  request when its surplus reaches the dispatch target, inbound first, at most one of each kind
  (a pending bit per kind and handle slot). The queue plans against the live stock and
  reservations and the [supplier index](#snapshots) of the previous hour's sweep. A slot's plans
  load their cargo at the slot's end, as the hourly pass loaded its plans after planning. Each carrier keeps a
  golden-ratio target in the hour ahead (`spread_slot`) and leaves in the least busy minute
  within 15 minutes of it, never before the next minute (`MinuteLeveler`, a saved 128-minute
  count ring, O(31) per departure). A shipment has one live trip event
  (`Shipment::trip_event_slot`); its leg times out at a trip check once the logistics hours
  since the leg started (`leg_started_hour`) reach its timeout; closed shipments leave the list
  whenever a new shipment id is a multiple of 256. Cooldowns are slot deadlines
  (`Building::shipment_cooldown_until_slot`): a failure or export planned for a request lasts
  until the phase that queued it comes round, so a failed inbound plan still blocks that
  phase's export; any other failure (a carrier that cannot leave, a shipment failing on the
  road, an endpoint removed) lasts until the building's next hourly phase, found from its
  handle's phase in O(1). No hourly pass creates or scans shipments.
- **Housing** (quota 64): a household's hourly phase queues its rehousing while it has an
  independent member and no usable home; its daily phase queues its housing review. The queue
  serves both oldest first, merged by (queue slot, household), a household's rehousing before
  its review of the same slot. Reviews read the live vacancy lists: the admission order keeps
  its rehousing homes per level and 512 m chunk (`by_level_chunk`), and a review's nearest
  affordable home is a ring search over that level's chunks. Exits are served by the
  `departures` queue (part 3).
- **Restock** (quota 64): after its hour a household queues its restock when the hourly pass's
  test (`restock_due`) holds for the hour of its phase and a member is free to shop; its
  shopper is the first eligible member in agent order. The queue plans a slot's requests in
  parallel and reserves them in queue order against live store stock, from the
  [household destinations](#snapshots). A reserved shopper leaves near its household handle's
  slot of the hour ahead (`spread_slot`), in the least busy minute within 15 minutes (a second
  `MinuteLeveler`). A household that moves while its shopper is still inside the old home
  cancels and refunds the purchase first.
- **Visits** (one a slot, paced by a credit): the phase's draw queues a visible service visit
  while fewer than 64 wait. The credit accrues 64 visits an hour (one every 15 slots) and holds
  at most one visit while nothing waits; each whole credit serves one request. The visitor
  leaves at the next slot.
- **Job search** (quota 64): an agent's hourly phase first ends an assignment to a workplace
  that stopped operating or of an agent who cannot work, then queues its search while it is in
  a building and unemployed, past its job lock or unpaid for two days. The queue runs a slot's
  searches together against the [job board](#snapshots): homes shared within the slot are
  searched once, options rank as before, and plans apply resident workers with a job at home
  first, then in agent order. Funded jobs come from each workplace's own row
  (`Building::funded_job_capacity`). A searcher who keeps its job, including one whose better
  jobs filled first or who finds no open or reachable job, is locked again; the lock stays a
  day deadline.
- **Planning slots:** the queues serve in the hour's planning slots 0–2 too, after the slot's
  fixed systems. They change the live city between the planning slots (hires, moves,
  purchases) but remove no building or household, so they never invalidate the published
  books and never mark a flow field; the later slots' live workforce counts and re-proofs see
  the hires.
- **Gone from the hour:** freight creation and the shipment progress scan; the export hold
  pass; the parallel O(households) unhoused check and the rehousing pass; the reviews'
  per-batch vacancy planner; the O(households) restock scan, the O(agents) shopper scan and the
  hourly store and service index builds; the job search's O(agents) ejection, key collection,
  plan and relock scans and its two O(B) job-supply scans. The household hour keeps the arrival
  count, the logistics clock and the commute-quote expiry.
- **Saved** (save format 102): `freight_requests`; `shipments.leg_started_hour` and
  `shipments.trip_event_slot`; `buildings.shipment_cooldown_until_slot`; in `scheduler_state`
  the next shipment id and the freight departure leveler (`logistics_next_shipment_id`,
  `freight_departure_first_minute`, `freight_departure_counts`); `household_requests` with its
  queue slot (`slot`, NULL for exits; kinds 0 housing reviews, 1 exits, 2 rehousings, 3
  restocks, 4 visits); `job_requests`; `household_queue_state` (the visit credit, the shopper
  leveler and the destination and job board sweep state); `household_destinations`;
  `job_board_members`. Marks and pending bits are derived. Quotas follow measured request
  rates; see [Decided while unattended](#decided-while-unattended).

**Implemented in `SIM-02-J`** (part 3; `nodes/sim/core/demand_requests.rs`,
`demand/execution/level_change.rs`, `households/admission.rs`, `households/removal.rs`,
`buildings/allocator/lifecycle.rs`): four more queues follow `job requests`, in this order:
`level changes`, `building removals`, `departures` and `admissions`. They carry out what the
hour's actions slot (slot 3) decided on the census published at slot 0.

- **Removal window:** building and household swap-removes run only in slots 4–479 of each hour
  (`hiring::removals_allowed`). A swap-remove in the census window (480–959) restarts the
  census, because the structure revisions are part of its key; one in slots 1–3 leaves the
  published books invalid for the live city, so the planning slot reading them skips. Over slots
  4–479 nothing reads the published books and no census runs, and the census input copy is
  filling: `CensusInputs::remove_building` and `remove_household` mirror each swap-remove into
  it in O(1), plus the O(households) parallel retargeting of copied home and store references.
  A removal requested in slots 480–959 or 0–3 waits for slot 4, at most 484 slots. A removal's
  dependent work (agents evicted or evacuated, households left unhoused, shipments and caches)
  happens with it, through the existing lifecycle helpers.
- **`level changes`** (quota 8, slots 4–959): slot 3 still proves the hour's business level
  changes in its construction epoch and checks that each target fits (lot, occupants, workers,
  economy profile, site), but no longer applies them. A proved change keeps its claims in the
  epoch for the changes after it and is queued with a permit: a digest of the building's asset,
  level, economy profile and, for a business, its door (`level_change_permit`). The door is
  derived afresh at both ends from save-stable identities, not read from the entrance cache, so
  a loaded game, whose cache is rebuilt, compares the same digest. A residential change needs no
  proof; its permit leaves out the door. The queue applies a change only while the building's
  digest equals its permit and the target still fits; a stale permit is rejected and nothing is
  proved again (`Work::LevelChangePermitRejections`). After a slot that applied any, the zone
  index and the entrance cache are rebuilt once, as slot 3 rebuilt them. A level change is an
  in-place change: `record_building_changes` names its row for the census input copy, which
  copies it again over slots 0–479 and defers it while a census reads the front, so it restarts
  no census. The queue does not serve slots 0–3, so the planning slots never see a building
  change under them.
- **`building removals`** (quota 8, slots 4–479): planned clearances from slot 3's hour plan
  and buildings whose compatibility expired at their daily phase, as generation-checked
  building handles, oldest first. Each is checked again when served: a clearance only while the
  site may still be cleared (`can_demand_despawn`), its remaining occupants evacuated first; an
  incompatible building only while the check, run again with no elapsed day, still says so, so a
  building that recovered stays. A slot of clearances then rebuilds the zone index and the
  entrance cache once, as slot 3 did; a slot that removed an incompatible building bumps the
  building revision (`finish_phase_removals`), as daily phases did. A building whose daily phase
  queues its removal skips that day's settlement, as the immediate removal skipped it. Editing
  paths (`allocator.maintain` after a player's edit, the bulldozer, undo) still remove at once.
- **`departures`** (quota 1, slots 4–479): the households whose daily phase claimed a departure
  from the daily exit credit (at most 48 a day), oldest first, each removed only if it may still
  leave (`HouseholdExitPolicy::is_eligible` on live state). The credit already paces the
  claims, so the queue adds no second one. A removal finds the household's members from its
  maintained member list instead of scanning every agent (O(members) plus the existing
  parallel O(agents) household remap).
- **`admissions`** (one per whole credit): slot 3 still decides the hour's admissions against
  the workforce calendar and claims each admitted family's home; the queue creates the
  households and their arrival carriers. The decided households wait in order (home handle,
  size, composition, arrival border). The credit, in slots, starts at one whole admission (960)
  at slot 3 and accrues the waiting count per slot, so the k-th of n is created at slot
  2 + ceil(k·960/n) (the first at slot 3) and the last before the next hour's slot 3. Each
  slot's carriers depart paced from their creation, so a lone carrier leaves at the next slot:
  the departure profile is the old one, about a slot later. An admission whose home left is
  dropped; one whose border no longer has a car road releases its home
  (`Work::AdmissionsDropped`). Queued admissions count as pending arrivals in the hourly recount.
  Appends neither restart the census nor invalidate the books.
- **Gone from the hour and slot 3:** the departures at slot 0 before the household hour
  (`remove_queued_exits`); slot 3's level-change application, despawns and carrier spawns; the
  building phase's immediate compatibility removal.
- **Saved** (save format 103): `level_change_requests` (slot, building handle, target asset,
  permit); `building_removal_requests` (slot, building handle, kind 0 clearance or 1
  incompatible); `admission_requests` (home handle, size, composition, border node through the
  save's node numbering, none when no saved road reaches it); `household_queue_state.
  admission_credit` and `admission_rate`. Departures keep `household_requests` kind 1.

### Pure jobs

A `PureJob` is a pure function of an immutable snapshot taken at slot `s`; its result is applied
at slot `s + latency`. It runs off the core mutex in a dedicated Rayon pool, so long jobs cannot
starve per-frame movement (the `ROAD-38` lesson), and it never touches `SimCore` (the `SIM-01`
lesson). If the job has not finished by its apply slot, the sim thread waits: results never
depend on completion time. Mutating passes are never pure jobs. Candidates: distance tables,
field diffusion and other read-only precomputation.

**Implemented in `SIM-02-I` (part 2): the funding census spread over slots** (decided
2026-10-17; see [Decided while unattended](#decided-while-unattended)). The census only reads
the city and writes its own books, but it costs about 96 ms a call at 100k. It is not a pure
job: it routes on the live lanes, CCH and road graph, which the sim thread writes every frame
(lane frontage delays, edge congestion) and which the per-frame agent planner shares, so a
background copy would mean moving those fields out of the lane and edge records and copying
about 90 MB of lanes at 100k after every road edit.

- **Inputs** (`hiring/inputs.rs`): a rolling copy of what the census reads besides the
  network: buildings, households, each building's paid freight, and the treasury, fiscal
  policy, city electricity funding and clock. A back copy fills over the hour's first 480
  slots in dense order and becomes the front at slot 479, with those values frozen there. A
  swap-remove is mirrored into both copies in O(1); a building removal also retargets the
  copied households' home and store references, O(households) in parallel like the live pass.
  Rows the allocator changes in place are copied again each slot. While a census reads the
  front, the front follows removals but defers in-place changes and appends, which a restart
  copies in.
- **Steps** (`hiring/steps.rs`, `hiring/rolling.rs`): from slot 480 the census runs over the
  front copy and the live network as resumable steps into a back `JobFunding`: routes and
  market setup, then for each closure pass import funding, household claims, source
  production, current staff, expansion, publication, repair, and business claims and proof.
  A sliced step takes ceil(R / S) rows a slot, never a wall-clock budget: R is the rows left if
  the current pass is the last, S the window slots left, and slot 959 takes the rest. The other
  steps run whole in the slot they start in. Frontage prices publish when a census starts. A
  change of the network revisions (CCH build and metric, lanes, frontage) or of the building
  or household structure revisions restarts the census, even after it finished early; appends
  and in-place changes do not. A fiscal or service funding change also restarts it, after
  writing the new values into the copy's frozen policy and recording an overridden building's
  row; the published census stands until the next one, while service workers the new funding no
  longer pays leave at once.
- **Publish** (`households/census.rs`): `census publish`, a fixed slot at offset 0 declared
  before the operational hour (`Cost::CityScan`), swaps the census in and copies funded jobs and
  activity onto the buildings, O(buildings). A census that is missing, unfinished or stale there runs whole.
- **Loading:** saves keep both copies, the deferred rows, the publish slot and whether the
  census is current. The census is a pure function of the copy and the network, so a load
  recomputes a current census whole, whatever step it had reached; a stale one restarts at the
  next window slot, or runs at slot 0.
- **Readers:** buildings take funded jobs and activity from their copies at their phases, and
  the demand snapshot sweep reads the same copies. Restock and visit requests read only the
  published census's walking proofs and household access rows, and job search the funded jobs
  copied onto each workplace (`SIM-02-J`). Demand planning lends the published books, with the
  buildings started since the copy appended as buyers, and runs no census of its own.
  Commercial and industrial construction is proved once an hour, in the hour's actions slot, on the same books, and keeps
  a saved permit until it starts (`SIM-02-I` parts 3 and 4). Planning slots 0–2 and the actions
  slot 3 all read the census published at slot 0; a save in those slots keeps what restores it
  exactly (`census_inputs.published`), so a load there plans on the same books (part 4). A load
  from slot 3 on runs a census of the loaded city, which nothing reads before the next hour's
  first slot publishes.

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
- **Snapshot columns** (from `SIM-02-K` part 2): `PERF07_DAY` also reports the render
  snapshot's mean and p99 (`mean_snapshot_ms`, `p99_snapshot_ms`). `METRUM_PERF07_MINUTES`
  caps the measured span for diagnostics; gate runs leave it unset and measure the full day.

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

**Timing runs need an idle machine** (calibrated 2026-10-17 on `SIM-02-H`). A release build on
the efficiency cores at the lowest CPU and I/O priority (`taskset -c 16-23 nice -n 19 ionice
-c3`), running alongside the pinned benchmark, raised the 10k and 30k 32× means by 50% and 41%
and their p99 by 14% and 28%. A 100k 1× run that overlapped another build read 15% slower per
frame. Builds and tests therefore never run during `PERF-07` or `PERF-01`; the envelope,
determinism checks and `./run.sh --test` are not timing runs and may share the machine.

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
| `SIM-02-K` | Legacy `FixedSlot` hour and day systems, `simulate_tick_internal`, `simulate_operational_hour_internal` and every minute-0 path are deleted (part 1 done); the render snapshot culls before posing (part 2 done). `PERF-07` acceptance at 30k and 100k. The cadence sections in `economy.md` and `demand.md` point here. |

#### `SIM-02-K` parts

Planned 2026-10-20 from a profile of the 100k `PERF-07` day on `SIM-02-I` part 3 (per-phase
timing and `perf` call stacks; an ordinary 100k frame was then 11.9 ms at 1× and 21 ms at 32×,
about 70% of it serial). Each part is gated and committed on its own, in this order:

1. **K1, legacy paths deleted** (done 2026-10-20). `simulate_tick_internal`, `simulate_operational_hour_internal`,
   the coarse-hour census callers and every minute-0 or hour-boundary path go; tests,
   benchmarks and the contract harness step the slot clock. The remaining fixed-slot systems
   state honest costs. Expected envelope: identical, or the difference explained.
2. **K2, render snapshot** (done 2026-10-20). Cull before posing (about 9% of the moving
   agents are in the camera), on each lane agent's lane bounds rather than its stored position
   (see [Decided while unattended](#decided-while-unattended)); site queries that skip distant
   sites, with one gather per vehicle; parallel pose and bucketing. Planned saving about 5.5 ms
   at 1× and 6.5 ms at 32× at 100k; the 100k snapshot mean fell 5.9 ms at 1× and 7.8 ms at 32×.
3. **K3, frame spikes** (first commit done 2026-10-20). Done: (a) generated cells built during
   loading instead of by the first lot regions served (25–134 ms a region at 100k, all day), the
   entrance part of (c) (a removal keeps a clean entrance cache in step), (d) the route scratch
   and flow fields built at load, and (e) test-build checks out of timing runs. Left for the next
   K3 commit: (b) census slices, the zone-index part of (c) (kept as a decision), (d)'s first
   trip events, and the clearance site publication. Every worst `PERF-07` frame of the K2 build
   was attributed to its systems first (table in the results section).
4. **K4, agent tick.** Lane buckets kept across appends and swap-removes instead of rebuilt each
   tick, filled in parallel; the unused per-edge `current_congestion` write removed; indoor agents
   off the per-frame movement pass (departures planned at their slot); movement shards balanced;
   fewer agents in the serial handoff set; cheaper pose sampling. Saves about 3 ms at 1× and
   7 ms at 32× at 100k together.
5. **K5, planning slots without city scans.** Per-zone lot lists, maintained workforce and
   market totals instead of the O(B + H) scans at slots 0–3 (`Cost::CityScan`), incremental
   downgrade candidates, the job board and destination index without their O(B) fallback
   builds, the commute quote cache cleared per building.
6. **K6, 100k traffic.** Find why trips stall in the 100k fixture (about 45,000 of 145,000
   agents always moving; junction throughput, routing) and keep movement within the `PERF-06`
   limits.
7. **K7, acceptance.** `PERF-07` max at most 8 ms at 30k and 100k, at 1× and 32×, over a full game
   day; envelope and burst limits; `SIM-02` and `PERF-07` marked done.

If a part cannot reach its target without changing a decision of this document (for example a
coarser simulation tier for distant agents, or GPU compute), the findings and options are
written here and the part stops for review. Timing comparisons run the previous part's binary
as a control in the same session, on the same snapshot: after a machine restart onto a newer
kernel on 2026-10-19 unchanged binaries measured `PERF-07` means about 30% higher, which the
owned thread pool of `CODE-17` then removed (handing work into Rayon's global pool from outside it
had become slower).

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

#### `SIM-02-H` — rolling environment grids and the cell-lot queue (2026-10-17)

- **Save format 94:** pollution and noise save their back grids beside their fronts;
  `scheduler_state` saves the environment sweep (frozen building handles and edges, next edge,
  next chunk); `zoning_cell_lot_queue` holds the pending lot regions in service order and
  `zoning_cell_lot_catalog` the asset sizes the lots were derived with. The Kuopio fixture gained
  the columns and tables and its hash in `placements.json`.
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): every metric of every
  workload equals `SIM-02-G`'s, the notes included. The grids are read only by the map
  overlays, and the hourly demand passes placed the same buildings on the lots the queue served.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`). Max / p99 / mean in
  ms, `SIM-02-G` in parentheses: 10k 1× 21.8 / 6.23 / 2.43 (192.2 / 6.29 / 2.43), 10k 32× 22.3 /
  8.89 / 2.86 (192.6 / 8.33 / 2.95), 30k 1× 128.6 / 7.40 / 3.34 (498.6 / 7.37 / 3.34), 30k 32×
  134.9 / 26.78 / 5.32 (500.4 / 26.85 / 5.28), 100k 1× 466.8 / 19.06 / 12.01 (1,970.1 / 19.41 /
  12.09), 100k 32× 474.5 / 188.89 / 22.30 (1,948.5 / 185.93 / 21.90). Every gate limit passes.
  - The worst frame fell to a quarter of `SIM-02-G`'s or less at every tier. The 01:00 spike
    was mostly the catalog refresh every load made, which re-derived every painted chunk's lots
    in the first hourly demand pass; a load now skips it when the catalog is unchanged.
  - The worst frames at 30k, 01:00, 03:00 and 06:00, carry 64–89 ms of lot work beside the
    hour. Saves leave out the generated cells, so loading invalidates them, and the first lot
    regions served after loading rebuild them: 61–88 ms for each of the first four regions at
    30k, then about 3 ms a region (a diagnostic run with the `economy_phases` channel). The
    first hour frames at 100k carry about 85 ms more slot work than on `SIM-02-G` for the same
    reason (cell generation up to 85 ms a region). `SIM-02-K` rebuilds them during loading or in
    bounded parts (phase inventory).
  - The first full run failed 100k 1× on p99 (22.94 against a limit of 19.10, mean 13.88 ms):
    a release build ran on the efficiency cores for 14 of its 20 minutes. The idle rerun above
    passes. See the benchmarking note under [`PERF-07`](#frame-budget-perf-07).
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 17.3 / 7.6–8.5 against 8.2–8.3 / 0.00; baseline
  30k 63.0–63.9 / 49.6–49.8 against 35.7 / 0.00; fiscal 10k 27.7–28.2 / 22.6–23.5 against
  15.8–15.9 / 0.00; fiscal 30k 79.3–80.7 / 68.8–70.1 against 53.1–53.8 / 0.00. The same eight
  work-count limits fail as on `SIM-02-G`; no time limit fails. The hour no longer includes
  cell-lot regeneration, which runs in the per-slot queue that `PERF-01` does not time:
  baseline 30k spends 260 ms a day on lots against 466 ms on `SIM-02-G`, whose total included
  the load refresh, and at 10k the refresh was the only lot work. The census, planning and
  logistics totals are within 2% of `SIM-02-G`'s, and the day is the calendar close alone.
- **Build identity:** envelope, determinism, `PERF-07`, `PERF-01` and the tests ran on the
  committed build (rustc 1.98.1).
- **Tests:** 2,448 release library tests, the integration tests and `./run.sh --test` (isolated
  profile) pass; clippy is clean. New: one day's sweep equals one whole-grid step bit for bit,
  and a sweep started mid-day publishes nothing; the sweep's saved edge indices skip deleted
  edges; the lot queue serves one region per slot, oldest first, and holds while a road edit is
  staged; queues run after the entity phases and before the rolling chunks; pending lot regions
  round-trip in service order; a load with an unchanged catalog keeps the saved lots, and another
  catalog re-derives them.

#### `SIM-02-I`, part 1 — counts, electricity, store sales, freight suppliers and the demand snapshot (2026-10-17)

- **Save format 95:** households' service purchases and the city hour (`city_hour_state`,
  `service_sales_resources`), each store's recorded sales capacity and each building's funded
  activity; the freight supplier sweep (`freight_supplier_sweep`, `freight_supplier_entries`);
  the demand snapshot sweep (three `scheduler_state` columns); the vacancy lists' order and
  whether the index was stale (`building_vacancy_state`, `building_vacancies`). The Kuopio
  fixture gained the tables and columns and its hash in `placements.json`.
- **What moved off the hour:** the membership census (each household recounts at its phase);
  the activity floor, service-store sales and electricity (each building and household at its
  phase, the city totals folded at the hour's last slot); the freight planning context (topology
  cached by revision, reserved amounts kept by every shipment change, the supplier index a
  rolling snapshot); the demand snapshot (a rolling snapshot plus the entities created since its
  sweep began). See [Phases](#phases) and [Snapshots](#snapshots).
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): money residuals fell
  to at most 0.04 an hour (0.83 on `SIM-02-H`), since the store-sales pool moves exact budget
  changes. One metric left its band: the 30k baseline's net income per resident, 12.21 against a
  low edge of 12.24 (12.38 on `SIM-02-H`), with bankruptcies at 0.27 per 1,000 buildings a day
  against 0.24. Envelope runs of each piece alone attribute it to two one-hour lags, the
  supplier index (−0.08 and most of the bankruptcies) and the store payment (−0.10); see
  [Decided while unattended](#decided-while-unattended). The fiscal-stress notes of earlier
  steps remain: employment 0.077 and 0.079 against about 0.052, final employment 0.70% and 0.73%,
  and 10k deliveries at 12.85 against a low edge of 13.05.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days). New continuation tests save a growing starter town mid-hour, with the vacancy
  lists out of building order and with a stale index, and compare every frame for a game day;
  the first diverged on `SIM-02-H`.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`). Max / p99 / mean in
  ms, `SIM-02-H` in parentheses: 10k 1× 17.8 / 6.07 / 2.36 (21.8 / 6.23 / 2.43), 10k 32× 18.4 /
  8.25 / 2.80 (22.3 / 8.89 / 2.86), 30k 1× 122.6 / 7.11 / 3.27 (128.6 / 7.40 / 3.34), 30k 32×
  126.6 / 26.16 / 5.20 (134.9 / 26.78 / 5.32), 100k 1× 440.6 / 18.37 / 11.74 (466.8 / 19.06 /
  12.01), 100k 32× 515.0 / 185.39 / 21.88 (474.5 / 188.89 / 22.30). Every gate limit passes. The
  worst frame at every tier is still the first hours after loading. At 100k 32× the 01:00 frame's
  hour fell to 343.7 ms (369.5 on `SIM-02-H`), but its other slot work rose to 156 ms (89): at
  32× a frame spans about eight slots, and right after loading those include lot regions that
  rebuild the generated cells and the rolling sweeps' one synchronous first build.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 17.3 / 7.8 against 8.6 / 0.00; baseline 30k
  62.4–63.2 / 47.6 against 35.0 / 0.00; fiscal 10k 27.6–28.5 / 22.4–22.5 against 15.5–15.6 /
  0.00; fiscal 30k 79.6–80.1 / 69.4–70.3 against 53.5–53.7 / 0.00. Seven limits fail, all
  work-count scalings `SIM-02-A` also fails (eight on `SIM-02-H`: the 10k city's buyer-site
  rebuilds now scale). The hour is within 4% of `SIM-02-H` (8.2–8.3, 35.7, 15.8–15.9 and
  53.1–53.8): the four passes that left it cost a few milliseconds at these sizes. The 10k
  census costs 4.9 ms a call against 3.7 on `SIM-02-H`, and one construction-execution census
  now runs there; the 30k census is unchanged per call (18.4 against 18.3 ms). Part 2 replaces
  both.
- **Build identity:** envelope, determinism, `PERF-07`, `PERF-01` and the library tests ran on
  the committed sources built with rustc 1.99.0; `./run.sh --test` ran on the same sources built
  with rustc 1.99.1.
- **Tests:** 2,470 release library tests and the integration tests pass; `./run.sh --test`
  passes in an isolated profile on a quiet machine (`agent_inspector_test` is timing-sensitive
  under CPU load, `DEBUG-07`); clippy is clean. New: member lists equal a rebuild through spawns, swap-removes,
  carrier conversion and household remaps, and a household's counts follow its phase; phase
  electricity equals the old hourly totals over unchanged state, and coverage and dispatch lag
  exactly one hour; the store-sales pool conserves money when a store leaves before its draw;
  maintained reservations equal a fresh build in every order, including an amount off the batch
  grid; a rolling supplier index equals a fresh build, and a road edit retires it; a rolling
  demand snapshot equals a whole-city build, entities created mid-sweep count once, and a sweep
  saved mid-hour continues identically; the vacancy order and a stale index survive a save.

#### `SIM-02-I`, part 2 — the funding census over slots on a rolling input copy (2026-10-18)

- **Save format 97:** the census input copy (`census_inputs`: both copies, the deferred in-place
  rows, the publish slot and whether the census is current). The Kuopio fixture gained the table
  and its hash in `placements.json`.
- **What moved off the hour:** the hourly funding census, about 96 ms a call at 100k
  ([Pure jobs](#pure-jobs)): its inputs are copied over the hour's first half, it runs as
  resumable steps over the second, and it publishes at the next hour's first slot. Demand
  planning lends the published books, and policy changes reach the census through its copy.
  Construction execution and, after a firm with inputs starts in the window, the planner still
  run their own censuses; part 3 removes them.
- **Defects found in part 1** (committed 2026-10-17), fixed here: the demand snapshot read the
  live census, which a load replaces mid-hour; and new households joined it after the hour's
  job search had hired them, counting each such job twice. With exact counts, planning now
  takes its workforce from the live city at the pass. See
  [Decided while unattended](#decided-while-unattended).
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass, money residuals
  at most 0.03 an hour, the busiest minute at most 3.3% of an hour's departures. The 30k
  baseline's net income per resident is back in its band, 12.34 (12.21 on part 1). Against part
  1, the 10k baseline's net income per resident fell from 13.43 to 13.18 and its bankruptcies
  rose from 0.04 to 0.06 per 1,000 buildings a day; the 30k baseline's bankruptcies rose from
  0.27 to 0.33. An envelope run of the input copy alone (the census still at the hour's first
  slot) gave 13.39 at 10k and 0.26 bankruptcies at 30k, so the rest comes from the census's half
  hour of extra age. The fiscal-stress notes of earlier steps remain (employment 0.077 and 0.080
  against about 0.052, final employment 0.62% and 0.75%, 10k deliveries 12.82 against a low edge
  of 13.05).
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days). New continuation tests save during the copy, after the swap, mid-census and
  before publishing, and after a policy change at minute 40, and compare the published censuses
  (with their books) or every frame for three hours.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`). Max / p99 / mean in
  ms, part 1 in parentheses: 10k 1× 71.1 / 5.93 / 2.34 (17.8 / 6.07 / 2.36), 10k 32× 70.3 / 7.05
  / 2.86 (18.4 / 8.25 / 2.80), 30k 1× 93.8 / 7.54 / 3.31 (122.6 / 7.11 / 3.27), 30k 32× 89.8 /
  23.44 / 5.38 (126.6 / 26.16 / 5.20), 100k 1× 338.3 / 18.67 / 12.08 (440.6 / 18.37 / 11.74),
  100k 32× 429.2 / 178.40 / 23.10 (515.0 / 185.39 / 21.88). Every gate limit passes. The 100k
  worst frame is still the first hour after loading, its operational hour down to 238 ms. The
  10k maximum is new and not the census: a slow-frame diagnostic (work counters and phase times
  per frame on the same snapshot) put 84.5 ms of the 06:00 frame and 68.7 ms of the 22:00 frame
  in the cell-lot queue. In both hours the planning pass removed a building, and the region it
  freed was the first served since loading, while its generated cells were still cold: the
  post-load cost `SIM-02-H` recorded for `SIM-02-K`. Part 1's snapshot city had no such removal
  in its measured day. The means rose up to 6% (100k 32×): the census work moved into slots,
  and the live workforce count is new.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 17.0–17.3 / 7.7 against 4.5–4.6 / 0.00; baseline
  30k 62.3–62.6 / 47.3–48.7 against 18.7–19.1 / 0.00; fiscal 10k 27.2–27.9 / 22.4–24.4 against
  13.1–13.2 / 0.00; fiscal 30k 79.0–80.1 / 67.6–68.0 against 43.3–44.2 / 0.00. Part 1's hours
  were 8.6, 35.0, 15.5–15.6 and 53.5–53.7: the census window runs in slots `PERF-01` does not
  time. Seven limits fail, the same work-count scalings as part 1, all of which `SIM-02-A` also
  fails. Planning ran no census of its own in any workload; construction execution ran 2 a day
  at 10k and 12 at 30k (part 3 removes them). Fiscal 30k's hour is mostly its planning pass,
  about 26 ms (part 4 splits it by zone type).
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1;
  the `SIM-02-A` runs used its retained binary.
- **Tests:** 2,488 release library tests and the 4 growth integration tests pass; clippy adds
  no warnings; `./run.sh --test` passes in an isolated profile, after a fix to
  `agent_inspector_test`'s races with the simulation thread (`DEBUG-07`). New: the census in slices of 1, 5, 37 and 1,000
  rows, with restarts and more than two closure passes, equals one run bit for bit, in isolated
  markets and in the game; the census restarts on network changes and continues identically
  across a load at four points of the hour; a home started in the window lends the published
  books, a firm with inputs runs the planner's census; a census rebuilt mid-hour leaves the
  snapshot sweep unchanged; an arrival hired at the next hour joins the snapshot before its
  hire; the planned workforce follows a hire after the sweep; a pending service followed by an
  appended building keeps its ceiling; a road split records the moved rows and their undo; a
  policy change in the window reaches the next census and survives a save. The two-year
  starter town and the scarce-labour case, which failed on the first part-2 build, pass.

#### `SIM-02-I`, part 3 — construction permits on the published census (2026-10-18)

- **Save format 99:** each queued spawn's permit (`pending_demand_spawns.permit`: a digest of the
  door with the profile and level it was proved for) and whether the planning slot counted it,
  each building's applicant seed and the city's seed counter, and whether the freight supplier
  index's saved front and sweep were built against the saved road layout. The Kuopio fixture
  gained the columns and its hash in `placements.json`.
- **What moved off the hour:** the last censuses outside the hourly one. Construction execution
  ran a full census and a whole-city snapshot in the frame of each new epoch (about 288 ms at
  100k, at 18:01 and 20:01 in part 2's `PERF-07` day); demand planning ran its own census after
  a firm with recipe inputs started in the window. Planning now lends the published books with
  those buildings appended as buyers, and the hour's planning slot proves every queued
  commercial and industrial spawn once on the same books, saving a permit the start checks.
  See [Decided while unattended](#decided-while-unattended).
- **A part 1 defect fixed here:** the freight supplier index saved its front and sweep without
  saying whether they were built against the saved road layout. A game saved between a road
  edit and the next slot rebuilt the index differently after loading. The save now records it
  per buffer, and the continuation tests compare the index every frame.
- **A loading defect fixed here:** a road edit that rebuilds the junction lane a walker is
  crossing leaves the walker at the node with no edge; the running game replans it from the
  node, but loading kept only cars there and sent walkers straight into their destination, so a
  game saved at that moment diverged (a grocery sale a day later in the supplier index's road
  edit test). Loading now keeps any agent at a valid node ([`traffic.md`](traffic.md)).
- **Census work** (`populated_census_window_churn`, release): over 336 hours, 2,485,050 census
  building rows at 10k and 6,841,159 at 30k (2,566,390 and 8,469,975 with part 2's 11 and 80
  execution censuses); every row is now the hourly census's. No planner census, no skipped pass
  and no skipped epoch in any run.
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass. Every home's
  applicant draws changed with the stable seeds, so the cities differ slightly from part 2, all
  within bands: baseline net income per resident 13.32 at 10k and 12.46 at 30k (13.18 and
  12.34), bankruptcies 0.068 and 0.281 per 1,000 buildings a day (0.058 and 0.326), 10k vacancy
  5.6% (5.2%). The fiscal-stress 10k deliveries, outside their band since `SIM-02-A`, are back in
  it (13.24). The fiscal-stress employment notes remain (0.075 and 0.077 against about 0.052,
  final employment 0.68% and 0.79%). The busiest minute is at most 3.3% of an hour's departures.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days). New continuation tests save with queued business permits mid-hour, after a road
  split and a bulldozed stub (edges and lanes renumbered by the load), after a mid-hour road edit
  with the supplier index stale, and with a walker left at a junction node, and compare every
  frame for a day.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`). Max / p99 / mean in
  ms, part 2 in parentheses: 10k 1× 101.2 / 5.95 / 2.41 (71.1 / 5.93 / 2.34), 10k 32× 100.7 /
  7.47 / 3.00 (70.3 / 7.05 / 2.86), 30k 1× 106.3 / 7.55 / 3.32 (93.8 / 7.54 / 3.31), 30k 32×
  110.0 / 13.95 / 5.28 (89.8 / 23.44 / 5.38), 100k 1× 354.4 / 18.05 / 11.81 (338.3 / 18.67 /
  12.08), 100k 32× 460.6 / 57.76 / 21.97 (429.2 / 178.40 / 23.10). Every gate limit passes. The
  construction-start frames are gone: part 2's 100k day had two of 297 and 302 ms; the 32× p99
  fell to a third at 100k. The maxima rose:
  - At 10k and 30k the worst frames are lot regeneration, as in part 2: a slow-frame diagnostic
    on the same snapshots put 85–165 ms of the 10k 01:00 and the 30k 03:00 and 07:00 frames in
    the cell-lot queue, each after a planning-pass removal, while the generated cells were
    still cold from the load (`SIM-02-K`). With the stable applicant seeds, those removals fall
    in other hours of the measured day.
  - At 100k the worst frame is still the first hour after loading: its hour rose from 238 to
    249 ms, because the planning slot now replays the construction queue twice, once for
    planning and once for the permits. Part 4 moves the permits to a later slot.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 17.3–17.6 / 7.4–7.9 against 4.7–4.8 / 0.00;
  baseline 30k 62.1–63.5 / 47.7–47.8 against 18.6–18.9 / 0.00; fiscal 10k 27.6–28.0 /
  22.2–22.9 against 13.8–13.9 / 0.00; fiscal 30k 79.7–80.1 / 68.2–68.7 against 47.5–47.6 / 0.00.
  Six work-count limits fail, against seven on part 2 and on `SIM-02-A`: census building rows
  now pass. The fiscal-stress hours rose 5–8% over part 2's (13.1–13.2 and 43.3–44.2), the
  second queue replay at the planning slot.
- **Build identity:** the envelope, determinism, library and growth tests and `./run.sh --test`
  ran on the committed sources built with rustc 1.99.1. `PERF-07` and `PERF-01` ran on the same
  sources before the supplier index and loading fixes, which change only what a save records and
  how a load repairs agents; the envelope is identical with and without them. The `SIM-02-A`
  runs used its retained binary.
- **Tests:** 2,495 release library tests and the 4 growth integration tests pass; `./run.sh
  --test` passes in an isolated profile; clippy and rustfmt add nothing. New: queued homes
  estimate the applicants they draw once placed; a voided permit waits for the next planning
  slot and a denied one rejects; a failed home voids only the permits its applicants backed; a
  workplace completed after the copy keeps one claim until a census sees it; queued business
  permits survive edge and lane renumbering across a save; a supplier index left stale by a road
  edit, and a loaded walker at a node without an edge, continue identically. The tests written
  for the old prove-at-start model now drive permits.

#### `SIM-02-I`, part 4 — demand planning one zone type a slot (2026-10-19)

- **Save format 101:** the hour plan between its slots (`demand_hour_plan`) and, for a save in
  the hour's slots 0–2, what restores the census those slots read (`census_inputs.published`).
  The Kuopio fixture gained the table, the column and its hash in `placements.json`.
- **What moved off the hour:** the hourly demand pass. Residential, commercial and industrial
  planning run at slots 0, 1 and 2 into a saved hour plan; slot 3 plans downgrades, admits,
  queues the hour's spawns in the single pass's order, proves the permits and level changes and
  runs the clearances. Each slot is still a whole-city scan of its zone (`Cost::CityScan`, a
  `SIM-02-K` item). See [Decided while unattended](#decided-while-unattended).
- **Defects found and fixed:** queueing each zone's spawns at its own slot (the first build)
  changed every later start, building index and applicant seed, and in the starter town led to
  an industrial overbuild (12 factories, no grocery); slot 3 now queues the hour in the single
  pass's order. Two determinism defects surfaced on the way: arrival carriers held across the
  planning slots were lost by a save, and since `SIM-02-G` flow fields were rebuilt only on days
  a building was removed while a load rebuilt every field. A review then found that a save in
  slots 0–2 lost the census those slots read, that the new flow-field drain rebuilt every marked
  zone every frame (up to about 50 ms at 100k road nodes), that the drain could build fields
  from a stale zone index, and that a spawn failing its re-proof kept its place in the plan;
  all are fixed.
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass, within 0.05 net
  income per resident of part 3 in every city, every metric in band: baseline 10k 13.36 (13.32),
  bankruptcies 0.058 per 1,000 buildings a day (0.068); baseline 30k 12.43 (12.46), bankruptcies
  0.298 (0.281). The four fiscal-stress employment notes of earlier steps remain. The busiest
  minute is at most 3.3% of an hour's departures, unchanged.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days). New continuation tests save between the planning slots, after the actions slot,
  at slot 1 after a fiscal change, and in the planning slots with invalid books, and compare
  the hour plan, the published census at load and the flow fields every frame.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`). Max / p99 / mean in
  ms, part 3 in parentheses: 10k 1× 66.4 / 6.23 / 2.42 (101.2 / 5.95 / 2.41), 10k 32× 67.2 /
  7.11 / 2.88 (100.7 / 7.47 / 3.00), 30k 1× 82.6 / 7.55 / 3.33 (106.3 / 7.55 / 3.32), 30k 32×
  98.1 / 18.93 / 5.22 (110.0 / 13.95 / 5.28), 100k 1× 235.4 / 17.93 / 11.66 (354.4 / 18.05 /
  11.81), 100k 32× 359.9 / 128.44 / 21.99 (460.6 / 57.76 / 21.97). Every gate limit passes, and
  the worst frame fell at every tier. The 32× p99 rose at 30k and 100k because the hour's work
  now spans two frames: slot 3's actions, permits and clearances often fall in the frame after
  the one that crosses slot 0, so a day has up to 48 heavy frames instead of 24, each lighter
  (the 100k 32× frames outside the hour boundary peak at 170.5 ms against 84.4). Measured on the
  build before two debug assertions were added; release code is the same.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 17.2–17.3 / 8.0–8.6 against 5.7 / 0.00; baseline
  30k 63.1–63.2 / 46.8–47.6 against 20.9–21.0 / 0.00; fiscal 10k 27.8–28.6 / 22.4–23.3 against
  15.7–15.8 / 0.00; fiscal 30k 80.1–80.9 / 68.3–70.2 against 51.2–51.4 / 0.00. No time limit
  fails; five work-count limits fail (six on part 3; `owner_slots_prepared` now passes), all of
  which `SIM-02-A` also fails. The continuation hours now end after the hour's actions slot:
  ending at slot 0 split the hour's building churn across two counted hours, which failed the
  fiscal fixture's check that every hour has churn, and left the planning slots that fell in
  the next frame out of the hour's time. The hours are 8–21% above part 3's (4.7–4.8,
  13.8–13.9, 18.6–18.9 and 47.5–47.6), the four slots' repeated setup (`Cost::CityScan`).
- **Build identity:** the envelope, determinism, library and growth tests, `PERF-01` and
  `./run.sh --test` ran on the committed sources built with rustc 1.99.1; `PERF-07` ran on the
  same sources before two `debug_assert!`s and test-only changes were added, which do not
  compile into its release test build. The `SIM-02-A` runs used its retained binary.
- **Tests:** 2,505 release library tests and the 4 growth integration tests pass; clippy and
  rustfmt add nothing. `./run.sh --test` (isolated profile): the Rust suites pass, and the Godot
  run fails only `road_junction_preview_test`'s terrain-delta polls, the `ROAD-54` race below,
  which a 30-run comparison reproduced on part 3 at the same rate (2 runs in 30 each); part 4
  does not change the Godot bridge. New: failed re-proofs leave the plan and free their workers
  for later slots; the published census continues identically across a save in each planning
  slot; a save in the planning slots on invalid books skips the hour identically; the flow-field
  drain rebuilds a stale zone index first; flow fields skip unchanged sources and equal a fresh
  build. The starter town and the scarce-labour case equal the first part 4 build's.
- **Found and recorded:** `ROAD-54`, a race in the road preview's poll that predates this part
  (about one `./run.sh --test` run in 15 on parts 3 and 4).

#### `SIM-02-J`, parts 1 and 2 — freight requests and the household queues (2026-10-19)

- **Save format 102:** the freight request queue, the shipments' leg start hour and trip event
  slot, the buildings' cooldown slot, the freight departure leveler and next shipment id, the
  household request queue slot, the job request queue, the household queue state, the household
  destinations and the job board ([Request queues](#request-queues)). The Kuopio fixture gained
  the tables and columns (two of them renamed) and its hash in `placements.json`.
- **What moved off the hour:** freight creation (part 1); rehousing, restock and visit planning
  and job search (part 2), each queued at an entity's phase and served a few per slot, on two
  new hourly rolling snapshots (household destinations and the job board). Housing reviews
  leave the next hour for the housing queue. See [Request queues](#request-queues),
  [Snapshots](#snapshots) and [Decided while unattended](#decided-while-unattended).
- **Review fixes:** queue marks follow the queue; a held shopper's reservation ends with the
  household's move; a restock is queued only with a shopper free; dead store entries have their
  own cap; housing requests are served oldest first across rehousings and reviews; a searcher
  that finds no open job keeps its job under a new lock; the job board is a rolling snapshot
  instead of being built at the hour's first search; the destination roll saves whether its
  buffers are current. See [Decided while unattended](#decided-while-unattended).
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass. Against part 4,
  baseline net income per resident 13.48 at 10k (13.36) and 12.93 at 30k (12.43), arrivals 30k
  12.96 (12.19); the baseline 30k out-of-stock rate 0.185 (0.256), below its band. Out of band
  and explained: fiscal-stress deliveries 8.70 and 10.44 per 1,000 buildings a day (13.22 and
  12.68) and fiscal 10k net income per resident 27.25 (28.36, band low 28.04), startup freight
  delivered before the measured window (see
  [Decided while unattended](#decided-while-unattended)). The busiest minute holds 2.6–3.8% of
  an hour's departures, 4.3% for fiscal 10k shopping (2 of 46 in one hour); the busiest
  settlement hour is unchanged.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days). The continuation tests also check every frame that the queue marks equal the
  ones a load rebuilds.
- **Request rates** (debug channels, the envelope cities, 3 sampled days at 10k and 30k and 1 at
  100k; requests an hour, mean, with the busiest hour in parentheses): housing 217 / 628 /
  2,119; restock 399 / 968 / 5,593 (972 / 2,984 / 9,550); visits 15 / 34 / 25 under the
  64-an-hour credit; jobs 164 / 1,348 / 1,708, the 100k city's startup hours filling 64 a slot
  with up to 9,799 waiting; freight 54 / 146 / 600. No housing, restock or freight slot carried
  a request to the next.
- **Planning slots** (`populated_census_window_churn`, 24 hours, release): at 10k and 30k,
  baseline and fiscal stress, 96 planning passes each, with no slot or epoch skipped, no
  re-proof drop and no census restart.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`). Max / p99 / mean in
  ms, part 4 in parentheses: 10k 1× 89.5 / 6.27 / 2.38 (66.4 / 6.23 / 2.42), 10k 32× 95.0 /
  6.99 / 2.87 (67.2 / 7.11 / 2.88), 30k 1× 95.2 / 7.50 / 3.42 (82.6 / 7.55 / 3.33), 30k 32× 95.0 /
  13.41 / 5.69 (98.1 / 18.93 / 5.22), 100k 1× 123.5 / 18.24 / 12.67 (235.4 / 17.93 / 11.66),
  100k 32× 249.0 / 60.31 / 25.09 (359.9 / 128.44 / 21.99). Every gate limit passes. The hour frames are small now:
  their maximum is 10–12 ms at 10k and 27 ms at 30k. At 100k the worst frame fell by half and
  the 32× p99 from 128 to 60 ms. The 10k and 30k maxima are single frames outside the hour
  boundary: a slow-frame diagnostic on the same snapshots found the hour's planning removal of a
  building in each (10k 13:00, 30k 01:00) and no other large work, the freed lot region served
  from cold generated cells, as in parts 2–4 (`SIM-02-K`). The 100k means rose 9–14% (and 30k
  32× 9%): the queues' work moved from the hour frame into ordinary frames, where `SIM-02-K`
  bounds it per slot.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 16.9–17.8 / 7.1–7.6 against 3.2 / 0.00; baseline
  30k 62.1–62.2 / 46.7–50.4 against 10.2–10.4 / 0.00; fiscal 10k 27.6–28.1 / 22.5–23.2 against
  13.2–13.3 / 0.00; fiscal 30k 78.5–80.5 / 68.3–71.2 against 39.8–40.3 / 0.00 (part 4: 5.7,
  15.7–15.8, 20.9–21.0 and 51.2–51.4 hourly). No time limit fails. Six work-count limits fail
  (five on part 4); the new one is the baseline city's CCH point searches, 3.47× from 10k to
  30k against a limit of 3.43×, which `SIM-02-A` passes: the queues route each request on its
  own, and the 30k fixture's job queue serves 8× the 10k one's searches (1,348 against 164 an
  hour, its job churn, the same cause as the `job_search_keys` scaling `SIM-02-A` also fails),
  while its households number 2.86× the 10k one's.
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1;
  the `SIM-02-A` runs used its retained binary.
- **Tests:** 2,521 release library tests and the 4 growth integration tests pass; clippy and
  rustfmt add nothing. `./run.sh --test` (isolated profile): the Rust suites and every headless
  Godot bridge test pass; the rendered vegetation level test on the desktop display was
  terminated before it finished, most likely by the isolated run's 600 s Godot watchdog with the
  display asleep (it passed on every earlier run). The step does not change the Godot bridge. `held_trips_leave_evenly_over_the_following_hour` now paces
  through `schedule_arrival_departures`, and the census case
  `an_arrival_hired_at_the_next_hour_joins_the_snapshot_as_before_its_hire` queues its arrivals'
  job searches at the hour's last slot.

#### `SIM-02-J`, part 3 — level changes, removals, departures and admissions as queues (2026-10-20)

- **Save format 103:** the `level changes`, `building removals`, `departures` and `admissions`
  queues, the level-change permits and the admission credit. The Kuopio fixture gained the tables
  and its hash in `placements.json`.
- **What moved off the hour:** slot 3 still plans and proves the hour's level changes and decides
  its admissions, but executes neither: level changes run from slot 4 under their saved permits,
  admitted households are created over the following hour by a credit, and clearances, expired
  compatibility removals and household exits are served only in slots 4–479, where the filling
  census copy mirrors swap-removes. See [Request queues](#request-queues) and
  [Decided while unattended](#decided-while-unattended).
- **Envelope and bursts** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass, within 1% of
  parts 1–2 in every city: baseline net income per resident 13.58 at 10k (13.48) and 12.97 at
  30k (12.93), 30k bankruptcies 0.298 per 1,000 buildings a day (0.309). The notes are parts
  1–2's (fiscal-stress deliveries 8.57 and 10.30, fiscal 10k net income 27.31, the 30k baseline's
  out-of-stock rate 0.183 below its band, the fiscal-stress employment notes). The envelope cities
  do not exercise departures or level changes (no household stays unhoused for a week; every
  bundled asset is level 1), so only admissions and clearances move, by a slot or so. The busiest
  minute holds at most 3.8% of an hour's departures; the arrival burst is unchanged (3.2%).
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days). A continuation test saves with all four queues waiting and compares every frame.
- **Census and planning slots:** 168 hours of the 10k fiscal-stress city removed 420 buildings,
  all outside the census window: no census restart, and no planning or epoch skip in 672 planning
  passes (`populated_census_window_churn`).
- **Slot 3** (`perf07_planning_slots`, 6 hours, pinned, both builds on the same cities, parts 1–2's
  snapshots migrated): 30k 1.88 / 2.34 ms mean / worst (2.96 / 4.82), 100k 9.25 / 12.42 ms (21.78
  / 25.93). At 100k slot 4's frame now carries the clearances' zone-index and entrance-cache
  repairs, about 10 ms; the worst frame across slots 3 and 4 is 31 ms. An incremental repair is a
  `SIM-02-K` item.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`). Max / p99 / mean in
  ms on this build's own snapshots: 10k 1× 64.6 / 7.19 / 3.37, 10k 32× 68.6 / 8.27 / 3.75, 30k 1×
  108.2 / 8.62 / 4.49, 30k 32× 99.7 / 15.90 / 7.05, 100k 1× 120.3 / 18.55 / 12.91, 100k 32× 200.7
  / 51.42 / 26.68. Every gate limit passes. These ran after the machine restarted onto a newer
  kernel, where the same binary on the same snapshot measures a mean about 30% higher than
  before (parts 1–2 at 30k 1×: 3.42 ms before the restart, 4.43 after), so they are not
  comparable with the parts 1–2 results above. `CODE-17` later traced the rise to handing work
  into Rayon's global pool from outside it; its owned pool brings the 10k and 30k means back
  (30k 1× 3.39 ms). Same-session control on the same 30k city (parts
  1–2's snapshot, migrated): parts 1–2 4.43 / 8.35 / 116.5 mean / p99 / max, part 3 4.43 / 8.30 /
  99.9; on its own snapshot part 3 4.44 / 8.46 / 90.9. At 100k the 32× worst frame fell from 249
  to 201 ms and the p99 from 60 to 51 ms even on the slower machine.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with `SIM-02-A` (hour / day
  means in ms, `SIM-02-A` first): baseline 10k 17.1–17.5 / 7.5–7.6 against 3.2–3.3 / 0.00;
  baseline 30k 63.6–67.2 / 50.7–52.7 against 10.1 / 0.00; fiscal 10k 28.5–28.6 / 22.5–23.7
  against 11.8–12.2 / 0.00; fiscal 30k 80.9–81.2 / 70.2–70.6 against 36.2–36.6 / 0.00. No time
  limit fails; the same six work-count limits as parts 1–2 fail.
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1;
  the `SIM-02-A` runs used its retained binary.
- **Tests:** 2,525 release library tests and the 4 growth integration tests pass; clippy and
  rustfmt add nothing. New: a save with all four queues waiting continues identically; a removal
  requested inside the census window waits for slot 4 and restarts nothing; a stale level-change
  permit is rejected. The step does not change the Godot bridge.

#### `SIM-02-K`, part 1 — legacy hour and day paths deleted (2026-10-20)

- **Deleted:** `simulate_tick_internal`, `simulate_operational_hour_internal`,
  `refresh_funding_census` with the census copy's live mode, the whole-hour drivers of the
  rolling snapshots, `run_every_hour_phase`, `run_every_day_phase` and `Cost::LegacyBoundary`. No
  game code runs because the clock crossed an hour or a day. Tests, benchmarks, the economy bench,
  `populated_city_scaling` and the contract harness step the slot clock.
- **Costs declared:** the census publication and the four planning slots `CityScan` (`SIM-02-K`
  part 5), the operational hour's remainder `Cost::Dirty` (proportional to entities created,
  admissions queued, arrivals in flight and expired commute quotes), the calendar close
  `Constant`. The operational hour and residential planning are now two declarations at slot 0,
  in the same order. See [Decided while unattended](#decided-while-unattended).
- **Envelope and determinism** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass; the envelope is
  bit-identical to the previous build's, and repeat, 1 and 3 workers and save/reload are identical.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`), in one session with the
  `CODE-17` build as control. Max / p99 / mean in ms, control in parentheses: 10k 1× 61.3 / 6.21 /
  2.44 (61.4 / 6.21 / 2.42), 10k 32× 63.3 / 7.12 (63.2 / 6.90), 30k 1× 86.0 / 7.47 / 3.40 (86.5 /
  7.54 / 3.44), 30k 32× 90.0 / 12.99 (91.7 / 13.24), 100k 1× 116.8 / 18.29 / 12.80 (116.2 / 18.10 /
  12.54), 100k 32× 182.7 / 48.81 / 24.70 (183.4 / 48.59 / 25.11). Every gate limit passes; level
  with the control.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the control: hour means
  2.8, 10.8–11.0, 8.5 and 32.2–32.3 ms (control 2.8, 10.8–10.9, 8.5 and 31.7–32.3); the same six
  work-count limits fail. The contract's census phase now times the input copy, the window's
  steps and the publication together, so it is not comparable with earlier steps' census phase.
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1.
- **Tests:** 2,531 release library tests and the 4 growth integration tests pass; the benchmarks
  build; clippy and rustfmt add nothing. Tests that drove the deleted hour now step the hour's
  slots with the same expected numbers. The step does not change the Godot bridge.
- **Fix (debug build):** two `building_site_terrain` tests tripped the save-time check that
  valid census books in an hour's planning slots keep the copy their census read. Both load
  the Kuopio fixture and save without moving the clock. The fixture is at day 1, 00:00, and
  its `census_inputs.published` was NULL (part 4 added the column without a value); a load
  reads NULL as a save after the actions slot, counts the loaded city and marks its books
  valid without a kept copy. The deleted live-copy exception (`!derive_funding`) had also
  covered that copy. No game save holds this state: a world's first slot never runs, so a new
  world saved before its first publication writes books that are not valid, and every
  publication, the whole-census fallback included, keeps its copy. The fixture now holds
  that value (what a re-save of it writes) and its new hash in `placements.json`; the check
  is unchanged. A census test saves in slots 0–2 after a whole census at the publication,
  both on the window's copy (an edit past the window's last slot) and on a copy taken then
  (an undo there), and continues identically. Debug and release library suites (2,532 each),
  the growth tests and the gate (bit-identical envelope and determinism) pass; clippy adds
  nothing. Alternatives rejected: refusing such a save at load (a release load deliberately
  counts the loaded city), loading NULL as books that are not valid (a shim for a state no
  game writes), and editing the two tests (the bad state would stay for every other user of
  the fixture). From this fix on, every step's gate runs the debug library suite as well.

#### `SIM-02-K`, part 2 — render snapshot culls before posing (2026-10-20)

- **What changed:** the snapshot culls before posing and poses and buckets on the shared pool;
  its instances are those of the serial full pose, bit for bit. Gameplay does not change.
  - **Cull.** A lane agent's pose interpolates two consecutive points of its lane, or, while a
    car changes lanes, blends the source and target lane at the same distance, so it lies in
    the planar box of those lanes' points. The cull skips an agent only when that box, widened
    1 m for interpolation rounding (valid for coordinates below 2^20 m), misses the camera. A
    lane-less agent (egress, ingress, a border start) is posed at its stored position, which
    the cull tests exactly. Lanes with fewer than two points or a non-finite coordinate, and
    non-finite lane distances, are always posed. Survivors are posed by the unchanged code and
    tested exactly. The cull tests the instance origin, as the snapshot always did, so vehicle
    length and render extent do not enter it. Lane boxes are cached in the lane system by
    revision and lane count and rebuilt in parallel after a lane rebuild.
  - **Surface query.** Each site query (support height, yard plants, planting) skips a site
    whose lot centre is farther from the point than the exact maximum site radius plus 0.5 m
    before reading its polygons, and a vehicle's 13 contact samples share one gathered set of
    nearby sites. The candidates, chunk ranges and lowest-index rule are unchanged, so the
    answers are. The query stays bounded by the sites in the 512 m chunks around the point
    (local density, not city size). The agent's own building cannot answer alone: the
    lowest-index containing site wins, and pads may touch or overlap within the polygon test's
    1 mm tolerance.
  - **Parallel.** Agents are culled and posed in 2,048-agent index chunks; each bucket then
    appends its instances in chunk order, which is agent order. Cost: O(agents) for the cull
    (one box test each), O(agents in the camera) for posing.
  - **Not done:** posing outside the core lock. It would need a copy of the in-camera agents
    and shared lane geometry; after part 2 the whole snapshot averages under 1 ms at 100k.
- **Equivalence:** a 2k populated city compares the culled snapshot with the serial full pose
  on 5,828 frames at 1× and 32× (identical), and 292,820 surface points and vehicle poses; the
  prepared 30k and 100k cities 3,301 frames each (identical). Lane changes do not occur in the
  populated fixtures, so a synthetic two-lane test
  (`culled_snapshot_matches_full_pose_through_lane_changes`) covers them. The populated-city
  comparison is an ignored release test in `frame_budget/snapshot_cull.rs`, outside
  `populated_city.rs`, whose bytes feed the snapshot fixture revision.
- **Envelope and determinism** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass; the envelope is
  bit-identical to part 1's, and repeat, 1 and 3 workers and save/reload are identical.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`), full day, in one
  session with the part 1 build as control. Max / p99 / mean in ms, control in parentheses:

  | Tier | Speed | Max | p99 | Mean | Snapshot mean |
  | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 59.6 (63.6) | 1.25 (6.25) | 0.54 (2.47) | 0.29 |
  | 10k | 32× | 61.0 (62.7) | 2.72 (7.17) | 1.07 (2.71) | 0.38 |
  | 30k | 1× | 83.5 (87.8) | 2.29 (7.49) | 1.30 (3.40) | 0.41 |
  | 30k | 32× | 89.2 (90.7) | 9.15 (13.06) | 3.32 (5.63) | 0.57 |
  | 100k | 1× | 109.9 (117.4) | 8.45 (18.62) | 6.04 (12.96) | 0.89 |
  | 100k | 32× | 176.8 (186.6) | 40.77 (49.71) | 17.20 (25.12) | 0.91 |

  Every gate limit passes. The maxima are single slot frames the snapshot never dominated (10k
  at 21:00, 30k at 01:00, 100k at 00:00 and 12:00; parts 3 and 5), and the 100k 32× frames
  keep about 8.7 ms of agent tick (part 4). A diagnostic on the part 1 snapshots with the part 1
  build plus the new columns as control measured the snapshot alone (mean / p99 ms, 1× over
  the first 20 game minutes, 32× over the day): 100k 6.67 / 8.06 → 0.81 / 1.11 at 1× and
  8.74 / 12.80 → 0.92 / 1.41 at 32×; 30k 1.67 / 2.25 → 0.34 / 0.64 and 2.77 / 7.25 → 0.58 /
  1.09; 10k 1.36 / 3.20 → 0.15 / 0.19 and 1.89 / 5.65 → 0.39 / 1.04.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the control: hour means
  2.8, 8.6–8.7, 10.9–11.4 and 31.6–31.9 ms (control 2.8–2.9, 8.6, 10.6–11.1 and 32.2); the same
  six work-count limits fail.
- **Build identity:** the timing runs and the first gate ran on the part's sources on part 1
  (rustc 1.99.1); the commit gate reran the release tests, envelope, determinism, clippy and
  `./run.sh --test` on these sources on top of the part 1 fix.
- **Tests:** 2,533 library tests pass in the release build and in the debug build (`./run.sh
  --test`), with the 4 growth integration tests; the benchmarks build; clippy and rustfmt add
  nothing. The snapshot is bridge-facing, so the isolated `./run.sh --test` ran: every bridge
  and asset test passes with no script errors, after `agent_inspector_test` was made to wait for
  the core's route answer (`DEBUG-08`), which the first run hit.

#### `SIM-02-K`, part 3 — frame spikes (2026-10-20, first commit)

- **Attribution** (temporary per-system timing of every slot system and of the cell-lot, removal
  and census steps, test build, pinned, K1 snapshots, one run each, not acceptance timing). The
  K2 build's worst frames:

  | Tier | Frame (ms) | Where it went | Item |
  | --- | --- | --- | --- |
  | 10k 1× | 1260, slot 4 (65.9) | one cell-lot region, 64.5 ms generating 9 cold chunks | K3 (a) |
  | 10k 1× | 1260, slot 3→4 (12.7) | a clearance, 11.5 ms: the building site query index built for the first time since loading (10 ms), the entrance-cache rebuild (0.7) and zone index (0.3) | K3 (a), (c) |
  | 10k 32× | 1260 (60.0) | one cold region, 57.0 ms (9 chunks) | K3 (a) |
  | 30k 1× | 60 and 1080, slot 4 (83.4, 32.5) | cold regions, 81.8 ms (9 chunks) and 31.2 ms (4) | K3 (a) |
  | 30k 1× | 60 and 120, slot 0 (21.4, 17.5) | residential planning 18.7 and 15.0 ms | K5 |
  | 30k 32× | 60 (94.7), 1080 (43.7) | cold regions 86.5 and 31.9 ms; clearances 3.3 and 2.7 ms (entrance rebuild 2.1 and 1.6); planning 1.6–5.0 ms | K3 (a), (c), K5 |
  | 100k 1× | 0, 60, 120, 240, slots 4–7 (112.2, 89.6, 86.5, 70.5) | cold regions, 65–83 ms (9 chunks each); at 00:00 also the first hour's job searches after loading, 16.5 ms | K3 (a); the job queue's first hour after loading is not a K3 item |
  | 100k 1× | 60 and 120, slot 0 (59.2, 49.9) | residential planning 48.9 and 40.1 ms | K5 |
  | 100k 32× | 720, 60, 120, 240, 300, 840 (174–132) | cold regions 75–134 ms (9–18 chunks); clearances 10–29 ms (entrance rebuild 5.7–5.9, zone index 2.5–2.6); planning slots 8–57 ms; agent tick 8.4–9.7 ms (24 ms at 05:00) | K3 (a), (c); K5; K4 |
  | 100k 1× | census slots (≤ 23.0) | the unsliced consumer-market refresh (`customers`) 23.0 and 9.8 ms; `current_staff` slices 9–20.6 ms; `household_claims` 9–14 ms; `routes` 9.9 ms | K3 (b), not done |
  | 100k 1× | first frames after loading (17.0, 15.3) | trip events 10.2 and 8.9 ms on cold route caches | K3 (d), not done |

- **(a) Generated cells at load.** Saves leave generated cells out, and loading marked every chunk
  cold, so the first lot region served next to a chunk generated it and its eight neighbours
  whole. Loading now generates every cold chunk within reach of a road that grows cells
  (half-width, sidewalk and eight cell rows) or holding reserved cells, in chunk order, and
  completes it (`CellStore::warm_cold_chunks`, called from `SimCore::warm_loaded_cells`). A chunk
  whose cells would need a grid frame the store has not registered stays cold
  (`CellStore::generate_on_known_frames`, which checks before publishing anything): frame ids
  follow registration order, so registering a frame at load that the saved game registers only
  when it later generates that chunk would give the two games different ids. No save format
  change. At 10k, 30k and 100k loading generates 38, 68 and 208 chunks and leaves none cold;
  `PERF-07` load time rises from 1.57 to 1.77 s, 4.44 to 4.89 s and 16.93 to 18.50 s. A
  deferred rebuild remains only for chunks needing an unregistered frame (none in the fixtures)
  and after a catalog refresh, which invalidates every chunk as before.
- **(c) Clearances.** A building's entrance is a function of its own building, the network and
  the asset registry, so a clean entrance cache now follows a removal's swap-remove
  (`remove_building_at_index`: swap-remove, entrance revision bumped, every zone's flow fields
  marked for comparison, as the rebuild does), and the removal queue rebuilds only a stale cache:
  −5.8 ms a clearance slot at 100k. The zone index is still rebuilt once a clearance slot, O(B),
  2.6 ms at 100k: that rebuild is also the `ALLOC-03` reset of the vacancy lists to building
  order, and an exact incremental equivalent still has to recompute every building's vacancy
  membership (see Decided while unattended). Within a slot that clears several buildings, a later
  removal's evacuation trips now read the current entrances, where they read the stale cache the
  first removal left.
- **(d) First frame after loading.** Loading builds what the first frame did
  (`SimCore::prepare_loaded_pathing`): the routing hierarchy if dirty, every flow field loading
  marked (when the loaded zone index and entrance cache are current) and the movement pass's
  route scratch for the pool's workers: the 100k 1× day's longest agent tick fell from 33.1 to
  22.7 ms (the first frame's scratch sizing was 31 ms of it). The first frames' trip events
  (8.9–10.2 ms at 100k) stay: they fill route and quote caches that saves do not keep.
- **(e) Test-build checks.** `timing_mode()` (`PERF-01`, `PERF-07`) also switches off the
  operational hour's test-build checks (freight reservations against a fresh build, household
  member lists against a rebuild, the published census's input supply) through
  `debug::work::set_city_checks`, as it switches off `VERIFY_LIVE_PASSES`. Ordinary tests and the
  envelope keep them.
- **(b) not done** (next K3 commit). Budgeting the census by staffing operations and trials would not bound its
  worst slot: at 100k a 20.6 ms `current_staff` slice had 2,408 trials, 2,106 operations and 973
  CCH point searches, while a 16 ms slice had 6,576 trials, 5,099 operations and 93 searches, so
  the cost follows the supplier routes and searches the staffing quotes trigger, not the trials;
  and the worst census slots are the unsliced consumer-market refresh (23 ms) and route step
  (10 ms), which no budget splits. Options in Decided while unattended.
- **Equivalence.** Load-time generation equals the cells the store had before saving and
  registers no frame (`warming_regenerates_saved_frames_and_leaves_unregistered_ones_cold`); a
  removal's entrance cache equals a rebuild
  (`building_removal_keeps_a_clean_entrance_cache_equal_to_a_rebuild`).
- **Envelope and determinism** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass after (a) and
  (e), and again after (c) and (d); both times the envelope is bit-identical to K2's, and
  repeat, 1 and 3 workers and save/reload are identical. The second run's binary predates a
  test-only fix (the flow-field drain test expected the entrance cache stale after a removal);
  the library code is the same.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`), full day, each build
  on its own prepared cities, in one session with K2 as the control. Milliseconds and frame
  counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 9.2 (58.7) | 1.24 (1.25) | 1.49 (1.58) | 0.54 (0.54) | 2 (2) | 0 (1) |
  | 10k | 32× | 11.2 (60.3) | 2.63 (2.70) | 5.29 (7.63) | 1.04 (1.09) | 2 (2) | 0 (1) |
  | 30k | 1× | 21.3 (84.8) | 2.31 (2.28) | 4.08 (4.07) | 1.32 (1.31) | 8 (9) | 2 (5) |
  | 30k | 32× | 23.1 (89.1) | 8.59 (9.20) | 22.60 (25.28) | 3.31 (3.38) | 33 (38) | 3 (7) |
  | 100k | 1× | 60.1 (112.2) | 8.46 (8.38) | 18.03 (19.68) | 6.10 (6.00) | 1,429 (1,206) | 131 (178) |
  | 100k | 32× | 81.9 (179.0) | 37.06 (39.79) | 79.87 (166.14) | 16.87 (17.17) | 2,700 (2,700) | 701 (656) |

  The control repeats K2's own run within about 2% on every maximum.
  The worst frames left: at 10k and 30k the hour's residential planning slot (8.5 and 19.7 ms of
  slot work, K5); at 100k 1× two frames in the census window (minutes 517 and 529, 54 and 34 ms
  of slot work outside the hour's planning slots; the instrumented runs, which covered the
  day's first five hours at 1×, measured census slices of up to 23 ms there, in the unsliced
  consumer-market refresh and `current_staff`) and the residential planning slot (52 ms); at
  100k 32× the planning slots (57–62 ms in one frame) with 9–10 ms of agent tick (K5, K4).
  Every gate limit passes for both builds.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with K2 as control: hour
  means 2.55–2.57, 7.63–7.66, 10.35–10.89 and 31.75–31.87 ms (control 2.84–2.85, 8.62–8.67,
  10.89–11.05 and 32.15–32.29). The baseline hours are 10–12% cheaper because the timed hours
  no longer run the test-build checks (e); the same six work-count limits fail.
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1;
  the `PERF-07` control is K2's retained binary. The commit gate reran the release tests,
  envelope, determinism, clippy, `./run.sh --test` and `PERF-01` on these sources on dev.
- **Tests:** 2,535 library tests pass in the release build and in the debug build (`./run.sh
  --test`), with the 4 growth integration tests; clippy and rustfmt add nothing. Loading changed,
  so the isolated `./run.sh --test` ran: every bridge and asset test passes with no script
  errors.

### Phase inventory

Every phase of today's operational hour, hourly demand pass and daily tick, and where it goes.

| Today | Cost today | Target | Step |
| --- | --- | --- | --- |
| `publish_frontage_prices` | O(lanes), par | `FixedSlot` hourly | C |
| `advance_construction_hour` | O(buildings); a completion rebuilds the zone index | Completion event; incremental zone-index update | D (done) |
| Farm and mine `produce_hourly` | O(sites) | `EntityPhase` hourly, at the site's building | F (done) |
| `electricity_funding_by_building` | O(buildings) | Rolling snapshot | I |
| `materialize_arrived_household_carriers` | O(agents) scan | Carrier-arrival event | E (done) |
| `rebuild_household_and_worker_counts` | O(agents + households + buildings) | Member lists maintained by the agent lifecycle; each household recounts at its hourly phase; worker counts follow job changes | I (done) |
| `sync_agent_money_from_households` | O(agents) | Agent `money` mirror deleted (never read); employed adults follow job changes and the household's phase | F (done), I (done) |
| `rehouse_unhoused_before_growth` | O(households) when any unhoused | Rehousing queued at the household's hourly phase; `housing requests` queue on the live vacancy lists | J (done) |
| `run_building_economy` production, `refresh_commercial_activity_floor` | O(buildings) | Production at the building phase; the floor follows the funding census | F (done), I (done) |
| `settle_hourly_service_store_sales`, `charge_households_for_aggregate_service_sales` | O(households + buildings), global pool | Pool as rolling snapshot; each household charged at its phase | I (done) |
| `run_electricity_hour` | O(households + buildings), one city-wide coverage | Coverage as rolling snapshot; billing at phase. Since F it bills the activity of each building's phase | I (done) |
| `decrement_building_cooldowns` | O(buildings) | Passive deadline | D (done) |
| `FreightPlanningContext::build` | O(nodes + edges + buildings), rebuilt hourly | Rolling supplier index; borders, components and speed cached by revision; reservations kept by shipment changes | I (done) |
| `create_profile_input_shipments`, `create_profile_output_exports` | O(buildings), serial reservations | Requests queued at the building's hourly phase; `freight requests` queue; cooldowns as slot deadlines | J (done) |
| `progress_shipments`, `dispatch_ready_carriers` | O(open shipments); all trucks leave on the hour | Dispatch and arrival events; departures levelled per minute; leg timeouts at trip checks | E (done), J (done) |
| `progress_households_for_operational_hour` | O(households) | `EntityPhase` hourly | F (done) |
| `apply_starvation_happiness_loss` | O(agents) | At the agent phase | F (done) |
| `process_active_household_shopping` | O(households) scan | Shopper events; leg timeouts at the household phase; shopper departures levelled per minute | E (done), F (done), J (done) |
| `process_visible_service_visit_returns` | O(agents) scan | Visit-end event | E (done) |
| `plan_and_apply_household_replenishment` | Indices rebuilt hourly when needed; since F a parallel O(households) scan finds whether any household can restock | Restock queued at the household's hourly phase with a shopper free; `restock requests` queue on the rolling household destinations | J (done) |
| `plan_and_apply_visible_service_visits` | Capped at 64 per hour | Visit queued at the household's hourly phase; `visit requests` queue paced by a 64-an-hour credit | J (done) |
| Funding census (`job_funding.refresh`) | Superlinear (`PERF-03`/`PERF-04`) | Hourly census as resumable steps over the hour's second half on a rolling input copy, published at slot 0; planning and construction permits read it, no other census runs | I (done) |
| Job search | Gated by the job lock (`job_lock_until_day`) | Search queued at the agent's hourly phase; `job requests` queue on the rolling job board; the lock stays a day deadline | J (done) |
| `prepare_cell_lots_internal` | O(dirty regions), can spike | Dirty-region queue with quota | H (done) |
| Demand snapshot (`from_runtime_with_funding`) | O(households + buildings + nodes) hourly | Rolling snapshot; entities created since its start folded before the next hour's job search; workforce counted live at the pass (`K` maintains it) | I (done) |
| Demand planning (`plan_private_building_actions`) | Scans all parcels hourly | One slot per zone type (slots 0–2) into a saved hour plan; slot 3 admits, queues and executes; each slot `Cost::CityScan` (per-zone lot lists are a `K` item) | I (done) |
| Household admission | Bounded; all carriers leave on the hour | Admission decided at slot 3; `admissions` queue paced by a credit over the hour; departure events | E (departures done), J (done) |
| Construction queue | One start per minute | Timed events | C (done) |
| `execute_building_actions` | Plan-bounded | Proved at slot 3 with a saved permit; `level changes` queue with quota from slot 4 | J (done) |
| `allocator.maintain`, `cleanup_stale_buildings` | O(buildings), serial | Compatibility check at building phase; removals queued, served in slots 4–479. `cleanup_stale_buildings` stays immediate: it runs only from editing paths, which delete the parcels in the same edit | G (done), J (done) |
| Pollution, noise, desirability | O(grid) plus O(buildings + edges) emission | Rolling grid sweeps | H (done) |
| Daily ledger begin and finish, `reset_daily_ledgers`, `reset_daily_input_accumulators` | O(households), O(buildings) | Each entity rolls its own window at its phase | G (done) |
| `job_lock_days` decrement | O(agents) | Passive deadline | D (done) |
| Bankruptcy, wages, property tax, utilities, profit tax | Serial over buildings and agents | Per building at its daily phase, same internal order; wages at each worker's phase | G (done) |
| `pay_household_transfers` | Serial over households until the treasury runs out | Per household at its phase; slot order decides who is paid | G (done) |
| `resolve_household_housing` | Serial over households | Check at phase enqueues housing reviews, served by the `housing requests` queue | G (done), J (done) |
| `workplace_route_cache.clear` | Daily clear | Entries expire 24 passes after they were added | G (done) |
| Road upkeep sum | O(edges) | Rolling road-length sum; upkeep charged at the calendar close | G (done) |
| `run_daily_pass`, `execute_demand_household_removal` | O(households) | Exit check at phase, paced by the daily exit credit; `departures` queue in slots 4–479 | G (done), J (done) |
| `finalize_daily_tax_window`, `record_daily_budget_ledger` | Serial over ledgers | Calendar close over running sums, O(zones) | G (done) |
| `agents.daily_update` | O(agents), par | `EntityPhase` daily | G (done) |

Per-frame costs found by the `SIM-02-A` `PERF-07` baseline, outside the boundary passes:

| Today | Cost today | Target | Step |
| --- | --- | --- | --- |
| Render snapshot (`build_snapshot_reusing`) | 3–12 ms: a lane pose for every agent before camera culling, serial | Cull before posing; bucket in parallel | K (part 2 done: 100k mean 6.7 → 0.8 ms at 1×, 8.7 → 0.9 ms at 32×) |
| Agent movement in departure frames | Up to 4 ms at 10k and 17 ms at 30k (32×) | Departures spread by events; `PERF-06` limits | E, K |
| One queued construction start | 14–21 ms for a single placement | Placement split across slots | K |
| First boundary after loading | About 10× an ordinary hour. `SIM-02-H` found most of it was the catalog refresh every load made, now skipped when the catalog is unchanged. What remained were the generated cells, which loading invalidated and the first lot regions served afterwards rebuilt: 25–134 ms a region at 100k, all day | Generated during loading (K3), except chunks that would register a new grid frame | H (refresh, done), K (K3 done) |
| 100k fixture traffic (found in `SIM-02-E`) | About 45,000 of 145,000 agents moving at any time (30,000 on lanes, 15,000 in junctions) and 15,000 shopping trips in flight around the clock, on `SIM-02-D` too: the network is saturated, so movement and trip checks run for most of the city | Find why trips stall at 100k (junction throughput, routing); movement within `PERF-06` limits | K |

### Open questions

1. **Demand planning slicing.** Resolved in `SIM-02-I` part 4: residential, commercial and
   industrial plan in slots 0, 1 and 2 of the hour into a saved hour plan, and slot 3 admits,
   queues the hour's spawns in the single pass's order and runs the actions; see
   [Decided while unattended](#decided-while-unattended). Per-district planning remains an
   option for `SIM-02-K` if a zone slot grows past its budget.
2. **Hot dense references.** Resolved in `SIM-02-B`; see
   [Entity handles](#entity-handles).
3. **Pure-job latency and pool size** per job.
4. **Behaviour envelope.** Resolved 2026-10-15; see
   [Behaviour envelope](#behaviour-envelope-sim-02-a).
5. **Electricity stability.** Dispatch against coverage one period old may oscillate. Checked in
   `SIM-02-F`: no loop exists yet. Coverage is still computed in one hourly pass from the
   activity of the previous hour's phases, and production never reads coverage, only billing
   and the daily meters do. The question returns in `SIM-02-I`, when coverage becomes a
   rolling snapshot. Resolved in `SIM-02-I`: each phase meters with the last finished hour's
   coverage and each plant generates that hour's dispatch of its current capacity, so both lag
   exactly one hour. Still no loop: dispatch and coverage are functions of the last hour's
   demand and capacity; demand comes from production activity and residents, never from
   coverage, and capacity from staffing, upkeep stock and coal. Coal is the only path back
   (burned at the lagged dispatch, it lowers the next hour's capacity), and generation never
   exceeds the current capacity, so a plant cannot burn coal it lacks; a shortage reaches
   coverage one hour later and settles there. Over unchanged demand and capacity the hourly
   totals equal the old pass's. A change shows for one hour: after capacity rises, plants
   generate the old share of the larger capacity, more than demand; after it falls, or a plant
   is removed or runs dry, meters still record the last hour's coverage; a city that had no
   capacity generates nothing in a new plant's first hour.

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
- **`SIM-02-H` — environment grids: emissions first, diffusion second.** The midnight pass rebuilt
  pollution, noise and desirability at 00:00 from the state at midnight. The sweep folds building
  handle slots and road edges over the day's first 11,520 slots, diffuses rows over the rest and
  swaps at the last slot, so readers see the previous day's sweep, built from the buildings and
  roads of its first 12 hours. Alternatives: diffuse each row band as soon as its emitters are
  folded (emitters are in handle order, not row order, so no band is complete before the
  emission half ends), or two day-long sweeps, one per stage (grids two days old). Evidence: the
  three grids are read only by the map overlays, so no simulation result depends on when they
  publish; every envelope metric equals `SIM-02-G`'s.
- **`SIM-02-H` — one O(grid) step a day.** The sweep's first slot zeroes the pollution and
  noise backs. At the default 40 m cells over 20 km that is a memset of 250,000 cells (1 MB)
  per grid. Alternative: a third buffer per grid, cleared in chunks during the diffusion half,
  for 2 MB more memory and a second cursor.
- **`SIM-02-H` — cell lots: one region per slot.** The `cell lots` queue serves at most one
  512 m region per slot, 960 an hour, oldest first; editing paths still drain every pending
  region at once, because their own previews and commits read the lots. A catalog refresh,
  which every load makes because the live asset registry replaces the saved one, is not saved
  and is served only in slots with nothing else queued, so it never delays the regions a save
  queued. Alternatives: a quota that grows with the queue (no per-slot bound), or draining the
  refresh during loading. Evidence: every envelope metric equals `SIM-02-G`'s, so the hourly
  demand pass placed the same buildings on lots served by the queue.
- **`SIM-02-H` — a load keeps lots derived with the same catalog.** Every load used to refresh
  the catalog, because the live asset registry replaces the saved one. The first gate run
  served that refresh one region per idle slot: about 10 ms a region at 30k, so after loading
  the 30k city ran hundreds of frames over 8 ms at 1× and up to eight regions a frame at 32×,
  where the p99 rose to 39.4 ms against a limit of 32.0. Before `SIM-02-H` the same refresh was
  most of the 01:00 spike. The derivation reads only the sorted list of compatible asset
  footprint sizes from the catalog, so saves now record that list and a load with an equal
  live list skips the refresh. Alternatives: serve refresh regions at a lower rate (each still
  costs about 10 ms), or split a region's derivation across slots (`SIM-02-K` may still need it
  for edits that dirty many regions).
- **`SIM-02-I` — member lists in agent order.** The hourly membership census rebuilt every
  household's member links and counts from all agents. The agent store now keeps a sorted
  singly linked list per household (`agents/household_members.rs`), updated by every join,
  removal, swap-remove and household remap in O(household size), and each household recounts
  its members, car access, stock days, employed adults and expected wages at its hourly phase.
  The lists are derived: loading rebuilds them in O(agents), and the canonical ascending order
  makes a rebuilt list sum wages in the same order as a maintained one. Alternatives: a counter
  per field updated by every mutator (each needs its own reload argument), or a rolling recount
  over agent chunks (needs the member links the lists provide anyway).
- **`SIM-02-I` — employed adults follow job changes.** With the count only at the household's
  phase, demand planned on employment up to an hour old, and the scarce-labour industrial test
  fell from 0.95 to 0.90 staffing. Every hire and release through the employment mutators
  (`reassign_worker`) now adjusts the household's count at once; removal paths that skip them
  are corrected at the household's next phase.
- **`SIM-02-I` — the unhoused check stays a city scan until `SIM-02-J`.** Whether a household is
  housed depends on its home building's state, which changes between phases, so a count of
  flags set at household phases could not be rebuilt at load without a new saved column. The
  hour keeps a parallel O(households) check before rehousing, a strict part of the old census's
  work; `SIM-02-J` replaces rehousing with a queue. Alternative: a saved per-household flag.
- **`SIM-02-I` — the funding census spreads over slots on a rolling input copy** (design;
  [Pure jobs](#pure-jobs)). At 100k the census costs about 96 ms a call and runs 49 times a
  day, half of them for construction execution (a diagnostic run of the `SIM-02-H` build). It
  is a global market solver: claims against shared household, supplier and buyer accounts in
  building order, over up to three closure passes. Alternatives:
  - A pure job on a background pool: its routing reads the lanes, CCH and road graph that the
    sim thread writes every frame and the agent planner shares; a safe copy would move fields
    out of the lane and edge records on the movement hot path and copy about 90 MB of lanes
    per road edit at 100k (a read-only survey of the census's network reads).
  - Slicing on live state: a loaded game could not rebuild a census in progress, and the city
    changes between its phases.
  - Per-building funding against last period's market totals: replaces the solver, the
    largest gameplay change of the options.
  The census keeps its solver; only its control flow becomes resumable steps, and the saved
  input copy keeps it deterministic across a load.
- **`SIM-02-I` — demand planning in one slot per zone type (open question 1).** Residential,
  commercial and industrial plan in separate slots of the hour, in that order, with the
  downgrades after the last, each on the published census books. The order keeps residential
  homes feeding the workforce calendar that commercial and industrial claim from, as in one
  pass. Alternative: per-district planning against the rolling snapshot, which bounds the cost
  by district size but changes which parcels are chosen. The zone-type split is closer to
  today's behaviour and simpler, as the unattended rules require.
- **`SIM-02-I` — residential standby splits per household place.** A home's standby load was
  shared among its residents by members; at each household's phase the household now takes
  `idle / occupancy` of it plus its members' load, so the home's total is unchanged while
  every place is housed. Alternative: a resident count per building maintained at about eight
  sites (admission, removal, relocation, eviction, repair, undo, remap, load).
- **`SIM-02-I` — the activity floor reads a saved copy of the census activity.** A building's
  phase reads `funded_activity`, which the hour's census copies onto it with its funded jobs,
  instead of the live census, so a census rebuilt by a load mid-hour never feeds the phases.
  Alternative: save the census itself.
- **`SIM-02-I` — reserved freight amounts as integer sums.** Kept up to date by every shipment
  change instead of rebuilt each hour, the reserved amounts summed in f32 could differ by one
  last-place step from a load's rebuild when a shipment's amount was off the batch grid, so a
  loaded game could diverge. Each slot now sums each shipment's amount converted once to
  2^-30 kg; whole-batch amounts read back exactly as before.
- **`SIM-02-I` — the demand snapshot counts new entities at the next pass.** Counting entities
  created during a sweep only in the next sweep made the planner plan a site placed mid-hour
  again, and the year-long grocery and industrial scenarios failed. The pass now folds entities
  in handle slots created since the sweep began. Construction execution keeps its own current
  snapshot (see [Snapshots](#snapshots)). Both choices were the only variants, of five tried,
  that pass both scenarios.
- **`SIM-02-I` — freight supplier eligibility one hour old.** The rolling supplier index lists a
  supplier as it stood at its slot of the previous hour, so a supplier that fills a truck load
  during an hour is found an hour later (a listed supplier that has since sold out is skipped by
  the live stock check and the search continues outward). With the service-store payment lag,
  this lowers the 30k baseline's net income per resident 1.4% (12.38 → 12.21, just under the
  band's low edge of 12.24) and raises its bankruptcies from 0.24 to 0.27 per 1,000 buildings a
  day; envelope runs of each piece alone attribute −0.08 of the income and most of the
  bankruptcies to the supplier index, −0.10 of the income to the store payment lag, and under
  0.02 each to the other pieces. Every other metric stays in band. Alternative: an index updated
  at every change of a building's saleable stock or outbound reservations, which reproduces the
  hourly build exactly but needs a hook at every stock change.
- **`SIM-02-I` — saves keep the vacancy lists' order.** A growing town saved mid-hour drew
  different households at its first admission after loading, on `SIM-02-H` as well: the
  vacancy lists are kept by push and swap-remove, so their order follows the claim and release
  history, while loading rebuilt them in building order, and admission, rehousing and the
  admission order all pick by position. Saves now record each list's order and whether the
  index was stale; a load restores the order over the rebuild, or marks the index stale, and
  every reader of a stale index rebuilds it first, so the rebuild never depends on which reader
  comes first. A saved order that does not match the rebuilt set keeps the rebuild (an asset
  pack changed between sessions), with a `save` debug line. Alternative: keep the lists sorted
  by building index, which also needs the admission order rebuilt on every claim. The
  determinism gate's 10k city had not exposed it; a starter-town test saved mid-hour on day 3
  now continues identically, checked every frame for a game day.
- **`SIM-02-I` — census inputs copied over the hour's first half, the census over its
  second.** The copy fills over slots 0–479 and freezes the treasury, fiscal policy,
  electricity funding and clock at slot 479. The census then runs over slots 480–959 and
  publishes at the next slot 0, so its copy is 31–60 minutes old when it publishes, where the
  census used to read the live city at :00.
  The slicing budget counts rows (ceil(rows left / slots left)), never wall-clock time, so the
  steps are the same on any machine. Alternative: copy and census interleaved over the whole
  hour, 30 minutes fresher, but the census would read a copy still being filled. At 100k the
  window totals 98.8 ms over 480 slots, median 0.03 ms a slot (a diagnostic run, unpinned).
- **`SIM-02-I` — frontage prices publish when a census starts.** The hourly frontage
  publication moved from slot 0 to slot 480, so a census never sees the prices change under
  it. The slot-0 fallback census and the whole-hour callers also publish first; restarts do
  not. Alternative: restart the census at every frontage publication, which would restart
  every census. Publishing at slot 0 again, with the books carrying the previous prices, left
  the starter town's two-year run unchanged (182 empty household-days in year 1, 6 in year 2;
  see the snapshot entries below), so the price timing did not cause that regression.
- **`SIM-02-I` — what restarts a census.** A change of the network revisions (CCH build and
  metric, lanes, frontage) or of the building or household structure revisions, which count
  swap-removes, remaps, clears and undo restores (`StructureRevisions`; the building reference
  revision keeps its old meaning). Appended entities and in-place changes do not restart it; the
  copy defers them until a restart. A change after the census finished early still restarts it,
  so a load can always recompute the census from the saved copy.
- **`SIM-02-I` — household exits stay at the start of the operational hour.** The design moved
  them after the hour's census readers. That changed a 270-day outcome: in
  `many_new_industrial_lots_respect_scarce_labor_and_earn_their_owner_capital` the entrant
  food processor ended at −1,938 cash where it must gain. The base build passes, exits at the
  hour's start pass, and exits just before the demand pass fail with the identical number: the
  households that chose to leave took part in one more hour of rehousing, restock and job
  search. Alternative: keep the move and retune the scenario, rejected as further from today's
  behaviour.
- **`SIM-02-I` — a departed household stays in the published customer book until the next
  publish.** Exits now swap-remove households after the census publishes. Mirroring an exit into
  the book is O(all receipts), since committed receipts chain per seller with no household
  index; invalidating the books on every exit made the planner run its own census in exit
  hours. The census key keeps the building and household structure revisions apart, and the
  planner's reuse check reads only the building part. Readers that join book rows to live
  households (`household_walking_access`) already check home and entrance. On earlier builds
  the census ran after exits, so this ghost customer, for up to an hour, is new. Alternative:
  also turn off the departed household's access buckets (O(K·4)); not needed, since the
  year-long scenarios and the library suite pass and the measured exit hours lent the published
  census.
- **`SIM-02-I` — the demand snapshot reads the published per-building copies.** A defect found
  after part 1 was committed: the rolling sweep read the live census every slot, and a load or
  a fiscal-policy change replaces that census mid-hour, so a mid-hour reload could fold other
  capacities into the next hour's planning. The sweep now reads `funded_job_capacity` and
  `funded_activity`, which equal the census in an uninterrupted game.
  `a_census_rebuilt_mid_hour_leaves_the_sweep_unchanged` fails on part 1. Construction
  execution still reads the live census's service ceilings until part 3.
- **`SIM-02-I` — a load recomputes the census whole instead of saving its steps.** The census
  is a pure function of the saved copy and the network, so the step and cursor are not saved.
  Alternative: save them and continue, which saves load time but adds a saved form for every
  step's working state.
- **`SIM-02-I` — the planner tolerates appended buildings without inputs.** The input book
  numbers projected sources from the live building count, so a home or other building without
  recipe inputs started after the copy no longer forces the planner's own census. A firm with
  inputs still does: the business book lays out buyer input ports only for census buildings.
  Part 3 removes that fallback.
- **`SIM-02-I` — new entities join the demand snapshot before the hour's job search.** A
  defect in part 1, which was committed with it: the pass folded them after the operational hour's job search, which hires the households that
  arrived during the swept hour: the swept workplace still showed the job open while the new
  household no longer counted its adult as unemployed, so admission counted each such job twice
  and let in that many adults too many. The starter town ended its third day with one adult
  more than its funded jobs, absorbed by the next census's 13th grocery job. With the census
  reading its half-hour-old copy it ended with two more (115 adults, 113 funded jobs), a town
  whose census funds the grocery for 12 workers even on the live city: an unemployed adult adds
  nothing to the recurring budgets, so their spending cannot fund their jobs. Two adults stayed
  unemployed until their support ran out (182 empty household-days in year 1), and the 160
  residents left matched the 12-worker grocery's 160 units a day exactly (6 empty days in year
  2). The operational hour now folds the new entities first, before exits and job search, as
  the sweep folded every other entity; the pass folds any created since. Alone, this gave 37,
  0 and 0 (19, 0 and 23 with the census at the hour's first slot and the old fold), but see
  the next entry.
- **`SIM-02-I` — planning counts its workforce on the live city.** With only the fold fixed,
  the scarce-labour case (`many_new_industrial_lots_respect_scarce_labor_and_earn_their_owner_capital`)
  funded a factory whose fourth worker never arrived (75% staffing). Apart from the new
  entities, the workforce the calendar and admission read came from the rolling sweep, so a
  hire, quit or release after an entity's chunk was up to an hour stale, and the double count
  had happened to give the calendar a margin. On `SIM-02-H` the pass built its snapshot whole,
  so it was exact. The pass now counts housed unemployed adults, displaced households' labour
  and the workplaces' open funded jobs, their net wage sum and their physical, funded and
  unfunded capacity on the live city, after the hour's job search, with funded jobs from the
  published copies; the other folds stay rolling, and the sweep still folds and saves the
  workforce fields, which only whole-city builds and tests read now. It is a parallel scan,
  O(H/p + B/p) a pass. Maintained totals would need hooks at every hire and release, each
  household's hourly recount, each building's hourly phase (its activity floor sets store and
  work-area capacity), construction, closures, desertion, homes becoming uninhabitable,
  removals, placement, publication and income tax changes, and a running f32 wage sum would
  not reproduce a recount exactly; `SIM-02-K` replaces the scan with them. The starter town
  grows to 80 households and 158 residents, all adults employed: 35 empty household-days in
  year 1, none in year 2 and no failed shopping trips, and the scarce-labour entrants are
  staffed.
- **`SIM-02-I` — policy changes reach the census through its copy.** A fiscal or service
  funding change ran a census of the live city at once, while the census in the window kept the
  policy its copy froze at the half hour and published it at the next hour, and a building's
  funding override changed its row without the copy taking it. A change now writes the new
  values into the copy's frozen policy, records the overridden row and restarts the census in
  progress, so the next hour's census carries the new ceilings; until then the published census
  stands. Service workers the new funding no longer pays leave at once, as before. Loading
  still runs a census of the loaded city for the derived jobs shown before the next publication.
- **`SIM-02-I` — planning lends the published census after any start in the window.** A firm with
  recipe inputs started after the census's copy used to force a census of planning's own: the
  books laid out business input ports per census building. The lent business book now lays out
  the appended buyers' ports after the census's live ports, as a census of the whole city numbers
  them, with no accounts, rebuilds its live port index with them (keeping the census's
  availability and finance classes) and lays out the proposed sites again after them; the
  census's own index waits aside and returns with the pass's rollback. Planning has no census
  of its own any more: under the slot clock the census published at its slot is current (a test
  build asserts it), and `planner_censuses` counts any pass that would not be. Alternative: keep
  the fallback census for firms started in the window, O(census) at the hour's first slot.
- **`SIM-02-I` — completions after the copy keep their constructing claims.** The census records
  which copied rows it saw complete. Construction supply, the workforce calendar and the live
  workforce view treat a building it did not see complete as still under construction, so a
  workplace completing between the copy and the hour's first slot keeps its constructing claim
  until a census funds its jobs, and its workers are never claimed twice. The constructing
  claim is still a proof: in the starter town a food processor completing in an hour's second
  half had spent 6,838 of its 13,100 opening equity on its first inputs by the next hour's
  first slot, failed the constructing proof on its live budget (its stock in transit is not counted) and held no claim for that hour; the next
  census funded its 10 jobs. Left for `SIM-02-K`.
- **`SIM-02-I` — construction permits at the planning slot (option P).** After the hourly pass
  queues its spawns, the hour's first slot opens one construction epoch on the census published
  there: its books lent under the pass journal as planning lends them, the planner's snapshot
  (rolling front, the live workforce the pass counted, the queue's capacity) and a replay of every
  constructing building and queued project in queue order, counting queued homes as the planner
  does. Each queued commercial or industrial spawn its proof funds and staffs gets a permit (a
  digest of its door, with its profile and level), saved with the spawn (save format 98; 99 for
  the stable identities below). The digest is what makes a load continue exactly: a game loaded between the slot
  and the start makes the same decision without proving again, because the start compares the
  current door, profile and level with what the slot proved. Alternatives: save only the workers
  (a start after a door-moving edit would place at a door nothing priced, or would need a proof
  at the start, the census this removes), or prove the permits again on load (the city has
  changed since the slot, so a loaded game would decide differently from one that kept running).
  At its start a business spawn places only with a permit for its current door,
  profile and level; otherwise it is rejected. A missing permit never means "prove now". A
  voided permit waits instead (below). Immediate level changes are proved in the same epoch, and every
  zone's level changes now run before any despawn, because a despawn swap-removes a building
  under the lent books; the epoch closes before the first despawn. Construction execution's own
  census and whole-city snapshot, the quote at the start and the epoch bookkeeping are gone; no
  census runs in a gameplay frame except the publication's fallback. Gameplay differences:
  a business start no longer reprices freight or re-proves after an edit within the hour
  (an edit that moves its door rejects it), queued homes now back queued firms, and level
  changes see the city before the hour's despawns. Measured (release): with the
  permits and the reorder alone, the scarce-labour case's town was unchanged through day 366
  (its day-366 record matched part 2's exactly); for the starter town with the stable seeds, see
  below. `populated_census_window_churn` over 24 hours: no planner censuses at 10k or
  30k. Over 336 hours: 2,485,050 census building rows at 10k (2,566,390 with 11 execution
  censuses before) and 6,841,159 at 30k (8,469,975 with 80); every row is now the hourly
  census's; 6 and 75 construction epochs, one per hour with a business spawn or level change.
  `perf_contract_queued_locality`: a start check costs 0.27 µs at 16,384 and at 131,072
  background buildings (0.99×). Alternatives: epochs on the
  published census across the hour (needs the books' swap-remove mirroring or a third census
  copy, and still re-proves earlier placements in index order after a load), or a delta census
  (a second sliced census, twice the CPU, no new information).
- **`SIM-02-I` — every home's applicants are seeded by a stable identity.** With the permit
  proved at the planning slot on the planner's own view, the scarce-labour case
  (`many_new_industrial_lots_respect_scarce_labor_and_earn_their_owner_capital`) failed: its
  food-processor entrant ran at 90.4% staffing (58,453/64,670 worker-hours) against the 95%
  bound. Both builds planned the same four entrants at day 366, 01:00 (three 4-worker machinery
  factories and a 10-worker food processor). Before, the start-minute check at 01:17, with the
  hour's homes placed and seeded by building index, rejected the food processor; the permit, a
  replay of the planner, counted the queued homes' applicants seeded by parcel id and permitted
  it. The town then reached 98 households against a grocery making about 197 units a day for 192
  residents; one household sat with an empty pantry from day 372 and left on day 380, leaving the
  processor at 9 of 10 workers. A first fix seeded a queued home by the building index it would
  have once placed, but any swap-remove shifts that index: a despawn in the same planning slot, a
  mid-hour bulldoze, a player-placed service or an undo restore. Each building now carries
  `applicant_seed`: the id of the queued spawn that placed it, or the next value of a saved city
  counter (tagged `CITY_APPLICANT_SEEDS`) for a building placed outside the demand queue. A queued
  home is estimated with its spawn id, which it keeps once placed, so planning, the workforce
  calendar, the permits, admission and the arrival's composition all read one stream, across
  remaps, removals and loads (save format 99). A proposed home is still seeded by its parcel id
  in the pass that proposes it. Gameplay: every home's draws change. Measured (release): the
  starter town ends at 80 households and 153 residents, 109 of 119 jobs filled, 24 empty
  household-days in year 1, none in year 2, no failed shopping trips (part 2: 80, 158, 121/131,
  35/0/0); the scarce-labour case's entrants are machinery factories on parcels 101, 103 and 104
  and a food processor on 102 at 99.9% staffing. Alternatives: the predicted index (stale after
  any swap-remove, and it needed its own voiding), fixing the planner's estimates and the grocery
  absorption margin instead (widens the part into demand balance; the grocery margin is its own
  issue), or counting only placed homes at the permit (stricter than before: same-hour firms
  would wait an hour).
- **`SIM-02-I` — a voided permit waits; a denied one rejects.** A home that fails to place after
  its applicants backed the slot's proof voids the queue's granted business permits. A voided
  spawn is not rejected: it moves, keeping queue order, to start after the next planning slot,
  which proves it again. A spawn the proof could not fund or staff is denied and rejected at its
  start, as before. Only a home the slot counted voids (its queued project active, not a later
  duplicate of a parcel or on a parcel no longer available); the slot records that per queued
  home and saves it, since a loaded game rebuilds the queue's projects from the parcels as they
  are at the load, and a home whose lot disappeared mid-hour would otherwise void in the running
  game only. Projects and permits are matched by spawn id. Alternative: reject voided spawns (a
  failed home then cancelled every business spawn still queued for its hour).
- **`SIM-02-I` — door digests use save-stable identities.** A save renumbers edges past a deleted
  one and a load rebuilds lanes compactly, while the session tombstones and appends, so a digest
  of edge and lane indices would reject queued spawns after a reload. The digest covers the door
  and curb positions, the frontage offset, side, access and flags, the frontage edge by its end
  nodes' ground positions, and each door lane by its direction, lane index and kind.
  `queued_business_permits_survive_edge_and_lane_renumbering_across_a_save` splits the trunk and
  bulldozes a stub before saving and continues every frame identically.
- **`SIM-02-I` — no pass or epoch lends books invalid for the live city.** A planning pass or
  construction epoch whose published census is not valid for the live key (building indices,
  network, the hour's freight phase and gateways) is skipped and counted (`planning_skips`,
  `construction_epoch_skips`); the queued business spawns of a skipped epoch wait for the next
  slot and its business level changes are dropped. Measured: no skips over 24 hours at 10k and
  30k, baseline and fiscal stress, or over six days of the starter town. Alternative: lend the
  stale books and count it (rows read by index could belong to other buildings).
- **`SIM-02-I` — census slices plan for the previous census's closure passes.** A census sliced
  assuming its current closure pass was the last left passes 2 and 3 to the window's last slots,
  and slot 959 took a whole pass. Slices now assume as many passes as the previous census took
  (one before the first), so the remaining work spreads over the slots left; a census needing
  more catches up at the end. Slicing never changes a census's result (the stepped-equals-one-run
  tests pass). Measured, `populated_census_step_costs` (release, eight workers, P-cores): at 10k
  a census needing two closure passes has its slowest slot at 1.07 ms (window slot 583; one run
  7.0 ms), with no spike at the window's end; at 30k (one pass) 1.94 ms (slot 597). Diagnostic
  timing, not acceptance.
- **`SIM-02-I` — the hour's plan is saved between the zone slots, and slot 3 queues it.**
  Residential, commercial and industrial plan at slots 0, 1 and 2 of the hour, each on the
  published books under its own journal, and append their spawns, upgrades and despawns to the
  hour's plan, which is saved (`demand_hour_plan`: upgrades kind 0, despawns 1, optional homes 2,
  spawns 3). Slot 3 plans the downgrades and the optional homes the workforce claims, admits,
  then queues the whole hour's spawns in the single pass's order (homes, claimed optional homes,
  shops, industry), proves the queue's permits and the level changes in one construction epoch
  and runs the despawns after it closes. This is the single pass: plan everything, then execute
  once; only the planning is spread over slots 0–2. A later slot first offers the residential
  slot's homes and optional homes to its calendar again (a home whose lot went since is skipped,
  an optional one dropped), then proves the earlier slots' business spawns and upgrades again,
  zone by zone, spawns before upgrades, as the single pass held their claims. Alternatives:
  (1) each zone slot queues its own spawns (the WIP): the 08:00 factories then started before
  the hour's optional homes, which changed every later start, building index and applicant
  seed; in the starter town ten factories completed before the census copy reached their rows,
  held no claim at the next hour (their constructing proof read budgets already spent on
  inputs, the gap left for `SIM-02-K`) and the industrial slot planned 12 more at day 1 21:00,
  so no grocery was ever built (29,928 empty household-days in year 1). (2) Queue per slot and
  re-sequence the hour's entries at slot 3: needs the start events rescheduled, and spawn ids,
  hence applicant seeds, still differ from the single pass. Measured: the starter town plans the
  same commercial and industrial spawns as part 3 on day 1 (traced), ends at 80 households, 153
  residents, 109/119 jobs, 25 empty household-days in year 1, none in year 2, no failed
  shopping trips (part 3: 80, 153, 109/119, 24/0/0); the scarce-labour case's entrants equal
  part 3's (machinery on parcels 101, 103 and 104, a food processor on 102 at 99.87%).
- **`SIM-02-I` — admission's arrivals depart paced from slot 3.** Admission runs at slot 3,
  between the plans and the actions, as the single pass ran it before its actions. Its carriers
  wait at the border for paced departures, which slot 0 schedules for the hour's other trips;
  slot 3 now schedules its own carriers over the following hour. Without it they waited a whole
  hour, and a save in that window lost their departure (a load rebuilds the arrival carriers
  and clears the held list), so the loaded game never admitted them: four continuation tests
  diverged by one household (`growing_town_…`, `stale_vacancy_index_…`,
  `a_supplier_index_left_stale_…`, `a_policy_change_in_the_census_window_…`), and the
  scarce-labour case's late arrivals left a machinery entrant at 75% staffing. Departures are
  three slots later than the single pass's; the last of a large batch may leave in the next
  hour's slots 0–2. Alternative: schedule every held trip at slot 3 (the hour's shoppers,
  visitors and freight would wait three slots, and a save between slots 0 and 3 would need the
  held lists).
- **`SIM-02-I` — a flow field is a function of its sources and the road graph.** A field
  equals `FlowField::build` of its zone's current sources (completed buildings of the zone in
  zone-index order, with their entrance-legal edge endpoints) on the current graph, which is
  what a loaded game builds. Since `SIM-02-G` only a phase that removed buildings passed the
  allocator's zone marks to the fields, so the running game routed on fields up to days old;
  the first part 4 build drained the marks at every frame but rebuilt both fields of every marked zone,
  including zones marked by a placement whose site, under construction, is no source. Now:
  - **Marks:** completion, removal (the removed and the moved building's zones), level
    changes, a building placed complete, and every zone after a zone-index or entrance-cache
    rebuild (both choose sources); a placement under construction marks nothing. Topology
    edits and loads mark every field as before.
  - **Skip:** a marked zone collects its sources into reused buffers and compares them with
    those its fields were built from; unchanged sources on an unchanged graph keep the field.
    The graph is identified by the CCH build generation and metric revision with the node and
    edge counts; `mark_all_dirty` drops the identity, and a changed identity rebuilds every
    field.
  - **Cached adjacency and storage:** the reverse adjacency each search walks is cached per
    mode in compact rows (offsets and a flat arc array, per-node arcs in the old order: graph
    edge order, forward arc at the end node before the backward arc at the start node), so
    fields are bit-identical to the old builds (the benchmark's field fingerprints match).
    Fields and search scratch (distances, source nodes, heap) keep their storage.
  - **Parallel:** each marked zone's car and foot slots collect, compare and build on Rayon
    workers; slots share only the adjacency, so the result never depends on scheduling. A
    frame with nothing marked returns without waking the pool (waking it every frame cost about
    0.08 ms a frame at 100k in a first version).
  - **Bound:** O(1) per frame with nothing marked; O(V + E) for the adjacency after a graph
    change; per marked zone and mode, in parallel, O(S) to collect and compare its S sources
    plus O((V + E) log V) only when they changed, at most six fields. Steady state allocates
    nothing; buffers grow only past earlier sizes. Memory: six fields and scratch, 20 bytes a
    node each, plus two adjacency tables, 4 bytes a node and 8 an arc.
  Measured (`benchmark_flow_field_build`, matched grids, release, pinned to the eight P-cores,
  two alternating rounds each, medians of 21): 10,000 nodes 764 µs before; 578 µs cold after
  (adjacency built in the call), 452 µs on cached adjacency and reused buffers, adjacency alone
  119 µs; 99,856 nodes 8.94 ms before, 7.57 ms cold, 5.62–5.72 ms cached, adjacency 1.57–1.92
  ms. On the `PERF-07` snapshots (`perf07_flow_field_drain`, `RAYON_NUM_THREADS=8`, 1× and 32×
  give the same counts): 30k (642 nodes, 1,201 edges), 4 game hours, before 0/1/1/1 zones
  drained and 0/2/2/2 fields built an hour; after 0/3/1/3 drained, 0/2/2/2 built, 0/4/0/4 left
  as they were. 100k (2,220 nodes, 4,283 edges), 3 hours, before 4/3/3 drained and 8/6/6 built;
  after 4/4/5 drained, 6/6/6 built, 2/2/4 left. Frames that built a field: 100k before mean
  0.68–0.91 ms, worst 1.16 ms; after 0.55–0.82 ms, worst 1.37 ms (32×); 30k before 0.20–0.27
  ms, after 0.08–0.40 ms. Frames with nothing to build: 0.001 ms (after) at both tiers. The
  snapshot cities are mature (one or two placements and removals an hour), so rebuild counts
  are small either way; the bound above, not these counts, is what a growing city meets.
  Alternatives: (1) save the fields' sources (larger saves, a second index of buildings to keep
  in step); (2) drain at a fixed slot (a load between drains still differs); (3) insert a
  completion into a field incrementally with the same tie-break (not needed at these counts);
  (4) spread one build over frames: a `SIM-02-K` item, for a grid like the benchmark's 100k
  where one build costs 5.7 ms; not done here.
- **`SIM-02-I` — the frame's flow-field drain reads a fresh zone index**. The drain
  read the zone index and the entrance cache without checking them, so a frame after a
  removal that left the index stale built fields from wrong sources (the removed building's
  entry names the moved one, the moved one's entry is past the end), while a loaded game,
  whose load rebuilds the index, built correct ones; the next planning slot repaired the index
  without marking the fields. The drain is now a reader under the existing rule: it rebuilds a
  stale index (`vacancy_index_stale`) and a stale entrance cache before collecting sources.
  The index is rebuilt at every frame that starts stale, not only when a field is marked: the
  field marks are not saved (a load marks every field) while the stale flag is, so a game
  loaded from a save between frames rebuilds at the same frame as the game saved, from the
  same buildings, and its vacancy and admission order continue identically. A stale entrance
  cache has no order to keep. Gameplay: a stale index is now rebuilt at the next frame
  instead of at its next reader in a slot, so claims and releases between them see a fresh
  index. Measured: on the 30k and 100k snapshots no frame started with a stale index, entrance
  cache or CCH over the counted hours (the phases' removals repair them in their slot). The
  player's bulldoze in the starter town does not leave the index stale either (its site query
  rebuilds it before returning); `the_flow_field_drain_rebuilds_a_stale_zone_index_first`
  removes a building through the lifecycle removal without the repair and checks every field
  against a fresh build after one frame; without the drain's repair the index is still stale
  after the frame and the test fails. Godot's building-LOD reader also rebuilds a stale index,
  between frames, where nothing else changes it, so the result is the same as at the drain.
- **`SIM-02-I` — a zone slot on invalid books waits for the next hour**.
  A skipped commercial or industrial slot plans nothing for its zone; the residential slot's
  plan stands and is queued at slot 3. A skipped slot 3 queues the hour's spawns with voided
  business permits (they wait an hour) and drops the downgrades and business level changes;
  residential level changes and despawns run. Measured: no skips in any slot over 24 hours at
  10k and 30k, baseline and fiscal stress (`residential/commercial/industrial/actions_planning
  _skips` and `construction_epoch_skips` all 0; 96 planning passes), or over 144 hours of the
  starter town (5 epochs).
- **`SIM-02-I` — no business start before slot 3's permits.** Slot 3 queues every spawn, due
  from minute 1 (slot 16).

- **`SIM-02-I` — a save in the hour's planning slots keeps the census they read**.
  The census the hour's first slot publishes is read by planning slots 1–3, slot 3's permit
  epoch, the workforce reset (`saw_operating`) and the project reservations (service ceilings,
  paid cargo). A load ran a census of the loaded city instead, so a game saved after slot 0, 1
  or 2 planned commerce and industry, proved permits and re-proved the plan on other books. A
  save before the hour's actions slot (`absolute_slot % 960 < 3`) now stores, in
  `census_inputs.published` (save format 101): whether the published books were valid for the
  live city, and for valid books the input copy that census read and the service ceilings it
  adopted for buildings appended after the copy (`service_capacities[copy rows..]`). The load
  recomputes the census from that copy as `complete_loaded` does (entrances derived from the
  copied rows, a whole census) and finishes it as `publish_into` left it: the adopted ceilings
  appended, `published` set to the live key, no `publish_funded_job_capacities` (the buildings'
  funded jobs and activity are saved). Books that were not valid at the save stay not valid
  (`published = None`), so the loaded game's slots skip as the saved game's do. The copy is
  provably unchanged from `consume()` until the fill's last slot swaps a new one in: while it
  is not pending, `remove_building`, `remove_household`, the retargeting pass,
  `copy_changed_rows` (front rows only when pending and not read) and `follow_policy` (only
  when pending) leave it alone; only `prepare` (a census start, at slot 480 or a whole-hour
  caller) and the swap change it, and both clear the new `published_front` flag. Valid books
  also imply the live job funding is unchanged since the publication: `register_pending_service`,
  `invalidate_building`, a non-empty remap and `clear` all set `published = None`, and the
  planning slots and epoch return the lent books exactly. Costs: the save adds the copy's rows
  only in slots 0–2 (in those slots no pending front is saved, so a save holds at most one
  copy plus the partial fill, as in the hour's second half); the load runs one census, as
  before. Alternatives: (1) save the front at every slot until the swap (exact derived jobs in
  the UI all hour, but every save in slots 3–478 grows by a city copy for no reader); (2) save
  the published census's books themselves (the customer, business and input books, with their
  caches, have no saved form, and would be a second encoding of the census's state); (3) leave
  the planning slots to run on a census of the loaded city (the defect). Evidence (release): `the_published_census_continues_identically_after_save_load_in_the_planning_slots`
  (busy town saved after slots 0, 1 and 2: same books, funded jobs, activity, headroom, service
  ceilings and freight capital, the loaded game saves the same copy again, and three hours of
  censuses match) and `a_save_at_slot_1_after_a_fiscal_change_plans_on_the_published_census`
  (busy town, commercial and industrial property tax 500 a day after slot 0, saved at slot 1;
  the published census at load, then the hour plan, queue permits and flow fields every frame
  for a day) pass, and both fail on a build without the restore (headroom, and the fiscal
  case's funded jobs: the loaded city's census funds the new policy's books). The starter-town
  between-slots test passes even without the restore, as on the first part 4 build: its decisions coincide
  on the two censuses, which is why the census comparison at load is the check.
- **`SIM-02-I` — a failed re-proof leaves the hour's plan**. A later slot proves
  the earlier slots' business spawns and upgrades again, in the single pass's order, before its
  own decisions. One that fails there (budgets, stock or workforce moved in the slots between,
  or its lot went) would get no permit on these books anyway, but it stayed in the plan with its
  claims released, so a later zone's projects could claim the same workers, customers and
  inputs and the epoch then granted in queue order. It is now dropped from the plan and counted
  (test-only `Work::ReproofDrops`); a dropped upgrade gets its pacing credit back, as a rejected
  one does ("economic rejection must not spend pacing credit"). An upgrade whose building no
  longer stands as planned holds no claims and stays: slot 3 skips it as before (dropping it
  too broke two unit tests whose fixtures have buildings without parcels). Alternatives: (1)
  keep the action and hold its old claims (no proof backs them); (2) re-plan the dropped
  project's zone (a second decision pass per slot). Counts: see Measurements (review).
- **`SIM-02-I` — hour-plan keys whose edge the save drops are not saved**. An
  upgrade or despawn key whose edge has no entry in the save's edge renumbering saved as -1
  and loaded as `usize::MAX`, which broke the edge-first order slot 3 binary-searches upgrades
  in. Such a key names a building that resolves in neither game, so it is skipped at save.
  No test: it needs a road edit that deletes a planned building's edge between the slots,
  which the fixtures do not offer cheaply. Alternative: re-sort after load (keeps a row that
  can never resolve).
- **`SIM-02-I` — the planning slots cost `CityScan`**. The three new fixed-slot
  systems were declared `LegacyBoundary`, which means a pass re-hosted unchanged. They are new
  splits of a whole-city pass, so a new cost, `CityScan` (O(B + H + P) a call in buildings,
  households and lots; `SIM-02-K` replaces the scans with maintained totals and indices), is
  accepted for fixed slots, and the declarations state the per-call bound: the live workforce,
  O((H + B)/p); the project market, O(B) serial; the queue's replay, O(Q log H); the earlier
  slots' plans, O(plan); a zone slot's lot scan, O(P/p) plus its zone's free lots' assets, and
  its zone's existing buildings, O(B/p); slot 3's downgrade candidates, O(B/p) per zone, and
  its epoch, O(B + Q log H). Reads and writes stay `ALL`. The existing-building scan now
  collects only what each slot uses: a zone slot its zone's upgrades and despawns, slot 3 every
  zone's downgrades (the old scan proved both directions every time and dropped half). Per-zone
  lot lists are not done: the parcel scan's work per lot outside the slot's zone is an
  availability check and a profile lookup, in parallel, and a list per zone would have to
  follow every parcel creation, removal, rezoning and occupancy change. `SIM-02-K` item, with
  the measured lot-scan time (Measurements (review)).
- **`SIM-02-J` — freight cooldowns as slot deadlines, ending at the building's next phase.** The
  cooldown was a logistics-hour tick (`hour_ticks + 1`), which under the hourly pass always
  expired by the next pass. With phases it blocked a building's later phase in the same hour
  after a mid-hour failure, and a dispatch failure early in the next hour could cost the
  requester that hour's phase too. `Building::shipment_cooldown_until_slot` is now an absolute
  slot compared at the phase. A failure or export planned for a request waits for the phase
  that queued it to come round (`phase slot + hours × 960`), so a failed inbound plan still
  blocks the same phase's export, as in the hourly pass. A failure at any other time (a carrier
  that cannot leave, a shipment failing on the road, an endpoint removed) waits for the
  building's first hourly phase after it, from the handle's phase offset in O(1), plus whole
  hours for a longer cooldown. Alternatives: keep hour ticks and correct by the phase offset
  (inexact for the trip events that fire at an hour's first slot, before the hour starts the
  logistics clock, when the cooldown is over an hour), or a plain `failure slot + 960` (retries
  one or two phases later depending on where the failure falls).
- **`SIM-02-J` — at most one queued freight request per building and kind.** A pending bit per
  kind and handle slot (with its generation) is set at enqueue and cleared when the request is
  served or dropped, so the queue holds at most two requests per building even if it ever falls
  behind. The bits are derived from the queue on load, not saved. The saved request slot is now
  read: a plan checks and sets the cooldown against the phase that queued it.
- **`SIM-02-J` — one live trip event per shipment.** `Shipment::trip_event_slot` (saved) records
  the slot of its departure or next check; a freight event due at another slot is a leftover and
  does nothing. Undo re-schedules restored shipments at the recorded departure or the next slot
  without starting a second check chain; a clock set directly moves the recorded slot with the
  overdue event.
- **`SIM-02-J` — bulldoze undo checks the restored shipments by id.** It compared the global
  counts of shipments and request failures, which the per-slot freight queue changes in almost
  every slot, so undo would have been refused after any freight activity. Now every restored
  shipment id must still be absent; request failures restore by key.
- **`SIM-02-J` — freight departures levelled per minute.** Each departure keeps its golden-ratio
  target in the hour ahead (`spread_slot`) and takes the least busy minute within 15 minutes of
  it, never before the next minute (`MinuteLeveler`, a saved 128-minute count ring, O(31) per
  departure). The targets alone put 5.0% of an hour's departures in its busiest minute at 10k
  baseline and 30k arrivals (2 of 40 or 3 of 60); per-hour counters keyed by a departure counter
  either thin the sequence again or wait about 52 minutes, and half-hour blocks still exceed 4%
  in 12–17% of hours at 40–50 departures. Simulated over 6,000 hours of uniform releases with
  flat and daily-cycle rates of 40–200 an hour, levelling kept every hour at or below 4.0%, with
  waits of 30–35 minutes; the gate's freight busiest minute fell from 5.0% to 3.3% at 10k
  baseline and 30k arrivals. Alternatives: a minimum-gap pacer from the last hour's count (backs
  up for hours after a sudden wave), or pacing each half-hour's departures at its end
  (O(departures) at a boundary).
- **`SIM-02-J` — building removals keep the network freight quotes.** A building removal or remap
  drops only the building-indexed freight quotes, not the network border sweeps, which the
  per-slot queue would otherwise rebuild in the slot after every removal (6–11 ms at 100k).
- **`SIM-02-J` — no export hold step.** The hourly pass's export hold held nothing in 24, 24 and
  12 measured hours at 10k, 30k and 100k with the one-hour retry cooldown, so it is gone.
  Exports still ship only stock above the one-day buffer.
- **`SIM-02-J` — fiscal-stress freight deliveries below the band.** The 10k fiscal-stress city's
  startup logs show earlier delivery, not lost freight: cumulative deliveries by day 2 are 1,781
  against 1,309 on the build before `SIM-02-J`; day 3 has 772 against 1,166; by day 10 the
  total is 3,125 against 3,004 (+4%); and there are no cooldown rejections. The envelope
  measures from day 2, so the front-loaded deliveries fall before its window. This explains the
  gate's fiscal deliveries (8.70 and 10.44 per 1,000 buildings a day at 10k and 30k, against
  13.22 and 12.68 on part 4) and the fiscal 10k net income per resident (27.25, band low
  28.04).
- **`SIM-02-J` — housing requests from a per-slot queue.** A household's hourly phase queues its
  rehousing while it has an independent member and no usable home; its daily phase queues its
  housing review as before. The `housing requests` queue serves both oldest first, merged by
  (queue slot, household), a household's rehousing before its review of the same slot; a
  household waits at most once per queue (a generation mark per handle slot, derived on load).
  Member lists are maintained since `SIM-02-I`, so a review no longer waits for the hour's
  membership census. This replaces the hour's parallel O(households) unhoused check ("the
  unhoused check stays a city scan until `SIM-02-J`" above). Alternative: rehousings first from
  the shared quota (the first build), which let a rehousing queued later overtake older reviews.
- **`SIM-02-J` — rehousing no longer runs before the hour's admission.** A household that loses
  its home is rehoused at its next hourly phase, which may fall after the next hour's admission
  claimed the vacancy it would have taken. Alternatives: drain the queue before the demand pass
  (an O(unhoused) boundary pass again), or a saved per-household unhoused flag served at the
  hour. Evidence (the part 2 gate against part 1, which passed with reload, repeat, 1 and 3
  workers identical): the unhoused rate is at or below the base everywhere but baseline 10k
  (9.4e-5 against 6.7e-5, about one household in a city of 10,600 residents and 5,300
  households), all in band; the vacancy rate is within 1.6% at 10k baseline, and 1.6e-4 against
  1.7e-4 at 30k baseline.
- **`SIM-02-J` — housing reviews read the live vacancy lists.** The review batch built a vacancy
  planner over every residential vacancy, O(V log V) per build; per slot that is 960 builds an
  hour. The admission order now also keeps its rehousing homes per level and 512 m chunk
  (`by_level_chunk`, updated with each vacancy change in O(log V)), and a review's nearest
  affordable home is a ring search over that level's chunks, with the planner's eligibility
  (operational, not under construction or redevelopment, a place left) checked per home. Over
  the same vacancies it picks the planner's home. A level's chunk bounds only grow until it
  empties, so a search may scan rings that have since emptied. Alternative: build the planner
  once per hour and keep it (its own claims would need saving to reload identically).
- **`SIM-02-J` — restock requests at the household's phase, only with a shopper free.** After
  its hour a household queues its restock when the hourly pass's test (`restock_due`: stock
  under the trigger, no trip or cooldown, and waiting for a shopper, out of stock while a store
  sells, or its check offset) holds for the hour of its phase and a member is free to shop. The
  phase walks the members; a household with none free is left waiting without a request and
  checks again at its next phase. The shopper is the household's first eligible member in agent
  order (its member list), as the O(agents) shopper scan found. The `restock requests` queue
  plans a slot's requests in parallel and reserves them in queue order against live store
  stock; the hourly pass planned all households in household order. Out of stock with no front
  for the hour yet counts as "a store sells". The first build also queued households waiting for
  their shopper, about four in five requests; queueing only with a shopper free cut restock
  requests from 1,080 / 3,340 / about 17,000 an hour to 407 / 913 / 5,584 at 10k / 30k / 100k
  (100k busiest hour 9,532, busiest slot 24, none carried). Alternatives: re-queue on a member's
  arrival home (an event per arrival at home), or one queue for restock and visits (one quota
  would let visits delay restocks).
- **`SIM-02-J` — a held shopper's reservation ends with the household's move.** Cancelling a
  purchase before pickup releases the shopper's departure hold; a household that moves while its
  shopper is still inside the old home cancels and refunds the purchase first. Alternative: keep
  the departure inside the household's next phase (no lifecycle change, but a household could
  still move between reservation and departure).
- **`SIM-02-J` — visible visits keep the hourly cap as a rate.** The hourly pass planned the 64
  drawn households with the smallest draw hash. Now the phase's draw queues the visit while
  fewer than 64 wait, and the `visit requests` queue serves one per whole credit, the credit
  accruing 64 visits an hour (one per 15 slots) and holding at most one visit while nothing
  waits. Visible visits stay at most 64 an hour, so agent traffic is unchanged; selection is
  first drawn, first served. Visitors leave at the next slot, already paced by the credit.
  Alternative: a per-slot quota of one (up to 960 visits an hour, about 15× the visible visitors
  at 100k). Evidence (the part 2 gate against part 1): 15 / 35 / 24 visits an hour at 10k / 30k
  / 100k, never over 64; no burst limit applies to visits, and the shopping burst is 2.4–3.6%.
- **`SIM-02-J` — household destinations as a rolling snapshot.** The stores selling household
  supplies and the service stores hosting visits are folded over each hour's slots by building
  handle slot into foot and car component and 512 m chunk buckets, published at the hour's last
  slot for the next hour (`household destinations`, after the demand snapshot sweep). Readers
  resolve each handle and check the store again (sold out, closed), so only eligibility lags; a
  store restocked since its chunk waits for the next sweep. A reader without a front for its
  hour and the current CCH build builds one at once, O(B). Walking proofs are prepared per
  inspected candidate instead of per store per hour; they are exact either way. Foot and car
  components are cached by CCH build generation (`HouseholdTopology`). Saved: both buffers and
  the sweep cursor (`household_queue_state`, `household_destinations`).
- **`SIM-02-J` — dead store entries have their own cap.** A store that left or sold out since its
  chunk counts against a cap the size of the live inspection cap, so a scan inspects at most 384
  live stores (visits 128) and passes over at most as many others.
- **`SIM-02-J` — shopper departures levelled like freight.** A reserved shopper leaves near its
  household handle's slot of the hour ahead (`spread_slot`), in the least busy minute within 15
  minutes (a second saved `MinuteLeveler`); the hourly pass spaced each hour's shoppers evenly
  with `paced_slot`. Arrivals keep `paced_slot`, from the hour's actions slot where admission
  runs (`SIM-02-I` part 4). With shoppers, visitors and freight on queues, the slot-0 pacing call
  (`schedule_trip_departures`) had nothing left to pace and is deleted.
- **`SIM-02-J` — census rows read mid-hour.** The census is published at slot 0 and read by
  planning slots 0–3; a load in slots 0–2 restores it, a later load runs a census of the loaded
  city. The queues' readers do not depend on which. Restock and visits read only the published
  census's walking proofs (checked against the network revisions) and household access rows
  (checked against the household's home and door), both exact shortcuts of the route test they
  skip, never a building row of the books. Job search and the job board read funded jobs from
  each workplace's own row (`Building::funded_job_capacity`, copied at publication and saved,
  which moves with the building) instead of `JobFunding::capacities` by dense index; at the
  hour's first slot the two are equal. The household destination index reads only live building
  state.
- **`SIM-02-J` — job searches from a per-slot queue.** An agent's hourly phase first ends an
  assignment to a workplace that stopped operating or of an agent who cannot work (the hour's
  O(agents) ejection scan), then queues its search when it is in a building and unemployed,
  past its job lock or unpaid for two days. The `job requests` queue runs a slot's searches
  together: homes shared within the slot are searched once, options are ranked as before, and
  plans apply resident workers with a job at home first, then in agent order. A search reads
  each board member's live funded jobs and workers, so a job filled since is no option; the
  hourly pass ranked the hour-start board and skipped filled jobs only when applying. The hour's
  O(agents) key collection, plan and relock scans and its two O(B) job-supply scans are gone.
- **`SIM-02-J` — the job board as a rolling snapshot** (`job board`, `Rolling { Hour }`): building
  handles with an open funded job, folded by handle slot over the previous hour and published at
  its last slot; front, back and cursor are saved (`job_board_members.buffer`,
  `household_queue_state.job_*`). Membership lags the census by one hour: the board folded
  during hour h−1 reads h−1's published funded jobs, so a job the hour's census opens at a
  building not on the board waits for the next board. The network side is rebuilt per board
  hour and CCH, metric and lane revision, and when a member's own door changes
  (`BuildingEntranceKey` per entry) rather than on any building's door; top walking and car
  speeds are cached by CCH build and metric revision (`HouseholdTopology`). Alternative: the
  first build's board, built at the hour's first search from the buildings with an open funded
  job then (current openings, but a scan over every building in one slot of each hour).
- **`SIM-02-J` — the job lock stays a day deadline.** Locks expiring the same day are searched at
  the agents' phases in the hour after the calendar close, which the job queue absorbs.
  Alternative: a slot deadline (spreads expiries by hire time; a saved column changes type).
  Evidence (the part 2 gate against part 1): employment rate +0.2–0.3% at baseline (10k 0.682,
  30k 0.670), final employment at the base; job requests 1,400 an hour at 30k with no slot
  carried except in the 100k startup hours.
- **`SIM-02-J` — a searcher who keeps its job is locked again.** An employed searcher whose better
  jobs all filled before it applied is locked again, as is one that finds the board empty or no
  reachable member on it. Before, a filled better job left it unlocked to retry next hour.
  Alternative: keep it unlocked and searching at every phase (each phase one search against a
  board that will not change until the next census). Evidence: as for the job lock.
- **`SIM-02-J` — the snapshot census case queues its arrivals' searches at the hour's last
  slot.** `an_arrival_hired_at_the_next_hour_joins_the_snapshot_as_before_its_hire` queues the
  arrivals' job searches at the hour's last slot, since searches no longer wait for the hour, so
  their hire still happens at the next hour's first slot, after the fold the test checks.
  Alternative: drop the case (the fold order it guards still holds by construction: queues run
  after the operational hour).
- **`SIM-02-J` — queue marks follow the queue.** `RequestQueue::pop` clears a handle slot's mark
  only for the popped request's generation; a request of an entity that left, taken after the
  entity that reused the slot queued, no longer clears the newer entity's mark. Continuation
  tests check every frame that the marks equal the ones a load rebuilds.
- **`SIM-02-J` — rolling buffers saved with whether they are current.** A save taken after a
  road edit and before the next slot holds a destination front or sweep folded with the old
  components; the saved game would rebuild or abandon it at its next use, but a load labelled
  every buffer with the loaded components, so it would have kept it. `household_queue_state` now
  records for the front and the sweep whether they were folded with the saved game's CCH build
  and no road change waits for the next rebuild, and a load labels only those, as the freight
  supplier index does since `SIM-02-I` part 3.
- **`SIM-02-J` — the commute quote cache still clears on any building or door change.**
  `workplace_route_cache` is cleared whenever the building or door revision moves, which with
  per-slot searches can happen mid-hour. Narrowing the clear to the changed buildings means
  scanning the cache for keys naming them (dense indices; a swap-remove renames two buildings),
  so it is left for `SIM-02-K`.
- **`SIM-02-J` — quotas from measured request rates.** Freight, 8 a slot: the performance-contract
  cities queue about 75, 314 and 1,041 requests an hour at 10k, 30k and 100k (busiest hours 130,
  416 and 1,209), with busiest slots of 3, 6 and 8 and none carried to the next slot. The
  household queues, from the envelope cities on the final build (debug channels on, 3 sampled
  days at 10k and 30k, 1 at 100k): housing 217 / 628 / 2,119 requests an hour (busiest slots
  4 / 6 / 10, none carried), quota 64 (16 on the first build); restock 399 / 968 / 5,593
  (busiest hours 972 / 2,984 / 9,550, none carried), quota 64, where the first build's 16 a slot
  (15,360 an hour) fell behind at 100k with up to 18,800 waiting while households waiting for
  their shopper still queued; visits capped at 64 an hour by the credit (up to 63 waiting); jobs
  164 / 1,348 / 1,708 at about 0.1 ms a search at 100k, quota 64, which only the 100k city's
  startup hours fill (every arrival searching, up to 9,799 waiting, each for up to a few hours).
  A million residents would queue about 22 housing requests a slot and, at the 30k rate, about
  34 restocks.

- **`SIM-02-J` — removals and departures only in slots 4–479.** A building or household
  swap-remove in the census window restarts the census (the structure revisions are part of its
  key), and one in slots 1–3 leaves the published books invalid for the live city, so the
  planning slot reading them is skipped. In slots 4–479 nothing reads the published books any
  more and no census runs; the input copy is filling and mirrors each swap-remove
  (`CensusInputs::remove_building` and `remove_household`: the back fill follows through
  `Rows::mirror_swap_remove` while `back.next_chunk` is set, the pending front from slot 479 on
  by its own swap-remove; slot 479's queues run before its fill chunk). A request in slots
  480–959 or 0–3 waits for slot 4, at most 484 slots (about half an hour). Alternatives: (1)
  make the census copy and the lent books follow swap-removes without a restart (the books have
  no O(1) removal: committed receipts chain per seller with no household index, `SIM-02-I`);
  (2) remove at slot 0 before planning, as exits did (a city-sized batch in one slot again).
  Evidence: 168 hours of the 10k fiscal-stress city removed 420 buildings, all outside the
  window, with 0 census restarts and 0 planning or epoch skips in 672 planning passes.
- **`SIM-02-J` — level changes applied by a queue under a saved permit, in slots 4–959.** Slot 3
  proves them in its epoch as before and checks that the target fits; the queue applies them
  from slot 4. The permit digests the building's asset, level and economy profile and, for a
  business, its door; the door is derived afresh at both ends rather than read from the entrance
  cache, so a loaded game, whose cache is rebuilt at load, compares the same digest. Execution
  rejects a stale permit and never proves again, as spawn starts do. The queue serves slots
  4–959, not only the removal window as the design said: a level change is in place, so in the
  census window it is a deferred row, not a structural change, and restarts no census; slots
  0–3 are excluded. Alternatives: (1) re-prove at execution (a census-backed proof outside slot
  3, which part 3 removed); (2) save the proof's claims (the books have no saved form). A
  residential change needs no proof, so its permit leaves out the door. Gameplay: a change
  proved at slot 3 now lands at slot 4 (normally all in that slot, at quota 8), and later
  proofs in the same epoch see the earlier changes' claims but not the changed building's new
  asset, which only matters when two changes in one hour depend on each other's live profile.
  Evidence: `a_stale_level_change_permit_is_rejected` and the census-window test; the envelope
  cities ship level-1 assets only and never exercise level changes.
- **`SIM-02-J` — compatibility removals queued at the building's phase; `cleanup_stale_buildings`
  stays immediate.** The phase advances the rezoning grace as before and queues the removal when
  the attachment is gone or the grace ran out; the queue checks again with no elapsed day. The
  building's day ends with that decision (no settlement), as it ended with the removal before.
  Editing paths keep removing at once (`maintain(0, …)` after `set_no_building_spawn`, the
  bulldozer, undo): queueing them would leave buildings on parcels the same edit removes
  (`remove_parcels_attached_to_edge` runs right after), and player edits already restart the
  census like road edits. This departs from the design, which listed `cleanup_stale_buildings`:
  since `SIM-02-G` it runs only from editing paths. Alternative: queue the editing paths too and
  keep the parcels until the removal (a second lifecycle for parcels, with player-visible
  buildings standing on deleted lots).
- **`SIM-02-J` — departures paced by the existing exit credit, one a slot.** The daily phase's
  claim already spends the exit credit (0.25 per eligible household, at most 48 a day); the
  queue serves the claims in order in the removal window, one a slot (476 an hour of capacity),
  each rechecked on live state. The design's second credit in the queue is not added: it would
  pace the paced claims again. `execute_demand_household_removal` and `run_daily_pass` are
  test-only helpers (`cfg(test)`) and stay so. Evidence: no envelope city departs households
  (`departures_per_household` 0 in every city, as on the previous build); the queue is covered
  by the new tests.
- **`SIM-02-J` — admissions created by a credit over the hour.** Slot 3 decides as before and
  claims the homes (so later offers in the batch and the next decisions see the claims); the
  queue creates households and carriers. The credit starts at one whole admission and accrues
  the hour's waiting count per slot, so the batch is created evenly, the first at slot 3 and
  the last by the next hour's slot 2. Each carrier departs paced from its creation, so the
  arrival departure profile is the old one shifted by about a slot. Alternatives: (1) schedule
  each admission's creation at a computed slot (the same spread, but timed events per admission
  and no credit); (2) keep creating carriers at slot 3 and only pace departures (the previous
  behaviour; carriers wait at the border up to an hour). The border node is saved through the
  save's node numbering; a border no saved road reaches is saved as none, and both games drop
  that admission (releasing its home) when it comes up. Evidence: the arrivals burst is 3.23%
  of departures in the busiest minute on both builds; admissions up to 16 an hour in the build
  hours and 1–3 once grown at 10k, busiest slot 1.
- **`SIM-02-J` — queued clearances repair the caches as slot 3 did.** The first build repaired
  every slot that removed a building with `finish_phase_removals`, as daily phases repaired
  compatibility removals. Its `bump_building_ref_revision` marks every building row changed, so
  in each hour with a clearance the census input copy recopied every row and the next hour's
  planning slot 0 rebuilt its site rows in full. Found by the slot timing: on each build's own
  `PERF-07` snapshot, slot 0's hourly work was 5.1 → 7.6 ms at 30k and 15.2 → 21.9 ms at 100k
  (worst 34.5 against 19.9 ms) while slots 1 and 2 were unchanged. A clearance now records its
  changed rows (`record_building_changes`), so a slot of clearances rebuilds the zone index and
  the entrance cache once, as slot 3 did; a slot that removed an incompatible building still
  bumps the revision, since that removal names no rows. The derived tables are the same either
  way (full and incremental rebuilds agree), and the gate's metrics are unchanged by the fix.
  Evidence: on the same city (the previous build's snapshot migrated to format 103), slot 0
  equals the previous build's (4.70 against 4.63–4.74 ms at 30k); the first build was not timed
  on the same city, so the share of its rise that came from its own snapshot is not separated.
- **`SIM-02-J` — household removal walks member lists.** `remove_households` collected the
  removed households' members by scanning every agent, O(agents) serial per call, which a
  per-slot departure queue would pay per departure. It now takes them from the maintained member
  lists (ascending agent index, the same set and the same descending removal order), and the
  O(households) selection-flag pass is gone.
- **`SIM-02-K` — SimCore-free subsystem test drivers stay (part 1).** Drivers that run one
  subsystem's slot functions on hand-built state, with no `SimCore` and no slot clock, test
  algorithms, not cadence: `DemandSystem::run_hourly_pass` and `run_planning_hour_for_test`,
  `publish_census_if_stale_for_test`, logistics `hourly_tick_with_trips`, households
  `run_household_operational_hour` and `assign_agent_workplaces_from_census`, and the building
  action planner's `plan_hour_for_test`. No game path sits behind them. Alternative: move every
  one onto a `SimCore` and the slot clock; rejected for part 1 as several thousand lines of
  test churn against the minimal-subsystem-setup rule.
- **`SIM-02-K` — `PERF-01` and the benchmark keep averaging 23 timed hours a day (part 1).**
  `EconomyHourTimes::demand_pass` still marks the 23 hours that do not start a day, although
  every hour now runs its four planning slots. The means stay comparable with earlier steps'
  and with the control binaries'. Alternative: time all 24 hours; better once no control
  predates part 1.
- **`SIM-02-K` — the operational hour and residential planning are two declarations at slot 0
  (part 1).** They were one `FixedSlot` costing `LegacyBoundary`. `operational hour` (`Dirty`)
  and `residential planning` (`CityScan`) are declared in that order at offset 0, so the order
  of work in slot 0 is unchanged (census publish, remainder, residential planning, then at a
  day's start the calendar close) and each states its own cost. Alternative: one declaration
  costing `CityScan`; rejected because it hides that the remainder does not grow with the city,
  and the residential slot belongs with the other planning slots that `SIM-02-K` part 5
  rewrites. `Cost::Dirty` is a new variant for the remainder: the fixed-slot rule already
  allowed O(dirty) work but no cost named it. Alternative: a `Publish` variant for the census
  publication; rejected, `CityScan` already means "grows with the city, `SIM-02-K` debt".
- **`SIM-02-K` — readers keep their build-at-once fallbacks (part 1).** The whole sweeps that
  built the supplier index, destination index and job board at once are deleted
  (`run_whole_supplier_sweep`, `run_whole_destination_sweep`, `run_whole_job_board` and the
  rolls' `run_whole`). A reader that finds no front for its hour still builds its own index at
  once (`members_for` for the job board), and planning that finds no demand snapshot front
  builds one at once. These O(B) fallbacks stay for part 5 and are not fixed slots. Alternative:
  fail the read; rejected, a world's first hour and an edit after the last window slot reach
  them in play.
- **`SIM-02-K` — the snapshot culls lane agents on their lanes' bounds, not their stored
  positions (part 2).** The plan culled on movement's `pos_x`/`pos_y`. That position cannot
  bound a lane agent's pose by any fixed margin: overlap correction moves cars back along their
  lane without rewriting it (one car spacing per stacked car), agents waiting for a replan keep
  it until they move, and a walker attaching to the sidewalk keeps its curb position for a
  frame. Daily happiness reads it, so rewriting it would change gameplay, and for lane agents it
  is not the snapshot's pose either (walker offsets, different sampling arithmetic). The cull
  therefore tests the planar box of the lane (or both lanes during a lane change) plus 1 m, a
  bound that holds by construction, and poses survivors with the unchanged code. Evidence:
  identical instances against the full pose on the 2k, 30k and 100k cities and through lane
  changes. Alternatives: a margin on the stored position (cannot be proven, so instances could
  differ), or keeping the stored position equal to the pose (a gameplay change).
- **`SIM-02-K` — posing stays inside the core lock (part 2).** Posing outside the lock needs a
  copy of the in-camera agents and shared lane geometry. After the cull the whole snapshot
  averages under 1 ms at 100k (p99 1.1–1.4 ms), so the copy would cost about what it saves.
  Revisit if part 7 is short by about a millisecond.

- **`SIM-02-K` part 3 — cells generated at load, not saved.** Alternatives: persist the generated
  cells (a save format change, every cell's frontages and curve sources in the save, larger
  saves); save which chunks were generated and regenerate those (a smaller format change, still
  a fixture and snapshot regeneration); a deferred rebuild bounded by cells per slot (the
  generated set would differ between a game and its reload, so a region would be served at a
  different slot after a reload and save/reload determinism would fail unless the progress were
  saved). Chosen: generate at load every cold chunk within reach of a road that grows cells or
  holding reserved cells, in chunk order, and leave cold any chunk whose cells would need a grid
  frame not yet registered. The check matters because frame ids follow registration order: a
  frame first registered at load would shift the ids of every frame the saved game registers
  later. Evidence: the fixtures register no new frame at load (0 chunks left cold at 10k, 30k
  and 100k); `PERF-07` load time rises 0.2, 0.45 and 1.6 s at 10k, 30k and 100k; the envelope
  is bit-identical and save/reload continues identically; the worst frame falls from 58.7 to
  9.2 ms at 10k 1× and from 179 to 82 ms at 100k 32×.
- **`SIM-02-K` part 3 — the zone-index rebuild after clearances stays.** The rebuild after a
  clearance slot also resets the vacancy lists to building order (`ALLOC-03`: admission breaks
  ties by vacancy position, and claims swap-remove vacancies out of order). An incremental repair
  equal to it must sort the vacancy lists and recompute every building's vacancy membership from
  its capacity, as the rebuild does, which leaves most of its O(B) cost (2.6 ms at 100k).
  Dropping the reset would change which homes admission picks (a gameplay change for 2.6 ms).
  Revisit with K5, whose per-zone lot lists replace the planning scans that read this index.
- **`SIM-02-K` part 3 — census work units not done.** Measured at 100k (attribution table): the
  operation and trial counters do not predict a staffing slice's cost (CCH point searches and
  supplier candidates do), and the worst census slots are unsliced steps. Options: (1) count the
  supplier searches, candidates and CCH point searches the staffing quotes make as work units
  (release-build counters in the freight quote path); (2) slice the consumer-market refresh and
  the route step; (3) run the census as a pure job off the frame, which needs its inputs frozen
  as the copy already is. (2) is needed in any case for the census's worst slot.
- **`SIM-02-K` part 3 — evacuations read current entrances.** Before, the clearance queue left
  the entrance cache stale between removals in one slot, so a later removal's evacuation trips
  could start from the entrance of the building the first removal swap-removed. The cache now
  stays current. The envelope cities show no difference: a clearance needs an empty building,
  so its evacuation rarely moves anyone.

### Corrections to part 2's statements (merge with the entries above)

- [Pure jobs](#pure-jobs), **Loading:** after "a stale one restarts at the next window slot, or
  runs at slot 0", add: "A save before the hour's actions slot also keeps the copy the census
  published at the hour's first slot read, whether its books were valid and the service
  ceilings it adopted; the load recomputes that census, which the hour's planning slots read.
  A later save runs a census of the loaded city, which only the derived jobs shown read before
  the next publication."
- Decided while unattended, "policy changes reach the census through its copy": replace the
  last sentence ("Loading still runs a census of the loaded city for the derived jobs shown
  before the next publication.") with: "A load before the hour's actions slot restores the
  published census (it still carries the old policy, as in the saved game); a later load runs a
  census of the loaded city for the derived jobs shown before the next publication."
- Part 2's premise "nothing reads the load-time census before the next publication" held while
  planning and the epoch ran at slot 0; from part 4 it holds only for loads after slot 3, and
  the test comment in `census_cases.rs` says so.
- `SIM-02-I` part 4 step results: save format 101 (`census_inputs.published`); the Kuopio
  fixture gained the column and its hash in `placements.json` (as NULL, which the `SIM-02-K`
  part 1 fix replaced with books that are not valid, what a game saved at the world's first
  slot writes).
