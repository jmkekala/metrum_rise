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
- Parcel geometry is stored in meters, with parcel dimensions authored in zoning cells converted
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
- frontage and depth in meters
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
- parcel gap in meters
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

The player chooses how much of the work they do themselves, per area, at any
time. Four levels, and none of them is the intended one.

- Draw a site. Mark the footprint where a building will go up, and let it be
  built there.
- Plop a building. Place an individual structure exactly where it goes.
- Zone and let it fill. Draw roads, zone the land, and let demand buy the
  lots. This is the mode the system above already implements.
- Sublet a chunk. Zone a block of land to a third-party developer and let
  them build it. A player who wants to could sublet most of an entire city,
  althought it is likely not a good idea.

The first three are stepped levels of granularity of placement authority. The
fourth is a different kind of thing and needs proper management or it is a free
skip-the-game button.

A development contract trades attention for money and control:

- The developer takes a cut. Income and resources from that area are reduced
  for the life of the contract, because someone else is profiting from the land.
- The player cannot build there while the contract runs, or pays a heavy
  penalty for breaking it. Handing over a district means handing it over.
- The developer builds to their own standard. They optimise for their margin,
  so the density, the mix, and the quality are theirs to choose. A subletted
  district will not look like one the player laid out, its grid might not match
  the cities layout, and it may not serve the city the way the player would have.
- Contracts have a term. Land comes back eventually, and what comes back is
  whatever the developer left, which the player then owns and maintains.
- A subletted area still counts as yours for every obligation. Its traffic,
  its services, its waste, and its unrest are the city's problem even while its
  profit is not.

The intended use is to allow a player to manage growth without going into the level
of obsessive detail other players enjoy. The tradeoff should feel like a decision
about attention and playstyle rather than a shortcut.

The city builder grid players complain about is not a style choice. It follows from
zoning implemented as square cells, which forces every lot into a rectangle from a
small set of sizes, which forces every neighborhood into a chequerboard whatever roads
were drawn around it. The method is published: Vanegas, Kelly, and Weber, *Procedural*
*Generation of Parcels in Urban Modeling*. A block is subdivided from its own boundary,
so a curved street yields wedge-shaped lots, an irregular block yields irregular
parcels, and nothing quantises to a cell.

The result is seamless and gridless: parcels meet each other and meet the
street with no leftover gaps, and a neighborhood's shape follows the roads that
made it.

Two real places fix the range, and a generator that cannot reach both ends is
not general enough:
- The Royal Crescent at Bath: Formal, planned in one act, uniform and identical
  frontage on a deliberate curve, with every parcel owning a distinct back.
- A favela: Nothing platted, parcels of every size and shape, buildings
  meeting at whatever angle the ground and their own structure allowed, circulation
  emerging as the gaps between what got built.

Procedurally generated buildings are what make gridless parcels work, because a
lot that changes shape needs a building that changes with it.

Urban density on a curved street means conjoined rowhouses with wider backs
than fronts. The parcels are wedges, the buildings share party walls, and the
frontage stays continuous around the curve. That is the Royal Crescent case,
and it is unreachable for any generator that starts from squares or straight
edges without curves.

Gridless does not mean never gridded. The generator draws seamless parcels
first, and may draw a grid inside one of them where that is what the content
calls for: a cookie-cutter subdivision, Brooklyn-style urban density, or the
open regular layout that docks, industrial plants, power and water facilities,
and complex factories actually use. The grid becomes a tool the generator
reaches for rather than the frame everything is forced into.

Brazilian-style favelas must be reachable, as a result the generator can
produce rather than an informal-settlement texture painted over ordinary lots.

A district is a painted area within a city or region holding at least these things:
- Architectural style; what the generator builds inside the boundary. This is
  how a city gets an old quarter, a colonial waterfront, and a glass financial
  district that read as different places rather than one texture.
- Local policy; rules that apply inside the boundary and not outside it,
  including the frontage permissions that decide what an alley fills up with.
- Ownership of it's services; first responders, civil services, and educational
  facilities first serve and are paid for by their own districts, which affects how
  well funded each districts services are. Emergency services still respond across
  districs boundaries, but they prioritze their own citizens first, so if multiple
  incidents are occuring at once emergency services will first answer within their
  own district.

## 12. Alleys

Alleys are a first class part of a city. Trash collection and deliveries run
through them in large cities and small towns alike, every block's back end is
different, and they are used constantly. The genre moslty overlooks them entirely.

A street is drawn first and its buildings follow it. Lots are cut against the
road, frontages align to it, and the result is regular because the road was.

An alley is the opposite. It is what is left over after buildings fill a
block from its edges, so its shape is dictated by their footprints rather than
the other way round. Drawing the alley first and hanging buildings off it
produces the uniform corridor that reads as fake.

What follows from that inversion:

- Alleys are not one width. A slot wide enough for a single vehicle in one
  place, a stretch wide enough for a parking court or a garden in another, on
  the same alley, and their width determines what uses are available to it.
- Width varies along a single alley, because the buildings either side vary
  along it. A constant-width alley is the tell that a machine drew it.
- The back ends of buildings do not line up. They vary widely, and the
  jagged interior line that produces is the thing worth rendering.
