# Metrum Rise — Economy Design Spec

## Purpose

Metrum Rise needs an economy model that is believable enough to make sense, abstract enough to stay fun and usable, and efficient enough to scale. The system cannot live as a pile of hardcoded constants in Rust, and it also cannot depend on per-agent shopping behavior that explodes pathfinding and logistics cost.

This document defines a building-centric economy with the following design goals:

- support a closed production and distribution loop that feels believable to the player
- preserve the 1,000,000-population performance target through aggregation and bounded runtime rules
- keep the simulation understandable, so cause and effect are visible rather than hidden behind opaque formulas
- stay fun and easy to use, avoiding mandatory micromanagement and per-agent shopping chores
- give developers a visual tool for balancing and validating economic relationships without hand-editing numbers in files

## Core Principles

### 1. Buildings are the primary economic actors

Individual agents are not the main production graph nodes. Buildings, terminals, service facilities, and other concrete runtime facilities are.

Agents still matter, but mostly in three roles:

- workers that satisfy labor demand
- households that consume from shared household stock
- optional low-frequency leisure or shopping travelers

This keeps the hot path building-to-building instead of turning every staple good into an individual errand.

### 2. Household essentials are replenished periodically, not bought daily by individual agents

Food and household basics must not require 1,000,000 agents to pathfind to shops every day.

The default model is:

- producers create goods
- logistics carriers move goods to distribution nodes, warehouses, and shops
- residential buildings host one or more households
- each household holds one shared stock buffer for the whole household
- that household stock is replenished by occasional shopping or pickup in `v0.1`
- residents consume from household stock while at home

An agent's everyday need is therefore not "buy bread now" but "does my household have access to supplies at home."

### 3. Physical logistics matter

Goods do not teleport through the economy. If a transfer is local and meaningful to gameplay, it should be represented by a physical movement job across the `RegionGraph`.

Important exception for `v0.1`:

- networked utilities such as `power`, `water`, and `sewage` should not behave like trucked goods in the first pass
- they use the separate `Utility Service Layer` described later in this document

This creates the intended feedback loop:

- delayed deliveries reduce local stock
- low stock reduces household satisfaction or business throughput
- congestion becomes an economic problem, not just a traffic problem

### 4. Balancing and validation are visual; persistence is data-driven

Developers should use a tool, not raw text files, to balance production chains, controllers, and developer-authored scenario rules.

Persisted data files still exist for save/load, export, version control, and modding, but they are outputs of the economy tool rather than the primary authoring surface.

Future player-facing fiscal controls such as income tax, property tax, real estate tax, value added tax (`VAT`), tariffs, and subsidies are a separate gameplay policy layer. They must not be treated as raw access to the developer economy editor.

### 5. Runtime cost must scale by building, household, policy scope, and shipment count

The economy must scale primarily with:

- number of active buildings
- number of active households as the authoritative home-economy records
- number of active logistics jobs
- number of active policy scopes, if later gameplay or scenario systems add them

Derived per-building or future per-policy-scope summaries may aggregate those households for UI and coarse analysis, but those summaries are not an alternative source of truth.

It must not require per-tick per-agent inventory searches, market scans, or mandatory shopping trips.

## Economy Time Scale

The economy needs an explicit time scale so labor, household consumption, replenishment, and travel all fit together.

### Multiple clocks are required

One single clock is not enough for this game.

The design should use at least two separate clocks:

- `operational clock`: used for travel, work, deliveries, household consumption, wages, and production
- `demographic clock`: used for aging, school progression, and other life-stage changes

These clocks must not be treated as the same thing.

### Target day length

At `1.0x` speed, the target economy pacing is:

- `1 in-game day = 24 real minutes`
- `1 in-game hour = 60 real seconds`
- `1 in-game minute = 1 real second`

This is the design target for economy balancing. The current prototype clock may use a different placeholder value, but economy rules should not be authored against an ultra-compressed day.

### Why this scale is the target

This pacing is intended to keep:

- local errands in the range of minutes, not seconds
- normal commutes in the range of tens of in-game minutes to a few in-game hours
- long cross-city trips inside the same in-game day under normal conditions

If routine travel starts taking multiple in-game days, the time scale, travel speeds, or network assumptions are wrong.

### Economy cadence

The simulation does not need to update every economic rule every render frame.

Baseline `v0.1` cadence:

- movement and deliveries: continuous, on the normal simulation tick
- labor availability, production, and household consumption: evaluated on coarse sub-daily steps such as once per in-game hour
- household replenishment checks: every few in-game hours or when stock falls below a threshold
- wages, building operating costs, and daily summaries: settled once per in-game day

Authoring units should follow this scale:

- production and consumption: `units/day`
- stock: `days of supply`
- wages and operating costs: `currency/day` or `currency/workday`
- prices: `currency/unit`

### Daily demand handoff

The demand layer must not read half-updated hourly economy state.

Deterministic day-boundary rule:

1. Run the final sub-daily operational-clock economy step for the current day.
2. Run one daily economy settlement pass for that operational day.
3. During that settlement pass, finalize the day-level economy state that demand is allowed to read:
   - household budgets, stock, utility charges, and affordability results
   - household relocation, eviction, and `unhoused` outcomes owned by economy
   - building budgets, operating-buffer values, staffing or input shortfall state, and other
     building-side viability summaries
   - settled source values and city-level daily summaries from which demand derives its own
     normalized input signals, such as household-slot capacity and vacancy, housed residents,
     reachable open jobs, stock stability, utility-service satisfaction, and
     external-connection state
4. Freeze that post-settlement city snapshot.
5. Run the daily demand pass exactly once from that frozen snapshot.
6. Apply the resulting demand-owned daily outputs in this exact order before the next operational
   day's sub-daily economy steps begin:
   - execute `households_to_remove_today` first from the already-frozen settled household snapshot
   - execute the demand-owned private building actions next
   - execute `households_to_admit_today` after those building actions so fresh residential spawns
     may contribute same-boundary vacancy
   - run one lightweight post-admission workplace-assignment pass for newly admitted households,
     without rerunning daily settlement or the daily demand pass

Interpretation:

- demand reads one post-settlement city snapshot per operational day
- buildings or households created, removed, upgraded, downgraded, relocated, or evicted during
  that demand pass do not rewrite the same day's demand inputs
- those changes become part of the next operational day's economy state and therefore affect the
  next daily demand pass
- same-boundary admissions are not eligible for same-boundary removals, because removal executes
  first from the settled candidate list
- same-boundary fresh residential spawns may be filled immediately by admissions later on that same
  midnight boundary
- newly admitted households may receive workplaces before the first daytime departure window, but
  that post-admission assignment pass does not rewrite the already-frozen daily demand inputs

### Operational clock runtime state

The operational clock needs an explicit shared runtime representation so traffic, labor, deliveries, and schools all use the same time source.

Recommended state:

- `day_index`: current operational day
- `minute_of_day`: current minute since operational midnight, in the range `0..1439` where `0 = 00:00`, `60 = 01:00`, `720 = 12:00`, and `1439 = 23:59`
- optional sub-minute interpolation for smooth movement and rendering, without changing the authored minute-based schedule rules

`minute_of_day` is the main authoring and debugging unit. Runtime code may advance smoothly between minute boundaries, but authored schedules should not depend on second-level precision.

### Schedule windows and authored time data

Operational schedules should be authored as windows, not as one exact timestamp.

Useful schedule fields:

- `arrival_windows`
- `departure_windows`
- `active_work_windows`
- `shift_change_windows`
- `departure_spread_minutes`
- `reliability_buffer_minutes`
- `freight_timing_profile`

These windows should be stored as minute ranges from midnight on the operational clock.

Examples:

- office arrival window: `07:00-09:00`
- school arrival window: `08:00-08:30`
- factory shift changes: `06:00`, `14:00`, `22:00` with surrounding stagger windows
- freight timing profiles such as `always_open`, `night_preferred`, `early_morning_preferred`, or `daytime_receive`

This keeps authored data readable and avoids unrealistic one-frame mass departures. It also makes clear that freight timing should not be forced into the same daytime pattern as office or school travel.

For `v0.1`, freight timing should usually be modeled as a soft preference profile rather than a strict accept/reject delivery window. A night-preferred or early-morning-preferred site should still be able to receive freight outside its preferred period, but with less favorable congestion, priority, or operating-cost characteristics.

### Stable offsets and departure planning

Workers, students, and similar repeated travelers should not choose a totally new random minute every day. They should receive a stable offset inside the relevant schedule window unless a strong reason forces a resample.

This gives the simulation:

- repeatable personal routines
- natural stagger inside a shared schedule
- fewer synchronized spikes than exact building-wide timestamps

Planned departure should follow the rule:

- `planned_departure = target_arrival - estimated_travel_time - reliability_buffer_minutes`

So the clock defines when an arrival is desired, while routing and traffic determine how early departure must happen.

For `v0.1`, `reliability_buffer_minutes` is an authored constant on the relevant schedule or trip-purpose profile rather than a dynamic variance model.

Recommended first-pass seed values:

- office or daytime work: `15` minutes
- school: `10` minutes
- three-shift industrial work: `10` minutes
- freight pickup or delivery runs: `20` minutes

Implementation note:

- `estimated_travel_time` should be treated as a cached or periodically refreshed planning estimate, not as a mandatory fresh path query for every agent on every tick
- exact destination travel should reuse the existing `CCH` pathfinding layer
- shared-destination travel should reuse existing flow-field routing where that already fits the destination type
- any per-agent planning state such as cached commute estimate, planned departure, or lateness should live in the existing agent SoA layout rather than in a parallel economy-only data structure

The economy must not introduce a second routing stack. It should build on the pathing and agent-storage systems the project already has.

### Traffic affects arrival reliability, not the clock itself

Traffic is part of the operational timing problem, but it should not define schedules on its own.

The correct relationship is:

- the operational clock defines when work, school, and freight timing preferences occur
- schedule profiles define when buildings expect arrivals or shift changes
- traffic and pathing estimate how long the trip should take
- actual congestion determines whether the trip arrives on time, late, or not at all

This means traffic creates lateness, reduced staffed time, delayed deliveries, and missed replenishment windows. It should not create a separate special-purpose rush-hour clock.

### Rush hour emerges from overlapping windows

Rush hour belongs to the operational clock, but it should be represented as overlapping authored windows rather than as a hardcoded flag.

It should emerge from synchronized or semi-synchronized departure and arrival windows for:

- schools
- offices
- daytime retail
- any other workplace profile that clusters arrivals and departures into morning and evening windows

Rush hour should not be treated as a universal rule for all labor. Some sectors will contribute strongly to the peak, while others operate across the whole day with flatter traffic demand.

The runtime representation should therefore be:

- one shared `minute_of_day`
- schedule profiles authored as minute windows
- stable entity offsets inside those windows
- estimated and actual travel time layered on top of those windows

That gives the city visible rush periods without a separate magic `rush_hour = true` system.

### Aging and education use the demographic clock

The economy day is a scheduling and balancing unit on the operational clock, not a biological life-year.

It should not be assumed that:

- `1 economy day = 1 year of life`
- school progression advances one full year per economy day
- birthdays, aging, and life-stage transitions run on the same cadence as wages and household consumption

Instead, aging and school progression should run on the separate demographic clock.

On that demographic clock, it is acceptable for:

- `1 demographic day = 1 year of life`
- one school-year step to advance on the same demographic cadence

That gives the game a fast enough life-stage progression without breaking commute time, work schedules, deliveries, or household consumption on the operational economy clock.

The exact demographic implementation is outside the v0.1 economy scope, but the clock separation is a required design rule.

## Money Model

Money ownership should stay simple in the first economy pass.

### Households

Households own the money used for essentials.

- wages earned by workers flow into the shared household budget
- household replenishment purchases are paid from that shared budget
- household-side utility charges such as residential `power`, `water`, and sewage service may also draw from that shared budget in `v0.1`
- those household utility charges are service payments to the utility operator rather than automatic city revenue by default
- basic consumption should not require one separate wallet transaction per resident

### Buildings

Buildings own the money used for production and operations.

- sellers receive revenue when households or other buildings buy goods
- workplaces pay wages and operating costs
- **Solvency-Based Hiring**: Buildings may only offer open recruitment slots if their current `operating_budget` can sustain the daily wages of all existing employees plus the new hire. This prevents bankrupt businesses from functioning as "zombie employers."
- non-residential utility consumption and sewage-management charges should count as building operating cost in `v0.1`
- utility-producing or utility-processing buildings are normal economic operators that earn service revenue from those utility charges
- producers buy or reserve required inputs through the building-level economy

This gives the simulation a readable money loop without requiring every essential purchase to be modeled as an individual per-agent checkout event.

### City treasury

The city owns one explicit treasury ledger.

Rules:

- the city treasury is a separate ledger from household budgets and building budgets
- startup treasury funds initialize that ledger at game start
- income tax, property tax, real estate tax, `VAT`, tariffs, and similar city-owned fiscal inflows deposit into the city treasury
- ordinary utility service payments do not deposit into the city treasury by default; only any tax portion or future city-owned utility revenue would do so
- subsidies and other city-funded support measures withdraw from the city treasury
- road building, infrastructure placement, and city-owned facility construction withdraw from the city treasury

### Logistics and Shipments

The movement of goods and money is represented through explicit shipments:

- **Shipments**: Discrete logistics jobs that carry a specific quantity of resource between a source and destination.
- **Cooldowns**: Buildings enter a mandatory settlement period after starting a shipment to prevent overwhelming the road network with micro-deliveries.
- **Batching**: Both local trades and OWA exports prioritize efficient loads by waiting for a `min_shipment_units` volume before dispatching a vehicle.
- **Capital Lockdown**: While a shipment is in transit, the associated budget or inventory is locked and cannot be double-spent.
- **Fulfilment**: The transaction is credited only when the physical vehicle reaches its destination. Failures (e.g. building removal) return the locked capital but may penalize the reputation or cooldown of the building.
- roads and city-owned facilities also create recurring maintenance or operating costs that withdraw from the city treasury
- `v0.1` should treat these as simple treasury costs rather than as a full construction-material or contractor simulation
- future city systems such as deeper services simulation, public works, debt, or borrowing may also use this ledger, but those richer layers are outside the first economy pass

