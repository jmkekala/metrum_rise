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

## Size, shape, and composition

A country is roughly 25 regions, and a region subdivides into tiles.

The country holds `100,000 km2` of buildable land, roughly `125 km x 800 km`
measured along the shape of a sideways U. The inland sea sits outside that figure
and holds roughly `50,000` to `75,000 km2` of open sea with dispersed island
chains, atolls, and a large, towering volcano sitting at the center. This volcano
smokes for the length of the entire game but does not go off.

There are about 20 buildable regions that land along the U and 5 or 6 are mainly
ocean. A land region averages `5,000 km2` and is sized from the seed inside a
`50 km` to `75 km` band; an ocean region runs `8,000` to `15,000 km2`.

Region shape follows the environment. A region is square only where the ground is
flat enough to offer no natural feature to bind an edge to. [`terrain.md`](terrain.md) owns
the generator that draws them.

Tile boundaries follow the world. Strips run along roads, natural features,
mountains, and rivers. Where no such feature exists the generator draws abstract
district-shaped tiles.

## Founding

Two choices, in order: first the region, chosen from the 20 to 30 with
difficulty and known resources visible, then the tile within it.

The region choice is the single most consequential decision in a run and it is
made before anything is built.

Tile selection runs either way:

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

The player's very first city gets a bootstrap grant. Every successive city must
source its grant from the regional budget.

It buys the city's initial limits. The grant purchases the inner region tiles
around the founding point, so a city's opening footprint is set by how much money
it was given plus whatever income it inherited. A well-funded founding starts
wide; a shoestring one starts tight and grows.

## Expansion

The player may buy tiles adjacent to the city, or one tile beyond that. Reach is
short, keeping growth contiguous.

Limits also move without purchase. A city naturally acquires neighboring tiles as
it expands, slowly and at no cost, so a player who never touches the tile map still
absorbs land. Purchase is how a player takes a tile they want now, for a reason: a
deposit, land value, a route.

The second city is unlocked by playing. After that, cities unlock on
population milestones across all citizens.

The milestone grows with every city held. If the first threshold is 100,000
with one city, the third might be 500,000 with three. The curve rises faster than
the city count, so each new city is earned against a higher bar than the last.

That shape does the balancing on its own. A player who builds one deep city and a
player who spreads wide both progress. What it forbids is founding a dozen towns
and letting them stagnate, because a stalled city counts toward the next
threshold without contributing to reaching it.

A new city holds most of the player's attention for its next few hours, until it
is stable enough to leave for longer than a few minutes. That, along with the
population threshold, is what stops eight cities being founded at once.

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

What is gated separately is kind. Outside city limits the buildable set is the
basic one: low-density residential, basic industrial, commercial, and office.
Specialized industry belongs to a city and is admitted to regional land only under
a documented exception, such as an extractor bound to a deposit that is simply
where the deposit is. `economy.md` owns which profiles fall on each side of that
line.

Zoned regional land develops on its own. Any cluster the director finds
sufficient for what that zoned area is doing becomes a settlement, so a timber
region grows timber towns and a mining region grows mining camps. Hand-placing
a doctor, a police station, a gas station, and a fire station within a hundred
feet of each other is a deliberate act of centralization, and the director grows
a settlement around that core.

A stated law caps regional settlement:

| Quantity | Value |
|---|---|
| A square mile | 640 acres |
| Smallest regional residence | half an acre |
| Average household | 2.5 people |
| A fully packed square mile | 6,400 people |
| The law allows | 2,560 per square mile, four people per acre |

The gap between what fits and what is allowed forces the director to concentrate
density instead of spreading it evenly, producing distinct hamlets in zoned land.

Two thresholds follow:

- 960 per square mile is a township or hamlet. The line where a cluster
  stops being scattered houses and becomes a place with a name.
- 2,560 is the ceiling. A settlement at the cap stops growing until it is
  upgraded or incorporated.

Ordinary settlements should never exceed 1,000 by more than a few dozen coming
and going, with 2,500 reachable only where the player has deliberately pushed.
The countryside is sparse and should read that way.

### The linear corridor

