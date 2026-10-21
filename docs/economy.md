# Economy

This is the current specification of household and business finances, production, services,
trade, labor and freight. It describes the runtime as it is. Dated diagnoses, measurements and
superseded designs up to 2026-10-10 are preserved in
[`archive/economy_history_2026-10-10.md`](archive/economy_history_2026-10-10.md).

## Purpose and ownership

Buildings produce and employ; households earn, spend and consume; the city collects revenue and
pays for infrastructure, services and transfers. Growth must be supported by those recurring
flows after starter savings and owner startup capital are spent.

| Concern | Owner |
| --- | --- |
| Household, business and city money; production; services; utilities; freight; labor; housing affordability; level-change viability | This document |
| Whether households arrive or leave, which private projects start, level changes and failed-site clearance | [`demand.md`](demand.md) |
| Legal zones, densities and level families | [`zoning.md`](zoning.md) |
| Build-site discovery, placement and removal execution | [`building_allocator.md`](building_allocator.md) |
| Door-to-door trip semantics | [`entrance_and_exit.md`](entrance_and_exit.md) |
| Asset metadata and the profile reference stored on assets | [`asset_editor.md`](asset_editor.md) |

Economy data is authored in `economy/profiles.toml` and checked with Rust balance tests and
headless scenarios. There is no in-game or developer economy editor.

## Growth redesign (`ECON-11`–`ECON-12`, `DEM-02`)

Status, 2026-10-10. The healthy-growth redesign agreed on 2026-10-05 is largely implemented:

- businesses enter only when recurring sales cover their operating costs and assets cover their
  working capital; owner equity is capital, never sales;
