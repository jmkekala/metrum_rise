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

`profiles` shares one immutable validated zoning-profile registry across systems and exports.
Compilation rejects more than 65,535 profiles before assigning nonzero `u16` runtime ids; id `0`
remains reserved. UI colours require six ASCII hexadecimal digits after `#` (surrounding whitespace
is accepted); non-ASCII or otherwise malformed colours return validation errors instead of
panicking on byte slices. `parcels` is the stable parcel store and spatial lookup owner. `config` provides world bounds
and zoning-cell size.

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
- Curved-run spacing rechecks the persisted normalized attachment after advancing a candidate;
  the initial station check alone does not bound the final frontage.
- Every corner must stay within world bounds.
- The parcel must not overlap existing parcels.
- The parcel must not overlap another road-owned corridor.
- The parcel must not overlap an explicit service-building site reservation.
- The parcel must not overlap a committed field (`ECON-07`), including when its profile is free/unzoned.
- Roads with `Edge::no_building_spawn = true` reject parcel attachment.
- A nonzero profile needs at least one legal initial-level asset with a valid shared flat-site
  solution. Zoning preview tests the same support solver used by demand and explicit placement;
  preview and commit never stamp terrain or insert a building. Unsupported lots remain visible
  in red with a reason, but cannot be committed as newly zoned lots. Runtime id `0` remains usable
  for free/unzoned parcels even without compatible building assets.
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

Site feasibility filters the geometrically legal layout: drag previews retain invalid lots in red,
and commit accepts only currently buildable lots. Existing saved parcels are not silently deleted
when blocked. Demand re-evaluates their compatible assets through the same dependency-aware cache;
hovering/repainting exposes a rejection reason. Road, terrain, neighboring-site and asset changes
invalidate affected solutions. Redevelopment ignores the parcel's own occupied site, not neighbors.
This guarantees geometric feasibility under unchanged inputs, not construction regardless of
demand, economic gates, later edits or installed assets.

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
get_zoning_site_dependencies() -> PackedInt64Array
apply_zoning_parcel_at(...)
apply_zoning_parcel_drag(...)
```

Single preview dictionaries include `valid` and `reason`. Packed drag dictionaries additionally
include `valid_count` and per-parcel `colors`; `parcel_count` includes rejected preview lots.
The tool displays the Rust-provided reason and drops retained preview geometry whenever dependency
epochs change, including while the cursor is stationary. Commit always checks current inputs.

Parcel rezone:

```text
get_zoning_parcel_profile_runtime_id_at(...)
apply_zoning_parcel_rezone_drag(...)
```

Drag rezone preview and commit skip parcels that overlap explicit service-building site
reservations, so player zoning cannot claim land already reserved by a city service lot.
The reservation covers the full lot even when imported structures occupy only a small part of it.
Indexed queries use lot extents across chunk boundaries; touching edges retain the existing overlap
tolerance. See the placement-query audit in [`building_allocator.md`](building_allocator.md).

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
`ArrayMesh`. Zoning, service/industry placement, road, and walkway tools show the parcel overlay
and orange no-build edge guides. Empty parcels remain visible constraints during road placement;
the Rust road preview uses the same indexed parcel-corridor intersection check as commit and
returns `parcel_overlap` before submission. Changing parcels invalidates cached road verdicts
through the zoning overlay revision.

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
that fails road-corridor overlap, existing-parcel overlap, or endpoint frontage validation is not
inserted into zoning.
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

Per-candidate placement, preview and rezone conflict checks use local indices. Updating one known
parcel's occupant uses the stable-id map in O(1) expected time. Bulk save/load reconstructs the
stored collection. Replacing one parcel geometry updates only old/new footprint chunks, retaining
shared memberships and storage-order picks. Stable-order parcel removal still rebuilds indices;
that maintenance path remains under `AUDIT-01` review.

### Profile loading audit (`AUDIT-01-Z1`)

The cached loader previously cloned all profile strings, vectors and maps into a new `Arc` per
request. It now stores the `Arc` in the existing `OnceLock`; successful repeated loads are O(1)
and allocate no profile copies. Unused whole-registry/profile `Clone` and empty-registry `Default`
implementations are removed. Compilation also moves the authored ID into its runtime record after
validation, avoiding one redundant string copy. Cold compilation keeps its existing O(P log P)
ordering/validation bound for P profiles.

All three regressions fail before correction: a seven-byte colour containing `€` panics, 65,536
profiles wrap the runtime-id space, and repeated loads return distinct registry instances. The
corrected full release suite passes 1,752 tests (47 ignored), including the valid 65,535-profile
boundary, invalid colours and shared immutable ownership.

Five alternating unprofiled release pairs measured the ignored
`simulation::zoning::profiles::registry::tests::benchmark_cached_zoning_profile_load` test on CPU 0
with `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, three warmups and 21 batches of 10,000 calls. Median
milliseconds per repeated load fell from **0.000793622 to 0.000009598** (about 0.79 µs to 9.6 ns).
Every run preserves the complete zoning style LUT. Initial TOML parsing and LUT construction are
outside the timing; this measures cache retrieval, not world construction or placement throughput.