This makes fiscal policy a real money flow instead of a pure abstract modifier layer.

### Startup funds

The first economy pass should define explicit startup money instead of leaving early cash flow implicit.

- immigrating households arrive with starter savings
- the city starts with a modest startup treasury for early construction and city-level obligations
- newly created businesses begin with a small one-time startup float in their own building budget so they can purchase initial imported stock and cover the first wage cycle before local revenue stabilizes
- this startup float is private startup capital or owner equity, not a city grant and not a withdrawal from the city treasury

These are startup tuning values, not long-term substitutes for a functioning local economy.

### Infrastructure costs

Roads and civic infrastructure should not be free.

Recommended `v0.1` rule:

- placing a road or city-owned facility applies an immediate one-time capital cost to the city treasury
- each road segment and city-owned facility also applies recurring maintenance or operating cost
- recurring infrastructure upkeep posts on the normal daily fiscal settlement cadence
- if treasury funds are insufficient, the city may still build or continue operating by going negative rather than by silently ignoring the cost

This keeps infrastructure economically meaningful without requiring a full public-works supply chain in the first pass.

### City-owned building placement and ownership

City-owned buildings should be explicit player-built facilities, not spontaneous economy spawns.

Rules:

- when the player wants to add a city-owned facility, the player selects a buildable asset in the game UI and places it on the map at a valid location
- roads, utility producers or processors, and other civic facilities follow this explicit player-placement rule
- city-owned buildings do not spawn automatically through economy simulation
- if a city-owned utility or service building earns operator revenue, that revenue deposits into the city treasury because the city is acting as the operator
- private companies may establish or spawn businesses through simulation rules instead, using private startup capital rather than treasury-backed grants
- the city treasury pays build cost and upkeep for city-owned facilities, but it does not fund the startup float of private companies by default

### Fiscal settlement cadence

`v0.1` should use simple periodic fiscal settlement rather than per-frame accounting.

Rules:

- fiscal ledgers settle once per operational day
- taxes, tariffs, subsidies, and recurring city upkeep may accrue during the day, but they post to the city treasury on the daily fiscal settlement pass
- this daily settlement updates household budgets, building budgets, and the city treasury in one deterministic step

This keeps the first fiscal model understandable and consistent with the rest of the economy cadence.

### Value Added Tax (`VAT`)

`VAT` should be modeled as a buyer-paid consumption tax on goods purchases.

Rules:

- the budget-owning buyer pays `VAT` as part of the final purchase price
- for baseline household essentials in `v0.1`, this effectively means the household budget pays the tax when buying goods
- for business or operational purchases, the building budget pays the tax unless a later system introduces another explicit budget-owning buyer type
- seller revenue is the pre-tax sale value; the `VAT` portion is city tax revenue rather than normal seller income
- `VAT` liability may accrue during the day, but it settles into the city treasury on the daily fiscal pass

This keeps `VAT` tied to actual consumption instead of treating it as a vague background modifier.

### Treasury deficits

The city treasury may go negative.

Rules:

- subsidies and other city obligations are not hard-blocked by a zero treasury balance
- negative treasury is allowed as an explicit fiscal state rather than an invalid one
- future debt, credit, or fiscal-stress systems may add consequences later, but the first economy pass only needs to preserve the negative balance deterministically

This keeps the early city economy recoverable without needing a full public-finance simulation on day one.

### v0.1 local pricing and wages

The first internal pricing pass should stay intentionally simple.

Rules for `v0.1`:

- internal goods use fixed base prices authored in economy data
- workplaces use fixed wage values or fixed wage bands authored by profile or building class
- shortages should show up primarily through lower stock, delayed replenishment, reduced throughput, and unmet demand rather than through a fully dynamic local market
- bounded modifiers such as subsidies or delivery-cost effects may still change effective paid cost where appropriate
- free-floating local price response and free-floating wage response are out of scope for `v0.1`

This keeps the first economy pass understandable and stable while still leaving room for later market complexity.

## Startup Economy and Outside World Exchange

An empty map cannot start with a fully self-contained local economy. The economy needs a startup phase.

### Startup economy

At the beginning of a new city:

- the outside world acts as the initial source and sink for goods, money, and externally admitted households
- external trade requires at least one connected border connection
- early admitted households arrive with starter savings and immediately create household demand
- early shops and workplaces may operate in `OWA`-backed mode until local supply chains exist
- surplus may later be exported, but exports are not required to start the city

This prevents the economy from deadlocking on day one when no households, producers, or internal supply chains exist yet.

This section uses `startup economy` to mean early-city money, stock, freight, and `OWA` support. It does not own any special fresh-map building-placement exception, and it does not own the demand-side decision about whether new households should be admitted.

[`docs/demand.md`](demand.md) owns whether the city admits households at all and how many it admits. This document owns the startup money, stock, freight, and runtime consequences once those households already exist.

### Outside World Exchange (`OWA`)

For `v0.1`, the outside world should be represented by an `Outside World Exchange` (`OWA`) rather than by goods appearing magically inside city inventories.

Rules:

- the `OWA` is an abstract external buyer and seller, not a normal local factory
- imports and exports are available only when the city has at least one connected border connection
- connected border connections act as the physical ingress and egress gateways of the `OWA`
- the `OWA` may sell supported external goods without consuming local inputs and may buy exported city goods as an external sink
- the `OWA` may also provide missing utility service such as imported `power`, imported `water`, or external `sewage` processing as paid external services
- the `OWA` owns per-resource `import_ask` prices and `export_bid` prices rather than reusing local building prices directly
- `import_ask` must always remain above `export_bid` for the same resource so trivial buy-and-sell arbitrage cannot exist
- `OWA` price updates happen once per operational day with smoothing and bounded daily movement rather than instant per-order jumps
- in `v0.1`, only industrial zone buildings with surplus output may export to the `OWA`; commercial zone buildings do not export their outputs
- utility fallback through the `OWA` is an external service purchase, not a trucked-goods delivery
- payments for `OWA` utility fallback leave the local economy as external service spend rather than becoming city treasury revenue
- player tariffs may later modify effective trade cost, but tariffs do not replace the base `OWA` rules

This gives the city a startup source and a surplus sink without requiring a full intercity supply-chain simulation.

Shared outside-gateway boundary:

- the same connected outside connection may later also serve household arrival and household
  departure visualization
- that shared physical gateway should not collapse freight and household movement into one logic
  path
- `OWA` still owns external goods and utility-service exchange
- [`docs/entrance_and_exit.md`](entrance_and_exit.md) should own any later household
  `OutsideGateway` arrival or departure trip semantics
- [`docs/demand.md`](demand.md) still owns whether households are admitted or removed at all

### Border connections and physical freight

External trade must be physically delivered through the city, not teleported into a warehouse or shop.

This physical border-freight rule applies to ordinary imported and exported goods. It does not apply to `OWA` utility fallback service purchases, which are handled through the `Utility Service Layer` instead.

Recommended `v0.1` rule:

- imported goods enter the city only through explicit border connections
- when an import request is accepted, the corresponding freight is created or queued at a border connection and must travel physically to its destination
- exported goods must be transported physically to a border connection before leaving the city
- in `v0.1`, those border connections are the `OWA`'s border terminals
- `OWA` border terminals use queueing, dispatch, and active-job limits rather than behaving like infinite instant-delivery portals
- each border connection must cap queued external freight jobs and active dispatched external freight vehicles
- congestion on the road network and border-terminal queues are the primary throughput limits for `v0.1`

This keeps outside trade grounded in the same traffic and delivery logic as local freight.

### Local advantage over permanent `OWA` reliance

The outside world should keep the city alive, but it should not be the best long-term strategy.

Local supply chains should usually beat permanent `OWA` dependence through:

- lower effective unit cost once production and labor are stable — enforced by `owa_import_price_multiplier`
- less exposure to border congestion and border-terminal queueing
- less exposure to player tariffs and other trade penalties later
- stronger local employment and tax base

The logistics system tries local suppliers first and falls back to the `OWA` only when no valid local source is available. The `OWA` import price is derived as `local_unit_price × owa_import_price_multiplier` (configured in `[runtime_tuning]` in `economy/profiles.toml`), ensuring that a healthy local producer is always cheaper than the `OWA` alternative.

Exports work as a safety valve for surplus, not as the default engine of city growth. When an industrial building's unreserved output inventory exceeds a **one-day production buffer** and no local buyer is available, the logistics system creates an outbound export shipment to the nearest valid `OWA` border terminal. 

**Export Constraints**:
- **Pricing**: The `OWA` pays `local_unit_price × owa_export_price_multiplier` (default 0.6x), ensuring that local sales are always more profitable than "dumping" surplus on the external market.
- **Efficiency**: Exports must meet the building's `min_shipment_units` threshold and respect the building's global shipment cooldown. This forces industrial sites to batch their overproduction into meaningful truckloads rather than spamming tiny hourly export shipments.
- **Zoning**: In `v0.1`, only Industrial buildings may export; Commercial buildings do not export their inventories.

### Household admission and removal handoff

Household admission and removal affect labor supply, consumption, service load, and business viability, but the city-level decision about whether that change should happen belongs to [`docs/demand.md`](demand.md), not to this document.

For `v0.1`, the economy-side contract is:

- household admission and household removal happen at whole-household granularity, not one unrelated resident at a time
- economy creates and owns the admitted `Household` runtime record once demand has already decided the outcome
- admitted households receive startup state such as shared savings and household stock through the economy rules in this document
- the economy spec does not require a physically simulated border-entry transport visualization path in `v0.1`
- whether a later transport layer visualizes arrival or departure through border spawns or exits is a separate transport-layer decision
- births and other within-household demographic change are later systems, not part of the `v0.1` economy model

### Household housing affordability, relocation, and eviction

Household affordability failure should not directly downgrade a residential building.

Cross-system rule:

- this document owns whether a household can afford to stay in its current home
- this document owns household-side relocation between vacant homes inside the city
- [`docs/demand.md`](demand.md) owns whether a household ultimately leaves the city
- [`docs/zoning.md`](zoning.md) owns what buildings and levels are legally allowed on the site if
  later redevelopment happens

Important `v0.1` rule:

- `poor household -> move-out / relocation / eviction`
- not `poor household -> building instantly downgrades`

Recommended derived value:

```text
household_daily_essential_cost =
    household_supply_cost_per_day
  + household_daily_utility_charges

household_reserve_days =
    household_budget / max(household_daily_essential_cost, epsilon)
```

Economy-owned residential affordability data:

- `residential_move_in_min_reserve_days_by_level[level]`
- `residential_stay_min_reserve_days_by_level[level]`

Deterministic `v0.1` household housing rule:

- each housed household is checked on the coarse daily economy cadence
- a household may stay in its current home if
  `household_reserve_days >= residential_stay_min_reserve_days_by_level[current_home_level]`
- if that stay rule fails for the required sustained period, the economy layer must try relocation
  before declaring the household unhoused

Deterministic relocation rule:

1. Build the set of vacant candidate homes with available household capacity.
2. Keep only candidates whose target home level satisfies
   `household_reserve_days >= residential_move_in_min_reserve_days_by_level[target_level]`.
3. Sort the remaining candidates by:
   - higher `target_level` first
   - then lower travel distance from the current home if the household is already housed
   - then lower `building_id`
4. If the current home failed the stay rule, move the household to the first affordable candidate
   in that sorted list.
5. If the current home did not fail the stay rule, a voluntary up-move is allowed only if the first
   affordable candidate has a strictly higher `target_level` than the current home.

Current live note:

- the live runtime now loads `runtime_tuning.households` from `economy/profiles.toml`
- the daily household pass now executes the stay check, relocation, eviction, and `unhoused`
  transitions before demand reads the settled daily snapshot

Eviction and unhoused rule:

- if a housed household fails the stay rule and no affordable vacant home exists, the household is
  evicted from its current home and becomes `unhoused`
- becoming `unhoused` is not the same thing as immediate city removal
- the household remains an explicit runtime record until demand later decides whether
  `households_to_remove_today` should remove it from the city

Deterministic `v0.1` household-removal selection rule:

1. When demand produces `households_to_remove_today = N`, build the ordered removal candidate list
   from the settled economy snapshot after relocation and eviction have already run.
2. Add every `unhoused` household first.
3. Sort that `unhoused` candidate subset by:
   - lower `household_reserve_days` first
   - then lower `stock_days`
   - then lower `household_id`
4. If `N` is larger than the `unhoused` candidate count, append housed households sorted by the
   same rule:
   - lower `household_reserve_days` first
   - then lower `stock_days`
   - then lower `household_id`
5. Remove the first `N` households from that deterministic ordered list.

Interpretation:

- `unhoused` households leave the city before housed households when removal pressure is present
- if broader weak-city conditions require more outflow than the `unhoused` pool alone provides,
  the weakest housed households leave next in deterministic order
- demand still owns the count; economy owns the runtime household records and the deterministic
  removal target order

- one rich household in an apartment building does not level the whole building up
- that household may relocate to a better vacant home if one exists and is affordable
- one poor household in a good building does not level the whole building down
- that household first tries to relocate; only the household's housing state changes immediately

### Demand boundary and decisions system

The economy should separate coarse city-level demand pressure from concrete decisions made by agents, households, and buildings.

#### Demand system

The demand system should track aggregated pressures such as:

- household admission and removal pressure
- residential, commercial, industrial, and any later explicitly-added private-use growth pressure
- unmet goods or service demand
- broad city stability signals that other systems consume

