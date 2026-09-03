# Zoning System

This document owns the current zoning design and implementation contract. Update it when parcel
zoning behavior, saves, allocator interaction, or Godot-facing zoning APIs change.

---

## 1. Authority

Zoning authority is Rust-owned road-aligned parcels.

- Godot submits tool input and uploads/display meshes returned by Rust.
- Rust owns parcel geometry, road attachment, overlap checks, stable parcel ids, save/load, and
  building occupancy.
- There is no map-wide zoning paint surface; render resources are derived display only.
- Zoning is not an engineered-ground client. Creating, previewing, resizing, dragging, or rezoning
  parcels must not alter source terrain, visual terrain, road surfaces, or building-site surfaces.
- Parcel geometry is stored in metres, with parcel dimensions authored in zoning cells converted
  through `WorldConfig::zone_cell_m`.
- The default tool parcel is `2 x 2` zoning cells (`20 m x 20 m` with the default `10 m` cell).

The owning Rust module is:

```text
rust/src/simulation/zoning/
```

---

## 2. Data Model

### `ZoningSystem`

```rust
pub struct ZoningSystem {
    pub profiles: Arc<ZoningProfileRegistry>,
    pub parcels: ParcelStore,
    pub config: WorldConfig,
}
```

`profiles` is the validated zoning-profile registry. `parcels` is the stable parcel store and
spatial lookup owner. `config` provides world bounds and zoning-cell size.

### `ZoningParcel`

Each parcel stores:

- stable `ParcelId`
- road `edge_idx`
- road `side`
- `frontage_center_t`
- frontage and depth in metres
- assigned zoning-profile runtime id, with `0` meaning free/unzoned
- optional occupied building index
- front center, center, tangent, normal, corners, and AABB

Parcel ids are persisted and used by buildings. Parcel geometry is reconstructed from road
attachment during load so saves stay road-provenance based.

### Rust Modules

```text
rust/src/simulation/zoning/mod.rs
rust/src/simulation/zoning/constants.rs
rust/src/simulation/zoning/zone_type.rs
rust/src/simulation/zoning/system.rs
rust/src/simulation/zoning/system/{queries,preview,editing,restore,occupancy,validation}.rs
rust/src/simulation/zoning/profiles.rs
rust/src/simulation/zoning/profiles/{runtime,registry,authored,compile}.rs
rust/src/simulation/zoning/parcels.rs
rust/src/simulation/zoning/parcels/types.rs
rust/src/simulation/zoning/parcels/store.rs
rust/src/simulation/zoning/parcels/geometry.rs
rust/src/simulation/zoning/parcels/geometry/{bounds,overlap,road_overlap,polyline,spatial}.rs
rust/src/simulation/zoning/parcels/placement.rs
rust/src/simulation/zoning/parcels/placement/projection.rs
rust/src/simulation/zoning/parcels/placement/run.rs
rust/src/simulation/zoning/parcels/placement/run/spacing.rs
```

- `mod.rs`: public API routing and re-exports
- `constants.rs`: public parcel defaults and edit limits
- `zone_type.rs`: broad land-use family enum
- `system.rs`: `ZoningSystem` state owner
- `system/queries.rs`: read-only parcel lookups
- `system/preview.rs`: non-mutating parcel and stroke previews
- `system/editing.rs`: mutating create, drag-run, and rezone operations
- `system/restore.rs`: save/load parcel restoration from road attachment data
- `system/occupancy.rs`: building claim bookkeeping
- `system/validation.rs`: shared parcel edit/profile validation
- `profiles.rs`: profile module routing and public re-exports
- `profiles/runtime.rs`: density and runtime profile value types
- `profiles/registry.rs`: public registry API and built-in registry cache
- `profiles/authored.rs`: TOML loading for zoning and demand growth profiles
- `profiles/compile.rs`: deterministic profile validation and runtime-id assignment
- `parcels.rs`: parcel module routing and public re-exports
- `types.rs`: ids, parcel structs, projected geometry, placement errors
- `store.rs`: stable parcel storage, chunk lookup, occupancy fields
- `geometry.rs`: geometry helper routing
- `geometry/bounds.rs`: parcel rectangle construction and world-bounds checks
- `geometry/overlap.rs`: SAT rectangle, point, and stroke overlap checks
- `geometry/road_overlap.rs`: road-corridor conflict checks
- `geometry/polyline.rs`: road polyline sampling
- `geometry/spatial.rs`: parcel-local chunk broad-phase helpers
- `placement.rs`: placement helper routing and single parcel projection
- `placement/projection.rs`: world-point to road-frontage projection
- `placement/run.rs`: same-road drag-run projection
- `placement/run/spacing.rs`: curved-run non-overlap spacing search

