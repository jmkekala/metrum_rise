# Simulation Layers

The physical and living systems the game runs on, and what each is for.

None of this is built in this repo. The code is written in gdscript and C++,
and will need translation or conversion before it can ship to the rust core.
It is recorded here so the ordering is settled before anyone starts: the
physical floor and the shared field come first, because everything else rides
on them. These layers come from an external Godot based physics engine and are
currently being ported.

Tracked as `PORT-01` in [`roadmap.md`](roadmap.md), at `P0`. It is the most
downstream decision in the project: everything built before it assumes the current
still-water fills and absent fire, so the longer it waits the more code is written
against assumptions it invalidates.

## The shared field

Wind, water, fire, time, materials, agents, everything the simulation governs is one combined field.
They read and write shared per-cell state, so their interactions are consequences of that shared state.
A fire's spread rate reads the wind blowing over it and the moisture and fuel in what it is burning.
Rain wets fuel, so weather days ago changes how a fire behaves today. Water flows downhill over real
terrain, and where it pools is where it pools. A firestorm generates its own wind, because the fire
writes to the same field the wind reads. Minerals and resources populate certain areas of the map
because the conditions of that world seed made it so. Every phenomenon emerges from general laws
reading fields, and the world instantiates no named events.

Water means real water that flows. This repository currently has authored still-water fills with a
surface shader. Real flow produces rivers that run where the terrain sends them, flooding that is
the water arriving somewhere, groundwater with a stock that depletes when drawn and recharges
through rain and snowmelt, and canals that work because they hold water.
 
Fire does not exist in this repository at all; there is a vehicle enum variant and a UI label. A
real fire has a fuel bed, a rate that reads weather and moisture, and a spread that reads what is
next to it. The fire keeps burning while the engine is in traffic, and it spreads to the next
building while it waits, which is what response time in `services.md` is measured against.

Minerals are owned by `terrain.md`. Deposits derive from how the rock formed, and extraction is
the terrain deformation the earthworks model already performs. Digging is terrain deformation,
and the dirt that comes out has to go somewhere.

Flora means every tree is distinct, with cheap fractal grass and particles so ground cover is
dense without hand placement. Vegetation is a layer the other systems read: fuel to a fire, shade
and evaporation to the weather, roots holding soil against erosion, and amenity value to a
household. All of this traditionally tanges from individually to collectively very expensive, but
when riding a shared substrate and field, every sytem feeds and compliments one another recursively
from the ground up from a floor of 10 physical constants that everything else is built in layers on
top of. The result is a simulation where level of achievable complexity is a boundary of the games
own scope, not the engines ability to achieve these levels of fidelity. A sun is a real sun,
burning real hydrogen for free, never being expensive because no one is looking inside the sun.
Sound that is a consequnce of interaction, weather that is a concequence of its environment,
behavior that is a concequence of real decisions, and planets with real scope and diversity.

## Disasters and natural events

Each disaster is an ordinary reading of a field that already exists. A flood is water on terrain,
a storm is weather, a wildfire is the fire layer meeting the flora layer, and volcanoes and
earthquake reads the tectonic layer that placed the mountains.

## Fauna

Animals get the same minds the sims get. They are agents reading the shared field, so a live
population can be depleted, moves, and responds to the habitat the flora layer describes. Hunting
and fishing run against that population, and overproduction or dar crop rotation decisions will
deplete natural resources.

What wildlife does in a city is not bounded by what the simulation forbids, and the allowed
behaviours are not enumerated in advance. Because of the shared field, rats will naturally reach
food waste and carry disease into the health system, raccoons dig through, and bears come into
rural settlements where the habitat meets the buildings. The limits of these interactions
therefore must be set by the game itslef through the engine's authoring layer.

Invasive species follow from the same machinery. A species that arrives and finds a habitat with
no check on it spreads, because spread is population against habitat.

### Environmental impact

The player's construction changes the habitat the fauna layer reads. Clearing forest, draining
wetland, damming a river, polluting water, and paving over a range all move the population that
lived there.

A city that destroys a predator's range gets more of whatever that predator ate, stimulating the
pest control industry. A river polluted upstream has fewer fish downstream, so fishing stops
paying. Waste left reachable feeds a population that carries disease into the `services.md`
health system.

No environmental score computes any of this. The population reads the habitat and the habitat
reads what the player built thriugh the shared field, and live simulation is then aggegated back
into the statistics and figures the player reads, and a perceptive player will also notice the
visual cues.

### Wildlife crossings

A highway, a rail line, or a canal cut through habitat leaves the population on either side
unable to reach the other, and the animals that try are roadkill. A crossing costs money to
build and maintain, occupies land at both approaches, and spans a corridor whose width sets its
span. Placement decides whether it works: one on the route animals already take carries them,
and one sited where the player found room does not.


The player builds wildlife bridges over highways, rail, and canals, and the crossing reconnects
the two sides for the fauna layer. Animals route across because the crossing is habitat
continuous with what they already use, with no separate pathing directing them. A reconnected
range holds a population a severed one cannot. Predators reach prey across the cut, so pest
pressure on the far side stays where it was. Collisions fall where the crossing carries the
traffic that was crossing at grade.

## Agents and their minds

Sims have more of their own mind than the genre gives them. The layer exists and is built on a
real neural network as `connectome_node.gd`, with the director running sims as agents. They have 
their own goals, motivations, desires, and a sufficiently disgruntled population riots, triggered
by a real incident between real agents. Every agent is genuine and the director watches them, so
media companies report events that actually happened. A day-night cycle long enough for sims to
approximate their lives is what makes rush hour, nightlife, and shift work exist. This is once
again made possible by the shared field, and the hashed lod that runs the background cheaply.

The detailed social simulation is a port target from 2.5D_edgine. It does not exist in this
repository, where crime, education, and parks are all deferred modifiers that nothing reads.
`services.md` owns crime and the rule that keeps it out of a death spiral, `narrative.md` owns the
political layer, and `economy.md` owns the money.

## What the world is made of

Procedurally generated buildings scaffold and go up with cranes, so a construction site is a visible
stage of the building. Pipes and power are real networks in the ground and on poles, so the utility
layer in `services.md` is a topology.

A building can carry several zone types at once. Retail at the ground floor, a bar in the cellar,
offices above, then apartments above that, and each part is its own economic unit. A side door into
a converted cellar is a real business with a real address on a service way, the alley content in
`zoning.md`.

## Scale

The target is **at least** 20,000,000 agents, reachable because only what is being looked at is
simulated live. Distant agents are a hashed summary, reconstituted when attention arrives. The
fractal and hashed level-of-detail method is the load-bearing piece of the port: without it,
real-sized cities with real-sized industry and individually tracked agents exceed the budget at
any agent count worth having without significant development.

A save is seed, delta, and tick with a content-version stamp; the world regenerates from that
record. An authored save survives a base change, because the edits replay onto the new generator.
Undo is replay: the edits are an ordered log, so undo truncates the log and regenerates the
world.
