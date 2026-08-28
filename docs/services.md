# Services and Utilities

Everything the government builds and runs: emergency response, health,
education, civic amenity, the networked utilities, and enforcement.

This material was scattered. `economy.md` owns what a service building costs to
staff and run, and its one line about `always_on_service` is the whole of what
was written down. `asset_editor.md` owns the `service_class` an asset declares.
Neither owns how a service reaches the people it serves, which is the part that
decides whether a service is a simulation or a radius.

A service is a building that produces coverage rather than goods. The economy
already knows how to staff it, pay it, and bankrupt it. What belongs here is
what it produces, how that reaches a household, and what happens when it does
not arrive.

`economy.md` owns money, staffing, and the utility resource ledger.
`demand.md` owns how unmet need becomes pressure. `region.md` owns which pool
pays. This document owns coverage, response, and the service catalog.

## The two shapes of service

Every service is one of two things, and the difference decides everything about
how it is modeled.

Dispatched services respond to incidents. Fire, police, ambulance, border patrol, and
disaster response. There is an event at a place, a vehicle leaves a station,
travels the road network, and arrives after a time that depends on distance,
traffic, and whether a route exists at all. Response time is the output, and
it is a real measurement rather than a radius.

Standing services are visited or supplied. Schools, clinics, parks,
libraries, and the utilities. The household goes to them or they come down a
pipe. Access is the output: whether a household can reach one, how far, and
whether it has capacity when they arrive.

Modeling a dispatched service as a radius is the genre's standard error. A fire
station across a river with no bridge covers nothing, and a coverage circle says
it covers everything within the circle.

For dispatched services the contract is:

- A station holds vehicles and crew. Both are finite. A station with every
  engine already out is a station that cannot respond.
- Response time comes from the road network, using the same routing the rest
  of the simulation uses. A road closure, a congested arterial, or a bridge out
  changes response time, and the player can see that it did.
- The nearest station is not always the responding one. It is the nearest
  station with a free unit.
- Failure is late arrival, not absence of service. A fire that burns longer
  does more damage and can spread. That is a curve, not a threshold.

A player fixes response times with roads as often as with stations.

## Utilities are networks, not coverage

Power, water, and sewage already have an economic ledger in `economy.md` as the
Utility Service Layer, deliberately outside the freight rules. What that layer
does not yet say is how supply physically reaches a building.

- Power is a grid. Generation, transmission, and a served or unserved
  building at the end of it. A grid that is short of generation browns out
  somewhere rather than everywhere.
- Water is drawn, treated, distributed, and returned. Where it is drawn from
  is a real place with a real stock, so a desert city on an aquifer is viable
  until it is not.
- Sewage is the same network run backwards, and what it discharges goes
  somewhere. Untreated discharge upstream of an intake is a consequence the
  simulation can produce on its own rather than a scripted event.

The two-tier voltage split is worth taking. High-tension transmission
between generation and substations, low-tension distribution beyond them, so
that a grid has a topology rather than a fill.

## Emergency and disaster

Fire, flood, earthquake, and storm are events the world produces rather than
buttons the game presses.

Disasters interact with everything already built: a flood reads terrain and
water, a storm reads weather, an earthquake reads the tectonic layer that placed
the mountains. None of them is a special-case event system. Each is an
ordinary reading of a field the simulation already carries.

## Crime and policing, without the death spiral

Crime is wanted. The failure mode is well known and specific.

SimCity's version and its descendants run a loop with no exit: crime lowers
land value, low land value raises crime, the district collapses, and the only
intervention that works is the bulldozer. The loop is a positive feedback with
no ceiling and no counterweight. That is the thing to avoid, not crime.

Three rules break it, and all three are requirements rather than preferences:

Crime is a rate, not a stain on the land. It attaches to conditions:
unemployment, density without services, unmet needs. It moves when they move. A
district that recovers economically sees its crime rate fall on its own, because
the rate was reading conditions rather than being stored on the tiles.

Policing has diminishing returns and real costs. More police reduces crime
along a falling curve, so a district cannot be solved by stacking stations, and
heavy policing carries its own consequences for how residents feel. Policing is
a lever with a cost, not a counter that cancels another counter.

No self-reinforcing land-value term. Crime may affect desirability, and
desirability may affect what gets built, but crime must not read a value that
crime itself lowered. The discipline `demand.md` already states applies
directly: a harmful raw quantity is inverted or remapped before the demand layer
reads it, and higher must mean more favourable by the time it arrives. Wiring
crime as a straight negative into desirability, and desirability back into
crime, is the death spiral.

The groundwork exists: `police` is already a `service_class` in the asset
schema, and `crime` already appears among the deferred local-modifier families.
What it needs is the loop drawn so that it terminates.

## Border patrol

Enforcement is a service, and policy sets how open a border is. Enforcement is
a thing the player builds, and without it the policy is a number nobody is
applying.

- A border patrol is a staffed service like any other, with a budget, a
  headcount, and a recruitment problem.
- Funding starts regional and moves national. Until a second region unlocks
  the national pool the region pays for its own border; after that the cost moves
  up. National parks follow the same path, and that migration is what makes a
  second region feel like a country rather than two cities.
- Checkpoints are placed structures. The four border states decide how a
  crossing presents itself; the player decides where the crossings are and how
  many.
- Walls are built, priced by length, and run where the player draws them. A
  wall is the expensive answer to a problem policy could have solved more
  cheaply, which is the honest tradeoff to model.