- Entrance count varies by block. Two, three, four or more, and the path
  between them is rarely direct.
- The leftover space holds things. An accessory dwelling, a clubhouse, an ADU, a
  back office, a parking court, or a garden, and in a large block several at
  once.

The generator fills a block from its street edges with buildings of differing
footprints, takes the alley as the negative space that remains, then places
whatever fits in the wider pockets. That produces varying width, irregular
paths, and an uneven entrance count without any of those being authored. A
city where every alley holds a garden is a possible outcome of that placement
rather than a special mode.

When a parcel is deep enough that the building cannot naturally meet the alley
at the back, the leftover ground between the two is not waste. It is the site
that produces:
- courtyard gardens, private or shared between abutting parcels
- businesses fronting the alley itself rather than the street: bars,
  baristas, accessory offices, reached from the interior of the block

The generative rule: the gap between a building's footprint and its service
frontage is a site. Small gap, nothing happens. Large gap, and the game has
somewhere to put a courtyard or a second business, so the emergent alley takes
its content from parcel geometry rather than from a table of decorations.

The wide uniform back alley running between two rows of linear houses is the
Miami and Kingman pattern, and it is not a degenerate case to avoid. It is
what a subdivision platted in one act produces, and the generator should make
these readily, without jitter added to disguise their regularity, because
the regularity is the truth about how they were made.

What that neighborhood looks like:
- detached garages opening onto the alley, with the pedestrian route from
  house to garage running down the side of the lot
- accessory dwellings bordering the alley
- highly efficient garbage collection, because the run is straight and off the
  main roads entirely
- less crime than urbal alleys, because the alley is residential, overlooked, and
  constantly used by the people who live on it
- few/no businesses, because it is a residential block

All three are supported and none is the intended one:

1. Emergent. Blocks fill in and the interior left over is the alley.
2. Rule-set. The player states the pattern a district should follow, such
   as wide and straight for a neighborhood, and the generator honors it as the
   district grows.
3. Drawn. The player draws the centerline through the middle of the block
   and the buildings grow to fill the gaps around it.

- Width gates four separate things, not one: fire access, waste collection,
  whether dwellings are legal, and how many vehicles fit.
- Block size and era decide whether a block has an alley at all. Small
  blocks never needed them.
- The network degrades two different ways: vacation, where a segment
  returns to private ownership and can turn a through-alley blind, and
  maintenance decay, where it still exists in law and rots anyway.
- Alley frontage is a legal state. A policy toggle for whether alleys may
  host dwellings, or commerce, reproduces observed history rather than
  inventing a mechanic. This belongs to district policy in section 11.3.
- Alley waste collection is slower and more popular. It is a
  street-appearance benefit bought with an operating-cost penalty, and it wins
  politically anyway.
- Danger is a function of being empty and permeable, and lighting does not
  fix it. Occupancy does. That gives four player levers with four
  evidence-backed outcomes, one of which should visibly fail.
- Alleys get cheaper relative to front-loading as lots get narrower, which
  is exactly the density regime where they historically appear.

In the Florida case the canal fills the same role as the alley, but for boats
and jet skis. A house has water behind it the way another house has an alley
behind it, and the `Water` frontage role records exactly that.

See section 12 for why that frontage prices as recreation rather than transit.

## 13. Water Frontage

A parcel may front navigable water, and the `Water` frontage role in
`building_allocator.md` records that the water is reachable from here.

Canal frontage is overwhelmingly recreational and amenity value, and it prices
into land value on those terms. In the Florida case a house on a canal has a
boat behind it, the boat goes out at weekends, possibly to the corner store,
and the household commutes by road like everyone else unless the canal is the best
option. very few people take a watercraft to work. It is possible but unlikely,
because they would have to dock and transfer. The realistic uses are recreation
and the short local errand.

The role deliberately says only that the water is reachable and leaves whether
anybody uses it to the trip planner comparing costs. In an ordinary
Florida-shaped city that planner answers no for commuting and yes for leisure,
which is the correct result rather than a failure of the transit model.

Working canals exist and stay buildable: freight on water, a canal district that
genuinely moves goods. That is a deliberate build, not the default reading of a
house with water behind it.

## 14. Parking Supply

There is space in this simulation to account for real parking, and to make the
player face the tradeoff rather than have a number satisfy it. Parking is not
natively a per-building attribute, excepting intentional parking lots. A vehicle
occupies a physical space, and the player chooses how to supply those spaces.
No option wins outright:

| Supply | Cost | Land | What it costs you |
|---|---|---|---|
| Open surface lot | Cheapest | Most | A district that solves parking this way stops being walkable |
| Multistory garage | Higher | Far less | A large blank object that damages the frontage around it |
| Underground | Highest by a wide margin | None visible | Constrained by what is under the ground, which terrain and water already know |
| Curbside lane | Lowest | Road width | Competes with travel lanes and loading; the `Parking` lane kind in `roads.md` |
| Alley parking court | Low | Block interior | What the back of the block was historically for |

Land against money against appearance. Demand for these spaces is owned by
`traffic.md`; this document owns the supply as a land use.