---

## 3. Placement Rules

Single-parcel placement is all-or-nothing.

- The selected zoning profile must exist, except runtime id `0` for free/unzoned parcels.
- The parcel must attach to a buildable road edge.
- The frontage must stay within the physical road edge span.
- Every corner must stay within world bounds.
- The parcel must not overlap existing parcels.
- The parcel must not overlap another road-owned corridor.
- The parcel must not overlap an explicit service-building site reservation.
- Roads with `Edge::no_building_spawn = true` reject parcel attachment.
- Successful parcel placement records only zoning/legal intent. Terrain integration is deferred
  until `BuildingAllocator` accepts an actual building placement and the `EARTH-02` building-site
  client is registered.

Drag-run placement projects bounded deterministic same-road candidate layouts across the current
drag span, then keeps the best legal layout. Rust may re-layout the candidate phase as the span
changes so legal parcels can pack beside existing parcels and near road-corridor blockers. Layout
selection prefers more legal parcels, then the layout that reaches closest toward the dragged end.
A blocked candidate caused by a road corridor, world edge, existing parcel, or another accepted
candidate, including an explicit service-building site reservation, is skipped rather than
cancelling the whole preview or commit. If no generated candidate is legal, the drag fails without
mutation. On curves, Rust may widen spacing between generated
parcels to preserve non-overlap, then stops when no further parcel fits inside the dragged span.

When dragging from an existing parcel, the first generated parcel starts after:

```text
existing_frontage / 2 + requested_gap_m + new_frontage / 2
```

This keeps the requested gap meaningful for extension runs.

---

## 4. Public Runtime API

Godot calls Rust through `SimulationNode`; Rust state lives under `SimCore`.

Profile registry:

```text
get_zone_profiles() -> Array[Dictionary]
```

Parcel creation and preview:

```text
get_zoning_parcel_preview(...)
get_zoning_parcel_drag_preview_packed(...)
apply_zoning_parcel_at(...)
apply_zoning_parcel_drag(...)
```

Parcel rezone:

```text
get_zoning_parcel_profile_runtime_id_at(...)
apply_zoning_parcel_rezone_drag(...)
```

Drag rezone preview and commit skip parcels that overlap explicit service-building site
reservations, so player zoning cannot claim land already reserved by a city service lot.

Parcel overlay:

```text
try_get_zoning_parcels_overlay_packed() -> Dictionary
```

Road no-build tool support:

```text
set_no_building_spawn(edge_idx, enabled)
get_no_building_spawn(edge_idx) -> bool
try_get_no_building_spawn_lines() -> Dictionary
```

Both renderer payloads return `busy = true` instead of waiting on the simulation mutex; Godot keeps
the previous overlay and retries while the authoritative state is busy.

No Godot API may compute zoning legality or repair parcel placement. Godot may only request,
preview, submit, and render Rust-authored results.

---

## 5. Godot Responsibilities

`godot/scripts/tools/zoning_tool.gd` owns tool input and UI state:

- selected zoning profile
- parcel width/depth in zoning cells
- parcel gap in metres
- single-click create/rezone
- drag-run create, with Rust-authored legal-candidate filtering
- drag rezone over existing parcels
- preview display

Single-parcel hover preview keeps the last Rust-authored legal parcel visible while the mouse is
over an illegal placement position. The preview moves only after Rust returns a new legal parcel.
Changing the selected profile or parcel dimensions clears this retained preview.

Drag preview follows the same retained-preview rule during one drag gesture: while the current
cursor position has no legal candidate set, Godot keeps showing the last Rust-authored legal drag
preview for that gesture. Releasing the mouse commits the displayed retained drag preview when one
exists.

