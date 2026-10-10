# Metrum Rise — Economy and Healthy Growth

> **Archived 2026-10-10.** Historical record of designs, diagnoses and measurements; statements here may be superseded. The current specification is [`economy.md`](../economy.md).

## Purpose

This document owns household and business finances, production, services, trade, and the economic
conditions for healthy city growth. Buildings produce and employ; households earn and spend; the
city collects revenue and funds infrastructure, services, and transfers. Growth must be supported
by those flows after starter savings and business startup capital cease to cover operating losses.

Economy data is authored in `economy/profiles.toml` and checked with Rust balance tests and
headless scenarios. There is no developer economy editor. Demand decisions belong to
[`demand.md`](demand_history_2026-10-10.md), placement legality to [`zoning.md`](../zoning.md), and physical trip
execution to [`entrance_and_exit.md`](../entrance_and_exit.md).

## Reading Guide and Status

### Electricity consumption and coal balance

Electricity accounting uses kWh, with MWh for large displayed totals and kW/MW
for generating capacity. The following are initial game-balance rates, not measured real-world
building specifications. Hourly usage is daily usage divided by 24. Operating load uses existing
activity (0–1); farms/mines multiply it by hectares. Area maintenance applies even while idle.

| Profile | Idle kWh/building/day | Operating kWh/day at full activity | Additional demand |
| --- | ---: | ---: | --- |
| Residential household | 1, shared once per home | 0 | 5 kWh/resident/day |
| Grocery | 8 | 12 | — |
| Food processor | 4 | 28 | — |
| Barber / pharmacy | 2 | 4 | — |
| Machinery factory | 4 | 26 | — |
| Water / wastewater plant | 4 | 8 / 10 | — |
| Grain farm | 2 | 3 per ha | 0.555556 kWh/ha/day maintenance |
| Coal mine | 2 | 540 per ha | 60 kWh/ha/day maintenance |
| Coal power plant | 10 | 400 at all 20 staffed slots | 50 kW capacity per worker |
| Other occupied workplaces / public services | 2 | 8 | Authored fallback profile |

Initial tariffs are 0.20 currency/kWh local and 0.25 imported. The resident component remains
1 currency/day before the shared building load. Water and sewage retain their existing model.
The coal recipe is 0.45 kg/kWh actually generated. Machinery upkeep is 0.2 kg per staffed
worker/day even when sales are low. Auxiliary demand follows staffed activity before dispatch;
own consumption earns no external revenue. OWA supplies any unmet local demand.

Coal is retuned to 0.10 currency/kg and 9,600 kg/ha/day (five workers/ha), preserving the mine's
960 currency/ha/day gross local sales before costs. New full-richness deposits hold 480 kg/m²,
retaining 500 full-production days. This is a balance change, not a change to the kg unit:
existing coal stocks, cargo and remaining deposits must not be multiplied. More mass can change
freight costs; the physical mine scenarios must validate this independently.
Each committed mine retains its reserve density when its boundary is edited. Older saves retain
6 kg/m²; new sites use 480 kg/m². Editing a legacy mine must not replenish it at the new density.

Power input buffers follow staffed capacity: one worker targets 1,620 kg coal, with an 810 kg
reorder threshold. An unstaffed plant requests no new inputs. Existing minimum freight batches
still apply to coal and machinery. UI electricity net is cash flow after purchases, wages and
utility upkeep; bulk purchases can make a reporting window differ from steady operating margin.

Reference worksheet: each 170-resident block has 85 occupied homes and 65 kWh/day of workplace
usage, totalling 1,000 kWh/day of paying external demand. Coal/machinery/water/sewage are imported;
coal is 0.125/kg, machinery 25/kg, wages 115/worker/day. One plant pays 4.375/day for water/sewage.
Freight uses a 0.5-hour one-way journey, the authored reorder lots and 20 t payload. This worksheet
is a steady operating margin, excluding construction and startup capital, not a physical-run result.

| Workers / reference residents | Capacity kW | Generation kWh/day (incl. own use) | External revenue | Coal cost | Wages | Machinery | Water/sewage | Freight | Daily surplus |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 / 170 | 50 | 1,030 | 200 | 57.94 | 115 | 5 | 4.38 | 4.68 | 13.01 |
| 2 / 340 | 100 | 2,050 | 400 | 115.31 | 230 | 10 | 4.38 | 4.69 | 35.62 |
| 3 / 510 | 150 | 3,070 | 600 | 172.69 | 345 | 15 | 4.38 | 4.73 | 58.21 |

Delivery determines bills independently of cash. Record amounts owed and paid separately; unpaid
electricity feeds financial distress and cannot create collected utility revenue. Public buildings
are paid by the city, businesses by their operating accounts, residences by their households.
Inactive/deserted/unfinished buildings consume nothing initially; an empty home has no household
load until occupied. Mixed-use residential and business components must each be charged once.
Unprofiled occupied buildings use the authored fallback.

Removal settles the workplace's outstanding daily bill and retains its energy, generation and
city input purchases/refunds for the day's report. Removal after midnight billing cannot charge
again. Same-day undo restores the meter and treasury change; undo cannot reopen a closed fiscal
day. These checks and mid-day save continuation pass. The results below validate the stated
workloads, not every saved-world state.