This layer should operate mostly on coarse aggregate data rather than per-agent decision logic. The detailed city-growth and migration-pressure contract belongs in [`docs/demand.md`](demand.md).

Important scope note:

- baseline `v0.1` demand-owned private growth covers only residential, commercial, and industrial
- future office or mixed-use growth belongs there only if zoning and demand add them as one
  explicit extension
- city-owned service or utility building expansion is not ordinary demand-owned private growth; it
  remains player-, scenario-, or city-management-owned instead

#### Decisions system

The decisions system should resolve choices made by agents, households, and buildings, such as:

- whether an agent goes to work
- **Insolvency Exit**: If a building is insolvent and fails to pay wages, employees will "fire themselves" after a threshold period (default: 2 consecutive unpaid days). This frees them to seek solvent employment elsewhere.
- when a household replenishes stock
- whether replenishment uses shop pickup in `v0.1`, with future delivery modes added later if the design expands
- which supplier or route is selected
- which schedule window a workplace is currently filling

In `v0.1`, household replenishment should be represented as a household-side economy/request state flow rather than as a new `TRANSIT_*` state in the agent FSM.

The **Household Economic Model** is data-driven via the `basic_household_demand` profile:
- `consumption_rate_per_resident`: base units consumed per agent per day.
- `stock_target_days`: the ideal pantry size (default: 5.0 days).
- `reorder_threshold_days`: the trigger point for a standard restock (default: 2.5 days).
- `critical_threshold_days`: the "starvation" trigger for emergency restock (default: 1.0 day).

This layer operates on the operational clock and consumes the pressures produced by the demand system.

Short version:

- demand answers "what pressures exist in the city?"
- decisions answer "what does this household, worker, or building do about them?"

### Building upgrade and downgrade viability

Demand pressure alone must not be enough to level a building up.

Cross-system rule:

- [`docs/demand.md`](demand.md) decides whether a building is under enough sustained pressure to be
  eligible for upgrade or downgrade
- [`docs/zoning.md`](zoning.md) decides which next level is legal inside the current
  `upgrade_family` and zone profile
- this document decides whether that legal next level is economically viable

This prevents cases such as poor households upgrading directly into mansion-tier residential assets
just because broad residential demand is high.

Important `v0.1` rule:

- because the absolute money scale is still an open balance question, upgrade viability should use
  relative affordability and operating-buffer ratios rather than hardcoded absolute currency
  thresholds

#### Residential upgrade viability

For residential buildings, the economy-side gate should use aggregate building viability plus
household affordability at move-in or stay time. It should not use one occupant's wealth as a
direct whole-building level trigger.

Recommended derived values:

```text
residential_occupancy_ratio =
    occupied_household_slots / max(total_household_slots, 1)
```

Deterministic `v0.1` residential rule:

- each target residential building level must have one economy-owned aggregate viability
  requirement `residential_min_occupancy_ratio_for_upgrade[target_level]`
- a residential building is eligible to upgrade from `level = N` to `level = N + 1` only if:
  - demand says upgrade pressure is high enough
  - zoning says the next family level is legal
  - `residential_occupancy_ratio >= residential_min_occupancy_ratio_for_upgrade[N + 1]`
  - every occupied household in that building satisfies the target level's move-in affordability
    rule
    for the required sustained period
- a residential building is eligible to downgrade if:
  - demand says downgrade pressure is high enough
  - the building is vacant or near-vacant for the required sustained period
  - the economy-owned residential redevelopment-retention floor for the current level is no longer
    met
  - occupant poverty by itself does not trigger whole-building downgrade

Interpretation:

- residential upgrades are aggregate building-side changes, not one-household wealth events
- occupied households still matter, because the building must not upgrade into a tier its current
  households cannot afford to occupy
- residential decline is primarily a vacancy or redevelopment path, not an occupant-poverty path
- poor households are handled by the relocation and eviction rules above before whole-building
  downgrade is considered

#### Non-residential upgrade viability

For baseline `v0.1` commercial and industrial buildings, and for any later explicit office or
mixed-use extensions, the economy-side gate should use business viability rather than household
affordability.

Recommended derived values:

```text
building_daily_operating_cost =
    daily_wages
  + daily_input_cost
  + daily_utility_cost
  + other_daily_operating_cost

building_operating_buffer_days =
    building_budget / max(building_daily_operating_cost, epsilon)
```

Deterministic `v0.1` non-residential rule:

- each target non-residential building level must have one economy-owned operating-buffer
  requirement `nonresidential_min_buffer_days_by_level[target_level]`
- a non-residential building is eligible to upgrade from `level = N` to `level = N + 1` only if:
  - demand says upgrade pressure is high enough
  - zoning says the next family level is legal
  - `building_operating_buffer_days` meets the target-level requirement for the required sustained
    period
  - the building has no critical unresolved utility failure
  - the building has no critical unresolved staffing or required-input shortfall
- a non-residential building is eligible to downgrade if sustained demand and economy conditions
  fall below the current level's downgrade floor

Interpretation:

- high city demand alone does not force a shop or factory to level up
- the business must also be able to support the more expensive tier

#### Zone-type viability summary

The same layered rule applies to every ordinary private zone:

- demand says whether upgrade, downgrade, spawn, or redevelopment pressure is high enough
- zoning says whether the next level or replacement form is legal
- economy says whether that concrete zone type can actually sustain the change

Deterministic `v0.1` summary by zone type:

- `Residential`
  - upgrade requires aggregate residential occupancy viability plus occupant affordability for the
    target level
  - household poverty triggers relocation, eviction, or unhoused outcomes first
  - downgrade is primarily a vacancy or redevelopment path, not a one-household poverty path
- `Commercial`
  - upgrade requires commercial demand plus enough staffing, enough stock coverage, enough
    operating-buffer days, and no critical utility failure
  - downgrade requires sustained weak demand or weak business viability at the current tier
- `Industrial`
  - upgrade requires industrial demand plus enough staffing, enough required-input coverage, enough
    output clearance or storage headroom, enough operating-buffer days, and no critical utility
    failure
  - downgrade requires sustained weak demand or weak industrial viability at the current tier

Later-extension note:

- baseline `v0.1` does not ship ordinary private `Office` or `Mixed` growth families
- if office or mixed-use return later as explicit zoning and demand extensions, they should reuse
  the same business-viability pattern here rather than bypassing the layered
  demand-plus-zoning-plus-economy gate

Baseline `v0.1` modifier rule:

- crime, noise, parks, education, and similar neighborhood conditions are not hard economy-side
  blockers in the baseline upgrade gate
- those systems may later feed demand-side local modifiers or bounded viability multipliers once
  their source simulations are trustworthy enough to use as upgrade inputs
- until then, baseline `v0.1` upgrade and downgrade viability should rely on the simpler staffing,
  stock, utility, occupancy, affordability, and operating-buffer signals described above

Authoring and data rule:

- `residential_move_in_min_reserve_days_by_level`,
  `residential_stay_min_reserve_days_by_level`,
  `residential_min_occupancy_ratio_for_upgrade`, and
  `nonresidential_min_buffer_days_by_level` belong to economy-owned tuning data, not to zoning
  profiles and not to individual building assets
- any later commercial, industrial, office, or mixed-specific staffing, stock, input, output, or
  occupancy thresholds also belong to economy-owned tuning data rather than to zoning profiles or
  building assets
- `v0.1` may ship one simple shared table per use family or one shared table for all residential
  and one for all non-residential buildings
- if no required economy-side viability entry exists for a target level, that level transition is
  not allowed

Current live note:

- the live runtime now loads `runtime_tuning.viability` from `economy/profiles.toml` and applies
  those thresholds during demand-owned upgrade and downgrade candidate selection
- residential level changes now use occupancy plus occupant-affordability gates in the live
  runtime
- commercial and industrial level changes now use staffing, operating-buffer, and
  utility-availability gates in the live runtime
- the live runtime now uses typed per-resource building inventories and per-resource shipment
  reservations instead of the old `stock` / `input_stock` split
- industrial viability now reads explicit input coverage and output headroom from that typed
  inventory state
- the remaining later work is no longer inventory generalization itself; it is broader fiscal,
  utility, and content-side expansion on top of the generalized runtime

## Product Shape

The economy editor should be a separate developer tool, built in the same Godot + Rust tool family as the game and asset editor.

Recommended shape:

- `metrum_rise_game`: play and inspect a live city
- `metrum_rise_asset_editor`: author assets and their economic interfaces
- `metrum_rise_economy_editor`: internal balancing, validation, and debugging tool for production graphs, controllers, recipes, and scenario overrides

This may exist as a separate executable or as a developer-only launch mode inside one shared application family. The important part is the responsibility split, not the packaging name.

The economy editor is not part of gameplay. Players should not be wiring production graphs or changing balancing variables from the live game UI.

If a future gameplay policy system introduces player-painted areas, that should live in the game UI rather than in the core `v0.1` economy editor workflow.

### Why it should be a separate tool

- The live game is too noisy for serious authoring. Traffic, weather, zoning churn, and population motion make systematic economy editing harder.
- The asset editor already has a narrow job: import, validate, preview, and package content assets. Economy graph authoring is a cross-asset systems task, not per-asset metadata editing.
- A dedicated developer tool can provide graph editing, scenario playback, and bottleneck visualization without inheriting the full gameplay shell.

### What still belongs in the game

The runtime game should still expose economy inspection tools:

- stock and shortage overlays
- route and shipment debugging
- player policy summary, when a gameplay policy layer exists
- building-level throughput, staffing, and utility-service inspectors

But the live game should remain read-only for developer-side economy tuning. It can expose inspection and diagnostics, and later a separate bounded policy UI, but not graph authoring or raw controller editing.

### Control boundaries

To keep authoring and gameplay cleanly separated, the economy should use three distinct control layers.

- `Developer authoring layer`: economy profiles, recipes, controller formulas, default coefficients, allowed policy ranges, and scenario overrides used for balancing and validation.
- `Simulation-owned outcomes`: wages and labor prices, staffing pressure, production throughput, utility-service availability, household demand, and delivery cost outcomes. These are calculated by the simulation and are not direct player inputs.
- `Future player policy layer`: curated fiscal or trade levers such as income tax, property tax, real estate tax, `VAT`, tariffs, and subsidies. Players change bounded values or presets in gameplay UI, not raw controller graphs or balancing formulas.

If a future player policy is backed by a controller, the controller is still authored by developers. Gameplay should only expose named policy inputs and allowed ranges, never the full controller graph.

## Responsibility Split

### Asset Editor

The asset editor defines the asset's stable identity and base building metadata. It should stay focused on importing and packaging assets, not on authoring economy recipes.

Examples:

- a residential building asset declares `household_capacity`
- a workplace asset defines `worker_capacity` (either in `asset.toml` or authoritatively in its bound economy profile)
- an asset may store one `economy_profile` reference that points at an existing live economy profile
- lot size, service class, and similar building facts remain asset-authored metadata
- those values may be derived from floor area or other building-shape logic inside the asset toolchain

The asset editor does not define city-wide wiring, future gameplay policy geography, recipes, inputs, outputs, or economy balancing rules. It only stores the profile reference, not the profile definition itself.

The asset editor should list or suggest currently available economy profiles from the live economy data. Asset importers should not be expected to invent new profile names ad hoc.

The shipped game/editor should include a baseline economy profile catalog for asset creators. When new profiles are added, creators may need the latest exported profile list or a newer game/editor build to stay in sync. If the local profile catalog is missing or outdated, the asset editor should warn clearly and degrade gracefully rather than blocking general asset import work.

If an asset references an unavailable `economy_profile`, the asset editor should mark that field as unresolved and surface a validation warning. It must not silently replace the missing reference with another profile name.

### Economy Editor

The economy editor is a developer-facing balancing and validation environment. It defines economy profiles and relationships between economic actors, then helps catch systemic design mistakes before those rules ship into the runtime.

Examples:

- which reusable economy profiles exist
- which producer classes can supply which consumer classes
- which controller formulas back wages, taxes, subsidies, later tariffs, or household replenishment rules
- which scenario-specific overrides apply in one test setup but not another
- which goods are required for household stability versus optional quality-of-life supply

It is also the main developer surface for validating and debugging shortages, dead chains, impossible recipes, and other balance failures before those rules ship into gameplay. The economy editor is not a prototype player policy screen.

If a profile is renamed or deleted while still referenced by assets or authored economy content, the economy editor should show reverse-reference warnings before export. It must not silently remap dependent assets to a different profile.

#### Business Solvency Validation

The economy editor sandbox must validate financial solvency alongside physical logistics flow. A supply chain that circulates goods perfectly but leaves businesses fundamentally bankrupt is a failed economy design that causes "zombie businesses" at runtime.

During a sandbox playback, the editor must ensure that simulated business nodes are financially viable. 

Deterministic sandbox solvency rule:

1. For each simulation day and each profile node, calculate the `daily_labor_cost`. The sandbox assumes businesses are fully staffed, so `daily_labor_cost = worker_capacity * wage_max_currency_per_day`.
2. Calculate the `daily_input_cost` based on the actual units of input consumed that day, multiplied by the inferred unit prices of those inputs.
3. Calculate the `daily_revenue` based on the actual units of output produced that day, multiplied by the profile's `unit_price_currency`.
4. Calculate net daily profit: `daily_profit = daily_revenue - (daily_labor_cost + daily_input_cost)`.
5. Track the cumulative profit for each node over the duration of the scenario.
6. A scenario fails validation with an `insolvent_profile` error if any profile node finishes the sandbox run with a cumulative profit less than zero.

This guarantees that both physical volume bottlenecks and financial deficits are caught in the editor before the tuning values reach the live game.

### Runtime Simulation

The runtime consumes exported economy definitions and simulates:

- building inventories
- household stock buffers and replenishment state
- staffing and labor demand
- shipment creation and delivery
- utility service availability, local utility production or processing, and `OWA` utility-service fallback
- future policy-scope modifiers, if that layer is later added
- household satisfaction from shared household stock