Policy without enforcement leaks, and enforcement without policy is expensive
theatre. A sealed border with no patrol admits people anyway if they are motivated
enough. A heavily patrolled open border is money spent waving traffic through.

That also gives the four border states something to be driven by other than a
slider. A crossing's appearance follows both what the policy says and what has
actually been built there.

The border states, the openness dial, and the geometry of a crossing live with
the network code. What lives here is that a patrol is staffed, funded, and can
be under-resourced like any other service.

## Education and civic amenity

Education, health, parks, libraries, and recreation are standing services.

Education produces the labor other systems spend. Educational tiers gate what
the economy can build, an endgame project cannot be worked by a country that never
built the schools for it.

### The school tiers

Primary and secondary school are ordinary municipal services. They are unlocked
from the start, every city needs them, and they produce the baseline workforce.

Higher education is three distinct institutions, not one building at three
levels. Each is unlocked differently, placed under different rules, and produces
a different grade of labor.

| Institution | Where | Produces |
|---|---|---|
| Community college | At least one per city; larger cities may hold several | The lower end of skilled labor |
| State college | One per region | Mid-skilled labor, the high end of an ordinary economy |
| Ivy league | Anywhere, but conditionally unlocked and hard to earn | The specialists every endgame goal requires |

The community college is the first expansion into higher education. It is
unlocked first and it is deliberately limited: it opens the bottom of the skilled
band and no further.

The state college is alloted one per region, which makes it a regional decision about
where the region's skilled workforce concentrates rather than a building a city
adds to itself.

The ivy league is earned. It unlocks only by meeting conditions inside the
education system itself, and the player has to work for it. More than one is
possible and each is harder than the last.

Every ivy league institution has a specialization, and it decides what the
school produces. The real examples are the model: the Colorado School of Mines,
Juilliard and Curtis, Wageningen, and Gobelins, are all specialized and ireplacable
schools and neither substitutes for the other. A country with a mining schools and
no arts conservatory has a gap it cannot fill by building another mining school, and
no number of highly trained geologists will be making Hollywood. This fact makes each
one a strategic decision rather than a repeated purchase. A player who wants both a
space program and a film industry has to earn two institutions and specialize them
differently.

Specializations map onto the endgame conditions rather than being a free list,
so every one of them is a path to something and none is a dead end.
Engineering and mining for heavy industry and extraction. Physical sciences for
the space program. Microelectronics for chip fabrication. Medicine, law, and
finance for the service economy. Arts and performance for the entertainment
endgame. Agriculture and earth science for the land itself.

Siting interacts with specialization. An archaeological site suits earth
science and the humanities; advanced technology industry nearby suits
microelectronics; a mining region suits the school that studies it. 

Placement is free but not neutral. Some sites are plainly better, and nearby
features add small modifiers: an archaeological site, or advanced technology
industry, each nudge an ivy league institution's output. The boon is slight, so
siting is a considered choice rather than a solved one.

Ivy league graduates are what the endgame runs on. Bleeding-edge chip fabrication,
a space program, a colossal entertainment industry, and bleeding edge horticulture
need people no lower tier produces, so the education chain is a prerequisite for
the victory conditions rather than a parallel amenity.

### Studying abroad

A country with no colleges cannot train the people it needs to build colleges.
The early-game and late game answers ar ethe same, policy: grant sims money to
study abroad.
- The player sets the policy and pays the grants to applicants.
- Those sims leave the country.
- They come back a few years later, educated, and enter the workforce at a
  tier the country cannot yet produce on its own.
- Sending sims to Ivy league schools is much more expensinve than community
  colleges, and isn't unlocked unlocked until you have your own state colleges.

The delay is the mechanic. Money spent now returns as labor much later, which
makes it a genuine early investment rather than a purchase, and a player who
wants higher education early has to use it. Building the first college
otherwise requires an educated workforce the country does not have, and it
carries the obvious risk worth modeling: some of them do not come back.

### Paying people to move here

The other way to get labor the country cannot produce is to buy it from somewhere
else. The immigration policy's lever the same shape as the study grant: money up
front, workforce later, and a rate the player nudges rather than a number they set.

The player can offer grants to skilled and educated immigrants. It costs money
per arrival and it ticks the immigration rate up a little. Not a switch, and not
a flood. A country that pays for engineers gets somewhat more engineers arriving
than it would have, and pays for every one of them.

It is faster than educating people domestically and it does not compound. A
university keeps producing graduates after it is built; a grant buys one worker
and then wants paying again.

The border policy in `narrative.md` sets the ceiling and the grant moves the rate
underneath it. A player paying for skilled arrivals while running a sealed border
is spending money on nothing, and the game should show that rather than quietly
reconcile it.

Paying foreigners to arrive while domestic sims are unemployed is the kind of
thing the media layer reports and the political layer answers for.

## What decides funding

Which pool pays is a property of the service, not of the building, and the
answer changes as the country grows. `economy.md` owns the pools themselves.

Before the region unlocks, the city budget pays for everything the player can
build, and taxes go up to a region the player does not control.

Once the region unlocks it holds one pool for everything: ordinary municipal
services, and the services no single city owns. Cities are separate data points
inside it rather than separate pools.

Once a second region unlocks, the national/federal pool takes the country-scale
responsibilities off the region: power, border patrol, national parks, and
anything else run for the country rather than a place. Fire, police, schools,
clinics, and traditional waste stay where they were.

A service migrating from regional to national funding as the country grows is
the mechanic, not an accounting detail.

## Open questions

- **Disaster Response:** Whether flood defense, evacuation, and
  rebuilding are their own systems or ordinary applications of the ones above.