Build command: `cargo test --offline --manifest-path rust/Cargo.toml --release --lib`, with Rust
1.98.1 (48a229cea 2026-09-01). The runner uses `taskset -c 0`, `--exact`, `--ignored`, `--nocapture`
and `--test-threads=1`. Before/after executable SHA-256 values are
`698c4b37964c9cf76343d56e1402b825eda55e231612a0866582a23062769cd3` and
`6dc89a511294abc67af9e927e17d5c70fce2c83895256bf123e93fd218bf4c10`.
Sources/binaries remained fixed during timings, with no competing builds/tests. Exact commands,
source identities, regression failures and matched results are in
`/tmp/metrum-full-audit/zoning-profiles-*`; the runner is `match_zoning_profiles.py`.

### Parcel occupancy audit (`AUDIT-01-Z2`)

Allocator cleanup, demand/player removal and demolition undo already know the moved building's
parcel ID. They now pass that ID to the existing parcel store instead of scanning every parcel for
an occupant index. A missing parcel or stale expected occupant changes neither state nor revision.
Successful updates retain the separate occupancy revision; geometry revision is untouched. Two
unnecessary whole-building clones in direct removal are also removed. No new index is introduced.

The existing revision test now covers remapping, a stale old occupant and reversal. The demolition
undo fixture now owns real parcels and verifies the surviving parcel before undo and both claims
afterward. The redevelopment fixture now removes one of two buildings and checks the survivor's
moved index; it reuses the existing building fixture and normal rezone mutator instead of copying
all building defaults and changing a parcel field directly. All 1,752 release tests pass (48 ignored).

Five alternating CPU-0 unprofiled release pairs ran
`simulation::zoning::tests::maintenance::benchmark_parcel_occupancy_remap`, with one Rayon worker,
`METRUM_DEBUG=0`, three warmup round trips and 21 samples of 32 remaps. One local occupied parcel
stays fixed while unrelated occupied records occupy distant chunks. Median milliseconds per remap:

| Total parcels | Before | After |
| --- | ---: | ---: |
| 1 | 0.000001281 | 0.000008844 |
| 1,024 | 0.000389406 | 0.000008844 |
| 65,536 | 0.099767656 | 0.000008875 |
| 262,144 | 0.808639344 | 0.000010813 |

Every run preserves the complete `(parcel ID, occupant)` checksum and geometry revision after the
round trips. The added hash lookup costs about 8 ns for one parcel, while larger inputs stay near
9–11 ns instead of scaling with unrelated parcels. Setup, checksum generation and all other
building-removal work are excluded. These are isolated parcel-store records, not a populated
routing simulation; this benchmark does not claim whole-demolition or road-planning timings.

