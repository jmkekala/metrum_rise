# Services and Utilities

Everything the government builds and runs: emergency response, health, education, civic amenity,
the networked utilities, and enforcement.

`economy.md` owns what a service building costs to staff and run.`asset_editor.md` owns the
`service_class` an asset declares. Neither owns how a service reaches the people it serves.

A service is a building whose output is coverage. This document owns what it produces, how that
reaches a household, and what happens when it fails to arrive.

`economy.md` owns money, staffing, and the utility resource ledger. `demand.md` owns how unmet
need becomes pressure. `region.md` owns which pool pays. This document owns coverage, response,
and the service catalog.

## Dispatched and standing services

Every service is one of two kinds.

Dispatched services respond to incidents: fire, police, ambulance, border patrol, and disaster
response. There is an event at a place, a vehicle leaves a station, travels the road network, and
arrives after a time that depends on distance, traffic, and whether a route exists at all. Response
time is the output, measured from the actual trip.

Standing services are visited or supplied: schools, clinics, parks, libraries, and the utilities.
The household goes to them or they come down a pipe. Access is the output: whether a household can
reach one, how far, and whether it has capacity when they arrive.

Modeling a dispatched service as a radius is the genre's standard error, and then giving the service
a severely cucked service radius is the other. A fire station across a river with no bridge 20 miles
in either direction covers nothing, and a factory several miles out but with a clear path to get to
it should fall under thier service coverage. Like cities: Skylines, METRUM RISE utilizes a network
service catchment heat-map, but unlike Cities: Skylines, METRUM RISE will feature realistic response
distances governed by district and priority. If an event happens in a district a service is tasked
and that service is required to respond across the map, it could a long time, just like calling the
sheriff in any rural area. 

For dispatched services the contract is:

- A station holds vehicles and crew. Both are finite. A station with every engine already out
  cannot respond.
- Response time comes from the road network, using the same routing the rest of the simulation
  uses. A road closure, a congested arterial, or a bridge out changes response time, and the player
  can see that it did.
- The responding station is the nearest one with a free unit.
- Failure means late arrival. A fire that burns longer does more damage and can spread.

A player fixes response times with roads as often as with stations.

## Utility networks

Power, water, and sewage already have an economic ledger in `economy.md` as the
Utility Service Layer, deliberately outside the freight rules. What that layer
does not yet say is how supply physically reaches a building.

- Power is a grid. Generation, transmission, and a served or unserved building at the end of it. A
  grid short of generation browns out in specific places.
- Water is drawn, treated, distributed, and returned. Where it is drawn from is a real place with a
  real stock, so a desert city on an aquifer is viable until the aquifer is pumped faster than it
  naturally replenishes.
- Sewage is the same network run backwards, and what it discharges goes somewhere. Untreated
  discharge upstream of an intake is a consequence the simulation produces on its own.

The two-tier voltage split is worth taking. High-tension transmission between generation and
substations, low-tension distribution beyond them, so that a grid has a topology.

## Emergency and disaster

Fire, flood, earthquake, hurricaine, and storm are events the world produces. The game presses
no buttons to start them in "campaign mode".

Disasters interact with everything already built: a flood reads terrain and water, a storm reads
weather, an earthquake reads the tectonic layer that placed the mountains. Each is an ordinary
reading of a field the simulation already carries.

## Crime and policing

Crime is wanted.

SimCity's version and its descendants run a loop with no exit: crime lowers land value, low land
value raises crime, the district collapses, and the only intervention that works is the bulldozer.
The loop is a positive feedback with no ceiling and no counterweight.

Three rules break it, and all three are requirements:

Crime is a rate. It attaches to conditions: unemployment, density without services, unmet needs. It
moves when they move. A district that recovers economically sees its crime rate fall on its own,
because the rate reads live conditions and the tiles store nothing.

Policing has diminishing returns and real costs. More police reduces crime along a falling curve,
so a district cannot be solved by stacking stations, and heavy policing carries its own
consequences for how residents feel.

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
- Funding starts regional and moves national. Until a second region unlocks the national pool the
  region pays for its own border; after that the cost moves up. National parks follow the same
  path.
- Checkpoints are placed structures. The four border states decide how a crossing presents itself;
  the player decides where the crossings are and how many.
- Walls are built, priced by length, and run where the player draws them. A wall is the expensive
  answer to a problem policy could have solved for less.

Policy without enforcement leaks, and enforcement without policy is expensive theatre. A sealed
border with no patrol admits people anyway if they are motivated enough. A heavily patrolled open
border is money spent waving traffic through.

A crossing's appearance follows both what the policy says and what has been built there, so the
four border states take a second input beyond the slider.

