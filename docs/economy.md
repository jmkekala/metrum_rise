# Metrum Rise — Economy Design Spec

## Open Issues For Next Session

The following gaps and contradictions still need resolution before the economy design is implementation-ready:

- **Bootstrap and external market rules are still too vague.** The outside world is the correct solution for starting an empty map, but the document does not yet define import pricing, export pricing, starter capital, border throughput, or why external supply should be intentionally worse than a healthy local chain. Without that, imports may either trivialize the economy or fail to solve the bootstrap deadlock.
- **Operational clock state is not specified deeply enough.** The document defines day length and multiple clocks, but not the runtime state needed to support them cleanly. We still need explicit rules for hour-of-day, minute-of-day, schedule windows, departure windows, and how rush-hour timing is represented in data rather than only described conceptually.
- **Household representation is still unresolved.** Households now matter for budgets, replenishment, migration, and consumption, but the document still leaves room for either true household objects or a more compressed representation inside residential buildings. This needs an explicit decision, because it affects save/load format, demand calculation, budgeting, and performance.
- **Logistics anti-explosion rules are missing.** The document defines shipments and carriers, but it does not yet define batching thresholds, minimum shipment sizes, reservation rules, retry behavior, outstanding-job limits, or failure handling. At Metrum Rise scale, these are not optional details; they are core performance and correctness constraints.
- **Pack churn and missing-profile failure modes need hard rules.** The new model is that assets carry an `economy_profile` reference while profile definitions live in economy data. The document still does not define what happens when an asset disappears, when a profile disappears, or when an asset references a profile that is not currently available. Since packs and economy data can change independently, this must be deterministic and visible to the user.
- **The first-pass pricing model is still unclear.** A `price-response controller` exists in concept, but the document does not yet say whether `v0.1` uses fixed prices and wages or already supports dynamic local pricing. This should be decided explicitly so early implementation does not accidentally build more market complexity than intended.
- **Developer-only tools versus future gameplay controls still need a sharper boundary.** The document now correctly says the economy editor is not gameplay, but some controller and area-override ideas still sound like future player-facing policy levers. We should explicitly separate what is design-time balancing only from what may later become player policy, so the document does not quietly blur those two layers again.

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

Individual agents are not the main production graph nodes. Buildings, terminals, service facilities, and districts are.

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
- that household stock is replenished by occasional shopping or later by an Automated Delivery System (`ADS`)
- residents consume from household stock while at home

An agent's everyday need is therefore not "buy bread now" but "does my household have access to supplies at home."

### 3. Physical logistics matter

Goods do not teleport through the economy. If a transfer is local and meaningful to gameplay, it should be represented by a physical movement job across the `RegionGraph`.

This creates the intended feedback loop:

- delayed deliveries reduce local stock
- low stock reduces household satisfaction or business throughput
- congestion becomes an economic problem, not just a traffic problem

### 4. Balancing and validation are visual; persistence is data-driven

Developers should use a tool, not raw text files, to balance production chains, controllers, and district policies.

Persisted data files still exist for save/load, export, version control, and modding, but they are outputs of the economy tool rather than the primary authoring surface.

### 5. Runtime cost must scale by building, household, district, and shipment count

The economy must scale primarily with:

- number of active buildings
- number of active households or equivalent aggregated household units
- number of active logistics jobs
- number of active districts or policy scopes

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

- `1 in-game day = 20 real minutes`
- `1 in-game hour = 50 real seconds`
- `1 in-game minute = ~0.83 real seconds`

This is the design target for economy balancing. The current prototype clock may use a shorter placeholder value, but economy rules should not be authored against an ultra-compressed day.

### Why this scale is the target

This pacing is intended to keep:

- local errands in the range of minutes, not seconds
- normal commutes in the range of tens of in-game minutes to a few in-game hours
- long cross-city trips inside the same in-game day under normal conditions

If routine travel starts taking multiple in-game days, the time scale, travel speeds, or network assumptions are wrong.

### Economy cadence

The simulation does not need to update every economic rule every render frame.

Recommended cadence:

- movement and deliveries: continuous, on the normal simulation tick
- labor availability, production, and household consumption: evaluated on coarse sub-daily steps such as once per in-game hour
- household replenishment checks: every few in-game hours or when stock falls below a threshold
- wages, building operating costs, and daily summaries: settled once per in-game day

Authoring units should follow this scale:

- production and consumption: `units/day`
- stock: `days of supply`
- wages and operating costs: `currency/day` or `currency/workday`
- prices: `currency/unit`

### Rush hour uses the operational clock

Rush hour belongs to the operational clock.

It should emerge from synchronized or semi-synchronized departures for:

- schools
- offices
- daytime retail
- any other workplace profile that clusters arrivals and departures into morning and evening windows

Rush hour should not be treated as a universal rule for all labor. Some sectors will contribute strongly to the peak, while others operate across the whole day with flatter traffic demand.

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
- basic consumption should not require one separate wallet transaction per resident

### Buildings

Buildings own the money used for production and operations.

- sellers receive revenue when households or other buildings buy goods
- workplaces pay wages and operating costs
- producers buy or reserve required inputs through the building-level economy

This gives the simulation a readable money loop without requiring every essential purchase to be modeled as an individual per-agent checkout event.

## Bootstrap, Migration, and Demand

An empty map cannot start with a fully self-contained local economy. The economy needs a bootstrap phase.

### Bootstrap economy

At the beginning of a new city:

- the outside world acts as the initial source and sink for people, goods, and money
- immigration is stronger than emigration by default, as long as the city can accept new households
- early households arrive with starter savings and immediately create household demand
- early shops and workplaces may operate in import-backed mode until local supply chains exist
- surplus may later be exported, but exports are not required to bootstrap the city

This prevents the economy from deadlocking on day one when no households, producers, or internal supply chains exist yet.

Bootstrap immigration should taper gradually as the city develops. It should not use a fixed hard cap such as "stop after N agents." The slowdown should be driven by household count and city conditions instead.

### Immigration and emigration

Immigration and emigration belong at the boundary between demography and economy.

They affect:

- available labor
- number of consuming households
- housing demand
- wage pressure
- business viability
- service load

Early game rules should favor immigration so the city can start growing. Later, migration should react to economic conditions such as:

- available housing
- job availability
- household cost pressure
- household stock stability
- commute burden
- service quality

The intended behavior is a soft transition, not a magic population wall. Immigration should slow as the city becomes more established or less attractive, while emigration rises when city conditions deteriorate.

### Demand system and decisions system

Yes, the long-term design should treat demand and decisions as separate simulation systems.

#### Demand system

The demand system should track aggregated pressures such as:

- household demand for essentials
- labor demand by workplace type
- unmet goods or service demand
- city attractiveness for immigration and risk factors for emigration

This layer should operate mostly on coarse aggregate data rather than per-agent decision logic.

#### Decisions system

The decisions system should resolve choices made by agents, households, and buildings, such as:

- whether an agent goes to work
- when a household replenishes stock
- whether replenishment uses shop pickup or `ADS`
- which supplier or route is selected
- which schedule window a workplace is currently filling

This layer operates on the operational clock and consumes the pressures produced by the demand system.

Short version:

- demand answers "what pressures exist in the city?"
- decisions answer "what does this household, worker, or building do about them?"

## Product Shape

The economy editor should be a separate developer tool, built in the same Godot + Rust tool family as the game and asset editor.

Recommended shape:

- `metrum_rise_game`: play and inspect a live city
- `metrum_rise_asset_editor`: author assets and their economic interfaces
- `metrum_rise_economy_editor`: internal balancing, validation, and debugging tool for production graphs, controllers, recipes, and district policies

This may exist as a separate executable or as a developer-only launch mode inside one shared application family. The important part is the responsibility split, not the packaging name.

The economy editor is not part of gameplay. Players should not be wiring production graphs or changing balancing variables from the live game UI.

If area-based economy overrides are used, they are defined by developers in test scenarios, balancing maps, or authored content data. Players do not paint economy-policy areas when starting a new map.