The runtime should evaluate authored rules efficiently, not reinterpret a fully dynamic visual graph every tick.

### How assets connect to the economy

The link between content assets and the economy works in three steps:

1. The asset editor defines stable asset identity plus base building metadata such as capacities and stores an `economy_profile` reference.
2. The economy editor defines the reusable economy profiles that those references point to.
3. The runtime resolves the profile reference and combines the placed asset with the referenced economy profile to create the building's economic behavior.

Example:

- an asset with id `base:building.industrial.food_processor_small` declares base metadata such as `worker_capacity`
- the same asset stores `economy_profile = "food_processor_basic"`
- the economy editor defines a profile such as `food_processor_basic`
- that profile declares inputs, outputs, schedule profile, storage caps, and production rules
- when the asset is placed as a building, the runtime combines the asset metadata and the referenced profile
- the placed building then knows both its base capacity and its economic role in the wider supply chain

Short version:

- assets define identity and base metadata
- assets reference one economy profile
- economy profiles define behavior
- runtime building instances execute the combined result

Ownership and placement rule:

- city-owned buildings are created by explicit player placement in gameplay using valid buildable assets
- private company buildings may be established by simulation-side spawning or development rules instead
- both ownership paths still resolve through the same asset metadata plus `economy_profile` contract once the building exists in the world

### Failure handling for missing assets and profiles

Pack churn and economy-data churn are expected over the life of the project, so failure behavior must be explicit.

Rules:

- missing `asset_id` and missing `economy_profile` are different failure cases
- neither case may silently remap to a different asset or a different economy profile
- both cases must be visible in the asset editor, economy editor, runtime diagnostics, and save-load warnings

#### Missing asset

If a save references an asset whose pack is no longer available:

- the placed building becomes a `broken asset`
- placement data such as position, frontage attachment, and zone context are preserved
- the building does not participate in staffing or the economy while broken
- the runtime may use a visible fallback render such as the existing broken-building error mesh
- if the missing asset returns later, the building can recover without being rebuilt from scratch

#### Missing economy profile

If an asset still exists but its referenced `economy_profile` cannot be resolved:

- the placed building becomes `economy-broken`
- the visual asset may remain visible, but economy behavior is disabled
- the building does not produce, consume, hire, or satisfy household demand while `economy-broken`
- the runtime and tools must report the unresolved profile explicitly
- if the missing profile returns later, the building can recover automatically

The important rule is visibility and determinism. Missing bindings should degrade to an explicit inert state, not to a hidden fallback profile.

## Economic Data Model

The authoring model should use a small set of explicit object types.

### 1. Resource Types

Resources are the units that move through the economy.

Examples:

- `grain`
- `flour`
- `staple_food`
- `household_supplies`
- `fuel`
- `power`
- `water`
- `sewage`
- `construction_materials`

Rules:

- keep the v0.1 set small and legible
- prefer broad gameplay-relevant categories over excessive micro-goods
- split a resource only when the distinction creates meaningful logistics or policy gameplay
- not every resource type must use the same transport model
- ordinary goods such as food, fuel, and materials use the normal shipment and logistics rules
- utility resources such as `power`, `water`, and `sewage` use the separate `Utility Service Layer` rather than the normal freight-delivery rules in `v0.1`

### 2. Utility Service Layer

Utilities are a baseline city-service layer, not ordinary freight-delivery goods, in `v0.1`.

The first-pass utility layer includes:

- `power`
- `water`
- `sewage`

Rules:

- a building connected to the road network is eligible for service through the baseline city utility grid in `v0.1`
- access to the utility grid does not make service free
- occupied residential, commercial, industrial, office, service, and utility-operation buildings require `power` and `water` by default unless a documented special-case rule explicitly says otherwise
- `sewage` is a baseline generated utility load produced automatically by occupied buildings and households
- utility-producing and utility-processing buildings such as power plants, water plants, pump stations, or wastewater-treatment facilities should use normal asset-backed `economy_profile` definitions
- utility-producing and utility-processing buildings may be privately operated or city-owned
- most ordinary utility consumers do not need those utility ports repeated explicitly on every profile unless they have a documented special case
- households still do not own `economy_profile`, but occupied residential households consume utility service and generate `sewage` load as a runtime consequence of occupancy and activity
- local utility service must first be satisfied by local utility-producing or utility-processing buildings connected through this utility layer
- `v0.1` utility service is a connected-service on/off model, not an aggregate-capacity simulation and not a detailed line-by-line grid simulation
- if a valid connected local utility producer or processor exists for the required service, that service is treated as locally available to eligible consumers in `v0.1`
- if no valid connected local utility producer or processor exists, the service is unavailable locally and must either fall back to `OWA` or remain unserved
- the downstream production formula therefore treats resolved utility service as a binary building-level gate in `v0.1`
- `power` and `water` consumption should create paid utility service cost rather than behaving as free background access
- `sewage` generation should create paid treatment or management cost rather than being a free passive output
- residential utility and sewage charges post to household budgets in `v0.1`
- non-residential utility and sewage charges post to building operating budgets in `v0.1`
- those utility charges become revenue for the local utility operator or processor rather than for the city treasury
- if the utility operator is city-owned, that operator revenue deposits into the city treasury instead of a private building budget
- utility-producing and utility-processing buildings should therefore behave like ordinary economic buildings that sell a service rather than like invisible free infrastructure
- any `VAT` or other future fiscal levy on utility service is separate from the operator's service revenue and follows the normal tax rules into the city treasury
- if no local utility service is available, `OWA` may provide that service as an external service purchase
- if no local sewage processing is available, `OWA` may provide external sewage processing
- `OWA` utility fallback should remain a paid fallback and should usually be more expensive than healthy local utility provision
- these utility fallback purchases are not trucked freight and do not use the normal shipment-delivery model
- `sewage` must clear through the utility layer rather than remaining inside the building forever
- if a building lacks required utility service, or if generated `sewage` cannot clear, its normal operation should be blocked or degraded
- this baseline utility layer is a connected-service on/off model rather than a trucked-goods model in `v0.1`
- if no local utility producer or processor exists yet, the player may place a city-owned utility building or rely on `OWA` fallback until local provision exists
- city-owned utility buildings do not auto-spawn; only private companies may spawn new utility operators through simulation rules
- later versions may add explicit utility-network capacity, outages, or service-quality simulation

### 3. Economy Profiles

An economy profile is a reusable template owned by the economy editor and referenced by one or more assets.

It defines:

- input ports
- output ports
- storage capacity per resource
- schedule profile
- base production rules
- optional economy-side selectors or grouping rules

Example:

- `bakery_basic`
  - inputs: `flour`, `labor`
  - outputs: `staple_food`
  - variables: `base_cycle_time`, `input_buffer_cap`, `output_buffer_cap`, `schedule_profile`

Base capacities such as `household_capacity` remain asset-authored metadata. However, `worker_capacity` is authoritatively derived from the building's bound economy profile if one is present, overriding any value in the asset manifest. Living standards for households are defined by the asset's `flat_size_m2` (authored in `asset.toml`).

The baseline utility defaults from the `Utility Service Layer` apply unless a profile or building defines a documented special case.

Utility-producing and utility-processing buildings are ordinary profile-bearing facilities in this model.

Examples:

- `power_plant_basic`
- `water_plant_basic`
- `wastewater_treatment_basic`

Economy profiles belong to assets and runtime facilities, not to households themselves.

Rules:

- households are explicit runtime consumer records, not asset-authored profile owners
- households do not store an `economy_profile`
- if household demand must appear in economy-editor graphs, it should be represented as an abstract demand sink or consumer class rather than as a profile-bound asset

### 4. Economy Profile References

An economy profile reference lives on the asset side and points to one named economy profile.

Rules:

- the asset stores only the profile name or ID, not the full economy definition
- the asset editor should offer a live list or suggestions of existing economy profiles
- asset importers should select from existing profiles rather than inventing new profile names
- the shipped game/editor should provide a baseline profile catalog so asset creators have a stable starting set
- when that local catalog is outdated, the editor should warn and allow refresh to a newer profile list or game/editor version
- multiple assets from different asset sets may reference the same profile
- unresolved profile references must remain explicit invalid states until the correct profile data is available again
- no system should silently remap one profile reference to another profile
- tags may help editor search and filtering, but they should not be the primary economy contract

### 5. Economic Node Instances

An economic node instance is one placed building or facility in the world.

It holds runtime state such as:

- current inventory by resource
- assigned workers / filled jobs
- utilization
- local modifiers
- shipment reservations
- current shortage flags

### 6. Controllers

Controllers are authored simulation rule objects that modify behavior across many nodes.

Examples:

- wage-response controller
- tax controller
- price-response controller
- subsidy controller
- household restock cost controller

Controllers are not arbitrary scripts. They are bounded, inspectable systems with defined inputs, outputs, scope, and update cadence.

Some controllers may later expose a small set of bounded parameters to the gameplay policy layer, but players do not edit controller graphs or formulas directly.

`wage-response` and `price-response` are later extensions, not required for the `v0.1` baseline. The first pass can ship with fixed internal prices and fixed wage bands.

`tariff` controllers belong to later trade-policy extensions and are not part of the `v0.1` baseline.

Each controller definition should specify:

- what it reads
- what it writes
- whether it is global, future gameplay-area-scoped, profile-scoped, or asset-category-scoped
- whether it affects authored preferences or runtime state

### 7. Connections

A connection is an authored allowed relationship between node classes, resource types, or controller scopes.

In `v0.1`, this section mainly applies to normal shipped economic flows rather than to baseline utilities, which use the separate `Utility Service Layer`.

Important: a connection is usually not a literal per-unit hard route. In most cases it defines:

- permission
- preference
- priority
- filtering
- weight

The runtime then resolves actual suppliers, deliveries, and routes using those authored rules.

This is the key scalability rule. The tool authors the economic topology; the simulation executes a compact runtime form of it.

## Household Supply Model

The household model should be explicit and building-centric from the start.

### Households inside residential buildings are the consumer units

Residential buildings remain the spatial anchors for logistics, but stock is tracked per household, not per individual agent.

Rules:

- `household_supplies` for baseline living stability
- one stock buffer per household
- single-family homes naturally map to one household
- multi-unit residential buildings host multiple explicit household records, but never one stock buffer per resident

Residents draw from their household buffer while at home.

### Household runtime representation

Households should be explicit lightweight runtime records anchored to residential buildings.

This means:

- each household has its own runtime record rather than being merged into one anonymous building-wide stock pool
- each household record stores at least `home_building_id`, derived `member_count`, shared budget, household stock, and replenishment state
- agents reference a `household_id` for home-life needs and shared household money
- immigration, emigration, and move-in or move-out should default to household-level events rather than isolated individual moves; the economy spec does not require a separate border-entry bootstrap choreography for those members in `v0.1`
- if a later transport layer visualizes admitted or departing households through shared outside
  gateways, economy still owns the household record before arrival and the household-side removal
  reason before departure; transport owns only the trip choreography
- households may also contribute baseline utility load through the `Utility Service Layer`, but that load is a runtime consequence of occupancy and activity rather than something authored through a household `economy_profile`
- residential buildings still own the physical location and capacity, but they do not become the source of truth for each household's budget or stock

Source-of-truth rule for `v0.1`:

- linked resident agents are the authoritative source of household membership
- every resident belongs to exactly one household
- every household member is represented by a linked agent in `v0.1`
- `member_count` is derived from linked resident agents and may be cached for UI or save/load efficiency, but it is not authoritative
- non-agent residents are not part of the `v0.1` model
- if cached household population disagrees with linked agents, the linked agents win and the cache must be rebuilt

This keeps labor supply, school demand, housing occupancy, migration, and household budgeting tied to one deterministic population model.

This is still an aggregated model. It does **not** require a deep family tree, detailed relationship simulation, or one complex AI object per household. The runtime household record should stay as small and data-oriented as possible.

Household consumption rule for `v0.1`:

- `consumption_rate` is expressed in `household_supplies / day / resident`
- a household's daily baseline consumption is `member_count * consumption_rate`
- `stock_days` should therefore be computed against that household-level daily consumption rather than against a flat per-household constant

For performance:

- household logic should run on coarse economy cadence, not every render frame
- per-building summaries may be derived from linked households for UI and fast aggregate checks
- the authoritative source of truth for home stock, household money, and replenishment remains the household record itself

This gives the game a clean unit for budgeting, migration, save/load, and replenishment without falling back to per-agent grocery logic or muddy building-wide averages.

### Agent Need Interpretation

Agents do not need a daily "buy food" trip. Instead:

- being housed in a stocked household satisfies baseline home-life needs
- lack of household stock reduces happiness, stability, or health-related metrics
- optional leisure or personal shopping trips remain low-frequency and non-essential

This keeps daily essentials in the logistics layer rather than the pathfinding layer.

### Recommended v0.1 Resource Chain

For the first useful loop, do not start with dozens of goods. Start with one essential household chain.

Example:

- `farm` or `food_industry` produces `staple_food`
- `distribution_center` or `grocery` converts or forwards `staple_food` into `household_supplies`
- households replenish from `grocery` or `distribution_center` in periodic batches rather than per-person daily errands
- `household` consumes `household_supplies`

If that chain works, the broader economy architecture is sound enough to extend.

## Labor Model

Labor should remain the main direct agent-to-building economic link.

### Buildings demand labor

Workplaces expose:

- open job slots
- fixed wage offer or wage band in `v0.1`
- skill preference later if needed

### Work schedule profiles

Workplaces should not all share one global workday. Each workplace asset type should declare a `schedule_profile` on the operational clock.

That profile should compile to authored minute-based arrival, departure, active-work, and shift-change windows rather than to one exact building-wide timestamp.

Useful first profiles:

