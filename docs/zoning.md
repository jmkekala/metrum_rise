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
- Parcel geometry is stored in metres, with parcel dimensions authored in zoning cells converted
  through `WorldConfig::zone_cell_m`.

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

### Parcel Modules

```text
rust/src/simulation/zoning/parcels.rs
rust/src/simulation/zoning/parcels/types.rs
rust/src/simulation/zoning/parcels/store.rs
rust/src/simulation/zoning/parcels/geometry.rs
rust/src/simulation/zoning/parcels/placement.rs
```

- `types.rs`: ids, parcel structs, projected geometry, placement errors
- `store.rs`: stable parcel storage, chunk lookup, occupancy fields
- `geometry.rs`: projection helpers, SAT overlap checks, road-corridor conflict checks
- `placement.rs`: road attachment, single parcel projection, drag-run projection
- `parcels.rs`: constants, module routing, public re-exports

---

## 3. Placement Rules

Parcel placement is all-or-nothing.

- The selected zoning profile must exist, except runtime id `0` for free/unzoned parcels.
- The parcel must attach to a buildable road edge.
- The frontage must stay within the physical road edge span.
- Every corner must stay within world bounds.
- The parcel must not overlap existing parcels.
- The parcel must not overlap another road-owned corridor.
- Roads with `Edge::no_building_spawn = true` reject parcel attachment.

Drag-run placement uses the same validation. On curves, Rust may widen spacing between generated
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

Parcel overlay:

```text
get_zoning_parcels_overlay() -> Array[Dictionary]
```

Road no-build tool support:

```text
set_no_building_spawn(edge_idx, enabled)
get_no_building_spawn(edge_idx) -> bool
get_no_building_spawn_edge_indices() -> PackedInt32Array
```

No Godot API may compute zoning legality or repair parcel placement. Godot may only request,
preview, submit, and render Rust-authored results.

---

## 5. Godot Responsibilities

`godot/scripts/tools/zoning_tool.gd` owns tool input and UI state:

- selected zoning profile
- parcel width/depth in zoning cells
- parcel gap in metres
- single-click create/rezone
- drag-run create
- drag rezone over existing parcels
- preview display

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
building lifecycle, entrance cache, and zone-family demand indices.

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

Changing the flag marks the allocator dirty and rebuilds building entrances so existing buildings
that face a newly blocked edge can be cleaned up by the allocator lifecycle.

---

## 9. Performance Contract

Hot placement checks use existing bounded spatial structures:

- road candidates come from `RegionGraph` spatial queries
- parcel overlap uses `ParcelStore` chunk lookup
- road-corridor conflict checks query nearby road AABBs before SAT tests

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
- road overlap and existing parcel overlap are rejected in Rust
- allocator, demand, save/load, and overlay consume parcel data
