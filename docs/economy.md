# Metrum Rise — Economy Design Spec

## Purpose

Metrum Rise needs an economy model that is believable enough to make sense, abstract enough to stay fun and usable, and efficient enough to scale. The system cannot live as a pile of hardcoded constants in Rust, and it also cannot depend on unbounded per-resident shopping behavior that explodes pathfinding and logistics cost. The baseline household-supply loop may use a bounded one-shopper carrier task per replenishing household.

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
- households that consume from shared household supplies
- optional low-frequency leisure or shopping travelers

This keeps the hot path building-to-building instead of turning every essential good into an individual errand.

### 2. Household essentials are replenished periodically, not bought daily by individual agents

Food and household basics must not require 1,000,000 agents to pathfind to shops every day.

The default model is:

- producers create goods
- logistics carriers move goods to distribution nodes, warehouses, and shops
- residential buildings host one or more households
- each household holds one shared supply reserve for the whole household
- that household supply reserve is replenished by occasional household-owned shopping in `v0.1`
- residents consume from household supplies while at home

An agent's everyday need is therefore not "buy bread now" but "does my household have access to supplies at home."

### 3. Physical logistics matter

Goods do not teleport through the economy. If a transfer is local and meaningful to gameplay, it should be represented by a physical movement job across the `RegionGraph`.

Important exception for `v0.1`:

- networked utilities such as `power`, `water`, and `sewage` should not behave like trucked goods in the first pass
- they use the separate `Utility Service Layer` described later in this document

This creates the intended feedback loop:

- delayed deliveries reduce local inventories
- low inventory or household supplies reduce household satisfaction or business throughput
- congestion becomes an economic problem, not just a traffic problem

### 4. Balancing and validation are visual; persistence is data-driven

Developers should use a tool, not raw text files, to balance production chains, controllers, and developer-authored scenario rules.

Persisted data files still exist for save/load, export, version control, and modding, but they are outputs of the economy tool rather than the primary authoring surface.

Player-facing fiscal controls such as tax sliders and household transfer levels are a separate
gameplay policy layer. Baseline income tax, household VAT, business profit tax, daily property
taxes, unemployment benefit, pension, and child support defaults come from authored runtime tuning,
then become live `CityFiscalPolicy` state exposed through curated bounded controls rather than
through the developer economy editor.

### 5. Runtime cost must scale by building, household, policy scope, and shipment count

The economy must scale primarily with:

- number of active buildings
- number of active households as the authoritative home-economy records
- number of active logistics jobs
- number of active policy scopes, if later gameplay or scenario systems add them

Derived per-building or future per-policy-scope summaries may aggregate those households for UI and coarse analysis, but those summaries are not an alternative source of truth.

It must not require per-tick per-agent inventory searches, market scans, or one shopping trip per resident. Low-frequency household-owned trips are allowed only when they stay bounded to one active shopper per replenishing household and reuse the normal building-origin trip planner.

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

This is the authored default for economy balancing. Clock construction uses validated tuning; it does not silently substitute another day length on configuration errors. Authored and saved day durations must both be finite and at least 60 seconds, validated without narrowing to `f32`. The supported live/save speed range is 0–32×; the shared Rust clock owns the existing UI steps (0, 0.5, 1, 2, 4, 8, 16, 32). These bounds keep a fixed 1/60-second runtime tick below 13 operational minute crossings (`AUDIT-01-C11`).

Trip estimators return physical travel seconds. The schedule cache converts those to operational
minutes using the live world's day duration; refresh intervals make the inverse conversion before
being added to physical agent simulation time. Employment ranking continues to use physical seconds.
Agent movement and scheduling borrow the same `TimeSystem` for date, minute and conversion rate.
The saved day duration remains authoritative when current tuning changes (`AUDIT-01-A2`);
configuration supplies the default for new worlds and the authored schedule/refresh intervals.
`AUDIT-01-A1` covers half, default and double day durations, including departure marks and refresh
deadlines. Five matched release process pairs (`benchmark_commute_cache_recording --ignored
--nocapture`, 24 Rayon workers, 11 samples of one million recordings) measure about 1.00 ms before
and 2.13–2.22 ms after the correction. This periodic O(1) arithmetic adds no allocation; it does not
measure route-search cost. Identities and raw logs are under `/tmp/metrum-full-audit/clock-*`.
The later live-clock ownership correction measures 2.172–2.178 → 2.409–2.425 ms per million
recordings in five CPU-0 matched release pairs. Full farm tick comparisons retain comparable
cost at 1,000/10,000/100,000 agents; 24-worker runs are noisy, while three CPU-0/one-worker
pairs give 1.375/1.775 → 1.322/1.717 ms for 100,000 idle/working agents. These are
movement-only fixtures with no route search, not whole-city economy timings; commands,
source/binary identities and raw logs are under `/tmp/metrum-full-audit/live-clock-*`.

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
- household replenishment checks: every few in-game hours or when the supply reserve falls below a threshold
- wages, building operating costs, and daily summaries: settled once per in-game day

Authoring units should follow this scale:

- production and consumption: `units/day`
- supply reserve: `days of supply`
- wages and operating costs: `currency/day` or `currency/workday`
- prices: `currency/unit`

### Demand Handoff

The demand layer must not read half-updated economy state. Hourly demand telemetry, household
admission, and private building actions run only after a completed operational-hour economy step;
daily removals run only after the daily settlement snapshot.

Deterministic day-boundary rule:

1. Run the final sub-daily operational-clock economy step for the current day.
2. Run one daily economy settlement pass for that operational day.
3. During that settlement pass, finalize the day-level economy state that demand is allowed to read:
   - household budgets, supplies, utility charges, and affordability results
   - household relocation, eviction, and `unhoused` outcomes owned by economy
   - building budgets, operating-buffer values, staffing or input shortfall state, and other
     building-side viability summaries
   - settled source values and city-level daily summaries from which demand derives its own
     normalized input signals, such as household-slot capacity and vacancy, housed residents,
     reachable open jobs, household supply stability, utility-service satisfaction, and
     external-connection state
   - candidate move-in inputs such as household starter savings, daily essential cost, exact
     candidate child/adult/elder composition, live transfer-policy amounts, treasury balance, and
     budget-backed open commercial or industrial jobs; demand owns the admission formula, while
     economy owns these source values
4. Freeze that post-settlement city snapshot.
5. Run the daily demand pass exactly once from that frozen snapshot.
6. Execute `households_to_remove_today` from the already-frozen settled household snapshot before
   the next operational day's sub-daily economy steps begin.

The daily agent-state pass retains its existing idle-in-building happiness bonus, nearest-cell
pollution penalty, happiness limits and nonnegative per-agent cash field. It now zips the existing
SoA arrays in Rayon with a minimum work length of 8,192; no additional agent state or per-agent
allocation is introduced (`AUDIT-01-A3`). Work remains O(A). Boundary/activity regressions exercise
8 and 32,768 agents, and matched release benchmarks verify identical result bits through one million.

Five matched process pairs on eight physical P cores (`RAYON_NUM_THREADS=8`, affinity
`0,2,4,6,8,10,12,14`) measure this daily pass:

| Agents | Before (ms) | After (ms) |
| ---: | ---: | ---: |
| 1,000 | 0.005727 | 0.008621 |
| 10,000 | 0.056888 | 0.067419 |
| 100,000 | 0.581686 | 0.090967 |
| 1,000,000 | 5.831440 | 0.893865 |

The 24-worker million-agent median improves 5.820677 → 0.649441 ms. The accepted tradeoff is
roughly 3–11 microseconds of extra daily work in the small physical-core fixtures and about 16%
overhead in the CPU-0/one-worker million-agent case (5.844838 → 6.762297 ms). These results concern
one daily state pass, not the full daily settlement or frame time. Initial fine-grained dispatch
was substantially worse for small collections and is retained only as diagnostic evidence.
The benchmark uses minimal agent SoA setup, a fixed pollution field, three warmups and 21 samples
of eight updates, resetting happiness outside timing. Command: the recorded test binary with
`--exact simulation::economy::agents::daily::tests::benchmark_daily_agent_update --ignored --nocapture`.
Identities, commands and raw measurements are under `/tmp/metrum-full-audit/environment-*`;
final comparisons are `environment-zipped-matched-bench.json` and
`environment-physical-matched-bench.json`.

Deterministic hourly-demand rule:

1. Run the completed operational-hour economy step.
2. Refresh RCI telemetry from that hourly state.
3. Advance `admission_action_credit` by `1/24` of the authored daily household-admission cap.
4. Execute `households_to_admit_today` immediately by launching household arrival carriers for
   available homes.
5. Advance private building-action credits by `1/24` of the authored daily building-action budgets.
6. Execute the selected demand-owned private building actions.
7. Do not execute household removal or daily settlement from this hourly pass.

At minute `00:00`, the hourly demand pass runs after the daily settlement and removal pass, so the
`1/24` cadence still produces 24 hourly demand slices per operational day without reading
pre-settlement midnight state.

Interpretation:

- demand reads one post-settlement city snapshot per operational day
- demand also reads completed operational-hour snapshots for RCI telemetry, household admission,
  and private building actions
- buildings or households created, removed, upgraded, downgraded, relocated, or evicted during
  a demand pass do not rewrite that pass's frozen demand inputs
- those changes become part of the next operational-hour economy state and therefore affect the
  next hourly demand pass
- hourly admissions are not eligible for same-day removals that were already selected from the
  settled daily snapshot
- fresh residential spawns may be filled by the next hourly admission pass
- newly arrived households may receive workplaces on a later economy tick, but admission and
  building actions do not rewrite the already-frozen hourly demand inputs

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
- city-owned service buildings are the municipal exception: their wages and placement costs are paid by the city treasury, and their local utility fees deposit into the treasury rather than into the building operating budget
- producers buy or reserve required inputs through the building-level economy

This gives the simulation a readable money loop without requiring every essential purchase to be modeled as an individual per-agent checkout event.

### Private building construction

Fresh demand-owned private building spawns enter the world as construction sites before becoming
operational buildings. The building record exists immediately so the parcel is claimed and the
renderer can show a site, but it is not live economy capacity until construction completes.

Rules:

- construction duration is authored in `economy/profiles.toml` under
  `runtime_tuning.construction`
- shipped starter durations are short and zone-aware: residential levels use `[6, 12, 18]` hours,
  commercial levels use `[8, 16, 24]`, and industrial levels use `[12, 24, 36]`
- an under-construction building has zero household capacity, zero worker capacity, no open jobs,
  no production, no shopping supply, no utility/service output, and no demand-live inventory flow
- under-construction output and housing capacity may be counted by demand as committed pipeline
  capacity when deciding whether to start more construction
- completion is checked on the coarse operational-hour cadence; when remaining construction hours
  reaches zero, the building becomes operational and can enter normal household, labor, logistics,
  and demand snapshots
- the first implementation applies construction only to fresh private spawns; upgrades,
  downgrades, and despawns stay instant until a later occupied-building redevelopment model can
  preserve residents, workers, inventory, and budgets safely
- construction visuals are MultiMesh-based: the renderer shows a dark compacted site pad, a raised
  neutral foundation, procedural scaffold bars, and the final building asset in its original
  material
- the final building asset must remain full-scale; construction progress is shown by translating
  that full-scale mesh upward from below the site until it reaches the normal finished transform
- the visual rise interpolates linearly through the current operational hour, even though
  construction completion and economy participation still advance on the operational-hour cadence
- construction must not recolor the final asset mesh, flatten it, or scale it like a reveal/balloon
  animation; progress is also exposed through diagnostics/inspection

This keeps growth visible and bounded without adding per-building Godot nodes or per-agent
construction behavior.

### City treasury

The city owns one explicit treasury ledger.

Rules:

- the city treasury is a separate ledger from household budgets and building budgets
- startup treasury funds initialize that ledger at game start
- income tax, daily property tax, household `VAT`, business profit tax, tariffs, and
  similar city-owned fiscal inflows deposit into the city treasury
- ordinary private utility service payments do not deposit into the city treasury by default; city-owned utility service payments do
- subsidies and other city-funded support measures withdraw from the city treasury
- road building, infrastructure placement, and city-owned facility construction withdraw from the city treasury

Baseline fiscal defaults live in `economy/profiles.toml` under `runtime_tuning.fiscal`. Runtime
simulation stores the active values in `CityFiscalPolicy`, which is persisted with the save and may
be changed by the player through the Economy Overview Policy tab.
There is no second hardcoded fiscal fallback. Tuning and the compiled catalog share one validated
startup file snapshot; invalid configuration reports an error instead of silently changing policy.

- `income_tax_rate`: fraction withheld from gross daily wages before households receive income
- `household_vat_rate`: fraction added to household store purchases
- `business_profit_tax_rate`: fraction charged daily on positive private business operating-budget
  growth after operating costs and settlement
- `residential_property_tax_per_home_per_day`: daily tax charged to each occupied residential
  household home
- `commercial_property_tax_per_building_per_day`,
  `industrial_property_tax_per_building_per_day`: daily tax charged to active private
  non-residential buildings in those zones
- `property_tax_level_multiplier`: multiplier applied per level above level 1
- `unemployment_daily_benefit_per_member`: daily transfer per unemployed adult within the
  unemployment time limit
- `pension_daily_benefit_per_elder`: daily transfer per elder
- `child_support_daily_benefit_per_child`: daily transfer per child
- `unemployment_max_days`: maximum unemployment-benefit runway before the household becomes
  removal-eligible through demand

### City fiscal policy

`CityFiscalPolicy` is the single live gameplay policy object for first-pass city finances.

Rules:

- every tax and household-transfer call site reads the current `CityFiscalPolicy`, not a fresh
  runtime-tuning load
- default values are initialized from `economy/profiles.toml` when a new simulation starts
- the policy is saved and loaded as authoritative simulation state; there is no compatibility shim
  for older saves in this slice
- `get_economy_overview()` exports both current policy values and UI metadata for bounded controls
- `set_economy_policy_value(policy_id, value)` is the runtime API for slider changes and clamps
  values to authored control bounds before mutating simulation state
- each accepted policy slider change emits an `economy` debug log line with the requested value,
  clamped value, treasury, current day/minute, and full fiscal policy snapshot
- daily budget history retains separate income-tax, household-`VAT`, business-purchase-tax,
  business-profit-tax, residential/commercial/industrial property-tax, unemployment, pension, and
  child-support buckets
- the Economy Overview Policy tab is a gameplay UI for taxes and social transfers; clicking one
  bounded control selects it and shows Today/7D/30D revenue or transfer-cost detail for that
  control's ledger bucket, while the economy editor remains the developer profile-authoring tool

Initial policy controls:

- transfer policy: unemployment benefit per adult per day, unemployment maximum days, pension per
  elder per day, child support per child per day
- wage and consumption taxes: income tax, household `VAT`
- business and property taxes: business profit tax, residential/commercial/industrial daily
  property-tax bases, property-tax level multiplier

### Logistics and Shipments

The movement of goods and money is represented through explicit shipments:

- **Shipments**: Discrete logistics jobs that carry a specific quantity of resource between a source and destination.
- **Cooldowns**: Buildings enter a mandatory settlement period after starting a shipment to prevent overwhelming the road network with micro-deliveries.
- **Batching**: Both local trades and OWA exports prioritize efficient loads by waiting for a `min_shipment_units` volume before dispatching a vehicle.
- **Capital Lockdown**: While a shipment is open, the associated budget or inventory is locked and cannot be double-spent. Source inventory is removed from the building only when the physical carrier is successfully dispatched.
- **Fulfilment**: The transaction is credited only when the physical freight vehicle reaches its destination endpoint. Failures (e.g. building removal, missing carrier, or in-transit timeout) return locked buyer capital when a buyer exists, restore dispatched local source inventory when the source still exists, and put involved buildings into cooldown.
- roads and city-owned facilities also create recurring maintenance or operating costs that withdraw from the city treasury
- `v0.1` should treat these as simple treasury costs rather than as a full construction-material or contractor simulation
- future city systems such as deeper services simulation, public works, debt, or borrowing may also use this ledger, but those richer layers are outside the first economy pass

This makes fiscal policy a real money flow instead of a pure abstract modifier layer.

### Startup funds

The first economy pass should define explicit startup money instead of leaving early cash flow implicit.

- immigrating households arrive with starter savings
- the city starts with a modest startup treasury for early construction and city-level obligations
- newly created businesses begin with a small one-time startup float in their own building budget so they can purchase initial imported inventory and cover the first wage cycle before local revenue stabilizes
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
- transaction-backed taxes update the city treasury when the underlying deterministic transaction
  succeeds: wage payment or store pickup
- business profit tax updates the city treasury during the daily settlement pass, after business
  budgets have absorbed that day's wages, utility charges, freight settlement, shopping revenue,
  and distress liquidation
- household transfer payments for unemployment, pensions, and child support post during daily
  settlement after wage payment and before housing affordability/relocation checks
- the treasury keeps pending tax buckets during the day and finalizes them into daily reporting
  buckets on the daily fiscal settlement pass
- the minute-0 operational-hour work and the midnight demand pass are part of the closing
  settlement boundary; finalized tax buckets are rolled after those deterministic phases complete
- recurring road upkeep posts on the daily fiscal settlement pass
- daily fiscal settlement updates household budgets, building budgets, and daily treasury
  reporting in deterministic phase order
- daily budget history reports household transfers as both a total benefits bucket and separate
  unemployment, pension, and child-support buckets