- `day_shift`: classic daytime work with strong morning and evening commute peaks
- `extended_day`: longer daytime operation with broader arrival and departure windows
- `two_shift`: partial evening coverage with a softer all-day labor curve
- `three_shift`: full 24/7 operation split across three labor windows
- `always_on_service`: hospitals, emergency services, and similar systems with constant staffing demand

This is especially important for industry. A `three_shift` factory should run 24/7, with workers distributed across all three shifts instead of all workers arriving in one morning rush.

Shift changeovers may still create local peaks, but they are smaller and more frequent than office-style rush hour.

Remote work may later exist for some high-skill office roles, but it is not part of `v0.1`. If added later, it should be modeled as a separate `work_mode` or job capability rather than as the default behavior for all labor.

### Agents supply labor

Agents decide whether to travel to work based on decision-utility scoring rather than a pure RNG cycle.

Early decision-utility inputs can stay simple:

- current money
- household stock situation at home
- commute cost
- job availability

Recommended `v0.1` work-decision formula:

```text
work_score =
    w_income  * income_pressure
  + w_stock   * household_stock_pressure
  + w_job     * job_availability_score
  - w_commute * commute_penalty
```

Where:

- all factors are normalized to `0.0..1.0` before weighting
- `income_pressure` is derived from the current household budget or reserve target
- `household_stock_pressure` is derived from current `stock_days` at home
- `job_availability_score` is `0.0` when no valid reachable open job exists and otherwise reflects the best currently available work option
- `commute_penalty` is derived from expected travel cost or time for the candidate job

Recommended seed weights for the first implementation:

- `w_income = 0.35`
- `w_stock = 0.35`
- `w_job = 0.20`
- `w_commute = 0.10`
- `go_to_work_threshold = 0.45`

Selection rule for `v0.1`:

- evaluate the score for reachable valid job options only
- choose the highest-scoring reachable job
- if the best score is at least `go_to_work_threshold`, the agent departs for work
- otherwise the agent stays in its non-work state for that decision pass

This keeps the first pass deterministic, bounded, and easy to debug. Richer nonlinear or probabilistic choice models can be added later if the design needs them.

### Building throughput depends on staffing

Production should derive from a bounded formula based on:

- filled worker count
- input availability
- required utility service availability through the `Utility Service Layer`, including `power`, `water`, and `sewage` clearance where relevant
- controller modifiers

Recommended `v0.1` formula:

```text
throughput = base_rate
           * staffing_factor
           * input_factor
           * utility_factor
           * controller_factor
```

Where:

- `base_rate` is the authored full-capacity output rate for the building or recipe
- `staffing_factor = clamp(filled_workers / worker_capacity, 0.0..1.0)`
- `input_factor` is the limiting required-input coverage for the current production step, clamped to `0.0..1.0`
- `utility_factor = 1.0` when the `Utility Service Layer` has resolved that required utility service is satisfied and generated `sewage` can clear for that building; otherwise `0.0` in the `v0.1` baseline
- `controller_factor` is a bounded multiplier from allowed controller effects

This keeps the first pass linear and readable. Hard minimum-staff step functions are not part of the baseline formula; if they are ever added later, they should be explicit profile-side rules rather than hidden default behavior.

This gives the player a meaningful connection between zoning, staffing, transit, and output without requiring arbitrary micromanagement.

## Logistics Model

### Shipment units

The simulation should create shipments at the building or terminal level, not one tiny packet per household resident. In `v0.1`, the only terminal-like freight gateways are `OWA` border terminals.

Each shipment should minimally contain:

- resource type
- amount
- source node
- destination node
- assigned carrier class
- status

### Carrier classes

Initial carrier hierarchy:

- trucks for local delivery
- later trains and ships for bulk long-distance transfer
- later airplanes only for special high-value chains

### Compression rule

One carrier represents a meaningful aggregate shipment, not one consumer purchase.

That means:

- one truck may represent many households' worth of supplies
- one train or ship represents many truckloads
- later internal bulk terminals split bulk flows into last-mile deliveries when necessary

### Demand accumulation and reorder thresholds

Shipments should not be created for every tiny consumption event.

Rules:

- destinations accumulate demand against a stock buffer rather than spawning a shipment immediately on every shortage
- a normal shipment request is created only when stock falls below a reorder threshold or when accumulated unmet demand reaches a meaningful batch size
- a smaller emergency shipment may be allowed below the normal batch threshold only when stock falls below a critical threshold
- shipment creation should run on a coarse economy cadence, not every render frame
- `reorder_threshold` is authored in `days_of_supply`
- `critical_threshold` is authored in `days_of_supply`
- UI may display equivalent percent-of-storage or absolute-unit values as derived information, but `days_of_supply` is the canonical authored format for stock urgency

This keeps logistics driven by buffer state rather than by micro-events.

### Minimum shipment size and carrier quantization

Each resource flow should have a practical minimum shipment size.

Rules:

- requests below the minimum shipment size should wait and accumulate unless they qualify as a critical replenishment case
- shipment sizes should be quantized to meaningful carrier loads rather than arbitrary tiny floating amounts
- a carrier job should represent a useful batch, not a one-item delivery unless the system explicitly treats it as a premium exception
- `minimum_shipment_size` is authored in absolute resource units, not percent-of-storage and not `days_of_supply`

This prevents the simulation from creating large numbers of meaningless micro-shipments.

### Outstanding request limits

Destinations and suppliers need hard caps so backlog count stays bounded.

Rules:

- a destination may have at most one open normal shipment request per `resource_type`
- an optional second request may exist only as an already assigned inbound shipment or an explicit emergency override
- suppliers must also cap total pending outbound reservations so one stockpile cannot over-promise itself to unlimited consumers

This keeps request count proportional to active economic nodes rather than to every individual stock tick.

### Reservation rules

Shipments must reserve both supply and demand explicitly.

Rules:

- when a shipment is created, the source reserves the promised stock immediately
- the destination reserves the corresponding unmet demand immediately
- reserved stock may not be sold twice, and reserved demand may not spawn duplicate requests
- if a shipment fails, expires, or is canceled, both reservations must be released deterministically

This prevents double-selling, phantom shortages, and duplicate jobs.

### Route creation

The authored economy graph chooses who is allowed or preferred to supply whom.

The runtime then resolves:

- which supplier has stock
- which consumer has demand
- whether a shipment is worth spawning
- which network path and carrier type to use
- the shipment ETA and border-terminal choice using the building entrance/access abstraction rather than a legacy edge-endpoint proxy

This keeps the simulation physical without forcing the editor graph to become a per-vehicle routing interface.

For ordinary trucked goods in `v0.1`, route cost and ETA must use the same exact car-access abstraction as the entrance cache:

- source and destination buildings are attached through their legal car frontage lanes, not through `building_depart_node()` or another one-endpoint shortcut
- same-edge direct frontage travel is valid when the exact attach and exact detach points lie in forward order on the same legal lane
- `OWA` / border fallback still uses a real border node as the external gateway, but the building side of that route must still use exact destination-side car access
- future freight/service systems may prefer authored `service` anchors, but until that extension exists the generic building entrance cache is the authoritative building-side freight attachment model

### Bounded supplier search

Supplier resolution must stay bounded.

Rules:

- runtime supplier search should start from a compatible-supplier index keyed by resource type and supplier class rather than by scanning all buildings
- candidate suppliers should then be gathered from nearby spatial chunks around the consumer first, reusing the project's existing bounded spatial-query patterns instead of introducing unbounded global scans
- nearby or already-preferred suppliers should be checked first inside that candidate list
- search should stop after a bounded chunk radius, a bounded candidate count, or both
- candidates that lack stock, fail authored compatibility rules, or fail route feasibility should be rejected before reservation
- for ordinary shipped goods, if no local supplier is valid, the system may fall back to the `OWA` when the economy rules allow it
- no request should perform an unbounded city-wide best-price scan

This keeps supplier lookup compatible with city scale and makes authored preferences matter.

### Retry cooldowns and failure states

Failed logistics work must back off instead of retrying every tick.

Rules:

- a failed request enters cooldown before it may search again
- retries should happen on coarse economy cadence or with explicit backoff, not every simulation tick
- after repeated failures, the request should escalate to a visible shortage or unresolved-demand state rather than spamming the same search forever
- every request should end in an explicit state such as `queued`, `reserved`, `in_transit`, `fulfilled`, `cooldown`, `expired`, or `failed_terminal`

This prevents retry storms and makes debugging easier.

### Household replenishment

Household replenishment in `v0.1` should use one fulfillment mode:

- periodic shopping or pickup, represented as an occasional household-level replenishment action rather than one trip per resident

Rules:

- replenishment is driven by the household stock system on coarse economy cadence, not by adding a new baseline `TRANSIT_*` movement state
- when stock falls below the household's replenishment threshold, the household creates a replenishment request
- that request reserves a valid supply source and then waits for pickup-side fulfillment
- on successful fulfillment, household stock increases and the request enters cooldown
- if fulfillment fails, the request follows the same bounded retry and cooldown rules as other economy requests

Useful first-pass household replenishment states are:

- `stable`
- `needs_replenishment`
- `reserved`
- `pickup_pending`
- `fulfilled`
- `cooldown`

This keeps the first household loop simple while still avoiding daily per-agent shopping.

If `ADS` is added later, it should be treated as a convenience layer:

- more expensive than normal shopping
- range-dependent carrier selection: nearby deliveries use pedestrians or bikes, while longer-distance deliveries use cars
- distance-based pricing: the farther the delivery origin is from the household, the more expensive the order becomes
- sensitive to congestion and local courier capacity
- more viable in dense, high-service areas than in sparse rural areas

### Household last-mile aggregation rule

Long-haul logistics should stay building-level, while household demand only enters the system at the last mile.

Rules:

- producer-to-distribution and distribution-to-shop flows should be batched building-to-building shipments
- households should trigger periodic replenishment demand, not one freight request per resident
- any future `ADS` mode should fulfill household demand as an occasional household-level event rather than permanent micro-shipment spam

This keeps household behavior believable without turning the freight model into a per-person courier simulator.

## Economy Editor UI

The economy system must be balanced and validated visually. Adjusting key numbers in text files is not acceptable as the primary workflow.

### Main views

The developer tool should have at least two coordinated views.

#### 1. Schema View

A graph canvas for reusable economic definitions.

Use it to author:

- resource chains
- controller placement
- allowed producer-to-consumer links
- default priorities
- conversion recipes

This is where a node-and-connection UI makes sense.

Example:

- the developer places a `food_processor` node with output `staple_food`
- the developer places a `grocery` node with input `staple_food` and output `household_supplies`
- the developer places a `household_demand_sink` node with input `household_supplies`
- the developer places a household stock or cost controller that affects replenishment pressure
- the graph then connects `food_processor -> grocery -> household_demand_sink`, with the controller linked to the household demand sink

At this stage the developer is defining the structure of the economy chain, not yet testing whether the numbers are balanced.

#### 2. Runtime Inspection View

A debug view for scenario playback and diagnosis of the authored balance rules.

Use it to inspect:

- stock levels
- blocked supply chains
- delivery latency
- unfilled labor demand
- controller effects
- shortage propagation

Example:

- the developer runs the `Grocery Bottleneck` test case for 30 simulated days
- the view shows that household stock drops below 1.0 days after day 12
- the diagnostics panel reports that the grocery has enough goods, but pickup-side replenishment demand is arriving in bursts and shop-side queueing is too high
- the controller panel highlights that household replenishment cadence and grocery throughput are misaligned
- the developer can immediately see that the problem is not food production, but local pickup balance and store throughput

### UI layout recommendation

Recommended shell:

- center: graph canvas
- left: resource and asset-category browser
- right: inspector for ports, variables, formulas, and controller settings
- bottom: warnings, validation, simulation log, and bottleneck list

### Editing workflow

The tool should allow a developer to:

1. pick an asset class or economic template
2. place or select a node on the graph
3. inspect its ports and variables
4. drag a connection from one output port to another node's input port
5. assign controller weights, caps, or policy overrides in the inspector
6. run a small scenario or sandbox playback to verify the chain

### Example developer setup

Example: `Grocery Bottleneck` test case

- Left panel: select the `Grocery Bottleneck` preset from a list of developer test cases.
- Center graph: show `food_processor -> grocery -> household_demand_sink`, with an optional replenishment-pressure controller connected to the household demand sink.
- Right inspector: expose values such as household count, household size, shop distance, pickup cadence, grocery throughput, and stock target.
- Bottom diagnostics: show stock days, average household cost, replenishment queue pressure, shortage warnings, and whether any recipe or connection is invalid.

In this example the graph, inspector, and diagnostics are enough to test whether local pickup and store throughput give the intended balance result.

### Validation requirements

The tool must validate common design mistakes before export:

- disconnected required inputs
- impossible recipes
- circular dependencies with no bootstrap supply
- scenario overrides that ban all legal suppliers
- throughput definitions that can never fill household demand
- assets that reference missing economy profiles
- profiles that are still referenced by assets or authored content but were renamed or removed

## Runtime Representation

The runtime should not execute the editor canvas directly. It should compile the authored graph into compact data tables.

### Rule export format

Economy rules should be exported as open, human-readable text files. They must not be hidden inside opaque editor-only data.

The canonical source of truth should be visible files in the exported pack or economy data folder, following the same philosophy as the asset editor manifests.

Recommended direction:

- use TOML as the canonical exported rule format
- keep the exported files readable and editable in a normal text editor
- treat any compiled or binary representation as optional derived cache only
- require the game and the economy editor to load correctly even when caches are missing
- regenerate caches whenever they disagree with the text source files

Manual editing is allowed. If a developer or modder wants to tweak the values in a text editor instead of the economy editor UI, that should be supported as long as the files still validate.

### Suggested exported structure

The exported economy structure is:

```text
economy/
  profiles.toml        # economy profiles and recipe definitions
  controllers.toml     # controller definitions and parameters
  scenarios.toml       # scenario overrides and test setups
  economy.index.bin    # optional derived cache
```