Builds use `cargo test --offline --manifest-path rust/Cargo.toml --release --lib`, Rust 1.98.1
(48a229cea 2026-09-01). `match_parcel_occupancy.py` records `taskset -c 0`, `--exact`, `--ignored`,
`--nocapture` and `--test-threads=1` commands. Before/after executable SHA-256 values are
`81d18be15f6374146486e583785c7892547d663aa21e37a5f16ad38accbc709f` and
`2868aec40c81e8e2b97f117e5d2acd1a7a937c703f0ad1e69be92dce07564907`.
Sources and binaries remained fixed with no competing builds/tests. Source identities, minimal
diffs, raw logs and matched summaries are in `/tmp/metrum-full-audit/parcel-occupancy-*`.

### Parcel geometry maintenance audit (`AUDIT-01-Z3`)

Geometry replacement previously cleared and rebuilt every parcel chunk entry. It now uses the
existing stable-ID lookup, removes memberships only from departed chunks and inserts memberships
only in newly entered chunks. Shared chunks remain untouched. New entries use storage order rather
than numeric parcel-ID order, preserving pick priority even for nonmonotonic loaded IDs. Occupancy
and profile assignment remain unchanged; empty departed buckets are removed.

Work is O(K + C), where K is the old/new footprint's chunk count and C is the total number of parcel
entries examined or shifted in changed chunks. The method uses constant temporary storage and may
allocate new index membership storage. It does not scan distant chunks. Chunk enumeration now
streams the same X-then-Z order instead of allocating a temporary vector. Rectangle overlap shares
the existing SAT routine; profile/geometry queries and drag previews reuse `parcel_at()`.

All 1,753 release tests pass (50 ignored), including the new cross-chunk move/reversal test for
storage order, point/stroke selection and occupancy. Existing curved drag, repair, save, demolition
and undo regressions remain in the full suite. A shared geometry-translation fixture replaces the
copied benchmark transform code.

Five alternating CPU-0 unprofiled pairs measured
`simulation::zoning::tests::maintenance::benchmark_parcel_geometry_replacement`, with
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, three warmup round trips and 21 samples of four replacements.
One local parcel moves between fixed disjoint chunks; distant parcel records grow independently.
Each move checks its point query, and full parcel-corner/occupancy checksums match after reversal.
Median milliseconds per replacement and point lookup:

| Total parcels | Before | After |
| --- | ---: | ---: |
| 1 | 0.000086500 | 0.000104500 |
| 1,024 | 0.033001750 | 0.000105000 |
| 65,536 | 1.735343000 | 0.000101750 |
| 262,144 | 7.109629000 | 0.000125250 |

The one-parcel case costs about 18 ns more; the new path remains local at city scale. Setup, checksum
construction, road-attachment search and building repair are excluded. This is a store-maintenance
measurement, not a complete road-edit timing.

Three alternating eight-worker pairs also ran the existing
`nodes::sim::core::tests::road_plan_scaling::populated_paved_road_plan_scaling` fixture on CPUs
`0,2,4,6,8,10,12,14`. Each build preserves identical local products while background buildings,
parcels, roads and agents increase. At 0 / 1,000 / 10,000 / 100,000 background buildings, worker
medians are **20.656 / 20.731 / 20.796 / 20.753 ms before**, and
**20.807 / 20.799 / 20.791 / 20.903 ms after**. The largest case has 600,024 agents. The separate
one-time snapshot grows from 0.0069 to 3.3666 ms before and 0.0066 to 3.3306 ms after. It is excluded
from repeated planning. These results preserve locality and establish no general planning speedup;
that fixture compares local products across background sizes within each build, not exported
cross-build payload hashes.

Build command: `cargo test --offline --manifest-path rust/Cargo.toml --release --lib`, Rust 1.98.1
(48a229cea 2026-09-01). Before/after executable SHA-256 values are
`62f0ed5f0b40a93b9595315a41d5e2ec3e9b517be34046e0581b290c7ddb48fd` and
`95200aa665985854acdb99d950fa382a04f9666f4a61a230b9fd19f892198fc0`.
Source/binaries remained fixed during measurement with no competing build/test work. Commands,
source identities and raw/summary results are `/tmp/metrum-full-audit/parcel-geometry-*`;
`match_parcel_geometry.py` and `match_parcel_geometry_planning.py` are the replay runners.

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