This keeps the first fiscal model understandable and consistent with the rest of the economy cadence.

### Income tax

Income tax is withheld from gross wage payments.

Rules:

- employers pay the full gross authored wage from their building operating budget
- households receive net wage after `CityFiscalPolicy.income_tax_rate`
- the withheld amount deposits into the city treasury as income tax revenue
- if the employer cannot pay the gross wage, no wage or income tax is paid for that worker
- wage payments apply in stable agent-index order after inactive employers have ejected workers

This keeps income tax tied to real employment instead of a background population modifier.

### Value Added Tax (`VAT`)

`VAT` should be modeled as a buyer-paid household consumption tax on goods purchases.

Rules:

- the budget-owning buyer pays `VAT` as part of the final purchase price
- for baseline household essentials in `v0.1`, this effectively means the household budget pays the tax when buying goods
- seller revenue is the pre-tax sale value; the `VAT` portion is city tax revenue rather than normal seller income
- household `VAT` is reserved in the gross shopping payment, but it is collected only once the
  shopper reaches the store and pickup succeeds
- business-to-business freight purchases are ordinary input trades and do not generate city tax
- daily fiscal reporting includes household `VAT` as its consumption-tax bucket

This keeps `VAT` tied to actual consumption instead of treating it as a vague background modifier.

### Business profit tax

Commercial and industrial buildings pay a daily tax on positive net operating-budget growth.

Recommended `v0.1` rule:

- `business_profit_tax_rate = 0.10`
- only active private commercial, industrial, and explicit field/extractor businesses are taxable
- broken, economy-broken, deserted, detached, or under-construction buildings are not taxable
- each building stores a daily profit-tax baseline equal to its operating budget after the most
  recent profit-tax settlement
- taxable profit is `max(0, operating_budget - profit_tax_budget_baseline)` after wages,
  utility costs, freight settlement, household shopping revenue, and distress liquidation have
  already posted
- tax is deducted from the building operating budget and deposited into the city treasury
- tax is capped to the building's positive operating budget and must not create a new negative
  budget after distress resolution has already run
- after settlement, the baseline resets to the post-tax operating budget so the same profit is not
  taxed again tomorrow
- there is no loss carryforward in `v0.1`; a loss day simply resets the next day's baseline lower
- startup floats are baseline capital, not taxable profit

This gives the city recurring revenue from profitable businesses without taxing gross sales or
making unprofitable starter firms fail faster than their actual cash flow warrants.

### Daily property tax

Property tax is a daily modeled money flow from private household/building budgets into the city
treasury.

Rules:

- residential tax applies once per occupied household home and is debited from that household's
  budget
- commercial and industrial tax applies once per active private building and is debited from the
  building operating budget
- explicit city-owned or `ZoneType::None` buildings do not pay private property tax
- the amount is selected from the matching live `CityFiscalPolicy` daily property-tax value and
  multiplied by
  `CityFiscalPolicy.property_tax_level_multiplier` for each level above level 1
- property tax must not be minted as treasury revenue without an equal household or building debit
- private construction start and construction completion do not charge a separate property tax

This gives the city recurring revenue tied to actual occupied homes and active private workplaces.
If a future one-time construction charge is needed, model it separately as a permit or development
fee rather than property tax.

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
- shortages should show up primarily through lower inventory coverage, delayed replenishment, reduced throughput, and unmet demand rather than through a fully dynamic local market
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

This section uses `startup economy` to mean early-city money, inventories, freight, and `OWA` support. It does not own any special fresh-map building-placement exception, and it does not own the demand-side decision about whether new households should be admitted.

[`docs/demand.md`](demand.md) owns whether the city admits households at all and how many it admits. This document owns the startup money, inventories, freight, and runtime consequences once those households already exist.

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

Local business and utility input demand also has priority over industrial exports. Before an industrial
building exports surplus output, the export planner computes affordable, non-terminal customer
input requests using the same truckload quantization, compatible-supplier index, component
reachability, and exact freight-route feasibility as inbound freight. Only demand that can be
served by a valid local supplier creates a source-specific output hold. Active inbound
reservations already count toward the buyer's expected inventory coverage, so this hold protects unmet local demand
without double-counting open freight jobs or blocking exports for disconnected/unreachable
buyers.

Daily demand snapshots read business and utility `OWA` input reliance as import-substitution pressure for
industrial growth. That pressure is diagnostic of actual outside-input use; industrial spawn
quantity remains guarded by the demand-side committed local input-capacity accounting in
[`demand.md`](demand.md).

Exports work as a lower-priced outside market for surplus. When an industrial building's
unreserved output inventory exceeds a **one-day production buffer** after local input holds and
its own same-resource input reserve, the
logistics system creates an outbound export shipment to the nearest valid `OWA` border terminal.
For explicit field producers and extractors, output inventory capacity uses the committed
area-scaled daily output, while the export buffer uses current active staffed output so a weakly
staffed starter farm is not forced to accumulate a full-field theoretical reserve before exporting.
`OWA` export demand is allowed to back explicit field/extractor staffing when scheduled exports are
enabled and the city has at least one connected outside freight gateway; local input holds and lower
`OWA` pricing preserve local buyers as the better market without making outside trade block
early-city job growth.

**Export Constraints**:
- **Pricing**: The `OWA` pays `local_unit_price × owa_export_price_multiplier` (default 0.60x), ensuring that local sales are always more profitable than selling surplus on the external market.
- **Saturation**: repeated fulfilled exports of the same resource reduce the effective export bid.
  Authored logistics tuning controls the truckloads needed to reach the floor, the floor factor,
  and the recovery hours. Queued, failed, expired, or still-in-transit export offers do not
  saturate the external market.
- **Distress pricing**: forced bankruptcy liquidation uses the separate `owa_distress_liquidation_multiplier`, which must be no higher than the scheduled export multiplier. The shipped value is lower than scheduled export pricing so fire-sale liquidation is a rescue path, not a preferred operating model.
- **Efficiency**: Exports must meet the building's `min_shipment_units` threshold and respect the building's global shipment cooldown. This forces industrial sites to batch their overproduction into meaningful truckloads rather than spamming tiny hourly export shipments.
- **Zoning**: In `v0.1`, only Industrial buildings may export; Commercial buildings do not export their inventories.

### Household admission and removal handoff

Household admission and removal affect labor supply, consumption, service load, and business viability, but the city-level decision about whether that change should happen belongs to [`docs/demand.md`](demand.md), not to this document.

For `v0.1`, the economy-side contract is:

- household admission and household removal happen at whole-household granularity, not one unrelated resident at a time
- economy creates and owns the admitted `Household` runtime record once demand has already decided the outcome
- admitted households receive startup state such as shared savings and household supplies through the economy rules in this document
- demand may read economy-owned starter savings, essential cost, exact candidate composition,
  transfer-policy amounts, treasury balance, and budget-backed job openings to calculate
  deterministic move-in acceptance; economy still owns the actual transfer payment and household
  materialization
- household admission does not require a physically simulated border-entry transport visualization path in `v0.1`
- whether a later transport layer visualizes household arrival or departure through border spawns or exits is a separate transport-layer decision
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
- eviction cancels an active grocery trip through the normal reservation lifecycle: before pickup,
  the household receives its reserved payment and the store regains reserved stock; after pickup,
  the settled sale remains paid. Cancellation is recorded once in the household ledger.
- becoming `unhoused` is not the same thing as immediate city removal
- the household remains an explicit runtime record until demand later decides whether
  `households_to_remove_today` should remove it from the city
- economy tracks `unhoused_days_elapsed` as the number of consecutive settled daily housing
  passes where the household still had no valid home after rehousing was attempted
- `unhoused_days_elapsed` resets to `0` when the household is housed or relocated, and starts at
  `0` on the day a household is first evicted
- demand owns the authored thresholds that interpret `unhoused_days_elapsed`, current `budget`,
  and the `stock_days` household supply-days value into persistent-exit eligibility

Housing audit (`AUDIT-01`, 2026-09-12): relocation reuses the economy chunk-ring iterator and
distance bound, visits only the ring perimeter, and does not clone level lists per household.
Scanning through radius `R` now visits O(R²) chunks instead of performing O(R³) interior checks;
candidate evaluation and existing chunk-index lookup costs are unchanged. Equal-distance candidates
on a later ring still participate in the building-ID tie-break.

Matched unprofiled release measurements used `RAYON_NUM_THREADS=24 cargo test --offline
--manifest-path rust/Cargo.toml --release --lib benchmark_housing_chunk_bounds -- --ignored
--nocapture`, 11 samples of 1,000 bound queries, without concurrent compilation or Godot work.
Median microseconds/query before → after: radius 8 `0.302 → 0.161`, 32 `4.954 → 0.639`,
80 `28.543 → 1.590`, 128 `71.441 → 2.541`. This measures the chunk-distance bound, not city FPS.
Source identities, baseline diff and logs are under `/tmp/metrum-full-audit/housing-*`.
The household suite freshly passes 102 tests (3 manual benchmarks ignored), including pre/post-pickup
eviction accounting and cross-ring distance ties; whole-codebase audit validation remains ongoing.

Household-removal index repair now composes chained swap moves and updates surviving agents once
in parallel: O(A + R) household-reference work and O(R) temporary mappings for A agents and R
removed households, instead of O(A × R). Candidate ranking and freight-carrier remapping are
separate costs. Resident and freight removal share the agent-record mutator; it repairs a moved
shopper's household reference immediately in O(1), preserving the active shopping reservation.

Matched unprofiled release removal measurements used `RAYON_NUM_THREADS=24 cargo test --offline
--manifest-path rust/Cargo.toml --release --lib benchmark_household_removal -- --ignored
--nocapture`, seven samples per size, 256 removed households and one resident per household.
Setup is excluded. Median milliseconds before → after: 1,024 households `1.547 → 0.113`,
8,192 `14.185 → 0.394`, 65,536 `116.395 → 2.535`. These fixtures have no freight orders;
they measure selection/removal and household-ID repair, not all possible emigration workloads.
Logs and pre-change source copies are under `/tmp/metrum-full-audit/removal-*`.

Deterministic `v0.1` household-removal selection rule:

1. When demand produces `households_to_remove_today = N`, build the ordered removal candidate list
   from the settled economy snapshot after relocation and eviction have already run.
2. Add every `unhoused` household first.
3. Sort that `unhoused` candidate subset by:
   - lower `household_reserve_days` first
   - then lower `stock_days` household supply-days value
   - then lower `household_id`
4. If `N` is larger than the `unhoused` candidate count, append housed households sorted by the
   same rule:
   - lower `household_reserve_days` first
   - then lower `stock_days` household supply-days value
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
- when a household replenishes supplies
- which household member, if any, carries a `v0.1` shopping replenishment task, with future delivery modes added later if the design expands
- which supplier or route is selected
- which schedule window a workplace is currently filling

In `v0.1`, household replenishment should be represented as a household-side economy/request state flow plus an ordinary building-origin trip for one selected household member. It must not add a new `TRANSIT_*` state to the agent FSM.

The **Household Economic Model** is data-driven via the `basic_household_demand` profile:
- `consumption_rate_per_resident`: base units consumed per agent per day.
- `stock_target_days`: the authored supply-reserve target, currently `5.0` in `economy/profiles.toml`.
- `reorder_threshold_days`: the trigger point for a standard restock, currently `2.5`.
- `critical_threshold_days`: the emergency restock trigger, currently `1.0`.

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
  - upgrade requires commercial demand plus enough staffing, enough inventory coverage, enough
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
  inventory, utility, occupancy, affordability, and operating-buffer signals described above

Authoring and data rule:

- `residential_move_in_min_reserve_days_by_level`,
  `residential_stay_min_reserve_days_by_level`,
  `residential_min_occupancy_ratio_for_upgrade`, and
  `nonresidential_min_buffer_days_by_level` belong to economy-owned tuning data, not to zoning
  profiles and not to individual building assets
- any later commercial, industrial, office, or mixed-specific staffing, inventory, input, output, or
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
  reservations instead of the old untyped `stock` / `input_stock` split
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

- inventory and shortage overlays
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

The current sandbox is a daily aggregate chain calculation. Its scenario controls are duration,
household count/size, starting pantry stock and explicit OWA import resources. The former scenario
target-stock, trigger-stock and pickup-cadence controls had no execution path and are removed;
live household replenishment remains owned by its runtime profile and operational-clock settings.
If several demand sinks are authored, validation warns and playback uses only the first authored
sink, excluding the others from both household consumption and incoming stock allocation.

Profile rates, wages and stock settings must be finite and nonnegative. Scenario duration must be
positive, average household size at least one, and starting stock finite and nonnegative.
Controller weights lie in `[0, 1]`; multipliers are finite, nonnegative and ordered. Startup
treasury must be finite, and export-saturation load/recovery settings finite and positive.

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
- household supply reserves and replenishment state
- staffing and labor demand
- shipment creation and delivery
- utility service availability, local utility production or processing, and `OWA` utility-service fallback
- future policy-scope modifiers, if that layer is later added
- household satisfaction from shared household supplies

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
- explicit farms are industry-area assets with `[building.field]`; the player places the farm building, then draws a nearby field polygon before the farm can produce its authored field resource such as `grain`; field output scales linearly by polygon area, with 10,000 m2 as the 1x authored-rate baseline

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
- `packaged_food`
- `household_supplies`
- `personal_services`
- `health_essentials`
- `coal`
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
- abstract commercial service resources such as `personal_services` and `health_essentials`
  represent staffed service capacity, not freight inventory
- `coal` is an ordinary shipped fuel resource in the starter utility loop
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
- utility-producing and utility-processing assets are explicit service buildings, not ordinary zoned-private assets; they must carry a utility `service_class` plus an `economy_profile` for the matching service
- utility-producing and utility-processing buildings may later be privately operated or city-owned; live explicit service assets are city-owned municipal facilities by default
- most ordinary utility consumers do not need those utility ports repeated explicitly on every profile unless they have a documented special case
- households still do not own `economy_profile`, but occupied residential households consume utility service and generate `sewage` load as a runtime consequence of occupancy and activity
- local utility service must first be satisfied by local utility-producing or utility-processing buildings connected through this utility layer
- `v0.1` utility service is not a detailed line-by-line grid simulation; `power` now uses aggregate daily produced units while `water` and `sewage` still use the starter provider-present fallback model
- utility availability resolves independently per service; a local `power` provider does not satisfy `water` or `sewage`
- each utility service settles through the same bill formula:
  `local_charge = demand_units * local_unit_price * local_coverage` and
  `owa_charge = demand_units * local_unit_price * owa_import_price_multiplier * missing_coverage`
- commercial, office, mixed, and industrial buildings consume one aggregate unit per day of each modeled utility service in `v0.1`; this is intentionally broad and profile-independent until utility demand becomes content-authored
- households keep their authored base utility cost split across power, water, and sewage ledger buckets; the daily utility settlement routes the locally covered share to local utility revenue and applies only the missing-service `OWA` surcharge to household budgets
- a valid connected local `power` producer contributes the service units actually accumulated during hourly operation from `base_rate_units_per_day * current throughput`, capped by fuel/input availability at those hours; end-of-day settlement must not credit unproduced capacity
- city service funding policies are runtime simulation state owned by Rust; the live electricity funding policy sets the default funded worker slots for city-owned power plants, and production follows the resulting staffed workers plus fuel/input availability
- live city and per-building funding controls reject NaN/infinite values without changing policy or staffing; finite values retain the existing `[0, 1]` slider clamp, and per-building controls reject non-finite pick coordinates
- individual city-owned power plants may carry a per-building funding override; citywide electricity funding changes do not clear those plant overrides
- if a valid connected local `water` producer or `sewage` processor exists and has positive current operational throughput, that service is treated as locally available to eligible consumers in `v0.1`
- if no valid connected local utility producer or processor exists for a service, that service falls back to `OWA` independently of the other utility services
- the downstream production formula still does not use a utility throughput gate in `v0.1`; utility failures are represented as local service coverage and external fallback cost
- `power_plant_basic` is coal-fueled in the starter runtime: it requests `coal` through ordinary freight logistics, can import coal from `OWA` while no local coal mine is producing reachable coal, and produces no local `power` when staffed but out of coal
- authored coal deposits can now be painted in WorldEditor; explicit coal-mine assets bind to `coal_mine_basic`, commit a player-drawn extraction polygon within 10 m of the building footprint, snapshot the enclosed reserve, consume that reserve into local `coal` output during hourly operation, persist both deposits and extractor depletion through city saves, and render committed pits through a terrain-shader coal-texture mask rather than a separate decal mesh; the committed area scales physical hourly output and physical worker capacity against a 10,000 m2 authored baseline, scheduled `OWA` exports can back area-scaled active worker slots when a connected outside freight gateway exists, and the first committed extraction area tops up startup operating budget to the area-scaled payroll runway
- explicit grain farms bind to `grain_farm_basic`, commit a player-drawn field polygon within 10 m of the building footprint, and produce renewable `grain` during hourly operation without consuming a map-authored resource deposit; daily output stays per hectare while worker capacity uses a separate authored staffing area (one worker per 100,000 m2 for grain farms), scheduled `OWA` exports can back area-scaled active worker slots when a connected outside freight gateway exists, and the first committed field tops up startup operating budget to the area-scaled payroll runway
- `power` and `water` consumption should create paid utility service cost rather than behaving as free background access
- `sewage` generation should create paid treatment or management cost rather than being a free passive output
- residential power, water, and sewage charges post to split household utility ledger buckets in `v0.1`
- non-residential power, water, and sewage charges post to building operating budgets in `v0.1`
- those utility charges become revenue for the local utility operator or processor rather than for the city treasury
- if the utility operator is city-owned, that operator revenue deposits into the city treasury instead of a private building budget
- city-owned utility wages, one-time placement costs, and required fuel/input purchases withdraw from the city treasury rather than from a provider operating budget
- daily city budget ledger buckets are recorded by Rust after the daily fiscal settlement and before daily building accumulators reset; Godot overview windows render those buckets and may send live policy changes but do not compute accounting outcomes
- utility-producing and utility-processing buildings should therefore behave like ordinary economic buildings that sell a service rather than like invisible free infrastructure
- any `VAT` or other future fiscal levy on utility service is separate from the operator's service revenue and follows the normal tax rules into the city treasury
- if no local utility service is available, `OWA` may provide that service as an external service purchase
- if no local sewage processing is available, `OWA` may provide external sewage processing
- `OWA` utility fallback should remain a paid fallback and should usually be more expensive than healthy local utility provision
- these utility fallback purchases are not trucked freight and do not use the normal shipment-delivery model
- `sewage` must clear through the utility layer rather than remaining inside the building forever
- in the current `v0.1` bankruptcy model, missing local utility service falls back to paid `OWA`
  service rather than adding a throughput gate; later capacity/outage models may block or degrade
  operation explicitly