These filenames and this top-level folder layout are the baseline contract for the first implementation.

The important runtime rules are:

- text files are authoritative
- caches are derived
- exported economy data remains inspectable and editable outside the tool

Examples of compiled forms:

- resource IDs
- asset-type recipe tables
- controller parameter blocks
- supplier-consumer compatibility lists

This gives the tool freedom to be expressive while keeping the simulation runtime predictable.

## Scope Recommendations

### v0.1 must stay narrow

The first implementation should solve one closed loop well instead of sketching ten unfinished ones.

Recommended v0.1 scope:

- one essential household resource chain
- lightweight explicit household records anchored to residential buildings
- per-building production buffers and per-household stock buffers
- household stock consumption
- workplace labor demand
- fixed internal base prices and fixed wage bands
- simple city treasury-backed infrastructure build cost and daily maintenance
- baseline `Utility Service Layer` with local utility producers/processors and `OWA` external-service fallback
- truck-based local and border freight delivery with batched reservation-based shipment rules
- utility-scored work/home decision logic
- one dedicated economy editor shell with graph view, inspector, and validation

### v0.1 non-goals

Do not make these blockers for the first pass:

- personal retail trips as a daily need
- deep commodity markets with dozens of goods
- full dynamic local market pricing or wage response
- arbitrary user scripting inside controllers
- remote or hybrid work simulation
- full multimodal freight from day one
- world-scale intercity import simulation

### v0.2 and later

After the first household supply loop is stable, add:

- internal bulk terminals and bulk transfer
- gameplay area-based policy differentiation
- more resource classes
- service economy layers
- Automated Delivery System (`ADS`) home-delivery fulfillment
- richer regional trade layers beyond the `OWA`
- additional transport modes for freight

## Example Chain

A good starter chain for both simulation and developer-tool tuning is:

- `food_processor`
  - inputs: `labor`
  - outputs: `staple_food`
- `grocery` or `distribution_center`
  - inputs: `staple_food`, `labor`
  - outputs: `household_supplies`
- `household_demand_sink`
  - inputs: `household_supplies`
  - runtime variables: `household_size`, `stock_days`, `consumption_rate` (`household_supplies / day / resident`), `replenishment_mode`

In this starter chain, baseline `power`, `water`, and `sewage` behavior comes from the `Utility Service Layer` unless a building is meant to define a documented special-case utility rule explicitly.

Controller layers that may affect this example chain:

These controllers do not add new buildings or shipment steps. They are cross-cutting simulation rules that can modify cost, viability, or effective household access across one or more parts of the chain.

For `v0.1`, the example should stay within the fixed-price and fixed-wage baseline:

- `local subsidy`: reduces cost or improves viability for targeted chain steps
- `household restock cost`: changes the effective cost or friction households face when restocking supplies

Later extensions may add richer controller effects to the same chain, for example:

- `wage pressure`: changes labor-cost pressure at workplaces in the chain
- `price response`: applies bounded price-pressure adjustments to relevant chain steps

Replenishment for this chain should happen through periodic shopping or pickup in `v0.1`. `ADS` is a later extension, not part of the first implementation scope.

This example is intentionally broad. It avoids modeling "one loaf of bread per person per day" while still creating meaningful logistics, staffing, and shortage gameplay.

### Seed values for first implementation

The first playable implementation should ship with a small shared seed-balance set so the example chain is runnable before the economy editor is heavily used for tuning.

These are implementation defaults, not final balance targets:

- household `consumption_rate`: `1.0 household_supplies / day / resident`
- household replenishment target: `3.0 days` of stock
- household replenishment trigger: below `1.5 days` of stock
- household replenishment check cadence: every `6` in-game hours
- `food_processor` `base_rate`: `160 staple_food / day`
- `food_processor` worker capacity: `4`
- `food_processor` wage band: `90-110 currency / workday`
- `grocery` or `distribution_center` throughput target: `200 household_supplies / day`
- `grocery` worker capacity: `3`
- `grocery` wage band: `80-100 currency / workday`
- grocery stock target: `3.0 days` of supply
- grocery reorder threshold: `2.0 days` of supply
- grocery critical threshold: `0.5 days` of supply
- grocery minimum shipment size: `40 household_supplies`
- local base price for `staple_food`: `4 currency / unit`
- local base price for `household_supplies`: `6 currency / unit`
- `OWA import_ask` for `staple_food`: `7 currency / unit` (local × `owa_import_price_multiplier = 1.75`)
- `OWA import_ask` for `household_supplies`: `10.5 currency / unit` (local × 1.75)
- initial `OWA export_bid` for `staple_food`: `2.4 currency / unit` (local × `owa_export_price_multiplier = 0.6`)

**`OWA` import price implementation:** the runtime derives the effective OWA import price as `local_unit_price × owa_import_price_multiplier`. A value of `1.75` means the OWA charges 75% more than the local producer, making local supply chains economically preferred once they are operational. Values below `1.0` are rejected at runtime. The multiplier also applies to the `adjusted_unit_price` freight-timing modifier on top.

**`OWA` export price implementation:** when an industrial building has unreserved output inventory exceeding one day's production buffer and no local buyer is available, the logistics system creates an outbound export shipment. The OWA pays `local_unit_price × owa_export_price_multiplier`. A value of `0.6` means the OWA pays 60% of the local price, keeping exports a loss-reducing safety valve rather than a preferred revenue source. Values outside `[0.0, 1.0]` are rejected at validation time.

These numbers are only a bootstrap reference pack. They should ship in the first editable economy data so all implementations and test scenarios start from the same baseline before the editor-driven balancing pass diverges.

## Suggested Implementation Order

Cross-doc sequencing note:

- the shared cross-doc order is defined in [`zoning.md`](zoning.md): zoning and asset-editor
  foundation first, demand-layer integration second, economy integration third
- this section is therefore the economy-local implementation order that should run once the zoning
  and demand ownership contracts already exist

The codebase already ships part of the `v0.1` starter loop: explicit household records, simple
building budgets and stock, bounded freight reservations, `OWA` import fallback, and the first
developer-side economy data path. The phases below are therefore the recommended continuation order
from the current partial implementation, not a claim that every earlier phase is still untouched.

Current status summary:

- Phase 1 is complete in the live runtime.
- Phase 2 is complete in the live runtime.
- Phase 3 is complete in the live runtime.
- Phase 4 is complete in the live runtime.
- Phase 5 is complete in the live runtime.
- Phase 6 is complete in the live runtime.
- Phase 7 is largely complete already.
- Phase 8 is ongoing cleanup rather than untouched future work.

Recommended continuation order from the current runtime:

1. land Phase 5 treasury and fiscal settlement
2. then land Phase 6 utility/service runtime actors
3. keep Phase 8 cleanup running alongside those phases instead of treating it as one final isolated pass

### Phase 1 - Stabilize the current starter loop

- Treat the explicit-household plus bounded-freight path as the only authoritative `v0.1` baseline.
- Keep one essential chain authoritative: local producer -> local shop or distribution -> household stock.
- Do not widen scope into dynamic pricing, per-agent daily shopping, or broad multi-resource simulation yet.

Current status:

- complete
- explicit household records, bounded freight reservations, `OWA` startup fallback, household
  stock, building operating budgets, and the starter industrial input/output slice are all live
- this phase should now be treated as the settled baseline to build on rather than as active work

Goal: keep the already-landed economy slice small, testable, and worth building on.

### Phase 2 - Add the shared operational clock and schedule contract

- Introduce the shared operational runtime state described earlier in this document, centered on `day_index` and `minute_of_day`.
- Move work timing, household replenishment cadence, and freight timing preferences onto authored schedule windows and stable offsets.
- Cache or periodically refresh travel estimates instead of opening fresh per-agent path queries on the hot path.

Current status:

- complete
- runtime time now persists `day_index` plus `minute_of_day` instead of the old coarse day-only
  handoff
- household replenishment, building production, utility spending, and freight now run on authored
  operational-hour cadence derived from the shared minute clock
- workplace trips now read authored work windows and stable per-agent offsets instead of relying on
  one implicit global workday
- commute estimates are cached and periodically refreshed on the agent hot path rather than opened
  every tick

Goal: give labor, deliveries, and later school or service timing one deterministic time base before more systems depend on it.

### Phase 3 - Make authored economy profiles drive runtime behavior

- Resolve asset-side `economy_profile` references into compiled runtime tables during load and placement.
- Keep exported TOML plus the economy editor as the authoritative authoring surface and treat runtime caches as derived data only.
- Replace hardcoded starter-loop constants incrementally with profile-backed worker caps, rates, buffers, and fixed `v0.1` price or wage values.
- Preserve explicit unresolved-profile and broken-economy behavior instead of silent fallback.

Current status:

- complete
- asset-side `economy_profile` references now resolve into a compiled runtime economy catalog during
  placement, level changes, and save/load
- live buildings now carry derived runtime `economy_profile` bindings and enter explicit
  `economy_broken` state when authored profiles are missing or unsupported
- starter work schedules, freight timing, wages, prices, throughput, reorder thresholds, and
  viability gates now read authored economy-profile data instead of broad-zone starter constants
- the current runtime catalog now compiles typed input/output ports and profile-backed work or
  freight timing, while broader fiscal and utility layers remain later phases rather than Phase 3
  drift

Goal: stop duplicating economy rules between runtime code, packs, and the editor data model.

### Phase 4 - Generalize inventories and freight one resource at a time

- Move from the old starter stock-plus-industrial-input buffers toward fully resource-typed building inventories, reservations, and shortage state.
- Keep shipment creation bounded, batched, and entrance-aware; do not regress into per-order or per-agent freight.
- Expand to additional resources only after the starter household-supply loop still works cleanly on the generalized runtime.

Current status:

- complete
- live buildings now carry typed per-resource inventories instead of the old `stock` /
  `input_stock` split
- shipment reservations and in-flight freight are now tracked by `(building, resource)` rather
  than by building only
- the compiled runtime economy catalog now resolves authored resource ids plus typed input/output
  ports, so new production chains can reuse the same inventory and freight model without another
  runtime rewrite
- the shipped starter content still only exercises a narrow baseline household-supply chain, but
  that is now a content choice rather than a runtime inventory limitation

Goal: support more than one production chain without rewriting the logistics foundation again.

### Phase 5 - Add treasury ownership and daily fiscal settlement

- Separate household budgets, building budgets, and the city treasury into explicit ledgers.
- Land build cost, upkeep, utility charges, operator revenue, and later `VAT` or subsidy hooks on the daily fiscal settlement cadence.
- Keep `v0.1` pricing and wage response fixed while this ledger split stabilizes.

Why this is needed before Phase 6:

- The grocery store (`grocery_basic`) requires `staple_food` input to produce `household_supplies`.
  Until a producing building and freight chain exist, stores produce nothing and household stock
  drains to zero.
- Once stock hits zero, `household_stock_stability` collapses to `0.0`, which kills
  `city_stability_factor` and drives admission pressure to zero regardless of startup support.
  Population cannot grow past the first wave of immigrants.
- Phase 5 must introduce either a seeded starting inventory for stores or a no-input starter
  profile, so the first household supply loop closes before the full production chain is in place.
  Without this, startup support cannot bootstrap the city population as designed.

Current status:

- complete
- `CityTreasury { balance, lifetime_build_cost, last_daily_upkeep }` lives in `SimCore`
- startup balance initialised at `100,000` currency
- road placement deducts `100 currency/meter` from the treasury; balance may go negative per spec
- daily road upkeep deducts `0.1 currency/meter/day` on the daily fiscal settlement pass
- `grocery_basic` now spawns with `starting_inventory_days = 3.0` (600 units of `household_supplies`)
  seeded in output slots, closing the first supply loop before the full production chain exists
- save version bumped to 24; treasury is persisted in the `city_treasury` SQLite table
- `get_treasury_balance()` Godot func exposes the live balance for UI display

Goal: make money flow explicit before adding richer service or policy behavior.

### Phase 6 - Add the `Utility Service Layer` and first service-building slice

- Replace the current placeholder utility-availability behavior with connected local utility producers or processors plus `OWA` fallback.
- Make city-owned and privately operated utility or service buildings real economy actors rather than invisible background rules.
- Land `CIV-01` here so city stability is no longer only conceptual.

Current status:

- complete
- `EconomyProfileRuntimeKind::UtilityProducer` and `UtilityProcessor` variants added; `utility_service`
  field (`"power"`, `"water"`, `"sewage"`) propagated from authored TOML through compiled runtime profile
- three profiles landed in `economy/profiles.toml`: `power_plant_basic` (power, 4 workers, three-shift),
  `water_plant_basic` (water, 3 workers), `wastewater_treatment_basic` (sewage, 3 workers)
- `resolve_building_utilities` rewritten as a three-phase pass: (1) scan for active providers and
  determine service availability, (2) charge consumers at local rates (6.5/day total) when all three
  services are locally present, or OWA rates (8.0/12.0/day) otherwise, (3) distribute local revenue
  evenly to active utility providers
- `ensure_building_startup_float` extended to seed `STARTUP_OPERATING_FLOAT` for utility buildings
  (ZoneType::None with UtilityProducer or UtilityProcessor profile) so they can pay wages on spawn
- city assets for these profiles must be added via the asset editor; no invisible buildings exist

Goal: turn baseline services into real runtime constraints without treating utilities as trucked goods.

### Phase 7 - Complete the demand and economy handoff

- Assume the demand-owned outputs from [`demand.md`](demand.md) already exist as part of the
  earlier cross-doc implementation order.
- Finish routing household admission or removal and city-growth pressure through those demand-owned
  daily outputs instead of allocator-local formulas.
- Keep economy responsible for creating and updating household or building economy state once
  demand has already decided the outcome.

Current status:

- largely complete
- demand-owned household admission, removal, startup support, and daily building action plans are
  already integrated into the live runtime
