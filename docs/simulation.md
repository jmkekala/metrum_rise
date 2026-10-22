# Metrum Rise — Simulation Engine

This file owns the simulation engine contract: the game clock, the scheduler that decides when
each system runs, entity handles, timed events, snapshot rules, the sim-frame budget and
simulation tiers. Economy and growth rules stay in [`economy.md`](economy.md) and
[`demand.md`](demand.md); traffic movement stays in [`traffic.md`](traffic.md).

**Status:** `SIM-02` done (2026-10-22). `SIM-02-A` (guards and pre-refactor baseline), `SIM-02-B`
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
snapshot) are done, and part 3 (frame spikes) is done in four commits: the first (cells at
load, entrance cache, first-frame pathing, timing-run checks), the second (census slices in
work units), the third (clearance patches and census refresh slices) and the fourth (the
census's whole steps in slices). Part 4 (agent tick) has two commits done (lane buckets kept
across appends and swap-removes with interleaved movement shards; then commute departures leaving
over their minute's slots, the unused per-edge congestion deleted, save format 105, and the first
tick after loading prepared at load), and part 5 (planning slots)
has fourteen commits done (planning storage estimates from free lots, one downgrade scan, and
planning books prepared at load and when a fresh census book finishes; then the census storage
copied from the prepared live books at load and spare capacity for row storage; then the
slots' trips on one owned route scratch that loading prepares with the job search's scratch;
then the hour's four planning slots a game minute apart, with removals, departures and level
changes from slot 64, saves that keep the admission and removal diagnostics (save format 104),
and a census storage copy that keeps the live books' arena capacity; then a building leaving a
lot it keeps derives the lots without regenerating the lot's generated cells; then each
finished census's book prepared for planning at minute 59 of the hour before it serves, while
the zone index is current, with large row growth filled on the pool; then a book's pass rows kept
across its lendings, admission's border arrival costs kept per routing revision, the minute-59
preparation in two halves, and the removal slot's scans and zone-index rebuild on the pool; then
the census book's finance masks refreshed in the minute-59 preparation's second half, and the
zone-index chunk lists filled over a dense chunk grid; then the request queues, trips and
construction starts kept out of the hour's seven heavy slots, and the building tables a load
rebuilds kept with spare rows; then each planning slot run as two halves of its game minute, a
setup and a plan half a minute apart, and the census book's pass rows built at slot 936, save
format 106; then the census's business claims counted by their scan events and carrier quotes, and
the zone index kept across the removal slot's swap-removes; then each removal's agents and
households found in one scan, the removal slot's terrain boxes refreshed in later slots, the
census book's preparation in four parts with slot 936 queue-free, the expansion sweep's priced
routes weighed, and admission's border sweeps prepared at load; then each planning setup's scans
run beside its workforce count, a workforce reset passes over the buildings once, the site
feasibility cache split by parcel, and the live books lent last at load; then the frontage
delays summed by lane range on the pool, and each planning slot's free-place scan run beside
its site preparation).
Part 6 (100k traffic) is diagnosed: findings and two new bugs (`ECON-22`, `TRAFFIC-07`), no code.
Part 7 (acceptance) is done: `PERF-07` reports the frames another task preempted and accepts on
the frames that were not (open question 7, decided 2026-10-22); 30k at both speeds and 100k at 1×
pass. 100k at 32× is deferred to `PERF-08` (open question 6, decided 2026-10-22).
See the
[`SIM-02-A` baseline](#sim-02-a-guards-and-pre-refactor-baseline) and the
[step results](#step-results). Until each migration step lands,
the current cadence in
[economy.md § Time and cadence](economy.md#time-and-cadence) and
[demand.md § Cadence and handoff](demand.md#cadence-and-handoff) remains authoritative for the
systems it describes.

The weather and Finnish-road feature merge keeps this scheduler authoritative and bumps the
current-only save format to 107. The committed Kuopio fixture carries deterministic default
weather in that format.

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

### Decisions (2026-10-22)

- **`PERF-07` accepts on the frames no other task preempted.** A tier passes when its worst
  unpreempted frame (`max_unpreempted_ms`), its raw p99.9 and its worst preempted frame less the
  wait (`max_preempted_less_wait_ms`) are each at most 8 ms. See
  [Open questions](#open-questions), question 7.
- **100k at 32× is deferred.** The decision is taken when players' cities start to approach 100k
  agents; until then it is tracked as `PERF-08` and `PERF-07` does not require it. See
  [Open questions](#open-questions), question 6.

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
    what changed or came due since the previous call. The census publication, the hour's four
    demand planning plans and their four setups declare `Cost::CityScan`, which grows with the city
    (a setup O(B + H + P), run beside its count since `SIM-02-K` part 5's thirteenth commit; a plan
    O(B + H + P + Q log H) a call in buildings, households, parcels
    and queued projects): splits
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
4. `Queue` matchers. In the hour's seven queue-free slots (below) no queue but the building
   removals serves; the slot after one serves the skipped slot's share first, then its own.
5. `Rolling` chunks, then swaps for periods that ended.
6. `PureJob` launches and due results.
7. On a day boundary only: the calendar close (budget report window), O(zones).

Slices above a size threshold use Rayon with fixed chunks and ordered merges; smaller slices run
serially and give the same result.

**Implemented in `SIM-02-C`.** `Schedule` (`simulation/core/scheduler.rs`) holds the declarations in
dispatch order and rejects one that breaks the contract: a fixed-slot offset outside its period, a
fixed-slot system without a bounded cost, or an events system that does not cost its events. The
simulation's list and its per-slot dispatch are in `nodes/sim/core/schedule.rs`: timed events; the
census publication, the operational hour's remainder, the four planning setups (hour, offsets 0,
16, 32, 48), the residential, commercial and industrial planning slots and the actions slot (hour,
offsets 8, 24, 40, 56: one planning slot a game minute since `SIM-02-K` part 5, each half a minute
after its setup since its tenth commit; since the ninth commit the dispatch leaves the hour's
queue-free
slots without queue work, `QUEUE_FREE_SLOTS`, [Request queues](#request-queues)); and the calendar
close (day, offset 0). The `sim` debug channel
prints the list with costs, tiers, reads and writes.

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
  `ConstructionStart { id }`, each due at its minute's first slot, or the next slot when that one
  is queue-free (`event_slot`, `SIM-02-K` part 5, ninth commit). The queue keeps the spawn payloads in order, so demand planning
  still reads it, and each queued spawn carries the id its event names. A spawn dropped from the
  queue (a loading repair) leaves an event that fires and does nothing. When the queue is
  replaced wholesale (undo, a new world) or the clock is set directly (tests, tools), its events
  are rescheduled: each spawn at its due minute, at most one per minute after the current one,
  as the per-minute queue check started them, at the same slot of its minute. A save stores the wheel as the next frame will see
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
    are checked at each trip check ([Request queues](#request-queues)). Since `SIM-02-K` part 5
    (ninth commit) a trip event due in a queue-free slot is scheduled one slot later, in the same
    game minute; a freight trip records that slot as its live event
    (`Shipment::trip_event_slot`). Construction completions keep their deadlines: passive checks
    read `construction_complete_slot`, so the event stays at it.

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
- **Queue-free slots** (`SIM-02-K` part 5, ninth commit; `QUEUE_FREE_SLOTS` in
  `nodes/sim/core/schedule.rs`): the hour's four planning setups (0, 16, 32, 48) and plans (8, 24,
  40, 56), the removal window's first slot (64) and the census book's three preparation slots
  (936, 944, 953), twelve slots, each hold 1.2–2.6 ms of their own system at 100k residents (936
  since part 5's twelfth commit). In them no queue serves except the `building removals`, slot 64's
  own work: the slot after one serves the skipped slot's share first, then its own (two calls, so
  two batches when a queue is full), so every queue keeps its quota per hour and every credit
  (visits, admissions) accrues and pays out exactly as before, a slot later. No two queue-free
  slots are adjacent and each one's next slot is in the same game minute (compile-time checks).
  Trip events and construction starts due in a queue-free slot fire in the next one
  (`event_slot`). With nothing queued a skipped queue costs nothing; a skipped share costs its
  queue's own bound in the next slot.
- **Planning slots:** the queues serve between the hour's planning setups and plans (slots 1–55
  except 8, 16, 24, 32, 40 and 48), after the slot's fixed systems. They change the live city
  between a setup and its plan (hires, moves, purchases) but remove no building or household, so
  they never invalidate the published books or the setup's building candidates and never mark a
  flow field; each plan's re-proofs and the later setups see the hires.
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
hour's actions slot (slot 56, in minute 3) decided on the census published at slot 0.

- **Removal window:** building and household swap-removes run only in slots 64–479 of each hour
  (household departures from slot 65, whose share includes slot 64's; minute 4 to the census window; slots 4–479 before `SIM-02-K` part 5, fourth commit;
  `hiring::removals_allowed`). A swap-remove in the census window (480–959) restarts the
  census, because the structure revisions are part of its key; one before
  slot 64 leaves the published books invalid for the live city, so a later planning slot reading
  them skips, and the window starts a game minute after the actions slot's setup and eight slots
  after its plan (at 32× the actions plan and the removals may share a frame: about 0.8 and 0.4 ms
  at 30k). Over slots
  64–479 nothing reads the published books and no census runs, and the census input copy is
  filling: `CensusInputs::remove_building` and `remove_household` mirror each swap-remove into
  it in O(1), plus the O(households) parallel retargeting of copied home and store references.
  A removal requested in slots 480–959 or 0–63 waits for slot 64, at most 544 slots (34 game
  minutes). A removal's
  dependent work (agents evicted or evacuated, households left unhoused, shipments and caches)
  happens with it, through the existing lifecycle helpers.
- **`level changes`** (quota 8, slots 65–959 but the census book's preparation slots, each
  skipped share served in the slot after): the actions slot still proves the hour's business
  level changes in its construction epoch and checks that each target fits (lot, occupants, workers,
  economy profile, site), but no longer applies them. A proved change keeps its claims in the epoch
  for the changes after it and is queued with a permit: a digest of the building's asset, level,
  economy profile and, for a business, its door (`level_change_permit`). The door is derived afresh
  at both ends from save-stable identities, not read from the entrance cache, so a loaded game,
  whose cache is rebuilt, compares the same digest. A residential change needs no proof; its permit
  leaves out the door. The queue applies a change only while the building's digest equals its permit
  and the target still fits; a stale permit is rejected and nothing is proved again
  (`Work::LevelChangePermitRejections`). After a slot that applied any, the zone index and the
  entrance cache are rebuilt once, as the actions slot rebuilt them. A level change is an in-place
  change: `record_building_changes` names its row for the census input copy, which copies it again
  over slots 0–479 and defers it while a census reads the front, so it restarts no census. The queue
  does not serve slots 0–63, so the planning slots never see a building change under them.
- **`building removals`** (quota 8, slots 64–479): planned clearances from the actions slot's hour
  plan and buildings whose compatibility expired at their daily phase, as generation-checked
  building handles, oldest first. Each is checked again when served: a clearance only while the site
  may still be cleared (`can_demand_despawn`), its remaining occupants evacuated first; an
  incompatible building only while the check, run again with no elapsed day, still says so, so a
  building that recovered stays. Each clearance finds the agents and households naming the removed
  or the moved building in one scan each and repairs only those, and the slot's site boxes refresh
  their terrain render patches one a light slot later (`SIM-02-K`). A slot of clearances then
  rebuilds the zone index and the entrance cache once, as the actions slot did; a slot that
  removed an incompatible building bumps the building revision (`finish_phase_removals`), as daily
  phases did. A building whose daily phase
  queues its removal skips that day's settlement, as the immediate removal skipped it. Editing paths
  (`allocator.maintain` after a player's edit, the bulldozer, undo) still remove at once.
- **`departures`** (quota 1, slots 65–479, slot 65 serving slot 64's share too): the households whose daily phase claimed a departure
  from the daily exit credit (at most 48 a day), oldest first, each removed only if it may still
  leave (`HouseholdExitPolicy::is_eligible` on live state). The credit already paces the
  claims, so the queue adds no second one. A removal finds the household's members from its
  maintained member list instead of scanning every agent (O(members) plus the existing
  parallel O(agents) household remap).
- **`admissions`** (one per whole credit): the actions slot (slot 56) still decides the hour's
  admissions against the workforce calendar and claims each admitted family's home; the queue
  creates the households and their arrival carriers. The decided households wait in order (home
  handle, size, composition, arrival border). The credit, in slots, starts at one whole admission
  (960) at slot 56 and accrues the waiting count per slot, so the k-th of n is created at slot 55 +
  ceil(k·960/n) (the first at slot 56) and the last before the next hour's slot 56, each credit
  slot's admissions created in the slot after when that slot is queue-free (the first at slot 57).
  Each slot's
  carriers depart paced from their creation, so a lone carrier leaves at the next slot: the
  departure profile is the old one, about a slot later. An admission whose home left is dropped; one
  whose border no longer has a car road releases its home (`Work::AdmissionsDropped`). Queued
  admissions count as pending arrivals in the hourly recount. Appends neither restart the census nor
  invalidate the books.
- **Gone from the hour and the actions slot:** the departures at slot 0 before the household hour
  (`remove_queued_exits`); the actions slot's level-change application, despawns and carrier spawns;
  the building phase's immediate compatibility removal.
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
  Each slot's budget is in work units, a deterministic estimate of census work made from counts
  alone, never from measured time (`SIM-02-K`): a step's rows at the step's weight and, in the
  serial household-claim, staffing and business-claim sweeps, the searches each row made (network
  route searches, supplier searches, candidates and routes, input quotes, consumer candidates and
  fallbacks, the business claims' scan events and carrier quotes, funded-operation searches); the
  expansion sweep's priced supplier routes weigh 230 (`EXPANSION_ROUTE_EXTRA`, `SIM-02-K` part 5),
  the current-staff sweep's 30. A uniform step takes the rows its budget covers; a
  serial sweep stops at the row whose work reaches the budget. The budget is ceil(R / D): R the
  previous census's work per step less this census's, summed, or, if more, the rows left at
  their weights with a sweep in progress at its observed rate; D = max(S − 96, S / 4, 1), S the window
  slots left. The census so plans to finish 96 slots before the window ends; from S = 128 on,
  work it did not plan for spreads over the slots left. Slot 959 takes the rest. The
  consumer-market refresh runs as eleven (its start; the household budgets, sliced and
  serial, with the walk's running treasury carried; the access rows, sliced; after a membership
  change the index entries, sliced, each mode's index build and each copy to another category,
  one at least a slice, and the reservation positions, sliced; the finish's decision, and for a
  book not retained the claims' accounts in household ranges and each index's availability, one
  at least a slice; its end) and setup as four (the port counts and the supplier order sliced,
  the import plans' layout and the end whole). Route quotes run as four steps (their start with
  the route cache's storage; after a routing or border change the border sweeps; when the quote
  inputs changed the door access and the quotes, in building ranges, named current only once
  every row is written). Each closure pass's input-source reset runs as five (its start; after
  a geometry change the supplier layout in source ranges, the index entries sorted into one run
  per resource, and the index one resource at least a slice; the rates and availability in
  source ranges), and business seeding as eight (its start; the buyers in ranges, serial; the
  book's layout start; after a layout change the buyers' access and recipe ports in ranges, the
  index entries and the indices one resource at least a slice; the accounts' availability); the
  business claims run as a serial sweep by seller (`SIM-02-K` part 5). A
  census restarted inside the refresh leaves the customer book torn, and the restarted refresh
  withdraws it first, as a load does; one restarted inside the quotes, the supplier index or
  the business layout redoes them, since none is named current part way. The remaining whole
  steps (repair, propagation, and the starts and ends the sliced steps run whole: the setup's
  layout and end, the supplier index entries, the business layout's start, entries and
  accounts) start only in a slot that has run no other step. Frontage prices publish when a census starts. A
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
  next window slot, or runs at slot 0. Loading also gives census storage that holds no census a
  prepared copy of the live books (`SIM-02-K` part 5), and a finished census's book is prepared
  for planning in four parts, at slots 936 (pass rows and the supplier rows' factory), 944 (the
  supplier layout), 953 (the rest) and 957 (the business finance masks) of the hour before it
  serves (sixth, seventh, tenth and twelfth commits; a second half that runs later than 953
  refreshes the masks itself, and loading prepares the whole at once).
- **Readers:** buildings take funded jobs and activity from their copies at their phases, and
  the demand snapshot sweep reads the same copies. Restock and visit requests read only the
  published census's walking proofs and household access rows, and job search the funded jobs
  copied onto each workplace (`SIM-02-J`). Demand planning lends the published books, with the
  buildings started since the copy appended as buyers, and runs no census of its own.
  Commercial and industrial construction is proved once an hour, in the hour's actions slot, on the same books, and keeps
  a saved permit until it starts (`SIM-02-I` parts 3 and 4). The planning slots (plans at slots 8,
  24 and 40) and the actions slot (plan at 56) all read the census published at slot 0; a save in
  slots 0–55 keeps what restores it exactly (`census_inputs.published`), so a load there plans on
  the same books (part 4; the span was slots 0–2 until `SIM-02-K` part 5, 0–47 until its tenth
  commit, save format 106). A load from slot 56 on runs a census of the loaded city, which nothing
  reads before the next hour's
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
- **Acceptance (decided 2026-10-22):** at 30k at 1× and 32× and at 100k at 1×, the worst frame no
  other task preempted (`max_unpreempted_ms`), the raw p99.9 and the worst preempted frame less
  its wait (`max_preempted_less_wait_ms`) are each at most 8 ms. `tools/sim02_gate.py perf07`
  checks it and fails a run that misses it or lacks the scheduler statistics. 100k at 32× is
  reported but not required: it is deferred to `PERF-08`.
- **Target (as first set):** max ≤ 8 ms at 30k and 100k, at both speeds. 32× at 100k is the binding
  case: one
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
- **Preemption** (`SIM-02-K` part 5): the eight pinned CPUs are shared with the rest of the
  machine, and a pool worker that another task keeps off its CPU stalls the parallel phase it
  holds work in, in whichever phase was running (render snapshot or agent tick), at a different
  minute in every run. A spike frame is attributed by reading the threads' scheduler statistics
  (`/proc/self/task/*/schedstat`: time on a CPU, time runnable but waiting for one) around it in
  a diagnostic build: a preemption frame has the phase's ordinary work plus one pool worker's
  run-queue wait, while the test's own thread runs about 0.07 ms a frame. Every snapshot frame
  over 2 ms and the agent-tick spikes at 10k and 30k held one worker waiting 1.0–5.4 ms for a CPU
  taken by another task on the pinned CPUs, at different minutes in every run; the largest, in a
  diagnostic 10k day, was a 10.0 ms frame whose 9.5 ms agent tick held a 9.6 ms wait. That is
  noise on the 1× maxima at 10k and 30k (up to about 3.6–5.3 ms in the gate runs), not
  simulation work; the count over 8 ms and p99.9 are robust to it, a single maximum is not. Timing runs keep the machine quiet, the
  run's own monitoring included. At 100k 1× a full day of per-phase timing with the same
  statistics found 59 of the 184 agent ticks over 6 ms preempted (a worker waiting 1.1–7.8 ms);
  the others were departure waves, removed by `SIM-02-K` part 4's second commit. Across the
  fourteenth commit's runs the 100k 1× count of frames over 8 ms moved between 12 and 36 for
  either build, and every recorded worst frame of the last build showed the preemption pattern,
  so a single raw maximum and the over-8 count are not reproducible on this machine; acceptance
  needs preempted frames separated (part 7).
- **Preemption columns** (`SIM-02-K` part 7). Before each timed frame and after it, outside the
  timed span, the benchmark reads the run-queue wait (`/proc/self/task/<tid>/schedstat`, the
  time a thread was runnable but waiting for a CPU) of every shared-pool worker and of the
  thread that runs the frame, and records the largest growth among them (`wait_ms`). A frame
  whose largest growth is 0.5 ms or more counts as **preempted** by another task. `PERF07_DAY`
  adds `max_unpreempted_ms` (the maximum over the other frames), `over_8_ms_preempted` and
  `over_8_ms_unpreempted`, `preempted_frames`, `max_preempted_less_wait_ms` (the largest
  preempted frame less its wait: a preempted frame that still exceeds 8 ms after its wait is
  removed hides work), `wait_classes` (frames, mean, maximum and frames over 8 ms by largest
  wait: under 0.05, 0.1, 0.25, 0.5, 1, 2 and 5 ms, and above), `worst_unpreempted`, a `wait_ms`
  on each worst frame, and the mean agent tick, slot work and pathing (`mean_agent_ms`,
  `mean_slot_ms`, `mean_pathing_ms`). Test builds only, Linux only (elsewhere no thread is
  watched and no frame counts as preempted), no dependency, no `unsafe`, no allocation per
  frame. `METRUM_PERF07_SCHEDSTAT=0` skips the reads, to measure their overhead against the same
  build. `tools/sim02_gate.py perf07` prints the split in each tier's line and a table of every
  run, also kept in the report (`perf07_table`). The gate's "no worse" limits stay on the raw
  maximum and p99. The threshold and the overhead are measured in the part 7 results.

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
3. **K3, frame spikes** (done 2026-10-20, four commits). Done: (a) generated cells built during
   loading instead of by the first lot regions served (25–134 ms a region at 100k, all day), the
   entrance part of (c) (a removal keeps a clean entrance cache in step), (d) the route scratch
   and flow fields built at load, (e) test-build checks out of timing runs, (b) for the sliced
   census steps, budgeted in work units (results section, part 3, second commit), the clearance
   site publication (one terrain box per site change, refreshing only the patches the changes
   touch), the consumer-market refresh and setup sliced, the slot plan taken per step, and the
   first census after loading (results section, part 3, third commit), and the route quotes,
   business seeding and its layout, the input-source reset, the refresh's finish, index build
   and copies, and cleared storage growth (results section, part 3, fourth commit). The
   zone-index part of (c) stays as a decision. Every worst `PERF-07` frame of the K2 build was
   attributed to its systems first (table in the results section).

   Left, carried as findings in [Decided while unattended](#decided-while-unattended):
   - repair's preparation (2.2 ms at 100k);
   - setup's import-plan layout (1.7 ms);
   - each customer index mode's build (1.35 ms);
   - business seeding's start (1.4 ms);
   - the first census after loading (its refresh start 1.8 ms, the route cache's first growth
     2.1 ms, the staffing catch-up);
   - the census window's first slot outside the steps (0.3–1.8 ms);
   - (d)'s first trip events (done by part 5's third commit: the slots' trips run on one owned
     route scratch that loading prepares).

   K3 closes with these left; `SIM-02-K` part 7 (acceptance) revisits them if they stand between
   a tier and the 8 ms limit.
4. **K4, agent tick** (two commits done, 2026-10-20 and 2026-10-21). Done: lane buckets kept
   across appends and swap-removes instead of rebuilt each tick and filled on the pool; movement
   shards interleaved (results section, part 4, first commit). Second commit: the 100k 1×
   agent-tick peaks were departure waves (every agent due in a minute planned its trip in the
   frame that crossed into it), and a minute's departures now leave over its 16 slots, a gameplay
   change of under a game minute; the unused per-edge `current_congestion` deleted (save format
   105); the first tick after loading prepared at load (results section, part 4, second commit).
   The 100k 1× agent tick now stays under 6 ms in every frame without preemption. Left, with
   findings in [Decided while unattended](#decided-while-unattended):
   - indoor agents off the per-frame movement pass (a departure wheel, worth 0.3–0.5 ms a frame,
     which does not touch the waves);
   - the serial handoff set;
   - pose sampling;
   - passes over every agent;
   - the trip-plan cost, about 120 µs a plan and 6.7 ms of CPU per 100k 32× frame, which is
     routing work and bears on open question 6.

   Planned for all of K4: about 3 ms at 1× and 7 ms at 32× at 100k; the first commit saves 1.4
   and 2.5 ms. Part 5 (planning slots) ran before part 4's remaining items, because the planning
   slots set the maximum at every tier.
5. **K5, planning slots** (fourteen commits done 2026-10-21/22; results sections, part 5). Measured
   first:
   the planned scans cost 0.1–1.5 ms each at 100k, and the measured maxima were cold planning books
   after loading (the residential slot of the first hours: 100k 49, 42 and 31 ms; 30k 17 and 14 ms),
   not those scans. Done: storage estimates from free lots; one downgrade scan in the actions slot;
   the parallel constructing filter; planning books prepared at load and when a fresh census book
   finishes (first commit); the census storage copied from the prepared live books at load (the
   census-book preparation after loading is gone); row storage grown with spare capacity (the second
   storage growth after the first appended building is gone) (second commit); the first frame after
   loading. It was cold route scratch: each pool worker's thread-local `TripQueryScratch` costs 7–9
   ms at 100k. Now one owned scratch is installed around the frame's slot loop, and loading prepares
   it together with the job search's shards and leg storage (third commit). The hour's four planning
   slots then ran a game minute apart (slots 0, 16, 32, 48) and the removal window, departures and
   level changes moved to slot 64, so at 32× no frame holds two planning slots or a planning slot
   and a removal; saves keep the admission and removal diagnostics (save format 104); and the census
   storage copy keeps the live books' receipt-arena and claim-staging capacity
   (`JobFunding::reserve_census_arenas_like`) while the census's receipt estimate keeps spare
   capacity, so the 01:00 slot after a load grows no journal (fourth commit). The spacing is a
   scheduling choice with a gameplay side, taken under the run's rule (keep the envelope closest,
   prefer the simpler option). A building leaving a lot it keeps now derives the lots without
   regenerating the lot's generated cells, which reproduced them, so the 30k 32× minute-4 frames
   fall from 6.9–8.7 to at most 5.7 ms (fifth commit; the 1× render-snapshot spikes were
   attributed to preemption of a pool worker, not snapshot work). 10k and 30k now meet 8 ms at
   both speeds in this run. Each finished census's book is then prepared for planning in minute
   59 of every hour, once its census is done (in two halves since the seventh commit; the window-slot preparation a new
   world's first census already had), so the residential slot no longer merges two hours of
   site-row changes and lays the supplier and buyer indices out again; the preparation waits
   while the zone index is stale; and large row growth is filled on the pool (sixth commit): the
   residential slot falls from 8.95 to 4.87 ms at 100k and from 2.59 to 1.37 ms at 30k, the
   actions slot from 7.03 to 6.46 ms at 100k (diagnostic), and the 30k 32× hour frames are 5.1–5.6
   ms (sixth commit). The seventh commit keeps a book's pass rows across its lendings (0.81–0.84
   → 0.05–0.07 ms a lending at 100k), keeps admission's border arrival costs per routing revision
   (about 1.1 ms of the actions slot), splits the minute-59 preparation into two halves nine slots
   apart (slot 944 the supplier rows, slot 953 or later the rest on kept rows), and moves the
   removal slot's scans and the zone-index rebuild onto the pool: at 100k residential 4.90 → 4.32,
   commercial 3.54 → 2.94, industrial 4.43 → 3.88, actions 6.45 → 4.15 ms, the removal window's
   first slot 4.62 → 3.46 ms and the minute-59 preparation 4.91 → 3.77 + 2.44 ms (diagnostic). In
   `PERF-07` the 100k 1× maximum falls from 14.6 to 12.9 ms (the first hour after loading) and the
   steady worst frames are 10.3–11.0 ms (slot work 5.7–6.9, agent tick 3.4–4.0); 30k 32× p99.9 is
   5.32 ms. The eighth commit found the residential slot's re-proof of constructing buildings to
   be mostly the business book's finance-mask refresh of every buyer, which the census's
   financing publication left for the hour's first masked claim (0.8 ms at 100k); the minute-59
   preparation's second half now runs that refresh, and the residential slot falls from 4.22 to
   3.42 ms at 100k and from 1.33 to 1.07 ms at 30k (diagnostic). The zone-index rebuild fills the
   chunk lists over a dense chunk grid instead of sorting every building (2.10 → 1.51 ms at
   100k). In `PERF-07` the 100k 1× maximum stays 13.0 ms (the first hour after loading) and the
   steady hour-slot frames 10.4–11.6 ms. **The 100k 1× target is not reached.** In steady hours,
   every 100k 1× frame over 8 ms that is not preemption fell in one of the hour's six slot minutes
   (:59 to :04), each holding 4.2–4.9 ms of slot work on a 3.6 ms agent tick and a 0.8 ms snapshot.
   Done in the ninth commit: queue-free heavy slots, spare rows at load, and events moved off the
   heavy slots. The request queues serve nothing in the hour's seven heavy slots (0, 16, 32, 48,
   64, 944, 953) and the next slot serves the skipped share first, so quotas and credits per hour
   are unchanged; trips and construction starts due in a heavy slot fire one slot later in the same
   minute; and the building tables a load rebuilds (site clients, entrance cache, vacancy
   positions, freight route access rows) keep spare rows, so the first construction start after
   loading falls from 2.7–3.2 to 0.77 ms and its freight requests from 2.24 to under 0.2 ms
   (identical results). This is a gameplay timing change taken under the run's rule (see [Decided
   while unattended](#decided-while-unattended)). **The 100k 1× target is still not reached:**
   frames over 8 ms fell from 146 to 111. In a schedstat diagnostic day, 86 work frames remained,
   each one a heavy slot running its own system: planning setup 2.8–3.9 ms, census-book preparation
   3.0–3.5 ms, or removals 3.3 ms (zone index 1.5). These sit on an agent tick of 3.6–4.1 ms and a
   snapshot of 0.8–1.1 ms, for frames of 8.0–10.8 ms (mean 8.6). The other 34 were preempted.
   Done in the tenth commit: each planning slot runs as two halves of its game minute. The setup
   at the minute's first slot (0, 16, 32, 48) counts the live workforce and, beside it, runs the
   plan's scans of lots and existing buildings (the actions slot scans every zone's downgrade
   candidates); the
   plan eight slots later (8, 24, 40, 56) lends the books, prepares site rows, proves the queue and
   the earlier plans again and decides, and the actions slot executes at 56 (the first admission
   at slot 57). A save between keeps the setup (save format 106, `demand_planning_setup`). The
   census book's pass rows are built in a lending of their own at slot 936. At 100k the planning
   halves hold 1.2–2.0 ms (setups) and 1.6–3.0 ms (plans) against 2.8–3.9 ms, and the preparation's
   first half 2.5 ms against 3.5 (diagnostic). In `PERF-07` the 100k 1× frames over 8 ms fall from
   88 to 36 in the same session; no planning half and no first-hour lending after a load is among
   them.
   Done in the eleventh commit: the census's business claims are counted (they ran in row slices
   at a fixed weight, and on some cities and hours the tail sellers' scans cost 20–70 times that;
   the business book now counts the scan events and carrier quotes of its claims, the census
   weighs them, and the claims step stops at the seller whose counted work reaches the budget), so
   no census slot holds more than about 2.2 ms at 100k (diagnostic; 10.6 before); and the zone
   index is kept across the removal slot's swap-removes, which then only return the vacancy lists
   to building order and recompute the admission order (zone index 1.3–1.5 to 0.6 ms, slot 64 3.46
   to 2.65 ms at 100k, diagnostic). Both are exact: the envelope is bit-identical and the save
   format is unchanged. In `PERF-07` (same session, same cities) the 100k 1x census-slice frames
   are gone and the maximum falls from 14.2 to 11.7 ms. **The 100k 1x target is still not
   reached:** 9 work frames a day over 8 ms remain, at 8.0–9.7 ms: slot 64, 4 frames (removals
   2.95–4.35 ms; the worst is an hour with three removals), slot 944's census-book preparation, 4
   frames (2.9–3.3 ms), and the first actions plan after loading, 1 frame (00:03, 4.4 ms). The
   other frames over 8 ms are preempted.
   Done in the twelfth commit: each removal finds its agents and households in one scan each, so
   its evacuation, eviction and remaps visit only those (0.69 → 0.37 ms a removal at 100k), and
   the removal slot's terrain boxes, render state, wait for one a light slot after it (the slot
   2.40 → 1.26 ms mean, 3.67 → 1.90 ms maximum, diagnostic); the census book's preparation runs
   in four parts (the pass and supplier rows at 936, which becomes queue-free, their layout at
   944, the rest at 953, the finance masks at 957; 944 2.65 → 1.69, 953 3.13 → 2.33 ms); the
   expansion sweep's priced routes weigh what they cost (its slices at most 2.20 → 1.69 ms); and
   loading prepares admission's border sweeps (the first actions plan's admission 1.15 → 0.03
   ms). All are exact for the simulation but slot 936's queue-free entry (a scheduling change;
   the envelope stays within tolerance, net income within 0.74%). In `PERF-07` (same session,
   same cities) the 100k 1× frames over 8 ms fall from 31 to 9 and p99.9 from 7.37 to 7.11 ms.
   **The 100k 1× target is still not reached:** in a schedstat diagnostic day 100k 1× has 1 work
   frame over 8 ms (the industrial planning setup, 8.0 ms); every other frame over 8 ms is
   preempted. In the `PERF-07` day eight of the nine follow the preemption pattern and the ninth
   is the first actions plan after loading (00:03, 3.4 ms of slot work on a 4.0 ms agent tick).
   Done in the thirteenth commit: each planning setup runs its scans beside its workforce count
   (the scans' lot priorities do not read the workforce), a zone setup's lot and building scans
   side by side and the count's three passes overlapped; a workforce reset passes over the
   buildings once with thread-local sums that wrap as the atomic adds did, keeping the homes in
   building order; the site feasibility cache is split into 32 locks by parcel; the free-lot
   storage estimate is kept while the parcel revisions hold (it only sizes storage); and loading
   lends the live books last when census storage took the warm copy, so the first planning slot
   keeps their pass rows (0.8 to 0.09 ms). The industrial setup falls from 2.16 / 2.90 to 1.49 /
   1.95 ms, the other setups by 0.1-0.3 ms, the actions slot from 3.02 to 2.78 ms mean
   (diagnostic). All exact: the envelope is bit-identical and the save format is unchanged.
   **The 100k 1x target is still not reached.** A schedstat diagnostic day at 100k 1x has no work
   frame over 8 ms: all 28 frames over 8 ms are preempted, and 9 of them are log stalls. In
   `PERF-07`'s own 12 worst frames, actions-slot frames still reach 9.3-10.0 ms (2.8-3.4 ms of
   slot work on a 4.5-5 ms agent tick), plus one removal-window frame at 8.8 ms. So the margin is
   not yet robust.
   Done in the fourteenth commit: the frontage delay update (once a simulated second, in the
   frame that crosses each game minute) notes each agent's lane in parallel, then one task per
   lane range adds its cars in agent order and updates its own lanes (0.68 → 0.45 ms at 1×, 0.72
   → 0.47 ms at 32×); and each planning slot (8, 24, 40, 56) runs the workforce reset's
   free-place scan beside its site preparation and queue replay, the constructing workplaces'
   jobs coming from the books' constructing rows (actions slot 2.74 → 2.56 ms, commercial plan
   1.50 → 1.32, industrial 1.64 → 1.42, residential 1.87 → 1.77; slot 56 in schedstat days
   2.78 / 3.38 → 2.72 / 2.89 ms, slot 8 1.84 / 3.57 → 1.57 / 2.64). Both exact: the envelope is
   bit-identical and the save format is unchanged.
   **Status after the fourteenth commit.** In a schedstat diagnostic day every 100k 1× frame
   over 8 ms that carries slot work was preempted. There were 46 frames over 8 ms: 23 preempted,
   22 diagnostic-log stalls, and 1 agent-tick frame in the evening commute window with no slot
   work and no waiting worker (21:45, 8.1 ms, a 6.8 ms tick). `PERF-07`'s 12 worst frames in
   both runs show the preemption pattern. The largest slot-work frames are the actions slot
   (2.7 / 2.9 ms) and slot 65 (2.6 / 3.2 ms), at 7.0–7.5 ms when quiet. The minute-3/4 agent
   tick is attributed: there is no departure, admission or arrival clustering (tick mean by
   minute 2.98–3.21 ms); the frame crossing each game minute runs the frontage update; the frame
   after a slot frame has a movement pass about 0.3 ms slower (cache eviction); commute hours
   add 0.4–0.6 ms.
   Left:
   - **The actions slot's execution in its own queue-free slot 60** (save the workforce view,
     the admission budget and the plan's validity: a save format change and a scheduling change;
     expected slot 56 about 1.4 ms and slot 60 about 1.3 ms).
   - **Slot 65:** 1–3 cell-lot regions an hour re-derive with no candidates (0.46–0.73 ms each)
     after `release_invalid_empty_cell_lot`, plus the first construction start.
   - **The frontage lane pass** (0.34 ms over 184k lane records).
   - **The census-book parts at 936 / 953** (2.1 / 2.5 ms).
   - **Warming the first queue re-proof after a load** (about 1.8 ms).
   - **Maintained workforce totals**, **part 4's indoor agents** and **zone lists.**
   - **The job board and destination fallbacks,** and the per-building commute-quote clear.

   The planning slots still declare `Cost::CityScan`.
6. **K6, 100k traffic** (diagnosed 2026-10-21; findings, no code; results section, part 6).
   No gridlock: the lane wait-for graph has no cycle at any tier, and the cars flow. The always-moving agents are
   shoppers walking 2 km (30k) to 3.2 km (100k) to the few stocked groceries, all in the city's
   first-built strip, because the funding census lets stores, in building order, claim the
   nearest households with no distance bound, and the fixture's store capacity is six times
   demand (`ECON-22`). Border imports also queue on the trunks, and a truck entering a lane at a
   border node skips the entry-slot check, so the queues stack trucks at one point
   (`TRAFFIC-07`). A 600 m claim bound prototype passes the envelope and removes 17,000–18,000 of
   the 100k city's 49,000–53,000 agents on the road, but saves only 0.4 ms (1×) and 0.7 ms (32×)
   of agent tick, and its 100k city failed the fixture's continuation check. Neither fix is
   landed: the census claim rule is an economy design decision. Open question 6 is not shrunk by
   traffic.
7. **K7, acceptance** (done 2026-10-22, after review; results section, part 7). `PERF-07` now
   reports the frames another task preempted (a run-queue wait of 0.5 ms or more in one of the
   frame's threads; test builds only), with the threshold and the reads' overhead measured.
   On the final code, two full days at every tier and speed:
   - **Passed:** the structural criteria (no `simulate_tick_internal`,
     `simulate_operational_hour_internal` or minute-0 path; an hour's slot 0 holds only declared
     fixed slots: the census publication, the operational hour's remainder, the residential
     setup and, once a day, the `Cost::Constant` calendar close; hour-boundary frames are no
     longer any run's worst); the envelope, determinism and burst limits, bit-identical to part
     5's last commit; 30k at 1× and 32× (raw maximum 4.6–6.1 ms); 100k at 1× on frames no other
     task preempted (7.35 and 7.73 ms; the raw maximum is 10.2–14.1 ms through preempted frames).
   - **Failed:** 100k at 32× (mean frame 13.6 ms, worst frames 22–24 ms without preemption).
   - **Decided 2026-10-22:** [question 7](#open-questions), option (a): `PERF-07` accepts on the
     unpreempted maximum with the raw p99.9 and the preempted frames less their wait each at most
     8 ms; on both acceptance days 30k at 1× and 32× and 100k at 1× pass it.
     [Question 6](#open-questions): 100k at 32× is deferred until players' cities approach 100k
     agents and tracked as `PERF-08`. With both, `SIM-02` and `PERF-07` are done.

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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section. Since the
  tenth commit each planning slot plans `PLANNING_SETUP_LEAD` (8) slots after its setup (plans at
  8, 24, 40, 56), a save in slots 0–55 keeps the published census (the rule follows
  `PlanningSlot::Actions`), and a save between a setup and its plan keeps the setup
  (`demand_planning_setup`, save format 106).

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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.

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
  membership (see Decided while unattended). Since `SIM-02-K` part 5's eleventh commit each
  clearance keeps a current zone index in step, and the slot's repair only returns the vacancy
  lists to building order and recomputes the admission order (0.6 ms at 100k). Within a slot that
  clears several
  buildings, a later
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

#### `SIM-02-K`, part 3 — census slices in work units (2026-10-20, second commit)

- **What changed:** the funding census's slot budget counts work units instead of rows (see
  [Pure jobs](#pure-jobs)). Slicing never changes a census's result.
  - **Counters.** Three monotonic counters, read only by the census's slicer: the freight route
    cache counts its network searches on a miss, the recurring supply its nearest-supplier
    searches, candidates visited, routes priced and input quotes, and the consumer market the
    households its claims visit and the trip fallbacks they try. Each is one integer add on a
    path that already does the work; the census sums them with weights, O(1) a row.
  - **Weights.** A temporary per-row profile of the three serial sweeps at 100k (the first window
    after loading, 456,636 rows over two closure passes) found no row above 0.30 ms. A
    least-squares fit of row time on the counts gave about 32 ns a row, 14.4 µs a network route
    search, 2.2 µs a funded-operation search, 155 ns a staffing trial, 54 ns a consumer
    candidate, 42 ns a supplier candidate and 33 ns a supplier route. The weights are rounded
    from it (15,000, 2,000, 100 an input quote, 50, 40, 30 and 25 a row; 500 a supplier search,
    5,000 a consumer fallback); the uniform steps' row weights come from their measured totals.
    Part 5's eleventh commit adds the business claims' scan events (270) and carrier quotes (150),
    fitted to their slices at 100k (about 285 ns a scan event).
  - **Plan.** R is the previous census's total less the work used, or the estimate of the work
    left if larger; a serial sweep in progress prices its rows left at its observed rate.
    D = max(S − 96, S / 4, 1). A first build without the observed rate and with D = max(S − 96,
    1) crawled on the rows' estimate when the previous census underestimated, then spent 3–7 ms
    a slot catching up at its planned last slots (859–864 at 100k); the observed rate and the
    S / 4 tail remove most of that.
  - **Unsliced steps** start only in a slot that has run no other step, so a slot holds at most
    one of them.
- **Fix found by the change:** a census restarted after the consumer-market refresh and before
  setup replayed the previous census's household quotes against a reset customer book. The
  refresh had reset the book (its inputs changed), but the recorded quotes were dropped only at
  setup; the restarted refresh found its inputs unchanged and reported the book retained. The
  refresh now drops the recorded quotes when it resets the book. Under row budgets the step after
  the refresh usually ran in the same slot; once unsliced steps start in their own slots, the
  existing sliced-and-restarted equivalence test (5 units a slice, restarted after the third
  slice) caught it. The envelope is unchanged.
- **Census cost per slot** (`perf07_census_slot_costs`, an ignored release test: the census of
  two consecutive windows of the prepared `PERF-07` city stepped slot by slot without the slot's
  other systems, `RAYON_NUM_THREADS=8`, pinned; diagnostic timing, not acceptance). Window 0 is
  the first after loading, window 1 the next hour's. Before is the part 3 first commit:

  | Tier | Window | Total ms | Max slot ms | Slots > 1 ms | Worst sliced-step slot ms | Last slot with work |
  | --- | --- | --- | --- | --- | --- | --- |
  | 100k | 0 | 144.0 → 143.8 | 21.6 → 21.5 | 17 → 19 | 20.4 → 2.8 | 952 → 936 |
  | 100k | 1 | 91.4 → 101.3 | 14.6 → 11.0 | 16 → 8 | 14.6 → under 1 | 920 → 741 |
  | 30k | 0 | 40.0 → 30.8 | 8.0 → 6.1 | 10 → 4 | 2.9 → 0.3 | 952 → 932 |
  | 30k | 1 | 20.5 → 20.0 | 3.1 → 2.6 | 7 → 3 | 3.1 → 0.3 | 920 → 742 |

  Every slot over 1 ms after the change is an unsliced step, except at 100k window 0, whose
  census did about 1.6× the previous census's work (current staff 68 against 42 ms the next
  hour) and caught up at 1.4–2.8 ms a slot at slots 924–925. The unsliced steps at 100k:
  consumer-market refresh 11.0–21.5 ms, route quotes 1.5–10.3 ms (10.3 when the frontage prices
  changed), business seeding 5.1–5.7, the input-source reset 3.5–5.2, setup 4.2–4.4, repair
  2.0–3.8, propagation about 1.0; at 30k 0.9–6.1, 0.2–2.5, 1.6–2.6, 1.0–1.3, 0.5–1.4, 0.5–1.1
  and 0.2.
- **Window bound.** The census finishes inside its window by construction: slot 959's budget is
  unbounded, and the publication at the next slot 0 runs whole a census that is missing,
  unfinished or stale, as before, so no city can need more than the window. Measured, the last
  slot with census work is 936 and 741 at 100k, 932 and 742 at 30k (of 480–959). A city with
  more census work raises the per-slot budget in proportion instead of running past the window;
  a census needing more than its plan spends the 96 slack slots and the S / 4 tail first, and
  only what is left at slot 959 runs in one slice.
- **Not done in this commit:** slicing the unsliced steps (options in
  [Decided while unattended](#decided-while-unattended)); the clearance site publication (found:
  one bounding box over every site change of the slot, see the same section).
- **Equivalence:** the busy town's census in slices of 1 and 2,000 work units equals one run bit
  for bit (`the_census_in_work_unit_slices_equals_one_run_on_its_copy`); the existing tests of
  slices of 1, 5, 37 and 1,000 units with restarts, the stepped window with a network restart
  and the save/load continuations pass.
- **Envelope and determinism** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass; the envelope is
  bit-identical to the first commit's, and repeat, 1 and 3 workers and save/reload are identical.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`), full day, each build
  on its own prepared cities, in one session with the first commit as the control. Milliseconds
  and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 9.5 (9.6) | 1.23 (1.25) | 1.45 (1.49) | 0.54 (0.54) | 2 (2) | 0 (0) |
  | 10k | 32× | 10.6 (10.9) | 2.35 (2.53) | 4.93 (5.21) | 1.06 (1.04) | 2 (2) | 0 (0) |
  | 30k | 1× | 20.9 (19.5) | 2.28 (2.28) | 3.48 (3.84) | 1.33 (1.30) | 6 (8) | 2 (2) |
  | 30k | 32× | 25.5 (23.1) | 7.32 (8.47) | 22.94 (22.20) | 3.30 (3.25) | 21 (33) | 3 (3) |
  | 100k | 1× | 60.9 (60.4) | 8.31 (8.44) | 16.35 (18.12) | 6.14 (6.10) | 1,454 (1,410) | 76 (133) |
  | 100k | 32× | 81.7 (81.4) | 33.75 (36.49) | 76.16 (78.50) | 16.95 (16.78) | 2,700 (2,700) | 984 (662) |

  Every gate limit passes. At 100k 1× the control's two census-window frames (minutes 517 and
  529, 59.4 and 38.7 ms, 53.8 and 33.3 ms of slot work) are gone, frames over 16.7 ms fall from
  133 to 76 and the p99.9 from 18.1 to 16.4 ms; the census window's worst frame is now its
  start at minute 30 (28.8 ms, the control 28.5: route quotes and the consumer-market refresh,
  unsliced). At 100k 32× the p99 falls from 36.5 to 33.8 ms and the p99.9 from 78.5 to 76.2 ms,
  but frames over 16.7 ms rise from 662 to 984: the census now spreads over about 300 slots of
  the window instead of about 60, adding about 3 ms to more frames whose ordinary cost (about
  17 ms, the agent tick, `K4`) is already at the threshold. The maxima are the hour's planning
  slots at every tier (`K5`); the 30k 1× and 32× maxima (20.9 and 25.5 ms against 19.5 and
  23.1) are the 01:00 residential planning frame in both builds, within the run-to-run
  variation the gate allows. Load time is unchanged (100k 18.5 s).
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the first commit's build
  as control: hour means 2.59–2.63, 7.74–7.78, 10.43–10.75 and 30.78–31.45 ms (control
  2.56–2.76, 7.66–7.68, 10.83–11.05 and 31.22–31.55); level, and the same six work-count limits
  fail.
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1;
  the control is the first commit's gate binary.
- **Tests:** 2,536 release and 2,536 debug library tests and the 4 growth integration tests
  pass; clippy adds nothing (the 59 warnings in `agents/data.rs` and `asset_document.rs`
  remain); rustfmt adds nothing in the touched files. The change does not touch the Godot
  bridge.

#### `SIM-02-K`, part 3 — clearance patches and census refresh slices (2026-10-20, third commit)

- **Clearance site publication.** The building allocator's terrain outbox merged every site
  change of a slot into one bounding box, so two or three removals kilometres apart refreshed,
  re-stamped and re-uploaded every render patch between them. The outbox now keeps one box per
  change; the publication marks each box's terrain (dirty render patches and the height-revision
  regions the cell overlay reads) and refreshes the ownership of the sorted union of the patches
  the boxes touch, each once, then bumps their payload generations. A patch's ownership is a
  function of the roads, the sites and the heightmap, so every touched patch gets the merged
  box's result, and the patches between keep their state and are no longer rebuilt or
  re-uploaded. O(boxes × patches a box touches); no gameplay change. Measured at 100k 32× over
  the `PERF-07` day (temporary timing of the publication, one run each, pinned; diagnostic, not
  acceptance):

  | Minute | Changes | Before: patches, ms | After: patches (merged box), ms |
  | --- | --- | --- | --- |
  | 300 | 2 | 110, 44.7 | 3 (110), 0.82 |
  | 0 | 3 | 132, 22.3 | 6 (132), 1.27 |
  | 660 | 3 | 77, 13.6 | 5 (77), 0.99 |
  | 240 | 2 | 63, 10.7 | under 0.3 ms |
  | 480 | 2 | 32, 9.9 | under 0.3 ms |

  Over the day 46 publications took over 0.3 ms (54 before), 25.2 ms and 99 patches in all; the
  merged boxes would have covered 559. The largest is 1.27 ms (six patches at about 0.2 ms
  each, mostly the road grading margins).
- **Consumer-market refresh in slices.** The refresh ran whole in one census slot (11.0–21.5 ms
  at 100k). It is now eight census steps over the same phases in the same order, with what the
  earlier phases pass on (the budget walk's running treasury and comparison, the revisions and
  the change flags) carried in the census progress:
  - start: the basket's categories, the network-scale walking proofs and component indices
    after a network change, the access rows' length; O(profile ports + B / workers);
  - household budgets in household ranges, serial in settlement order
    (`visit_consumer_budgets_in`, the single walk's ranges in order);
  - access rows in household ranges, in parallel;
  - after a change of household count, bucket membership or category count: the index entries
    in household ranges; the index build, each mode's index once, its entries sorted in parallel
    first (sorted output is unique, so `from_entries` builds the same index); copies of the
    first category's indices to the others (every category has the same membership, so a copy
    is the index its own build gives, where the old code built K × 2 identical indices); the
    reservation positions in household ranges;
  - finish: whether the book is retained, the claims' reset otherwise, the receipt storage.
  A refresh begun and not finished leaves the book torn (rows partly rewritten); beginning
  another withdraws it first, so a census restarted inside the refresh reports the book not
  retained, as after a load. The census's results are a function of the copy and the network
  alone, so a withdrawn book changes no result (the restart test below checks it with a book
  the reference retains).
- **Setup in slices.** Setup's first pass (customer port offsets, funding-phase counts and five
  running port counts) and second pass (counted buyers, supplier order, provisional shares) run
  in building ranges with the counts carried; the import plans' layout and the end (port-sized
  books, household-quote preparation, closure rows) run whole between and after them.
- **Slot plan per step.** R, the work the census plans for, is now the previous census's work per
  step less this census's in that step, summed, or the rows' estimate if more. With one total, a
  step that took more than before (the first census after loading's staffing sweeps) used up the
  plan of every later step.
- **First census after loading.** Loading gives the census's own book the walking proofs and
  component indices the loaded census built (`ConsumerMarket::adopt_walking_proofs`, only into a
  book that has none): an uninterrupted game's census storage, the previous published census,
  holds them. Both are functions of the network alone, keyed by its revisions; the first window's
  refresh start fell from 8.2 to 1.8 ms at 100k and its window total from 143.8 to 136.9 ms.
- **Census cost per slot** (`perf07_census_slot_costs`, the ignored release diagnostic of part 3's
  second commit, two consecutive windows of the prepared `PERF-07` city, `RAYON_NUM_THREADS=8`,
  pinned; window 0 is the first after loading; diagnostic timing). Before is part 3's second
  commit:

  | Tier | Window | Total ms | Max slot ms | Slots > 1 ms | Last slot with work |
  | --- | --- | --- | --- | --- | --- |
  | 100k | 0 | 143.8 → 136.9 | 21.5 → 12.3 | 19 → 25 | 936 → 938 |
  | 100k | 1 | 101.3 → 90.5 | 11.0 → 5.4 | 8 → 9 | 741 → 749 |
  | 30k | 0 | 30.8 → 29.8 | 6.1 → 3.4 | 4 → 3 | 932 → 935 |
  | 30k | 1 | 20.0 → 20.4 | 2.6 → 2.9 | 3 → 3 | 742 → 741 |

  The slowest slot of each step at 100k, window 0 / window 1, before → after:

  | Step | Before ms | After ms |
  | --- | --- | --- |
  | consumer-market refresh | 21.5 / 11.0 (whole) | 1.8 / 2.7 (its slowest phase: start / finish); index build 1.8 / 1.9, copies 1.2 / 1.3, sliced phases under 1 |
  | setup | 4.2 / 4.4 | 1.3 / 1.7 (port counts / import plans' layout) |
  | route quotes | 10.3 / 1.5 | 10.2 / 1.5 (unchanged, whole) |
  | business seeding | 5.1 / 5.7 | 5.2 / 5.4 (unchanged, whole) |
  | input-source reset | 3.5 / 5.2 | 3.5 / 4.3 (unchanged, whole) |
  | repair | 2.0 / 3.8 | 1.8 / 3.2 (unchanged, whole) |

  Every slot over 2 ms is now a whole step outside the refresh and setup, except the refresh's
  finish (2.7 ms, the claims' reset of a book not retained) and window 0's staffing catch-up at
  slots 926–928 (2.2 ms, 2.8 before): that census's `current_staff` did 68 ms against the
  previous census's 37 ms, more than its own step's plan. More slots exceed 1 ms in window 0
  (25 against 19) because the refresh's whole phases (index build, copies, finish, start) each
  take a slot of 1.2–1.8 ms where the refresh took one of 21.5.
- **Equivalence.** Two removals far apart refresh the union of their patches; refreshing the box
  merged over both afterwards changes no ownership and bumps no generation, and every patch
  between keeps its ownership, payload generation and render state
  (`site_changes_far_apart_refresh_only_the_patches_they_touch`). A census restarted after any
  slice of 1 work unit, inside the refresh's household phases too, equals one run bit for bit,
  with an unchanged income (the reference retains its book) and a changed one, and some restart
  lands inside the refresh (`a_census_restarted_after_any_slice_equals_one_run_bit_for_bit`).
  The existing slice, restart, window and save/load tests pass.
- **Envelope and determinism** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass after each change;
  the envelope is bit-identical to the second commit's, and repeat, 1 and 3 workers and
  save/reload are identical.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`), full day, each build
  on its own prepared cities, in one session with the second commit as the control. Two rows
  are a rerun of both builds after a background process disturbed the first run: 10k 1× and
  100k 32× (the first full run measured 113.3 / 46.12 / 84.76 / 20.95 ms at 100k 32× for this
  build, the control 81.9 / 33.88 / 77.78 / 16.94, with the untouched render snapshot 19%
  slower); each build reran on the city it had prepared. Milliseconds and
  frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× (rerun) | 9.4 (9.2) | 1.24 (1.25) | 1.48 (1.51) | 0.54 (0.54) | 2 (2) | 0 (0) |
  | 10k | 32× | 10.0 (10.7) | 2.29 (2.30) | 4.96 (4.96) | 1.05 (1.06) | 2 (2) | 0 (0) |
  | 30k | 1× | 20.1 (19.9) | 2.25 (2.30) | 3.35 (3.55) | 1.30 (1.33) | 4 (4) | 2 (2) |
  | 30k | 32× | 23.6 (25.8) | 7.33 (7.71) | 23.26 (23.03) | 3.34 (3.33) | 21 (25) | 3 (3) |
  | 100k | 1× | 59.1 (62.2) | 8.27 (8.43) | 16.24 (16.48) | 6.10 (6.11) | 1,306 (1,629) | 76 (82) |
  | 100k | 32× (rerun) | 82.3 (80.5) | 33.52 (33.99) | 59.75 (79.74) | 16.96 (16.89) | 2,700 (2,700) | 1,042 (965) |

  Every gate limit passes for both builds. At 100k 1× frames over 8 ms fall from 1,629 to 1,306
  and the control's frame at minute 300 (29.1 ms, the clearance site publication) is gone. At 100k 32× the p99.9
  falls from 79.7 to 59.8 ms: the control's minute-0 and minute-300 frames (79.7 and 67.5 ms,
  site publications of 22 and 45 ms) are gone; frames over 16.7 ms rise from 965 to 1,042, as the
  sliced refresh and setup add their work to more frames whose ordinary cost (about 17 ms, the
  agent tick, `K4`) is already at the threshold. The maxima are the hour's planning slots at
  every tier (`K5`: minutes 60 and 120); the 30k 1× and 10k maxima are the same planning frames
  in both builds, within the run-to-run variation. Load time is unchanged (100k 18.6 s).
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the second commit's
  build as control: hour means 2.53–2.61, 7.78–7.85, 10.57–10.63 and 31.21–31.32 ms (control
  2.59–2.60, 7.73–7.82, 10.49–10.60 and 31.02–31.49); level, and the same six work-count limits
  fail.
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1;
  the `PERF-07` control is the second commit's gate binary.
- **Tests:** 2,538 release and 2,538 debug library tests and the 4 growth integration tests pass
  (2,537 after the first change); clippy adds nothing (the 59 warnings in `agents/data.rs` and
  `asset_document.rs` remain); rustfmt adds nothing in the touched lines. The site publication feeds the terrain payloads the
  bridge reads, so the isolated `./run.sh --test` ran: 2,538 debug library tests, the 4 growth
  tests and every bridge, asset and shader test pass, with no script errors.

#### `SIM-02-K`, part 3 — the census's whole steps in slices (2026-10-20, fourth commit)

- **What changed.** Every census step that ran whole and cost over 1.5 ms at 100k in the third
  commit, except repair, is split into phases that keep their order and results, with what the
  earlier phases pass on carried in the census progress ([Pure jobs](#pure-jobs)):
  - **Route quotes.** The start (the route cache's revisions and storage), the border sweeps,
    the door access and the quotes (`ShipmentSystem::begin_border_carrier_quotes`,
    `sweep_carrier_borders`, `prepare_border_carrier_access`, `write_border_carrier_quotes_in`).
    Each building's access and quote depend on its own entrance and the prepared border
    minimum alone, so ranges give the whole call's rows. The census names its quotes current
    (`quote_revision`) only after the last range. The route cache's new and cleared tables are
    filled in parallel (`resize_in_parallel`, `clear`), so a first growth's page faults spread
    over the workers.
  - **Consumer-market refresh.** The index build one mode a slice and the copies one category
    a slice; the finish as its decision, the claims' accounts in household ranges, each of the
    2 × K indices' availability and its end (`begin_refresh_finish`, `reset_refresh_claims`,
    `reset_refresh_availability`, `end_refresh`). A retained book skips to the end.
  - **Input-source reset** (`RecurringSupply::begin_reset`, `reset_layout`, `reset_entries`,
    `build_reset_index`, `reset_rates`). The supplier index's entries are sorted once and split
    into one run per resource; each resource's index is built from its own sorted run, which is
    the index `from_entries` gives for that resource's entries in any order (sorted output is
    unique). The book names no layout from the start of a rebuild until its last resource is
    built, so a reset begun again after a restart rebuilds it. The rates and availability run
    in source ranges: a source's rate and its availability toggle depend on that source alone,
    and the toggles keep source order.
  - **Business seeding.** The start (`begin_business_seeding`, the accounts' reset), the buyers
    in ranges (`seed_business_rows`, serial in buyer order, the source receipts linking buyers
    in the same order) and the book's layout (`BusinessMarket::begin_layout`, `layout_access`,
    `layout_ports`, `collect_layout_entries`, `build_layout_indices`, `finish_layout`). The
    layout was rebuilt by most censuses, since appended buildings change the buyer profiles.
    A rebuild clears the layout's name at its start and writes it last, so propagation after a
    restart sees no layout and lays one out itself, as before.
  - **Cleared storage grows without copying** (`utils::storage::reserve_cleared`). The census
    storage and the live books swap at publication, and planning reserves the live input book's
    receipt arena at about 1.4 million receipts at 100k, so a cleared arena, withdrawal list or
    consumer receipt book often needs more capacity than it holds. `Vec::reserve` then
    reallocated and copied the whole old allocation although no row survived (the consumer
    receipt book's reserve at the refresh's end took 1.8 ms at 100k, from 363,540 to 727,080
    receipts' capacity with none kept). An empty vector now takes fresh storage of the
    capacity `reserve` would give.
  - **Planning.** `CensusProgress::remaining_work` no longer claims to bound the work left: it
    counts steps the census may skip once it reaches them, and later sweeps' searches it does
    not count. The driver takes the larger of it and the previous census's work per step, so
    either error moves work between slots, never a result.
- **Census cost per slot** (`perf07_census_slot_costs`, two consecutive windows of the prepared
  `PERF-07` city, `RAYON_NUM_THREADS=8`, pinned; window 0 is the first after loading;
  diagnostic timing). Before is the third commit's binary, run in the same session:

  | Tier | Window | Total ms | Max slot ms | Slots > 1 ms | Last slot with work |
  | --- | --- | --- | --- | --- | --- |
  | 100k | 0 | 137.9 → 137.7 | 12.7 → 3.9 | 27 → 31 | 938 → 935 |
  | 100k | 1 | 90.9 → 88.0 | 5.4 → 2.2 | 9 → 9 | 749 → 760 |
  | 30k | 0 | 33.4 → 30.4 | 4.9 → 1.6 | 3 → 1 | 935 → 931 |
  | 30k | 1 | 19.1 → 18.7 | 2.7 → 0.6 | 2 → 0 | 741 → 753 |

  The slowest slot of each step at 100k, window 0 / window 1, before → after ("—" not among
  the window's 12 slowest slots, which end at 1.31 and 0.87 ms after the change; "under 1.3"
  a slot below the window 0 cut-off):

  | Step | Before ms | After ms |
  | --- | --- | --- |
  | route quotes | 10.9 / 1.3 | 2.1 / 0.8 (the start; the sweeps, access and quotes —) |
  | business seeding | 5.3 / 5.4 | under 1.3 / 1.4 (the start); accounts 1.0, layout indices 0.9 |
  | input-source reset | 3.3 / 4.1 | under 1.3 / 1.0 (the index entries); rates 0.8 |
  | refresh finish | 1.6 / 2.5 | 0.4 / 0.3 (the end / the availability) |
  | refresh index build | 1.5 / 1.8 | 1.35 / 1.34 (one mode) |
  | refresh copies | — / 1.2 | — / — |
  | repair (unsliced) | 1.7 / 3.9 | under 1.3 / 2.2 |

  At 30k every census slot is under 1 ms except the window's first in window 0 (1.6 ms: the
  route quotes' start 0.9 ms after loading, and the census start outside the steps).
  More slots exceed 1 ms in 100k window 0 (31 against 27): the staffing catch-up at slots
  910–930 (1.3–1.8 ms, unchanged; that census's `current_staff` does 68 ms against the
  previous census's 37) now holds most of them.
- **What is left above 1 ms at 100k** (findings and options in
  [Decided while unattended](#decided-while-unattended)): repair 2.2 ms in window 1, mostly
  its storage preparation; setup's import-plan layout 1.7; each customer index mode's build
  1.35; business seeding's start 1.4 (the accounts' reset); the refresh's start 1.8 in window 0
  only; the route cache's first growth after loading 2.1 in window 0 only; window 0's staffing
  catch-up; and the window's first slot also spends 0.3–1.8 ms outside the census steps (the
  census start's frontage price publication and input-copy swap, not attributed further).
- **Equivalence.** A census restarted after any slice of 1 work unit equals one run bit for bit,
  with an unchanged income (the reference retains its book), a changed one and now on a cold
  book, where the census rewrites its route quotes, builds its supplier index and lays out its
  business book; the test checks that restarts land inside the route access and quotes, the
  customer index build and availability reset, the supplier layout, index and rates, the
  buyer seeding and the business layout's access, ports and indices
  (`a_census_restarted_after_any_slice_equals_one_run_bit_for_bit`). The existing slice,
  restart, window and save/load tests pass.
- **Envelope and determinism** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass after each change;
  the envelope is bit-identical to the third commit's, and repeat, 1 and 3 workers and
  save/reload are identical.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the third commit's build
  as control: hour means 2.55–2.58, 7.75–8.23, 10.41–10.64 and 31.20–31.83 ms (control 2.53–2.61,
  7.63–7.87, 10.62–10.63 and 31.55–31.62); the same six work-count limits fail. The 30k census's
  day total rose about 4% (589–592 against 564–572 ms, 8,674 calls against 8,420): the extra
  slice starts of the newly sliced steps. Its worst slot fell, which is what this commit is for.
- **`PERF-07`**: `tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline`, `RAYON_NUM_THREADS=8`), full day, in one
  session with the third commit as the control; the save format is unchanged, so this build ran
  on the cities the control prepared. Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 9.4 (8.9) | 1.21 (1.25) | 1.44 (1.49) | 0.54 (0.54) | 2 (1) | 0 (0) |
  | 10k | 32× | 10.0 (10.7) | 2.16 (2.29) | 4.78 (5.22) | 1.02 (1.05) | 2 (2) | 0 (0) |
  | 30k | 1× | 20.3 (20.1) | 2.25 (2.28) | 3.38 (3.41) | 1.29 (1.30) | 4 (4) | 1 (2) |
  | 30k | 32× | 23.6 (23.9) | 7.22 (7.57) | 22.97 (22.01) | 3.28 (3.35) | 22 (24) | 3 (3) |
  | 100k | 1× | 57.2 (57.3) | 8.21 (8.30) | 16.19 (16.32) | 6.06 (6.09) | 1,215 (1,402) | 73 (75) |
  | 100k | 32× | 81.3 (83.9) | 33.25 (33.25) | 58.74 (63.49) | 16.85 (16.94) | 2,700 (2,700) | 1,005 (1,018) |

  Every gate limit passes for both builds. At 100k 1× frames over 8 ms fall from 1,402 to
  1,215; at 100k 32× the p99.9 falls from 63.5 to 58.7 ms and frames over 16.7 ms from 1,018 to
  1,005. No census-window frame is among either build's 12 slowest at 100k; those are the
  hour's planning slots (`K5`: minutes 60 and 120, the maxima at every tier), the day's first
  frame and other hourly frames at 32×, and the 1× frames at minutes 300–312, 787 and 1260
  (not census work). The 10k 1× maximum (9.4 against 8.9 ms) is the same 01:00 planning frame
  in both builds, within the run-to-run variation the gate allows. Load time is unchanged
  (100k 18.5 s).
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1;
  the `PERF-07` control is the third commit's gate binary.
- **Tests:** 2,538 release and 2,538 debug library tests and the 4 growth integration tests pass
  after each change; clippy adds nothing after the second change (the first left three
  dead-code warnings on the whole-call forms only tests now use, which the second marks test
  only; the 59 warnings in `agents/data.rs` and `asset_document.rs` remain); rustfmt adds
  nothing in the touched lines. The change does not touch the Godot bridge.

#### `SIM-02-K`, part 4 — agent tick (2026-10-20, first commit)

- **Measured first** (temporary per-phase timing of the agent tick and of every slot system,
  never committed; the prepared `PERF-07` cities, `RAYON_NUM_THREADS=8`, pinned to the
  performance cores; 1× over the day's first 180 game minutes, 32× over the whole day; means
  over ordinary frames, those without an hour slot; diagnostic, not acceptance). Part 3's
  build, in ms per frame:

  | Phase | 30k 1× | 30k 32× | 100k 1× | 100k 32× |
  | --- | --- | --- | --- | --- |
  | **Agent tick** | **0.73** | **1.69** | **4.67** | **9.62** |
  | · tick-start bucket rebuild and claim reset | 0.03 | 0.24 | 0.26 | 1.18 |
  | · IDM speed | 0.07 | 0.10 | 0.36 | 0.42 |
  | · claim classification | 0.11 | 0.16 | 0.54 | 0.57 |
  | · parallel movement | 0.24 | 0.54 | 1.75 | 3.63 |
  | · serial Dynamic movement | 0.03 | 0.24 | 0.20 | 1.81 |
  | · end-of-tick bucket rebuild and congestion | 0.25 | 0.31 | 1.54 | 1.62 |
  | · frontage pass | 0.00 | 0.10 | 0.01 | 0.38 |
  | Render snapshot | 0.31 | 0.58 | 0.81 | 0.91 |
  | Slot work | 0.04 | 0.95 | 0.25 | 5.92 |
  | Frame | 1.11 | 3.26 | 5.78 | 16.52 |

  The slowest movement shard ran at 1.64–1.71× the shard mean at every tier and speed; the
  serial Dynamic set held 128 / 893 / 568 / 4,400 agents.
- **Lane buckets kept across appends and swap-removes.** The retained occupancy
  snapshot was rebuilt from every agent at the start of any tick after an agent was appended or
  swap-removed, which at 32× is every frame. Now:
  - an appended agent is read at the next tick start, after its spawn call has placed it
    (a carrier's plan, a border start); O(appended × bucket);
  - a swap-remove patches the snapshot before the removal: the removed agent's entries go, and
    the last agent's are renamed to its new index, or added if it was appended after the
    snapshot; O(entries on their lanes);
  - the household carrier, relocation and eviction mutators, which move an agent into a
    building or onto an access leg, remove that agent's entries instead of invalidating;
  - every other mutator of lane state between ticks still invalidates (building-reference
    scrub, evacuation, road edits and reattachment, undo, load), and a patch that cannot find
    an entry invalidates.

  Entries are found from the agents' current lane state, which only the tick and the
  invalidating mutators change (audited: the freight route changes move a carrier between
  states that hold no entry). Test builds compare every patched snapshot with a rebuild from
  every agent at the next tick, entry for entry, with the dirty-lane list and flags; timing
  runs skip it with the other whole-city checks. A full rebuild at the tick start now happens
  in 0.1–2.5% of frames (100k 1× and 32×).
- **The end-of-tick fill on the pool.** One pass over 4,096-agent
  chunks groups each chunk's lane agents by lane range; then each of 2 × workers contiguous lane
  ranges clears its previous entries, fills its lanes chunk by chunk in agent order, sorts them
  and corrects overlaps; ghosts and overlap write-backs are serial and O(their count). O(n + L)
  work over the workers, no allocation once the retained groups have grown. Two rules make the
  snapshot the function of the agents' state that a rebuild gives, which keeping it across ticks
  needs:
  - after an overlap clamp ties a rear car with a leader of lower index (both at the lane
    start), the bucket is sorted again; before, the tie kept the pre-clamp order until the next
    rebuild, so the order of tied cars depended on whether an agent was appended or removed in
    that frame;
  - lane-change ghosts are taken after the correction; before, the candidates were taken from
    the uncorrected distances.

  The test-build check found the first in the bridge suite's growing town at 32× (border
  arrivals queued at a lane start); the envelope is bit-identical, so neither changed an
  envelope outcome. The edge congestion sums
  stay a serial pass in agent order (0.32–0.35 ms at 100k), so the `f32` sums are unchanged; the
  claim reset runs in 4,096-lane chunks from 65,536 lanes.
- **Movement shards interleaved.** Each of the worker-count shards takes every
  shards-th chunk of 256 agents in index order with its own route scratch, instead of one
  contiguous range: moving agents cluster in index ranges. The slowest shard falls from
  1.64–1.71× to 1.11–1.38× the mean. A scratch only memoizes route storage, so results do not
  depend on the layout, and its search counts stay repeatable for a worker count.
- **After** (same measurement, this commit), ms per frame, part 3 in parentheses:

  | Phase | 30k 1× | 30k 32× | 100k 1× | 100k 32× |
  | --- | --- | --- | --- | --- |
  | **Agent tick** | **0.56** (0.73) | **1.35** (1.69) | **3.24** (4.67) | **7.14** (9.62) |
  | · tick-start bucket work and claim reset | 0.01 (0.03) | 0.02 (0.24) | 0.03 (0.26) | 0.05 (1.18) |
  | · IDM speed | 0.06 (0.07) | 0.11 (0.10) | 0.31 (0.36) | 0.39 (0.42) |
  | · claim classification | 0.08 (0.11) | 0.16 (0.16) | 0.51 (0.54) | 0.54 (0.57) |
  | · parallel movement | 0.19 (0.24) | 0.48 (0.54) | 1.26 (1.75) | 2.86 (3.63) |
  | · serial Dynamic movement | 0.03 (0.03) | 0.24 (0.24) | 0.21 (0.20) | 1.89 (1.81) |
  | · end-of-tick fill and congestion | 0.18 (0.25) | 0.24 (0.31) | 0.90 (1.54) | 1.01 (1.62) |
  | · · of it, the congestion pass | 0.07 | 0.08 | 0.32 | 0.34 |
  | · frontage pass | 0.00 (0.00) | 0.10 (0.10) | 0.01 (0.01) | 0.39 (0.38) |
  | Render snapshot | 0.26 (0.31) | 0.55 (0.58) | 0.77 (0.81) | 0.90 (0.91) |
  | Slot work | 0.04 (0.04) | 0.94 (0.95) | 0.25 (0.25) | 5.96 (5.92) |
  | Frame | 0.87 (1.11) | 2.90 (3.26) | 4.31 (5.78) | 14.07 (16.52) |
  | Slowest shard / mean | 1.18 (1.64) | 1.38 (1.71) | 1.11 (1.71) | 1.24 (1.68) |

  The plan estimated about 3 ms at 1× and 7 ms at 32× at 100k for all of part 4; these changes
  save 1.4 and 2.5 ms. The items not done are below.
- **Not done** (findings and options in [Decided while unattended](#decided-while-unattended)):
  `current_congestion` deleted (0.32–0.35 ms at 100k: the serial congestion pass above), indoor
  agents off the per-frame movement pass, fewer agents in the serial handoff set, cheaper pose
  sampling, and an index so the per-agent passes skip agents that need nothing.
- **Equivalence:** `lane_buckets_kept_across_appends_and_swap_removes_equal_a_rebuild` (a
  covered agent leaving, an appended agent taking a covered slot, the last agent leaving, an
  append read at the tick start; the snapshot equals a rebuild and six ticks after it move every
  agent as from a rebuilt one, bit for bit) and
  `parallel_lane_bucket_fill_is_independent_of_the_worker_count` (1,500 agents with overlapping
  cars on two lanes: buckets, dirty-lane order and agent state equal with 1, 3 and 8 workers,
  through three ticks of parallel movement); the dispatch test now covers chunk boundaries. The
  test-build snapshot check ran in every library test, the envelope and the determinism runs.
- **Envelope and determinism** (`tools/sim02_gate.py envelope determinism --bursts
  departures,settlements --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass; the
  envelope is bit-identical to part 3's last commit, and repeat, 1 and 3 workers and
  save/reload show no differences.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`), full day, both builds on
  copies of the same prepared cities (save format unchanged), in one session with part 3 as the
  control. Milliseconds and frame counts, control in parentheses:

  | Tier | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms |
  | --- | --- | --- | --- | --- | --- | --- |
  | 10k 1× | 9.5 (9.1) | 1.19 (1.24) | 1.44 (1.47) | 0.51 (0.54) | 1 (1) | 0 (0) |
  | 10k 32× | 11.3 (10.6) | 2.16 (2.23) | 4.72 (5.07) | 0.96 (1.03) | 2 (2) | 0 (0) |
  | 30k 1× | 19.1 (19.9) | 2.03 (2.25) | 2.91 (3.38) | 1.05 (1.29) | 4 (4) | 1 (1) |
  | 30k 32× | 23.3 (22.1) | 6.38 (6.64) | 22.29 (21.34) | 3.00 (3.31) | 21 (20) | 3 (3) |
  | 100k 1× | 54.7 (59.5) | 6.68 (8.19) | 11.48 (16.15) | 4.56 (6.03) | 298 (1,171) | 7 (73) |
  | 100k 32× | 77.9 (80.8) | 30.86 (32.99) | 59.04 (58.66) | 14.31 (16.95) | 2,700 (2,700) | 383 (1,040) |

  Every gate limit passes for both builds; load time is unchanged (100k 18.5–18.6 s). The
  means fall 6–8% at 10k, 9–19% at 30k and 16–24% at 100k; at 100k 1× frames over 8 ms fall
  from 1,171 to 298 and the p99 is 6.7 ms (8.2), and at 100k 32× frames over 16.7 ms from 1,040
  to 383. The maxima are the hour's planning slots at every tier (minutes 60 and 120, `K5`), the
  same frames in both builds within the run-to-run variation; the 100k 32× worst frames carry
  6.2–10.7 ms of agent tick (control 8.8–10.0). Control: the gate binary of part 3's last
  commit; this build ran right after it.

- **`PERF-06`** (`benchmark_agent_tick_contract`, three alternating process pairs with part 3,
  pinned, `RAYON_NUM_THREADS=8`, `METRUM_DEBUG=0`): every work count is identical to part 3's and
  grows exactly 3× from 10k to 30k (lane counts 16). The single timed tick per tier, median
  [range] ms, part 3 then part 4: network 0.21 [0.21–0.56] / 0.46 [0.25–0.87] at 10k and 0.65
  [0.52–0.70] / 0.52 [0.47–0.68] at 30k; egress 0.49 [0.44–0.61] / 0.55 [0.48–1.06] and 1.33
  [1.26–1.72] / 1.40 [1.23–1.42]; ingress 0.47 [0.22–0.49] / 0.27 [0.25–0.47] and 0.44
  [0.42–0.59] / 0.50 [0.29–1.02]. The ranges overlap; these are routine regression signals, not
  a speedup claim. The contract's fixture puts every agent on one lane, the parallel fill's
  worst case (one range holds all entries), which may cost a few hundredths of a ms at 10k.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with part 3's last commit as
  control: hour means 2.45–2.54, 7.55–7.67, 10.64–10.86 and 31.72–31.77 ms (control 2.54–2.56,
  7.60–7.63, 10.33–10.43 and 31.24–31.80); level (the contract times economy slot work, which
  this commit does not touch), and the same six work-count limits fail.
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1;
  the per-phase tables on temporary timing builds of the same sources.
- **Tests:** 2,540 release and 2,540 debug library tests and the 4 growth integration tests pass;
  clippy adds nothing (the 59 warnings in `agents/data.rs` and
  `asset_document.rs` remain); rustfmt adds nothing in the touched lines. The step changes no
  code under `rust/src/nodes/` or GDScript; the isolated `./run.sh --test` ran anyway, since a
  re-sorted tie can change a later movement: the 2,540 debug library tests, the 4 growth tests
  and every headless bridge, asset and shader test pass with no script errors (run without a
  display, so the rendered vegetation tests skip).
- **Frame budget at 32×** (means over ordinary frames, after part 4; the maximum adds the
  hour's planning slots, `K5`, and the departure waves):

  | Tier | Agent tick | Snapshot | Slot work | Other | Frame | `PERF-07` max |
  | --- | --- | --- | --- | --- | --- | --- |
  | 30k 32× | 1.35 | 0.55 | 0.94 | 0.05 | 2.90 | 23.3 |
  | 100k 32× | 7.14 | 0.90 | 5.96 | 0.07 | 14.07 | 77.9 |

  Slot work at 100k 32× per ordinary frame (8.5 slots): restock requests 1.93, trip events
  1.71, the census 1.00, building hours 0.25, household hours 0.24, freight requests 0.23, job
  requests 0.21, agent hours 0.09, the demand snapshot 0.07, the rest under 0.05 each. At 30k
  32×: trip events 0.25, the census 0.24, restock requests 0.19, building and household hours
  0.06 each.

  - **30k 32×** has room: an ordinary frame is 2.9 ms. Its maximum (about 24 ms) is the hour's
    planning slots (`K5`), then the departure waves; 8 ms needs `K5` and the slots' worst
    single items (census and queue slices) under about 4 ms in a frame.
  - **100k 32×** does not: the mean ordinary frame alone is 14 ms, before any hour, wave or
    census peak. For a worst frame of 8 ms at 8.5 slots a frame, every part would need about
    half of today's mean at once: agent tick about 3.5 ms (today 7.1), slot work about 3.5 ms
    (5.96), snapshot 0.9 ms, and no frame's slot or departure wave above the mean by more than
    a few ms. The remaining agent-tick items save at most about 2–3 ms (below), so the agent
    tick stays near 4.5–5 ms; restock planning (1.9 ms, serial CCH point searches) and trip
    events (1.7 ms, about 1,500 serial events at 1.1 µs) are per-entity work proportional to
    the simulated time a frame carries, not boundary work that spreading removes. About 45,000
    of 145,000 agents are always moving in the 100k fixture (`K6`); in a city whose trips do
    not stall, the moving set and the per-frame movement work would be smaller.
  - **Conclusion:** 100k 32× cannot reach 8 ms without changing a spec decision. The options
    are in [Decided while unattended](#decided-while-unattended) and
    [Open questions](#open-questions), item 6. 30k at both speeds and 100k 1×
    (4.3 ms ordinary frame, p99 6.7 ms, 298 frames over 8 ms) are within reach of `K5` and the
    remaining slot and departure-wave peaks.

#### `SIM-02-K`, part 5 — planning slots (2026-10-21, first commit)

- **Attribution first.** Every planning slot of the `PERF-07` day was timed on the part 4 build
  by system and by planning phase (the `economy_phases` channel and temporary timing points, one
  pinned run per tier, diagnostic). The scans the plan named cost 0.1–1.5 ms a slot at 100k: the
  lot scan 0.3–1.0, the live workforce count 0.45–0.5, the zone's existing buildings 0.5–0.7,
  slot 3's three downgrade scans 1.45 together, the workforce calendar's reset 0.45–0.5. The
  maxima were something else: the residential slot of the first three hours after loading (100k
  49.1, 41.9 and 31.5 ms; 30k 16.6, 14.3 and 6.0 ms), whose cost was the planning storage and the
  supplier and buyer site rows built on books that loading left cold. The census storage and the
  live books swap every hour, so each of the two books was cold once, and each grew its storage
  again at its next use. Part 5 therefore took those costs first (see
  [Decided while unattended](#decided-while-unattended)).
- **What changed** (first commit):
  1. **Storage estimates count free lots.** Planning's warm-start storage estimates sized every
     arena as if every parcel opened a project with the largest recipe's ports (owners =
     buildings + 2 × parcels). New projects open only on free lots, so the estimates now count
     two owner slots per free lot; the owner index space is unchanged. A shortfall still grows
     exactly through the proposal replay (`demand/supply/storage.rs`), so an estimate never
     changes a result. Counting free lots is O(P/p), about 0.02–0.05 ms at 100k.
  2. **One downgrade scan in slot 3.** Slot 3 scanned every building once per zone; one scan now
     collects all three zones' candidates, each zone's list in the order its own scan gives (the
     attachment order is total; test builds compare each list with the zone's own scan).
  3. **Constructing buildings found in parallel.** The project market's two serial scans of every
     building for constructing ones take one parallel filter in building order, and both loops
     visit only those.
  4. **Planning books prepared outside the planning slots.** Saves keep none of what a planning
     slot prepares on the books it lends: the border carrier tables, the supplier and buyer site
     rows and their layouts, the pass's owner rows and the books' planning storage. Loading now
     prepares the live books (`DemandSystem::prepare_planning_books`), and a finished census
     whose book no planning pass has prepared (the first after loading, on fresh storage;
     `JobFunding::planning_prepared`) is prepared for the hour it serves in the next window slot.
     Since the sixth commit every census start clears its book's flag, so this runs every hour,
     in two halves, at slot 944 (minute 59's first slot, the supplier rows) and from slot 953
     (the rest, on kept pass rows), while the zone index is current.
     The books are lent and restored exactly as a planning slot does (test builds compare their
     fingerprints), and what is prepared is derived and keyed by the city's revisions.
- **Planning slot costs** (diagnostic; `PERF-07` city, 1×, `RAYON_NUM_THREADS=8`, pinned, one run
  per build). The residential slot of the first three hours after loading, from the
  `economy_phases` channel over the full day (ms):

  | Tier | Hour | Part 4 | Part 5 |
  | --- | --- | --- | --- |
  | 30k | 1 | 16.6 | 1.5 |
  | 30k | 2 | 14.3 | 4.3 |
  | 30k | 3 | 6.0 | 3.6 |
  | 100k | 1 | 49.1 | 5.5 |
  | 100k | 2 | 41.9 | 21.7 |
  | 100k | 3 | 31.5 | 22.0 |

  The census book's preparation (temporary per-system timing, 200-minute runs): one window slot
  at 00:58, 11.5 ms at 30k (frame 13.1 ms) and 34.5 ms at 100k (frame 38.7 ms). The same runs
  give each step's share at 100k: the free-lot estimates alone took hour 1 from 48.9 to 43.5 ms
  and hour 2 from 39.9 to 30.4; slot 3 fell from 8.1–8.3 to 6.7–6.8 ms; the commercial and
  industrial slots from 3.8–4.7 to 3.7–4.4 ms.

  Steady hours (hour 4 on, `economy_phases`, full day, slot means / maxima in ms):

  | Tier | Slot | Part 4 | Part 5 |
  | --- | --- | --- | --- |
  | 30k | residential | 2.93 / 11.51 | 2.83 / 7.50 |
  | 30k | commercial | 0.97 / 1.14 | 0.99 / 1.14 |
  | 30k | industrial | 1.33 / 2.13 | 1.33 / 2.08 |
  | 30k | actions | 1.57 / 1.93 | 1.35 / 1.76 |
  | 100k | residential | 9.25 / 10.18 | 8.88 / 9.81 |
  | 100k | commercial | 4.19 / 4.89 | 3.93 / 4.61 |
  | 100k | industrial | 4.75 / 5.13 | 4.57 / 6.24 |
  | 100k | actions | 8.53 / 8.79 | 7.03 / 7.63 |

  The project market's constructing scan fell from 0.67 to 0.32 ms a slot at 100k and slot 3's
  downgrade scans from 1.47 to 0.55 ms (30k 0.42 to 0.17). The 30k residential maximum in steady
  hours is 06:00, a storage growth of the same kind as the post-loading hours (part 4 11.5 ms,
  part 5 7.5 ms); the 100k industrial maximum (6.2 ms at 07:00) is one existing-buildings scan of
  1.9 ms, once in the day.
- **What is left in the planning slots** (findings and options in
  [Decided while unattended](#decided-while-unattended)):
  - (the second commit removed the census book's preparation after loading and the business
    storage's second growth; see its results section);
  - every slot's own setup, each an O(B/p) pass at about 0.3–0.8 ms at 100k: the pass's owner
    rows (`prepare_pass_rows`, 0.77 ms, twice in slot 3), the live workforce count (0.45–0.5),
    the workforce calendar's reset (0.45–0.5), the zone's existing buildings (0.5–0.7), the lot
    scan (0.3–1.0) and the snapshot compose; about 4 ms a slot at 100k together, so the
    commercial and industrial slots cost about 4 ms and the residential slot about 9 ms in steady
    hours (it also merges the hour's site-row changes and lays the rows out again, about 3.5 ms).
- **Equivalence.** Envelope bit-identical to part 4's first commit (every `envelope` metric of
  the gate report equals part 4's); determinism shows no differences (repeat, save/reload, 1 and
  3 workers); burst limits pass. The new paths run in the gate: fresh cities prepare their first
  census books in a window slot, and the reload check prepares the loaded books. Test builds
  compare the merged downgrade lists with each zone's own scan and the prepared books'
  fingerprints with the lent ones.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one session
  with part 4 as the control, both on copies of the same prepared cities (the save format is
  unchanged). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 5.7 (9.1) | 1.21 (1.21) | 1.45 (1.50) | 0.51 (0.51) | 0 (2) | 0 (0) | 1.7 (1.8) |
  | 10k | 32× | 4.9 (10.5) | 2.16 (2.24) | 4.34 (5.43) | 0.96 (0.96) | 0 (2) | 0 (0) | 1.7 (1.8) |
  | 30k | 1× | 12.8 (20.0) | 2.09 (2.06) | 2.98 (3.01) | 1.07 (1.04) | 3 (4) | 0 (1) | 4.9 (4.9) |
  | 30k | 32× | 17.9 (22.8) | 6.21 (5.96) | 13.07 (21.69) | 2.96 (2.96) | 18 (19) | 1 (3) | 4.9 (4.9) |
  | 100k | 1× | 40.2 (55.6) | 6.76 (6.60) | 11.43 (11.39) | 4.63 (4.55) | 308 (292) | 4 (6) | 18.6 (18.5) |
  | 100k | 32× | 55.8 (74.6) | 30.04 (30.92) | 51.06 (55.01) | 14.39 (14.37) | 2,700 (2,700) | 394 (383) | 18.5 (18.5) |

  Every gate limit passes for both builds. 10k is now under 8 ms at both speeds (its maxima were
  the 01:00 and 02:00 planning frames; the worst left are agent-tick frames, 5.7 and 4.9 ms). The
  hour-after-loading planning frames are gone at every tier. What sets each maximum now: at 30k
  1× the census book's preparation at 00:58 (12.8 ms), then the 06:00 residential slot (9.5 ms,
  a storage growth); at 30k 32× the 06:00 frame (17.9 ms, slots 0–3 together) and the
  preparation (14.7); at 100k 1× the preparation (40.2 ms), the day's first frame (28.1 ms, the
  job and restock queues after loading, not planning) and the 02:00 and 03:00 residential slots
  (26.6 ms, the second storage growth) and agent-tick frames up to 15.9 ms; at 100k 32× the
  02:00 frame (55.8 ms), the day's first frame (53.2) and every hour's slots 0–3 together (about
  49–51 ms, about 23 ms of it the hour's slots). Means are within 3% of the control's and p99 within 5% (100k 1× mean
  +1.8%, p99 +2.4%, frames over 8 ms +5%; 30k 32× p99 +4%), inside the run-to-run variation the
  gate allows; apart from the first three hours the planning work fell slightly (steady table
  above). A 100k load takes 0.1 s longer (the live books' preparation).

- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with part 4's first commit as
  control: hour means 1.54–1.58, 5.66–5.67, 9.46–9.66 and 29.33–29.52 ms (control 2.50–2.53,
  7.64–8.27, 10.28–10.66 and 31.34–31.50). The baseline hours are about 35% cheaper, because
  the timed hours no longer grow cold planning storage, and the fiscal-stress `cch_searches`
  work-count limit now passes; the other five work-count limits fail as before.

- **Build identity:** rustc 1.99.1. `PERF-07` ran on the sources before a clippy-only change
  (`drop(SiteSupply::new(..))` written as a plain statement, which drops the value at the same
  point); the commit gate then reran the release library and growth tests, the envelope,
  determinism, clippy and `PERF-01` on the committed sources. The `PERF-07` control is part 4's
  retained binary.
- **Tests:** 2,540 release and 2,540 debug library tests and the 4 growth integration tests pass
  on the commit; clippy adds nothing (the 59 warnings in `agents/data.rs` and
  `asset_document.rs` remain); rustfmt adds nothing in the touched files. `./run.sh --test` in an
  isolated profile passes with no script errors (its 2,540 debug library tests included; the two
  Forward+ vegetation tests skip without a display); part 5 changes what loading prepares, which the bridge's load tests exercise.
- **Save format:** unchanged (103). Nothing new is saved: `planning_prepared` is a field of the
  `JobFunding` books, which loading rebuilds.

#### `SIM-02-K`, part 5 — post-load planning storage (2026-10-21, second commit)

- **Attribution first.** The first commit's build was traced over the first 400 game minutes at
  30k and 200 at 100k (pinned, `RAYON_NUM_THREADS=8`, diagnostic): each lent book's row lengths
  and capacities before and after every preparation, and each frame's systems. Two mechanisms set
  the planning maxima the first commit left:
  1. **The census book's preparation after loading** (one window slot at 00:58: 11.7 ms at 30k,
     36.5 ms at 100k). Loading leaves the census storage empty (`JobFunding::default()`), so the
     first census after a load fills fresh storage, and its planning storage, site rows and
     walking proofs were all built in that slot.
  2. **Exact row capacity.** Every row array indexed by the planning owners (buildings + 2 ×
     parcels: the business book's owners, finance, buyer heads and tails, the input book's owner
     receipt chains, the customer book's owners, the replacement proof's marks, the reservations'
     operations and sites, and the journals sized by them; about 20 arrays) was sized exactly at
     its first preparation after loading. The first building appended later made each of them one
     row too short, so every one reallocated at twice its capacity and copied its rows in one
     planning slot: at 100k 228,323 → 228,324 owners at 02:00 and 03:00 (residential slot 22.2
     and 20.7 ms, about 13 ms of it this growth), at 30k 61,085 → 61,086 at 06:00 (7.1 ms).
     Arenas grown by request behaved the same way: a request just past the capacity doubled the
     business book's input tranches (54,387 → 108,774) and the input book's receipts, and with
     them every journal and port map sized by those capacities.
- **What changed:**
  1. **Census storage copied at load.** An uninterrupted game's census storage holds the previous
     published census, with the planning storage, site rows and walking proofs its planning
     passes prepared. Loading now gives census storage that holds no census a copy of the live
     books after preparing them, and prepares the copy too (`RollingCensus::warm_storage_from`,
     `SimCore::warm_loaded_planning`); a census that the load finished (one current at the save)
     is prepared at load instead of in a window slot. A census's result never depends on what its
     storage held: what it reuses from earlier content (household quote prefixes, retained
     customer accounts, site rows, walking proofs) is keyed by exactly its inputs, as with the
     previous census in an uninterrupted game. The window-slot preparation of a fresh census book
     stays for a new world's first census (every census since the sixth commit). `JobFunding` and three of its parts derive `Clone`.
  2. **Spare row capacity** (`utils/storage.rs`: `reserve_rows`, `resize_rows`, `grow_rows`,
     `spare_capacity`). Storage indexed by rows that follow the city, and the arenas grown by
     request whose capacity also sizes their journals, keep spare capacity: when fewer than
     `len / 8` spare rows would remain, the capacity grows to the larger of `len + len / 4` and
     twice the old capacity, the amortized growth `Vec::reserve` gives. An appended building or
     parcel now lands in spare rows; growth happens at load, where the books are prepared, and
     then once per eighth of growth of the city at the earliest. Capacity never changes a result:
     every commit still preflights its storage and a shortfall replays the proposal after growing
     exactly as before (`demand/supply/storage.rs`). O(1) per call without growth.
- **Targeted costs** (diagnostic; the same city, pinned, first commit → second commit; ms):

  | Tier | Cost | First commit | Second commit |
  | --- | --- | --- | --- |
  | 30k | census book's preparation (00:58 window slot) | 11.7 (frame 13.1) | none |
  | 30k | 06:00 residential slot | 7.1 (frame 9.6) | 2.5 |
  | 30k | 01:00 residential slot | 1.6 | 3.7 |
  | 100k | census book's preparation (00:58 window slot) | 36.5 (frame 41.8) | none |
  | 100k | 02:00 residential slot | 22.2 (frame 28.8) | 9.0 (frame 14.8) |
  | 100k | 03:00 residential slot | 20.7 (frame 27.9) | 9.3 (frame 15.4) |
  | 100k | 01:00 residential slot | 5.5 (frame 13.0) | 12.3 (frame 18.7) |
  | 100k | first frame after loading | 27.6 | 28.0 (not addressed) |

  The 01:00 slot now plans on the copied book's first lending after a census refilled it: it
  merges the site rows of the hour's changes and lays them out again, as every residential slot
  does (2:00 and 3:00 are the steady 9 ms), plus about 3 ms of row merges and owner storage on
  the copy (diagnostic: supplier rows 3.5 against 2.5 ms, owner storage 1.4 against 0.4 ms,
  queued replay 2.0 against 1.2 ms). Steady hours are unchanged (residential slot 8.2–9.8 ms at
  100k). The largest census window slot at 100k is now one of the census's own sliced steps
  (3.9 ms).
- **Equivalence.** Envelope bit-identical to the first commit (every `envelope` metric of the
  gate report equals the first commit's); determinism shows no differences (repeat, save/reload,
  1 and 3 workers); burst limits pass. The new paths run in the gate: every reload of the
  determinism check copies the loaded live books into the census storage, or prepares a census
  the load finished, and the census reload continuation tests compare the next three censuses
  bit for bit with the saved game's. A new test compares a census run on a copy of the live books
  with one on fresh storage (`the_census_on_a_copy_of_the_live_books_equals_one_on_fresh_storage`),
  and one checks the spare-capacity growth (`row_storage_keeps_spare_capacity_and_grows_amortized`).
  Test builds still compare each prepared book's fingerprints with the lent ones.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one session
  with the first commit's binary as the control, both on copies of the same prepared cities (the
  save format is unchanged). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 4.1 (8.5) | 1.17 (1.19) | 1.42 (1.44) | 0.51 (0.51) | 0 (1) | 0 (0) | 1.8 (1.8) |
  | 10k | 32× | 4.6 (5.3) | 2.07 (2.17) | 3.24 (4.90) | 0.94 (0.96) | 0 (0) | 0 (0) | 1.8 (1.7) |
  | 30k | 1× | 8.1 (12.2) | 2.05 (2.07) | 2.93 (2.96) | 1.06 (1.06) | 1 (3) | 0 (0) | 4.9 (4.9) |
  | 30k | 32× | 14.6 (18.1) | 6.59 (6.38) | 13.07 (13.22) | 2.96 (2.98) | 16 (16) | 0 (2) | 4.9 (4.9) |
  | 100k | 1× | 29.9 (40.0) | 6.63 (6.77) | 11.27 (11.40) | 4.57 (4.59) | 292 (313) | 4 (4) | 18.6 (18.6) |
  | 100k | 32× | 52.5 (57.6) | 29.58 (30.38) | 50.41 (50.55) | 14.28 (14.41) | 2,700 (2,700) | 373 (402) | 18.5 (18.6) |

  Every gate limit passes for both builds. The census book's preparation and the second storage
  growth are gone from every tier. What sets each maximum now: at 30k 1× an agent-tick frame at
  15:47 (8.1 ms, 7.2 ms of it agent tick), the 06:00 frame 5.0 ms (10.3 in the control); at 30k
  32× the hour frames carrying slots 0–3 (12–14.6 ms; the 06:00 frame fell from 18.1 to 13.6 ms);
  at 100k 1× the day's first frame after loading (29.9 ms: the job and restock queues), then hour
  frames of 15.5–17.1 ms, an agent-tick frame (05:51, 16.9 ms, agent tick 15.6 ms) and the 01:00
  frame (16.7 ms, the copied book's first planning); at 100k 32× the first frame and the hour
  frames (45–52.5 ms; open question 6), the 02:00 frame of the control (57.6 ms) being gone.
  Means are equal or lower and p99 is within 3.3% (30k 32× p99 +3.3%, inside the run-to-run
  variation the gate allows). Load times are unchanged at 0.1 s resolution (100k 18.55 against
  18.58 s, 30k 4.92 against 4.90 s): the copy and its preparation cost a few hundredths of a
  second.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the first commit as
  control: hour means 1.56–1.58, 5.51–5.66, 9.59–9.68 and 30.30–30.90 ms (control 1.57–1.61,
  5.69–5.75, 9.51–9.71 and 29.44–29.63). The 30k fiscal-stress hour is about 3% dearer, in
  planning (729–744 against 710–715 ms a day), consistent with work that walks the larger row
  capacity; the third commit checks it. A new work-count failure, `buyer_sites_built` (10 → 92
  against the 3.43× scaling limit), is the opposite of a regression: loading now builds the buyer
  site rows, and the timed day builds 10 and 92 against 1,212 and 3,564 before. The other
  work-count limits fail as before (the fiscal-stress `cch_searches` limit sits at its edge and
  failed in one run of each build).
- **Build identity:** the gate, `PERF-07` and the test suites ran on the committed sources built
  with rustc 1.99.1 (`$TEST_BINARY`). The control is the first commit's gate binary.
- **Tests:** 2,542 release and 2,542 debug library tests (two new) and the 4 growth integration
  tests pass; clippy adds nothing (the 59 warnings in `agents/data.rs` and `asset_document.rs`
  remain). `./run.sh --test` in an isolated profile passes with no script error; the change
  alters what loading prepares, which the bridge's load tests exercise.
- **Save format:** unchanged (103). Nothing new is saved: the copy and the spare capacity are
  made at load.
- **Memory:** the census storage holds a book from load on, as it does after the first hour of an
  uninterrupted game; spare capacity reserves up to twice the length of the owner-indexed arrays,
  untouched until rows are written.

#### `SIM-02-K`, part 5 — route scratch prepared at load (2026-10-21, third commit)

- **Attribution first.** The second commit's build was traced over the first 70 game minutes at
  100k and 10 at 30k, 1×, and over the full day at 30k 32× (pinned, `RAYON_NUM_THREADS=8`,
  instrumented, with every route-scratch preparation printed with its pool worker). Everything
  the second commit's handoff measured in the day's first frame and minutes was cold route
  scratch, not cold simulation state:
  1. **Each pool worker's thread-local trip scratch** (`TripQueryScratch`: lane labels, the
     pedestrian connector table and its completeness proof, the search heap, the CCH labels)
     costs 7–9 ms to prepare at 100k, O(lanes + connections). Serial slot work prices and plans
     trips on whichever worker runs the frame's install, so each of the eight workers paid it
     the first time it ran such work after loading: the restock queue's reservations
     (`apply_replenishment_plan` → the shopping leg's timeout budget, 7.2 ms of its 7.7 ms) and,
     one per worker over the first seven minutes, single freight dispatches (7.9–8.9 ms, four
     times) and shopper checks (7.3–7.6 ms, twice).
  2. **The job search's eight shard scratches** (`build_home_job_options`, 9.6 ms in parallel:
     each shard's `prepare_deferred_quotes` prepares the same storage, 9.1–9.4 ms) and **its
     shared network-leg storage** (`SharedNetworkQuotes::sync`, 4.3 ms: one bucket pair per lane,
     allocated at the first search). The legs themselves are exact per CCH and lane revision.

  The movement pass's shards were already prepared at load (part 3).
- **What changed:**
  1. **One owned route scratch for the slots.** The frame runs its slots with
     `SimCore::slot_route_scratch` installed as the thread's route scratch
     (`TripQueryScratch::with_owned`, as the movement pass installs its shards), so the slots'
     serial trip pricing and planning use one scratch whichever pool worker runs the frame.
     Parallel work inside a slot keeps each worker's own scratch, as before (work the frame's
     own thread steals uses the slot scratch). Scratch only memoizes storage and exact
     per-revision quotes, so which scratch serves a query never changes its result (the
     determinism checks' 1 and 3 workers already cross threads). Besides the first frames, a
     road edit now re-prepares one slot scratch instead of each worker's as it next runs a slot.
  2. **Prepared at load.** `SimCore::prepare_loaded_pathing` prepares the slot scratch with its
     route buffers, and `HouseholdSystem::prepare_job_search_scratch` sizes the job search's
     network-leg storage and prepares one shard scratch per worker of the pool, as the first
     search would. Both are functions of the loaded network only. Cost at load: about 20 ms at
     100k (the shards in parallel about 9 ms, the slot scratch about 6.0 ms, the leg storage
     4.4 ms; instrumented).
- **Targeted costs** (instrumented; the same city, pinned, second commit → third commit; ms):

  | Tier | Cost | Second commit | Third commit |
  | --- | --- | --- | --- |
  | 100k | first slot frame after loading | 28.2 | 6.8 |
  | 100k | its job requests (shard scratch, leg storage, supply) | 14.8 (9.6, 4.3, 0.8) | 1.1 (0.4, 0, 0.6) |
  | 100k | its restock requests (scratch in the leg budget) | 7.8 (7.2) | 0.3 (0) |
  | 100k | cold trip events in minutes 0–7, each with a worker's scratch | six of 7.3–8.9 | none |
  | 100k | route scratch prepared in any frame of the first 70 minutes | 16 times, 6.7–9.4 | none |
  | 30k | first slot frame after loading | 6.6 | under 3.0 (largest frame of the first 10 minutes) |

  The first frame after loading at 100k is now its agent tick's first frame (10.8 ms, agent
  tick 10.0 ms against about 3 ms steady), probably the first tick's building-reference scrub
  and lane-bucket snapshot rebuild, which loading leaves for it (not measured; `SIM-02-K` part
  4's territory, not taken here).
- **Equivalence.** Envelope bit-identical to the second commit (every `envelope` metric of the
  gate report equals the K5 reference report and the second commit's gate report);
  determinism identical (repeat, save/reload, 1 and 3 workers); burst limits pass. Every reload
  of the determinism check runs on the prepared scratch, against the continuous game whose
  scratch was prepared lazily. A new test loads one save twice, discards one game's slot and
  job-search scratch, and compares both games every frame for three hours
  (`route_scratch_prepared_at_load_runs_as_cold_scratch_does`).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one
  session with the second commit's binary as the control, both on copies of the same prepared
  cities (the save format is unchanged). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 6.7 (5.9) | 1.18 (1.18) | 1.44 (1.46) | 0.51 (0.52) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 10k | 32× | 4.9 (5.0) | 2.07 (2.13) | 4.00 (4.17) | 0.94 (0.93) | 0 (0) | 0 (0) | 1.7 (1.8) |
  | 30k | 1× | 5.4 (10.0) | 2.07 (2.07) | 2.90 (2.99) | 1.07 (1.07) | 0 (1) | 0 (0) | 4.9 (4.9) |
  | 30k | 32× | 13.2 (13.5) | 5.69 (5.71) | 13.06 (13.31) | 2.92 (2.97) | 13 (16) | 0 (0) | 4.9 (5.0) |
  | 100k | 1× | 17.9 (29.1) | 6.69 (6.76) | 11.08 (11.24) | 4.58 (4.57) | 303 (302) | 2 (5) | 18.6 (18.6) |
  | 100k | 32× | 52.5 (52.6) | 29.45 (29.85) | 50.14 (49.22) | 14.15 (14.18) | 2,700 (2,700) | 354 (363) | 18.7 (18.6) |

  Every gate limit passes for both builds. The day's first frame after loading is gone at every
  tier: at 100k 1× it set the control's maximum (29.1 ms: job and restock queues), and the
  maximum is now an agent-tick frame (05:09, 17.9 ms, of it agent tick 16.3 ms), then the 01:00
  frame (17.6 ms, the residential slot on the copied book) and hour frames of 15.5–16.6 ms; at
  30k 1× the control's first frame (7.0 ms) is gone, and both builds' largest frames are single
  render-snapshot or agent-tick frames (the control's 10.0 ms at 23:44 is a snapshot of
  9.0 ms; this build's 5.4 ms an agent-tick frame), so 30k 1× now meets 8 ms, as does 10k at
  both speeds; the 10k 1× maximum of 6.7 ms (control 5.9) is a snapshot frame (6.2 ms of render
  snapshot), within run-to-run variation. 30k 32× and 100k 32× are level: hour frames (30k
  11.5–13.2 ms: planning slots, the removal window's first slots and the cell-lot region a
  removal dirtied; 100k 45–52.5 ms, open question 6). Means are equal (within 1.7%) and p99
  equal or lower; 100k 1× frames over 16.7 ms 2 (5). Load times are level (100k 18.57 against
  18.56 s): the prepared scratch costs about 20 ms at 100k.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the second commit as
  control: hour means 1.56–1.58, 5.46–5.54, 9.48–9.78 and 29.23–29.74 ms (control 1.53–1.55,
  5.38–5.42, 9.52–9.93 and 29.51–30.53); level, and the same work-count limits fail. The
  control's 30k fiscal-stress hour spread (29.51–30.53 ms) covers most of the second commit's
  3% rise, which was therefore largely run-to-run variation.
- **Build identity:** the gate, `PERF-07` and the test suites ran on the committed sources built
  with rustc 1.99.1 (`$TEST_BINARY`). The control is the second commit's gate binary.
- **Tests:** 2,543 release and 2,543 debug library tests (one new) and the 4 growth integration
  tests pass; clippy adds nothing (the 59 warnings in `agents/data.rs` and `asset_document.rs`
  remain). `./run.sh --test` in an isolated profile passes with no script error; the change
  alters what loading prepares, which the bridge's load tests exercise.
- **Save format:** unchanged (103). Nothing new is saved: the scratch is prepared at load.
- **Memory:** one more route scratch (the slots'), the size of a worker's; the job search's
  shards and leg storage exist from load on, as they do after the first search.

#### `SIM-02-K`, part 5 — planning a minute apart (2026-10-21, fourth commit)

- **What set the 30k 32× maximum.** The third commit's full-day trace (pinned, 8 workers,
  diagnostic): the even hours' first frames (11.1–14.0 ms) held slots 0–4, that is the four
  planning slots (residential 2.2–2.6, commercial 0.7–0.9, industrial 0.6–1.2, actions 0.6–0.8
  ms), one building removal at slot 4 (1.1–1.3 ms) and the cell-lot region it dirtied (2.2–3.4
  ms of generated-cell regeneration that creates no lot), with the agent tick (1.2–2.0 ms),
  trip events and the snapshot. Hours without a removal still reached 8.1–9.0 ms. A 32× frame
  crosses about 8.5 slots, so any two systems fewer than 9 slots apart can share one.
- **What changed** (first commit, a scheduling change; see
  [Decided while unattended](#decided-while-unattended)):
  1. **Planning slots a game minute apart.** Residential, commercial and industrial planning and
     the actions slot run at the first slots of the hour's minutes 0, 1, 2 and 3 (slots 0, 16,
     32, 48; `PlanningSlot` is the slot of the hour) instead of slots 0–3, so at 32× each runs in
     a frame of its own. Every slot still plans on the census published at slot 0.
  2. **Removals a minute after the actions slot.** The removal window (`building removals`,
     `departures`) and the `level changes` queue start at slot 64 (`REMOVAL_WINDOW_START`,
     minute 4) instead of slot 4; the window still ends before the census window (slot 480), so
     no removal restarts a census or invalidates the books a later planning slot lends. A
     compile-time check keeps the window after the actions slot.
  3. **Saves** (format 104, the only one that loads): a save in slots 0–47 (slots 0–55 since the
     tenth commit) keeps the published
     census the remaining planning slots read (`census_inputs.published`; slots 0–2 before), and
     every save keeps the last admission and removal diagnostics (`demand_diagnostics`), which
     a load used to recount from the loaded city. The Kuopio fixture gained the table (default
     diagnostics, what a re-save writes) and its version, and its hash in `placements.json`
     follows.

  Second commit (identical results): the census storage copy made at load keeps the live
  books' capacity for the arenas a census refills (business receipts, consumer claim staging;
  `JobFunding::reserve_census_arenas_like`), and the census's receipt estimate keeps spare
  capacity (`spare_capacity`), as the arenas grown by request already did.
- **Targeted costs** (diagnostic; the same instrumentation, pinned, 8 workers; third commit on
  its cities, fourth commit on its own; ms):

  | Tier | Cost | Third commit | Fourth commit |
  | --- | --- | --- | --- |
  | 30k 32× | worst frame of the day | 14.0 | 8.5 |
  | 30k 32× | worst planning frame (hours' first frames) | 14.0 | 6.7 (residential slot 2.7–3.2) |
  | 30k 32× | removal frame (minute 4: removal 1.0–1.2, its cell-lot region 2.5–3.7) | shared the hour frame | 6.7–8.5 |
  | 100k 1× | 01:00 residential slot (planning on the copied book) | 10.9–11.9 | 9.3 (02:00: 9.6) |
  | 100k 1× | receipt journal growth at 01:00 (entries) | 99,316 → 198,632 | none |
  | 100k 1× | census refill growth of the copy (receipts; claim staging) | 46,280 → 99,342; 0 → 163,290 | none |

  The 30k 32× worst frames are now the hours' removal frames (minute 4), which carry the hour's
  clearances and the cell-lot regions they dirtied in one frame (the region is served at the
  next slot); the planning frames peak at 6.7 ms. At 100k the receipt arena is 124,075 from load
  on (the load-time census's estimate with spare rows), and the copy keeps it.
- **Envelope** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass, no failures; the second commit's
  envelope is bit-identical to the first's. Against the third commit (control in parentheses):

  | City | Net income per resident day | Employment rate | Households per target | Jobs per target | Bankruptcies per 1k buildings day | Deliveries per 1k buildings day | Out-of-stock rate | Trip failures per 1k residents day |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | baseline 10k | 13.60 (13.58) | 0.6775 (0.6767) | 0.5309 (0.5306) | 1.386 (1.387) | 0.058 (0.039) | 172.9 (172.1) | 0.0935 (0.0930) | 0 (0) |
  | fiscal 10k | 27.24 (27.31) | 0.0743 (0.0743) | 0.5299 (0.5295) | 1.032 (1.034) | 15.16 (15.15) | 8.63 (8.57) | 0.784 (0.784) | 0.020 (0.007) |
  | baseline 30k | 12.87 (12.97) | 0.6720 (0.6733) | 0.5060 (0.5060) | 1.328 (1.328) | 0.316 (0.298) | 174.3 (175.0) | 0.184 (0.183) | 0.007 (0.007) |
  | fiscal 30k | 28.35 (28.36) | 0.0769 (0.0771) | 0.5059 (0.5059) | 0.995 (0.995) | 15.75 (15.79) | 10.58 (10.30) | 0.775 (0.774) | 0.014 (0) |
  | arrivals 30k | 12.85 (12.90) | 0.6751 (0.6759) | 0.5037 (0.5036) | 1.329 (1.329) | 0.277 (0.256) | 174.2 (174.3) | 0.175 (0.175) | 0 (0.014) |

  Every difference follows from the hour's planning and removals moving by up to 60 slots:
  money-like metrics move by at most 0.8% (baseline 30k net income per resident 12.87 against
  12.97), employment and stock metrics by at most 0.3%, households, jobs and buildings by at
  most 0.2%. The relative changes in bankruptcies (baseline 10k +50%, 30k +6–8%) and trip
  failures are counts of a few events over the measured fortnight (bankruptcies: baseline 10k 6
  against 4, baseline 30k 90 against 85, arrivals 30k 79 against 73; trip failures 0–6 events a
  city), inside their bands. The eight notes are the earlier
  steps' (fiscal-stress deliveries and employment, fiscal 10k net income, the baseline 30k
  out-of-stock rate below its band), with values within 3% of the third commit's except the
  fiscal 30k final employment rate (0.0069 against 0.0080, both above its band as before).
  Bursts: the busiest minute holds at most 4.1% of an hour's departures (third commit 3.75%), the
  busiest settlement hour 7.4% (unchanged).
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days). The reload check found the household diagnostics difference above before the
  diagnostics were saved.
- **Tests:** 2,543 release and 2,543 debug library tests and the 4 growth integration tests pass
  on the final commit; clippy adds nothing (61 warning lines, all in `agents/data.rs` and
  `asset_document.rs`); rustfmt adds nothing on the touched lines. The planning-slot tests now
  name the slots by `PlanningSlot` and `REMOVAL_WINDOW_START`; the census continuation test saves
  after each planning slot and just before the next (slots 0, 1, 16, 31, 32 and 47); the census
  copy test runs the census on a copy with reserved arenas
  (`the_census_on_a_copy_of_the_live_books_equals_one_on_fresh_storage`); a business book test
  that needed an exactly sized first arena to force a replay sizes its reset smaller.
  `./run.sh --test` in an isolated profile passes on the final commit (2,548 Rust tests, the Godot
  bridge tests, no script error, no segfault; the rendered vegetation tests skip without a
  Forward+ display); the save format changed, which the bridge's save and load tests exercise.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one
  session with the third commit's binary as the control; the save format changed, so each build
  ran on cities it prepared itself (the control on the third commit's). Milliseconds and frame
  counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 5.0 (4.4) | 1.17 (1.17) | 1.43 (1.42) | 0.50 (0.51) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 10k | 32× | 4.7 (4.7) | 2.00 (2.07) | 2.44 (3.16) | 0.94 (0.94) | 0 (0) | 0 (0) | 1.7 (1.8) |
  | 30k | 1× | 8.1 (7.6) | 2.06 (2.05) | 3.07 (2.93) | 1.06 (1.06) | 1 (0) | 0 (0) | 4.9 (4.9) |
  | 30k | 32× | 8.2 (13.4) | 5.38 (5.69) | 8.01 (12.51) | 2.91 (2.92) | 3 (17) | 0 (0) | 4.9 (4.9) |
  | 100k | 1× | 16.5 (19.1) | 6.50 (6.66) | 11.30 (11.14) | 4.44 (4.58) | 281 (296) | 0 (2) | 18.6 (18.6) |
  | 100k | 32× | 27.9 (53.1) | 23.03 (29.15) | 25.98 (49.17) | 14.01 (14.14) | 2,700 (2,700) | 406 (344) | 18.6 (18.6) |

  Every gate limit passes for both builds. 30k 32×: the hour frames are gone from the worst
  frames (the hour's first frame, with the residential slot, peaks at 6.3 ms against 13.4, and
  no frame with a planning slot is among the worst, all minute-4 frames down to 7.3 ms); the
  maximum is now the hours'
  minute-4 frames (8.0–8.2 ms: 5.3–6.3 ms of slot work, the hour's clearances at slot 64, the
  cell-lot regions they dirtied at slot 65 and the hour's first construction start, also due at
  minute 4, 0.65–0.83 ms; agent tick 1.1–1.7; snapshot 0.5–1.1), 3 frames over 8 ms against 17,
  p99.9 8.0 against 12.5 ms. The 8 ms target is missed by 0.2 ms there (see
  [Decided while unattended](#decided-while-unattended) for the options). 30k 1×: both builds'
  maxima are render-snapshot frames (8.1 ms with a 6.9 ms snapshot at 04:36; control 7.6 ms, 4.9
  ms snapshot), not slot work; at 1× every slot already ran in a frame of its own. 100k 1×: the
  control's 00:00 and 01:00 frames (19.1 and 16.7 ms) are gone; the maximum is an agent-tick
  frame (05:42, 16.5 ms, agent tick 15.1 ms, part 4), then the hour frames (15.2–16.4 ms:
  residential slot about 9–9.5 ms with an ordinary frame). 100k 32×: the hour frames (control
  45–53 ms, 23 ms of them the hour's slots) are split into one frame a slot, so the maximum
  falls to 27.9 ms (minute-4 frames, 25–28 ms) and p99 from 29.2 to 23.0 ms; more frames pass
  16.7 ms (406 against 344), each lighter (open question 6). Means are equal or lower (100k 1×
  −3%); 10k and 30k p99 equal or lower. Load times are level.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the third commit as
  control, each build on cities it prepared: hour means 1.59–1.72, 6.25–6.30, 9.79–9.87 and
  29.88–30.74 ms (control 1.56–1.73, 5.51–5.63, 9.91–10.18 and 29.48–29.69). The 30k baseline
  hour is about 12% dearer because the timed hours do more work, not because work got dearer:
  workplace queries 3,646 calls against 2,248 (about 12 µs a call in both), building actions 16
  against 11, lot runs 17 against 12. The new schedule moves which actions fall inside the
  contract's timed hours. The work-count limits fail as before (the fiscal-stress `cch_searches`
  limit, at its edge, passed in both runs of this build).

- **Build identity:** the gate, `PERF-07` and the test suites ran on the committed sources built
  with rustc 1.99.1 (`$TEST_BINARY`). The first commit was amended after its gate for one clippy
  warning (a `to_saved` taking its `Copy` self by value); its tree was checked with clippy again,
  and the second commit's gate covers both. The control is the third commit's gate binary.
- **Save format:** 104. New: `demand_diagnostics` (the last admission and removal diagnostics);
  `census_inputs.published` kept for saves in slots 0–47.
- **Memory:** the business receipt arena keeps up to a quarter of spare capacity (about 25,000
  receipts at 100k); the census storage copy holds the live books' receipt and claim staging
  capacity from load on, as the second book does after an hour of play.

#### `SIM-02-K`, part 5 — lots derived without regenerating cells (2026-10-21, fifth commit)

- **What set the 30k maxima.** 32×: the hours' minute-4 frames (8.0–8.2 ms, all twelve worst
  frames of the fourth commit's day), which cross slots 64–65: the hour's clearances at slot 64
  (1.0–1.2 ms), the cell-lot regions those clearances dirtied, served at slot 65 (2.5–3.7 ms),
  and the hour's first construction start, also due at minute 4 (0.65–0.83 ms), with the agent
  tick (1.1–1.7) and the snapshot (0.5–1.1). Each region spent 2.2–3.2 ms regenerating generated
  cells and 0.34–0.53 ms deriving lots, with no candidate and no lot created (diagnostic,
  `economy_phases` channel). 1×: single render-snapshot frames (8.1 ms with a 6.9 ms snapshot at
  04:36, 7.8 ms with 6.3 ms at 13:51; snapshot mean 0.36 ms, p99 0.88 ms).
- **The cell regeneration reproduced the cache.** A cleared building's lot keeps its claims when
  its paint is intact, but the release marked the lot with `mark_cell_lots_dirty`, which also
  invalidates the generated cells of the lot and a halo of `CELL_DEPTH + 1` cells, the
  invalidation meant for edits that change what generation reads. Generation reads the roads and
  their alignments, paint, lot claims, the cells outside the regenerated envelope, curve sources,
  authored parcels and the caller's blockers (compiled roads, explicit sites, fields). A building
  leaving a lot it keeps changes none of these: occupancy is parcel-store state, a cell-lot
  building is no explicit site, and a removed field invalidates its own polygon
  (`publish_pending_production_site_removals`). A comparison of the cell store around every
  regeneration of the 30k 32× day (17 regions: cells, paint, claims and frontages over the
  region's chunks and their neighbours, the store revision and the curve sources) found every
  one identical before and after (diagnostic).
- **What changed** (identical results): `release_invalid_empty_cell_lot` calls
  `ZoningSystem::mark_cell_lots_for_derivation` when the lot keeps its claims. It queues the
  lot's region for derivation alone: the queue mark and the cell-tool epoch, no invalidation of
  generated cells. A release that frees the claims (paint erased while occupied) still calls
  `mark_cell_lots_dirty` and invalidates the halo, as do roads, paint, sites and fields. The
  region still refreshes the vacated lot's frontage and retires it if its road is gone, so the
  contract in `zoning.md` holds. Cost: the region's generation step is O(chunks) lookups when
  nothing else dirtied it; derivation is unchanged.
- **The render-snapshot spikes are preemption, not snapshot work** (no code change; see
  [Decided while unattended](#decided-while-unattended)). An instrumented test build recorded,
  for every frame of a full 1× day at 30k and 10k (pinned, 8 workers), the snapshot's phases, its
  rebuild flags and every thread's scheduler statistics (`/proc/self/task/*/schedstat`: time on
  a CPU, time runnable but waiting for one). Every snapshot frame over 2 ms held one pool worker
  that waited 1.0–2.8 ms for a CPU, and the stretched phase was a different one each time
  (entering the pool, bucket fill, grounding off-lane cars); no frame rebuilt anything (site and
  zone index clean, lane boxes current — the lane revision changes only with a road edit —, node
  positions, terrain and water clean). The agent-tick spikes show the same: the 30k frames with a
  4.2–6.3 ms tick held worker waits of 3.6–5.4 ms. Every 4096th frame, sampled, held no wait over
  0.5 ms. The spikes fall at different minutes in every run (fourth commit 04:36 and 13:51, its
  control 20:58 and 13:02, this diagnostic 00:56, 13:46, 20:02 and 21:19). The test's own thread
  ran about 0.07 ms a frame, so another task on the eight pinned CPUs held the worker's CPU.
- **Targeted costs** (ms; diagnostic where marked, both builds on the fourth commit's cities):

  | Tier | Cost | Fourth commit | Fifth commit |
  | --- | --- | --- | --- |
  | 30k 32× | worst frame of the day (`PERF-07`) | 8.7 (control run) | 6.8 (an hour frame) |
  | 30k 32× | minute-4 frames (`PERF-07`, worst) | 6.9–8.7 | at most 5.7 |
  | 30k 32× | a clearance's cell-lot region: generation (diagnostic) | 2.2–3.2 | 0.001 (no generation) |
  | 30k 32× | a clearance's cell-lot region: derivation (diagnostic) | 0.34–0.53 | 0.34–0.53 |
  | 30k 32× | cell-generation calls over the day (diagnostic) | 83 | 68 |
  | 30k 1× | largest render snapshot of the day (`PERF-07`) | 4.7 (control run) | 3.6 |
  | 10k 1× | largest render snapshot of the day (`PERF-07`) | 2.6 (control run) | 2.9 |

- **Envelope** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass, no failures, bit-identical to the
  fourth commit's envelope; the same eight notes. Bursts: identical (busiest minute 4.1% of an
  hour's departures, busiest settlement hour 7.4%).
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation are identical (10k,
  three days); the gate shows no differences.
- **Tests:** 2,544 release and 2,544 debug library tests and the 4 growth integration tests pass;
  clippy adds nothing (61 warning lines, all in `agents/data.rs` and `asset_document.rs`);
  rustfmt adds nothing on the touched files. New:
  `releasing_an_occupied_lot_with_intact_paint_derives_without_regenerating_cells` frees one of
  four occupied lots on a painted road at three angles and serves the queued region twice, as
  this commit does and with the former halo invalidation: this commit generates no cell, and both
  give the same derivations, cells, frontages, store revision and parcels. No bridge-facing code
  changed, so `./run.sh --test` was not required; it ran anyway, in an isolated profile on the
  committed tree: the Rust tests and the Godot bridge tests pass, no script error, no segfault
  (the rendered vegetation tests skip without a Forward+ display).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one
  session with the fourth commit's binary as the control, both on copies of the same prepared
  cities (save format 104, unchanged). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 5.3 (4.6) | 1.18 (1.16) | 1.43 (1.40) | 0.51 (0.50) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 10k | 32× | 3.7 (4.6) | 1.98 (2.00) | 2.73 (3.80) | 0.96 (0.96) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 30k | 1× | 7.1 (8.0) | 2.05 (2.06) | 2.97 (3.16) | 1.05 (1.03) | 0 (1) | 0 (0) | 4.9 (4.9) |
  | 30k | 32× | 6.8 (8.7) | 5.25 (5.45) | 6.34 (8.23) | 2.89 (2.88) | 0 (4) | 0 (0) | 5.0 (4.9) |
  | 100k | 1× | 16.4 (17.3) | 6.65 (6.58) | 11.44 (11.30) | 4.49 (4.44) | 294 (279) | 0 (2) | 19.0 (18.6) |
  | 100k | 32× | 26.4 (28.1) | 22.70 (23.97) | 25.11 (27.45) | 14.07 (14.05) | 2,700 (2,700) | 438 (405) | 18.9 (18.6) |

  Every gate limit passes for both builds; neither prepared a city (both loaded the same
  snapshots). 10k and 30k meet 8 ms at both speeds in this run. 30k 32×: no frame over 8 ms
  (control 4), p99.9 6.34 ms (8.23); the minute-4 frames are gone from the worst (the highest,
  14:04, 5.7 ms; the control's twelve worst were all minute-4 frames, 6.9–8.7 ms), and the
  maximum is now the 06:00 hour frame (6.8 ms: 4.0 ms of slot work, 3.0 of it the hour's first
  slot with the residential planning slot, agent tick 1.6, snapshot 1.2), then the other hour
  frames (5.8–6.5 ms). 30k 1×: the cell-lot frames of minute 4 (control 06:04, 5.1 ms with 3.4 ms
  of slot work) are gone too; the maximum is a single frame at 21:44 with 4.1 ms of slot work,
  which a repeat run of the same binary on the same city did not have (its largest frames were
  two render-snapshot frames at 21:43, 6.5 and 6.1 ms with 3.4 and 4.0 ms of snapshot), so it is
  noise, not simulation work; then an agent-tick spike (10:26, 6.8 ms, tick 6.4 ms); the
  control's maximum is its 15:00 hour frame (8.0 ms, 6.0 ms of slot work). The largest render
  snapshot of the day: 2.9 ms at 10k 1× (control 2.6), 3.6 ms at 30k 1× (4.7; one frame over 3
  ms against three), 6.3 ms at 100k 1× (6.1), 1.2–2.5 ms at 32×; snapshot means and p99 equal
  (30k 1× 0.36 and 0.88 ms). 100k: the maxima are an hour frame at 1× (06:00, 16.4 ms:
  residential slot plus an ordinary frame, and a 15.5 ms agent-tick frame) and agent-tick frames
  at 32× (26.4 ms, open question 6), within the control's variation. Means are level (within
  2%). Load times are level.

- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the fourth commit as
  control: hour means 1.60–1.69, 6.29–6.36, 9.54–9.55 and 29.21–29.22 ms (control 1.63–1.71,
  6.23, 9.48–9.62 and 29.38–30.27); level, and the same work-count limits fail.

- **Build identity:** the gate, `PERF-07` and the test suites ran on the committed sources built
  with rustc 1.99.1 (`$TEST_BINARY`); the control is the fourth commit's gate binary.
- **Save format:** 104, unchanged; the Kuopio fixture is unchanged.

#### `SIM-02-K`, part 4 — departures over the minute (2026-10-21, second commit)

- **Measured first** (temporary per-phase timing of the agent tick, never committed: every phase,
  the trip plans and commute estimates the movement pass makes and their CPU time summed over the
  workers, the slowest shard, and each thread's scheduler statistics per frame
  (`/proc/self/task/*/schedstat`, the run-queue wait of each pool worker); the fifth part-5
  commit's prepared cities, `RAYON_NUM_THREADS=8`, pinned to the performance cores, full day;
  diagnostic, not acceptance). At 100k 1× the agent tick exceeded 6 ms in 184 frames; 59 of them
  had a pool worker waiting 1.1–7.8 ms for a CPU (preemption, see `PERF-07`), and 124 of the other
  125 were departure waves:
  - every one fell in the frame that crossed into a minute of 04:47–05:59, 13:04–13:59 or
    21:00–21:57, the three-shift windows' departures;
  - each held 78–405 trip plans in the parallel movement pass (`plan_building_origin_trip`, CCH
    point searches and the exact path), 24–50 ms of CPU at 90–190 µs a plan;
  - neither the serial handoff set (0.35–0.6 ms) nor the commute estimates caused them;
  - the parallel movement pass took 2.7–9.9 ms (slowest shard 1.1–2.5× the mean), the rest of
    the tick 2.2–3.6 ms; ticks 6.0–12.4 ms (13.9 ms with preemption).

  Every agent due to leave in an authored minute planned its trip in the first tick after the
  clock crossed into that minute: about 370 agents a minute in the waves. At 1× that is one frame
  in 60. 30k 1× had no wave frame over 2.5 ms of agent tick; its 13 frames over 2.5 ms are
  preemption (11 with waits of 2.2–4.8 ms).
- **A minute's departures leave over its slots.** A cached work or home departure now leaves at
  its agent's slot of the minute, `floor(frac(seed/φ) × 16)` (`slot_in_minute_for_key`, the
  golden-ratio fraction `spread_slot` uses within the hour), in the first tick at or after it, so
  the departures of one minute spread over its 16 slots. The day's last minute keeps slot 0, so a
  32× frame that spans midnight cannot carry a departure into the next day. This is a gameplay
  change: a departure leaves up to 15/16 of a game minute later than before, within the same
  minute, so each departure keeps its minute. Alternatives and evidence in
  [Decided while unattended](#decided-while-unattended).
- **`current_congestion` deleted** (save format 105). The per-edge field was written by a serial
  pass over every agent at the end of each tick (0.34 ms at 100k, 0.07 ms at 30k) and saved, and
  read only by tests and the road-edit copy. The field, its setter, the save column, the pass,
  its five per-edge scratch arrays and the `PERF-06` contract's `congestion_agents` phase are
  gone; `rebuild_lanes_and_congestion` is `rebuild_lanes`. Three tests that only checked the
  congestion values are deleted (`rebuilding_with_live_congestion_preserves_free_flow_costs`,
  `test_congestion_zero_at_free_flow_speed`, `test_congestion_proportional_to_speed_deficit`);
  four others lose their congestion assertions. The Kuopio fixture drops the column (its hash in
  `placements.json` follows).
- **The first agent tick after loading.** It measured 9.4 ms of agent tick at 100k (frame 10.3
  ms): 166 trip plans and 26 commute estimates due at the save's first tick, a full lane-bucket
  rebuild (1.6 ms; the snapshot is not saved) and the building-reference scrub (0.2 ms; its
  revision is not saved). Loading now runs the scrub and the bucket rebuild
  (`AgentSystem::prepare_loaded_tick`, after the planning warm-up): both are functions of the
  loaded state the first tick would read unchanged, so results are identical. The first frame
  after loading is now 6.6 ms at 100k (agent tick 5.7 ms: 98 trip plans and 26 estimates, which
  the uninterrupted game makes in the same tick).
- **After** (same measurement on this build's cities, ms per ordinary frame; before in
  parentheses):

  | Phase | 30k 1× | 30k 32× | 100k 1× | 100k 32× |
  | --- | --- | --- | --- | --- |
  | **Agent tick** | **0.55** (0.68) | **1.26** (1.32) | **3.07** (3.43) | **6.76** (7.11) |
  | · tick-start buckets and claim reset | 0.01 (0.01) | 0.02 (0.02) | 0.04 (0.04) | 0.05 (0.05) |
  | · IDM speed | 0.07 (0.08) | 0.11 (0.11) | 0.34 (0.35) | 0.37 (0.38) |
  | · claim classification | 0.09 (0.11) | 0.15 (0.15) | 0.51 (0.51) | 0.54 (0.53) |
  | · parallel movement | 0.21 (0.22) | 0.47 (0.46) | 1.26 (1.26) | 2.83 (2.83) |
  | · · trip plans in it, CPU over the workers | 0.03 (0.03) | 0.78 (0.77) | 0.26 (0.24) | 6.73 (6.76) |
  | · · commute estimates in it, CPU | 0.01 (0.01) | 0.23 (0.23) | 0.08 (0.08) | 1.74 (1.76) |
  | · serial Dynamic movement | 0.04 (0.04) | 0.24 (0.23) | 0.25 (0.25) | 1.92 (1.87) |
  | · end-of-tick fill | 0.11 (0.20) | 0.15 (0.24) | 0.62 (0.99) | 0.65 (1.04) |
  | · · of it, the congestion pass | — (0.07) | — (0.08) | — (0.34) | — (0.35) |
  | · frontage | 0.00 (0.00) | 0.09 (0.09) | 0.01 (0.01) | 0.37 (0.38) |
  | Render snapshot | 0.33 (0.37) | 0.55 (0.54) | 0.75 (0.77) | 0.86 (0.86) |
  | Slot work | 0.04 (0.04) | 0.94 (0.94) | 0.23 (0.22) | 5.95 (5.98) |
  | Frame | 0.93 (1.11) | 2.80 (2.85) | 4.09 (4.48) | 13.64 (14.02) |

  Waves (agent tick per frame): at 100k 1× no agent tick exceeds 6 ms without a pool worker
  waiting at least 1.9 ms for a CPU (17 frames of 6.0–7.8 ms, all preempted; before: 125 frames of
  6.0–12.4 ms without preemption, and a peak of 13.9 ms); a wave frame holds at most 34 trip plans
  (405). At 100k 32× a frame covers about half a minute, so spreading within the minute only
  halves its waves: a wave frame still holds 171–258 plans (254–431), 22–33 ms of CPU, and its
  agent tick is 11.1–12.3 ms without preemption (11.0–16.7). At 30k neither speed has an agent
  tick over 2.5 ms (1×) or 3 ms (32×) without preemption.
- **Equivalence and targeted tests:** `due_departures_leave_at_their_seeds_slot_of_the_minute`
  (no departure before the seed's slot, one at it and in the next minute; the day's last minute at
  slot 0; the seeds that share a departure minute of a shift window spread over all 16 slots,
  each within half to twice the even share) and `lane_buckets_prepared_at_load_equal_the_first_ticks`
  (the prepared snapshot is valid with the scrub revision current, and six ticks move every agent
  bit for bit as without the preparation).
- **Envelope** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): passes with no failures. The congestion
  deletion alone is bit-identical to the fifth part-5 commit's envelope; the departure slots move
  every city a little, within tolerance: net income per resident within 0.46%, employment within
  0.3% (the fiscal-stress cities' final employment rates, 0.5–0.8%, already outside their bands
  before), jobs, households, residents and buildings within 0.15%, deliveries within 0.7% at
  baseline (fiscal stress 2.1–4.6%, already outside its band before), stock-outs within 1.1%.
  Larger relative changes are of rates near zero, a few events in the measured window: unhoused
  5e-5 to 1.4e-3, vacancy at 30k baseline 3.8e-5 → 5.2e-5, bankruptcies at 30k baseline 0.316 →
  0.291 per 1,000 buildings a day, trip failures 0–0.04 per 1,000 residents a day. The same 8
  notes as before (the fiscal-stress and 30k baseline stock-out metrics outside the `SIM-02-A`
  bands since earlier steps). Bursts: worst freight minute 3.1% (3.0%), shopping 2.7% (2.7%),
  arrivals 3.2% (3.2%), worst settlement hour 7.3% (7.3%), all within the limits.
- **Determinism:** repeat run, 1 and 3 workers and save/reload continuation show no differences
  (10k, 3 days).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"
  --perf07-baseline` the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`), full day, in one session
  with the fifth part-5 commit as the control: the control on copies of its cities (save format
  104), this build on its own (save format 105, the same city table by table). Milliseconds and
  frame counts, control in parentheses:

  | Tier | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms |
  | --- | --- | --- | --- | --- | --- | --- |
  | 10k 1× | 3.4 (4.8) | 1.13 (1.19) | 1.35 (1.44) | 0.48 (0.51) | 0 (0) | 0 (0) |
  | 10k 32× | 2.8 (2.8) | 2.06 (2.00) | 2.52 (2.37) | 0.91 (0.94) | 0 (0) | 0 (0) |
  | 30k 1× | 10.3 (9.8) | 1.84 (2.07) | 2.41 (3.02) | 0.89 (1.05) | 1 (1) | 0 (0) |
  | 30k 32× | 7.3 (6.8) | 5.33 (5.43) | 6.29 (6.65) | 2.85 (2.94) | 0 (0) | 0 (0) |
  | 100k 1× | 18.4 (17.3) | 6.11 (6.60) | 9.72 (11.39) | 4.04 (4.46) | 128 (293) | 1 (2) |
  | 100k 32× | 26.3 (29.0) | 22.10 (21.79) | 25.05 (25.23) | 13.68 (13.90) | 2,700 (2,700) | 347 (403) |

  Every gate limit passes for both builds; load time is unchanged (100k 18.6 s). At 100k 1× the
  frames over 8 ms fall from 293 to 128, p99.9 from 11.4 to 9.7 ms and the mean from 4.46 to 4.04
  ms. The means fall 3–15% at 10k and 30k. What sets each maximum:
  - 10k: a render-snapshot frame at 1× (3.4 ms, snapshot 3.0) and an agent-tick frame at 32× (2.8);
  - 30k 1×: one agent-tick frame (10.3 ms at 22:44, tick 9.8 ms; the control's maximum is the same
    kind of frame, 9.8 ms at 17:17); in the profile run every 30k 1× agent tick over 2.5 ms had a
    worker waiting 2.2–4.8 ms for a CPU, so these are preemption, not simulation work; the 8 ms
    target holds at 30k except for preempted frames;
  - 30k 32×: the 22:00 and 06:00 hour frames (7.1–7.3 ms: planning slot work 3.9–4.2, agent tick
    1.7–2.2, snapshot 1.1–1.2; part 5's per-slot setup);
  - 100k 1×: the hour's planning-slot frames (14.4–18.4 ms, slot work 9.8–12.1 ms; part 5), plus
    one agent-tick frame of 14.9 ms with the preemption pattern (the profile found no 100k 1×
    agent tick over 6 ms without a waiting worker);
  - 100k 32×: agent-heavy and slot-heavy frames of 23.5–26.3 ms (open question 6).

- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with part 5's fifth commit as
  control, each build on cities it prepared: hour means 1.59–1.62, 6.06–6.12, 9.13–9.18 and
  29.46–29.93 ms (control 1.61–1.62, 6.21–6.22, 9.48–9.63 and 28.74–29.22); level, and the same
  work-count limits fail.

- **`PERF-06`** (`benchmark_agent_tick_contract`, three alternating process pairs with the control,
  pinned, `RAYON_NUM_THREADS=8`, `METRUM_DEBUG=0`): every work count equals the control's except the
  deleted `congestion_agents`, and every count grows exactly 3× from 10k to 30k (lane counts 16).
  The single timed tick per tier, median [range] ms, control then this build: network 0.24
  [0.23–0.30] / 0.40 [0.33–0.40] at 10k and 0.78 [0.48–0.79] / 0.45 [0.42–0.58] at 30k; egress 0.68
  [0.49–0.99] / 0.47 [0.44–0.49] and 1.28 [1.21–1.28] / 1.25 [1.20–1.31]; ingress 0.27 [0.24–0.34]
  / 0.32 [0.22–0.62] and 0.31 [0.27–0.42] / 0.46 [0.45–0.52]. The ranges overlap or swap direction
  between tiers; routine regression signals, not a speedup claim.
- **Tests:** 2,543 release and 2,543 debug library tests and the 4 growth integration tests pass
  (three congestion tests deleted, two added); clippy adds nothing (the 59 warnings in
  `agents/data.rs` and `asset_document.rs` remain); rustfmt adds nothing in the touched lines. The
  save format and the agent columns the render snapshot reads changed, so the isolated
  `./run.sh --test` ran: the Rust suites and every headless bridge, asset and shader test pass with
  no script errors (run without a display, so the rendered vegetation tests skip).
- **Build identity:** every result above ran on the committed sources built with rustc 1.99.1; the
  per-phase tables on temporary timing builds of the same sources.
- **Save format:** 105; `SAVE_VERSION` is the only version constant and the only one that loads.
  Each build prepares its own `PERF-07` cities; this build's equal the control's table by table
  except the version row and the unordered row order of `lane_connections` (compared at 10k).

#### `SIM-02-K`, part 5 — the census's book prepared at minute 59 (2026-10-21, sixth commit)

- **Attribution first.** Every planning slot was timed per phase on part 4's second commit (the
  `economy_phases` channel, which already splits each slot into its setup phases and the site
  preparation into its row merges, layouts and storage; diagnostic, pinned, 8 workers; 100k 1×
  over 250 game minutes and 30k 32× over the full day on the `PERF-07` cities). Steady hours,
  slot means in ms (maxima in parentheses):

  | Tier | Phase | Residential | Commercial | Industrial | Actions |
  | --- | --- | --- | --- | --- | --- |
  | 100k 1× | whole slot | 8.95 (9.91) | 3.85 (4.14) | 4.66 (4.96) | 7.03 (7.19) |
  | 100k 1× | pass rows (`begin_live_pass`) | 0.77 | 0.83 | 0.84 | 0.86 |
  | 100k 1× | snapshots (front, live workforce, compose) | 0.52 | 0.59 | 0.50 | 0.51 |
  | 100k 1× | site preparation | 4.27 | 0.77 | 0.78 | 0.81 |
  | 100k 1× | · of it supplier and buyer row merges | 1.27 | — | — | — |
  | 100k 1× | · of it supplier and buyer layouts | 2.21 | — | — | — |
  | 100k 1× | queued projects' replay | 1.55 | 0.32 | 0.33 | 0.33 |
  | 100k 1× | planning (calendar, existing buildings, lots, decisions) | 1.79 | 1.29 | 2.17 | 1.13 |
  | 100k 1× | admissions / spawn queue and construction epoch | — | — | — | 1.04 / 2.29 |
  | 30k 32× | whole slot | 2.59 (2.73) | 1.10 (1.33) | 1.58 (2.05) | 1.40 (2.26) |
  | 30k 32× | site preparation | 1.43 | 0.24 | 0.23 | 0.23 |
  | 30k 32× | · of it row merges and layouts | 1.19 | — | — | — |
  | 30k 32× | queued projects' replay | 0.34 | 0.10 | 0.10 | 0.10 |

  The residential slot is the only one that merges site rows and lays them out again: a book is
  lent in every slot of one hour and refilled by the census of the next, so its residential slot
  finds rows two hours old (every building and occupancy change of two hours) and a census book
  whose row base and port offsets moved, and rebuilt both layouts (supplier
  `SupplierCandidateIndex`, buyer port index) in full. The later slots of the hour found the rows
  current (no building change between minutes 0 and 3: removals start at minute 4 and the queue's
  construction starts from minute 4 too). Every slot pays the same O(B/p) setup, about 3.8 ms at
  100k, which is the whole commercial slot (its decisions take 0.002 ms). Finer timing points in
  a test build (100k, ms per lending of the books): pass rows 0.26–0.30 (funded operations) plus
  0.49–0.51 (operation sites); owner storage 0.36–0.38 (business book), 0.12 (input book), 0.07
  (customer book); live workforce 0.45–0.56; workforce calendar reset 0.39–0.42; actions slot
  admissions 1.19, construction epoch 2.28 (begin, front and compose 0.62, site preparation 0.79,
  replay 0.85).
- **What changed:**
  1. **Each finished census's book is prepared for planning before its hour.** A census start
     now clears its book's `planning_prepared` (`RollingCensus::begin`), so the existing
     window-slot preparation (`SimCore::prepare_census_book_for_planning`, which lends the books
     once as a planning slot does) runs for every census, every hour, not only for a new world's
     first: from minute 59's first slot (`PLANNING_PREPARATION_SLOT`, slot 944) once the census
     is done. It merges the two hours' site-row changes and lays both indices out on the refilled
     book, so the residential slot at slot 0 finds them current unless a building or occupancy
     changed in between, and then merges only those changes. Minute 59 leaves only that minute's
     events (a construction start or completion) to invalidate the rows, and is a game minute
     before the residential slot, so at 32× the two never share a frame. A census that finishes
     later (the window's last slot gets what remains) is prepared at its next window slot, or by
     its residential slot as before. What is prepared is derived and keyed by the city's
     revisions, as for the first-census preparation, so no result changes. One input of the
     preparation is not derived: it prepares the building site index first, and a stale zone
     index's rebuild also resets the vacancy lists to building order, which admission's
     tie-breaks read (`ALLOC-03`; a load re-marks a saved stale index for the same reason). The
     window therefore waits while the zone index is stale
     (`BuildingAllocator::zone_index_needs_rebuild`) and prepares at the first window slot after
     the index's own reader rebuilt it.
  2. **Large row growth is filled on the pool** (`utils/storage.rs`: `resize_rows`,
     `grow_rows`). A planning pass truncates the owner rows it appended to its three books
     (business owners, finance, buyer heads and tails, staging; the input book's owner receipt
     chains; the customer book's owners) and the next pass appends them again: about 190,000 rows
     an array at 100k, written by one thread in 0.07–0.38 ms a book. An append of at least 16,384
     rows now uses `par_extend` with `rayon::iter::repeat_n`, so the rows equal a serial fill's.
     O(appended rows / p).
- **Targeted costs** (diagnostic, same cities, pinned, 8 workers, steady hours; ms):

  | Tier | Cost | Before | First change | Both changes |
  | --- | --- | --- | --- | --- |
  | 100k 1× | residential slot | 8.95 (9.91) | 5.13 (5.15) | 4.87 (4.88) |
  | 100k 1× | · its site preparation | 4.27 | 0.79 | 0.47 |
  | 100k 1× | commercial / industrial slot | 3.85 / 4.66 | 3.79 / 4.73 | 3.71 / 4.54 |
  | 100k 1× | actions slot | 7.03 | 7.13 | 6.46 |
  | 100k 1× | census book's preparation (slot 944, own slot) | — | 5.1–6.4 | 4.4–6.4 |
  | 30k 32× | residential slot | 2.59 (2.73) | 1.40 (1.65) | 1.37 (1.45) |
  | 30k 32× | census book's preparation | — | 1.72 (2.03) | 1.4–2.0 |

  (Diagnostic builds with timing points; the zone-index guard came after these runs.) In every
  steady hour of these runs the residential slot found the site rows and layouts current (no row
  merge or layout ran in it). Still in every lending at 100k: pass rows 0.8, live workforce 0.5,
  calendar 0.4, existing buildings 0.5–0.78, lot scan 0.27–0.94, snapshot and storage about 0.5.
  A full day with the final code and more timing points (100k and 30k 1×; every hour's book was
  prepared at slot 944, and no hour waited for a stale zone index) attributes the rest: the
  residential slot's replay of the constructing buildings is their re-proof, 1.19 ms for 21
  buildings against 0.14 ms for the same buildings in the commercial slot (30k: 0.29 against
  0.05 ms for 7), because the hour's first quotes on the book are cold; the actions slot's
  admissions take 1.17 ms, of it 1.13 ms `BorderArrivalCosts::new` (one CCH sweep per border), a
  function of the network alone. Two census-book preparations of these runs took far longer than
  their phases (14.4 ms with 5.0 ms inside the timed phases at 100k, 11.2 ms at 30k 1× just after
  loading, with 0.7 ms of work in a stretched parallel phase), on hours that took 5.4 and 1.7–1.8
  ms in a run of the same simulation: the preemption pattern of `PERF-07`.
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass, no failures; every `envelope` metric and
  every burst share bit-identical to part 4's second commit's gate report; determinism shows no
  differences (repeat, save/reload continuation, 1 and 3 workers; 10k, three days). The window
  preparation runs every hour of every gate city, and the reload checks load saves taken across
  the window and the planning span.

- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with part 4's second commit as
  control: hour means 1.44–1.46, 4.92–4.98, 8.46–8.94 and 28.03–28.34 ms (control 1.57–1.58,
  6.03–6.10, 9.06–9.22 and 29.53–29.76): the 30k baseline hour is about 18% cheaper, and the
  same work-count limits fail.

- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one session with part
  4's second commit as the control, both on copies of the same prepared cities (save format 105,
  unchanged). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms |
  | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 4.2 (3.2) | 1.11 (1.10) | 1.35 (1.31) | 0.47 (0.48) | 0 (0) |
  | 10k | 32× | 6.7 (2.9) | 1.97 (1.90) | 2.38 (2.20) | 0.90 (0.89) | 0 (0) |
  | 30k | 1× | 10.7 (6.3) | 1.83 (1.85) | 2.52 (2.61) | 0.89 (0.91) | 1 (0) |
  | 30k | 32× | 10.6 (7.0) | 4.77 (5.20) | 5.53 (6.22) | 2.81 (2.83) | 1 (0) |
  | 100k | 1× | 13.7 (18.0) | 6.14 (6.14) | 9.88 (9.74) | 4.05 (4.05) | 168 (139) |
  | 100k | 32× | 25.8 (27.6) | 20.76 (21.97) | 23.29 (25.22) | 13.66 (13.69) | 2,700 (2,700) |

  **Recheck** (the coordinator, the same session, `--perf07-tiers 10000,30000`, two rounds
  alternating the control and this build, each on a fresh copy of the same prepared cities,
  every lock held, no backup running). Max ms, round 1 / round 2, control in parentheses: 10k 1×
  3.59 / 3.02 (7.24 / 3.86), 10k 32× 2.44 / 2.55 (3.21 / 3.13), 30k 1× 5.59 / 7.21 (7.50 / 7.48),
  30k 32× 5.86 / 6.35 (6.85 / 8.57); p99 4.77–4.84 ms at 30k 32× (5.15–5.22). This build's
  maxima are at or below the control's in every tier and round, so the first run's 6.7, 10.7
  and 10.6 ms were stalls, as attributed; 10k and 30k meet 8 ms at both speeds in all four of
  this build's days.

  Every gate limit passes for both builds. **100k 1×:** the hour frames are gone from the worst
  frames (control 14.3–18.0 ms at :00, of it 8.8–11.1 ms the hour's slots); the maximum is now a
  removal frame at minute 4 (13.7 ms), then minute-3 actions frames (12.0 ms, slot work 6.8–7.2).
  Frames over 8 ms rise from 139 to 168, because the census book's preparation adds about one
  9–10 ms frame an hour (an ordinary frame plus about 5 ms), each below the old hour frames; p99
  is equal and p99.9 1.4% higher. **30k 32×:** p99 −8% and p99.9 −11%; the hour frames fall from
  6.0–7.0 ms to 5.1–5.6 ms. The maxima at 10k 32× (6.7 ms), 30k 1× (10.7 ms) and 30k 32×
  (10.6 ms) did not reproduce in the run's repeat or in a day with per-thread scheduler
  statistics, whose slow frames all held pool-worker run-queue waits of 0.4–3.7 ms: the
  simulation is deterministic, so a frame that is 10 ms in one run and under 5 ms in another
  holds the same work plus a stall, the preemption pattern of
  [Frame budget](#frame-budget-perf-07). Means are level everywhere. Load times are level (100k
  18.8 against 18.6 s).
- **Tests.** 2,546 release and 2,546 debug library tests (three new) and the 4 growth
  integration tests pass; clippy adds nothing. `./run.sh --test` in an isolated profile passes
  with no script errors (the change touches `nodes/sim/core`, which the bridge's load and save
  tests exercise). New: `a_stale_zone_index_defers_the_census_books_preparation`,
  `census_books_prepared_in_the_window_play_as_books_the_planning_slots_prepare` (two games
  loaded from one save; one marks each finished census book prepared so its residential slot
  prepares it as before; both are compared every frame for three game hours, and the window
  prepared at least two books in the first game only), and
  `large_row_growth_fills_on_the_pool_as_a_serial_fill_does`.
- **Build identity:** the gate, `PERF-07` and the test suites ran on the committed sources built
  with rustc 1.99.1 (`$TEST_BINARY`); the control is part 4's second commit's gate binary.
- **Save format:** 105, unchanged; nothing new is saved (`planning_prepared` is derived; a loaded
  census book is prepared at load as before).
- **Memory:** unchanged; the census book's planning storage was already kept between hours.

#### `SIM-02-K`, part 5 — rows kept across lendings (2026-10-21, seventh commit)

- **Attribution first.** Every system of every slot was timed in a test build (a timing point
  around each scheduled system, plus the existing `economy_phases` lines; diagnostic, pinned,
  8 workers; 100k 1× over 190 game minutes and 30k 32× over the full day on the `PERF-07`
  cities, the sixth commit's code with only these timing points). The 100k 1× minute-4 frames
  (12.2–13.7 ms in the sixth commit's `PERF-07`) are the removal window's first slot: the
  `building removals` queue 3.9–5.1 ms, of it one zone-index rebuild 2.6 ms (0.43 ms the
  admission order) and per removal three serial scans of every agent or household (evacuation
  0.25, eviction 0.22, household invalidation 0.17 ms, about 0.65 ms together) plus the agent,
  household and freight remaps (0.15, 0.08, 0.08 ms) and the site publication (0.37 ms); with it
  the queue's construction start (0.8 ms) and restock requests (0.6 ms). The first hour after
  loading adds a cold construction start (2.5 ms) and a cold freight request (2.1 ms) to 00:04.
  No level change, departure or epoch runs there. Steady hours, means in ms:

  | Tier | Slot | Before | After |
  | --- | --- | --- | --- |
  | 100k 1× | residential (slot 0) | 4.90 | 4.32 |
  | 100k 1× | commercial (slot 16) | 3.54 | 2.94 |
  | 100k 1× | industrial (slot 32) | 4.43 | 3.88 |
  | 100k 1× | actions (slot 48) | 6.45 | 4.15 |
  | 100k 1× | removal window's first slot (`building removals`, slot 64) | 4.62 | 3.46 |
  | 100k 1× | · of it the zone-index rebuild | 2.62 | 2.08 |
  | 100k 1× | census book's preparation (slot 944) | 4.91 | 3.77 |
  | 100k 1× | its second half (slot 953) | — | 2.44 |
  | 100k 1× | pass rows a lending (`reservations_ms`) | 0.81–0.84 | 0.05–0.07 |
  | 30k 32× | residential | 1.38 | 1.22 |
  | 30k 32× | commercial | 1.03 | 0.89 |
  | 30k 32× | industrial | 1.52 | 1.33 |
  | 30k 32× | actions | 1.32 | 1.15 |
  | 30k 32× | removal window's first slot | 1.16 | 1.14 |
  | 30k 32× | census book's preparation / second half | 1.68 | 1.14 / 0.82 |

  (The "after" column measured the second half at slot 945, before it moved to 953; the work is
  the same.) The planning snapshots grew by 0.1–0.4 ms a lending where the pass rows no longer
  run first: the snapshot's parallel phases now wake the pool.
- **What changed:**
  1. **Pass rows kept across a book's lendings.** Every lending derived every live building's
     operation row (funded operation, 0.24–0.31 ms at 100k) and site row (door and cargo,
     0.49–0.55 ms) again, six lendings an hour of one book. `ProjectReservations` now keeps
     the rows with their inputs besides the buildings' own (the lent book's rows generation,
     the freight price phase, catalog, tuning and fiscal policy) and the building and entrance
     revisions they cover; a lending with the same inputs recomputes only the rows the
     allocator's change log names since then, the buildings appended since and the live rows
     the previous pass overwrote (an accepted replacement and its affected financing, now
     recorded), and every site row after an entrance change; otherwise, or for a change set
     over an eighth of the city, it derives every row on the pool as before. A book's rows
     generation (`JobFunding::rows_generation`) is drawn afresh for every new or cloned book, by
     every run of the census steps and at every change of `published`, which every invalidation
     of a published book makes, all through `set_published`. Each row is a function of its own
     building and entrance and those inputs, so the rows equal a full recomputation; test builds
     compare them bit for bit at every lending. O(changed) warm, O(B/p) otherwise.
  2. **Border arrival costs kept per routing revision.** Admission's border sweeps (1.13 of the
     actions slot's 1.17 ms of admissions at 100k) are kept by the allocator under the CCH build
     generation, its metric revision and the graph's node-role revision, the revisions the
     freight topology keys its border list on; while edited chunks wait for their CCH rebuild a
     batch sweeps afresh and keeps nothing, as before. Test builds compare kept sweeps with a
     fresh sweep. O(1) warm; about 1.1 ms saved in the actions slot.
  3. **The minute-59 preparation in two halves.** Slot 944 lends the census book for the
     supplier rows and their layout alone; from slot 953 (`PLANNING_PREPARATION_REST_SLOT`, nine
     slots later, more than a 32× frame's 8.5) a window slot lends it again, on kept pass rows,
     and prepares the buyer rows and the rest, finding the supplier rows current. A census
     finished later or a zone index stale at 944 is prepared in full from 953 or by its
     residential slot; a load prepares in full. Derived and keyed by the city's revisions, so
     no result changes.
  4. **A removal finds what it changes on the pool, and the zone index rebuilds on the pool.**
     Evacuation and eviction visit only the agents any of whose building references name the
     removed building, and the household invalidation only the households whose home or
     reserved store it was, found by one parallel filter each and updated in ascending order
     with the same per-entity updates. The zone-index rebuild derives each building's chunk,
     zone list, vacancy list, lot radius and support on the pool and fills the lists in building
     order from those compact rows, each chunk's list from a chunk-sorted copy with one map
     lookup a chunk, keeping the lists' storage; the admission order keys its rehousing homes on
     the pool and bulk-builds each level's sets. Every list, set and maximum equals the serial
     pass's. Per removal at 100k: evacuation 0.25 → 0.14, eviction 0.22 → 0.07, household
     invalidation 0.17 → 0.08 ms; the rebuild 2.62 → 2.08 ms, now 1.1–1.5 ms of it the parallel
     derivation (reading every building and the asset registry's capacity per home).
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass (360 s), no failures, the same eight
  notes; every `envelope` metric and every burst share bit-identical to the sixth commit's gate
  report; determinism shows no differences (repeat, save/reload continuation, 1 and 3 workers;
  10k, three days).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one session with the
  sixth commit as the control, both on copies of the same prepared cities (save format 105,
  unchanged). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 9.0 (8.0) | 1.13 (1.13) | 1.37 (1.40) | 0.48 (0.49) | 1 (1) | 0 (0) |
  | 10k | 32× | 2.3 (2.4) | 1.97 (1.93) | 2.22 (2.28) | 0.89 (0.89) | 0 (0) | 0 (0) |
  | 30k | 1× | 5.1 (5.0) | 1.82 (1.85) | 2.41 (2.56) | 0.90 (0.90) | 0 (0) | 0 (0) |
  | 30k | 32× | 5.6 (9.2) | 4.70 (4.97) | 5.32 (6.91) | 2.78 (2.83) | 0 (1) | 0 (0) |
  | 100k | 1× | 12.9 (14.6) | 6.06 (6.36) | 8.75 (9.93) | 4.02 (4.07) | 144 (178) | 0 (0) |
  | 100k | 32× | 25.4 (26.6) | 20.23 (21.75) | 23.27 (24.23) | 13.48 (14.01) | 2,700 (2,700) | 282 (452) |

  Every gate limit passes for both builds; means are level or lower, load times level (100k
  18.8 s). **100k 1×:** the maximum is 00:04 (12.9 ms, control 14.6), the first hour after
  loading: a cold construction start and a cold freight request (above) on top of the removal
  slot. In steady hours the worst are the residential slot's hour frames (10.3–11.0 ms: slot
  work 5.7–6.9, agent tick 3.4–4.0) and the minute-4 removal frames (10.3–10.7 ms); the actions
  frames at minute 3 (control 12.3–13.6 ms) left the worst frames; frames over 8 ms fall from
  178 to 144, p99.9 from 9.93 to 8.75 ms. **30k 32×:** p99 −5%, p99.9 −23%; the maximum is a
  removal-slot frame (5.6 ms at 23:04), hour frames 5.1–5.3 ms. **10k 1× and 30k 1×:** the
  maxima (9.0 and 5.1 ms) are an agent-and-snapshot frame and a snapshot frame with no slot
  work, above the control's (8.0 and 5.0 ms, also such frames). A recheck of the 10k and 30k
  tiers in the same session (`--perf07-tiers 10000,30000`, two rounds alternating the control
  and this build, each on a fresh copy of the same cities, every lock held), max ms round 1 /
  round 2, control in parentheses: 10k 1× 3.46 / 3.86 (3.90 / 8.08), 10k 32× 2.46 / 2.33
  (2.35 / 2.56), 30k 1× 4.82 / 4.70 (6.06 / 8.05), 30k 32× 5.45 / 5.76 (5.85 / 6.39); p99.9 at
  30k 32× 5.43 / 5.47 (5.54 / 6.31). The 9.0 ms frame did not recur: the preemption pattern of
  [Frame budget](#frame-budget-perf-07). 10k and 30k meet 8 ms at both speeds. **100k 32×:**
  25.4 ms (26.6), open question 6.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the sixth commit as
  control: hour means 1.21–1.22, 4.33–4.47, 8.11–8.14 and 27.50–28.05 ms (control 1.47,
  4.93–5.06, 8.27–8.50 and 28.46–28.49): the baseline hours are 11–18% cheaper, and the same
  work-count limits fail.
- **Tests.** 2,548 release and 2,548 debug library tests (two new) and the 4 growth integration
  tests pass; clippy adds nothing (61 warning lines, all in `agents/data.rs` and
  `asset_document.rs`); rustfmt adds nothing on the touched lines. `./run.sh --test` in an
  isolated profile passes (2,553 Rust tests, the Godot bridge tests, no script error, no
  segfault; the rendered vegetation tests skip without a Forward+ display); the change touches
  `nodes/sim/core`, which the bridge's load and save tests exercise.
  New: `kept_pass_rows_equal_a_full_recomputation_across_lendings` (a second lending after a
  recorded edit and an appended building keeps the rows; another policy or book derives them
  again; every lending is compared with a full recomputation),
  `kept_border_arrival_costs_follow_the_routing_revisions`. Changed:
  `a_stale_zone_index_defers_the_census_books_preparation` expects the book prepared at the
  second half's slot.
- **Build identity:** the gate, `PERF-07` and the suites ran on the committed sources built with
  rustc 1.99.1 (`$TEST_BINARY`); the control is the sixth commit's gate binary.
- **Save format:** 105, unchanged; nothing new is saved (the kept rows, the border sweeps and the
  rows generation are derived).
- **Memory:** the kept border sweeps (one cost table per car-connected border, about 2,200 nodes
  each at 100k) and the zone-index scratch (about 3.6 MB at 100k).

#### `SIM-02-K`, part 6 — 100k traffic (2026-10-21, findings)

Measured first, on copies of the prepared `PERF-07` cities (save format 105, day 12 00:00),
with a temporary diagnostic test that is not committed: every frame it follows each agent by
its render id from departure to arrival; every 1–8 game hours it counts agents by transit state,
mode and purpose, bins car speeds, sorts stopped cars by where they wait, measures each lane's
occupancy, builds the lane wait-for graph (a lane whose head car waits at its end for a lane
whose entry slot is taken) and looks for cycles in it, and reads every grocery's staff, stock
and position. Pinned (`taskset -c 0,2,…,14`), `RAYON_NUM_THREADS=8`, release, 32× over a full
day (two days for the prototype) and 1× over four hours.

**There is no gridlock.** The wait-for graph had no cycle in any sample at 10k, 30k or 100k,
over two days at 100k. Cars flow: at 100k 00:00, of 9,500 cars on lanes 4,100 run at 10 m/s or
more and 2,400 at 5–10 m/s; 1,300 stand in queues and 73 wait at a connector's end. The phase
inventory's "15,000 in junctions" are pedestrians: at 100k 17,600 agents are on junction
connectors and 1,900 of them are cars; the other 15,700 are walkers on crossings, where a
walker going north along a cross street spends about a third of its path (a junction every
50 m).

**The always-moving population is shoppers on foot, walking kilometres.** Day 12, 32×:

| | 10k | 30k | 100k |
| --- | --- | --- | --- |
| Agents | 11,100 | 31,400–32,400 | 114,500–117,200 |
| On the road (cars / walkers) | 2,600 (800 / 1,800), day mean | 8,700–14,900 (1,900–5,400 / 6,700–9,700) | 48,400–62,900 (9,100–16,600 / 39,000–46,300), day mean 52,900 |
| Of them on a shopping leg | 1,200 | 4,300–4,900 | 21,700–23,100 |
| Shopping legs completed in the day (walk) | — | 11,614, mean 503 game minutes | 35,306, mean 825 game minutes, 5,461 over 24 h |
| Home–store distance of legs in flight (walk) | — | mean 2.0 km | mean 3.2 km; 5,600 over 4 km, 660 under 500 m |
| Groceries / staffed / with stock | — | 882 / 145 / 92–115 | 3,023 / 525 / 362–420 |
| Homes with a grocery within 250 m | — | 100% | 97% |
| Homes with a stocked grocery within 250 m / beyond 1 km (1 home in 20) | — | 19% / 31–47% | 15% / 59–70% |
| Wait-for cycles (gridlock) | 0 | 0 | 0 |

A walking trip costs one game minute per second of movement at 4 m/s, so a 3.2 km shopping
walk takes about 13 game hours each way. Households wait meanwhile: at 100k 15,500 are on the
way to a store, 3,600 on the way home and 11,900 wait for a shopper to come home.

**Cause: the funding census funds only the stores first in building order.** Every staffed
grocery at 100k lies in the northernmost 1.6 km of the 6.4 km city (the first rows the fixture
filled); at 30k in the northernmost 0.8–1.6 km. In `reserve_consumer_outputs` each store, in
stable building order, claims its whole net output (200 units a day) from the households
nearest to it that still have budget (`ConsumerMarket::claim_reachable`, nearest-first by
512 m chunk ring, no distance bound; exhausted households leave the availability index, so the
next store's scan reaches past them). The fixture fills every commercial lot, so grocery
capacity is about six times demand (3,023 × 200 units a day against about 109,000): the first
sixth of the stores in building order claims every household in the city, nearest-first from
the north, and the rest get no customers, no funded jobs (7,360 of 45,345 at 100k, mean funded
activity 0.16), no workers and no stock. Shoppers then walk to the nearest stocked store, in the
north strip. Any city with more store capacity than demand behaves the same way: the oldest
stores take every customer, however far.

**Prototype, not landed:** the same scan stopped at the first chunk ring beyond 600 m
(`K6_CLAIM_CAP_M`, an environment variable read by the diagnostic build only).
- On the saved 100k city over two days at 32× (control in parentheses at the same clock):
  staffed groceries 830–890 through day 13 and 701 at its end (507–525); homes with a stocked
  grocery within 500 m 83% (20%); shopping legs over 4 km 3,154 (5,753) and under 500 m 1,623
  (642); walkers 37,700 (41,200). The saved city changes slowly: a leg takes most of a day.
- On cities grown under it (the fixture's 12 days, `perf_contract_prepare` with the variable set):
  30k built and passed the fixture's continuation check; 100k built but **failed the
  save/reload continuation check** ("snapshot agent state differs at continuation hour"),
  unexplained; its straight-through save was used for diagnostics only. On the grown 100k city,
  same pinned settings, one session, control first:

  | 100k | On the road | Walkers | On crossings | Agent tick mean (max) | Slot work mean |
  | --- | --- | --- | --- | --- | --- |
  | 1×, 00:00–04:00, control | 48,800 | 39,400 | 17,700 | 2.83 ms (7.8) | 0.23 ms |
  | 1×, 00:00–04:00, grown with cap | 30,900 | 22,300 | 8,500 | 2.41 ms (6.8) | 0.22 ms |
  | 32×, full day, control | 52,900 | 41,300 | 18,300 | 7.19 ms (15.9) | 5.97 ms |
  | 32×, full day, grown with cap | 36,100 | 25,000 | 9,500 | 6.46 ms (26.2) | 5.63 ms |

  Shopping legs in flight still average 2.0 km there: with production equal to the forecast and
  no stock buffer, a household's nearest stores are often sold out at the moment it reserves.
- Envelope (`tools/sim02_gate.py envelope --bursts departures,settlements`, fresh cities, the
  variable set): PASS, no failures, the same eight notes as `SIM-02-K` part 5's seventh commit;
  baseline 30k out-of-stock rate 0.184 → 0.137 (arrivals 0.175 → 0.114), net income per
  resident +2.3%, employment +0.3%, deliveries −1.1%, bankruptcies 0.29 → 0.22 per 1,000
  buildings a day, vacancy and unhoused rates 5e-5 → 8e-5 and 1.1e-4 → 2.2e-4 (a few
  households), the rest within ±0.5%; the eight notes are the base build's own (the collapsed
  fiscal-stress cities' employment and deliveries, and the 30k out-of-stock rate, now lower
  still); bursts within limits.

**Second finding: imports queue at the borders, and network entry stacks them.** Inbound trunk
lanes (the 538 m outside the city at 100k) fill over the PERF-07 day: 974 cars at 00:00–04:00,
2,869 at 24:00 (1,871 stopped), and freight on the road grows from 5,700 to 8,400 (9,500 by day
13). The queues' heads wait at the trunk's first junction, mostly for the turn onto a 39 m
cross-street block whose entry is taken. A truck that starts at a border node enters its first
lane at distance 0 without the entry-slot check every other lane acquisition makes
(`tick/movement/network/lane_entry.rs`, `prepare_lane_entry`), so once the lane is full the end
of tick overlap correction clamps the newcomers to distance 0: the saved cities already hold 8
(10k), 78 (30k) and 382 (100k) freight trucks stacked at one point of a trunk lane (max 78 on
one lane), and one 100k trunk lane held 421 cars at 24:00 where 199 fit. A prototype that makes network
entry check and claim the entry slot (a car that cannot enter waits at its node; not landed)
removes the stacks but not the queue: at 24:00 the 100k inbound trunk lanes hold 719 cars (60 stopped)
and 2,108 freight trucks wait at border nodes, against 2,869 cars (1,871 stopped) and 38
waiting without it; 10k: 323 waiting at its border against 374 cars on its trunk. Imports
exceed the borders' throughput at every tier, one single-lane trunk per 10,000 residents, and
the queue grows through the day. The waiting trucks are serial handoff agents every tick:
agent tick 7.37 against 7.19 ms at 32× and 2.91 against 2.83 ms at 1× (same session). Its
envelope passes with no determinism differences, but adds two notes at 10k: out-of-stock rate
0.095 → 0.122 (band up to 0.114) and fiscal stress 0.786 → 0.811 (up to 0.787), not yet
explained; 30k metrics move under 1%. Not landed (see "Decided while unattended").

**Frame budget after part 6** (no code landed; `SIM-02-K` part 5's seventh commit stands): 100k
`PERF-07` max 12.9 ms at 1× and 25.4 ms at 32×, p99 6.06 and 20.23 ms; the ordinary 100k 32×
frame is about 13.5 ms (agent tick 6.8, slot work 6.0, snapshot 0.9). Bearing on open question
6: the two runs above put the moving agents' share of the 32× agent tick at about 0.043 ms per
thousand agents on the road (0.023 ms at 1×). At 100k that is about 2.3 of 7.2 ms at 32×; a
healthy city with shopping trips of a few hundred metres (about 20,000 fewer walkers) would save
about 0.9 ms at 32× and 0.5 ms at 1× (an estimate: the two cities differ in more than their
walkers). The rest of the agent tick is per-agent passes over all
115,000 agents and trip planning (6.7 ms of CPU a 32× frame), which shorter trips do not remove.

#### `SIM-02-K`, part 5 — finance masks and zone-index fill (2026-10-21, eighth commit)

- **Attribution first.** The residential slot's re-proof of the constructing buildings
  (`reserve_projects`, 1.34–1.53 ms at 100k against 0.32–0.40 ms in the hour's later slots;
  diagnostic, pinned, 8 workers, 100k 1× over 190 game minutes on the `PERF-07` cities) was
  timed inside: of the 21 constructing buildings' proofs (1.1–1.3 ms), 0.79–0.85 ms is the
  business book's finance-mask refresh in the hour's first masked business claim. A census
  publishes every buyer's financing (`BusinessMarket::publish_financing`) and marks every buyer's
  finance masks for refresh, 76,127 at 100k, and nothing refreshed them before the residential
  slot's first claim, which refreshed them one buyer at a time; later lendings find 41. The
  rest, 0.12–0.33 ms, is the freight route cache repricing the 128–197 retained
  building-to-building quotes whose frontage prices the census start republished (geometry
  unchanged, 2–11 network searches). The earlier guess, cold freight-route quotes, is that
  remainder: the prices really changed. The removal slot's zone-index rebuild (2.10 ms at 100k)
  was split as well: the parallel derivation 1.27 ms, of it 0.65 ms collecting and sorting every
  building's (chunk, building) pair; registry lookups by asset id are about 0.1 ms of it.
- **What changed:**
  1. **The minute-59 preparation refreshes the census book's finance masks.** The preparation's
     second half (slot 953, or a full preparation) runs the same dirty-buyer refresh on the
     census book that the hour's first masked claim would run, so the residential slot refreshes
     only the buyers changed since (20–41). Finance masks only prune buyers that cannot claim at
     a price class (test builds assert after every pass that they are never narrower than
     exact), so no result changes. O(changed buyers' tranches and their bucket paths), moved
     from slot 0 to slot 953.
  2. **The zone-index rebuild fills the chunk lists over a dense chunk grid.** Instead of
     collecting every building's (chunk, building) pair and sorting them on the pool, the
     rebuild moves each chunk list into a dense grid over the chunk bounds (bounds reduced on
     the pool), pushes each building in building order in the ordered fill it already makes,
     and moves the lists back into the map, keeping their storage; a grid of more than four
     cells a building falls back to direct map fills. The lists equal the sorted fill's (each
     chunk's buildings in building order); test builds compare them with a direct fill at every
     rebuild. O(B/p + chunks) plus the O(B) ordered fill, against O(B/p + B log B / p).
- **Per slot** (diagnostic as above; before = the seventh commit's code with the same timing
  points, 100k hours 1–3 and 30k 32× hours 1–23; after = this commit with a timing point per
  scheduled system, 100k 1× over the full day, and with the finer points over the same spans as
  before for the split rows), means in ms:

  | Tier | Slot | Before | After |
  | --- | --- | --- | --- |
  | 100k 1× | residential (slot 0) | 4.22 | 3.42 |
  | 100k 1× | · its re-proof of constructing buildings | 1.34–1.53 | 0.57–0.72 |
  | 100k 1× | commercial (slot 16) | 2.93 | 3.06 |
  | 100k 1× | industrial (slot 32) | 3.86 | 3.74 |
  | 100k 1× | actions (slot 48) | 4.24 | 4.16 |
  | 100k 1× | removal window's first slot (`building removals`, slot 64) | 3.56 | 3.44 |
  | 100k 1× | · of it the zone-index rebuild | 2.10 | 1.51 |
  | 100k 1× | census book's preparation (slot 944) | 3.49 | 3.70 |
  | 100k 1× | its second half (slot 953) | 2.66 | 3.35 |
  | 30k 32× | residential | 1.33 | 1.07 |
  | 30k 32× | removal window's first slot | 1.17 | 0.93 |
  | 30k 32× | · of it the zone-index rebuild | 0.55 | 0.31 |
  | 30k 32× | census book's second half (slot 953) | 0.87 | 1.11 |

  Commercial, industrial, actions and slot 944 run unchanged code and vary by about ±0.3 ms
  between runs; slot 953 carries the moved refresh. The remaining 0.12–0.33 ms of the re-proof is
  the route cache repricing the quotes whose frontage prices really changed.
- **Not taken, with findings** (see [Decided while unattended](#decided-while-unattended)):
  registry rows kept across zone-index rebuilds (built, measured at about 0.1 ms, reverted); the
  zone lists for existing buildings and the lot scan; maintained workforce totals; the first
  hour after loading (below); slicing a slot further.
- **The first hour after loading at 100k** (diagnostic, not changed): 00:04 holds the first
  construction start (`demand spawn tick` 2.9–3.1 ms, against 0.5–0.6 ms an hour later) and the
  slot's freight requests (2.1–2.2 ms, against 0.25–1.2 ms in other slots of the hour) on top of
  the removal slot. The supplier roll has no front yet, so the first freight request builds the
  supplier index at once.
- **Attribution of the 100k 1× frames over 8 ms** (diagnostic day with the threads' scheduler
  statistics, the final tree with a timing point per scheduled system, pinned, full day): 191
  frames, of them 126 work frames, every one in the hour's slot minutes (:59 to :04, one at
  :30), with slot work of 4.2–4.9 ms (the minute's system 3.0–4.4 ms, restock requests and timed
  events 0.3–1.4 ms) on an agent tick of 3.6 ms and a snapshot of 0.8 ms, 8.0–10.7 ms; 53
  preempted frames (a pool worker waited 1.0–5.6 ms for a CPU; 8.0–20.5 ms); and 12 frames of
  about 10.5 ms of slot time outside every timed system, which are the diagnostic build's log
  writes waiting on writeback (`PERF-07` logs nothing and has none).
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass, no failures; every `envelope` metric
  and every burst share bit-identical to the seventh commit's gate report; determinism shows no
  differences.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one session with the
  seventh commit as the control, both on copies of the same prepared cities (save format 105,
  unchanged). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms |
  | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 4.6 (9.5) | 1.12 (1.14) | 1.36 (1.42) | 0.48 (0.48) | 0 (1) |
  | 10k | 32× | 3.6 (2.4) | 1.91 (1.96) | 2.46 (2.32) | 0.90 (0.91) | 0 (0) |
  | 30k | 1× | 7.3 (4.7) | 1.90 (1.84) | 2.41 (2.44) | 0.90 (0.89) | 0 (0) |
  | 30k | 32× | 7.3 (5.3) | 4.74 (4.71) | 5.25 (5.22) | 2.83 (2.79) | 0 (0) |
  | 100k | 1× | 13.0 (13.0) | 6.35 (6.26) | 8.78 (8.91) | 4.07 (4.06) | 163 (159) |
  | 100k | 32× | 27.2 (25.3) | 20.70 (20.49) | 24.74 (23.18) | 13.65 (13.48) | 2,700 (2,700) |

  Every gate limit passes for both builds. **100k 1×:** the hour frames' hourly part falls from
  4.8 to 3.7–3.9 ms, but those frames still total 10.4–11.6 ms, so the count over 8 ms and p99
  do not move (163 against 159; p99.9 8.78 against 8.91 ms). The maximum, 13.0 ms at 00:04, is
  the first hour after loading. **10k and 30k:** the full-day maxima above the control's were
  single agent frames (the preemption pattern of [Frame budget](#frame-budget-perf-07)). A
  recheck of the 10k and 30k tiers in the same session (two rounds alternating the control and
  this build, each on a fresh copy of the same cities), max ms round 1 / round 2, control in
  parentheses: 10k 1× 3.66 / 3.07 (7.66 / 4.06), 10k 32× 2.35 / 2.35 (2.76 / 2.35), 30k 1×
  3.93 / 5.00 (4.98 / 7.19), 30k 32× 5.43 / 6.25 (5.92 / 5.48); no frame over 8 ms in any round.
  10k and 30k meet 8 ms at both speeds. **100k 32×:** 27.2 ms (25.3), within the run-to-run
  variation of that tier (the seventh commit measured 25.4 and 26.6 ms), open question 6.
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the seventh commit as
  control: hour means 1.13–1.16, 4.07–4.08, 7.93–8.16 and 27.38–27.81 ms (control 1.20–1.40,
  4.33–4.46, 8.14–8.34 and 27.38–27.43); level to slightly lower, and the same work-count limits
  fail.
- **Build identity:** rustc 1.99.1; the gate, the suites, `PERF-07`, its recheck and
  `./run.sh --test` ran on the committed sources, `PERF-01` on the same binary in the
  coordinator's session; `PERF-07` pinned with `--cpus 0,2,4,6,8,10,12,14`,
  `RAYON_NUM_THREADS=8`, the control on the same prepared cities.
- **Tests.** 2,550 release and 2,550 debug library tests (two new) and the 4 growth integration
  tests pass; clippy adds nothing new. `./run.sh --test` in an isolated profile passes, with no
  script error. New: `the_census_books_preparation_refreshes_its_business_finance_masks` (the
  census leaves changed buyers; the preparation's first half keeps them, its second half
  refreshes them all) and `rebuilt_chunk_lists_hold_each_chunks_buildings_in_building_order` (the
  grid path, the direct path for chunks far apart, and back).
- **Save format:** 105, unchanged; nothing new is saved (the masks and the grid are derived).

#### `SIM-02-K`, part 5 — queue-free heavy slots (2026-10-21, ninth commit)

- **Measured first.** The eighth commit's diagnostic day (a timing point per scheduled system and
  the threads' scheduler statistics around every frame over 8 ms, 100k 1×, full day, pinned, 8
  workers) put every 100k 1× frame over 8 ms that was not preemption in one of the hour's seven
  heavy slots: the planning slots (0, 16, 32, 48), the removal window's first slot (64) and the
  census book's two preparation slots (944, 953), each holding its own system (3.0–4.4 ms) plus the
  slot's restock and job requests and trip events (0.3–1.4 ms) on a 3.6 ms agent tick and a 0.8 ms
  snapshot. The first hour after loading was timed inside the first construction start and the
  freight planning context (diagnostic, 100k 1×, the first 70 minutes): the first start cost 2.73 ms
  against 0.51–0.57 ms at the next two, of it 1.74 ms growing the site clients, 0.41–0.67 ms the
  entrance cache and 0.05 ms the zone index's vacancy positions; the freight requests after it 2.24
  ms, of it 2.20 ms growing the freight route cache's access rows. Each table had been rebuilt at
  load exactly to the building count (capacity 76,107 for 76,107 buildings), so the first appended
  building copied it. The supplier index was not built at once: the save holds a current supplier
  front.
- **What changed:**
  1. **The request queues serve nothing in the hour's seven heavy slots** (`QUEUE_FREE_SLOTS`,
     `nodes/sim/core/schedule.rs`). In each, every queue except the building removals (slot 64's own
     system) leaves its share to the next slot, which serves the skipped share first and then its
     own, so every queue keeps its quota per hour and each credit (visits, admissions) accrues and
     pays out as before, a slot later. A scheduling choice with a gameplay side; see [Decided while
     unattended](#decided-while-unattended). O(1) per slot for the rule; a skipped share costs its
     queue's own bound in the next slot.
  2. **Tables rebuilt at load keep spare rows.** The entrance cache and the site clients fill in
     place with the spare rows of `reserve_rows`, as do the zone index's vacancy positions and the
     freight route cache's access rows, so the first building appended after a load copies none of
     them. Capacity never changes a result: identical.
  3. **Trips and construction starts due in a heavy slot fire in the next** (`event_slot`), in the
     same game minute; the hour's first construction start moves from slot 64 to slot 65.
     Construction completions keep their deadlines.
- **Per slot** (diagnostic as above: before = the eighth commit's diagnostic day, after = this
  commit with the same per-system timing points; 100k 1× full day, 30k 32× full day, hours 1–23;
  slot work is the sum of the slot's systems over 0.2 ms, so a slot's small systems are left out),
  means in ms:

  | Slot | 100k before | 100k after | 30k 32× before | 30k 32× after |
  | --- | --- | --- | --- | --- |
  | 0 (residential) | 4.25 (system 3.42) | 3.37 (3.32) | 1.09 | 1.02 |
  | 1 | 0.49 | 0.99 | 0.00 | 0.04 |
  | 16 (commercial) | 3.44 (2.93) | 2.77 (2.77) | 0.95 | 0.88 |
  | 17 | 0.47 | 0.99 | 0.00 | 0.01 |
  | 32 (industrial) | 4.01 (3.59) | 3.59 (3.59) | 1.75 | 1.28 |
  | 33 | 0.47 | 0.91 | 0.00 | 0.00 |
  | 48 (actions) | 4.38 (3.99) | 3.88 (3.88) | 1.21 | 1.07 |
  | 49 | 0.44 | 0.80 | 0.00 | 0.00 |
  | 64 (removals) | 4.49 (3.30) | 3.30 (3.30) | 0.98 | 0.56 |
  | 65 | 0.98 | 2.42 (trip events and the construction start 1.10, cell lots 0.87) | 0.27 | 0.68 |
  | 944 (preparation) | 3.89 (3.54) | 3.49 (3.49) | 1.08 | 1.06 |
  | 945 | 0.37 | 0.73 | 0.00 | 0.00 |
  | 953 (its second half) | 3.58 (3.20) | 2.97 (2.97) | 1.06 | 1.05 |
  | 954 | 0.61 | 1.01 | 0.00 | 0.00 |

  The 30k "before" column is the eighth commit's finer diagnostic build over the same span; its
  industrial mean holds one stalled pass (8.9 ms). The systems themselves run unchanged code and
  vary by about 0.2 ms between runs; the slots after the heavy ones carry the moved shares.
- **The first hour after loading at 100k** (`PERF-07`): the 00:04 frame falls from 12.2–13.0 to
  under 8 ms (not among the day's worst frames); the first construction start costs what the later
  ones cost. The first hour's remaining frames over 8 ms are its first planning lendings after the
  load (00:01 commercial 4.8 ms, 00:02 industrial 4.8 ms, 00:03 actions 5.6 ms against 2.8, 3.6 and
  3.9 ms in steady hours; frames 9.2–10.5 ms, diagnostic): the commercial slot's pass rows are not
  kept across a load (`begin_live_pass` 0.87 ms against 0.06) and its re-proof of constructing
  buildings runs on cold freight route quotes (1.59 ms against 0.32). Not changed here; see the
  findings below.
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass (364 s), no failures, the same eight notes;
  determinism shows no differences (repeat, save/reload continuation, 1 and 3 workers; 10k, three
  days). Against the eighth commit, which is a scheduling change (items 1 and 3; item 2 is exact):
  net income per resident within 0.68% (30k baseline 12.93 → 12.84, 10k 13.56 → 13.62, arrivals
  12.82 → 12.88), employment within 0.71% (final employment 30k 0.658 → 0.653), jobs, households,
  residents and buildings within 0.2%, deliveries within 0.5% except the 10k fiscal-stress city
  (8.23 → 8.75, its startup freight, already a note), out-of-stock rates within 0.6%. The larger
  relative changes are small counts: bankruptcies about 6 → 4 events (10k baseline), 83 → 87 (30k
  baseline) and 81 → 86 (arrivals); unhoused households 1–4 of about 15,000; vacancy rates of a few
  homes; trip failures 0–6 events a city. Bursts pass: the worst minute holds at most 3.7% of an
  hour's departures (freight 3.2–3.6%, shopping 2.7–3.4%; 3.0–3.9% before), the worst settlement
  hour 7.4% (unchanged). Money residuals 0.004–0.044 (0.006–0.037 before).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one session with the
  eighth commit as the control, both on copies of the same prepared cities (save format 105,
  unchanged). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms |
  | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 4.1 (3.8) | 1.15 (1.14) | 1.40 (1.41) | 0.48 (0.49) | 0 (0) | 0 (0) |
  | 10k | 32× | 2.6 (3.9) | 1.96 (1.95) | 2.23 (2.67) | 0.90 (0.91) | 0 (0) | 0 (0) |
  | 30k | 1× | 8.0 (6.8) | 1.86 (1.91) | 2.35 (2.52) | 0.91 (0.90) | 0 (0) | 0 (0) |
  | 30k | 32× | 5.6 (5.6) | 4.79 (4.78) | 5.35 (5.42) | 2.86 (2.82) | 0 (0) | 0 (0) |
  | 100k | 1× | 12.2 (15.2) | 6.24 (6.27) | 8.15 (8.58) | 4.11 (4.04) | 111 (146) | 0 (0) |
  | 100k | 32× | 25.3 (25.1) | 21.49 (20.44) | 24.46 (23.52) | 13.69 (13.63) | 2,700 (2,700) | 336 (320) |

  Every gate limit passes for both builds; means are level (within 1.7%), load times level. **100k
  1×:** frames over 8 ms fall from 146 to 111 and p99.9 from 8.58 to 8.15 ms; the maximum is a
  preempted agent frame (06:07, agent 9.8 ms, slot work 0.9 ms; the control's 15.2 ms at 12:59
  likewise, agent 12.4 ms); the worst work frames are 04:03 10.8 ms (actions slot, agent 5.0 ms) and
  the first hour's actions slot (00:03, 10.1 ms). A diagnostic day with the threads' scheduler
  statistics (this commit with a timing point per scheduled system, pinned, full day) attributes its
  129 frames over 8 ms (191 in the eighth commit's): **86 work frames**, every one a heavy slot's
  frame (actions 23, industrial 17, preparation 15, removals 15, residential 11, commercial 4, the
  preparation's second half 1; three of them the first hour's first lendings), with slot work
  3.3–4.2 ms (the slot's system alone, 2.8–4.5 ms), an agent tick of 3.6–4.1 ms and a snapshot of
  0.8–1.1 ms, 8.0–10.8 ms (mean 8.6); **34 preempted frames** (a pool worker waited 1.2–7.2 ms for a
  CPU; 8.0–11.9 ms); **9 log stalls** (about 10.5 ms of slot time outside every timed system, the
  diagnostic build's logging; `PERF-07` logs nothing). No frame of a slot after a heavy slot is over
  8 ms without preemption. **10k and 30k:** the maxima above the control's (10k 1× 4.1, 30k 1× 8.0
  ms) are single agent frames with no slot work (agent 3.4 and 7.0 ms); a recheck in the same
  session (`--perf07-tiers 10000,30000`, two rounds alternating the control and this build, each on
  a fresh copy of the same cities, every lock held, then two more rounds of 10k alone), max ms by
  round, control in parentheses: 10k 1× 3.54 / 8.16 / 5.75 / 3.29 (4.65 / 3.19 / 5.19 / 3.59), 10k
  32× 5.06 / 2.82 / 2.37 / 5.14 (3.56 / 3.51 / 2.33 / 2.36), 30k 1× 5.50 / 5.23 (6.07 / 6.94), 30k
  32× 6.27 / 5.51 (5.70 / 5.94); p99 and p99.9 level. The full day's 30k 1× 8.0 ms (an agent frame)
  did not recur. One rechecked frame exceeded 8 ms, 10k 1× round 2's 8.16 ms: an agent tick of 6.2
  ms with no slot work, where a 10k agent tick is 0.2–0.7 ms of work, the preemption pattern of
  [Frame budget](#frame-budget-perf-07) (the eighth commit's control showed a 9.5 ms one); nothing
  this commit changes runs in that frame. 10k and 30k meet 8 ms at both speeds apart from preempted
  frames. **100k 32×:** 25.3 ms (25.1), agent-heavy frames (21:38, agent 12.7 ms, slots 11.3 ms),
  within the tier's run-to-run variation (open question 6).

- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the eighth commit as
  control: hour means 1.18–1.22, 4.22–4.24, 8.42–8.91 and 27.92–27.93 ms (control 1.17–1.21,
  4.09–4.31, 8.24–8.76 and 27.55–27.86); level, and the same work-count limits fail.

- **Tests.** 2,553 release and 2,553 debug library tests (three new) and the 4 growth integration
  tests pass; clippy adds nothing (61 warning lines, all in `agents/data.rs` and
  `asset_document.rs`); rustfmt adds nothing on the touched lines. `./run.sh --test` in an isolated
  profile passes (2,558 Rust tests, the Godot bridge tests, no script error, no segfault; the
  rendered vegetation tests skip without a Forward+ display). New:
  `queue_free_slots_pass_their_share_to_the_next_slot` (an hour's shares sum to one a slot, none in
  a queue-free slot, two after; trips and construction starts move to the next slot in the same
  minute), `rebuilt_building_tables_keep_spare_rows_for_the_next_building`,
  `building_access_rows_keep_spare_capacity_for_the_next_building`. Changed:
  `an_arrival_hired_at_the_next_hour_joins_the_snapshot_as_before_its_hire` steps to the hour's
  second slot, where the queued searches now run.
- **Build identity:** rustc 1.99.1; the gate, the suites, `PERF-07`, its recheck and `./run.sh
  --test` ran on the committed sources (`$TEST_BINARY`); `PERF-07` pinned with `--cpus
  0,2,4,6,8,10,12,14` and `RAYON_NUM_THREADS=8`, the control (the eighth commit's binary) on the
  same prepared cities.
- **Save format:** 105, unchanged; nothing new is saved (the rule depends on the slot alone; event
  slots are saved as before, and events a save holds keep their slots).

#### `SIM-02-K`, part 5 — planning in two halves (2026-10-22, tenth commit)

- **Measured first** (the ninth commit's diagnostic day, 100k 1×, full day, pinned, 8 workers):
  the four planning slots held 2.8–3.9 ms each, of which the pure reads of the live city were
  about half: the workforce count with the snapshot (0.5–0.7 ms), the existing-building scan
  (0.47–0.8) and the lot scan (0.24–0.8). The books' site preparation (0.5), the queue's re-proof
  (0.3–0.55), the earlier slots' re-proof with the workforce reservations (0.4–0.6) and the
  decisions (at most 0.35) were the rest; the actions slot's execution took 1.26 ms. The census
  book's preparation, first half (slot 944, 3.5 ms) spent about 0.85 ms building the book's
  planning pass rows, which its previous lending (the live book's) could not keep.
- **What changed** (a scheduling choice with a gameplay side; see
  [Decided while unattended](#decided-while-unattended)):
  1. **Setup and plan.** Each planning slot's setup runs at its minute's first slot (0, 16, 32,
     48): it counts the live workforce and runs the plan's scans of the live city, the zone's
     free lots (ranked by the commercial demand a snapshot composed now leaves) and existing
     buildings, or for the actions slot every zone's downgrade candidates, and holds them
     (`demand/setup.rs`). The plan, `PLANNING_SETUP_LEAD` (8) slots later in the same minute (8,
     24, 40, 56; `PlanningSlot`'s value), lends the books, prepares their site rows, proves the
     queue and the earlier slots' plans again and decides on what the setup held. The actions
     slot then executes as before, now at slot 56, so spawns, level changes and clearances are
     queued and stamped 8 slots later and the first admission comes at slot 57. A plan without a
     held setup (a world's first hour, a clock set directly, a test driver) counts and scans
     itself, as before. Every setup and plan slot is queue-free (`QUEUE_FREE_SLOTS` has eleven
     entries: 0, 8, 16, 24, 32, 40, 48, 56, 64, 944 and 953; the compile-time checks still hold).
  2. **Pass rows built half a minute before the census book's preparation**
     (`PLANNING_PREPARATION_ROWS_SLOT`, slot 936): a lending that only opens and closes a pass,
     so the supplier half at 944 keeps the rows and recomputes only those of the buildings
     changed since. The rows are derived and keyed: exact. The slot is not queue-free.
  3. **Saves** (format 106): a save between a setup and its plan keeps the setup
     (`demand_planning_setup`, one blob: the workforce, the spawn sites and the building
     candidates by dense index, whose keys a load takes from the loaded buildings). The
     census-restore rule follows `PlanningSlot::Actions`, so saves in slots 0–55 keep the
     published census (0–47 before). The Kuopio fixture gained the table and its version, and its
     hash in `placements.json` follows.
- **Per slot** (diagnostic as above, hours 1–23; before = the ninth commit's day on its cities,
  after = this commit's day on the cities it prepared; mean slot work in ms, the system itself in
  parentheses where the slot holds more):

  | Slot | 100k before | 100k after | 30k 32× before | 30k 32× after |
  | --- | --- | --- | --- | --- |
  | 0 | residential 3.37 (3.32) | residential setup 1.71 (1.66) | 1.02 | 0.57 |
  | 8 | — | residential plan 1.76 | — | 0.58 |
  | 16 | commercial 2.77 | commercial setup 1.51 | 0.88 | 0.41 |
  | 24 | — | commercial plan 1.61 | — | 0.54 |
  | 32 | industrial 3.59 | industrial setup 2.04 | 1.28 | 0.67 |
  | 40 | — | industrial plan 1.76 | — | 0.51 |
  | 48 | actions 3.88 | actions setup 1.24 | 1.07 | 0.34 |
  | 56 | — | actions plan and execution 2.97 | — | 0.76 |
  | 64 | removals 3.30 | removals 3.42 | 0.56 | 0.40 |
  | 936 | — | the census book's pass rows 0.85 | — | — |
  | 944 | preparation 3.49 | preparation 2.53 | — | — |
  | 953 | second half 2.97 | second half 2.95 | 1.05 | 1.02 |

  The slots after the queue-free ones serve the skipped shares (0.5–1.0 ms); slot 65 keeps 2.4 ms
  (trip events with the hour's first construction start, the cell-lot region the removals
  dirtied, two queue shares).
- **Attribution of the 100k 1× frames over 8 ms** (diagnostic day with a timing point per
  scheduled system and the threads' scheduler statistics, pinned, full day). On the cities this
  build prepared: **21 work frames** (8.0–14.2 ms):
  - the removal window's first slot, 11 (removals 3.3–4.7 ms; frames 8.1–9.6 ms);
  - census slices, 6 (4.1–9.6 ms of work by one pool worker, at six slots of two hours,
    04:40–04:53 and 23:43–23:52, the same slots in two runs; frames 8.5–14.2 ms);
  - the census book's preparation, 1 at slot 944 and 1 at slot 953;
  - one residential setup frame of 8.0 ms (setup 2.0 ms with a 0.8 ms worker wait and a 1.6 ms
    snapshot);
  - one agent frame of 12.4 ms just after a 226 ms frame that held 220 ms outside every timed
    system, a stall of the diagnostic run that recurs neither in the other diagnostic day nor in
    `PERF-07`.

  Also **36 preempted** frames and **5 log stalls**. No planning half and no first-hour lending
  is among the work frames. On copies of the ninth commit's cities (save 105, given the new table
  and version) the same build's day has **18 work frames**, all 8.0–9.2 ms: the removal window's
  first slot 15, slot 944 2, slot 953 1; 39 preempted, 10 log stalls; no census slice over 2 ms.
  The heavy census slices therefore belong to the city this build prepared, not to running this
  code on a given city: the census code is unchanged, and its slices there are 0.3–0.9 ms in
  other hours.
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass (362 s), no failures, the same eight
  notes; determinism shows no differences (repeat, save/reload continuation, 1 and 3 workers;
  10k, three days). Against the ninth commit (a scheduling change; item 2 is exact): net income
  per resident within 0.50% (fiscal 10k 27.29 → 27.15, baseline 10k 13.62 → 13.59, baseline 30k
  12.84 → 12.89, arrivals 12.88 → 12.92), employment within 0.33%, households, residents, jobs
  and buildings within 0.24%, deliveries within 0.32% except the 10k fiscal-stress city (8.75 →
  8.44, its startup freight, already a note), out-of-stock rates within 0.26%. The larger
  relative changes are small counts: bankruptcies about 4 → 2 events (10k baseline), about 87 →
  70 (30k baseline, 0.305 → 0.246 per 1,000 buildings a day, inside its band) and 86 → 80
  (arrivals); vacancy and unhoused rates of a few homes; trip failures about 6 → 9 events
  (fiscal 30k). Bursts pass: the worst minute holds at most 4.3% of an hour's departures (the
  fiscal 10k city's shopping; 3.7% before), the worst settlement hour 7.4% (unchanged).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day), in one session with the
  ninth commit as the control. The save format changed, so the control ran on its own cities
  (save 105) and this build on the cities it prepared (save 106). Milliseconds and frame counts,
  control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 7.7 (8.5) | 1.14 (1.14) | 1.38 (1.41) | 0.49 (0.48) | 0 (1) | 0 (0) | 1.8 (1.8) |
  | 10k | 32× | 2.4 (2.3) | 2.08 (1.97) | 2.40 (2.21) | 0.89 (0.89) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 30k | 1× | 5.1 (8.6) | 1.84 (1.87) | 2.21 (2.34) | 0.88 (0.90) | 0 (1) | 0 (0) | 4.9 (4.9) |
  | 30k | 32× | 6.5 (7.8) | 4.69 (4.80) | 5.90 (5.46) | 2.79 (2.87) | 0 (0) | 0 (0) | 5.0 (4.9) |
  | 100k | 1× | 14.2 (11.4) | 6.18 (6.15) | 7.42 (8.01) | 4.03 (4.07) | 36 (88) | 0 (0) | 18.8 (18.6) |
  | 100k | 32× | 27.1 (24.7) | 20.65 (20.49) | 24.88 (23.12) | 13.71 (13.56) | 2,700 (2,700) | 297 (299) | 18.6 (18.7) |

  Every gate limit passes for both builds, and 10k and 30k meet 8 ms at both speeds; means are
  level. **100k 1×:** frames over 8 ms fall from 88 to 36 and p99.9 from 8.01 to 7.42 ms; the hour
  frames peak at 7.75 ms. The maximum is a census slice of this build's city (04:46, 14.2 ms with
  9.9 ms of slot work; then 13.3 ms at 04:40, 13.0 at 23:52, 10.6 at 04:53, 10.4 at 23:44, the
  slices of the attribution above), then a preempted agent frame (19:38, agent 10.5 ms, no slot
  work). It is within the gate's 25% allowance on the maximum against the control's 11.4 ms. The
  control's worst are its planning frames (00:03 11.4 ms, the first actions slot after loading;
  23:00 11.2 ms, residential). **10k and 30k:** the maxima are single agent or snapshot frames
  without slot work (10k 1× 7.7 ms at 04:50, agent 6.9; 30k 1× 5.1 ms at 06:02, snapshot 4.4),
  the control's two frames over 8 ms likewise (agent 8.0 and 6.2 ms). The 10k 32× maximum above
  the control's (2.4 against 2.3 ms) was noise: a recheck in the same session (two rounds
  alternating the control and this build, each on fresh copies of its cities, every lock held,
  `--perf07-tiers 10000`) gave max ms, 10k 1× 3.86 / 6.64 (8.80 / 4.77) and 10k 32× 2.35 / 2.17
  (2.60 / 8.66), with p99 and p99.9 level; the control's two frames over 8 ms are single agent
  frames. **100k 32×:** 27.1 ms (24.7), slot-heavy frames (19:52, slots 20.3 ms, agent 5.9 ms),
  within the tier's run-to-run variation (open question 6).

- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the ninth commit as
  control, each build on cities it prepared: hour means 1.13–1.14, 3.96–4.17, 8.58–8.75 and
  26.78–26.91 ms (control 1.19–1.41, 4.16–4.30, 8.28 and 27.34–27.37); level within run-to-run
  spread (the 10k fiscal hour 3–6% higher, the 30k one 2% lower), and the same work-count limits
  fail.

- **Tests.** 2,555 release and 2,555 debug library tests (two new) and the 4 growth integration
  tests pass; clippy adds nothing (61 warning lines, all in `agents/data.rs` and
  `asset_document.rs`); rustfmt adds nothing on the touched lines. `./run.sh --test` in an
  isolated profile passes (2,560 Rust tests, the Godot bridge tests with no failures, no script
  error, no segfault; the rendered vegetation tests skip without a Forward+ display); the save
  format changed, which the bridge's save and load tests exercise. New:
  `a_planning_setup_continues_identically_after_save_load_inside_its_span` (a save and load at
  each setup slot and just before each plan holds and saves the same setup, and both games plan
  to the same hour plan, construction queue, demand queues and diagnostics) and
  `a_plan_on_its_setup_equals_one_that_scans_an_unchanged_city_itself` (two copies of a game at
  each setup slot plan at once, one on the held setup, one without, to the same tables). Changed:
  `the_input_copy_follows_building_and_household_removals` removes buildings no shopping
  household uses; the new schedule's busy town had a shopper out from a home the test removed,
  whose household then advances its search cursor and drops its store, live state the census
  input copy keeps as copied (the copy is a snapshot, not a mirror of in-place changes).
- **Build identity:** rustc 1.99.1; the gate, the suites and `./run.sh --test` ran on the
  committed sources (`$TEST_BINARY`). `PERF-07` ran pinned (`--cpus 0,2,4,6,8,10,12,14`,
  `RAYON_NUM_THREADS=8`), each build on cities it prepared (the save format changed). This
  build's cities were prepared by its first build, whose only simulation difference is the exact
  slot-936 lending.
- **Save format:** 106 (`demand_planning_setup`).

#### `SIM-02-K`, part 5 — counted business claims and a kept zone index (2026-10-22, eleventh
commit)

- **Measured first** (the tenth commit's code with a timing point per census step call, the tenth
  commit's 100k cities, full day, pinned, 8 workers; diagnostic): every heavy census slice of the
  day was the business validation's **claims** step. It ran in row slices at a fixed 20 work units
  a seller, and on this city in two hours (the censuses publishing at 05:00 and 00:00) the
  nearest-buyer scans of the sellers past about building 52,000 cost 0.4–1.4 µs a seller, with
  almost no route searches: slices planned at about 0.5 ms ran 4.0–10.6 ms on one worker. No
  single seller took over 0.25 ms. In the other hours the claims' scans end at once (about 30 ns a
  seller). The removal window's first slot (slot 64) held 3.3–4.6 ms: about 0.7 ms a removed
  building (evacuation, eviction, the agents', households' and freight remaps), the zone-index
  rebuild 1.3–1.5 ms (0.4–0.55 ms of it the admission order) and the site publication 0.3–1.7 ms
  (its terrain boxes).
- **What changed** (both exact: no result changes; see [Decided while
  unattended](#decided-while-unattended)):
  1. **The business claims are a serial counted sweep.** The business book counts the scan events
     its claims visit and the carrier quotes they price (monotonic counters, one add each, never
     read by a result), and the census weighs them as work units (270 and 150 nominal ns, fitted
     to the claims' slices: about 285 ns a scan event and 14 ns a seller's row at 100k). The
     claims step now stops a slice at the seller whose counted work reaches the budget, as the
     household-claim and staffing sweeps do, and its rate observed so far prices the rows left.
     Sellers are claimed in the same order; only the slice boundaries move. A storage shortfall
     still grows the arena and seeds again.
  2. **The removal slot keeps the zone index across its swap-removes.** A clearance's swap-remove
     applies to a current zone index (`keep_index_across_swap_remove`): the removed building
     leaves its zone and chunk lists and the last building takes its place there (both stay in
     building order), the vacancy lists drop it and rename the last building, and the
     explicit-site count and the counts of buildings at the support and lot-radius maxima follow
     (new: `max_building_support_count`, `max_lot_radius_count`, kept by rebuilds and appends).
     The removal queue's repair (`rebuild_zone_index_after_removals`) then gives what a rebuild
     gives without deriving every building: the vacancy lists back in building order (`ALLOC-03`)
     in O(B + V) on reused storage, the admission order recomputed, the maxima only when a removal
     took a maximum's last building, the chunk bounds only when an edge chunk emptied. Anything
     else that marks the index stale drops the kept state (level changes, placements, a completion
     on a stale index, a stale saved order, an incompatible building's removal); an append after a
     kept removal waits for the rebuild as before; every other caller of `rebuild_zone_index`
     still rebuilds in full. A swap-remove that finds an entry where the index's invariants do not
     put it leaves the index to a full rebuild. Test builds with the whole-city checks compare the
     kept index with a rebuild after every repair (lists, positions, chunk lists and bounds,
     maxima bit for bit with their counts, the admission order), and the rebuild then stands.
- **Census slices** (diagnostic, the same cities, before = the tenth commit's code with the same
  timing points): before, seven census slots of 4.0–10.6 ms (the business claims above); after, no
  census slot over 2.24 ms, the largest the expansion staffing sweep's slices (2.0–2.1 ms, about
  twice its weights' estimate, unchanged) and the window's first slot; the business claims' slices
  are at most 0.83 ms.
- **Removal slot** (diagnostic, slot 64, mean over the day's 24 hours / maximum, ms):

  | | Before | After |
  | --- | --- | --- |
  | building removals (the slot's system) | 3.46 / 4.63 | 2.65 / 4.35 |
  | · of it the zone index | 1.27–1.55 | 0.59 / 0.85 (admission order 0.55, vacancy lists 0.03, maxima never recomputed) |
  | · of it the site publication | 0.70 / 1.72 | 0.72 / 1.72 (unchanged) |
  | · per removed building | about 0.7 | about 0.7 (unchanged) |

  The other heavy slots are unchanged within the runs' variation (slot 65 2.48 to 2.55 ms, slot
  953 3.15 to 3.24 ms, the planning halves within 0.1 ms). Slot 944 read 2.61 to 2.94 ms across
  the session's diagnostic days in code this commit does not touch, with the census book's results
  identical; it is taken as run-to-run variation of a memory-bound phase (not resolved).
- **Attribution of the 100k 1x frames over 8 ms** (schedstat diagnostic day, the tenth commit's
  100k cities): **9 work frames** (8.0–9.7 ms): the removal window's first slot 4 (removals
  2.95–4.35 ms; the worst is an hour with three removals), the census book's preparation at slot
  944 4 (2.9–3.3 ms) and the first actions plan after loading 1 (00:03, 4.4 ms); **24 preempted**
  frames; **18 diagnostic-log stalls** (a thread blocked off its CPU for about 10 ms while writing
  the diagnostic log; none occurs without the log). No census slice and no planning half is among
  the work frames. The tenth commit's code had 17 work frames, 6 of them census slices.
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): the envelope is bit-identical to the tenth
  commit's, and so are the bursts; determinism shows no differences. The `PERF-07` day's 100k
  end-of-day city records are identical to the control's.
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day, one session with the
  tenth commit as the control; the save format is unchanged, so both builds ran on copies of the
  same prepared cities). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms |
  | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 3.6 (4.4) | 1.14 (1.11) | 1.35 (1.33) | 0.49 (0.48) | 0 (0) |
  | 10k | 32× | 2.3 (2.3) | 1.88 (1.88) | 2.21 (2.10) | 0.88 (0.87) | 0 (0) |
  | 30k | 1× | 4.4 (7.6) | 1.87 (1.88) | 2.34 (2.35) | 0.89 (0.89) | 0 (0) |
  | 30k | 32× | 6.5 (6.6) | 4.66 (4.66) | 5.95 (5.76) | 2.85 (2.80) | 0 (0) |
  | 100k | 1× | 11.7 (14.2) | 6.20 (6.10) | 7.43 (7.32) | 4.02 (4.04) | 37 (30) |
  | 100k | 32× | 27.0 (26.4) | 21.65 (20.76) | 26.47 (25.05) | 13.69 (13.64) | 2,700 (2,700) |

  Every gate limit passes and means are level. **100k 1x:** the control's census-slice frames are
  gone, and this build's maximum is a preempted agent or snapshot frame. The count of frames over
  8 ms (37 against 30) follows preemption: a rerun of the 100k 1x tier in the same session gave
  the control 57 frames over 8 ms (max 13.7 ms) and this build 39 (max 11.9 ms), so the count
  varies by about a factor of two between runs of one build. **10k and 30k** stay under 8 ms at
  both speeds. **100k 32x:** 27.0 ms (26.4), within the tier's run-to-run variation (open question
  6).

- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the tenth commit as
  control: hour means 1.13–1.14, 4.06–4.10, 8.57–9.05 and 27.14–27.21 ms (control 1.12–1.14,
  3.99, 8.52–8.99 and 26.90–27.11); level, and the same work-count limits fail.

- **Tests.** 2,556 release and 2,556 debug library tests (one new) and the 4 growth integration
  tests pass; clippy adds nothing; `./run.sh --test` in an isolated profile passes with no script
  errors. New: `a_zone_index_kept_across_removals_equals_a_rebuild` (claims take homes out of
  building order, four bulldozes, the last building first, then a home, a store and a full home,
  each keeping the index; the repair equals a rebuild, the support maximum's last building and an
  edge chunk included). Changed: `a_census_restarted_after_any_slice_equals_one_run_bit_for_bit`
  also requires a restart inside `business_claims`.
- **Build identity:** rustc 1.99.1; the gate, the suites, `PERF-07` and `./run.sh --test` ran on
  the committed sources (`$TEST_BINARY`); `PERF-07` pinned with `--cpus 0,2,4,6,8,10,12,14` and
  `RAYON_NUM_THREADS=8`, both builds on copies of the same prepared cities.
- **Save format:** unchanged (106); the kept index and the claim counters are derived and never
  saved.

#### `SIM-02-K`, part 5 — one scan a removal and the preparation in parts (2026-10-22, twelfth
commit)

- **Measured first** (the eleventh commit's code with a timing point per scheduled system over
  0.2 ms, inside the removal slot, the census book's preparation, the supplier layout and the
  terrain-box refresh, every staffing-sweep slice with its counted searches, and the threads'
  scheduler statistics around every frame over 8 ms; the tenth commit's 100k cities, full day,
  pinned, 8 workers; diagnostic, day D1): ten 100k 1× work frames over 8 ms (8.1–8.5 ms): the
  removal window's first slot four times (slot 64, 2.7–3.7 ms), the census book's preparation
  five times (slot 944 three, 2.8–3.0 ms; slot 953 two, 3.3–3.5 ms) and one industrial planning
  setup (slot 32, 2.6 ms); 56 preempted frames and 8 diagnostic-log stalls. Inside them:
  - **Removals** (42 in the day, about 0.69 ms each): three scans of every agent per removal
    (the evacuation's 0.12 ms, the eviction's 0.09 ms and the agent remap's eight column passes,
    0.14 ms) and two of every household (invalidation 0.07, remap 0.06 ms), each finding the
    few agents or households that name the removed or the moved building; the freight remap
    0.09 ms. The slot's site publication (0.69 / 1.42 ms) was all its terrain boxes, and of each
    box's 0.25–0.55 ms the road grading margins of its one to three render patches took nearly
    all (the site ownership loop 0.02–0.05 ms): render state that a building removal leaves as
    it was, recomputed for every box.
  - **The census book's preparation** (four parts of about 1 ms each): the supplier rows'
    factory 0.8–1.1 ms (incremental) and their layout 1.1–1.4 ms (offsets 0.14, entries 0.36,
    the supplier index 0.74 ms) at slot 944, the buyer rows' factory 0.4–0.8 and layout 0.9–1.2
    ms and the business finance masks 0.75–0.82 ms at slot 953, each lending 0.6–0.7 ms besides;
    slot 936's lending (the pass rows) 0.82 ms.
  - **Census expansion slices:** 51 of 1,254 over 1.2 ms (at most 2.20 ms). In them every row
    was an operation (85–120) with 6,000–8,700 supplier candidates and 5,000–7,800 priced
    supplier routes; a least-squares fit gives about 230 ns a priced route in the expansion sweep
    against 25–30 in the current-staff sweep, whose slices fit their weights.
  - **The first actions plan after loading** (00:03, 4.08 ms against 2.8–3.4 later): its
    admission's border arrival sweeps (1.15 ms), which later hours keep per routing revision.
- **What changed** (exact for the simulation except slot 936's queue-free entry, a scheduling
  change with a gameplay side; the terrain boxes' later refresh is render state; see
  [Decided while unattended](#decided-while-unattended)):
  1. **One scan of the agents and one of the households per removal.** A clearance finds, in one
     parallel scan, the agents naming the removed building or the last building (which the
     swap-remove moves into its index), and in another the households naming either
     (`AgentSystem::agents_referencing_buildings`,
     `HouseholdSystem::households_referencing_buildings`). The evacuation, eviction and agent
     remap, and the household invalidation and remap, then visit those alone
     (`evacuate_building_among`, `evict_building_among`, `remap_building_among`,
     `invalidate_building_among`): no other agent or household names either building, and none
     comes to name the moved one, since only the visited ones change. A cancelled shipment that
     removes its carrier renumbers agents, so the agent remap then scans every agent as before.
     The freight remap rebuilds its request-failure map only when a key moves. Test builds with
     the whole-city checks assert after every such remap that no agent or household names the
     moved building's old index. Other removal paths (the bulldozer, incompatible buildings)
     keep the full scans.
  2. **The removal slot's terrain boxes follow in later slots.** The removal queue publishes its
     clearances' production sites, yards and cell-lot marks at once and queues each site box's
     terrain render-patch ownership (`deferred_site_terrain_boxes`), which one box at a time is
     refreshed in each light slot (a slot serving a single queue share: neither queue-free nor
     after one), while the zone index and site clients are current. The ownership is render state
     no simulation system reads and each refresh recomputes its patches from the live roads and
     sites; a load recomputes it whole. At 1× the terrain under a cleared building updates two
     to four slots (0.1–0.25 s) later; at 32× in the same frame or the next.
  3. **The census book's preparation in four parts.** Slot 936's lending, which built the pass
     rows, now also runs the supplier rows' factory (`PlanningPreparation::SupplierRows`, through
     `RecurringSupply::prepare_projected_rows`, which marks the layout stale); slot 944 lays the
     supplier rows out (`Suppliers`); slot 953 prepares the rest (`Whole { masks: false }`) and
     slot 957 (`PLANNING_PREPARATION_MASKS_SLOT`) refreshes the business finance masks; a second
     half that runs later than 953 (a stale zone index) refreshes them itself, and loading
     prepares the whole at once. Slot 936 joins the queue-free slots (twelve: 0, 8, 16, 24, 32,
     40, 48, 56, 64, 936, 944, 953; the compile-time checks hold), so its queue shares and the
     trips and construction starts due there move to slot 937. The supplier index's resources are
     built on the pool and large bucket indices sort on the pool (both give the serial result).
     The rows are derived and keyed, so every part's result is the one the single preparation
     gave.
  4. **The expansion sweep's priced routes weigh 230 work units** (`EXPANSION_ROUTE_EXTRA`, 200
     on top of `WORK_WEIGHTS`'s 30); the current-staff sweep keeps 30. Only slice boundaries
     move.
  5. **Admission's border arrival sweeps are prepared at load** (`prepare_loaded_pathing`), so
     the first admission batch after loading finds them current.
- **Per slot** (diagnostic, D1 = the eleventh commit's code, D2 = this commit's, the same cities;
  slot work in ms, mean / maximum over the day's 24 hours):

  | Slot | Before | After |
  | --- | --- | --- |
  | 56 actions plan | 3.03 / 4.08 | 3.02 / 3.38 |
  | 64 removals | 2.40 / 3.67 | 1.26 / 1.90 |
  | · per removal | 0.69 | 0.37 (agent scan 0.13, household scan 0.10, freight 0.11) |
  | · site publication | 0.69 / 1.42 | 0.003 (boxes deferred) |
  | 65 | 2.44 / 2.93 | 2.46 / 3.22 |
  | 66, 67 (deferred boxes, 0.44 / 0.86 ms each) | 0.68 / 1.31, 0.56 / 1.21 | 1.03 / 1.84, 0.80 / 1.86 |
  | 936 rows (and now supplier rows) | 1.11 / 1.44 | 2.03 / 2.57 (queue-free) |
  | 944 supplier rows and layout (now layout) | 2.65 / 3.04 | 1.69 / 1.89 |
  | 953 second half | 3.13 / 3.80 | 2.33 / 2.80 |
  | 957 (now the finance masks) | 0.52 / 1.01 | 1.24 / 1.83 |

  Census expansion slices: at most 2.20 → 1.69 ms, p99 1.78 → 1.35 ms, slices over 1.2 ms 51 →
  28; current-staff slices unchanged (at most 1.6 ms). The first actions plan's admission: 1.15 →
  0.03 ms, and that actions plan (2.98 ms) now equals later hours.
- **Attribution of the 100k 1× frames over 8 ms** (diagnostic day D2, threads' scheduler
  statistics): **1 work frame** (22:02, 8.0 ms: the industrial planning setup, slot 32, 2.9 ms on
  a 4.1 ms agent tick; the same slot held D1's 06:02 frame), **39 preempted**, **8 log stalls**
  (a thread blocked off its CPU for about 10 ms writing the diagnostic log, five of them inside
  the census system). No removal, preparation, census-slice or first-lending frame remains.
  The D1 day of the eleventh commit's code had ten work frames (slot 64 four, slot 944 three,
  slot 953 two, slot 32 one), 56 preempted, 8 log stalls.
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): pass, no failures, the same eight notes;
  determinism shows no differences (repeat, save/reload continuation, 1 and 3 workers; 10k,
  three days). Every change but one is exact: a build of this commit with slot 936 left serving
  queues gives an envelope bit-identical to the eleventh commit's (every metric of every city,
  every burst). The differences against the eleventh commit therefore all come from slot 936
  joining the queue-free slots, where the queue shares and the trips and construction starts due
  at 936 move to slot 937 (62.5 ms of game time at 1×): net income per resident within 0.74%
  (fiscal 10k 27.15 → 27.35, baseline 10k 13.59 → 13.67, the 30k cities within 0.06%),
  employment within 0.13%, households, residents, jobs and buildings within 0.31%, deliveries
  within 0.24% except the fiscal-stress cities' startup freight (+2.6% and +2.8%, already
  notes), out-of-stock rates within 0.40%. The larger relative changes are small counts:
  bankruptcies in the 30k baseline 0.246 → 0.281 per 1,000 buildings a day (about 70 → 80
  events, inside its band) and arrivals 0.281 → 0.295; vacancy and unhoused rates of a few
  homes; trip failures of a few events. Bursts pass: the worst departure minute holds at most
  3.8% of an hour's departures (baseline 10k freight; 3.6% before), the worst settlement hour
  7.4% (unchanged).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day; in one session with the
  eleventh commit as the control; the save format is unchanged, so both builds ran on copies of
  the same prepared cities). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 3.8 (3.7) | 1.14 (1.13) | 1.39 (1.36) | 0.49 (0.49) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 10k | 32× | 4.0 (2.4) | 2.03 (1.89) | 2.66 (2.13) | 0.91 (0.87) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 30k | 1× | 6.5 (8.3) | 1.85 (1.87) | 2.22 (2.26) | 0.89 (0.90) | 0 (1) | 0 (0) | 4.9 (5.0) |
  | 30k | 32× | 6.2 (6.7) | 4.75 (4.75) | 5.96 (5.83) | 2.85 (2.84) | 0 (0) | 0 (0) | 5.0 (5.0) |
  | 100k | 1× | 12.5 (10.6) | 6.08 (6.22) | 7.11 (7.37) | 4.05 (4.04) | 9 (31) | 0 (0) | 18.7 (18.7) |
  | 100k | 32× | 24.3 (26.1) | 20.17 (21.16) | 22.50 (25.63) | 13.61 (13.63) | 2,700 (2,700) | 290 (292) | 18.7 (18.7) |

  Every gate limit passes for both builds; means are level. **100k 1×:** frames over 8 ms fall
  from 31 to 9 and p99.9 from 7.37 to 7.11 ms. Eight of the nine follow the preemption pattern:
  little or no slot work, with an inflated agent tick (5.7–8.7 ms) or snapshot (3.3–4.8 ms); the
  maximum, 00:03 12.5 ms, is an 8.7 ms agent tick with 2.9 ms of slot work. The ninth, 00:03
  8.6 ms, is a work frame: the actions plan with 3.4 ms of slot work on a 4.0 ms agent tick. The
  control's frames include five of 2.0–3.2 ms slot work (the removal slot, slot 944, a planning
  slot). **10k and 30k** stay under 8 ms at both speeds. The 10k maxima rose above the control's,
  so the tier was run twice more for both builds (`--perf07-tiers 10000 --perf07-speeds 1,32`,
  fresh city copies, every lock held): 1× 4.92 and 3.88 ms (control 7.34 and 8.07), 32× 3.31 and
  2.41 ms (2.09 and 2.68), p99 level. The main run's 10k maxima are taken as preemption: a 2.5
  ms agent tick where it is normally 0.7, and in one run one minute-59 frame. This is unproven,
  because gate runs have no scheduler statistics. A 100k 1× recheck in the same session
  (`--perf07-tiers 100000 --perf07-speeds 1`, fresh city copies, every lock held, the control
  first) gave the control 36 frames over 8 ms (max 17.7 ms) and this build 19 (max 13.4 ms), a
  noisier session for both. **100k 32×:** 24.3 ms (26.1), slot-heavy frames (open question 6).
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the eleventh commit as
  control: hour means 1.13–1.14, 4.15–4.41, 8.72–8.78 and 27.85–28.46 ms (control 1.14–1.23,
  4.02–4.14, 8.64–8.72 and 26.84–27.39). The 30k hours are 3–5% dearer: in fiscal stress the
  planning phase (436 against 413–419 ms a day, the preparation's extra parts) and the census
  (366 against 350); in the baseline the census is 6% cheaper (539–547 against 576–583). The
  slot-936 scheduling change also shifts the workload slightly (30k logistics calls 6,010
  against 5,869). The hour means stay far inside the `SIM-02-A` reference, and the same
  work-count limits fail.
- **Tests.** 2,557 release and 2,557 debug library tests (one new) and the 4 growth integration
  tests pass; clippy adds nothing; `./run.sh --test` in an isolated profile passes (no script
  error). New: `a_removal_over_its_scanned_agents_equals_the_full_scans` (agents naming the
  removed and the moved building in different columns; eviction and remap over the scanned
  candidates equal the full scans). Changed:
  `the_census_books_preparation_refreshes_its_business_finance_masks` (the second half in its
  own slot leaves the masks; the masks slot refreshes them).
- **Build identity:** rustc 1.99.1; the gate, the suites, `PERF-07` and `./run.sh --test` ran on
  the committed sources (`$TEST_BINARY`); `PERF-07` pinned with `--cpus 0,2,4,6,8,10,12,14` and
  `RAYON_NUM_THREADS=8`, both builds on copies of the same prepared cities.
- **Save format:** unchanged (106); the deferred boxes, the candidate sets and the preparation
  parts are derived and never saved.

#### `SIM-02-K`, part 5 — setup scans beside the count (2026-10-22, thirteenth commit)

- **Measured first** (the twelfth commit's code, the tenth commit's 100k cities, pinned, 8
  workers; diagnostic):
  - **The planning setups** (slots 0, 16, 32, 48; 1.2-2.2 ms mean, the industrial one 2.1 / 2.9
    ms): the live workforce count, 0.5-0.9 ms, then the scans, 0.5-1.5 ms (industrial lots
    1.0-1.3 ms and existing buildings 0.9-1.2 ms), one after the other; the site index, road
    surface and queue refresh about 0. Inside the count: the housed-unemployed pass over the
    households (random reads of each home building) about 1.0 ms, the displaced list 0.2 ms, the
    jobs fold 0.14 ms. The scans need the snapshot only for the commercial lots' resource
    priorities, which come from its output absorption and unmet commercial demand: `compose`
    derives both without the workforce, and the queue's reservation does not read it.
  - **The actions slot** (slot 56, 2.9-3.4 ms): its plan 1.6 ms and the execution 1.3 ms, of
    which the construction epoch's site preparation 0.48, workforce reset 0.47, project market
    0.27, existing claims 0.08 and pass start 0.05 ms. Every lending's site preparation (about
    eight an hour, 0.45-0.55 ms) counted the free lots for its storage estimates, 0.14 ms each.
  - **The workforce reset** (every plan and the epoch, five a game hour): two passes over the
    buildings (the latest construction hour 0.12 ms, then the homes 0.44 ms), the second adding
    every free place's workers to shared atomic bins.
  - **The lot scan** looks every candidate pose up in the site feasibility cache, one mutex for
    every stored parcel, from all pool workers at once.
  - **The first residential plan after a load** (3.6-3.9 ms against 1.7-1.9 later): its pass
    rows 0.8 ms, rebuilt whole because loading lends the census storage's warm copy after the
    live books (the rows are kept per book, so the plan found the copy's), and its queue re-proof
    1.8 against 0.36-0.54 ms later (not changed).
- **What changed** (all exact; see [Decided while unattended](#decided-while-unattended)):
  1. **The setup's scans run beside the workforce count** (`rayon::join`), on a snapshot composed
     with an empty workforce: the scans read only its output absorption and unmet commercial
     demand. Test builds with the whole-city checks compose both snapshots and assert that their
     priorities are bit-identical. A zone setup's lot scan and existing-building scan also run
     side by side, and the count's three passes overlap.
  2. **A workforce reset passes over the buildings once.** One fold sums, in thread-local bins by
     construction hours left, the arrival places' workers and members, the ready places and the
     constructing or not yet operating workplaces' jobs, and collects the homes displaced
     households may claim; the bins merge in building-range order and are stored into the shared
     bins once. The sums wrap as the atomic adds did, so they are equal in any order; the homes
     keep building order. O(B / p + Q + hours).
  3. **The stored-parcel site feasibility cache is split into 32 locks by parcel**, so the lot
     scan's lookups rarely wait on one another; the hourly prune visits every shard. The cache is
     derived and each pose's verdict is validated before reuse, so no verdict changes.
  4. **The free-lot storage estimate is kept** while the parcels' layout and occupancy revisions
     and count hold (`ProjectReservations::free_lot_estimate`): estimates size storage and never
     change a result.
  5. **Loading lends the live books last.** Without a finished census, loading prepares the live
     books, gives census storage its warm copy and prepares it, and now prepares the live books
     once more, so the first planning slot after the load finds their pass rows kept. With a
     finished census the order is unchanged (that book is published next and lent last).
- **Per slot** (diagnostic, the instrumented twelfth-commit day against this commit's
  instrumented day, the same cities, full day, slot work in ms, mean / maximum; the load change
  is not in the instrumented day; the slot 8 and slot 56 figures cover the first item only):

  | Slot | Before | After |
  | --- | --- | --- |
  | 0 residential setup | 1.71 / 2.11 | 1.47 / 2.11 |
  | 16 commercial setup | 1.56 / 2.20 | 1.28 / 1.96 |
  | 32 industrial setup | 2.16 / 2.90 | 1.49 / 1.95 |
  | 48 actions setup | 1.27 / 1.49 | 1.17 / 1.55 |
  | 8 residential plan | 1.90 / 3.92 | 1.84 / 3.57 |
  | 24 commercial plan | 1.67 / 2.19 | 1.52 / 1.69 |
  | 40 industrial plan | 1.75 / 2.09 | 1.63 / 2.07 |
  | 56 actions | 3.02 / 3.38 | 2.78 / 3.38 |
  | 936 / 944 / 953 census-book parts | 2.03 / 2.57, 1.69 / 1.89, 2.33 / 2.80 | 2.08 / 2.47, 1.58 / 2.01, 2.13 / 2.53 |

  A lending's site preparation 0.50 → 0.35 ms (median); the epoch's reset 0.47 → 0.40 ms. The
  first residential plan after the load keeps its pass rows (0.8 → 0.09 ms in a short
  instrumented run); its queue re-proof stays about 1.8 ms.
- **Attribution of the 100k 1× frames over 8 ms** (this commit's instrumented day, threads'
  scheduler statistics): **0 work frames**; all 28 frames over 8 ms are **preempted**, and 9 of
  them are **log stalls** (a thread blocked writing the diagnostic log, about 10 ms, one of them
  in the removal slot). Some preempted frames carry real slot work besides a worker's wait of
  1.2-7.1 ms: 10:03 8.1 ms (the actions slot, 3.4 ms), 14:31 8.7 ms (3.4 ms), 15:23 10.7 ms
  (restock and job requests, 5.6 ms inflated by a 7.1 ms wait), 18:00 8.0 ms (the residential
  setup, 2.1 ms), 22:01 / 22:02 9.3 / 8.9 ms (the commercial and industrial setups, 1.5 / 1.7
  ms), 23:32 8.3 ms (a census slice, 2.5 ms). The twelfth commit's day had 1 work frame (the
  industrial setup, 2.9 ms), 39 preempted, 8 stalls.
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): PASS for the first four changes (425 s) and
  for the whole commit (364 s); **every metric and burst of every city is bit-identical to the
  twelfth commit's**, the same eight notes; determinism shows no differences (repeat,
  save/reload continuation, 1 and 3 workers; 10k, three days).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day; in one session with the
  twelfth commit as the control; the save format is unchanged, so both ran on copies of the
  cities the tenth commit prepared). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 5.5 (9.2) | 1.15 (1.16) | 1.38 (1.43) | 0.49 (0.49) | 0 (1) | 0 (0) | 1.8 (1.8) |
  | 10k | 32× | 3.1 (2.2) | 1.93 (1.88) | 2.41 (2.17) | 0.90 (0.88) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 30k | 1× | 7.0 (12.6) | 1.88 (1.91) | 2.33 (2.33) | 0.88 (0.90) | 0 (1) | 0 (0) | 5.0 (4.9) |
  | 30k | 32× | 5.7 (6.0) | 4.67 (4.67) | 5.49 (5.71) | 2.80 (2.81) | 0 (0) | 0 (0) | 5.0 (5.2) |
  | 100k | 1× | 12.3 (10.7) | 6.21 (6.10) | 7.29 (7.15) | 4.08 (4.04) | 27 (20) | 0 (0) | 18.7 (18.6) |
  | 100k | 32× | 24.2 (24.3) | 20.12 (20.14) | 23.01 (22.24) | 13.62 (13.68) | 2,700 (2,700) | 284 (321) | 18.8 (18.7) |

  Every gate limit passes for both builds; means are level. **10k and 30k** stay under 8 ms at
  both speeds; the 30k maxima fall (7.0 and 5.7 ms against 12.6 and 6.0, the control's 1×
  maximum a 12.1 ms agent tick). The 10k 32× maximum rose above the control's (3.1 against 2.2
  ms; the frame a 2.6 ms render snapshot with 0.3 ms of slot work), so the tier was run twice for
  both builds (`--perf07-tiers 10000 --perf07-speeds 1,32`, fresh city copies, every lock held,
  the control first): 32× 3.78 and 2.20 ms (control 2.25 and 2.39), p99.9 level; the 3.78 ms
  frame holds 1.6 ms of measured phases (2.2 ms outside them), and the next frame of that run is
  2.25 ms, so it is taken as preemption. **100k 1×:** 27 frames over 8 ms (20), p99.9 7.29 ms
  (7.15); a recheck in the same session (`--perf07-tiers 100000 --perf07-speeds 1`, fresh city
  copies, every lock held, the control first) gave 28 (35), p99.9 7.25 (7.46), max 13.6 (12.0).
  Across both sessions the count is 27-28 for this build against 20-35 for the control: run-to-run
  noise, larger than the difference between the builds. Of the recorded 12 worst frames, the work
  frames are the actions slot (frames of 9.3-10.0 ms with 2.8-3.4 ms of slot work on 4.5-5 ms
  agent ticks) and one removal-window frame (21:04 8.8 ms); the rest follow the preemption
  pattern (an agent tick of 5.2-8.3 ms or a snapshot of 2.0-6.2 ms with under 2 ms of slot
  work). The control's include the same actions slot (06:03 9.8 ms, 3.3 ms) and its minute-4
  frame. **100k 32×:** 24.2 ms (24.3), slot-heavy frames (open question 6).
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the twelfth commit as
  control: hour means 0.93–0.95, 3.64–3.67, 8.14–8.33 and 26.37–26.70 ms (control 1.15,
  4.14–4.16, 8.75–8.86 and 27.92–28.45): the baseline hours 12–19% cheaper and the fiscal-stress
  hours 5–7%, and the same work-count limits fail.
- **Tests.** 2,557 release and 2,557 debug library tests and the 4 growth integration tests pass
  (no new test: every change is exact, the bit-identical envelope with 1- and 3-worker
  determinism covers them, and the setup's independence from the workforce has a test-build
  assertion); clippy adds nothing; `./run.sh --test` in an isolated profile passes (no script
  error).
- **Build identity:** rustc 1.99.1; the gate, the suites, `PERF-07` and `./run.sh --test` ran on
  the committed sources (`$TEST_BINARY`); `PERF-07` pinned with `--cpus 0,2,4,6,8,10,12,14` and
  `RAYON_NUM_THREADS=8`, both builds on copies of the same prepared cities.
- **Save format:** unchanged (106); the cache shards, the estimate and the lending order are
  derived and never saved.

#### `SIM-02-K`, part 5 — frontage sums by lane range (2026-10-22, fourteenth commit)

- **Measured first** (the thirteenth commit's code, the tenth commit's 100k cities, pinned, 8
  workers; diagnostic: every frame of a 100k 1× day with the agent tick split into its
  sub-phases, and 6 game hours with the `economy_phases` channel):
  - **The agent tick does not rise in minutes 3 and 4.** By minute of the hour its mean is
    2.98–3.21 ms (minutes 3 and 4: 3.05 and 3.10 ms), and path searches stay at 14–16 a frame
    in every minute: no departures, admissions or arrivals cluster there. By hour of day it is
    2.79–3.63 ms, highest in the commute hours (05:00, 06:00, 13:00, 21:00, 22:00: 3.38–3.63).
    Two frame kinds carry more: **the frame that crosses a game minute** (3.65 ms against 2.94
    for other slot frames), whose tick runs the once-a-simulated-second frontage delay update,
    0.71 ms there and nothing elsewhere; at 1× a simulated second is a game minute, so the
    update falls in the frame of every minute's first slot, which is the frame of the planning
    setups (0, 16, 32, 48), the removal slot (64) and the census book's part at 944. And **the
    frame after a slot frame** (3.27 against 2.97 ms after a frame without slot work: the
    movement pass 1.48 against 1.22 ms, its data evicted by the slot work). The actions slot's
    own frames (slot 56, mid-minute) hold an agent tick at the hour's median; the 4.5–5.0 ms
    ticks in `PERF-07`'s worst actions-slot frames were commute hours plus preemption (a
    22:03 window of 4.8–5.6 ms ticks in the diagnostic day had every neighbouring frame as
    high).
  - **The frontage update** (184,092 lanes, 115,000 agents at 100k): a serial pass over the
    agents adding each car's speed to its lane (about 0.27 ms; serial so each lane adds its
    cars in agent order), the scratch clear (0.07 ms) and a parallel pass over the lanes, which
    reads each lane's and its edge's records (0.34 ms).
  - **The planning slots' workforce reset** (every plan, 0.4–0.5 ms) passes over the buildings
    after the site preparation (0.35–0.5 ms) and the queue's replay (0.36 ms), though its free
    places read neither the books nor the queue; only the constructing workplaces' jobs read the
    replay's workers.
- **What changed** (both exact; see [Decided while unattended](#decided-while-unattended)):
  1. **The frontage delays sum by lane range on the pool.** Each agent's lane is noted in
     parallel (none for an agent that is not a car on a lane, 4 bytes an agent); the lanes then
     split into one range per worker, and each range's task zeroes its sums, reads the notes in
     agent order, adds the cars on its own lanes and updates those lanes' delays. Each lane's
     cars are added in agent order whatever the split, so the sums, and the delays, are the
     serial pass's bit for bit with any worker count. O(agents / p) for the notes, O(agents) a
     range task over the notes, O(lanes / p). The update: 0.68 → 0.45 ms at 1× (240 updates),
     0.72 → 0.47 ms an update at 32× (diagnostic).
  2. **The planning slots scan the free places beside their site preparation.** The reset's
     pass over the buildings splits into a free-place scan (every building's construction hours
     left and its free places, for the snapshot: `WorkforceReservations::scan_free_places`),
     which each planning slot (8, 24, 40, 56) runs beside its site preparation and queue replay
     (`rayon::join`), and the rest (`reset_scanned`), which adds the constructing or not yet
     operating workplaces' jobs from the books' constructing rows (the replay's list of every
     building that predicate selects) instead of a second test of every building. The sums
     wrap, so they are equal in any order; test builds with the whole-city checks assert that
     the rows hold every constructing building. The construction epoch keeps its own order (its
     overlap measured no gain: its site preparation, market and scan are each well parallel).
- **Per slot** (diagnostic; the schedstat days of the thirteenth commit and of this commit,
  the same cities, full day, slot work in ms, mean / maximum, log stalls dropped):

  | Slot | Before | After |
  | --- | --- | --- |
  | 8 residential plan | 1.84 / 3.57 | 1.57 / 2.64 |
  | 24 commercial plan | 1.52 / 1.69 | 1.35 / 1.51 |
  | 40 industrial plan | 1.63 / 2.07 | 1.46 / 1.87 |
  | 56 actions | 2.78 / 3.38 | 2.72 / 2.89 |
  | 0, 16, 32, 48 setups | 1.47, 1.28, 1.49, 1.17 | 1.61, 1.22, 1.50, 1.09 |
  | 64, 65 | 1.33 / 1.76, 2.51 / 3.27 | 1.30 / 1.74, 2.56 / 3.16 |
  | 936, 944, 953 | 2.08 / 2.47, 1.58 / 2.01, 2.13 / 2.53 | 2.07 / 2.53, 1.58 / 1.87, 2.16 / 2.48 |

  In 6 game hours with the `economy_phases` channel (the thirteenth commit against a build
  with both overlaps): the actions slot 2.74 → 2.56 ms mean, the commercial plan 1.50 → 1.32,
  the industrial 1.64 → 1.42, the residential 1.87 → 1.77; the epoch 1.19 → 1.18 ms (so its
  overlap was not kept). Every minute-first frame loses the frontage update's 0.24 ms.
- **Attribution of the 100k 1× frames over 8 ms** (this commit's instrumented day, threads'
  scheduler statistics): every frame over 8 ms that carries slot work is **preempted**. Of 46
  frames over 8 ms, **23 are preempted** (a pool worker waiting 1.9–7.6 ms for a CPU), **22 are
  log stalls** (a thread blocked about 10.5 ms writing the diagnostic log; 9 in the thirteenth
  commit's day) and **1 is an agent-tick frame** without slot work or a waiting worker (21:45,
  8.1 ms: a 6.8 ms tick in the evening commute window, where both builds' days hold their
  largest ticks). Preempted frames that carry slot work: 20:03 8.2 ms (the actions slot, 2.84
  ms, wait 2.1 ms), 20:04 8.2 ms (slot 65, 3.1 ms, wait 1.9 ms) and 21:59 8.9 ms (slot 953, 2.5
  ms, wait 4.1 ms).
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): PASS (361 s), no failures, the same eight
  notes; **every metric and burst of every city is bit-identical to the thirteenth commit's**;
  determinism shows no differences (repeat, save/reload continuation, 1 and 3 workers; 10k,
  three days).
- **`PERF-07`** (`tools/sim02_gate.py perf07 --cpus 0,2,4,6,8,10,12,14 --binary "$TEST_BINARY"`
  against the pinned `SIM-02-A` report, `RAYON_NUM_THREADS=8`, full day; in one session with the
  thirteenth commit as the control; the save format is unchanged, so both ran on copies of the
  cities the tenth commit prepared). Milliseconds and frame counts, control in parentheses:

  | Tier | Speed | Max | p99 | p99.9 | Mean | Frames > 8 ms | > 16.7 ms | Load s |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 2.0 (7.7) | 1.12 (1.16) | 1.31 (1.41) | 0.48 (0.49) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 10k | 32× | 2.3 (2.2) | 1.92 (1.95) | 2.15 (2.16) | 0.87 (0.89) | 0 (0) | 0 (0) | 1.8 (1.8) |
  | 30k | 1× | 6.7 (4.7) | 1.87 (1.84) | 2.22 (2.21) | 0.88 (0.89) | 0 (0) | 0 (0) | 4.9 (5.0) |
  | 30k | 32× | 7.5 (6.0) | 4.82 (4.73) | 5.89 (5.61) | 2.76 (2.82) | 0 (0) | 0 (0) | 4.9 (5.0) |
  | 100k | 1× | 16.0 (10.2) | 6.20 (6.05) | 7.47 (7.09) | 4.01 (4.02) | 36 (20) | 0 (0) | 19.0 (18.8) |
  | 100k | 32× | 29.4 (24.3) | 19.57 (20.02) | 22.58 (22.27) | 13.47 (13.62) | 2,700 (2,700) | 252 (291) | 18.6 (18.7) |

  Every gate limit passes for both builds; means and p99 are level or lower (100k 32× mean 13.62
  → 13.47 ms: the frontage update runs in about every second 32× frame). **10k and 30k** stay
  under 8 ms at both speeds. The 30k maxima and the 10k 32× maximum rose above the control's, so
  10k and 30k were run twice more for both builds and 100k 1× once more (`--perf07-tiers
  10000,30000 --perf07-speeds 1,32` and `--perf07-tiers 100000 --perf07-speeds 1`, fresh city
  copies, every lock held, the control first): 10k 1× 3.14 / 6.53 ms (control 5.19 / 2.99), 10k
  32× 2.14 / 2.27 (2.94 / 3.22), 30k 1× 15.02 / 4.00 (6.82 / 4.52), 30k 32× 5.92 / 6.61 (6.23 /
  6.17), p99 and p99.9 level. The raised 10k and 30k maxima are single preemption-pattern
  frames: an agent tick of 5.1–11.9 ms (the tiers' mean frames are 0.5–0.9 ms) with under 2 ms
  of slot work; no 10k or 30k frame over 8 ms is a work frame. **100k 1×:** 36 frames over 8 ms
  (20), p99.9 7.47 ms (7.09); the recheck gave 19 (12), p99.9 7.14 (7.04), max 11.06 (11.58).
  The count moved between 12 and 36 across this and the last session's runs of either build.
  The 12 worst frames of both runs show the preemption pattern (agent ticks of 5.2–12.7 ms,
  slot work at most 2.2 ms); no actions-slot or removal-window frame is among them (the
  control's main run has one, 14:04 9.1 ms, the removal window with 4.4 ms of slot work). The
  frontage change adds about 0.1 ms of parallel phase per simulated second, too little to
  change how often a preempted worker lands in one. **100k 32×:** 29.4 ms (24.3), a frame with
  a 12.9 ms agent tick (about 6.5 normally); the steady worst frames are 21.6–23.7 ms of
  slot-heavy work (open question 6).
- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with the thirteenth commit as
  control: hour means 0.82–0.83, 3.38–3.39, 8.28–8.33 and 25.82–26.64 ms (control 0.91–0.95,
  3.68–3.69, 8.47–8.49 and 26.55–26.67): the baseline hours 8–12% cheaper, fiscal stress level,
  and the same work-count limits fail.
- **Tests.** 2,558 release and 2,558 debug library tests (one new) and the 4 growth integration
  tests pass; clippy adds nothing. New: `frontage_delay_sums_are_independent_of_the_worker_count`
  (2,000 cars over a two-edge road's vehicle lanes with order-sensitive f32 speeds; the delays
  of 1, 3 and 8 workers are bit-identical). `./run.sh --test` was not needed: nothing in the
  bridge or the save format changed.
- **Build identity:** rustc 1.99.1; the gate, the suites and `PERF-07` ran on the committed
  sources (`$TEST_BINARY`); `PERF-07` pinned with `--cpus 0,2,4,6,8,10,12,14` and
  `RAYON_NUM_THREADS=8`, both builds on copies of the same prepared cities; `./run.sh --test`
  was not needed, because nothing in the bridge or the save format changed.
- **Save format:** unchanged (106); the lane notes and the free-place scan are scratch.

#### `SIM-02-K`, part 7 — acceptance (2026-10-22)

Stopped for review under the part's rule: 100k at 32× cannot reach 8 ms without changing a
decision of this document ([open question 6](#open-questions)), and whether the acceptance reads
the raw maximum or the maximum over frames no other task preempted is the user's decision ([open
question 7](#open-questions)). No simulation code changed.

- **What changed (measurement only, test builds).** `PERF-07` reads the run-queue wait of every
  shared-pool worker and of the frame's own thread before and after each timed frame, outside
  the timed span, and counts a frame whose largest wait growth is 0.5 ms or more as preempted
  (the [Frame budget](#frame-budget-perf-07) bullet "Preemption columns" lists the new fields).
  `tools/sim02_gate.py perf07` prints the split in each tier's line and a table of the runs; the
  gate's limits stay on the raw maximum and p99, so earlier gates keep their meaning.
- **Threshold** (see [Decided while unattended](#decided-while-unattended)): frames with under
  0.5 ms of wait are 0.0–0.3 ms slower on average than frames with none, those with 1–2 ms
  1.0–1.6 ms slower; `max_unpreempted_ms` is the same at any threshold from 0.1 to 0.5 ms in
  every run below; 1 ms and 2 ms let preempted frames into it, 0.05 ms drops work frames.
- **Overhead** (same build, `METRUM_PERF07_SCHEDSTAT=0` against the reads, alternating, two
  rounds each, 30k, one session): means 0.877 → 0.873 ms at 1× and 2.799 → 2.771 ms at 32×
  (−0.4% and −1.0%, within the 2.4% and 1.2% between rounds with the reads off); agent tick,
  slot work, snapshot and p99 move under 2% the same way; the reads add about 4 µs a frame to
  the run's wall time, outside the timed span. Two builds of the same simulation code differ by
  about 4% in the 30k 1× mean (code layout), so timing comparisons keep a same-session control.
- **Equivalence** (`tools/sim02_gate.py envelope determinism --bursts departures,settlements
  --binary "$TEST_BINARY"`, `RAYON_NUM_THREADS=8`): PASS (361 s), no failures, the same eight
  notes as part 5's fourteenth commit; every day record of every city, with its fingerprint,
  bursts, money residual and invariant counts, is bit-identical to that commit's gate (only the
  runs' wall-clock seconds differ); determinism shows no differences (repeat, save/reload
  continuation, 1 and 3 workers; 10k, three days). The burst limits pass.
- **`PERF-07`, two full days on the final code** (`tools/sim02_gate.py perf07 --cpus
  0,2,4,6,8,10,12,14 --binary "$TEST_BINARY" --perf07-baseline <the pinned SIM-02-A report>`,
  `RAYON_NUM_THREADS=8`, every lock held, the machine otherwise idle, fresh copies of the cities
  part 5's tenth commit prepared; every gate limit passes). Milliseconds and frame counts; a
  frame is preempted when one of its threads waited 0.5 ms or more for a CPU; the last column is
  the largest preempted frame less its wait:

  Run 1:

  | Tier | Speed | Max | Max unpreempted | p99 | p99.9 | Mean | Frames > 8 ms (preempted / unpreempted) | > 16.7 ms | Preempted frames | Preempted less wait |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 8.3 | 2.20 | 1.14 | 1.38 | 0.48 | 1 (1 / 0) | 0 | 165 / 86,376 | 1.6 |
  | 10k | 32× | 2.6 | 2.13 | 1.98 | 2.45 | 0.87 | 0 | 0 | 35 / 2,700 | 1.9 |
  | 30k | 1× | 5.1 | 3.19 | 1.86 | 2.21 | 0.87 | 0 | 0 | 1,528 / 86,376 | 2.5 |
  | 30k | 32× | 5.9 | 5.87 | 4.73 | 5.77 | 2.80 | 0 | 0 | 207 / 2,700 | 5.0 |
  | 100k | 1× | 14.1 | 7.35 | 6.27 | 7.51 | 4.04 | 28 (28 / 0) | 0 | 4,686 / 86,376 | 6.2 |
  | 100k | 32× | 24.4 | 24.28 | 20.22 | 22.51 | 13.63 | 2,700 (242 / 2,458) | 314 | 242 / 2,700 | 20.6 |

  Run 2:

  | Tier | Speed | Max | Max unpreempted | p99 | p99.9 | Mean | Frames > 8 ms (preempted / unpreempted) | > 16.7 ms | Preempted frames | Preempted less wait |
  | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
  | 10k | 1× | 5.1 | 1.63 | 1.13 | 1.36 | 0.48 | 0 | 0 | 374 / 86,376 | 1.6 |
  | 10k | 32× | 3.7 | 2.38 | 1.90 | 2.51 | 0.88 | 0 | 0 | 40 / 2,700 | 0.9 |
  | 30k | 1× | 4.6 | 2.56 | 1.83 | 2.17 | 0.87 | 0 | 0 | 1,208 / 86,376 | 2.7 |
  | 30k | 32× | 6.0 | 6.04 | 4.68 | 5.52 | 2.77 | 0 | 0 | 82 / 2,700 | 4.1 |
  | 100k | 1× | 10.2 | 7.73 | 6.07 | 7.14 | 4.05 | 11 (11 / 0) | 0 | 4,498 / 86,376 | 7.1 |
  | 100k | 32× | 24.4 | 24.43 | 19.88 | 22.04 | 13.59 | 2,700 (126 / 2,574) | 280 | 126 / 2,700 | 18.9 |

  Two earlier days, on a build of the same code without the reads' off switch (max / max
  unpreempted / p99.9 / frames over 8 ms, preempted and unpreempted): 10k 1× 6.8 / 2.03 / 1.30 /
  0 and 10.2 / 1.60 / 1.29 / 1 (1, 0); 10k 32× 2.1 / 2.12 / 2.02 and 2.2 / 2.17 / 2.01; 30k 1×
  5.8 / 2.62 / 2.18 and 6.0 / 3.26 / 2.10; 30k 32× 6.1 / 6.09 / 5.56 and 5.8 / 5.81 / 5.50, none
  over 8 ms; 100k 1× 13.1 / 7.49 / 6.90 / 10 (10, 0) and 9.1 / 7.28 / 7.25 / 14 (14, 0); 100k 32×
  24.1 / 24.13 / 21.98 and 27.4 / 21.90 / 25.62, every frame over 8 ms.
- **Against the targets** (max ≤ 8 ms at 30k and 100k, 1× and 32×, a full day):
  - **30k, both speeds: pass** on the raw maximum (4.6–6.1 ms in all four days) and on the
    unpreempted maximum (2.6–6.1 ms); no frame over 8 ms. 10k (not a target tier) stays under
    2.4 ms on unpreempted frames; on the raw maximum two of its four 1× days have one preempted
    frame over 8 ms (8.3 and 10.2 ms: agent ticks of 7.8 and 8.8 ms with waits of 7.0 and 8.1 ms,
    against a mean tick of 0.19 ms).
  - **100k 1×: passes on unpreempted frames, fails on the raw maximum.** Unpreempted maximum
    7.28–7.73 ms in four days, p99.9 6.90–7.51 ms; the raw maximum is 9.1–14.1 ms through 10–28
    preempted frames a day, none of which exceeds 7.1 ms once its wait is taken off. The margin is
    thin: the unpreempted maxima are the removal window's first minute (22:04 7.7 ms: 3.1 ms of
    slot work on a 3.4 ms agent tick and a 1.2 ms snapshot; also 14:04, 21:04, 23:04), the actions
    slot (06:03 7.1–7.2 ms) and the evening's largest agent ticks (21:15–21:46, 4.4–5.0 ms).
  - **100k 32×: fails.** Every frame is over 8 ms; the mean frame is 13.6 ms (agent tick 6.6,
    slot work 6.1, snapshot 0.85, pathing 0.01), and the worst frames, 21.8–24.4 ms without
    preemption, fall in 07:38–08:35 (13.7–17.0 ms of slot work on a 5.6–7.5 ms agent tick) and
    05:37–05:48 (a 9.4–9.9 ms agent tick on 11.1–11.4 ms of slot work); the report does not split
    slot work by system. See open question 6.
- **Hour boundaries.** The 24 frames a day that cross an hour's first slot hold at most 1.2–1.3 ms
  (10k 1×), 1.8–2.1 (10k 32×), 2.1–2.2 (30k 1×), 4.6–4.8 (30k 32×), 6.7–6.8 (100k 1×) and 17.4–17.5
  ms (100k 32×), under each run's maximum at every tier and speed; in the pinned pre-refactor
  reference the worst frame of every run was the first hour boundary after loading (198–2,021
  ms).
- **Structural checks** (`git grep` over the tree):
  - `simulate_tick_internal`, `simulate_operational_hour_internal` and `step_authored_minute`:
    no match in code, tests, benchmarks or tools; only this document's history and the archive
    name them.
  - Outside the dispatcher's fixed slots, game code reads the slot of the hour only as a
    position inside a declared window or sweep: each rolling sweep's chunk index (chunk 0 is one
    chunk like the others), the removal window of the building-removal and departure queues
    (slots 64–479), the funding census's input half and step half, the census book's preparation
    parts (slots 936, 944, 953 and 957, part 5), the city hour's close at the hour's last slot
    (O(1)), the event shift out of queue-free slots, and the save's note of whether the hour's
    census was published. Agents leave at their own departure minute. A debug-channel summary
    skips a day's first slot because the calendar close prints it. None adds work at an hour's
    slot 0. `TimeAdvance::crossed_day_boundary` remains only for its own unit test.
  - **What runs at an hour's slot 0** (all declared, in dispatch order): the timed events due
    then (every slot); `census publish` (fixed slot, `Cost::CityScan`: an O(1) swap and an
    O(B/p) parallel copy of funded jobs and activity, a whole census only when the window left
    it missing or stale, such as a world's first hour); `operational hour` (fixed slot,
    `Cost::Dirty`: the demand snapshot's newly created entities, the pending-arrival recount
    from queued admissions and carriers in flight, the logistics clock, expired commute
    quotes); `residential setup` (fixed slot, `Cost::CityScan`: the workforce count beside the
    plan's lot and building scans); at a day's slot 0 the `calendar close` (`Cost::Constant`:
    the day's totals become the budget report; road upkeep reads the rolling road length); then
    the slot's hourly and daily entity phases and the rolling chunks. Slot 0 is queue-free:
    slot 1 serves its queue shares. The two `CityScan` systems are the ones the
    [Scheduler](#scheduler) section allows as recorded `SIM-02-K` debt (with the other planning
    setups and plans); at 100k their slot held 1.5–1.6 ms in part 5's last diagnostic days.
- **Left** (none blocks 30k or 100k 1×; all are part 5's and part 4's lists): the actions slot
  in its own slot or one lending; slot 65; the census book's parts at 936 and 953; the first
  queue re-proof after a load; maintained workforce totals, part 4's indoor agents and zone
  lists; the planning slots still declare `Cost::CityScan`.
- **Criteria** (part 7's acceptance, each against the final code):

  | Criterion | Result |
  | --- | --- |
  | No legacy or boundary paths | pass |
  | Envelope and burst limits | pass |
  | 30k, both speeds | pass, on raw and unpreempted maxima |
  | 100k 1× | passes on `max_unpreempted_ms` (thin margin); fails on raw max through preempted frames |
  | 100k 32× | fails |

- **Acceptance as decided (2026-10-22, [question 7](#open-questions) (a); 100k at 32× deferred to
  `PERF-08`).** Each value at most 8 ms; both acceptance days, ms, day 1 / day 2:

  | Tier | `max_unpreempted_ms` | Raw p99.9 | `max_preempted_less_wait_ms` | Result |
  | --- | --- | --- | --- | --- |
  | 30k 1× | 3.19 / 2.56 | 2.21 / 2.17 | 2.52 / 2.67 | pass |
  | 30k 32× | 5.87 / 6.04 | 5.77 / 5.52 | 4.96 / 4.13 | pass |
  | 100k 1× | 7.35 / 7.73 | 7.51 / 7.14 | 6.15 / 7.10 | pass |
  | 100k 32× (deferred) | 24.28 / 24.43 | 22.51 / 22.04 | 20.58 / 18.89 | not required |

  `tools/sim02_gate.py perf07` now evaluates these limits in every run and fails one that misses
  them or lacks the scheduler statistics; applied to the two acceptance reports it passes both,
  and it fails a row over the limit and a row without the statistics. `SIM-02` and `PERF-07` are
  done.

- **`PERF-01`** (`tools/perf_contract.py --binary "$TEST_BINARY"` under `taskset -c
  0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`), two runs alternating with part 5's fourteenth
  commit as control: hour means 0.81–0.92, 3.31–3.40, 8.36–8.51 and 26.09–26.64 ms (control
  0.82–0.84, 3.24–3.32, 8.08–8.29 and 26.69–26.97); level, and the same work-count limits fail.

- **Tests.** 2,559 release and 2,559 debug library tests (one new, for the preemption
  bookkeeping) and the 4 growth integration tests pass; clippy adds nothing. An isolated
  `./run.sh --test` passed with no script errors.
- **Build identity:** rustc 1.99.1; the gate, the suites, both acceptance days of `PERF-07` and
  `./run.sh --test` ran on the committed sources (`$TEST_BINARY`), `PERF-01` on the same binary;
  `PERF-07` pinned with `--cpus 0,2,4,6,8,10,12,14` and `RAYON_NUM_THREADS=8`, every lock held,
  no other timing run or build on the machine, both days on copies of the same prepared cities.
- **Save format:** unchanged (106); the measurement is test-build code only.

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
| Demand planning (`plan_private_building_actions`) | Scans all parcels hourly | One setup and plan per zone type (setups 0, 16, 32; plans 8, 24, 40) into a saved hour plan; the actions slot (setup 48, plan 56) admits, queues and executes; each slot `Cost::CityScan` (per-zone lot lists are a `K` item) | I (done) |
| Household admission | Bounded; all carriers leave on the hour | Admission decided at the actions slot; `admissions` queue paced by a credit over the hour; departure events | E (departures done), J (done) |
| Construction queue | One start per minute | Timed events | C (done) |
| `execute_building_actions` | Plan-bounded | Proved at the actions slot with a saved permit; `level changes` queue with quota from slot 65 | J (done) |
| `allocator.maintain`, `cleanup_stale_buildings` | O(buildings), serial | Compatibility check at building phase; removals queued, served in slots 64–479. `cleanup_stale_buildings` stays immediate: it runs only from editing paths, which delete the parcels in the same edit | G (done), J (done) |
| Pollution, noise, desirability | O(grid) plus O(buildings + edges) emission | Rolling grid sweeps | H (done) |
| Daily ledger begin and finish, `reset_daily_ledgers`, `reset_daily_input_accumulators` | O(households), O(buildings) | Each entity rolls its own window at its phase | G (done) |
| `job_lock_days` decrement | O(agents) | Passive deadline | D (done) |
| Bankruptcy, wages, property tax, utilities, profit tax | Serial over buildings and agents | Per building at its daily phase, same internal order; wages at each worker's phase | G (done) |
| `pay_household_transfers` | Serial over households until the treasury runs out | Per household at its phase; slot order decides who is paid | G (done) |
| `resolve_household_housing` | Serial over households | Check at phase enqueues housing reviews, served by the `housing requests` queue | G (done), J (done) |
| `workplace_route_cache.clear` | Daily clear | Entries expire 24 passes after they were added | G (done) |
| Road upkeep sum | O(edges) | Rolling road-length sum; upkeep charged at the calendar close | G (done) |
| `run_daily_pass`, `execute_demand_household_removal` | O(households) | Exit check at phase, paced by the daily exit credit; `departures` queue in slots 65–479 | G (done), J (done) |
| `finalize_daily_tax_window`, `record_daily_budget_ledger` | Serial over ledgers | Calendar close over running sums, O(zones) | G (done) |
| `agents.daily_update` | O(agents), par | `EntityPhase` daily | G (done) |

Per-frame costs found by the `SIM-02-A` `PERF-07` baseline, outside the boundary passes:

| Today | Cost today | Target | Step |
| --- | --- | --- | --- |
| Render snapshot (`build_snapshot_reusing`) | 3–12 ms: a lane pose for every agent before camera culling, serial | Cull before posing; bucket in parallel | K (part 2 done: 100k mean 6.7 → 0.8 ms at 1×, 8.7 → 0.9 ms at 32×) |
| Agent movement in departure frames | Up to 4 ms at 10k and 17 ms at 30k (32×) | Departures spread by events; `PERF-06` limits | E, K |
| One queued construction start | 14–21 ms for a single placement | Placement split across slots | K |
| First boundary after loading | About 10× an ordinary hour. `SIM-02-H` found most of it was the catalog refresh every load made, now skipped when the catalog is unchanged. What remained were the generated cells, which loading invalidated and the first lot regions served afterwards rebuilt: 25–134 ms a region at 100k, all day | Generated during loading (K3), except chunks that would register a new grid frame | H (refresh, done), K (K3 done) |
| 100k fixture traffic (found in `SIM-02-E`) | About 49,000–56,000 of 115,000 agents on the road at any time (2026-10-21; 9,000–14,000 cars, 39,000–46,000 walkers), 15,700 of them walkers on junction crossings; about 22,000 on shopping legs, mean 3.2 km on foot (about 13 game hours a leg); no gridlock (no wait-for cycle); border imports queue on the trunks (up to 2,900 cars) and stack at lane start | Census claims bounded by locality (`ECON-22`), worth about 0.4 ms (1×) and 0.7 ms (32×) of agent tick at 100k; network entry checks the entry slot (`TRAFFIC-07`), no gain | K (part 6 diagnosed, not fixed) |

### Open questions

1. **Demand planning slicing.** Resolved in `SIM-02-I` part 4: residential, commercial and
   industrial plan in the first slots of the hour's minutes 0, 1 and 2 into a saved hour plan, and
   minute 3's first slot admits, queues the hour's spawns in the single pass's order and runs the
   actions; see [Decided while unattended](#decided-while-unattended). `SIM-02-K` part 5 spaced the
   slots a game minute apart so that at 32× each runs in a frame of its own. Per-district planning
   remains an option for `SIM-02-K` if a zone slot grows past its budget. `SIM-02-K` part 5 found
   the slots' cost at 100k to be setup that each split slot would pay again (about 4 ms), not
   decisions (0.1–2 ms), so per-district planning is not reopened; it stays an option for after the
   setup follows what changed. After part 5's sixth commit the 100k slots' decisions are 0.002–0.35
   ms of each slot, so slicing them is not needed, and a split slot would pay the setup again.
   Planning now runs as two halves per zone slot, a setup and a plan half a minute apart
   (`SIM-02-K` part 5, tenth commit).
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
6. **100k at 32× within 8 ms.** After part 4's first commit the ordinary 100k 32× frame averages
   14.07 ms: agent tick 7.14, slot work 5.96 (restock requests 1.93, trip events 1.71, census
   1.00, the rest about 0.2–0.25 each), snapshot 0.90. A frame carries about 8.5 slots of slot
   work at 32×. A worst frame of 8 ms would need about 3.5 ms of agent tick plus 3.5 ms of slot
   work with no peaks. Part 4's remaining items save at most 2–3 ms. Restock planning and trip
   events are per-entity work that scales with the simulated time a frame carries. Options, all
   of which change a decision of this document: a coarser simulation tier for distant agents; a
   cap on simulated minutes per 32× frame (the clock slips behind real time); movement
   sub-stepping; GPU compute (deferred by decision). Part 6 (the 100k traffic stalls: about
   45,000 of 145,000 agents always moving) may shrink the problem and runs first. Part 7
   decides under the stop rule: findings, options, `needs_revalidation`, stop for review. 30k at
   both speeds and 100k at 1× remain within reach. After part 5's fourth commit the 100k 32×
   maximum is 27.9 ms (the minute-4 frames and agent-heavy frames, 25–28 ms), p99 23.0 ms; after the
fifth commit 26.4 ms (agent-tick frames), p99 22.7 ms. After part 4's second commit the 100k 32×
ordinary frame averages 13.6 ms (agent tick 6.8: trip plans 6.7 ms of CPU in the movement pass;
slot work 6.0) and its wave frames carry 22–33 ms of planning CPU; spreading departures over a
minute's slots halves the waves at 32× but cannot spread them further, since a 32× frame covers
about half a minute. The cost of a trip plan is routing work.

Part 6 (2026-10-21) found no stalled traffic: the 100k city's 49,000–56,000 agents on the road
are mostly shoppers walking 3.2 km to the few stocked groceries (`ECON-22`), not trips stuck in
junctions. A city grown with a 600 m bound on the census's store claims (prototype) has about
36,000 agents on the road at 32× and an agent tick of 6.46 ms against 7.19 ms on the control
(2.41 against 2.83 ms at 1×, 00:00–04:00), diagnostic runs in one session. Moving agents cost
about 0.04 ms per thousand at 32×, so even a healthy city keeps about 5 ms of 32× agent tick in
passes over every agent and in trip planning. The 32× arithmetic of this question therefore
stands: about 6.5 ms of agent tick plus about 5.6 ms of slot work. 100k 32× within 8 ms still
needs one of the options above.

**Part 7 (2026-10-22), the final count.** On the final code (two full `PERF-07` days, part 7's
results): the 100k 32× frame averages 13.6 ms, of which agent tick 6.6 ms, slot work 6.1 ms,
render snapshot 0.85 ms and pathing 0.01 ms. Every frame exceeds 8 ms, p99 is 19.9–20.2 ms,
280–314 frames a day exceed 16.7 ms, and the worst frames without preemption are 21.8–24.4 ms
(13.7–17.0 ms of slot work on a 5.6–7.5 ms agent tick in 07:38–08:35; a 9.4–9.9 ms agent tick on
11 ms of slot work in 05:37–05:48). 100k 1× now meets 8 ms on the frames no other task preempted
(7.28–7.73 ms in four days), so 32× is the one target the refactor leaves. The arithmetic of this
question stands, and with numbers from the final code:

- **Slot work is per-entity work for the simulated time a frame carries.** At 32× a frame
  crosses 8.5 slots, and slot work rises linearly with speed: 0.23 ms a frame at 1×, 0.45 at 2×,
  0.86 at 4×, 1.63 at 8×, 3.14 at 16× and 6.1 at 32× (a 100k speed sweep, one diagnostic day
  each, the same cities), about 16–20 s of slot work per game day at every speed. The agent tick
  rises from 3.0 ms (1×) to 6.6 ms (32×): passes over every agent plus trip planning (about 120
  µs a plan, routing work, part 4).
- **Neither half fits alone.** The agent tick and snapshot alone are 7.4 ms at 32×, so slot work
  would have to fall from 6.1 ms to under 0.6 ms in every frame, its peaks of 11–17 ms included.
- **Part 6's traffic finding does not change it.** The 100k city's shoppers walk kilometres
  because the funding census lets stores claim the nearest households without a distance bound
  (`ECON-22`); a 600 m bound took 17,000–18,000 agents off the road but under 1 ms of agent tick
  (part 6's prototype: 0.4 ms at 1×, 0.7 ms at 32×).
- **Lower speeds do not fit either.** The worst frame without preemption at 100k is 7.3–7.7 ms at
  1×, 9.8 ms at 2×, 9.5 ms at 4×, 12.7 ms at 8×, 16.0 ms at 16× and 21.8–24.4 ms at 32× (sweep;
  1× and 32× from the acceptance days). At 2× and 4× the frames over 8 ms without preemption (17
  and 42) are mostly the removal window's first minute (minute 4 of an hour: up to 4.5 ms of slot
  work on a 3.4–5.8 ms agent tick).

The options, each changing a decision of this document:

- **(a) A coarser tier for distant areas.** Outside the area of interest, agents skip the
  per-frame movement (a trip becomes its arrival event, timed from the routed travel time) and
  households and buildings settle as district aggregates in that tier's own phase buckets
  ([Simulation tiers](#simulation-tiers)). It removes most of the movement (about 9% of moving
  agents are in the camera, part 2) and, through the aggregates, the per-entity slot work of
  distant districts, so it is the only option that reaches both halves at any size. It changes
  the decision that every entity runs `Full`, needs its own design (the aggregates, the hand-over
  between tiers, what the player sees), and is what goal 4 built the scheduler for and the
  project's route to a million residents.
- **(b) A cap on simulated slots per frame** (the clock slips behind real time when a frame would
  carry more). Deterministic, since results follow the slot sequence alone, and no new system, but
  it changes the decision that a speed simulates its stated rate, and at 100k it would bind at
  1×: the sweep's 2× day already has 17 frames over 8 ms without preemption.
- **(c) Movement sub-stepping** (each agent moves every k-th frame by k frames' time). It changes
  the time model's per-frame movement and the traffic it produces (coarser car following,
  junction and lane-change steps), and reaches only the movement part of the agent tick, not
  trip planning or the 6.1 ms of slot work: it cannot reach 8 ms alone.
- **(d) GPU compute**, deferred by decision ([GPU compute](#gpu-compute)): movement is its only
  GPU-shaped part, so it shares (c)'s limit, and WGSL does not guarantee bit-identical results.

**Recommendation: (a)**, as its own roadmap item that takes over the 100k 32× target, so that
`SIM-02` can close on the targets it meets. Only (a) and (b) reach slot work, and (b) would make
100k a 1× game. A changed target (for example 100k 32× max ≤ 16.7 ms, one frame per display
refresh, so the clock never slips) is a further option, but 280–314 frames a day exceed 16.7 ms
today, so it too needs (a) or (b).

**Decided by the user (2026-10-22): deferred.** The option is chosen when players have built
cities that start to approach 100k agents. Until then the target is tracked as `PERF-08`, outside
`SIM-02`; `PERF-07`'s acceptance covers 30k at both speeds and 100k at 1×, and the gate keeps
reporting 100k at 32× so the gap stays visible.

7. **`PERF-07` under preemption** (`SIM-02-K` part 7). The benchmark's eight pinned CPUs are
   shared with the rest of the machine, and another task keeps a pool worker or the frame's own
   thread off its CPU for 0.5–10.6 ms in 0.2–9% of a day's frames (100k 1×: 4.6–5.4%). Such a
   frame stalls for about its wait, at a different minute in every run, so a day's raw maximum
   measures the machine as much as the simulation: in four days of the same code 100k 1× had raw
   maxima of 9.1–14.1 ms and 10–28 frames over 8 ms, every one of them preempted, against
   7.28–7.73 ms without preemption. The acceptance needs a decision:
   - **(a) Accept on the unpreempted maximum**: `max_unpreempted_ms` ≤ 8 ms, with the raw p99.9 ≤
     8 ms (so at most 0.1% of the day's frames, 86 at 1×, may exceed 8 ms through preemption) and
     `max_preempted_less_wait_ms` ≤ 8 ms (so no preempted frame hides more than 8 ms of work). No
     machine change; the raw columns stay reported and the gate's "no worse" limits stay on them.
     On the four days: 30k passes at both speeds, 100k 1× passes (p99.9 6.90–7.51 ms, preempted
     frames less their wait at most 7.1 ms), 100k 32× fails.
   - **(b) Isolated CPUs**: boot with the benchmark's CPUs and their SMT siblings isolated (for
     example `isolcpus=` and `nohz_full=` on 0–15, or a cpuset that moves every other task off
     them), or run on a quiet machine. The raw maximum then means what it says, but it is machine
     configuration outside the repository, needs a reboot each way, and does not describe the
     players' machines, which are never isolated.
   - **(c) Fewer workers than pinned CPUs** (for example 7 workers on the 8 CPUs), so a waiting
     worker can move to a free CPU. It shortens the stalls without removing them (the kernel
     moves a runnable task to an idle CPU only at its next balancing), and it measures a pool
     1/8 smaller than the game runs.

   **Recommendation: (a).** It needs no machine change, the benchmark reports it from part 7 on,
   its two guards keep a preempted frame from hiding work, and the threshold data show that it
   does not depend on the threshold's exact value. (b) only if a raw maximum must be quoted.

   **Decided by the user (2026-10-22): (a).** `tools/sim02_gate.py perf07` checks it for 30k at
   1× and 32× and 100k at 1×, and fails a run that misses it or lacks the scheduler statistics
   (see [Frame budget](#frame-budget-perf-07)).

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
  Correction (`SIM-02-K` part 6, 2026-10-21): the fixture is not gridlocked, and the lane
  wait-for graph has no cycle. The in-flight trips are long walking shopping trips (3.2 km,
  about 13 game hours a leg at 100k); the back-off still fits them.
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
  The slicing budget counts work units (rows at each step's weight plus the searches the claim
  and staffing sweeps make; `SIM-02-K` part 3), never wall-clock time, so the
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
  (one before the first), so the remaining work spreads over the slots left (since `SIM-02-K`
  part 3 the plan also takes the previous census's total work); a census needing
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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.
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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.
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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.

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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.
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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.
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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.
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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.
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
  Moved in `SIM-02-K` part 5 (fourth commit): the planning slots are now slots 0, 16, 32 and 48, and
  removals, departures and level changes start at slot 64; see its results section.
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

- **`SIM-02-K` part 3 — census slices in work units.** Alternatives: budgets in staffing
  operations and trials (a 20.6 ms slice had 2,408 trials and 973 network searches, a 16 ms
  slice 6,576 trials and 93 searches, so they do not predict cost); wall-clock budgets
  (forbidden: slices must not depend on the machine); the census as a pure job (it routes on the
  live lanes and network, see [Pure jobs](#pure-jobs)); fixed row budgets per step (cost per
  row varies by orders of magnitude within one step). Chosen: weighted counts of the searches
  the census already makes, from release counters in the route cache, the supplier book and the
  consumer market, with weights fitted to a per-row profile at 100k. Slicing reads only these
  counts, so it is deterministic, and never changes a result. Evidence: at 100k the worst slot
  of a sliced step fell from 20.4 to under 1 ms in the steady hour; the envelope is
  bit-identical.
- **`SIM-02-K` part 3 — the census's plan: the previous total, the observed rate and slack.**
  Alternatives: a fixed budget per slot (a census finishes early, but at 32× a frame crosses
  about 8.5 slots, so the census frames would carry 8.5 budgets); the rows left at their weights
  alone (measured: a census heavier than its estimate crawled, then spent 3–7 ms a slot at its
  planned last slots); a floor of about 0.25 ms a slot (dropped: it changed nothing at 100k and
  made small towns' censuses finish in the window's first slots, so the restart tests no longer
  saw a census in progress; spreading costs nothing). Chosen: R = max(previous total − used,
  rows left at their weights with the sweep in progress at its observed rate), D = max(S − 96,
  S / 4, 1). Remaining weakness: the first census after loading did 1.6× the previous one's work
  at 100k and still caught up at 1.4–2.8 ms a slot near its end.
- **`SIM-02-K` part 3 — the census spreads evenly at 32× too.** At 100k 32× the evenly spread
  census raised the frames over 16.7 ms from 662 to 984 while lowering the p99 and p99.9.
  Alternative: concentrate the census in fewer slots (a fixed budget a slot), which at 32× puts
  about 8.5 budgets into each census frame and raises the worst census frames instead. Kept:
  `PERF-07`'s target is the maximum, and the 32× frames at 100k exceed 8 ms on the agent tick
  alone (`K4`), so the count over 16.7 ms is decided there.
- **`SIM-02-K` part 3 — unsliced census steps start in their own slot.** A step that runs whole
  starts only in a slot that ran nothing else, so no slot stacks two of them or one on top of a
  sliced step.
- **`SIM-02-K` part 3 — a reset customer book drops its recorded quotes at once.** See the fix in
  the results section. Alternatives: make the refresh report the book not retained after a
  restart (it would need to know a census was abandoned, which is the driver's state, not the
  book's); clear the quotes at every census start (loses the cross-census replay the books keep
  on purpose). Chosen: the book and its replay record change together.
- **`SIM-02-K` part 3 — the unsliced census steps stay whole for now (findings).** At 100k: the
  consumer-market refresh 11–22 ms (household budgets in order with a running treasury, access
  rows, the reachable-household index rebuilt when membership changed, the claim reset), route
  quotes 1.5–10 ms (border sweeps, building access and quotes, after any frontage price change),
  business seeding 5–6 ms (one serial pass over buildings inside the business book's reset),
  the input-source reset 3.5–5 ms, setup 4–4.5 ms (two serial passes building offsets and the
  supplier order), repair 2–4 ms and propagation about 1 ms. Options, each a resumable form of
  the step with its carried state in `CensusProgress`: budgets in household ranges carrying the
  running treasury and the change flags, access rows in household ranges, one index category a
  slice and the claim reset in ranges; one border sweep a slice, then access and quotes in
  building ranges; business seeding in building ranges between a reset start and end; setup's
  passes in building ranges carrying their counters. The step order and every result stay as
  they are.
- **`SIM-02-K` part 3 — the clearance site publication (findings).** Measured at 100k 32× over
  the `PERF-07` day (temporary timing, one run): 53 site publications took over 0.3 ms; the
  largest took 44.7, 22.3, 13.6, 10.7 and 9.9 ms, all of it in the terrain patch ownership refresh. Yard
  publication took under 0.06 ms and the site query index was already current. The allocator
  merges every site change of the slot into one bounding box, so two or three removals
  kilometres apart refresh every render patch between them: boxes 3.2–5.1 km across, 32–132
  patches, about 0.34 ms a patch. Each patch's ownership is a function of the roads, the sites
  and the heightmap, so refreshing a patch no change touched reproduces its state. Option (no
  gameplay change): keep one box per change, merging only boxes that overlap, and refresh the
  union of their patches; the cost then follows the changed sites, O(changes × patches a site),
  and patches far from every change are no longer re-uploaded. Not done: it needs its own gate.

- **`SIM-02-K` part 3 — one terrain box per site change.** Alternatives: merge only overlapping
  boxes (the same patches, more code: the union of patch keys already removes the overlap); keep
  the merged box and refresh only patches some site overlaps (still visits every patch in the
  box, O(box area)); move ownership onto the patch from the site index (a new structure next to
  the 512 m site chunks). Chosen: one box per change and one refresh of the sorted, deduplicated
  union of their patches, so the cost follows the changed sites. Evidence: the largest
  publication at 100k 32× fell from 44.7 ms (110 patches) to 1.27 ms (6). This settles the
  findings in "the clearance site publication (findings)" above.
- **`SIM-02-K` part 3 — a torn customer book is withdrawn, not rolled back.** A census restarted
  inside the consumer-market refresh finds its book partly rewritten. Alternatives: double-buffer
  the budgets, access rows and indices and swap at the finish (about 8 MB more at 100k and a full
  copy of unchanged rows every hour); journal the rewritten rows and roll them back (more code on
  a rare path); sticky change flags kept across the restart (wrong when the restarted copy
  changed a row back). Chosen: the market records a refresh in progress; beginning another
  withdraws the book first (`clear`), so the refresh reports it not retained, as after a load.
  The census's results are a function of the copy and the network alone, so no result changes;
  a restart there costs one census without the retained book.
- **`SIM-02-K` part 3 — each mode's customer index built once and copied.** The refresh built
  every category's two indices from the same entries. Alternatives: keep K × 2 builds sliced one
  a slot (six slots of about 3 ms at 100k); share one membership structure with per-category
  availability (changes the index's ownership across claims, quotes and revalidation). Chosen:
  build each mode once, entries sorted in parallel, and clone it into the other categories; a
  clone of a deterministic build equals another build.
- **`SIM-02-K` part 3 — the slot plan is per step.** Alternatives: the previous census's total
  (K3b; one step's overrun took every later step's plan); a larger tail (more slack for every
  census, finishing later for none). Chosen: per step, the previous census's work less this
  census's, summed, with the rows' estimate as the floor. Remaining: window 0's `current_staff`
  still overruns its own step's plan (68 against 37 ms) and catches up at up to 2.2 ms a slot.
- **`SIM-02-K` part 3 — loading gives the census book its walking proofs.** Alternatives: build
  them at load for the census separately (the same network-scale build twice); save them (a
  format change for a derived structure). Chosen: copy the loaded census's proofs and component
  indices into the census's own book when it has none; they are keyed by the network revisions,
  so a changed network still rebuilds them.
- **`SIM-02-K` part 3 — the remaining whole census steps (findings).** At 100k: route quotes up to
  10.2 ms after a frontage price change (`write_border_carrier_quotes`: border sweeps, then each
  building's quotes; sliceable as one border sweep a slot, then the quotes in building ranges);
  business seeding 5.2–5.4 ms (the business book's reset with one serial seeding pass over
  buildings inside its closure; sliceable in building ranges between a reset start and end, the
  receipts' order kept); the input-source reset 3.5–4.3 ms (`RecurringSupply::reset`: after a
  geometry revision change it rebuilds the supplier candidate index and offsets, O(B log B);
  sliceable as entries, build and spatial bounds, as the customer index now is); repair 1.8–3.2
  ms (requotes withdrawn provisional claims; needs an ordered resumable requote); the refresh's
  finish 1.6–2.7 ms (the claims' reset: account budgets in household ranges and each index's
  availability a slot); the refresh's index build 1.8–1.9 ms and copies 1.2–1.3 ms (one mode or
  one category a slot would halve them); setup's import-plan layout 0.9–1.7 ms. For the refresh
  and setup this supersedes "the unsliced census steps stay whole for now (findings)" above.
- **`SIM-02-K` part 3 — route quotes in phases, named current at the end.** Alternatives: one
  border sweep a slot with the rest whole (the door projections and quotes cost 3–4 ms after a
  routing change at 100k); keep the quotes' revision through a rewrite and record a dirty
  range (more state for the same restart). Chosen: four steps, the quote revision cleared when
  a rewrite starts and set after its last range, so a restarted census rewrites every row.
  The border sweeps count work only for borders actually swept, so a warm cache's census does
  not plan for sweeps it never makes.
- **`SIM-02-K` part 3 — the supplier and business indices one resource a slice.**
  Alternatives: incremental insertion into the bucket indices for appended buildings (the
  reservation positions of every later item move, so it is a rebuild in disguise); one slice
  for the whole build (1–3 ms at 100k). Chosen: entries collected and sorted once, each
  resource's index built from its own sorted run in a later slice; a resource's index is a
  function of its entries alone.
- **`SIM-02-K` part 3 — a torn supplier or business layout is unnamed, not rolled back.** As
  for the customer book (third commit): the layout's revision is cleared when a rebuild starts
  and written when it ends, so any restart rebuilds it; no journal, no double buffer.
- **`SIM-02-K` part 3 — cleared storage takes fresh allocations.** Alternatives: keep each
  book's capacity stable across the publication swap (swap contents, not storage: a full copy
  every hour); shrink the live book's planning reserve (planning's own bound). Chosen: an empty
  vector that must grow takes a new allocation of the capacity `reserve` would give, so
  capacities, and every path that reads them, stay as before; nothing is copied for rows that
  no longer exist.
- **`SIM-02-K` part 3 — route cache tables filled in parallel.** The census's route cache is
  empty after loading and its first growth (door access and pair tables, about 18 MB at
  100k) and every routing change's clear filled them on one thread (5 ms of the route
  quotes' start in window 0). Alternatives: an epoch tag on each door projection so a clear
  needs no fill (changes the cache's validity rules, shared with dispatch); slicing the fills
  (a half-filled table is read by nothing, but the clear's callers are not all the census).
  Chosen: the same rows written in parallel; the start's first growth after loading fell to
  2.1 ms, and a clear in dispatch is faster too.
- **`SIM-02-K` part 3 — a slice still gets the slot's whole budget.** Each step called in a slot
  receives the slot's budget, not what is left of it (since part 3's second commit), so a slot
  can do up to about two budgets (about 0.6 ms at 100k). Kept: it bounds the slot well under
  the remaining whole steps, and changing it moves the census's finishing slot.
- **`SIM-02-K` part 3 — `PERF-07` on shared cities.** The save format is unchanged, so this
  commit's build ran on the cities its control prepared, in the same session; the comparison
  is then on identical cities and saves the build's own preparation.
- **`SIM-02-K` part 3 — repair stays whole (findings).** This supersedes "the remaining whole
  census steps (findings)" above for every step but repair. At 100k repair costs 1.7–2.2 ms in
  window 1 after the storage change (3.9 before). A timing build put the requote loop at
  0.2–0.6 ms; the rest is its preparation, `RecurringSupply::prepare_owners`: the receipt
  arena's and withdrawal list's reserve (1–2.1 ms) and the revalidation journal's storage
  (0.5 ms), sized to the arena's capacity, which alternates between the census's and the live
  book's (about 63,000 and 1.4 million receipts at 100k). Options: size the revalidation
  journal to the receipts the repair can touch rather than the arena's capacity; keep the
  census storage's capacity apart from planning's reserve; then, if the loop still exceeds
  1 ms, an ordered resumable requote whose journal survives between slots and is discarded at
  a restart (`begin_reset` would abort an active revalidation).
- **`SIM-02-K` part 3 — the other slots still above 1 ms (findings).** At 100k: each customer
  index mode's build 1.3 ms (about 0.3 ms of it the parallel sort; splitting sort and bucket
  build into two slices would leave about 1 ms); setup's import-plan layout 1.4–1.7 ms
  (`import_plans.prepare`, not sliced); business seeding's start 1.0–1.4 ms (the accounts'
  reset: fills of buyer-sized rows); the refresh's start 1.8 ms in window 0 only (the
  household budgets' start and the query scratch on the census's first use after loading);
  the route cache's first growth after loading 2.1 ms in window 0 only; window 0's staffing
  catch-up (1.3–1.8 ms at slots 910–930; its census did 68 ms of current-staff work against
  the previous census's 37, more than its own step's plan); and 0.3–1.8 ms of the window's
  first slot outside the steps (frontage price publication and the input copy's swap).
- **`SIM-02-K` part 4 — the lane-bucket snapshot follows appends and swap-removes.**
  Alternatives: a per-agent record of the lanes each agent's entries sit in (a reverse index,
  eight bytes an agent more and a write per agent in every fill), rejected because entries can be
  found from the agents' current lane state once every mutator between ticks either invalidates
  or removes the agent's entries itself (audited, and checked against a rebuild after every
  patch in test builds); patching at the insert, rejected because spawn calls place the agent
  after inserting it; patching the remaining invalidating mutators too (building-reference
  scrub, evacuation, road edits), not done: they touch many agents and are rare (a full rebuild
  now runs in at most 2.5% of 32× frames at 100k).
- **`SIM-02-K` part 4 — buckets sorted again after an overlap clamp, ghosts after the
  correction.** A kept snapshot must be the function of the agents' state a rebuild gives. Before,
  a clamp that tied two cars at the lane start kept them out of index order until the next
  rebuild, and the next tick's gap and slot checks read whichever order the frame happened to
  have (rebuilt when an agent had spawned or left). Alternative: keep the old order and rebuild
  whenever it would differ, which is a rebuild every frame. The check found the tie in the
  bridge suite's growing town (border arrivals queued at a lane start); the envelope is
  bit-identical.
- **`SIM-02-K` part 4 — the fill groups agents by lane range in one pass.** The first version
  had each lane range scan every agent's key (O(n × ranges)), which saved only 0.1–0.2 ms at
  100k and cost 0.08 ms at 30k 32×; grouping per 4,096-agent chunk first makes it O(n + L) and
  the fill 0.6 ms at 100k (the old serial scan, sort and correction: about 1.3 ms). The
  dirty-lane list is now ascending within each range (was first-touch agent order); only the
  junction stall debug scan reads its order. Alternatives: a flat sort of every agent's
  `(lane, distance, index)` (O(n log n), more work than the grouping), or a lane-major CSR
  layout for the buckets (fewer allocations and cache misses, but it changes every bucket reader
  and makes the swap-remove patch O(M)).
- **`SIM-02-K` part 4 — movement shards interleave chunks of 256 agents, each with its own
  scratch.** Alternatives: more shards than workers drawing scratch from a pool (Rayon work
  stealing balances dynamic hot spots too, but the scratch serving an agent then depends on
  scheduling, which makes the route-memo search counts vary between runs — the reason movement
  scratch became owned by agent ranges on 2026-10-14, [`economy.md`](economy.md)); one scratch
  per small shard (64+ scratches at several MB each at 100k). Interleaving balances the
  structural imbalance (moving agents clustered by index) without either: 1.11–1.38× after
  1.64–1.71×. What is left is random hot spots (trip plans at 32×).
- **`SIM-02-K` part 4 — `current_congestion` kept (findings).** Resolved in part 4's second
  commit (entry "`SIM-02-K` part 4 (second commit) — `current_congestion` deleted, save format
  105"). The first commit kept it: it was written each frame and saved, and read only by tests
  and the road-edit copy, and deleting it (0.32–0.35 ms a frame at 100k, 0.06–0.08 ms at 30k, and
  the per-edge scratch) bumps `SAVE_VERSION`, rewrites the Kuopio fixture's `network_edges` and
  its hash in `placements.json`, touches about 40 files (mostly test edges) and needs each build's
  own `PERF-07` cities, which did not fit that part's timing session.
- **`SIM-02-K` part 4 — indoor agents stay in the movement pass (findings).** About 61% of the
  agents are indoors. An indoor agent with a cached departure that is not yet due costs a few
  loads in each per-agent pass (claims, speed, dispatch, the fill's grouping), and one whose
  departure is due plans its trip inside the parallel movement pass (CCH point searches; at 32×
  about 50 plans and 40 commute estimates a frame at 100k, historical profile). An exact
  departure wheel needs every writer of what `handle_in_building` reads to wake the agent: the
  planned target (shopper, visit, freight and arrival dispatch), the home and work buildings,
  the activity, the replan gate, the household and building-reference mutators, plus the
  cached departure's own minute. That is a cross-system change through households, logistics,
  hiring and the building store, and the departure would still be planned in a frame, so its
  CCH cost moves rather than disappears unless the plans are batched (a pure job with CCH
  buckets). Estimated saving at 100k: the per-pass visits of 70,000 indoor agents, about
  0.3–0.5 ms a frame over claims, speed and dispatch, plus the 32× plan cost moved off the
  movement pass's critical path. Option for `K4` follow-up: an active-agent list maintained by
  the transit transitions and a departure wheel keyed by absolute minute, with the wake calls in
  the lifecycle and household mutators; the departure stays in the same frame, so gameplay is
  unchanged.
- **`SIM-02-K` part 4 — the serial handoff set stays (findings).** At 100k 32× it holds 4,400
  agents a frame (1.9 ms serial). Pedestrians cannot simply leave it: they reach lane ends and
  claim connector entries like cars (`claim_connector_entry`), so their claim order matters.
  Extending the minimum-index reservations to handoffs whose target lane is known at
  classification (a single-candidate connector, a frontage detach, a car's egress attach) is
  deterministic, but it changes which agent wins a contested lane: today the serial order lets a
  lower-index agent take a lane as its fallback choice before a higher-index agent's first
  choice; with reservations the reserver wins, as fixed lateral moves already do. That is a
  gameplay change in tie-breaking only, for review before it lands.
- **`SIM-02-K` part 4 — 100k 32× needs a spec decision (findings and options).** After part 4
  an ordinary 100k 32× frame is 14.1 ms (agent tick 7.1, slot work 6.0, snapshot 0.9): one frame
  carries 0.53 s of movement for about 45,000 moving agents and 8.5 slots of per-entity work.
  Options, none implemented:
  1. **A coarser tier for distant agents** (the spec's `Simulation tiers`): agents outside the
     area of interest advance on their trip's planned time, not lane by lane, and join the
     full tier when the camera or a road edit reaches them. It removes most of the agent tick
     and of the trip events' per-frame cost at 100k, and is the project's stated scaling path.
     Cost: the tier's systems, the hand-over rules, and the envelope at 100k.
  2. **A cap on simulated time per frame at 32×** (for example 4 slots, 0.25 s): the frame stays
     within budget and the clock slips when the work exceeds it, as an over-budget game does
     today. Cheap and honest, but 32× is then not 32× at 100k.
  3. **Movement sub-stepped over frames, slots over the frame's span**: run a 32× frame's movement
     in two halves on alternating frames. Changes the movement step (`FRAME_DT_S × speed`),
     which the time model fixes.
  4. **GPU movement** (deferred by the 2026-10-15 decision): the per-agent passes are regular, but
     the serial handoff set and the trip events are not, and the WGSL determinism caveats stand.
  5. **Fix the 100k traffic stall first** (`K6`): if most of the 45,000 always-moving agents are
     stalled trips, a healthy 100k city moves fewer agents a frame; measure before choosing.

  Recommendation: `K6` first (it changes the size of the problem), then decide between 1 and 2
  for 100k 32× (part 7 decides under the stop rule; see
  [Open questions](#open-questions), item 6); 30k and 100k 1× need only `K5`.
- **`SIM-02-K` — part 5 before part 4's remaining items.** After part 4's first commit the
  maximum at every tier and speed is the hour's planning slots (19–23 ms at 30k, 55–78 ms at
  100k), while part 4's remaining items save at most 2–3 ms of agent tick. Alternative: finish
  part 4 first. Rejected, because it leaves every tier's maximum where it is.
- **`SIM-02-K` part 5 — measured costs first.** The part was planned from a profile of part 3's
  build. On part 4 the planned scans cost 0.1–1.5 ms a slot at 100k (lot scan 0.3–1.0, live
  workforce 0.45–0.5, existing buildings 0.5–0.7, slot 3's downgrades 1.45 together), while the
  tier maxima were the post-loading residential slots (100k 49 ms, of it about 23 ms storage
  growth and first touch, and about 14 ms full site-row builds and layouts). Alternative: the
  planned order (lot lists and maintained totals first); rejected because together they would
  remove about 1.5 ms of a 49 ms slot and move no tier's maximum.
- **`SIM-02-K` part 5 — planning storage estimates count free lots.** Alternatives: keep the
  worst case (a project on every parcel, so each book grew in a planning slot until it held
  arenas sized for that); no estimate at all (every proposal that adds rows would replay once to
  grow storage); a maintained free-lot count (O(1), part of the
  per-zone lot lists, not done). The O(P/p) count costs about 0.02–0.05 ms at 100k. An estimate
  never changes a result: commits preflight their storage and a shortfall replays the proposal
  after growing it exactly (`demand/supply/storage.rs`).
- **`SIM-02-K` part 5 — slot 3's downgrades in one scan.** A building's downgrade candidacy reads
  only the city the pass holds immutably (allocator, catalog, tuning, residential occupants), so
  one scan before the zone loop gives each zone the list its own scan gave; the attachment order
  ends in the building index, so the split lists are in the same order. Alternative: incremental
  candidate sets kept by the building, occupancy and finance mutators that feed the viability
  checks; not done (about 0.5 ms a slot at 100k left).
- **`SIM-02-K` part 5 — planning books prepared at load and when a fresh census book finishes.**
  Alternatives: save the derived rows and storage sizes (a save format change for derived data);
  keep the site rows with the live book across the census swap (the business book's port maps
  belong to its layout, and a torn pair would be read between the swap and the next layout);
  give the empty census storage the live book's capacities at load (it would still fault in its
  journals at its first planning, and it reaches into three books' internals); compute a second
  census at load. Chosen: lend the books once, as a planning slot does, at load for the live
  books and in the window slot after a fresh census book finishes; it reuses the planning code,
  so what it prepares equals what the slot would. A book is prepared once
  (`JobFunding::planning_prepared`, set when attempted, so a book not valid for the live city is
  left to its first slot rather than retried every slot). Cost: a 100k load takes 0.1 s more
  (`PERF-07` load 18.6 against 18.5 s), and the census book's preparation takes one window slot
  once per fresh book: 34.5 ms at 100k and 11.5 ms at 30k (diagnostic), the new maxima at 30k
  and 100k 1×. Since the second commit a load gives empty census storage a prepared copy of the
  live books and prepares a census the load finished, so the window-slot preparation served only a
  new world's first census until the sixth commit made it run every hour.
- **`SIM-02-K` part 5 — what is left in the planning slots (findings).** (1) The census book's
  preparation after loading (34.5 ms at 100k, 11.5 ms at 30k, once): it is the cold storage
  growth (owner and supply storage 5.6 ms at 30k, about 13 ms at 100k), the full site-row build
  and layout, and the pass's owner rows. Options: slice it (the site rows in owner ranges as the
  census slices its steps, the storage growth in chunks of capacity); size the planning journals
  by length and grow them on demand instead of by capacity. (2) At 100k each book's business
  storage grows once more at its second planning after loading (7.0–7.5 ms in `prepare_owners`,
  1.7–1.9 ms in its `begin_pass`): the census refill grows the accounts' capacity by doubling,
  and planning sizes its journals, masks and port maps by capacity. Options: grow with headroom
  at preparation, or size them by length. (3) Each slot's own O(B/p) setup (about 4 ms a slot at
  100k): the owner rows, the live workforce, the calendar's reset, the existing buildings, the
  lot scan and the snapshot. Options: the planned maintained totals and lot lists; owner rows
  computed on demand for the owners a proposal reads; one fused pass for the building-indexed
  setup. (4) The residential slot's site-row merge and layout each hour (about 3.5 ms at 100k):
  the layout follows the census book's revision, which changes every hour. (5) Not measured as
  frame costs: the job board's and destination index's O(B) fallback builds (only without a
  front: a world's first hour, an edit after the window's last slot) and the commute quote
  cache's clear (a narrower clear changes which quotes are reused, a gameplay change).
- **`SIM-02-K` part 5 — open question 1 not reopened.** Per-district planning or more slots per
  zone would split the decisions, but at 100k the slots' cost is setup that each split slot would
  pay again (about 4 ms), not decisions (0.1–2 ms). Slicing waits until the setup follows what
  changed.
- **`SIM-02-K` part 5 (second commit) — census storage copied at load rather than the preparation
  sliced.** The census book's preparation after loading (36.5 ms at 100k, 11.7 ms at 30k,
  diagnostic) was building planning storage, site rows and walking proofs on storage that loading
  had left empty. Alternatives: (a) slice the preparation over window slots as the census steps
  are: its phases are 4–12 ms each at 100k (the business owner storage alone 4.3 ms, the supplier
  and buyer rows 6.3 and 7.4 ms), so reaching 2 ms a slot meant resumable growth inside three
  books' storage and range-sliced site-row factories, with the books lent across slots; (b) size
  the planning journals by length instead of capacity: a pass appends rows up to the arenas'
  capacity without growing anything inside a proposal (the exact preflight design), and its
  journals must cover them; (c) run a second census at load into the census storage (about
  0.2–0.3 s per 100k load, and its storage would still be sized exactly); (d) keep the
  window-slot preparation. Chosen: give empty census storage a copy of the prepared live books
  and prepare the copy, which is what an uninterrupted game's census storage holds (a previous
  published census its planning passes prepared), following the load-time walking-proof adoption
  of part 3. A census's result never depends on its storage's content (the reload checks compare
  continuous and reloaded games, whose census storage held different censuses, bit for bit; a new
  test compares a census on a copy with one on fresh storage). Cost: one copy and one
  preparation per load, within the load time's noise (100k `PERF-07` load 18.55 against 18.58 s,
  30k 4.92 against 4.90 s); the census storage holds a book from load on, as it does from the
  first hour on in an uninterrupted game.
- **`SIM-02-K` part 5 (second commit) — spare capacity for row storage that follows the city.**
  Row arrays indexed by planning owners, buildings or parcels were sized exactly at their first
  preparation after loading, so the first appended building reallocated about twenty of them at
  twice their capacity, copying every row, in one planning slot (13 ms at 100k, 4.6 ms at 30k,
  diagnostic); arenas grown by request (input tranches, receipts) did the same when a request
  passed the capacity by a few rows, doubling every journal sized by them. Alternatives: Vec's
  own doubling alone (the status quo: exact at first use); size every journal by length (see
  alternative (b) of the entry above); grow in slices over slots (state across slots for a growth
  that a spare margin removes). Chosen: keep at least an eighth of spare rows, growing to the
  larger of a quarter more than needed and twice the old capacity, so growth stays amortized O(1)
  and happens at load, then at most once per eighth of city growth. Capacity never decides a
  result (exact preflight and replay). Memory: up to twice the owner-indexed arrays' length is
  reserved, untouched until written. Journals sized by an arena's capacity still are, so they
  cover its spare rows too.
- **`SIM-02-K` part 5 (second commit) — the first frame after loading not taken.** At 100k the
  day's first frame is 29.9 ms in `PERF-07` (28 ms diagnostic: job requests 14.8 ms and restock
  requests 7.6 ms), then trip events and restock requests about 8 ms in 5–8 frames over the first
  minutes (diagnostic). The job board and destination fronts are saved, so these are not their
  O(B) fallback builds. Timing points in a test build (one 100k run just after a heavier one, so
  about a fifth high) put the first frame's job search in its home-option routes
  (`build_home_job_options`, 13.1 ms) and the network quotes' sync
  (`workplace_network_quotes.sync`, 4.5 ms), the restock queue's cost in reserving the planned
  restocks (`apply_replenishment_plan`, 9.3 ms in each of the first two frames that serve it), and
  the trip events' in single freight dispatches (8.0–9.5 ms, four times in the first five
  minutes), one shopper check (9.2 ms) and one freight check (9.1 ms); none recurs after the
  first hour (trip events at most 1.2 ms a frame). Building what they build at load is exact only
  for what is a pure function of the loaded network and city; commute quotes read live
  congestion, so warming them changes which quotes later searches reuse, a gameplay change. Left
  for review with these findings.
- **`SIM-02-K` part 5 (second commit) — the per-slot setup not taken.** This commit took the two
  load-path items only; the findings of "what is left in the planning slots (findings)" stand
  (about 4 ms a slot at 100k: owner rows, live workforce, calendar reset, existing buildings, lot
  scan, snapshot compose). With the census-book preparation and the storage growth gone, this
  setup now sets the 30k 32× and 100k hour frames.
- **`SIM-02-K` part 5 (third commit) — one owned route scratch for the slots, rather than
  warming every worker's.** The day's first frames after loading paid each pool worker's
  thread-local trip scratch (7–9 ms at 100k) the first time that worker ran serial slot work,
  eight times over the first minutes (instrumented). Alternatives: (a) prepare every worker's
  thread-local scratch at load with `rayon::broadcast`: the same frames, but a broadcast waits
  for every worker, so a worker blocked outside Rayon during a load would stall it, and every
  road edit still re-prepares up to eight scratches over the following frames; (b) make the
  preparation itself cheaper (the connector table and its completeness proof are
  O(lanes + connections) per scratch); (c) leave it, as the cost falls once per worker per
  network revision. Chosen: install one owned scratch for the slots' duration, as the movement
  pass installs its shards, and prepare it at load. A query's result never depends on which
  scratch serves it (the scratch memoizes storage and exact per-revision quotes; the
  determinism checks already run 1 and 3 workers), so the envelope stays bit-identical. Cost:
  one more scratch (a worker's size); parallel work inside slots keeps the workers' own.
- **`SIM-02-K` part 5 (third commit) — the job search's scratch and leg storage prepared at
  load.** The first search after loading built its eight shard scratches (9.6 ms at 100k) and
  its network-leg storage (4.3 ms). Both are functions of the loaded network: the shards are
  sized by the pool and prepared as the search prepares them, and the leg storage is emptied
  per CCH and lane revision, its legs exact per revision. The legs themselves are not warmed:
  which legs the next searches need is not known at load, and warming the complete commute
  quotes, which carry a historical frontage price, would change which quotes later searches
  reuse (a gameplay change). Alternative: build the hour's job supply (its arrival targets,
  0.6–0.8 ms) at load too; not done, it is keyed by the hour and cheap.
- **`SIM-02-K` part 5 (third commit) — the 30k 32× hour frames left for the fourth commit.**
  Full-day trace after the third commit (instrumented): the worst 30k 32× frames (11.5–13.2 ms
  in `PERF-07`; 11.1–14.0 ms traced) are the even hours' first frames, each holding the hour's
  planning slots 0–3 (about 4.5–5 ms: residential 2.2–2.6, commercial 0.7–0.9, industrial
  0.6–1.2, actions 0.6–0.8 ms), one building removal at slot 4 (1.1–1.3 ms), the cell-lot region
  that removal dirtied (2.2–3.4 ms of generated-cell regeneration over the vacated envelope's
  halo, which creates no lot), the agent tick (1.2–2.0 ms), trip events (about 0.4 ms) and the
  snapshot. The per-slot setup is only about 1–1.5 ms of such a frame (pass rows 0.5–0.7, live
  workforce 0.35–0.4, owner storage about 0.3, summed over its slots). Hours without a removal
  peak at 9.5–10.2 ms, so no setup work alone reaches 8 ms. Options: (a) start the removal
  window (`REMOVAL_WINDOW_START`, slot 4) later than slot 4, one 32× frame or more after the
  planning slots (for example slot 16), so removals and their cell-lot regions leave the hour
  frame; buildings are removed about 45 game seconds later; (b) space the four planning slots
  apart (offsets 0, 0, 0, 1, 2, 3 → for example 0, 16, 32, 48) so each lands in its own 32×
  frame; each slot then plans on a city some slots older or newer than now; (c) make the
  cell-lot regeneration after a removal cheaper: the halo of a small vacated envelope
  regenerates about 1,000–2,000 cell candidates along 40–50 roads (zoning work, exact); (d) make
  the setup incremental (maintained totals, lot lists, owner rows on demand), worth at most
  1.5 ms at 30k and 2–3 ms a planning slot at 100k. With (a) or (b) and (d) the 30k 32× hour
  frame would be about 7–8.5 ms. (a) and (b) are scheduling choices with a gameplay side (the
  timing of removals, or the age of the city a slot plans on); the rules for this run let the
  coordinator decide them (keep the envelope closest, prefer the simpler option), so this is
  not a decision of this document. Left for the fourth commit.
- **`SIM-02-K` part 5 (third commit) — the 01:00 residential slot at 100k left for the fourth
  commit.** A storage trace (instrumented) shows the census storage's copy prepared at load
  with the live books' row counts and spare rows, but the census's refill of the copy in the
  next window then grows its business receipt arena (capacity 46,280 → 99,342) and allocates
  its customer staging rows (0 → 163,290), so the 01:00 planning on that book grows the business
  book's receipt journals, which follow the arena's capacity (99,316 → 198,632 entries), and
  writes fresh pages: the residential slot costs 10.9–11.9 ms there against 8–9.2 ms steady
  (owner storage 1.6–2.1 against 0.35 ms), and the frame 17.6 ms. Options: copy the live books
  into census storage with their capacities rather than their lengths (`Clone` gives exact
  capacity), so the census refill finds what an uninterrupted game's census storage holds; or
  grow the receipt journals by the spare-capacity rule when the arena grows during a census.
  Worth about 2 ms on one frame a load.
- **`SIM-02-K` — the `PERF-01` planning cost at 30k fiscal stress (part 5).** The second
  commit's 3% planning rise (729–744 against 710–715 ms a day) was checked. No planning-setup
  loop walks capacity instead of length: the owner rows grow from the live length to the owner
  count each slot (O(owners − live) writes in about five arrays a book, as before), the journals'
  `seen` arrays are cleared only when their generation counter wraps, and only the receipt
  journals follow arena capacity, and only when the arena grows. The remaining suspect is the
  larger arrays' memory footprint, which was not measured.
- **`SIM-02-K` part 5 (fourth commit) — the hour's planning slots a game minute apart, and the
  removal window a minute after them.** The 30k 32× maximum (13.4 ms in `PERF-07`) was the even
  hours' first frame, which crossed slots 0–4 together: the four planning slots (about 4.5–5 ms),
  one building removal at slot 4 (1.1–1.3 ms) and the cell-lot region it dirtied (2.2–3.4 ms), with
  the agent tick (1.2–2.0 ms), trip events and the snapshot. Hours without a removal still reached
  8.1–9.0 ms (diagnostic): the four planning slots alone, with an ordinary frame's agent tick and
  snapshot, exceed 8 ms. This is a scheduling choice with a gameplay side, taken under the run's
  rule (keep the envelope closest, prefer the simpler option). Alternatives rejected: (a) only start
  the removal window later: removes the removal and its region (about 4.5 ms) but leaves the 8–9 ms
  planning frame; (b) space the planning slots but start removals right after the actions slot (slot
  49): the actions slot, the removal and its region share a frame again (about 8–9 ms); (c) a
  cheaper cell-lot regeneration after a removal: exact, but the region's generated cells are rebuilt
  by the zoning completeness cache over the vacated envelope's halo, and skipping work that creates
  no lot needs a proof that the removal changed no blocker, zoning work beyond this part; alone it
  leaves the 8–9 ms planning frame; (d) the incremental setup: at most 1.5 ms at 30k, alone not
  enough, and the larger change. Chosen: the residential, commercial and industrial slots and the
  actions slot at the first slots of the hour's minutes 0, 1, 2 and 3 (slots 0, 16, 32, 48;
  `PlanningSlot` gives the offsets), and the removal window and the level-change queue from minute 4
  (slot 64, `REMOVAL_WINDOW_START`). One slot a game minute does not depend on `SLOTS_PER_MINUTE`
  and clears a 32× frame's 8.5 slots with room; a speed above 32× would put two slots in one frame
  again. Removals still wait out the census window (slots 480–959) for the reason `SIM-02-J` part 3
  records, and still never run between the planning slots, where a swap-remove would invalidate the
  published books the later slots lend (a compile-time check keeps the window after the actions
  slot). Gameplay: the commercial, industrial and actions slots plan 16, 32 and 48 slots later on
  the same published census (the freight price phase's authored boundaries are on the hour or half
  hour, so the books stay current); the hour's spawns are queued from minute 4 instead of minute 1;
  clearances, compatibility removals, departures and level changes are served from slot 64 instead
  of slot 4 (3.75 game minutes later); the hour's admissions are created from slot 48. The removal
  window shrinks from 476 to 416 slots an hour, far above the queues' rates (quota 8 removals and 1
  departure a slot; at most 48 departures a day). Evidence: the envelope passes with no failures
  against the third commit: net income within 0.8% (baseline 30k 12.87 against 12.97), employment
  and stock metrics within 0.3%, households, jobs and buildings within 0.2%, bankruptcies small
  counts (6 against 4, 90 against 85, 79 against 73 events), trip failures 0–6 events a city, the
  same 8 notes. Bursts pass: the worst minute holds 4.1% of an hour's departures (3.75% before), the
  worst settlement hour 7.4% (unchanged). Determinism shows no differences. `PERF-07`: the 30k 32×
  maximum falls from 13.4 to 8.2 ms.
- **`SIM-02-K` part 5 (fourth commit) — saves keep the planning span's census and the household
  diagnostics; save format 104.** A save before the actions slot keeps the census the hour's
  first slot published (`census_inputs.published`); the span grows from slots 0–2 to 0–47, so a
  save at slots 3–47, which the old build saved without it, is not loadable by the new rules:
  `SAVE_VERSION` and the only loadable version become 104. The reload continuation check then
  found the hour's first record after a load at 13:37 differing in net open jobs (1 against 0):
  the load recounted the admission and removal diagnostics from the loaded city
  (`refresh_household_diagnostics`), while the uninterrupted game shows the actions slot's count,
  taken at slot 48 now and 45 slots of job searches earlier than slot 3 was. It matched before
  by chance. Alternatives: (1) recount at load and exclude the diagnostics from the comparison
  (hides a real difference: the shown housing balance and the records differ after a load);
  (2) recount them at load from saved inputs (the live workforce of slot 48 is gone). Chosen:
  save both diagnostics (`demand_diagnostics`, every field, four bytes each) and restore them at
  load; the load no longer recounts them, a policy change still does. The Kuopio fixture gains
  the table with default diagnostics, what a re-save of it writes, and its hash follows.
- **`SIM-02-K` part 5 (fourth commit) — the census storage copy keeps the live books' arena
  capacity, and the census's receipt estimate keeps spare capacity.** At 100k the 01:00
  residential slot cost 10.9–11.9 ms against 8–9.2 ms steady (diagnostic, third commit): the
  load-time copy (`Clone`, exact to its rows) of the business receipt arena (capacity 46,280
  after preparation) and the consumer claim staging (0) were grown by the census that refilled
  the copy (to 99,342 and 163,290), and the 01:00 planning slot lending the copy then doubled the
  receipt journals sized by the arena (99,316 → 198,632) and wrote fresh pages. Copying with
  capacities alone would not do: the census reset sized the arena exactly to its estimate
  (`reserve_cleared(2 × (input ports + output ports))`), so the load-time census left the live
  arena exact (99,316) and the next census, whose estimate was 26 receipts larger, doubled it
  and then every journal it sizes. Alternatives: (1) grow the journals by the spare rule when
  the arena grows (they already do: `spare_capacity` doubles them, the growth itself is the
  cost); (2) a capacity-preserving clone of every book (dozens of arrays, most of which the
  preparation already grows with spare rows). Chosen: the reset's estimate takes the spare rule
  the other arenas grown by request have (`spare_capacity`), and the copy reserves the live
  books' capacity for the two arenas the census refills (`JobFunding::reserve_census_arenas_like`).
  Capacity never changes a result; memory grows by at most a quarter of the receipt arena.
  One unit test that relied on an exactly sized first arena to force a replay
  (`finite_finance_replay_preserves_every_receipt`) sizes its reset smaller.
- **`SIM-02-K` part 5 (fourth commit) — the 30k 32× minute-4 frames (findings, not taken).** After
  the spacing the 30k 32× maximum is the frame that crosses slots 64–65 of an hour with clearances
  (`PERF-07` 8.0–8.2 ms, three frames over 8 ms; diagnostic 6.7–8.5 ms): the hour's clearances at
  slot 64 (1.0–1.2 ms with the zone-index and entrance-cache repair), the cell-lot regions they
  dirtied, served at slot 65 (2.5–3.7 ms, generated-cell regeneration that creates no lot), and the
  hour's first queued construction start, due at minute 4 too (0.65–0.83 ms), with the agent tick
  (1.1–1.7) and snapshot (0.5–1.1). Options, cheapest first: (a) start the queued spawns a minute
  later than the removals, or the removals a minute later than the first spawn (each queued spawn
  starts one a minute, so with several spawns an hour one still shares the removal frame; worth
  about 0.7 ms); (b) let the `cell lots` queue serve a region no sooner than a game minute after it
  was dirtied, which splits the clearances and their regions into separate 32× frames (worth 2.5–3.7
  ms; lots appear up to a minute later, a gameplay change); (c) make the regeneration after a
  removal cheaper and exact: the removal marks the vacated envelope dirty in the cell store's
  completeness cache, and the region regenerates every generated cell of the envelope's halo,
  probably although no blocker of those cells changed (not verified; zoning work, identical
  results); (d) the incremental per-slot setup does not help this frame. (b) or (c) alone brings the
  frame to about 5 ms. Taken in the fifth commit: option (c).
- **`SIM-02-K` part 5 (fourth commit) — the 100k 1× hour frames (findings, not taken).** The
  remaining 15.2–16.4 ms hour frames at 100k 1× (`PERF-07`) are each hour's residential slot
  (about 9–9.5 ms; 8.0–9.2 ms in the third commit's 300-minute trace, diagnostic) with an
  ordinary frame's agent tick
  (3.6–4.4 ms), restock requests (0.4–1.0), trip events (about 0.5) and snapshot (about 1). The
  residential slot is its site preparation (`SiteSupply::new`: the hour's supplier and buyer
  site-row merge and layout and the owner storage, 3.6–5.0 ms), the queued projects' replay
  (1.2–1.4), the decisions (1.6–1.8), the pass's reservations (0.75–0.8) and the snapshots
  (0.46–0.54). No cheap identical fix: the site rows and the setup would have to follow what
  changed (the per-slot setup item); spacing the slots does not change 1× frames, each of which
  already holds at most one slot.
- **`SIM-02-K` part 5 (fifth commit) — a building leaving a lot it keeps derives the lots
  without regenerating cells.** The 30k 32× maximum (8.2 ms) was every hour's minute-4 frame:
  the hour's clearances (1.0–1.2 ms), the cell-lot regions they dirtied (2.5–3.7 ms, of it
  2.2–3.2 ms regenerating generated cells that a comparison found identical before and after
  in all 17 regions of the day, and 0.34–0.53 ms deriving lots, creating none) and the hour's
  first construction start (0.65–0.83 ms), with the agent tick and snapshot. Alternatives, in
  the order this run's rules prefer them: (c) an exact, cheaper regeneration: either bound the
  regeneration by the cells the removal touched (it touched none: no input of generation
  changed, so any bound regenerates cells that cannot change) or skip the regeneration that
  provably changes nothing; (b) serve a dirtied region at least a game minute after it was
  dirtied: splits the frame but keeps the 2.2–3.2 ms of work in minute 5's frame, next to the
  hour's second construction start, and lots appear up to a minute later (a gameplay change);
  (a) start the hour's first construction a minute away from the removals: 0.65–0.83 ms, not
  enough alone. Chosen (c), the skip, with its proof by inputs: cell generation reads roads and
  their alignments, paint, lot claims, cells outside the envelope, curve sources, authored
  parcels and the caller's blockers (compiled roads, explicit sites, fields), and a building
  leaving a lot whose paint is intact changes none of them (occupancy is parcel-store state; a
  cell-lot building is no explicit site; a removed field invalidates its own polygon). A
  release that frees the claims still invalidates the halo. The region's derivation is kept, so
  the vacated lot's frontage is still refreshed and a lot left on a removed road retired. The
  skip relies on the completeness cache's own invariant (a complete chunk holds what generation
  of its area produces from current inputs), which every ready-chunk lookup already relies on.
  Evidence: envelope and bursts bit-identical to the fourth commit, determinism shows no
  differences, the new unit test, the cell store compared before and after all 17 regenerations
  of the 30k 32× day (every one identical), and the `PERF-07` table.
- **`SIM-02-K` part 5 (fifth commit) — the render-snapshot spikes are preemption (findings, no
  code change).** The 30k 1× maximum (8.1 ms, of it 6.9 ms of render snapshot) and earlier
  snapshot maxima (10k 6.2 ms on the third commit, 30k 9.0 ms on the second's control) were
  single frames at different minutes in every run, against a snapshot mean of 0.36 ms. Per-frame
  phase timing and scheduler statistics (results section) put every snapshot frame over 2 ms on a
  pool worker that waited 1.0–2.8 ms for a CPU, in a different phase each time, with no rebuild;
  the agent tick has the same spikes (worker waits of 3.6–5.4 ms in the 30k frames with a
  4.2–6.3 ms tick). None of the suspected snapshot causes applies: the lane boxes rebuild only
  after a road edit, the site and zone index were clean, and each stretched phase took its
  ordinary time plus the worker's wait. Alternatives: (1) run the snapshot's small phases
  serially: the stalled thread time is the same, so the expected count is too; (2) fewer workers
  than pinned CPUs, so other tasks find an idle CPU: a change of the gate's configuration (8
  workers on the 8 performance cores) for every step's comparison; (3) reserved CPUs or
  real-time priority: machine configuration, outside the project. Chosen: no code change; the
  frame budget section states the effect and how to attribute a frame, and timing runs keep the
  machine quiet, their own monitoring included. The final `PERF-07` run, during which the run's
  own monitoring started only three short commands ten minutes apart, still had them (largest
  snapshot 3.6 ms at 30k 1× and 6.3 ms at 100k 1×; control 4.7 and 6.1 ms), so they come from the
  machine, not from the run.

- **`SIM-02-K` part 4 (second commit) — a minute's departures leave over its slots.** This is a
  gameplay change, taken under the run's rule for gameplay-side choices (keep the envelope
  closest, prefer the simpler option) because the envelope stays within tolerance (net income
  within 0.46%, employment within 0.3%; results section). The 100k 1× agent-tick peaks
  (10–14 ms) were the departure waves: 250–405 trip plans of about 120 µs each in the movement
  pass of the frame that crossed into the minute. Alternatives:
  1. *Plan the wave on the pool in a balanced batch and apply in agent order* (identical): 40–50
     ms of planning CPU over 8 workers still leaves at least 5–6 ms plus the tick's other
     2–3.5 ms, so it cannot reach the target.
  2. *A departure wheel or active-agent list* (identical): removes the per-frame visits of about
     62,000 indoor agents at 100k (0.3–0.5 ms a frame over the claim, speed and movement passes),
     but the wave's plans happen in the same frame as before, so it does not touch the waves; it
     also needs wake calls in every writer of what an indoor agent reads (households, logistics,
     hiring, the building store).
  3. *Plan ahead, in the frames before the minute*: not identical, since a plan reads the live
     frontage delays, and it keeps a second copy of each plan alive.
  4. *Cheaper plans*: routing work (`NET-05`), not attempted in this part.
  5. *Spread each minute's departures over its 16 slots by the agent's seed* (taken): the
     simplest change; departures leave within the same minute, up to 15/16 of a minute later, so
     each departure keeps its minute and the minute-based burst counts see no new peak.

  The slot is `floor(frac(seed/φ) × 16)` (`slot_in_minute_for_key`): the schedule seeds that
  share a departure minute (the minute is the seed modulo the window span) spread evenly over
  all 16 slots, which a plain `seed mod 16` would not (with a span that is a multiple of 16 the
  slot would be a function of the minute). The day's last minute leaves at its first slot, so a
  32× frame (about 8.5 slots) that spans midnight cannot skip a departure; elsewhere a frame that
  passes the slot leaves at the next tick, still in the minute or in the next one, as before. At
  100k 32× a frame covers about half a minute, so the waves only halve there (open question 6).
- **`SIM-02-K` part 4 (second commit) — `current_congestion` deleted, save format 105.** Nothing
  in gameplay read it; the frontage delay cache, which plans do read, is separate and unchanged.
  The Kuopio fixture drops the column by SQL (`ALTER TABLE … DROP COLUMN`, version 105).
- **`SIM-02-K` part 4 (second commit) — loading prepares the first tick's scrub and lane
  buckets.** Alternative: save the lane-bucket snapshot (a save change for derived state the load
  rebuilds in about 1.6 ms). The preparation runs last in the load, after the planning warm-up, so
  nothing between it and the first tick changes agents or building references; a later revision
  or invalidation makes the tick scrub or rebuild as before. The 98 trip plans left in the first
  tick are ones the uninterrupted game makes too.
- **`SIM-02-K` part 4 (second commit) — indoor agents stay in the per-frame passes (findings).**
  The profile shows the indoor visits are cheap per frame (part of the 1.26 ms parallel movement,
  0.51 ms claims and 0.34 ms speed at 100k 1×) and not the source of any peak. The exact wheel
  design of the entry "`SIM-02-K` part 4 — indoor agents stay in the movement pass (findings)"
  stands (an active-agent list kept by the transit transitions, a departure wheel keyed by
  absolute slot now that departures have a slot, wake calls in the household, logistics, hiring
  and building-reference mutators). With slot departures the wheel key is simpler: `(day, minute
  × 16 + seed slot)`. Worth 0.3–0.5 ms a frame; it does not touch the waves.
- **`SIM-02-K` part 4 (second commit) — the cost of a trip plan (findings).** A building-origin
  plan costs 90–190 µs (about 120 µs typical) at 100k: walking and driving candidates over the
  node-rank pairs, then the exact path. At 100k 32× plans are 6.7 ms of CPU per ordinary frame
  (51 plans) and commute estimates 1.7 ms (35); wave frames still carry 171–258 plans because a
  32× frame covers half a minute of departures. Halving the plan cost would take about 0.4 ms off
  the 32× ordinary tick and 1.5–2 ms off its wave frames; it is routing work for `NET-05`, and
  bears on open question 6.
- **`SIM-02-K` part 4 (second commit) — the remaining 100k 1× agent-tick frames over 6 ms are
  preemption.** All 17 had a pool worker waiting 1.9–4.7 ms for a CPU in that frame, as in the
  entry "`SIM-02-K` part 5 (fifth commit) — the render-snapshot spikes are preemption (findings,
  no code change)".
- **`SIM-02-K` part 5 (sixth commit) — each census's book prepared for planning at minute 59.**
  The residential slot was the only planning slot that merged site rows and laid them out (100k
  4.27 ms of its 8.95: row merges 1.27, layouts 2.21; 30k 1.43 of 2.59; diagnostic): a book is
  lent in one hour's slots and refilled by the next census, so its residential slot met rows two
  hours old and a refilled census book whose source base and port offsets had moved, and rebuilt
  the supplier index and the buyer port index in full. Alternatives: (a) keep one set of site
  rows across the census swap (rejected in the entry "`SIM-02-K` part 5 — planning books
  prepared at load and when a fresh census book finishes": the business book's port maps belong
  to its layout; it would also still merge an hour of changes and lay out again every hour,
  because the refilled book's offsets move); (b) incremental layouts that insert and remove only
  the changed rows in both indices: source handles are numbered after the census's live slots, so
  every append shifts them, a deep change in two books (the planned "site rows kept
  incrementally" item, left); (c) prepare in the window slot right after the census finishes, as
  for a new world's first census: the census plans to finish 96 slots early, so up to six game
  minutes of construction starts (one a minute from minute 4 while the queue lasts) would
  invalidate the rows and layouts again; (d) prepare at the hour's last slot: at 32× it would
  share a frame with slot 0's residential slot; (e) slice the preparation over two slots
  (supplier rows, buyer rows): two lendings, each with its pass rows, for about 2 + 2 ms, not
  needed while the actions slot is the larger frame. Chosen: a census start clears its book's
  `planning_prepared`, and the existing window-slot preparation runs from minute 59's first slot
  (`PLANNING_PREPARATION_SLOT`, 944) once the census is done; a census done later is prepared at
  its next window slot or by its residential slot, as before. Minute 59 leaves only that
  minute's events (its construction start, a completion) to invalidate the rows, and is a game
  minute before slot 0, so at 32× the two are in different frames. Evidence: in every steady hour
  of the diagnostic runs (100k 1× over 250 minutes, 30k 32× over the day) the residential slot
  found its rows and layouts current; the preparation takes 4.4–6.4 ms at 100k and 1.4–2.0 ms at
  30k in its own slot; the envelope and bursts are bit-identical to part 4's second commit; a new
  test plays a game whose window prepares the books against one whose residential slots prepare
  them, frame by frame for three hours. Results never depend on when the derived rows are
  prepared: they are keyed by the city's revisions (the first-census preparation's argument, now
  exercised every hour).
- **`SIM-02-K` part 5 (sixth commit) — the preparation waits while the zone index is stale.** The
  preparation first prepares the building site index, and a stale zone index's rebuild also
  resets the vacancy lists to building order, which admission's tie-breaks read (`ALLOC-03`; a
  load re-marks a saved stale index for the same reason). Alternative: let the window rebuild the
  index; it would move the vacancy order at minute 59 instead of where the index's own reader
  puts it, a gameplay change. Chosen: the window waits while
  `BuildingAllocator::zone_index_needs_rebuild` holds and prepares at the first window slot after
  the index's own reader rebuilt it; a book still unprepared at its hour is prepared by its
  residential slot. A new test checks that a stale index defers the preparation. No hour of the
  full-day runs waited.
- **`SIM-02-K` part 5 (sixth commit) — large row growth filled on the pool.** Every pass truncates
  the owner rows it appended to its three books (the business book's owners, finance, buyer heads
  and tails and staging; the input book's owner receipt chains; the customer book's owners) and
  the next pass appends them again: about 190,000 rows an array at 100k, 0.56 ms of serial fills
  a lending (business 0.37, input 0.12, customer 0.07; timing points in a test build).
  Alternatives: keep the appended rows across passes, restoring them to their defaults through a
  journal of the rows a pass wrote (the books' row counts would then differ between passes, which
  the census refill, the test-build fingerprints and every reader of the lengths see; not taken);
  keep the serial fill. Chosen: `resize_rows` and `grow_rows` append 16,384 rows or more with
  `par_extend` over `rayon::iter::repeat_n`; every appended row is the fill value, so the rows
  equal a serial fill's. The fills fell to 0.15, 0.08–0.11 and 0.03 ms a lending; the actions
  slot, which lends twice, by 0.6 ms.
- **`SIM-02-K` part 5 (sixth commit) — the 100k 1× slot target not reached (findings).** After
  this commit (diagnostic, 100k, steady hours): residential 4.87 ms, commercial 3.71, industrial
  4.54, actions 6.46 and the census book's preparation 4.4–6.4 ms in their own slots, planning
  frames up to about 12 ms. Every lending pays about 3 ms of O(B/p) setup at 100k, each part a
  parallel pass over about 60,000 buildings or 45,000 households at 0.3–0.9 ms: the pass's
  funded operations (0.24–0.31) and operation sites (0.49–0.55), the live workforce (0.45–0.56),
  the workforce calendar's reset (0.39–0.42), the zone's existing buildings (0.48–0.78), the lot
  scan (0.27 commercial, 0.86–0.94 industrial), and the snapshot and storage (about 0.5). The
  decisions are 0.002–0.35 ms. Slot-specific: the actions slot's admissions (1.17 ms, of it 1.13
  ms `BorderArrivalCosts::new`, one CCH sweep per car-connected border, which depends on the
  network alone and could reuse the route cache's border sweeps, kept per CCH generation and
  metric revision) and its construction epoch (1.9 ms: a second lending with its own pass rows,
  front, compose and site storage, 1.05 ms, and its replay, 0.84 ms); the residential slot's
  re-proof of the constructing buildings (1.19 ms for 21 buildings, against 0.14 ms for the same
  buildings in the next slot: the hour's first quotes on the book are cold). None of these needs a
  decision of this document: the next step keeps them by what changed — pass rows kept across the
  lendings of a book (which also removes most of the construction epoch's second preparation;
  keyed by the book's census, the building log, the entrance revision, policy and the freight
  price phase, with the live rows a pass wrote recomputed), border arrival costs reusing the
  route cache's border sweeps, workforce and calendar totals maintained by the job, household and
  building mutators, and the zone lists for existing buildings and free lots. Slicing the
  decisions (open question 1) would not help: each split slot would pay the setup again.
- **`SIM-02-K` part 5 (seventh commit) — pass rows kept across a book's lendings, keyed and
  recomputed from the building change log.** Every lending of the planning books derived every
  live building's operation and site row again (0.8 ms at 100k, six lendings of one book an
  hour). Alternatives: (a) two row sets per book (the census book's and the live book's): the
  same book is lent by the minute-59 preparation before its publication and by the hour's slots
  after it, through one `ProjectReservations`, so one set suffices; (b) maintain the rows from
  every mutator of their inputs (the census, building edits, entrance edits, policy): many
  writers in other systems; (c) key the rows and recompute what the existing building change log
  names. Chosen: (c). The key is the lent book's rows generation, the freight price phase
  (`freight_price_phase`, the same key the census books' quotes use), catalog and tuning by
  address and the fiscal policy by value; the building log (`building_changes_since`) names the
  changed rows, appended rows are those past the previous live count, and the entrance revision
  decides whether every site row is derived again. The pass's own writes to live rows (an
  accepted replacement, the financing its proof moved) are recorded and derived again by the
  next start. A change set larger than an eighth of the city derives every row on the pool.
  Evidence: test builds compare the kept rows with a full recomputation at every lending, across
  the library suite; envelope bit-identical; pass rows 0.81–0.84 → 0.05–0.07 ms a lending at
  100k.
- **`SIM-02-K` part 5 (seventh commit) — the rows generation drawn per census-step run and per
  `published` change.** Alternative: bump it in each column writer. All funded-row writers are
  the census steps and the invalidations, and every invalidation already clears `published`, so
  `JobFunding::rows_generation` is drawn afresh when a book is created or cloned, on every
  census-step run and on every `published` change, all through `set_published`.
- **`SIM-02-K` part 5 (seventh commit) — `CityFiscalPolicy` derives `PartialEq`.** The kept rows
  compare the fiscal policy by value, as the decision above keys it; the comparison is a derive
  on the existing struct, with no new state.
- **`SIM-02-K` part 5 (seventh commit) — border sweeps kept in the allocator, not reused from
  the freight route cache.** The earlier finding suggested the route cache's border sweeps. They
  use a different border filter (any non-deleted edge, against admission's car-connected road
  edge) and would couple admission to another system's cache lifetime. Chosen: the allocator
  keeps them under the CCH build generation, metric revision and node-role revision, as the
  freight topology keys its border list, and sweeps afresh without keeping while edited chunks
  wait for the CCH rebuild. The sweeps read the CCH alone and the border set the node roles and
  edges, which every topology edit rebuilds. Evidence: a targeted test and the test-build
  comparison with a fresh sweep; the actions slot fell by about 1.1 ms at 100k.
- **`SIM-02-K` part 5 (seventh commit) — the minute-59 preparation's second half at slot 953,
  not 945.** The preparation took 4.9–5.3 ms at 100k in slot 944, about 2.2 ms supplier rows and
  layout and 1.7 ms buyer rows and layout. Alternatives: slot 945 for the second half: at 32×
  both halves share a frame (30k: 1.14 + 0.82 ms against 1.68 ms before); a slot a minute away
  (the next hour's minute 0 holds the residential slot). Chosen: slot 944 prepares the supplier
  rows alone, slot 953 or a later window slot everything else: nine slots apart, more than a 32×
  frame crosses. The second lending pays almost no pass rows (they are kept). Gameplay: none;
  derived state.
- **`SIM-02-K` part 5 (seventh commit) — no removal spacing; identical-result parallel scans and
  a rebuild instead.** The 100k 1× minute-4 frames were the removal window's first slot
  (3.9–5.1 ms of `building removals`: one zone-index rebuild 2.6 ms and about 0.65 ms of serial
  scans per removal, plus remaps), not a removal plus a cell-lot derivation plus a construction
  start; spacing the removals (one a slot) would leave the zone-index rebuild in each slot and
  change when buildings leave. Chosen: no scheduling change; the scans filter on the pool and
  update the found entities in ascending order with the same updates, and the rebuild derives
  each building's entries on the pool and fills the lists in building order, so every result is
  identical. An incremental zone index (no rebuild per removal slot) was rejected: it would reset
  the vacancy order differently (`ALLOC-03`), a gameplay change. The eleventh commit's kept index
  preserves that order (see "the zone index kept across the removal queue's swap-removes" below).
- **`SIM-02-K` part 5 (seventh commit) — maintained workforce totals deferred.**
  `WorkforceView::count_live` sums open-job wages as `f32` in fixed chunks; a total maintained
  by the job, household and building mutators would sum in mutation order and change the bits,
  and a "nothing changed" key would miss every slot, because the job queue hires every slot.
  Exact maintenance needs integer (fixed-point) wage sums, a rounding choice with a gameplay
  side, which the run's rules let the coordinator make; deferred.
- **`SIM-02-K` part 6 — the stall is not traffic; no junction, fixture or census change.**
  Diagnosed with a lane wait-for graph and trip tracking (results section, part 6). Alternatives considered for the long trips: (a) a
  junction or gridlock fix: rejected, there is no cycle and cars flow; (b) a fixture change
  (fewer commercial lots, demand-grown commerce): rejected as the fix, because the census flaw
  shows in any city with spare store capacity and a fixture change would hide it, and it would
  change every later baseline; (c) a bound on the census's store claims (`ECON-22`): prototyped,
  envelope passes, but it needs an authored radius (a new gameplay parameter), leaves households
  beyond it unforecast in sparse cities, does not make replenishment shop locally while stores
  hold no buffer, and its 100k city failed the fixture's continuation check; (d) a
  household-major claim (each household's demand to its nearest store with capacity, then the
  rest): the honest rule, but it restructures the census sweep (resumable slices, journals,
  rejection and retry). Both (c) and (d) are economy design decisions with a measured benefit of
  0.4–0.9 ms of agent tick at 100k, so part 6 lands no code and records the findings.
- **`SIM-02-K` part 6 — network entry without an entry claim, not fixed here.**
  `prepare_lane_entry` puts an agent that starts at a node (border-origin freight, arrival
  carriers, a replan at a node) on its first lane at distance 0 without
  `lane_entry_slot_clear` and `claim_lane_entry`, unlike connector entry and exit, direct
  continuations and building egress (`TRAFFIC-02` leaves claims to the requesting subsystem).
  Agents without a lane are already classified `Dynamic`, so the check fits the serial handoff
  phase. Prototyped (results section, part 6): stacks gone, the same queue now waits at the
  border nodes as serial agents (+0.1–0.2 ms of agent tick at 100k), envelope passes but with two
  new 10k out-of-stock notes not yet explained. Not landed in part 6: it needs that explanation,
  a border hold that keeps waiting trucks off the per-tick serial pass, render-visible checks
  (`./run.sh --test`) and own `PERF-07` cities, and it does not shorten the queue, which is the
  borders' throughput (`TRAFFIC-07`).
- **`SIM-02-K` part 6 — pedestrians keep car spacing.** Walkers are in the lane buckets, so the
  connector entry claim and the end-of-tick overlap correction space them by `CAR_LENGTH +
  IDM_S_MIN` (2.7 m), one walker per 0.7 s per crossing at 4 m/s. The measured walking trips run
  at about free speed (825 game minutes for about 4 km of path), so this is not what makes them
  long; recorded as a limit in `traffic.md`.
- **`SIM-02-K` — part 6 closes as findings.** `ECON-22` (the census's unbounded store claims) and
  `TRAFFIC-07` (network entry without an entry claim) are filed as roadmap bugs outside
  `SIM-02`'s path, because neither changes what `SIM-02` must reach: the census change is an
  economy design choice worth 0.4–0.9 ms of agent tick at 100k, and open question 6 stands with
  or without it; the entry claim removes stacked trucks but not the border queue. The order
  continues with part 5's remaining 100k 1× items, then part 7 (acceptance).
- **`SIM-02-K` part 5 (eighth commit) — the census book's finance masks refreshed at the
  minute-59 preparation.** A census publishes every buyer's financing and marks every buyer's
  finance masks for refresh (76,127 at 100k); the hour's first masked business claim, in the
  residential slot's re-proof of the constructing buildings, refreshed them one at a time
  (0.79–0.85 ms). Options: (a) the same dirty-buyer refresh in the preparation's second half
  (taken: the same function in the same order, moved from slot 0 to slot 953, which has room;
  results unchanged because masks only prune buyers that cannot claim, and test builds assert
  after every pass that masks are never narrower than exact); (b) a parallel full mask reset
  when most buyers are dirty (exact masks, probably cheaper, but narrower masks change scan
  costs and the test builds' quote counters; not needed for the slot); (c) the refresh inside
  the census step that publishes the financing (unbudgeted work in a work-unit-sliced census
  slot, and a census publishes after every proof slice). Evidence: dirty buyers at the first
  claim 76,127 before, 20 after; residential slot 4.22 → 3.42 ms at 100k, slot 953
  2.66 → 3.35 ms (diagnostic).
- **`SIM-02-K` part 5 (eighth commit) — the chunk lists filled over a dense grid, not kept
  registry rows.** The zone-index derivation was first given registry rows kept across rebuilds
  where the building change log names what changed (exact, with a test-build comparison); it
  saved about 0.1 ms, because the lookups by asset id are a small part of reading every
  building, and it would have tied the zone index to every asset and zone change being logged,
  so it was reverted. A counting sort of the (chunk, building) pairs saved only 0.24 ms (its two
  serial passes cost 0.35 ms); filling the chunk lists in the ordered fill over a dense grid of
  the chunk bounds saves 0.59 ms and needs no sort at all. Same lists, no gameplay side.
- **`SIM-02-K` part 5 (eighth commit) — whole-city test-build checks in timing runs.** `PERF-07`
  and the diagnostic runs execute the library test binary, so `#[cfg(test)]` code runs in them;
  a new whole-city check must be gated by `debug::work::city_checks()` (switched off by the
  populated runs), as the kept pass rows' check is. Found when an ungated check doubled the
  zone-index rebuild in a diagnostic run.
- **`SIM-02-K` part 5 (eighth commit) — the 100k 1× target not reached; scheduling choices
  next.** After this commit the 100k 1× frames over 8 ms that are not preemption all fall in the
  hour's six slot minutes (:59 census-book preparation, :00 residential, :01 commercial, :02
  industrial, :03 actions, :04 removals): slot work 4.2–4.9 ms (the minute's system 3.0–4.4 ms
  plus restock requests and timed events 0.3–1.4 ms) on a 3.6 ms agent tick and a 0.8 ms
  snapshot. No remaining single setup cost is large enough: the largest left are the zone lists
  and lot scan (0.45–1.1 ms a zone slot), the zone-index derivation (0.74 ms) and the maintained
  workforce totals (about 0.9 ms a lending, needing fixed-point wage sums). Options: (a) slice
  each planning pass over two slots of its minute (preparation and snapshot, then the plan), a
  scheduling change of a few game seconds; (b) keep the queues (restock, job, freight requests)
  out of the six heavy slots, costing them six of 1,024 slots an hour; (c) the agent-tick items
  of part 4 (indoor agents off the per-frame pass, 0.3–0.5 ms a frame). These are scheduling
  choices with a gameplay side, which the run's rules let the coordinator make (keep the
  envelope closest, prefer the simpler option), not decisions of the spec. Chosen order: (b)
  first, the simplest, in the next commit; then the first hour after loading; then (a) if
  needed. Not taken in this commit: each changes gameplay timing and none was needed to show the
  attribution.
- **`SIM-02-K` part 5 (ninth commit) — the request queues leave the hour's seven heavy slots to
  the slot after.** After the eighth commit every 100k 1× frame over 8 ms that was not preemption
  held one of the hour's planning slots (0, 16, 32, 48), the removal window's first slot (64) or
  the census book's two preparation slots (944, 953): the slot's own system (3.0–4.4 ms) plus
  restock and job requests and timed events (0.3–1.4 ms) on a 3.6 ms agent tick and a 0.8 ms
  snapshot. **This is a gameplay timing change**, taken under the run's rule (keep the envelope
  closest, prefer the simpler option). Options: (a) skip the queue in those slots without carrying
  its share (simplest, but every queue loses seven slots of quota an hour and a credit capped at
  one whole visit loses its accrual, so the hourly totals change); (b) the slot after serves the
  skipped share and its own as one batch of twice the quota (one batch where two slots made two: a
  full job or restock queue then plans two slots' requests against one state); (c) spread the
  skipped share over the following slots (a saved debt per queue and a rule for overlapping debts;
  more state for no measured gain); (d) the slot after serves the skipped slot's share first, then
  its own, as two calls (taken: every queue keeps its quota per hour, each credit accrues and pays
  out exactly as before one slot later, a full queue keeps one batch per slot's share, and the rule
  needs no saved state because it depends on the slot alone). Rejected: dropping the share, one
  double batch, a saved debt. Which slots: those whose own system costs 3–4 ms at 100k; the
  cell-lot queue is skipped too (a region regenerated in a planning slot added 0.5 ms there). No
  two of the slots are adjacent, and each one's next slot is in the same game minute (compile-time
  checks), so no share waits more than one slot (62.5 ms of real time at 1×, about 3.75 game
  seconds) and no minute-level count changes. Saves need nothing new. Evidence, against the eighth
  commit: the envelope passes with no failures and the same 8 notes. Net income per resident is
  within 0.68%, employment within 0.71%, jobs, households, residents and buildings within 0.2%,
  deliveries within 0.5% except the 10k fiscal-stress city (8.23 → 8.75, its startup freight,
  already a note). The larger relative changes are small counts: bankruptcies about 6 → 4, 83 → 87
  and 81 → 86 events, 1–4 unhoused households out of about 15,000, 0–6 trip failures a city.
  Bursts pass (the worst minute holds at most 3.7% of an hour's departures, the worst settlement
  hour 7.4%) and determinism shows no differences. The slots after the heavy ones now carry the
  moved work: 0.7–1.0 ms at 100k (0.4–0.6 ms before), and slot 65 2.42 ms (0.98 ms) (diagnostic).
- **`SIM-02-K` part 5 (ninth commit) — the building removals keep slot 64.** The removal window's
  first slot is queue-free for every queue but `building removals`, because the removals are slot
  64's own system (3.3 ms at 100k, 1.5 of it the zone-index rebuild). Moving them a slot would
  only relocate the 3.3 ms to slot 65 and make that slot a heavy one, with the cell-lot regions
  they dirty. Household departures and level changes, which are other queues, start at slot 65
  and serve slot 64's share there.
- **`SIM-02-K` part 5 (ninth commit) — trips and construction starts due in a queue-free slot
  fire in the next, moved when scheduled.** The removal window's first slot also held the hour's
  first construction start (0.5–0.6 ms; 2.7–3.0 ms the first hour after loading) and every heavy
  slot its share of trip events (0.1–0.4 ms on average, up to 1 ms). Options: (a) defer at pop
  time, leaving the slot's events in the wheel for the next slot (rejected: the wheel's next slot
  must equal the clock's next slot, or every frame restarts the wheel, `align_events_to_clock`,
  and saves would hold a wheel one slot behind its clock); (b) move only the construction start,
  or start every construction at the middle of its minute (the second changes every start by eight
  slots and every completion with it, a larger envelope change; the first leaves the trips); (c)
  schedule a trip or construction start due in a queue-free slot one slot later (taken:
  `event_slot`, the same game minute, a freight trip recording the slot it fires in as its live
  event). Construction completions keep their deadlines: passive checks read
  `construction_complete_slot`, and a completion due in a queue-free slot only follows a start
  placed there by an editing path, since queued starts no longer are. Events saved before the
  change keep their slots.
- **`SIM-02-K` part 5 (ninth commit) — spare rows at every rebuild, not a load hook.** Measured
  first (diagnostic timing points in the first placement and the freight planning context, 100k
  1×, the first 70 minutes after loading): the first construction start (2.73 ms against 0.51–0.57
  ms at the next two) spent 1.74 ms growing the site clients, 0.41–0.67 ms the entrance cache and
  0.05 ms the zone index's vacancy positions, each rebuilt at load exactly to the building count,
  so the first appended building copied it (capacity 76,107 for 76,107 buildings; 152,214 after);
  and the freight requests after it spent 2.20 ms growing the freight route cache's access rows
  the same way. Not the supplier index: the save holds a current supplier front for the hour, and
  no at-once build ran. Options: (a) a load hook that reserves spare rows in each table (it would
  have to know every table a load rebuilds, and the next full rebuild of one, by an asset reload
  or a frontage move, would size it exactly again); (b) every rebuild of these tables keeps spare
  rows (taken: `reserve_rows`, the rule the planning books' rows already follow; the entrance
  cache and the site clients fill in place). Capacity never changes a result, so the results are
  identical by construction; a test checks each rebuilt table keeps room for the next building and
  the access rows do not move when one more building is prepared.
- **`SIM-02-K` part 5 (ninth commit) — no spec stop; the 100k 1× target not reached.** After this
  commit the 100k 1× frames over 8 ms that are not preemption are the heavy slots' own systems
  alone (86 of 129 in the diagnostic day; slot work 3.3–4.2 ms on an agent tick of 3.6–4.1 ms and
  a 0.8–1.1 ms snapshot, 8.0–10.8 ms, mean 8.6 ms). The systems are the planning passes' setup,
  not their decisions: per lending at 100k the snapshot and live workforce 0.7–0.9 ms, the site
  preparation 0.45–0.5, the re-proof of constructing buildings 0.3–0.7, the zone's workforce
  0.4–0.6, its existing buildings 0.45–0.8 and the lot scan 0.3–1.3, against decisions of at most
  0.3 ms; the census book's preparation halves 3.0–3.5 ms; the removal slot's zone-index rebuild
  1.5 ms of 3.3. None needs a spec decision. Options, for the next commit: (a) slice each planning
  pass over two slots of its minute, the setup in the first and the plan in a slot eight later,
  which needs the setup's products (workforce view, composed snapshot, site supply, re-proved
  queue) held across the slots and kept by saves inside the span, or derived only from state the
  slots between cannot change (generalising the slots 0–47 save rule); (b) maintained workforce and
  calendar totals (about 1 ms of each lending; needs the fixed-point wage-sum choice); (c) part 4's
  indoor-agent item (0.3–0.5 ms of every frame); (d) the first lending after a load (00:01–00:03
  frames, 9.2–10.5 ms): one live pass at load would keep the pass rows the first lending builds
  (exact by the seventh commit's key), and the re-proof's freight route quotes are cold after a
  load. (a) is the only option that brings every heavy frame under 8 ms on its own; (b)+(c)
  together are worth about 1.3–1.5 ms and might.
- **`SIM-02-K` part 5 (tenth commit) — each planning slot in a setup and a plan half a minute
  apart.** After the ninth commit every 100k 1× work frame over 8 ms was a heavy slot's own
  system, and the four planning slots held 2.8–3.9 ms each (3.37, 2.77, 3.59 and 3.88 ms at
  100k, diagnostic). This is a scheduling choice with a gameplay side, taken under the run's rule
  for gameplay-side choices (keep the envelope closest, prefer the simpler option). Options:
  (a) setup = the books' lending, site preparation and re-proofs, plan = the decisions (rejected:
  the books would stay lent across slots in which the queues and the census input copy read the
  live city, and a save cannot keep an open pass and its journals);
  (b) two independent passes per zone, spawns in the first and existing buildings in the second
  (rejected: each repeats about 2 ms of books, site preparation and re-proofs, so the halves stay
  at 2.1–3.1 ms, and the claim order inside a zone changes);
  (c) only the actions slot's execution moved eight slots after its plan (rejected alone: the
  zone slots keep 2.9–3.7 ms);
  (d) the pass's pure reads of the live city, the workforce count and the plan's scans of free
  lots and existing buildings (every zone's downgrade candidates for the actions slot), at the
  minute's first slot, held for the plan `PLANNING_SETUP_LEAD` (8) slots later, which lends the
  books, prepares their site rows, proves the queue and the earlier slots' plans again, decides
  and, in the actions slot, executes (taken: pure reads at the setup, everything that lends the
  books at the plan). The held products are small and save as one blob; nothing is held outside
  the four spans. The plan composes its snapshot from the held workforce and the front, pending
  arrivals, treasury and borders as they stand at the plan; the scans ranked lots by the
  commercial demand a snapshot composed at the setup left, which reads only the front and the
  construction queue, so on an unchanged city the plan equals the unsliced pass (a test checks
  it). **Gameplay side:** the plan decides on a workforce and candidates half a game minute (8
  slots, 0.5 s of real time at 1×) old; the queues that serve between (job searches, housing,
  restocks, freight) change hires and occupancy there, and the plan's re-proofs, the level-change
  permits and the queues' own checks see the live city. The actions slot's execution moves from
  slot 48 to 56: spawns, level changes and clearances are queued and stamped 8 slots later
  (construction starts still from minute 4, level changes and removals still from slot 64/65),
  and the admission credit starts at 56 (the first admission at slot 57). Saves: a save in a span
  keeps the setup (`demand_planning_setup`, save format 106; building candidates by dense index,
  which saves keep); the census-restore rule follows `PlanningSlot::Actions` and now covers slots
  0–55. Which slots: the setups at 0, 16, 32, 48 (each minute's first slot, where the planning
  slots were) and the plans at 8, 24, 40, 56, all queue-free (eleven queue-free slots an hour, no
  two adjacent, each one's next slot in its minute: compile-time checks). Evidence, against the
  ninth commit: the envelope passes with no failures and the same 8 notes; net income per
  resident within 0.50%, employment within 0.33%, households, jobs and buildings within 0.24%,
  deliveries within 0.32% except the 10k fiscal-stress city (8.75 → 8.44, already a note),
  bankruptcies small counts (about 4 → 2, 87 → 70 inside its band, 86 → 80); bursts pass (worst
  minute at most 4.3% of an hour's departures, settlement hour 7.4%) and determinism shows no
  differences. At 100k the halves hold 1.2–2.0 and 1.6–3.0 ms (diagnostic).
- **`SIM-02-K` part 5 (tenth commit) — the census book's pass rows in a lending of their own at
  slot 936.** The preparation's first half (slot 944, 3.5 ms at 100k) built the census book's
  planning pass rows in full (about 0.85 ms), because the previous lending was the live book's
  and the kept rows follow the book they were built for. Options: (a) build them in the second
  half (953, already 3.0 ms); (b) a third, earlier lending that opens and closes a pass only, at
  slot 936, after which the supplier half keeps the rows (taken: exact, the rows are derived and
  keyed by the book's generation, price phase and policies; 936 is not queue-free, 0.85 ms plus
  its queues; a census not done by 936 is prepared at 944 as before); (c) keep the rows of two
  books (more state for one slot).
- **`SIM-02-K` part 5 (tenth commit) — the removals stay at slot 64.** The removal window's first
  slot stays eight slots after the actions plan: moving it would only relocate its 3.4 ms to a
  slot that is not queue-free. Its zone-index rebuild and site publication are the next commit's
  options (see the findings entry below).
- **`SIM-02-K` part 5 (tenth commit) — the census input copy test picks buildings no shopping
  household uses.** The new schedule's busy town had a household whose shopper was out when the
  test removed its home: the household's search cursor advances and its store is cleared in
  place, which the copy, a snapshot, keeps as copied, so the test's tag check failed. Options:
  (a) mirror in-place household changes into the copy (rejected: the copy is a snapshot by
  design); (b) remove only buildings no shopping household uses (taken: the test's subject is
  swap-remove mirroring).
- **`SIM-02-K` part 5 (tenth commit) — the 100k 1× target not reached (findings).** After this
  commit no planning half and no first lending after a load is among the 100k 1× frames over
  8 ms. Left (diagnostic, attribution in the results section): (1) the removal window's first
  slot, 3.3–4.7 ms at 100k: about 0.7 ms a removed building (evacuation, eviction, the agents'
  and households' remaps), the zone-index rebuild 1.5 ms (0.37–0.44 ms of it the admission
  order) and the site publication 0.3–0.5 ms, frames 8.0–9.6 ms on a 3.5–4.4 ms agent tick;
  options: (a) keep the zone index across a swap-remove (the removed building's chunk lists and
  the moved building's index, O(chunk) instead of O(B/p); exact), (b) publish the site changes
  at slot 65 (it already serves the cell-lot regions they dirty), (c) fewer removals a slot (each
  slot with removals still rebuilds the index; rejected); (2) census slices of 4.1–9.6 ms on the
  cities this build prepared (above), where the previous build's cities have none over 2 ms, so
  the city triggers them and the census code is unchanged; (3) the census book's preparation
  halves, 2.5 ms (supplier rows' factory and layout) and 3.0 ms (buyer rows 1.5–2.0, finance
  masks 0.8), a frame or two a day. None needs a change to the user's spec decisions; (1a) and
  the census step are the next commit's work.
- **`SIM-02-K` part 5 (eleventh commit) — business claims counted rather than reweighted.** Every
  census slice over 2 ms of the 100k day on the tenth commit's cities was the business claims
  (4.0–10.6 ms, seven slices in the censuses publishing at 05:00 and 00:00), whose tail sellers'
  scans cost 0.4–1.4 µs a seller against a row weight of 20; no seller took over 0.25 ms, and
  almost no route searches ran. Options: (a) raise the claims' row weight (rejected: a weight
  covering the heavy hours would slice the ordinary ones 20–70 times finer and still miss a city
  with heavier tails); (b) count the claims' scan events and carrier quotes as work units and stop
  the slice at the seller whose work reaches the budget, as the household claims and staffing do
  (taken: deterministic counts, the same sellers in the same order, only slice boundaries move);
  (c) parallelise the claims (rejected: sellers claim from shared buyer accounts in building
  order, a dependent sweep, so it cannot be parallelised); (d) slice inside one seller's scan (not
  needed: no seller took over 0.25 ms). Weights come from a least-squares fit of the slices' time
  at 100k (14 ns a row, 285 ns a scan event; carrier quotes were too few to fit and keep 150).
  Evidence: the census restart test lands a restart inside the sweep; the envelope is
  bit-identical to the tenth commit's; no census slot is over 2.24 ms (diagnostic).
- **`SIM-02-K` part 5 (eleventh commit) — the zone index kept across the removal queue's
  swap-removes.** The removal slot's rebuild (1.3–1.5 ms at 100k) derived every building's entries
  again although a clearance changes two buildings'; the rebuild is also the `ALLOC-03` reset of
  the vacancy lists to building order, which admission's ties by vacancy position read. The
  earlier entry that rejected an incremental zone index (seventh commit, "the removal slot's scans
  and zone-index rebuild on the pool") rejected one that resets the order differently; this one
  preserves the reset. Options: (a) keep the lists across each swap-remove and, at the queue's
  repair, return the vacancy lists to building order and recompute the admission order (taken:
  exact, the reset is kept); (b) a parallel rebuild (already done: the derivation is on the pool,
  the fill is the serial rest); (c) defer the rebuild to the next slot (rejected: moves the cost
  into slot 65, which holds 2.55 ms). Scope and safety: only the removal queue's clearance repair
  takes the kept path; any other stale mark drops it, an append after a kept removal waits for the
  rebuild, and any broken invariant falls back to a full rebuild; test builds compare every kept
  repair with a rebuild under `city_checks()`. Found after measuring: the support and lot-radius
  maxima keep counts of the buildings at each maximum (as the site radius already did), because a
  removed building nearly always shared a maximum, so a flag refolded both over every building at
  every repair (0.79 ms, serial). The vacancy lists return to building order by tagging each
  listed building and reading the tags in building order (O(B + V), 0.03 ms). Evidence: kept
  repair 0.59 ms (admission order 0.55), slot 64 3.46 to 2.65 ms mean (diagnostic); the envelope
  is bit-identical.
- **`SIM-02-K` part 5 (eleventh commit) — the removal slot's site publication stays in slot 64.**
  Moving it to slot 65 was considered: its cost is the terrain boxes (0.72 ms mean, up to 1.72),
  but the production-site remap it also runs must stay with the swap-remove (the agriculture and
  extraction rows follow building indices), and slot 65 already holds 2.55 ms, so moving the boxes
  would make slot 65 the heavier frame. Not taken; publishing the boxes in a lighter slot is
  listed as next work.
- **`SIM-02-K` part 5 (eleventh commit) — slot 944's drift treated as variation (unconfirmed).**
  Slot 944 read 2.61 to 2.94 ms across the session's diagnostic days (supplier factory 0.68 to
  0.82 ms, layout 1.29 to 1.40 ms) in code this commit does not touch, with the census book's
  results identical; builds that differ only by the claims' weights and the kept index's maxima
  counts, neither of which slot 944 reads, read 2.61, 2.69, 2.67 and 2.94 ms. It is taken as
  run-to-run variation of a memory-bound phase, not confirmed.
- **`SIM-02-K` part 5 (twelfth commit) — a removal's agents and households in one scan each.**
  Each removal scanned every agent three times (evacuation, eviction, the remap's eight column
  passes) and every household twice, about 0.5 of its 0.69 ms at 100k, to find the few naming
  the removed building or the last building, which its swap-remove moves. Options: (a) batch the
  slot's remaps into one pass after its removals (rejected: each removal's evacuation and
  eviction read agent and household references in the live index space, and the evacuation
  plans trips home from building indices, so a deferred remap would need every reference
  translated between index spaces mid-slot); (b) serve fewer removals in slot 64 (a scheduling
  change, and every removal slot still pays its zone-index repair; rejected: prefer exact); (c)
  one scan per store for both buildings, the visits restricted to it (taken: exact, since only
  the visited agents and households change before the remap, so none outside comes to name the
  moved building; a carrier removal, which renumbers agents, falls back to the full remap; test
  builds assert the claim after every remap). Evidence: per removal 0.69 → 0.37 ms (diagnostic);
  a build without slot 936's queue-free entry has an envelope bit-identical to the eleventh
  commit's; the new unit test equals the full scans.
- **`SIM-02-K` part 5 (twelfth commit) — the removal slot's terrain boxes in later slots.** The
  terrain boxes cost 0.25–0.55 ms each, nearly all the road grading margins of the patches they
  touch, which a building removal does not change. This follows the eleventh commit's entry "the
  removal slot's site publication stays in slot 64", which left the boxes for a lighter slot.
  Options: (a) keep the road margins per patch and reuse them for site-only refreshes (rejected
  for now: the margins follow the road surface and the heightmap, which keep no single revision
  covering every input, and a stale margin is a visual fault that no gate detects); (b) publish
  them at slot 65 (rejected: it holds 2.4–3.2 ms of trip events, the cell-lot region the
  removals dirtied and two queue shares); (c) queue them and refresh one box a slot in the slots
  that serve a single queue share (taken: the ownership is render state no simulation system
  reads, each refresh recomputes its patches from the live state, and a load recomputes all of
  it, so the simulation is exact and the terrain under a cleared building updates 0.1–0.25 s
  later at 1×; the refresh waits while the zone index or site clients are stale, since repairing
  them would reset the vacancy order admission reads). Evidence: slot 64's publication 0.69 /
  1.42 → 0.003 ms, the boxes 0.44 / 0.86 ms in slots 66–67 (diagnostic).
- **`SIM-02-K` part 5 (twelfth commit) — the census book's preparation in four parts.** Slots
  944 (2.65 / 3.04 ms) and 953 (3.13 / 3.80) each held two parts of about 1 ms plus a lending of
  0.6–0.7 ms. Options: (a) a new heavy slot for the supplier layout (948) and the masks at 957
  (rejected: one more lending, 0.7 ms more work an hour, and 948 and 953 could share a 32×
  frame); (b) the supplier rows' factory in slot 936's lending, which already opens and closes
  a pass, the layout alone at 944, the rest at 953 and the masks at 957 (taken: no extra
  lending; 936 becomes queue-free at 2.0 / 2.6 ms; the halves at 944 and 953 still never share a
  32× frame); (c) only a faster layout (partly taken as well: the supplier index builds its
  resources and sorts large bucket lists on the pool, exact). The masks only prune unclaimable
  buyers, so where they are refreshed changes no result; a second half later than 953 refreshes
  them itself. Evidence: 944 → 1.69 / 1.89, 953 → 2.33 / 2.80, 957 → 1.24 / 1.83 ms
  (diagnostic). The parts are exact (the build without 936 queue-free is bit-identical); 936's
  queue-free entry is a scheduling choice with a gameplay side, of the ninth commit's kind (queue
  shares, trips and construction starts due at 936 move to 937, in the same minute), made under
  the run's rule (keep the envelope closest, prefer the simpler option). Envelope evidence
  against the eleventh commit: the gate passes with no failures and the same eight notes; net
  income per resident within 0.74%, employment within 0.13%, households, residents, jobs and
  buildings within 0.31%, deliveries within 0.24% (the fiscal-stress startup freight +2.6% and
  +2.8%, already notes), out-of-stock rates within 0.40%, the 30k baseline's bankruptcies 0.246
  → 0.281 per 1,000 buildings a day (about 70 → 80 events); bursts and determinism pass. The
  alternative, 936 serving queues at 2.0–2.6 ms plus 0.4–0.9 ms of queue shares, would make it
  the next frame over 8 ms.
- **`SIM-02-K` part 5 (twelfth commit) — the expansion sweep's priced-route weight.** The
  expansion slices were up to 2.4 times their estimate (at most 2.2 ms against about 0.9 ms
  budgets) when every row was an operation with thousands of priced routes; a least-squares fit
  over 1,197 slices gives about 230 ns a priced route in the expansion sweep, 25–30 in the
  current-staff sweep. Options: (a) raise the shared candidate and route weights (rejected: the
  current-staff slices fit their weights, so they would be cut finer for nothing); (b) a higher
  operation weight for the expansion (rejected: it overprices the operations with few routes,
  about 5× in total); (c) an expansion-only extra of 200 a priced route (taken: the heavy
  slices' actual over estimated time is then at most 1.0, the expansion's total estimate 2.0×
  its time, which raises the census's slot budget by about 4.5%). Exact: only slice boundaries
  move. Evidence: slices at most 2.20 → 1.69 ms, p99 1.78 → 1.35 (diagnostic).
- **`SIM-02-K` part 5 (twelfth commit) — admission's border sweeps at load.** The first actions
  plan after loading spent 1.15 ms sweeping the border arrival costs, which later hours keep per
  routing revision. Loading now sweeps them (`prepare_loaded_pathing`); exact, the tables are a
  function of the routing hierarchy and the graph's node roles. Evidence: the 00:03 admission
  1.15 → 0.03 ms, the actions plan 2.98 ms, equal to later hours.
- **`SIM-02-K` part 5 (twelfth commit) — the 100k 1× target not reached (findings).** After
  this commit no removal slot, census-book preparation, census slice or first-lending frame is
  among the 100k 1× work frames over 8 ms: the D2 diagnostic day has one (the industrial
  planning setup, 2.9 ms, 22:02, 8.0 ms) and the `PERF-07` day one (the first actions plan after
  loading, 3.4 ms, 00:03, 8.6 ms), against ten and five before. Left (diagnostic): (1) the
  planning setups (the workforce count 0.57–0.71 ms in every setup; scans industrial 1.28 /
  1.52 ms, residential 0.99, commercial 0.72, actions 0.50); options: the scans on the pool or
  sliced over the setup's half minute (exact), the workforce counted once a minute for both
  halves (a gameplay change of half a minute); (2) the actions plan (2.9–3.0 ms; plan 1.65 and
  the queue's re-proof 1.27 ms); (3) the first residential plan after loading (3.7 ms against
  1.8: its pass rows, 0.77 ms, because loading lends the census books after the live one, and
  the queue's first re-proof, 1.7 against 0.5 ms). None needs a change to the user's spec
  decisions.

- **`SIM-02-K` part 5 (thirteenth commit) — the setup's scans beside its workforce count.** The
  setups held 1.2-2.2 ms (industrial 2.9 ms at most), the count and the scans one after the
  other. Options: (a) slice the scans over the setup's half minute (rejected: the queues change
  the city between those slots, so slices would scan a city no single scan saw, and the plan
  decides on the setup's scan; not exact); (b) count the workforce once a minute for both halves
  (rejected: the plan would decide on a workforce half a minute old, a gameplay change); (c) run
  the scans beside the count on a snapshot composed with an empty workforce (taken: the scans
  read only the snapshot's output absorption and unmet commercial demand, which `compose` and
  the queue reservation derive without the workforce, so the scans are the ones the serial setup
  ran; test builds assert the priorities bit for bit), with the lot and building scans and the
  count's passes side by side as well. Evidence: industrial setup 2.16 / 2.90 → 1.49 / 1.95 ms,
  residential 1.71 → 1.47, commercial 1.56 → 1.28, actions 1.27 → 1.17 ms mean (diagnostic);
  envelope bit-identical.
- **`SIM-02-K` part 5 (thirteenth commit) — one pass for a workforce reset.** Each reset (five a
  game hour) passed over the buildings twice and added free places to shared atomic bins.
  Options: (a) keep the reset's results across the actions slot's two resets (rejected: the
  epoch's project claims and queue differ from the plan's, and a cache would need revision keys
  for every building field the reset reads); (b) one fold with thread-local sums (taken: the sums
  wrap as the atomic adds did, so any order gives the same bins, and `reduce` merges adjacent
  building ranges in order, so the displaced households' homes keep building order). Evidence:
  the epoch's reset 0.47 → 0.40 ms (diagnostic); envelope bit-identical, determinism with 1 and 3
  workers equal.
- **`SIM-02-K` part 5 (thirteenth commit) — the site feasibility cache in shards.** The lot scan
  looks up every candidate pose from all pool workers under one mutex. Options: a concurrent map
  (rejected: a new dependency); 32 mutex shards by parcel (taken: each lot is resolved by one
  worker, so lookups rarely meet; the cache stays derived and validated before reuse, so
  verdicts are unchanged).
- **`SIM-02-K` part 5 (thirteenth commit) — the free-lot storage estimate kept per parcel
  revision.** Every lending's site preparation counted the free lots (0.14 ms) to size storage.
  The count is kept while the parcels' layout and occupancy revisions and count hold; a stale
  count could only size storage differently, which never changes a result (the storage grows
  through the exact preflight). Evidence: a lending's site preparation 0.50 → 0.35 ms median.
- **`SIM-02-K` part 5 (thirteenth commit) — the live books lent last at load.** The first
  residential plan after loading rebuilt its pass rows (0.8 ms) because loading lent the census
  storage's warm copy after the live books. Options: (a) build the copy without lending it
  (rejected: the copy's storage then lacks the spare rows its first census needs); (b) lend the
  live books once more, last, when no finished census is held (taken: a lending restores the
  books exactly and its rows are derived, so the second preparation changes nothing but which
  book's rows are kept; with a finished census, which is published and lent next, the order is
  unchanged). Evidence: the first plan's pass rows 0.8 → 0.09 ms in a short instrumented run;
  envelope bit-identical (the gate's save/reload continuation included).
- **`SIM-02-K` part 5 (thirteenth commit) — the actions slot not split (findings).** After these
  changes the actions slot is the largest steady slot (2.8 / 3.4 ms). Splitting its execution
  into a later queue-free slot of minute 3 would take about 1.2 ms off it, but the state the
  plan hands the execution (the workforce view, the admission budget, the slot's validity) is
  not saved, so it is a save format change besides a scheduling change; the exact alternative,
  one lending for the plan and the epoch, needs the pass journal to rewind without truncating
  the owner rows. Neither fitted this session; neither needs a change to the user's spec
  decisions.
- **`SIM-02-K` part 5 (fourteenth commit) — one lending for plan and epoch: not done.** The
  actions slot (2.78 / 3.38 ms) lends the books twice, for its plan and for the construction
  epoch. One lending for both would be exact, but the admission decision, the queue's refresh
  and a stale entrance cache's rebuild run between the plan and the epoch on the live city, and
  the three books would need a pass journal that rewinds deeply to the prepared state, keeping
  the projected source slots, the temporary owners' rows and the site rows while restoring what
  the plan claimed on them. It saves only the epoch's pass start and site preparation, about
  0.4 ms, so it was not worth that rewrite.
- **`SIM-02-K` part 5 (fourteenth commit) — the execution split to slot 60: deferred.** The
  actions slot's execution (1.2–1.3 ms) in a queue-free slot of its own in minute 3 would take
  about 1.2 ms off slot 56 (expected slot 56 about 1.4 ms, slot 60 about 1.3 ms). It is a
  scheduling change and a save format change: the workforce view, the admission budget and the
  plan's validity would be saved, and the tests that expect the execution at 56 change. It did
  not fit this session with its gate, `./run.sh --test` and fresh `PERF-07` cities.
- **`SIM-02-K` part 5 (fourteenth commit) — the free-place scan: overlapped for the plans,
  dropped for the epoch.** The workforce reset's free-place scan reads neither the books nor the
  queue, so each plan runs it beside the site preparation and queue replay (taken: exact;
  0.15–0.25 ms off each plan). The epoch's overlap measured 1.19 → 1.18 ms, no gain, since its
  parts are each well parallel, so the epoch keeps its order. The constructing workplaces' jobs
  come from the books' constructing rows; test builds assert that the rows are complete.
  Evidence: 6 game hours with the `economy_phases` channel and the schedstat days (diagnostic);
  envelope bit-identical.
- **`SIM-02-K` part 5 (fourteenth commit) — no spreading within minute 3 (findings).** A
  per-frame dump of a 100k 1× day found no departures, admissions or arrivals clustered in
  minutes 3 and 4 (path searches 14–16 a frame in every minute, the tick's mean flat by minute,
  2.98–3.21 ms). The higher ticks in the worst frames were the frontage update in every
  minute-first frame, the movement pass after a slot frame (about 0.3 ms slower, cache
  eviction), the commute hours (0.4–0.6 ms) and preemption. Spreading work within a minute
  would be a gameplay change with nothing to spread, so none was made; the frontage update was
  made cheaper instead.
- **`SIM-02-K` part 5 (fourteenth commit) — the frontage sums by lane range.** The update sums
  each lane's car speeds in f32 in agent order, serially, so delays never depend on the worker
  count. Options: (a) per-chunk partial sums merged (rejected: f32 addition is not associative,
  so the delays would change with the split); (b) a phase shift of the update off the minute
  boundary (rejected: a gameplay change, and the frame it moves to may hold a planning slot);
  (c) one task per lane range scanning the agent columns (tried: 0.82 ms, slower than the serial
  0.71, each task reading the agents' mode and lane columns); (d) compact lane notes, 4 bytes an
  agent filled in parallel, then one task per lane range reading the notes in agent order and
  updating its own lanes' delays (taken: each lane's cars still add in agent order, so the
  delays equal the serial pass's bit for bit; a unit test compares 1, 3 and 8 workers).
  Evidence: 0.68 → 0.45 ms an update at 1×, 0.72 → 0.47 at 32× (diagnostic); envelope
  bit-identical. The lane pass itself (0.34 ms) is left.
- **`SIM-02-K` part 5 (fourteenth commit) — rising maxima and over-8 counts treated as noise.**
  The 10k and 30k maxima and the 100k 1× count of frames over 8 ms rose against the control in
  the first run. After the rechecks (the count between 12 and 36 for either build; the raised
  maxima single frames with an agent tick of 5.1–11.9 ms and under 2 ms of slot work) they were
  taken as preemption noise, not as a cost of this commit.
- **`SIM-02-K` part 7 — `PERF-07` reports the frames other tasks preempted.** The eight pinned
  CPUs are shared with the rest of the machine, and rechecks of one binary gave 30k 1× maxima
  from 4 to 15 ms (see "rising maxima and over-8
  counts treated as noise"): a single raw maximum is not reproducible.
  Options: (a) keep the raw maximum only (rejected: it measures the machine as much as the
  simulation); (b) per-frame scheduler statistics of the threads that do the frame's work, read
  outside the timed span (taken: test builds only, Linux only, no dependency, no `unsafe`,
  O(threads) reads a frame and no allocation; the raw columns and the gate's limits stay as
  they were); (c) the part 5 diagnostic, which read every task of the process with its name
  (rejected for the benchmark: it opens and allocates per task and frame, and only the pool's
  workers and the frame's own thread hold the frame's work); (d) isolated CPUs (machine
  configuration, not a code change; left to the user, open question 7). The workers' thread
  ids come from a `rayon::broadcast` on the shared pool when the run starts, each worker reading
  its own `/proc/thread-self` link, rather than from a start handler in `thread_pools.rs`
  (rejected: it would change the game's pool code for a test need; the shared pool keeps its
  workers until it stops, so both give the same set). Each thread's `schedstat` stays open and
  is read with one positioned read at offset 0, which procfs renders afresh.
- **`SIM-02-K` part 7 — the preemption threshold is 0.5 ms of run-queue wait.** Options: 0.05,
  0.1, 0.25, 0.5, 1, 2 or 5 ms (the report's wait classes). Evidence, from four full days of
  every tier and speed and four more of 30k (`wait_classes`, `tools/sim02_gate.py perf07`
  reports): frames whose largest wait is under 0.5 ms are on average 0.0–0.3 ms slower than
  frames with no wait (100k 1×: 0.06–0.34 ms on a 4.0 ms mean), frames with 0.5–1 ms of wait
  0.3–0.6 ms slower, 1–2 ms 1.0–1.6 ms and 2–5 ms 1.5–5.2 ms. `max_unpreempted_ms` is the same at
  every threshold from 0.1 to 0.5 ms in every run. 1 ms raises the 10k or 30k 1× value in seven of
  the eight full 1× days at those tiers (by 0.03–0.7 ms), and 2 ms puts a 100k 1× frame over 8 ms
  (8.00 and 8.12 ms) in two of the four 100k 1× days. 0.05 ms drops three frames with 0.05–0.08 ms
  of wait, among them a 100k 32× day's worst frame (24.3 ms, 16.8 ms of it slot work), which is
  work, not preemption. So 0.5 ms is the largest threshold whose maxima equal those of every
  threshold down to 0.1 ms.
- **`SIM-02-K` part 7 — an off switch for the reads' overhead.** The first A/B ran the new build
  against the fourteenth commit's binary and found the new build's means 4–5% lower at 30k 1×
  (the snapshot mean 13% lower), a layout difference between two builds, not the reads (the
  harness change touches no simulation code). Options: accept that A/B (rejected: it cannot
  separate the reads from the build); a same-binary A/B (taken): `METRUM_PERF07_SCHEDSTAT=0`
  watches no thread. Same build, 30k, the reads off and on alternating, two rounds each, one
  session: means 0.877 → 0.873 ms at 1× (−0.4%) and 2.799 → 2.771 ms at 32× (−1.0%), against a
  spread of 2.4% and 1.2% between the two rounds with the reads off; agent tick, slot work,
  snapshot and p99 move by under 2% the same way. The reads add about 4 µs a frame to the run's
  wall time at 1×, outside the timed span. The two builds' layouts differ by about 4% in the
  30k 1× mean (0.835–0.851 against 0.873–0.888 ms with the reads on in both), so a timing
  comparison across builds keeps needing a same-session control.
- **`SIM-02-K` part 7 — stopped under the rule.** 100k at 32× cannot reach 8 ms without changing
  a decision of this document (open question 6), and whether the acceptance reads the raw
  maximum or the unpreempted one is the user's decision (open question 7). Per the rule, part 7
  changes no simulation code, writes the findings and options here, marks `SIM-02` and
  `PERF-07` `needs_revalidation` and stops for review. The only code change is the measurement
  (test builds), which leaves every simulation result bit-identical. The user's review on
  2026-10-22 decided both questions ([Decisions (2026-10-22)](#decisions-2026-10-22)), and
  `SIM-02` and `PERF-07` are done.
- **`SIM-02-K` part 7 — the day report carries its components' means.** Open question 6 needs
  the 100k 32× frame's budget by part; `PERF07_DAY` reported only the components' maxima and the
  snapshot's mean. It now also reports the mean agent tick, slot work and pathing per frame
  (sums over the samples it already keeps, no per-frame cost). A 100k speed sweep (2×, 4×, 8×,
  16×, one diagnostic day each) measured how the frame grows between 1× and 32×, for the cap
  option of open question 6.

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
