# Region

The land between and around cities, how a player takes it, and how it develops
without being told to.

`zoning.md` owns parcels and land use inside a city. This document owns the tier
above: the region, the tiles it is made of, and the settlements that appear in
zoned land on their own.

A city builder in this genre treats everything outside the city as scenery. Here
the region is a simulated tier with its own growth, its own population, and its
own economy, and the player's relationship to it decides how expensive the next
city is.

A country is 20 to 30 regions, and a region subdivides into tiles.

Tile boundaries follow the world rather than a lattice. Strips run along
roads, natural features, mountains, and rivers. Where no such feature exists
the generator draws abstract district-shaped tiles, so the subdivision is legible
everywhere without pretending a river exists where none does.

## Founding

Two choices, in order: first the region, chosen from the 20 to 30 with
difficulty and known resources visible, then the tile within it.

The region choice is the single most consequential decision in a run and it is
made before anything is built.

Tile selection runs either way, and neither is the default in a way that
penalizes the other:

- Automatic, an algorithmic best fit over resources, population, production,
  and terrain.
- Manual, the player buying the specific tiles they want.

The automatic fit exists because founding a twelfth city by hand is tedious, not
because hand selection is wrong.

A new city is founded with a portion of the regional budget. That grant does two
jobs at once.

It is the starting capital when there is nothing already there. If an
unincorporated municipality with its own income already sits on the tile, the new
city inherits that income and starts from it. On empty ground with zero
population the grant is all there is, and loans are likely on top of it.

The players very first city gets a bootstrap grant, every succesive city must
source its grant from the regional budget.

It buys the city's initial limits. The grant purchases the inner region tiles
around the founding point, so a city's opening footprint is set by how much money
it was given plus whatever income it inherited. A well-funded founding starts
wide; a shoestring one starts tight and grows.

## Expansion

The player may buy tiles adjacent to the city, or one tile beyond that. Reach
is short on purpose: growth is contiguous and earned rather than a land grab.

Buying is not the only way limits move. A city naturally acquires neighboring
tiles on its own as it expands, slowly and at no cost. Purchase is how a player
takes a tile they want now, for a reason: a deposit, land value, a route. A
player who never touches the tile map still ends up with a city that has slowly
absorbed neighboring tiles.

The second city is unlocked by playing. After that, cities unlock on
population milestones across all citizens.

The milestone grows with every city held. If the first threshold is 100,000
with one city, the third might be 500,000 with three. The curve rises faster than
the city count, so each new city is earned against a higher bar than the last.

That shape does the balancing on its own. A player who builds one deep city and a
player who spreads wide both progress. What it forbids is founding a dozen towns
and letting them stagnate, because a stalled city counts toward the next
threshold without contributing to reaching it.

A new city will majory need the players attention until it is running. It will be
the primary focus of their next few hours until stable enough to leave for longer
than a few minutes. That, along with the population threshold, is what stops eight
cities being founded at once.

The game does not forbid it. It lets the player find out. Prerequisites gate
founding a little; past that the difficulty ceiling becomes unmanageable because
each new city demands attention it cannot get.

Capacity grows with the empire. A player with eight running cities absorbs a
ninth far more easily than a player with one absorbs a second: the region has
income, infrastructure is already out there, and founding is no longer starting
from nothing. This is by design, a player at hour 80 should not be bogged down by
early game struggle.

Plan regional development around where you intend to build later. Zone
agriculture and run power and roads out toward a future site outside of city
limits and zone, and you arrive to find a place that already has modest people,
income, and infrastructure. Founding cold in an empty tile is the expensive version.

## How zoned land grows itself

Regional infrastructure (power, water, roads) is placeable outside city limits
early. What is gated is scale: only a few farms and limited industry or similar
may be placed outside city limits.

Zoned regional land develops on its own. Any cluster the director finds
sufficient for what that zoned area is doing becomes a settlement, so a timber
region grows timber towns and a mining region grows mining camps.

The player can shape it rather than watch it. Hand-placing a doctor, a police
station, a gas station, and a fire station within a hundred feet of each other is
a deliberate act of centralization, and the director grows a settlement around
that core rather than wherever it would have chosen.

Regional settlement is capped by a stated law rather than a soft discouragement,
and the numbers are fixed so the director has something exact to solve against.

| Quantity | Value |
|---|---|
| A square mile | 640 acres |
| Smallest regional residence | half an acre |
| Average household | 2.5 people |
| A fully packed square mile | 6,400 people |
| The law allows | 2,560 per square mile, four people per acre |