`godot/scripts/renderers/zoning_overlay.gd` renders Rust-authored parcel geometry with an
`ImmediateMesh`. It also draws orange no-build edge guide lines while the zoning tool is active.

Godot must not rasterize zoning state into an authoritative grid or resolve placement conflicts.

---

## 6. Allocator Interaction

The building allocator consumes parcels as private-building candidate authority.

- Candidate discovery scans available parcels.
- Zone legality comes from parcel runtime profile id plus `ZoningProfileRegistry`.
- Placement claims a parcel through `ZoningSystem::occupy_parcel`.
- Removal or allocator remap clears/remaps parcel occupancy through zoning helpers.
- Buildings save their claimed parcel id.

Zoning owns parcel legality and occupancy bookkeeping. The allocator owns asset selection,
building lifecycle, entrance cache, building-site support height selection, and zone-family demand
indices.

---

## 7. Save / Load

Zoning saves parcel records, not a zoning paint surface.

Persisted parcel fields:

- `parcel_id`
- `edge_idx`
- `side`
- `frontage_center_t`
- `frontage_m`
- `depth_m`
- `zone_profile_runtime_id`

Normal load restores each parcel through the same road attachment, bounds, existing parcel overlap,
and road-corridor overlap validation used by `restore_parcel_from_attachment(...)`. A parcel record
that fails road-corridor or existing-parcel overlap validation is not inserted into zoning.
Building parcel occupancy is rebuilt after buildings load.

Old save compatibility is best-effort and must preserve live invariants. The SQLite loader may
quarantine malformed legacy parcel records, then remove buildings and pending demand spawns that
referenced those quarantined parcel ids through the normal lifecycle invalidation hooks. This repair
path is for invalid saved data only; it must never leave illegal parcel geometry in `ZoningSystem`.

---

## 8. Road No-Build Flag

`Edge::no_building_spawn` blocks parcel attachment on that edge.

- Default: `false`
- Automatic: high-speed roads are marked no-build when created
- Player toggle: road properties panel checkbox
- Persistence: saved on `network_edges`
- Topology: split edges copy the flag to both children
- Overlay: zoning tool draws no-build edge guide lines

Enabling the flag runs allocator maintenance immediately, removes buildings facing the newly
blocked edge, and removes zoning parcels attached to that edge so saves cannot retain invalid
parcel attachments. Changing the flag also marks the allocator dirty and rebuilds building
entrances.

---

## 9. Performance Contract

Hot placement checks use existing bounded spatial structures:

- road candidates come from `RegionGraph` spatial queries
- parcel overlap uses `ParcelStore` chunk lookup
- road-corridor conflict checks query nearby road AABBs before SAT tests
- explicit service-site blockers use the allocator building-site chunk index before SAT tests

No full-world zoning scan is part of parcel placement, preview, rezone, save/load, or overlay
generation.

---

## 10. `ZONE-01` Status

`ZONE-01` is complete as the active zoning architecture:

- authored road-aligned parcels replace the previous zoning authority
- parcels may be pre-zoned or free/unzoned
- single-click create/rezone works
- parcel-run drag works, including extension from an existing parcel
- drag rezone works over existing parcels
- hover/drag previews are Rust-authored
- single parcel overlap is rejected in Rust; drag-run overlap candidates are skipped in Rust
- allocator, demand, save/load, and overlay consume parcel data

## 11. Build Granularity

The player chooses how much of the work they do themselves, per area, at any time. Four levels, all
equally valid:

- **Draw a site:** Mark the footprint component of where a building will go up, and let it be built
  there.
- **Plop a building:** Place an individual structure.
- **Zone and let it fill:** Draw roads, zone the land, and let demand buy the lots.
- **Sublet a chunk:** Zone a block of land to a third-party developer and let them build it. A player
  who wants to could sublet most of an entire city, although that is likely a poor idea.

The first three are stepped levels of placement authority. The fourth needs proper management to
avoid becoming a free skip-the-game button.