- this baseline utility layer is an aggregate service model rather than a trucked-goods model in `v0.1`
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
  - outputs: `packaged_food`
  - variables: `base_cycle_time`, `input_buffer_cap`, `output_buffer_cap`, `schedule_profile`

Standalone neighborhood service businesses use `kind = "service_store"` profiles. A service store
is a labor-and-utility commercial profile whose outputs are aggregate service capacity rather than
stored goods. For example, `personal_service_small` can back a barber, salon, tailor, or laundromat
asset, while `health_essentials_small` can back a pharmacy asset. These profiles may have no freight
inputs in the first pass; small supplies are treated as ordinary operating cost until a future
`commercial_consumables` resource becomes worthwhile.

Base capacities such as `household_capacity` remain asset-authored metadata. However, `worker_capacity` is authoritatively derived from the building's bound economy profile if one is present, overriding any value in the asset manifest. Living standards for households are defined by the asset's `flat_size_m2` (authored in `asset.toml`). Starter move-in sizing treats this as one household's interior area: 25 m2 baseline space, a two-person household may fit as one adult plus one child-weighted member, and larger households reserve two adult-equivalent members at 22 m2 each plus child-weighted extra members at 12 m2 each.

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

Residential buildings remain the spatial anchors for logistics, but household supplies are tracked per household, not per individual agent.

Rules:

- `household_supplies` for baseline living stability
- one supply reserve per household
- single-family homes naturally map to one household
- multi-unit residential buildings host multiple explicit household records, but never one supply reserve per resident

Residents draw from their household supply reserve while at home.

### Household runtime representation

Households are explicit lightweight runtime records anchored to residential buildings or to the
single household slot provided by a farm (`ECON-08`).

This means:

- each household has its own runtime record rather than being merged into one anonymous building-wide supply pool
- each household record stores at least `home_building_id`, derived `member_count`, shared budget, household supplies, and replenishment state
- agents reference a `household_id` for home-life needs and shared household money
- immigration, emigration, and move-in or move-out should default to household-level events rather than isolated individual moves; the economy spec does not require a separate border-entry bootstrap choreography for those members in `v0.1`
- if a later transport layer visualizes admitted or departing households through shared outside
  gateways, economy still owns the household record before arrival and the household-side removal
  reason before departure; transport owns only the trip choreography
- households may also contribute baseline utility load through the `Utility Service Layer`, but that load is a runtime consequence of occupancy and activity rather than something authored through a household `economy_profile`
- residential buildings still own the physical location and capacity, but they do not become the source of truth for each household's budget or supplies

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
- the `stock_days` supply-days value should therefore be computed against that household-level daily consumption rather than against a flat per-household constant

For performance:

- household logic should run on coarse economy cadence, not every render frame
- per-building summaries may be derived from linked households for UI and fast aggregate checks
- the authoritative source of truth for home supplies, household money, and replenishment remains the household record itself

This gives the game a clean unit for budgeting, migration, save/load, and replenishment without falling back to per-agent grocery logic or muddy building-wide averages.

### Agent Need Interpretation

Agents do not need a daily "buy food" trip. Instead:

- being housed in a stocked household satisfies baseline home-life needs
- lack of household supplies reduces happiness, stability, or health-related metrics
- optional leisure or personal shopping trips remain low-frequency and non-essential
- standalone commercial services such as barbers and pharmacies are satisfied through aggregate
  per-resident demand; personal-service demand may also schedule capped representative visits for
  visible city life, but those visits do not change service revenue or inventory accounting

Essential replenishment may create a visible shopping task, but it is household-owned and limited
to one selected carrier. This keeps daily essentials in the household/logistics layer rather than
turning every resident into an independent grocery pathfinder.

### Recommended v0.1 Resource Chain

For the first useful loop, do not start with dozens of goods. Start with one essential household chain.

Example:

- `grain_farm` grows `grain` from a player-drawn field polygon
- `food_processor` converts `grain` into `packaged_food`
- `distribution_center` or `grocery` converts or forwards `packaged_food` into `household_supplies`
- households replenish from `grocery` or `distribution_center` in periodic batches rather than per-person daily errands
- `household` consumes `household_supplies`

If that chain works, the broader economy architecture is sound enough to extend.

### Standalone Service Commercial

The first standalone service-commercial extension is intentionally shallow:

- `service_store` profiles output demand-facing service capacity, not inventory
- `personal_service_small` outputs `80 personal_services/day` with `4` full-staff worker slots
- `health_essentials_small` outputs `120 health_essentials/day` with `6` full-staff worker slots
- `personal_service_demand` creates `0.03 personal_services/day/resident`
- `health_essentials_demand` creates `0.05 health_essentials/day/resident`
- hourly service sales are aggregate: no inventory, freight, or per-agent service accounting is
  created by service demand
- `personal_services` may create capped representative shopping-style trips to staffed reachable
  personal-service buildings; these trips use existing agent target/activity fields and are
  visual-only over the aggregate sales model
- `health_essentials` remains aggregate-only in the first pass
- active service worker slots scale from aggregate resident demand divided by total live authored
  capacity for the service resource, rounded up to one slot when demand exists and to zero when it
  does not
- effective service capacity then scales from live staffed workers through the normal building
  operation factors
- served units are capped by resident demand and staffed capacity; household budgets are debited
  proportionally, and revenue is distributed to matching service buildings by capacity share
- service outputs must not accumulate in building inventory and must not be sold through freight or
  distress liquidation

Barber and pharmacy variety is therefore asset-level variety: different meshes, signs, tags, and
profile references, not different bespoke economy systems.

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

### Household age groups

Resident agents carry one fixed baseline age group in `v0.1`:

- `child`: consumes household resources, but does not work and cannot carry household shopping
- `adult`: may work and may carry household shopping
- `elder`: may carry household shopping, but does not work

Age composition is deterministic when an immigrant household is created. A household may contain at
most two adults and at most two elders. There is no child cap, but every household with children
must have at least one adult. The first member of a multi-member household is an adult, and
remaining members use the baseline deterministic mix within those caps. A single-member household is
always independent: adult or elder, never child-only. There is no aging or lifecycle transition yet.
The current mix is code-defined starter behavior; if it becomes authored tuning later, this section
owns that contract.

Starter household size is selected from the deterministic admission mix, then capped by
`flat_size_m2`, not by `household_capacity`. A single-family house with `household_capacity = 1`
still reserves one family slot; a larger `flat_size_m2` permits a larger family, but it does not
force every arriving household to fill the maximum possible bedroom count. One-person households
are valid starter arrivals.

Household records cache child/adult/elder counts from the parallel membership reduction. Hot economy
passes use those counts instead of scanning household members. A valid housed household must have at
least one adult or elder; child-only households cannot claim or keep a residential slot and should
enter the normal unhoused/removal path rather than being silently repaired.

### Agents supply labor

Agents decide whether to travel to work based on decision-utility scoring rather than a pure RNG cycle.

Early decision-utility inputs can stay simple:

- current money
- household supplies at home
- commute cost
- job availability

Recommended `v0.1` work-decision formula:

```text
work_score =
    w_income  * income_pressure
  + w_supplies * household_supply_pressure
  + w_job     * job_availability_score
  - w_commute * commute_penalty
```

Where:

- all factors are normalized to `0.0..1.0` before weighting
- `income_pressure` is derived from the current household budget or reserve target
- `household_supply_pressure` is derived from the current `stock_days` supply-days value at home
- `job_availability_score` is `0.0` when no valid reachable open job exists and otherwise reflects the best currently available work option
- `commute_penalty` is derived from expected travel cost or time for the candidate job

Job candidate pruning uses the same capped commute penalty as candidate ranking. A strictly
larger penalty lower bound can stop the chunk scan; equal penalties must keep searching because
wage, capacity, vacancy count and building ID can still decide the order. At the 30-minute physical
travel cap, farther candidates therefore remain eligible. This preserves the existing O(J)
worst-case search bound over reachable jobs; saturated searches may inspect more jobs than the
incorrect early-stop path. This bound correction is covered by a regression, not a route-timing claim.

Recommended seed weights for the first implementation:

- `w_income = 0.35`
- `w_stock = 0.35`
- `w_job = 0.20`
- `w_commute = 0.10`
- `go_to_work_threshold = 0.45`

Selection rule for `v0.1`:

- only adult agents are work-eligible; children and elders must be ignored by job assignment,
  workplace retention, wage payment, and work-trip scheduling
- evaluate the score for reachable valid job options only
- choose the highest-scoring reachable job
- if the best score is at least `go_to_work_threshold`, the agent departs for work
- otherwise the agent stays in its non-work state for that decision pass

This keeps the first pass deterministic, bounded, and easy to debug. Richer nonlinear or probabilistic choice models can be added later if the design needs them.

### Building throughput depends on staffing

Production should derive from a bounded formula based on:

- filled worker count
- input availability
- utility service costs and provider revenue settled by the `Utility Service Layer`
- controller modifiers

Recommended `v0.1` formula:

```text
throughput = base_rate
           * staffing_factor
           * input_factor
           * controller_factor
```

Where:

- `base_rate` is the authored full-capacity output rate for the building or recipe
- `staffing_factor = clamp(filled_workers / worker_capacity, 0.0..1.0)`
- explicit field producers and extractors replace `worker_capacity` in this ratio with the
  area-scaled active worker capacity; an uncommitted area has zero active worker capacity, and
  10,000 m2 receives exactly the authored worker count
- commercial store `filled_workers` is capped by the larger of recent household sales and a
  local household-demand floor before this ratio is calculated; the demand floor includes current
  resident consumption plus household supply recovery spread over the authored pantry target days
- a zero-sales store still keeps one bootstrap worker slot, but household shortage can open more
  active worker slots before sales recover so essential shops do not deadlock on empty shelves
- `input_factor` is the limiting required-input coverage for the current production step, clamped
  to `0.0..1.0`; production compares input inventory to the current effective hourly input need,
  not to the full authored daily input for a fully staffed building
- utility costs are paid through the utility/bankruptcy sequence; there is no `utility_factor` term
  in the current `v0.1` throughput multiplier
- `controller_factor` is a bounded multiplier from allowed controller effects

This keeps the first pass linear and readable. Hard minimum-staff step functions are not part of the baseline formula; if they are ever added later, they should be explicit profile-side rules rather than hidden default behavior.

This gives the player a meaningful connection between zoning, staffing, transit, and output without requiring arbitrary micromanagement.

## Logistics Model

### Shipment units

The simulation should create shipments at the building or terminal level, not one tiny packet per household resident. In `v0.1`, the only terminal-like freight gateways are `OWA` border terminals.

Each shipment should minimally contain:

- stable shipment id
- resource type
- amount
- source endpoint (`Building` or `OWA` border node)
- destination endpoint (`Building` or `OWA` border node)
- assigned carrier class
- status (`Queued`, `InTransit`, `Returning`, `Fulfilled`, `Failed`, or `Expired`)
- active carrier agent id when dispatched

### Carrier classes

Initial carrier hierarchy:

- trucks for local delivery
- later trains and ships for bulk long-distance transfer
- later airplanes only for special high-value chains

### Physical carrier lifecycle

Truck freight in `v0.1` is represented by ordinary lane-bound vehicle agents using the freight
truck model under `godot/assets/models/vehicles/freight/`.

Rules:

- a shipment may be planned only after route feasibility is proven through the existing entrance /
  car-access planner
- building-to-building freight spawns one carrier at the source building and drives to the
  destination building, then returns empty to the source building before the carrier is removed
- `OWA` imports spawn one carrier at the selected border node and drive to the destination building
  before returning empty to the same border node
- `OWA` exports spawn one carrier at the source building and drive to the selected border node
  before returning empty to the source building
- shipment delivery settlement runs on the coarse logistics cadence; an arrived carrier may settle
  cargo on the next logistics pass rather than on the exact render frame of arrival
- after cargo settlement, the shipment remains open in `Returning` only to track and clean up the
  empty carrier; the return leg must not keep source inventory or destination demand reserved
- delivery starts a fresh return-leg clock. Both traveling legs are bounded by
  `max(6, 4 * eta_hours + 2)` operational hours (saturating at the stored hour limit).
  A missing or timed-out return carrier closes the already settled shipment as fulfilled without
  another inventory or money transfer. Carrier cleanup checks both vehicle class and stable
  shipment ID before removing an agent slot.
- `eta_hours` is an estimate for timing/debug/capacity decisions, not the authority that completes a
  shipment
- active carrier removal, building removal, invalid endpoint state, or a trip that exceeds its
  bounded in-transit timeout must resolve the shipment deterministically as fulfilled, failed, or
  expired; no carrier may remain orphaned after its shipment is closed

This keeps goods visible in traffic without adding a second movement stack.

The `AUDIT-01` return-clock/identity fix adds O(1) allocation-free work per active shipment.
Five alternating unprofiled release process pairs, 24 Rayon workers, 11 × 16 progression calls
per size, measured 65,536 active returns at median `0.335 → 0.399 ms` per logistics pass.
This isolates progression with no arrivals/removals or route planning. The additional 0.064 ms
buys the missing bounded lifecycle; it does not establish cleanup-batch scaling. Before/after
executables, hashes and raw runs are under `/tmp/metrum-full-audit/carrier-*`.

Carrier swap repair uses the moved agent's stable shipment ID and binary-searches the existing
ordered shipment vector. Appends, retention, SQLite load and undo preserve ID order; undo stores
jobs without a duplicate historical vector index. Repair is O(1) for a moved resident and
O(log S) for a moved freight carrier, with no new index or allocation. Five alternating release
process pairs (24 Rayon workers, 11 samples of 256 repairs, setup/SoA removal excluded) measured
1,024 / 8,192 / 65,536 shipments at median `0.155 / 0.560 / 12.413 ms` before and
`0.004 / 0.005 / 0.005 ms` after. Exact sources, executables, hashes and raw runs:
`/tmp/metrum-full-audit/carrier-remap-*`. Actual cleanup regressions also exercise mixed resident
and freight swap moves with noncontiguous shipment IDs.

### Agent Initialization Audit Measurements (2026-09-12)

`AUDIT-01-A13` shares initialization and SoA insertion between housed residents, household
arrival carriers and freight. Role-specific state remains explicit: freight has no household,
home or resident money, and its existing non-working age marker remains unchanged. Schedule and
visual seeds still use the origin building and insertion index; render IDs use the existing
monotonic allocator. The unused random-spawn helper and three unused border-spawn arguments are
removed. Initialization is O(1), with amortized O(1) column insertion and no additional storage or
allocation. Insertion remains serial because it appends to coupled columns and allocates identity.

Five alternating unprofiled release process pairs use CPU 0 with one worker. The fixture reserves
column capacity, then measures three warmups and 21 batches per case. Capacity reservation,
clearing, clock/ID reset and a hash of every generated column are outside timing. Housed cases
include all three age classes; border cases include whole-household carrier sizes; all cases
include ordinary and sentinel origin IDs. Every output hash matches across builds and runs.

| Agents | Housed before → after, ms | Border before → after, ms | Freight before → after, ms | Mixed before → after, ms |
| --- | --- | --- | --- | --- |
| 1,024 | 0.044233 → 0.045014 | 0.045033 → 0.045690 | 0.043986 → 0.045065 | 0.044633 → 0.045611 |
| 16,384 | 0.752458 → 0.768472 | 0.749509 → 0.757451 | 0.758311 → 0.779345 | 0.766196 → 0.784328 |
| 131,072 | 22.883909 → 22.814488 | 6.022510 → 6.106335 | 8.815425 → 8.959475 | 8.846070 → 9.011130 |

These median process medians show comparable cost with a measured 1–2.8% increase in most
cases; the largest mixed batch adds 0.165 ms. This measures warmed insertion, excluding route
planning, household admission, capacity growth and gameplay ticking. It is not a frame-time or
full immigration benchmark. No other builds or benchmark jobs ran during these comparisons.