- one funding census ([below](#funding-census-econ-17)) drives hiring, payroll, store output,
  construction viability and household admission;
- each new project reserves its workers, ready homes, customers and inputs against every other
  project in the same pass;
- imports are 1.25× and export bids 1.0× catalog price, freight is priced per trip and outside
  trade has no quotas;
- failed businesses are cleared and replaced; households leave through one individual exit
  policy.

`DEM-02` closed on 2026-10-11 ([`demand.md`](demand.md#exit-criteria)). What remains is tracked by
runnable checks: runtime performance under the [performance contract](#performance-contract-perf-01perf-05).
Healthy-growth acceptance is defined [below](#healthy-growth-acceptance).

## Core principles

1. **Buildings are the economic actors.** Agents supply labor, carry household shopping and make
   visible trips; production and trade happen between buildings.
2. **Households replenish periodically.** Each household holds one shared supply reserve. One
   member at a time makes an ordinary `Home → Store → Home` trip to refill it. Residents never
   buy food individually every day.
3. **Goods move physically.** Local and outside freight travels as lane-bound trucks. Networked
   utilities (power, water, sewage) use a separate service layer, and emergency liquidation is
   an explicit cash-settlement exception.
4. **Data is hand-authored text.** Profiles, recipes, prices, wages and tuning live in TOML and
   are validated at startup; invalid data fails loudly instead of falling back to defaults.
5. **Cost scales with buildings, households and shipments.** No per-tick per-agent inventory
   search, market scan or per-resident shopping trip. Every hot path has a stated bound and uses
   the existing spatial indices.
6. **Determinism.** Same save, inputs and tick sequence give the same result. Parallel folds use
   fixed chunks and ordered merges; ties break by stable IDs.

## Time and cadence

This section describes the current cadence. Its replacement is planned in
[`simulation.md`](simulation.md#sim-02-scheduler-refactor) (`SIM-02`).

### Clocks and day length

- **Operational clock:** travel, work, deliveries, consumption, wages and production. At 1×
  speed one game day is 24 real minutes (`seconds_per_day = 1440`), so one game hour is 60 real
  seconds and one game minute is one real second. Supported speeds are 0–32×.
- **Demographic clock:** aging and schooling, deliberately separate. There is no aging yet.
- The saved day duration is authoritative for an existing world; configuration supplies the
  default for new worlds. Day durations must be finite and at least 60 seconds.
- Trip estimators return physical seconds. Schedules convert them to operational minutes with the
  live day duration.

### Order of operations

`SimCore::step_frame` advances agents by one frame, then runs every authored minute the clock
crossed:

1. Each minute: the next queued private construction start, at most one per minute, and any
   construction site whose deadline falls in that slot completes
   ([`simulation.md`](simulation.md#deadlines-and-timed-events)).
2. Each whole hour: the **operational hour**: the pooled household hour (below), then the **hourly
   demand pass**: project planning, the household admission decision and the building actions'
   proofs ([`demand.md`](demand.md#cadence-and-handoff)). The households whose exit was claimed at
   their daily phase leave through the `departures` queue in slots 65–479, the admitted households
   are created over the hour by the `admissions` queue, and level changes and clearances are applied
   by their queues from slot 64 for clearances and slot 65 for level changes (`SIM-02-J`). 00:00 is an ordinary hour (`SIM-02-G`).
3. At 00:00, after the hour, the **calendar close**: the day's running totals become the
   budget report, the tax window rolls, road upkeep is charged from the rolling road length and
   the day's exit budget resets. It costs O(zones); the 00:00 hour still belongs to the closing
   day.
4. Each slot, after everything above: the **hourly phases** of the buildings, households and
   agents whose phase falls in it (`SIM-02-F`), then their **daily phases** (`SIM-02-G`), then
   at most one dirty cell-lot region (`SIM-02-H`) and the freight, housing, restock, visit and
   job request queues (`SIM-02-J`), then one chunk of the rolling road-length
   sum and one of the day's pollution, noise and desirability sweep
   ([`simulation.md`](simulation.md#snapshots)). Each entity has one hourly phase per hour,
   about 1/960 of the city per slot, and one daily phase per day, about 1/23,040. Readers see
   the environment grids the previous day's sweep finished at its last slot.

Household hour (`households/tick.rs`): count the households still arriving; logistics clock;
the commute-quote expiry. Freight, housing, restock, visits and job search are request queues
served each slot, except in the hour's seven queue-free slots, whose share the next slot serves
(`SIM-02-J`, `SIM-02-K`, [`simulation.md`](simulation.md#request-queues)).
The funding census is no longer part of the hour (`SIM-02-I`,
[`simulation.md`](simulation.md#pure-jobs)): its inputs are copied over each hour's first half,
it runs in steps over the second half, and it publishes at the next hour's first slot, just
before this pass, copying funded jobs and activity onto the buildings. Every reader that hour
sees that census; its copy is 31–60 minutes old when it publishes. Membership is a maintained
list of each household's members (`SIM-02-I`): each household recounts its members, car access, stock days, employed adults and expected wages at its own
hourly phase, a job change also updates the household's employed adults at once, and
buildings' worker counts follow every job change.

Hourly phases (`nodes/sim/core/entity_phases.rs`): a building sets its activity floor from the
hour's census, produces and consumes its recipe inputs, and a farm or mine adds its output; a
service store then draws its sales and the building meters its electricity, a power plant
generating at the last hour's dispatch. A household recounts its members, buys services, meters
its electricity, consumes supplies, pays utilities and times out a stalled shopping leg; an
agent whose household pantry is empty loses happiness. Phases read the city totals of the last
finished hour: power dispatch and coverage, and the residents, store capacity and prices of
service sales. The hour's last slot folds the hour's own totals for the next hour (`city
hour`, `SIM-02-I`).

Trips do not leave when they are decided (`SIM-02-E`,
[`simulation.md`](simulation.md#deadlines-and-timed-events)). Freight carriers and shoppers leave
near a target slot in the hour ahead, in the least busy minute within 15 minutes of it, and a
visible service visitor leaves at the next slot (`SIM-02-J`). Household arrival carriers leave paced
from their household's creation: the actions slot (minute 3, slot 56) decides the admissions and
the `admissions` queue creates the households evenly over the hour that follows (`SIM-02-I` part 4,
`SIM-02-J` part 3). Each trip is then checked as it travels (every one to eight minutes, growing
with its time on the road) and settles when it arrives: a delivery, a store pickup, a shopper's
return or a household moving in no longer waits for the next hour. Leg timeouts still count
operational hours.

Daily phases (`nodes/sim/core/entity_days.rs`), each over the entity's own 24-hour window:

1. **Building:** the zoning compatibility check and its rezoning grace; then the bankruptcy check
   (a building that ended its last window in distress and is still negative), property tax,
   water, sewage and electricity bills, distress liquidation of a negative private budget, and
   business profit tax. Its daily accumulators then roll into the `recent_*` values.
2. **Household:** transfers, residential property tax, water, sewage and electricity bills; the
   ledger window closes and the next opens. The household then queues a housing review
   (affordability, relocation, eviction) and, if it may leave, claims its share of the day's
   exit budget.
3. **Agent:** the wage, paid while the workplace has paid fewer than its funded jobs in its own
   window, and the daily happiness update.

The `housing requests` queue serves the housing reviews a few per slot, and the `departures` queue
removes the claimed exits, one a slot in slots 65–479, each walking the household's member list
(`SIM-02-J`). Wages read
the funded jobs the last hourly census copied onto each building; water and sewage coverage is
whether a provider settled operational in the last closed day. Complete commute quotes expire 24 workplace passes after they were added; bounded
network-only legs remain valid until their CCH or lane revision changes. Every payment also
posts into the calendar day's totals, which the close reports in O(1).

Demand never reads half-updated economy state. Buildings or households changed by a demand pass
take effect in the next operational hour.

### Schedules and departures

Work and freight timing are authored as minute windows in
`runtime_tuning.operational_clock`:

- `daytime_work` (commercial): arrive 07:00–09:00, leave 16:00–18:00, 15-minute reliability
  buffer.
- `three_shift_work` (industry, farms, mines, utilities): three arrival and departure windows,
  10-minute buffer.
- Freight profiles `always_open`, `daytime_receive` and `early_morning_preferred` are soft
  preferences: deliveries outside the window pay a cost multiplier (1.10 and 1.05) and an ETA
  penalty.

Workers hold stable offsets inside their windows. Departure is
`target_arrival − estimated_travel − reliability_buffer`; the estimate is cached and refreshed
every `travel_estimate_refresh_minutes` (360), never re-queried per frame. A cached departure leaves
at its seed's slot of its minute, `floor(frac(seed/φ) × 16)` of the minute's 16 slots, so the
agents due in one minute plan over the minute instead of all in its first frame; the day's last
minute keeps slot 0. Rush hour emerges from overlapping windows.

## Authored data

### Economy profiles file

`economy/profiles.toml` holds resources, economy profiles and `runtime_tuning`. It is loaded once
by `load_runtime_economy_catalog`, which validates and compiles it into the runtime catalog
(`definitions.rs`). Demand pacing lives separately in `demand/growth_profiles.toml`.

Validation rejects non-finite or negative rates, wages, prices and stock settings, duplicate IDs,
invalid quantity units, freight batches on non-physical resources, oversized batches, and
processing or retail recipes that create net material mass. Each resource appears at most once
per port direction. A resource may appear on both sides for self-upkeep.

Resource `runtime_id`s are explicit, contiguous from 1 and persisted in saves: append new IDs and
never renumber. Profile runtime IDs are append-only by authored order. Import-only resources may
declare `import_unit_price_currency` until a producing profile owns their price.

### Resource units and freight payload

Quantities are canonical per resource and compiled once. Production rates are per operational
day.

| Resource | Unit | Min. freight batch | Notes |
| --- | --- | ---: | --- |
| `household_supplies` (1) | kg | 40 | Retail basket; 1 kg per resident/day |
| `packaged_food` (2) | kg | 50 | |
| `grain` (3) | kg | 50 | |
| `coal` (4) | kg | 40 | Extracted from finite deposits |
| `health_essentials` (5) | service | — | Aggregate service capacity, never freight |
| `personal_services` (6) | service | — | Aggregate service capacity, never freight |
| `machinery` (7) | kg | 40 | Equipment and spare parts |
| `steel` (8) | kg | 100 | Import-only, 3.2/kg reference price |
| `metals` (9) | kg | 100 | Import-only, 6.4/kg reference price |
| Electricity | kWh | — | Utility layer; MWh in large displays |
| Water / sewage | m³ | — | Utility layer |

`runtime_tuning.logistics.truck_payload_kg = 20000` caps every shipment. `shipment_batch_kg` is
the per-resource minimum and rounding quantum: a 400 kg order travels in one truck without waiting
for a full load. Quantization rounds down the smallest of requested stock, available stock,
affordability and payload, with O(1) arithmetic per candidate. Vehicle volume and heterogeneous
vehicles are not modeled.

### Profile kinds

| Kind | Meaning |
| --- | --- |
| `processor` | Converts inputs to stored goods (food processor, machinery factory) |
| `store` | Retail goods sold to household shoppers (grocery) |
| `service_store` | Aggregate service capacity with no inventory or freight (barber, pharmacy) |
| `field_producer` | Player-drawn field; output and staffing scale with area (grain farm) |
| `extractor` | Player-drawn extraction area over a finite deposit (coal mine) |
| `utility_producer`, `utility_processor` | Power, water and sewage providers |
| `demand_sink` | Household consumption profiles; not bound to assets |
| `unsupported` | Electricity fallback for occupied buildings without a recipe |

### Shipped catalog

At full operation, before staffing, input, stock, deposit and customer limits:

| Profile | Workers | Inputs per day | Output per day | Price |
| --- | ---: | --- | --- | ---: |
| `grain_farm_basic` | 2–6 by area | 0.1 kg machinery per ha | 20 kg grain per ha | 4.8 |
| `coal_mine_basic` | 5 per ha | 4 kg machinery per ha | 9,600 kg coal per ha | 0.1 |
| `food_processor_basic` | 10 | 200 kg grain + 2 kg machinery | 200 kg packaged food | 12 |
| `grocery_basic` | 15 | 200 kg packaged food | 200 kg household supplies | 25 |
| `machinery_factory_basic` | 4 | 30 kg steel + 10 kg metals + 2 kg machinery | 42 kg machinery (40 net) | 20 |
| `personal_service_small` | 4 | — | 80 service | 20 |
| `health_essentials_small` | 6 | — | 120 service | 14 |
| `power_plant_basic` | 20 | up to 10,800 kg coal + 4 kg machinery | up to 24,000 kWh (1,000 kW) | 0.20/kWh |
| `water_plant_basic` | 3 | 0.5 kg machinery | provider presence | 2.0/m³ |
| `wastewater_treatment_basic` | 3 | 0.5 kg machinery | provider presence | 1.5/m³ |

Wages are authored ranges (80–100 for most industry and retail, 50–70 barber, 70–95 pharmacy,
100–130 power plant, 90–110 water). Household demand sinks: 1 kg supplies, 0.10 personal-service
and 0.16 health equivalents per resident/day.

The reference chain is grain farm → food processor → grocery → households. Farms, mines,
processors and utilities consume machinery; the machinery factory imports steel and metals and
keeps 2 kg of its own output for upkeep. A profile without a matching asset cannot be built.

### Asset binding and failure states

- An asset stores one `economy_profile` reference; the profile defines behavior. `worker_capacity`
  comes from the bound profile, overriding the asset manifest. Household capacity and
  `flat_size_m2` remain asset metadata.
- Explicit farms and mines are industry-area assets placed by the player, followed by a drawn
  polygon within 10 m of the building.
- **Missing asset:** the building becomes a *broken asset*: placement is preserved, it takes no
  part in the economy, and it recovers if the pack returns.
- **Missing profile:** the building becomes *economy-broken*: it stays visible but inert, and
  recovers if the profile returns.
- Nothing is ever silently remapped to another asset or profile.

### Runtime tuning

`[runtime_tuning]` holds trade multipliers, the startup treasury, transfer defaults and the
logistics, construction, fiscal, operational-clock, household and viability tables. Shipped
values are quoted in the sections that use them. City fiscal values become live
`CityFiscalPolicy` state ([fiscal policy](#fiscal-policy)).

## Money model

### Households

Households own the money for essentials. Net wages and transfers enter one shared budget;
shopping, utilities and residential property tax leave it. There is one budget per household,
never one per resident.

### Buildings

Each private commercial, industrial, field, extractor or privately operated utility building holds
an `operating_budget`. Sales, export receipts, liquidation receipts and private utility revenue
enter it; wages, input purchases, utilities and business taxes leave it.

- **Startup capital.** A new private business receives owner equity once, at creation:
  `max(workers × average wage × 7 + first imported input buffer, 500)`. A farm or mine receives its
  area-scaled payroll, utilities and opening inputs on its first area commit; resizing never adds
  equity. Equity is capital, not revenue, and there is no refill.
- **City-owned facilities** (player-placed utilities and services) pay wages, construction and
  inputs from the treasury, deposit local fees into the treasury, and never enter the private
  bankruptcy loop.

### City treasury

One ledger, separate from households and buildings. It starts at `startup_treasury_balance =
500,000`; saved balances stay authoritative.

- **In:** income tax, household VAT, business profit tax, property tax and city-owned utility fees.
- **Out:** road construction (15/m) and upkeep (0.1/m/day), city facility construction and
  operation, and household transfers.
- The treasury may go negative for obligations other than transfers. A negative balance is a
  valid state.

### Fiscal policy

`CityFiscalPolicy` is the single live policy object, initialized from `runtime_tuning.fiscal` and
the transfer defaults, saved with the world and changed through the Economy Overview Policy tab
(`set_economy_policy_value`, clamped to authored bounds). Every tax and transfer call site reads
the live policy.

| Policy | Default | Rule |
| --- | ---: | --- |
| Income tax | 12% | Withheld from gross wages. An employer that cannot pay the gross wage pays neither wage nor tax. |
| Household VAT | 8% | Added to store purchases, reserved with the shopping payment and collected at pickup. Seller revenue excludes it. Business purchases pay none. |
| Business profit tax | 10% | At the building's daily phase, on `max(0, operating_budget − baseline)` after its window's flows; the baseline then resets. No loss carryforward; startup equity is never taxed. |
| Property tax | 2 / 25 / 35 per day | Per occupied home / active commercial / active industrial building, × 1.75 per level above 1. City-owned and unzoned buildings pay none. |
| Unemployment benefit | 30/day | Per unemployed adult, for at most `unemployment_max_days` (30) |
| Pension | 30/day | Per elder |
| Child support | 10/day | Per child |

Daily budget history keeps separate buckets for each tax, each property class and each transfer.
Pending transaction taxes roll into the report at the daily close.

### Household transfer payments

Transfers are paid at each household's daily phase, before its property tax and bills and
before its housing review (`households/benefits.rs`, `SIM-02-G`). Unemployed adults are the
household's adults minus the members paid a wage in its 24-hour window, so a worker whose
insolvent employer could not pay draws the benefit at once. When the treasury runs short, slot
order decides who is paid.

- Unemployment benefit stops once every adult is employed (resetting the counter) or after the
  maximum duration. Exhausting it does not by itself cause departure.
- Pension and child support depend only on age composition and need no valid home.

#### Money source

Transfers come from the existing treasury and are cash-limited: they pay in full while the balance
covers them, pay the remainder when it does not, and never deepen a deficit. Admission treats
transfers as reliable only in proportion to treasury coverage of all current claims for
`move_in_benefit_treasury_coverage_days` (7).

## Outside world exchange (`OWA`)

The `OWA` is the external buyer and seller: a startup source of goods and households, a sink for
surplus, and a paid fallback for missing utility service. It is not a local factory.

- Trade requires at least one connected border road.
- Local supply is tried first; the `OWA` is used only when no valid local source exists.
- Prices are `catalog price × owa_import_price_multiplier` (1.25) for imports and × 1.0 for
  export bids. Imports always cost more than exports, so buy-and-resell is unprofitable.
  Freight-window modifiers apply to goods.
- There are no buying quotas, saturation or price decay. Physical delivery, payload, roads,
  labor, inputs, cash and stock limit trade.
- Only industrial-zone producers and explicit field or extractor businesses export. Stores and
  services never export inventory.
- Local buyers have priority: before exporting, a producer holds stock for reachable local
  buyers that can afford it, using the same batching, supplier index and route checks as inbound
  freight. Open inbound shipments already count toward a buyer's coverage.
- Export accumulation: output above a one-day buffer (after local holds and the producer's own
  input reserve) ships once a day's net output, rounded up to a batch and capped at one payload,
  has accumulated. Field and extractor buffers use current staffed output.

### Border connections and physical freight

Imports start at the fastest connected border node and drive to the buyer; exports drive to the
nearest valid border node. Border terminals accept every load at once. There is no per-border job
limit or queue, so road traffic is the only throughput limit (the former four-job cap was removed
on 2026-10-09). Utility fallback is a service purchase, not freight.

### Freight pricing

Each accepted contract stores a carrier quote:

```text
round_trip_vehicle_hours = cached_one_way_movement_seconds × 48 / seconds_per_day
freight_cost = dispatch_fee (4) + round_trip_vehicle_hours × vehicle_hour_cost (4)
```

- Local and import buyers must afford goods plus freight before stock is reserved. The seller
  receives the goods value; the carrier fee leaves the economy.
- Exports require gross sales above the fee and are credited net of freight on delivery.
- Failed inputs refund everything; failed exports earn nothing and pay nothing.
- The quote stays fixed through queueing, delivery and return. Queues and windows add no
  vehicle-hour cost.

## Households

### Representation and composition

- Households are explicit runtime records anchored to a residential home or a farm's single home
  slot. Linked resident agents are the authoritative membership; each household's cached
  counts are recounted from its member list at its hourly phase.
- Age groups: a **child** consumes and cannot work or shop; an **adult** works and shops; an
  **elder** shops and does not work.
- Composition is deterministic at admission:
  - at most two adults and two elders, and no child cap;
  - any household with children has an adult;
  - one-person households are an adult or an elder.
- Household size is capped by the home's `flat_size_m2`: 25 m² baseline, 22 m² per extra
  adult-equivalent and 12 m² per extra child-weighted member. It is not set by household
  capacity.
- A child-only household cannot hold a home.
- Arriving households receive 15 currency per member (at least 10) and 3 days of supplies.

### Household replenishment

`households/replenishment.rs`; `basic_household_demand`: target 5 days of supplies, reorder at
2.5, critical at 1.

- Checks run every 6 hours at a stable per-household offset hashed from home and household IDs,
  so households spread evenly over the check slots (`ECON-19`). A household at zero days may skip
  its offset if a store has stock.
- A sale is claimed only together with an eligible shopper at home. An eligible shopper is an
  adult or elder, in the home building, idle and not carrying another task. Otherwise the
  household waits for a shopper without reserving stock or money.
- Candidate stores must pass both directed trips, `Home → Store` and `Store → Home`, under the
  shopper's real trip planner. Walking proofs answer most candidates from cached pedestrian
  components with identical results. Store discovery continues past depleted nearest stores
  through a deterministic cursor that wraps.
- A household shops at the nearest stocked store whatever the census claimed for it, so a
  household the census assigned to a distant store walks there when its own neighbourhood's
  stores are unfunded (`ECON-22`).
- An unaffordable full refill becomes the largest affordable partial basket.
- The reservation moves stock and money; supplies rise only when the shopper returns home.
  Shopping owns both legs and the wait for pickup and delivery acknowledgement; the work
  scheduler skips it. The shopper waits at home for its departure slot, picks up as soon as
  it reaches the store (VAT is collected then) and delivers as soon as it is home (`SIM-02-E`).
- Each leg's deadline is its travel estimate plus the authored allowance. A timeout before pickup
  refunds; one after pickup records a failure. After 3 failures the household enters
  `failed_terminal` and retries only on its normal cadence.
- States: `stable`, `needs_replenishment`, `waiting_for_shopper`, `shopping_to_store`,
  `shopping_returning`, `fulfilled`, `cooldown`, `failed_terminal`.

### Services

Service stores sell aggregate capacity each hour. Resident demand is capped by staffed capacity,
households are debited proportionally, and revenue goes to service stores by capacity share.
Since `SIM-02-I` this runs through a money pool (`households/service_sales.rs`): each housed
household pays at its hourly phase its members' share of the gross revenue the last finished
hour's residents and store capacity allowed, up to its budget; each store at its phase draws its
share of the pool the last hour collected, by the capacity it recorded in that hour, which sets
its activity for its electricity meter. Money a removed store did not draw carries into the next
pool. The pool is saved and counted with city cash by the money-conservation checks.

A service store's active worker slots follow its own funded census activity: the share of output
its reachable, affordable households reserved. There is no inventory, freight, VAT or
liquidation for services. Personal services also schedule capped representative visits for
visible city life (`households/service_visits.rs`); these never change accounting.

### Household housing affordability, relocation, and eviction

`households/relocation.rs`. Household poverty moves the household; it never downgrades the
building.

```text
household_daily_essential_cost = supply cost per day + daily utility charges
household_reserve_days = household_budget / max(household_daily_essential_cost, ε)
```

- **Staying:** `reserve_days ≥ stay threshold[level]` (0.5 / 3 / 6 days for levels 1–3).
- **Moving in:** `reserve_days ≥ move-in threshold[level]` (0.5 / 6 / 12 days).
- After 2 consecutive stay failures (`stay_failure_days_before_eviction`), the household moves to
  the first affordable vacant home. Candidates are ordered by higher level, then shorter
  distance, then lower building ID, and found by chunk-ring search. A household that still
  passes may move only to a strictly higher level.
- With no affordable vacancy the household is evicted and becomes unhoused. Eviction cancels an
  active shopping trip through the normal refund rules and releases the shopper; once the
  household is rehoused, a member away from home and work goes to the new home after its
  current trip.
- An unhoused household with an independent member queues its rehousing at its hourly phase;
  the `housing requests` queue serves rehousings and the daily phases' housing reviews oldest
  first, a few per slot. Rehousing claims the highest affordable vacancy by level and lowest
  building ID, which the next hour's admission may already have taken (`SIM-02-J`).
- `unhoused_days_elapsed` counts consecutive settled days without a home. Demand's exit policy
  interprets it ([`demand.md`](demand.md#household-removal)).

### Farm households (`ECON-08`)

Each explicit farm provides exactly one household slot, filled by ordinary admission or
relocation. Resident adults claim on-site jobs first with zero commute and switch activity at
shift times without a road trip. Shrinking a field reduces jobs but keeps the household. A
bankrupt farm withdraws its vacancy; removing a farm leaves its family unhoused through the normal
lifecycle.

## Funding census (`ECON-17`)

`JobFunding` (`hiring.rs`) is rebuilt in every hourly workplace pass. It is the single
answer to "how much of this operation can run":

- **Capacities:** the largest workforce whose forecast recurring sales cover operating costs, and
  whose cash, usable stock and paid in-transit cargo cover working capital
  (`operation.rs`). Owner equity is capital, never sales.
- **Activities:** accepted recipe activity, as a share of full output. Sales-scaled stores,
  service stores, farms and mines produce, restock and staff to it.
- **Customers:** each store, in stable building order, claims its output from the nearest
  reachable households with exact directed shopping access (`reserve_consumer_outputs` →
  `ConsumerMarket::claim_reachable`, `customers/routing.rs`), with no distance bound, from
  recurring after-tax wages and funded permanent transfers after housing and utility costs.
  Savings and temporary benefits are excluded. With spare store capacity the first-built stores
  claim every household in the city (`ECON-22`).
- **Business buyers:** business buyers claim inputs from local suppliers in recipe order, with
  jointly checked financing (`customers/business.rs`). Suppliers of buyers that cannot pay are
  withdrawn; cyclic groups resolve in stable building order (`hiring/closure.rs`).
- **Up-front proofs:** an operation gets a zero ceiling before the first pass when it cannot fund
  one worker, or when its best possible revenue per unit of activity does not exceed its
  cheapest possible purchased inputs. The best revenue counts every net output at the better of
  the local and export price. The cheapest input takes the cheapest authored seller or the
  outside price, at the site's goods multiplier. These prices are fixed for the census, so full
  replay could never fund such an operation either. Its suppliers then lose that possible buyer
  through customer propagation, which prepares the business layout itself when a pass ended
  before validation. A loss-making dependency cycle therefore resolves in two censuses at any
  length (`ECON-13-C`).
- **City services:** they use their explicit fiscal funding limit. Unfinished and inactive
  private employers advertise no live jobs.

Hiring, daily payroll, worker shedding, store output, construction viability and household
admission all read this census. Each census copies every building's funded jobs onto it
(`funded_job_capacity`, saved). A worker is paid at its own daily phase while its workplace has
paid fewer than those jobs, bounded by the physical workplace, in the workplace's own window
(`SIM-02-G`). Service stores and explicit work areas release the workers whose phases come after
their funded jobs were paid; other unpaid workers quit after 2 unpaid days.

Costs: market aggregation is O(B·ports + households·categories) in fixed parallel chunks.
Household and supplier claims commit serially through the existing component and chunk indices.
Pending household purchases reduce authoritative balances immediately but defer downward
availability-index updates until owner acceptance. Rejection of an ordinary quote restores only
balances; releasing an incumbent still publishes restored eligibility immediately and rolls it
back on rejection. Thus a pending index can overestimate availability but cannot hide a buyer.
Repeated queries check balances before routing. With `Q` resource queries and `S` staged accounts
in a proposal, deferral can add at most `O(Q*S)` candidate checks; it adds no route queries or
allocations and avoids tree updates for rejected purchases. `Q` follows proposal recipe size,
not city population. Acceptance and enclosing transaction rollback publish exact eligibility.
Consumers read capacities in O(1).

## Labor

### Buildings demand labor

Workplaces expose funded job slots and fixed authored wage ranges. There are no dynamic wages.
Work schedules come from the profile or zone (`work_profile_by_zone_type`).

### Agents supply labor

Working-age adults without a job accept any reachable funded offer, regardless of savings. This
matches the labor pool admission assumes. Employed adults reconsider when their job lock expires
(5–9 days, spread per agent) or after 2 unpaid days, and switch only for a better score:

```text
work_score = 0.35 × income_pressure + 0.35 × supply_pressure + 0.20 − 0.10 × commute_penalty
```

The commute penalty is commute seconds ÷ 1,800, capped at 1. Switching requires a score of at
least 0.45; unemployed adults take the best offer regardless.

**Job search.** Each searching home key `(home, has_car)` keeps the 24 best options, ranked by
commute penalty, then higher wage, capacity, vacancy and lower building ID. Two exact lower bounds
keep searches local:

- **Ring stop.** Scanning 512 m chunk rings outward stops once the next ring's bound (the home's
  door-to-network time plus straight-line travel at the top network speed) cannot beat the 24th
  option.
- **Per-pair pruning.** Once 24 options are held, a workplace is routed only if one of its trip
  candidates can finish within the 24th option's commute; candidates are routed in bound order.

Ranked options and commute seconds therefore match an exhaustive search; `commute_bounds.rs`
checks over 10,000 pairs. Commutes include live frontage congestion, so a home's options are not
reused between hours.

Two caches serve the search:

- **Route cache:** commute seconds by `(home, work, car)`, cleared on any building, entrance or
  CCH change. Each quote expires 24 workplace passes (hours) after it was added (`SIM-02-G`).
- **Network legs** (`SharedNetworkQuotes`): the road-network part of each commute, shared by
  every worker thread across hours. A car leg uses the cost-only CCH query; a walking leg keeps
  the path for the pedestrian connector check. It is emptied on a CCH or lane change. Loading
  sizes it and prepares the search's per-worker route scratch, so the first search after a load
  prepares nothing (`SIM-02-K` part 5).

Neither cache changes a result.

Searches run from the `job requests` queue, queued at each agent's hourly phase and served a
slot's searches together against the hourly job board (`SIM-02-J`,
[`simulation.md`](simulation.md#request-queues)). Within a slot, resident workers whose best
job is at home are assigned first; all others apply in agent order against live slot counts. A
worker who keeps its job, including one whose better options filled before it applied, is
locked again.

## Production

### Building throughput depends on staffing

```text
throughput = base_rate × staffing_factor × input_factor × output_headroom_factor
```

- `staffing_factor = filled / worker_capacity`. Farms and mines use their area-scaled capacity.
- `input_factor` is the limiting input coverage against the current hourly need, not the full
  daily recipe.
- There is no utility factor. Missing utilities cost money through the
  [`OWA`](#utility-service-layer) fallback; they do not throttle production.
- **Census activity bounds staffing and output.** A store or service store without census
  activity produces nothing and holds no paid staff. Groceries refill their shelf at full staffed
  rate up to `stock_target_days` of funded sales, then produce at their census share, so rounding
  up to whole workers creates no surplus. Farms and mines staff to the exports and local buyers
  they can finance.

Hourly production is O(buildings) with Rayon; area production follows the indexed site list.
Input stock targets hold at least two minimum batches; reorder thresholds keep at least one.

### Field staffing density (`ECON-06`)

Field and extractor output and storage scale per 10,000 m². Staffing uses
`worker_capacity_area_m2`:

- Grain farms have `min(6, max(2, ceil(ha / 10)))` jobs.
- Mines have 5 jobs per hectare, with ordinary rounding.

Uncommitted areas have zero jobs. Construction (20,000/ha farm, 25,000/ha mine) and area utilities
scale with the full area, even after the staffing cap.

### Field placement and editing (`ECON-07`)

New and resized fields must:
- stay inside the world;
- be at least the profile minimum (4 ha for grain) and within 10 m of the farm;
- avoid roads, sidewalks, building sites, zoning parcels and other fields (shared edges are
  allowed).

Clearance uses exact polygon intersection over the existing 512 m chunk layout. Each vertex
release in the inspector's **Edit Field** mode validates and commits atomically; a rejected move
restores the last accepted polygon. Resizing grants no equity. Agriculture owns the saved
polygons; loading rebuilds the reservations.

### Machinery upkeep (`ECON-09`)

Machinery is a recurring input, with no individual machines or wear timers:
- farms use 0.1 kg/ha and mines 4 kg/ha;
- food processors use 2 kg, the power plant 4 kg, and water and sewage plants 0.5 kg each;
- the factory keeps 2 kg of its own output.

Consumption follows actual production. An idle producer consumes nothing. A depleted mine loses
its productive area, jobs and machinery demand, but keeps its pit and stock. Water and sewage
operation and power generation need machinery stock as well as staff.

Deposits: new full-richness coal deposits hold 480 kg/m², or 500 full-production days; older
saves keep 6 kg/m². Editing a mine keeps its cumulative extraction and never replenishes it.

### Utility service layer

Power, water and sewage are networked services resolved independently per service. They are not
freight.

- **Power** is dispatched hourly against metered kWh demand. Connected plants generate the least
  of demand, staffed capacity (50 kW per worker) and available coal (0.45 kg/kWh). Unmet demand
  is imported. Each phase meters with the last finished hour's coverage and each plant generates
  that hour's dispatch of its current capacity (`SIM-02-I`), so a change in demand or capacity
  reaches both one hour later. A home's standby load splits equally among its household places.
- **Water and sewage** are local when a staffed connected provider settled operational in the
  last closed calendar day; otherwise they fall back to the `OWA`. Providers share that day's
  local receipts (`SIM-02-G`).
- **Bills:** `local share × local price + missing share × local price × 1.25`. Local prices are
  0.20/kWh, 2.0/m³ water and 1.5/m³ sewage.
- **Rates:** electricity per profile (`electricity = {idle, operating, per_resident}`):
  households use 1 kWh per home plus 5 kWh per resident each day. Workplaces add 1 m³ water and
  1 m³ sewage per day, plus authored per-hectare upkeep for farms and mines.
- Households pay 2 currency per resident per day for water and sewage. Their electricity is
  metered separately; unpaid bills carry forward as arrears and reduce housing reserves.
- Private businesses pay daily even into a negative budget. Unpaid amounts are recorded as owed,
  never as collected revenue.
- City-owned providers deposit fees into the treasury, and `OWA` fallback spend leaves the city.
- **Electricity funding policy:** the citywide setting sets power-plant staffing; per-plant
  overrides survive citywide changes. Non-finite inputs are rejected.

## Logistics model

`logistics.rs` and `logistics/`:

- **Shipments:** building-level batched shipments with a stable ID, resource, kg (at most one
  payload), source and destination (building or border node), carrier, quote and status:
  `Queued`, `InTransit`, `Returning`, `Fulfilled`, `Failed` or `Expired`.
- **Reorders:** inputs reorder below `reorder_threshold_days` toward `stock_target_days`, at most
  one open normal request per resource and buyer. Suppliers cap outbound reservations, and hourly
  review is not hourly trucks.
- **Reservations:**
  - creating a shipment reserves source stock and buyer demand;
  - the planning slot loads the stock into the shipment at its end; the carrier leaves near a
    target slot in the hour ahead, levelled per minute (`SIM-02-J`), and the seller is paid only
    on delivery;
  - failure, expiry, cancellation or building removal releases both sides deterministically,
    refunding buyers (or the treasury for city services);
  - a delivered shipment stays settled when an endpoint is removed.
- **Carriers:** one truck agent per shipment. It goes source → destination, then returns empty
  to its origin; the return leg holds no reservation. Delivery and return are settled when the
  truck arrives. Travelling legs time out after
  `max(6, 4 × eta_hours + 2)` operational hours.
- **Supplier search:** `SupplierCandidateIndex` lists a supplier under a resource only while its
  unreserved stock fills one batch. Searches scan chunk rings outward from the buyer and stop at
  the nearest qualifying supplier, so an empty search ends at once (`ECON-15`). The index is a
  rolling snapshot (`SIM-02-I`): freight requests read the listing the previous hour's
  sweep built, each supplier as it stood at its slot of that hour, and the choice still checks
  the supplier's live stock.
- **Routing:** freight routes use the exact car-access abstraction of the entrance cache, with
  direct same-lane frontage travel when the exact points are in forward order.
- **Border quote preparation:** retain the existing per-border network tables for dispatch,
  and derive their exact per-node minimum once per routing revision or border-set change.
  This additional preparation costs O(nodes × borders), with O(nodes + borders) storage;
  subsequent building quote refreshes cost O(buildings), evaluating live frontage delay once
  per arrival end. The minimum uses the original table's f32 values and the original order of
  frontage/local additions, so fees remain bit-identical. It does not select a dispatch border.
- **Retries:** failures enter a cooldown that ends at the building's next hourly phase (one
  hour for a failure the building's own plan made) and escalate to a visible terminal state
  after 3 attempts. Terminal states never block recovery after cooldown, budget, supplier or road
  changes.

## Private construction

Demand-selected private buildings enter the world as construction sites. The record exists at
once, so the parcel is claimed and the site renders, but it supplies no homes, jobs, production,
shopping or utilities until completion.

- **Durations** (`runtime_tuning.construction`): 6/12/18 hours for residential, 8/16/24 for
  commercial, 12/24/36 for industrial, by level. A site completes exactly its duration after it
  starts, at its own slot (`SIM-02-D`), not on the hour.
- **Committed capacity:** demand counts constructing and queued projects as committed, so no
  duplicate is started.
- **Instant changes:** upgrades and downgrades are instant replacements; only fresh spawns build.
- **Visuals:** MultiMesh site pad, foundation and scaffold, with the full-scale final asset rising
  continuously with construction progress. It never scales, flattens or recolors.

## Building Bankruptcy

This applies to private commercial, industrial, field, extractor and privately operated utility
businesses; city-owned facilities are excluded. At the building's daily phase, in settlement
order (`SIM-02-G`):

1. **Check:** if its last window ended in `budget_distress` and this one is insolvent too, mark
   `is_deserted`. A building always gets one full distress window first. A window is insolvent
   when the budget is negative or a wage went unpaid for lack of cash after selling output.
2. **Utilities:** charged even into a negative budget.
3. **Distress:** if the budget is negative, liquidate all output on hand at distress price
   (`catalog × owa_distress_liquidation_multiplier` = 0.25) and set `budget_distress`; a window
   that left a wage unpaid for lack of cash sets it too; otherwise clear it. Cargo a shipment has
   loaded has already left the stock.

**Wages** are paid at each worker's own daily phase from the budget. When cash is short, the
employer first sells enough output at distress price to cover the wage. Otherwise the worker
goes unpaid, and wages never push the budget negative.

Desertion is permanent. A deserted building leaves supplier search, hiring, production and demand
capacity; its workers are ejected at the start of the next workplace pass. Clearing empty deserted
sites belongs to demand ([`demand.md`](demand.md#failed-site-clearance)).

## Level-change viability

Demand proposes level changes, zoning defines the legal family, and this section defines economic
viability (`demand/viability.rs`, `runtime_tuning.viability`). Poverty never triggers a building
level change.

- **Residential upgrade:**
  - occupancy is at least the target level's minimum (0 / 0.65 / 0.85);
  - every occupant household meets the target level's move-in reserve.
- **Residential downgrade:**
  - occupancy is at most the current level's maximum (1.0 / 0.20 / 0.15);
  - the lost homes fit within the ready housing left after workplace claims, the 10% vacancy
    reserve and homes for unhoused households.
- **Business upgrade:**
  - staffing ratio is at least 0.85, and stores also need stock;
  - industry also needs input coverage of at least 0.75 and output headroom of at least 0.25;
  - selection then requires the target's recurring income to cover its costs (retained staff and
    target-level tax), and cash, usable stock and same-recipe paid cargo to cover its working
    capital;
  - extra workers must come from unemployed adults or ready homes, and its customers are reserved
    alongside other projects.
- **Business downgrade:** allowed only to repair an operation that fails its income or capital
  check, using a replacement that passes both.

## Runtime constraints

New economy work reuses the household and building stores, resource catalog, supplier index,
customer indices and routing and chunk indices:

- **Aggregation bound:** O(H × C + B × P + S) per pass for H households, C consumption
  categories, B buildings, P profile ports and S shipments. Routing and candidate selection are
  bounded separately.
- **No full scans:** no household-to-every-shop or building-to-every-supplier scans, no per-agent
  inventory, and no hot-loop allocations.
- **Parallelism:** bulk passes use Rayon with deterministic reduction order.

## Validation

### `ECON-11` balance tests

`rust/src/simulation/economy/definitions/balance_tests.rs` checks the shipped file on paper:

- **Solvency:** every business is solvent at full staffing and maximum wages, with local inputs
  where a producer exists and imports otherwise (`shipped_businesses_are_solvent_at_full_staffing`).
- **Self-check:** a test confirms the solvency check reports a loss.
- **Jobs baseline:** the closed-chain jobs-per-1,000-residents figure matches its recorded value.
- **Exports:** basic industries profit from exporting all output.

These are necessary, not sufficient, because they omit overhead, weak sales and timing. Worksheets
(`starter_ledger`, `producer_balance_sheet`) are ignored tests run on demand. Run:
`cargo test --lib economy::definitions::balance_tests`.

### `ECON-12` growth scenario

`GrowthScenario::starter_town` (`nodes/sim/core/scenario.rs`) builds a flat 2 km town through the
player paths:
- a border road and main street with commercial zoning;
- residential and industrial cross streets;
- the `godot/bootstrap/mods` pack.

It then runs frame by frame at 32×. `rust/tests/growth_scenario.rs`:

- `concurrent_runs_record_identical_years`: determinism across concurrent runs.
- `read_only_observer_preserves_growth`: observers do not change outcomes.
- `starter_town_funds_industry_services_households_and_city_for_two_years`: the two-year
  healthy-growth acceptance, with firms reconciled individually.
- `producer_town_grows_and_funds_its_firms_households_and_city_for_a_year`: `DEM-02-A`, using
  `GrowthScenario::producer_town` (the same streets plus a placed farm, coal mine and power plant).

Both two-year and producer-town tests also assert the `DEM-02-C` vacancy bound.

The library suite adds scarce-labor expansion, recovery and two reload-continuation cases.
`METRUM_GROWTH_{REPORT,LONG_REPORT,EXPANSION_REPORT,RECOVERY_REPORT}` and `METRUM_PRODUCER_TOWN_REPORT` write reports, and the
saved-town replay (`diagnose_saved_city_growth`, ignored) uses `METRUM_DIAG_{SAVE,MODS,REPORT,DAYS}`
with a copied save and read-only mods. Economic changes that must not alter behavior compare
these five reports byte for byte.

### Performance contract (`PERF-01`–`PERF-05`)

**Scope clarified 2026-10-14:** the deliverable is a fast diagnostic benchmark suite.
`PERF-01`, `PERF-02` and `PERF-06` infrastructure is complete; fixing the simulation scaling
issues in `PERF-03`–`PERF-05` is parked. Existing budgets and counters remain visible and
unchanged. A reported budget failure describes measured simulation performance; it does not
mean the diagnostic suite is unfinished. Earlier implementation notes below retain their
historical acceptance status.

Routine commands:

```bash
./run.sh --perf-contract                 # Full overview; latest cached run: 64.20 s
./run.sh --perf-contract --phase census  # One phase at both tiers
./run.sh --perf-contract --agents        # Short agent-tick checks only
./run.sh --perf-contract --diff HEAD     # Select affected phases; keep full-day coverage
```

The runner writes timings, deterministic work counts and failures to `report.json` under its
artifact directory. Current full runs return exit code 1 for the known budget/scaling failures
while retaining the complete report. Compilation and initial snapshot generation are reported
separately from cached runtime; `--binary "$TEST_BINARY"` skips compilation. Use `--plan` to
inspect selection without running, and `--prepare --keep-baseline` to prepare snapshots and
retain the first baseline executable (it deliberately refuses to overwrite an existing one).

Every economy change keeps these checks. They replace `ECON-13` (closed 2026-10-12), whose
100k runs took hours. The contract uses saved cities, deterministic work counts and one run per
tier, so a full check takes minutes. The 100k and 1M tiers are deferred.

**Fixtures.** Two separate `PopulatedCity` workloads at 10k and 30k residents use the ordinary
save format. The day-12 baseline retains the original census, staffing and search workload.
The day-16 fiscal-stress workload starts from that baseline, sets commercial and industrial
property tax to 500 through the ordinary policy API, then advances normally. It must place
or remove buildings in every subsequent hourly interval. Initial job locks are unchanged.
Both workloads independently retain all time and work-scaling limits below; their results
are never pooled. Churn coverage belongs to the fiscal workload, while the original baseline
churn is still reported. Bankruptcy reduces employment, so fiscal results cannot substitute
for baseline performance acceptance.

Each snapshot records its save-format and fixture revisions, tier and boundary; the runner
regenerates it when these change. Generation must show that loading preserves every subsequent
hourly `GrowthDayRecord`, placement/removal counts and the checked agent movement/schedule state
through the next daily boundary. Fiscal generation also enforces 24/24 active intervals before
publishing its cache. Snapshots are local build products and are not committed.

Fixture revisions hash the fixture source and algorithm marker, bootstrap JSON/TOML
manifests, `economy/profiles.toml`, and `demand/growth_profiles.toml`. Relative paths and
contents participate, so changed tuning or renamed assets cannot silently reuse a city
generated from different inputs. Meshes and textures are excluded because fixture setup
does not read them. Simulation changes that alter fixture generation still require a
fixture-source change or an explicit algorithm-marker bump.

Fresh revision-guard validation (2026-10-14), release binary SHA-256
`ad0c54a2a25b9b8bdf6af14609ec83675b499542c0b2a184e09c2ea9b0dca8a6`:
the isolated configuration/asset mutation test and lane-geometry regression pass, as do
all 14 runner regressions. New 10k/30k snapshots pass exact continuation; generation and
validation take **25.00/72.30 seconds**, reported separately. With eight Rayon workers and
affinity `0,2,4,6,8,10,12,14`, the complete cached runner then finishes in **56.05 seconds**;
both preparation reports have `generation_s: null`, proving cache reuse. Next-day records
and every work count match the earlier hourly-churn reference. Hour/day samples are
23.15/7.03 ms and 69.26/34.77 ms. All isolated phases, locality checks and agent checks pass;
the same nine full-day gates fail (two fixture-coverage gates, 30k daily time and six work
counts). The old snapshots are retained separately and the revised snapshots populate the
default local cache. This verifies cache invalidation, not overall `PERF-01` acceptance.

The one-day report includes `building_churn`: 24 chronological `[placed, removed]`
pairs, one per simulated hour interval. These count successful allocator insertions and
removals, including cleanup, rather than reference revisions or execution-census events.
The runner rejects missing coverage and any interval with neither event. A nonzero daily
total alone does not prove the required hourly churn. Instrumentation is test-only, O(1)
per insertion/removal, and allocates nothing in simulation loops.

Fresh coverage check (2026-10-14), release binary SHA-256
`f0da0caa727a0b604e9f44cbd596c96610405b58ea1372534bc5babc56f6e0d5`:
the current day-12 snapshots have churn in only **8/24** intervals at 10k and **20/24**
at 30k. Both fail fixture acceptance; an execution census is not evidence of a placement.
With `RAYON_NUM_THREADS=8`, affinity `0,2,4,6,8,10,12,14`, and
`METRUM_PERF_RESIDENTS` set separately to each tier, run
`METRUM_PERF_CACHE="$ARTIFACTS/snapshots" taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" nodes::sim::core::populated_city::contract::perf_contract_day --exact --ignored --nocapture --test-threads=1`.
The two runs took **2.98/9.11 seconds**, including loading, with unchanged next-day records
and all prior work counts. Hour/day samples were 22.90/7.01 ms and 68.77/34.41 ms;
the previous seven time/work failures remain, plus the two newly enforced coverage failures.
Fourteen runner regressions and 38 allocator regressions pass (five allocator checks ignored).
These are targeted checks, not a fresh complete-contract or five-growth-report run.

A bounded follow-up uses the ignored `perf_contract_churn_scan` test at the same module path
and with the same environment. It follows each existing snapshot for seven days without
changing job locks, economic inputs or cached saves. Binary SHA-256
`d5b23a838a1183d81e1babc750db5e6dafdd00365e338dd0572332581bacd344` completed the
10k/30k scans serially in **10.28/34.26 seconds**, including loading. Active hourly intervals
on days 12–18 were **8, 6, 4, 2, 2, 0, 2** at 10k and **20, 15, 14, 13, 15, 14, 14**
at 30k. No boundary in that window meets fixture coverage. Removal pacing accumulates
`eligible_candidates × authored_daily_fraction / 24` credits per operational pass
(`demand/building_actions.rs`, `demand/credits.rs`); merely waiting longer is not evidence
of sustained churn. Keep the current snapshots as diagnostic references while developing
a representative fixture with sustained demand-driven rebuilding. This scan is diagnostic
only and is not added to the routine one-day contract.

The complementary `perf_contract_early_churn_scan` builds the ordinary city, advances to
the first post-warm-up midnight and checks days 4–12. Binary SHA-256
`7bfd8a4a9a1758e54a1356e85e32f785f020c62f7ff8b11778de3b5feaa45840`, with the same
workers and affinity, found **0, 0, 0, 0, 0, 0, 6, 8, 8** active intervals at 10k and
**0, 0, 0, 0, 0, 0, 3, 15, 20** at 30k. Generation to the first boundary took
**9.86/26.77 seconds** separately; total diagnostic runtimes were **20.74/61.34 seconds**.
Together, the scans rule out every midnight boundary from day 4 through day 18 in the
current setup. The historical every-hour observation from day 7 applied to **100k**, not
these smaller tiers. The hourly coverage requirement remains unchanged.

A setup-only industrial oversupply trial (2026-10-14) repainted one residential row per
16-row cycle as industrial before filling any lots, retaining streets, admission and initial
job-lock behavior. At 10k, days 4–12 had **0, 0, 0, 0, 0, 0, 3, 6, 6** active intervals,
with no placements. Initial preparation took **10.13 seconds** and the complete diagnostic
**20.99 seconds**, using eight workers and the affinity above. It also exhausted the spare
housing, reaching 5,236 households with zero vacant homes. This variant was rejected before
running 30k; the original snapshots and all acceptance gates remain unchanged. Diagnostic
binary SHA-256: `cd11954eed5b2f0b8c4ea7f9d56c71180dea4574db34b2b8c66f580907ddccec`.
The trial source and log are retained with its local artifacts; it is not a routine test.

A separate fiscal-stress diagnostic finds a real hourly-removal window at both tiers.
`perf_contract_fiscal_churn_scan` loads the unchanged day-12 snapshot, sets commercial and
industrial property tax to 500 through the ordinary policy API, then advances normally for
14 days. With eight workers and the same affinity, both tiers have **24/24 active intervals
on every day 16–25**. Total diagnostic runtimes are **23.78/68.63 seconds**, including
1.42/4.04 seconds for loading and policy setup. Binary SHA-256:
`f96299357ce6a76417b6ec352cb750a27c759098d6fdd2e38d3903c4ce05c462`.
The day-16 intervals remove 3–6 buildings at 10k and 11–19 at 30k, with no placements.
This is a bankruptcy stress case: employment falls sharply, so it cannot replace the
original workload's census, staffing or search gates. The original snapshots remain intact.
Saving and validating the day-16 boundary and integrating both workloads into the runner
remain pending; these diagnostic results do not close fixture acceptance.
The first day-16 save/load check fails at 10k after **8.53 seconds**: in the first subsequent
hour, the loaded city has 268 vacant homes and one unhoused household versus 267 and zero
straight-through. Population, employment, jobs, buildings, treasury and six removals match.
Binary SHA-256: `b42e6206683e823db828b6022f4dc83ef8f52735e46642315817c26fc9730af0`.
The 30k continuation check was not started after this failure. The housing difference must
be resolved before this stress fixture can be accepted.
The focused follow-up identifies household 157: the straight run assigns it home 162,
while the loaded run leaves it unhoused. Loading rebuilt vacancy/admission indices while
the allocator's asset registry was empty, then restored the registry without rebuilding
those indices. The load bridge now rebuilds them immediately after restoring the registry;
the existing registry-restoration regression checks that a vacant home is available before
the first tick. This costs O(buildings + vacancies log vacancies) once during loading and
adds no tick-path work. Fresh release validation passes the registry-restoration regression,
16 save regressions (three ignored), and three queued-execution reload regressions.
Both fiscal continuations now match every hourly record and placement/removal count, with
24/24 active intervals. Serial elapsed times including preparation are **11.26/31.82 seconds**
at 10k/30k; detailed agent-state comparison and revision-checked runner integration remain
pending. Binary SHA-256:
`6345fbad1bfff2bdd099ba877ac539f3408949d3abe4362a0adc5076fe646888`.
The original cached workload retains identical records, every work count and hourly churn
against the retained local-capital build. Fresh single-run hour/day samples are
**22.77/6.62 ms** and **69.29/34.06 ms**, with total day checks taking **3.05/9.33 seconds**.
These are routine checks with eight workers and the affinity above, not speedup evidence;
the original scaling failures and 30k daily-budget failure remain open.

The two-workload runner and v5 snapshot integration now compile, and 19 Python runner
regressions pass, including independent workload limits and partial-failure reporting.
Release binary `ec499f9b0a39a9137371b98afc0d81bf25af9f2abc55860e722538a1f1b218fa`
passes both snapshot unit regressions (one manual test ignored). Fresh baseline generation
passes the strengthened hourly record/churn/agent-state comparison in **25.23/72.22 seconds**.
The 10k fiscal snapshot also passes in **11.39 seconds**. At 30k, fiscal preparation fails
after **24.50 seconds** at continuation hour 1: agent 627 has identical position, route,
home and active target, but the loaded planned target is absent instead of home 14930.
The SQLite snapshot contains that target correctly. No 30k fiscal cache was published and
the expanded full timed contract was not started. This new plan-state mismatch must be
resolved before fixture acceptance; matching aggregate records alone is insufficient.
The load-only probe reproduces it in **4.07 seconds**, before a tick. The orphan-repair
guard preserved delayed retries but missed an immediate replan after demolition: an empty
path with valid node, edge and destination and zero retry counters. The loader now preserves
that live state as well; agents with no usable destination retain the existing repair path.
The focused retry regression covers both cases. Release binary
`cb0d58b84562146fb8e77513bb70f853d675863ac87853b25508e77b37ac8240` passes all
33 save regressions (three manual checks ignored) and the load-only plan probe in 4.05 seconds.
Both gameplay queued-growth and busy-city save-continuation regressions pass in 23.05 seconds.
The 30k fiscal snapshot now passes exact hourly continuation, including agent state, in
**31.72 seconds** and is published. Both baseline snapshots and 10k fiscal stress are reused.

The complete two-workload contract takes **67.16 seconds**, including all workload/phase
loads, locality checks and the short agent contract; the 31.72-second initial fiscal
generation is reported separately. All 14 isolated phase checks, both locality gates and
all six agent samples pass. The retained baseline binary's hash is verified. Reproduction:
`RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS"`.

| Workload | 10k hour/day (ms) | 30k hour/day (ms) | Active intervals, 10k/30k |
|---|---:|---:|---:|
| Baseline day 12 | 23.09 / 6.83 | 69.92 / 34.24 | 8/24, 20/24 |
| Fiscal stress day 16 | 58.06 / 47.13 | 128.73 / 150.26 | 24/24, 24/24 |

**Thirteen limits fail.** Baseline retains the same seven time/work failures: 30k daily
settlement, census rows, owner slots, staffing trials, supplier searches/routes and job keys.
Its records, every work count and churn sequence match the retained local-capital build.
Fiscal stress fails all four timing budgets plus supplier searches (**2,818 → 12,533**) and
combined CCH searches (**1,914 → 15,752**) against the same **3.4176×** work bound. Its census
rows (**182,930 → 501,935**) and staffing trials (**241,255 → 683,638**) pass scaling.
Fiscal lot regeneration accounts for **965.90/1,788.22 ms** inclusive over the full day and
is the next timing investigation; removals total **112/357**. These single release samples
establish current failures, not speedup claims. Fixture coverage is now validated; performance
acceptance remains open, and no original baseline time/work gate was removed.

A subsequent **diagnostic-only** 10k fiscal trace with the same binary and worker/affinity
settings finishes in **3.23 seconds**. `cell_lot_detail` reports **93 local regions**,
**958.36 ms** in cell generation and **31.74 ms** in lot derivation, with **zero candidates,
created, retired or reattached lots**. Command:
`METRUM_DEBUG=1 METRUM_DEBUG_FILTER=economy_phases METRUM_PERF_RESIDENTS=10000 METRUM_PERF_CACHE="$ARTIFACTS/snapshots" RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" nodes::sim::core::populated_city::contract::perf_contract_fiscal_day --exact --ignored --nocapture --test-threads=1`.
Inspection traces this to ordinary occupancy release queuing the existing 512 m dirty regions;
it does not establish a whole-city scan. Cell regeneration after release is the next target,
while retaining the occupied-lot road-removal/frontage revalidation contract in `zoning.md`.
Trace timings are for diagnosis only and do not replace the unprofiled contract above.

**Demand cell-cache integration (2026-10-14).** Demand preparation now consumes the existing
cell-generation completeness cache used by selection, generating cold chunks or local dirty
envelopes and marking them complete. Every dirty region still derives lots; occupancy and
frontage invalidation are unchanged. The work bound is requested chunks plus generation in
dirty envelopes, with no additional spatial index. Targeted cell and zoning regressions pass
**107 tests, seven ignored**. Release binary SHA-256
`97af8fc6b7dbbfe082b9936f2e16a998aa16c1f8dd2e84f75b0a4ac00839e5ba`, eight Rayon workers,
affinity `0,2,4,6,8,10,12,14`: four short next-day checks preserve records, work counts and
hourly churn exactly against the retained immediate-replan build. The integrated cached run
(`RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS"`)
finishes in **64.91 seconds**, with no fixture regeneration. All isolated phases, locality
and agent checks pass. Baseline hour/day samples are **16.95/7.08 ms** and **51.63/34.86 ms**;
fiscal samples are **30.17/37.50 ms** and **85.62/129.19 ms**. Fiscal hourly budgets now pass;
the original seven failures plus fiscal daily budgets and supplier/CCH scaling leave **eleven
failed limits**. Fiscal lot-phase totals are **320.49/771.40 ms**. These are routine single
samples, not a statistical speedup claim. Recovery and scarce-labor expansion pass with
byte-identical reports. After correcting an initial stale executable selection, all four growth
integration tests pass on the Cargo-emitted executable (SHA-256
`aa36e1369d9f22f02777b8c7d9ab7a2ec2eb24ffbd6727b14ff5850e8053c4da`). All five reports
are byte-identical to the retained import-replay reports. This integration build used the default
x86-64 target and is correctness evidence only, not a matched timing comparison.

A diagnostic-only 30k fiscal day on this binary finishes in **7.66 seconds** using the
trace command above with `METRUM_PERF_RESIDENTS=30000`. Midnight workplace assignment reports
**58.05 ms preparation**, **58.84 ms total**, and `skipped_reason=no_open_jobs`; household
daily settlement totals **62.18 ms**, inside **129.34 ms** for the full daily phase.
Preparation includes commercial activity refresh, the live funding census and inactive-worker
ejection. This identifies a narrower next investigation, not proof that any of those steps
can be skipped. Traced samples are not acceptance timings.

A fixed-cost profitability bound was tested and removed: even maximum output revenue below
property tax plus one wage cannot fund an operation, but this condition prunes no additional
work in either saved workload. Candidate SHA-256
`feeb99f97cd0693b506ddafd924a79b8f836ec4a9f451101daaf27a5ca707f24` passes the new tax-change
oracle and 14 market regressions (six ignored); all four next-day records, churn sequences
and work counts match the retained cell-cache build. These checks used the executable emitted
by Cargo. An initial script copied a stale executable; those results were discarded. The
candidate was built from the repository root without the Rust directory's x86-64-v2 settings,
so its timings are not a matched performance comparison. No production bound or associated
test remains. The existing detailed trace assigns **53.56 ms** to the combined household-claim
and serial funding phase at midnight; this phase needs finer attribution before another fix.
For standalone checks, build from `rust/` and use Cargo's emitted executable path; do not reuse
a hard-coded dependency hash. The contract runner already follows both rules.

**Deferred household eligibility (2026-10-14).** Test-only timing separates the first household
funding pass into reservation, staffing and acceptance/rejection. Diagnostic binary
`f9e583a205f786060ddb5d7b928f5852d2106316ef376dc6f374f247853535cb` attributes the 30k fiscal
midnight pass to **38.97/0.70/14.44 ms**, respectively. Timer calls run only with diagnostic
logging enabled and are absent from gameplay builds. This motivates deferring availability
tree updates for pending reductions while retaining immediate publication of released receipts.

Candidate SHA-256 `ac57451a679d09e3a3f71eb8c91c49a50d4944c507bb9ae79f1e554682ec00d0`, built
from `rust/` with x86-64-v2 settings, passes **179 targeted tests, 15 ignored** covering
employment, customer transactions and demand journals. Four one-day checks preserve all records,
churn and bounded work counts exactly. With eight workers and affinity `0,2,4,6,8,10,12,14`,
the full cached runner command above completes in **65.36 seconds**. All phase, locality and
agent gates pass; the same **eleven limits** remain open. Baseline hour/day samples are
**17.03/6.88 ms** and **55.38/42.86 ms**; fiscal samples are **28.41/28.01 ms** and
**77.27/91.48 ms**. The preceding targeted baseline 30k day sampled **33.92 ms**, so these
single runs do not establish a stable baseline timing change or a statistical speedup.
All four growth integration tests plus recovery and scarce-labor expansion pass. All five
reports are byte-identical to the retained import-replay reports. The growth executable was
selected from Cargo's JSON output, SHA-256
`e0f567b7633f16d7b029928df86bce657c36040facbe5ca543192eb088473e5f`.
A follow-up 30k fiscal diagnostic takes **7.46 seconds**, with midnight reservation/staffing/
finish at **24.93/0.69/1.73 ms**. Midnight demand remains **53.27 ms**: cell preparation
**12.97 ms**, planning **29.39 ms**, execution **10.91 ms**. These traced samples confirm the
intended reduction in provisional-index work and locate remaining cost; they are not acceptance
timings. The supplier/job-search and repeated census/staffing scaling failures remain unchanged.

**Empty supplier markets (`PERF-05`, 2026-10-14).** Recurring quotes now reuse the existing
resource-availability proof before starting a nearest-supplier search. An empty live/projected
resource skips buyer-component resolution and spatial traversal, then takes the ordinary import
fallback. No counter definition changed. The guard uses existing resource lookups, adds no
allocation or index, and preserves normal searches when supply is available. The regression
checks both empty-market avoidance and recovery after funded production returns.

Binary SHA-256 `0a0a25ad501f802d249cb7aab1d80c825f6c405f62b6617866e7933be7c4b113` passes
**238 targeted tests, 19 ignored**. Four short days preserve records, hourly churn and every
counter except supplier searches: baseline **38,018/196,356 → 20,978/105,798**, fiscal
**2,818/12,533 → 0/0**. Baseline search growth is still **5.0433×**, above **3.4176×**;
route-quote and job-search scaling are unchanged. With the same eight-worker affinity and
full-runner command, the cached contract finishes in **64.50 seconds**. All phase, locality
and agent checks pass. **Ten limits remain:** the original seven baseline limits, both fiscal
daily budgets and fiscal CCH scaling. Hour/day samples are baseline **16.92/6.70 ms** and
**51.58/34.34 ms**, fiscal **27.63/26.58 ms** and **77.77/89.64 ms**. These are routine
samples, not a speedup claim. All four growth integration tests plus recovery and scarce-labor
expansion pass; all five reports are byte-identical to the retained import-replay reports.

**Staffing attribution (`PERF-03`/`PERF-04`, 2026-10-14).** Test-only
`METRUM_TRACE_STAFFING=1`, together with the `economy_phases` debug channel, emits per-operation
profile, import/local mode, physical capacity, closure ceiling and rejection counts. It is
absent from gameplay builds and disabled in acceptance runs. Binary SHA-256
`90e1b170c819d434e5097d576919bfeb84c485d3a6858e46a62fa108a1548e46` passes **54 targeted
funding/employment tests, 9 ignored**. Two serial traced baseline days take **3.08/9.45 seconds**
including loading and retain every record, churn entry and work count. These are diagnostic
durations, not acceptance timings; no full contract or growth reports were rerun for this
test-only instrumentation.

Run the cached-day command below with `METRUM_DEBUG=1 METRUM_DEBUG_FILTER=economy_phases
METRUM_TRACE_STAFFING=1`. Exclude the initial load census (through its first
`household_import_funding_detail` line) when aggregating operation rows: the remaining trial
totals exactly match the contract's **60,333/291,769**. Successful trials alone are
**46,721/208,765**; capital rejects are **3,494/77,526**, profit rejects **10,118/5,478**.
Food processors account for **33,049/192,302** trials, combining import and local paths.
At the current 10k count, the 30k limit is **206,196**: even eliminating every 30k rejected
trial cannot pass it while all successful evaluations remain. Exact worker-range exclusion
may reduce cost, but cannot independently close this gate. Reusing successful evaluations or
reducing repeated funding passes/censuses is therefore necessary; changing counter definitions
is not an optimization. Neither tier rejects any measured trial with the existing free-input
profit bound, which also rules out that weaker bound as a useful next change for this workload.

An exact derived-share import memo guard was tested and removed next. Rather than requiring
identical raw household quotas, it accepted identical capped `provisional_local_share` bits,
using the same arithmetic as revenue, inventory valuation and exports. Existing per-refresh,
activity and ceiling guards remained. Binary SHA-256
`dac14541aa4a7c3a0d1d897b88721b1004d030836c2633e01b032439d0188ff7` passes **128 targeted
funding, employment and logistics tests, 13 ignored**. Four serial cached days preserve records,
churn and **every work count**: this broader guard avoids no additional evaluations. Baseline
10k/30k runs take **2.93/8.94 seconds**, fiscal runs **2.48/7.45 seconds**, including loading.
Hour/day samples are baseline **17.13/6.81 ms**, **51.83/34.31 ms**, fiscal **27.94/27.00 ms**,
**79.15/92.20 ms**; they establish no speedup. The candidate was removed rather than retaining
an extra O(output ports) comparison with no measured benefit. No full contract or growth-report
acceptance is claimed for this trial. The next investigation is repeated funding-pass/census
invalidation, rather than further relaxation of this raw-quota guard.

A fresh 30k baseline input-difference trace on the retained staffing-attribution binary
preserves records, churn and counters. All 15 execution censuses follow hourly censuses with
identical staffing trial/success totals, but only four have unchanged captured building,
household, freight and dependency inputs. Every paired household-budget vector is unchanged;
the other pairs include worker changes or building removal/remapping. Equal aggregate results
therefore do not authorize wholesale census reuse. The next implementation needs explicit
changed-owner/dependency handling, preserving remaps and ordered market reductions.

**Worker-change journal foundation (`PERF-03`, 2026-10-14).** The allocator now owns a
separate workforce revision and reuses `IndexChangeLog` for changed worker-count owners.
Employment, capacity enforcement and agent removal use the allocator setter. Each mutation
is allocation-free O(1); preparation reserves bounded history before staffing. Overflow or
unprepared storage marks history incomplete rather than allocating or silently losing changes.
Building-index changes invalidate the history. The existing parallel recount detects repairs
within its current traversal and invalidates conservatively, without another city scan.
Unchanged counts do not advance the revision.

This is an incremental-census prerequisite, **not a census optimization yet**: funding does
not consume the journal, and its read API is currently unused outside tests (reported by
`cargo check` as dead code). No acceptance limit is closed. Release binary SHA-256
`b41398d0b71a6b75925f5d18336a071f07960a1618227ce1f8d378cf66fa7179` passes **362 targeted
change-log, allocator, household and agent tests, 39 ignored**, including allocation-capacity,
overflow, no-op, index invalidation, unpaid-worker departure and recount regressions.
Four serial cached days preserve every record, churn entry and work count. Baseline durations
are **2.89/9.00 seconds**, fiscal **2.47/7.47 seconds**, including loading; hour/day samples
are baseline **16.85/6.74 ms**, **52.16/35.22 ms**, fiscal **27.85/26.96 ms**,
**79.14/94.13 ms**. These are routine samples, not speedup evidence. Full contracts and the
five growth reports have not been rerun for this intermediate integration stage.

The journal now covers **production inputs**, not only worker counts: allocator-owned
work-area updates, abandonment and construction completion record affected owners.
Agriculture and extraction use the allocator setter, retaining sanitized scales, startup
capital and missing-owner behavior. Road/entrance and catalog revisions remain separate
required cache guards. The module is `allocator/production_changes.rs`; the mutation path
remains allocation-free O(1).

The opt-in funding-input diagnostic now asserts that every changed captured production row
appears in a complete journal when the other dependency revisions match. Release binary
SHA-256 `6e5fff16123a43cc8b9bb53c9fe3eb4ad1c468f3af7f137e903e64561c92457d` passes
**375 targeted tests, 40 ignored**. Four serial cached days again preserve all records, churn
and work counts, taking baseline **2.91/8.93 seconds**, fiscal **2.48/7.45 seconds**.
Hour/day samples are baseline **17.00/7.38 ms**, **51.77/34.87 ms**, fiscal **27.93/26.34 ms**,
**79.17/91.11 ms**. A separate **9.72-second** 30k diagnostic checks all 15 execution pairs:
nine have complete history (four empty, five with one or two changed owners); six correctly
require full rebuilding after layout changes. The assertion passes and outcomes/counters remain
unchanged. These checks establish journal coverage for this workload, not incremental funding
acceptance. Full funding integration, full contracts and growth-report validation remain pending.

**Production-market chunk reuse (`PERF-03`, 2026-10-14).** Funding now consumes the production
journal to rebuild only affected 256-building market chunks. Catalog identity, building layout,
registry, entrance and topology guards force conservative rebuilding. Existing chunk arithmetic
and ordered final reduction are unchanged; closure-withdrawn buyer flags are restored before
changed chunks overwrite membership. With K logged changes, D dirty chunks and R resources,
preparation costs O(K log K + 256D + BR/256), plus original buyer-list restoration; other census
passes still scan all buildings. Census counter definitions are unchanged.

Release binary SHA-256 `e9d7bf1bb57dfe205062694d1712ae038cabf55dc34e8ab65e5e657c0287131c`
passes **379 targeted tests, 40 ignored**, including exact full-census comparisons after worker,
area, catalog and history changes. Four serial cached days with eight Rayon workers and affinity
`0,2,4,6,8,10,12,14` preserve every record, churn entry and work count. Baseline runs take
**2.88/8.87 seconds**, fiscal runs **2.44/7.43 seconds**, including loading. Hour/day samples
are baseline **16.90/6.85 ms**, **51.92/35.49 ms**, fiscal **28.17/27.75 ms**,
**79.06/95.36 ms**. These establish no speedup and close no gate. A subsequent full cached
contract on the same binary finishes in **63.95 seconds**, with all 14 isolated phases,
both locality checks and all six agent samples passing. The same **ten limits fail**:
baseline 30k daily time and six work-scaling limits, both fiscal daily budgets and fiscal
CCH scaling. Baseline hour/day samples are **16.97/7.02 ms**, **51.67/34.22 ms**; fiscal
samples are **28.00/27.41 ms**, **78.84/95.48 ms**. No snapshots regenerate. All four growth
integration tests plus recovery and scarce-labor expansion subsequently pass, with all five
reports byte-identical to the retained import-replay reference. The integration executable is
SHA-256 `ce7d74b7a6e016b2328f1ac2499e5b6511d9bc253c2e42b24a7675d50df78bf8`; recovery and
expansion use the retained chunk-cache library binary above. These serial correctness runs
take **96.62, 43.16 and 55.77 seconds**, respectively, with the same eight-worker affinity;
they are not performance acceptance measurements. Full funding still needs exact
dependency-aware reuse.

A buyer-specific supplier-incumbent trial was tested and removed next (`PERF-05`). It kept
one previous placed supplier per buyer, repriced it as an upper bound, and retained the exact
candidate and tie checks. Binary SHA-256
`88bc05095e09f5ac41e30473a9ad6d57845dd905c6de4aba7af8ba740f2cabba` passes **129 targeted
tests, 13 ignored**. Four serial cached days use the same build settings, eight workers and
affinity as the chunk trial, preserving all records and churn. Baseline supplier routes fall
**433,189/2,023,720 → 407,313/1,903,633**, but the **4.6736×** ratio still exceeds **3.4176×**.
Point CCH searches fall **85,568/288,059 → 85,286/286,731**; fiscal counts are unchanged.
Baseline runs take **2.86/8.82 seconds**, fiscal runs **2.46/7.37 seconds**, including loading.
Hour/day samples are baseline **17.25/6.80 ms**, **51.92/33.86 ms**, fiscal **28.15/27.67 ms**,
**78.67/93.80 ms**. These show no clear timing gain. The trial was removed rather than adding
O(buildings) storage and pass-rollback integration for essentially unchanged scaling. No full
contract or growth-report acceptance is claimed for this exploratory build.

Inspection also rules out simply skipping employed workers' dominated job searches: those
searches publish historical complete commute quotes retained across traffic-price revisions.
Suppressing publication can change later job choices even when the current choice cannot improve.
Any reuse must preserve that publication behavior as well as current choices and lock timing.

**Import-plan eligibility diagnostic (2026-10-14).** Test-only
`METRUM_TRACE_IMPORT_PLANS=1` with the `economy_phases` channel records exact import-plan
cost/sales inputs, capital, desired/accepted workers and required capital. It does not authorize
production reuse. Binary SHA-256
`d6a52d243c056591b0748a83cb7d6dd85e17171500b8bac17255a8f47d08a7c9` passes **59 targeted
tests, nine ignored**. Serial traced baseline days take **3.10/9.52 seconds**, retaining
records, churn and every counter. These are diagnostic durations, not acceptance timings.

For unchanged cost/sales inputs, a previously successful maximal plan remains viable when
current capital still covers its exact stored requirement; capital surplus must be recomputed.
Offline comparisons confirm exact accepted workers and capital requirements for every observed
match. An optimistic shared history keyed by parcel finds **7,340/50,940** eligible reuses,
including **3,494/10,648** with changed capital. It ignores layout invalidation and separate
live/execution ownership, so it is not a production hit-rate claim. Clearing history when the
building count changes reduces this to **6,218/27,704**; additionally separating hourly live
and non-boundary execution censuses reduces it to **2,641/7,140**. Even the optimistic case
leaves staffing-trial scaling at **4.5445×**, above **3.4176×**. A broader capital guard alone
cannot close the gate; further work needs exact sharing/incremental handling across census
instances and their owner remaps, rather than another isolated per-instance memo.

**Exact import-plan handoff (`PERF-04`, 2026-10-14).** Construction censuses now borrow
successful maximal import-only plans from the live employment census. Reuse requires the same
catalog identity, profile, physical capacity, area, all prices, export mode, demand-limited
activity, desired workers and exact output-share bits. Current cash, inventory and paid cargo
are valued again; they must cover the stored exact f64 working-capital requirement. Profit
headroom is unchanged and capital headroom is recomputed. Customer and supplier reservations
remain owned and rebuilt by each census. Partial owner remaps retain sparse share offsets;
replacement catalogs invalidate proofs. No submaximal or failed operation is reused.

Storage is O(B + output ports), retained across refreshes. Preparation adds O(B + sum of
per-profile output-port counts squared), using existing share lookup arithmetic; proof lookup
is O(output ports), without allocations inside quotation. Remapping costs O(mapping + new
slots). Independent import quotation remains parallel. This avoids actual staffing evaluations,
without changing work-counter definitions or weakening acceptance limits.

Release binary SHA-256 `72a1ae82551b32b59bc3e506c256b41abd22919e741846014ec346e0f0545aa6`
passes **131 targeted tests, 12 ignored**, including comparison with cold funding after capital,
customer income, price, area and catalog changes, plus remap and capital-threshold tests.
Four serial cached days with eight workers and affinity `0,2,4,6,8,10,12,14` preserve records,
churn and every count except reduced baseline staffing trials: **60,333/291,769 →
56,488/251,537**. The **4.4529×** ratio still exceeds **3.4176×**. Durations including loading
are baseline **2.87/8.88 seconds**, fiscal **2.47/7.35 seconds**. Hour/day samples are baseline
**17.16/6.73 ms**, **52.05/35.54 ms**, fiscal **27.95/27.04 ms**, **78.12/92.98 ms**.
These are routine samples, not evidence of a timing speedup. The full cached contract on the
same binary takes **64.15 seconds**, without snapshot generation: all 14 isolated phases,
both locality checks and all six agent samples pass. The same **ten limits remain open**:
baseline 30k daily time and census/owner/staffing/supplier-search/supplier-route/job-key scaling,
both fiscal daily budgets, and fiscal CCH scaling. Full-run hour/day samples are baseline
**17.11/6.90 ms**, **52.32/34.53 ms**, fiscal **27.81/27.22 ms**, **78.36/93.33 ms**.
All four growth integration tests, recovery and scarce-labor expansion now pass; all five
reports are byte-identical to the retained import-replay reference. Integration executable
SHA-256 `367f5aa55b4f2c7d4d1005cfd62d1c560db17ea0a7b32494776eea5f2e18417c` takes
**96.58 seconds**; recovery and expansion use the handoff library binary and take
**43.64/56.58 seconds**, serially with the same worker/affinity settings. These are correctness
checks, not speedup measurements. Reproduce the contract with the retained `$TEST_BINARY`:
`RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 python3 tools/perf_contract.py --binary "$TEST_BINARY" --artifacts "$ARTIFACTS"`.


The preceding exact staffing trace separates import/local trials into **37,430/22,903** at
10k and **139,446/152,323** at 30k. Subtracting the freshly measured handoff savings implies
current import counts **33,585/99,214** (2.9541×) and unchanged local counts
**22,903/152,323** (6.6508×). This is attribution inferred from historical component counts,
not a freshly instrumented split. Local trials must still reserve inputs and revalue inventory
at their actual supplier prices; reusing only sales arithmetic does not remove that work.

A fresh opt-in 30k fiscal diagnostic on the handoff binary takes **7.98 seconds**, preserving
the record, churn and every work count. At the daily boundary, household import reservations
take **25.94 ms**, staffing **4.84 ms**, and claim completion **1.82 ms**. These diagnostic
times locate work and are not acceptance timings. None of the **101,067** logged import
plans (including load) has import-valued capital below property tax alone, so that simple
necessary affordability bound would skip no observed plan. A broader inventory-aware bound
is not established. Remaining fiscal diagnosis should focus on costly customer reservations
for rejected plans while preserving exact customer ordering and routing behavior.

**Rejected household-query attribution (2026-10-14).** Test-only
`METRUM_TRACE_CONSUMER_QUOTES=1` plus the `economy_phases` channel records exact request,
price, routing chunk, mode components, walking groups and endpoint-node keys. It counts
visited candidates and route fallbacks. Account refresh, release, acceptance, reset and
transaction changes emit conservative invalidation markers. Ordinary rejection restores
accounts; only a single traced query with no pre-existing staged claims or released owner
is eligible for comparison. Queries requiring any route fallback are excluded.

Release binary SHA-256 `0c698a22b09314befb23007900c631abd94d52dbb60428a6855a8bc875d2ae30`
passes **78 targeted tests, nine ignored**. Two serial fiscal diagnostic days with eight Rayon
workers and affinity `0,2,4,6,8,10,12,14` take **2.54/7.66 seconds**, including loading,
and preserve both records, churn and every work count. After excluding load, 10k/30k query
counts are **7,312/20,554**, visiting **2,440,131/8,452,018** candidates. Rejected exact-key
matches between invalidations account for **2,551/6,609** queries and
**956,010/3,183,275** candidates. Every match returns identical f64 quantity bits and uses
no route fallback. These are measured reuse opportunities, not an implemented cache or speedup.

At the daily boundary specifically, matches account for **233,327 of 282,811** candidates
at 10k (**82.50%**) and **836,082 of 993,896** at 30k (**84.12%**). The next implementation
can extend the existing lazy household quotation: reuse a rejected single-output scalar
quote only under this exact key and unchanged account revision, then perform the ordinary
reservation if staffing subsequently accepts. It must invalidate on all surviving account
changes and preserve the existing owner-prefix replay, rollback and customer order. Quotation
storage must be prepared outside the claim loop. These diagnostic changes add no gameplay
cache and close no acceptance gate.

**Exact rejected household-quote reuse (2026-10-14).** The census now retains rejected
single-output quantities under the exact geometry/request key and customer-book revision.
Only queries using direct walking proofs qualify. Surviving account mutations invalidate
the result; ordinary rejection restores accounts exactly. A later acceptance repeats ordinary
quotation to materialize receipts before committing. The table is cleared/preallocated outside
quotation, with O(consumer firms) retained storage and expected O(1) lookup; hash collisions
still compare the complete key. Existing ordered-prefix replay is preserved.

Release library SHA-256 `e3fdfadb75f18f0631a199ad91d34ef5aafb8af5e43f4397850bb697e11dbbe3`
passes **147 targeted tests, 12 ignored**, including cached-versus-ordinary funding and
receipt equivalence when a later solvent owner accepts a previously rejected query. Four
short cached days preserve records, churn and every existing work count. With eight Rayon
workers pinned to `0,2,4,6,8,10,12,14`, the full cached contract takes **64.20 seconds**,
without snapshot generation. All 14 isolated phases, two locality checks and six agent samples
pass. Baseline hour/day samples are **17.17/6.99 ms**, **52.05/34.90 ms**; fiscal samples
are **27.33/22.21 ms**, **76.16/72.67 ms**. The same ten timing/scaling limits remain open.
These are single-run regression signals, not matched speedup evidence. Reproduce with the
contract command above and this retained `$TEST_BINARY`. Fresh growth validation passes all
four integration tests, recovery and scarce-labor expansion; all five reports are byte-identical
to the retained import-replay reference. Integration executable SHA-256
`8638a5e8aea664bd82df76db4966c124bbf4733ba494780a7bd1d649911727a8` takes **102.09 seconds**;
recovery and expansion take **46.17/59.42 seconds** on the library executable, serially with
the same worker/affinity settings. These runs establish correctness, not a timing improvement.

**Contract run (`PERF-01`).** `./run.sh --perf-contract` loads both tiers of both workloads,
simulates one day each (23 operational hours and one daily settlement) and reports per-phase
work counts (`debug::work`) and timings. One release run per tier and workload, no repeats;
at most **5 minutes** in total, including loading. Baseline and fiscal generation are reported
separately outside that runtime budget. Isolated `--phase` diagnosis prepares only baseline
snapshots. The full contract is required before merging an economy change.

**Limits.**
1. **10k time:** at most **40 ms** per mean operational hour and **10 ms** per mean daily
   settlement.
2. **30k time:** at most **120 ms** per mean hour and **30 ms** per mean day: 3× the 10k
   *budget*, never 3× the measured 10k time, so a faster 10k city cannot tighten it.
3. **Work scaling:** from 10k to 30k each bounded count grows at most **1.2× the final
   household ratio**. Bounded: census building rows, reservation rows copied, owner slots
   prepared, proposed site rows generated, spawn parcels scanned, staffing trials, supplier
   searches, supplier route quotes, household job-search keys, and CCH searches (point searches
   plus bucket sources, which replace them). Pass, census and rebuild event counts are reported
   but not bounded.
4. **Locality checks:** a profitability-failing supply cycle resolves in at most **3** funding
   censuses at lengths 4/16/64 (`benchmark_combined_financing_cascades`), and a warm queued
   check after a frontage-price change costs at most **1.2×** as much at 131,072 background
   buildings as at 16,384 (`benchmark_queued_project_validation`).

The five growth reports stay the correctness check: byte-identical for performance-only
changes, reviewed and re-baselined for intentional behaviour changes. A single timing sample
is informative, not a significance claim. A failed limit is investigated from the phase counts
first. Matched alternating timings ([recipe](#econ-13-economy-tick-baseline)) are reserved for
milestones and for changes that claim a speedup.

**Per-phase checks (`PERF-02`).** Isolated checks run one phase on a loaded snapshot with frozen
inputs: the funding census (staffing, household claims), hourly planning and site preparation,
the execution epoch, the logistics hour, lot regeneration, daily settlement and route queries.
A path map selects the checks a diff can affect. Shared code (customer and business books,
allocator, save/load, the contract itself) runs every check. An isolated check cannot see a
change elsewhere that feeds its phase more work; the one-day run covers that.

**Known scaling work.** The 2026-10-12 100k counts found three superlinear groups, to be
re-measured at 30k: the execution epoch's own hourly census (`PERF-03`), the staffing search
(`PERF-04`) and supplier and job-search fan-out (`PERF-05`).

**Implementation in progress (2026-10-13).** `./run.sh --perf-contract` now dispatches an
engine-independent release-library-test runner. Snapshot continuation and end-to-end acceptance
are still being validated; no current-build performance acceptance is claimed. The last historical
10k measurement (ECON-13 recipe, 2026-10-12) was 8.17 ms per hour and 8.31 ms per day.

Snapshot validation exposed incomplete bulk fixture setup: its connector heights had not been
fitted to the compiled road surface, so ordinary loading changed connector lengths and arrival
times. Setup now compiles the final border edits and synchronizes lane heights before agents
run, using the existing road geometry path. This is one-time O(lane geometry vertices) setup,
outside the contract timing. The 10k continuation passes. The 30k probe now isolates the first
divergence to workplace reassignment at 04:00: complete commute quotes retained by the live city
are absent after loading (one retained quote is 220 seconds versus 40 seconds when recomputed).
Their documented daily expiry preserves a historical frontage price, so these quotes must survive
save/load; repricing them more often would change simulation behavior. Save version 86 now
retains valid complete quotes in a sorted compact blob (20 bytes per quote). Saving costs
O(Q log Q + B), loading O(Q + B), including the B-building entrance dependency guard,
outside agent and economy tick loops. The previously failing 30k
day-7 continuation now passes, with generation and diagnostics taking 73.07 seconds; the
134,932 retained quotes occupy 2,698,652 bytes. Fresh 10k and 30k counts show no execution
census on day 7. Days 12–13 exercise the missing work. That boundary also exposed a loader
repair that discarded a legitimate empty-route agent's retry timer and retargeted it home.
Loading now preserves pending network retries; legacy orphans without retry state retain their
repair path. Both tiers pass next-day continuation at the day-12 boundary (generation and
validation: 29.17 seconds at 10k, 80.63 seconds at 30k). This window exercises four execution
censuses at 10k and fifteen at 30k; it does not establish the originally assumed every-hour
building churn. Fixture coverage and full performance acceptance remain open.

Fresh cached diagnosis (2026-10-14), release binary SHA-256
`f95ee313f5df20b80d68968576e6598b07ae103b3836d4f045ef40416ccc0c52`, eight Rayon workers,
inherited affinity, no concurrent build or measurement:
`python3 tools/perf_contract.py --binary "$TEST_BINARY"` (the runner defaults to eight workers).
Both next-day runs and all fourteen isolated phases completed; the run stopped at the queued
locality gate after 55.86 seconds including loading. The 10k hour/day measured 26.63/7.07 ms;
30k measured 76.36/37.26 ms, failing the 30 ms daily limit. Snapshot loads took 1.47/4.07 seconds.
Against a 3.4176× work limit (15,180/5,330 final households), counts were:

| Bounded work | 10k | 30k | Result |
|---|---:|---:|---|
| Census building rows | 214,476 | 814,404 | fail |
| Owner slots prepared | 621,256 | 2,382,279 | fail |
| Staffing trials | 72,192 | 396,486 | fail |
| Supplier searches | 38,018 | 196,356 | fail |
| Supplier route quotes | 433,189 | 2,023,720 | fail |
| Job-search keys | 659 | 7,759 | fail |

Copied reservation rows remain zero; proposed-site rows, spawn scans and combined CCH searches
meet their scaling limits. Queued locality measured 0.957/2.363 ms per 1,024-check batch, failing
1.2×; the short batch's timing stability needs investigation. Financing locality and the agent
contract were not reached in this run. These are failure diagnostics, not acceptance or a
speedup claim. The runner now retains completed day-limit failures if a later check aborts,
and continues independent locality and agent gates after a gate failure while time remains.

The subsequent complete unprofiled run uses the documented physical-core affinity:
`RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 python3 tools/perf_contract.py --binary "$TEST_BINARY"`,
binary SHA-256 `06e8aa0a4a2899a89cff7ad78bcfea5a6c2eb32ee2bdabbd3f2c33449347074c`.
All checks execute in **55.00 seconds**, including loading. Hour/day times are 22.81/6.70 ms
at 10k and 69.04/33.66 ms at 30k: the 30k daily limit still fails. The six failed work counts
above are unchanged. Queued locality uses one warmed 131,072-check batch per tier and passes
at 124.53/125.15 ms (1.005×); financing cycles use two censuses at all three lengths. The
agent contract also passes. An earlier short profile found substantial Rayon scheduling
overhead under mixed performance/efficiency-core affinity; its timings are not acceptance.

Isolated lot and settlement checks now prepare load-invalidated derived lots outside their
phase timer and report `setup_s` separately; that setup remains inside the overall deadline.
Local lot regeneration measures 15.32/14.45 ms, instead of accidentally timing whole-map
reconstruction. All fourteen isolated phase checks and thirteen runner regressions pass,
completing `PERF-02`. Save, queued post-load execution and bulk lane geometry regressions
pass together (35 tests, three ignored). Full growth-report comparison remains pending.
Standard `cargo clippy --all-targets` still fails on the pre-existing `mut_from_ref` error in
`agents/tick/slices.rs`; allowing only that lint completes the check with existing warnings.

`PERF-01` is still incomplete because of the failed limits and fixture coverage. Earlier runs
also exposed nondeterministic CCH work. All non-CCH economy counts matched; point searches differed by
18 at 10k and 1,015 at 30k, and bucket-source searches by 1 and 4. Parallel shared-leg cache
misses can compute the same quote twice, and two-slot eviction order depends on scheduling.
The counters correctly exposed actual searches. The ownership change below addresses this;
changing counter attribution would have concealed the problem.

**Deterministic routing work (2026-10-14).** Workplace searches now assign sorted key slices to
retained query buffers rather than Rayon threads. Each parallel phase reads a frozen shared
network-leg cache; ordered publication visits only changed local slots after the phase.
Agent movement also assigns query scratch to interleaved agent shards, one scratch each, with
separate storage for serial movement. Scoped TLS swaps preserve existing planner entry points and
restore the caller's scratch on return or unwind. Household reset discards workplace scratch
and shared quotes. Scratch ownership and deferred publication add no per-query allocations;
preparation and retained storage are bounded
by worker count, graph size, target count and current/high-water job-key counts. Warm ordered
publication is O(changed quote slots), without a whole-network scan.

Release binary SHA-256 `c7ca3b0bf9514d00a0e89d5c7f02a16b47fc0489da9536418073c08de8a2d66a`
passes 125 targeted movement, employment and save regressions (20 manual checks ignored).
With eight workers pinned to `0,2,4,6,8,10,12,14`, all economy work counts match across three
fresh processes per tier, including the complete contract. Point/bucket counts are
85,568/220 at 10k and 288,059/2,951 at 30k; combined routing work stays within the scaling
bound. Both next-day records match the retained pre-change binary
`51bfd66e9e59a7c537a431255106158d6704d76e8182681dfce1696543a7459d`.
The full pinned command above completes in **55.20 seconds**, with hour/day times
22.96/6.76 ms and 69.25/34.43 ms. Locality and agent gates pass; the same seven economy
limits still fail. These samples establish reproducible work, not a timing speedup.
Fresh-process peak RSS is 493,232/1,051,036 KiB versus 483,620/1,022,664 KiB before the
change (`os.wait4`, ordinary one-day contract, same workers/affinity). These are whole-process
peaks including load, not exact cache sizes. An earlier measurement inherited the executable
hashing parent's high-water RSS and is excluded. All five growth reports are byte-identical
to the retained pre-change executable. The four growth integration tests and four selected
balance-case tests pass, including busy-city and queued-construction save continuation.
The all-targets Clippy check completes with the previously documented `mut_from_ref` lint
disabled; existing warnings remain. Targeted regression command:
`RAYON_NUM_THREADS=8 "$TEST_BINARY" simulation::economy::agents::tick:: simulation::economy::households::employment::tests:: simulation::economy::households::tests::employment:: simulation::save:: --test-threads=1`.

**Short staffing diagnosis (2026-10-14).** The ignored phase runner now initializes the
existing debug channels, so `METRUM_DEBUG=1 METRUM_DEBUG_FILTER=economy_phases` exposes
funding-search detail without a long city-generation benchmark. A 30k census trace takes
4.0 seconds including loading and reports 8,666 import trials, 1,518 capital rejections and
1,700 profit rejections, plus 4,468 local trials. Trace timing is diagnostic only.
An exact import-payroll affordability cap was tested and removed: the complete pinned
contract finished in 54.85 seconds, but daily staffing trials only changed from
72,192/396,486 to 72,192/396,482 at 10k/30k. Both next-day records matched, five targeted
hiring regressions passed, and the same seven contract limits failed. This does not justify
the extra search logic or establish a speedup. The rejected executable has SHA-256
`b35181e143bc54eac21d203ef95f4b56008cb25cae82de6877a8f0c7017861e8`; its report is retained
separately from the restored implementation. The run used the same eight-worker pinned
command above. The pre-trial executable (`51bfd66e9e59a7c537a431255106158d6704d76e8182681dfce1696543a7459d`)
also passed all four growth integration tests and both recovery/expansion tests, producing
fresh reference copies of all five reports for subsequent production changes. Repeated
execution censuses remain the next scaling target; the existing structural revision keys
do not cover changes to staffing, budgets, stock or household funding inputs, so reusing
their funding books solely on those keys would be incorrect.

Inspection of the owned-routing build's cached 30k day trace narrows that reuse target:
all 15 execution censuses occur in distinct hours, one minute after the live census.
Splitting queue-replay invalidation alone therefore removes none of this workload's censuses.
An opt-in test-only input trace (deleted in `SIM-02-I` part 3, with the execution censuses)
compared selected building/household scalar bits, exact
ordered consumer budgets, freight capital and structural/routing revisions between the live
census and execution. It allocates diagnostic rows and is excluded from gameplay; matching
rows locate reuse candidates but are not a complete production validity proof.
With `METRUM_TRACE_FUNDING_INPUTS=1 METRUM_DEBUG=1 METRUM_DEBUG_FILTER=economy_phases`,
the cached `perf_contract_day` test completes in 3.15/10.06 seconds at 10k/30k including loading
(eight workers, the pinned affinity above). Binary SHA-256:
`d4bfa8e9cd2fd96048c2bdc0fbf8f399f384faf6c34f4a2b3aef323f3fa83c3c`.
Both next-day records and all work counts match the owned-routing baseline; 92 targeted
hiring, freight-timing, employment and save tests pass (12 manual tests ignored).
These instrumented durations are diagnostic turnaround, not performance acceptance.

The trace finds unchanged selected inputs at minutes 121, 541, 901 and 1141 at 30k, but
also at minute 541 at 10k. Treasury balances change while actual consumer budgets remain
bit-identical. The other execution boundaries change staffing or buildings. Reusing only
these candidates would leave 207,081/732,964 census rows, a **3.5395×** ratio against the
**3.4176×** limit: it cannot close `PERF-03`. With one census removed at 10k, at least six
would need eliminating at 30k. Further work must cover changed inputs incrementally or
avoid more full censuses, while retaining exact books. Any live-book loan must also restore
the books before household shopping/service visits consume them. No runtime behavior or
acceptance limits changed; the new trace prevents treating the four-case shortcut as sufficient.

An exact per-buyer nearest-supplier memo was also tested and removed. It reused a positive
winner only while resource eligibility had not grown, using sparse authored input slots and
route/registry guards. The full pinned contract took 55.85 seconds and retained all seven
failures. Supplier searches changed from 38,018/196,356 to 38,010/195,196, and route quotes
from 433,189/2,023,720 to 433,030/2,011,929. This sub-1% reduction does not justify retaining
another cache. Both next-day records matched and 117 targeted logistics/hiring/employment
tests passed (12 manual checks ignored). The rejected binary was
`66a0b62e2aa7f14be3cf4d1fc8d655af3b6f464752e2eb35ee20a9ea85f6b22c`; its patch and report
are retained separately. This is diagnostic rejection evidence, not a speedup or current-source
acceptance result. The existing per-operation offer replay remains unchanged.

A one-worker, freight-free import capital bound was tested and removed as well. After the
first capital rejection it could prove that no positive staffing plan was affordable, counting
the extra bound evaluation as a staffing trial. At 10k trials fell from 72,192 to 71,046;
at 30k they rose from 396,486 to 396,844, so the shortcut worsened scaling. Both next-day
records matched and all other work counts were unchanged. The 118 targeted hiring, logistics
and employment tests passed (12 manual checks ignored). Two serial cached day tests took
2.98/9.13 seconds including loading, with eight Rayon workers pinned to
`0,2,4,6,8,10,12,14`; hour/day samples were 22.97/7.33 ms and 69.43/34.91 ms. These are
diagnostic samples, not full-contract acceptance or a speedup claim. Command:
`METRUM_PERF_RESIDENTS=<10000|30000> METRUM_PERF_CACHE="$ARTIFACTS/snapshots" RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" nodes::sim::core::populated_city::contract::perf_contract_day --exact --ignored --nocapture --test-threads=1`.
Trial binary SHA-256: `89c77c33a7e9849d5fc25de1fe438c9626df1e678aafc997402746fca2bdc839`.
Its patch and results are retained separately; no runtime shortcut remains.

Import-only funding now reuses exact results across closure retries within one census.
Building assets, prices and aggregate market inputs are immutable during that census;
reuse additionally requires bit-identical activity and household quotas, a nonincreasing
worker ceiling and a cached result that still fits it. Capacity, activity and financial
headroom return together. Household claim reserve/finish and all local-input reservations
still execute normally. Storage is O(buildings + output ports), prepared outside quotation;
checks are O(1) for nonconsumer firms and O(output ports) for household sellers. Every new
census clears validity, so changing income, prices or capital cannot reuse an old result.

Fresh serial cached day tests, using the command and eight-worker affinity above, took
2.99/9.13 seconds including loading. Staffing trials fell from 72,192/396,486 to
60,333/291,769; staffing operations fell from 65,026/298,794 to 54,809/214,075. All other work
counts and both next-day records matched. The **4.8360×** staffing ratio still exceeds
**3.4176×**, so `PERF-04` remains open. Hour/day samples were 23.11/7.04 ms and
69.55/34.90 ms; these single samples establish no timing gain. Binary SHA-256:
`cf251d64ca4e3f8f2b281f81e0b6dcea324750a0960d5f7aacc396061a9ad5fa`.
All 118 targeted hiring, logistics and employment tests pass (12 manual checks ignored),
including cached/uncached funding comparisons after income, import-price and capital changes.
All five growth reports are byte-identical to the retained owned-routing baseline. Four
growth integration tests pass in 96.75 seconds; recovery, expansion, queued save/load and
busy-city save/load checks pass in 116.76 seconds. Those are serial correctness runs, not
performance comparisons. The integration binary SHA-256 is
`b38076aa02ed9cca0d76f707556c593044bc9333478eab4981899ef912145457`.
The complete pinned contract (`RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 python3 tools/perf_contract.py --binary "$TEST_BINARY"`)
finishes in 56.6 seconds. Locality and short agent checks pass; seven limits remain failed:
30k daily settlement (35.85 ms), census rows, owner slots, staffing trials, supplier searches,
supplier routes and job-search keys. Counts match the short day diagnostics above. This
change is partial progress on `PERF-04`, not full performance acceptance.

Local-input funding now skips an unused import-priced inventory valuation; its existing
supplier-priced valuation still runs after each quote. This removes one O(I + O×I)
recipe-port traversal per local operation, with no reservation or decision changes.
Release binary SHA-256 `14a18ab71409a69605fe8ecedad7e917c733446b441032babb2bb3c1a7b72638`
passes 122 hiring, logistics, employment and operation tests (12 ignored). Using the serial
cached-day command above, 10k/30k checks took 3.02/9.27 seconds including loading and preserved
both next-day records, hourly churn and every work count against the revision-guard build.
Hour/day samples are 22.84/6.81 ms and 69.09/34.57 ms. These samples establish no speedup;
the 30k daily budget and all previous work/coverage failures remain. The five growth reports
and full contract have not been rerun for this change.

An additional supplier network-bound check was tested and removed (`PERF-05`, 2026-10-14).
It used the existing metric-scoped entrance-endpoint lower bound before exact route quotes,
with strict rejection to retain ties. Binary SHA-256
`7f61b0ba5ab5091b592c16a304f4e001af79941e6dfcbeec806683b924f6a838`
passes 120 targeted hiring, logistics, employment and CCH-bound tests (12 ignored).
Serial cached 10k/30k day checks took 3.03/9.31 seconds with the command above, preserving
both records, churn and every work count: the bound rejected no additional route quotes.
Hour/day samples were 23.10/6.84 ms and 70.20/35.28 ms. The check added work without pruning
this workload, so it is not retained; `cargo check --tests` passes after restoration.
This also agrees with the earlier rejected L1 supplier-bound experiment below.

A complete entrance-ETA bound was also tested and removed (2026-10-14). It reused
`FreightEntranceAccess::eta_to`, substituting coordinate potentials for network queries while
retaining driveway/frontage arithmetic and strict incumbent rejection. Release binary SHA-256
`8b6eed2d27714dc5f1e735c3675e319af819f3be093cdb06d6a4d4cdc490edc8` passes **127 targeted
logistics, employment and CCH-bound tests, 13 ignored**. The same pinned, serial cached-day
command above takes **3.20/10.33 seconds**, including loading, and preserves both records and
hourly churn. Supplier quotes fall **433,189/2,023,720 → 64,673/374,717**, but their scaling
worsens to **5.7940×**, still above **3.4176×**. CCH point searches fall only
**85,568/288,059 → 85,143/285,609**; all other work counts remain unchanged. Hour/day samples
are **28.14/13.02 ms** and **87.58/76.03 ms**. These single samples are not matched speedup
evidence; the extra per-candidate entrance calculations do not justify retaining the trial.
Review also found that missing network proofs must disable pruning rather than assume zero
for arbitrary negative metrics. No candidate code is retained and no full-contract or growth
report acceptance is claimed for it. Further work should reduce repeated searches/staffing
evaluations or candidate traversal, rather than price the same candidate set twice.

An exact whole-result home-job cache was tested and removed (`PERF-05`, 2026-10-14).
It required unchanged ordered open-job supply and geometry/route revisions, and expired
with the existing complete commute cache. Full 96-quote publication batches were ineligible:
skipping the next search could otherwise suppress historical prices published on that search.
All 55 targeted employment tests passed (nine manual checks ignored). One serial release
day per tier, with the same eight workers and physical-core affinity, took **3.03/9.22 seconds**
including loading, with unchanged records and all work counts: job-search keys remained
**659/7,759**. Binary SHA-256:
`321fbf5afab7d6d734a03acc0a15dd1c2e55a56d9bf91d3592d0698be0d1fab7`.
A deliberately optimistic diagnostic also ignored metric/frontage revisions when no
over-budget estimate occurred and complete quotes were retained. It still saved **zero**
searches (2.99/9.21 seconds). That second trial was not correctness-eligible: ring pruning
also depends on the home's live departure price. Both trials were removed; a fresh
`cargo check --lib` passes after restoration. Retaining this O(open jobs) comparison and
O(searched homes) result storage would add cost without reducing the measured work.

A cross-census extension was subsequently tested and removed. It compared exact recipe
fields, all operation prices, physical capacity, area, valued capital and per-output local
sales shares, clearing on recipe/layout changes. With 119 targeted tests passing and both
next-day records unchanged, it reduced 10k trials from 60,333 to 56,932 but left 30k at
291,769. All other bounded counts were unchanged. The extra guards and wider memo therefore
did not address the failing tier; only the within-census replay above remains. The two serial
cached runs took 2.97/9.19 seconds including loading, using the same command, eight workers
and affinity. Hour/day samples were 23.01/6.98 ms and 69.39/35.71 ms. Trial binary SHA-256:
`3b2a77e650323657bb405a3d790bca69d2560b640fdfc874d224f40562d288b1`.
This is rejection evidence, not acceptance or a speedup claim; its patch and results are
retained separately. Cross-census input changes require more than retaining the last quote.

The fresh 30k diagnosis used eight Rayon workers and inherited CPU affinity, with release binary
SHA-256 `2b6afd9c980530c8a4ae50611798e523a8736419ab8200298c5b0c3f1d6df6a9`:
`RAYON_NUM_THREADS=8 METRUM_CCH_BENCH_RESIDENTS=30000 METRUM_ECON_WORK_DAYS=15 METRUM_SKIP_PASS_VERIFY=1 "$TEST_BINARY" nodes::sim::core::populated_city::routing_benchmark::benchmark_populated_work_counts --exact --ignored --nocapture --test-threads=1`.
It completed in 95.86 seconds including generation. The day ending at day 13 visited 814,404
census rows, ran 15 execution censuses and 396,486 staffing trials, issued 196,356 supplier
searches and 2,023,718 supplier route quotes, and searched 7,759 household job keys. This is
diagnostic evidence for the active workload, not acceptance of the cached contract or a speedup.

Runner commands (from the repository root):

```bash
./run.sh --perf-contract --plan                 # inspect selection without building
./run.sh --perf-contract --prepare              # generate and validate snapshots separately
./run.sh --perf-contract --keep-baseline        # keep this release test binary and identity
./run.sh --perf-contract --diff HEAD            # affected isolated phases plus full next day
./run.sh --perf-contract --phase census         # diagnosis only, not merge acceptance
./run.sh --perf-contract --agents               # short traffic contract only
```

`--artifacts "$ARTIFACTS"` selects local logs, JSON reports, snapshots and retained executables;
`--binary "$TEST_BINARY"` reuses an already built release test executable. Compilation and first
fixture generation are reported separately. Cached verification has a 300-second deadline,
including loading, both one-day runs, selected isolated phases and locality checks. Eight Rayon
workers are the default; `RAYON_NUM_THREADS` and inherited CPU affinity are recorded with the
binary hash and source revision. Test-only full-book fingerprints are disabled in timing runs;
ordinary correctness tests retain them. Counters and inclusive phase attribution compile out of
gameplay builds. Nested phase totals overlap and must not be summed.

Snapshots are published only after next-day continuation matches the straight-through fixture.
Save-version changes, fixture source changes and bootstrap manifest changes invalidate them.
Simulation changes outside those fixture inputs that intentionally alter fixture generation must
bump its explicit algorithm revision. Unknown or shared changed paths select every phase;
documentation-only diffs skip runtime checks. Isolated phase runs always load their own snapshot,
so selecting a different preceding phase cannot alter their inputs.

### `ECON-13` economy tick baseline

Closed 2026-10-12 and replaced by the [performance contract](#performance-contract-perf-01perf-05).
The records below are history.


Performance acceptance uses matched, unprofiled release runs at 10k residents. The 1M tier is
deferred.

- **Fixture:** `PopulatedCity::build(residents)` (`nodes/sim/core/populated_city.rs`):
  - a square grid with streets every 50 m and cross streets every 400 m;
  - zoning in eight-row repeats of six residential, one commercial and one industrial row;
  - every lot built through the demand spawn executor;
  - households admitted through demand admission, with carriers placed at home;
  - one `OWA` trunk per 10,000 residents and an effectively unlimited treasury;
  - two warm-up days. It then runs frame by frame, so trips complete.
- **Benchmark:** `rust/benches/economy_tick_benchmark.rs`. Each `operational_hour` sample is one
  of 23 demand hours, timed through the hour's fixed slots to its actions slot; each
  `daily_settlement` sample is one of ten days, timed through the calendar close. Since 2026-10-15 only
  the 10k tier runs by default; `ECONOMY_TICK_TIERS=10k,100k,1m` requests the larger ones.
- **Recipe:** run `cargo bench --bench economy_tick_benchmark -- /10k` with
  `RAYON_NUM_THREADS=8` and `taskset -c 0,2,4,6,8,10,12,14`. Alternate builds over four runs each,
  check that `GrowthDayRecord`s are identical, and compare run means with an exact permutation
  test. Record build identity, worker settings, results and artifact paths with the change.
- **Current baseline** (2026-10-10, `24705c42` plus shared workplace legs): 34.45 ms hourly and
  8.73 ms daily. Day 15 has 5,341 households and 7,134 employed.
- **`DEM-02-C` housing need** (2026-10-11, `01888e88`, four alternating runs against
  `47c5a053`): 34.48 → 34.32 ms hourly (p = 0.37) and 8.70 → 8.71 ms daily (p = 0.91), with
  identical day-15 records. See [`DEM-02-B`](demand.md#status-2026-10-11-one-working-tree-build-on-47c5a053).
- **`ECON-13` hourly frontage prices and profit proof** (2026-10-11, working tree on
  `01888e88`, four alternating runs): 34.95 → 34.93 ms hourly (p = 0.94) and 8.90 → 8.98 ms
  daily (p = 0.26), with identical day-15 records. See the
  [exit-criteria status](#status-2026-10-11-one-working-tree-build-on-01888e88).
- **`NET-02` exact CCH layout/turn-cache optimization** (2026-10-11, working tree against
  `03c1cf45`, four alternating runs): **35.50 → 25.13 ms hourly** and **9.22 → 9.01 ms daily**
  (two-sided exact permutation p = 0.0286 each). All eight day-15 records and the five growth
  reports are byte-identical. The 10k budget passes. Matched isolated queries improve 35–38%
  at 10k/100k; the full 100k economy run was not repeated, so this does not close `ECON-13-B`.
  Commands, build identity and query results are in the
  [routing acceptance record](entrance_and_exit.md#cch-routing-performance-track-net-02net-05).

A populated timing fixture does not demonstrate organic growth or solvency. Earlier fixtures and
timings are in the [archive](archive/economy_history_2026-10-10.md#econ-13-economy-tick-baseline).

For diagnosis, `METRUM_DEBUG=1 METRUM_DEBUG_FILTER=economy_phases` emits aggregate hourly,
settlement, funding, workplace, logistics and demand phase timings without per-family logs.
The ignored `benchmark_populated_midnight_boundaries` driver enables logging only for selected
boundary frames; `METRUM_ECON_BENCH_REPEATS=12` covers days 4–15. Such traces include the
preceding operational hour and are separate from unprofiled acceptance measurements.

#### `ECON-13` exit criteria

Closed 2026-10-12: A, C and D passed; B retired with the 100k tier. The contract above carries
the checks forward. `ECON-13` was done when all four checks passed on one build. The 1M tier stays deferred and is not
part of this acceptance.

1. **10k budget (`ECON-13-A`).** The recipe above measures at most **40 ms** per mean hour and
   **10 ms** per mean day, with identical `GrowthDayRecord`s across runs. **PASS** on the
   [2026-10-12 build](#econ-13-work-counts-and-live-book-planning-2026-10-12): 8.17 / 8.31 ms.
2. **100k scaling (`ECON-13-B`).** Both parts must pass on the same build as A.
   - **B1, work scaling.** `benchmark_populated_work_counts` runs one 10k and one 100k city for
     11 days ([work counts](#econ-13-work-counts-and-live-book-planning-2026-10-12)). Each bounded
     total may grow at most 1.2× the final household ratio (10.2× for the current fixture, so
     12.2×). The bounded totals are census building rows, reservation rows copied, owner slots
     prepared, proposed site rows generated, spawn parcels scanned, staffing trials, supplier
     searches, supplier route quotes, household job-search keys, and CCH searches (point
     searches plus bucket sources, which replace point searches). Counts are deterministic and
     need one run per tier; cache effects cannot pass or fail them. Pass, census and rebuild
     event counts are reported but not bounded.
   - **B2, 100k time budget.** The recipe's `/100k` run must take at most 10× A's fixed budget:
     **400 ms** per mean hour and **100 ms** per mean day, with identical records across two
     runs. Faster 10k runs never tighten it.

   The earlier bound compared 100k with the same build's measured 10k time. Any speedup of
   linear work lowered 10k proportionally more, so that ratio rose even when both cities got
   faster (2026-10-12: 10k −6%, 100k −3.5%, ratio up). It could not show progress and was replaced.

   **Status on the 2026-10-12 build: FAIL.** B2: hour 111 ms passes, day 175 ms fails. B1 fails in
   three groups. Execution-epoch censuses: census rows 13.3×, owner slots 13.4× (rows copied are now 0).
   Staffing search: trials 20×. Supplier and job search fan-out: searches 15.9×, routes 19.7×,
   job-search keys 36×. Within the bound: CCH searches 10.5×, spawn scans 10.3×, proposed site
   rows 11.2×.
3. **Bounded census cascades (`ECON-13-C`).** A profitability-failing dependency chain resolves
   in a number of funding censuses that does not grow with chain length. The existing
   `benchmark_combined_financing_cascades` cycle case must need at most **3 censuses** at chain
   lengths 4, 16 and 64 (was 4/16/64, one per link). Results must match full replay bit for
   bit, as the existing cascade regressions require. **PASS** (see status below): 2/2/2.
4. **Price-change locality (`ECON-13-D`).** The frontage congestion delay is recomputed every
   simulated second. After any frontage-delay revision, a warm queued-construction check costs
   at most 1.2× as much at 131,072 background buildings as at 16,384. Before the fix, every
   revision invalidated the queued execution epoch and paid a full census (12.5–13 ms at
   131,072). One-time census cost is reported separately from the repeated check. **PASS**
   (latest frozen routing build): 1.07×; the earlier build below measured 1.02×.

##### Routing follow-up 2026-10-11

The frozen `NET-04` production revision measures **10.789 ms/hour and 14.333 ms/day at 10k**
(four alternating pairs against `NET-03`), and **219.810 ms/hour and 1,206.616 ms/day at 100k**
(one complete pair). Its 100k daily mean is 6.2% lower than `NET-03`; every sampled day 6–15 is
lower. The hourly difference, 214.444 → 219.810 ms, has no significance claim from one pair.
Both tiers' final records match `NET-03` exactly, as do all five growth reports. `ECON-13-A/B`
remain unaccepted. C/D were freshly rerun on that same frozen revision: two censuses at every
required chain length, and 1.07× warm queued checks. Commands, identities, raw results and the
approved hierarchy's earlier record changes are in the
[routing measurements](entrance_and_exit.md#cch-routing-performance-track-net-02net-05).

Dedicated phase diagnostics locate the 10k day-15 regression: a building is removed at day 14,
minute 1,380, and the next midnight's `prepare_cell_lots_internal` spends **55.97 ms** regenerating
dirty cell lots. Building-index and road-surface preparation are negligible in that sample.
The complete demand phase is 62.03 ms, against 5.33 ms for household settlement; its planner
costs 5.97 ms and execution 0.05 ms. This cold work must not be excluded from the daily budget.
At 100k, two fresh post-build diagnostic hours spend **61.96 / 145.02 ms** preparing workplace
funding, but only **1.15 / 0.55 ms** building job options. Local funding takes 20.06 / 73.22 ms,
household claims 13.06 / 42.80 ms and border quotes about 17.2 ms in those two hours. These are
diagnostic timings, not acceptance samples or attribution of every later-day spike. Logs are
under `$ARTIFACTS/net05/` and `$ARTIFACTS/net05-block/`. Funding and job-lock behavior have not
been changed by the routing pass. On 2026-10-11 the user authorized continued exact funding
and lot/site performance fixes through ECON-13 acceptance, preserving economic decisions and
job-lock timing. The 1M tier remains deferred.

A subsequent full 100k diagnostic confirms that late daily spikes are predominantly cell-lot
preparation: **2,096.972 / 3,454.189 / 3,014.826 ms** on days 13–15. Household settlement takes
226.975 / 207.032 / 257.689 ms in those samples. This run is profiled and has verbose economic
logging enabled; these are attribution measurements, not acceptance timings.

The first exact economy candidate separates explicit-site cell invalidation from terrain
publication, uses the existing road-surface triangle grid for conservative clearance lookup,
and prepares the minimum of the existing per-border freight cost tables. Two matched 10k pairs
measure **14.290 → 13.342 ms/day** and **10.814 → 10.926 ms/hour**, with identical final records;
this candidate still fails A. Its isolated 100k freight check covers 76,109 buildings and ten
borders: roughly **17 → 1.8 ms** per complete quote refresh, including a frontage-price revision,
with every fee's bits identical. Individual dispatch tables and their f32 accumulation remain
unchanged. Targeted correctness and populated local-road planning checks pass. Artifacts are
under `$ARTIFACTS/econ13-exact/lots/`; further cell-generation work remains under validation.
Candidate source fingerprint (Rust diff plus named untracked Rust sources):
`f0982c81e3d465527d578c19089d3bc24d28172bf8978341ccbabdd536bc481b`.
The comparison uses the frozen NET-04 baseline, root-invoked release builds with default CPU
settings, Rust 1.98.1, eight Rayon workers and CPU affinity `0,2,4,6,8,10,12,14`. Commands:

```bash
RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$BENCH_BINARY" --bench --noplot /10k
RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" --ignored --nocapture populated_cell_road_plan_scaling
METRUM_CCH_BENCH_RESIDENTS=100000 RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" --ignored --nocapture benchmark_populated_border_quotes
```

The freight microbenchmark alternates original/candidate order for 11 samples of four refreshes.
100k arithmetic means are **17.279 → 1.839 ms** before and **16.794 → 1.841 ms** after a frontage
revision; cold preparation costs 5.614 ms separately. At 10k's single border, means are
0.193 → 0.192 and 0.180 → 0.184 ms: no improvement is claimed there. Surface, agriculture,
cell zoning/save, logistics, allocator, field and terrain-publication targeted suites pass.

Adding an exact maintained authored-parcel count skips the existing exclusion lookup when
every parcel is cell-derived. Its frozen source fingerprint is
`01eda2377602df090fb28865271f5a9317b052efec4c961343d21dd2dbd43bbd`.
Two fresh 10k runs average **10.937 ms/hour and 12.081 ms/day**, with identical final records;
zoning and cell-zoning save tests pass. One complete 100k run now measures **198.695 ms/hour
and 362.638 ms/day**, with the final record identical to NET-04: 54,388 households, 108,669
residents, 69,963 employed, 138,091 jobs, 76,039 buildings and treasury
1,000,000,017,473,930.5; vacancy 3,239, reserve 5,439, unhoused 2, open jobs/departures zero.
Against the earlier complete NET-04 run, the daily mean is 69.9% lower; this single 100k
comparison is descriptive, not a significance claim or the required repeat acceptance.
Same-build scaling remains **18.17× hourly / 30.02× daily**. A/B are still open. Artifacts,
including the pending focused cell-generation/funding attribution, are under
`$ARTIFACTS/econ13-exact/`. The 100k command adds `--bench --noplot /100k` to the same pinned
benchmark invocation above. Its setup takes 108.1 s; the complete process takes 766.7 s.

| Day | NET-04 daily ms | Exact lot/border candidate daily ms |
| --- | ---: | ---: |
| 6 | 275.66 | 275.50 |
| 7 | 167.77 | 171.54 |
| 8 | 210.23 | 213.75 |
| 9 | 318.68 | 321.18 |
| 10 | 439.26 | 427.53 |
| 11 | 646.82 | 631.40 |
| 12 | 474.97 | 365.94 |
| 13 | 2,440.64 | 412.10 |
| 14 | 3,727.63 | 379.71 |
| 15 | 3,364.50 | 427.73 |

The next cell candidate adds the exact explicit-site summary, conservative local-width corridor
queries, and early quad-bound rejection. Its fingerprint is
`16740738f7adbcdfbf539cbfe6dbe155396304c3323a0cfb2d1568e686f1fe9b`.
Matched, unprofiled repeated neighborhood preparation falls **41.007 → 26.166 ms** (44 samples
per build, city setup excluded), with unchanged lots and saved paint. Two complete 10k runs
average **11.167 ms/hour and 11.133 ms/day**, preserving the same final record. The surface,
agriculture, zoning/save, allocator and terrain suites pass, including a full-query oracle for
the narrowed corridor search; populated local-road planning still produces identical local
products. This remains an intermediate candidate: A is not yet accepted and its 100k tier has
not been run. Artifacts are under `$ARTIFACTS/econ13-exact/local/`; the focused command is:

```bash
RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" --ignored --nocapture benchmark_populated_cell_preparation
```

The subsequent funding candidate (`9ae28f42ee2133033c60af56169b3a47de6a52da65fd3f155ef3f3127097d1e4`)
fills the existing freight entrance cache before parallel border quotations and retains unchanged
household claims across frontage-price revisions when every attempted trip used an exact walking
proof. Any ordinary trip fallback conservatively keeps price sensitivity through prefix resets,
including rejected quotes. Budget, seller-request, acceptance and layout checks still apply.
Targeted logistics, hiring, household and demand tests pass, including repeated full-replay
comparisons after frontage revisions. A matched unprofiled fixed-state 100k refresh (44 samples,
fixture excluded) improves **38.735 → 22.665 ms**. This is a microbenchmark: two complete 10k
runs average **10.564 ms/hour and 10.884 ms/day**, with unchanged final records; A/B remain open.
Artifacts are under `$ARTIFACTS/econ13-exact/funding/`.

```bash
METRUM_CCH_BENCH_RESIDENTS=100000 RAYON_NUM_THREADS=8 taskset -c 0,2,4,6,8,10,12,14 "$TEST_BINARY" --ignored --nocapture benchmark_populated_funding_preparation
```

The shopping candidate (`f1f5f55e1eeeb7040f66db539b71fddcb253677f6aebb7ba02cf2e3f1e0e6b8b`)
rejects destinations below a full shortlist with one comparison, then uses binary search for
insertion under the original distance/building-ID order. Service visits reuse grocery shopping's
exact walking proof, with the ordinary route fallback. Household, zoning and trip-planning tests
pass. Two unprofiled 10k runs average **10.338 ms/hour and 10.498 ms/day**, with unchanged final
records. One complete 100k run averages **185.227 ms/hour and 336.480 ms/day** and preserves the
same final record as NET-04. A/B still fail at **17.92× / 32.05×**; this is one 100k run, not
repeat acceptance or a significance claim. Artifacts are under `$ARTIFACTS/econ13-exact/shopping/`.

Focused 100k boundary-frame profiles exclude city setup and intervening frames. The hour sample
still spends substantial CPU in CCH sweeps and shopping candidate insertion. Its freight counters
cover all intervening frames: 9,434,370 requests, 9,293,562 retained hits and 136,618 misses after
the old cache filled. This differs from the fixed-state refresh's all-hit workload. The three
midnight samples include the immediately preceding operational hour in sampled CPU attribution;
their reported settlement times exclude it. These profiles are diagnosis, not acceptance timing.

The cache candidate (`6f70cc889f139471204b087960a85457e963d03aa27f8d25c75cda2617d1bc50`)
retains walking connectivity by CCH/lane revision while refreshing each caller's building
entrances. A maintained lane revision also invalidates proofs after terrain-driven lane rebuilds
that preserve lane count. Freight node costs use two exact entries per fixed hash bucket:
worst-case O(1) lookup/replacement, O(roads) retained storage, and no query allocation. Collisions
always run the original query. Complete freight quotes still honor changing frontage prices.
Fresh verification passes 2,312 library tests and four growth tests; all five growth reports are
byte-identical. Two unprofiled 10k runs average **10.294 ms/hour and 10.440 ms/day**; A still fails.
The fixed 100k hour diagnostic preserves its final record and reduces replacement-related misses
from 136,618 to 11,589 among 9,434,370 requests. These counters cover intervening frames too.
One complete unprofiled 100k run preserves the NET-04 final record and measures **165.039 ms/hour
and 342.112 ms/day**, or **16.03× / 32.77×** the same build's 10k means. A/B remain open;
one run does not establish a daily improvement or repeat acceptance. A tested polygon-separation
precheck failed its matched cell benchmark (25.634 → 25.948 ms) and is removed from the following candidate; only stack
storage for short temporary contours is retained. Artifacts: `$ARTIFACTS/econ13-exact/cache/`.

The commute-retention candidate (`e2b2465ba78207fa7864f8b98fcf4645bae58889795054dfd25426471ba22225`)
passes 2,311 library tests, all four growth tests, and all five report comparisons. Two unprofiled
10k runs average **10.175 ms/hour and 10.399 ms/day**, with identical final records. The 100k
boundary diagnostic stopped before collecting samples: its newly enabled debug initializer calls
`debug::crash::init`, whose compiled entry reserves `0x270060` bytes (about 2.44 MiB) on the stack
even when crash diagnostics are disabled. The following candidate moves the fixed 4,096-event recorder to one
heap allocation at initialization, keeping event recording allocation-free. This is an
instrumentation failure, not a completed 100k measurement; the next candidate must rerun it.
Artifacts: `$ARTIFACTS/econ13-exact/commute/`.

The geometry candidate (`f440db1a4f58bf8bdb4df3626b6d5ea9f010b41971e478a5ffc4c71b20549952`)
skips conflict tests between distinct addresses on one canonical lattice and converts triangle/
quad vertices to canonical integers once per overlap comparison. The matched 44-sample local
cell benchmark improves **25.772 → 23.482 ms**, with unchanged products. Fresh verification passes
2,314 library tests, all four growth tests, and all five byte-identical reports. Two unprofiled
10k runs average **10.307 ms/hour and 10.222 ms/day**; one complete 100k run averages
**160.971 ms/hour and 328.184 ms/day**, with unchanged final records. A/B still fail at
**15.62× / 32.11×**. These are candidate measurements, not completed repeat acceptance.
The diagnostic initializer now reserves 272 bytes in its compiled stack frame; the 100k hour
and three-midnight diagnostics complete and preserve their previous final records.

The boundary-only hourly trace attributes about 69.5 ms to funding preparation (8.0 ms consumer
preparation, 26.7 ms household claims, 30.6 ms local financing), 37.4 ms to household operations
and 30.7 ms to logistics; workplace option construction is 0.9 ms. Logged phase times are not
acceptance timing. The three-midnight CPU sample still spends 19.1% checking selected pedestrian
paths; it includes the preceding operational hour, with setup/intervening frames excluded.
Artifacts: `$ARTIFACTS/econ13-exact/geometry/`.

The footprint/frontier candidate
(`e213b2497c8331b6c248ff9a72a9d9ad5b7f4a91e46a3b4a9341f541684b42d5`)
checks each distinct cell square once per generation batch and retains exact pedestrian path
arrival lanes within an existing workplace bucket query. Wider arrival frontiers keep the
ordinary path check. The matched local cell benchmark improves **23.449 → 19.931 ms**;
2,314 library tests, four growth tests and all five byte-identical reports pass. Two full 10k
runs average **10.273 ms/hour and 10.319 ms/day**; one full 100k run measures
**162.376 ms/hour and 326.976 ms/day**, preserving both final records. These full-run results do
not establish an improvement over the preceding candidate, and A/B remain open. The three-day
100k diagnostic records 11,667,864 complete freight quote requests: 2,696,513 valid hits,
3,554,132 absent keys that the full cache cannot retain, and 5,417,219 stale retained keys.
Those counters include intervening frames. Bounded replacement for complete quotes and faster
traversal of fully available customer-tree ranges are under validation; neither changes budget
or job-lock timing. Artifacts: `$ARTIFACTS/econ13-exact/frontier/`.

The bounded complete-quote/customer-tree candidate
(`2e01ff26add864f8aa090f2d535daa428a1497c070f720d317cd69df999ac8f4`)
passes 2,315 library tests, four growth tests and all five report comparisons. Two unprofiled
10k runs average **10.415 ms/hour and 10.347 ms/day**; one 100k run measures
**161.424 ms/hour and 328.339 ms/day**, with unchanged final records. There is no demonstrated
full-run improvement over the preceding candidate; A/B remain open. Three-day diagnostic node
requests fall **23,462,812 → 21,856,491**, while complete-quote hits are
**2,696,513 → 2,615,296**: replacement changes which pairs survive and is not itself acceptance.
The next candidate checks actual seller entrances, customer membership and freight frontage
price bits before discarding retained work. Artifacts: `$ARTIFACTS/econ13-exact/retention/`.

The dependency/geometry candidate
(`93a224471e1c923413ea22e712180d50c7cc701fd0637238931a9610b7674752`)
compares actual household membership and seller entrances before rebuilding customer indices,
checks the four relevant freight frontage-price bits, and prepares only pedestrian geometry
for positive walking proofs. Ordinary fallback still prepares both travel modes. Fresh checks
pass 2,315 library tests, four growth tests and all five byte-identical reports. Two unprofiled
10k runs average **10.250 ms/hour and 10.251 ms/day**; one 100k run measures
**158.926 ms/hour and 331.151 ms/day**, with unchanged final records. A/B remain open at
**15.51× / 32.30×**; these measurements do not establish a full-run improvement. The hourly
boundary diagnostic has 25 customer refreshes, all with changed budgets and none retaining
claims; 24 also change home access. Those counters include intervening frames. Further reuse
must respect those actual changes. Artifacts: `$ARTIFACTS/econ13-exact/membership/`.

A separate 12-midnight CPU profile of the preceding retention build excludes setup and
intervening frames, but includes each midnight's preceding operational hour. At 100k,
connector-target collection accounts for 13.0% of sampled CPU, CCH upward sweeps 8.9%, entrance
projection 6.5%, and polyline sampling 3.5%. Call chains are incomplete; these are leaf samples,
not reliable phase attribution or acceptance timings. Both diagnostic final records match
NET-04. The resulting connector candidate is measured below.
Artifacts: `$ARTIFACTS/econ13-exact/late-profile/`.

The connector candidate
(`e297c511dba25de797de950c378c5812c906f008ec28485e3cd089423581ec2a`)
prepares compact, ordered connector targets per worker and lane revision. An exhaustive positive
proof permits cost-only CCH queries when every physical incoming foot lane reaches every
outgoing foot lane at its junction, and every live road edge has both pedestrian directions.
Any missing connection withdraws the shortcut; duplicate connections do not count twice.
Stored routes retain their witness and connector validation. Preparation/storage are
O(roads + lanes + connections); warm forecasts allocate nothing. Fresh verification passes
2,315 library tests, four growth tests and all five byte-identical reports, including the
independent allocating-route oracle across changed metrics, broken connectors and lane rebuilds.
Two unprofiled 10k runs average **9.557 ms/hour and 10.337 ms/day**. One 100k run measures
**146.819 ms/hour and 312.696 ms/day** with unchanged final records. A/B remain open at
**15.36× / 30.25×**; these are candidate timings, not completed repeat acceptance or a
significance claim. Equivalent CCH query keys and shared entrance projection are being tested
next. Artifacts: `$ARTIFACTS/econ13-exact/connectors/`.

The equivalent-query/entrance candidate
(`bf622a6e75c4980e1b2a74f378addc5a223642d3ff2134759f762c2f774581f6`)
normalizes only network-query inputs proved irrelevant by CCH semantics and the complete-turn
proof. Restricted vehicle origins and unproved pedestrian lane constraints remain distinct.
Entrance projection is shared across walking departure/arrival and repeated candidate ranks;
frontage arithmetic and candidate comparison order stay unchanged. Fresh checks pass 2,316
library tests, four growth tests and all five byte-identical reports. Two unprofiled 10k runs
average **9.216 ms/hour and 10.447 ms/day**; one 100k run measures
**134.949 ms/hour and 287.727 ms/day**, preserving both final records. A/B remain open at
**14.64× / 27.54×**. The two 10k daily means are 10.146 and 10.748 ms; do not infer a daily
improvement at that tier. A separate phase-only 12-midnight trace locates 72–219 ms in local
funding on days 8–15, and 79–109 ms in demand execution on days 10–15. Day 11 spends
341 ms in settlement (318 ms workplace work, including 269 ms funding preparation),
19 ms in cell preparation, 50 ms in demand planning and 109 ms in demand execution.
These logged timings diagnose this build; they are not acceptance measurements.
Artifacts: `$ARTIFACTS/econ13-exact/query-context/`.

The entrance-input/allocation candidate
(`d91639b02db3be9892b8b0a063ffb9e942f66bf0ae5e74032ec61d53b6ebd7ea`)
compares exact home entrance keys before repeating unchanged geometry, and clears the complete
daily commute quote map at the same instant while retaining its allocation. Fresh checks pass
2,316 library tests, four growth tests and all five identical reports. Two unprofiled 10k runs
average **10.026 ms/hour and 10.528 ms/day**; one 100k run measures
**143.993 ms/hour and 305.381 ms/day**. Every setup and final record matches NET-04.
This candidate demonstrates no overall improvement; A/B still fail at **14.36× / 29.01×**.
The next candidate separates car availability from walking geometry, parallelizes ordered
entrance rebuilding and replaces single-ID agent-remap hashing with parallel comparisons.
Artifacts: `$ARTIFACTS/econ13-exact/access-inputs/`.

The agent-remap/entrance candidate
(`e0f7b3926905a4beb7d6c2df87be1e2f7b6d05d5a59c97a03751469130708799`)
replaces eight hash lookups per agent with parallel comparisons for a single building remap,
preserving simultaneous-map semantics for larger remaps. Ordered entrance rebuilding uses
Rayon and retains its output allocation; car-only changes reuse walking geometry. Fresh checks
pass 2,317 library tests, four growth tests and all five identical reports. The isolated single-ID
remap averages **5.115 → 0.222 ms at 100k agents** across 44 alternating matched samples,
with setup excluded and every reference compared. Two unprofiled 10k city runs average
**9.591 ms/hour and 10.101 ms/day**; one 100k run measures **140.869 ms/hour and
264.652 ms/day**, with all setup/final records unchanged. A/B remain open at
**14.69× / 26.20×**; the 10k daily means are 10.130 and 10.072 ms. These candidate runs
do not constitute repeat acceptance. A separate profiled 12-midnight trace locates
35–151 ms in staffing preparation and 36–70 ms in business funding validation on late days;
input-claim repair stays below 0.6 ms. Demand execution is 21–32 ms on days 10–15.
Logged/profiled phase timings are diagnostic only. Next under validation: household remap
comparisons, physical car-access membership retained across ownership changes, and rigorous
metric-derived freight lower bounds for rejecting unaffordable quotes before route searches.
Artifacts: `$ARTIFACTS/econ13-exact/remap/`; commands and worker settings follow the recipe above.

The physical-car-membership/freight-potential candidate
(`8ca2d65ea362bd84adf81757b60eb7a0c2d6f0d92631184edd6215f145662411`)
keeps physical customer component membership while car ownership changes only reservation
eligibility. Single-ID household remaps use parallel comparisons with unchanged ledger and
funding timing. Ring-distance bounds evaluate four exact minimizers in O(1), checked bitwise
against the original perimeter scan. Freight affordability uses integer coordinate potentials:
each road cost dominates its integer displacement times a power-of-two unit, with sums capped
at 2^24 units so monotone f32 additions preserve the bound under any CCH addition tree.
Preparation/storage are O(nodes + edges)/O(nodes) per metric; each entrance query is O(1),
allocation-free. Same-edge direct frontage receives zero network cost. Invalid or stale proofs
fall back to the original zero-carrier precheck. Only offers unable to claim any quantity at
the proven lower price skip routing; actual quotations and claim order are unchanged.
Fresh verification passes **2,322 library tests**, four growth tests and all five byte-identical
reports, including randomized CCH bounds and financial monotonicity checks. Two unprofiled
10k runs average **9.384 ms/hour and 10.345 ms/day**; one 100k run measures
**143.022 ms/hour and 252.148 ms/day**, preserving every setup/final record. A/B remain open
at **15.24× / 24.38×**. This is a candidate measurement, not repeat acceptance or a significance
claim; the 10k daily result is not improved. Next under validation: freight door-dependency
retention across unrelated building changes. Artifacts: `$ARTIFACTS/econ13-exact/potentials/`.

The freight-door dependency candidate
(`72a7a3de00b5f62db5613003703217cac4846a4e692c3e805683e1b9b3e713b2`)
retains projected access and complete freight quotations across unrelated building edits only
when both exact entrance keys, lane geometry revisions, attached-edge speed bits and all
frontage-price bits still agree. Invalidation remains O(1); lazy validation touches only the
two endpoints. Fresh verification passes 2,323 library tests, four growth tests and all five
reports. Two 10k runs average **9.690 ms/hour / 9.985 ms/day** (individual daily means
10.076 and 9.895 ms); one 100k run measures **141.835 / 266.759 ms**. Records are unchanged;
14.64× / 26.72× scaling still fails B, and A has no repeat acceptance. A separate daily-only
profile counts 2,130,532 business-port prechecks, of which 2,128,322 reject; freight potentials
prove 50,118 rejections beyond the free-freight check. Profiling is diagnostic, not acceptance.
Artifacts: `$ARTIFACTS/econ13-exact/door-retention/`.

The proposed-door and household-visit candidate
(`a1b52ffac2075348eb3c5d31d169b850459b1401a51d7f8e2823084d4c26394f`)
reuses two exact proposed-door projections until the lane geometry revision changes. Shopping
and service scans share the existing employment worker-local epoch labels, preserving both
mode order and candidate limits while replacing O(candidates²) duplicate checks with
O(candidates) warm work and no query allocation. A targeted oracle covers moved doors and
lane rebuilds; fresh verification passes 2,324 library tests, four growth tests and all five
identical reports. The 44-pair isolated duplicate-check benchmark averages **19.842 → 0.458 µs**
for 384 distinct plus 384 repeated candidates, excluding setup (`$TEST_BINARY --ignored --nocapture
benchmark_bounded_candidate_visits`). Two unprofiled 10k runs average **9.459 ms/hour /
10.018 ms/day**; one 100k run measures **136.344 / 256.697 ms**, with unchanged setup/final
records. A/B remain open at **14.41× / 25.62×**; these runs do not establish significance or
repeat acceptance. A separate 10k phase trace locates the day-15 spike in site generation
(16.122 ms) and demand planning (5.827 ms), versus 5.149 ms settlement. Next under validation:
exact-price affordability masks in the existing business customer trees, with joint-financing
queries unfiltered and rollback restoring eligibility. Artifacts:
`$ARTIFACTS/econ13-exact/site-access/`; commands and worker settings follow the recipe above.

The exact-price eligibility trial
(`0e847284b4decd359e092c26b688aaab8a836ef82de97108be1b0e13663fc1e0`)
OR-reduces up to 64 exact authored price classes through existing customer reservation trees.
Unknown prices and joint-financing queries retain the ordinary scan. A conservative free-freight
quantity bound avoids pruning tiny valid f64 requests when a larger financing-limited request
would round to zero in f32. Initial preparation is O(accounts × classes + indexed ports);
warm updates touch changed buyers' tranches and O(log bucket size) ancestor paths, with no
query allocation. Rollback marks changed buyers for refresh from restored budgets. Fresh
checks pass **2,327 library tests**, four growth tests and all five identical reports, including
filtered/unfiltered transaction comparisons and quantity-boundary cases. Two 10k runs average
**9.358 ms/hour / 10.051 ms/day**; one 100k run measures **138.147 / 255.012 ms** and preserves
all records. A/B remain open at **14.76× / 25.37×**. This does not establish an overall gain;
the trial remains under performance review. A separate 12-midnight 100k trace reduces
business-port prechecks **2,130,532 → 52,328** (97.5%), with the same 2,210 route quotes.
The remaining 50,118 rejections require the carrier bound. Business funding still costs
33–62 ms on late days; leaf filtering alone does not eliminate empty chunk-ring traversal.
Artifacts:
`$ARTIFACTS/econ13-exact/finance-masks/`; the same release recipe and worker settings apply.

The borrowed-footprint/shopping candidate removes canonical cell-corner copies and skips
route checks only when a candidate cannot enter the already-full distance/ID shortlist.
The shortlist oracle compares every prefix, including ties and nonfinite coordinates, while
preserving scan limits and visited membership. Fresh checks pass **2,329 library tests**,
four growth tests and all five reports. Four alternating isolated cell-preparation pairs
(44 samples each) average **18.933 → 19.160 ms**: no speedup is established. Two 10k runs
average **9.480 ms/hour / 10.097 ms/day**, with identical records; A remains open. The separate
day-15 trace measures 16.322 ms cell generation, 5.901 ms demand planning and 4.835 ms
settlement. These are candidate measurements, not final acceptance. The test/growth source
is `4ded65443f4639609138ce5a8450d67e7702f9f8454867e4a56d91347dbb79c0`;
the benchmark source `42b24fffc48e0e0e6f7947174dca5b83a8517ee55572855b5f558566bf94f94a`
additionally contains diagnostic-only business phase timing. The next candidate must unify
those builds. Component-wide eligibility counts are queued to avoid empty class searches
before walking any chunk rings. Artifacts: `$ARTIFACTS/econ13-exact/borrowed-footprints/`.

The component-price guard
(`114c9a0b3bdb689e526bbd7a25c7e67d94a1a76dce8fd6c874f6fcc754eeb5df`)
counts each eligibility bit in existing resource components. Empty-class queries now stop
after component lookup, before chunk-ring traversal; updates touch at most 64 class counts
and the existing bucket ancestors. Quantity depletion may leave stale-positive masks, never
false exclusions. Fresh verification passes **2,330 library tests**, four growth tests and
all five reports. Eleven alternating isolated pairs at 16²/64²/128² occupied chunks measure
**9.368 / 299.205 / 1,379.138 µs → 4.619 / 4.767 / 5.284 ns** per empty query
(`$TEST_BINARY --ignored --nocapture benchmark_empty_price_class_queries`); these synthetic
results establish empty-query locality, not a whole-city gain. Two 10k means are
**9.283 ms/hour / 10.030 ms/day**. One complete 100k run measures
**135.219 ms/hour / 215.708 ms/day**, preserving all records. The daily mean is 15.4% lower
than the preceding finance-mask candidate's one-run result; this is descriptive, not a
significance claim or repeat acceptance. A/B remain open at **14.57× / 21.51×**.
A separate 12-midnight trace preserves the exact prior candidate/route counts and final
record; all 50,118 remaining business-account rejections require the carrier bound. Day-15
business funding falls **44.957 → 5.232 ms**, with 1.6–2.0 ms account preparation and
0.3–0.6 ms matching per pass. The remaining day-11 staffing spike is **145.729 ms** across
three censuses; day-15 staffing costs 34.775 ms. Day-15 site preparation, demand planning
and execution cost 27.164 / 49.887 / 20.364 ms. These diagnostic phase comparisons explain
the affected work; they are separate from acceptance timings. Artifacts:
`$ARTIFACTS/econ13-exact/component-masks/`.

The local-candidate build
(`6393cfe5483d6fb87cdb9878477711f6b3f45fa5bdec0149ef544024617cd2a7`)
changes cell conflict queries to exclude the same canonical frame before
deriving footprints and reuse the spatial query's exact corners. Shopping/service shortlists
retain their squared distances in bounded stack storage, preserving distance/ID ordering,
scan limits and ring termination. Existing chunk-ring scans also clip coordinates to the
requested components' existing envelope, preserving every item and ring-completion event.
Fresh verification passes **2,334 library tests**, four growth tests and all five reports,
including targeted prefix/order oracles. Eleven alternating shortlist pairs average
**3.808 → 1.957 µs** for 8 retained/128 scanned candidates and **17.200 → 8.716 µs** for
24/384 (`$TEST_BINARY --ignored --nocapture benchmark_cached_shopping_distances`). Four
alternating cell-preparation pairs, 44 samples each, average **19.460 → 15.224 ms**, with
identical products. Local generation stays at **121.600 / 122.452 / 121.425 µs** as distant
roads/cells increase from 0/0 to 1,000/16,384 and 10,000/131,072; setup is excluded
(`$TEST_BINARY --ignored --nocapture benchmark_cell_zoning_locality`). Two 10k runs average
**9.235 ms/hour / 9.673 ms/day**; both daily means (9.743 and 9.603 ms) meet A's budget and
all records are identical. One complete 100k run measures **129.426 ms/hour / 205.531 ms/day**,
with identical records; B still fails at **14.01× / 21.25×**. A has no final one-build repeat
acceptance yet. A separate 10k phase trace puts day-15 cell preparation at 13.177 ms, demand
planning at 5.761 ms and settlement at 5.136 ms (25.233 ms total). The separate 23-hour 100k
profile records 350,552 nearest-supplier searches, 4,818,263 candidates and 4,174,801 route
quotes, identifying remaining routing work; profiled timings are not acceptance measurements.
Artifacts: `$ARTIFACTS/econ13-exact/local-candidates/`.

The workplace-cache build
(`59be0c07e68ee248aad8bb165e917f1fe7e2b8d8f822139919c896c8a141b935`)
uses the existing fixed fast hasher. This cache has no iteration-based consumers; its
complete-route price expiry is unchanged. Fresh checks pass **2,334 library tests**, four
growth tests and all five reports. Eleven alternating microbenchmark pairs at 160,000 entries
reduce insertion **17.356 → 8.519 ns** and lookup **18.044 → 5.015 ns** per key; all values,
including cached unreachable routes, match (`$TEST_BINARY --ignored --nocapture
benchmark_workplace_route_cache_hashing`). Two 10k runs average **9.298 ms/hour / 9.664 ms/day**;
one 100k run measures **128.997 / 203.781 ms**. All records match. This establishes the cache's
isolated gain, not a significant whole-city improvement; B still fails at **13.87× / 21.09×**.
A separate 12-midnight trace also preserves the final record and previous business/freight
request counts. It records 5,339,313 nearest-supplier searches and 64,692,162 route quotes
across all intervening frames. Day-11 staffing remains 135.095 ms across three censuses;
day-15 staffing is 33.700 ms, with cell preparation / planning / execution at
22.366 / 53.507 / 19.607 ms. These are diagnostic attributions, not acceptance timings.
Artifacts: `$ARTIFACTS/econ13-exact/route-hash/`.

The endpoint-sweep candidate
(`4d83a31a4fa252b00ca7d1c160c7f608d384ac7c3cd84dac98e43bd1acf65897`)
retains each CCH endpoint sweep in its existing scratch until its complete input key changes,
keeping exact label/witness order with O(1) additional storage. It also extends the integer
network bound to the L1 displacement and applies the bound before recurring-supplier route
quotes. Preparation remains O(nodes + edges), storage O(nodes), and each candidate bound O(1);
strict rejection preserves route/price/index ties. Fresh checks pass **2,337 library tests**,
four growth tests and all five reports. Exact oracles cover cached/fresh CCH costs and witnesses,
metric/clone/epoch invalidation, subnormal bounds, and supplier receipts through retries and
depletion. Eleven alternating sweep pairs average **1,409 → 1,072 ns/query** for two queries
per source and **1,179 → 749 ns** for 16; ungrouped queries average **1,409 → 1,413 ns**
(`$TEST_BINARY --ignored --nocapture benchmark_cch_endpoint_sweeps`). Two alternating populated
10k random-query pairs preserve every report byte, with cost-only **0.667 → 0.675 µs** and
path-producing **0.847 → 0.851 µs**; this establishes no random-query gain. Two whole-city 10k
runs average **9.509 ms/hour / 9.794 ms/day**; one 100k run measures **129.291 / 208.337 ms**,
preserving records (13.60× / 21.27×). This establishes no overall gain. The L1 extension and
recurring-supplier bound were removed before the next build; endpoint sweep reuse remains.
A separate daily-only profile and phase trace attribute day-15 planning to **33.668 ms site
preparation**, **10.751 ms reservation copying**, **2.603 ms snapshots**, **1.936 ms funding
fork**, **0.897 ms queue replay**, and **4.984 ms actual planning**. Day-12 site preparation
is 32.397 ms. These diagnostic timings identify repeated proposed-door preparation; they are
not acceptance measurements. Artifacts: `$ARTIFACTS/econ13-exact/endpoint-sweeps/`.

The striped-cache build
(`614e43ae9be26e6761d64a30c29e4212d6074c9881331f603bf40686ac7322d4`)
partitions shared commute-cache synchronization into 32 stripes, preserving the original global
bucket hash, capacity, replacement policy and revision guards. It adds O(1) lock storage; each
query accesses one bounded bucket without allocation. Fresh checks pass **2,336 library tests**,
four growth tests and all five reports. Eleven alternating contention pairs at eight workers
reduce cold access **162.834 → 17.325 ns/query** and warm access **124.880 → 15.172 ns**.
Single-worker cold/warm access changes **21.813 → 23.465 / 19.850 → 21.526 ns**
(`$TEST_BINARY --ignored --nocapture benchmark_striped_network_quotes`). Two 10k runs average
**9.259 ms/hour / 9.619 ms/day**; one 100k run measures **122.964 / 201.521 ms**, preserving
all records. B remains open at **13.28× / 20.95×**; final matched acceptance repeats remain
pending. Artifacts: `$ARTIFACTS/econ13-exact/striped-quotes/`.

The retained-site build
(`5cedbdf04a627c19ae491d37ad4b4b0fd09537e4a41661243edabd421ea9413a`)
keeps proposed entrances across live-market forks. Existing prepared owner/asset records carry
exact door bits, frontage edge/side, CCH generation and lane revision; a factory reuses geometry
only when those inputs match. Financial books and candidate indices refresh normally. Cold
preparation stays O(variants log variants + ports log ports), with no per-quote allocation or
new spatial index. Fresh checks pass **2,337 library tests**, four growth tests and all five
reports, including changed poses/assets, lane/network rebuilds and retained-book replacement.
Two 10k runs average **9.238 ms/hour / 9.909 ms/day** (individual daily means 9.633 and 10.185 ms).
One 100k run measures **121.719 / 200.622 ms**, preserving records at **13.18× / 20.25×**.
These results establish no clear whole-city gain; A/B acceptance remains pending. A separate
12-midnight trace preserves every final record and prior business/freight request count. Site
preparation remains **28.716–33.421 ms** on days 10–15; day-15 reservation copying is 9.236 ms
and actual planning 5.451 ms. Reusing entrance geometry has not removed the preparation cliff.
Artifacts: `$ARTIFACTS/econ13-exact/retained-sites/`.

Next validation counts sites sharing the exact maximum radius, so removing or shrinking one
of several maxima does not reduce the whole city. It also reuses the household census's walking
proofs after checking household/home identity, exact entrance bits and network revisions.
Shopping stores retain compact directed groups/node IDs instead of full trip ends; unproven
trips keep the original route fallback. No new spatial or household index is introduced.
Asset level lookup reuses its already-borrowed manifest instead of hashing the same ID twice.
The test-only phase trace now counts staffing trials and rejection causes; free-input cost
probes are diagnostic only and do not reject operations. This candidate is commit `5a268e64`
(2,339 library tests; the debug `cargo test` and Godot bridge stages pass after a corrected
price-mask test). It is the baseline below and was not measured separately.

#### `ECON-13` work counts and live-book planning (2026-10-12)

**Work counts.** Test builds count work with `debug::work` (compiled out elsewhere). The ignored
driver `benchmark_populated_work_counts` runs one demand-hour day plus ten settlement days
(`METRUM_ECON_WORK_DAYS`, default 11) after the fixture build and prints per-day and total counts
(`METRUM_CCH_BENCH_RESIDENTS` selects the tier). On `5a268e64` the 100k city is 10.2–10.3× the
10k city, but its daily work grows faster:

| Work over 11 days | 10k | 100k | Ratio |
|---|---:|---:|---:|
| Planning passes / live censuses | 255 / 266 | 256 / 267 | 1.0× |
| Execution-epoch censuses | 15 | 96 | 6.4× |
| Census building rows | 2.08M | 27.6M | 13.3× |
| Reservation rows copied | 33.2M | 425.7M | 12.8× |
| Site-table rebuilds | 30 | 197 | 6.6× |
| Proposed rows generated (buyer / supplier) | 36k / 36k | 2.42M / 2.41M | 67× |
| Supplier searches / route quotes | 0.43M / 4.28M | 6.89M / 84.2M | 16× / 20× |
| Household job-search keys | 8.5k | 308k | 36× |
| Staffing trials | 0.71M | 14.1M | 20× |
| CCH point searches | 1.39M | 14.3M | 10.3× |

The gap is therefore algorithmic, not only cache effects. From day 7 the 100k city places or
removes a building every hour, so each hour also pays an execution-epoch census, a market
copy and two full site-table rebuilds. A diagnostic phase trace (`economy_phases`, profiled
timings) attributes the largest superlinear time to the live census: staffing 23.6× and
household claims 57.8×; site preparation grows 50–71×.

**Changes, all exact.**
- *Changed-owner site rows.* Proposed supplier and buyer rows are regenerated only for the
  buildings named by the allocator's building log and the parcels named by zoning's occupancy
  log (see [`building_allocator.md`](building_allocator.md#allocator-owned-indices-and-caches)
  and [`zoning.md`](zoning.md#6-allocator-interaction)). Other rows move unchanged; parcel owners
  shift with the building count; owner/key order is preserved. Registry, overlay, parcel-layout,
  lane and CCH changes still rebuild all rows. Rows cache their door chunk and components.
- *Separate proposed-site index.* Business site ports keep their own bucket index after the
  live ports. A two-index ring scan reproduces the merged event order exactly, so preparing
  sites no longer re-indexes every live buyer port.
- *Live-book planning.* With a current employment census, the hourly pass plans on the live
  customer, business and input books inside one first-touch pass journal per book and restores
  them when it closes (see [`demand.md`](demand.md#construction-queue-and-execution)). Finance
  masks are a conservative pruning cache: rollback may leave them wider, never narrower, and
  claims still test availability and exact affordability.

**Verification.** 2,344 library tests pass. Test builds compare full fingerprints of all three
live books after every live planning pass, including the 10k populated city's 255 passes.
All five growth reports are byte-identical to `5a268e64`. The 10k and 100k work drivers end
with the same final records as before (day 14: 5,330 / 54,388 households). Counts after the
change:

| Work over 11 days | 10k | 100k |
|---|---:|---:|
| Reservation rows copied | 1.53M | 100.0M (execution epochs) |
| Proposed rows generated (buyer / supplier) | 1,349 / 1,347 | 15,153 / 15,143 |

Census rows, staffing trials, supplier searches and planning passes are unchanged. Cache-hit
counters moved slightly (CCH point searches +0.2%), because planning and execution no longer
share one forked walking cache and search hint.

Site-row updates walk rows rather than every owner, and preparing sites resets only the
proposed-site ports: no account maps to them between proposals. Two clean 100k phase traces
(`economy_phases`, so absolute values include logging) over days 7–10, when a building changes
every hour:

| Per simulated day | `5a268e64` | This build |
|---|---:|---:|
| Site preparation in planning passes | 275.5 ms | 106.2 ms (−61%) |
| All site preparation, including execution epochs | 556.6 ms | 222.6 ms (−60%) |
| Reservation reset (market copy) | 124.4 ms | 22.3 ms (−82%) |

The rest of site preparation, about 0.6 ms of row updates and 1.1–1.5 ms of layout per rebuild,
rebuilds the proposed-site and projected-source indices whenever live ports are renumbered.
Removing it needs an incremental bucket index with stable reservation positions.

**Execution epochs** now borrow their census's customer and business books inside the same pass
journals instead of copying them, and return them restored before the next census. Their
rollback never reads the allocator, so it stays exact across placements within the epoch.
Test builds compare both books at lend and return. Reservation rows copied fall to **0** at 10k
and 100k (from 1.53M and 100.0M); reports and populated records are unchanged. This last change
was not re-timed.

**Matched timings** (`5a268e64` vs the working tree; `RAYON_NUM_THREADS=8`,
`taskset -c 0,2,4,6,8,10,12,14`; `$BENCH_BINARY --bench --noplot /10k` and `/100k`; binary
SHA-256 `16f927d2c4be7294cdad76732f62ccd0c83b06ada03b76b274a5e188ce186a45` and
`a4376e23f42b634eb64e6db80f1f8638b49084ec045f3c5c2740d33b1a214d6c`):

| Tier | HEAD hour / day | Working tree hour / day |
|---|---:|---:|
| 10k, four alternating runs each | 8.719 / 8.792 ms | **8.171 / 8.311 ms** |
| 100k, two alternating runs each | 115.46 / 181.52 ms | **111.38 / 174.93 ms** |

At 10k every working-tree run is faster than every HEAD run on both measures (two-sided exact
permutation p = 0.0286); the hour falls 6.3% and the day 5.5%. All runs of both tiers end with
identical day-15 records. At 100k both working-tree runs are faster than both HEAD runs (hour −3.5%, day −3.6%);
two runs per side establish no significance. `ECON-13-A` passes on this build; C/D were not rerun. `ECON-13-B` still
fails under its replacement work-count and time-budget form (see the exit criteria).
Execution-epoch censuses and copies, the live census's staffing search, and supplier/job searches
remain superlinear.
Logs: `$ARTIFACTS/econ13-live-books/`.

##### Status 2026-10-11 (one working-tree build on `01888e88`)

This section records the earlier build. The later approved `NET-03` hierarchy changes route
ties and f32 sums, and therefore the populated city's evolution. Its 10k hour improves to
11.14 ms but the day costs 14.15 ms: `ECON-13-A` is no longer accepted. `NET-04` preserves that
new city's exact records; its completed measurements and remaining blockers are above. See the
[routing measurements](entrance_and_exit.md#cch-routing-performance-track-net-02net-05).
The C/D results below are historical; the routing follow-up records their fresh reruns.

All runs use one release build of the working tree, `RAYON_NUM_THREADS=8` and
`taskset -c 0,2,4,6,8,10,12,14`, on an otherwise idle machine. Logs are under
`$ARTIFACTS/econ13/final`.

- **`ECON-13-A` PASS.** The recipe above, four alternating runs each of `01888e88` and the working
  tree (`$BENCH_BINARY --bench --noplot /10k`): hour 34.95 → **34.93 ms** (p = 0.94), day 8.90 →
  **8.98 ms** (p = 0.26). All eight runs end with the same day-15 record: 5,341 households,
  10,668 residents, 7,134 employed. The five growth reports (`METRUM_GROWTH_REPORT`, `_LONG_`,
  `_EXPANSION_`, `_RECOVERY_` and `METRUM_PRODUCER_TOWN_REPORT`) are byte-identical to HEAD's,
  so neither change alters a growth trajectory.
- **`ECON-13-B` BLOCKED.** Same build, two runs each, records identical between runs (day 15:
  10k 5,341 households / 7,134 employed; 100k 54,393 / 70,351):

  | Tier | Hour, run 1 / 2 | Day, run 1 / 2 |
  |---|---:|---:|
  | 10k | 36.52 / 34.37 ms | 8.82 / 8.82 ms |
  | 100k | 910.5 / 908.8 ms | 3.635 / 3.628 s |
  | Ratio | **25–26×** | **411–412×** |

  `01888e88` measures the same 100k city (hour 934.6 ms, day 3.626 s, identical day-15 record),
  so the failure predates this work. A third 100k run with economy timing logs and per-phase
  daily timers (`diag-100k.log`; logging inflates absolute times) located the cost:

  - **Job search.** The 100k city has 26–153 open job entries against 141k jobs. Each hour
    700–2,900 homes search, at 1.5–3.8 s. A search ranks 24 options with exact commutes and can
    stop early only once it holds them, so with fewer open workplaces than that it routes every
    reachable one (about 19 routes per searching home). Job locks set during the build's
    admission expire together, so from day 6 on, re-searches of about 8,000 homes at once recur
    (1.7–6.0 s each). The daily household settlement takes 70–100 ms on days 2–5 and
    0.6–10 s on days 6 and 8–15. At 10k, job search averages 0.2 ms per hour.
  - **Funding-census spikes.** In some hours the census's local funding step takes 2.2–8.3 s,
    against its usual 20–37 ms (about 2 ms at 10k). The midnight demand pass, which runs a
    census, grows from 22 ms to 0.3–1.8 s on days 11–15. The trigger is not yet identified.
  - **Steady growth.** In ordinary hours, household trips are 18×, logistics 20× (input
    shipments 132×, with late-run spikes to 300+ ms) and the census 40×. Border carrier quotes
    take 16.7 ms against 0.2 ms, because the fixture adds one `OWA` border per 10,000
    residents and each building takes the minimum over all borders. CCH searches grow
    linearly (about 0.3M per day at 10k, 2.5M at 100k), at a similar 90–156 heap pops per search,
    so part of the steady excess is the larger working set.

  Decisions needed before this can pass: (1) whether job search may stop or cache below
  exact 24-option ranking when open jobs are scarce, and whether the fixture should spread
  initial job locks rather than expire them together; (2) whether the bound uses a fixed `OWA`
  border count, or work counts instead of wall-clock time, given the cache effects above. The
  census spikes need their own investigation first.
- **`ECON-13-D` PASS.** Measured first at 10k on the pre-change build: the live delay revised 60
  times per simulated hour (once per simulated second, which is one authored minute), and in
  the producer town 10 of 22 queued checks over 30 days rebuilt the census only for that
  revision. The fixed 10k fixture has no queued checks, so it paid none. Economy prices now read
  `Lane::frontage_delay_penalty_s`, which `TransitNetwork::publish_frontage_prices` copies from
  the live delay at the start of each operational hour. Agent trips keep the live delay
  ([entrance and exit](entrance_and_exit.md#exact-frontage-congestion-penalty-cache)). The
  revision now advances once per hour, and the producer town's frontage-only rebuilds drop to
  zero (22 → 12 rebuilds). 10k day-4 and producer-town day-31 records are unchanged. Save
  format 85 keeps both delays. The per-lane pass of the live update now runs in parallel; the
  per-agent speed sum stays serial, so its f32 summation order does not depend on the worker
  count. `$TEST_BINARY --ignored --nocapture benchmark_queued_project_validation` changes every
  vehicle lane's live delay before each of 1,024 warm checks:

  | Background buildings | Census + first check | 1,024 warm checks | Check after an hourly publish |
  |---:|---:|---:|---:|
  | 16,384 | 1.363 ms | 0.917 ms | 1.464 ms |
  | 131,072 | 12.927 ms | 0.934 ms | 13.291 ms |

  Warm checks: 131,072 / 16,384 = **1.02×**. Each hourly publish still pays one census, which the
  hourly census at that boundary pays anyway. `live_frontage_delay_reaches_queued_checks_only_when_published`
  pins that a live revision anywhere, including the project's own frontage, leaves the revision
  and the warm check unchanged until the publish, after which the check reprices.
- **`ECON-13-C` PASS.** `$TEST_BINARY --ignored --nocapture benchmark_combined_financing_cascades`
  cycle case, censuses per refresh:

  | Length | Before (cold / warm) | Now, background 0 (cold / warm) | Now, background 16,384 (cold / warm) |
  |---:|---:|---:|---:|
  | 4 | 4 / 4 | 2 / 2 | 2 / 2 |
  | 16 | 16 / 16 | 2 / 2 | 2 / 2 |
  | 64 | 64 / 64 | 2 / 2 | 2 / 2 |

  The 64-link cold census at 16,384 background buildings takes 1.8 ms (was 26–27 ms). Two
  changes in the [funding census](#funding-census-econ-17): the up-front profit proof pins the
  loss-making link, and propagation prepares the business layout when the pass that failed
  never reached validation. A zero-ceiling operation also starts with zero offered output. The
  rival cases stay at 2. `unprofitable_supplier_cycles_match_replay_within_their_depth` now runs
  lengths 2, 8 and 64 against full replay (proof and propagation disabled), with identical
  results and at most 2 censuses. All other cascade replay regressions pass unchanged.

### Healthy-growth acceptance

Population alone is insufficient. Scenarios must expose:

- **Households:** disposable income, essential stockouts, employment, funded vacancies and
  migration with limiting reasons.
- **Businesses:** sales and utilization, input costs, payroll, operating cash, closures and
  replacements, with startup equity separated from recurring receipts.
- **City:** recurring income and expense by source, separated from one-time costs and the
  starting treasury.
- **Trade and limits:** imports and exports, committed construction, and which of land,
  transport, labor or customers limits growth.

Run beyond startup windows, include recovery after a supply interruption, and test more than one
zoning/service mix. Passing by raising the starting treasury, or by repeatedly replacing insolvent
businesses, is not acceptance. Mature stability is valid when capacity is satisfied.

## Retired mechanisms

| Mechanism | Current contract |
| --- | --- |
| Hourly utility-budget freeze (`ECON-01`) | Utilities settle daily; failure uses the bankruptcy lifecycle. |
| Inactive capacity blocking replacement (`ECON-02`) | Deserted and inactive buildings leave demand capacity. |
| Daily startup-float refill (`ECON-03`) | Equity is granted once. |
| Zoned land generating arbitrary businesses (`ECON-04`) | Entry follows funded customers, workers and inputs. |
| Pioneer demand floor (`ECON-05`) | Removed; transfers are explicit fiscal flows. |
| Economy editor (`ECON-10`) | Removed; authored TOML, balance tests and scenarios. |
| Citywide R/C/I pressure, spawn thresholds and hysteresis | Removed by save format 83; demand uses concrete project checks. |
| Citywide store and service activity ratios | Removed 2026-10-10; each store follows its own census activity. |
| Border job limits and `OWA` quotas | Removed 2026-10-06/09; roads limit trade. |
