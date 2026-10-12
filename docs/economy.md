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
8. Daily workplace pass (complete commute quotes are dropped first; bounded network-only legs
   remain valid until their CCH or lane revision changes).

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
- **Border quote preparation:** retain the existing per-border network tables for dispatch,
  and derive their exact per-node minimum once per routing revision or border-set change.
  This additional preparation costs O(nodes × borders), with O(nodes + borders) storage;
  subsequent building quote refreshes cost O(buildings), evaluating live frontage delay once
  per arrival end. The minimum uses the original table's f32 values and the original order of
  frontage/local additions, so fees remain bit-identical. It does not select a dispatch border.
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

`ECON-13` is done when all four checks pass on one build. The 1M tier stays deferred and is not
part of this acceptance.

1. **10k budget (`ECON-13-A`).** The recipe above measures at most **40 ms** per mean hour and
   **10 ms** per mean day, with identical `GrowthDayRecord`s across runs. The historical
   build below passed; the routing replacement currently fails the daily bound (14.33 ms).
2. **Linear 100k scaling (`ECON-13-B`).** On one build, `-- /100k` measures a mean operational
   hour at most 10× and a mean daily settlement at most 10× the same build's `/10k` values,
  with identical records across two runs. Routing work is tracked in
  [`NET-02`–`NET-05`](entrance_and_exit.md#cch-routing-performance-track-net-02net-05);
  that work alone does not close this scaling criterion. An older, economically inert fixture met the hour
   (3.1×) but not the day (15×). **FAIL** on the latest frozen routing build: hour 20.37×,
   day 84.18× in one complete pair. The earlier live-economy fixture measured 26× / 411× at
   `01888e88`; that historical result is retained below.
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
probes are diagnostic only and do not reject operations. Compilation, regressions and
matched measurements for this combined candidate remain pending.

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