The cap applies per square mile (we can covert to kilometers later 1.4, I know,
this is just how my dumb Amrtican brain works), so it constrains density.
Arrangement is free. A player who reads that can run dozens of settlements in a
line along a navigable river with a road beside it, and hold a large population
outside any city while every individual square mile stays legal. Study the Ohio
River valley below Pittsburgh, where a chain of separate small communities runs
roughly a hundred miles downriver as one continuous built strip without ever
becoming a city. The density law produces that shape once the player supplies the
line.

The corridor's advantage is frontage. A hundred-mile strip one mile wide has the
same area, and therefore the same legal population, as a ten-mile square, while
touching ten times as much river. Every settlement on it is a barge stop, and the
road down the spine stays short because it never reaches across anything. It pays
for that in distance. The worst trip along a hundred-mile spine is a hundred miles,
against about fourteen corner to corner in the equivalent square. Commutes,
dispatched services, and freight all cross the full length, so a corridor that
outgrows its transport fails on response times while its density stays legal. The
player chooses between frontage and travel distance.

Placement there stays limited to the basic uses allowed outside city limits:
low-density commercial and industrial, residential, and the occasional office.
Specialized industry stays inside a city unless a documented exception admits it.
Director-generated regional plots run from about two acres up to half a mile by
one mile, 320 acres, depending on what is being grown. Non-grid plots follow
the same size range.

In genuinely flat country the generator prefers one mile by one mile Jeffersonian
grids. The grids orientation should choose North-South if there are no features
nearby worth of anchoring too instead, and the grid direction should occasionally
change to match significant roads or natural features as the progresses and it fills
in more.

It must generally lay partial grids and never grid an large flat area completely
uniformly, even if it would be the most realistic thing to do. A two by five mile
block, a ten mile stretch with scattered grids growing off it, an ungridded area,
or sometimes straight roads that a grid later grows off. Uniformity across a whole
region is repetitive froma gods-eye-view camera.

Auto-generated roads respect the topography: around the hill, along the valley,
crossing the river where crossing is cheap. A few deliberate exceptions appear,
because real roads have those too.

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

Unlocking a second city leaves regional building rules unchanged. The unlock
grants a city, the regional view, and the ability to, with limits, govern
unincorporated communities.

## National parks

The player can create national parks and forests at real size, spanning whole
tracts of a region, including into regions they do not operate yet and place
facilities inside them for visitors: lodges, trailheads, ranger stations,
campgrounds, and access roads that carry no through traffic.

A park is a zoning outcome that forbids development while permitting a specific
class of building, so it uses the same machinery as everything else here and
produces a different result.

Funding follows the same path as other national services. Until a second region
unlocks the national pool the region pays for its own parks; after that the cost
moves up.

## Land acquisition

When a city expands into zoned regional land, the expansion tile is somebody's
farm, and buying it is a negotiation with that landowner:

- It is abstracted into a price, with some owners more stubborn than others.
- That sim and their family get the money, and it stays in the economy. A
  landowner who just sold a farm is a household that suddenly receives that capital.

A player who zoned agriculture toward a future city site made that land more
valuable, and now pays more for it.

## The other nations

The player's country is the simulated one. Everything outside it is a numerical
abstraction.

The director governs the other nations. They hold prices, demand, relationship
state, and their own internal events, with no cities simulated. What reaches the
player is trade, migration, and the occasional request or offer.

A region inside the player's country works differently: starting a new region is
somewhat like starting over. The director has been running a sparse rural population
there in conformance to the population law, so on arrival there are one to three
township candidates and some basic industry such as timber and agriculture, roughly
equivalent to maybe 2/3 of what the player set up in their first region, the rest of
their initial regional funding comes from ticket price of purchasing the region.

## Open questions

- **Land acquisition:** when public infrastructure or anything the player wants
  to build is designated over land somebody owns: purchase at market price, price
  negotiation, eminent domain, or a refusal that forces the player to route around
  it.
- **Infrastructure in unowned regions:** certain projects need to cross a
  region the player has not bought, which holds only the small unincorporated
  communities the director runs. A mountain region nobody wants to build in
  may still need complex highway and tunnel systems through it. How far the
  permission goes is undecided: transit corridors alone, corridors plus
  extraction, or anything non-residential.
- **Foreign operations in unowned regions:** whether external companies may be
  sublet the right to prospect and set up operations in regions the player
  does not control yet, and if so whether the player inherits or buys out
  those operations when the region is purchased. `economy.md` owns the money
  and `narrative.md` owns the companies making the offer.