These levels apply to industrial and freight sites as well as to buildings that fill a lot. A
player composing a terminal out of rail spurs, warehouses, docks, and truck bays uses the same
draw, plop, and zone authority as anyone laying out a neighborhood. [`transit.md`](transit.md)
owns what those pieces are, including the airport classes built the same way.

The same stepping applies one level down, to the repeated pieces inside a site. Parking stalls,
loading bays, aircraft stands, storage racks, container stacks, and tank rows are many identical
pieces filling a drawn area. The player draws the area, chooses spacing and orientation, and the
area fills. Hand-placing each piece stays available.

The fill produces ordinary placed pieces. Each one can be moved, rotated, or deleted afterward.

### Who authors the building

The game generates the building from its own rules and the player's conditions: the district
style from section 11, the economic level of the building, and the site conditions on the lot. A
player painting sites never authors a structure directly.

Plopping lets the player cycle through candidates in the UI before placing, and the generator
produced every candidate.

Authoring a building part by part, at the level Tiny Glade and Miniopolis give their players,
happens in the editor reached from the main menu, outside a running city.
[`asset_editor.md`](asset_editor.md) owns it.

A development contract trades attention for money and control:

- The developer takes a cut. Income and resources from that area are reduced for the life of the
  contract, because someone else is profiting from the land.
- The player cannot build there while the contract runs, or pays a heavy penalty for breaking it.
  Handing over a district means handing it over.
- The developer builds to their own standard. They optimise for their margin, so the density, the
  mix, and the quality are theirs to choose. A subletted district looks like the developer laid it
  out, its grid may miss the city's layout, and it may serve the city poorly.
- Contracts have a term. Land comes back eventually, carrying whatever the developer left, which
  the player then owns and maintains.
- A subletted area still counts as the player's for every obligation. Its traffic, its services,
  its waste, and its unrest are the city's problem while its profit goes elsewhere.

The intended use is to let a player manage growth without the obsessive detail other players
enjoy, trading attention over even zoning for a cost.