The border states, the openness dial, and the geometry of a crossing live with
the network code. What lives here is that a patrol is staffed, funded, and can
be under-resourced like any other service.

## Education and civic amenity

Education, health, parks, libraries, and recreation are standing services.

Education produces the labor other systems spend. Educational tiers gate what the economy can
build; an endgame project cannot be worked by a country that never built the schools for it.

### The school tiers

Primary and secondary school are ordinary municipal services. They are unlocked from the start,
every city needs them, and they produce the baseline workforce.

Higher education is three distinct institutions, each a separate building with its own unlock, its
own placement rules, and its own grade of labor.

| Institution | Where | Produces |
|---|---|---|
| Community college | At least one per city; larger cities may hold several | The lower end of skilled labor |
| State college | One per region | Mid-skilled labor, the high end of an ordinary economy |
| Ivy league | Anywhere, but conditionally unlocked and hard to earn | The specialists every endgame goal requires |

The community college is the first expansion into higher education. It unlocks first and stays
deliberately limited: it opens the bottom of the skilled band and stops there.

The state college is allotted one per region, which makes it a regional decision about where the
region's skilled workforce concentrates. A city cannot add one on its own.

The ivy league is earned. It unlocks only by meeting conditions inside the education system
itself. More than one is possible and each is harder than the last.

Every ivy league institution has a specialization, and it decides what the school produces. The
Colorado School of Mines, Juilliard and Curtis, Wageningen, and Gobelins are the model: each
substitutes for none of the others. A country with a mining school and no arts conservatory
cannot fill that gap by building another mining school, and no number of highly trained
geologists will be making Hollywood. A player who wants both a space program and a film industry
has to earn two institutions and specialize them differently.

Specializations map onto the endgame conditions. Engineering and mining for heavy industry and
extraction. Physical sciences for the space program.
Microelectronics for chip fabrication. Medicine, law, and finance for the service economy. Arts and
performance for the entertainment endgame. Agriculture and earth science for the land itself.

Siting interacts with specialization. An archaeological site suits earth science and the
humanities; advanced technology industry nearby suits microelectronics; a mining region suits the
school that studies it.

Placement is free and carries consequences. Some sites are plainly better, and nearby features add
small modifiers: an archaeological site, or advanced technology industry, each nudge an ivy league
institution's output. The boon is slight, so siting stays a considered choice with several viable
answers.

Ivy league graduates are what the endgame runs on. Bleeding-edge chip fabrication, a space
program, a colossal entertainment industry, and bleeding edge horticulture need people no lower
tier produces, so the education chain is a prerequisite for the victory conditions.

### Studying abroad

A country with no colleges cannot train the people it needs to build colleges. The early-game and
late-game answers are the same policy: grant sims money to study abroad.
- The player sets the policy and pays the grants to applicants.
- Those sims leave the country.
- They come back a few years later, educated, and enter the workforce at a tier the country cannot
  yet produce on its own.
- Sending sims to ivy league schools costs much more than community colleges, and unlocks only once
  the country has its own state colleges.

The delay is the mechanic. Money spent now returns as labor years later. Building the first
college otherwise requires an educated workforce the country does not have, so a player who wants
higher education early has to use grants. Some of them do not come back.

### Immigration grants

The other way to get labor the country cannot produce is to buy it. The immigration grant has the
same shape as the study grant: money up front, workforce later.

The player offers grants to skilled and educated immigrants. It costs money per arrival and ticks
the immigration rate up a little, a dial with a narrow range. A country that pays for engineers
gets more engineers arriving, and pays for every one of them.

It is faster than educating people domestically and it compounds nothing. A university keeps
producing graduates after it is built; a grant buys one worker and then wants paying again.

The border policy in `narrative.md` sets the ceiling and the grant moves the rate underneath it. A
player paying for skilled arrivals while running a sealed border is spending money on nothing.

Paying foreigners to arrive while domestic sims are unemployed is the kind of thing the media layer
reports and the political layer answers for.

## Funding

Which pool pays is a property of the service. The building has no say, and the answer changes as
the country grows. `economy.md` owns the pools themselves.

Before the region unlocks, the city budget pays for everything the player can build, and taxes go
up to a region the player does not control.

Once the region unlocks it holds one pool for everything: ordinary municipal services, and the
services no single city owns. Cities are separate data points inside that single pool.

Once a second region unlocks, the national pool takes the country-scale responsibilities off the
region: power, border patrol, national parks, and anything else run for the whole country. Fire,
police, schools, clinics, and traditional waste stay where they were.

A service migrating from regional to national funding as the country grows is a mechanic with
gameplay consequences.

## Open questions

- **Disaster Response:** Whether flood defense, evacuation, and
  rebuilding are their own systems or ordinary applications of the ones above.