Reproduce with `python3 /tmp/metrum-full-audit/match_agent_spawning.py`. The runner records
commands for `BINARY --exact simulation::economy::agents::lifecycle::tests::benchmark_agent_spawning
--ignored --nocapture --test-threads=1`, with `taskset -c 0`, `RAYON_NUM_THREADS=1` and
`METRUM_DEBUG=0`. Raw runs and summaries are in `agent-spawn-matched-{bench,summary}.json` under
that directory. `agent-spawn-{before,after}-identity.json` records source hashes and Rust 1.98.1
(`48a229cea`, 2026-09-01). Binary SHA-256 values are
`e6beeb312519a402df54cd27681ccf134968dda35970feebacc144b479087d83` before and
`bf921775fe34e6a3db8b1f496ae64797959d3a62622527c39c25f81ce7373ecb` after.

### Compression rule

One carrier represents a meaningful aggregate shipment, not one consumer purchase.

That means:

- one truck may represent many households' worth of supplies
- one train or ship represents many truckloads
- later internal bulk terminals split bulk flows into last-mile deliveries when necessary

### Demand accumulation and reorder thresholds

Shipments should not be created for every tiny consumption event.

Rules:

- destinations accumulate demand against an inventory buffer rather than spawning a shipment immediately on every shortage
- a normal shipment request is created only when inventory falls below a reorder threshold or when accumulated unmet demand reaches a meaningful batch size
- a smaller emergency shipment may be allowed below the normal batch threshold only when inventory falls below a critical threshold
- shipment creation should run on a coarse economy cadence, not every render frame
- `reorder_threshold` is authored in `days_of_supply`
- `critical_threshold` is authored in `days_of_supply`
- UI may display equivalent percent-of-storage or absolute-unit values as derived information, but `days_of_supply` is the canonical authored format for inventory urgency

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

This keeps request count proportional to active economic nodes rather than to every individual inventory change.

### Reservation rules

Shipments must reserve both supply and demand explicitly.

Rules:

- when a shipment is created, the source reserves the promised inventory immediately
- the destination reserves the corresponding unmet demand immediately
- reserved inventory may not be sold twice, and reserved demand may not spawn duplicate requests
- when a carrier is dispatched from a building source, the source inventory is moved out of the
  building and into the shipment; seller revenue is still credited only on successful delivery
- if a shipment fails, expires, or is canceled, both reservations must be released deterministically
- building removal cancels unsettled orders through the same payment refund path: private buyers
  regain operating funds; city-service orders refund the treasury and reverse the service's input
  expense. This also applies when the removed building is the city-service destination itself.
- delivered shipments with returning carriers remain settled when either endpoint is removed;
  no goods or payment are refunded again. Building-deletion undo restores the order and reverses
  only that deletion's city refund, preserving unrelated treasury transactions.

This prevents double-selling, phantom shortages, and duplicate jobs.

### Route creation

The authored economy graph chooses who is allowed or preferred to supply whom.

The runtime then resolves:

- which supplier has inventory
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
- candidates that lack inventory, fail authored compatibility rules, or fail route feasibility should be rejected before reservation
- a bounded candidate window must not permanently exclude reachable suppliers outside the first window;
  if demand is still unresolved, the next coarse retry should resume or widen the deterministic
  search frontier, or use a component-level allocation pass that can draw from farther reachable
  suppliers
- for ordinary shipped goods, if no local supplier is valid, the system may fall back to the `OWA` when the economy rules allow it
- no request should perform an unbounded city-wide best-price scan

This keeps supplier lookup compatible with city scale and makes authored preferences matter.

### Retry cooldowns and failure states

Failed logistics work must back off instead of retrying every tick.

Rules:

- a failed request enters cooldown before it may search again
- retries should happen on coarse economy cadence or with explicit backoff, not every simulation tick
- after repeated failures, the request should escalate to a visible shortage or unresolved-demand
  diagnostic state rather than spamming logs; this state must not permanently suppress future
  retries after cooldown, budget recovery, supplier recovery, or route-topology edits
- every request should end in an explicit state such as `queued`, `reserved`, `in_transit`, `fulfilled`, `cooldown`, `expired`, or `failed_terminal`

This prevents retry storms and makes debugging easier.

### Household replenishment

Shopping and sampled service visits share their bounded nearest-destination selection and
round-trip feasibility checks. Candidate replacement never grows an already reserved buffer;
distance ties still prefer the lower building ID. Grocery retry-cursor ordering is preserved.
`AUDIT-01-H15` validation used the isolated `benchmark_shopping_candidate_selection --ignored
--nocapture` release fixture: 1,000 queries per sample, 11 samples, descending candidate streams
with distance ties, limits/scans of 8/128 and 24/384, and five alternating process pairs with
`RAYON_NUM_THREADS=24`. Median milliseconds per 1,000 queries were `0.656 → 0.667` and
`2.862 → 2.907`; this establishes no speedup. The verified improvement is removing transient
capacity growth. Selection remains O(K) per candidate and O(K) scratch storage for fixed K;
index construction complexity is unchanged. Sources, executable identities and raw runs are
under `/tmp/metrum-full-audit/shopping-*`.

Household replenishment in `v0.1` uses one visible fulfillment mode:

- a bounded household-shopping carrier task, represented by one selected household member making
  an ordinary `Home -> Store -> Home` trip

The household record owns supplies, money, reservations, and replenishment state. The selected agent
is only the visible carrier.

Rules:

- replenishment is driven by the household supply system on coarse economy cadence, not by adding a
  new baseline `TRANSIT_*` movement state
- one household may have at most one active shopping task at a time
- the shopper uses the ordinary building-origin trip planner with `planned_target_building` and
  `planned_activity`; movement code must not directly mutate household supplies, store inventory, or
  store revenue
- when household supplies fall below the household's replenishment threshold, the household creates a
  replenishment need
- when household supplies reach `0.0 days`, the household may bypass its normal staggered check offset if a
  valid commercial store currently has sellable household supply; reservation and cooldown guards
  still apply
- before reserving store inventory or spending household budget, the household must find an eligible
  shopper currently at home
- an eligible starter shopper belongs to the household, is an adult or elder, is
  `TRANSIT_IN_BUILDING`, is currently in `home_building_id`, has home activity, has no existing
  planned target, and is not already carrying another household task
- if no eligible shopper is at home because all members are travelling, at work, or otherwise away,
  the household enters `waiting_for_shopper`; it does not reserve inventory, spend budget, or enter
  cooldown
- when an eligible member later returns home, the next household economy pass may claim store inventory
  and assign that shopper
- reservation and shopper assignment are one deterministic serial apply step; store inventory and
household budget must not be held without an assigned shopper
- an agent swap-remove must repair a moved shopper's household reference in the same operation;
  the schedule-seed guard remains a stale-reference check, not a substitute for index repair
- candidate stores must be reachable by the same ordinary building-origin trip planner the selected
  shopper will use for both `Home -> Store` and `Store -> Home`; unreachable candidates are rejected
  before inventory or budget is reserved
- store discovery must be fair across reachable supply, not a permanent nearest-N cutoff: a household
  whose local stores are empty must eventually consider farther reachable stores on later coarse
  replenishment attempts, using a deterministic continuation cursor/frontier or a shared store
  allocation pass
- continuation windows must wrap or otherwise reset after exhausting the compatible supplier index,
  so unlucky cursor values cannot permanently skip a subset of reachable stores
- if serial reservation contention drains every store in a household's first candidate set, the
  household should retry from the next deterministic reachable-supplier window rather than repeatedly
  failing against the same depleted nearest stores
- if the household cannot afford the full target refill, it may reserve the largest affordable
  partial basket from the store's available inventory rather than failing the request outright
- once a valid shopper and sale are both claimed, the store inventory is reserved or removed,
  household budget is reserved or spent, the household enters `shopping_to_store`, and the selected
  agent receives a trip to the store
- when the shopper arrives at the store, the household tick observes the arrival, credits store
  revenue or budget, changes the household to `shopping_returning`, and schedules the same agent
  back home
- household supplies increase only after the shopper returns home
- if the store becomes invalid before pickup, restore reserved store inventory, refund the
  household budget, clear the shopper assignment, and enter bounded cooldown
- if the shopper task is lost or invalidated before pickup, restore the reservation and retry from
  `waiting_for_shopper` or cooldown according to tuning
- each active shopping leg has an explicit operational-hour timeout; timeout before pickup restores
  the reserved store inventory and household budget, while timeout after pickup clears the task and
  records a failed fulfillment
- if fulfillment fails, the request follows the same bounded retry and cooldown rules as other
  economy requests; after the authored terminal-failure count it enters `failed_terminal` /
  unresolved shortage instead of retrying forever
- a `failed_terminal` household may retry only on its normal replenishment cadence, so player fixes
  such as a new reachable grocery can recover the shortage without returning to per-hour retry spam

Useful first-pass household replenishment states are:

- `stable`
- `needs_replenishment`
- `waiting_for_shopper`
- `shopping_to_store`
- `shopping_returning`
- `fulfilled`
- `cooldown`
- `failed_terminal`

The `economy` debug output should expose one daily per-household ledger line for active households
with wage income, transfer income split into unemployment/pension/child-support buckets, shopping
spend or refunds, utility plus supply-consumption cost, budget before and after the daily window,
unemployed adult count, and completed / failed shopper trips. The same daily output should include a
household ledger summary with households at the budget floor, households below `1`, `2`, and `3`
days of supplies, total wages paid, total household shopping spend, total transfers paid, and the
transfer sub-buckets. Transfer diagnostics should identify recipient `household_id`, composition,
amounts paid, and `unemployment_days_elapsed` where unemployment support is involved.

The old abstract pickup ETA is no longer part of the baseline. Any timeout must be an explicit
shopping timeout or failure rule, not hidden fulfillment.

This keeps essentials household-level while making the store trip visible. The scale bound is one
active shopper task per replenishing household, planned on coarse cadence, with contested store
reservations applied deterministically.

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

- the developer places a `grain_farm` node with output `grain`
- the developer places a `food_processor` node with input `grain` and output `packaged_food`
- the developer places a `grocery` node with input `packaged_food` and output `household_supplies`
- the developer places a `basic_household_demand` node with input `household_supplies`
- the developer places a household supply or cost controller that affects replenishment pressure
- the graph then connects `grain_farm -> food_processor -> grocery -> basic_household_demand`, with the controller linked to the household demand sink

At this stage the developer is defining the structure of the economy chain, not yet testing whether the numbers are balanced.

#### 2. Runtime Inspection View

A debug view for scenario playback and diagnosis of the authored balance rules.

Use it to inspect:

- inventory levels
- blocked supply chains
- delivery latency
- unfilled labor demand
- controller effects
- shortage propagation

Example:

- the developer runs the `Grocery Bottleneck` test case for 30 simulated days
- the view shows that household supplies drop below 1.0 days after day 12
- the diagnostics panel reports that the grocery has enough goods, but shopper-side replenishment demand is arriving in bursts and shop-side queueing is too high
- the controller panel highlights that household replenishment cadence and grocery throughput are misaligned
- the developer can immediately see that the problem is not food production, but local shopping balance and store throughput

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
- Center graph: show `food_processor -> grocery -> basic_household_demand`, with an optional replenishment-pressure controller connected to the household demand sink.
- Right inspector: expose values such as household count, household size, shop distance, replenishment cadence, grocery throughput, and supply target.
- Bottom diagnostics: show supply days, average household cost, replenishment queue pressure, shortage warnings, and whether any recipe or connection is invalid.

In this example the graph, inspector, and diagnostics are enough to test whether local shopping and store throughput give the intended balance result.

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
- compile the validated TOML into the shared in-memory runtime catalog at startup

Manual editing is allowed. If a developer or modder wants to tweak the values in a text editor instead of the economy editor UI, that should be supported as long as the files still validate.

### Suggested exported structure

The exported economy structure is:

```text
economy/
  profiles.toml        # economy profiles and recipe definitions
  controllers.toml     # controller definitions and parameters
  scenarios.toml       # scenario overrides and test setups
```

These filenames and this top-level folder layout are the baseline contract for the first implementation.

The important runtime rules are:

- text files are authoritative
- compiled runtime data is derived from those text files
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
- per-building production/inventory buffers and per-household supply buffers
- household supply consumption
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

- `grain_farm`
  - inputs: `labor`
  - outputs: `grain`
  - placement: explicit farm building plus player-drawn field polygon; 10,000 m2 receives the authored output rate, while `worker_capacity_area_m2` determines the area receiving the authored worker count
- `food_processor`
  - inputs: `grain`, `labor`
  - outputs: `packaged_food`
- `grocery` or `distribution_center`
  - inputs: `packaged_food`, `labor`
  - outputs: `household_supplies`
- `basic_household_demand`
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

Replenishment for this chain should happen through the bounded one-shopper household shopping flow
in `v0.1`. `ADS` is a later extension, not part of the first implementation scope.

This example is intentionally broad. It avoids modeling "one loaf of bread per person per day" while still creating meaningful logistics, staffing, and shortage gameplay.

### Seed values for first implementation

The first playable implementation should ship with a small shared seed-balance set so the example chain is runnable before the economy editor is heavily used for tuning.

These are shipped `economy/profiles.toml` values, not Rust defaults:

- household `consumption_rate`: `1.0 household_supplies / day / resident`
- household replenishment target: `5.0 days` of supplies
- household replenishment trigger: below `2.5 days` of supplies
- immigrant starting supplies: `3.0 days`
- immigrant starting budget: `15.0 currency / resident`
- unemployment benefit: `30.0 currency / unemployed adult / day`
- pension: `30.0 currency / elder / day`
- child support: `10.0 currency / child / day`
- household utility cost: `3.0 currency / resident / day`
- residential stay reserve thresholds by level: `0.5`, `3.0`, `6.0` days
- household replenishment check cadence: every `6` in-game hours
- `grain_farm` `base_rate`: `290 grain / day / hectare`
- `grain_farm` worker capacity: `1 / 10 hectares`, rounded up with a two-worker minimum for committed fields
- `grain_farm` wage band: `80-100 currency / workday`
- `food_processor` `base_rate`: `160 packaged_food / day`
- `food_processor` worker capacity: `10`
- `food_processor` wage band: `80-100 currency / workday`
- `grocery` or `distribution_center` throughput target: `200 household_supplies / day`
- `grocery` worker capacity: `15`
- `grocery` wage band: `80-100 currency / workday`
- grocery inventory target: `3.0 days` of supply
- grocery reorder threshold: `2.0 days` of supply
- grocery critical threshold: `0.5 days` of supply
- grocery minimum shipment size: `40 packaged_food`
- local base price for `grain`: `6 currency / unit`
- local base price for `packaged_food`: `15 currency / unit`
- local base price for `household_supplies`: `25 currency / unit`
- `OWA import_ask` for `packaged_food`: `26.25 currency / unit` (local × `owa_import_price_multiplier = 1.75`)
- `OWA import_ask` for `household_supplies`: `43.75 currency / unit` (local × 1.75)
- initial `OWA export_bid` for `packaged_food`: `9.00 currency / unit` (local × `owa_export_price_multiplier = 0.60`)
- saturated `OWA export_bid` bottoms at `75%` of the normal export bid after roughly `4` same-resource truckloads, then recovers over `24` operational hours with no further exports
- OWA utility fallback uses the same per-service unit prices as local utility billing; with no local power, water, or sewage provider, a private non-residential or explicit field/extractor business pays `(3.0 + 2.0 + 1.5) × 1.75 = 11.375 currency/day`

**`OWA` import price implementation:** the runtime derives the effective OWA import price as `local_unit_price × owa_import_price_multiplier`. A value of `1.75` means the OWA charges 75% more than the local producer, making local supply chains economically preferred once they are operational. Values below `1.0` are rejected at runtime. The multiplier also applies to the `adjusted_unit_price` freight-timing modifier on top.

**`OWA` export price implementation:** when an industrial building has unreserved output inventory exceeding one day's production buffer after reachable local business/utility input holds and its own same-resource input reserve, the logistics system creates an outbound export shipment. The initial OWA bid is `local_unit_price × owa_export_price_multiplier`. A value of `0.60` means the OWA initially pays 60% of the local price, keeping exports a loss-reducing safety valve rather than a preferred revenue source. Repeated same-resource exports apply the authored saturation factor only after the freight reaches the `OWA` border and revenue settles. Values outside `[0.0, 1.0]` are rejected at validation time.

**Commercial store scaling implementation:** commercial store active worker capacity and input
inventory targets scale from the larger of recent household sales and local essential demand.
The local demand floor is computed from housed household consumption plus below-target pantry
recovery demand, divided across live household-facing shop output capacity. A new or zero-sales
store keeps one active worker slot and at least one minimum truckload worth of input target so it
can bootstrap; household shortage can raise that floor before sales recover. Larger stores still
use sales as the long-run brake against immediately operating at full authored capacity when the
resident base does not justify it.

These numbers are only a bootstrap reference pack. They live in editable economy data so all implementations and test scenarios start from the same baseline before the editor-driven balancing pass diverges. Runtime code must validate this data and fail loudly when required values are missing; it must not silently substitute balance values from Rust.

## Suggested Implementation Order

Cross-doc sequencing note:

- the shared cross-doc order is defined in [`zoning.md`](zoning.md): zoning and asset-editor
  foundation first, demand-layer integration second, economy integration third
- this section is therefore the economy-local implementation order that should run once the zoning
  and demand ownership contracts already exist

The codebase already ships part of the `v0.1` starter loop: explicit household records, simple
building budgets and inventories, bounded freight reservations, `OWA` import fallback, and the first
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
- Keep one essential chain authoritative: local producer -> local shop or distribution -> household supplies.
- Do not widen scope into dynamic pricing, unbounded per-agent daily shopping, or broad multi-resource simulation yet.