### Why it should be a separate tool

- The live game is too noisy for serious authoring. Traffic, weather, zoning churn, and population motion make systematic economy editing harder.
- The asset editor already has a narrow job: import, validate, preview, and package content assets. Economy graph authoring is a cross-asset systems task, not per-asset metadata editing.
- A dedicated developer tool can provide graph editing, scenario playback, bottleneck visualization, and district overlays without inheriting the full gameplay shell.

### What still belongs in the game

The runtime game should still expose economy inspection tools:

- stock and shortage overlays
- route and shipment debugging
- district policy summary
- building-level throughput and staffing inspectors

But the live game should remain read-only for economy tuning. It can expose inspection and diagnostics, not balancing controls or graph authoring.

## Responsibility Split

### Asset Editor

The asset editor defines the asset's stable identity and base building metadata. It should stay focused on importing and packaging assets, not on authoring economy recipes.

Examples:

- a residential building asset declares `residential_capacity`
- a workplace asset declares `worker_capacity`
- an asset may store one `economy_profile` reference that points at an existing live economy profile
- lot size, service class, and similar building facts remain asset-authored metadata
- those values may be derived from floor area or other building-shape logic inside the asset toolchain

The asset editor does not define city-wide wiring, area policies, recipes, inputs, outputs, or economy balancing rules. It only stores the profile reference, not the profile definition itself.

The asset editor should list or suggest currently available economy profiles from the live economy data. Asset importers should not be expected to invent new profile names ad hoc.

The shipped game/editor should include a baseline economy profile catalog for asset creators. When new profiles are added, creators may need the latest exported profile list or a newer game/editor build to stay in sync. If the local profile catalog is missing or outdated, the asset editor should warn clearly and degrade gracefully rather than blocking general asset import work.

### Economy Editor

The economy editor is a developer-facing balancing and validation environment. It defines economy profiles and relationships between economic actors, then helps catch systemic design mistakes before those rules ship into the runtime.

Examples:

- which reusable economy profiles exist
- which producer classes can supply which consumer classes
- which controllers affect pricing, taxes, subsidies, or household delivery rules
- which area-specific overrides apply in one place but not another
- which goods are required for household stability versus optional quality-of-life supply

It is also the main developer surface for validating and debugging shortages, dead chains, impossible recipes, and other balance failures before those rules ship into gameplay.

### Runtime Simulation

The runtime consumes exported economy definitions and simulates:

- building inventories
- household stock buffers and replenishment state
- staffing and labor demand
- shipment creation and delivery
- district modifiers
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
- `construction_materials`

Rules:

- keep the v0.1 set small and legible
- prefer broad gameplay-relevant categories over excessive micro-goods
- split a resource only when the distinction creates meaningful logistics or policy gameplay

### 2. Economy Profiles

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
  - inputs: `flour`, `power`, `labor`
  - outputs: `staple_food`
  - variables: `base_cycle_time`, `input_buffer_cap`, `output_buffer_cap`, `schedule_profile`

Base capacities such as `worker_capacity` or `residential_capacity` remain asset-authored metadata and are consumed by the profile rather than redefined inside it.

### 3. Economy Profile References

An economy profile reference lives on the asset side and points to one named economy profile.

Rules:

- the asset stores only the profile name or ID, not the full economy definition
- the asset editor should offer a live list or suggestions of existing economy profiles
- asset importers should select from existing profiles rather than inventing new profile names
- the shipped game/editor should provide a baseline profile catalog so asset creators have a stable starting set
- when that local catalog is outdated, the editor should warn and allow refresh to a newer profile list or game/editor version
- multiple assets from different asset sets may reference the same profile
- tags may help editor search and filtering, but they should not be the primary economy contract

### 4. Economic Node Instances

An economic node instance is one placed building or facility in the world.

It holds runtime state such as:

- current inventory by resource
- assigned workers / filled jobs
- utilization
- local modifiers
- shipment reservations
- current shortage flags

### 5. Controllers

Controllers are authored policy objects that modify behavior across many nodes.

Examples:

- wage policy controller
- local tax controller
- price-response controller
- subsidy controller
- household delivery cost controller

Controllers are not arbitrary scripts. They are bounded, inspectable systems with defined inputs, outputs, scope, and update cadence.

Each controller definition should specify:

- what it reads
- what it writes
- whether it is global, district-scoped, profile-scoped, or asset-category-scoped
- whether it affects authored preferences or runtime state

### 6. Connections

A connection is an authored allowed relationship between node classes, resource types, or controller scopes.

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
- multi-unit residential buildings host multiple households or an equivalent aggregated per-unit representation, but never one stock buffer per resident

Residents draw from their household buffer while at home.

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
- later, `ADS` may satisfy the same replenishment request through home delivery
- `household` consumes `household_supplies`

If that chain works, the broader economy architecture is sound enough to extend.

## Labor Model

Labor should remain the main direct agent-to-building economic link.

### Buildings demand labor

Workplaces expose:

- open job slots
- wage offer or wage band
- skill preference later if needed

### Work schedule profiles

Workplaces should not all share one global workday. Each workplace asset type should declare a `schedule_profile` on the operational clock.

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

Agents decide whether to travel to work based on utility scoring rather than a pure RNG cycle.

Early utility inputs can stay simple:

- current money
- household stock situation at home
- commute cost
- job availability

### Building throughput depends on staffing

Production should derive from a bounded formula based on:

- filled worker count
- input availability
- power or utility availability if applicable
- controller modifiers

This gives the player a meaningful connection between zoning, staffing, transit, and output without requiring arbitrary micromanagement.

## Logistics Model

### Shipment units

The simulation should create shipments at the building or terminal level, not one tiny packet per household resident.

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
- terminals split bulk flows into last-mile deliveries when necessary

### Route creation

The authored economy graph chooses who is allowed or preferred to supply whom.

The runtime then resolves:

- which supplier has stock
- which consumer has demand
- whether a shipment is worth spawning
- which network path and carrier type to use

This keeps the simulation physical without forcing the editor graph to become a per-vehicle routing interface.

### Household replenishment

Household replenishment should support two fulfillment modes:

- periodic shopping or pickup, represented as an occasional household-level replenishment action rather than one trip per resident
- `ADS`, which fulfills the same household demand through delivery jobs once the basics system is already stable

`ADS` should be treated as a convenience layer:

- more expensive than normal shopping
- range-dependent carrier selection: nearby deliveries use pedestrians or bikes, while longer-distance deliveries use cars
- distance-based pricing: the farther the delivery origin is from the household, the more expensive the order becomes
- sensitive to congestion and local courier capacity
- more viable in dense, high-service districts than in sparse rural areas

## Economy Editor UI

The economy system must be balanced and validated visually. Adjusting key numbers in text files is not acceptable as the primary workflow.

### Main views

The developer tool should have at least three coordinated views.

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
- the developer places a `household` node with input `household_supplies`
- the developer places an `ADS cost controller` node that affects household replenishment
- the graph then connects `food_processor -> grocery -> household`, with the controller linked to the household node

At this stage the developer is defining the structure of the economy chain, not yet testing whether the numbers are balanced.

#### 2. Area View

A simple map view where developers define named economy areas on a test scenario or authored city layout.

This is a developer-side balancing tool, not a player-facing map setup step.

Use it to author:

- tax modifiers
- subsidies
- service focus
- `ADS` availability or delivery-cost modifiers
- local bans or restrictions

This keeps local balancing tied to geography without turning the whole economy into one unreadable giant cable graph.

#### 3. Runtime Inspection View

A debug view for scenario playback and diagnosis of the authored balance rules.

Use it to inspect:

- stock levels
- blocked supply chains
- delivery latency
- unfilled labor demand
- controller effects
- shortage propagation

Example:

- the developer runs the `Shop vs ADS` test case for 30 simulated days
- the view shows that household stock drops below 1.0 days after day 12
- the diagnostics panel reports that the grocery has enough goods, but bike couriers are saturated and car deliveries are too expensive at the current distance-cost multiplier
- the controller panel highlights that `ADS` cost is pushing too many households back to shop pickup, which then causes grocery-side queueing
- the developer can immediately see that the problem is not food production, but last-mile replenishment balance