The city builder grid players complain about follows from zoning implemented as square cells,
which forces every lot into a rectangle from a small set of sizes and every neighborhood into a
checkerboard whatever roads were drawn around it. The method is published: Vanegas, Kelly, and
Weber, [Procedural Generation of Parcels in Urban Modeling](https://www.cs.purdue.edu/cgvlab/www/resources/papers/Vanegas-Eurographics-2012-Procedural_Generation_of_Parcels_in_Urban_Modeling.pdf). A block is subdivided from its own
boundary, so a curved street yields wedge-shaped lots, an irregular block yields irregular
parcels, and nothing quantises to a cell. Parcels meet each other and meet the street with no
leftover gaps, and a neighborhood's shape follows the roads that made it.

Urban density on a curved street means conjoined rowhouses with wider backs than fronts. The
parcels are wedges, the buildings share party walls, and the frontage stays continuous around the
curve. Procedurally generated buildings are the only thing that make gridless parcels work, and
are a top priority once the engine layer has been fully adopted and adapted.

Real places fix the scale, and the generator must reach both ends:
- [The Italianate style rowhouses of Baltimore, MD:](https://baltimoreheritage.org/bbotw-italianate-rowhouses/) Almost completley uniform, very little
  difference between each building if any at all.
- [The Royal Crescent at Bath:](https://en.wikipedia.org/wiki/Royal_Crescent) Formal, planned in one act, uniform and identical frontage on a
  deliberate curve, with every parcel owning a distinct back-end.
- [The Back Bay rowhouses in Boston:](https://commons.wikimedia.org/wiki/File:1975_BackBay_Boston_4725870095.jpg) Mostly-uniform, with every building having its
  own distinct variations.
- [A Brazilian favela:](https://en.wikipedia.org/wiki/Favelas_in_the_city_of_Rio_de_Janeiro) Nothing platted, parcels of every size and shape, buildings meeting at whatever angle
  the ground and their own structure allowed, incuding underneath and on top of one another,
  circulation emerging as the gaps between what got built. This will only define extremely poor
  areas with very lax building regulations, nearly impossible for the player to reach unless they
  distinctly plan a district that way, or the shape of the bigger automatically generated homeless
  camps.

Brazilian-style favelas must be reachable as real generator output, with the parcel geometry and
building placement producing them. An informal-settlement texture painted over ordinary lots does
not qualify.

Gridless still permits grids. The generator draws continuous parcels first, and may draw a grid
inside one of them where the content or block calls for it: a cookie-cutter subdivision,
Brooklyn-style urban density, or the open regular layout that docks, power and water facilities,
industrial plants, and complex factories use.

### Drawing a lot by hand

In Manor Lords, parcels (known as Burgage Plots) are divided using a flexible, dynamic four-point
construction system. Instead of rigid, pre-sized grids, four unique points appear on the map to
draw custom shapes. The game then calculates the dimensions of the area and slices it into
individual residential parcels. The first two points clicked define the Frontage, and third and
fourth clicks stretch the plot backward setting the Depth.

The player draws individual lot boundaries directly, at the granularity Manor Lords gives its
burgage plots: drag out a shape against the street and the parcel takes it. This is the
per-parcel level of the placement authority above, below drawing a site and above plopping one
building.

The generator and the hand-drawn case produce the same kind of parcel. A drawn lot carries the
same frontage attachment, the same road provenance, and the same occupancy record as a generated
one, so the building generator fills it the way it fills any other irregular parcel.

Site conditions fill an awkward shape. The player sets them per lot and the generator solves
against them:

- Wall to wall, so the building meets its neighbors with no gap.
- A courtyard in the leftover pocket.
- An accessory dwelling on the site.
- Parking on site, with a size and a position.

They make the strips and wedges between streets and existing lots usable. An irregular remnant
that no standard footprint fits takes a wall-to-wall building with the odd corner given over to
a courtyard.

### Rezoning part of a building

A zone applies to part of a building, down to one floor or one unit. A medium-density
residential block sells its ground floor to a commercial tenant, and only that floor changes
zone.

The facade of the changed part follows its new use, and the rest of the building is untouched.
How far it goes depends on the building: a historical structure gets signage and little else,
and an ordinary one takes a full remodel and renovation of that floor.

Plopping into part of a building is the same operation with the player choosing the tenant.
Select the building, select the part, and the menu preview shows that part alone as it would be
generated. This places the alley-facing businesses in section 12: a cellar speakeasy, a club
that feeds from the alleyway, and any commercial, office, or industrial front on the back of a
building that is not a secondary frontage for the pre-existing resident.

Rezoning without plopping produces the same result and picks the tenant itself. Painting a
partial rezone is deferred.

A district is a painted area within a city or region holding at least these things:
- Architectural style; what the generator builds inside the boundary. This is
  how a city gets an old quarter, a colonial waterfront, and a glass financial
  district that each read as their own place.
- Local policy; rules that apply inside the boundary only, including the frontage permissions that
  decide what an alley fills up with.
- Ownership of its services; first responders, civil services, and educational facilities serve and
  are paid for by their own districts first, which affects how well funded each district's services
  are. Emergency services still respond across district boundaries, and they prioritize their own
  citizens, so with multiple simultaneous incidents they answer within their own district first.

## 12. Alleys

Alleys are a first class part of a city. Trash collection and deliveries can often run through
them in large cities and small towns alike, every block's back end is different, and they are used
constantly. The genre mostly overlooks them entirely.

A street is drawn first and building sites follow it. Lots are cut against the road and frontages
align to it, so the result is as regular as the road.

An alley inverts that order. It is what is left over after buildings fill a block from its edges,
so their footprints dictate its shape. The generator fills a block from its street edges with
buildings of differing footprints, takes the alley as the negative space that remains, then places
whatever fits in the wider pockets. That produces varying width, irregular paths, and an uneven
entrance count, all unauthored. 

What follows from that inversion:

- Alley width varies along a single alley, because the buildings either side vary along it. A slot
  wide enough for a single vehicle in one place, a stretch wide enough for a parking court or a
  garden in another, and the width determines what uses are available. A constant-width alley behind
  a diverse city block is immersion breaking unless planned or seeminyl intentional.
- The back ends of buildings vary widely, producing a jagged interior line.
- Entrance count varies by block and are generated in the space between buildings as they zone in.
  There could be two, three, or more, and the path between them is often not direct.
- The leftover space can hold things. An accessory dwelling, a clubhouse, an ADU, a back office, a
  parking court, or a garden, and depending on the zoning, in a large block several at once.



The generative rule: the gap between a building's footprint and its service frontage is a site.
Small gap, nothing happens. Large gap holds:
- courtyard gardens, private or shared between abutting parcels
- businesses fronting the alley itself: bars, baristas, and accessory offices, reached from the
  interior of the block

The wide uniform back alley running between two rows of linear houses is the Miami and Kingman
pattern, and it is a valid target. Drawing the alley yourself can produce a uniform corridor, or it
can be set as a target for that block/district/develepment. It is what a subdivision platted in one
act produces, and the generator should make these readily, without jitter added to disguise their
regularity.

What that neighborhood looks like:
- detached garages opening onto the alley, with the pedestrian route from
  house to garage running down the side of the lot
- accessory dwellings bordering the alley
- highly efficient garbage collection, because the run is straight and off the
  main roads entirely
- less crime than urban alleys, because the alley is residential, overlooked, and constantly used by
  the people who live on it
- few or no businesses, because it is a residential block

All three are supported and equally valid:

1. Emergent. Blocks fill in and the interior left over is the alley.
2. Rule-set. The player states the pattern a district should follow, such
   as wide and straight for a neighborhood, and the generator honors it as the
   district grows.
3. Drawn. The player draws the centerline through the middle of the block
   and the buildings grow to fill the gaps around it.

- Width gates four separate things: fire access, waste collection, whether dwellings are legal, and
  how many vehicles fit.
- Block size, local polict, and player will decide whether a block has an alley at all. Some blocks
  may not have them.
- The network degrades two ways: vacation, where a segment returns to private ownership and can
  turn a through-alley blind, and maintenance decay, where it still exists in law and rots anyway.
- Alley frontage is a legal state. A policy toggle for whether alleys may host dwellings, or
  commerce, reproduces observed history. This belongs to district policy in section 11.
- Services and waste collection are slower in irregular alleys and faster in straigh-wide: a
  street-appearance benefit bought with an operating-cost penalty.
- Crime is a result of emptiness. Occupancy and use fixes, lighting does not.
- Alleys get cheaper relative to front-loading as lots get narrower, which is the historical
  density regime.

Canals can fill some of the same roles as the alley for small watercraft. The `Water` frontage role
below records it.

## 13. Water Frontage

A parcel may front navigable water, and the `Water` frontage role in `building_allocator.md`
records that the water is reachable from here.

Canal frontage is overwhelmingly recreational in amenity value, and it prices into land value
under those conditions. In Florida, a house on a canal might have a boat behind it, or it even
might be on stilts with canal access underneath it. The boat goes out at weekends, possibly to
the corner store, and the household otherwise commutes by road unless the canal beats the
alternatives. Very few people take a watercraft to work because they would likely have to dock
and transfer to another mode of transportation.

The role states that the water is reachable and leaves usage to the trip planner comparing costs.
In an ordinary Florida-shaped city that planner answers no for most commuting and yes for leisure.
Gondolas ride these canals like a taxi, and shipping commercial goods by canal is possible with
wider canals that would commonly be found in residential areas and downtown.

## 14. Parking Supply

Parking lots, garges, and underground parking can be zoned, plopped, or drawn just like buildings. 
Parking is a land or street use, with intentional parking stalls as a per-building exception. The
player chooses how to supply the spaces, and every option carries a cost:

| Supply | Cost | Land | What it costs you |
|---|---|---|---|
| Open surface lot | Cheapest | Most | A district that solves parking this way stops being walkable |
| Multistory garage | Higher | Far less | A large blank object that damages the frontage around it |
| Underground | Highest by a wide margin | None visible | Constrained by what is under the ground, which terrain and water already know |
| Curbside lane | Lowest | Road width | Competes with travel lanes and loading; the `Parking` lane kind in `roads.md` |
| Alley parking court | Low | Block interior | What the back of the block was historically for |

The player trades land, money, and appearance against each other. Demand for these spaces is owned
by `traffic.md`; this document owns the supply as a land use.