Current status:

- complete
- explicit household records, bounded freight reservations, `OWA` startup fallback, household
  supplies, building operating budgets, and the starter industrial input/output slice are all live
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

- Move from the old starter supply-plus-industrial-input buffers toward fully resource-typed building inventories, reservations, and shortage state.
- Keep shipment creation bounded, batched, and entrance-aware; do not regress into per-order or per-agent freight.
- Expand to additional resources only after the starter household-supply loop still works cleanly on the generalized runtime.

Current status:

- complete
- live buildings now carry typed per-resource inventories instead of the old untyped `stock` /
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

- The grocery store (`grocery_basic`) requires `packaged_food` input to produce `household_supplies`.
  The bootstrap path therefore needs explicit money and freight support before local industry is
  stable.
- Once household supplies hit zero, the household supply stability signal collapses to `0.0`, which kills
  `city_stability_factor` and drives admission pressure to zero regardless of startup support.
  Population cannot grow past the first wave of immigrants.
- Phase 5 must close the first household supply loop through authored profile data: startup
  operating float, paid `OWA` imports when local suppliers are absent, and household starting
  supplies. The runtime must not silently seed store inventory from Rust.

Current status:

- complete
- `CityTreasury` lives in `SimCore` and tracks balance, lifetime build cost, lifetime tax revenue,
  daily road upkeep, and pending/finalized daily tax buckets
- startup balance initialised at `100,000` currency
- road placement deducts `50 currency/meter` from the treasury; balance may go negative per spec
- daily road upkeep deducts `0.1 currency/meter/day` on the daily fiscal settlement pass
- commercial and industrial startup budgets now include a seven-day wage runway plus the expected
  first full `OWA` input import cost, computed from authored profile prices and
  `runtime_tuning.owa_import_price_multiplier`
- `grocery_basic` currently ships with `starting_inventory_days = 0.0`; stores do not receive
  hidden Rust-seeded output inventory
- treasury is persisted in the `city_treasury` SQLite table
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
- four utility-adjacent profiles landed in `economy/profiles.toml`: `coal_mine_basic`
  (`coal` extractor/price profile), `power_plant_basic` (coal-fueled power, 20 workers, three-shift),
  `water_plant_basic` (water, 3 workers), `wastewater_treatment_basic` (sewage, 3 workers)
- daily utility settlement scans active providers per service, resolves `power` from accumulated
  produced units, charges commercial, office, mixed-use, and industrial consumers independently for
  `power`, `water`, and `sewage`, routes split household utility ledger payments into matching local
  utility revenue when covered, applies missing-service `OWA` surcharges to household budgets,
  falls back to OWA service spend for uncovered private and city-service demand, and records
  city-owned local utility fees and city-paid OWA fallback in the city budget ledger
- active utility providers must be non-broken, non-deserted, and connected to the network; starter
  `water` and `sewage` providers also require current workers with positive operational throughput,
  while `power` settlement uses the day's already accumulated produced units
- explicit city service assets are registry-discovered, road-frontage placed through the Services
  toolbar, charged to the treasury at placement, and staffed through the normal job system with
  city-funded wages and city-funded fuel/input purchases
- the gameplay Economy Overview window reads Rust-owned daily budget history, displays city income /
  expense / net / treasury trends, and exposes a live electricity funding slider whose changes apply
  without OK/Cancel/Apply confirmation; the slider sets the default staffed power-worker slots
  instead of directly multiplying production
- power plant inspectors show actual worker count and can set a per-plant funding override that
  persists separately from the citywide electricity funding slider
- no invisible utility buildings exist

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
- demand-owned household admission, removal, startup support, and hourly building action plans are
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

## Household Transfer Payments

**Status: implemented.** Household transfers are live in `households.rs`. Defaults are authored in
`economy/profiles.toml`, copied into `CityFiscalPolicy` at runtime, exposed through the Economy
Overview Policy tab, and persisted with the save.

The first transfer set is:

- unemployment benefit paid per unemployed adult within the unemployment time limit
- pension paid per elder
- child support paid per child

These transfers replaced the old `pioneer_demand` floor as the mechanism that keeps early-city
households solvent. Unlike the Pioneer floor, they are real simulation money flows: funds move from
the city treasury into household budgets, households spend through the normal replenishment loop,
stores earn real revenue, and that revenue creates real commercial and industrial pressure.

### Ownership

This section owns the household-transfer spec. `demand.md` documents the demand-owned move-in
acceptance formula that reads exact candidate composition, policy amounts, and treasury balance as
source values. `households.rs` owns the runtime disbursement implementation. `nodes/sim/core.rs`
owns the `CityTreasury` struct; starting balance is authored in `economy/profiles.toml`.

### Design Invariants

- Transfers are household-level daily settlement payments, not per-agent tick-time micro-payments.
- Money is drawn from the **existing `CityTreasury`** (`SimCore::treasury`). It is not printed from nothing.
- Unemployment benefit is self-terminating: once an adult agent is employed, disbursement stops for
  that household member. Once all adult household members are employed, the household exits
  unemployment benefit entirely.
- Pension and child support are age-composition transfers. They do not require work eligibility,
  and they allow elder-only or child-heavy households to be evaluated by demand from their actual
  composition instead of from an average worker assumption.
- Transfers must generate real purchasing activity. A household that receives support must still
  use the normal household replenishment and utility-payment systems.
- Unemployment benefit must not create infinite runway. A household that cannot find work within
  the configured `unemployment_max_days` becomes removal-eligible through demand rather than
  subsisting on unemployment payments indefinitely. Pension and child support are not capped by
  unemployment duration.

### Money Source

`CityTreasury` already exists in `nodes/sim/core.rs` and is fully implemented:

- **Starting balance**: `startup_treasury_balance = 100_000` authored in `economy/profiles.toml` `[runtime_tuning]`.
- **Current deductions**: road build cost ($15/meter) and daily road upkeep ($0.1/meter/day);
  unemployment, pension, and child-support disbursements.
- **Persisted**: saved and loaded via the `city_treasury` SQLite table.
- **Exposed**: `get_treasury_balance()` GDScript bridge already exists.

Transfer disbursements draw from the same `treasury.balance`. The disbursement connection from
`HouseholdSystem` to `CityTreasury` is live (`pay_household_transfers` called from
`daily_settlement_tick`).

The treasury balance may go negative for other obligations. Transfer disbursement is intentionally
cash-limited: it pays only while a positive treasury balance remains and never deepens the deficit.

### Eligibility Rule

An adult member is eligible for unemployment benefit on a given day if **all** of the following hold:

- `household.member_count > 0`
- `household.home_building_id` is a valid, operational building with household capacity (including a farm)
- the member is an adult
- the adult member has `work_building == usize::MAX` (is unemployed)
- `household.unemployment_days_elapsed < unemployment_max_days`

`unemployment_days_elapsed` increments each day any adult household member remains unemployed, and
resets to zero once all adult members are employed. Child/elder-only households have no unemployed
adult workers and do not receive this benefit.

An elder member is eligible for pension on a given day if the member belongs to a live household.
Pension does not require a valid residential home, a work search, or unemployment duration.

A child member is eligible for child support on a given day if the member belongs to a live
household. Child support does not require a valid residential home.

### Disbursement Rule

Once per operational day, after `pay_daily_wages` and before `resolve_household_housing`, iterate
households in stable household-id order:

```
unemployed_adults = count of adult agents in household where work_building == usize::MAX
elders = count of elder agents in household
children = count of child agents in household

unemployment_today =
    unemployed_adults × CityFiscalPolicy.unemployment_benefit_per_adult_per_day
    when unemployment_days_elapsed < CityFiscalPolicy.unemployment_max_days
pension_today = elders × CityFiscalPolicy.pension_per_elder_per_day
child_support_today = children × CityFiscalPolicy.child_support_per_child_per_day
transfer_today = unemployment_today + pension_today + child_support_today

if treasury.balance >= transfer_today:
    household.budget += transfer_today
    treasury.balance -= transfer_today
else if treasury.balance > 0.0:
    household.budget += treasury.balance   // pay what remains
    treasury.balance  = 0.0
// if treasury.balance == 0.0: skip silently
```

`treasury` here is `SimCore::treasury`, passed into `daily_settlement_tick` by the caller.
Daily household ledgers retain separate `unemployment_benefits`, `pension_income`, and
`child_support_income` buckets; daily city budget history reports both total transfer expense and
those sub-buckets.

### Termination Conditions

| Condition | Outcome |
|---|---|
| All adult household members find employment | Unemployment disbursement stops; `unemployment_days_elapsed` resets to 0 |
| `unemployment_days_elapsed >= unemployment_max_days` | Household becomes emigration-eligible at normal removal priority; benefit stops |
| `treasury.balance <= 0.0` | Transfer disbursement stops for all households; support-backed bootstrap ends organically |

### Authored Tuning Parameters

`unemployment_daily_benefit_per_member`, `unemployment_max_days`,
`pension_daily_benefit_per_elder`, `child_support_daily_benefit_per_child`,
`startup_treasury_balance`, household starter values, household utility cost, private construction
durations, fiscal tax defaults, and OWA import/export multipliers all live in the `runtime_tuning` block of
`economy/profiles.toml`.

| Parameter | Location | Role |
|---|---|---|
| `startup_treasury_balance` | `economy/profiles.toml` runtime_tuning | Total treasury at map start |
| `unemployment_daily_benefit_per_member` | `economy/profiles.toml` runtime_tuning | Default currency paid per unemployed adult per day |
| `unemployment_max_days` | `economy/profiles.toml` runtime_tuning | Days before an unemployed household becomes emigration-eligible |
| `pension_daily_benefit_per_elder` | `economy/profiles.toml` runtime_tuning | Default currency paid per elder per day |
| `child_support_daily_benefit_per_child` | `economy/profiles.toml` runtime_tuning | Default currency paid per child per day |
| `runtime_tuning.fiscal.*` | `economy/profiles.toml` runtime_tuning | Default tax and property-tax policy values |
| `runtime_tuning.households.*` | `economy/profiles.toml` runtime_tuning | Household starter budget, starter supplies, reserve rules, and utility cost |
| `runtime_tuning.construction.*` | `economy/profiles.toml` runtime_tuning | Private construction durations for fresh demand-owned spawns |
| `owa_import_price_multiplier` | `economy/profiles.toml` runtime_tuning | Price multiplier for OWA imports and missing local utility service fallback |

### Spawn Signal: Replacing the Pioneer Floor

The removed Pioneer demand floor (`pioneer_demand = 0.70`) existed because the `stock_stab` supply-stability metric and
`afford` metrics collapse to near-zero on a fresh map, starving the spawn system of signal.
Household transfers restore these signals through real economic activity:

1. Disbursement gives households money based on exact adult/elder/child composition -> `afford` rises.
2. Households with money attempt grocery replenishment -> the `stock_stab` supply-stability metric rises.
3. The grocery earns real revenue → absorption gate threshold is met sooner → second grocery spawns.
4. More groceries need supply → industrial spawn pressure rises.
5. Industrial buildings hire workers → households exit unemployment → benefit drain slows.

The pioneer demand floor has been removed from `demand.rs`. The unemployment benefit is the
work-search cash-support mechanism for adult workers, while pension and child support cover
non-working age groups. Demand uses exact candidate composition plus treasury coverage when
calculating deterministic move-in acceptance.

### Shipped Tuning

Live values in `economy/profiles.toml` `[runtime_tuning]`:

| Parameter | Value | Role |
|---|---|---|
| `startup_treasury_balance` | 100,000 | Total treasury at map start |
| `unemployment_daily_benefit_per_member` | 30.0 | Currency paid per unemployed adult per day |
| `unemployment_max_days` | 30 | Days before unemployed household becomes emigration-eligible |
| `pension_daily_benefit_per_elder` | 30.0 | Currency paid per elder per day |
| `child_support_daily_benefit_per_child` | 10.0 | Currency paid per child per day |
| `runtime_tuning.households.immigrant_starting_stock_days` | 3.0 | Supply days granted to arriving households |
| `runtime_tuning.households.immigrant_starting_budget_per_member` | 15.0 | Starting currency per arriving resident |
| `runtime_tuning.households.household_starting_budget_floor` | 10.0 | Minimum carried budget for materialized arriving households |
| `runtime_tuning.households.utility_cost_per_member_per_day` | 3.0 | Daily utility cost per resident |
| `runtime_tuning.households.residential_move_in_min_reserve_days_by_level` | [0.5, 6.0, 12.0] | Reserve days required to move into residential levels |
| `runtime_tuning.households.residential_stay_min_reserve_days_by_level` | [0.5, 3.0, 6.0] | Reserve days required to stay housed by residential level |
| `runtime_tuning.construction.residential_hours_by_level` | [6, 12, 18] | Fresh residential construction hours by target level |
| `runtime_tuning.construction.commercial_hours_by_level` | [8, 16, 24] | Fresh commercial construction hours by target level |
| `runtime_tuning.construction.industrial_hours_by_level` | [12, 24, 36] | Fresh industrial construction hours by target level |
| `coal_mine_basic.base_rate_units_per_day` | 120.0 units/day/hectare | Full-staffed starter coal output before reserve, staffing, and buffer limits |
| `coal_mine_basic.worker_capacity` | 5 / hectare | Full-staffed starter coal-mine worker demand for a 10,000 m2 extraction area |
| `coal_mine_basic.unit_price_currency` | 8.0 | Baseline local coal unit price used for local sourcing and OWA import pricing |
| `grain_farm_basic.base_rate_units_per_day` | 290.0 units/day/hectare | Full-staffed starter farm output before staffing and buffer limits |
| `grain_farm_basic.worker_capacity` | 1 | Worker count for the profile's staffing reference area |
| `grain_farm_basic.worker_capacity_area_m2` | 100,000 m2 | One worker per ten hectares, rounded up with a two-worker minimum |
| `power_plant_basic.base_rate_units_per_day` | 1200.0 units/day | Full-staffed starter power service production before staffing and coal-input limits |
| `power_plant_basic.inputs.coal` | 96.0 units/day | Coal fuel consumed by a fully staffed starter power plant |
| `power_plant_basic.unit_price_currency` | 3.0 | Local power service price per aggregate power unit |
| `water_plant_basic.unit_price_currency` | 2.0 | Local water service price per aggregate water unit |
| `wastewater_treatment_basic.unit_price_currency` | 1.5 | Local sewage service price per aggregate sewage unit |
| `owa_import_price_multiplier` | 1.75 | OWA import and missing local utility fallback price multiplier |
| `owa_export_price_multiplier` | 0.60 | Scheduled OWA surplus export price multiplier |
| `owa_distress_liquidation_multiplier` | 0.25 | Forced liquidation fire-sale price multiplier; must be no higher than scheduled export |

## Machinery Upkeep (`ECON-09`)

Machinery is one freight resource representing equipment and spare parts. It is a recurring
production input; there are no individual machines, wear timers, replacement events or agent
life-cycle rules. Inputs are bought locally first, then imported from OWA through existing
paid freight. Until a Machinery factory asset is installed, all Machinery comes from OWA.

Initial full-operation rates in `economy/profiles.toml`:

| Consumer profile | Machinery / operational day | Scaling |
| --- | ---: | --- |
| `grain_farm_basic` | 1 | Per hectare of committed field |
| `coal_mine_basic` | 4 | Per hectare of committed extraction area |
| `food_processor_basic` | 2 | Per building |
| `power_plant_basic` | 4 | Per building |
| `water_plant_basic` | 0.5 | Per building |
| `wastewater_treatment_basic` | 0.5 | Per building |
| `machinery_factory_basic` | 2 | Per building, retained from its own product stock |

A 10-hectare mine requires 40 Machinery/day; a 10-hectare farm requires 10. Farms keep their
existing worker density and two-worker minimum. Staffing, input availability and output headroom
scale actual consumption. Farms and mines consume inputs exactly once alongside actual production;
a nearly exhausted mine pays only for the output remaining in its reserve. An idle producer
consumes nothing. Depletion clears the mine's cached productive area, removing its worker capacity,
committed output capacity and Machinery orders/demand. The saved pit and extracted stock remain;
loading reconstructs the same zero capacity from the depleted reserve. Water/sewage operation and
power generation require Machinery stock as well as staff; the existing OWA utility fallback remains
available.

Changing an existing mine's extraction polygon retains that mine's cumulative extracted amount
(`AUDIT-01-M1`). The new authored-deposit sample sets its total reserve, floored at the amount
already extracted. Shrinking below that amount leaves zero remaining reserve/capacity; expanding
again adds only the reserve above cumulative extraction. Editing never grants startup funds again.
Commit, save and load share finite non-negative reserve validation with `extracted <= total`;
invalid depletion is rejected, and a failed save preserves the previous file. This retains the
existing per-building aggregate reserve model and save schema.

Farm and mine sites retain unique ascending building-owner indices through placement, deletion,
swap-remapping and undo (`AUDIT-01-M2`). Lookup and unrelated building removal cost O(log S)
without allocation. Replacing an existing mine updates its slot; adding or remapping a site may
shift O(S) entries in the existing vector, with no additional persistent index. Farm clearance
remains owned by the allocator and its existing lifecycle calls.

Five alternating, unprofiled release process pairs on CPU 0 with one Rayon worker measured 128
queries after three warmups, with 21 samples per process. Isolated site construction, checksums
and assertions are outside timing; the fixture does not run placement or production.