### UI layout recommendation

Recommended shell:

- center: graph canvas or district map, depending on mode
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

Example: `Shop vs ADS` test case

- Left panel: select the `Shop vs ADS` preset from a list of developer test cases.
- Center graph: show `food_processor -> grocery -> household`, with an optional `ADS` controller connected to household replenishment.
- Right inspector: expose values such as household count, household size, shop distance, `ADS` enabled, bike range, car fallback range, base delivery fee, and distance-cost multiplier.
- Bottom diagnostics: show stock days, average household cost, replenishment mode split, shortage warnings, and whether any recipe or connection is invalid.

In this example the developer does not need to use `Area View` at all. The graph, inspector, and diagnostics are enough to test whether local shopping or `ADS` gives the intended balance result.

### Validation requirements

The tool must validate common design mistakes before export:

- disconnected required inputs
- impossible recipes
- circular dependencies with no bootstrap supply
- district policies that ban all legal suppliers
- throughput definitions that can never fill household demand

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

The exact filenames are still open, but the intended structure is:

```text
economy/
  profiles.toml        # economy profiles and recipe definitions
  controllers.toml     # controller definitions and parameters
  areas.toml           # optional area-based overrides
  economy.index.bin    # optional derived cache
```

The important rule is not the exact folder shape. The important rule is:

- text files are authoritative
- caches are derived
- exported economy data remains inspectable and editable outside the tool

Examples of compiled forms:

- resource IDs
- asset-type recipe tables
- controller parameter blocks
- district override tables
- supplier-consumer compatibility lists

This gives the tool freedom to be expressive while keeping the simulation runtime predictable.

## Scope Recommendations

### v0.1 must stay narrow

The first implementation should solve one closed loop well instead of sketching ten unfinished ones.

Recommended v0.1 scope:

- one essential household resource chain
- per-building production buffers and per-household stock buffers
- household stock consumption
- workplace labor demand
- truck-based local delivery
- utility-driven work/home decision logic
- one dedicated economy editor shell with graph view, inspector, and validation

### v0.1 non-goals

Do not make these blockers for the first pass:

- personal retail trips as a daily need
- deep commodity markets with dozens of goods
- arbitrary user scripting inside controllers
- remote or hybrid work simulation
- full multimodal freight from day one
- world-scale intercity import simulation

### v0.2 and later

After the first household supply loop is stable, add:

- terminals and bulk transfer
- district-level policy differentiation
- more resource classes
- service economy layers
- Automated Delivery System (`ADS`) home-delivery fulfillment
- intercity import/export abstractions
- additional transport modes for freight

## Example Chain

A good starter chain for both simulation and developer-tool tuning is:

- `food_processor`
  - inputs: `labor`, `power`
  - outputs: `staple_food`
- `grocery` or `distribution_center`
  - inputs: `staple_food`, `labor`
  - outputs: `household_supplies`
- `household`
  - inputs: `household_supplies`
  - runtime variables: `household_size`, `stock_days`, `consumption_rate`, `replenishment_mode`

Controllers that can modify this chain:

- wage pressure
- local subsidy
- household delivery cost
- price response

Replenishment for this chain can happen through periodic shopping first, with `ADS` added later as an alternative fulfillment path.

This example is intentionally broad. It avoids modeling "one loaf of bread per person per day" while still creating meaningful logistics, staffing, and shortage gameplay.

## Summary

The economy should be balanced and validated through a visual, building-centric developer tool, not through hardcoded numbers and not through gameplay UI controls.

The recommended design is:

- assets define identity, base metadata, and an `economy_profile` reference
- the economy editor lets developers tune graphs, controllers, and district policies
- runtime simulation executes compiled building-level inventories, labor, and shipment rules
- households consume shared household supply so agents do not need constant shopping trips

That gives Metrum Rise a debuggable economy authoring workflow without violating the project's scale and performance constraints.