- economy-side relocation, eviction, viability, and building-change gates already participate in
  that handoff
- any remaining work here is boundary hardening, not a missing first-pass integration

Goal: finish the demand and economy ownership boundary cleanly instead of leaving allocator-local or
economy-local fallback growth decisions behind.

### Phase 8 - Remove transitional hardcoding and old assumptions

- Delete remaining transitional zone-type-only economy branches and hidden fallback paths that
  bypass profile or runtime state.
- Rewrite tests, save/load expectations, and tooling diagnostics around explicit households, compiled profiles, utility or service resolution, and bounded freight.
- Keep the `v0.1` scope intentionally narrow even during cleanup; later market complexity should extend this model, not compete with it.

Current status:

- ongoing
- allocator-owned immigration heuristics are already gone
- the main remaining cleanup themes are broader profile-driven runtime resolution, richer utility
  actors, generalized inventories, and the building-loss displacement fallback still called out in
  the cross-doc cleanup work

Goal: finish with one coherent economy model instead of a mix of prototype and authored code paths.

---

## Unemployment Benefit

**Status: implemented.** The unemployment benefit is live in `households.rs`. All tuning parameters
are in `economy/profiles.toml` under `runtime_tuning`. The pioneer demand floor has been fully
removed from `demand.rs` — the benefit is the replacement and is active.

The unemployment benefit is a **household-level cash disbursement** paid to every unemployed member of an eligible household each operational day. It replaced the `pioneer_demand` floor as the mechanism that keeps households solvent during the early city bootstrap phase. Unlike the Pioneer floor, the benefit is a real simulation mechanism: money flows through the economy, stimulates real consumption demand, and generates real spawn pressure on commercial and industrial buildings.

### Ownership

This section owns the unemployment benefit spec. `demand.md` documents the (now removed) Pioneer floor. `households.rs` owns the runtime disbursement implementation. `nodes/sim/core.rs` owns the `CityTreasury` struct; starting balance is authored in `economy/profiles.toml`.

### Design Invariants

- The benefit is a household-level daily transfer, not a per-agent micro-payment.
- Money is drawn from the **existing `CityTreasury`** (`SimCore::treasury`). It is not printed from nothing.
- The benefit is self-terminating: once an agent is employed, disbursement stops for that household member. Once all household members are employed, the household exits the benefit entirely.
- The benefit must generate real purchasing activity. A household that receives the benefit must actually attempt replenishment at a grocery store if its stock is below the trigger threshold. The benefit amount must be large enough that this attempt succeeds at prevailing prices.
- The benefit must not create infinite runway. A household that cannot find work within the configured `unemployment_max_days` should emigrate rather than subsisting on benefit payments indefinitely.

### Money Source

`CityTreasury` already exists in `nodes/sim/core.rs` and is fully implemented:

- **Starting balance**: `startup_treasury_balance = 100_000` authored in `economy/profiles.toml` `[runtime_tuning]`.
- **Current deductions**: road build cost ($100/meter) and daily road upkeep ($0.1/meter/day); unemployment benefit disbursements.
- **Persisted**: saved and loaded via the `city_treasury` SQLite table.
- **Exposed**: `get_treasury_balance()` GDScript bridge already exists.

Unemployment benefit disbursements draw from the same `treasury.balance`. The disbursement
connection from `HouseholdSystem` to `CityTreasury` is live (`pay_unemployment_benefits` called
from `daily_settlement_tick`).

The treasury balance may go negative (existing behavior). Disbursement should be skipped once the balance reaches zero to avoid deepening deficit spending for welfare.

### Eligibility Rule

A household is eligible for unemployment benefit on a given day if **all** of the following hold:
- `household.member_count > 0`
- `household.home_building_id` is a valid, non-broken residential building
- At least one member of the household has `work_building == usize::MAX` (is unemployed)
- `household.unemployment_days_elapsed < unemployment_max_days`

`unemployment_days_elapsed` increments each day any household member remains unemployed, and resets to zero once all members are employed.

### Disbursement Rule

Once per operational day, after `pay_daily_wages` and before `resolve_household_housing`, iterated across all households:

```
unemployed_members = count of agents in household where work_building == usize::MAX
benefit_today = unemployed_members × unemployment_daily_benefit_per_member

if treasury.balance >= benefit_today:
    household.budget += benefit_today
    treasury.balance -= benefit_today
else if treasury.balance > 0.0:
    household.budget += treasury.balance   // pay what remains
    treasury.balance  = 0.0
// if treasury.balance == 0.0: skip silently
```

`treasury` here is `SimCore::treasury`, passed into `daily_settlement_tick` by the caller.

### Termination Conditions

| Condition | Outcome |
|---|---|
| All household members find employment | Disbursement stops; `unemployment_days_elapsed` resets to 0 |
| `unemployment_days_elapsed >= unemployment_max_days` | Household becomes emigration-eligible at normal removal priority; benefit stops |
| `treasury.balance <= 0.0` | Disbursement stops for all households; pioneer phase ends organically |

### Authored Tuning Parameters

`unemployment_daily_benefit_per_member`, `unemployment_max_days`, and `startup_treasury_balance` all live in the `runtime_tuning` block of `economy/profiles.toml`. `STARTUP_TREASURY_BALANCE` is currently a hardcoded Rust constant in `nodes/sim/core.rs` and must be migrated to the TOML tuning block as part of this implementation.

| Parameter | Location | Role |
|---|---|---|
| `startup_treasury_balance` | `economy/profiles.toml` runtime_tuning (migrate from Rust constant) | Total treasury at map start — currently hardcoded at $100,000 |
| `unemployment_daily_benefit_per_member` | `economy/profiles.toml` runtime_tuning (new) | Currency paid per unemployed household member per day |
| `unemployment_max_days` | `economy/profiles.toml` runtime_tuning (new) | Days before an unemployed household becomes emigration-eligible |

### Spawn Signal: Replacing the Pioneer Floor

The Pioneer demand floor (`pioneer_demand = 0.70`) currently exists because `stock_stab` and `afford` metrics collapse to near-zero on a fresh map, starving the spawn system of signal. The unemployment benefit restores these signals through real economic activity:

1. Disbursement gives households money → `afford` rises.
2. Households with money attempt grocery replenishment → `stock_stab` rises.
3. The grocery earns real revenue → absorption gate threshold is met sooner → second grocery spawns.
4. More groceries need supply → industrial spawn pressure rises.
5. Industrial buildings hire workers → households exit unemployment → benefit drain slows.

The pioneer demand floor has been removed from `demand.rs`. The unemployment benefit is the
replacement and is now the sole bootstrap mechanism.

### Shipped Tuning

Live values in `economy/profiles.toml` `[runtime_tuning]`:

| Parameter | Value | Role |
|---|---|---|
| `startup_treasury_balance` | 100,000 | Total treasury at map start |
| `unemployment_daily_benefit_per_member` | 15.0 | Currency paid per unemployed member per day |
| `unemployment_max_days` | 30 | Days before unemployed household becomes emigration-eligible |

## Building Bankruptcy

**Status: implemented.** The two-day `budget_distress` bankruptcy check is live in `households.rs`
(`run_bankruptcy_check`, `daily_settlement` four-step sequence). `budget_distress: bool` is
persisted in the SQLite schema and loaded by `world.rs`.

This section is the authoritative spec for how commercial, industrial, and utility buildings manage
their operating budget, pay obligations, and enter bankruptcy. The previous system used an hourly
utility gate (`utility_service_available`) that permanently froze any building whose budget dipped
below a single hourly charge — see ECON-01 in the Current Simulation Status section for the
incident record. This spec replaced that system entirely.

### Operating Budget

Each commercial, industrial, and utility building holds an `operating_budget: f32` cash balance.
It is separate from household budgets and the city treasury.

Money enters the budget from:

- sales revenue when households or other buildings purchase the building's output
- utility service revenue distributed to local provider buildings when consumers pay charges

Money leaves the budget from:

- daily wage payments to workers
- daily utility cost charged once per day on the same cadence as wages

The budget is allowed to go negative. A negative budget is not immediately fatal — it triggers a
distress window with a forced liquidation attempt before bankruptcy is declared.

### Startup Float

When a commercial or industrial building first spawns it receives a one-time startup float set at
construction time in the spawn path:

```
startup_budget = max(worker_capacity × average_daily_wage × STARTUP_RUNWAY_DAYS, STARTUP_OPERATING_FLOAT)
```

Constants: `STARTUP_RUNWAY_DAYS = 7`, `STARTUP_OPERATING_FLOAT = 500.0`.

No daily refill mechanism. The float is given once at spawn. If the building spends it without
becoming viable, the daily settlement sequence handles the outcome.

### Daily Settlement Sequence

The following steps execute once per day for every commercial, industrial, and utility building
that is not already `is_deserted` or `broken`. Order is fixed and deterministic.

**Step 1 — Bankruptcy check.**

```
if building.budget_distress AND operating_budget < 0:
    mark is_deserted = true
    exit sequence for this building
```

`budget_distress` is set at the end of the **previous** day's Step 4. Step 1 therefore asks: "did
yesterday end in distress, and is the budget still negative right now (before today's wages and
utility)?" If the forced liquidation on the previous day recovered the budget to ≥ 0, then
`operating_budget ≥ 0` here and the building is not bankrupt. `is_deserted` is never set on the
same day as the first negative budget — the building always gets one full distress day (including
the OWA sale attempt) before bankruptcy is declared.

**Step 2 — Pay wages.**

For each employed worker, deduct `daily_wage` from `operating_budget` and credit the worker's
household. If `operating_budget < daily_wage` for a given worker, that worker goes unpaid for the
day (`consecutive_unpaid_days` increments). Workers self-terminate after `JOB_UNPAID_ABANDON_DAYS`
(currently 2) consecutive unpaid days. Budget does not go negative from wage payments — a building
that cannot pay a worker simply fails to pay, not force-debits.

**Step 3 — Pay utility cost.**

Deduct the full daily utility cost unconditionally. Budget may go negative from this step.

| Zone type   | OWA rate (no local utility buildings) | Local rate (all three present) |
|-------------|---------------------------------------|--------------------------------|
| Commercial  | 8.0 / day                             | ~8.0 / day (local split)       |
| Industrial  | 12.0 / day                            | ~8.0 / day (local split)       |

When local utility providers exist, the collected cost is distributed to those provider buildings
as revenue. When no local providers exist, the cost leaves the simulation (OWA rate).

Residential buildings pay household utility costs from the household budget on the existing hourly
cadence and are not part of this sequence.

**Step 4 — Distress resolution.**

```
if operating_budget < 0:
    forced_owa_liquidation()   // sell all unreserved inventory at OWA prices
    budget_distress = true     // flag checked tomorrow in Step 1
else:
    budget_distress = false    // recovered: clear the flag
```

`budget_distress` is set to `true` whenever the budget ends the day negative, regardless of
whether the forced sale partially recovered it. The sale happens first; if it brought the budget
back to ≥ 0, Step 1 tomorrow will see `budget_distress = true` but `operating_budget ≥ 0` and
will not declare bankruptcy. If the sale could not recover the budget (empty inventory, or sale
revenue insufficient), Step 1 tomorrow sees both `budget_distress = true` and
`operating_budget < 0` — bankruptcy is declared.

`forced_owa_liquidation` iterates every output resource slot and sells the full unreserved
inventory at the standard OWA export price, crediting `operating_budget` immediately. It bypasses
the normal `min_shipment_units` buffer check — the sale is a distress action, not a scheduled
shipment. If inventory is empty (e.g. a ghost farm with no workers and no production), the
liquidation yields nothing and `budget_distress` is still set to `true`.

### Throughput Factor

`run_building_economy` fires each hourly tick and computes:

```
throughput_factor = staffing_factor × input_factor × output_headroom_factor
```

There is no `utility_factor` term. A building that is in budget distress continues operating
normally during the distress day — it still produces, sells, and receives wages. Only `is_deserted`
removes a building from all flows.

### Bankruptcy and Desertion

A building is permanently bankrupt when `is_deserted = true`. This is set by Step 1 of the daily
settlement sequence. Once set it is never cleared — there is no recovery from bankruptcy.

Deserted buildings are excluded from:

- all logistics supplier searches and shipment acceptance
- all hiring and worker assignment
- throughput computation (`run_building_economy` skips them)
- demand system capacity accounting (fixes ECON-02: frozen buildings no longer block new spawns)

The demand system is responsible for physically removing or redeveloping deserted buildings.

#### Worker ejection on bankruptcy

The daily call order must be:

```
daily_settlement_sequence (Steps 1–4, all buildings)
    → assign_agent_workplaces
```

`assign_agent_workplaces` runs after the full settlement pass so that all `is_deserted` flags set
during Step 1 are visible before job assignment begins.

At the start of `assign_agent_workplaces`, before the job-scoring loop, do a single ejection pass:

```
for each agent i:
    if work_building[i] != MAX and allocator.buildings[work_building[i]].is_deserted:
        reserved_workers[work_building[i]] -= 1
        work_building[i] = MAX
        job_lock_days[i] = 0
        consecutive_unpaid_days[i] = 0
```

Ejected workers then enter the normal job-scoring loop on the same day and can be assigned
immediately. Do not rely on the unpaid-wage path to clear these workers — that path takes two
additional days and leaves workers attached to a building that no longer runs throughput, producing
a misleading `worker_count` reading on the dead building.

`assign_agent_workplaces` currently does not filter on `is_deserted` in its candidate scoring
loop. This must be added: skip any candidate building where `is_deserted == true`, regardless of
whether the agent is already assigned there.

### Replacement Targets

The following code, fields, and constants are made redundant by this spec and must be removed
during implementation:

**`Building` struct fields to remove:**

- `utility_service_available: bool` — replaced by `is_deserted` for logistics gating; no longer
  used as a throughput multiplier