| Mine sites | Lookup before → after, ms | Unrelated removal before → after, ms |
| --- | --- | --- |
| 16 | 0.000497 → 0.000432 | 0.001608 → 0.000433 |
| 1,024 | 0.017615 → 0.001215 | 0.065513 → 0.001203 |
| 65,536 | 4.907083 → 0.003491 | 13.201446 → 0.002077 |

The release command is `taskset -c 0 BINARY --exact
simulation::extraction::tests::benchmark_extraction_site_lookup --ignored --nocapture
--test-threads=1`, with `RAYON_NUM_THREADS=1`. Source/binary identities, individual logs and
results are `/tmp/metrum-full-audit/site-order-{before,after}-identity.json` and
`site-order-matched-bench.json`. These timings cover lookup and removal of buildings without
production sites; they do not measure the O(S) shifts required by affected-site edits.

Machinery reference price is 20 currency/unit; OWA imports cost 35 at the shipped 1.75 multiplier.
These are initial game-balance values. Heavy mines are intended to benefit from local customers
and local Machinery: at full operation per hectare, 120 Coal sells locally for 960, imported
Machinery costs 140 and five workers cost 450/day before other charges. Low-price OWA exports
alone do not guarantee a profitable mine. Existing output and staffing rates are preserved.
A regression checks positive full-operation margins after maximum authored wages, Machinery/raw-material
imports, property tax, conservative utility charges and profit tax: existing grain/coal inputs are
local, new Machinery/Steel/Metals are imported, producers have local buyers, and utility plants have
sufficient service demand. Water/sewage examples use 200/250 daily service units. This proves viable
operating cases, not profitability for tiny fields, insufficient customers or every import/export
mix. Personal services and groceries do not receive Machinery inputs in this first pass.

Input stock targets scale with work area and hold at least two minimum shipment quantities
(80 units with the shipped 40-unit minimum). Positive reorder thresholds retain at least one
shipment quantity; zero thresholds use the existing target-minus-minimum-batch rule. This lets
small consumers reorder before running empty without dispatching tiny deliveries every hour.
Commercial activity scaling runs before this minimum is applied, so small shops retain the same
two-batch reserve. Input orders and export protection share the restock decision and indexed
supplier query; a buyer's export holds share one remaining purchase budget across all recipe ports.
An uncommitted field/mine requests nothing. Private startup capital covers seven days of wages
and the first input buffers at OWA prices; input inventory is delivered and paid for normally.
City service inputs are treasury-funded and included for every utility in daily city expenses;
power input costs and actual power-worker wages remain separate service subtotals. Refunds reduce
input spending on the day they arrive, including refunds for an earlier day's order; negative
input spending is displayed as returned money. The old money-divided-by-coal-price purchase
estimate was removed because mixed input spending cannot measure coal units. Coal stock and
consumption remain visible. Household consumption summaries derive their total from the supply
and service costs instead of maintaining a duplicate hourly ledger counter. Format 60 removed
the obsolete coal-purchase history field; format 61 also preserves unsettled household utility
payments and continues to read formats 57–60. Local demand is
protected before exports, including industrial, farm, mine and utility customers. A producer that
consumes its own output keeps its input target stock available when offering goods to local freight,
scheduled OWA exports or emergency liquidation. All three paths share the same stock-reserve helper.
Hourly output headroom uses net inventory growth when a recipe consumes its own output, including
the storage freed by that same production step.
The inspector shows all input buffers, including empty Machinery stock, and nominal daily usage.
Its fill percentage counts each resource once; a shared input/output slot uses the larger of its
input target and finite output capacity, rather than counting the same stock twice.

### Incoming small Machinery factory asset

The reusable `machinery_factory_basic` processor profile is ready; there is deliberately no
placeholder production asset. Register the user's model as a level-1, low-density, zoned-private
industrial building and select that profile. Staffing is four workers at 80–100 currency/day.
At full operation its recipe is **12 Steel + 4 Metals + 2 Machinery → 42 Machinery/day**,
leaving **40 Machinery/day net** for customers. Steel and Metals are separate goods: Metals
means non-steel metals. Their import-only reference prices are 8 and 16, so OWA prices are 14 and
28. Both materials can be imported immediately; ore/coal-to-steel and metal refining assets are
future content. The net output can cover one 10-hectare mine, four 10-hectare farms, or twenty
food-processing buildings at these initial rates.

Industrial demand now counts actual staffed business/area/utility input needs as well as the
existing market-scaled commercial inputs. Input shortages and OWA purchases do not erase the
local production opportunity; full output buffers and inactive worker slots do not create upstream
needs. Retained farm workers above current active capacity are excluded using the production
system's staffing rule. Matching resource deficits determine eligible factory types and spawn
volume. Existing, under-construction and selected factory outputs reserve capacity, including the factory's own
same-resource upkeep. Adding a profile without an asset cannot spawn a building. See
[`demand.md`](demand.md) and the authoring checklist in [`asset_editor.md`](asset_editor.md).

### Resource identity and cost

`[[resources]]` entries in `economy/profiles.toml` own explicit, contiguous, one-based `runtime_id`
values. These IDs are persisted by inventories, shipments and freight bookkeeping: append new
IDs and never reassign existing ones. IDs 1–6 retain the previous resource identities; Machinery,
Steel and Metals are 7, 8 and 9. File/name ordering no longer assigns identity. Import-only goods
may specify `import_unit_price_currency` without introducing a fake producer profile. Remove
that import-only price when an output profile becomes the resource's price owner. The economy
editor JSON/export round trip preserves the resource definitions. The unused exported compatibility
cache is removed; runtime compilation reads the canonical TOML. Profile IDs remain append-only by
authored order. Input and output ports share one compiler:
each resource appears at most once per direction, with a finite nonnegative rate. A resource may
appear on both sides for internal upkeep; duplicate entries cannot cause unaccounted consumption.

Sandbox scenarios declare `owa_import_resources` explicitly. The shipped grocery scenario imports
Machinery; its farms and processor pay the OWA price for actual missing input units while local
connected inputs retain their graph price. Undeclared disconnected inputs remain validation errors.
The economy editor exposes the import list on the scenario inspector and accepts fractional port
rates. This is aggregate editor playback; live freight still uses deliveries and stock batches.

### Runtime bounds and verification

Hourly production remains O(buildings) for a fixed catalog, with Rayon on the ordinary-building pass;
area production keeps its existing indexed site traversal. Input consumption allocates no memory
per building. Recipe work traverses inputs/outputs and performs bounded same-resource port lookups.
Demand computes each profile's capacity scale once per building and reuses its output-port pass
for the relevant resource totals. It reuses the existing parallel resource reduction and compact
resource arrays; with a fixed catalog it remains O(buildings), plus candidate output-port checks. No agent traversal
or additional spatial index is introduced. Freight uses the existing supplier/component indices,
route cache, reservations and bounded border capacity; protecting additional customer types adds
recipe-port work to its existing building pass.
The outbound-only reservation query scans active shipments in O(S), retains only source inventory
slots, and does not allocate inbound or border-job views. Its flattened storage ends at the last
active source/resource slot, independently of destination IDs. Both views share source-cargo
eligibility and reject resource IDs outside the catalog. A matched unprofiled release fixture with
4,096 orders, 256 sources and destination IDs through 69,631 improves from `0.741` to `0.015 ms`
per query (11 × 100 queries, 24 Rayon workers); logs and binary identities are under
`/tmp/metrum-full-audit/reservations-*`. This is a reservation-query measurement, not whole freight
throughput or a city-frame claim.
Daily-to-hourly production conversion has one shared constant. Demand and household settlement
share the same compact-resource aggregation helpers, and service activity/sales share one demand-rate
query. Service stores and explicit production areas also share their profile activity-scale calculation.
Resource order and accumulation order are preserved. Mine depletion updates its existing
cache in O(1) per site; inventory inspection and reserve protection perform bounded recipe-port work
without adding allocations or city-wide scans.

The `AUDIT-01` follow-up uses shared fixed-size parallel chunk folds and input-order merging for
floating-point activity and demand totals. Accumulators update in place; worker scheduling cannot
change the totals. Work remains O(buildings + households) for a fixed catalog, with O(N / chunk size)
temporary accumulators. No per-resident allocation or sort of chunk results is needed.

Fresh matched unprofiled release comparison: six alternating process pairs, `RAYON_NUM_THREADS=24`,
the existing `benchmark_business_demand_snapshot` factory fixture, 10 warmups then 11 samples of
100 snapshots per size; setup is excluded and no compilation/Godot work runs concurrently.
Median milliseconds/snapshot before → after: 1,024 buildings `0.092 → 0.028`,
8,192 `0.125 → 0.094`, 65,536 `0.2905 → 0.252`. This measures business snapshots, not city FPS
or all mixed household/industry workloads. Command on each saved release test executable:
`benchmark_business_demand_snapshot --ignored --nocapture`. Raw runs and exact binary identities:
`/tmp/metrum-full-audit/reduction-long-matched-bench.json` and `reduction-long-binaries.sha256`.
The before build restores the initially inventoried metrics/snapshot sources in the isolated
`reduction-baseline/` crate. Earlier short-window timings are superseded by this comparison.

Follow-up audit verification on 2026-09-12: the release Rust suite passed 1,709 tests (16 ignored),
including proportional upkeep, paid startup imports, shared-budget local holds, inactive farm demand, net
output headroom, small-shop restocking, invalid recipe rejection, industrial absorption and save
round trips. Added regressions cover depleted-mine capacity restoration, upkeep/shipment reserves
during liquidation, and distinct-resource inventory fill. Profitability checks include maximum
authored wages and farm/mine property taxes; utility accounting checks distinguish power payroll
and include prior-day input refunds.
All eight headless Godot regressions passed, including native economy-editor export/playback,
the Machinery asset selector, refund display, farm inspector and camera save/load. A copy of the
existing `farms.sqlite` passed two load/New Game cycles with 22 grounded models and chunk `(22, 0)`
published; the original save was untouched. Logs: `/tmp/metrum-machinery-audit2-tests.log`,
`/tmp/metrum-machinery-audit2-<test_script>.log`, `/tmp/metrum-machinery-audit2-farms-load.log`.
The editor checks cover Godot's floating-point JSON representation of whole-number resource IDs
and construction durations; fractional durations remain invalid. The release library was rebuilt
and deployed. The 15 Python runner/report checks, benchmark-target compile check, formatting and
diff checks pass. Rustdoc reports no missing-doc warnings. Clippy reports no warnings on changed
lines; it completes with existing repository warnings when the pre-existing `clippy::mut_from_ref`
error in `agents/tick/slices.rs` is allowed. Compiler/check logs use the same
`/tmp/metrum-machinery-audit2-` prefix (`build`, `clippy`, `rustdoc`, `bench-check`,
`python-terrain`, `python-report`).

Fresh follow-up audit demand benchmark on 2026-09-12: matched, unprofiled release builds, 24 Rayon
workers, 11 samples per size, median snapshot time. The fixture holds the road graph fixed and varies
staffed food processors with stocked input buffers and a 10,000 operating budget; setup/cloning
is excluded. Both builds include Machinery. Before is the previously audited working tree over
`a91d6159`, captured in `/tmp/metrum-machinery-audit2-before.diff`; after includes the follow-up
fixes and shared resource helpers.

| Buildings | Before audit | After audit |
| ---: | ---: | ---: |
| 1,024 | 0.106 ms | 0.108 ms |
| 8,192 | 0.141 ms | 0.132 ms |
| 65,536 | 0.448 ms | 0.281 ms |

Command: `RAYON_NUM_THREADS=24 cargo test --offline --manifest-path rust/Cargo.toml --release --lib benchmark_business_demand_snapshot -- --ignored --nocapture`.
Fixture: `rust/src/simulation/economy/demand/tests.rs::benchmark_business_demand_snapshot`.
Logs: `/tmp/metrum-machinery-audit2-bench-before.log`, `/tmp/metrum-machinery-audit2-bench-after.log`.
The audited source/config identity is recorded in `/tmp/metrum-machinery-audit2-source.sha256`
(`750a4070e2bf599585e849e7a479b48975c4a77dbe1313f5be1c2dce60aae987`).
The final run saves 0.167 ms at 65,536 buildings; small-fixture results are shown above.
The fixed-catalog linear bound is unchanged. These measurements cover demand aggregation,
not end-to-end freight or million-agent throughput.

## Farm Households (`ECON-08`)

Each explicit field-producing farm provides exactly one household slot, independent of field
area and worker capacity. Ordinary demand-owned immigration or household relocation fills that
slot; placing a farm does not manufacture residents. A pending arrival reserves the slot once
and materializes the whole household through the existing carrier lifecycle.

Farm residents are ordinary agents linked to one ordinary household, with the existing adult,
child, and elder composition, shared supplies, budget, shopping, and transfer payments. Only
adults may work. No ownership, inheritance, family trees, or aging transitions are introduced.
Farmhouse living area uses `flat_size_m2`, defaulting to 120 m2 when unspecified, and the normal
deterministic starter-family sizing rules. The asset editor fixes farm household capacity at one
and preserves the farmhouse living-area setting.
Rust's building asset contract owns the farmhouse default and one-family capacity; editor
import/export preserves custom living areas and normalizes farm capacity to one.

The farm retains `max(2, ceil(field_hectares / 10))` physical grain-farm jobs for a committed
field. These are total positions, not two additional vacancies beyond the resident workers.
Resident adults consider their own farm with zero commute cost and claim available on-site jobs
before external applicants in the same assignment pass. Remaining positions use normal hiring
from other homes; growth does not create extra households. Existing market, affordability, job
lock, and payroll rules still apply. Employed resident adults switch home/work activity at their
authored shift times, including overnight shifts, without planning or taking a road trip.

Housing and employment remain separate: shrinking a field can reduce jobs but preserves the
household and its residence. Losing an on-site job returns an agent at home to home activity.
Ordinary housing affordability, relocation, abandonment, demolition, and save/load rules still
apply. Farm operating money remains separate from household money; existing farm business taxes
remain in place.
Bankruptcy withdraws any remaining vacancy immediately, before admission or forced rehousing.
Automatic removal invalidates the home address while preserving household membership and family
money/supplies; the ordinary unhoused-household lifecycle handles the displaced family.

Farms share the residential vacancy list while remaining explicit industry sites outside the
residential zoning/growth index. Housing capacity and claims remain O(1). Demand's existing
parallel building reduction counts the farm's home and jobs independently. On-site employment
reuses the sorted job-supply snapshot in O(log J) per home option build, with deterministic
resident-first claims; it adds no spatial index or per-agent job scan. The movement addition is
allocation-free O(1) per in-building worker and performs no pathfinding for on-site work.
The farm inspector displays household occupancy out of its one slot, resident children/adults/elders,
and the ordinary household budget, supplies, and replenishment details alongside the business.
It reuses the residential inspector's O(H) household aggregation on opening/hourly refresh;
there is no additional simulation-tick work or agent scan, and commuting workers are excluded.

Pre-audit validation on 2026-09-11: `cargo test --lib` passed 1,681 tests (13 ignored), including
family admission/relocation, mixed-age employment, housing/job counts, in-place shifts,
home-bound trip preservation, and occupied-farm SQLite load, removal/remapping and demolition
undo. `cargo build` and `cargo doc --no-deps` passed without warnings. The deployed extension
passed the headless field-tool regression and inspector/asset-editor script checks.
Logs: `/tmp/metrum-farm-household-all-tests.log`, `/tmp/metrum-farm-household-build.log`,
`/tmp/metrum-farm-household-rustdoc.log`, `/tmp/metrum-farm-household-field-ui.log`,
and `/tmp/metrum-farm-household-godot-check.log`.

Matched unprofiled release timing compares idle residents with on-site workers in the same
minimal one-road/one-building fixture, with four Rayon workers, one test thread, three warmup
ticks, and 21 samples of ten ticks each. Agent creation and fixture setup are untimed; the
synthetic population isolates movement and does not model household admission or economy cost.
Both cases preserve in-building state and issue zero pathfinding requests.

| In-building agents | Idle, ms/tick | On-site work, ms/tick |
|---|---|---|
| 1,000 | 0.056 | 0.031 |
| 10,000 | 0.059 | 0.069 |
| 100,000 | 0.634 | 0.575 |

These timings show comparable tick cost, not a speedup claim; small batches include scheduling
overhead. Build identity: `325f0402` plus the farm changes; release test binary SHA-256
`5f0663cc15bc2c67996e309dad22b618d2e55e3caf2ea2b0b2b4fe4d97460107`;
`agents/tick/schedule.rs` SHA-256
`df8b76823bcbbcaf404317304090fb6a719e846c1fb45d8a7ffeaa53c3cb3e13`;
`households/tests/farms.rs` SHA-256
`d475a97bb4dec9d1b8c758078a1cf5b1198cf1ed28d13cb378f48affdc381782`.
Commands from `rust/`: `cargo test --release --lib --no-run`, then
`RAYON_NUM_THREADS=4 target/release/deps/metrum_rise-9cc8d57998aa4451 benchmark_farm_household_tick --ignored --nocapture --test-threads=1`.
Build/run logs: `/tmp/metrum-farm-household-release-build.log` and
`/tmp/metrum-farm-household-benchmark.log`. Timing ran after builds and other checks finished.

### Farm audit validation (2026-09-11)