Fresh tests, physical runs and matched 10k timings are recorded in
[Electricity validation](#electricity-validation-econ-14).

The healthy-growth plan below was agreed on 2026-10-05. `ECON-14` has implemented causal
accounting, failed-business recovery, grocery viability, recurring household affordability,
village services, area-based producer costs and canonical resource units. The earlier capped
market build sustained 58 residents and 45 employed through two years; its dated results below
remain evidence for that build.

The current [production and freight revision](#production-and-freight-economics-econ-14)
removes fixed OWA buying quotas, prices physical deliveries, and stages export-led factory
openings behind existing industry's operation and earned cash. Grain farms now use a two-worker
minimum, one worker per 10 ha above it, and a six-worker whole-site ceiling. Annualized output is
20 kg/ha/day (7.3 t/ha/year); construction and upkeep still scale with the full area. Grain field
placement and edits require at least 4 ha. Imports remain 1.25× and export bids 1.0×.
Current validation is recorded with that revision; earlier passing runs do not validate it.

[Resource units and freight payload](#resource-units-and-freight-payload) define physical goods
in kg, electricity in kWh (MWh display), water/sewage in m³ and services as service equivalents. Trucks carry
at most 20,000 kg, independently of their minimum order batches. These are explicit game-balance
rates, not calibrated real-world yields. Additional funded activities, varied settlement
fixtures and broader demand simplification remain `ECON-14` / `DEM-02` work.

| Read for | Owning section |
| --- | --- |
| Current diagnosis, economic design, and delivery order | [Growth redesign](#growth-redesign-econ-11econ-14-dem-02) |
| Reproducible starter budgets and candidate tuning | [Starter-town balance sheet](#starter-town-balance-sheet-econ-14) |
| Observed business failures, fiscal losses and recovery blockers | [Village failure timeline](#village-failure-timeline-econ-14) |
| Failed-site repair, isolated rerun and rejected import tuning | [Failed-business recovery](#failed-business-recovery-econ-14) |
| Grocery viability, household supply and service-opening threshold | [Grocery startup repair](#grocery-startup-repair-econ-14) |
| Recurring industrial income, household affordability and municipal funding | [Regional industrial markets](#regional-industrial-markets-econ-14) |
| Existing barber/pharmacy viability, farm and mine evidence | [Village services and producer audit](#village-services-and-producer-audit-econ-14) |
| Uncapped exports, carrier pricing, producer investment and staged factory growth | [Production and freight economics](#production-and-freight-economics-econ-14) |
| Current quantities, building output rates, and truck capacity | [Resource units and freight payload](#resource-units-and-freight-payload) |
| Timing and authoritative economy/demand handoff | [Economy time scale](#economy-time-scale) |
| Household, business, city, and external money flows | [Money model](#money-model), [startup and trade](#startup-economy-and-outside-world-exchange), [transfers](#household-transfer-payments) |
| Authored content and runtime execution | [Responsibility split](#responsibility-split), [data model](#economic-data-model), [household supply](#household-supply-model), [labor](#labor-model), [logistics](#logistics-model) |
| Business failure and redevelopment boundary | [Building bankruptcy](#building-bankruptcy) |
| Tests, scenario trajectories, and performance evidence | [Validation evidence](#validation-evidence) |

## Growth Redesign (`ECON-11`–`ECON-14`, `DEM-02`)

### Decision and scope

The immediate objective is a city that can establish viable businesses, support its households,
and develop a broader employment base. `ECON-14` covers this economic redesign; `DEM-02` makes
growth decisions express the resulting needs directly. Profitable exports are one part of the
design, alongside viable imported-input startup and affordable local commerce and services.

**Use 10k residents for the current performance acceptance work. The 1M run is deferred and is
not a prerequisite for design or implementation.** Larger recorded runs remain historical
evidence. The long-term scale target still constrains complexity, allocations, and determinism;
the immediate work must not be diverted into completing larger benchmark tiers.

Accounting remains aggregate at household, building, service, and shipment granularity. Additional
consumption categories must not create per-resident daily purchasing or route searches. Current
freight and household replenishment still settle through their existing delivery lifecycles;
representative service visits are visual. This plan does not silently change those delivery or
congestion contracts into instantaneous settlement.

### Diagnosis

The numerical diagnosis below records the 2026-10-05 baseline at 1.75× imports. The
[grocery repair](#grocery-startup-repair-econ-14) changes imports to 1.25× together with demand,
production and lifecycle fixes. The subsequent [regional-market repair](#regional-industrial-markets-econ-14)
establishes recurring industrial income and municipal funding in the starter fixture.

**The employment base is too narrow.** `ECON-11` measures 130.55 jobs per 1,000 residents for the
shipped private consumption chain, assuming all locally producible inputs are made locally and
there are no exports. This is the historical content baseline, not a target employment ratio.
The village service tuning below raises the current result to **139.55** (5 personal-service and
8 pharmacy jobs per 1,000, before rounding); the employment base remains narrow:

| Activity | Jobs per 1,000 residents |
| --- | ---: |
| Grocery retail | 75.00 |
| Food processing | 50.00 |
| Farming and machinery supporting that chain | 1.55 |
| Personal services | 1.50 |
| Health essentials | 2.50 |
| Total | 130.55 |

These are fractional capacity equivalents at city scale, before small-building and field staffing
rounding. Public services, export employment, and their additional inputs are excluded. Almost all
modeled private employment serves food consumption. More meshes using the same recipe divide the
same customers; additional economic activities need their own funded customers and cost model.

**Imported-input startup is structurally unprofitable for key businesses.** At full staffing,
full output sales, and runtime average wages, the shipped recipes give:

| Business | Sales/day | Payroll/day | Local input cost/day | Imported input cost/day | Margin with local inputs | Margin with imported inputs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Grocery | 5,000 | 1,350 | 2,400 | 4,200 | +1,250 | −550 |
| Food processor | 2,400 | 900 | 1,000 | 1,750 | +500 | −250 |

These are paper margins before utilities, taxes, freight timing modifiers, and working-capital
delays. Imports cost `1.75×` catalog price. The existing full-staffing solvency test assumes a
local input price whenever a producing profile exists, even if no such supplier exists in the
city; passing that test does not establish startup viability. Partial operation also faces whole
worker slots and fixed building costs. A larger one-time float delays a recurring loss.

**Exports do not yet provide a dependable employment base.** The normal export multiplier is
`0.60`, with saturation reducing receipts by up to another `0.75` factor. The current export
target test fails for the food processor, coal mine, and machinery factory even before overhead.
Merely raising the base multiplier to `1.0` does not establish profitability after saturation,
actual input costs, and overhead. Industrial spawn need and absorption currently count local
household/business/utility customers; they do not reserve capacity against regional customers.
Improved export prices alone therefore do not create an export-led construction signal.

**Demand discounts related symptoms repeatedly.** Job pull, marginal commercial jobs, regional
pull, household stability, candidate savings runway, transfer coverage, failure penalties,
thresholds, and construction credits interact. Financial failure weakens both purchasing power
and the growth response. Necessary constraints must remain explicit without repeatedly applying
the same distress as several independent pressure penalties.

**The 600-household term is not a hard immigration cap.** Only regional pull decays with
`1 - households / 600`; job-driven pull remains possible above 600. At 42 households that size
factor is still 0.93, so it cannot by itself explain the starter town's early failure. Remove the
arbitrary size cutoff in the redesign, but do not mistake that for repairing business finances.

**Business failure precedes fiscal exhaustion; site recovery alone does not fix it.** The original
[365-day village report](#village-failure-timeline-econ-14) traces processor bankruptcy to day 14,
last grocery closure to day 188 and treasury exhaustion to day 192. At year end, households still
hold cash but every supply buffer is empty. All 32 standing commercial buildings are deserted;
positive commercial demand prevented them from becoming clearance candidates. The
[2026-10-06 repair](#failed-business-recovery-econ-14) clears those sites, but its isolated run
still depends on repeated startup capital and fails healthy-growth acceptance. Startup viability
remains open. The original sparse and older 25-household trajectories remain dated evidence below.

### Economic model to define before tuning

The first deliverable is a reproducible ledger connecting household spending to sales, inputs,
wages, supported jobs, and the city budget. Derive employment needs from actual household age
composition and work eligibility; do not pick an employment ratio independently of spending,
productivity, and wages.

| Ledger | Required quantities and checks |
| --- | --- |
| Household | Wages after income-tax withholding and funded transfers; housing, essentials, utilities and remaining taxes; desired reserve; discretionary spending from the remainder. Include worker-only, child-heavy, and elder households. |
| Private activity | Buyer and resource/category; affordable demand; realized sales; local/imported inputs; staffed output; payroll, utilities and taxes; retained profit; cash tied up in stock and shipments. |
| Public service | Explicitly funded jobs and inputs, service fees and tax support, operating cost at the population served. Public hiring must have recurring funding. |
| External exchange | Imports and exports by resource and value, effective prices after modifiers, delivery capacity, and net cash entering or leaving the city. |
| Startup and capital | Household starter savings, private owner equity, inventory grants and treasury seed funds recorded separately from recurring income. Repeated replacement businesses must not hide losses behind fresh float. |

Use accounting identities rather than treating production capacity as revenue:

```text
household recurring surplus = net wages + funded transfers - recurring spending
business operating margin = realized sales - consumed-input cost - payroll - utilities - taxes
city recurring balance = taxes + service receipts - transfers - service costs - infrastructure upkeep
```

Cash-flow checks also include actual stock purchases, payment timing, construction expenditure,
and explicit capital injections. Do not charge inputs twice by mixing consumed-input cost and
inventory purchases in one profit calculation. Trace where profits, housing payments, taxes and
external spend go; a closed goods chain alone does not establish sustainable money circulation.
Count withheld income tax and VAT remitted to the city once each, keeping household charges,
seller receipts, and treasury revenue consistent.

For each supported business, define a minimum viable market and break-even utilization, then test
startup, ordinary partial staffing, and steady operation. The existing barber and pharmacy should each support one worker around 60 residents; other
business types still need an explicit viable market. Available workers, customers, affordable inputs, and physical delivery
must agree before nominal job capacity is treated as durable employment.

### Starter-town balance sheet (`ECON-14`)

Initial worksheet completed 2026-10-05 against the shipped catalog at `21100889`, with test-only
ledger additions. It reads compiled profiles, uses runtime fiscal arithmetic and household
composition, and extends the existing `ECON-11` balance helpers. Reproduce from `rust/`:

```bash
cargo test --lib economy::definitions::balance_tests
cargo test --lib economy::definitions::balance_tests::starter_ledger::starter_town_balance_sheet -- --ignored --nocapture
```

Source: `rust/src/simulation/economy/definitions/balance_tests/starter_ledger.rs`. This is
static accounting, not a growth simulation or proof of the smallest viable town. It assumes
level-one buildings, paid demand, steady inventories, sufficient workers/inputs, and imported
utilities. Average authored wages match current payroll. Business margins include consumed
inputs, payroll, property tax and utilities; they exclude cash-based profit tax, startup
purchases and delivery interruptions.

Tables in this initial worksheet retain the 1.75× baseline and its candidate comparisons.
The current catalog uses 1.25× and higher service consumption; these historical worksheet
numbers are not current household budgets. See the later scenario evidence for acceptance and limits.

**Household affordability.** A grocery/industrial worker earns 90/day gross and 79.20 after
income tax. With all baseline consumption categories served, daily spending per resident was
25 supplies + 2 grocery VAT + 0.60 personal services + 0.70 health + 5.25 imported utilities
= **33.55**, plus **2 per household** property tax. Service settlement currently charges no VAT;
there is no separate rent/mortgage payment. Removing an unavailable service reduces paid spending
but does not establish that a fully served household is affordable.

| Household | Workers | Current daily surplus (`1.75×` imports) | Hypothetical surplus (`1.25×` imports) |
| --- | ---: | ---: | ---: |
| One adult | 1 | +43.65 | +45.15 |
| Two adults | 1 | +10.10 | +13.10 |
| One adult, one child | 1 | +20.10 | +23.10 |
| One adult, two children | 1 | −3.45 | +1.05 |
| One adult, three children | 1 | −27.00 | −21.00 |
| Two adults, two children | 2 | +42.20 | +48.20 |
| One elder | 0 | −5.55 | −4.05 |
| One unemployed adult, benefit active | 0 | −5.55 | −4.05 |
| One unemployed adult, benefit expired | 0 | −35.55 | −34.05 |

Transfers are 10/child and 30/elder/day; unemployment pays 30/adult/day for at most 30 days.
The import comparison reduces utility bills; it leaves retail prices, wages and transfers
unchanged. **Employment alone cannot repair every household type.** At `1.25×`, a single elder
needs at least 34.05/day to cover this basket; one working adult with three children needs total
child support of 51/day (17/child) just to break even. These are diagnostic bounds, not selected
benefits: larger households and lower service wages require separate cases, and the treasury
must fund any increase. A near-zero surplus also provides no recovery reserve.

**Imported-input businesses.** Full staffing, all net output sold locally, ordinary freight
timing, currency/day; these figures include overhead omitted by the older `ECON-11` check:

| Business | Workers | Sales | Current input cost | Payroll | Current overhead | Current margin | Margin at `1.50×` imports | Margin at `1.25×` imports |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Grocery | 15 | 5,000 | 4,200 | 1,350 | 36.38 | −586.38 | +15.25 | +616.88 |
| Food processor | 10 | 2,400 | 1,750 | 900 | 46.38 | −296.38 | −44.75 | +206.88 |
| Machinery factory | 4 | 800 | 280 | 360 | 46.38 | +113.63 | +155.25 | +196.88 |

Machinery sells 40 net units after its two-unit self-upkeep; a full cycle still consumes all
12 Steel and four Metals. Do not price external inputs at `40/42` of a cycle. Local demand is
not assumed sufficient to buy a full factory's output merely because its margin is positive.

The grocery's outside-window freight multiplier is `1.10`. Full-output margins then become
−1,006.38 / −344.75 / +316.88 at import multipliers 1.75 / 1.50 / 1.25. Consequently **1.50
is not a robust startup repair**. At maximum authored wages of 100/day, the full-output,
outside-window break-even import multiplier is approximately 1.313:
`(5000 - 15*100 - 25) / (2400*1.10 + 6.5)`. The `1.25×` comparison has some headroom, but it
was initially a candidate for scenario testing; the combined grocery repair below now accepts it.

**Small markets and staffing steps.** For one grocery with stocked households, no pantry
recovery demand and one supply unit purchased per resident/day, steady staffing is
`ceil(15*N/200)` for `1 <= N <= 200`. At `1.25×` imports its ordinary daily margin is
`10*N - 90*ceil(15*N/200) - 33.125`; outside-window freight replaces `10*N` with `8.5*N`.

| Residents | Workers | Ordinary margin | Outside-window margin |
| --- | ---: | ---: | ---: |
| 13 | 1 | +6.88 | −12.63 |
| 14 | 2 | −73.13 | −94.13 |
| 26 | 2 | +46.88 | +7.88 |
| 27 | 3 | −33.13 | −73.63 |
| 40 | 3 | +96.88 | +36.88 |
| 100 | 8 | +246.88 | +96.88 |

The first positive point is not a safe minimum market: hiring the next worker can restore a
loss. The formula stays nonnegative through every subsequent whole population up to 200 only
from 31 residents with ordinary freight, or 68 with outside-window freight. These are paper
thresholds, not admission rules. Actual stock recovery, competing shops, payment shortfalls,
input batches and staffing changes need the live scenario. Current `1.75×` imports lose money
throughout this range.

At the baseline service rates, personal and health services first covered one worker plus overhead at **161 and 170
residents**, respectively (156 and 166 at `1.25×`). A tiny village cannot support both merely by
placing their buildings. Adding variants of these profiles does not increase customer spending.

**Employment and the city budget.** The worksheet creates deterministic demographic samples
with 20/50/100 homes of 100 m², `home_id = household_id`, and occupancy slot zero. It uses actual
runtime household-size and age functions. This is a declared synthetic sample, not the observed
growth-scenario population. The following necessary income bounds assume 90 gross per job,
permanent child/pension transfers, no expired-benefit replacement, and all categories paid:

| Households | Residents | Eligible adults | Children / elders | Jobs needed at current prices | Jobs needed at `1.25×` imports | Local-chain job equivalents |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 20 | 39 | 26 | 12 / 1 | 15.13 | 14.39 | 5.09 |
| 50 | 109 | 67 | 32 / 10 | 39.61 | 37.54 | 14.23 |
| 100 | 221 | 137 | 64 / 20 | 80.49 | 76.30 | 28.85 |

Required jobs equal `(household spending - permanent transfers) / 79.20`; round up for actual
workers. Household cash is not pooled, so even that number of jobs does not guarantee individual
solvency. At current prices, two/six/twelve households in these samples still have deficits when
**every eligible adult** earns 90/day. At `1.25×`, one/three/six still do. Local-chain figures
use the fractional 130.55/1,000 baseline; they exclude exports/public employment and assume
local intermediate supply. Small-building rounding adds slots but does not fund their wages.
Jobs in the current services also pay less than 90, making the equal-wage comparison optimistic.

In the 109-resident case, employing all 67 adults at 90/day produces 1,041.60 in household
income/VAT/property taxes and costs 620 in child/pension transfers: **421.60/day remains before
business taxes, roads and public services**. At 38 employed adults it is only **108.40/day**,
assuming all purchases are still paid and remaining adults' unemployment support has expired.
Business property/profit taxes can add revenue; imported utility payments go to the outside
world. A 400 m road costs 40/day upkeep and 6,000 once to build, versus 280/day and 42,000 for
2,800 m. These are footprint comparisons, not measured layouts. Public payroll and service
inputs must be added explicitly; the budget does not justify filling the employment gap with
unfunded municipal workers. The 500,000 starting treasury is capital, not recurring revenue.

**Export condition.** With `1.25×` imported inputs, full-output food processing needs an
effective export bid of at least **0.9138×** catalog price; machinery needs **0.7539×**. At
maximum authored wages these rise to **0.9555×** and **0.8039×**, before profit-tax cash timing.
The pre-regional-repair saturated bid was `0.60 * 0.75 = 0.45×`; raising the base bid to 1.0 while keeping
the saturation floor still gives only `0.75×`. Neither case supports both industries. Viability
requires sufficient committed sales volume as well as effective price; the regional customer
model must supply both. Retained business profit does not automatically become household income.

**Current implementation boundary.** The later [grocery repair](#grocery-startup-repair-econ-14)
uses this ledger with observed payroll, inventory, supply failures and replacement capital.
Grocery demand/production and displacement are repaired, 1.25× imports are tested together, and
service openings require enough market to cover their first worker and overhead. Next connect
viable industrial sales to industrial demand and address household support against recurring
city funding before selecting additional activities. Retail prices, wages and transfers remain
unchanged; price-only tuning and extra startup capital do not establish healthy growth.

Two accounting details need explicit treatment in that scenario: business profit tax charges
positive daily **cash-budget changes**, resetting its baseline each day without loss carryover;
inventory purchase days therefore differ from this consumed-input ledger. Household admission's
reserve estimate also omits grocery VAT, property tax and the imported-utility surcharge. Both
matter when explaining survival and admission; neither has been changed by this worksheet.

Worksheet verification: five active balance checks, the manually enabled worksheet, 103 active
household checks and three fiscal checks pass. The known export target and seven household
targets remain ignored in these commands. No growth or performance run was repeated for this
test-only addition; earlier growth failures remain evidence for their recorded build. Scenario
diagnostics were completed in the subsequent [village audit](#village-failure-timeline-econ-14);
selected tuning and live healthy-growth acceptance remain open under `ECON-14`.

### Broader employment and content

Develop three complementary sources of demand using existing profiles, resource ports, services,
and fiscal accounting:

| Customer | Employment source | Funding |
| --- | --- | --- |
| Households | Essentials, repairs, personal care, household goods, leisure | Affordable household spending |
| Local businesses and public services | Maintenance, intermediate goods, utility and civic work | Business receipts and recurring public revenue |
| Regional customers | Industrial exports; other exportable activities only if separately designed | Outside purchases with explicit market and delivery capacity |

Candidate additions for the first small expansion are repairs/maintenance, clothing/household
goods, and leisure/hospitality. They are **candidates, not selected profiles or approved numerical
rates**. First recalibrate the existing personal-service and health profiles, which currently
support only four jobs per 1,000 residents together.

Each selected activity must name its payer, spending category, recipe/service output, viable
business size, and added jobs. Replacing imports can retain expenditure locally; an additional
factory still needs buyers. Food eaten in a restaurant must account for household food it
replaces. Discretionary purchases must not become new compulsory bills that bankrupt poorer
households. Reuse a shared demand category for cosmetic variants rather than multiplying demand.

Public-service expansion must name its funding and service need; it is not an unlimited job
generator. Additional resource chains should stay shallow until the existing startup loop works.
New market pricing, new zones, and per-agent shopping systems are not prerequisites.

### Demand and regional trade handoff

`DEM-02` should derive one explainable signal per zone from explicit economic quantities:

- Residential: households able to support themselves and needing homes, supported by funded
  employment or sustainable non-wage income; account for existing residents seeking work first.
- Commercial: affordable spending and unmet service/goods capacity by category, after existing
  and committed supply. A shortage in one category cannot justify an unrelated shop.
- Industrial: local input requirements plus viable regional sales opportunities by resource,
  after existing and committed output. Regional customers must participate in both the signal
  and the capacity reservation used to select actual factories.

Keep home availability, suitable zoned land/assets, access, construction already underway, and
execution cadence as named constraints. Explain the binding constraint to the player. Avoid
stacking several normalized penalties for the same household or business failure. One display
signal must retain resource-specific accounting underneath it, including reservations for
buildings selected earlier in the same pass.

The `DEM-02` implementation now pairs legal housing candidates with workplace completion times
and reserves finite workers per selected project, including queued construction and its delayed
completion dates; [implementation status and evidence](demand_history_2026-10-10.md#implementation-contract-and-progress)
remain authoritative. Employment and admission now share a recurring-sales and working-capital
forecast; unfinished workplaces supply housing pull but no live jobs. The global export-entry
gate is now removed: each project revalidates placed workforce and recurring sales before owner
equity is spent. Failed or bulldozed supporting homes cannot fund another project's labour claim.
Commerce selects from recurring unmet sales value, and spawn percentages no longer suppress
viable construction need. The opt-in growth report also records individual firms' owner
capital, operating cash change and staffing hours, retaining failed/removed firms by site and
generation. This allows expansion checks to expose losses hidden by profitable firms sharing the
same profile. These diagnostics do not add per-firm history to ordinary simulation execution.
Nonworker arrivals can use only ready homes left unreserved by workforce planning. The latest
independent-entry full run passes 2,164 library tests and all three growth scenarios. Its new scarce-labour
expansion check then passes separately after correcting the test's capital bound to count actual
arriving adults: 46 new industrial lots produce three factories, each above 95% staffing and
operating cash positive, with exactly 26,940 owner equity spent. The pre-existing utility-machinery
quantity expectation remains failing. The saved-town replay grows from 7 to 85 residents in
30 days, retains 85 residents / 69 employed after a year, and all 14 recorded firms recover
positive cumulative operating cash without bankruptcy/removal. Full-redesign performance,
candidate-site freight, local buyer affordability/connectivity and remaining demand/UI cleanup
are open.

The subsequent admission cleanup deletes the hypothetical-family runway, marginal commercial
job forecast, regional household cutoff and repeated distress multipliers. Actual batch claims
remain authoritative; income rejection consumes no wage or home reservation. Format 74 removes
unused admission credit and recent-failure memory, while older saves ignore those extra columns.
The deleted marginal forecast removes a commercial-building scan from each demand snapshot.
The focused release run passes 80 demand tests; the full run passes 2,161 library tests and all
three growth scenarios, with the existing utility quantity failure. The saved town retains its
one-year result and the matched incremental 10k comparison detects no significant timing change.
Save continuation is repaired in format 75: traffic smoothing and live agent decisions survive
reload, whole-network costs remain free-flow, and freight quotes reevaluate current frontage delay.
Both queued-construction and developed-town replays now match through 30 days, with cash within
0.01 and discrete outcomes exact; all 35 focused save-related checks pass. The final run passes
2,169 library tests and all three growth scenarios, with only the pre-existing utility quantity
failure. The matched 10k reload-fix comparison detects no significant hourly/daily timing change.
See the demand document for the diagnosis, logs and measurements.

Housing selection now counts each selected asset's actual household capacity against missing
places, after ready and committed homes. It removes averaged building-size quotas and accumulated
spawn credits (save format 76). Unselected legal homes remain available for viable workplaces'
completion-time labour claims; shared reservations still limit entry.
Fresh validation passes 2,165 library tests and all three growth scenarios, with only the existing
utility quantity failure. Both cash continuations pass; two-year firm survival and scarce-labour
expansion results are unchanged, and the matched 10k comparison detects no regression.

Failed-site clearance now depends on actual desertion and empty occupancy/staffing for every
private use. Healthy empty capacity is retained, and market pressure cannot strand failed sites.
Execution rechecks lifecycle state; the existing half-per-day cleanup cadence remains. Despawn
thresholds and saved hysteresis are removed (format 78). A recovery regression also exposed early
reuse of employed workers when their employer's funding fell: three factories claimed a failing
grocery's 12 staff before bankruptcy, preventing its replacement. Snapshot unemployment now follows
actual assignments; revenue-backed funding continues to limit vacancies and admission income.

Household departures now use the same household-specific homelessness/resource policy in planning
and execution. Citywide crisis pressure no longer accelerates departures, and unused slots cannot
remove housed families or those still within their rehousing period. Format 79 drops crisis-removal
credit; the existing departure fraction remains explicit pacing. All 17 housing tests pass after a
fixture-unit correction; the full release run passes all three growth scenarios and both cash
continuations, with the existing utility assertion still tracked. A fresh year of the original
stalled save grows from 7 to 80 residents, with positive operating cash at all 15 firms. Matched
removal timings improve at all three measured sizes; evidence and limits are in the demand document.

Instant business upgrades now check target recurring costs and share customer/workforce claims
with other projects. They credit their own existing customers and jobs once, retain incumbent
payroll, and claim added staff only from unemployed adults or ready housing. Future construction
cannot supply an instant upgrade; rejection leaves market claims, labour and pacing credit intact.
Fresh verification passes 2,173 library tests and all three growth scenarios, including both cash
continuations; only the existing utility assertion fails. The saved-town year retains 80 residents
and all 15 firms with positive operating cash.
The subsequent format-80 cleanup removes upgrade pressure thresholds and saved hysteresis.
Upgrades and hiring share cash, usable stock and paid-cargo valuation; target working capital
replaces the duplicate four/eight-day cash-only buffer. Fresh verification passes 2,174 library
tests and all three growth scenarios, including both cash continuations, with only the existing
utility assertion failing.
Format 81 then removes the remaining downgrade thresholds, transient cash/stock/staffing triggers
and saved hysteresis. Residential downsizing runs after worker claims and cannot remove reserved
ready housing, the vacancy reserve or homes needed by unhoused households. Zoning no longer refers
to demand growth profiles. Fresh verification passes 90 demand tests, 2,178 library tests and all
three growth scenarios, including both reload continuations; only the existing utility assertion
fails. Full-redesign acceptance remains open.

Project costing now uses the proposed asset's actual entrance and outside freight route, sharing
the live hiring/dispatch tariff and cached border sweeps. New entries, alternatives and level
changes no longer budget only a flat dispatch fee. Queued construction requotes before owner
funding; building placements preserve unchanged network tables. Consumer demand now uses each
household's recurring after-tax wages and funded permanent transfers, after housing and utility
costs, with VAT excluded from sellers' revenue. Savings and temporary benefits cannot promise
ongoing sales. Hiring and construction share this calculation; format 82 preserves its income
census across reload instead of rebuilding it at a different hourly phase. Construction and
hiring reserve the same full net recipe capacity, including the interval after completion before
the first worker arrives. Self-produced maintenance is excluded from both saleable output and
external customer demand, and replacement credits release only that firm's net capacity. The shared
operating calculator now accepts per-resource input quotations and matching stock replacement
values. Live hiring now stages finite local production claims with actual supplier prices and
directed freight routes, committing accepted operations and rolling rejected claims back exactly.
New-entry planning reserves those residual inputs for constructing and queued projects before
new candidates. Physical execution refreshes live commitments once per execution epoch and
commits input claims only after placement; rejected alternatives and failed placements release
their claims. Rebuilding project claims releases pending output capacity in reusable private
scratch, then restores each full commitment in sequence; unfinished shops cannot each price
only the final leftover demand and under-reserve inputs.
Live city services now share those finite input reservations. Their material-backed activity
uses the same quote/retry buffers, while public payroll retains the existing fiscal limits.
The service follow-up passes 129 household tests, 99 demand tests, both continuations, all three
growth integrations, expansion and recovery, with all five growth/saved-town reports unchanged.
Upgrades/downgrades now release only their incumbent's accepted input receipts, use quoted stock
values and restore rejection exactly. The subsequent full release run passes 2,207 library tests,
all three growth scenarios and both continuations, with only the known utility assertion failing;
all five reports match. Matched 10k timing detects no incremental change (5.547 → 5.581 ms hourly).
Constructing farms/extractors now quote actual work-area inputs against current capital, and
pending services reserve inputs under a separate fiscal ceiling without advertising live jobs.
Placement and undo register fiscal ceilings immediately; remapping retains fiscal/cargo records,
including restored last indices. Fresh final verification passes 2,211 library tests, all three
growth scenarios and both continuations, with only the known utility assertion failing and all
five reports unchanged. The small initial hourly increase does not reproduce in reversed order
(5.667 ms baseline / 5.661 ms current, p = 0.93). Exact funded recipe activity now bounds private snapshot input customers and recurring source
quantities, including fractional activity below rounded job counts. Public material demand
uses fiscal staffing limits. A valid import operation remains available when local stock
repricing cannot finance equal staffing; rejected local claims are released in full.
This build passes 2,213 library tests, all growth integrations and both continuations, with only
the known utility assertion failing and all five reports unchanged. The initial 4% hourly increase
does not reproduce in reverse order (5.608 ms baseline / 5.503 ms current, p = 0.14).
Equal-route recurring quotes now use actual goods price before building index, matching
physical dispatch for equal batches, including cached searches and rollback. The focused
release economy suite passes 419 tests; full-suite and growth figures above belong to the
preceding funded-activity build.
Recurring quotes now share dispatch's current receiving-window goods multiplier for local
and imported inputs, including buffers and stock valuation; carrier fees are unchanged.
Funding and queued execution revalidate at authored price boundaries. Planning snapshots
consume that same refreshed funding view, so obsolete input customers cannot survive repricing.
Fresh verification passes 2,216 library tests, all three growth integrations and both 30-day
continuations, with only the known utility assertion failing; all five long-run reports match.
The initial 8% hourly timing increase does not reproduce in reverse order (5.628 / 5.657 ms,
p = 0.73). Benchmark discrete counts match, but final treasury is 54 currency lower; this is
not full-redesign performance acceptance. Physical execution verifies 1,024 factories in 110.62 ms.
The initial source set is import-funded staffed production. Accepted private operations now
publish additional locally funded output, capped by current staff, into the same finite book.
Compiled recipe phases place suppliers before downstream buyers; cyclic groups retain stable
building order. Existing claims and construction forks preserve the published quantities.
This publication build passes 2,219 library tests, all growth integrations and both continuations;
only the known utility assertion fails, and all five long-run reports match the freight-window build.
The initial incremental hourly increase does not reproduce in reverse order (5.663 / 5.565 ms,
p = 0.23); discrete benchmark counts match, with current treasury 54 currency higher.
Live consumer funding now stages individual family quantities and post-bill recurring spending,
split by authored basket cost shares. Quotes use seller prices including VAT for affordability,
exclude savings, prove both directed shopping trips, and restore exact accounts and bucket
eligibility on rejection. Accepted net-capacity quotas survive local-input staffing retries.
The household car-access census shares the membership pass and is rebuilt from saved agents.
Commercial construction now shares a fork of the live customer book, including input-free
services, queued projects and incumbent replacement receipts. Quotes provide the worker counts
used by selection, construction calendars and execution. Rejected transactions restore inputs
and customers together. Fallback shopping and freight cost queries now reuse prepared CCH
search storage; exact pedestrian component traversal avoids constructing shopping paths.
Fresh focused checks pass 431 economy and 24 pathing tests before the final bounded route-cache
follow-up. The subsequent current-source release run passes 2,227 library tests, all growth
integrations and both reload continuations; only the known utility assertion fails. The saved-town
year passes with its report unchanged and all 15 firms operating-cash positive. Fresh matched
hourly timing is 6.508 → 6.462 ms against pre-consumer (p = 0.88), but 2.545 → 6.556 ms
against ready-rehousing. The full regression remains; the daily workload changes final employment
and does not establish equivalent economic outcomes. Funded business-buyer closure and replacement
output/downstream revalidation are still unfinished.
The subsequent seller-price correction and reusable commute-query storage pass 2,229 library
tests and all growth integrations/reload continuations, with only the known utility assertion
failing. Growth and saved-town reports are unchanged. Matched hourly/daily samples against the
preceding build show no significant change (6.599 → 6.553 ms / 630.34 → 563.60 ms); the full
ready-rehousing comparison remains regressed. Local forecasts now use seller prices while
exports retain their outside tariff. These results precede the new business-financing regressions.
The subsequent business-customer ledger/router now checks provisional live funding jointly:
shared buyer profit/capital limits and directed seller prices support solvent recipe cycles and
withdraw suppliers of unfunded buyers. Both required financing regressions now pass; the release
economy suite passes 442 tests after excluding household-reserved output from business input
offers. Subsequent work uses f64 forecast money and exact stock-repricing checks, links additional
business deliveries into owned input receipts, and reserves current operations before expansion.
Construction now forks financed business accounts for shared candidate/construction/queue claims;
actual local receipts, export freight, output inventory values and supported staffing travel
together through the quote. The first integration passes 445 optimized economy tests. The preceding
release includes receiving-price boundary checks and passes 2,241 library tests, all growth
integrations, both continuations and the unchanged saved-town year; only the known utility
assertion fails. Matched hourly timing regresses about 51%; reversed order still regresses about
16% (7.0712 → 8.2182 ms). Daily differences are not significant. The populated benchmark ends
with six fewer employed residents, reproducibly; it does not establish equal economic outcomes.
Replacement output
and downstream changes, mixed-market closure and the joint pass's scaling remain unfinished. Subsequent complete freight caches use
prepared bounded hash storage. Live mixed household/business sales preserve fractional activity,
and a joint financing rejection returns household customers to viable competitors within the
same census. These corrections pass 448 optimized economy tests and a fresh release run with
2,244 library passes, all growth integrations and both reload continuations; only the known
utility assertion fails. All four growth reports and the fresh saved-town year match the
preceding build. The first matched hourly increase (7.123 → 8.001 ms) does not reproduce in
reversed order (7.497 / 7.584 ms, p = 0.71); complete economic records match. Earlier
regressions and full performance acceptance remain open.
Replacement probes now enclose input and business-finance changes in first-touch transactions,
so a failed downstream proof can undo several accepted inner owner quotes exactly. The affected
buyer traversal and changed output publication remain unfinished. A new fragmentation regression
also disproves the previous receipt-capacity estimate. Live matching now requests exact storage
before commitment and retries after growing between matching attempts; 452 optimized economy tests
pass. Matched sequential 10k release timing detects no additional regression (7.165 → 6.815 ms
hourly; 590.26 → 591.31 ms daily), with identical economic records; earlier regressions remain.
Construction/replacement arena growth remains unfinished. The preceding rollback/numeric
revision passes 2,247 release library tests, all growth integrations, both reload continuations and
the unchanged saved-town year, with only the known utility failure. See the current evidence in
[`demand.md`](demand_history_2026-10-10.md#continuation-implementation--2026-10-08).
Proposed replacement recipes now have prepared supplier entries with actual doors/prices and
transactional output changes. CCH metric updates retain this geometry while invalidating prices
and refreshing route bounds. The focused release suite passes 453 economy tests. Replacement
admission still needs to connect output publication to affected-buyer financing revalidation.
The small initial hourly timing increase is not significant in reversed order (6.641 ms baseline /
6.742 ms current, p = 0.30); complete economic records match. Earlier regressions remain open.
The next revision retains accepted operation plans for construction, caps replacement payroll
at the target's physical slots, and adds buyer-local business receipt withdrawal with exact
rollback through the existing journal. Retained prices include the site's receiving-window
multiplier. Its 455 release economy tests pass, and matched 10k timing detects no additional
regression with identical economic records (6.882 → 6.962 ms hourly, p = 0.38).
Replacement admission still needs to invoke withdrawal and revalidate affected operations;
older performance regressions and full acceptance remain open.
Customer lookup now separates fixed buyer/recipe ports from financial tranches, retaining
geometry across funding, fragmentation and metric changes. Fresh release checks pass 2,251
library tests, all growth integrations, both continuations and the unchanged saved-town year;
only the known utility assertion fails. Matched hourly timing improves in both run orders
(8.116 → 6.930 ms initially; 8.333 ms baseline / 6.959 ms current reversed), with identical
economic records. Full-redesign performance and replacement integration remain unfinished.
The subsequent bucket-scan correction stops after the final active entry, preserving item
order without visiting distant inactive geometry. All 456 focused release economy tests and
both locality benchmarks pass. Its initial small hourly increase does not reproduce in
reversed order (7.011 ms baseline / 6.940 ms current, p = 0.51); economic records match.
Full-suite evidence above precedes this correction; remaining runtime and performance
acceptance is still open, with commands and artifact identities recorded in `demand.md`.
Buyer input reseeding now reuses withdrawn tranches, preflights account capacity before mutation,
and restores account links, quote data, financing and routing availability on outer rejection.
Its 457 focused release economy tests pass. The initial hourly timing increase does not
reproduce in reversed order (6.955 ms baseline / 6.869 ms current, p = 0.29), with identical
economic records. Runtime replacement integration and full performance acceptance remain
pending. The preceding full suite does not validate this later ledger change.
The following revision connects household claims to the runtime replacement transaction and
recycles owner receipts. Multi-owner rejection restores exact funds, links and mode eligibility;
458 optimized economy tests pass. Fresh broad verification passes 2,254 release library tests,
all growth integrations, both reload continuations and the unchanged saved-town year; only the
known utility assertion fails. The initial apparent hourly gain does not reproduce in reversed
order (6.947 ms baseline / 6.951 ms current, p = 0.95), with identical economic records.
Output publication, affected-operation financing and full performance acceptance remain unfinished.
New, queued and constructing business inputs now become shared future demand only after the
project's own financing proof. Placed construction uses actual cash, paid cargo and usable stock;
later placement/workforce rejection restores provisional customer/input commitments together.
Prepared buyer doors retain directed routing and receiving-price changes; input surcharges never
increase the startup equity grant. Fresh checks pass 459 focused economy tests, 2,255 release
library tests, all growth integrations, both continuations and the unchanged saved-town year;
only the known utility assertion fails. Matched timing detects no significant change in either
order (6.957 ms baseline / 7.066 ms current hourly in reverse order, p = 0.22). Replacement
dependency revalidation and full performance acceptance remain open, with evidence in `demand.md`.
The subsequent input-ledger repair preserves unaffected supplier tranches and quotes only a
withdrawn source's gap, with exact outer rollback. Its 460 focused release economy tests pass;
the following selection integration now withdraws/publishes replacement output and revalidates
affected buyers and suppliers together. It retains actual-priced future purchases without
double-counting their physical delivery and rolls back the entire proof before growing receipt
storage. All 463 focused release economy tests pass, including rejection when a replacement
would leave its buyer or former supplier insolvent. The subsequent broad run passes 2,259
library tests, all growth integrations, both reload continuations and the unchanged saved-town
year, with only the known utility assertion failing. Matched hourly timing regresses
6.9024 → 7.1106 ms (+3.0%, p = 0.01). The subsequent physical execution guard also passes
2,259 library tests, all growth/reload checks and the unchanged saved-town year; its matched
hourly timing increases 6.9887 → 7.2055 ms (+3.1%, p = 0.01). Physical and household receipt
capacity preflight and shared full-proposal replay pass 465 focused release economy tests,
including forced arena growth and exact runtime replacement rollback. Matched hourly timing
shows no significant additional change (7.2829 → 7.3068 ms, p = 0.78), with identical economic
records. Subsequent queue execution retains financed commitments backed by placed housing
and retires failed entries before dependent revalidation; 14 targeted queue tests pass,
including pending-customer reload. The final queue source passes 2,263 library tests, all
growth/reload checks and the unchanged saved-town year, with only the known utility assertion
failing. Its initial hourly timing increase does not reproduce in reversed order (7.2386 ms
baseline / 7.2114 ms current, p = 0.74), with identical economic records. Fresh profiling
identifies CCH query work as a major cost; direct road-end label indexing passes 25 pathing tests and the final build passes 2,264 library tests, all growth/reload checks and the unchanged saved-town year (only the known utility assertion fails). Matched hourly timing improves 7.1613 → 6.9396 ms; daily timing shows no significant change. The full regression remains. Prepared household bucket positions subsequently pass 466 economy tests, including four-regime populated-market isolation. The first timing run regresses; reversed and repeated comparisons improve, the latter 7.1301 → 6.7020 ms hourly, with identical economic records. That runtime build passes 2,265 library tests, all growth/reload checks and the unchanged saved-town year, with only the known utility assertion failing. Connected-market repricing now passes after correcting premature customer ceilings; 2,266 library tests, all three growth integrations and the saved-town year pass apart from the known utility assertion. Affected input repair passes 468 focused economy tests and restores mixed-market closure from five to three full passes (53.262 → 34.494 ms at 102,400 residents); matched whole-economy timing shows no significant change and identical economic records. Its broad run passes 2,267 library tests, all three growth integrations and the saved-town year apart from the known utility assertion, with all five reports unchanged. The failed-expansion receipt-loss fix passes 469 focused economy tests and 2,268 library tests plus all three growth integrations and the saved-town year, apart from the known utility assertion; all five reports match. Repeated mixed-market timings are closer, with middle-size increases still recorded. Exact household-quote prefix reuse passes 470 focused economy tests, including an uncached oracle and changed-prefix reconstruction, and improves the largest mixed-market diagnostic from 35.408 to 28.114 ms. Whole-economy timing has no significant change and identical records. Broad validation passes 2,269 library tests, all three growth integrations and the saved-town year apart from the known utility assertion; all five reports match. A fresh whole-workload profile points to daily commute routing and hourly entrance geometry; sample loss and incomplete stacks limit the diagnostic. Lazy freight geometry and indexed business-customer route reuse pass 472 focused economy tests. Matched whole-economy timing has no significant change and identical economic records; after temporary-quota recovery, 2,271 library tests, all three growth integrations and the saved-town year pass apart from the known utility assertion; all five reports match. Subsequent worker-local commute-leg reuse and job-query scratch retention pass 473 focused economy tests. Matched timing improves daily settlement 514.89 → 103.30 ms (−79.94%) with identical economic records; hourly 7.0818 → 7.0013 ms has no significant change. Broad validation passes 2,272 library tests, all three growth integrations and the saved-town year apart from the known utility assertion; all five reports match. Exact cross-census household quotation reuse now validates budget/access inputs and seller requests before retaining claims; changed prefixes reset prepared accounts without recalculating the census. Fresh validation passes 475 focused economy tests, 2,275 library tests, all three growth integrations and the saved-town year, with only the known utility assertion and all five reports unchanged. Matched hourly timing improves 6.4418 → 5.2694 ms (−18.2%) with identical economic records; daily timing shows no significant change. Mixed-market warm medians decrease at all five measured sizes in both run orders. Full-route selection now also reuses prepared scratch, eliminating discarded candidate paths and connector allocations. Fresh validation passes 476 focused economy tests, 2,276 library tests, all three growth integrations and the saved-town year, with only the known utility assertion; all five reports match. Exact route-product comparisons pass and warm route medians improve at all four sizes in both run orders. Whole-economy timing detects no significant change (5.3495 → 5.3587 ms hourly; 99.079 → 97.666 ms daily), with matching economic records. The replacement matrix now also passes five queued-buyer variants, with exact all-owner financing/input/household rollback, 15 supported workers and subsequent physical grocery placement. Its final build passes 476 focused economy tests, 2,276 library tests, all three growth integrations and the saved-town year, with only the known utility assertion and all five reports unchanged. Matched timing detects no significant change. Corrected retry attribution puts local funding at 3.352 ms versus import funding at 0.283 ms in the diagnostic profile. Globally absent-customer propagation now reduces the 64-link withdrawal case from about 30 ms to 1.1 ms and from 64 censuses to two. Buyer counting shares the parallel market census; retries scan staffed input buyers. Its final build passes 478 economy tests, 2,278 library tests, all three growth integrations and the saved-town year, with only the known utility assertion and all five reports unchanged. Whole-economy timing detects no significant change; the initial largest mixed-market penalty no longer appears. A subsequent stale projected-supplier handle fix validates current membership before owner lookup; its fresh broad checks retain these counts and reports, with no significant timing change. The per-buyer search-cache trial was removed without a whole-economy gain. Disconnected chains now use component-local buyer tallies: the 64-link/16,384-background case falls from 29.975 to 1.261 ms (64 censuses to two). Final checks pass 2,280 library tests, all three growth integrations and the saved-town year with only the known utility assertion and all five reports unchanged; matched 10k timing detects no significant change (5.4304 → 5.4566 ms hourly, p = 0.84). The remaining unused-method warnings are resolved: superseded probes/harnesses become test-only. The long-standing utility assertion (540 versus 96) was a stale fixture from the retired MJ power recipe; it now expects 540 kg, so later runs have no known failure. The 2026-10-09 correctness follow-up propagates recounted purchase bounds through partly active supplier chains (32 censuses to at most three, exact against replay), preflights census receipt fragmentation exactly, and covers queued supplier-replacement and cyclic-replacement reloads; 2,283 library tests, all three growth integrations and all five reports pass unchanged, while one of four 10k hourly comparisons detected +12.9% that the others did not reproduce. The 2026-10-09 performance follow-up attributes the hourly gap to serial joint financing (supplier search rejected 90% of visited candidates), skips those regions with exact door bounds, hashes registry lookups faster and proves cashless firms unfundable up front: 10k hourly time falls 5.9985 → 5.0895 ms (−15.15%, p = 0.0014) with identical records, the Step-1 +12.9% flag is not reproduced (p = 0.25), the queue slowdown is explained as new required work, and cashless-rival cascade combinations need two censuses instead of one per link; profitability-failing chains still need one census per link, and the payroll/hiring mismatch (`ECON-17`) is fixed: payroll and worker shedding read the funded census capacity, removing the 10k fixture's 355 daily unpaid quits and 611 re-hires with all five reports unchanged. Broader populated execution/scaling stays open; `demand.md` owns the evidence.
Broader replacement coverage and both incremental/full-redesign performance remain open.
`demand.md` owns the evidence and validation boundaries.
Preceding broad verification passes 2,238 library tests, all growth integrations, both reload
continuations and the unchanged saved-town year; only the known utility assertion fails.
Matched hourly timing still regresses 7.0255 → 7.8825 ms (+12.199%); daily timing has no significant
change and complete economic records match. The evidence and its validation boundary are maintained in
[`demand.md`](demand_history_2026-10-10.md#continuation-implementation--2026-10-08).
Owner receipts and cached directed pedestrian proofs now pass final focused checks: 430 economy,
23 pathing and 25 save tests, both reload continuations and the unchanged saved-town year.
That historical matched 10k hourly cost increases 5.7515 → 6.4500 ms (+12.1%);
this regression remains open. See the [continuation work](demand_history_2026-10-10.md#continuation-implementation--2026-10-08)
for remaining work and the distinction between final focused and earlier full-suite evidence.
Funded-buyer closure, cyclic local chains and replacement output publication are implemented;
complete market and performance acceptance remain open under `DEM-02`. Physical dispatch
compares delivered local/import prices when both are executable.
The preceding full construction-input release run passes 2,204 library tests and all three growth scenarios, including
both cash continuations; only the existing utility assertion fails. The saved town retains
80 residents and all 15 operating-cash-positive firms. The matched 10k hourly regression remains
unresolved at 2.69 → 5.79 ms after reducing the initial local-supply cost of 13.30 ms.
Construction integration adds no detected hourly regression (5.672 → 5.655 ms); all city records
match. A separate physical-execution benchmark places 1,024 import-backed factories in 113.36 ms,
excluding setup and first-check preparation, while verifying every placement and 13,414,400
owner equity. Populated local-market and full-redesign scaling remain open.
Validation evidence belongs in the
[demand contract](demand_history_2026-10-10.md#implementation-contract-and-progress).

The growth HUD now reads cached hourly decisions instead of normalized pressure. It shows exact
selected/queued/constructing counts and housing, asset, site, customer, income and labour constraints.
Unused spawn thresholds and saved display hysteresis are removed (format 77). Counters reuse the
existing parallel discovery and snapshot passes; the UI itself performs no simulation decisions.
Format 83 removes the remaining diagnostic R/C/I pressure values and normalization work. Daily
departures use only the existing parallel household census and individual eligibility; hourly
projects and the HUD retain their concrete decisions. Full-redesign acceptance stays open.
Display cleanup passes 2,167 library tests and all three growth scenarios, with the existing
utility failure, plus isolated headless/rendered HUD checks. A reversed matched 10k comparison
records +0.126 ms (about 5.3%) hourly; daily timing is statistically unchanged. This cost remains
under investigation, rather than being recorded as performance equivalence.

Business selection now tries viable alternatives on a rejected lot, including a smaller factory
when the preferred workforce cannot be supplied and other service categories after customers
are reserved. The average-output quota and single export opening per pass no longer cap business
selection; shared worker/customer reservations limit projects and the minute queue paces starts.
The scarce-labour expansion regression passes with five new machinery firms, each above 99.8%
lifetime staffing and operating-cash positive. Owner equity is 34,600 against the 38,060 bound
from 22 observed additional available workers; no incumbent industrial losses are hidden.
All three growth scenarios pass, with 19 individually profitable firms retained through two years;
The selection revision passed 2,165 library tests; its cash-continuation failure is repaired above,
while the utility quantity failure remains separate. The matched
incremental 10k comparison detects no significant hourly/daily change; active-option scaling is
not established by that fully built fixture. Evidence is recorded in the demand document.

Project entry now shares its expense arithmetic in `economy/operation.rs`: profile-specific kWh,
net external inputs, gross wages, property/other utilities and freight based on the existing batch
rounding rules. Opening inventory capital is separate from recurring expenses. Both groceries and
services need sales covering their proposed workforce. Live hiring uses actual cached border
route charges and counts stock, paid inbound cargo and dispatched receivables as working capital,
without treating owner equity as sales. Candidate-site quotes and affordability/connectivity of
local buyers remain open; successful growth runs alone do not establish those contracts.

Regional opportunity needs a deterministic definition of buyers, purchasable output, effective
price, and delivery limits. Prevent duplicate factories from all claiming the same opportunity.
Imports must allow the supported starter businesses to survive; established local suppliers
should improve costs and employment. Ordinary supported exporters must cover costs after any
saturation rule. Preserve local input reservations and prevent profitable buy/resell arbitrage.
The old `export multiplier >= 1.0` proposal is replaced by these viability requirements; the
actual price and volume rules must be derived from the ledger before implementation.

### Delivery plan

`ECON-11` and `ECON-12` remain completed foundations. Extend their tests and scenarios as part of
`ECON-14`; they are not evidence that healthy growth is already solved. Numeric acceptance ranges,
sector selection, and regional market formulas remain to be designed.

| Sequence | Owner | Deliverable | Acceptance before proceeding |
| --- | --- | --- | --- |
| Define budgets and employment | `ECON-14` | Household-to-business ledger, actual working population, per-activity margins and supported jobs; scenario diagnostics that explain failures. | Reproducible arithmetic using shipped data, named recipients/sources for money flows, explicit gaps and chosen acceptance ranges. |
| Repair the startup economy | `ECON-14` | Viable starter grocery and industry with imported inputs, partial staffing, overhead, delivery delays and finite startup funds; failed sites can be replaced under positive demand. | Targeted balance/recovery checks plus a village run that survives beyond starter capital; local suppliers improve the supported operating case. |
| Establish wider economic demand | `ECON-14` | Viable regional purchases and a small selected set of additional activities; recalibrate existing services. | Each activity has affordable customers, an explicit funding source and viable scale. Exports pass cost and capacity checks. |
| Simplify growth decisions | `DEM-02`, coordinated with `ECON-14` | Capacity-based residential/commercial/industrial signals and explicit execution constraints, including regional customers. | Signals can be calculated by hand; new and pending construction reserve the correct demand; no duplicate pressure model remains. |
| Validate village, town, and diversified city | `ECON-14` and `DEM-02` | Deterministic scenarios with realistic starting funds, suitable land/access and stated player-placed facilities. | Business survival, household affordability, recurring fiscal balance and sustained growth meet the agreed ranges; mature stability is valid when capacity is satisfied. |
| Measure relevant changes | `ECON-13` | Matched hourly/daily release comparisons at 10k; record setup separately and inspect slow settlement days. | No unexplained regression for the changed workload; no 100k or 1M run required. This runs alongside implementation, not ahead of all design work. |

The trade and demand steps must be developed together where regional demand changes the capacity
calculation. Do not land a price-only fix and assume industrial construction will discover the
new customers automatically. Tests should extend the existing ledger and scenario infrastructure;
the removed editor/sandbox is not to be recreated.

### Healthy-growth acceptance

Population alone is insufficient. Scenarios must expose:

- household disposable income, essential stockouts, employment by eligible population, funded
  vacancies, and immigration/emigration with their limiting reasons;
- business sales/utilization, local/imported input costs, payroll, operating cash, closures and
  replacement rates, distinguishing private startup equity from recurring receipts;
- city recurring income/expense by source, transfers, service costs and infrastructure upkeep,
  separately from one-time build costs and starting treasury;
- import/export quantities and values, committed construction, and whether growth has exhausted
  available land, transport, labor, or customer demand.

Run beyond the applicable household support and business startup windows. Require sustainable
operation and growth where opportunity exists, not monotonically rising population in a finite
map or zero bankruptcies. Include recovery after a temporary supply interruption. Check more than
one viable zoning/service mix so tuning cannot merely memorize the original starter fixture.

The original year-one test only compares final versus midyear households and requires positive
treasury throughout. Retain it as an initial regression, then add the above causal measurements
and explicit bounds under `ECON-14`. Do not pass it by increasing initial treasury or repeatedly
replacing insolvent businesses. Prepopulated performance cities with unlimited funding do not
establish growth or solvency.

### Runtime constraints

Reuse the existing household/building stores, resource catalog, supplier index, and routing/chunk
indices. Aggregate spending and capacity on the existing hourly/daily cadence. With a fixed
authored catalog, new aggregation work should be O(H × C + B × P + S), where H is households, C is
consumption categories, B is buildings, P is profile ports, and S is active shipments; bound
routing, existing agent/labor passes, and candidate selection separately. Use existing indices
for searches. No household-to-every-shop or
building-to-every-supplier scans, per-agent inventory model, or new hot-loop allocations.
Independent bulk passes use Rayon with deterministic reduction/order. Record build, workload,
worker settings and measurements when accepting runtime changes.

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

Ordinary freight follows that rule. The existing emergency inventory liquidation is an explicit
immediate cash-settlement exception; see [building bankruptcy](#building-bankruptcy).

Important exception for `v0.1`:

- networked utilities such as `power`, `water`, and `sewage` should not behave like trucked goods in the first pass
- they use the separate `Utility Service Layer` described later in this document

This creates the intended feedback loop:

- delayed deliveries reduce local inventories
- low inventory or household supplies reduce household satisfaction or business throughput
- congestion becomes an economic problem, not just a traffic problem

### 4. Economy data is hand-authored text; balance is checked outside the game

Economy profiles, recipes, prices, wages, and runtime tuning are hand-authored in
`economy/profiles.toml`. Startup validation and compilation into the runtime catalog reject
malformed data. The game has no developer economy editor: the former editor, sandbox playback,
controllers, and scenarios were removed on 2026-10-02 because none of them fed the runtime.

Player-facing fiscal controls such as tax sliders and household transfer levels are a separate
gameplay policy layer. Baseline income tax, household VAT, business profit tax, daily property
taxes, unemployment benefit, pension, and child support defaults come from authored runtime tuning,
then become live `CityFiscalPolicy` state exposed through curated bounded controls.

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
Agent movement and scheduling borrow the same `TimeSystem` for date, minute and conversion rate. The
saved day duration remains authoritative when current tuning changes (`AUDIT-01-A2`); configuration
supplies the default for new worlds and the authored schedule/refresh intervals. `AUDIT-01-A1`
covers half, default and double day durations, including departure marks and refresh deadlines. Five
matched release process pairs (`benchmark_commute_cache_recording --ignored --nocapture`, 24 Rayon
workers, 11 samples of one million recordings) measure about 1.00 ms before and 2.13–2.22 ms after
the correction. This periodic O(1) arithmetic adds no allocation; it does not measure route-search
cost. The later live-clock ownership correction measures 2.172–2.178 → 2.409–2.425 ms per million
recordings in five CPU-0 matched release pairs. Full farm tick comparisons retain comparable cost at
1,000/10,000/100,000 agents; 24-worker runs are noisy, while three CPU-0/one-worker pairs give
1.375/1.775 → 1.322/1.717 ms for 100,000 idle/working agents.

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

- physical production and consumption: `kg/day`; utility energy `MJ/day`, volume `m³/day`, services `service equivalents/day`
- supply reserve: `days of supply`
- wages and operating costs: `currency/day` or `currency/workday`
- prices: currency per resource's canonical quantity (`kg`, `MJ`, `m³`, or service equivalent)

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

The 24-worker million-agent median improves 5.820677 → 0.649441 ms. The accepted tradeoff is roughly
3–11 microseconds of extra daily work in the small physical-core fixtures and about 16% overhead in
the CPU-0/one-worker million-agent case (5.844838 → 6.762297 ms). These results concern one daily
state pass, not the full daily settlement or frame time. Initial fine-grained dispatch was
substantially worse for small collections and is retained only as diagnostic evidence. The benchmark
uses minimal agent SoA setup, a fixed pollution field, three warmups and 21 samples of eight
updates, resetting happiness outside timing. Command: the recorded test binary with `--exact
simulation::economy::agents::daily::tests::benchmark_daily_agent_update --ignored --nocapture`.

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

The runtime caches that departure minute and compares it with the active clock during indoor
movement updates. This is a cheap deadline check, not a route search each frame. Hourly polling
would miss departures between hours; preserve minute-based timing and cached commute estimates.

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
- **Funded hiring**: Employment and admission share the maximum workforce whose forecast recurring sales cover operating expenses and whose cash, stock and unsettled freight assets cover working capital. A normal inventory purchase or payroll settlement therefore does not erase jobs solely by moving cash. Owner equity supplies capital, never sales. Unfinished and inactive private employers advertise no live jobs; city services use their explicit funding limit. Daily payroll pays at most that same census capacity, bounded by the current physical workplace; service stores and explicit work areas release workers above it, power funding changes shed to the same fiscal ceiling, and other unpaid workers leave after two days (`ECON-17`). See the current [implementation and remaining market constraints](demand_history_2026-10-10.md#implementation-contract-and-progress).
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
- `unemployment_max_days`: maximum unemployment-benefit duration; loss of income can lead to
  housing distress, while demand owns the separate removal rules

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
  control's ledger bucket; economy profiles stay hand-authored in `economy/profiles.toml`

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
- **Batching and payload**: Local deliveries, imports and exports round cargo to the resource's `shipment_batch_kg`, capped at `truck_payload_kg` (20,000 kg) per carrier. Industrial inputs reorder below two days of stock toward a four-day target; exports accumulate a daily production lot before dispatch. Hourly review does not mean hourly trucks.
- **Capital Lockdown**: While a shipment is open, the associated budget or inventory is locked and cannot be double-spent. Source inventory is removed from the building only when the physical carrier is successfully dispatched.
- **Deadlines**: Route costs are movement seconds. Convert them using `24 / seconds_per_day`
  before comparing to operational shipment age; outside-window penalties remain operational
  minutes. Existing bounded transit/return timeouts still apply.
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

These mechanisms support initial establishment. Sustained viability requires paying customers
and recurring household income; the regional-market checks and evidence below establish that
for the starter fixture. Startup grants alone are not evidence of viability.

This section uses `startup economy` to mean early-city money, inventories, freight, and `OWA` support. It does not own any special fresh-map building-placement exception, and it does not own the demand-side decision about whether new households should be admitted.

[`docs/demand.md`](demand_history_2026-10-10.md) owns whether the city admits households at all and how many it admits. This document owns the startup money, inventories, freight, and runtime consequences once those households already exist.

### Outside World Exchange (`OWA`)

For `v0.1`, the outside world should be represented by an `Outside World Exchange` (`OWA`) rather than by goods appearing magically inside city inventories.

Rules:

- the `OWA` is an abstract external buyer and seller, not a normal local factory
- imports and exports are available only when the city has at least one connected border connection
- connected border connections act as the physical ingress and egress gateways of the `OWA`
- the `OWA` may sell supported external goods without consuming local inputs and may buy exported city goods as an external sink
- the `OWA` may also provide missing utility service such as imported `power`, imported `water`, or external `sewage` processing as paid external services
- current import asks and export bids derive from catalog unit prices and authored multipliers;
  freight timing can modify goods prices. Carrier fees cover dispatch and round-trip vehicle time.
  Export bids are fixed, with no buying quota or price-decay market
- import asks must exceed comparable export bids so buying and reselling the same resource
  cannot create profit; `ECON-14` must preserve this across effective price modifiers
- industrial-zone producers and explicit field producers/extractors may export surplus; the
  latter can use `ZoneType::None`. Commercial service outputs cannot be exported as inventory
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
- [`docs/entrance_and_exit.md`](../entrance_and_exit.md) should own any later household
  `OutsideGateway` arrival or departure trip semantics
- [`docs/demand.md`](demand_history_2026-10-10.md) still owns whether households are admitted or removed at all

### Border connections and physical freight

External trade must be physically delivered through the city, not teleported into a warehouse or shop.

This physical border-freight rule applies to ordinary imported and exported goods. It does not apply to `OWA` utility fallback service purchases, which are handled through the `Utility Service Layer` instead.

Recommended `v0.1` rule:

- imported goods enter the city only through explicit border connections
- when an import request is accepted, the corresponding freight starts at the fastest connected border connection and must travel physically to its destination
- exported goods must be transported physically to a border connection before leaving the city
- in `v0.1`, those border connections are the `OWA`'s border terminals
- `OWA` border terminals accept every load at once; there is no per-border job limit or queue
- every load is still a physical freight vehicle, so road traffic is the only throughput limit for `v0.1`

The former limit of four active and four queued jobs per border (removed 2026-10-09) let one
border starve a 10k city's 900 industrial firms of all imports and exports.

This keeps outside trade grounded in the same traffic and delivery logic as local freight.

### Local advantage over permanent `OWA` reliance

Imports should support establishment, and local supply should improve costs and employment.
The following describes the current sourcing/export rules. Their prices do not yet satisfy the
[healthy-growth viability requirements](#demand-and-regional-trade-handoff).

Local supply chains should usually beat permanent `OWA` dependence through:

- lower effective unit cost once production and labor are stable — enforced by `owa_import_price_multiplier`
- shorter freight routes than hauling through a border connection
- less exposure to player tariffs and other trade penalties later
- stronger local employment and tax base

The logistics system tries local suppliers first and falls back to the `OWA` only when no valid local source is available. The `OWA` import price is derived as `local_unit_price × owa_import_price_multiplier` (configured in `[runtime_tuning]` in `economy/profiles.toml`), giving local goods a lower base price. Route-dependent carrier fees also affect the delivered price; local-first sourcing remains the current policy.

Local business and utility input demand also has priority over industrial exports. Before an industrial
building exports surplus output, the export planner computes affordable, non-terminal customer
input requests using the same batch rounding and payload limit, compatible-supplier index, component
reachability, and exact freight-route feasibility as inbound freight. Only demand that can be
served by a valid local supplier creates a source-specific output hold. Active inbound
reservations already count toward the buyer's expected inventory coverage, so this hold protects unmet local demand
without double-counting open freight jobs or blocking exports for disconnected/unreachable
buyers.

Daily demand snapshots read business and utility `OWA` input reliance as import-substitution pressure for
industrial growth. That pressure is diagnostic of actual outside-input use; industrial spawn
quantity reserves local customer capacity, while export-led openings use the staged operating-viability gate in
[`demand.md`](demand_history_2026-10-10.md).

Exports sell surplus to outside buyers without a fixed quantity ceiling. When an industrial building's
unreserved output inventory exceeds a **one-day production buffer** after local input holds and
its own same-resource input reserve, the
logistics system creates an outbound export shipment to the nearest valid `OWA` border terminal.
For explicit field producers and extractors, output inventory capacity uses the committed
area-scaled daily output, while the export buffer uses current active staffed output so a weakly
staffed starter farm is not forced to accumulate a full-field theoretical reserve before exporting.
`OWA` export demand is allowed to back explicit field/extractor staffing when scheduled exports are
enabled and the site has a reachable outside freight gateway. Actual hiring requires recurring
sales to cover costs and assets to bridge replenishment/payroll; production still needs inputs
and output headroom. Net delivered sales fund subsequent operation;
local input holds preserve priority for reachable local buyers.

**Export Constraints**:
- **Pricing**: The normal bid is `local_unit_price × owa_export_price_multiplier` (currently
  1.0×). Imports remain 1.25×. Prices alone do not authorize factory expansion: remaining
  customers must cover full payroll, imported inputs and overhead.
- **Freight economics**: every contract quotes `4 + 4 × round_trip_vehicle_hours`, using the
  existing cached one-way route time and operational clock. The quote stays fixed through queueing,
  delivery and return. Export gross receipts must exceed that fee; physical delivery earns the
  sale, less freight. Local/import buyers must afford goods plus freight before reserving stock.
- **No buyer quotas**: buildings and borders share no daily purchase ledger. Inventory,
  20 t payloads, terminal active/queued job limits, cooldowns, roads, labor, inputs, cash and
  area maintenance still constrain production and delivery. Outside demand is not added as
  infinite local unmet capacity; factory expansion is staged separately.
- **Distress pricing**: forced bankruptcy liquidation uses the separate `owa_distress_liquidation_multiplier`, which must be no higher than the scheduled export multiplier. The shipped value is lower than scheduled export pricing so fire-sale liquidation is a rescue path, not a preferred operating model.
- **Efficiency**: Beyond the retained local buffer, exports accumulate one day's net output, rounded up to the resource's `shipment_batch_kg` and capped at the largest legal truck load. They retain the existing shipment cooldown. Small producers may ship partial loads; output above one payload can require several trucks per day. A backlogged site can also clear accumulated stock across multiple loads.
- **Eligibility**: Industrial-zone producers and explicit field producers/extractors may export;
  commercial stores do not export inventory through scheduled freight.

### Household admission and removal handoff

Household admission and removal affect labor supply, consumption, service load, and business viability, but the city-level decision about whether that change should happen belongs to [`docs/demand.md`](demand_history_2026-10-10.md), not to this document.

For `v0.1`, the economy-side contract is:

- household admission and household removal happen at whole-household granularity, not one unrelated resident at a time
- economy creates and owns the admitted `Household` runtime record once demand has already decided the outcome
- admitted households receive startup state such as shared savings and household supplies through the economy rules in this document
- demand may read economy-owned starter savings, essential cost, exact candidate composition,
  transfer-policy amounts, treasury balance, and budget-backed job openings to calculate
  deterministic move-in acceptance. Ongoing wages plus funded pension/child support must cover
  essential consumption including VAT, imported utilities and home property tax. Temporary
  unemployment support and starter savings cannot justify permanent admission. Economy owns
  actual transfer payment and household materialization
- current household admission launches an arrival carrier and materializes the household when
  that carrier reaches its home; approved composition travels with the carrier. Pending adults
  and dependents reserve forecast jobs and benefit coverage, and each launch consumes the batch's
  remaining income capacity. Outside applicants advance with simulation hours even after a
  rejection. The shared headless growth scenario therefore advances agents
- trip execution belongs to the transport layer; demand owns admission decisions and economy
  owns the household created on arrival
- births and other within-household demographic change are later systems, not part of the `v0.1` economy model

### Household housing affordability, relocation, and eviction

Household affordability failure should not directly downgrade a residential building.

Cross-system rule:

- this document owns whether a household can afford to stay in its current home
- this document owns household-side relocation between vacant homes inside the city
- [`docs/demand.md`](demand_history_2026-10-10.md) owns whether a household ultimately leaves the city
- [`docs/zoning.md`](../zoning.md) owns what buildings and levels are legally allowed on the site if
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
- already-unhoused households also attempt affordable rehousing after each hourly membership
  census, before job assignment and arrival planning. They claim real vacancies through the
  existing occupancy mutators, so a completed home cannot also admit an arriving household.
  The admission index carries level/building ordering for this lookup; unaffordable higher-level
  homes remain available to other households. Hourly retries do not advance daily eviction or
  unhoused-duration counters. The fully housed case skips this phase in O(1); active rehousing
  uses O(H + U*(levels + log V) + moved members), with reused member links and no vacancy scan.
  Both hourly and daily relocation release occupancy in an old unusable home when its household
  moves elsewhere. Fresh verification passes 2,189 library tests and all three growth scenarios,
  including both cash reload checks; only the existing utility quantity assertion fails. The
  saved-town year and growth reports are unchanged. Matched 10k timing shows no significant
  change, and 128 rehousing claims stay near 0.02 ms through 65,536 unrelated vacant homes;
  see [the validation evidence](demand_history_2026-10-10.md#outputs).
- future-home workforce reservations apply the same individual relocation thresholds before
  crediting incoming workers. Displaced residents reserve physical household places, contributing
  only their actual unemployed adults; households with no available workers still consume a
  place. Post-employment money synchronization refreshes household employment counts from current
  agent assignments, so unhoused employees do not erase housed unemployed adults. See the
  [future-home accounting and validation](demand_history_2026-10-10.md#outputs).

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
Median microseconds/query before → after: radius 8 `0.302 → 0.161`, 32 `4.954 → 0.639`, 80 `28.543 →
1.590`, 128 `71.441 → 2.541`. This measures the chunk-distance bound, not city FPS. The household
suite freshly passes 102 tests (3 manual benchmarks ignored), including pre/post-pickup eviction
accounting and cross-ring distance ties; whole-codebase audit validation remains ongoing.

Household-removal index repair now composes chained swap moves and updates surviving agents once
in parallel: O(A + R) household-reference work and O(R) temporary mappings for A agents and R
removed households, instead of O(A × R). Candidate ranking and freight-carrier remapping are
separate costs. Resident and freight removal share the agent-record mutator; it repairs a moved
shopper's household reference immediately in O(1), preserving the active shopping reservation.

Matched unprofiled release removal measurements used `RAYON_NUM_THREADS=24 cargo test --offline
--manifest-path rust/Cargo.toml --release --lib benchmark_household_removal -- --ignored
--nocapture`, seven samples per size, 256 removed households and one resident per household.
Setup is excluded. Median milliseconds before → after: 1,024 households `1.547 → 0.113`, 8,192
`14.185 → 0.394`, 65,536 `116.395 → 2.535`. These fixtures have no freight orders; they measure
selection/removal and household-ID repair, not all possible emigration workloads.

Deterministic household-removal selection:

1. Demand counts households satisfying `HouseholdExitPolicy` after relocation and eviction, then
   applies the explicit daily departure fraction/cap described in [demand outputs](demand_history_2026-10-10.md#outputs).
2. Execution rechecks that same policy against live state. A household must still be unhoused and
   past either the destitution delay or the maximum rehousing period. Housed households never qualify.
3. Eligible households are ranked by lower essential cash reserve days, then lower pantry days,
   then lower household ID. Remove at most the planned count; no other households fill unused slots.
4. Remove in descending ID order through the existing lifecycle and repair survivor/agent references.

Citywide crisis pressure no longer sets departures. A household that recovered after planning is
protected at execution. Eligibility uses an O(H) parallel pass and a reused O(H) candidate buffer;
ranking is O(E log E) for E eligible households. This replaces the separate allocated housed/unhoused
lists and whole-population ranking. Agent-removal and survivor-reference repair are unchanged.

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

This layer should operate mostly on coarse aggregate data rather than per-agent decision logic. The detailed city-growth and migration-pressure contract belongs in [`docs/demand.md`](demand_history_2026-10-10.md).

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

- [`docs/demand.md`](demand_history_2026-10-10.md) decides whether a building is under enough sustained pressure to be
  eligible for upgrade or downgrade
- [`docs/zoning.md`](../zoning.md) decides which next level is legal inside the current
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
  - zoning says the next family level is legal
  - `residential_occupancy_ratio >= residential_min_occupancy_ratio_for_upgrade[N + 1]`
  - every occupied household in that building satisfies the target level's move-in affordability
    rule
    for the required sustained period
- a residential building is eligible to downgrade if:
  - its lost household capacity fits the unreserved ready-housing surplus after workplace claims
  - it preserves the vacancy reserve and homes needed by unhoused households
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

working_capital_assets = cash + usable_inventory_value + paid_compatible_cargo
```

Deterministic `v0.1` non-residential rule:

- a non-residential building is eligible to upgrade from `level = N` to `level = N + 1` only if:
  - zoning says the next family level is legal
  - target recurring income covers its operating costs, including retained staff and target-level tax
  - cash, usable stock and paid cargo cover the target's working-capital requirement; cargo counts
    only when the recipe is unchanged
  - extra workers and customers can be reserved alongside other selected projects; unfinished homes
    cannot staff an immediate upgrade
  - the building has no critical unresolved utility failure
  - the building has no critical unresolved staffing or required-input shortfall
- a non-residential building is eligible to downgrade only if it repairs an unsupported operation:
  - selection requires the current operation to fail its recurring-income or
    working-capital check and the replacement to pass both
  - the replacement retains current staff and reserves its customers and any extra ready workers
    alongside other proposals; rejection preserves those claims and pacing credit
  - cash, usable stock and same-recipe paid cargo count together, so ordinary purchases or low
    stock alone cannot downsize a supportable operation

Interpretation:

- high city demand alone does not force a shop or factory to level up
- the business must also be able to support the more expensive tier

#### Zone-type viability summary

The same layered rule applies to every ordinary private zone:

- demand selects viable projects and reserves their workers, housing and customers
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
  and `residential_min_occupancy_ratio_for_upgrade` belong to economy-owned tuning data, not to zoning
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

## Authoring Boundaries

Economy authoring is not part of gameplay. Players do not wire production chains or change
balancing variables from the live game UI.

The runtime game still exposes read-only economy inspection:

- inventory and shortage overlays
- route and shipment debugging
- player policy summary
- building-level throughput, staffing, and utility-service inspectors

### Control boundaries

To keep authoring and gameplay cleanly separated, the economy should use three distinct control layers.

- `Developer authoring layer`: economy profiles, recipes, default coefficients, and allowed policy ranges in `economy/profiles.toml`.
- `Simulation-owned outcomes`: wages and labor prices, staffing pressure, production throughput, utility-service availability, household demand, and delivery cost outcomes. These are calculated by the simulation and are not direct player inputs.
- `Player policy layer`: curated fiscal or trade levers such as income tax, property tax, real estate tax, `VAT`, tariffs, and subsidies. Players change bounded values or presets in gameplay UI, not balancing formulas.

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

### Economy Profiles File

`economy/profiles.toml` defines the reusable economy profiles, resources, and runtime tuning. It is
edited by hand and loaded once at startup by `load_runtime_economy_catalog`, which validates the
runtime tuning and compiles the profiles into the runtime catalog. Load fails on invalid data
instead of falling back to defaults.

Profile rates, wages and stock settings must be finite and nonnegative. Startup treasury must be
finite, and carrier prices finite and nonnegative. Resource and profile IDs
must be unique; resource `runtime_id` values are persisted in saves and are never renumbered.

Renaming or deleting a profile that assets still reference leaves those buildings
`economy-broken` (see below); nothing remaps them silently.

#### Business Solvency Requirement

A supply chain must cover business costs as well as move goods. The existing `ECON-11` paper
check evaluates full staffing, maximum authored wages, all output sold at catalog price, and
inputs bought locally whenever their producing profile exists (otherwise imported):

```text
daily_profit = daily_revenue - (worker_capacity * wage_max_currency_per_day + daily_input_cost)
```

The Rust test `shipped_businesses_are_solvent_at_full_staffing` checks this on the shipped file.
It is a necessary baseline, not runtime solvency acceptance: it omits overhead, weak sales,
partial staffing, delivery/payment timing, and imported inputs when a local producing profile
exists but no local supplier operates. `ECON-14` extends acceptance to the supported startup and
steady-state cases defined in the [economic ledger](#economic-model-to-define-before-tuning).
The separate export target still fails; see [`ECON-11` balance tests](#econ-11-balance-tests).

### Runtime Simulation

The runtime consumes the compiled economy definitions and simulates:

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
2. `economy/profiles.toml` defines the reusable economy profiles that those references point to.
3. The runtime resolves the profile reference and combines the placed asset with the referenced economy profile to create the building's economic behavior.

Example:

- an asset with id `base:building.industrial.food_processor_small` declares base metadata such as `worker_capacity`
- the same asset stores `economy_profile = "food_processor_basic"`
- `economy/profiles.toml` defines a profile such as `food_processor_basic`
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
- both cases must be visible in the asset editor, runtime diagnostics, and save-load warnings

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

### Resource units and freight payload

**Implemented 2026-10-06.** Canonical quantities are explicit in `economy/profiles.toml` and
compiled once into the runtime catalog. Recipe ports cache their resource's delivery batch.
Inventories, recipe rates, deposits, shipments, resource-specific demand and prices use that
resource's canonical measure. Names ending in `units` remain generic schema fields, not an
additional conversion layer. The inspector displays units for stock, inputs, household supplies,
coal reserves and electricity; the utility overview displays kWh/MWh and kg.

The conventions follow [BIPM's SI Brochure](https://www.bipm.org/en/publications/si-brochure):
mass is kg, volume m³ and energy J; electricity is accounted in kWh. A tonne (`t`) is 1,000 kg;
1 kWh is 3.6 MJ (3,600,000 J). Electricity consumed over a day is energy, not instantaneous power in watts.
Area remains m², with hectares (10,000 m²) in site authoring and UI. Production rates are per
**operational game day**; the compressed wall-clock duration does not change cargo mass or
multiply a daily recipe by real-world seconds. Service equivalents are counts, not SI quantities.

| Resource / service | Canonical quantity | Meaning | Minimum freight batch |
| --- | --- | --- | ---: |
| `grain` | kg | Harvested grain | 50 kg |
| `coal` | kg | Extracted fuel; reserve/depletion use the same mass | 40 kg |
| `packaged_food` | kg | Food mass supplied to retailers | 50 kg |
| `household_supplies` | kg | Retail supply basket; 1 kg consumed per resident/day | 40 kg |
| `machinery` | kg | Equipment and spare-parts material, not a count of machines | 40 kg |
| `steel` | kg | Imported steel; no local producer profile yet | 100 kg |
| `metals` | kg | Imported non-steel metals; no local producer profile yet | 100 kg |
| `personal_services` | service equivalent | Completed aggregate personal-service demand | None; no freight/storage |
| `health_essentials` | service equivalent | Completed aggregate pharmacy-style service demand | None; no medicine cargo yet |
| Electricity (`power`) | kWh | Energy produced, supplied and consumed through the utility layer | None; network service |
| Water / sewage | m³ | Supplied water / treated wastewater | None; network service |

All currently authored building outputs, at full operation before staffing, input, stock,
deposit and customer limits:

| Building/profile | Material inputs per day | Output per day |
| --- | --- | --- |
| Grain farm | 0.1 kg machinery per ha | 20 kg grain per ha |
| Coal mine | 4 kg machinery per ha | 9,600 kg coal per ha, removed from the deposit |
| Food processor | 200 kg grain + 2 kg machinery | 200 kg packaged food |
| Grocery | 200 kg packaged food | 200 kg household supplies |
| Machinery factory | 30 kg steel + 10 kg metals + 2 kg own machinery | 42 kg machinery gross; 40 kg saleable after self-upkeep |
| Barber / personal service | No authored material input | 80 service equivalents |
| Pharmacy / health essentials | No authored material input | 120 service equivalents |
| Coal power plant | Up to 10,800 kg coal + 4 kg staffed machinery upkeep | Up to 24,000 kWh at full 20-worker staffing; demand-limited |
| Water plant | 0.5 kg machinery | No authored throughput limit yet; provider presence enables local water coverage |
| Wastewater treatment | 0.5 kg machinery | No authored throughput limit yet; provider presence enables local sewage coverage |
| Residential / farm household | 1 kg supplies per resident, plus aggregate service demand | No saleable output; households consume and supply labor |
| Office or other asset without a supported economy profile | No authored recipe | No defined saleable output; job slots alone do not define a funded activity |

Household service demand remains 0.10 personal and 0.16 health equivalents per resident/day.
An equivalent is economic service capacity, not a promise that each settlement produces one
visible visit. Electricity demand now follows the profile table at the top of this document.
Private workplaces retain 1 m³ water and 1 m³ sewage/day, plus per-ha maintenance of
`[1 m³ water, 0 m³ sewage]` for grain farms and `[10 m³ water, 10 m³ sewage]` for coal mines.
Household water/sewage retains a 2 currency/resident/day allowance; electricity is physically
metered and billed separately. Water/sewage volume labels do not imply that
their existing provider-presence model now simulates pipes or measured production capacity.

**Initial unit conversion (before the production/freight revision).** Old food/grain bundles become 1.25 kg and old steel/metals
bundles 2.5 kg; their prices divide by those factors. This fixes the old 160-in/200-out retail
mass creation and 16-in/40-net-out machinery recipe while preserving full-cycle monetary flows.
Grain is 4.8 currency/kg, packaged food 12, steel 3.2, metals 6.4; household supplies remain 25,
coal 8, machinery 20. The later production/freight revision reduces farm yield from 362.5 to
20 kg/ha/day and removes regional buying caps; other listed recipe prices remain unchanged.

Prices and industrial energy budgets remain game-balance values. Grain output is now an
annualized agricultural yield: a full-output 100 ha farm produces 2,000 kg/day with six workers.
That mass fits in one partial 20 t load; dispatch uses accumulated production as described below.
The previous "29,000 units / 40-unit truck = 725 trips" inference was wrong: the old 40 setting
rounded shipment quantities and did not cap payload.

**Payload contract.** `runtime_tuning.logistics.truck_payload_kg = 20000` caps one shipment's
mass for local delivery, import and export. `shipment_batch_kg` is a resource-specific minimum
and rounding quantum. A 400 kg order can use one truck; it need not wait for 20 t. Quantization
rounds down the minimum of requested stock, available stock, affordability and payload. Excess
stock stays at source and unmet demand is reconsidered by later existing passes. Each accepted
job retains one carrier, one payment and the existing reservation/refund/delivery lifecycle.
No list of trips is expanded during planning. The added limit is O(1) arithmetic per candidate,
with no new allocation or route query. This is a mass limit; vehicle volume, cargo density,
cargo-specific tariffs and heterogeneous vehicle capacities are not modeled. Carrier pricing is
defined in the production/freight contract below.

Catalog validation rejects missing/invalid quantity units, nonphysical freight resources,
nonpositive/oversized batches, inconsistent utility units and processing/retail recipes that
create net material mass. Services cannot be ordered as freight. Save format **71** persists
carrier quotes and omits the obsolete quota ledger. Pre-71 jobs retain their original zero-fee
contracts; pre-70 inventories and shipment cargo convert once by stable resource ID. Household
stocks, coal reserves, money and reserved payments keep their existing numeric values. New
saves do not convert again. Loading an old shipment exceeding the new payload fails explicitly;
it is not silently truncated, delivered or split into duplicate payments/carriers.

### Production and freight economics (`ECON-14`)

**2026-10-06, following the unit/payload pass.** Fixed outside buying quotas are removed from
export planning, producer staffing and factory demand. A connected outside gateway supplies an
export opportunity at the catalog bid; actual sales still require physical delivery. No quota
vector, refill pass or saved saturation state remains.

Each accepted contract stores a carrier quote:

```text
round_trip_vehicle_hours = cached_one_way_movement_seconds * 48 / seconds_per_day
freight_cost = dispatch_fee_currency + round_trip_vehicle_hours * vehicle_hour_cost_currency
```

Both carrier rates are currently 4 currency. This is O(1) arithmetic using the route result
already required by dispatch, with no new routing query, spatial index, trip list or per-tick
repricing. A 0.5 h one-way journey costs 8; a 4 h journey costs 36. The same fee on a partial
load naturally costs more per kg. Queue and preferred-window waiting do not add vehicle-time
fees; congestion affects the quote only through the existing cached estimate. Goods retain
existing preferred-window pricing. The return allowance is a symmetric estimate using the
same cached route cost as existing export ETA calculation.

Local/import buyers reserve goods plus freight only if the rounded cargo fits their budget.
On local delivery, the seller receives the goods component; the carrier fee leaves the local
economy. Exports record gross sales and credit cash less freight; dispatch requires gross sales
to exceed freight. Failed input contracts refund the whole reservation, including freight;
failed exports earn nothing and pay no fee. Returning carriers cannot settle again. This is a
carrier contract paid on successful delivery, including failed-delivery insurance, not a fuel
simulation. Save formats 71 and later preserve the quote. Earlier accepted jobs keep zero carrier fees.

The physical run also exposed excessive small deliveries: food factories repeatedly topped up
grain and exported 50 kg whenever the hourly check found another minimum batch. Four factories
could create roughly 32 daily import/export loads for only 800 kg of output. The revised producer
profiles reorder inputs below two days of stock toward four days, retaining the existing
minimum two-batch input reserve. Scheduled exports accumulate one production day's net output
above their separate local buffer, rounded up to a legal batch and capped at the 20 t payload.
For a fully staffed food factory this targets a 400 kg grain order about every two days and a
200 kg export about once per day. These are accumulation thresholds, not a fixed daily trip
quota: delayed stock or output exceeding payload can need additional loads. The threshold is
O(1) arithmetic before route lookup; hourly checks do not generate trucks for tiny top-ups.

The two-year expansion experiment exposed an employment inconsistency: demand reserves open
jobs for unemployed adults, but the job score previously let household savings suppress every
job offer for those same adults. Unemployed working-age residents now accept the best reachable,
payroll-funded offer regardless of savings or pantry fullness. Existing workers keep the
current score, lock and better-job rules. This changes one predicate in the existing bounded
search, with no extra scan, route query or allocation. It aligns actual hiring with the labor
pool already used by admission.

Outside buyers never become an infinite factory-demand quantity. Existing local capacity still
reserves local customers. An export-led opening is limited to one while previous zoned industry
is unfinished, idle or has not recovered its startup cash; live industry must also have
paid revenue. Bulk-input payment days do not erase recovered reserves. Candidate viability includes full payroll,
imported inputs, utility/property bills and minimum dispatch cost. Actual route costs then
matter through cash recovery. This is a bounded expansion rule, not a private investment market:
new sites still receive the existing explicit owner-equity grant. Broader demand simplification
and investor financing remain separate work. See [`demand.md`](demand_history_2026-10-10.md).

The producer worksheet uses full staffing, paid export sales, imported inputs/utilities,
level-one industrial property tax of 35/day and 10% tax on positive steady cash margins. It
uses the authored input reorder lots and daily export accumulation to estimate average
loads/day; real delivery timing and queues require the physical run. Construction includes the fixture's 22,500 farm / 50,000 mine
footprint plus 20,000 / 25,000 per hectare. Startup cash is additional owner equity and is **not**
included in the construction payback column. Delivery delays, labor ramp-up, partial operation
and exhaustion require the separate physical runs.

| Producer | ha | Workers | kg/day | Imported inputs/day | Payroll/day | Imported utilities/day | Investment | Freight/day, 0.5 h | After-tax margin/day | Construction payback, days |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Farm | 1 | 2 | 20 | 2.50 | 180 | 18.12 | 42,500 | 3.22 | −142.84 | Never |
| Farm | 4 | 2 | 80 | 10 | 180 | 48.12 | 102,500 | 6.48 | 93.96 | 1,091 |
| Farm | 20 | 2 | 400 | 50 | 180 | 208.12 | 422,500 | 8.40 | 1,294.63 | 326 |
| Farm | 100 | 6 | 2,000 | 250 | 540 | 1,008.12 | 2,022,500 | 10.00 | 6,981.19 | 290 |
| Mine | 1 | 5 | 120 | 100 | 450 | 201.88 | 75,000 | 8.80 | 147.89 | 507 |
| Mine | 4 | 20 | 480 | 400 | 1,800 | 783.12 | 150,000 | 11.20 | 729.61 | 206 |
| Mine | 20 | 100 | 2,400 | 2,000 | 9,000 | 3,883.12 | 550,000 | 12.00 | 3,842.89 | 143 |
| Mine | 100 | 500 | 12,000 | 10,000 | 45,000 | 19,383.12 | 2,550,000 | 12.00 | 19,412.89 | 131 |

The intermediate 200 kg/ha/day, two-workers/ha tuning is rejected: it implied 73 tonnes/ha/year
and 200 workers on a 100 ha cereal farm. The replacement uses 20 kg/ha/day, or 7.3 tonnes/ha/year,
spread evenly across simulation days. For comparison, England's 2024/25 Farm Business Survey
reports 7.5 tonnes/ha of wheat on its cereal farms; the game baseline is a tuning choice, not a
claim that every climate has that yield. [Primary yield reference, page 10](https://farmbusinesssurvey.co.uk/wp-content/uploads/2026/04/The-Field-Book-Key-Results-24-25.pdf).

Grain staffing is `min(6, max(2, ceil(hectares / 10)))` for a positive committed area; uncommitted
sites retain zero jobs. The profile's optional `area_worker_capacity_max` is shared by physical
jobs, hiring/production equivalents and payroll startup runway. Mines and other crops do not
inherit the grain ceiling. This adds O(1) arithmetic without allocation or a new scan. Larger
grain fields still pay 20,000/ha construction, 0.1 kg/ha/day machinery upkeep, 2 MJ/ha/day
electricity and 1 m³/ha/day supplemental water. Irrigation does not add sewer demand; the farm
building retains its base utility demand. These upkeep quantities are explicit game assumptions,
not measured agricultural input budgets. Seasonal harvest/hiring and fuel/fertilizer recipes are
not modeled. Large fields buy more capital and upkeep even after staffing reaches six.

Both small and large farms should remain available. A **4 ha grain farm** is the placement
minimum: two workers and a positive full-operation margin on both route assumptions. A 1 ha
cereal plot cannot fund two full-time wages and is shown as a loss-making comparison, not an
accepted starter business. Future vegetable/greenhouse profiles may support smaller plots; they
are not implemented by this change. The profile's `minimum_field_area_m2` is checked by field
preview, commit and resize before land reservations or charges change. The polygon tool displays
the minimum when the proposed area is too small. Existing saved geometry is not altered on load.

The closed household-consumption comparison ledger now gives **144.50 jobs/1,000 residents**,
excluding public services and exports. It assumes repeatable 20 ha grain farms; subdivision
changes staffing under the floor/ceiling, so this is not a universal city employment prediction.

Longer routes matter most to small loads: at 4 h one-way, the 4 ha farm margin falls to
73.54/day, with construction payback of 1,394 days. The 1 ha mine margin falls to 120.17/day
and payback to 624 days. At current richness a full
coal deposit holds 60,000 kg/ha, or 500 full-production days. A 1 ha mine therefore earns an
operating surplus but cannot repay construction before exhaustion even on the near route.
The 4 ha mine's near-route payback fits that resource life. This remains an investment warning,
not a claim that every positive operating margin is a sensible placement.

Fresh physical and performance validation follows. No previous capped-market timing validates this
build.

**Corrected farm/minimum-area validation, 2026-10-06.** Fresh release checks pass: **395 tests**
across economy, work-area staffing, agriculture, saves, field preview/edit/load/undo, diagnostics
and producer cash flow (32 manual checks ignored). The new minimum regression checks rejected
preview/commit/resize, unchanged budgets/geometry and acceptance at exactly 40,000 m². Tests also
cover the six-worker ceiling through hiring, payroll, production and reconstructed area state.
Test build uses eight Rayon workers and `-C target-cpu=x86-64-v2`; performance comparisons below
explicitly match baseline flags.

Fresh physical experiment: `RAYON_NUM_THREADS=8 METRUM_BALANCE_DAYS=730
METRUM_BALANCE_REPORT_DIR=$ARTIFACTS/metrum-farm-minimum-producers-730 cargo test --release
--lib producer_balance_matrix -- --ignored --nocapture`. The actual run used the compiled
release test executable with these same arguments and environment. Each case receives its
**one-time declared city construction endowment before observation**. Initial private capital is
recorded separately; no later producer capital is added, and operating changes exclude startup
cash.

| Grain farm | Physical jobs | Operating cash change, 730 days | Final 90 days | Farm bankruptcies | City residents at end | City empty-household-days, year 2 / final 90 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 4 ha | 2 | +65,880 | +8,342 | 0 | 165 | 24 / 0 |
| 10 ha | 2 | +401,975 | +49,325 | 0 | 64 | 8 / 0 |
| 20 ha | 2 | +951,925 | +117,566 | 0 | 35 | 0 / 0 |
| 100 ha | 6 | +5,071,787 | +633,215 | 0 | 74 | 365 / 90 |

All four grain farms replenish paid inputs, pay workers and retain positive recurring cash
without bankruptcy or replacement capital. **This validates farm survival, not healthy-city
acceptance.** The village variants still expose household admission/replenishment failures;
the 100 ha case has one continuously short household despite wage/transfer income. The 4 ha
case clears shortages in its final quarter but has 24 empty-household-days during year two.

The same matrix retains finite-ore evidence: 0.25 ha and 100 ha village-start mines fail;
1/4/20 ha mines do not go bankrupt in two years. The 1/4 ha mines exhaust their deposits and
each spends 3,881 in its final quarter without sales; the 20 ha mine still earns 76,943 in that
quarter. Mines remain five workers/ha, so the grain ceiling does not disguise an oversized
mine's labor and maintenance requirements. Depletion and household failures remain visible.

**Matched 10k performance.** Same HEAD `c51973df97a94fa9c9e7b2182be223dec9a512fd` plus
the recorded dirty changes; eight Rayon workers, release/bench profile, no profiling or
concurrent CPU jobs, and **empty Rust flags on both benchmark builds**. The ordinary local
test build defaults to `-C target-cpu=x86-64-v2`, so the after benchmark was explicitly rebuilt
with `RUSTFLAGS='' CARGO_ENCODED_RUSTFLAGS='' cargo bench --bench economy_tick_benchmark --no-run`
to match the saved before binary. Run filter `/10k` only; no larger city tier was executed.

| Measurement | Before | After | Criterion result |
| --- | ---: | ---: | --- |
| Setup, untimed | 15.7 s | 16.0 s | Separate from tick samples |
| Operational hour, mean [95% CI] | 2.7085 [2.4524, 2.9834] ms | 2.6042 [2.3991, 2.8281] ms | No significant change, p=0.56 |
| Daily settlement, mean [95% CI] | 808.92 [607.64, 982.74] ms | 849.67 [641.70, 1,035.8] ms | No significant change, p=0.79 |

The fixture matches at 11,120 residents, 5,597 households, 7,619 employed, 13,881 jobs and 7,396
buildings both after setup and after day 15. It uses ample city funds and stationary agents;
these are economy timing results, not growth or household-supply acceptance. Daily intervals
remain broad; no speedup or tight regression bound is claimed. The isolated 10k-query staffing
check also passes (21 samples; density 3.60 ns/query, capped grain 1.99 ns/query), with no
allocation in the calculation. That microbenchmark is not a city timing comparison.

Benchmark SHA-256 before `c7331b86c41889071edd35c3420d3466d95bf1cb4180cfac1017043d14f85f6d`; after
`95d6f45c7c885986a5df9d39ab1b340db31eacbcb1f2970d061676971c45ef7b`. The corrected extension is
deployed to `godot/bin/libmetrum_rise.so`; isolated headless `field_edit_tool_test.gd` and
`economy_machinery_test.gd` both pass, including the minimum-area error during drawing. `cargo doc
--no-deps` completes with no missing-doc warnings; its existing unrelated `VegetationGenerator`
link warning remains. Targeted source format checks and `git diff --check` pass; unrelated
pre-existing formatter differences in asset-authoring code were left untouched.

**Shopping delivery acknowledgement repair, 2026-10-06.** The physical 100 ha farm trace
found a paid shopper arriving home at 04:24, then leaving for work in the next frame before
the 05:00 household pass could credit the cargo. Repeated timeouts discarded purchases despite
sufficient recurring income. Both legs now retain shopping activity until the household observes
pickup/delivery; work scheduling skips that owned task, and fulfillment clears stale return
plans before restoring home activity. Cancellation still releases the carrier. This adds only
O(1) activity/cache checks to existing movement, with no route query, allocation or new scan.
Travel-aware cached departure minutes remain authoritative; they are not rounded to hours.

Fresh release tests: **343 economy tests pass**, including due-departure suppression at home
and work, multi-frame arrival purpose, delivery accounting and the existing commute-clock
regressions. The physical 730-day matrix completes and reconciles accounting. Producer
initial capital remains separate from sales.

| Grain farm | Operating cash change, 730 days | Final 90 days | Residents at end | City empty-household-days, year 2 / final 90 |
| --- | ---: | ---: | ---: | ---: |
| 4 ha | +65,813 | +8,344 | 164 | 42 / 0 |
| 10 ha | +401,776 | +49,325 | 98 | 0 / 0 |
| 20 ha | +951,662 | +117,566 | 35 | 0 / 0 |
| 100 ha | +5,109,725 | +635,325 | 118 | 0 / 0 |

All four farms survive with no bankruptcy or replacement startup cash. The 100 ha town's
continuous shortage is resolved, but the 4 ha town still has second-year shortages before its
final quarter. Mines at 1/4/20 ha survive and now all exhaust their deposits by the final quarter;
0.25/100 ha village-start mines fail. Final-quarter household shortages remain in the 4/20/100 ha
mine towns (19/18/60 empty-household-days).

The ordinary village still fails healthy-city acceptance: **243 second-year empty-household-days**,
60 in the final quarter, with 98 residents and 72 employed at the end. Every business-profile
survival/cash assertion passes before that failure; concurrent-year determinism and observer
neutrality also pass. The final short household has an unemployed adult and elder, no home, and
pension income insufficient for two people's supplies. Admission still validates one preview for an
entire batch, does not reserve funded work for incoming carriers, and recomputes household
composition after arrival. Those were separate admission/lifecycle defects under `DEM-02`; this
delivery fix did not establish healthy-city acceptance.

Fresh performance for this repair uses eight Rayon workers, empty Rust flags, sequential
unprofiled release binaries and no concurrent CPU jobs. The matched 10k economy fixture is
unchanged (11,120 residents, 5,597 households, 7,619 employed, 13,881 jobs, 7,396 buildings and
matching treasury at setup/day 15). Setup is outside timing: 16.0 s before, 16.1 s after.

| Measurement | Before mean [95% CI] | After mean [95% CI] | Result |
| --- | ---: | ---: | --- |
| Operational hour | 2.7449 [2.5725, 2.9295] ms | 2.6602 [2.4675, 2.8666] ms | No significant change, p=0.55 |
| Daily settlement | 843.63 [636.41, 1,024.7] ms | 839.10 [632.74, 1,019.8] ms | No significant change, p=0.98 |

The economy fixture does not move agents. A separate new `agent_benchmark` workload measures
the complete tick for 10,000 indoor adults with valid homes/jobs and a cached future departure:
**76.994 µs [76.420, 77.511]**; the corresponding shopping-owned workload is
**73.868 µs [73.236, 74.538]**. Both assert zero path searches and no departures. These are
absolute isolated costs, not before/after speedups, active traffic costs or the cost of initial
route planning. They do not justify rounding departures to hours. All benchmark filters stay
at 10k; no larger tier runs.

```bash
RUSTFLAGS='' CARGO_ENCODED_RUSTFLAGS='' cargo bench --manifest-path rust/Cargo.toml \
  --bench agent_benchmark --bench economy_tick_benchmark --no-run
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-shopping-perf \
  $ARTIFACTS/metrum-shopping-before-bench /10k --save-baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-shopping-perf \
  $BENCH_BINARY /10k --baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-shopping-perf \
  $BENCH_BINARY \
  'indoor_(schedule|shopping)/10000$' --bench
```

Economy binary SHA-256 before `95d6f45c7c885986a5df9d39ab1b340db31eacbcb1f2970d061676971c45ef7b`,
after `80e7ea40efbfa5c94860c97ec3320cd9abe9c2b6f84804c51c02906a2fb1b2ca`; indoor benchmark
`f8bfae54bb5f14f939b3307f119e59545b2f83355ec62bc0f0779fee0e61a1aa`. Catalog is unchanged between
these matched runs: `a30680e6f8cde47bc36db89e704e9946cb189610564145cf5eb48205e4cad2f7`.

**Admission and continued growth repair, 2026-10-06.** The admission batch now consumes finite
recurring-income capacity per actual family, reserves adults and transfer claims while carriers
travel, and preserves approved composition through arrival ordering and save/load (format 72).
This adds O(1) arithmetic per bounded candidate and per pending carrier in the existing scan,
with two u16 fields per agent and no new route search. It reserves aggregate wage opportunities;
ordinary reachable-work search still chooses an employer. See [the admission contract](demand_history_2026-10-10.md).

The first reservation run exposed two further growth stalls. Bulk-input purchases put otherwise
profitable factories on alternating cash-negative days, so export expansion now requires earned
cash above startup capital and paid revenue without requiring every factory to report a positive
cash day simultaneously. Secondly, selecting the next family from accepted-household count made
one unaffordable applicant recur indefinitely. Outside candidates now advance with simulation
hours; the same seed feeds preview and launch, and load derives it from the saved clock. Refused
families still fail the same affordability test. Cached commute-aware departure timing is unchanged.

Fresh physical village validation passes **all three integration tests**, including concurrent-year
determinism and observer neutrality. The two-year town grows from **114 residents / 88 employed**
at year one to **156 / 119**, with 83 households at the end. Grocery, food processor, machinery
factory, barber and pharmacy profiles all have zero bankruptcies and zero replacement startup cash.
All have positive operating cash in year two and the final quarter. There are **zero
empty-household-days in year two** and zero unpaid eligible transfers. City recurring surplus is
**+787,656 in year two**, **+205,124 in the final quarter**; the treasury stays positive. Private
construction equity is recorded separately from operating cash.

| Business profile | Year-two operating cash | Final 90 days |
| --- | ---: | ---: |
| Food processor | +339,570 | +89,693 |
| Grocery | +244,438 | +63,754 |
| Machinery factory | +255,672 | +71,549 |
| Personal service / barber | +35,533 | +9,999 |
| Health essentials / pharmacy | +44,872 | +12,048 |

The fresh 730-day producer matrix completes and reconciles accounting for all nine cases. Every
grain farm survives with no bankruptcy or replacement capital, and every farm town has zero
empty-household-days in year two and the final quarter. City final-quarter recurring finances are
positive in all nine scenarios. Explicit producer construction is charged before observation; the
diagnostic supplies that one-time infrastructure endowment and no later rescue capital.

| Grain farm | Operating cash, 730 days | Final 90 days | Residents / employed at end |
| --- | ---: | ---: | ---: |
| 4 ha | +65,765 | +8,318 | 162 / 118 |
| 10 ha | +401,787 | +49,328 | 156 / 117 |
| 20 ha | +953,222 | +117,969 | 155 / 119 |
| 100 ha | +5,108,046 | +635,613 | 163 / 121 |

Mines at 1/4/20 ha survive, earning +67,452 / +381,563 / +1,688,762 across the full run, but exhaust
their deposits and each spend 3,881 on upkeep in the final quarter without producing. The 0.25 ha
mine loses 3,735 and the 100 ha village-start mine loses 494,458; each fails once. Their towns, and
the 1/4 ha mine towns, have no second-year empty-household-days. **The 20 ha mine town remains a
depletion-transition failure:** 182 empty-household-days in year two, 74 in the final quarter;
at the end 40 adults are unemployed, 27 have exhausted temporary support, and 155 residents have
80 employed. Grocery inventory exists, while several pharmacies lose money after contraction.
This is not healthy-city acceptance for that scenario. Re-employment and contraction after a large
finite-resource closure remain `ECON-14` / `DEM-02` work; unlimited outside buyers do not replace
an exhausted deposit or guarantee jobs. The passing matrix test asserts reconciliation, not
universal solvency or household supply.

Fresh targeted release validation: **392 tests pass**, covering economy, save/load, allocator
lifecycle and field placement. New regressions cover finite batch claims, pending worker/benefit
reservations, approved composition through reversed arrival order and save migration, continued
applicants after rejection, and export expansion across bulk-input cash days. `cargo doc
--no-deps` completes with no missing-doc warnings; its unrelated existing `VegetationGenerator`
link warning remains.

Fresh performance uses sequential unprofiled release binaries, empty `RUSTFLAGS` and
`CARGO_ENCODED_RUSTFLAGS`, and eight Rayon workers. Unrestricted scheduling on this mixed-core
machine gave 2.5450 → 2.9052 ms for the hourly pass (p=0.02), and the reverse-order pair gave
2.1262 → 2.4170 ms (about 14% in each comparison). Pinning both builds to the same eight physical
performance cores removes that difference. The fixed-affinity comparison below is the controlled
result; the earlier measurements remain recorded, not discarded as passing checks.

| 10k measurement, pinned CPUs `0,2,4,6,8,10,12,14` | Before mean [95% CI] | After mean [95% CI] |
| --- | ---: | ---: |
| Operational hour | 2.0863 [2.0382, 2.1360] ms | 2.0502 [1.9965, 2.1090] ms |
| Daily settlement | 837.04 [630.74, 1,017.5] ms | 809.85 [606.51, 989.00] ms |

No significant change is detected (hour p=0.36, daily p=0.86); daily intervals remain broad and
no speedup is claimed. Setup stays outside timing: 15.5 → 15.4 s. Both runs contain 11,120 residents,
5,597 households, 13,881 jobs and 7,396 buildings at setup and day 15. The changed admission behavior
leaves **one additional employed resident** (7,619 → 7,620), with correspondingly different tax
receipts. Inputs/configuration match, but the resulting records are not identical. This static
fixture neither moves traffic nor establishes healthy household supplies.

The rebuilt indoor-agent benchmark measures the complete cached-schedule tick at **76.210 µs
[75.739, 76.695]** for 10,000 adults; shopping-owned agents take **73.786 µs [72.890, 74.548]**.
Both assert zero path searches and no departures. These are isolated absolute measurements,
not full-traffic frame costs or route-planning timings. They preserve minute-accurate departures.
All performance filters remain at 10k; no larger tier was run.

```bash
RUSTFLAGS='' CARGO_ENCODED_RUSTFLAGS='' cargo bench --manifest-path rust/Cargo.toml \
  --bench economy_tick_benchmark --bench agent_benchmark --no-run
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-applicants-perf-pinned \
  taskset -c 0,2,4,6,8,10,12,14 $ARTIFACTS/metrum-admission-before-bench \
  /10k --save-baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-applicants-perf-pinned \
  taskset -c 0,2,4,6,8,10,12,14 \
  $BENCH_BINARY \
  /10k --baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-applicants-perf \
  $BENCH_BINARY \
  'indoor_(schedule|shopping)/10000$' --bench
```

Build base is `c51973df97a94fa9c9e7b2182be223dec9a512fd` plus the recorded dirty-tree patch.
Before binary SHA-256 `80e7ea40efbfa5c94860c97ec3320cd9abe9c2b6f84804c51c02906a2fb1b2ca`; after
`5f48f9f9743a3164ea8af8c67b383a44db80ad62f56b0af251c4120dc86f1f41`; indoor
`ee88a68aa8cbd71f5ebf6736a77507f8add5674b55158eb75905f9477cd0a74a`. Catalog hash stays
`a30680e6f8cde47bc36db89e704e9946cb189610564145cf5eb48205e4cad2f7`. The before sources are the
shopping-repair artifacts listed above. Rust checks are fresh for this repair; Godot UI checks and
deployment described earlier belong to their earlier builds.

### 1. Resource Types

The current catalog is listed above. Potential categories, including not-yet-implemented goods:

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
- `v0.1` utility service is not a detailed line-by-line grid simulation; `power` now dispatches against hourly kWh demand while `water` and `sewage` still use the starter provider-present fallback model
- utility availability resolves independently per service; a local `power` provider does not satisfy `water` or `sewage`
- each utility service settles through the same bill formula:
  `local_charge = demand_units * local_unit_price * local_coverage` and
  `owa_charge = demand_units * local_unit_price * owa_import_price_multiplier * missing_coverage`
- electricity follows each profile's idle/activity/residential/area rates; workplaces retain 1 m³ water and 1 m³ sewage/day, plus authored field/mine area upkeep
- household electricity delivery and payments are separate: unpaid local/import bills carry forward and reduce housing reserves. The water/sewage allowance is split between those two ledger buckets
- connected plants generate the lesser of hourly demand, staffed kW capacity and available coal; maintenance follows staffing while coal follows actual generation. Metered local/import shares determine daily bills
- city service funding policies are runtime simulation state owned by Rust; the live electricity funding policy sets the default funded worker slots for city-owned power plants, and production follows the resulting staffed workers plus fuel/input availability
- live city and per-building funding controls reject NaN/infinite values without changing policy or staffing; finite values retain the existing `[0, 1]` slider clamp, and per-building controls reject non-finite pick coordinates
- individual city-owned power plants may carry a per-building funding override; citywide electricity funding changes do not clear those plant overrides
- if a valid connected local `water` producer or `sewage` processor exists and has positive current operational throughput, that service is treated as locally available to eligible consumers in `v0.1`
- if no valid connected local utility producer or processor exists for a service, that service falls back to `OWA` independently of the other utility services
- the downstream production formula still does not use a utility throughput gate in `v0.1`; utility failures are represented as local service coverage and external fallback cost
- `power_plant_basic` is coal-fueled in the starter runtime: it requests `coal` through ordinary freight logistics, can import coal from `OWA` while no local coal mine is producing reachable coal, and produces no local `power` when staffed but out of coal
- authored coal deposits can now be painted in WorldEditor; explicit coal-mine assets bind to `coal_mine_basic`, commit a player-drawn extraction polygon within 10 m of the building footprint, snapshot the enclosed reserve, consume that reserve into local `coal` output during hourly operation, persist both deposits and extractor depletion through city saves, and render committed pits through a terrain-shader coal-texture mask rather than a separate decal mesh; the committed area scales physical hourly output and physical worker capacity against a 10,000 m2 authored baseline, scheduled `OWA` exports can back area-scaled active worker slots when a connected outside freight gateway exists, and the first committed extraction area funds the one-time area-scaled payroll/utility runway and opening inputs
- explicit grain farms bind to `grain_farm_basic`, commit a player-drawn field polygon within 10 m of the building footprint, and produce renewable `grain` during hourly operation without consuming a map-authored resource deposit; daily output stays per hectare while worker capacity uses a separate authored staffing area (one worker per 100,000 m2 for grain farms), scheduled `OWA` exports can back area-scaled active worker slots when a connected outside freight gateway exists, and the first committed field funds the one-time area-scaled payroll/utility runway and opening inputs
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

An economy profile is a reusable template authored in `economy/profiles.toml` and referenced by one or more assets.

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
`commercial_consumables` resource becomes worthwhile. A service store's active worker slots follow
its own funded census activity (`DEM-02`, 2026-10-10), as goods stores do: the share of its
output that reachable, affordable households reserved. Hourly service sales remain the city's
resident demand capped by total staffed capacity and credited by capacity share, so that share
now follows the customers each store reserved instead of a citywide demand ratio.

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
- household demand is represented as an abstract `demand_sink` profile rather than as a profile-bound asset

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

### 6. Cross-Cutting Rules

Cross-cutting economic rules such as taxes, transfers, and service funding live in runtime tuning
and live `CityFiscalPolicy` state. Authored "controller" objects were an editor-only concept with
no runtime consumer and were removed with the economy editor.

### 7. Connections

A connection is an authored allowed relationship between node classes and resource types.

In `v0.1`, this section mainly applies to normal shipped economic flows rather than to baseline utilities, which use the separate `Utility Service Layer`.

Important: a connection is usually not a literal per-unit hard route. In most cases it defines:

- permission
- preference
- priority
- filtering
- weight

The runtime then resolves actual suppliers, deliveries, and routes using those authored rules.

This is the key scalability rule. Authored profiles define the economic topology; the simulation executes a compact runtime form of it.

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

The current essential inventory chain is:

- `grain_farm` grows `grain` from a player-drawn field polygon
- `food_processor` converts `grain` into `packaged_food`
- `grocery` converts `packaged_food` into `household_supplies`
- households replenish from groceries in periodic batches rather than per-person daily errands
- `household` consumes `household_supplies`

Machinery upkeep and aggregate service outputs also operate today. A working goods chain alone
does not establish sufficient jobs or household income; expansion follows the healthy-growth plan.

### Standalone Service Commercial

The first standalone service-commercial extension is intentionally shallow:

- `service_store` profiles output demand-facing service capacity, not inventory
- `personal_service_small` outputs `80 personal_services/day` with `4` full-staff worker slots
- `health_essentials_small` outputs `120 health_essentials/day` with `6` full-staff worker slots
- `personal_service_demand` creates `0.10 personal_services/day/resident` (2.00 currency/day)
- `health_essentials_demand` creates `0.16 health_essentials/day/resident` (2.24 currency/day)
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

Working-age residents without a job accept a reachable, payroll-funded offer even when their
household has savings. This matches the available local labor pool used by demand. Existing
workers use decision-utility scoring to compare voluntary job changes; assigned work travel
follows the operational schedule.

Early decision-utility inputs can stay simple:

- current money
- household supplies at home
- commute cost
- job availability

Existing-worker job-switch score:

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
travel cap, farther candidates therefore remain eligible. The worst case is still O(J) over
reachable jobs, but two exact lower bounds keep ordinary searches local (`ECON-13`, 2026-10-02):

- **Ring stop.** The bound for the next 512 m chunk ring adds the home's own door-to-network time
  (local access plus frontage, per mode and node) to straight-line travel at the network's top
  speed. The straight-line part is shortened by the home's node offset and by the largest
  workplace-centre-to-node offset in the job snapshot, and is zero within reach of a shared node
  or frontage.
- **Per-pair pruning.** Once 24 options are held, a workplace is routed only if one of its
  candidates can finish within the 24th option's commute. Each candidate's bound is its exact
  access and frontage terms plus straight-line travel between its two network nodes. Candidates
  are routed in bound order and skipped once their bound loses to the best mode-choice cost found.

Both bounds hold because route cost is never below length ÷ max(speed limit,
`MIN_ROUTE_SPEED_MS`), and no penalty factor is below one. The ranked options, and the commute
seconds of every routed pair, are therefore identical to an exhaustive search.
`commute_bounds.rs` checks this on more than 10,000 home-job pairs. Workplace access terms are
computed once per job snapshot, and home terms once per home.

Network legs are shared across workers and hours (2026-10-10). Door, frontage and live
congestion terms are recomputed for every routed pair, but the road-network leg between its two
lane ends depends only on those lanes and nodes and on the CCH build, metric and lane count. A
car leg is priced with the cost-only CCH query. A walking leg still needs its path, because the
pedestrian connector check reads it. The household system keeps these legs in one table
(`SharedNetworkQuotes`), shared by every job-search worker. The table is emptied when that
revision changes, and dropped daily with the workplace route cache. Commute seconds, rankings
and assignments are unchanged; `commute_bounds.rs` compares bounded estimates that read the table
with exhaustive ones that do not. A home's ranked options are not reused between hours: car
commutes include live frontage delays, and at 100k the set of open employers turns over almost
completely every hour ([measurement](demand_history_2026-10-10.md#growth-redesign-dem-02)).

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

Recommended `v0.1` formula:

```text
throughput = base_rate
           * staffing_factor
           * input_factor
```

Where:

- `base_rate` is the authored full-capacity output rate for the building or recipe
- `staffing_factor = clamp(filled_workers / worker_capacity, 0.0..1.0)`
- explicit field producers and extractors replace `worker_capacity` in this ratio with the
  area-scaled worker capacity; an uncommitted area has zero worker capacity, and 10,000 m2
  receives exactly the authored worker count. Their active slots are that capacity times their
  own funded census activity (`DEM-02`, 2026-10-10): the exports and local buyers they can
  finance. A citywide local-demand ratio, or full activity whenever a border existed, no longer
  applies
- a grocery's `filled_workers` and production are capped by its own funded census activity
  (`DEM-02`): the share of full output that its reachable, affordable household customers and
  staffed business buyers reserved, after financing. Upstream input demand and input inventory
  use that continuous share, retaining the existing minimum shipment buffer; recent bulk
  purchases do not open worker slots. Once the shelf holds the profile's `stock_target_days` of
  funded sales, production also follows that share, so rounding jobs up to whole workers does
  not create surplus output. Below that target the paid staff work at their full rate to refill
  the shelf, because bulk pantry purchases otherwise find a store that only matches its sales
  sold out; this opens no job and stays within the output buffer. Before 2026-10-09 a citywide
  demand-to-output ratio also capped every store. It throttled the stores the census had funded,
  and the 10k fixture produced about a third of what its households consumed
- a store without funded census activity produces nothing; hiring and payroll already follow
  the same census (`ECON-17`), so it holds no paid staff either
- `input_factor` is the limiting required-input coverage for the current production step, clamped
  to `0.0..1.0`; production compares input inventory to the current effective hourly input need,
  not to the full authored daily input for a fully staffed building
- utility costs are paid through the utility/bankruptcy sequence; there is no `utility_factor` term
  in the current `v0.1` throughput multiplier

This keeps the first pass linear and readable. Hard minimum-staff step functions are not part of the baseline formula; if they are ever added later, they should be explicit profile-side rules rather than hidden default behavior.

This gives the player a meaningful connection between zoning, staffing, transit, and output without requiring arbitrary micromanagement.

## Logistics Model

### Shipment units

The simulation should create shipments at the building or terminal level, not one tiny packet per household resident. In `v0.1`, the only terminal-like freight gateways are `OWA` border terminals.

Each shipment should minimally contain:

- stable shipment id
- resource type
- amount in kg (at most one truck payload)
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
per size, measured 65,536 active returns at median `0.335 → 0.399 ms` per logistics pass. This
isolates progression with no arrivals/removals or route planning. The additional 0.064 ms buys
the missing bounded lifecycle; it does not establish cleanup-batch scaling.

Carrier swap repair uses the moved agent's stable shipment ID and binary-searches the existing
ordered shipment vector. Appends, retention, SQLite load and undo preserve ID order; undo stores
jobs without a duplicate historical vector index. Repair is O(1) for a moved resident and O(log
S) for a moved freight carrier, with no new index or allocation. Five alternating release process
pairs (24 Rayon workers, 11 samples of 256 repairs, setup/SoA removal excluded) measured 1,024 /
8,192 / 65,536 shipments at median `0.155 / 0.560 / 12.413 ms` before and `0.004 / 0.005 / 0.005
ms` after. Actual cleanup regressions also exercise mixed resident and freight swap moves with
noncontiguous shipment IDs.

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

The runner records commands for `BINARY --exact
simulation::economy::agents::lifecycle::tests::benchmark_agent_spawning --ignored --nocapture
--test-threads=1`, with `taskset -c 0`, `RAYON_NUM_THREADS=1` and `METRUM_DEBUG=0`. Binary SHA-256
values are `e6beeb312519a402df54cd27681ccf134968dda35970feebacc144b479087d83` before and
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
- shipment sizes round down to the resource batch and never exceed the vehicle payload
- a carrier job should represent a useful batch, not a one-item delivery unless the system explicitly treats it as a premium exception
- `shipment_batch_kg` is authored per resource in kg; `truck_payload_kg` is the separate mass capacity

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

This keeps physical delivery in the shared runtime route and shipment systems.

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
`RAYON_NUM_THREADS=24`. Median milliseconds per 1,000 queries were `0.656 → 0.667` and `2.862 →
2.907`; this establishes no speedup. The verified improvement is removing transient capacity
growth. Selection remains O(K) per candidate and O(K) scratch storage for fixed K; index
construction complexity is unchanged.

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
- routine checks run every `household_replenishment_check_interval_hours` (6) at a stable per-household
  hour offset. The offset is a full hash of home and household id, so a city's households fill every
  check slot evenly (`ECON-19`); saves before version 84 recompute their offsets on load
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
- shopping activity owns both legs and the wait for hourly pickup/delivery acknowledgement,
  including at the shopper's own workplace or home. The work scheduler skips that owned task.
  Delivery restores home activity and clears queued return reminders before releasing the carrier;
  otherwise a between-hour arrival can leave for work before the household receives its paid cargo.
- if the store becomes invalid before pickup, restore reserved store inventory, refund the
  household budget, clear the shopper assignment, and enter bounded cooldown
- if the shopper task is lost or invalidated before pickup, restore the reservation and retry from
  `waiting_for_shopper` or cooldown according to tuning
- each active shopping leg has a deadline equal to its estimated physical travel time (rounded
  up to operational hours) plus the authored delay allowance. One existing route-cost query is
  made when each leg is accepted; none is added per frame. Timeout before pickup restores
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

## Runtime Representation

The runtime compiles the authored TOML into compact data tables at startup.

The authored files are:

```text
economy/
  profiles.toml        # resources, economy profiles, recipes, and runtime tuning
demand/
  growth_profiles.toml # demand-owned growth tuning (see demand.md)
```

The important runtime rules are:

- text files are authoritative and stay readable and editable in a normal text editor
- compiled runtime data is derived from those text files and never written back

Examples of compiled forms:

- resource IDs
- asset-type recipe tables
- supplier-consumer compatibility lists

## Scope and Extension Boundaries

The baseline already has explicit households, typed resource inventories, labor, fiscal
settlement, local utilities and OWA fallback, bounded freight, private construction, transfers,
and a demand/economy handoff. The former eight implementation phases are retired; the active
sequence is the [healthy-growth delivery plan](#delivery-plan).

Extend these systems with a small set of economically justified activities after startup
viability is established. Fixed authored prices and wages remain the baseline. Dynamic markets,
arbitrary scripting, remote work, new zoning categories, bulk terminals, multimodal freight,
policy geography, and Automated Delivery System (`ADS`) home delivery remain outside this work.
They must not become prerequisites for a healthy village or town.

The shipped reference chain is grain farm → food processor → grocery → household supplies.
Farms and processors also consume machinery; machinery factories import steel and metals and
retain machinery for their own upkeep. Personal services and health essentials are aggregate
service outputs alongside this inventory chain. Exact recipes, capacities and prices live in
`economy/profiles.toml`; [standalone service commercial](#standalone-service-commercial) and
[machinery upkeep](#machinery-upkeep-econ-09) own their runtime contracts.

The data-driven inventory and service systems can represent more economic activities, but that
alone does not supply affordable demand, appropriate assets, or viable margins. Each addition
must pass the healthy-growth ledger and scenario checks before it expands the shipped catalog.

## Household Transfer Payments

**Status: implemented.** Household transfers are live in `households.rs`. Defaults are authored in
`economy/profiles.toml`, copied into `CityFiscalPolicy` at runtime, exposed through the Economy
Overview Policy tab, and persisted with the save.

The first transfer set is:

- unemployment benefit paid per unemployed adult within the unemployment time limit
- pension paid per elder
- child support paid per child

These transfers replaced the old `pioneer_demand` floor with explicit support payments from city
treasury to household budgets. They can fund purchases through the normal replenishment loop;
they do not guarantee stock availability, business solvency, or recurring public funding.

### Ownership

This section owns the household-transfer spec. `demand.md` documents the demand-owned move-in
acceptance formula that reads exact candidate composition, policy amounts, and treasury balance as
source values. `households/benefits.rs` owns disbursement. `nodes/sim/core/budget.rs`
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
- Unemployment benefit stops after the configured `unemployment_max_days`. Exhaustion alone is
  not an automatic emigration trigger: housing distress and demand-owned removal rules determine
  departure. Pension and child support are not capped by unemployment duration.

### Money Source

`CityTreasury` lives in `nodes/sim/core/budget.rs`:

- **Starting balance**: `startup_treasury_balance = 500_000` authored in `economy/profiles.toml` `[runtime_tuning]`. Increased from 100,000 for new games; saved treasury balances remain authoritative.
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
| `unemployment_days_elapsed >= unemployment_max_days` | Unemployment benefit stops; housing and demand rules separately determine whether the household leaves |
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
| `unemployment_max_days` | `economy/profiles.toml` runtime_tuning | Maximum unemployment-benefit duration |
| `pension_daily_benefit_per_elder` | `economy/profiles.toml` runtime_tuning | Default currency paid per elder per day |
| `child_support_daily_benefit_per_child` | `economy/profiles.toml` runtime_tuning | Default currency paid per child per day |
| `runtime_tuning.fiscal.*` | `economy/profiles.toml` runtime_tuning | Default tax and property-tax policy values |
| `runtime_tuning.households.*` | `economy/profiles.toml` runtime_tuning | Household starter budget, starter supplies, reserve rules, and utility cost |
| `runtime_tuning.construction.*` | `economy/profiles.toml` runtime_tuning | Private construction durations for fresh demand-owned spawns |
| `owa_import_price_multiplier` | `economy/profiles.toml` runtime_tuning | Price multiplier for OWA imports and missing local utility service fallback |

### Spawn Signal: Replacing the Pioneer Floor

The removed Pioneer demand floor (`pioneer_demand = 0.70`) existed because the `stock_stab` supply-stability metric and
`afford` metrics collapse to near-zero on a fresh map, starving the spawn system of signal.
Household transfers can improve these signals through real economic activity:

1. Disbursement gives households money based on exact adult/elder/child composition -> `afford` rises.
2. Successful, affordable grocery replenishment improves household stock stability.
3. Grocery sales support payroll and inputs; additional shops require unmet customer demand.
4. Operating shops create input demand that can support local industry.
5. Sustainable hiring reduces unemployment claims; the city must still fund other transfers.

The pioneer demand floor has been removed from `demand.rs`. The unemployment benefit is the
work-search cash-support mechanism for adult workers, while pension and child support cover
non-working age groups. Demand uses exact candidate composition plus treasury coverage when
calculating deterministic move-in acceptance.

### Shipped Tuning

Live values in `economy/profiles.toml` `[runtime_tuning]`:

| Parameter | Value | Role |
|---|---|---|
| `startup_treasury_balance` | 500,000 | Total treasury at map start |
| `unemployment_daily_benefit_per_member` | 30.0 | Currency paid per unemployed adult per day |
| `unemployment_max_days` | 30 | Maximum unemployment-benefit duration |
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
| `coal_mine_basic.base_rate_units_per_day` | 120.0 kg/day/hectare | Full-staffed starter coal output before reserve, staffing, and buffer limits |
| `coal_mine_basic.worker_capacity` | 5 / hectare | Full-staffed starter coal-mine worker demand for a 10,000 m2 extraction area |
| `coal_mine_basic.unit_price_currency` | 8.0 | Baseline local coal unit price used for local sourcing and OWA import pricing |
| `grain_farm_basic.base_rate_units_per_day` | 20 kg/day/hectare | Annualized 7.3 t/ha/year before staffing and buffer limits |
| `grain_farm_basic.worker_capacity` | 1 | Worker count for the profile's staffing reference area |
| `grain_farm_basic.worker_capacity_area_m2` | 100,000 m² | One worker per ten hectares, rounded up with a two-worker minimum |
| `grain_farm_basic.area_worker_capacity_max` | 6 | Whole-site staffing ceiling; area output and costs still scale |
| `grain_farm_basic.minimum_field_area_m2` | 40,000 m² | Minimum 4 ha for new field commits and resizing |
| `power_plant_basic.base_rate_units_per_day` | 1200.0 MJ/day | Full-staffed starter power service production before staffing and coal-input limits |
| `power_plant_basic.inputs.coal` | 96.0 kg/day | Coal fuel consumed by a fully staffed starter power plant |
| `power_plant_basic.unit_price_currency` | 0.20 | Local electricity price per kWh |
| `water_plant_basic.unit_price_currency` | 2.0 | Local water price per m³ |
| `wastewater_treatment_basic.unit_price_currency` | 1.5 | Local sewage treatment price per m³ |
| `owa_import_price_multiplier` | 1.25 | OWA import and missing local utility fallback price multiplier |
| `owa_export_price_multiplier` | 1.0 | Fixed outside bid; delivery and net freight economics constrain sales |
| `owa_distress_liquidation_multiplier` | 0.25 | Forced liquidation fire-sale price multiplier; must be no higher than scheduled export |

## Machinery Upkeep (`ECON-09`)

Machinery is one freight resource representing equipment and spare parts. It is a recurring
production input; there are no individual machines, wear timers, replacement events or agent
life-cycle rules. Inputs are bought locally first, then imported from OWA through existing
paid freight. Until a Machinery factory asset is installed, all Machinery comes from OWA.

Initial full-operation rates in `economy/profiles.toml`:

| Consumer profile | Machinery kg / operational day | Scaling |
| --- | ---: | --- |
| `grain_farm_basic` | 1 | Per hectare of committed field |
| `coal_mine_basic` | 4 | Per hectare of committed extraction area |
| `food_processor_basic` | 2 | Per building |
| `power_plant_basic` | 4 | Per building |
| `water_plant_basic` | 0.5 | Per building |
| `wastewater_treatment_basic` | 0.5 | Per building |
| `machinery_factory_basic` | 2 | Per building, retained from its own product stock |

A 10-hectare mine requires 40 kg Machinery/day; a 10-hectare farm requires 10 kg. Farms keep their
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
--test-threads=1`, with `RAYON_NUM_THREADS=1`. These timings cover lookup and removal of
buildings without production sites; they do not measure the O(S) shifts required by
affected-site edits.

Machinery reference price is 20 currency/kg; OWA imports cost 25/kg at the current 1.25 multiplier.
These are initial game-balance values. Heavy mines are intended to benefit from local customers
and local Machinery: at full operation per hectare, 120 kg Coal sells locally for 960, imported
Machinery costs 100 and five workers cost 450/day before other charges. Regional buying limits
and area-based utility bills also constrain profitability.
A regression checks positive full-operation margins after maximum authored wages, Machinery/raw-material
imports, property tax, conservative utility charges and profit tax: existing grain/coal inputs are
local, new Machinery/Steel/Metals are imported, producers have local buyers, and utility plants have
sufficient service demand. Water/sewage examples use 200/250 m³/day. This proves viable
operating cases, not profitability for tiny fields, insufficient customers or every import/export
mix. Personal services and groceries do not receive Machinery inputs in this first pass.

Input stock targets scale with work area and hold at least two minimum shipment quantities
(80 kg for machinery, 100 kg for food/grain, 200 kg for steel/metals). Positive reorder thresholds retain at least one
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
[`demand.md`](demand_history_2026-10-10.md) and the authoring checklist in [`asset_editor.md`](../asset_editor.md).

### Resource identity and cost

`[[resources]]` entries in `economy/profiles.toml` own explicit, contiguous, one-based `runtime_id`
values. These IDs are persisted by inventories, shipments and freight bookkeeping: append new
IDs and never reassign existing ones. IDs 1–6 retain the previous resource identities; Machinery,
Steel and Metals are 7, 8 and 9. File/name ordering no longer assigns identity. Import-only goods
may specify `import_unit_price_currency` without introducing a fake producer profile. Remove
that import-only price when an output profile becomes the resource's price owner. Runtime
compilation reads the canonical TOML. Profile IDs remain append-only by
authored order. Input and output ports share one compiler:
each resource appears at most once per direction, with a finite nonnegative rate. A resource may
appear on both sides for internal upkeep; duplicate entries cannot cause unaccounted consumption.

Live freight imports Machinery through OWA deliveries and stock batches when no local factory
supplies it.

### Runtime bounds and verification

Hourly production remains O(buildings) for a fixed catalog, with Rayon on the ordinary-building
pass; area production keeps its existing indexed site traversal. Input consumption allocates no
memory per building. Recipe work traverses inputs/outputs and performs bounded same-resource port
lookups. Demand computes each profile's capacity scale once per building and reuses its output-port
pass for the relevant resource totals. It reuses the existing parallel resource reduction and
compact resource arrays; with a fixed catalog it remains O(buildings), plus candidate output-port
checks. No agent traversal or additional spatial index is introduced. Freight uses the existing
supplier/component indices, route cache and reservations; protecting additional customer types adds
recipe-port work to its existing building pass. The outbound-only reservation query scans active
shipments in O(S), retains only source inventory slots, and does not allocate inbound views. Its
flattened storage ends at the last active source/resource slot, independently of destination IDs.
Both views share source-cargo eligibility and reject resource IDs outside the catalog. This is a
reservation-query measurement, not whole freight throughput or a city-frame claim. Daily-to-hourly
production conversion has one shared constant. Demand and household settlement share the same
compact-resource aggregation helpers, and service activity/sales share one demand-rate query.
Stores, service stores and explicit production areas all follow their funded census activity.
Resource order and accumulation order are preserved. Mine depletion updates its existing cache in
O(1) per site; inventory inspection and reserve protection perform bounded recipe-port work without
adding allocations or city-wide scans.

The `AUDIT-01` follow-up uses shared fixed-size parallel chunk folds and input-order merging for
floating-point activity and demand totals. Accumulators update in place; worker scheduling cannot
change the totals. Work remains O(buildings + households) for a fixed catalog, with O(N / chunk size)
temporary accumulators. No per-resident allocation or sort of chunk results is needed.

Fresh matched unprofiled release comparison: six alternating process pairs, `RAYON_NUM_THREADS=24`,
the existing `benchmark_business_demand_snapshot` factory fixture, 10 warmups then 11 samples of
100 snapshots per size; setup is excluded and no compilation/Godot work runs concurrently. Median
milliseconds/snapshot before → after: 1,024 buildings `0.092 → 0.028`, 8,192 `0.125 → 0.094`,
65,536 `0.2905 → 0.252`. This measures business snapshots, not city FPS or all mixed
household/industry workloads. Command on each saved release test executable:
`benchmark_business_demand_snapshot --ignored --nocapture`. The before build restores the initially
inventoried metrics/snapshot sources in the isolated `reduction-baseline/` crate. Earlier
short-window timings are superseded by this comparison.

Follow-up audit verification on 2026-09-12: the release Rust suite passed 1,709 tests (16 ignored),
including proportional upkeep, paid startup imports, shared-budget local holds, inactive farm
demand, net output headroom, small-shop restocking, invalid recipe rejection, industrial absorption
and save round trips. Added regressions cover depleted-mine capacity restoration, upkeep/shipment
reserves during liquidation, and distinct-resource inventory fill. Profitability checks include
maximum authored wages and farm/mine property taxes; utility accounting checks distinguish power
payroll and include prior-day input refunds. All eight headless Godot regressions passed, including
native economy-editor export/playback, the Machinery asset selector, refund display, farm inspector
and camera save/load. The editor checks cover Godot's floating-point JSON representation of
whole-number resource IDs and construction durations; fractional durations remain invalid. The
release library was rebuilt and deployed. The 15 Python runner/report checks, benchmark-target
compile check, formatting and diff checks pass. Rustdoc reports no missing-doc warnings. Clippy
reports no warnings on changed lines; it completes with existing repository warnings when the
pre-existing `clippy::mut_from_ref` error in `agents/tick/slices.rs` is allowed.

Fresh follow-up audit demand benchmark on 2026-09-12: matched, unprofiled release builds, 24 Rayon
workers, 11 samples per size, median snapshot time. The fixture holds the road graph fixed and
varies staffed food processors with stocked input buffers and a 10,000 operating budget;
setup/cloning is excluded. Both builds include Machinery.

| Buildings | Before audit | After audit |
| ---: | ---: | ---: |
| 1,024 | 0.106 ms | 0.108 ms |
| 8,192 | 0.141 ms | 0.132 ms |
| 65,536 | 0.448 ms | 0.281 ms |

Command: `RAYON_NUM_THREADS=24 cargo test --offline --manifest-path rust/Cargo.toml --release --lib
benchmark_business_demand_snapshot -- --ignored --nocapture`. Fixture:
`rust/src/simulation/economy/demand/tests.rs::benchmark_business_demand_snapshot`. The final run
saves 0.167 ms at 65,536 buildings; small-fixture results are shown above. The fixed-catalog linear
bound is unchanged. These measurements cover demand aggregation, not end-to-end freight or
million-agent throughput.

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

The grain farm has `min(6, max(2, ceil(field_hectares / 10)))` physical jobs for a committed
field, with a 4 ha minimum for new fields and edits. These are total positions, not two
additional vacancies beyond the resident workers.
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
`5f0663cc15bc2c67996e309dad22b618d2e55e3caf2ea2b0b2b4fe4d97460107`; `agents/tick/schedule.rs`
SHA-256 `df8b76823bcbbcaf404317304090fb6a719e846c1fb45d8a7ffeaa53c3cb3e13`;
`households/tests/farms.rs` SHA-256
`d475a97bb4dec9d1b8c758078a1cf5b1198cf1ed28d13cb378f48affdc381782`. Commands from `rust/`: `cargo
test --release --lib --no-run`, then `RAYON_NUM_THREADS=4
$TEST_BINARY benchmark_farm_household_tick --ignored --nocapture
--test-threads=1`. Timing ran after builds and other checks finished.

### Farm audit validation (2026-09-11)

The audit fixed lost farmhouse living-area exports, stale bankrupt-farm vacancies, household
membership loss during automatic removal, and attachment checks using a differently rotated lot
from the visible guide. Farm classification/defaults, the field worker floor, vacancy removal,
and the plot-boundary payload now have shared owners. Unused agent-owned home selection and
person-counted occupancy rebuilding were removed; housing fixtures claim household slots through
the allocator.

Fresh checks: `cargo test --lib` passed 1,684 tests (14 ignored); `cargo build` and `cargo doc
--no-deps` passed without warnings. The rebuilt/deployed extension passed the headless field-tool
regression and industry-tool, inspector, and asset-editor script checks.

Release checks ran after compilation finished. The vacancy benchmark holds 10,000 claim/release
pairs on one farm fixed, with untimed setup and index allocation, three warmups, 21 samples, one
test thread, and `RAYON_NUM_THREADS=4`. Median times were 0.583 / 0.580 / 0.560 ms with 1 /
1,000 / 100,000 buildings, consistent with allocation-free O(1) claims independent of background
housing. The capacity benchmark retained 2.43 ns/query for the field minimum through 1,000,000
queries (`RAYON_NUM_THREADS=1`). Run the release test binary with
`benchmark_farm_vacancy_claims` or `benchmark_work_area_capacity`, then `--ignored --nocapture
--test-threads=1`.

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
with population, and do not measure end-to-end city economy cost. Build identity is `325f0402`
plus the working farm changes and audit fixes. Before/after release binary SHA-256:
`5f0663cc15bc2c67996e309dad22b618d2e55e3caf2ea2b0b2b4fe4d97460107` /
`5db37fd42543f05b5d62dd6c34792693a4a1d21fd4085eecf93ee28a0726ff9a`. Audit `allocator/index.rs`
SHA-256: `550e5d36c95865740f6c71e33ef10991f2c140204fa9cf2b50d98a0b95f2917e`;
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
The original farm save remains unchanged. Default Clippy still rejects the pre-existing
`mut_from_ref` in unchanged agent `tick/slices.rs`; the audit run allowing only that lint
completes with the existing lint warnings and no errors.

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

Repeated planning remains local; the existing one-time snapshot is separate and reaches 3.369 /
3.394 ms at the largest size.

Three alternating headless before/after industry-tool runs use 21 timed batches of 10,000 manual
idle updates after three warmups. Median cost falls from 1.116–1.150 to 0.066–0.069 microseconds per
direct call; actual inactive tools now disable processing entirely. Setup is untimed, both scripts
use the same release extension, and no build or other benchmark runs concurrently.

## Field Staffing Density (`ECON-06`)

Field producers and extractors keep output and storage rates per 10,000 m2. Their integer
`worker_capacity` applies to `worker_capacity_area_m2` instead, which defaults to 10,000 m2
and must be positive and finite. Ordinary building profiles still use fixed worker counts.

The runtime compiles workers per hectare once, then calculates physical field jobs as
`max(2, ceil(workers_per_hectare * field_hectares))`, capped by an optional profile maximum,
for positive areas and worker densities.
Uncommitted fields and disabled worker densities have zero jobs; extractors retain ordinary
rounding without the field minimum. Market availability and payroll funding still gate active
jobs. Current grain farms provide two jobs from the 4 ha placement minimum through 20 ha,
three at 30 ha, five at 50 ha, and at most six thereafter (including 100 ha).
The production/freight revision rejects its intermediate two-workers-per-ha experiment.

Full staffing currently produces 20 kg grain/day/hectare. Partial staffing uses the physical
capacity as its denominator; demand equivalents, export reserves, and first-field startup
payroll use the same density, minimum and maximum. Construction and upkeep still scale with the
entire field after staffing caps. Preview and commit enforce the profile's minimum field area.
Saved field areas rebuild this scale on load, and the daily wage pass releases surplus workers
through the existing assignment lifecycle. No save migration
or new per-agent work is needed. The inspector reports physical field capacity, with active
positions shown separately when market-limited. Density queries remain allocation-free O(1).

The original density reduction increased margins; the later production/freight worksheet
rebalances yield and density together. The following timing evidence belongs to the original build.

Regression coverage checks hectare boundaries, hourly yield, startup payroll, demand equivalents,
surplus-worker release, export reserves, invalid staffing areas, and editor export/reload.
Validation before the two-worker floor: `cargo test --lib` passed 1,663 tests (11 ignored), `cargo
doc --no-deps` reported no warnings, and Godot `--check-only` passed for the economy editor and
building inspector. The isolated release timing is reproducible from `rust/` with `cargo test
--release benchmark_work_area_capacity -- --ignored --nocapture`.

Capacity-only timing before the two-worker floor on 2026-09-11 used the release build from
`325f0402` plus the initial staffing change, one test thread, 21 unprofiled samples per size, and
eight fixed hectare values repeated across each query batch. With other builds finished, median
times were 0.023 ms / 10,000 queries, 0.233 ms / 100,000, and 2.328 ms / 1,000,000 (about 2.33
ns/query). This measures the capacity helper only; it is not an end-to-end city-speed comparison.
The measured `work_area.rs` SHA-256 is
`4002cbb2b49af54bef393c29cf211aa28463016345d3889bbdd44fd3eef2719c`.

Fresh validation with the two-worker floor and plot guides on 2026-09-11: `cargo test
--lib` passed 1,674 tests (12 ignored), and `cargo build` plus `cargo doc --no-deps`
completed without warnings. The deployed debug extension passed the headless field-tool
regression; industry-tool and inspector script checks also passed. Headless checks used
`--log-file $ARTIFACTS/...` so they did not need to write the game's user-data directory.

The current release capacity benchmark compares the same helper with minimums of zero and
two, using the same eight hectare values, 21 unprofiled samples per size, one test thread,
and `RAYON_NUM_THREADS=1`. It performs no city setup or allocation in the timed query loop.
After builds finished, medians were:

| Queries | Density only, minimum 0 | Farm, minimum 2 |
|---|---|---|
| 10,000 | 0.047 ms | 0.024 ms |
| 100,000 | 0.254 ms | 0.243 ms |
| 1,000,000 | 2.499 ms | 2.431 ms |

This checks O(1) capacity-query cost in the current build; it is not a whole-city comparison or a
comparison against the old implementation. Build identity: `325f0402` plus the field changes,
`work_area.rs` SHA-256 `da5d368d395d3e8dec02b9e3574efdf637d7f07453ade6e12f65310a74851048`, release
test binary SHA-256 `fd548c59bc9d5574a705be67d6cc9a0bbc7952569cac14c8810d671ac932ad15`. Commands
from `rust/`: `cargo test --release --lib --no-run`, then `RAYON_NUM_THREADS=1
$TEST_BINARY benchmark_work_area_capacity --ignored --nocapture
--test-threads=1`.

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

Validation before plot guides and the two-worker floor on 2026-09-11: `cargo test --lib` passed
1,673 tests (12 ignored); `cargo build` and `cargo doc --no-deps` completed without warnings.
The rebuilt extension passed the headless field editor and native road junction/preview
regressions. Input manager and inspector script checks also passed.

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
`populated_field_road_plan_scaling`, followed by `--ignored --nocapture --test-threads=1`. Build:
`325f04021c2f1d8be7f189aa3e46120bb1010c03` plus these working-tree changes; binary SHA-256
`03157cecf2434837da368ae298433024367b9817967ea5bb897b47ae9e5b9ded`. These measure local
placement, not whole-city tick throughput or interactive GPU rendering.

## Building Bankruptcy

**Status: implemented.** The two-day `budget_distress` bankruptcy check is live in `households.rs`
(`run_bankruptcy_check`, `daily_settlement` four-step sequence). `budget_distress: bool` is
persisted in the SQLite schema and loaded by `world.rs`.

This section is the authoritative spec for how commercial, industrial, and privately operated
utility buildings manage their operating budget, pay obligations, and enter bankruptcy. City-owned
explicit service buildings are treasury-funded municipal facilities in the live `v0.1` path; they
provide staffed service and city revenue but do not enter the private-building bankruptcy loop. The
previous system used an hourly utility gate (`utility_service_available`) that permanently froze
any building whose budget dipped below a single hourly charge. `ECON-01` retired that mechanism;
the [retired mechanisms](#retired-mechanisms) table records the completed corrections.

### Operating Budget

Each commercial, industrial, and privately operated utility building holds an `operating_budget:
f32` cash balance. It is separate from household budgets and the city treasury. City-owned service
buildings still have runtime telemetry fields, but their payroll and local operator receipts settle
against the city treasury.

Money enters the budget from:

- sales revenue when households or other buildings purchase the building's output
- scheduled export receipts and explicit emergency liquidation receipts
- utility service revenue distributed to private local provider buildings when consumers pay charges

Money leaves the budget from:

- daily wage payments to workers
- input purchases and business taxes
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
zero active area-backed workers and therefore receives only `STARTUP_OPERATING_FLOAT`. On the
first valid area commit, its private float is raised to opening imported input buffers plus
seven days of area-scaled payroll and imported utilities. These are one-time owner funds,
separate from the city-paid building and land-preparation costs. Resizing an existing site
never adds private startup money.

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
`benchmark_payroll_phase` includes payroll and final money publication, four calls per sample. At
65,536 workers, 24-worker medians are 0.235 → 0.253 ms without power quotas, 0.463 → 0.580 ms with
quotas, and 2.683 → 2.081 ms for payroll. Process variation is substantial (power before
0.379–1.841 ms, after 0.395–0.665 ms), so these do not establish an overall speedup. Single-worker
staffing medians are 0.130 → 0.106 ms and 0.377 → 0.317 ms; payroll ranges overlap. The bounded
quota overhead is accepted for the existing hourly/daily cadence; there is no new per-agent
allocation or city-wide index. Commands are the two ignored benchmark names with `--ignored
--nocapture --test-threads=1` on preserved release binaries. These fixtures do not measure
complete job route searches.

**Step 3 — Pay utility cost.**

Private businesses pay daily utility costs even when this makes their operating budget negative.
Electricity is metered hourly using the profile rates above. Water/sewage retains 1 m³ each
per workplace/day; farms/mines add authored area maintenance. Unpaid electricity is a deficit,
not collected utility revenue.
The authored baseline prices are shared across these businesses:

| Service | Local unit price | OWA unit price (× 1.25) |
| --- | ---: | ---: |
| Electricity / kWh | 0.20 | 0.25 |
| Water / m³ | 2.0 | 2.50 |
| Sewage / m³ | 1.5 | 1.875 |
| Daily total | Depends on metered electricity and area | Depends on metered electricity and area |

`power`, `water`, and `sewage` resolve independently. Power uses hourly metered kWh:
consumers pay the local authored utility price only for the covered share, and uncovered private
demand pays the OWA fallback share. If a staffed local `water` or `sewage` provider exists,
consumers pay that service's local authored utility price; otherwise they pay that service's OWA
fallback share. Local service fees for city-owned providers deposit into the city treasury;
provider `revenue` remains telemetry. OWA fallback spend leaves the local economy.

Residential electricity is metered hourly and billed at daily settlement. Households pay only
from available cash; the remaining local/import charges carry forward as arrears and reduce
housing reserves. Water/sewage retains its hourly allowance and cash-limited OWA surcharge.
Electricity is excluded from that allowance, so no charge is duplicated. Provider revenue records
cash actually collected; physical demand is independent of cash.
Household payment ledgers record the actual cash debit at the account's stored precision, so
repeated fractional hourly fees reconcile even as household savings grow.

Save format 73 stores each building's current/previous-day meters, daily city-funded input costs,
closed accounts from removed buildings, and each household's electricity meter, collected payments
and arrears. Mid-day continuation preserves the next electricity settlement, including demolition.
Older power quantities convert MJ to kWh (divide by 3.6); coal inventories, shipments and remaining
deposits keep their physical kg values. Old files have no consumer meters, local/import payment
split, removed accounts or input-cost ledger to recover. Those new fields start at zero: the first
day after a pre-73 load combines legacy generation/payment totals with newly metered consumption,
so it is a conversion boundary, not an exact replay of the old electricity settlement. Household
cash and recorded payments are retained without charging them again.

Save format 61 stores the three collected utility payments with each household and restores them
before load-time repair or settlement, without charging cash again. Formats 57–60 had no such
fields and start those amounts at zero; previously discarded payments cannot be reconstructed.
Other diagnostic counters begin a new report window after loading. Saved payment amounts must be
finite and nonnegative, and all four household age/member counts must fit their `u16` storage.
The daily simulation step owns the accounting-window reset after settlement and reporting;
diagnostic printing is read-only. The unused `HouseholdSystem::clone` implementation is removed.


Utility accounting validation (`AUDIT-01`, 2026-09-12): both cash-limit regressions failed before
the fix; the full release suite now passes 1,731 tests (25 ignored). The matched unprofiled release
fixture runs 24 hourly household charge/stock passes plus one daily utility settlement, with 11
samples and five alternating process pairs, 24 Rayon workers. At 1,024/8,192/65,536 households,
median daily-block time is 1.689/1.869/4.723 → 1.519/1.904/5.187 ms. Large-fixture process ranges
overlap (before 3.823–6.467 ms, after 3.543–5.463 ms); no speedup is claimed. The fix adds O(1)
arithmetic per household to the existing O(H + B) settlement, with no new allocation. Setup and
ledger reset are excluded; the fixture uses funded households and OWA-only utility service. Run
`benchmark_household_utility_settlement --ignored --nocapture --test-threads=1` on the preserved
release binaries. The subsequent save/lifecycle batch validates mid-day payment persistence
separately.



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
model. It bypasses the normal `shipment_batch_kg` buffer check — the sale is a distress action,
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

The demand system clears empty deserted commercial and industrial sites with no workers
independently of demand pressure, using its existing bounded demolition credit. Healthy business
and residential removal remain demand-driven. Clearance permits a subsequent normal placement;
it does not resurrect the bankrupt company or preserve its cash.

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

## Retired Mechanisms

These are completed corrections, not additional healthy-growth deliverables. Their old incident
reports and replacement instructions must not be read as descriptions of the current runtime.

| ID / mechanism | Current contract |
| --- | --- |
| `ECON-01`: hourly utility-budget freeze | Utilities settle daily. Throughput has no `utility_factor`; private failure uses the distress/desertion lifecycle above. |
| `ECON-02`: inactive nominal capacity blocking replacement | Demand excludes inactive states and distinguishes live output from committed construction. See `demand.md` for absorption accounting. |
| `ECON-03`: repeated startup float refill | Private float is assigned once at creation. No daily rescue refill; insolvency can cause desertion. |
| `ECON-04`: more zoned land generating arbitrary businesses | Spawn volume follows missing economic capacity; eligible sites cap placement rather than manufacture customers. |
| `ECON-05`: pioneer floor creating non-residential demand | The pioneer floor is removed. Household transfers are explicit fiscal flows; commercial and industrial demand read their customers. |
| Bankruptcy gap | `budget_distress` and `is_deserted` are implemented. Do not add another economic-death timer as if bankruptcy were missing. |
| Economy editor (`ECON-10`) | Removed. Authored TOML, Rust balance checks, and headless runtime scenarios are the balancing workflow. |

Seven days of payroll float and first-import financing repaired initial capitalization; they do
not prove recurring solvency. Transfers replaced the pioneer floor, but they neither guarantee
stock availability nor establish a sustainable city budget. The prior claim that a startup
starvation window was acceptable is not a healthy-growth acceptance criterion.

Preserve the ownership boundaries: explicit household records, demand-owned growth decisions,
player-placed city facilities, one-time private owner equity, and shared freight/trip execution.
New activity should extend those systems without restoring old fallback growth logic.

## Validation Evidence

The following records distinguish test infrastructure, dated failures and current acceptance.
The regional-market section records the passing starter-economy checks; older timings and
trajectories retain their original dates and do not validate a later build.

### Electricity validation (`ECON-14`)

**Current correctness (2026-10-07).** The lifecycle revision passes 362 economy tests and 25
save/load tests. Coverage includes fuel limits, fixed staffed upkeep, cash-poor delivery, mixed-use
bills, multiple providers, rounded cash debits, legacy mine density, removed-building meters/input
costs and removal after midnight billing. Fourteen field-lifecycle checks pass. Actual-core
demolition/undo and mid-day save continuation pass. Continuation compares uninterrupted and loaded
runs with the plant present or demolished, through the next settlement: energy, bills, fuel,
revenue and net cash agree within 0.01. This proves format-73 continuation; pre-73 migration has
the cutover limits documented above.

The broader core batch passes 113 tests. The allocator batch passes 59 and fails two startup
residential family/variant selection fixtures before their selection assertions. Both fail
identically on unchanged HEAD `318b1634`; they remain tracked under `DEM-02`.

The headless Godot economy regression passes against the current extension in an isolated user
profile, checking rendered kWh/MWh values and kW/MW formatting as well as the existing machinery
authoring checks. Fresh `cargo doc --no-deps` reports no missing-doc warnings; its one
unresolved `VegetationGenerator` link is unrelated.

All three growth integration tests pass after adding the industrial labor/housing guard. It
rejects startups when no unemployed workers remain beyond existing vacancies and neither vacant,
unfinished nor additional candidate homes can house more, reusing existing residential
placement/upgrade candidates. This fixes the late unstaffed factory failure found by the initial
two-year run. The village ends with 155 residents and 115 employed, no bankruptcies/replacement
capital, no second-year empty-household days, and positive recurring city finances. This village
contains no power plant, exercising OWA electricity supply and affordability.

**Physical plant/mine runs.** `coal_power_balance_matrix --ignored --nocapture --test-threads=1`,
with `RAYON_NUM_THREADS=8`, grows the village for 365 days, places the plant while housing remains
available, then observes 365 days of real hiring, production, freight and settlement. Mine cases
place a 1 ha mine at the same time. The producer fixture declares a one-time construction
endowment before observation; business opening cash is excluded from operating cash changes.
Plant construction is treasury-funded. All six cases pass accounting, actual staffing, generation,
final-quarter household supply and positive recurring city-budget checks. Every private profile
has positive observed operating cash change and no bankruptcy or replacement startup capital.

| Coal source | Workers | Final residents | Generation/day, kWh | Imports/day, kWh | Plant cash flow/day |
| --- | ---: | ---: | ---: | ---: | ---: |
| Imported | 1 | 153 | 1,200.00 | 106.76 | +41.36 |
| Imported | 2 | 156 | 1,352.47 | 0 | −61.93 |
| Imported | 3 | 156 | 1,342.47 | 0 | −183.73 |
| Local mine | 1 | 155 | 1,200.00 | 699.40 | +54.96 |
| Local mine | 2 | 156 | 1,899.97 | 0 | +37.79 |
| Local mine | 3 | 156 | 1,919.97 | 0 | −77.39 |

These are trailing-90-day cash averages, including input purchases/freight, wages and utility
upkeep; they are distinct from the steady operating worksheet above. Bulk machinery stocks can
span a reporting window. The mine adds electricity demand and supplies fuel; its observed-year
operating cash gain is 62,931–65,735, excluding opening cash. More staffed capacity alone does not
create customers.

**Matched 10k performance.** Sequential unprofiled release runs, eight Rayon workers pinned to
CPUs `0,2,4,6,8,10,12,14`; no concurrent tests/builds. Both use the existing `/10k` fixture:
11,120 residents, 5,597 households, 7,620 employed and 7,396 buildings. Setup is outside timing;
23 hourly samples and ten daily samples advance the same workload. No larger tier ran.

| Build | Setup, s | Hour mean, ms (95% interval) | Daily mean, ms (95% interval) |
| --- | ---: | ---: | ---: |
| HEAD `318b1634` | 15.3 | 2.205 (2.133–2.281) | 823.09 (618.50–1,004.10) |
| kWh + lifecycle accounting | 15.3 | 2.389 (2.329–2.454) | 795.91 (594.74–972.64) |

The final revision adds about 0.18 ms/hour (+8.4%, `p<0.01`); daily change is not significant
(`p=0.85`). These runs establish added hourly cost, not unchanged performance or a
lifecycle-optimization speedup; process variation affects its magnitude. New work is O(buildings +
households), with reused scratch, parallel passes, fixed-order reductions and no added routing or
per-agent scheduling. The broad daily cost remains `ECON-13` work.

Builds use `RUSTFLAGS='' CARGO_ENCODED_RUSTFLAGS='' cargo bench --manifest-path rust/Cargo.toml
--bench economy_tick_benchmark --no-run`. Executable SHA-256 prefixes are `3cbb0ec1b218f55c` (HEAD)
and `2eaee9667c7b2df1` (final lifecycle build); catalog prefixes are `a30680e6f8cde47b` and
`753ce26467a405b2`. Run commands:

```bash
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-power-final-perf taskset -c 0,2,4,6,8,10,12,14 $ARTIFACTS/metrum-power-head-economy --bench /10k --save-baseline before
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-power-final-perf taskset -c 0,2,4,6,8,10,12,14 $ARTIFACTS/metrum-power-final-economy-bench --bench /10k --baseline before
```

### `ECON-11` balance tests

Done 2026-10-02. `rust/src/simulation/economy/definitions/balance_tests.rs` checks the shipped
`economy/profiles.toml` on paper, without running the simulation. Each business is evaluated at
full staffing with `wage_max_currency_per_day`, one building or (for field and extraction
profiles) the smallest area where the field staffing floor stops binding. Outputs sell, and
locally produced inputs are bought, at the catalog unit price. Import-only inputs pay
`owa_import_price_multiplier`. Utility bills, taxes, freight and finite customer volumes are left out,
so passing is necessary for solvency but not sufficient.

| Test | State | Result on 2026-10-02 |
| --- | --- | --- |
| `shipped_businesses_are_solvent_at_full_staffing` | active | All producers, processors, stores, service stores and extractors profit at local prices. |
| `solvency_check_reports_a_profile_whose_wages_exceed_revenue` | active | Guards the check itself: a grocery paying 1,000/day per worker is reported. |
| `closed_chain_jobs_per_thousand_residents_matches_recorded_baseline` | active | 130.55 jobs per 1,000 residents when everything residents consume is made locally and nothing is exported. City-owned utilities are excluded. Change the recorded constant deliberately when tuning moves it. |
| `basic_industries_profit_from_exporting_all_output` | active since regional repair (2026-10-06) | At `owa_export_price_multiplier = 0.60`: food processor −560/day, coal mine −4/day, machinery factory −200/day. Only the grain farm profits from exporting. |

Run them with `cargo test --manifest-path rust/Cargo.toml --lib economy::definitions::balance_tests`.
All four are active after the regional-market repair; the last column preserves original results.

Review verification on 2026-10-05 at `21100889`: `cargo test --lib
economy::definitions::balance_tests -- --include-ignored` passed the three active checks and
failed the export target with the same −560/−4/−200 breakdown. These export figures use maximum
authored wages; the startup margin table in the diagnosis uses runtime average wages.

### `ECON-12` growth scenario

Done 2026-10-02. The sim thread and headless runs now advance the city through the same code:

- `SimCore::step_frame` (`rust/src/nodes/sim/core/frame.rs`) is one ~60 Hz frame. It refreshes
  pathing, moves agents by `FRAME_DT_S × speed`, then runs every authored minute the clock
  crosses: queued demand spawns, the operational hour on whole hours, daily settlement at
  midnight. A paused clock changes nothing. The sim thread keeps command handling, the core
  lock, cell-chunk preparation, snapshots and perf logging; lock ownership is unchanged.
- `SimCore::commit_road` (`road_commit.rs`) is the former `SimCommand::AddRoad` body: plan reuse,
  transactional commit, lane, agent, entrance and routing refresh, road meshes and road-locked
  terrain.
- `SimCore::new` replaces five copied `SimCore` literals. `SimCore::replace_asset_registry` is
  the Godot-free half of pack loading; Godot still imports mesh bounds first.

`GrowthScenario::starter_town` (`rust/src/nodes/sim/core/scenario.rs`) builds a flat 2 km world
through those player paths. It has:

- an `OWA` border road into a main street, with three cross streets;
- residential paint on two cross streets, industrial on the third, commercial on the main street;
- the repository's `godot/bootstrap/mods` pack.

Then it runs at maximum speed (32×), frame by frame, because a household is admitted only once
its carrier reaches home. `rust/tests/growth_scenario.rs` initially held these two tests; the
subsequent diagnostic audit also adds observer-equivalence and accounting checks:

| Test | State | Result on 2026-10-02 |
| --- | --- | --- |
| `concurrent_runs_record_identical_years` | active | Two 365-day runs, concurrent on one Rayon pool, record identical days. About 10 s in the `test` profile. |
| `starter_town_keeps_growing_and_solvent_through_year_one` | historical; replaced by active two-year acceptance on 2026-10-06 | Fails: the town stalls and goes broke (trajectory below). |

| Day | Households | Residents | Employed / jobs | Buildings | Treasury |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 2 | 1 | 1 | 0 / 0 | 5 | 57,329 |
| 32 | 18 | 28 | 11 / 39 | 29 | 52,102 |
| 122 | 25 | 40 | 20 / 47 | 61 | 35,985 |
| 242 | 25 | 40 | 19 / 39 | 88 | 7,741 |
| 272 | 25 | 40 | 16 / 24 | 95 | 892 |
| 302 | 24 | 39 | 1 / 6 | 59 | −4,955 |
| 366 | 18 | 28 | 0 / 0 | 59 | −20,235 |

In that 2026-10-02 run, households reach 25 by day 107 and stop. Buildings keep being added while
the treasury falls about 130 per day. Once it is negative, near day 280, jobs fall to zero and
households start leaving.

Review verification on 2026-10-05 at `21100889`, `cargo test --test growth_scenario --
--include-ignored` in the test profile: concurrent-year determinism passed; the growth target
failed. Selected records from this newer trajectory:

| Day | Households | Residents | Employed / nominal jobs | Buildings | Treasury |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 32 | 18 | 30 | 14 / 51 | 32 | 52,462 |
| 122 | 29 | 50 | 27 / 87 | 74 | 37,473 |
| 182 | 42 | 74 | 20 / 29 | 97 | 7,392 |
| 212 | 29 | 51 | 0 / 0 | 68 | −3,897 |
| 366 | 12 | 19 | 0 / 0 | 55 | −42,465 |

That review used the historical 25-household ignore description. The diagnostic extension below
replaces it with the unmet healthy-growth target. A positive initial treasury is not evidence of
recurring fiscal solvency.

The same review ran the demand, household and balance unit suites; the fresh diagnostic-extension
counts below distinguish active tests from ignored targets. No performance benchmark was rerun for that assessment or this
documentation revision.

Limits:

- Mesh bounds need Godot's importer, so building sites use the meshless support path.
- The explicit-placement farm, coal mine and power plant are not placed.
- Results are reproducible for one build and asset pack. Any change to simulation code, tuning
  or the bootstrap pack can move them, so the test pins determinism, not the trajectory.

Run the active checks with `cargo test --test growth_scenario`. Add `-- --ignored` to run the
unmet `ECON-14` target and include the compact failure report in its assertion output.

#### Village failure timeline (`ECON-14`)

The 2026-10-05 diagnostic extension observes the existing frame/hour/settlement paths; prices,
wages, demand formulas and the fixture are unchanged. `GrowthScenario::run_diagnosed_days`
returns ordinary daily records plus a `GrowthReport`. The report reads household ledgers before
removal, business receipts and cash before lifecycle actions, and the city ledger at close.
Business identities use parcel id plus build generation rather than swap-removable indices.
The live runner uses a statically dispatched empty observer and stores no diagnostic history.

Reproduce from the repository root:

```bash
METRUM_GROWTH_REPORT=$ARTIFACTS/metrum-village-daily-2026-10-05.txt \
  cargo test --manifest-path rust/Cargo.toml --test growth_scenario
cargo test --manifest-path rust/Cargo.toml --test growth_scenario \
  starter_town_keeps_growing_and_solvent_through_year_one -- --ignored
```

The first command saves all 365 settlement records and checks deterministic accounting and
cash reconciliation. The second intentionally fails the existing healthy-growth target and
prints selected days plus first failure events. Day labels identify the newly opened day;
the first window starts at day 1, 07:30. Household accounting/counts precede departures; city
population follows them. Launched carriers are distinguished from resident arrivals.

**Observed sequence, 2026-10-05 catalog, bootstrap pack and recovery rule:**

| Day | Evidence | Interpretation |
| ---: | --- | --- |
| 2–6 | Treasury starts at 57,640 after 42,360 of road construction; road upkeep is 283.40/day. Household cash-flow deficits first appear on day 4, supply stockouts on day 6. | The road network has a recurring bill immediately. A single household purchase-day deficit is not proof of persistent unaffordability. |
| 10–14 | First food processor is cash-negative on day 10; it closes on day 14. Employment falls from 20 on day 10 to 10 on day 14. Day-14 taxes are 270.97 against 500 transfers and 283.40 upkeep. Treasury still holds 56,261.56. | Private business failure precedes fiscal exhaustion and raises transfer needs. |
| 18–41 | First machinery factory closes day 18, personal-service store day 26, health store day 40. The first exhausted unemployment window with no benefit payment is day 41. | Early business losses are not caused by the city's eventual inability to pay benefits. Services also lack enough sales to cover their minimum operation. |
| 98–163 | First grocery closes day 98. Firms are repeatedly replaced with fresh startup funds. First household departure is day 163. | Replacement capital temporarily sustains activity; building growth alone is not healthy economic growth. |
| 188–192 | Last grocery closes day 188. By day 190 all 42 households have empty supplies while holding 103,898.72 in cash. Treasury turns negative on day 192. | Loss of supply/recovery is decisive even while households still have purchasing power. |
| 200–366 | All jobs are gone by day 200. Year ends with 12 households / 19 residents, no jobs, treasury −42,464.59. Remaining households hold 71,291.49 but all have empty supplies. | Continuing fiscal losses and missing businesses prevent recovery; aggregate household cash alone is a misleading health measure. |

Over the year the city collects **102,575.15** in taxes, pays **99,238.74** in transfers and
**103,441.00** in road upkeep. Recurring cash flow is **−100,104.59**, with 344 deficit days;
no further city construction is charged. Starting capital is separate. This fixture has no
municipal utility providers, so municipal payroll/input costs are zero. Its road footprint is
expensive relative to its small tax base; this is evidence for this fixture, not a universal
road-price conclusion.

Private businesses receive **380,310** of startup cash across repeated placements and suffer
**53 bankruptcies**. Actual gross private payroll totals **264,592.50**. Inputs below are net
cash commitments/refunds, not consumed-input margins or delivered inventory values:

| Profile | Sales, including liquidation | Input cash | Startup cash | Bankruptcies |
| --- | ---: | ---: | ---: | ---: |
| Food processing | 154,484.99 | 232,100.00 | 174,020 | 11 |
| Grocery | 173,174.75 | 128,444.99 | 44,100 | 2 |
| Machinery | 46,491.24 | 78,960.00 | 86,800 | 10 |
| Personal services | 3,302.87 | ≈0 | 26,880 | 16 |
| Health services | 3,634.62 | ≈0 | 48,510 | 14 |

**Specific growth blockers:**

- At day 10, two vacant household slots exist, but the candidate has estimated daily income
  40 against essential-cost estimate 58.60 and only 1.61 days of search runway. Admission
  acceptance is zero despite positive incoming need. This is the current planner's estimate,
  with the omissions described in the worksheet above, not a corrected affordability model.
- By year end, commercial pressure is **1.0**, yet the planner returns **zero placement
  candidates** and **zero despawn candidates**. All **32 standing commercial buildings are
  deserted**: two groceries, 16 personal-service stores and 14 health stores. Source inspection
  of the then-current `collect_existing_building_candidates` confirmed that deserted-building
  clearance required **negative growth pressure**. Thus unmet demand prevented the clearance
  needed to replace failed businesses. The isolated repair is recorded below.
- Industry then has no local intermediate demand and zero construction pressure. Household
  admission has zero incoming need, zero funded jobs and zero acceptance despite 11 vacant
  homes. Raising residential zoning or nominal job capacity does not address these conditions.

**Next repair identified by this run:** separate deserted-site recovery from negative demand,
then test viable startup imports, minimum staffing/market size and household affordability together. Require actual
sales, supplied households and recurring fiscal funding after startup capital and temporary
benefits. The report does not establish a final import multiplier, wage, tax or service size.

Accounting limits: private input cash is inferred from hourly sales minus operating-cash change,
where the existing runtime posts freight payments/refunds; `f32` rounding leaves sub-unit
residues (including approximately zero service inputs). Delivered input value is reported
separately; neither is consumed-input cost. Household bills are actual cash payments and shopping
reservations/refunds, so cash-limited spending must be read alongside stockouts. Payroll is the
actual aggregate private gross payment, reconstructed from net wages plus income tax, excluding
municipal wages. The observer also checks transfers lost from the UI ledger after household
removal; this run loses **0**, so the earlier source-ordering concern does not explain its deficit.

The headless observer uses deterministic ordered accumulation: O(B log B) per observed economy/
lifecycle boundary, O(H) per daily household capture, O(B + days × profiles) retained storage.
These scans/allocations exist only in the opted-in small scenario, never the live tick or 10k
benchmark. Validation and the matched live-path performance comparison are recorded below.

**Verification for this diagnostic extension (2026-10-05).** Fresh targeted unit checks pass:
103 household, 62 demand, five balance, three fiscal and three city-budget tests (**176 active**,
11 ignored targets/manual worksheets). Commands from the repository root:

```bash
cargo test --manifest-path rust/Cargo.toml --lib -- \
  economy::households::tests economy::demand::tests economy::fiscal::tests \
  economy::definitions::balance_tests nodes::sim::core::budget::tests
```

The scenario checks compare two concurrent diagnosed years, compare observed/unobserved daily
states over a full year, and attach the observer after day 40 to verify that existing capital
is not counted again. The healthy-growth assertion remains ignored by default and fails when
enabled; that economic failure is the subject of the report, not a passing growth acceptance.
City/business reconciliation errors round to 0.0000; the largest individual household residual
is 0.0215 currency from `f32` accumulation, below the 0.1 test tolerance.

Authored economy/demand/zoning data and bootstrap assets match. Both release binaries were built
from the repository root with `cargo bench --manifest-path <checkout>/rust/Cargo.toml --bench
economy_tick_benchmark -- /10k` and default target flags (the nested `rust/.cargo` CPU override
is not loaded from that working directory). Eight Rayon workers, no profiling, 23 hourly samples
and ten daily samples per process. Setup is outside timing. The fixture has 11,120 residents,
5,597 households and 7,396 buildings after its warm-up; all runs finish with identical recorded
state. Its large treasury is a timing-fixture property, not growth evidence.

The first pair flagged +13.7% hourly cost (2.204 → 2.507 ms); daily intervals overlapped.
This justified an alternating **baseline / changed / changed / baseline** repeat with saved
binaries, no simultaneous builds/tests, and shared Criterion output:

```bash
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-village-paired \
  $ARTIFACTS/metrum-village-head-bench /10k --save-baseline head_A --bench
# Same command with after-bench/after_A, after-bench/after_B, head-bench/head_B.
```

| Run | Setup, s | Hour mean, ms (95% interval) | Daily mean, ms (95% interval) |
| --- | ---: | ---: | ---: |
| Baseline A | 15.0 | 2.825 (2.626–3.026) | 818.8 (614.2–996.2) |
| Changed A | 14.6 | 2.141 (2.044–2.259) | 832.6 (625.7–1,017.0) |
| Changed B | 14.6 | 2.341 (2.183–2.513) | 850.1 (639.0–1,036.2) |
| Baseline B | 14.7 | 2.701 (2.436–2.973) | 814.4 (612.5–992.0) |

The hourly regression does not reproduce; process-to-process variation is larger than the
initial comparison implied. No consistent hourly slowdown is demonstrated, and daily confidence
intervals overlap. Do not claim a speedup or use these broad daily intervals as proof that
`ECON-13` performance work is complete. No larger tier was run.

Saved binary SHA-256 prefixes: baseline `8b4fa257985b0cc8`, changed `2e1ec6347b2c40f5`.

#### Failed-business recovery (`ECON-14`)

**Implemented 2026-10-06; economic acceptance still fails.** Deserted, empty commercial and
industrial buildings now receive demolition weight `1.0` at any demand pressure. The existing
hourly credits, `0.5` daily demolition fraction, deterministic deserted-first order and execution
safeguards remain authoritative. Staffed or occupied sites are protected. This recovery weight
does not enable contraction hysteresis for healthy buildings. Residential behavior is unchanged.

The implementation adds O(1) work per candidate to the existing parallel building scan and
combines pressure/hysteresis accumulation in the existing O(D) despawn reduction. Existing
candidate collection and sorting bounds remain unchanged; no new simulation allocation or
spatial structure was added. The village observer separately reports rebuilt-site startup cash
using the parcel's demolition generation; it is a subset of total startup cash, not sales. New
placements on previously unused parcels remain in total startup cash, so this subset alone does
not measure every replacement of a failed firm's economic activity.

The same 365-day village fixture was rerun before any tuning. The first processor still fails
on day 14; clearance fixes site availability, not operating losses:

| Measure | Original diagnostic run, 2026-10-05 | Recovery only, 2026-10-06 | Import-only experiment, rejected |
| --- | ---: | ---: | ---: |
| Import multiplier | 1.75 | 1.75 | 1.25 |
| Year-end households / residents | 12 / 19 | 22 / 35 | 22 / 37 |
| Year-end employed | 0 | 20 | 19 |
| Year-end households with empty supplies | 12 | 13 | 17 |
| Year-end treasury | −42,464.59 | 104.30 | 106.30 |
| First negative treasury day | 192 | 153 | 162 |
| Recurring city net over year | −100,104.59 | −57,535.70 | −57,533.70 |
| Private startup cash | 380,310 | 923,475 | 653,370 |
| Of which rebuilt-site startup cash | Not split in original report | 659,715 | 463,815 |
| Business bankruptcies | 53 | 95 | 76 |
| Grocery bankruptcies | 2 | 5 | 9 |

The recovery-only run ends with active businesses and available commercial sites, rather than
32 stranded deserted commercial buildings. Its 39 nominal job slots and 20 employed are not a
sustainable outcome: even the final 30 days need **92,015** of new private capital and record
**seven bankruptcies**. That period's recurring city net of **+117.39** is supported by the
circulation of startup money, while supplies remain unreliable. A positive final treasury does
not establish solvency; it already went negative during the year, and benefit payments become
cash-limited. The healthy-growth assertion remains ignored by default and fails when enabled.

The subsequent experiment changed only the import multiplier to the worksheet's `1.25` candidate.
It reduced total startup capital and industry closures but increased grocery closures from five
to nine and left 17 households without supplies at year end. It did not meet acceptance and was
**reverted at that stage**. The subsequent combined repair below retests 1.25× after correcting
the demonstrated staffing, production and physical-displacement defects.

**Follow-up at this stage:** measure actual grocery staffing/payroll against recurring sales and
pantry-replenishment demand; establish the paid market needed for a service to fund its first
worker plus overhead; test import costs together with those constraints and household budgets.
Industrial export losses remain separate evidence that local-demand volume is insufficient to
guarantee viability. Keep startup and rebuilt-site capital outside operating income. Additional
activities and broad demand simplification follow this foundation, as specified in the plan.

**Fresh correctness checks:** 180 targeted unit tests pass; 12 existing ignored targets/manual
checks remain ignored. The village suite passes concurrent-year determinism, complete accounting
comparison, observed/unobserved equivalence and cash reconciliation; its enabled growth target
fails. Maximum recovery-only residuals: city/business `0.0000`, household `0.0266` currency.

```bash
cargo test --manifest-path rust/Cargo.toml --lib -- \
  economy::households::tests economy::demand::tests economy::fiscal::tests \
  economy::definitions::balance_tests nodes::sim::core::budget::tests \
  buildings::allocator::tests::demand_actions
METRUM_GROWTH_REPORT=$ARTIFACTS/metrum-recovery-only-2026-10-06.txt \
  cargo test --manifest-path rust/Cargo.toml --test growth_scenario -- --include-ignored
```

**Fresh matched 10k performance:** pre-repair diagnostic binary SHA-256 prefix
`2e1ec6347b2c40f5`, repaired binary `3e763b5a4042b07b`. Both derive from `21100889` plus the
existing worksheet/diagnostic work; the repaired build additionally contains this recovery rule,
regression and replacement-capital reporting. Same root-invoked release configuration, unchanged
economy/demand data, eight Rayon workers, unprofiled runs with no concurrent builds/tests.
The fixture starts and ends identically: 11,120 residents, 5,597 households, 7,619 employed,
13,881 nominal jobs and 7,396 buildings. Setup is outside timing; the observer is disabled.

| Build | Setup, s | Hour mean, ms (95% interval) | Daily mean, ms (95% interval) |
| --- | ---: | ---: | ---: |
| Before | 14.6 | 2.491 (2.256–2.762) | 838.5 (630.2–1,020.5) |
| Recovery | 14.8 | 2.578 (2.410–2.762) | 820.0 (615.0–999.7) |

Criterion detects no significant change (hour `p=0.60`, daily `p=0.91`). This is a populated-city
cost comparison, not organic-growth acceptance or closure of the broad daily-cost work in
`ECON-13`. No larger tier was run.

```bash
cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-recovery-perf-before \
  $ARTIFACTS/metrum-village-after-bench /10k --save-baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-recovery-perf-before \
  $ARTIFACTS/metrum-recovery-after-bench /10k --baseline before --bench
```

The import experiment used the same scenario executable in a fresh process after editing only the
multiplier; the runtime loader reads authored data at process startup.

#### Grocery startup repair (`ECON-14`)

**Implemented 2026-10-06.** The earlier import-only failure had multiple causes. Stores counted
empty household pantries as production demand even when goods were already on retail shelves
or reserved for shoppers, hired from bulk daily sales spikes, and produced at rounded worker
capacity. The recovery-only trace reached about 605 units on grocery shelves while 13 households
had no supplies. Some household shoppers were unavailable because a job change stranded them
inside a former workplace; demolition could then leave them without any valid building location.

The repair uses recurring household consumption plus net pantry recovery, subtracting retail
inventory and reservations. Whole worker slots remain necessary, but production, operating input
demand and input stock targets follow the continuous demand rate. Former workers return home
through ordinary routing; demand demolition evacuates physical occupants before deleting their
origin. Existing removal/remapping, shipment and shopping-reservation lifecycles remain in use.

Imports change from **1.75× to 1.25×** with these fixes. Wages, retail prices, recipes, worker
capacities, transfers and initial treasury are unchanged. Service openings must cover the first
worker, current-policy property tax, imported utilities and recipe inputs from remaining market
demand. At this repair stage, personal/health service thresholds were 156/166 residents,
assuming paid demand; the later village-service tuning lowers them to 47/52. This check is necessary, not
a household affordability guarantee or a complete model of future sales.

The observer records paid wages before job reassignment, hourly production and sales units,
inventory, staffing, household shortage state, and startup cash independently of operating cash.
The initial 18,450 grocery float includes imported opening inventory funding; no later grocery
capital appears, including on previously unused parcels.

| Observed measure | First 365 days | Second 365 days |
| --- | ---: | ---: |
| Households / residents at end | 16 / 26 | 16 / 26 |
| Grocery sales | 248,963.91 | 244,173.47 |
| Grocery gross wages paid | 94,950 | 79,740 |
| Grocery operating cash change after purchases, wages and taxes | −3,024.36 | +14,764.16 |
| Grocery new startup cash | 18,450 | 0 |
| Grocery bankruptcies | 0 | 0 |
| Household-days with an empty pantry at settlement | 4 / 6,125 | 0 / 5,840 |
| Year-end grocery cash | 15,425.64 | 30,189.80 |
| Year-end city treasury | −51.00 | −123.65 |

The first year still uses working capital; its final 30 days generate about 1,035 operating cash
with no empty pantries. The second year demonstrates receipts covering grocery wages and other
costs without recapitalization. The active two-year regression checks no grocery bankruptcies,
no second-year startup funding, paid wages, positive second-year operating cash and no second-year
empty household pantries. Concurrent-year determinism, observer neutrality and cash reconciliation
also pass; maximum city/business residual is below 0.0001 and household residual below 0.001.

**This is grocery acceptance in this fixture, not a healthy city.** First-year industrial firms
still receive 345,780 startup cash and fail 29 times; their wages and cheap local inputs help
support grocery sales. City recurring net is −57,691 in year one. Year two still has industrial
recapitalization, no population growth and cash-limited transfers. Grocery staffing cliffs in the
worksheet remain; this result is not proof that every small population or zoning mix is viable.
The broader growth/solvency test still fails when enabled. Industrial sales, funded employment,
household affordability and municipal funding are the next work; neither `ECON-14` nor `DEM-02`
is complete.

**Complexity:** grocery aggregation reuses the existing ordered parallel O(H + B) reduction with
constant catalog size; per-building production/input and per-agent return decisions remain O(1).
Service opening adds O(profile ports) per candidate and one catalog utility-price scan per plan.
Evacuation reuses serial lifecycle mutation: O(A) per bounded demolition, already the removal's
complexity, plus route planning only for displaced occupants. Housed occupants avoid border
search. The richer O((H + B) log B + A) observer is opt-in and absent from live/performance runs.

Fresh targeted checks: **272 unit tests pass**, including employment, movement, shopping,
demolition/remapping, production, demand and fiscal regressions; 20 existing manual/target tests
remain ignored. The ordinary scenario suite has three passing tests; broader healthy growth
remains an explicitly ignored failing target.

```bash
cargo test --manifest-path rust/Cargo.toml --lib -- \
  economy::households::tests economy::demand::tests economy::fiscal::tests \
  economy::definitions::balance_tests economy::agents \
  buildings::allocator::tests::demand_actions nodes::sim::core::budget::tests
METRUM_GROWTH_REPORT=$ARTIFACTS/metrum-grocery-final-year.txt \
METRUM_GROWTH_LONG_REPORT=$ARTIFACTS/metrum-grocery-two-years.txt \
  cargo test --manifest-path rust/Cargo.toml --test growth_scenario -- --include-ignored
```

These are local artifacts; the acceptance tests and figures above are the durable record.

**Fresh matched 10k performance:** recovery-only binary SHA-256 prefix `3e763b5a4042b07b`
versus grocery repair `1defbd8b1b5abe26`. Both processes load the same current 1.25× catalog
(`profiles.toml` SHA-256 prefix `3fce7b69d4e13062`), isolating runtime changes from import tuning.
Both derive from `21100889` plus the existing ledger/diagnostics/recovery work; the latter adds
the grocery, service-opening and displacement changes above. Root-invoked unprofiled release
builds, eight Rayon workers, sequential runs with no concurrent builds/tests, observer disabled.

| Build | Setup, s | Hour mean, ms (95% interval) | Daily mean, ms (95% interval) |
| --- | ---: | ---: | ---: |
| Recovery baseline | 14.7 | 2.750 (2.554–2.971) | 823.26 (619.65–1,002.2) |
| Grocery repair | 15.5 | 2.130 (2.041–2.229) | 827.89 (621.96–1,008.1) |

Criterion reports hourly improvement of 22.6% (`p < 0.01`); daily change is not significant
(`p = 0.97`). The fixture is identical before/after: 11,120 residents, 5,597 households,
7,619 employed, 13,881 nominal jobs and 7,396 buildings, with matching treasury records. Each
sample is one tick: 23 operational-hour and ten daily-settlement samples, with setup excluded.
These figures validate this populated workload, not organic growth or worst-case demolition;
the broad daily cost remains `ECON-13` work. No larger tier was run.

```bash
cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-grocery-perf \
  $ARTIFACTS/metrum-recovery-after-bench /10k --save-baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-grocery-perf \
  $ARTIFACTS/metrum-grocery-after-bench /10k --baseline before --bench
```

#### Regional industrial markets (`ECON-14`)

**Implemented 2026-10-06, after the grocery repair at `c51973df`.** The delivered-freight trace
isolated industrial losses even with 1.25× imports. The baseline processor group paid 293,700
for delivered imports and 142,560 in wages while earning 87,360 from local deliveries and
118,522.50 from exports. Machinery paid 92,800 for imported inputs and 82,800 in wages, against
6,400 local sales and 76,800 exports. Other receipts included distress liquidation. Operating
cash fell by 228,089.49 / 97,112.07 respectively; 15 / 14 bankruptcies and 170,300 / 83,040 in
replacement capital concealed the recurring losses.

The repair replaces declining export bids with fixed catalog-price bids and **finite shared
regional purchase volumes**: food 480, grain 580, coal 160 and machinery 80 units/day. Unlisted
resources have no regional buyers. These are authored external customers, not resident-derived
consumption or a claim of unlimited regional purchasing power. They are a small starter market;
future regional contracts and added activities need their own funding and volume design.

| Factory at full operation | Saleable units/day | Sales at 1.0× | Imported external inputs | Payroll | Property + imported utilities | Pre-profit-tax margin | Break-even sales volume |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Food processor | 160 | 2,400 | 1,250 | 900 | 43.125 | +206.875 | >146.21 |
| Machinery | 40 | 800 | 200 | 360 | 43.125 | +196.875 | >30.16 |

Machinery's two units of own upkeep are already deducted from its 42-unit output. Input purchases
and inventory timing still matter; these paper margins do not replace delivered-sales evidence.
New factories must have enough remaining local/regional customers to cover full payroll,
imported external inputs and overhead. Existing and selected construction reserve capacity
against the same buyers. Extra border connections do not multiply market volume. Freight
continues to protect reachable local input requests and a producer's own input reserve.

The larger employment base exposed household lifecycle faults. Admission now requires ongoing
net wages plus funded pension/child support to cover consumption with VAT, imported utilities
and home property tax. Temporary unemployment benefits and starter cash cannot fund permanent
admission. Grocery pantry refill work is excluded from permanent job forecasts. Rehousing
resumes building ingress for residents whose old home disappeared; shopping deadlines include
estimated travel plus delay allowance, and shopping at one's workplace retains shopping purpose.
The existing persistent-exit rule recognizes insufficient daily reserves even when a small
transfer leaves positive cash. It retains the two-day grace and bounded departure cadence.

The observer separates local/export receipts, input purchases, delivered inputs, payroll,
startup capital and rebuilt-site capital. It also records eligible transfers before the treasury
cash limit, so a surplus cannot be explained by omitted benefit obligations. The original
UI benefit summary misses 10 currency on a departure day; acceptance uses the complete observer
ledger and reconciles it with actual treasury changes.

| Observed measure | First 365 days | Second 365 days |
| --- | ---: | ---: |
| Ending households / residents / employed | 36 / 58 / 43 | 36 / 58 / 43 |
| Food operating cash change, after all purchases and charges | +188,458 | +220,139 |
| Machinery operating cash change | +108,557 | +120,998 |
| Grocery operating cash change | +72,597 | +75,796 |
| Total new business startup capital | 71,590 | 0 |
| Of which rebuilt-site capital | 0 | 0 |
| Business bankruptcies | 0 | 0 |
| Empty household pantry-days at settlement | 20 | 0 |
| Eligible household transfers left unpaid | 0 | 0 |
| City recurring balance after transfers and road upkeep | +169,606 | +172,711 |
| Ending treasury | 227,246 | 399,958 |

The final 90 days remain cash-positive for every business profile and the city. The three food
processors and two machinery factories earn all recorded receipts from delivered local/export
sales, with no liquidation. The grocery serves ongoing resident demand. Initial infrastructure
costs 42,360 from the original 100,000 treasury; recurring road upkeep is 283.40/day. This town
can maintain that network and accumulate funds for later investment. The run does not build an
expanded infrastructure network or establish the affordability of every future public service.

The finite starter market fills, then population stabilizes. The previous ignored test required
more households in the second half of every year even without new customers. It is replaced by
an active two-year test requiring establishment, retained population/employment, profitable
second-year and trailing-quarter business operation without new capital, full benefits,
positive recurring city balances in each year, positive treasury and no second-year stockouts.
Concurrent-year determinism, observer neutrality and cash reconciliation remain separate tests.
The 20 first-year pantry-days are transient startup failures; none persists into the second year.
This validates the foundation, not the full diversification or demand-refactor roadmap.

**Complexity:** regional quotas add O(R) refill/storage and O(1) work per export to existing
O(S) shipment reservation construction. Candidate viability is O(profile ports); grocery
recurring-demand fields reuse the existing ordered O(H + B) reduction. The new reserve check
runs in the existing household reduction. Shopping adds one existing route-cost estimate per
accepted leg, none per frame. Agent arrival and rehousing changes are O(1) per affected agent.
Benefit-claim accounting adds one scalar per household ledger and O(1) work in its existing pass.
Delivered-trade diagnostics remain opt-in; the no-op live observer does not accumulate them.

**Fresh correctness verification:** 339 unit tests pass, including regional quota sharing/refill
and cancellation, profitable factory selection, recurring household admission, pantry-job
forecasting, field employment, benefits, shopping, arrival purpose, rehousing, demolition and
SQLite round-trip preservation of the export-volume vector. Twenty-five existing manual or
separate target tests are ignored. All three scenario tests pass, with no ignored growth target.
Cash residuals are below 0.0001 for the city/business ledgers and 0.007 for households. Changed
Rust files pass formatting and `git diff --check`. `cargo doc --no-deps` reports no missing-doc
warnings; its one unrelated existing broken link is `VegetationGenerator` in
`nodes/simulation_node/vegetation_api.rs:155`.

```bash
cargo test --manifest-path rust/Cargo.toml --lib -- \
  economy::demand::tests economy::definitions economy::logistics economy::households \
  economy::agents economy::fiscal::tests buildings::allocator::tests::demand_actions \
  buildings::allocator::tests::lifecycle nodes::sim::core::budget::tests \
  save::tests::sqlite_round_trip
METRUM_GROWTH_REPORT=$ARTIFACTS/metrum-industry-final-year.txt \
METRUM_GROWTH_LONG_REPORT=$ARTIFACTS/metrum-industry-final-two-years.txt \
  cargo test --manifest-path rust/Cargo.toml --test growth_scenario
```

The temporary files are local diagnostics; the checked-in assertions and figures above are the
durable acceptance record.

**Fresh matched 10k performance:** unprofiled release binaries, eight Rayon workers, sequential
runs with no concurrent builds/tests and the observer disabled. Baseline binary SHA-256 prefix
`1defbd8b1b5abe26` uses the grocery-repair catalog (`3fce7b69d4e13062`); the current binary
`aa92b9d7b5222a0c` uses the regional-market catalog (`4a845a36a7e6c058`). Both derive from
`c51973df`; the current binary adds the work described above. Catalogs intentionally differ in
trade rules: this measures the complete change, not an isolated quota-lookup microbenchmark.

| Build | Setup, s | Hour mean, ms (95% interval) | Daily mean, ms (95% interval) |
| --- | ---: | ---: | ---: |
| Grocery baseline | 15.2 | 2.753 (2.531–2.988) | 835.82 (626.41–1,019.0) |
| Regional-market repair | 15.2 | 2.675 (2.495–2.857) | 843.37 (634.65–1,026.6) |

No significant change is detected (hour `p=0.61`, daily `p=0.96`). Each sample is one tick:
23 operational hours and ten daily settlements. Both runs build and retain 11,120 residents,
5,597 households, 7,619 employed, 13,881 nominal jobs and 7,396 buildings, with matching treasury
records. The 10k fixture deliberately seeds a populated city; it does not measure organic
establishment or establish worst-case routing costs. The broad daily cost remains `ECON-13`
work. No larger tier was run.

```bash
cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run
# Each binary loads its recorded catalog; restore the current catalog after the baseline run.
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-industry-perf \
  $ARTIFACTS/metrum-grocery-after-bench /10k --save-baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-industry-perf \
  $ARTIFACTS/metrum-industry-after-bench /10k --baseline before --bench
```

The current catalog was restored and its hash checked.

### Village services and producer audit (`ECON-14`)

This evidence predates the SI conversion below; its quantities use the previous abstract units.
See [resource units](#resource-units-and-freight-payload) for current kg rates and market volumes.

**2026-10-06, continuing the regional-market repair.** The target is that roughly 60 residents
support the existing barber and pharmacy. Demand remains funded and resource-specific: raising
nominal worker capacity or permitting a loss-making shop is not the solution. Wages, retail unit
prices and the first-worker viability gate are preserved; personal-service consumption changes
from 0.03 to 0.10 units/resident/day and pharmacy consumption from 0.05 to 0.16.

| Shop | Full spending at 60 residents/day | One worker + property + imported utilities/day | First viable whole population |
| --- | ---: | ---: | ---: |
| Barber | 120.00 | 93.125 | 47 |
| Pharmacy | 134.40 | 115.625 | 52 |

Actual payments remain cash-limited, with no guaranteed revenue. Full consumer spending is now
34.99/resident/day (groceries, grocery VAT, both services and imported utilities), plus 2/home
property tax. One industrial worker with one child has 17.22/day surplus at the current wages
and child support; a single barber worker has 15.81/day surplus. This does not fix every household
composition: a lone pensioner receiving 30/day still cannot afford the full 36.99/day basket.
The fractional closed-chain employment baseline rises from 130.55 to **139.55 jobs/1,000**;
exports and public jobs remain outside that calculation.

The two-year starter run opens both services at 60 residents and settles at 58 residents,
36 households and **45 employed / 63 nominal jobs**. Each shop employs one worker. Year-two
operating cash is +5,719 for the barber and +2,685 for the pharmacy; the last 90 days are also
positive. All five business profiles survive with no replacement capital or year-two startup
injections. Year-two pantry stockouts are zero, all eligible transfers are paid, and recurring
city income is +197,111. Actual service receipts are below the full-spending figures above;
passing these checks does not mean every household pays its complete desired basket.

**Producer replenishment fault.** The first 180-day placement matrix found that every producer
stopped receiving machinery after its opening 80 units. A one-hectare mine initially earned a
profit but failed with 57,600 coal still underground. Local freight kept timing out: route costs
were movement seconds but ETA conversion divided by 3,600, even though an operational hour is
60 movement seconds under the authored 1,440-second day. ETA now uses `24 / seconds_per_day`,
with outside-window operational-minute penalties added afterwards. Existing bounded transit and
return deadlines, physical deliveries and refunds remain authoritative. The isolated rerun
restored farm/mine replenishment without changing yields, wages or input prices.

**Area investment and utility upkeep (historical capped-market rates).** Grain staffing, yields,
utility rates and minimum area below are superseded by the
[production/freight revision](#production-and-freight-economics-econ-14). Player-drawn production sites have two independent
size costs. The building footprint retains its existing placement cost; every successfully added
hectare charges the city treasury for site preparation. Recommitting the same area or shrinking
costs zero and grants no refund. Expanding after a shrink pays for the newly added area again.
Invalid edits change neither money nor the committed site. The existing treasury policy permits
a negative construction balance; these charges are not an affordability gate.

| Site | Building footprint | Preparation / added ha | Extra power / water / sewage per ha/day | Imported utilities/day at 1 ha / 4 ha / 100 ha |
| --- | ---: | ---: | --- | ---: |
| Grain farm | 22,500 | 20,000 | 20 / 160 / 20 | 520.625 / 2,058.125 / 51,258.125 |
| Coal mine | 50,000 | 25,000 | 40 / 10 / 10 | 201.875 / 783.125 / 19,383.125 |

These are authored balance starting points in `profiles.toml`, not real-world prices. Utility
demand is `1 + hectares × area_utility_units_per_hectare` for each service, in power/water/sewage
order. Power demand consumes measured local production capacity; water/sewage retain the existing
provider-presence coverage rule. All three pay existing local/imported prices, and local receipts
reach the municipal provider ledger. Demand is independent of staffing, input stock,
output headroom and customer demand. An idle operational site therefore still pays. Depleted
mines have zero productive area and return to the base building utility bill; desertion stops
normal operation. Wages and machinery consumed by production are separate expenses.

At 100 ha, preparation alone is 2,000,000 for a farm or 2,500,000 for a mine. Grain buyers in this
village can absorb at most 480 local + 580 regional units/day (about 3.66 full-output hectares);
coal exports are limited to 160/day. Larger areas therefore continue raising investment and
utility costs after customer income stops growing. Both drawing and field-resize previews show
the construction charge and all-import daily utility estimate from Rust before commit.
Farms and mines can both export through a reachable OWA road border; local consumption is not
required. The 580 grain/day and 160 coal/day outside markets are shared across all producers and
refill hourly. These fixed starter volumes do not grow with field area or population. Expanding
regional markets for larger export industries remains design work under `ECON-14`.

The read-only observer includes explicit sites, uses position/generation identities for sites
without a parcel, and records area, staffing and remaining/cumulative extraction. Opening cash
is separate from subsequent operating cash. The producer matrix explicitly endows each case with
its city construction capital before observation, so it tests business operation rather than
pretending a fresh 100,000 treasury can finance a 100-ha project. No later city or business
capital is injected. Ordinary cases retain the 2-km fixture; 100-ha cases use a 4-km world to fit
the parcel west of the same town. These fixtures remain small-population diagnostics.

Runtime complexity is unchanged: constant-sized stack arrays and O(1) arithmetic in the existing
daily utility scans; O(log S) site lookup and O(1) charging on area edits. No new per-agent work,
spatial index, save field, or per-tick allocation is added. Preview geometry uses the existing
polygon validation and local site queries. Existing saved areas rebuild their cached hectare
scale without retroactively charging construction.

**Fresh 730-day producer results.** Operating cash excludes opening private capital and city
construction/endowment. A closed site's zero trailing cash flow means it has stopped operating,
not that it recovered. The 180-day checks separately establish positive operating cash, repeated
input purchases and paid workers for both one-hectare producers.

| Site | Total operating cash | Last 90 days | Producer outcome | Last-90 empty household-days |
| --- | ---: | ---: | --- | ---: |
| Farm 0.1 ha | −3,687 | 0 | Bankrupt | 120 |
| Farm 1 ha | +643,437 | +79,302 | Operating | 0 |
| Farm 4 ha | +2,627,045 | +325,260 | Operating | 90 |
| Farm 20 ha | −78,230 | 0 | Bankrupt | 0 |
| Farm 100 ha | −389,909 | 0 | Bankrupt | 0 |
| Mine 0.25 ha | −3,707 | 0 | Bankrupt | 120 |
| Mine 1 ha | +72,951 | −3,881 | Reserve depleted; remaining overhead | 90 |
| Mine 4 ha | −20,269 | 0 | Bankrupt | 0 |
| Mine 100 ha | −494,641 | 0 | Bankrupt | 150 |

The one-hectare mine physically extracts and sells its complete 60,000-unit deposit to OWA;
its final-quarter deficit is the base utility/property bill after extraction ends. No producer
receives fresh capital during these runs. All nine city recurring balances are positive in the
last quarter, and city/business/household cash reconciliation errors stay below 0.1.
Tiny sites fail the fixed-cost floor; large sites can exceed paying demand despite adequate
reserves. The profitable four-hectare farm still has a high return relative to its setup cost:
these rates close the oversized-site exploit, not every longer-term investment-balance question.
Pantry shortages in several variants and pensioner affordability remain unresolved. The passing
starter village therefore does not establish healthy growth for all town compositions.

**Fresh correctness checks:** 369 targeted unit checks pass (368 in the broad run, then the
updated startup-runway expectation in a focused rerun); 30 manual/separate tests remain ignored
in that broad selection. The three growth integration tests pass, including concurrent-year
determinism, observer neutrality and two-year recurring-income acceptance. The nine-case
730-day producer matrix also passes its accounting assertions. Both successful-area charging
and rejection without charges are covered, as are partial local power supply and imported
area utilities on unstaffed sites. The isolated Godot field-edit/placement-preview regression
passes against the freshly built release extension. Changed Rust files pass `rustfmt --check`
and `git diff --check`. Rustdoc reports no missing docs; its existing unrelated
`VegetationGenerator` link warning remains at `nodes/simulation_node/vegetation_api.rs:155`.

```bash
cargo test --manifest-path rust/Cargo.toml --lib -- \
  economy::demand::tests economy::definitions economy::logistics economy::households \
  economy::agents economy::fiscal::tests buildings::allocator::tests::demand_actions \
  buildings::allocator::tests::lifecycle nodes::sim::core::budget::tests \
  nodes::sim::core::scenario::balance_cases nodes::sim::core::tests::fields \
  simulation::extraction::tests simulation::agriculture::tests simulation::work_area::tests \
  save::tests::sqlite_round_trip
METRUM_GROWTH_REPORT=$ARTIFACTS/metrum-area-costs-year.txt \
METRUM_GROWTH_LONG_REPORT=$ARTIFACTS/metrum-area-costs-two-years.txt \
  cargo test --manifest-path rust/Cargo.toml --test growth_scenario
METRUM_BALANCE_DAYS=730 METRUM_BALANCE_REPORT_DIR=$ARTIFACTS/metrum-area-costs-730 \
  cargo test --manifest-path rust/Cargo.toml --lib producer_balance_matrix -- --ignored --nocapture
```

**Fresh matched 10k performance:** sequential, unprofiled release runs with eight Rayon workers,
no concurrent builds/tests and diagnostics disabled. Both binaries use the same current catalog
(SHA-256 prefix `cf693add3409fe98`). The regional-market baseline binary is
`aa92b9d7b5222a0c`; the current binary is `a92d599359619f00`, both derived from `c51973df` plus
their recorded worktree changes. Setup is outside the tick timing.

| Build | Setup, s | Hour mean, ms (95% interval) | Daily mean, ms (95% interval) |
| --- | ---: | ---: | ---: |
| Regional-market baseline, current service catalog | 16.3 | 2.826 (2.619–3.040) | 850.54 (637.38–1,034.3) |
| Area costs and freight clock repair | 15.6 | 2.672 (2.437–2.905) | 818.26 (615.65–996.16) |

No significant timing change is detected (hour `p=0.36`, daily `p=0.83`). Both fixtures build and
retain 11,120 residents, 5,597 households, 7,619 employed, 13,881 nominal jobs and 7,396 buildings.
Opening treasuries match; final treasuries differ by 545.4, so the changed freight deadlines are
not presented as transaction-identical behavior. The fixture contains zoned businesses, not
explicit farms/mines: it checks the shared utility scans and freight runtime, while the small
producer scenarios check area economics. Wide daily timing intervals and overall daily cost
remain `ECON-13` work. Only the 10k tier ran.

```bash
cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-area-costs-perf \
  $ARTIFACTS/metrum-industry-after-bench /10k --save-baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-area-costs-perf \
  $ARTIFACTS/metrum-area-costs-after-bench /10k --baseline before --bench
```

The catalog was unchanged between the runs.

### Unit and payload validation (`ECON-14`)

**2026-10-06, after the area-cost audit above.** The [unit contract](#resource-units-and-freight-payload)
converts upstream quantities and prices together; it does not claim identical floating-point
transaction timing. Fresh tests check canonical measures, mass conservation, full-cycle sales,
batch/payload independence, local/import/export cargo and payment conservation, and format-70
save round-trip/legacy conversion. Oversized saved freight is rejected explicitly.

The broad relevant Rust run has **350 passing tests, 26 existing ignored tests and two failures**.
Both failures are in `buildings::allocator::tests::demand_selection`:
`test_startup_demand_residential_family_selection_uses_parcel_hash_order` and
`test_startup_demand_residential_variant_selection_uses_parcel_hash_order`. Their startup fixture
constructs no residential building before the selection assertion. Both fail identically on an
isolated reconstruction of the pre-unit source (`c51973df` plus the recorded area-cost patch,
with the exact pre-unit catalog), so they are existing startup/selection work under `DEM-02`.
No unrelated selection-test or gameplay change is included in this pass.

All **three growth integration tests pass**: concurrent-year determinism, observer neutrality,
and the two-year healthy village. The latter retains 58 residents, 36 households, 45 employed and
63 nominal jobs, no bankruptcies/replacement startup cash, no second-year pantry stockouts and
all eligible transfers paid. Year-two recurring city cash is **+199,536**; barber and pharmacy
operating cash are **+5,719 / +2,685**. These replace the previous run's fiscal total, not its
historical evidence. The targeted producer checks still pass for one-hectare ongoing operation,
area charging and oversized-site failure under the retained buying caps.

The release extension builds; isolated Godot `economy_machinery_test.gd` passes catalog loading,
kg/tonne formatting and stock/recipe/household unit labels. Rustdoc adds no missing-doc warnings;
the existing `VegetationGenerator` broken link in `vegetation_api.rs:155` remains. Changed Rust
files pass rustfmt, and the worktree diff passes whitespace checks.

```bash
cargo test --manifest-path rust/Cargo.toml --lib -- \
  economy::definitions economy::logistics economy::households economy::demand \
  economy::agents::tests buildings::allocator::tests agriculture::tests extraction::tests \
  work_area::tests save::tests nodes::sim::core::scenario::balance_cases
METRUM_GROWTH_REPORT=$ARTIFACTS/metrum-units-year.txt \
METRUM_GROWTH_LONG_REPORT=$ARTIFACTS/metrum-units-two-years.txt \
  cargo test --manifest-path rust/Cargo.toml --test growth_scenario
cargo build --manifest-path rust/Cargo.toml --release
cargo doc --manifest-path rust/Cargo.toml --no-deps
XDG_DATA_HOME=$ARTIFACTS/metrum-area-costs-godot-dwh9yb30/data \
XDG_CONFIG_HOME=$ARTIFACTS/metrum-area-costs-godot-dwh9yb30/config \
  godot --headless --path $ARTIFACTS/metrum-area-costs-godot-dwh9yb30/project \
  --script res://tests/economy_machinery_test.gd
```

**Fresh matched 10k performance.** Sequential unprofiled release processes, eight Rayon workers,
no concurrent builds/tests, setup outside timing. Baseline executable SHA-256 prefix
`a92d599359619f00` uses the pre-unit catalog `cf693add3409fe98`; the unit/payload executable
`c7331b86c4188907` uses `df0f63be8ea5fd25`. Both start from `c51973df` plus their recorded
worktree changes. The catalog conversion is part of the change being measured.

| Build | Setup, s | Hour mean, ms (95% interval) | Daily mean, ms (95% interval) |
| --- | ---: | ---: | ---: |
| Before units/payload | 15.7 | 2.980 (2.795–3.166) | 814.63 (613.99–991.79) |
| With units/payload | 15.8 | 2.853 (2.692–3.024) | 809.06 (608.61–984.61) |

No significant timing change is detected (hour `p=0.33`, daily `p=0.98`). Both runs retain
11,120 residents, 5,597 households, 7,619 employed, 13,881 nominal jobs and 7,396 buildings;
opening and final treasury records also match. Only the 10k tier ran. The fixture exercises the
shared economy/freight path; it is not proof of viable large farms or of a freight market without
fixed buying caps. Wide daily timing intervals and daily settlement cost remain `ECON-13` work.

```bash
cargo bench --manifest-path rust/Cargo.toml --bench economy_tick_benchmark --no-run
# Baseline ran before changing the catalog; the preserved binary alone does not pin its data.
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-units-perf \
  $ARTIFACTS/metrum-area-costs-after-bench /10k --save-baseline before --bench
RAYON_NUM_THREADS=8 CRITERION_HOME=$ARTIFACTS/metrum-units-perf \
  $ARTIFACTS/metrum-units-after-bench /10k --baseline before --bench
```

No per-trip transport-price calculation is added.

### `ECON-13` economy tick baseline

**Current scope, 2026-10-05: in progress at 10k residents.** Capture matched, unprofiled release
hourly and daily runs for economy changes, recording build/workload identity, worker settings,
setup cost, distributions and artifacts. Diagnose relevant slow daily ticks separately from
ordinary hourly cost. Historical 100k/1M work is not a gate for the healthy-growth redesign.
The 1M benchmark run is deferred; do not schedule it as the next economy task.

Exit for this iteration: a reproducible 10k baseline and matched comparisons for changed runtime
paths, with regressions explained and resolved. The historical 9.5 ms hourly result is evidence
for its original build, not a current acceptance measurement. A populated timing fixture does
not demonstrate organic growth, business survival, or city solvency.

**Benchmark city.** `PopulatedCity::build(residents)` (`rust/src/nodes/sim/core/populated_city.rs`)
builds a square city sized for the requested residents (about 2,800 per km², plus 8% headroom):

- Streets run every 50 m, with back-to-back lots between them, and cross streets every 400 m.
  Roads are added through the bulk path of the in-game benchmark city; cross streets go in one
  block at a time so rows never split a city-length road (`ROAD-52`).
- `OWA` trunks run from the world edge along evenly spaced row streets, one per 10,000
  residents, alternating west and east (since 2026-10-10; 10k keeps its single trunk along the
  middle street from the west). A 100k city with one trunk fulfilled about 2,000 of its
  15,000–45,000 daily imports, so its firms starved; roads are the only limit on trade.
- Streets are zoned with the player's brush in a repeating pattern of eight rows: six
  residential, one commercial and one industrial.
- Every lot is built through the demand spawn executor with the `godot/bootstrap/mods` pack, and
  construction is completed.
- Households are admitted through demand admission until the resident count is reached. Their
  carriers are placed at home instead of driving in. This is the only step that bypasses the
  game.
- The treasury is effectively unlimited, so city services stay funded in every tier.
- Two simulated days run before timing starts.

The city then runs frame by frame at maximum speed, as the growth scenario does, so agents move
and shopping, service and freight trips finish. Only the economy ticks are timed.

**Benchmark.** `rust/benches/economy_tick_benchmark.rs` builds each tier once, outside the timed
region, and keeps advancing the city hour by hour. Only the tick under test is timed, and every
sample is one tick:

- `operational_hour`: each of the 23 demand-pass hours of one day.
- `daily_settlement`: ten consecutive days.

In the original 10k/100k records, household, resident, employment, job and building counts
were unchanged across the short runs. This does not establish financial stability. Use the
explicit 10k filter:
`cd rust && cargo bench --bench economy_tick_benchmark -- /10k`. The unfiltered harness still
includes larger tiers; those runs are deferred. Criterion warns that the target time is too short;
that is intended.

#### Live-economy fixture (`ECON-18`, 2026-10-09)

Before 2026-10-09 the fixture ticked the economy without moving agents, and its one border took
only four active and four queued freight jobs. No shopping, service or freight trip finished.
Grocery imports held all eight border slots. About 1,190 firms sat on freight-retry cooldown,
and the 459 food processors and 440 machinery factories (about 6,350 jobs) never bought inputs or
exported. They paid wages from startup cash until the funding census ran out; once `ECON-17`
payroll followed the census, employment fell from 7,260 to 55 by day 18. Timings recorded before
this date measure that collapsing city and are not comparable with later records.

The fixture now steps `SimCore::step_frame`, and `OWA` freight is uncapped
([border connections](#border-connections-and-physical-freight)). Over days 4–19:

- Employment stays between 6,945 and 7,261.
- Processors and factories deliver 11,805 export loads and 6,230 import loads.
- Groceries restock from local processors.
- 36 food processors fail and are replaced.

Two shortfalls remained:

- Grocery output was capped by the citywide activity floor (`DEM-02`): about 3,400 units a day
  against 10,625 consumed. About 3,500 of 5,320 households had empty pantries, most in terminal
  replenishment failure. Fixed 2026-10-10: each store now follows its own funded census
  activity ([store staffing](#building-throughput-depends-on-staffing)). Output is about 10,300
  a day, and empty pantries fall to 169 by day 19.
- In one benchmark run, 5,530 of 13,768 freight jobs timed out in transit. Diagnosed 2026-10-10
  as road throughput: every new firm orders its opening inputs from the one border at once
  (about 2,240 carriers in the first hour), and all of them share one trunk road. Fulfilled
  border imports plateau at about 20–70 an hour, jobs left waiting expire six hours after each
  dispatch wave, and their retries form the next wave. In the current build 4,941 of the 5,012
  timeouts fall on days 0–2 and later days see 0–29. Roads are the intended limit on trade;
  giving the fixture more border roads would change the workload and is left open.

**Baseline.** `dc87e396` plus the `ECON-18` changes, release `bench` profile,
`RAYON_NUM_THREADS=8`, `taskset -c 0,2,4,6,8,10,12,14`. Four sequential unprofiled runs of
`cargo bench --bench economy_tick_benchmark -- /10k`, with identical city records in every run.
The build took 18.0 s.

| Benchmark | Mean of run means | Run-mean stdev | Sample median | Sample max |
| --- | ---: | ---: | ---: | ---: |
| `operational_hour` | 152.10 ms | 0.85 ms | 204–209 ms | 263–273 ms |
| `daily_settlement` | 17.93 ms | 0.09 ms | 9.0–9.6 ms | 78–81 ms |

**Where the hour goes.** One run with `METRUM_DEBUG=1 METRUM_DEBUG_FILTER=economy`:

- The household hour takes 132 of the 151 ms. Logistics takes 11 ms, mostly shipment progress,
  and the workplace pass 7 ms.
- The household hour follows the number of grocery candidates examined (correlation 0.992, about
  7.3 µs each). Candidates average 14,253 an hour and peak at 32,664.
- Each candidate is route-checked out and back through the live trip planner before its stock
  is tested. In one peak hour, 28,545 of 29,784 candidates were then rejected as sold out.

Testing stock before routing, or skipping sold-out stores, was the next `ECON-13` candidate.
Either changes which candidates are kept; the 2026-10-10 walking-proof check instead keeps every
candidate and makes the route checks themselves cheap (below).

**`ECON-19` and store activity (2026-10-10).** Same settings; three executables in rotated order,
four runs each: the baseline above, `85228b40` (`ECON-19` stock-check offsets) and the `DEM-02`
store-activity change. Each executable's city records are identical across its runs. p is an
exact permutation test over the four run means.

| Benchmark | Baseline | `ECON-19` | Store activity |
| --- | ---: | ---: | ---: |
| `operational_hour` mean | 150.62 ms | 169.59 ms (+12.6%, p = 0.029) | 106.23 ms (−37.4% vs `ECON-19`, p = 0.029) |
| `operational_hour` max sample | 259–265 ms | 206–217 ms | 152–160 ms |
| `daily_settlement` mean | 17.54 ms | 25.50 ms (+45.4%, p = 0.029) | 9.89 ms (−61.2% vs `ECON-19`, p = 0.029) |
| Day 15 households / employed | 5,320 / 7,156 | 5,320 / 7,129 | 5,330 / 7,166 |

Even check slots lower the worst hours but raise the mean hourly and daily cost; that increase was
not attributed further, because the store change removes it. With groceries producing what
households buy, fewer shoppers search sold-out stores, and the store build is 29.5% below the
baseline hourly and 43.6% below it daily. The service-store follow-up (same settings, four
alternating runs against the store build) measures **105.85 ms hourly** (p = 0.60, no change) and
**8.81 ms daily** (−11.1%, p = 0.029), with a largest hourly sample of 157–161 ms. The
explicit-work-area follow-up leaves timing unchanged. The walking-proof shopping check then answers
most grocery route checks from cached pedestrian components with identical results: **34.18 ms
hourly** (−67.6%, p = 0.029), largest sample 56–59 ms, and **8.60 ms daily**. This is the current
10k baseline.

**Workplace search legs (2026-10-10).** Cost-only car legs and the shared network-leg table ([job
search](#agents-supply-labor)) against `24705c42`; same settings, four alternating runs each. Every
run of both builds ends with identical `GrowthDayRecord`s (day 15: 5,341 households, 7,134
employed). Hourly time is **34.93 → 34.45 ms** (−1.4%, p = 0.20) with a largest sample of 56–60 ms
in both; daily time is **8.74 → 8.73 ms** (p = 0.89). Neither change is significant: the 10k
workplace pass is a few milliseconds of the hour. The 100k effect is recorded under
[`DEM-02`](demand_history_2026-10-10.md#growth-redesign-dem-02).

#### Historical larger-city builds and initial timings

The following 2026-10-02–04 measurements and projections are retained for their original builds.
They do not expand the current 10k acceptance scope. Initial ECON-13 timings were single runs;
the later matched logistics comparison is recorded under `ECON-15`.

1M build, 2026-10-04. A temporary build-only probe, since removed, ran
`PopulatedCity::build_timed(1_000_000)`. Release, `a4635166`, 24 logical CPUs, default Rayon
workers, single run. The projection comes from the 30k–300k `populated_city_scaling` run.

| Phase | 1M measured | Projected |
| --- | ---: | ---: |
| Street grid | 1,032 s | 711 s |
| Zoning | 325 s | 427 s |
| Lots | 311 s | 330 s |
| Admission | 646 s | 631 s |
| Warm-up, 48 hours | 39 s | 26 s |
| Total | 2,353 s | 2,125 s |
| Peak RSS | 16.5 GB (11.5 GB after build) | about 10 GB after build |

Result: 1,104,604 residents, 553,789 households, 753,750 employed, 1,436,219 jobs and 742,165
buildings. The street grid grew faster than the 1.34 exponent fitted at 300k (`ROAD-52`). The
admission peak was about 5 GB above the post-build RSS, which the scaling test did not sample
then (`ECON-16`).

Scaling after `ROAD-52` and `ECON-16`, 2026-10-04. `POPULATED_CITY_TIERS=30000,100000,300000
cargo test --release --test populated_city_scaling -- --ignored --nocapture`; release, default
Rayon workers, single run. Each tier now records its build peak (`VmHWM` after resetting it). The
cross-street order changed the city, so these records are not comparable with earlier rows:

| Tier | Build | Street grid | Peak RSS | RSS after build | Buildings |
| --- | ---: | ---: | ---: | ---: | ---: |
| 30k | 30.5 s | 6.6 s | 684 MB | 526 MB | 20,368 |
| 100k | 123 s | 22.6 s | 1,915 MB | 1,333 MB | 76,133 |
| 300k | 393 s | 66.5 s | 4,750 MB | 3,386 MB | 221,095 |
| 1M projected | about 24 min | about 220 s | about 12.9 GB | about 9.4 GB | |

The street grid, zoning and lots now grow linearly (exponent 0.98). Admission (1.26) and the
daily settlement (1.21) still grow faster than the city. The projected 1M peak is below the
16.5 GB measured on the old city, so the 300k fit still underestimates memory at 1M.

1M runtime, 2026-10-03. Memory now fits, but time does not. Phase timers on the 10k, 30k and
100k builds (temporary, not committed; release, default workers):

| Build phase | 10k | 100k | 1M projected | Owner |
| --- | ---: | ---: | ---: | --- |
| Roads, zoning paint, lot preparation | 8 s | 86 s | about 15 min | linear |
| Building spawns | 1.7 s | 175 s | about 5 h | `ALLOC-02`, fixed: 9.7 s at 100k, about 1.6 min at 1M |
| Household admission | 1.5 s | about 132 s | about 3.5 h | `ALLOC-03` and `ALLOC-04`, fixed: border route queries 33.6 s → 0.15 s at 100k |
| Admission hours (two ticks) | 2.6 s | 48 s | about 10 min | |
| Warm-up, 48 hours | 1.7 s | 90 s | about 2 h | `ECON-15`, fixed: 100k hour 1.47 s → 29 ms, about 15 s at 1M |
| Peak RSS | 434 MB | 1.9 GB | about 20 GB | `ROAD-44` |

The benchmark then ticks 263 more hours (23 hourly samples and 10 settlement days), which
projected to about 10 hours at the old 150 s hour. The 1M tier waited for `ALLOC-02`, `ALLOC-03`
and `ECON-15`, fixed in that order before one 1M run (decided 2026-10-03). All of these are done,
along with `ALLOC-04`:

- The 100k build fell from 505 s to 177 s, with identical records.
- The 1M hour projects to about 0.3 s.
- The then-planned 1M tick run did not complete; it is deferred by the 2026-10-05 scope decision.

Build: branch `economy` at `522aae16` plus uncommitted `ECON-13` changes, `bench` profile, 24
logical CPUs, 64 GB RAM, default Rayon workers.

| Tier | Residents / employed / jobs | Buildings | Build | Peak RSS | `operational_hour` | `daily_settlement` |
| --- | --- | ---: | ---: | ---: | --- | --- |
| 10k | 11,143 / 7,634 / 14,009 | 7,428 | 17.4 s | about 1.5 GB | 17.5 / 19.2 / 21.2 ms | 16.8 ms / 327 ms / 771 ms |
| 100k | 110,867 / 76,628 / 141,573 | 75,965 | 518 s | 14.8 GB | 1.60 / 1.62 / 1.63 s | 212 ms / 5.04 s / 12.0 s |
| 1M | 1,104,604 / 753,750 / 1,436,219 | 742,165 | 2,353 s | 16.5 GB | not run | not run |

The 1M row is the 2026-10-04 build-only run above, at `a4635166`. Times are Criterion's lower
bound, estimate and upper bound. The daily range is very wide: some
days settle in tens of milliseconds and others take seconds. Why some days are slow has not been
diagnosed yet.

Findings so far:

- **Job search routed every home to every workplace (fixed).** An hour that founded 5,000
  households at 10k ran 50.5 million network queries, about 10,700 per home: every home routed
  every workplace in the city, with up to eight queries per pair. The hour took 232.6–234.7 s.
  - Two changes fixed it. The network query's search labels now use a fixed-seed `foldhash`
    hasher instead of SipHash (`rust/src/simulation/pathing/cch.rs`), which took the hour to
    112 s. The exact pruning bounds described under [Agents supply labor](#agents-supply-labor)
    took it to 1.7 s and 2.29 million queries.
  - The resulting city is identical: 5,617 households, 11,143 residents, 7,634 employed and
    14,009 jobs, both before and after. Building the 10k city fell from about 290 s to 17.4 s.
  - These are single release runs of the same workload, not matched acceptance runs.
- **Logistics dominated the hourly tick and grew faster than the city (`ECON-15`, fixed).** At
  30k residents the hour took about 90 ms, of which `logistics.hourly_tick` was 87 ms. The hour
  was 19 ms at 10k and 1.6 s at 100k, close to quadratic.
  - Supplier searches that found nothing scanned every supplier.
  - With the fix, the hour is 9.5 ms at 10k and 29 ms at 100k, with identical records.
- **Road-surface caches set the memory ceiling (`ROAD-44`).** At 10k, 1.32 GB of the city's
  1.34 GB of live heap is retained road-surface visual output. The rest of the city takes about
  30 MB, buildings, households and zoning included. At the 1M layout that projected to about
  110 GB. `ROAD-44` progress (2026-10-03) cut 100k peak RSS to 2.0 GB (from 14.8 GB), with identical
  city records and tick times.

### `ECON-15` logistics hourly scaling

Done, 2026-10-03. At 30k residents, `ShipmentSystem::hourly_tick` was 87 of the 90 ms
operational hour. The hour grew from 19 ms at 10k to 1.6 s at 100k, which projected to about 150 s
per hour at 1M.

**Cause.** Local supplier searches that never found a supplier walked the whole supplier set.
`find_local_supplier` scans supplier chunks outward from the buyer, and the scan stops at a ring
only once it holds a choice whose travel time beats the next ring's lower bound. When no
supplier of the resource could ship, the scan never held a choice, so it visited every supplier
of that resource in the buyer's freight component. Temporary counters in the 10k city found
1,209 searches per hour scanning 547,677 suppliers, none with stock. The exports hold pass
(`reserve_reachable_local_input_holds`) then repeated the same searches. Requests and suppliers
both grow with the city, so the hour grew with their product.

**Fix.** `SupplierCandidateIndex::build` (`logistics/supplier_index.rs`), rebuilt once per hour
in the planning context, lists a supplier under a resource only while its unreserved saleable
stock fills one resource batch (`fills_one_batch` in `logistics/quantization.rs`). A smaller stock
quantizes to nothing for every request, so the search would skip that supplier anyway.

**Why results are unchanged.** Planning only adds reservations and never adds stock. The index
built at the start of the pass therefore still lists every supplier that can ship, and the scan
visits them in the same order. The search returns the same choice, and an empty search now ends
at once. A search that does find stock is still bounded by its nearest qualifying supplier, as
before.

Verification, fresh on 2026-10-03:

- Matched runs: release `bench` profile, 24 logical CPUs, default Rayon workers.
- Before: `c2410028` in a detached worktree. After: the same commit plus this change.
- Command: `cargo bench --bench economy_tick_benchmark -- /10k`, then the same with `/100k`.
- Peak RSS is the bench process's `VmHWM`.

| Measure | 10k before | 10k after | 100k before | 100k after |
| --- | ---: | ---: | ---: | ---: |
| `operational_hour` (estimate) | 16.5 ms | 9.5 ms | 1.467 s | 29.2 ms |
| `operational_hour` bounds | 15.8–17.3 ms | 8.7–10.3 ms | 1.460–1.474 s | 26.3–32.4 ms |
| `daily_settlement` (estimate) | 324 ms | 324 ms | 4.81 s | 4.98 s |
| City build | 15.3 s | 14.7 s | 247.2 s | 176.7 s |
| Peak RSS | | | 3.05 GB | 3.07 GB |

City records:

- Identical before and after, both after the build and after the 263 benchmark hours.
- 10k: 5,617 households, 11,143 residents, 7,634 employed, 14,009 jobs, 7,428 buildings.
- 100k: 56,481 / 110,867 / 76,628 / 141,573 / 75,965.
- Treasuries also match: 1000000001165726.6 at 10k and 1000000012048676.4 at 100k.
- The year-one trajectory of the ignored growth scenario is also identical.

From 10k to 100k the hour now grows 3.1× for a 10× city, which meets the exit criterion of no
faster than linear. Logistics is about 2 ms of the 10k hour and about 8 ms of the 100k hour; the
rest is outside logistics.

Projected to 1M at linear growth from 100k (not measured):

- Hour: about 0.3 s.
- The build's 48 warm-up hours: about 15 s.
- The benchmark's 263 hours: about 80 s.

Daily settlement grows faster than the city: 324 ms → 4.98 s from 10k to 100k. Its slow days are
tracked under `ECON-13`.

Coverage: `supplier_index_lists_only_suppliers_with_an_unreserved_batch` in
`logistics/tests.rs`.
