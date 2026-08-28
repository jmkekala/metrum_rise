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

## The shared field is the floor

Wind, water, and fire are one combined field, not three systems that call each
other. They read and write shared per-cell state, so their interactions are
consequences rather than special cases. A fire's spread rate reads the wind
blowing over it and the moisture in what it is burning. Rain wets fuel, so
weather days ago changes how a fire behaves today. Water flows downhill over
real terrain, and where it pools is where it pools. A firestorm generates its
own wind, because the fire writes to the same field the wind reads. None of
that is scripted.

Anything bundled into a named event is wrong. A phenomenon emerges from general
laws reading fields; it is never a thing the world instantiates.

Water means real water that flows, not authored still-water fills with a
surface shader, which is what this repository has today. What follows from real
flow: rivers that run where the terrain sends them, flooding that is the water
going somewhere rather than a triggered event, groundwater with a stock that
depletes when drawn, and recharges through rain and snowmelt, and canals that
work because they are water rather than because they are painted.

Fire does not exist in this repository at all; there is a vehicle enum variant
and a UI label. A real fire has a fuel bed, a rate that reads weather and
moisture, and a spread that reads what is next to it. The fire keeps burning
while the engine is in traffic, and it spreads to the next building rather than
waiting, which is what response time in `services.md` is measured against.

Minerals are owned by `terrain.md`. Deposits derive from how the rock formed
rather than from a table, and extraction is the terrain deformation the
earthworks model already performs. Excavation is not a separate system: digging
is terrain deformation, and the dirt that comes out has to go somewhere, which
is what makes a cut-and-fill decision real.

Flora means trees that are all different rather than a handful of meshes
repeated, with cheap fractal grass and particles so ground cover is dense
without being placed by hand. Vegetation is a layer the other systems read: it
is fuel to a fire, shade and evaporation to the weather, roots holding soil
against erosion, and amenity value to a household. Fauna means real animals,
which makes hunting and fishing real activities rather than a resource yield
per tile: a population that can be depleted, that moves, and that responds to
the habitat the flora layer describes.

Disasters are simply a special-case event system. Each is an ordinary reading
of a field that already exists: a flood is water on terrain, a storm is weather,
a wildfire is the fire layer meeting the flora layer, an earthquake reads the
tectonic layer that placed the mountains.

## Agents and their minds

Sims have more of their own mind than the genre gives them. The layer to port is
the one built on a real neural network, with the director running sims as
agents. The hard constraint from this repository: determinism is the default.
Given the same save, inputs, and tick sequence the simulation produces the same
result, and the existing worker-selection model explicitly rejects RNG-driven
choice. Any neural variant has to stay deterministic or it does not land.

What that buys: a sufficiently disgruntled population riots, and the trigger is
a real incident between real agents rather than a meter crossing a line. The
news is real, because every agent is genuine and the director watches them, so
media companies can report events that actually happened rather than the canned
feedback the genre uses. And a day-night cycle long enough for sims to
approximate their lives is what makes rush hour, nightlife, and shift work
exist at all.

The detailed social simulation is a port target. It does not exist in this
repository, where crime, education, and parks are all deferred modifiers that
nothing reads. `services.md` owns crime and the rule that keeps it out of a
death spiral, `narrative.md` owns the political layer, and `economy.md` owns
the money.

## What the world is made of

Buildings are not swapped in when a timer expires. Procedurally generated
buildings actually scaffold and go up with cranes, so a construction site is a
visible stage of the building rather than a placeholder mesh. Pipes and power
are real networks in the ground and on poles, so the utility layer in
`services.md` is a topology rather than a coverage radius.

A building is not one zone type. Retail at the ground floor, offices above,
apartments above that, and a bar in the cellar reached from the alley are the
same building, and each part is its own economic unit. A side door into a
converted cellar is a real business with a real address on a service way, which
is the alley content in `zoning.md`.

## Scale, and how it stays affordable

The target is 20,000,000 agents, and what makes that reachable is that only
what is being looked at is simulated live. Distant agents are a hashed summary
rather than a running state machine, reconstituted when attention arrives. The
fractal and hashed level-of-detail method is the load-bearing piece of the port:
without it, real-sized cities with real-sized industry and individually tracked
agents are not affordable at any agent count worth having.

A save is seed, delta, and tick with a content-version stamp, not a dump of
world state; the world regenerates from the record. Two things follow. An
authored save survives a base change, because the edits replay onto the new
generator. And undo is replay: the edits are an ordered log, so undo truncates
the log and regenerates rather than restoring a snapshot.