The audit fixed lost farmhouse living-area exports, stale bankrupt-farm vacancies, household
membership loss during automatic removal, and attachment checks using a differently rotated lot
from the visible guide. Farm classification/defaults, the field worker floor, vacancy removal,
and the plot-boundary payload now have shared owners. Unused agent-owned home selection and
person-counted occupancy rebuilding were removed; housing fixtures claim household slots through
the allocator.

Fresh checks: `cargo test --lib` passed 1,684 tests (14 ignored); `cargo build` and
`cargo doc --no-deps` passed without warnings. The rebuilt/deployed extension passed the headless
field-tool regression and industry-tool, inspector, and asset-editor script checks. Logs:
`/tmp/metrum-farm-audit-all-tests-final.log`, `/tmp/metrum-farm-audit-build.log`,
`/tmp/metrum-farm-audit-doc.log`, and `/tmp/metrum-farm-audit-{field-tool,industry-tool,inspector,asset-editor}-stdout.log`.

Release checks ran after compilation finished. The vacancy benchmark holds 10,000 claim/release
pairs on one farm fixed, with untimed setup and index allocation, three warmups, 21 samples, one
test thread, and `RAYON_NUM_THREADS=4`. Median times were 0.583 / 0.580 / 0.560 ms with
1 / 1,000 / 100,000 buildings, consistent with allocation-free O(1) claims independent of
background housing. The capacity benchmark retained 2.43 ns/query for the field minimum through
1,000,000 queries (`RAYON_NUM_THREADS=1`). Logs: `/tmp/metrum-farm-audit-vacancies.log` and
`/tmp/metrum-farm-audit-capacity.log`. Run the release test binary with
`benchmark_farm_vacancy_claims` or `benchmark_work_area_capacity`, then
`--ignored --nocapture --test-threads=1`.

Matched pre/post-audit resident-worker tick runs reuse the fixture above and its unchanged
three-warmup/21-sample/ten-tick protocol with four Rayon workers. Six alternating before/after
runs gave the following medians of run medians; every run retained in-building state and zero
pathfinding requests:

| Resident workers | Before audit, ms/tick | After audit, ms/tick |
|---|---|---|
| 1,000 | 0.022 | 0.021 |
| 10,000 | 0.072 | 0.082 |
| 100,000 | 0.619 | 0.567 |

Sub-millisecond timings vary between runs; these isolated checks show no consistent regression
with population, and do not measure end-to-end city economy cost. The full idle/working samples
are in `/tmp/metrum-farm-audit-tick-comparison.json`; individual runs are
`/tmp/metrum-farm-audit-{before,after}-tick-{0..5}.log`.
Build command: `cargo test --release --lib --no-run`; build log:
`/tmp/metrum-farm-audit-release-build.log`. Compare `/tmp/metrum-farm-audit-before-tests` and
`rust/target/release/deps/metrum_rise-9cc8d57998aa4451` using
`benchmark_farm_household_tick --ignored --nocapture --test-threads=1`.
Build identity is `325f0402` plus the working farm changes and audit fixes. Before/after release
binary SHA-256: `5f0663cc15bc2c67996e309dad22b618d2e55e3caf2ea2b0b2b4fe4d97460107` /
`5db37fd42543f05b5d62dd6c34792693a4a1d21fd4085eecf93ee28a0726ff9a`.
Audit `allocator/index.rs` SHA-256:
`550e5d36c95865740f6c71e33ef10991f2c140204fa9cf2b50d98a0b95f2917e`;
`households/tests/farms.rs` SHA-256:
`a12909922973c94c3d06dd0a7e2d1e3f281707336733a10f3d1d9bfd92ceddfd`.

### Full changed-file cleanup (2026-09-11)

The 85-file audit removes unused native/editor APIs, duplicate polygon-area/yield helpers,
repeated housing checks, and manifest-worker fallbacks in live UI/accounting paths. Field and
extractor validation return the already-computed finite area; both reuse the hectare scaler.
Field preview and commit share readiness preparation. Road-footprint preparation computes bounds
while converting vertices, eliminating its intermediate polygon buffer without changing geometry.
Housing checks use the allocator's capacity contract. City diagnostics classify both farm job
capacity and filled jobs through the same helper, correcting missing employed farm workers.

Industry placement resets only when deactivated or completed, preserves committed farms during
cleanup, and displays placement errors in the tool notification. It performs no inactive frame
work. Regressions cover cancellation exactly once, committed-farm preservation, finite production
areas, and farm household/job diagnostics. The existing field, household, terrain and camera
regressions remain in place.

Fresh verification: **1,689 release Rust tests pass**, with 15 measurement cases ignored by the
ordinary suite; release build and rustdoc have no warnings. All 12 changed GDScripts parse, and
nine headless runs pass: field editing, vehicle support, network chunks, road metrics, junction
preview, preview streaming, camera save/load in both modes, and two actual farm-save load → New
Game cycles. The latter checks 22 grounded model parts and successful chunk `(22, 0)` publication.
Logs use `/tmp/metrum-full-audit-`, including `final-tests.log`, `build.log`, `rustdoc.log`,
`field_edit_tool.log`, and `farms_load_new_game.log`. The original farm save remains unchanged.
Default Clippy still rejects the pre-existing `mut_from_ref` in unchanged agent `tick/slices.rs`;
the audit run allowing only that lint completes with the existing lint warnings and no errors.

Unprofiled release locality uses `populated_field_road_plan_scaling --ignored --nocapture`,
24 Rayon workers, 100 samples per level and the existing fixed four-site neighborhood. Both
builds retain identical local products as background fields/buildings/parcels grow, reaching
391 remote roads and 600,024 agents. Build and renderer checks finish before timing.

| Remote fields/buildings | Compile p50 before / after (ms) | Readiness p50 before / after (ms) |
|---:|---:|---:|
| 0 | 20.302 / 19.986 | 0.101 / 0.091 |
| 1,000 | 19.816 / 19.476 | 0.102 / 0.099 |
| 10,000 | 20.375 / 19.789 | 0.107 / 0.104 |
| 100,000 | 20.240 / 20.884 | 0.116 / 0.121 |

Repeated planning remains local; the existing one-time snapshot is separate and reaches
3.369 / 3.394 ms at the largest size. Logs: `/tmp/metrum-full-audit-field-perf-{before,after}.log`.
Before/after release test binaries have SHA-256 `320340841a98d0a72e7e025e3108b2cf1cb7bf991fd0f0bf0f3abc17202326cc`
and `eae0e5612f497741331efb9600b1db2d96e55480db7fc3ab656920b052599fd8`;
paths and deployed-library identity are in `/tmp/metrum-full-audit-build-identity.json`.

Three alternating headless before/after industry-tool runs use 21 timed batches of 10,000 manual
idle updates after three warmups. Median cost falls from 1.116–1.150 to 0.066–0.069 microseconds
per direct call; actual inactive tools now disable processing entirely. Setup is untimed, both
scripts use the same release extension, and no build or other benchmark runs concurrently.
Reproduction script: `/tmp/metrum-full-audit-idle-benchmark.gd`, selecting the saved baseline or
current tool with `METRUM_AUDIT_INDUSTRY_SCRIPT`; logs: `/tmp/metrum-full-audit-idle-{before,after}-{1,2,3}.log`.

## Field Staffing Density (`ECON-06`)

Field producers and extractors keep output and storage rates per 10,000 m2. Their integer
`worker_capacity` applies to `worker_capacity_area_m2` instead, which defaults to 10,000 m2
and must be positive and finite. Ordinary building profiles still use fixed worker counts.
The economy editor exposes the staffing area for fields/extractors; its sandbox uses a
one-hectare site for both output and payroll.

The runtime compiles workers per hectare once, then calculates physical field jobs as
`max(2, ceil(workers_per_hectare * field_hectares))` for positive areas and worker densities.
Uncommitted fields and disabled worker densities have zero jobs; extractors retain ordinary
rounding without the field minimum. Market availability and payroll funding still gate active
jobs. Grain farms provide two jobs up to 20 ha, three just above 20 ha, and ten at 100 ha.
The two-worker floor preserves the authored one-worker-per-ten-hectares scaling above it.
This is a game-balance starting point, not a measured real-world agricultural staffing rate.

Full staffing still produces 290 grain/day/hectare. Partial staffing uses the new physical
capacity as its denominator; demand equivalents, export reserves, and first-field startup
payroll use the same density and minimum. The editor sandbox applies this minimum too.
Saved field areas rebuild this scale on load, and the daily wage pass releases surplus workers
through the existing assignment lifecycle. No save migration
or new per-agent work is needed. The inspector reports physical field capacity, with active
positions shown separately when market-limited. Density queries remain allocation-free O(1).

This corrects job density, not total farm profitability: reduced payroll increases margins.
Crop yields, sale prices, wages, and mechanization costs need a separate economy calibration.

Regression coverage checks hectare boundaries, hourly yield, startup payroll, demand equivalents,
surplus-worker release, export reserves, invalid staffing areas, and editor export/reload.
Validation before the two-worker floor: `cargo test --lib` passed 1,663 tests (11 ignored), `cargo doc --no-deps`
reported no warnings, and Godot `--check-only` passed for the economy editor and building inspector.
The Rust test log is `/tmp/metrum-farm-all-tests.log`.
The isolated release timing is reproducible from `rust/` with
`cargo test --release benchmark_work_area_capacity -- --ignored --nocapture`.

Capacity-only timing before the two-worker floor on 2026-09-11 used the release build from
`325f0402` plus the initial staffing change, one test thread, 21 unprofiled samples per size,
and eight fixed hectare values repeated across each query batch. With other builds finished, median times were
0.023 ms / 10,000 queries, 0.233 ms / 100,000, and 2.328 ms / 1,000,000 (about 2.33 ns/query).
This measures the capacity helper only; it is not an end-to-end city-speed comparison.
The measured `work_area.rs` SHA-256 is
`4002cbb2b49af54bef393c29cf211aa28463016345d3889bbdd44fd3eef2719c`.
Build/run logs: `/tmp/metrum-farm-capacity-benchmark.log` and
`/tmp/metrum-farm-capacity-benchmark-quiet.log`; the latter reruns the built release test binary
with `benchmark_work_area_capacity --ignored --nocapture --test-threads=1`.

Fresh validation with the two-worker floor and plot guides on 2026-09-11:
`cargo test --lib` passed 1,674 tests (12 ignored), and `cargo build` plus
`cargo doc --no-deps` completed without warnings. The deployed debug extension passed the
headless field-tool regression; industry-tool and inspector script checks also passed.
Logs: `/tmp/metrum-farm-minimum-tests.log`, `/tmp/metrum-farm-boundary-build.log`,
`/tmp/metrum-farm-boundary-rustdoc.log`, `/tmp/metrum-farm-boundary-ui.log`, and
`/tmp/metrum-farm-boundary-parse.log`. Headless checks used `--log-file /tmp/...` so they
did not need to write the game's user-data directory.

The current release capacity benchmark compares the same helper with minimums of zero and
two, using the same eight hectare values, 21 unprofiled samples per size, one test thread,
and `RAYON_NUM_THREADS=1`. It performs no city setup or allocation in the timed query loop.
After builds finished, medians were:

| Queries | Density only, minimum 0 | Farm, minimum 2 |
|---|---|---|
| 10,000 | 0.047 ms | 0.024 ms |
| 100,000 | 0.254 ms | 0.243 ms |
| 1,000,000 | 2.499 ms | 2.431 ms |

This checks O(1) capacity-query cost in the current build; it is not a whole-city comparison
or a comparison against the old implementation. Build identity: `325f0402` plus the field
changes, `work_area.rs` SHA-256
`da5d368d395d3e8dec02b9e3574efdf637d7f07453ade6e12f65310a74851048`, release test binary SHA-256
`fd548c59bc9d5574a705be67d6cc9a0bbc7952569cac14c8810d671ac932ad15`.
Commands from `rust/`: `cargo test --release --lib --no-run`, then
`RAYON_NUM_THREADS=1 target/release/deps/metrum_rise-9cc8d57998aa4451 benchmark_work_area_capacity --ignored --nocapture --test-threads=1`.
Logs: `/tmp/metrum-farm-minimum-release-build.log` and `/tmp/metrum-farm-minimum-benchmark.log`.

## Field Placement and Editing (`ECON-07`)

Committed fields reserve their polygon interiors. A new or resized field must stay inside the
world, meet the existing 100 m2 minimum and 10 m farm-link rule, and avoid road carriageways,
curbs, sidewalks, building sites (including its own farm), every authored zoning parcel, and
other fields. Shared edges/vertices are allowed. Clearance uses polygon intersection, including
concave shapes and full containment. Empty/free parcels still reserve their land.

Road creation, explicit building placement, and zoned building/parcel placement reject field
overlap too. Final road-plan validation checks the compiled local junctions as well as the
stroke width; it rechecks live reservations before consuming the plan. Road preview caches
include the field revision, so editing or deleting a field invalidates their land-use verdict.

The farm inspector's **Edit Field** button exposes its existing vertices. Each mouse release
validates and commits the new polygon atomically, updates area/work-area scale and the field
reservation, and returns current worker capacity and production ratio to the open inspector.
Rejected moves restore the last accepted polygon and show the reason. Escape/right-click ends
editing and discards only the unfinished drag. Previously accepted releases remain committed.
Resize grants no additional startup money. Hourly output uses the new area; the existing daily
employment pass handles any surplus workers after shrinking. A changed building identity or
previous polygon rejects a stale editor session.

Initial field drawing and inspector resizing display the farm lot in yellow and the exact
blocking building/yard footprint in red, with a translucent red fill and a matching legend.
These guides remain visible throughout polygon authoring and clear when the tool finishes or
cancels. Rust exports both authoritative boundaries when the farm is placed or inspected;
Godot builds the guide once per tool session, without per-frame simulation queries. Boundary
export, outlines, and the convex site fill are O(V).
Attachment checks reuse the same authored lot as the guide, including custom frontage rotation;
the separate road-aligned footprint reconstruction and duplicate bridge payload are removed.

Agriculture owns saved polygons. The allocator keeps derived field references in the existing
512 m world chunk layout, covering their entire bounds: the building-center index alone cannot
find distant field corners without increasing every building query's search radius. Replacement
touches only the old/new field chunks. Parcel and building checks reuse their existing indices;
road checks reuse the surface query grid. Query cost follows touched chunks and local polygon
geometry, independent of remote population/field count. Rectangle broad-phase queries use stack
storage; exact intersection scratch is limited to local candidates. There is no new per-agent
work or city-wide render snapshot on vertex release.

SQLite load rebuilds reservations from saved sites. Both ordinary allocator cleanup and explicit
demolition publish production-site owner remaps; demolition undo restores the field reservation.
Rust regressions cover land conflicts, stale/invalid drag rollback, capacity changes, startup
funding, owner remapping, SQLite load and demolition undo. The headless
`godot/tests/field_edit_tool_test.gd` exercises vertex selection, release, rollback, cancellation,
and boundary-guide visibility during drawing and resizing. It runs in `./run.sh --test`.

Validation before plot guides and the two-worker floor on 2026-09-11:
`cargo test --lib` passed 1,673 tests (12 ignored); `cargo build`
and `cargo doc --no-deps` completed without warnings. The rebuilt extension passed the headless
field editor and native road junction/preview regressions. Input manager and inspector script
checks also passed. Logs: `/tmp/metrum-field-all-tests.log`, `/tmp/metrum-field-ui-test.log`,
`/tmp/metrum-field-road-bridge.log`, `/tmp/metrum-field-native-build.log`,
`/tmp/metrum-field-rustdoc.log`.

Matched unprofiled release runs used four Rayon workers, one test thread, 100 measured samples
per size after three warmups, and the same fixed local road/sites. Fixture setup was untimed;
the existing once-per-edit snapshot grew from 0.007 to 3.464 ms and is excluded from repeated
planning. Both runs preserved identical local terrain products through 100,000 background
buildings/parcels, 391 background roads, and 600,024 total agents. The field run also adds one
fixed local reservation and one per background building.

| Remote buildings/fields | Compile p50, no fields / fields (ms) | Readiness p50, no fields / fields (ms) |
|---:|---:|---:|
| 0 | 18.818 / 18.628 | 0.016 / 0.094 |
| 1,000 | 18.662 / 18.673 | 0.017 / 0.101 |
| 10,000 | 18.854 / 18.600 | 0.017 / 0.103 |
| 100,000 | 18.740 / 18.645 | 0.017 / 0.101 |

Reproduce from `rust/`: build with `cargo test --release --lib --no-run`, then run the emitted
test binary separately with `RAYON_NUM_THREADS=4` and either `populated_road_plan_scaling` or
`populated_field_road_plan_scaling`, followed by `--ignored --nocapture --test-threads=1`.
Build: `325f04021c2f1d8be7f189aa3e46120bb1010c03` plus these working-tree changes; binary SHA-256
`03157cecf2434837da368ae298433024367b9817967ea5bb897b47ae9e5b9ded`.
Results: `/tmp/metrum-field-road-baseline.log`, `/tmp/metrum-field-road-scaling.log`,
`/tmp/metrum-field-benchmark-identity.json`. These measure local placement, not whole-city tick
throughput or interactive GPU rendering.

## Building Bankruptcy

**Status: implemented.** The two-day `budget_distress` bankruptcy check is live in `households.rs`
(`run_bankruptcy_check`, `daily_settlement` four-step sequence). `budget_distress: bool` is
persisted in the SQLite schema and loaded by `world.rs`.