The gap between what fits and what is allowed is the design. The director
cannot spread density evenly; it has to choose where to concentrate, which is
what produces distinct hamlets in zoned land instead of uniform sprawl.

Two thresholds follow:

- 960 per square mile is a township or hamlet. The line where a cluster
  stops being scattered houses and becomes a place with a name.
- 2,560 is the ceiling. A settlement at the cap stops growing until it is
  upgraded or incorporated.

Ordinary settlements should never exceed 1,000 by more than a few dozen coming and
going, with 2,500 reachable only where the player has deliberately pushed. The countryside
is sparse and should read that way.

Director-generated regional plots run from about two acres up to half a mile by
one mile, 320 acres, depending on what is being grown. Non-grid plots follow
the same size range.

In genuinely flat country the generator prefers one mile by one mile
Jeffersonian grids, because that is what real surveying produces in flat
country. The grids orientation should choose North-South if there are no features nearby
worth of anchoring too instead, and the grid direction should occasionally change to match
significant roads or natural features.

It must generally not lay a whole grid at once. A two by five mile block, or a
ten mile streach with scattered grids growing off of it, or not a grid at all,
sometimes just straight roads a grid later grows off. Uniformity across a whole region,
even though it occurs frequently, is visually jarring.

Auto-generated roads respect the topography: around the hill rather than over it,
along the valley, crossing the river where crossing is cheap. With a few deliberate
exceptions, because real roads have those too.

## Incorporation

Growing a settlement into a city costs the least and demands the most forward
planning, because the population, the economy, and the infrastructure are
already there. The founding grant does not have to buy what already exists.

Settlements can ask to be incorporated. A hamlet that grows until it hits the
density ceiling will, after a while, appeal for township or city status itself.
The flavour is legal: the settlement asking to be recognized because the law will
not let it grow further. Accepting is cheaper than founding, or carries a
different benefit, because the initiative came from them.

Three arrangements are explicitly supported:

- Adjacent settlements, two right next to each other, if the player wants
  that.
- Twin border cities, one on each side of a regional boundary.
- Non-contiguous incorporation. Real cities incorporate a wealthy
  neighborhood or a data center as a bubble outside the main limits regularly.

A player may keep dozens of hamlets and make Oklahoma if they want, and
they should appear across zoned land at a realistic spread or a little wider.

Unlocking a second city does not unlock the ability to build freely across
the region. The region stays the region, governed the way regions are governed
and developed by zoning rather than by placement. What the unlock grants is a
city, plus the regional view, plus the ability to govern unincorporated
settlements.

## National parks

The player can create national parks and forests at real size, spanning whole
tracts of a region, and place facilities inside them for visitors: lodges,
trailheads, ranger stations, campgrounds, and roads that serve access rather than
through traffic.

A park is a zoning outcome that forbids development while permitting a specific
class of building, so it uses the same machinery as everything else here and
produces a different result.

Funding follows the same path as other national services. Until a second region
unlocks the national pool the region pays for its own parks; after that the cost
moves up.

## Buying land from the people who own it

Zoned regional land fills with farms and settlements, and then the city wants to
expand into it. The expansion tile is somebody's farm and buying it is a negotiation
with that landowner, not a menu price:

- It is abstracted into a price, with some owners more stubborn than others.
- That sim and their family get the money. It does not vanish into the
  accounting. This is a real economy, and a landowner who just sold a farm is a
  household that suddenly recieves that capital.

The consequence worth having: a player who zoned agriculture toward a future
city site made that land more valuable, and now pays more for it. Planning ahead
is not free.

## The other nations

The player's country is the simulated one. Everything outside it is a
numerical abstraction and should stay that way.

The director governs the other nations. They hold prices, demand, relationship
state, and their own internal events, and none of that requires simulating their
cities. What reaches the player is trade, migration, and the occasional request
or offer.

A region inside the player's country is different: starting a new region is
like starting over. It is not governed by anyone. The director has been
running a sparse population there, so on arrival there are one to three township
candidates and some basic industry such as timber and agriculture, roughly
equivalent to what the player set up in their first region, or half of it.

## Open questions

- **Land acquisition:** when public infrastructure or anything the player wants
  to build is designated over land somebody owns: purchase at market price, price
  negotiation, eminent domain, or a refusal that forces the player to route around
  it.

