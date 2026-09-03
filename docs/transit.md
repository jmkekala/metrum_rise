# Transit

Nothing here is built. It exists so the rail decision is recorded before anyone
implements three separate systems. `roads.md` owns the road network and its
lanes, `traffic.md` owns vehicle movement and routing, and this document owns
the modes and their alignments.

The modes, all first class and all running: taxi, bus, tram, train, subway, light rail, monorail,
funicular, cable car, gondola, ferry, barge, blimp, and airplane. The gondola is the boat. The
barge is freight only.

## Rail has use rules

Tram, subway, light rail, and train are all rail. Real cities run them mixed: San Francisco's BART,
Frankfurt's U-Bahn, Boston's MBTA green line, and Tokyo's Through-Running Service are all examples
of this; they contain light rail on street track downtown with heavy alignment out of town and
subways that surface as trams sharing a corridor with regional services. Modeling them as three
unrelated systems is the mistake that leaves transit in this genre feeling like disconnected toys.

A corridor carries an alignment class and a vehicle carries what it can accept, what varies is how
it is used. Trams need street-embedded or shared alignment, subways need grade separation, a light
rail needs overhead power, and a train needs heavier alignment and longer curves that exclude tight
street geometry. Which vehicle can traverse what track is a property of the vehicle. Mixed running
falls out of the two overlapping, and so does the refusal: a train will not take a tram's turning
radius, the same shape as the lane model in `roads.md`, a band declares which modes
it admits and the network stays whole.

The other modes are physically distinct, although the funicular is undecided. Ferries and gondolas
require navigable water, cable cars require large pylons, and blimps and airplanes require
airstrips and landing pads. Buses and taxis ride the road network and belong to `roads.md`.

Infrastructure is sized to realistic scales: real international airports and train stations that
move millions, handling traffic and freight from around the world.

Freight travels visibly, for as long as it takes. When freight is packed into a train in the far
corner of the map, that train carries it the whole way to its destination, where it is unloaded
and possibly goes somewhere else. A cargo ship might take an in-game week or two to cross the sea,
and the player can watch it traverse the sea lane the entire time. A resource chain spanning
several regions is legible because a failing chain shows the failure in transit, at the point
where it happens.

## Barges

A barge is the inland freight mode. It is slow, it carries a great deal, and it goes only where
the water already goes. The player cannot route it, only decide whether to sit on it.

It is a distinct mode from the sea-going cargo ship. A river has a direction of flow, a depth, and
a width, so it admits some tows and refuses others, using the same overlapping-permission model as
rail: the channel declares what it can carry and the vessel declares what it needs. Locks and
shallow reaches are where that refusal becomes visible.

Barge freight lands at a river terminal, so the last leg is a truck or a rail spur. A corridor
along a navigable river in [`region.md`](region.md) shortens that last leg to almost nothing.

## Freight terminals

The genre's freight depot is one building that absorbs a truck and emits goods. It abstracts away
the facility the shipped model requires. A depot as a single object survives only as a unique
special.

A freight terminal is a site the player composes from parts: rail spurs, storage warehouses, barge
docks, truck terminals, container yards, gantries. The player places them, then draws the roads,
rails, and paths that link them.

Freight sites use the same build granularity as everything else in [`zoning.md`](zoning.md): draw
the site, plop the pieces, or zone the land and let it fill. A small industrial strip and a port
loading ships with gantries are one system at two scales.

### Load points

A load point is a placed piece, and sharing one is the ordinary case. A warehouse district served
by a couple of spurs running down the middle of two facing rows is a common real arrangement. No
tenant there ships enough to justify track alone, which is why the spur gets built: a hundred of
them together take the great majority of their freight off the road and each stops paying for
final-mile trucking.

A single tenant may still own its own. One warehouse can hold ten bays where the volume through it
justifies ten, which describes a large operation. A building with no load point is served by
trucks.

The player's decision is aggregation: how many tenants sit within reach of one piece of track, and
how their yards face it. Layout decides the outcome.

Freight currently moves between a building and a border node with nothing in between, and a shared
spur is neither, so it has no way to exist today. A freight endpoint has to become its own kind of
thing that buildings attach to. [`economy.md`](economy.md) owns that model and the cost of using
it.

The infrastructure is priced as real construction. A spur requires the length and curve radius a
train needs, a dock requires navigable frontage, and a gantry yard requires room. Total freight
within reach decides whether it pays, so an ordinary warehouse district clears the bar while an
isolated factory of the same floor area falls short.

That puts the shared spur early and the dedicated single-tenant spur late. The endgame chains
reach the second when one site moves enough by itself and truck haulage has become the limit.

## Airports

Three classes, separated by what they can accept and by what earns them. Each is composed the same
way as a freight terminal, so the class sets the ceiling and the player's layout decides what the
site does.

### International

The largest class, with no ceiling on strips: the player adds runways as traffic justifies them,
limited only by land. A player reaches this class late, and holding two is a late-game position.

| Airport | Code | Range |
|---|---|---|
| Honolulu International Airport | HNL | Medium |
| Beijing Daxing International Airport | PKX | Large |
| Denver International | DIA | Larger |

### Regional

A regional airport accepts international traffic without being a through-port for it. The player
meets this class first: the starting city gets one early, and it stays the only one for a long
while.

Reference airports:

| Airport | Code | Range |
|---|---|---|
| Rogue Valley International | MFR | Small |
| Hilo International | ITO | Large |
| Tallahassee International | TLH | Largest, bordering on upgrade to INTL |

Two gates control how many exist. A region unlocks its first once the region's size justifies it.
After that, each additional city unlocks one only once that city is individually large enough to
justify it, and no city reaches that bar as fast as the first did. The bar rises with each
regional airport the region already holds, the way the city-founding milestone in
[`region.md`](region.md) rises with each city held.

### Local

Two shapes, one growing from the other with use, both cheap enough to appear early and in numbers.

The barebones strip is a farm's duster strip or a small rural community field where several
people keep dusters and private planes.

The larger local airport accepts smaller cargo planes and passenger planes. Reference airports:

| Airport | Code | Range |
|---|---|---|
| Negrito Airstrip | ONM7 | smallest |
| Johnson Creek Airport | U86 | Small |
| Lukla | LUA | Small/Medium |
| California City Municipal | L71 | Medium |
| Olympic Dam | OPL | Medium |
| Dawson Community | GDV | Large |
| Apple Valley | APV | Largest, bordering on upgrade to RGNL |

Redding Regional (RDD) is about as large as these are allowed to get.

### Airports are drawn as industrial sites

Runways, taxiways, aprons, terminals, hangars, fuel farms, and freight buildings are placed
pieces, and the player draws the roads, rails, and paths that link them. The class governs what
pieces the site admits and how much it can hold.

Composing a site out of pieces must not mean hand-placing every stall. Site layout carries the
same stepped authority [`zoning.md`](zoning.md) gives everything else, so a player may draw an
apron and have it filled with stands at a chosen spacing and orientation, or place each stand by
hand. The generated result is ordinary placed pieces, editable individually.

This applies to industrial sites generally. Parking rows, loading bays, storage racks, container
stacks, and tank rows are all repeated pieces on a drawn area, and each needs the fill-then-edit
path.

### Freight and connections

Every class carries an industrial freight area sized to it. The cargo area holds load points,
buildings attach to them, and freight without one is trucked.

What connects the site inland follows from the player's economy. A cargo complex moving enough
volume needs a rail spur or a barge dock or both, on the same terms any other freight site does,
and the same test decides whether it pays: total freight within reach. An international airport
with no rail works, and its cargo all leaves by truck.