This section is the authoritative spec for how commercial, industrial, and privately operated
utility buildings manage their operating budget, pay obligations, and enter bankruptcy. City-owned
explicit service buildings are treasury-funded municipal facilities in the live `v0.1` path; they
provide staffed service and city revenue but do not enter the private-building bankruptcy loop. The
previous system used an hourly utility gate (`utility_service_available`) that permanently froze
any building whose budget dipped below a single hourly charge — see ECON-01 in the Current
Simulation Status section for the incident record. This spec replaced that system entirely.

### Operating Budget

Each commercial, industrial, and privately operated utility building holds an `operating_budget:
f32` cash balance. It is separate from household budgets and the city treasury. City-owned service
buildings still have runtime telemetry fields, but their payroll and local operator receipts settle
against the city treasury.

Money enters the budget from:

- sales revenue when households or other buildings purchase the building's output
- utility service revenue distributed to private local provider buildings when consumers pay charges

Money leaves the budget from:

- daily wage payments to workers
- daily utility cost charged once per day on the same cadence as wages

The budget is allowed to go negative. A negative budget is not immediately fatal — it triggers a
distress window with a forced liquidation attempt before bankruptcy is declared.

### Startup Float

When a commercial or industrial building first spawns it receives a one-time startup float set at
construction time in the spawn path:

```
startup_budget = max(
  worker_capacity * average_daily_wage * STARTUP_RUNWAY_DAYS + first_owa_input_import_cost,
  STARTUP_OPERATING_FLOAT
)
```

For explicit field producers and extractors, `worker_capacity` in this formula means the current
area-scaled active worker capacity. A newly placed uncommitted field or extraction building has
zero active area-backed workers and therefore receives only `STARTUP_OPERATING_FLOAT`.

`first_owa_input_import_cost` is computed from the building profile's input target units, the
catalog resource unit price, and `runtime_tuning.owa_import_price_multiplier`. Constants:
`STARTUP_RUNWAY_DAYS = 7`, `STARTUP_OPERATING_FLOAT = 500.0`.

No daily refill mechanism. The float is given once at spawn. If the building spends it without
becoming viable, the daily settlement sequence handles the outcome.

### Daily Settlement Sequence

The following steps execute once per day for every commercial, industrial, utility, and explicit
field/extractor business that is not already `is_deserted` or `broken`. Explicit field/extractor
assets may keep `ZoneType::None` for placement legality, but their economy profile classifies them
as private industrial businesses for utility billing, industrial property tax, distress liquidation,
and bankruptcy. Order is fixed and deterministic.

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

For each employed worker, deduct `daily_wage` from the employer budget and credit the worker's
household. Private employers pay from `operating_budget`; city-owned service buildings pay gross
wages from the city treasury. If a private employer's `operating_budget < daily_wage` for a given
worker, the payroll step first sells enough unreserved output inventory through the emergency
`OWA` liquidation path to cover that wage, using
`local_unit_price × owa_distress_liquidation_multiplier`. If cash is still insufficient after
that sale, the worker goes unpaid for the day (`consecutive_unpaid_days` increments). Workers
self-terminate after `JOB_UNPAID_ABANDON_DAYS` (currently 2) consecutive unpaid days. Private
building budget does not go negative from wage payments — a building that cannot pay a worker
simply fails to pay, not force-debits. The city treasury may go negative as a fiscal state.

Household budgets remain authoritative throughout settlement. Agent money mirrors are published
once after wages, transfers, taxes, utility settlement and housing/workplace updates; payroll does
not publish an intermediate copy that no settlement phase reads.

Employment cleanup (`AUDIT-01`, 2026-09-12) shares active/funded/budget-backed job capacity and
quota enforcement. Staffing retains lower agent IDs first in O(A + B), with O(B) temporary storage;
when no buildings need a quota, it skips the O(A) retention scan. Capacity evaluation remains
parallel; final integer count publication is serial. The funding limit vector now uses explicit
optional capacities (eight rather than four bytes per building), replacing the sentinel convention.
Ordered Rayon collection already preserves agent/building/key order, so duplicate sorting is gone;
the distinct resident-first workplace assignment sort remains.

Matched unprofiled release measurements use 1,024/8,192/65,536 workers, 11 timing samples, five
alternating process pairs, and 24 or one Rayon worker. Setup/reset is outside staffing timing;
`benchmark_payroll_phase` includes payroll and final money publication, four calls per sample.
At 65,536 workers, 24-worker medians are 0.235 → 0.253 ms without power quotas,
0.463 → 0.580 ms with quotas, and 2.683 → 2.081 ms for payroll. Process variation is substantial
(power before 0.379–1.841 ms, after 0.395–0.665 ms), so these do not establish an overall speedup.
Single-worker staffing medians are 0.130 → 0.106 ms and 0.377 → 0.317 ms; payroll ranges overlap.
The bounded quota overhead is accepted for the existing hourly/daily cadence; there is no new
per-agent allocation or city-wide index. Commands are the two ignored benchmark names with
`--ignored --nocapture --test-threads=1` on preserved release binaries. Exact identities, workload
source and raw samples are under `/tmp/metrum-full-audit/employment-{before,final}-*` and
`employment-final-matched-bench.json`. These fixtures do not measure complete job route searches.

**Step 3 — Pay utility cost.**

Private businesses pay daily utility costs even when this makes their operating budget negative.
Each private non-residential or explicit field/extractor business consumes one aggregate unit per
service daily. The authored baseline prices are shared across these businesses:

| Service | Local unit price | OWA unit price (× 1.75) |
| --- | ---: | ---: |
| Power | 3.0 | 5.25 |
| Water | 2.0 | 3.50 |
| Sewage | 1.5 | 2.625 |
| Total daily cost | 6.5 | 11.375 |

`power`, `water`, and `sewage` resolve independently. `Power` uses aggregate daily produced units:
consumers pay the local authored utility price only for the covered share, and uncovered private
demand pays the OWA fallback share. If a staffed local `water` or `sewage` provider exists,
consumers pay that service's local authored utility price; otherwise they pay that service's OWA
fallback share. Local service fees for city-owned providers deposit into the city treasury;
provider `revenue` remains telemetry. OWA fallback spend leaves the local economy.

Residential buildings pay household utility costs from the household budget on the existing hourly
cadence. The hourly charge is split into power, water, and sewage ledger buckets. Daily utility
settlement routes the power bucket into local power revenue only up to aggregate local power
coverage, and routes water/sewage buckets to local providers only when those services are available,
without charging households a second time.
Only cash actually deducted is recorded as a household utility payment. The hourly base charge
and daily OWA surcharge are capped by remaining household cash; they do not create household
debt. Unpaid amounts cannot become provider revenue or recorded OWA spending. The base utility
prices and service-name lookup are shared with operating-buffer estimates.

Save format 61 stores the three collected utility payments with each household and restores them
before load-time repair or settlement, without charging cash again. Formats 57–60 had no such
fields and start those amounts at zero; previously discarded payments cannot be reconstructed.
Other diagnostic counters begin a new report window after loading. Saved payment amounts must be
finite and nonnegative, and all four household age/member counts must fit their `u16` storage.
The daily simulation step owns the accounting-window reset after settlement and reporting;
diagnostic printing is read-only. The unused `HouseholdSystem::clone` implementation is removed.


Utility accounting validation (`AUDIT-01`, 2026-09-12): both cash-limit regressions failed before
the fix; the full release suite now passes 1,731 tests (25 ignored). The matched unprofiled release
fixture runs 24 hourly household charge/stock passes plus one daily utility settlement, with
11 samples and five alternating process pairs, 24 Rayon workers. At 1,024/8,192/65,536 households,
median daily-block time is 1.689/1.869/4.723 → 1.519/1.904/5.187 ms. Large-fixture process ranges
overlap (before 3.823–6.467 ms, after 3.543–5.463 ms); no speedup is claimed. The fix adds O(1)
arithmetic per household to the existing O(H + B) settlement, with no new allocation. Setup and
ledger reset are excluded; the fixture uses funded households and OWA-only utility service.
Run `benchmark_household_utility_settlement --ignored --nocapture --test-threads=1` on the
preserved release binaries. Source/binary identities, raw runs and tests are under
`/tmp/metrum-full-audit/utility-*`. The subsequent save/lifecycle batch validates mid-day payment persistence separately.



**Step 4 — Distress resolution.**

```
if operating_budget < 0:
    forced_owa_liquidation()   // sell all unreserved inventory at distress OWA prices
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

`forced_owa_liquidation` iterates every output resource slot and sells unreserved inventory at
`local_unit_price × owa_distress_liquidation_multiplier`, crediting `operating_budget`
immediately. Payroll liquidation sells only enough inventory to cover the next wage; end-of-day
distress liquidation sells all available unreserved output inventory. The distress multiplier must
be no higher than the scheduled `owa_export_price_multiplier` and is lower in the shipped tuning
(`0.25` vs `0.60`) so liquidation is a fire-sale rescue path rather than a profitable operating
model. It bypasses the normal `min_shipment_units` buffer check — the sale is a distress action,
not a scheduled shipment. If inventory is empty (e.g. a ghost farm with no workers and no
production), the liquidation yields nothing and `budget_distress` is still set to `true`.

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
        worker_count[work_building[i]] -= 1
        work_building[i] = MAX
        job_lock_days[i] = 0
        consecutive_unpaid_days[i] = 0
```

Ejected workers then enter the normal job-scoring loop on the same day and can be assigned
immediately. Do not rely on the unpaid-wage path to clear these workers — that path takes two
additional days and leaves workers attached to a building that no longer runs throughput, producing
a misleading `worker_count` reading on the dead building.

`assign_agent_workplaces` skips any candidate building where `is_deserted == true`, regardless of
whether the agent is already assigned there.

Before daily payroll, the wage pass also sheds stale workers above current active capacity for
profiles whose active slots are demand-limited rather than purely physical: `service_store`
commercial profiles and explicit field/extractor profiles. This keeps logged `worker_count` aligned
with the current market-backed slots instead of waiting two unpaid days for surplus workers to quit.

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

- unbounded per-agent daily shopping or `Home -> Work -> Shop -> Home` loops as the baseline
  essentials model; the allowed `v0.1` replacement is a bounded one-shopper household
  replenishment task
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

Startup capital is computed as `max(500, worker_capacity * avg_daily_wage * 7 + first_owa_input_import_cost)` for all commercial and industrial buildings at spawn. The shipped `food_processor_basic` receives **15,820**: 6,300 for wages, 6,720 for Grain and 2,800 for Machinery. The shipped `grocery_basic` receives **22,050**, and `machinery_factory_basic` receives **8,680**, including their first full input buffers at `owa_import_price_multiplier = 1.75`. The `500` floor still applies to low-wage or zero-worker buildings.

### 2. The "Starving Pioneer" Trap — Mostly Resolved

Immigrant households arrive with `runtime_tuning.households.immigrant_starting_budget_per_member = 15.0` (30 for a standard 2-person household).
- **Utility Drain**: `runtime_tuning.households.utility_cost_per_member_per_day = 3.0`, so a 2-person household pays 6/day. Budget runway on utilities alone is about 5 days.
- **Starting supplies**: `runtime_tuning.households.immigrant_starting_stock_days = 3.0` days of household supplies pre-loaded on spawn.
- **Transfer floor**: `unemployment_daily_benefit_per_member = 30.0` per unemployed adult,
  `pension_daily_benefit_per_elder = 30.0`, and
  `child_support_daily_benefit_per_child = 10.0` provide baseline support from exact household
  composition rather than from total resident count.
- **Gap**: Starting supplies run out around day 4. If no valid store has sellable inventory, households may still be unable to restock even with adequate benefit income.

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

### 6. ~~ECON-01: Commercial/Industrial Budget Deadlock — No Recovery Path~~ — Fixed

**Observed**: In a 594-day run, the grocery (idx=22) entered a permanent freeze on Day 64 at `budget=-2.0`, `utility_service_available=false`. Eight farms entered the same state with `budget=0.0`. All remained frozen for 530+ days with inventory sitting unused.

**Mechanism**:
1. In the older implementation, an hourly utility charge derived from Rust constants fired in `resolve_building_utilities`.
2. If `operating_budget < hourly_cost` → `utility_service_available = false`.
3. In the older implementation, `utility_service_available = false` set `utility_factor = 0.0`
   in `run_building_economy`, making `throughput_factor = 0.0` — no production, no sales, no revenue.
4. No revenue → budget never recovers → permanent freeze with no exit.

**Result**: A single budget dip below the utility threshold permanently locks the building out of the economy. 12 buildings deadlocked in the 594-day run. The grocery had 108 units of packaged_food stuck in inventory the entire time.

**Resolution**: The live `v0.1` model removed the `utility_factor` throughput gate. Utility costs
settle through the daily utility/bankruptcy sequence, and the explicit deserted-building lifecycle
removes permanently inactive buildings from all economy flows.

### 7. ECON-02: Absorption Gate Uses Nominal Capacity, Ignores Operational State

**Observed**: Only 1 commercial building (the initial grocery) was ever spawned across 594 days, despite 31 commercial candidates remaining available throughout. `spawns_today=1` was calculated for 402 of those days but no placement occurred.

**Mechanism**: `nonresidential_passes_absorption_gate` in `demand.rs` computes:
```
placed_capacity = sum of nominal output (units/day) for all non-broken, non-economy_broken buildings
consumer_demand = consumption_rate_per_resident × housed_resident_count
```
The grocery profile outputs 200 `household_supplies`/day. The `basic_household_demand` profile has `consumption_rate_per_resident = 1.0`. At 131 residents: `consumer_demand = 131`. Gate condition `placed_capacity < consumer_demand` -> `200 < 131` -> **false** -> second grocery permanently blocked.

The gate does not check `utility_service_available`. A frozen, non-functional grocery still counts at full 200/day nominal capacity. The self-correction mechanism the economy needs (spawn a second grocery when the first fails) is blocked by the very building that failed.

**Resolution**: The live demand absorption path uses current output absorption context and excludes
inactive building states instead of applying the legacy `utility_factor` throughput gate.

### 8. ECON-03: One-Time Bankruptcy Reset Fires Repeatedly

**Observed**: Idle farms (workers=0, revenue=0) cycle through budget 3150→0→3150 on ~250-day cycles indefinitely. The startup float refill is intended as a bootstrap rescue but becomes a permanent subsidy for buildings that are locationally unviable (too far from residential for agents to commute).

**Mechanism**: `ensure_building_startup_float` fires every daily tick for any Commercial/Industrial building where `operating_budget < STARTUP_FLOAT_REFILL_THRESHOLD && revenue == 0.0 && worker_count == 0`. There was no guard preventing repeat fires.

**Fix**: Removed the startup refill path. Startup operating budget is now assigned once when the
building is created, and the two-day `budget_distress` bankruptcy rule handles buildings that spend
that float without becoming viable. A building that cannot attract workers or earn revenue now
enters `is_deserted`, which is the correct signal for the demand system to consider removal.

### 9. ECON-04: Commercial/Industrial Spawn Volume Scales with Road/Zone Area

**Observed**: Adding roads between two daily ticks caused commercial candidates to jump from 13 → 79 and industrial from 34 → 90, spawning 4 grocery stores and 5 farms in a single day — far exceeding the 0–1 that is normal when the road network is stable.

**Fix**: Demand spawn planning no longer sums candidate pressure into the spawn rate. The spawn path computes a deterministic missing-building need from the frozen city snapshot, multiplies that need by the average normalized spawn pressure for eligible candidates, and uses `eligible_spawn_count` only as the final placement cap. Residential need is based on missing household slots against a small vacancy reserve, commercial need is based on unmet household-facing output units/day, and industrial need uses resource-specific business/utility input gaps divided by eligible factories' net output. Existing, pending and selected output capacity reserves that demand. Industrial growth pressure can also rise from actual business/utility `OWA` input dependency. The exact formulas are owned by [`demand.md`](demand.md).

### 10. ECON-05: Pioneer Demand Floor Leaks into Non-Residential Spawn Rate

**Observed**: `spawn_limit` for commercial and industrial is `resident_presence.max(pioneer_demand * 0.5)`. At the pioneer baseline of `pioneer_demand = 0.700`, this floor is 0.35 — meaning even with zero residents the system keeps non-residential spawn pressure non-zero.

**Fix**: The pioneer spawn floor and non-residential `spawn_limit` path have been removed. Commercial growth comes from household purchase stability and missing household-facing shop capacity; industrial growth comes from missing local industrial capacity for active business/utility inputs and actual `OWA` input dependency. Household transfers are the bootstrap income source for households.

## Future Calibration Targets

Remaining open items for the pioneer phase:
- **Dynamic Wage Scaling**: Allow buildings to pay partial wages from available budget instead of stopping at the first worker the budget cannot cover.
- **Liquidation Logic**: Implement an "Economic Death" trigger — despawn a business that stays at $0 budget for a sustained period even when demand pressure is high (Ghost Business problem, issue #4 above).
- **Household bootstrap gap**: The 2-3 day starvation window (days 4-7, between starting-supply depletion and first wages) is the remaining residual from issue #2. Resolved by household transfers — see [Household Transfer Payments](#household-transfer-payments).
- **Pioneer Floor Retirement**: ~~Done.~~ Pioneer demand floor removed from `demand.rs`; household transfers are the replacement and are live.



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
- households consume shared household supply, with bounded one-shopper replenishment trips rather
  than constant per-resident shopping
- fresh-map startup support and later private development remain bounded systems, and zoning alone must not spam empty buildings

That gives Metrum Rise a debuggable economy authoring workflow without violating the project's scale and performance constraints.
