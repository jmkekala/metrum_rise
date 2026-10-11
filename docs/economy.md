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

## Growth redesign (`ECON-11`–`ECON-14`, `DEM-02`)

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
runnable exit criteria: performance and scaling under [`ECON-13`](#econ-13-exit-criteria), and
economic health beyond the starter towns under [`ECON-14`](#econ-14-exit-criteria).
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

1. Each minute: the next queued private construction start, at most one per minute.
2. Each whole hour: the **operational hour**: construction progress, farm and mine production,
   then the household hour (below).
3. At minutes other than 0, the **hourly demand pass**: project planning, household admission and
   immediate building actions ([`demand.md`](demand.md#cadence-and-handoff)).
4. At minute 0, after the operational hour: the **daily tick**: allocator maintenance, the daily
   settlement (below), road upkeep, the daily demand pass, household removals, then the minute-0
   hourly demand pass. Daily tax buckets roll into the report only after all of these.

Household hour (`households/tick.rs`): materialize arrived households; rebuild membership and
worker counts; rehouse unhoused households; building production and electricity dispatch;
logistics; household consumption, replenishment and service sales; the workplace pass (funding
census plus job search); agent money synchronization.

Daily settlement, in fixed order:

1. Bankruptcy check (buildings that ended yesterday in distress and are still negative).
2. Wages, within the funded census capacity.
3. Household transfers.
4. Property tax.
5. Utility bills, then distress liquidation of negative private budgets.
6. Business profit tax.
7. Housing affordability, relocation and eviction.
8. Daily workplace pass (the commute caches are dropped first).

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
every `travel_estimate_refresh_minutes` (360), never re-queried per frame. Rush hour emerges from
overlapping windows.

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
| Business profit tax | 10% | Daily, on `max(0, operating_budget − baseline)` after the day's flows; the baseline then resets. No loss carryforward; startup equity is never taxed. |
| Property tax | 2 / 25 / 35 per day | Per occupied home / active commercial / active industrial building, × 1.75 per level above 1. City-owned and unzoned buildings pay none. |
| Unemployment benefit | 30/day | Per unemployed adult, for at most `unemployment_max_days` (30) |
| Pension | 30/day | Per elder |
| Child support | 10/day | Per child |

Daily budget history keeps separate buckets for each tax, each property class and each transfer.
Pending transaction taxes roll into the report at the daily close.

### Household transfer payments

Transfers are paid daily, after wages and before housing checks, in stable household order
(`households/benefits.rs`).

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
  slot. Linked resident agents are the authoritative membership; cached counts are rebuilt from
  agents.
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
- An unaffordable full refill becomes the largest affordable partial basket.
- The reservation moves stock and money; supplies rise only when the shopper returns home.
  Shopping owns both legs and the wait for pickup and delivery acknowledgement; the work
  scheduler skips it.
- Each leg's deadline is its travel estimate plus the authored allowance. A timeout before pickup
  refunds; one after pickup records a failure. After 3 failures the household enters
  `failed_terminal` and retries only on its normal cadence.
- States: `stable`, `needs_replenishment`, `waiting_for_shopper`, `shopping_to_store`,
  `shopping_returning`, `fulfilled`, `cooldown`, `failed_terminal`.

### Services

Service stores sell aggregate capacity each hour. Resident demand is capped by staffed capacity,
households are debited proportionally, and revenue goes to service stores by capacity share.

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
  active shopping trip through the normal refund rules.
- Unhoused households try to rehouse after each hourly membership census, claiming real
  vacancies before arrivals can. This costs O(1) when everyone is housed.
- `unhoused_days_elapsed` counts consecutive settled days without a home. Demand's exit policy
  interprets it ([`demand.md`](demand.md#household-removal)).

### Farm households (`ECON-08`)

Each explicit farm provides exactly one household slot, filled by ordinary admission or
relocation. Resident adults claim on-site jobs first with zero commute and switch activity at
shift times without a road trip. Shrinking a field reduces jobs but keeps the household. A
bankrupt farm withdraws its vacancy; removing a farm leaves its family unhoused through the normal
lifecycle.

## Funding census (`ECON-17`)

`JobFunding` (`hiring.rs`) is rebuilt in every hourly and daily workplace pass. It is the single
answer to "how much of this operation can run":

- **Capacities:** the largest workforce whose forecast recurring sales cover operating costs, and
  whose cash, usable stock and paid in-transit cargo cover working capital
  (`operation.rs`). Owner equity is capital, never sales.
- **Activities:** accepted recipe activity, as a share of full output. Sales-scaled stores,
  service stores, farms and mines produce, restock and staff to it.
- **Customers:** households claim finite quantities from reachable stores with exact directed
  shopping access (`customers/routing.rs`), from recurring after-tax wages and funded permanent
  transfers after housing and utility costs. Savings and temporary benefits are excluded.
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
admission all read this census. Daily payroll pays at most the census capacity, bounded by the
physical workplace. Service stores and explicit work areas release workers above it; other unpaid
workers quit after 2 unpaid days.

Costs: market aggregation is O(B·ports + households·categories) in fixed parallel chunks.
Household and supplier claims commit serially through the existing component and chunk indices.
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
  CCH change and dropped daily.
- **Network legs** (`SharedNetworkQuotes`): the road-network part of each commute, shared by
  every worker thread across hours. A car leg uses the cost-only CCH query; a walking leg keeps
  the path for the pedestrian connector check. It is emptied on a CCH or lane change and dropped
  daily.

Neither cache changes a result.

Resident workers whose best job is at home are assigned first; all others apply in agent order
against live slot counts.

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
  is imported.
- **Water and sewage** are local when a staffed connected provider operates; otherwise they fall
  back to the `OWA`.
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
  - dispatch moves stock into the shipment, and the seller is paid only on delivery;
  - failure, expiry, cancellation or building removal releases both sides deterministically,
    refunding buyers (or the treasury for city services);
  - a delivered shipment stays settled when an endpoint is removed.
- **Carriers:** one truck agent per shipment. It goes source → destination, then returns empty
  to its origin; the return leg holds no reservation. Travelling legs time out after
  `max(6, 4 × eta_hours + 2)` operational hours.
- **Supplier search:** `SupplierCandidateIndex` lists a supplier under a resource only while its
  unreserved stock fills one batch. Searches scan chunk rings outward from the buyer and stop at
  the nearest qualifying supplier, so an empty search ends at once (`ECON-15`).
- **Routing:** freight routes use the exact car-access abstraction of the entrance cache, with
  direct same-lane frontage travel when the exact points are in forward order.
- **Retries:** failures enter a cooldown (1 hour) and escalate to a visible terminal state after
  3 attempts. Terminal states never block recovery after cooldown, budget, supplier or road
  changes.

## Private construction

Demand-selected private buildings enter the world as construction sites. The record exists at
once, so the parcel is claimed and the site renders, but it supplies no homes, jobs, production,
shopping or utilities until completion.

- **Durations** (`runtime_tuning.construction`): 6/12/18 hours for residential, 8/16/24 for
  commercial, 12/24/36 for industrial, by level. Progress is checked hourly.
- **Committed capacity:** demand counts constructing and queued projects as committed, so no
  duplicate is started.
- **Instant changes:** upgrades and downgrades are instant replacements; only fresh spawns build.
- **Visuals:** MultiMesh site pad, foundation and scaffold, with the full-scale final asset rising
  through the hour. It never scales, flattens or recolors.

## Building Bankruptcy

This applies to private commercial, industrial, field, extractor and privately operated utility
businesses; city-owned facilities are excluded. Daily, in settlement order:

1. **Check:** if yesterday ended in `budget_distress` and the budget is still negative, mark
   `is_deserted`. A building always gets one full distress day first.
2. **Wages:** pay each worker from the budget. When cash is short, first sell enough unreserved
   output at distress price (`catalog × owa_distress_liquidation_multiplier` = 0.25) to cover the
   wage. Otherwise the worker goes unpaid, and wages never push the budget negative.
3. **Utilities:** charged even into a negative budget.
4. **Distress:** if the budget is negative, liquidate all unreserved output at distress price and
   set `budget_distress`; otherwise clear it.

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

### `ECON-13` economy tick baseline

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
  of 23 demand hours; each `daily_settlement` sample is one of ten days.
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

A populated timing fixture does not demonstrate organic growth or solvency. Earlier fixtures and
timings are in the [archive](archive/economy_history_2026-10-10.md#econ-13-economy-tick-baseline).

#### `ECON-13` exit criteria

`ECON-13` is done when all four checks pass on one build. The 1M tier stays deferred and is not
part of this acceptance.

1. **10k budget (`ECON-13-A`).** The recipe above measures at most **40 ms** per mean hour and
   **10 ms** per mean day, with identical `GrowthDayRecord`s across runs. **PASS** (see status
   below): 34.93 / 8.98 ms.
2. **Linear 100k scaling (`ECON-13-B`).** On one build, `-- /100k` measures a mean operational
   hour at most 10× and a mean daily settlement at most 10× the same build's `/10k` values,
   with identical records across two runs. An older, economically inert fixture met the hour
   (3.1×) but not the day (15×). **BLOCKED** (see status below): on the live-economy fixture the
   hour is 26× and the day 411×, identically at `01888e88`. Most of the excess comes from job
   search across a sparse open-job supply and from funding-census spikes. Fixing them needs the
   decisions named in the status.
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
   (see status below): 1.02×.

##### Status 2026-10-11 (one working-tree build on `01888e88`)

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

#### `ECON-14` exit criteria

`ECON-14` is done when all three checks pass on one build. Each is a test in
`rust/tests/growth_scenario.rs` or a non-ignored library scenario. A failure caused by design is
recorded here with evidence rather than tuned away. Already covered elsewhere: a second
zoning/service mix (`DEM-02-A`, `producer_town`), recovery after a business failure
(`bankrupt_grocery_is_cleared_and_replaced_without_rezoning`) and scarce-labor expansion
(`many_new_industrial_lots_respect_scarce_labor_and_earn_their_owner_capital`).

1. **Finite-resource closure (`ECON-14-A`).** The starter town with the `producer_balance_matrix`
   20 ha coal mine runs two years. Within 60 days of the mine's last sale, its former workers are
   re-employed or have left through the household exit policy. Over the last 90 days of year two
   there are no empty-household days, no unpaid household transfers and a positive recurring city
   balance. Last measured: 40 unemployed adults and 182 empty-household days in year two.
2. **Oversized producer warning (`ECON-14-B`).** A producer the town cannot staff or fund is
   flagged before the player pays for it. Losing money without rescue is intended:
   `hundred_hectare_start_distinguishes_mechanized_farm_from_labor_intensive_mine` pins that a
   100 ha mine in the starter town spends its owner's capital while the 100 ha farm trades
   sustainably. The check: the placement preview for that 100 ha mine reports the limiting
   reason (labor, operating income or working capital) from the same operation check that
   private entry uses, and the 100 ha farm's preview reports none. Whether the preview warns or
   refuses is a design decision.
3. **Pension-funded households (`ECON-14-C`).** A scenario whose arrivals include pension-funded
   elder households runs one year. Pension and child-support transfers stay fully paid, elder
   households have no empty-household days over the last 90 days, and the recurring city balance
   stays positive. It covers the open pensioner-affordability question.

Additional funded economic activities and the broader demand redesign move to later tracks; they
are not part of `ECON-14`.

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
