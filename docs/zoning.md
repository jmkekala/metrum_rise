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

Load restores each parcel through `restore_parcel_from_attachment(...)`, which revalidates road
attachment, bounds, existing parcel overlap, and road-corridor overlap before inserting it. Building
parcel occupancy is rebuilt after buildings load.

Old save compatibility is not required for this project stage.

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