- `economy_dead_days: u32` — replaced by `budget_distress: bool` (simpler two-day rule)
- `startup_reset_used: bool` — the refill mechanism it guards is removed; startup float is set
  once at spawn

**New `Building` field to add:**

- `budget_distress: bool` — true if budget was negative at end of previous daily settlement

**Functions to remove or replace:**

- `resolve_building_utilities` Phase 2 (the per-building hourly deduction and gate-set loop) —
  replaced by Step 3 of the daily settlement sequence; Phase 1 (find utility providers) and
  Phase 3 (distribute local revenue) are retained and called from the daily path
- `run_desertion_check` — replaced by Step 1 of the daily settlement sequence
- `ensure_building_startup_float` — the daily refill scan is removed; float is set at construction

**Constants to remove:**

- `STARTUP_FLOAT_REFILL_THRESHOLD` — only used by the refill mechanism
- `DESERTED_THRESHOLD_DAYS` — replaced by the two-day consecutive rule
- `OPERATIONAL_HOURS_PER_DAY` divisor on utility costs — utility is now a flat daily charge

**Throughput formula change:**

- Remove `utility_factor` from `run_building_economy`; the term and the `if utility_service_available` branch that sets it are deleted

**Save/load schema:**

- Remove `utility_service_available`, `economy_dead_days`, and `startup_reset_used` from
  `BuildingSchema` in `save/schema.rs` and the corresponding `world.rs` serialisation and
  deserialisation paths. Add `budget_distress`.

## Legacy Cleanup Targets

As implementation starts, remove or refactor any code, tests, editor UX, or helper structures that still assume the older economy model rather than this spec.

The current spec replaces these legacy assumptions:

- per-agent daily shopping or `Home -> Work -> Shop -> Home` loops as the baseline essentials model
- implicit or aggregated household representation instead of explicit household runtime records
- global demand counters as the primary economy loop rather than as one coarse pressure layer
- probabilistic or RNG-driven activity selection instead of decision-utility scoring plus authored schedule profiles
- `ADS` or household home delivery as a `v0.1` baseline feature
- free-floating local price response or wage response in `v0.1`
- abstract external-market or throughput-budget trade models instead of `OWA` plus physical border freight for ordinary goods
- utilities as trucked goods or free background access instead of the `Utility Service Layer`
- `district` or gameplay-area-scoped economy-editor workflows in `v0.1`
- player-facing raw controller editing instead of a separate bounded policy layer
- auto-spawned city-owned facilities instead of explicit player placement for city-owned buildings
- city-grant startup funding for private businesses instead of private startup float or owner equity

## Current Simulation Status: The Pioneer Phase (v0.1.x)

As of the first full implementation of the agent-driven demand system, the simulation has encountered a specific "Pioneer Bootstrap Phase" deadlock that prevents organic growth without manual intervention.

### 1. ~~The "Salary Bomb" Deadlock~~ — Fixed

Startup capital is now computed as `max(500, worker_capacity × avg_daily_wage × 7)` for all commercial and industrial buildings at spawn. The same formula is used in the pre-revenue hourly refill (`ensure_building_startup_float`). A 16-worker farm at $100/day receives **$11,200** on spawn instead of $500 — enough to pay all workers for a full week. The `$500` floor still applies to low-wage or zero-worker buildings.

### 2. The "Starving Pioneer" Trap — Mostly Resolved

Immigrant households arrive with a starting budget of **$15/member** ($30 for a standard 2-person household).
- **Utility Drain**: Daily utility costs average **$6/day** ($3/member/day). Budget runway on utilities alone: ~5 days.
- **Starting stock**: 3 days of household supplies pre-loaded on spawn.
- **Gap**: Starting stock runs out around day 4. From day 4 to ~day 7 (first wages), households may be unable to restock.

With the salary bomb resolved, business wages reach workers by day 7. The 2–3 day starvation window (days 4–7) is the remaining residual of this trap and is acceptable for the pioneer phase. The circular deadlock that previously kept households permanently broke is broken.

### 3. ~~Circular Dependency Failure~~ — Resolved by fix #1

~~Because the starter households cannot buy, the **Grocery Store** earns $0 revenue. Because the store earns no revenue, it never places a paid shipment order with the **Farm**. The Farm remains at $0 revenue and continues to fail its wage checks, completing the deadlock.~~

With businesses now seeded with a full 7-day wage runway, this chain no longer jams on first boot.

### 4. The "Ghost Business" (The Bankruptcy Gap)

Currently, a business can reach a $0.0 budget and 100% unemployment but remain on the map indefinitely.
- **Logic Gap**: The `DemandSystem` only considers a building for removal (despawn) if **Demand Pressure** for that zone falls below the `despawn_threshold` (e.g., < 0.15).
- **Missing Trigger**: There is no "Liquidation" or "Bankruptcy" event triggered by the building's internal economy. Even if a business is economically dead, it is kept "on life support" by the city's overall scarcity signals.
- **Result**: The city results in a collection of non-functional shell buildings that take up valuable land but cannot produce goods or pay workers.

The authoritative spec for the **Deserted Building** lifecycle state — which resolves this gap — is in [`demand.md § Building Desertion`](demand.md#building-desertion). That section owns the trigger rule, all system effects, rendering contract, and data-model changes.

### 5. The "Starving Pioneer" Glue (Low Emigration)
Households that find themselves broke and starving are currently "trapped" in the city rather than emigrating.
- **Logic Gap**: The `pioneer_demand` floor (0.70) designed to attract the first wave of immigrants is also being used as a floor for **City Stability**.
- **Result**: Because stability never falls below 0.70, the calculated **removal pressure** remains artificially low. The simulation "protects" the pioneer wave so hard that they are unable to leave even when their economic situation is hopeless.
- **Calibration Target**: De-couple the bootstrap floor from removal calculations so that "despair-driven emigration" can function independently of "attraction-driven immigration."

### 6. ECON-01: Commercial/Industrial Budget Deadlock — No Recovery Path

**Observed**: In a 594-day run, the grocery (idx=22) entered a permanent freeze on Day 64 at `budget=-2.0`, `utility_service_available=false`. Eight farms entered the same state with `budget=0.0`. All remained frozen for 530+ days with inventory sitting unused.

**Mechanism**:
1. Hourly utility charge (`UTILITY_COST_COMMERCIAL / OPERATIONAL_HOURS_PER_DAY`) fires in `resolve_building_utilities`.
2. If `operating_budget < hourly_cost` → `utility_service_available = false`.
3. `utility_service_available = false` sets `utility_factor = 0.0` in `run_building_economy`, making `throughput_factor = 0.0` — no production, no sales, no revenue.
4. No revenue → budget never recovers → permanent freeze with no exit.

**Result**: A single budget dip below the utility threshold permanently locks the building out of the economy. 12 buildings deadlocked in the 594-day run. The grocery had 108 units of staple_food stuck in inventory the entire time.

**Fix direction**: Add a recovery path. Options: (a) allow operating debt up to a configurable threshold before cutting utility, (b) treat a `budget < 0` building as "utility suspended" but still allow it to earn revenue from stock it already holds, (c) make the utility cut gradual (reduce throughput proportionally rather than zeroing it). The right fix preserves pressure for well-capitalized buildings without creating an inescapable trap.

### 7. ECON-02: Absorption Gate Uses Nominal Capacity, Ignores Operational State

**Observed**: Only 1 commercial building (the initial grocery) was ever spawned across 594 days, despite 31 commercial candidates remaining available throughout. `spawns_today=1` was calculated for 402 of those days but no placement occurred.

**Mechanism**: `nonresidential_passes_absorption_gate` in `demand.rs` computes:
```
placed_capacity = sum of nominal output (units/day) for all non-broken, non-economy_broken buildings
consumer_demand = consumption_rate_per_resident × housed_resident_count
```
The grocery profile outputs 200 `household_supplies`/day. The `household_demand_sink` profile has `consumption_rate_per_resident = 1.0`. At 131 residents: `consumer_demand = 131`. Gate condition `placed_capacity < consumer_demand` → `200 < 131` → **false** → second grocery permanently blocked.

The gate does not check `utility_service_available`. A frozen, non-functional grocery still counts at full 200/day nominal capacity. The self-correction mechanism the economy needs (spawn a second grocery when the first fails) is blocked by the very building that failed.

**Fix direction**: Exclude buildings where `!utility_service_available` from `placed_capacity` in the absorption gate, or compare against actual effective throughput (`nominal × staffing_factor × utility_factor`) rather than nominal output.

### 8. ECON-03: One-Time Bankruptcy Reset Fires Repeatedly

**Observed**: Idle farms (workers=0, revenue=0) cycle through budget 3150→0→3150 on ~250-day cycles indefinitely. The startup float refill is intended as a bootstrap rescue but becomes a permanent subsidy for buildings that are locationally unviable (too far from residential for agents to commute).

**Mechanism**: `ensure_building_startup_float` fires every daily tick for any Commercial/Industrial building where `operating_budget < STARTUP_FLOAT_REFILL_THRESHOLD && revenue == 0.0 && worker_count == 0`. There was no guard preventing repeat fires.

**Fix**: Added `startup_reset_used: bool` field to `Building`. The reset now fires at most once per building lifetime (`FIXED` — `households.rs`, `allocator/mod.rs`, all construction sites, and the save/load schema). After the one-time rescue, a building that still cannot attract workers or earn revenue stays bankrupt permanently, which is the correct signal for the demand system to consider removal.

### 9. ECON-04: Commercial/Industrial Spawn Volume Scales with Road/Zone Area

**Observed**: Adding roads between two daily ticks caused commercial candidates to jump from 13 → 79 and industrial from 34 → 90, spawning 4 grocery stores and 5 farms in a single day — far exceeding the 0–1 that is normal when the road network is stable.

**Mechanism**: `normalized_spawn_pressure` in `demand.rs` is computed as a **sum** over all spawn candidates:

```rust
let normalized_spawn_pressure = spawn_candidates.iter()
    .filter_map(|c| profile.map(|p| normalized_positive_pressure(pressure, p.spawn_threshold)))
    .sum::<f32>();
```

Since all candidates of the same zone/density share the same profile and the same growth_pressure, each candidate contributes an equal fixed value. The total is therefore:

```
spawn_budget_units ≈ candidate_count × per_candidate_value × batch_fraction × spawn_limit
```

More roads → more zoned cells → more candidates → proportionally larger spawn budget. This is the wrong signal for commercial and industrial: the spawn rate should reflect economic demand (purchasing power, labour supply, output absorption) — not how much land was zoned this tick.

Residential is less sensitive because its `spawn_limit` is bounded by `housing_shortage²`, which is a real demand signal. Non-residential `spawn_limit` uses `resident_presence.max(pioneer_demand * 0.5)` and provides no candidate-count damping.

**Fix direction**: For commercial and industrial, replace the candidate-sum with either a single representative pressure value (max, or one sample) or a mean. The candidate list should gate *which* slots are eligible, not *how many* buildings spawn.

### 10. ECON-05: Pioneer Demand Floor Leaks into Non-Residential Spawn Rate

**Observed**: `spawn_limit` for commercial and industrial is `resident_presence.max(pioneer_demand * 0.5)`. At the pioneer baseline of `pioneer_demand = 0.700`, this floor is 0.35 — meaning even with zero residents the system keeps non-residential spawn pressure non-zero.

**Intended role**: Allows the first commercial and industrial buildings to appear before the population fully materialises, bootstrapping the supply chain. Once ECON-04 is fixed (spawn volume no longer scales with candidate count), this floor directly controls the pioneer-era spawn rate.

**Planned replacement**: The pioneer demand system (flat income floor, pioneer_demand coefficient) is scheduled to be replaced by an explicit unemployment-benefits mechanism. When that change lands, the `pioneer_demand * 0.5` spawn floor should be removed or replaced by a signal derived from the new system, since the concept of a "pioneer pressure" constant will no longer exist.

## Future Calibration Targets

Remaining open items for the pioneer phase:
- **Dynamic Wage Scaling**: Allow buildings to pay partial wages from available budget instead of stopping at the first worker the budget cannot cover.
- **Liquidation Logic**: Implement an "Economic Death" trigger — despawn a business that stays at $0 budget for a sustained period even when demand pressure is high (Ghost Business problem, issue #4 above).
- **Household bootstrap gap**: The 2–3 day starvation window (days 4–7, between starting-stock depletion and first wages) is the remaining residual from issue #2. Resolved by the unemployment benefit — see [Unemployment Benefit](#unemployment-benefit).
- **Pioneer Floor Retirement**: ~~Done.~~ Pioneer demand floor removed from `demand.rs`; unemployment benefit is the replacement and is live.



## Summary

The economy should be balanced and validated through a visual, building-centric developer tool, not through hardcoded numbers and not through gameplay UI controls.

The recommended design is:

- assets define identity, base metadata, and an `economy_profile` reference
- the economy editor lets developers tune graphs, controllers, and developer-only scenario overrides
- runtime simulation executes compiled building-level inventories, labor, and shipment rules
- `v0.1` uses fixed internal base prices and fixed wage bands rather than a full dynamic local market
- the city treasury covers simple road, infrastructure, and civic-facility build cost plus daily upkeep in the first pass
- wages and labor prices remain simulation outcomes rather than direct player inputs
- missing assets and missing economy profiles both degrade into explicit broken states rather than silent remaps
- logistics uses batched, reservation-based shipment rules with bounded supplier search and cooldown-based failure handling
- the `OWA` acts as an external buyer and seller, but all imported and exported goods still move through physical border freight
- future player policies such as income tax, property tax, `VAT`, tariffs, and subsidies use a separate bounded gameplay UI
- households consume shared household supply so agents do not need constant shopping trips
- fresh-map startup support and later private development remain bounded systems, and zoning alone must not spam empty buildings

That gives Metrum Rise a debuggable economy authoring workflow without violating the project's scale and performance constraints.
