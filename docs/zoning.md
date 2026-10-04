# Zoning System

This document owns the current zoning design and implementation contract. Update it when parcel
zoning behavior, saves, allocator interaction, or Godot-facing zoning APIs change.

Sections 1–10 describe the shipped parcel workflow. [Section 11](#11-road-generated-cell-zoning--zone-04)
owns the additional cell workflow under implementation; it does not retire parcel zoning.

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
- Run spacing checks the final station and keeps its normalized saved attachment within the
  strict frontage bounds. If normalization rounds a legal endpoint outside those bounds, it
  moves one representable float inward; actual frontage overhang still fails.
- Every corner must stay within world bounds.
- The parcel must not overlap existing parcels.
- The parcel must not overlap another road-owned corridor.
- The parcel must not overlap an explicit service-building site reservation.
- The parcel must not overlap a committed field (`ECON-07`), including when its profile is free/unzoned.
- Roads with `Edge::no_building_spawn = true` reject parcel attachment.
- Missing compatible initial-level assets do not block zoning, including when installed assets
  have another density, require a later building level or exceed the selected lot dimensions.
  Such parcels retain their selected profile and wait for compatible content before growth.
- Every density tests the selected lot's level interior and shared 2 m perimeter grading strip
  through the existing site-support solver, independent of installed assets. The footprint follows
  the selected frontage and depth; tiny lots use the same capped inset as building lots. Failed
  road, terrain or neighboring-site tie-ins remain red and cannot become new zoned lots.
  Preview and commit never stamp terrain or insert a building. Runtime id `0` remains available
  for free/unzoned parcels even when terrain support fails.
- Successful parcel placement records only zoning/legal intent. Terrain integration is deferred
  until `BuildingAllocator` accepts an actual building placement and the `EARTH-02` building-site
  client is registered.

An unanchored drag projects bounded deterministic same-road candidate layouts across the current
drag span, then keeps the best legal layout. Rust may re-layout the candidate phase as the span
changes so legal parcels can pack beside existing parcels and near road-corridor blockers. Layout
selection prefers more legal parcels, then the layout that reaches closest toward the dragged end.
A blocked candidate caused by a road corridor, world edge, existing parcel, or another accepted
candidate, including an explicit service-building site reservation, is skipped rather than
cancelling the whole preview or commit. If no generated candidate is legal, the drag fails without
mutation. On curves, Rust may widen spacing between generated
parcels to preserve non-overlap, then stops when no further parcel fits inside the dragged span.

Site feasibility filters every geometrically legal layout; previews retain failed lots in red.
The common lot result is cached by footprint and road attachment, without profile or asset keys,
so switching density cannot change its terrain verdict. Road, terrain and neighboring-site edits
invalidate affected solutions; asset registration alone does not. Geometry and reservations remain
enforced independently. Existing saved parcels are not silently deleted when blocked. Demand uses
the same grading solver with each actual asset's footprint and dependency-aware cache; a smaller
building may still fit an existing lot whose full zoning pad fails. Redevelopment ignores the
parcel's own occupied site, not neighbors. Buildings still require compatible assets, valid site
support, demand and economic eligibility at construction time.

When dragging from an existing parcel, the first generated parcel starts after:

```text
existing_frontage / 2 + requested_gap_m + new_frontage / 2
```

This keeps the requested gap meaningful for extension runs. Manual drags and automatic road fill
share the same anchored layout routine in `system/preview/road.rs`. Existing lots within or beside
the dragged interval anchor manual placement even when the gesture starts on empty ground. A drag
affects only its selected side and interval; an unanchored drag retains its free-placement layout
search. Standalone off-road clicks still place one parcel at the cursor.

### Both-side road zoning (`ZONE-03`)

Hovering a road or its sidewalk previews new parcels along both sides of that graph edge, using
the selected profile, frontage, depth and gap. The target is one segment between graph nodes;
selection does not traverse connected streets. Equal-distance road picks use the lowest edge ID.
No-build roads remain hover targets but reject placement.

On an empty side, candidates start half a frontage from the physical edge start. Otherwise, existing
parcels anchor that side: fill extends toward both road ends from the outermost lots and from both
ends of each gap between existing lots. The first offset uses the existing and selected frontages
plus the requested gap. Different widths, insertion order and the opposite row do not impose a
new station grid on a manual group. Full-size lots are preserved; if a gap between separate groups
cannot hold another full lot, the remainder stays between the new rows. Existing parcels are never
moved or resized to consume that remainder.

Each advancing front steps by at least `frontage + gap`, using the manual drag spacing solver.
Parcel rectangles measure frontage in XZ while road stations measure 3D distance; a slope or bend
can therefore need a small additional advance to clear an overlap. The road fill adjusts that
station instead of discarding an entire lot. Rows remain independent of pointer movement, and a
temporary `ParcelStore` checks all projected candidates through the existing local index. Existing
parcels participate in spacing before final filtering, so overlaps shift the row instead of
discarding lots from an unrelated endpoint grid. A queue retires completed fronts; opposing fronts
update their stop limits as they advance, avoiding searches through already filled spans.
Endpoint normalization cannot cancel a row merely because its first legal station rounds just
outside the road span when converted to a saved attachment.
The existing run validator then skips lots outside the world, occupied by existing parcels or
intersecting other road corridors. Site feasibility marks unsupported
geometry red, with the same terrain, service-site and field checks used by drag zoning.
This operation creates new lots in available space; existing parcels keep their profiles.

A left-button press commits the currently valid lots on both sides immediately. Rust re-picks the
road and validates the layout under one simulation lock; a stale hover ID cannot select another
edge at commit. Release performs no second action. Zoning remains terrain-neutral, and ordinary
off-road single placement, extension drags and rezone gestures retain their existing behavior.

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
get_zoning_road_at(world_x, world_z) -> int # edge id, or -1 outside road corridors
get_zoning_road_preview_packed(edge_idx, profile, frontage_cells, depth_cells, gap_m)
get_zoning_site_dependencies() -> PackedInt64Array
apply_zoning_parcel_at(...)
apply_zoning_parcel_drag(...)
apply_zoning_road_at(world_x, world_z, profile, frontage_cells, depth_cells, gap_m)
```

Single preview dictionaries include `valid` and `reason`. Packed drag and road dictionaries additionally
include `valid_count` and per-parcel `colors`; `parcel_count` includes rejected preview lots.
Reasons remain available in the Rust API for diagnostics. The zoning tool uses preview colors for
feedback and shows no cursor warning text, including after filling a road or for frontage failures.
It drops retained preview geometry whenever dependency epochs change, including while the cursor
is stationary. Commit always checks current inputs.

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
- both-side road hover preview and immediate press-to-zone
- single-click create/rezone
- drag-run create, with Rust-authored legal-candidate filtering
- drag rezone over existing parcels
- preview display

Single-parcel hover preview keeps the last Rust-authored legal parcel visible while the mouse is
over an illegal placement position. The preview moves only after Rust returns a new legal parcel.
Changing the selected profile or parcel dimensions clears this retained preview.

Road previews cache the selected edge, profile, dimensions, gap and existing site dependencies.
Pointer movement along the same edge reuses the mesh. Switching edges, changing options or changing
dependencies rebuilds it, even with a stationary cursor. A blocked road clears prior preview
geometry; road previews are never retained over another road or adjacent land.

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
- Parcels without compatible initial assets remain zoned but produce no growth candidate. Loading
  suitable assets makes them eligible through the existing discovery and registry invalidation path.
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

Road hover uses the allocation-free edge visitor in O(log E + S), where E is indexed edges and S
is polyline segments in the existing 128 m corridor-query neighborhood. Uncached road geometry
costs O(Q * S_e + P * log E + K log K + C), where P is projected lots, S_e is the selected edge's
polyline size, K is existing anchors, Q is spacing-solver probes, and C is total local
parcel/road/chunk overlap work. Anchor lookup clips the road polyline to the requested station
interval and visits its parcel chunks once, then sorts matching attachments by station and ID.
Manual lookup includes one maximum attachment offset beyond each gesture boundary; it never
collects the whole road's parcels for a short drag. Its final ordering adds O(P log P).
The existing solver
advances by 0.5 m while blocked and refines each accepted bracket in ten binary steps, giving
Q = O(L / 0.5 m + 10P + K) for the covered road length L. Temporary projected parcels reuse `ParcelStore`, and
overlap-query scratch is reused across probes; no probe scans the entire previously projected row.
The shared run validator supplies final legality filtering. Ordered acceptance is sequential because
each accepted lot constrains later lots; this editor action does not iterate over city residents.
The advancing-front queue is preallocated for at most two fronts per span and uses O(K + P) queue
operations, without repeatedly visiting completed spans. No persistent spatial structure is added.
Only a target/options/dependency change allocates and rebuilds the road preview payload/mesh.

`ZONE-03` spacing correction (2026-09-13): fixed stations discarded whole lots when road grade
or curvature produced a small XZ overlap. New regressions fail on the original implementation:
a 120 m road rising 18 m fits only 6 rather than 12 lots, and a 140 m-radius inner curve fits only
5 rather than the 8 lots produced by manual extension. Reusing the drag-run spacing solver with
an indexed projected-parcel query corrects both. The solver now probes the remaining endpoint
interval before rejecting a last lot when its 0.5 m search step would overshoot; forward and
reverse graded extensions both keep that last lot. Saved frontage bounds and overlap tolerance
remain enforced. Earlier tests checked non-overlap but missed packing density.

`ZONE-03` endpoint correction (2026-09-13): on some road lengths, dividing the first legal
station by edge length and multiplying it back rounds below half a frontage. Both rows stopped
before generating any candidate. A controlled 140.17 m acute-junction fixture accepts nine manual
lots but rejects automatic fill before this fix; the fix retains at least nine automatic lots
in total across both sides, and their saved attachments restore successfully. A second regression
covers rounding at either endpoint and keeps actually out-of-bounds requests rejected. Both tests were run and
failed before the correction. This reproduces the reported symptom, not the unavailable original
map. Normalizing a legal endpoint inward costs O(1) per spacing probe with no allocation.

Fresh verification after shared anchored fill: the full release library suite passes
**1,779 tests** (61 ignored), including flat/graded/curved manual groups, mixed frontages,
nonzero gaps, insertion-order independence, matching manual/automatic layouts, identical hillside
terrain verdicts across profiles, missing-content growth eligibility, field reservations,
endpoint/spacing and cross-system tests.
Headless `zoning_road_tool_test.gd` passes all nine zoning profiles without assets, matching hillside
red/valid masks across residential densities, single-lot terrain rejection and partial road commit,
preview/commit geometry equality, invalid-profile and no-build rejection, repeated clicks, cache
invalidation and off-road gestures. It also compares manual and automatic packed geometry around
three- and five-lot manual groups, verifies zero-gap joins at both ends, exact preview/commit
equality, unchanged authored parcels/profiles and a second click producing no duplicates.
The headless `road_junction_preview_test.gd` also passes. Rustdoc and benchmark-target compilation pass.
The rebuilt release library is deployed to
`godot/bin/libmetrum_rise.so`.

The earlier endpoint-correction measurement used three alternating unprofiled release pairs with the ignored
`simulation::zoning::tests::placement::road::benchmark_road_zoning_locality` test with a fixed
120 m road and twelve 20 x 20 m lots while distant roads and parcels increase together. CPU 0,
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, three warmups and nine samples of 64 calls are fixed.
Both builds measure the graded case that exercises spacing adjustment.
Median of run medians, in microseconds:

| Background roads and parcels (each) | Flat preview before | Flat preview after | Graded preview before | Graded preview after |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 3.620 | 3.480 | 14.243 | 14.877 |
| 1,000 | 4.185 | 4.081 | 14.829 | 15.519 |
| 10,000 | 4.280 | 4.274 | 14.961 | 15.627 |
| 100,000 | 4.656 | 4.478 | 15.094 | 15.986 |

The extra normalization during refinement adds 0.6–0.9 µs to the graded fixture (4–6%); flat previews
show no regression and work remains local. Road picking remains 0.017–0.052 µs.
The graded case verifies twelve lots and neighboring frontage gaps within
2 mm; flat before/after checks preserve twelve lots at matching local stations and sides. Setup
is timed separately and excluded. These measurements cover Rust picking and geometry validation,
excluding site feasibility, Godot transfer/rendering and resident simulation. Sources and binaries
stay fixed during measurement with no competing build/test work. Historical first-implementation
and spacing results remain in `/tmp/metrum-zoning-road/` and `/tmp/metrum-zoning-gap/`; they do not
validate this endpoint correction.

Build: `cargo test --offline --manifest-path rust/Cargo.toml --release --lib`, Rust 1.98.1
(48a229cea 2026-09-01). Before/after test executable SHA-256 values:
`261b5793fba3b04401ccfe9356f03d21bc132c0761cbe5c83c13b0fb0f8e6c89` and
`1cc51de4d10e85c9bbfc8f23c0bad382f688079293a1805a9c62c0ff59dbf16b`.
Exact commands, source snapshots, retained binaries, regression failures, raw runs and summary
are in `/tmp/metrum-zoning-endpoint/`; replay with `python3 /tmp/metrum-zoning-endpoint/measure.py`.
Native/tool validation: `godot --headless --path godot --script res://tests/zoning_road_tool_test.gd`
(Godot 4.7.2).

Shared zoning terrain verification (2026-09-13): the previous missing-asset exemption skipped
terrain checks for densities without matching content, while low density checked the installed
houses. The new native hillside regression fails against that preceding library and passes with
the common selected-lot terrain check. All densities now agree with and without assets; the junction
bridge test also covers asset-free single placement, medium-density drag and rezoning. Portable
Rust tests cover asset-independent terrain failures, lot-size cache keys, pruning, road publication,
remote terrain reuse and local terrain invalidation. Missing content leaves flat lots zoned without
growth; only a matching density, initial level and fitting footprint permits a growth candidate.
The existing grading solver, neighbor checks and local dependency snapshots are reused. Warm
zoning queries need no asset scan or allocation and share one terrain verdict across profiles.

Three alternating matched unprofiled before/after Godot pairs use the installed low-density houses on a flat
192 m road with eighteen 20 x 20 m lots. Every density retains identical geometry; valid counts
remain 18/18/18. Median road-preview API time in µs (before → after): low
30.844 → 28.516, medium 26.203 → 28.578, high 26.188 → 28.438. CPU 0, one Rayon worker,
`METRUM_DEBUG=0`, three warmups and nine samples of 64 calls are fixed; setup is excluded.
Site queries are warm, while road geometry and Godot payloads are rebuilt per call. This is
2.3 µs faster for low density and 2.3–2.4 µs extra per 18-lot query for the previously exempt
densities, with no rendering or resident simulation included. The first low-density call solves
the eighteen lots in 0.624 ms (previously 0.674 ms); first medium/high calls reuse those solutions
and take 0.040/0.039 ms. These cold-call values are medians of one initial call per process,
not tail-latency measurements.

Three alternating release pairs also run the existing
`nodes::sim::core::tests::road_plan_scaling::populated_zoning_feasibility_scaling` test. The four
local sites remain fixed while background reaches 100,000 buildings, 100,004 total parcels,
600,024 agents and 391 remote roads. CPU affinity is `0,2,4,6,8,10,12,14`, with eight Rayon workers
and `METRUM_DEBUG=0`. Each size checks 300 warm queries; verdicts stay valid and the total local
solve count stays one. Median of run medians in µs:

| Background buildings | Before | After |
| ---: | ---: | ---: |
| 0 | 0.731 | 0.178 |
| 1,000 | 0.752 | 0.178 |
| 10,000 | 0.748 | 0.178 |
| 100,000 | 0.727 | 0.177 |

Cached feasibility remains local and is about four times faster in this comparison. The initial
solve takes 0.046 ms; revalidation after remote edits takes 0.011–0.017 ms without another solve.
Fixture setup is excluded. No competing build or test work ran during measurements. These
measurements validate the shared terrain check before the subsequent anchoring change;
the endpoint table above records still earlier builds.
Rust 1.98.1 release and Godot 4.7.2 are used over base `0b813dfed291a8b250c204b376dbea9127b76308`.
Before/after shared-library SHA-256 values are
`164c2871ce23cc3d57cdebf8b46c4f99c4ab107b69bd5426901aff7b83c8ce55` and
`4e2b12e22de94d975a49352c77d6c007394bd47a4138a6ebb71e0b5d049bef42`.
Build commands are `cargo build --offline --manifest-path rust/Cargo.toml --release` and
`cargo test --offline --manifest-path rust/Cargo.toml --release --lib`. Exact measurement commands,
test-executable identities, changed-source snapshots, retained binaries, regression failures and
raw results are in `/tmp/metrum-zoning-terrain/` (`metadata.json`, `measure.py`, `measure.gd`,
`summary.json`, and logs). Replay with `python3 /tmp/metrum-zoning-terrain/measure.py`.
The previous missing-asset policy's measurements remain historical in `/tmp/metrum-zoning-assets/`.

Shared manual/automatic anchoring verification (2026-09-13): the preceding endpoint-based fill
dropped whole candidates overlapping manual lots, leaving unused fractions at both group ends.
Rust and native Godot regressions reproduce this before the change and pass afterward. Existing
lots now determine the phase, using one interval-fill routine for road fill and manual drags.
The former separate existing-parcel drag generator is removed. No terrain or asset rules change.

Fresh matched unprofiled release measurements use three alternating process pairs, CPU 0,
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, three warmups and nine samples of 64 queries. The existing
`simulation::zoning::tests::placement::road::benchmark_road_zoning_locality` workload holds a
120 m road and twelve lots fixed while background roads and parcels increase together. Both
builds retain identical flat geometry and graded packing within 2 mm; setup is excluded.
Median of process medians, in µs:

| Background roads/parcels (each) | Flat before → after | Graded before → after |
| ---: | ---: | ---: |
| 0 | 3.562 → 4.151 | 14.884 → 15.751 |
| 1,000 | 4.486 → 5.146 | 15.903 → 17.600 |
| 10,000 | 4.601 → 5.080 | 15.895 → 17.504 |
| 100,000 | 4.827 → 5.550 | 16.124 → 18.051 |

Local anchor discovery and live-parcel spacing add 0.5–0.7 µs on flat roads and 0.9–1.9 µs on this
graded fixture; neither introduces a city-wide scan. Picking is unchanged at 0.017–0.052 µs.

Three matched Godot process pairs also measure road and drag previews on a flat 192 m road,
before and after placing the eight manual lots. Geometry and payloads rebuild per call; terrain
solutions are warm. Same CPU/worker/sample settings, with no resident simulation or rendering:

| Request | Valid new lots before → after | API median before → after (µs) |
| --- | ---: | ---: |
| Empty road fill | 18 → 18 | 28.828 → 30.891 |
| Fill around manual groups | 8 → 10 | 19.422 → 20.672 |
| Empty manual drag | 9 → 9 | 61.453 → 63.953 |
| Manual drag through existing groups | 6 → 6 | 59.875 → 13.938 |

The corrected fill computes two additional lots for the same input. Its first query after manual
placement takes 0.364 ms versus 0.041 ms previously: new anchored poses need terrain solves, whereas
the old endpoint grid reused the earlier empty-road results. The initial empty-road query takes
0.671 ms versus 0.640 ms. These are medians of one first call per process, not tail-latency estimates.
No assistant builds or tests ran alongside these timings. Rust 1.98.1 release and Godot 4.7.2 are
used over base `0b813dfed291a8b250c204b376dbea9127b76308`; before/after shared-library SHA-256 values are
`4e2b12e22de94d975a49352c77d6c007394bd47a4138a6ebb71e0b5d049bef42` and
`8ea7dc9ef42bc571460fcc7ccf4e92588098b0aea7ec25108ed32b3bb4a68fae`.
Build with `cargo test --offline --manifest-path rust/Cargo.toml --release --lib` and
`cargo build --offline --manifest-path rust/Cargo.toml --release`. Exact commands, test-binary hashes,
changed-source snapshots, retained binaries, failed/passing regressions and raw results are in
`/tmp/metrum-zoning-anchors/`. Replay with `measure-locality.py` and `measure-native.py`; results
are in `summary.json` and `native-summary.json`, with identities in the corresponding metadata files.

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

---

## 11. Road-generated cell zoning — `ZONE-04`

**Status: feature implementation complete, 2026-09-28. Generation, tools, building integration,
undo and persistence are connected and verified. The historical checkpoints below retain
their original scope.**

See [the final handoff](#feature-handoff--zone-04) and
[performance measurement scope](#performance-measurement-scope).

### Goal and confirmed requirements

Add Rust-owned road-generated zoning cells alongside the existing authored-parcel workflow.
Both workflows must work in the same city without overlapping land reservations, and both must
use the existing zoning profiles and building simulation.

- Keep `WorldConfig::zone_cell_m`: the confirmed working size is **10 m × 10 m**.
- Each eligible road generates **six rows on each side**, giving 60 m nominal zoning depth
  from its frontage boundary. Obstructions and competing roads can remove individual cells.
- Straight roads produce regular square cells. At 90° L and T junctions, adjoining rows should
  share both orientation and grid origin, so the corner contains one continuous lattice.
- Curves use short, rigid, locally aligned groups of square cells. Cells must not bend, shear,
  become partial polygons, or overlap where groups meet. Gaps are permitted around curves and
  incompatible road alignments.
- Individual-cell, marquee, fill and brush selection all work with either a selected zoning
  profile or a separate **Erase** selection.
- Fill affects the connected same-profile area on the clicked local grid, stopping at roads,
  gaps and grid changes. Unzoned cells form a fillable profile class too.
- The existing parcel tool remains available, including its dimensions, spacing, road fill,
  rezoning and free/unzoned parcel behavior.

The supplied visual references establish straight rows, shared orthogonal corners, complete
rectangular blocks, curved groups, acute intersections, competing nearby roads and mixed painted
regions as acceptance cases. They do not specify hidden algorithms or override these contracts.

### Geometry and identity

Use sparse road-generated blocks under `simulation::zoning`, with a shared grid frame containing
an origin, orthogonal basis and cell size. A block stores a bounded range of integer coordinates,
validity masks, profile ids and lot claims. Derive cell corners from the frame; do not store a
heap-allocated polygon or a full `ZoningParcel` for every cell. No dense world paint texture is
authoritative. Opposite sides separated by a road need not share grid phase.

Grid orientation alone is insufficient at a right-angle junction. Compatible adjoining road
sides must use the same frame and integer coordinates. Duplicate candidates then become one
cell with the relevant frontage provenance. A shared corner must not force a single road
attachment onto every cell or prevent a later lot from choosing a valid fronting road.

Use horizontal XZ dimensions for squares, including on sloped roads; map actual lot frontage
back to road stations for attachment and entrance generation. Zoning geometry and previews remain
terrain-neutral. The generation boundary must come from the authoritative frontage/road geometry,
including junction mouths; do not infer legal land from rendered colour or a centreline alone.
Preserve no-build restrictions and require usable ground frontage; an elevated span must not
create floating building lots merely because it projects onto ground.

Persist frame/block identity independently of mutable graph array indices. Road splits, merges,
deletions and load-time remaps update provenance through topology transactions. Unchanged cells
retain identity and paint. Block storage boundaries must not introduce visible grid seams or
stop fill within a continuous frame.

**Confirmed alignment policy:** arbitrary existing road positions and widths can impose different
grid phases, even when their tangents are perpendicular. Joining two such grids cannot guarantee
flush road frontage, unchanged painted cells and a seamless square lattice simultaneously.
Preserve painted cells and buildings, align new empty grids where possible, and retain an explicit
seam where existing frames are incompatible. This exception is intentional; do not move existing
roads, paint or buildings to hide it. Compatible orthogonal joins must still share one lattice.

**Road placement assistance:** the road options panel offers `Snap to zoning grid` for straight
roads. The checkbox is the only snapping control; the Shift shortcut is removed. Connections
to existing roads remain enabled. Spline mode disables the checkbox without forgetting its setting.

The cursor uses the nearest eligible straight road's persistent curb frame, with edge-id ties,
within two six-row zoning strips plus 30 m. It selects the curb facing the stroke and captures
a frame axis only within 5 degrees and half a cell of lateral distance. Outside either bound,
the cursor stays at the freely drawn angle. Captured strokes snap their terminating centreline
to a cell boundary offset by the selected road's half-width plus sidewalk. Thus parallel roads leave whole cells between facing
curbs: centreline separation = integer cell lengths + both curb half-widths. A branch starting
on an existing road keeps that road's centreline. Starting a first road without a nearby frame
uses the same capture bounds around cardinal directions, the configured cell length and space
for two same-width end corners. Existing junctions and road connections remain exact and take priority; this option cannot repair incompatible existing road positions.
Fixed 80 m ghost guides do not override zoning snap. The preview and click use the same resolved
cursor, and heights are sampled again after its XZ position changes.

Road alignment authority now uses the existing `imbl` persistent map. The road-tool snapshot
shares its root in O(1), paired with the same graph generation and configured cell size. Queries
use the existing edge R-tree, allocate no buffers, and require O(log E + K log A) work for E
indexed edges, K local candidates and A alignment records. Each map lookup follows a bounded
hash-trie path. Local alignment updates copy touched map paths, adding O(K log A) work; there is
no full-grid snapshot copy and no simulation mutex on the cursor path. This does not change the
existing wider road-preview snapshot costs.

The initial generator uses four-cell (40 m) horizontal arc-length groups, with the last group
using the remaining length. Each group uses its endpoint chord orientation; intermediate XZ
road points constrain the frontage support line. Height-only polyline subdivision does not move
the grid. Straight groups use one endpoint phase, with a perpendicular incident road contributing
its curb offset at a junction. The first slice reuses the parcel frontage offset
`edge.width / 2 + SIDEWALK_WIDTH`; compiled surface exclusions are supplied by the caller.
Ground-level eligible road spans generate cells; bridge and tunnel spans do not.

Frames normalize quarter-turn-equivalent direction vectors on a one-part-per-trillion component
grid, then normalize the basis. Origin phase and footprint vertices use the road overlay's
micrometre coordinate grid. Exact `i128` projection predicates distinguish interior overlap from
shared boundaries without importing the parcel SAT overlap tolerance. Perpendicular incident
directions use the larger of a 1e-6 angular bound and the f32 road-source coordinate uncertainty;
straight-line classification uses the larger of 0.1 mm lateral residual and that same source
uncertainty. These are explicit numeric geometry contracts, not camera/render thresholds.
Independent road/cell boundary normalization uses a road-contact uncertainty bound of
`4 micrometres + 2e-12 × maximum absolute world coordinate`. This covers vertex/phase rounding
and direction normalization. Tests against current road corridors and compiled pavement also
include `2 × f32::EPSILON × scale`, where scale is the larger of the tested coordinate magnitude
and bounds diagonal, to cover road subdivision/surface representation. Reciprocal road placement
and load validation use the same contact interior. Cell/cell, field and site ownership still
use full canonical footprints; every positive-area cell/cell overlap is rejected. Straight groups
keep the same endpoint origin and direction throughout. A column crossing a graph-edge endpoint
is valid only when an indexed local query proves continuous, eligible collinear road coverage;
each supplier retains its fraction of the frontage. This prevents a seam behind a T junction.
The first rotated-corner regression failed with coarser direction normalization and independently
computed group frontage; it passes after these corrections.

Straight roads now retain a selected basis and two curb phases even before any cells are painted
or visible. Road edits prepare those choices for the changed roads and one endpoint-adjacency
ring. Existing compatible choices seed new roads; entirely new local components start at their
lowest edge id. A road must fit the selected basis along its full sampled centreline. Phases share
an exact frame only when their periodic difference fits the source-coordinate uncertainty.
An unchanged straight curb, including a same-width split or reversed child, retains its prior
directed phase when no perpendicular corner constrains that side. A junction anchor across the
road cannot rephase the uninterrupted backside; same-side corners retain their alignment rules.
Incompatible existing choices remain separate, and painted/claimed cell frames remain pinned.
Ordered constraint propagation does not traverse beyond the affected ring; independent source
sampling uses Rayon. Work is O((K + A) log K + X) for affected roads K, adjacency incidences A and
tested source/point pairs X. Generation reads each recorded choice in expected O(1).

Save version 67 records these choices with an exact source fingerprint and remaps live edge ids.
Load validates the source, basis, cell pitch and curb-normal phase before accepting them; runtime
endpoint references come from the remapped graph. Signed zero is canonicalized because SQLite
REAL does not preserve its sign. Older saves initialize choices once from their network, while
retaining authoritative paint/claims. Empty generated cells still remain rebuildable caches.

The ownership rule retains historical straight strips or curved groups constrained by paint or
a lot claim, then orders candidates by depth row, canonical frame key and integer cell coordinates.
Current road candidates establish a square's priority before retained copies are merged, so
painting an unchanged grid cannot promote its strip and rearrange nearby empty cells. Shallower
rows take priority over competing deeper rows, preserving each road's frontage rather than
letting one entire lattice dominate the overlap. Incompatible grids may leave boundary gaps.
A cell loses to every directly intersecting higher-priority raw candidate, even if
that candidate itself loses elsewhere. This intentionally permits gaps rather than cascading
greedy repacking. Each conflict result needs one cell-diagonal ring, and raw candidates are queried
through a temporary local instance of the existing cell-block store. After conflict resolution,
empty cells require an uninterrupted same-frame rectangular column to complete, eligible road
frontage in one of four lattice directions, within six rows. A missing cell cuts off the cells
behind it unless another valid column supplies them. Support filtering never promotes a losing
candidate into the resulting gap. It evaluates five extra rows outside publication bounds, plus
their conflict ring, independently of chunk residency. External exclusion edits invalidate the
same bounded dependency halo. Painted/claimed cells and
cells outside the edit's publication region remain pinned; duplicate cells in one frame merge.
No whole-city candidate index or full frame-registry clone is required for a local generation.

### Non-overlap and coexistence

Cell interiors must be disjoint; shared edges and corners are allowed. Test complete footprints,
not only centres or corners, against neighboring cells, road corridors and land reservations.
Reject a conflicting whole cell rather than clipping or shrinking it. Coincident cells in a
shared frame are deduplicated, not discarded as competing cells.
Define these tests on canonical numeric geometry shared by generation, picking and rendering.
The current parcel SAT predicate permits a 1 mm overlap tolerance; do not inherit that allowance
as permission for overlapping cell interiors. Precision normalization must precede ownership
tests, with zero positive-area overlap in the authoritative representation.

For incompatible frames, apply the stable ownership rule above before emitting cells. Prefer
nearby frontage rows over distant rear rows and canonical geometry for ties. Painting a different
profile must not itself move the grid or change geometric ownership. The first geometry milestone
must prove that conflict resolution has a bounded spatial dependency: an acceptance/rejection
chain must not propagate through an entire city after one local edit. Avoid an unbounded greedy
repacking pass; a deterministic result alone is not proof of acceptable locality.

The two workflows share these reservation rules:

| State | Land reservation |
| --- | --- |
| Generated, unpainted cell | Available space only; does not block parcel or road placement. |
| Painted cell | Reserves zoning intent; a manual parcel cannot claim the same land. |
| Authored parcel, including profile `0` | Keeps its existing reservation and suppresses conflicting generated cells. |
| Building lot, service site or field | Uses the existing reservation owner; conflicting cells are unavailable. |

Placement and painting both check the other workflow within one authoritative transaction.
Erasing empty painted cells releases their reservation and leaves the valid grid visible.
Erasing a manual parcel's profile retains that parcel's existing free/unzoned reservation;
removing the parcel remains a separate operation. Occupied land stays reserved until the existing
building lifecycle releases it. Generating or painting cells must never evict existing parcels,
service buildings or fields as an incidental conflict-resolution step.

Field commit and resize invalidate cached cells over both previous and accepted footprints;
removal and undo invalidate the removed or restored field's full footprint, beyond the farm lot.
Use the existing chunk invalidation and lot-work queue. A field whose owner index changes during
an allocator swap keeps its polygon and cached cells. A later resize of that field rejects the
older demolition undo rather than restoring its obsolete shape. Rejected edits leave paint, reservations
and cache revisions unchanged. Authored-parcel attachment repair likewise checks painted cells
before mutation and refreshes cells at both the released and claimed footprints.

Demolition undo also rechecks reservations made since removal. Restored building lots must
clear current fields; lots without a parcel owner and restored field polygons must clear both
zoning workflows. Existing cell-lot inverse validation still applies to retained paint and claims.
A conflict leaves authority and the inverse journal unchanged, allowing retry after land is
released. These checks use current local indices before restoration mutates simulation state.

### Tool contract

Provide an explicit **Cells / Parcels** workflow choice. Cell mode separates **selection shape**
from **operation**: `Cell | Marquee | Fill | Brush` selects cells, while a profile or `Erase`
determines the edit. There is one Rust selection/validation/commit path for paint and erase.

Proposed gesture details:

| Tool | Selection behavior |
| --- | --- |
| Cell | Click one cell; dragging visits all cells intersected by the pointer path. |
| Marquee | Drag a ground-plane rectangle aligned to the starting cell's frame; select valid cell centres inside it, including other frames within that area. |
| Fill | Four-neighbor traversal within the clicked frame and starting profile; no diagonal-only connections and no crossing roads or gaps. |
| Brush | Adjustable circular radius in metres; select cell centres inside the swept disc along the drag path, including other frames. |

Cell and brush paths must not skip cells because input events are sparse. Rust resolves exact
selection, stable boundary ties and duplicates. Godot supplies gestures/settings and displays
packed Rust previews. Preview and commit share predicates and dependency revisions; stale input
is revalidated, and a gesture cannot silently apply to replacement cells after a topology edit.
Cancellation applies no edit. Each completed gesture is one edit transaction; retained changed-cell
deltas provide the basis for undo without copying city state. Persistent undo beyond a gesture
is not assumed to exist in the current parcel tool.

### Growth, terrain and persistence

Cell paint owns legal zoning intent. Buildings require a full rectangular footprint of valid,
unclaimed cells with the same profile and usable road frontage. Disconnected rear paint can
remain painted but cannot produce a landlocked building. Compatible assets determine usable lot
dimensions; do not force one building per cell or choose a fixed parcel size for all paint.

Reuse `ParcelStore`, `ParcelId`, profile validation, allocator placement, demand, site feasibility,
occupancy and redevelopment. Derived building lots carry an explicit cell-origin/coverage record;
manual parcels retain manual origin. Only disjoint lots enter the existing parcel-based growth
pipeline, and claiming a lot claims its cells atomically. Dirty painted areas and relevant asset
catalog changes drive lot derivation; idle simulation must not scan all generated cells.
No compatible asset means paint remains with no growth. Assets deeper than six cells can still
use suitably sized manual parcels; they do not extend the cell grid.

Reusing parcels requires extending their geometry/restore contract: a grid-aligned lot must retain
its canonical frame footprint rather than being rotated again from a sampled road tangent on
load. Road attachment remains separately authoritative for access. Partial lot repaint/erase
must invalidate affected pending spawn actions and use the existing occupied-building rezoning
grace/redevelopment lifecycle. It must neither silently erase neighboring paint nor immediately
free an occupied footprint for another building. A compatible repaint can cancel redevelopment.

For cell mode, distinguish paintable geometry from actual building feasibility. Painting does not
stamp terrain or promise construction; actual lot footprints use the existing terrain/site solver
before growth. Manual-parcel terrain validation retains its current behavior. Terrain, road and
neighboring-site changes invalidate only affected derived feasibility and lot candidates.

Save both workflows: existing manual parcels, persistent frames/provenance, painted cell data,
derived lot coverage/ids and redevelopment generations. Regenerate caches and overlay meshes on
load. Existing saves restore their manual parcels without conversion; new generated cells occupy
only remaining space. Loading must validate coverage, reservations, occupancy and pending demand
references together. Save/load must reproduce cell ownership and paint, not merely similar meshes.

### Reuse and performance gates

Start with dependencies already in `rust/Cargo.toml`: `glam` for geometry, existing `rstar` road
queries, current rectangle predicates or `parry2d` where appropriate, `i_overlay` for necessary
road-boundary polygon operations, Rayon, serde and SQLite. Existing polygon and road-geometry
adapters should be reused. `imbl` (locked to 7.0.2) additionally supplies structural sharing for
the existing road chunk-to-owner indices; their whole-index snapshot copies were measured as a
dominant edit cost. This changes storage, not spatial coverage or geometric predicates. See
[shared road owner indices](#shared-road-owner-indices). The upstream
[rstar query contract](https://docs.rs/rstar/latest/rstar/struct.RTree.html) treats bounding-box
hits as candidates, so exact footprint checks remain necessary;
[iOverlay](https://github.com/iShape-Rust/iOverlay) already supplies polygon boolean operations.
Implementation must use the repository's pinned versions rather than assume newer APIs exist.

Reuse the existing 512 m spatial chunk convention and land-reservation queries for block lookup;
do not add a parallel city spatial tree by default. A global axis-aligned `DataGrid` cannot alone
represent multiple rotated frames, but that does not justify duplicating road/building indices.
Store masks/profile data compactly and reuse scratch buffers. Generate independent blocks with
Rayon and resolve dependent ownership in stable order.

Let `K` be affected candidate cells, `H` visited chunks, `B` candidate blocks in those chunks,
`X` exact local conflict comparisons and `F` fill-selected cells. Target selection cost is
`O(H + B + K)`; fill is `O(F)` with constant-degree lattice neighbors. Local generation includes
road-query/polyline work, `O(K log K)` stable ordering and `O(X)` conflict work. `X` is not assumed
constant: measure dense intersections and document the actual worst-case bound. A large requested
fill may legitimately touch a large connected area; an unrelated distant population must not
increase a fixed local brush or road-edit cost.

Road changes invalidate the union of old/new zoning extents plus the provable conflict halo.
Preserve source dependency links so removal reveals previously suppressed neighboring cells.
Do not rebuild all frames, clone the world or repartition an entire connected orthogonal network
for a local edit. Generated overlays need visible/dirty chunk payloads; the current single parcel
overlay rebuild must not become a whole-city cell upload after each brush gesture. Repeated idle
hover should reuse geometry and buffers.

### Implementation milestones and acceptance

1. **Geometry contract:** specify deterministic generation rules under the confirmed alignment
   policy. Specify stable identities, topology remaps and the conflict-dependency bound before
   writing the generator.
2. **Generation and overlay:** straight roads, shared L/T/cross junctions, complete blocks,
   curves, acute/obtuse joins, nearby competing roads and existing reservation blockers.
3. **Editing:** all four selectors with paint and Erase, identical preview/commit selection,
   fill boundaries, cancellation, fast drags and local overlay invalidation.
4. **Simulation integration:** derived lots, demand/asset updates, occupancy, partial repaint,
   redevelopment, entrance/cache updates and road/no-build changes through existing mutators.
5. **Persistence and acceptance:** both workflows round-trip together; old parcel saves remain
   usable; matched correctness, locality and performance evidence is recorded here.

Required fixtures include all seven supplied visual arrangements, rotations away from world axes,
unequal road widths, off-phase orthogonal joins, tight curves, slopes, endpoints, world/chunk
boundaries, road split/merge/delete, terrain edits, no-build toggles and mixed manual/cell zones.
Check full-square geometry, six-row unobstructed depth, unique ownership, zero positive-area
intersection, identical preview/commit products and preservation of unaffected paint/buildings.
Replay identical saved state and edits across worker counts and save/load; compare authority,
not just screenshots. Independently constructed networks with different histories are not assumed
to have identical persisted ids or phases.

Run targeted Rust and Godot bridge regressions when implementing. Extend existing Criterion or
locality fixtures for generation, picking, marquee, fill, swept brush, derived lots and overlay
upload. Hold the edited neighborhood fixed while increasing remote roads, cells, parcels,
buildings and agents; verify matching local products and record time, allocations, memory and
uploaded bytes. Include dense local conflicts and a legitimately large fill separately. Use
matched unprofiled release runs, record build/worker/workload identities, and separate snapshot
setup from repeated editing. Numeric latency and memory budgets must be established from those
fixtures before performance acceptance; none is claimed by this design.

### Implementation checkpoint

The Rust foundation is under `simulation/zoning/cells.rs` and its `geometry`, `store`, `selection`
and `generation` children. It includes sparse 8 × 8 storage tiles, exact footprint predicates,
Cell/Marquee/Fill/Brush selection, shared paint/erase deltas, local inverse edits, direct candidate
ownership and generation from nearby roads. `ZoningSystem` now owns this store. Manual parcel
placement rejects painted cell reservations; generation/paint reject existing manual parcels,
including free/unzoned parcels. Paint revalidates reservations before any write.

The earlier foundation release passed **61 zoning tests** (five manual benchmarks
ignored), followed by the full library suite: **1,900 passed, 66 ignored**. Coverage includes
six-row straight roads, rotated and axis-aligned L/T corners, a complete 11 × 11 inner block,
curves, competing roads, pinned paint, grade-independent XZ geometry, manual/cell reservations,
identical one-/four-worker output and unchanged local work/products with 1,000 remote roads.
Commands: `cargo test --offline --release --manifest-path rust/Cargo.toml --lib simulation::zoning::`
and `cargo test --offline --release --manifest-path rust/Cargo.toml --lib`.

The isolated locality benchmark fixes one 120 m straight road and 144 cells while adding remote
roads and separately populated remote cell blocks. Three unprofiled release runs use CPU 0,
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, five generation warmups and 21 batches of 16 calls per
background size. Picking uses 4,096 calls; a 20 m swept brush uses 512 calls. Setup is excluded.
Median across the three runs (generation values are medians of batch means), in microseconds:

| Background roads | Background cells | Local generation | Point pick | Swept brush |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 600.913 | 1.465 | 8.035 |
| 1,000 | 16,384 | 602.137 | 1.468 | 8.014 |
| 10,000 | 131,072 | 602.547 | 1.444 | 8.172 |

Each case returns identical local cells and counters: one queried road, 144 raw/accepted cells,
zero competing-frame comparisons. This establishes locality for the isolated straight-road cell
subsystem, not old/new parcel performance, dense intersection cost, full road-edit planning,
allocation/memory acceptance or Godot upload cost. Those gates remain open.

Runner: `RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 <release-test-binary> --exact
simulation::zoning::cells::tests::generation::benchmark_cell_zoning_locality --ignored --nocapture
--test-threads=1`. Binary SHA-256:
`e985d49fcf15b75b59ebd85c9fe673fdceeb6168aca6856d2eaaf94d767049b4`.
`/tmp/zone04-foundation/metadata.json` records the exact command, compiler, binary and source-file
hashes; `trial-{1,2,3}.log` contains raw timings and `library-tests.log` the full suite output.
Source/binary identity remained fixed with no competing build/test work during measurements.

The next Rust slice adds canonical rectangular `CellLot` coverage to derived parcels. A lot's
frontage direction, width and depth address cells in its persistent frame; road access metadata
cannot rotate or translate that footprint. Authored parcels retain no cell coverage. Shared
painting excludes manual reservations but can rezone cells already owned by their building lot.
Parcel tools cannot independently rezone a derived lot and leave its paint inconsistent.

Partial erase/repaint removes an incompatible **empty** lot with local index updates and retains
neighboring paint. An **occupied** lot keeps all claims and becomes incompatible through profile
`0`; the allocator's existing grace countdown handles demolition. Restoring a uniform compatible
profile cancels that countdown. Releasing the building removes invalid coverage, while a still
compatible lot retains its id and advances its redevelopment generation. Gesture inverse records
restore affected lot identities/generations as well as paint. The existing building-demolition
undo journal now also captures/restores a released cell lot, including erased grace claims.
Known-id removal uses dense swap removal plus affected parcel chunks; ids remain stable.

Save **v65** adds frame, reserved-cell and lot-coverage tables. It persists all registered frame
ids, nonzero paint, cells still claimed during grace, exact rectangular coverage and the existing
parcel/building generations. Generated empty cells and render payloads remain rebuildable caches.
Loading checks serialized frame bounds, overlapping cells, profile ids, coverage, duplicate claims
and missing parcel references. Derived footprints come from frames, and existing transform
restoration consumes those footprints. Occupancy restoration permits incompatible paint for an
existing building in grace; new growth still requires compatible paint. Older save versions do
not query the new tables and keep their authored parcels without conversion.

Ownership/persistence slice verification: **1,914 passed, 67 ignored** using
`cargo test --offline --release --manifest-path rust/Cargo.toml --lib`. Added coverage includes
all four lot frontage directions, partial erase/undo, retained occupied claims, compatible repaint,
actual allocator grace expiry and occupant remapping, demolition undo, rotated mixed-workflow
SQLite round trips, malformed cell saves and a v64 save without cell tables. This slice exposed
lot installation for callers that had already validated road access and external sites; the
following slice supplies automatic selection and the runtime caller.

Let `K` be changed cells, `L` affected lots, `A` their covered cells, and `P` entries in affected
parcel chunks. Beyond indexed selection/reservation queries, the paint/lot transaction costs
`O(K + L log L + A + P)` and stores an inverse proportional to its changed cells and lots.
It performs no background parcel/cell scan. Snapshot serialization is separate whole-world work:
it visits stored blocks, sorts reserved cell addresses and serializes parcel coverage. Existing
road removal/planning paths and their full populated-map locality gate are not certified by this
transaction measurement.

Three unprofiled release trials fix one changed cell and one six-cell empty lot. Each round trip
selects, erases, releases the lot and undoes the gesture; 64 warmups precede 31 batches of 512
round trips. CPU 0, `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`; fixture construction is excluded.
Median of the three trial medians:

| Distant derived lots | Distant claimed cells | Local erase + undo (µs) |
| ---: | ---: | ---: |
| 0 | 0 | 2.302 |
| 1,024 | 6,144 | 2.327 |
| 10,000 | 60,000 | 2.321 |

Every case restores the same local paint, lot id and claims. Runner:
`RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 <release-test-binary> --exact
simulation::zoning::cells::tests::lots::benchmark_cell_lot_edit_locality --ignored --nocapture
--test-threads=1`. Binary SHA-256:
`50824901244ea5f1d23f3716e6f51daa98641932c4c832be8e63705a047c81c1`.
`/tmp/zone04-lots/metadata.json` records the command, compiler, binary and source hashes;
`trial-{1,2,3}.log` and `library-tests.log` retain the measured/tested outputs. No competing
build/test work ran during measurements. These results cover local ownership edits, not frame
generation, asset packing, live road edits, save throughput, allocation budgets or rendering.

### Automatic lot derivation and demand integration checkpoint

Generated front-row cells retain their supplying road, side and canonical frontage boundary.
Orthogonal corners can retain both road choices. These links are a transient geometry cache;
saved paint and occupied claims remain authoritative. A candidate lot requires uniform nonzero
paint, unclaimed coverage and complete frontage support from one eligible road. Rear-only paint
cannot produce a landlocked building lot. Local candidates use compatible initial-level asset
footprints from the allocator's existing profile/content/economy selector, capped at six depth
cells and the existing 80 m maximum frontage. Missing assets leave paint available for later growth.

Proposals are generated in parallel, sorted by canonical frame/frontage address, then decreasing
area/width/depth and road identity. Cell claims publish sequentially in that order. Supported
existing lots and occupied lots remain fixed. This is a specified deterministic allocation order,
not a global packing search. An asset change retires an unsupported **empty** lot and lets the
new catalog derive a replacement; it never moves an occupied footprint. Local paint, source-road
and external reservation checks are repeated before installation. Derivation checks the supplying
road's corridor too, including a regression for a 1 mm encroachment after road widening.

The runtime queues affected 512 m neighborhoods after paint, road edits/no-build/class changes,
terrain/site changes and local road undo. Load and catalog refresh schedule painted blocks.
Before hourly demand selection and committed-road entrance rebuilds, queued neighborhoods
regenerate transient frontage links, repair supported occupied attachments, retire unsupported
empty lots and derive replacements. Pending spawns revalidate their current parcel,
profile, asset and site when dequeued; an erased/retired lot cannot grow through a stale action.
The ordinary demand allocator then handles asset choice, site/terrain feasibility, construction,
occupancy and redevelopment. Asset footprints are cached until explicit catalog invalidation.
With no queued work, preparation is an allocation-free constant-time check.

The runtime reuses compiled-road query chunks, field clearance and the allocator's explicit-site
index. Compiled road polygons retain their double-precision boundaries through polygon overlap;
their former float conversion could incorrectly remove rotated frontage cells. Road contact uses
the foundation's bounded coordinate/basis rounding allowance, while cell ownership keeps exact
full-footprint intersection rules. Field and explicit-site reservations do not receive that road
contact allowance.

Fresh release library verification: **1,926 passed, 67 ignored**, command
`cargo test --offline --release --manifest-path rust/Cargo.toml --lib`, recorded in
`/tmp/zone04-derivation/library-tests.log`. New regressions cover actual building growth on straight
and rotated roads, rear paint, partial readiness, corner frontage choices, worker/asset-order
determinism, missing/replaced catalogs, occupied preservation, road widening and no-build toggles.
The later benchmark-only addition increases the ignored test count by one.

Let `C` be local cells, `S` compatible size choices, `Q` supported proposals, `A` their bounded
coverage, `L` local empty lots and `R` queried road/field/site geometry. Derivation costs
`O(C·S·A + Q log Q + L·S + R)` plus local indexed publication, with temporary storage proportional
to local proposals. Source-road polyline checks include the participating edges' segments.
Existing spatial indices keep unrelated roads/cells/parcels out of these queries. Queued regions
are sorted once; catalog refresh is explicitly whole-catalog/painted-block work. Local authoring
does not scan the pending spawn queue; dequeue-time validation uses the existing parcel lookup.
This is event work, not a per-agent or per-tick city scan.

Three matched, unprofiled release trials on CPU 0 with `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`
measure one painted 120 m rotated frontage. Each round trip derives four 30 × 20 m lots from ten
supported proposals, then retires those empty lots; setup is excluded. There are 16 warmups and
21 batches of 64 round trips. Median of the three trial medians:

| Distant roads / derived lots | Distant claimed cells | Derive + retire (µs) |
| ---: | ---: | ---: |
| 0 | 0 | 31.587 |
| 1,024 | 6,144 | 32.225 |
| 10,000 | 60,000 | 32.463 |

The same build reruns the previous generation and paint benchmarks: local generation is
603.422 / 606.440 / 607.988 µs at 0 / 1,000 / 10,000 distant roads with 0 / 16,384 / 131,072
distant cells. Local erase/undo is 2.399 / 2.413 / 2.397 µs at 0 / 1,024 / 10,000 distant lots.
Local products and counters match in every case. Runner:
`RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 <release-test-binary> --exact
simulation::zoning::cells::tests::generation::lots::benchmark_cell_lot_derivation_locality
--ignored --nocapture --test-threads=1`. Binary SHA-256:
`563eb7ea66a8be92e13cdcf1ee5dff77ba7ae199d835f8055fda13aa1a187aac`.
`/tmp/zone04-derivation/metadata.json` records commands, compiler, worker settings and source/binary
hashes; `derive-{1,2,3}.log`, `generation-{1,2,3}.log` and `paint-{1,2,3}.log` hold the results.
No competing build/test work ran during measurements. This isolates Rust geometry/lot ownership;
it does not establish end-to-end compiled-surface, populated-building/agent, memory or rendering
acceptance.

### External placement reservation checkpoint

Field previews/commits and explicit service/industry previews/commits now query the shared zoning
reservation authority. This includes painted cells before a derived lot exists, occupied claims
after erase, and authored parcels. An explicit building cannot claim an existing manual parcel;
the parcel must first be removed. Generated empty cells remain available to external placement.
Concave footprints use the existing polygon overlay implementation, preserving notches and
allowing shared edges. A constant-time reservation count also covers fully erased occupied lots.

Road readiness and commit revalidate cell reservations against both the requested corridor and
the finalized span/junction polygons. Paint added after preview invalidates that preview without
consuming its topology products; erase restores readiness when there is no remaining claim.
Non-preview insertion also prepares the final junction check when cell reservations exist.
Road-contact rounding follows the same bound as generation; external sites/fields use strict
positive-area intersection. Queries use existing cell blocks, parcel chunks and local planned
road products; no new world spatial index is introduced.

Fresh full release verification is **1,930 passed, 69 ignored** using
`cargo test --offline --release --manifest-path rust/Cargo.toml --lib`. The four added regressions
exercise a field and explicit industry site over paint without a building lot, stale road
readiness, concave-notch/shared-edge overlap, and fully erased occupied coverage. The terrain
fixture that used manual parcels as explicit-site position markers now removes each empty marker
before placement, preserving its original terrain assertions. Populated road-planning locality
is measured separately from fixture/snapshot construction using the existing scaling harness.

### Tool, undo and visible-chunk checkpoint

Godot now offers **Cells / Parcels**, **Cell / Marquee / Fill / Brush**, and an independent
**Erase** toggle. Selecting a zoning profile leaves Erase; changing the selection shape does not.
Brush radius defaults to 20 m and is adjustable from 5 to 100 m. Existing parcel dimensions,
spacing, road fill and free/unzoned parcels remain under the Parcels workflow.

Godot sends pointer paths to the packed Rust bridge and renders the returned exact selection.
Cell and brush paths retain their segments; marquee retains its start/end, and fill retains its
clicked seed. Release commits one transaction. Escape, right-click, changing workflows/settings
or releasing over UI cancels without an edit. A retained hover reuses its mesh. Authoring dependency
changes reject/cancel stale gestures; materializing another visible chunk alone does not. Commit
revalidates the retained cell identities and current reservations before writing any paint.

Generation completeness is metadata on the existing cell chunk index. Fill materializes unseen
neighbors while following the same four-neighbor/profile/frame traversal as an already generated
selection. Empty regions are cached too. Road eligibility, geometry, external sites and manual
parcel changes invalidate intersecting cached chunks, including the direct conflict halo.
Paint updates only its own overlay chunk; cached empty geometry stays usable. Cold and differently
ordered view queries have matching cell footprints around competing rotated roads.

The renderer polls visible chunk metadata in one nonblocking call and uploads at most two changed
chunks per frame. Each full cell and authored parcel belongs to its centre's 512 m chunk. Derived
lots are represented by their cells, so they do not cause a second whole-city parcel mesh upload.
Parcel insert/remove/rezone/occupancy operations maintain versions beside the existing parcel
chunk index. Idle polling is `O(visible chunks)`; payload work visits only changed chunks' cell
blocks and parcel candidates. The 6% cell inset is visual only; selection and reservations retain
the complete canonical square. Height changes refresh only chunks whose 512 m region (plus a
96 m draping margin) changed samples; see
[road-commit overlay latency](#road-commit-cell-overlay-latency--2026-09-30).
Rendering memory acceptance remains outstanding.

Completed gestures use the shared Rust undo stack with local paint and affected-lot journals.
An unchanged immediate inverse restores the original lot identities and redevelopment generations.
After intervening lot generation, growth or other authoring, undo restores paint against current
reservations and reconciles the current lots. Existing buildings and their claims survive; erased
occupied lots enter the existing redevelopment lifecycle. Consecutive undo works without rewinding
simulation time or copying a city snapshot. All reservation preconditions are checked atomically.

Fresh release verification: **1,941 passed, 69 ignored**, using
`cargo test --offline --release --manifest-path rust/Cargo.toml --lib`. New cases cover cold fill
across chunks, same-profile barriers, view-order-independent geometry, chunk-local invalidation,
one rendering owner per cell, stale previews, consecutive undo after growth, and atomic inverse
reservation checks. `cargo doc --offline --no-deps --manifest-path rust/Cargo.toml` passes.
Headless `zoning_cells_tool_test.gd` and the existing `zoning_road_tool_test.gd` both pass using
the rebuilt debug extension. The cell test includes toolbar operation separation, all selectors,
cancellation, retained previews, native mixed-workflow reservations, unchanged chunk payloads and
queued native undo. `./run.sh --test` includes the new bridge test. Evidence is under
`/tmp/zone04-tools/`, including source/binary identities and the library, bridge and rustdoc logs.

Three matched unprofiled release trials rerun the same isolated workloads on CPU 0 with
`RAYON_NUM_THREADS=1` and `METRUM_DEBUG=0`. Median of the three trial medians:

| Background roads | Background cells | Generation (µs) | Pick (µs) | Swept brush (µs) |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0 | 640.300 | 1.655 | 8.559 |
| 1,000 | 16,384 | 642.267 | 1.655 | 8.527 |
| 10,000 | 131,072 | 644.414 | 1.652 | 8.361 |

Derive/retire is 29.152 / 29.775 / 30.135 µs with 0 / 1,024 / 10,000 distant roads and lots
(0 / 6,144 / 60,000 claimed cells). The immediate erase/undo round trip is
2.603 / 2.613 / 2.612 µs at those same distant lot/cell counts. Local results match throughout.
Reservation queries use each block's reservation mask, so road checks skip generated empty cells.
Grid corner evaluation reuses one normalized basis while retaining per-vertex canonical rounding.

The populated road-planning fixture holds the edited junction, four occupied sites and one painted
cell fixed. Background cases add equally many buildings, parcels and painted cells; agents scale
from 24 to 600,024 and distant roads from 0 to 391. Each trial has 100 samples and three warmups.
Local compiled products match in every case. Values below are medians across the three trials:

| Background buildings/parcels/cells | Cell readiness p50 / p95 (ms) | Baseline readiness p50 (ms) | Cell compile / worker p50 (ms) | Cell snapshot setup (ms) |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 0.176 / 0.199 | 0.017 | 34.791 / 35.444 | 0.007 |
| 1,000 | 0.177 / 0.206 | 0.018 | 34.251 / 35.175 | 0.061 |
| 10,000 | 0.182 / 0.211 | 0.019 | 34.365 / 35.299 | 0.344 |
| 100,000 | 0.183 / 0.212 | 0.019 | 34.398 / 35.354 | 3.571 |

The matched baseline worker median is 35.304 ms at 100,000 background buildings. The cell
reservation readiness work stays local; one-time worker snapshot setup still scales with city
state and is shown separately. This does not establish full road-commit or rendering acceptance.
Regression ceilings for these pinned isolated fixtures are 1 ms generation, 3 µs pick, 15 µs
swept brush, 5 µs immediate erase/undo, 50 µs derive/retire and 0.3 ms populated readiness p95.
Cold fill, runtime gesture, overlay upload, memory and allocation budgets require their own fixtures.

Runner: `RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 <release-test-binary> --exact <test>
--ignored --nocapture --test-threads=1`. Exact test names, commands, sources and raw trial logs are
recorded in `/tmp/zone04-tools/metadata.json` and `{generation,derive,paint,cells,baseline}-{1,2,3}.log`.
Binary SHA-256: `355729e9766c5ad508fc6a34dd59675f654a58ca96cb6a7e6d1b00851bdffd2f`.
No competing build/test process ran during the measurements. The debug extension used by the
Godot checks has SHA-256 `c90ec9d2cf754eca8bfcbdfea3a6e6e7e7cffd84cfa610491a07180d44cae543`.

### Loaded reservations and boundary precision

Load now cross-checks painted cells and occupied grace claims against saved field polygons,
explicit service lots and compiled road/curb/sidewalk/junction polygons. This runs after site,
road and occupancy reconstruction, before publishing the loaded world. A conflicting save returns
an error without replacing the live city. Unpainted saved cells must belong to a saved lot; empty
generated cells remain rebuildable cache data. Legacy parcel-only saves take the constant-time
no-cell-reservations exit and retain their existing quarantine policy.

Both parcel and cell-lot restoration accept otherwise valid saved reservations when a road's
no-build flag has changed. This avoids quarantining valid geometry during load. Placement and
growth still enforce current eligibility; the existing allocator policy in section 8 still removes
facing buildings and parcels when the player enables no-build. Load acceptance does not override
that runtime policy.

Cell painting and lot validation now retain canonical double-precision corners through field
and explicit-site overlap queries. The reciprocal placement and load checks use the same polygon
contract. Narrowing a corner to single precision could previously hide a micrometre intrusion;
shared edges remain legal, while the targeted positive/negative boundary regressions distinguish
those cases. Manual parcels keep their existing overlap tolerance.

The one-time load pass parallelizes independent external owners and queries the existing cell
chunk/reservation masks. Cost follows external field/site/road polygon vertices, their visited
chunks and candidate blocks, and exact local overlap work. It does not compare every cell with
every external owner or add per-tick work. The runtime site query retains the existing allocator
index and its local candidate bound.

Fresh release verification passes 1,945 library tests, including mixed rotated saves, completely
erased occupied lots, field boundary contact/intrusion, explicit-site and road conflicts, orphan
unpainted saved cells, no-build restoration and the precise runtime boundary checks. The log is
`/tmp/zone04-load-validation/library-tests.log`. The additional ignored site-query timing fixture
was added after this full run; production code is unchanged by that fixture.

Three matched unprofiled release trials use CPU 0, `RAYON_NUM_THREADS=1` and `METRUM_DEBUG=0`,
without concurrent builds/tests. The load fixture times only reservation validation after world
hydration; fixed local paint, its occupied lot and compiled road remain unchanged. Values are
medians across trials, in microseconds:

| External fields + explicit sites | No extra painted cells | 16,384 extra cells | 131,072 extra cells |
| ---: | ---: | ---: | ---: |
| 0 + 0 | 54.589 | 54.493 | 54.636 |
| 1,000 + 1,000 | 159.751 | 160.107 | 159.181 |
| 10,000 + 10,000 | 1,053.315 | 1,037.670 | 1,041.806 |

Independent precise cell/service-site queries take 0.518 / 0.516 / 0.517 µs with
0 / 1,000 / 10,000 distant sites and the same intersecting local lot. The existing generation
fixture still produces 144 cells with identical work counters: generation is
645.308 / 648.651 / 647.753 µs, pick is 1.650 / 1.652 / 1.623 µs, and swept brush is
8.375 / 8.366 / 8.584 µs for its three background sizes. Isolated regression ceilings are
1 µs for the precise site query and 1.5 ms for this load-validation fixture with 10,000 fields
and 10,000 explicit sites. These measurements exclude full load, overlay upload and memory costs.

Commands, source hashes, environment and raw trial logs are in
`/tmp/zone04-load-validation/{metadata.json,summary.json,sites-*.log,load-*.log,generation-*.log}`.
The runner uses `taskset -c 0 <release-test-binary> --exact <test> --ignored --nocapture
--test-threads=1`; exact test paths are recorded in `metadata.json`.
Release test binary SHA-256: `0743f8121b8ee1e1b80103ef266d5368ffd04bf054e5ef628972f46986f39817`.
Fresh `zoning_cells_tool_test.gd` and `zoning_road_tool_test.gd` both pass with debug extension
SHA-256 `43015e11e1c6010adc3ae0b992e3f854cfbf65163ea8fa1dca506ea4f0dd28a6`.
Their logs and the warning-free `cargo doc --offline --no-deps --manifest-path rust/Cargo.toml`
result are in the same artifact directory.

### Straight split frontage and regeneration undo

Splitting an unchanged straight road previously preserved painted squares but could remove their
usable frontage when the second edge chose a different grid phase. Generation now recovers a
reserved frame when the current straight curb matches its integer lattice boundary. A constraint
applies only to a canonical four-column, six-row strip containing paint or a lot claim, plus the
existing direct conflict ring. Camera bounds and 512 m storage chunks do not define that strip.
Querying a neighboring cell therefore sees the same constraint even when its painted marker is
outside the requested payload. Existing incompatible frames retain their seams.

Current collinear road intervals jointly cover a frontage cell. Each supplier records its covered
fraction of that cell's boundary; a developable lot requires complete eligible coverage of every
front cell. Disabling one split edge cannot leave a partially supported cell eligible through its
other supplier. The lot's front centre selects its attachment from those suppliers. Empty lots
retain their stable ids, footprints, claims and build generations while their attachment updates.
The following checkpoint extends this to occupied buildings. Curved split/merge provenance is
still incomplete.

Erase can release a strip's alignment constraint, allowing its empty cached addresses to change.
Gesture undo validates current reservations first, restores the original frame/addresses and
removes only conflicting unreserved cache cells. Paint changes invalidate the bounded strip and
conflict halo, so later chunk requests cannot reuse stale geometric ownership.

The additional generation work is bounded by nearby reserved frames times queried local road
points, local interval sorting and emitted candidates. Interval coverage is linear in each lot's
front cells and their local suppliers. Existing cell/parcel indices remain authoritative; no
whole-city scan, new spatial index or per-agent work was added. The ordered conflict/publication
pass remains local and deterministic.

Fresh release verification passes **1,949 tests** with 72 manual timing fixtures ignored. New
regressions cover five split positions at two orientations with and without existing lots,
partial frontage after no-build, erase/regenerate/undo, and narrow/full query agreement.
The full log is `/tmp/zone04-continuity/library-tests.log`.

Three matched unprofiled release trials on an Intel Core i9-12900K use CPU 0,
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0` and no competing builds/tests. Medians across trials are
in microseconds; setup and topology mutation are outside the timed refresh:

| Operation | No distant background | Medium background | Large background |
| --- | ---: | ---: | ---: |
| Empty generation | 666.215 | 668.831 | 668.817 |
| Pick | 1.647 | 1.621 | 1.649 |
| Swept brush selection | 8.346 | 8.519 | 8.326 |
| Lot derive/retire | 32.037 | 33.013 | 33.129 |
| Immediate erase/undo | 2.884 | 2.886 | 2.893 |
| Painted split regeneration and lot refresh | 1,565.992 | 1,570.612 | 1,572.955 |

The first three rows use 1,000/10,000 distant roads and 16,384/131,072 distant cells. Lot fixtures
use 1,024/10,000 distant lots and 6,144/60,000 distant painted cells, with matching distant roads
for derivation and split refresh. The split fixture retains 72 painted cells and four lot ids;
all sizes produce the same two queried roads, 204 unique candidates, 640 comparisons and 138
accepted cells. Its isolated regression ceiling is 2 ms; the preceding single-road fixture
ceilings remain unchanged. These measurements do not time a full road transaction, entrance
repair, populated simulation snapshot, overlay upload or rendering.

Exact commands, source hashes and raw trials are in
`/tmp/zone04-continuity/{metadata.json,summary.json,generation-*.log,derive-*.log,paint-*.log,split-*.log}`.
The runner uses `taskset -c 0 <release-test-binary> --exact <test> --ignored --nocapture
--test-threads=1`; test paths are recorded in `metadata.json`. Release test binary SHA-256:
`efbc40de4d0398d845a5b15ea7942274de16db1add7a1143a5dceccab8d19617`.

Both Godot zoning checks pass with deployed debug extension SHA-256
`f3cae9f5e956fc06d6918fac25ba9d8593b40f2e338745bda51d98846871d45b`.
Commands use `XDG_DATA_HOME=/tmp/zone04-godot-data XDG_CONFIG_HOME=/tmp/zone04-godot-config
godot --headless --path godot --script res://tests/<test>.gd`, with tests
`zoning_cells_tool_test` and `zoning_road_tool_test`. Logs and the warning-free
`cargo doc --offline --no-deps --manifest-path rust/Cargo.toml` result are in the same artifact
directory. An initial debug build exhausted disk space; clearing generated incremental cache
allowed a successful rebuild before these checks.

### Occupied attachments through straight-road edits

Local lot refresh now visits occupied cell lots as well as empty lots. Complete current frontage
coverage repairs the occupied lot's road reference without moving its canonical footprint,
changing its id/build generation, or releasing its claims. The changed parcel ids drive indexed
occupant updates before entrance-cache rebuilds. Cell lots no longer use the manual-parcel
nearest-road fallback or require their entire frontage to fit on one edge.

Dirty-region draining includes retained claims when all paint has been erased. Preparation runs
before committed-road entrance rebuilding and defers while a road transaction is staged. The
same path handles an accepted road undo, so restoring the road also restores valid lot, building
and entrance references. No new undo snapshot or whole-city zoning scan was added.

A disabled road can supply attachment metadata for existing lot claims until allocator policy
processes them. It cannot generate new empty cells, and a partial eligible interval cannot supply
a complete empty frontage cell. New construction still requires full eligible coverage. The
existing no-build removal policy in section 8 is unchanged.

Fresh release verification passes **1,952 tests**, with 72 manual timing fixtures ignored.
The added regressions cover occupied split/restored-road metadata at two orientations, erased
claims, disabled frontage, and actual road commit/undo with a grown building and entrance cache.
The runtime regression preserves exact building position/facing, parcel footprint and ownership,
and verifies current road stations and entrance edge ids. Log:
`/tmp/zone04-occupied/library-tests.log`.

Three matched unprofiled trials use the preceding CPU/worker settings and no competing builds.
The occupied fixture alternates immutable split and restored road graphs and changes all four
local occupied attachments on every measured refresh. Graph cloning and background setup remain
outside timing. Values are medians across trials, in microseconds:

| Operation | No distant lots | 1,024 distant lots | 10,000 distant lots |
| --- | ---: | ---: | ---: |
| Occupied split/restore refresh, average per operation | 1,263.807 | 1,270.129 | 1,270.815 |
| Empty-lot split refresh | 1,571.145 | 1,581.242 | 1,582.072 |
| Lot derive/retire | 30.419 | 30.811 | 31.364 |
| Immediate erase/undo | 2.815 | 2.816 | 2.832 |

Backgrounds include 6,144/60,000 painted cells and matching roads; occupied-mode background lots
also carry occupant claims. Both topology variants retain identical local products at all sizes.
Single-road generation remains 659–664 µs, pick 1.649–1.669 µs and brush selection 8.567–8.642 µs
in the existing 0/1,000/10,000-road control. Existing isolated regression ceilings still pass;
the occupied alternating fixture uses the same 2 ms refresh ceiling. These are zoning refresh
measurements, excluding full road transactions, building entrance-cache rebuilding and rendering.
Occupant metadata publication is O(changed lot ids), with no global building lookup or spatial
index invalidation; the existing entrance rebuild remains a separate cost.

Commands, source hashes, raw trials and medians are in `/tmp/zone04-occupied/metadata.json`,
`summary.json` and `{generation,derive,paint,split}-{1,2,3}.log`. Release test binary SHA-256:
`c8660a468cfb9b9f67c4123aaadcff3957bae7d4218b612c03c61b6b36d49cc9`.
Both Godot zoning tests pass using the preceding headless commands and deployed debug extension
SHA-256 `50f791f59ad46dad2e7118a28fb75def14240904812d96a5375bb5ef26e66c62`.
Their logs and the warning-free Rustdoc result are in the same artifact directory.

### Curved frontage through subdivision, reversal and reload

Curved groups now retain their original bounded centreline guide when painted or claimed by a
lot. The guide describes one four-column/six-row strip and contains no mutable road ids.
Generation queries these guides through the existing cell chunk index and verifies complete
coverage of each original segment by current local road segments of the same width. Subdivision,
reversed edge direction and restoration of the original curve can therefore recover current
frontage suppliers without moving the grid or its lots. A geometrically changed curve cannot
borrow the old guide simply by being nearby. No-build suppliers retain only existing claim
metadata; new construction still requires complete eligible frontage.

Generation and guide validation count a non-terminal curved group's columns with the same rule,
`CellCurveSource::open_columns`, measured from the lattice-rounded strip origin. Lattice phases
are stored in micrometres, so the road start can lie up to about 0.5 µm from that origin.
Generation previously measured column fronts from the start instead. A chord ending a fraction of
a micrometre past a column boundary then produced a sliver fifth column that validation rejected.
That debug assertion panicked zoning chunk preparation and poisoned the simulation core. Fixed
2026-09-29 with regression `curved_group_ignores_submicrometre_sliver_columns`, which uses the
recorded road. The same bug caused the two long-standing zoning test failures (partial paint/erase
chunk rebuild parity and node-merge regeneration).

Each group shares one reference-counted guide across its candidates. Contiguous intervals from
one supplier are merged before publishing cells. Retained groups use the existing candidate
priority, road-surface exclusion and exact cell conflict checks. The guide identity check adds
the explicit `2 × f32::EPSILON × coordinate magnitude` interpolation rounding bound, needed
when the road graph inserts subdivision points at large coordinates. This allowance applies to
guide matching and frontage intervals; cell/cell and field/site overlap predicates are unchanged.
The generator clips guide points to the group's horizontal station interval before computing
curvature support, so samples outside that interval cannot move its frontage line.

Guide lookup visits local chunks and sorts/deduplicates their group keys. Matching costs
O(sum of local guide segments × queried road segments + matched-interval sorting), followed by
the existing local candidate conflict/publication work. Independent groups use Rayon. Each
reservation check visits at most 24 cells. No city-wide source lookup or new spatial index was
added. Empty guides are pruned when their local cells are evicted, and a group key replaces its
previous guide instead of accumulating obsolete geometry versions. Erase inverses retain only
guides touched by the gesture, allowing undo after empty-cell regeneration.

Save **v66** adds `zoning_cell_curve_sources`. It stores guides only for painted or claimed groups,
validates their frame, rectangle, width, side and complete geometric relationship on load, and
rejects duplicates. Guide coordinates and width serialize as integer IEEE-754 bit patterns,
preserving exact geometry without changing JSON parsing for unrelated systems. V65 cell saves
and older manual-parcel saves continue to load. V65 contains
no historical curve guides; an incompatible old split cannot recover provenance that was never
saved. Existing painted addresses and occupied footprints remain authoritative.

Fresh release verification passes **1,961 tests**, with 73 timing fixtures ignored:
`cargo test --offline --release --manifest-path rust/Cargo.toml --lib`.
The regressions cover empty/occupied lot retention, exact source serialization with fractional
widths and all orientation quadrants, large coordinates,
subdivision, restoration, reversed attachment sides, erase/cache/undo, changed-curve rejection,
SQLite round trips against compiled road surfaces, malformed guides and v65 compatibility.
Rustdoc also completes without warnings. Logs are in `/tmp/zone04-curves/`.

Three matched unprofiled trials use CPU 0, one Rayon worker and `METRUM_DEBUG=0`, without
competing builds or tests. The curve fixture alternates split and restored graphs, refreshing
18 occupied local lots and 232 cells. Distant setup and graph cloning are excluded. Medians
across trials:

| Background curves | Total occupied lots | Total reserved cells | Retained guides | Curve refresh |
| ---: | ---: | ---: | ---: | ---: |
| 0 | 18 | 232 | 12 | 5.789 ms |
| 128 | 658 | 8,168 | 524 | 5.806 ms |
| 1,024 | 5,138 | 63,720 | 4,108 | 5.811 ms |

Local canonical cells and generation work counters match at every background size. This is a
curve-refresh baseline and locality result, excluding complete road transactions, entrance-cache
rebuilds, allocator building instances, agents and rendering. Full gameplay latency and memory
acceptance remain open. The earlier straight/control fixtures still satisfy their recorded
ceilings: generation 0.663–0.667 ms, pick 1.014–1.018 µs, brush 5.093–5.290 µs, lot derive/retire
30.955–32.031 µs, erase/undo 2.251–2.274 µs, empty split refresh 1.565–1.574 ms and occupied
split/restore 1.260–1.267 ms. These controls include up to 10,000 distant roads/lots as before.

Commands, worker settings, source hashes, raw trials and medians are retained in
`/tmp/zone04-curves/{measure.py,metadata.json,summary.json}` and
`{curve,generation,derive,paint,split}-{1,2,3}.log`. Release test binary SHA-256:
`3dcfb4d0ad81890d2bdb755d46a76aa188130693c164df4dd1feb105bc341ee2`.
The metadata separately records the final Rustdoc-only source annotation. Both Godot zoning
scripts pass with the preceding headless commands against the deployed release extension,
SHA-256 `d4f8c4ecd1681fb2c5724d58b43c627a817ea2acf0fb0318f16c0ff6abc08dd1`.
Logs: `/tmp/zone04-curves/godot-cells.log` and `godot-parcels.log`.

### Native road transactions and rendered references

The native curved-road transaction regression creates an actual asset-backed residential building,
adds a branch that splits its road, then undoes that transaction. Cardinal and rotated fixtures
cover both painted land and erased occupied claims. Paint, lot footprint, occupancy and building
pose remain unchanged; lot/building frontage and entrance attachments follow the current edge.

`godot/tests/zoning_reference_test.gd` now reproduces all seven supplied arrangements through
native preview/commit and the production road/cell renderers. It checks square dimensions,
unique ownership, exported-cell non-overlap, six unobstructed rows, shared T/block frames,
complete chunk uploads and mixed paint colours. The T fixture excludes the two squares actually
occupied by the compiled junction sidewalk. Rust retains the exact integer cell-overlap check;
the independent exported-f32 check permits only its documented 0.1 mm representation bound.
The headless fixture is included in `run.sh --test`; image capture requires a real renderer.

Those fixtures exposed and corrected three integration defects in addition to the road-contact
precision contract above:

- Straight-road endpoint columns now accept complete coverage from adjacent eligible collinear
  segments, retaining fractional suppliers. A T split no longer cuts an empty row out of the
  uninterrupted roadside. The extra endpoint query is local: O(log R + P + k log k), for local
  polyline points P and matching intervals k.
- Endpoint projection keeps double precision until its final coordinate, including full-segment
  refinement. Reconstructing a point from a rounded f32 parameter had displaced an orthogonal
  junction and split a block into near-identical competing frames. This remains O(1).
- Terrain-loop clipping retains the exact tile-side intersection when an adjacent interior
  point has the same quantized identity. Keeping the interior point created a sliver triangle
  that rejected an otherwise flat block road. Deduplication remains O(n); see
  [the terrain contract](terrain.md) and [road projection](roads.md).

The reference build passes **1,967 release library tests** (73 timing fixtures ignored), the
`zoning_cells_tool_test`, `zoning_road_tool_test` and `road_junction_preview_test` headless scripts,
and warning-free Rustdoc. All seven real-renderer fixtures pass and their PNGs were inspected:
curves, straight road, T, block, angled joins, competing roads and mixed paint. Capture used
Godot **4.7.2**, Forward+ Vulkan, Wayland and an AMD Radeon RX 7900 XTX, with the production
clipped terrain mesh using a flat green inspection material. This checks zoning geometry and
overlay appearance; it is not terrain-art or frame-time acceptance.

Artifacts are `/tmp/zone04-references/{library-tests.log,rustdoc.log,rendered.log}`,
the three named Godot test logs and `rendered/{manifest.json,01_*.png,...,07_*.png}`.
The reference build's release extension SHA-256 is
`c45de65fdad2409be82f1de606397ae6e33c61db44b258721efc265289f23c64`.
Capture command, from the repository root:

```bash
METRUM_ZONE_REFERENCE_DIR=/tmp/zone04-references/rendered \
XDG_DATA_HOME=/tmp/zone04-godot-data XDG_CONFIG_HOME=/tmp/zone04-godot-config \
godot --display-driver wayland --rendering-method forward_plus --audio-driver Dummy \
  --resolution 64x64 --path godot --script res://tests/zoning_reference_test.gd
```

Three matched unprofiled release trials compare the reference build with the retained binary
before endpoint-column, road-contact precision and endpoint-projection changes. Both binaries
already include the clipping fix, so this comparison does not measure that fix. Trials alternate
before/after, use CPU 0, `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, and run without competing builds
or tests on an i9-12900K. Medians of the three trials:

| Operation | Distant background | Before | Reference build |
| --- | ---: | ---: | ---: |
| Generate one road | 0 / 10,000 roads | 0.700 / 0.682 ms | 0.672 / 0.683 ms |
| Empty split refresh | 0 / 10,000 lots | 1.618 / 1.611 ms | 1.745 / 1.730 ms |
| Occupied split/restore | 0 / 10,000 lots | 1.290 / 1.299 ms | 1.346 / 1.367 ms |
| Curved split/restore | 0 / 1,024 curves | 5.913 / 5.939 ms | 5.857 / 5.854 ms |
| Native planning worker, p50 | 0 / 100,000 buildings | 36.137 / 36.179 ms | 36.714 / 38.309 ms |
| Native readiness, p50 | 0 / 100,000 buildings | 0.179 / 0.184 ms | 0.177 / 0.187 ms |
| One snapshot per edit | 0 / 100,000 buildings | 0.008 / 3.524 ms | 0.009 / 3.900 ms |

Local generation counters/cells and native planning products match at every background size.
Straight split refresh adds about 4–8% local cost for endpoint coverage and remains below its
existing 2 ms ceiling. Picking stays 1.057–1.073 µs and swept brush 5.178–5.319 µs. Curved fixtures
retain 18 local occupied claims while background totals reach 5,138 claims and 63,720 cells.
Native planning grows to 600,024 agents, 100,004 parcels, 391 remote roads and 100,000 remote
painted cells. Snapshot cost is reported separately from repeated local planning. Worker p95
at the largest background rises from 36.941 to 42.768 ms; trial p50 ranges overlap substantially,
so no planning speedup or unchanged tail latency is claimed. These results establish local
products and bounded observed cost, not final frame-time or allocation acceptance.

Commands, compiler/source hashes and raw results are retained under `/tmp/zone04-references/` in
`measure.py`, `metadata.json`, `summary.json` and
`{generation,split,curve,road-plan}-{before,after}-{1,2,3}.log`. Release test binary SHA-256:
before `cf800034c8e5d2a68ab8e3cb5b4c62527f2ae17413e04afb564f1da39105590e`,
reference `129dc169e6013248a1dc60f2ce0012b9d247cb78e82a4c57e1153d6f1c8355be`.

Subsequent native SQLite/dequeue verification passes for queued cell growth at angles 0 and
0.35 radians. An unchanged lot keeps its identity through load and asset-work preparation and
receives its scheduled building. Erasing part of its paint or disabling its road before saving
leaves the queued action harmless when dequeued after load. No replacement lot receives the old
action. Paint, due time, occupancy and cell claims are checked. Command:
`cargo test --offline --release --manifest-path rust/Cargo.toml --lib
queued_cell_growth_reloads_and_revalidates_erased_or_disabled_lots -- --nocapture`;
log `/tmp/zone04-references/queued-load.log`. This adds tests only; it does not change the measured
runtime or deployed extension.

The subsequent rotated native block regression exposed 29 missing interior cells and seven
near-identical bases/phases. The historical failure is retained in
`/tmp/zone04-references/rotated-block.log`. Independent f32 endpoint rounding had become independent
grid authority. The road-owned choices described above resolve this without coarsening cell
overlap predicates or depending on resident render chunks.

### Persistent straight-road alignment checkpoint

The corrected native block passes at angles 0, 0.35, -0.55, 0.8 and 2.2 radians. Each fixture
checks all 300 interior sample cells, saves before materializing empty cells, reloads with compacted
edge ids, changes chunk warm-up order, and compares canonical cell products and saved source
choices. Sampled perpendicular roads at (-5000, 2500) also share a complete corner in all four
rotations. Save tests reject malformed, mismatched and displaced source records and exercise
version 66 initialization. The queued-growth and curved-source round-trip regressions pass too.
Direct non-bulk road insertion now records choices from its existing affected-edge set, and
node movement invalidates both old and new corridors. Rejected staged edits and node-move undo
restore the original recorded choices. Selection matches with one and four Rayon workers.
Marking a road end as a border connection, which moves that end `BORDER_EXTENSION_M` outward,
refreshes the road's alignment the same way (`SAVE-02`); before that fix the save wrote the stale
source and its load was rejected.

The release library suite passes **1,971 tests** (73 timing fixtures ignored). All three Godot
bridge suites pass with the rebuilt extension, and warning-free Rustdoc passes. The real Forward+
Vulkan/Wayland renderer reproduces all seven original fixtures plus a rotated block; all eight
pass and all PNGs were inspected. Cardinal and rotated blocks each export 1,055 cells, with all
300 interior samples on a shared grid. Every fixture uploads exactly its exported cells through
the production chunk overlay. These captures establish geometry and appearance, not frame-time
acceptance.

Artifacts: `/tmp/zone04-alignment/{library-tests-external.log,rustdoc.log,build-final.log,rendered.log}`, the
three named bridge-test logs, and `rendered/{manifest.json,01_*.png,...,08_*.png}`. The capture
command above is unchanged except for `METRUM_ZONE_REFERENCE_DIR=/tmp/zone04-alignment/rendered`.
Release extension SHA-256:
`2e5dc72a03ee8e28be83c6359379bbc1725dcbc95e3606a519df283d0210e180`.
The final library log also contains the direct-insertion, node-move and rollback assertions;
earlier logs in that directory retain the intermediate failures and their fixes.

Three further matched unprofiled release trials use the retained baseline above and the final
alignment binary, alternating before/after on CPU 0 with one Rayon worker and `METRUM_DEBUG=0`.
No builds, tests or GPU captures compete with these timings. This compares the combined endpoint,
contact and alignment changes, not alignment alone. The updated split fixture also prepares
local road choices during each measured refresh and retains distant road choices; the baseline
fixture predates that work. Medians of the three trials:

| Operation | Distant background | Retained baseline | Alignment build |
| --- | ---: | ---: | ---: |
| Generate one road | 0 / 10,000 roads | 0.665 / 0.667 ms | 0.663 / 0.666 ms |
| Point pick | 0 / 131,072 cells | 1.038 / 1.035 µs | 1.651 / 1.649 µs |
| Swept brush | 0 / 131,072 cells | 5.131 / 5.082 µs | 8.396 / 8.388 µs |
| Empty split refresh | 0 / 10,000 lots | 1.568 / 1.572 ms | 1.694 / 1.703 ms |
| Occupied split/restore | 0 / 10,000 lots | 1.259 / 1.265 ms | 1.680 / 1.685 ms |
| Curved split/restore | 0 / 1,024 curves | 5.774 / 5.795 ms | 5.726 / 5.753 ms |
| Native planning worker, p50 | 0 / 100,000 buildings | 35.194 / 35.205 ms | 35.662 / 35.644 ms |
| Native readiness, p95 | 0 / 100,000 buildings | 0.201 / 0.215 ms | 0.196 / 0.212 ms |
| One snapshot per edit | 0 / 100,000 buildings | 0.009 / 3.439 ms | 0.008 / 3.425 ms |

All current-build local products match across background sizes. The split fixture preserves
the four original lots and paint; its candidate counts differ from the baseline because it
initializes recorded authority after the original split, alongside already pinned paint.
Occupied refresh adds about 33% in this comparison, while pick and brush add roughly 60–65%.
These costs remain within the recorded 2 ms split, 3 µs pick and 15 µs brush ceilings; generation
and readiness also meet their existing ceilings. No isolated cause or speedup is claimed.
The planning fixture still reaches 600,024 agents, 100,004 parcels and 100,000 remote painted
cells. Its current worker p95 at the largest background is 36.608 ms; snapshots remain separate
whole-world setup costs. These are local subsystem/planning measurements, not complete road
commit or rendering acceptance.

Commands, environment, source hashes and summaries are in
`/tmp/zone04-alignment/{measure.py,metadata.json,summary.json}` with
`{generation,split,curve,road-plan}-{before,after}-{1,2,3}.log`. The retained current test binary
is `/tmp/zone04-alignment/after-tests`, SHA-256
`b81ff4b5e99f81ff6ddde21373077885843c7b9d7b0d5947cd84aab7ccb9e1a0`.

A separate glibc `memusage --no-timer` run of the split fixture reports heap peaks of
24,484,721 bytes before and 33,690,239 bytes after, with 823,069 versus 964,831 total
malloc/calloc/realloc calls and no failed allocations. These totals include every background
fixture, graph/index setup, initial source preparation and all repeated operations; they do not
establish per-edit allocations or idle-tick behavior. Profiled timings are excluded from the
table. Commands and scope are in `/tmp/zone04-alignment/{memory.py,memory-metadata.json}` and
`memory-{before,after}.log`. An initial `--mmap` trace produced an invalid peak counter; its
`memory-mmap-*` artifacts are discarded diagnostics, not memory evidence. Operation-isolated
allocation and broader memory acceptance remain open.

### Occupied cell lots during road removal

Road removal previously detached occupied cell lots before their buildings were removed. It now
keeps those claims, including erased occupied cells, until the allocator clears the occupant.
The road-removal inverse captures the same subset of detached records. Occupancy release queues
only the lot's bounds for frontage revalidation, so intact paint cannot leave an empty lot
attached to a deleted road indefinitely. Existing manual-parcel removal behavior is unchanged.
The added removal predicate costs O(1) per visited parcel; it does not change the existing
parcel-removal scan's complexity. This is not full bulldoze locality acceptance.

The native deletion regression covers cardinal and rotated curves, intact and erased paint,
immediate road undo, then a second deletion followed by building maintenance. Pose, lot identity,
occupied coverage and paint survive while the building exists; claims are released after its
removal. The full release suite passes **1,972 tests** and all three bridge suites pass with the
rebuilt extension. Logs: `/tmp/zone04-alignment/library-tests-lifecycle.log`, `build-lifecycle.log`
and `lifecycle-*_test.log` (the initial failure is `curved-road-delete.log`). Extension SHA-256:
`1ead3f393a4b6f6d8d1cb75defc60f60115c3a9669e19bc669e4a69f41b7233a`.
The preceding timing and image evidence applies to its recorded alignment build.

Three unprofiled one-worker trials on CPU 0 put the six-cell occupy/clear round trip at
0.458 / 0.461 / 0.461 µs with 0 / 1,024 / 10,000 distant lots. Every case retains the same local
lot and queues exactly one local dirty region. Setup, warm-up and region draining are excluded;
this is an after-only locality measurement, not a before/after speedup. Matched erase/undo stays
within its 5 µs ceiling: 2.836 / 2.861 / 2.855 µs on the preceding alignment binary versus
2.774 / 2.797 / 2.794 µs after the retention fix. No concurrent build/test load ran during timing.
The new ignored benchmark is `simulation::zoning::cells::tests::lots::benchmark_cell_occupancy_release_locality`.
Commands, source hashes, worker settings and medians are in
`/tmp/zone04-alignment/{lifecycle-metadata.json,lifecycle-summary.json}`, with raw logs
`lifecycle-{edit-before,edit-after,release-after}-{1,2,3}.log`. The retained lifecycle test binary
is `lifecycle-tests`, SHA-256
`711e5069d9de91a651ee70988522f18ed1ba18a18a8cd40d6ef8d5b610f53509`.

### Road-attached parcel removal and inverse locality

`ParcelStore` now keeps a road-to-parcel membership index and one inverse membership slot per
dense record. Road removal cannot rely only on a spatial search: an occupied parcel's preserved
footprint can be far from its road after a geometry change. The relationship index follows
insertion, load, geometry/attachment replacement, local removal, restoration and bulk cleanup.
It reuses the existing chunk index for spatial work and adds no dependency or spatial tree.

Capture visits the road's attachments and orders removed records by their original storage
indices. Descending swap removals preserve the validity of the remaining journal indices;
ascending append/swap restoration reverses them exactly. Chunk pick order is repaired only for
the removed or moved records. This also fixes an existing local-removal defect where the chunk
list retained the moved parcel's old precedence, violating subsequent ordered insertion.
Occupied cell lots retain the lifecycle policy documented above.

For A attached records, K removed records and Q entries visited/shifted in the affected chunk
lists, capture/removal/undo costs O(A + K log K + Q), with O(K) temporary journal/selection space.
Q includes the footprints of at most K records swapped from the dense tail, which can be distant;
the algorithm does not walk other city records or rebuild their indices. Attachment membership
updates are expected amortized O(1). Persistent index space is O(P + E) for P parcels and E
occupied attachment buckets. Independent attachment selection uses Rayon; ordered mutations and
inverse replay are sequential. The separate raw-id bulk cleanup still uses its existing full
rebuild, and this checkpoint does not certify the entire bulldoze/network pipeline.

The regressions exercise all 256 subsets of eight records with non-storage-order ids, exact dense
order and occupancy restoration, coincident-footprint pick precedence, distant and same-chunk
attachment changes, local detach/restore, bulk cleanup and clear/reinsert. The baseline fails the
local-removal chunk-order assertion; the corrected release passes **1,974 library tests** with
75 timing fixtures ignored. All three headless Godot bridge suites and warning-free Rustdoc pass.
Artifacts: `/tmp/zone04-removal/{baseline-correctness.log,local-tests.log,library-tests.log,build.log,
rustdoc.log}` and the three named bridge-test logs. Extension SHA-256:
`b0b63556065eb427b1e0d742bfb885ca03bbc17be076f27011c621493cc95cc8`.

Three matched unprofiled release trials alternate the retained baseline and corrected binary on
CPU 0 with `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, and no concurrent build/test/capture. The new
fixture holds six local manual parcels fixed and adds 0 / 1,024 / 10,000 / 100,000 distant parcels.
Four warmups precede 15 batches of eight capture/remove/validate/restore round trips; background
setup and full index/product verification are outside the timing. Every round trip restores the
same parcel ids, dense order and query results. Medians across the three trials:

| Background parcels | Before capture/removal/undo | Indexed capture/removal/undo |
| ---: | ---: | ---: |
| 0 | 0.695 µs | 1.270 µs |
| 1,024 | 90.761 µs | 4.763 µs |
| 10,000 | 939.366 µs | 6.515 µs |
| 100,000 | 10,555.494 µs | 5.541 µs |

The empty-world case adds 0.575 µs; populated cost now follows the affected and swapped records'
chunk density rather than total city parcels. These fixtures establish a 10 µs regression budget
for the six-parcel round trip; they do not bound arbitrarily dense local chunks or an entire road
deletion. Matched erase/undo remains within its existing 5 µs ceiling (2.884–2.905 µs before,
2.866–2.872 µs after). Empty split/restore improves from 1.764–1.795 ms to 1.263–1.273 ms;
occupied split/restore improves from 1.715–1.724 ms to 1.230–1.251 ms. Both retain identical local
products across background sizes and pass the existing 2 ms ceiling.

The populated native planning fixture still produces identical local products through 600,024
agents, 100,004 parcels and 100,000 remote painted cells. Its first three trials give worker p50
35.954 / 37.664 ms before versus 37.686 / 37.647 ms after at 0 / 100,000 background buildings.
One snapshot per edit is separately 0.008 / 3.408 ms before versus 0.008 / 3.600 ms after.
Readiness p95 medians at 10,000 and 100,000 backgrounds are 0.337 and 0.349 ms on the current
binary, exceeding the existing 0.3 ms ceiling; the corresponding baseline medians are 0.231 and
0.267 ms. This is not a blanket planning-performance pass. The third current trial falls back
below the ceiling. Two further matched control pairs retain every original result and reverse
the fourth pair's execution order. The current-build largest-background p95 is 0.252 ms in the
fourth trial and 0.303 ms in the fifth. Across all five trials its median is **0.303 ms**, versus
0.247 ms before; the 0.300 ms readiness ceiling therefore remains unmet. The baseline also has
one 0.340 ms trial, so no specific causal explanation is established by these timings. Five-trial
worker p50 at the largest background is 35.854 ms before and 35.995 ms after, with matching local
products throughout. Further investigation must explain the tail rather than repeat until a pass.
The control commands/order and all five-trial values are in
`/tmp/zone04-removal/{recheck.py,recheck-metadata.json,summarize-controls.py,controls-summary.json}`
and `road-plan-{before,after}-{4,5}.log`.

Commands, compiler/CPU/worker settings, source hashes and original medians are in
`/tmp/zone04-removal/{measure.py,metadata.json,before-source.json,summarize.py,summary.json}`;
raw logs are `{removal,edit,split,road-plan}-{before,after}-{1,2,3}.log`. The exact same measured
fixtures run on both binaries. Retained baseline test binary SHA-256:
`1bbd60425b70ba8ebfd99b6932b5f56d24b0121dcc3b7fa74e2f940aadeb43a7`;
corrected test binary SHA-256:
`d5effb163f906e22575d0c5f11c20da2c146a36e08f0ae1e9dd55803e83ad31e`.

### Readiness reservation query work

Cell-block visits now prepare their frame basis and metric phase once, while retaining the same
per-vertex canonical arithmetic. Reservation callers reuse the corners already computed for the
query envelope. A road contour is prepared for exact overlap only when the existing local block
index finds a reserved-cell candidate, and that candidate's four corners are borrowed rather
than copied into another allocated `PolygonFootprint`. The existing precise even-odd polygon
operation and road-contact interior rule are unchanged; concave shapes still require the exact
operation. There is no new persistent cache, spatial index, save version or dependency.

Query complexity remains O(H + B + K + X): visited chunks H, their candidate blocks B, visited
mask bits K (at most 64 per block), and exact polygon work X. Frame preparation is O(1) per block.
Query bounds take O(V) for a V-vertex road contour; preparing its owned contour is deferred until
needed. The temporary frame transform and four-corner footprint stay on the stack. This removes
wrapper allocations, not the geometry library's own allocations for actual overlap work, and
does not establish the broader allocation/memory gate.

A separate `perf record -F 997 -e cycles:u --call-graph dwarf,4096` run of the populated planning
fixture identified repeated corner and polygon preparation in its whole-workload leaf samples.
Call-chain decoding was incomplete, so it does not explain individual readiness tail events.
The retained `--no-inline` report guided the source investigation; profiled timings are excluded
from acceptance. Artifacts are `/tmp/zone04-readiness/{before.perf.data,before-report.txt,
profile-before.log}`. The command and decoding limitation are recorded in `metadata.json` there.

The full release suite passes **1,976 tests** (75 timing fixtures ignored), including new checks
for borrowed/owned overlap agreement at rotated and distant concave boundaries, exact contact,
empty/distant road polygons, and unique reserved footprints across blocks and chunks. All three
headless Godot bridge suites and warning-free Rustdoc pass. Logs are under
`/tmp/zone04-readiness/{cell-tests.log,library-tests.log,build.log,rustdoc.log}` and the three named
bridge-test logs. Extension SHA-256:
`644b0067e408b152a78388b1301567ed7a8946521c321f061c61fbda7be2a513`.

Three matched unprofiled release pairs use the preceding removal binary and this query build,
CPU 0, one Rayon worker and `METRUM_DEBUG=0`. The second pair reverses execution order; no
build/test/capture runs concurrently. Every benchmark fixture's source is identical between
binaries, and generation, curved refresh and populated planning preserve their local products.
Medians across the three trials, at the smallest and largest background:

| Operation | Distant background | Removal build | Query build |
| --- | ---: | ---: | ---: |
| Generate one road | 0 / 10,000 roads | 0.463 / 0.466 ms | 0.480 / 0.482 ms |
| Point pick | 0 / 131,072 cells | 1.650 / 1.653 µs | 1.034 / 1.006 µs |
| Swept brush | 0 / 131,072 cells | 8.347 / 8.531 µs | 5.297 / 5.460 µs |
| Erase/undo | 0 / 10,000 lots | 2.866 / 2.872 µs | 2.204 / 2.195 µs |
| Curved split/restore | 0 / 1,024 curves | 5.027 / 5.038 ms | 5.016 / 5.033 ms |
| Existing site-overlap control | 0 / 10,000 sites | 0.497 / 0.496 µs | 0.505 / 0.504 µs |
| Native readiness, p95 | 0 / 100,000 buildings | 0.208 / 0.212 ms | 0.184 / 0.190 ms |
| Native planning worker, p50 | 0 / 100,000 buildings | 35.458 / 35.497 ms | 35.625 / 35.597 ms |
| One snapshot per edit | 0 / 100,000 buildings | 0.009 / 3.474 ms | 0.007 / 3.735 ms |

Generation adds 3.4–3.6% while remaining below its 1 ms budget; pick, brush and erase improve and
pass their existing ceilings. Every current-build readiness p95 is below 0.198 ms across all
three trials and four background sizes, so the 0.300 ms readiness gate passes for this build.
The fresh baseline controls do not reproduce the earlier tail spikes either; the historical
misses are retained above, without attributing individual outliers to a particular cause. The
largest fixture still contains 600,024 agents, 100,004 parcels and 100,000 remote painted cells.
Snapshot costs remain separate whole-world setup costs; these results do not certify complete
road-edit locality or all interactive runtime paths.

Commands, compiler/CPU/worker settings, source hashes, all raw trials and summary are in
`/tmp/zone04-readiness/{measure.py,metadata.json,before-source.json,summarize.py,summary.json}` and
`{generation,curve,edit,site,road-plan}-{before,after}-{1,2,3}.log`. The retained query test binary
is `after-tests`, SHA-256
`1de75067e367d8c2f872ce651a0e7b0f582124660a8946adb91ef180007eb25d`;
the baseline is `/tmp/zone04-removal/after-tests`, whose hash is recorded above.

The same extension also passes all eight real Forward+ Vulkan/Wayland reference captures on
the RX 7900 XTX. All PNGs were inspected and are byte-for-byte identical to the accepted alignment
captures; the manifest reports identical cell/mesh uploads, including 1,055 cells in each block
orientation. Artifacts: `/tmp/zone04-readiness/rendered/{manifest.json,01_*.png,...,08_*.png}` and
`rendered.log`. The capture command above uses
`METRUM_ZONE_REFERENCE_DIR=/tmp/zone04-readiness/rendered`. These are geometry/appearance checks,
not frame-time acceptance.

### Node merge, terrain and road-class acceptance

Three additional native regressions pass without production changes:

- A road crossing just inside an existing endpoint creates an asserted node alias through the
  actual preview/commit path. At 0, 0.35 and -0.55 radians, both painted and erased occupied lots
  retain their cell keys, canonical corners, claims, building pose and frontage/entrance links
  through regeneration. Save/load compacts the aliases while preserving the occupied lot; undo
  restores the original graph counts, geometry, grid choices and attachments.
- A completed terrain sculpt stroke retains paint and lot geometry while making the targeted
  site unbuildable. A retained zoning gesture and a previously collected growth action are both
  rejected. Undo restores feasibility and the original action can build on the same lot. The
  axis-aligned and rotated cases use the production terrain authoring and allocator paths.
- Ground-to-bridge and ground-to-tunnel changes preserve paint, retire empty frontage lots and
  reject stale gestures. Returning to ground restores the same lot footprints in both tested
  orientations. Existing no-build toggle coverage remains in the same suite.

Fresh release verification is **15 cell integration tests** and **1,979 full library tests**
(75 timing fixtures ignored), all passing. Commands:
`cargo test --offline --release --manifest-path rust/Cargo.toml --lib
nodes::sim::core::tests::zoning_buildability::cell_zoning -- --nocapture` and
`cargo test --offline --release --manifest-path rust/Cargo.toml --lib`.
Logs and source/compiler identities are in
`/tmp/zone04-merges/{cell-tests.log,library-tests.log,metadata.json}`. Test binary SHA-256:
`ddc9830f3270802685189532d0054c27292233324d6fc7a6a4f9494f0b4b71f4`.
Source hashes confirm only test modules changed from the query build; the deployed extension
still has the SHA-256 recorded above. Godot captures and performance measurements were not rerun
for these test-only additions; the preceding evidence retains its recorded scope.

### Node movement reservation guard

The move tool previously changed a road's geometry directly even when its new footprint crossed
painted cells. A native regression reproduced that mutation before the fix. Movement now compiles
a bounded prospective surface through the existing preview graph-copy and incidence-halo helpers,
then checks its carriageway, curb, sidewalk and node polygons against cell paint and occupied
claims using the same precise road-contact predicates as placement. Failed compilation or overlap
rejects the move before changing graph, paint, undo history, lanes or routing. Empty, unreserved
generated cells remain available. Aliased node IDs resolve to their canonical node; nonfinite
positions and unchanged positions produce no edit.

Accepted movement rebuilds clips at affected endpoints and refreshes their incident approach
dependencies. It no longer performs a whole-graph clip rebuild. The bounded check costs
O(K log K + P + S + Q), where K is copied local edges, P their profile points, S local surface
compilation and Q indexed reservation work. Only fixed adjacency layers are copied; distant
roads, buildings and agents are not scanned by the new check. Existing accepted-move finalization
still includes broader entrance/routing work, so this is not complete movement-locality acceptance.

The Rust bridge returns whether movement was accepted. Godot retains the accepted cursor position
and skips renderer rebuild requests on refusal. Regressions cover painted empty cells, painted
occupied lots and erased occupied claims at two orientations, clear extension/regeneration/undo,
and a terminal curb/sidewalk cap that overlaps paint while the straight corridor stays clear.
Rejected moves retain authoring epochs and undo depth. Native and controller bridge cases check
both rejection and accepted movement.

Initial guard release verification: **1,982 library tests pass**, 76 timing fixtures ignored; all three
headless Godot bridge suites and warning-free Rustdoc pass. Commands are the full library test,
release build, the three named bridge scripts above, and
`cargo doc --offline --no-deps --manifest-path rust/Cargo.toml`. Logs are under
`/tmp/zone04-movement/{before-tests.log,library-tests.log,build.log,rustdoc.log}` and the three
named bridge logs. Extension SHA-256:
`e8df9a35957d97244b500f4d0f0e0185a60db33823f3abf2d30a7c5d0093028b`.
Test binary SHA-256:
`98ed4f2f031378542043c099d390f16279479a57d1d075246b56c01471f6a510`.

Three matched unprofiled road-planning pairs and three node-validation trials use CPU 0,
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, rustc 1.98.1 and an i9-12900K. Each component has
100 samples per background size after three road warmups or five node warmups. Fixture construction
is excluded; road-preview snapshot cost is recorded separately. The second road pair reverses execution order. Both
retained binaries run the same road-placement workload; the current fixture additionally exposes
the separate node-validation mode. The baseline production code is the preceding query build.
All source and binary hashes remained unchanged during measurement. Commands, hashes, raw logs
and parsed rows are in `/tmp/zone04-movement/{measure.py,metadata.json,results.json}` and
`road-{before,after}-{1,2,3}.log` / `node-after-{1,2,3}.log`.

The table reports the median of each trial's p95, in milliseconds. Each move compiles one span
and two terminals and checks reservations; destruction of that temporary surface is also timed.

| Remote buildings / painted cells | Road readiness before | Road readiness after | Clear move validation | Blocked move validation |
| --- | ---: | ---: | ---: | ---: |
| 0 | 0.179922 | 0.184194 | 7.182769 | 7.628517 |
| 1,000 | 0.184805 | 0.186399 | 7.200434 | 7.667128 |
| 10,000 | 0.184710 | 0.189291 | 7.283397 | 7.692516 |
| 100,000 | 0.190626 | 0.182980 | 7.313347 | 7.768363 |

All local products match exactly through 600,024 agents, 100,004 parcels and 391 remote roads.
The highest current road-readiness p95 is 0.193891 ms, below its 0.300 ms ceiling. Median road-worker
p50 at 0 / 100,000 remote buildings is 35.654 / 35.547 ms before and 35.439 / 35.436 ms after.
The separate current preview snapshot still grows from 0.007654 to 3.509697 ms. Node validation's
highest p95 is **7.808511 ms**, exceeding the **5 ms** component budget recorded before measurement;
its performance gate remains open despite the stable local work. There is no equivalent node
validation in the earlier production code, so these node figures are not a before/after speedup.

A separate `perf record -F 499 -g --call-graph dwarf,16384` run of the node fixture locates self
samples in span-boundary generation and polygon crossing checks, among other work. Artifacts are
`/tmp/zone04-movement/node-perf{.data,.log,-report.txt,-children.txt}`. This profile includes setup;
its timings are diagnostic only and do not replace the unprofiled figures or establish a complete
breakdown of validation cost.

The initial guard also required separate terminal pieces for bridges, although the compiler
intentionally assigns those ends to the span. A native regression with unrelated cell paint
reproduced refusal of a clear bridge extension. The guard now uses the compiler's explicit failure
ledger to distinguish missing required geometry from intentionally omitted node pieces. The
required-span checks and exact reservation checks remain in place. The new regression verifies
the accepted bridge extension, unchanged remote paint, and undo restoring its endpoint.

After this correction, **1,983 release library tests pass** (76 timing fixtures ignored), all three
headless Godot bridge suites pass, and Rustdoc is warning-free. The commands are unchanged; logs
are `/tmp/zone04-movement/bridge-{library-tests,build,rustdoc}.log` and
`bridge-{zoning_cells_tool_test,zoning_road_tool_test,road_junction_preview_test}.log`. The failing
regression is retained as `bridge-before-fixed-fixture.log`; the earlier `bridge-before.log` stopped
in a ground-terrain fixture assertion and is not evidence of the movement bug. Current extension
SHA-256: `ab265ff347e1aed9b0707d6186498ab84864771b6fc5170801d28f59cfa19137`.
Current test binary SHA-256:
`4db2d9ab81113cc363711a7cc9e9e99e38f4471d5994c98344399db0faf3094c`.
The preceding road-planning controls and profile apply to the initial guard build recorded above;
the correction changes only the node movement validator and its new regression.

Three further matched node-validation pairs compare the retained initial guard with this correction,
using the same CPU, worker count, backgrounds, warmups and samples. The second pair reverses order;
source and binary identities remain unchanged throughout. Median p95 for clear moves at 0 / 100,000
remote buildings is 7.223 / 7.474 ms before and 7.194 / 7.378 ms after; blocked moves are
7.673 / 7.789 ms before and 7.853 / 7.754 ms after. Every run retains identical local products across
background sizes. Maximum p95 across all current cases is **8.437701 ms**, versus 8.361399 ms before;
the **5 ms** component gate still fails. This is a correctness correction, with no established
performance improvement. Commands, source/binary identities, rows and raw logs are
`/tmp/zone04-movement/bridge-{measure.py,metadata.json,results.json}` and
`bridge-node-{before,after}-{1,2,3}.log`. Accepted-move finalization remains outside this component
measurement, and no new rendered-reference captures were taken for the movement changes.

### Staged node validation and boundary lookup cost

Node movement now checks each prospective span's actual compiled carriageway, curb and sidewalk
polygons before solving terminal or junction pieces. Any span overlap is sufficient to reject the
move, so that case returns a deliberately partial, transient surface and never mutates or publishes
it. A clear span is retained through the existing exact preview-artifact certificate for the
remaining compile; terminal-cap-only overlap still requires and receives the node check.

The same certificate can offer immutable node pieces from the bounded source neighborhood.
Only a current, successfully published live generation can supply candidates, and the existing
complete input comparison decides reuse. A moved road's sampled mouth geometry can change at
both ends even when one endpoint stays fixed; those pieces must be rebuilt. Regressions compare
reused and dirty-cache results with cold compilation, including remapped local IDs. No proximity,
rotation or quantization tolerance substitutes for that input comparison.

Two shared compiler loops also no longer repeat quadratic work. Terrain-clip source endpoints use
a temporary sorted table of existing XZ/height keys: O(P log P + E log P) time and O(P) scratch for
P loop points and E source edges. Original loop indices resolve duplicate keys, preserving the
first matching coordinate and its exact height; unmatched source coordinates remain unchanged.
Earthwork vertex normals use winding computed once per loop, reducing that part from O(P²) to
O(P). These are edit-time temporaries, with no new persistent spatial index or dependency.

The separate diagnostic run identified terminal solving as the dominant cost in the measured
clear move (about 6.9 ms for two nodes versus 0.4 ms for the span). Logs are
`/tmp/zone04-boundaries/node-phases-expanded-stack.log` and `reuse-phases.log`; the command uses the
retained release test fixture with `RUST_MIN_STACK=33554432 METRUM_DEBUG=1 METRUM_DEBUG_PERF=1
METRUM_DEBUG_FILTER=road RAYON_NUM_THREADS=1`. The diagnostic run overflowed the ordinary Rust
test-thread stack before emitting phase measurements (`node-phases.log`); that failed run produced no phase evidence.
Diagnostic timings include logging and are not acceptance measurements. Normal timing runs leave
diagnostics disabled and use the default test-thread stack.

Fresh validation: **1,986 release library tests pass** (76 timing fixtures ignored), all three
headless Godot suites pass, and Rustdoc is warning-free. The full-library, release-build, headless
script and Rustdoc commands are unchanged from the movement checkpoint. Logs are
`/tmp/zone04-boundaries/{final-library-tests-2,build,rustdoc}.log` and the three named Godot suite
logs. Current extension SHA-256:
`53cc46e8bcf72073641ca71bfc854b5b0132b1d31123e56e2c7df6b728bd3e1b`.
Current release test binary SHA-256:
`f5f58b2e6a70b4bce01ce022d45cc620c51a4d03051460331950f7ab56304bb9`.

Three matched unprofiled pairs for both node validation and road planning use CPU 0,
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`, rustc 1.98.1 and an i9-12900K. The second pair reverses
binary order. Each background has 100 measured samples after five node or three road warmups;
construction is excluded and the preview snapshot is timed separately. Before/after inputs are
the same moves and reservations. An after-build span conflict intentionally has no compiled node
pieces, since it already proves rejection; clear moves retain the full products. Each build
asserts identical local products across backgrounds. No competing build, test or capture ran.
Commands, source/binary hashes and all raw rows are in
`/tmp/zone04-boundaries/{measure.py,metadata.json,results.json}` and
`{node,road}-{before,after}-{1,2,3}.log`. All measured identities remained unchanged.

Median of the three trial p95 values, in milliseconds:

| Remote buildings / painted cells | Clear move before | Clear move after | Span conflict before | Span conflict after |
| --- | ---: | ---: | ---: | ---: |
| 0 | 7.787353 | 7.692591 | 8.088856 | 0.482203 |
| 1,000 | 7.701971 | 7.718906 | 8.092903 | 0.476689 |
| 10,000 | 7.761311 | 7.770288 | 8.120599 | 0.480802 |
| 100,000 | 7.798812 | 7.769653 | 8.269307 | 0.495023 |

At 100,000 remote buildings, median span-conflict p50 drops from 7.797955 to 0.327490 ms.
All span-conflict p95 values are below 0.547 ms. Clear-move maximum p95 is **7.976454 ms**, so the
**5 ms node-validation gate remains unmet**. Reusing only exact inputs cannot skip the two changed
caps in this clear-extension fixture. The shared boundary-loop changes establish improved
complexity, not a measured clear-move speedup. The populated workload still reaches 600,024 agents,
100,004 parcels and 391 remote roads with matching local results.

Road-readiness p95 medians at 0 / 1,000 / 10,000 / 100,000 remote buildings are
0.182214 / 0.292617 / 0.222480 / 0.231472 ms before and
0.191348 / 0.188166 / 0.191536 / 0.212525 ms after, below the established 0.300 ms ceiling.
Individual third-trial p95 values exceed it: 0.338798 and 0.314699 ms before, and 0.314206 ms after.
These tails are retained; no cause or blanket tail-latency pass is claimed. Median road-worker p50
at 0 / 100,000 backgrounds is 37.307 / 37.395 ms before and 37.425 / 37.294 ms after. The separate
current preview snapshot grows from 0.007927 to 4.066154 ms; complete edit-path locality remains open.

Eight fresh Godot 4.7.2 Forward+ / RX 7900 XTX captures and their fixture records are byte-identical
to the accepted query-build references, with 667 / 432 / 760 / 1,055 / 1,077 / 1,002 / 672 / 1,055
cells and four cell meshes each. The angled-junction image was also inspected. The real-renderer
command recorded above uses `METRUM_ZONE_REFERENCE_DIR=/tmp/zone04-boundaries/rendered`;
artifacts are `rendered.log` and `rendered/{manifest.json,01_*.png,...,08_*.png}`. These verify
geometry and appearance, not frame-time or allocation acceptance.

### Reuse of immutable ring source preparation

Node ownership cleanup now prepares source-point and rail-path lookup data once per distinct
source and policy within a solve. The context immutably borrows the existing rail set and is dropped
after cleanup. Its key includes owner, material, mouth, band, generated-cap eligibility and source
canonicalization policy. Contours still run through every existing canonicalization, noding and
cleanup pass; this does not reuse their results or change collision tolerances. The preparation
operations themselves are unchanged.

For U distinct source/policy combinations and R region visits across cleanup passes, source
preparation runs U times, followed by O(R log U) context lookups and the unchanged contour work.
Temporary storage is the sum of those prepared point/path indices; none is retained between
compiles. U depends on the local node's sources, not the city's roads or cells. No dependency or
persistent spatial index is added. This reduces repeated preparation, without claiming an
improvement to the underlying geometry algorithms' complexity or closing the allocation gate.

Fresh validation passes **1,987 release library tests** (76 timing fixtures ignored), all three
headless Godot suites and warning-free Rustdoc. The new regression checks repeated and reversed
source visits, changed contours, different bands/mouths/owners/materials, source-less regions,
cap eligibility and canonicalization policies against both explicit expected coordinates and
fresh preparation. Existing ambiguity, curved/junction, cold/reused compilation and zoning
movement regressions also pass. Commands match the previous checkpoint; logs are
`/tmp/zone04-terminals/{final-library-tests,build,rustdoc}.log` and the three named suite logs.
Release test binary SHA-256:
`f9367b60fbf2237761365c3f33931975124440acd527cdcd68471207f19d66e1`.
Deployed extension SHA-256:
`9975dd56cbfe8f50d097834487a64e05f8b987e4368b1bc6d4071e5943298818`.

Three matched unprofiled node-validation and road-planning pairs use CPU 0, one Rayon worker,
`METRUM_DEBUG=0`, rustc 1.98.1 and the same i9-12900K. Trial two reverses binary order. The retained
baseline is the previous checkpoint's `f5f58b2...` test binary. Each fixture has 100 measured
samples after five node or three road warmups; construction is excluded and preview snapshots
are separate. No build, test, renderer or profiler competed with timing. Exact local products
match across 0 / 1,000 / 10,000 / 100,000 remote buildings and painted cells, reaching 600,024
agents, 100,004 parcels and 391 remote roads. Commands, all source/binary hashes and raw rows are
`/tmp/zone04-terminals/{measure.py,metadata.json,results.json}` and
`{node,road}-{before,after}-{1,2,3}.log`. Measured identities remained unchanged.

Clear-move times, median of the three trial percentiles, in milliseconds:

| Remote buildings / painted cells | p50 before | p50 after | p95 before | p95 after |
| --- | ---: | ---: | ---: | ---: |
| 0 | 7.048005 | 6.552630 | 7.293662 | 6.759197 |
| 1,000 | 7.047310 | 6.523535 | 7.165992 | 6.683908 |
| 10,000 | 7.078706 | 6.543605 | 7.208829 | 6.799696 |
| 100,000 | 7.112319 | 6.661256 | 7.266831 | 7.131222 |

The largest-background p50 improves by 6.3%. Maximum clear-move p95 is 7.586863 ms before and
**7.247544 ms after**, still above the **5 ms gate**. Span-conflict rejection does not enter node
cleanup; its largest-background p50 is 0.324264 / 0.324870 ms, with no improvement claimed.
Road-readiness p95 medians are 0.181429 / 0.181021 / 0.189392 / 0.192670 ms before and
0.179334 / 0.186154 / 0.188227 / 0.190697 ms after. All individual trial values are below the
0.300 ms ceiling (current maximum 0.197335 ms); this does not erase the previous checkpoint's
recorded tails. Worker p50 at 0 / 100,000 backgrounds is 35.355 / 35.221 ms before and
34.851 / 34.775 ms after. Current one-time snapshot cost grows from 0.007617 to 3.420721 ms,
so whole edit-path locality remains open.

After timing, eight fresh Godot 4.7.2 Forward+ / RX 7900 XTX captures and their fixture records
match `/tmp/zone04-boundaries/rendered` byte-for-byte. The rotated block was also inspected.
The previously recorded renderer command uses `METRUM_ZONE_REFERENCE_DIR=/tmp/zone04-terminals/rendered`;
results are `rendered.log` and `rendered/{manifest.json,comparison.json,01_*.png,...,08_*.png}`.
These are geometry/appearance checks, not renderer frame-time or allocation acceptance.

### Isolated cell heap budgets

[`benchmarks/zoning_memory.py`](../benchmarks/zoning_memory.py) now drives two ignored Rust
fixtures using the installed glibc 2.44 `memusage --no-timer` profiler. Its
[event format](https://github.com/bminor/glibc/blob/master/malloc/memusage.c) records requested
live heap after successful normal malloc/calloc/realloc/free events. These are heap events,
including frees and reallocations; their count is not an allocation-only count. The measurements
do not cover RSS, aligned-allocation APIs, direct mappings or GPU memory.

Unique allocation/free markers delimit each operation. Setup, logging and markers are excluded;
returned products remain live at the end of the window. Every process must pass an empty-phase
calibration, a retained 12,345-byte allocation and a known grow/shrink/free sequence. Missing,
ambiguous or truncated boundaries fail the reader. Separate synthetic checks verify exact peaks
and retention and reject truncated traces, missing closes and duplicate starts. Complete raw
traces, including excluded setup, are retained as gzip files and pass integrity checks.

Budgets are set in the fixture before measurement: **zero heap events** for repeated idle lot
preparation, completed-chunk preparation and picking; **256 KiB additional peak heap** for each
fixed local preview or gesture edit; **256 KiB + 128 bytes per cell** for constructing the painted
store and selecting a large fill. These are scoped regression ceilings, not whole-game memory
budgets. They are not enlarged in response to the measurements.

Three runs pass **168 phases**, including 18 calibrations. They use CPU 0, one Rayon worker,
`METRUM_DEBUG=0`, rustc 1.98.1 and the existing populated fixture through 600,024 agents,
100,004 parcels, 391 remote roads and 100,000 remote painted cells. The Rust fixture compares
exact local selections across backgrounds and restores paint and undo depth after each edit.
Event counts, cumulative growth/release, additional peak heap and retained bytes are identical
across all four backgrounds and all three runs. **101,376 idle preparation/pick operations record
zero heap events.** No profiled timings are used as latency evidence.

Per-operation results for the fixed native neighborhood, in bytes:

| Operation | Heap events | Additional peak | Retained at return |
| --- | ---: | ---: | ---: |
| Cell preview | 1 | 64 | 64 |
| Marquee preview | 3 | 256 | 256 |
| Brush preview | 5 | 1,024 | 1,024 |
| Fill preview | 25 | 8,864 | 2,048 |
| Paint one cell | 369 | 196 | 24 |
| Erase one cell | 3 | 40 | 24 |
| Undo Erase | 367 | 180 | -24 |
| Undo paint | 1 | 0 | -24 |

Small peak heap does not imply low allocation traffic: paint grows/releases 11,740 / 11,716
bytes over its 369 events, and undo Erase grows/releases 11,700 / 11,724 bytes. Negative retention
means the operation releases existing storage. Paint/Erase undo entries each retain 24 bytes in
this one-cell fixture; their corresponding undo releases it.

The separate six-deep painted-store fixture extends up to 19 km. It covers compact saved paint
and store-level fill traversal; it excludes road-frontage generation, lots and native cold-fill
materialization. Results are identical in all three runs:

| Cells | Store construction peak | Store retained | Fill additional peak | Fill result retained |
| --- | ---: | ---: | ---: | ---: |
| 768 | 34,660 | 23,876 | 68,768 | 16,384 |
| 3,072 | 134,404 | 91,700 | 274,592 | 65,536 |
| 11,400 | 535,844 | 364,052 | 1,097,888 | 262,144 |

Reproduce with a release library test binary:

```bash
cargo test --offline --release --manifest-path rust/Cargo.toml --lib --no-run
python3 benchmarks/zoning_memory.py --binary /tmp/zone04-allocations/final-tests --output /tmp/zone04-memory-new --runs 3
```

The retained measured binary is a copy of the compiled library test executable; substitute the
executable printed by Cargo when measuring a new build. Its SHA-256 is
`26a3a6b34247d8450ac9333702744c2cde2c33958546034ebbb398e2b699abf2`.
Commands, source/binary identities, raw logs, compressed traces and exact locality checks are
`/tmp/zone04-allocations/repeated/{metadata.json,results.json,summary.json,*.log,*.data.gz}`.
Fresh ordinary verification passes **1,987 release library tests**, with 78 timing/profiling
fixtures ignored (`/tmp/zone04-allocations/library-tests.log`). Production source and the deployed
extension are unchanged from the preceding geometry checkpoint, verified by hash. Its Godot
bridge/rendering and latency evidence remains attached to that build; those checks were not
rerun for this test-only addition.

These backend idle/query/edit and painted-store heap budgets now pass. Generated frontage/lot
memory, native cold/large-fill materialization, dense local conflicts, and Godot bridge/renderer
allocation costs still need their own acceptance evidence. The following fixture measures the
fixed-view geometry payload and mesh surface buffers separately.

### Native overlay locality and frame cost

[`zoning_overlay_benchmark.gd`](../godot/tests/zoning_overlay_benchmark.gd) exercises the unchanged
production cell/parcel renderer and native bridge on an 8 km world. A fixed 1,440 × 960 viewport
holds a 560 m road, its cells and one manual parcel constant while remote roads increase from
0 to 8, 32 and 128, carrying 0 / 1,536 / 6,144 / 24,576 painted cells. The fixture has no buildings
or agents. World creation and incremental background setup are recorded separately. It disables
the parent overlay's unrelated road-eligibility update and measures the cell/parcel child directly.

Each background has 20 warmup and 101 measured operations for both idle and alternating
single-cell paint/Erase. Each operation waits until **all visible generation flags and versions
settle**, including work that returns unchanged geometry. Timing covers production `_process`
calls; gesture authoring, assertions, surface inspection and waiting for frames are outside it.
`update_us` is the worst update call in an operation; `operation_update_us` sums its calls.
The fixed ceilings, set before measurement, are idle p95 ≤ 0.5 ms, edit-frame p95 ≤ 5 ms,
viewport GPU p95 ≤ 2 ms and ≤ 128 KiB packed geometry per single-cell edit.

Three isolated measurement runs cover **2,424 measured operations**. Exact local mesh arrays
remain identical across backgrounds and after each paint/Erase sequence. All 16 requested chunk
instances remain resident; four contain geometry. Idle replaces no meshes and uploads no
geometry. Every edit replaces exactly chunk `(0, 0)`, preserving every other instance and mesh.
The edit's logical packed geometry is **28,224 bytes** and its new mesh surface buffers total
**16,128 bytes**, identical across every sample and background. Resident geometry remains
112,616 packed bytes / 64,352 surface-buffer bytes. Buffer sizes use Godot's
[`mesh_get_surface`](https://docs.godotengine.org/en/stable/classes/class_renderingserver.html#class-renderingserver-method-mesh-get-surface)
vertex/attribute/index data; these are logical buffer sizes, not measured driver transfers,
allocation counts or total GPU memory.

Median of the three trial statistics, in milliseconds:

| Remote roads | Remote painted cells | Idle update p95 | Worst edit frame p95 | Total edit updates p50 |
| --- | ---: | ---: | ---: | ---: |
| 0 | 0 | 0.292 | 18.392 | 20.818 |
| 8 | 1,536 | 0.290 | 16.814 | 19.161 |
| 32 | 6,144 | 0.288 | 16.577 | 19.616 |
| 128 | 24,576 | 0.290 | 18.860 | 19.615 |

Maximum individual trial idle p95 is **0.327 ms**; worst edit-frame p95 is **24.556 ms**.
Every trial fails its four edit-latency checks. Geometry, mesh reuse, upload caps and other
assertions pass. The isolated viewport draws five calls / 1,342 primitives in every measured
sample; maximum GPU p95 is 0.027 ms. This does not measure a complete gameplay frame or dense
local zoning. Background independence does not make the present edit cost acceptable.

Separate diagnostic runs isolate the expensive path: `mark_cell_keys_dirty` invalidates
a direct conflict halo, and `prepare_cell_chunk_internal` then regenerates each affected entire
512 m chunk. One edit dirties four visible chunks in this fixture. The renderer normally drains
them over two frames, although only one mesh changes. An unchanged neighboring chunk still costs
about 4.2–4.5 ms in the diagnostic. Clean native export is about 1.7 ms median and mesh replacement
about 0.19 ms. These diagnostic numbers locate work; they are not acceptance timings. Reducing
regeneration must preserve pinned-frame, erase and conflict semantics; the latency gate stays open.

Measured with the existing release extension SHA-256
`9975dd56cbfe8f50d097834487a64e05f8b987e4368b1bc6d4071e5943298818`, one Rayon worker,
unrestricted CPU affinity, Godot 4.7.2 Forward+ and RX 7900 XTX. No production source changed.
Run with an unused output directory:

```bash
METRUM_ZONE_OVERLAY_DIR=/tmp/zone04-overlay-new METRUM_DEBUG=0 RAYON_NUM_THREADS=1 XDG_DATA_HOME=/tmp/zone04-godot-data XDG_CONFIG_HOME=/tmp/zone04-godot-config godot --display-driver wayland --rendering-method forward_plus --audio-driver Dummy --resolution 64x64 --path godot --script res://tests/zoning_overlay_benchmark.gd
```

Artifacts are `/tmp/zone04-overlay/{metadata.json,summary.json,settled-{1,2,4}.log}` and
`settled-{1,2,4}/results.json`; source/binary hashes remain unchanged through all runs. Trial 2
finished all measurements and printed its failure summary but hung during engine shutdown and
was interrupted. Trial 3 began during that shutdown and is excluded; trial 4 replaces it after
both earlier processes ended. The initial `trial-1` prototype stopped at the first changed mesh
and is diagnostic only. Full-settling trials supersede its timings. Fresh script parsing and the
cell-tool bridge regression pass; Rust tests and reference captures were not rerun for this
benchmark-only addition. Their earlier evidence remains attached to the unchanged build.

### Bounded generation batches after local edits

Each existing cell chunk now retains the union of its invalidated world bounds, clipped to the
chunk. Cold caches still require the whole chunk; global invalidation clears any partial-work
record. A ready query remains one hash lookup. A dirty query examines at most nine neighboring
entries and combines their pending bounds only when both resulting dimensions fit within 512 m.
Otherwise it regenerates the requested chunk's own pending area. This bounds each generation
request independently of distant city size and avoids a large road edit becoming one world-sized
render request.

After generation, only the requested chunk and warm neighbors whose entire dirty bounds were
covered become current. Partly covered and cold neighbors retain their pending work. Both native
overlay preparation and gesture materialization use this path. Dirty bookkeeping takes constant
space per chunk, O(touched chunks) invalidation and O(1) bounded query/completion work. The existing
conflict halo, exact footprint predicates, pinned-frame priorities and generated geometry remain
authoritative; no new index, dependency or save-format field is introduced.

Fresh release verification passes **1,990 library tests**, with 78 manual fixtures ignored.
New tests compare partial/batched generation with forced full regeneration on straight,
perpendicular, competing and curved roads at two rotations, including repaint/Erase, external
exclusions, frontage and erased occupied claims. Additional checks cover accumulated edits,
negative chunk boundaries, cold/global invalidation, complete neighboring batches and pending
work that exceeds the batch bounds. The test log is
`/tmp/zone04-region-batches/library-tests.log`. All three Godot bridge suites and Rustdoc also pass
on the candidate extension. The benchmark now returns from the draw callback before requesting
engine shutdown; this follows the reference fixture's teardown order and does not change its
timed work.

Three fresh heap repeats pass all **168 calibrated phases**. Fixed native query/edit event counts,
peak heap and retention match the preceding heap checkpoint exactly and stay identical across
backgrounds; 101,376 idle preparation/pick operations again record zero events in the profiler's
covered APIs. The 11,400-cell painted store now peaks at 540,964 bytes and retains 369,172 bytes,
an additional 5,120 retained bytes for the chunk work records. Large-fill peak remains
1,097,888 bytes. These still meet the existing budgets. Full traces, calibration logs and exact
metric comparisons are under `/tmp/zone04-region-batches/memory/`; profiled time is not latency
evidence. The measured test executable SHA-256 is
`be1ab98f3fc66112d72049bdffa33edc752b7cc7798aeaabf8f0f3694e42d397`.

Three matched, unprofiled before/after renderer pairs use the preceding overlay fixture unchanged
inside its timing windows. Pair 2 reverses build order. Profiling, compilation and other tests
finish before timing. Both builds use one Rayon worker, unrestricted CPU affinity and the same
Godot 4.7.2 Forward+ / RX 7900 XTX device. Median trial statistics, in milliseconds:

| Remote roads / painted cells | Total edit updates p50, before → after | Worst edit frame p95, before → after |
| --- | ---: | ---: |
| 0 / 0 | 18.378 → 7.085 | 12.350 → 8.699 |
| 8 / 1,536 | 18.987 → 7.051 | 12.904 → 9.259 |
| 32 / 6,144 | 18.928 → 7.070 | 11.959 → 8.569 |
| 128 / 24,576 | 18.921 → 7.137 | 11.318 → 8.501 |

Total update work improves **61–63%**; typical settling falls from two frames to one. Across
4,848 measured operations, local mesh arrays, the one-chunk edit replacement, 28,224 packed bytes,
16,128 surface-buffer bytes, five draw calls and 1,342 primitives remain identical. Idle uploads
nothing. Candidate maximum trial idle p95 is 0.246 ms and GPU p95 is 0.028 ms. Maximum edit-frame
p95 falls from 14.025 to **11.142 ms**, still above the **5 ms gate** in every case. This is a
measured improvement, not renderer performance acceptance. Remaining generation/export cost and
renderer allocations still need work; dense local conflicts and complete gameplay-frame cost
are outside this fixture.

The candidate extension SHA-256 is
`045fefdd103d16b64693c1427db9e0fc1429c6be0654b867f007915d521640f0`; baseline is
`9975dd56cbfe8f50d097834487a64e05f8b987e4368b1bc6d4071e5943298818`.
Commands, source/build identities, run ordering, individual results and exact assertions are
`/tmp/zone04-region-batches/{measure.py,measurement-metadata.json,summary.json,before-*.log,after-*.log}`
and `{before,after}-{1,2,3}/results.json`. All measurement windows are isolated; candidate 1,
baseline 2 and candidate 3 required interruption after completing measurements but hanging in
engine shutdown. The draw-callback teardown change does not resolve the hang; its cause is unverified.
Each process ended before the next started, and the candidate library was restored and hash-checked.

Eight fresh reference captures and fixture records match `/tmp/zone04-terminals/rendered`
byte-for-byte; the rotated block was also inspected. Artifacts are
`/tmp/zone04-region-batches/rendered/{manifest.json,comparison.json,01_*.png,...,08_*.png}` and
`rendered.log`. That process also printed all eight passes and then required interruption during
shutdown. All six compressed heap traces pass integrity checks. The preliminary candidate that
only clipped dirty bounds is retained under `/tmp/zone04-dirtyregions/`; its timings describe that
intermediate build, not the final batching implementation.

Outstanding: remaining external edit-path acceptance; complete road-edit/removal locality;
node-validation latency; remaining memory/allocation and matched rendering performance acceptance.
`ZONE-04` remains in progress until these are verified. Each historical measurement
applies only to its recorded source/binary identity.

### Exact conflict pruning

Generation now skips three provably irrelevant checks. A candidate wholly inside the publication
bounds uses the existing reservation mask: untouched empty cells lie outside those bounds and
cannot overlap it. Boundary-crossing candidates retain the full check. A temporary candidate
store containing one canonical frame needs no competing-frame lookup, since distinct lattice
addresses cannot overlap. Road-corridor queries reject segments whose full-width envelope misses
the cell before computing the contracted corridor and exact integer overlap. These changes add
no persistent state, index, dependency or allocation. They preserve the existing local complexity
bounds; possible conflicts still use the same exact predicates and priority rules.

Fresh verification passes **1,992 release library tests**, with 78 manual fixtures ignored, and
all three headless bridge suites. New regressions cover an untouched empty cell outside a small
publication area that conflicts with a crossing candidate, retained incompatible paint, and
1,998 corridor queries compared with exhaustive segment testing across three rotations, two
coordinate offsets and near-boundary contacts. Existing partial/full generation, occupied claims,
curves and deterministic ownership checks also pass. Rustdoc, formatting and diff checks pass.

One fresh heap repeat passes all **56 calibrated phases**. Every measured heap metric matches
the previous batching checkpoint: 33,792 idle operations have zero observed events, and the
11,400-cell store retains 369,172 bytes with a 540,964-byte peak. These are the previously defined
libc-event/native-fixture measurements, not renderer or GPU allocation acceptance. Both compressed
traces pass integrity checks. Artifacts are `/tmp/zone04-conflict-cost/memory/`; its executable
SHA-256 is `f88613d64507bd6d50d608e7a59a72456e9cc1685db2009101d943f6f6b3a15d`.

Three fresh matched, unprofiled renderer pairs retain the same fixture, one Rayon worker,
unrestricted CPU affinity, Godot 4.7.2 Forward+ and RX 7900 XTX. Pair 2 reverses build order;
compilation, tests and profiling finish before timing. Median trial statistics, in milliseconds:

| Remote roads / painted cells | Total edit updates p50, before → after | Worst edit frame p95, before → after |
| --- | ---: | ---: |
| 0 / 0 | 9.256 → 8.899 | 17.421 → 14.508 |
| 8 / 1,536 | 8.570 → 8.607 | 16.407 → 15.595 |
| 32 / 6,144 | 8.331 → 7.059 | 17.799 → 12.675 |
| 128 / 24,576 | 8.510 → 8.803 | 15.268 → 16.077 |

These measurements **do not establish a consistent renderer speedup**. Results vary substantially,
and two background cases have slightly higher candidate medians. Maximum trial edit-frame p95
is 22.866 ms before and **17.103 ms after**, with every case still failing the unchanged **5 ms**
gate. Candidate maximum idle p95 is 0.402 ms and GPU p95 is 0.032 ms, within their budgets.
All 4,848 measured operations preserve geometry checks, zero idle uploads, one changed chunk per
edit, 28,224 packed bytes, 16,128 surface-buffer bytes, five draw calls and 1,342 primitives.
This remains a fixed visible neighborhood with distant roads/paint, not whole-gameplay acceptance.

The candidate extension SHA-256 is
`5ec3f961afc671462d80d45d663e46c3617df0ed1de80a0b56c305e9199259e0`; baseline is
`045fefdd103d16b64693c1427db9e0fc1429c6be0654b867f007915d521640f0`.
Commands, source/build identities, ordering and checks are in
`/tmp/zone04-conflict-cost/{measure.py,measurement-metadata.json,summary.json,checkpoint.json}`;
individual results are `{before,after}-{1,2,3}/results.json`. Baseline 1 and all three candidate
runs required interruption after recording their complete results and hanging during shutdown.
Each process ended before the next began; the candidate extension was restored and hash-checked.

Eight fresh reference PNGs and the entire manifest match the preceding batching checkpoint
byte-for-byte. See `/tmp/zone04-conflict-cost/rendered/{manifest.json,comparison.json,*.png}` and
`rendered.log`. That process also required interruption after printing all eight passes.
Headless phase diagnostics and a sampled process profile are retained in the same parent directory;
they are not acceptance timings. The profile includes setup and has incomplete worker stacks.
Shared road-footprint preparation remains a candidate for investigation, not a demonstrated fix.
The remaining acceptance gates listed above stay open; `ZONE-04` is still in progress.

### Road footprint preparation

Shared reservation queries now reduce polygon extrema in double precision before converting
only the final bounds outward to floats. Directed conversion is monotone, so the conservative
bounds match the former per-vertex conversion. Road overlap queries first test those bounds and
copy a precise contour only when an exact overlay is needed. The same even-odd overlay, road
query grid, duplicate-owner checks and reservation rules remain authoritative. Work remains
O(local polygon vertices), with no new spatial index, dependency or retained cache. Prepared
footprints use a second linear pass over their owned points; rejected road contours allocate no
point array. This removes repeated conversion and copying rather than approximating geometry.

Fresh release verification passes **1,994 library tests**, with 78 manual fixtures ignored, all
three headless bridge suites, Rustdoc, formatting and diff checks. New tests compare reduced and
pointwise outward bounds across positive/negative chunk edges and large coordinates, and verify
concavity, containment, edge contact, gaps and micrometre-scale intrusion. Eight reference PNGs
and the complete manifest remain byte-identical to the preceding checkpoint.

One fresh calibrated heap repeat passes all **56 phases**. Across 0 / 1,000 / 10,000 / 100,000
background buildings, a local paint operation drops from **369 to 9 observed heap events**;
undoing Erase drops from **367 to 7**. Requested cumulative growth drops from 11,740 to 220 bytes
and 11,700 to 180 bytes respectively; peaks fall from 196 to 132 bytes and 180 to 116 bytes.
Retention is unchanged. All other phase metrics match, including 33,792 zero-event idle
operations and the 369,172-byte retained 11,400-cell store. These are normal libc heap events,
including frees, in the existing isolated native fixtures; they do not cover renderer/GPU memory
or latency. Both compressed traces pass integrity checks. Artifacts are
`/tmp/zone04-road-footprints/memory/`, with test executable SHA-256
`80ac18ec4337afd3575816485cb4e9cbeb1314a2f7dab782811b4e22b8276fd4`.

Three matched, unprofiled renderer pairs keep the previous fixture, one Rayon worker,
unrestricted CPU affinity, Godot 4.7.2 Forward+ and RX 7900 XTX. Pair 2 reverses build order.
Tests, compilation and profiling finish before timing. Median trial statistics, in milliseconds:

| Remote roads / painted cells | Total edit updates p50, before → after | Worst edit frame p95, before → after |
| --- | ---: | ---: |
| 0 / 0 | 3.101 → 2.323 | 7.556 → 10.135 |
| 8 / 1,536 | 3.116 → 2.305 | 9.657 → 3.268 |
| 32 / 6,144 | 3.034 → 2.360 | 3.629 → 9.077 |
| 128 / 24,576 | 3.047 → 2.353 | 8.238 → 9.422 |

Median edit work improves **22–26%**, but tail latency does not consistently improve. Seven of
twelve cases on each build miss the unchanged 5 ms edit-frame gate; maximum trial p95 is
9.923 ms before and **12.220 ms after**. This is not renderer latency acceptance. Candidate
maximum idle p95 is 0.315 ms and GPU p95 is 0.035 ms, within their budgets. Across all 4,848
measured operations, geometry checks, idle zero-upload behavior, the one-chunk edit replacement,
28,224 packed bytes, 16,128 surface-buffer bytes, five draw calls and 1,342 primitives remain
identical. The fixture does not measure complete gameplay frames or dense local development.

Three additional matched native pairs preserve identical local products through **600,024
agents**, 100,000 background buildings, 100,000 distant painted cells and 391 distant roads.
These use CPU 0 and one Rayon worker, with no concurrent renderer, build, test or profiler.
Clear node-move validation remains essentially unchanged: maximum p95 is 6.972376 ms before and
**6.964345 ms after**, still above its 5 ms gate. Road-readiness maximum p95 is 0.158853 →
**0.158459 ms**, within 0.300 ms. Largest-background one-time snapshot medians are 3.379054 →
3.399228 ms; this separate background-dependent cost remains an outstanding locality issue.
Commands, source/binary identities, raw logs and summaries are in
`/tmp/zone04-road-footprints/native/`.

Candidate extension SHA-256 is
`a4a882469725d7e1906bafa6791b717c555ca2de52b57db21254f448e0017050`; baseline is
`5ec3f961afc671462d80d45d663e46c3617df0ed1de80a0b56c305e9199259e0`.
Renderer commands, identities, ordering and results are in
`/tmp/zone04-road-footprints/{measure.py,measurement-metadata.json,summary.json}` and
`{before,after}-{1,2,3}/results.json`. Baseline 2 required interruption after completing results;
all other processes exited. The runner now allows ten seconds after observing the final result
before interrupting a shutdown wait, without changing any measured window. Each process ended
before the next began and the candidate extension was restored and verified.

The separate headless export diagnostic places world-size snapshot reads at 1 µs median on both
builds; it does not implicate that read in this fixture's cost. Dirty-export median is 2.936 →
2.147 ms, with p95 5.953 → 2.738 ms. Those diagnostics and a whole-process CPU sample, including
setup, are under the same artifact directory and are not renderer acceptance timings. Reference
captures under GDB produced the identical images and manifest, and Godot exited normally, so the
intermittent shutdown wait was not reproduced or fixed. GDB returned 1 because its queued
post-run inspection/kill command had no live inferior; fixture verification and log inspection
confirm eight passes and normal inferior exit. See `reference-debug-metadata.json`, `rendered-debug.log`, and
`rendered/comparison.json`. Remaining edit-path, whole-edit locality, node latency, memory and
renderer acceptance gates stay open. `ZONE-04` remains in progress.

### Queued visible cell preparation

Visible chunk export now uses the existing simulation command queue for cold or invalidated
cell generation and unprepared building/road queries. The Godot call only tries the core lock,
checks readiness and exports completed geometry. A busy call leaves the retained mesh in place.
The simulation thread derives from current authority under that lock; it does not publish a
captured cell snapshot. World replacement/loading advance the existing global terrain payload
epoch, so an older request is discarded. Commands run while paused and do not request a new
movement tick, road-preview snapshot or render snapshot merely to prepare a cell cache.

One completion receiver per simulation node bounds outstanding preparation to one chunk job.
Repeated polls allocate no additional completion channel while that job is pending; completion
or channel disconnection permits retry. A new job creates a bounded acknowledgment channel.
The renderer requests current visible chunks on subsequent frames, retaining its two-upload
cap. This adds no spatial index, persisted state, dependency or thread. Scheduling and readiness
checks are O(1); generation retains the existing bounded dirty region and local query costs.
Completed chunks can still be exported while another chunk is pending. Synchronous gestures
continue to materialize the cells needed for their selection.

Before measuring this change, the overlay benchmark adds a **50 ms p95** completion and
presentation ceiling, alongside the unchanged **5 ms** edit-frame, **0.5 ms** idle-frame and
**2 ms** GPU ceilings. Completion measures from the finished paint commit until every visible
generation/version matches. Presentation additionally waits for the following post-draw signal;
it conservatively includes the intervening geometry checks. This is a renderer completion
measurement, not input-to-display latency or whole-gameplay frame acceptance. Both builds run
the same updated harness, and total main-thread update time, retry frames and upload invariants
remain recorded. Worker time and queue waits are included in elapsed completion/presentation,
so moving work off the frame cannot pass merely by deferring it indefinitely.

Fresh verification passes **1,996 release library tests**, with 78 manual fixtures ignored,
all three headless bridge suites, Rustdoc, formatting and diff checks. Regressions cover bounded
requests and completion/disconnection retry, edits arriving after a request, world replacement,
idempotent ready preparation, outside chunks, and cold preparation through the paused bridge.
Eight fresh reference PNGs and their complete manifest match the preceding checkpoint exactly.

Three matched unprofiled pairs use one Rayon worker, unrestricted CPU affinity, Godot 4.7.2
Forward+, RX 7900 XTX and the same 16 visible chunks / four meshes / one manual parcel fixture.
The simulation node sets the effective frame cap to **60 Hz** during initialization; this follows
the harness's earlier zero-cap assignment. Pair 2 reverses build order. Builds, tests and profiling
are absent during timings. Median trial statistics, in milliseconds:

| Remote roads / painted cells | Total main-thread edit work p50, before → after | Worst edit frame p95, before → after | Post-draw p95, before → after |
| --- | ---: | ---: | ---: |
| 0 / 0 | 2.281 → 0.543 | 2.707 → 0.701 | 16.760 → 33.416 |
| 8 / 1,536 | 2.304 → 0.552 | 2.552 → 0.699 | 16.725 → 33.390 |
| 32 / 6,144 | 2.344 → 0.545 | 3.129 → 0.663 | 16.718 → 33.419 |
| 128 / 24,576 | 2.331 → 0.557 | 2.943 → 0.547 | 16.695 → 33.402 |

Total main-thread edit work falls **76–77%**. Typical settling increases from one frame to two.
Both builds pass every fixed budget in all twelve cases; the baseline's earlier sporadic frame
tails are not reproduced consistently here, so this does not establish their sole cause.
Maximum candidate trial p95 is **2.190 ms** for an edit frame, **19.072 ms** until versions match,
and **34.013 ms** through post-draw. Idle p95 reaches 0.269 ms and GPU p95 0.027 ms. Individual
candidate post-draw samples reach **83.108 ms**, versus 49.888 ms before: the 50 ms acceptance
ceiling is a p95 limit, not a worst-case guarantee. All **4,848** measured operations preserve
exact local geometry, idle zero uploads and the one-chunk / 28,224 packed-byte / 16,128 surface-byte
edit replacement. Every sample renders five draw calls and 1,342 primitives. These backgrounds
contain no native buildings or agents; dense development and whole-gameplay frames remain open.

Artifacts are `/tmp/zone04-async-cells/`: `library-tests.log`, `bridge-results.json`, `rustdoc.log`,
`measure.py`, `measurement-metadata.json`, `summary.json`, `{before,after}-{1,2,3}/results.json`,
and `rendered/{manifest.json,comparison.json,*.png}`. All six timing processes and the reference
process exit normally. This does not prove the earlier intermittent shutdown issue fixed.
Source identities stay unchanged during measurement; the candidate library is restored and
verified afterward. Candidate extension SHA-256 is
`4ce88ce15dd941508d4170e990ea66f5fbac21967697130fac42131cbbf7d90a`; baseline is
`a4a882469725d7e1906bafa6791b717c555ca2de52b57db21254f448e0017050`. The test executable is
`90adc4ebcdf640e7a1fc9fad29ecfcd6782a51f7b8e972e6ff5a2573e4186627`.

Native node-validation and whole-edit locality measurements above belong to the preceding
build and are not rerun for this bridge scheduling change. Their unresolved gates remain open.
The new bounded acknowledgment allocation has not received a calibrated heap measurement;
previous native store/paint heap results do not cover it. Remaining edit-path, memory and broader
renderer acceptance also remain open. `ZONE-04` is still in progress.

### Shared road owner indices

Road surface, earthwork and fine query chunk-to-owner maps now use `imbl::HashMap`, with
`Arc<BTreeSet<_>>` values preserving sorted owner traversal. The six existing indices retain
their chunk sizes and membership rules. Their snapshot clones share roots in O(1), while writes
copy only affected trie paths and owner sets. Hash-trie lookup/update is expected O(log N) in
indexed chunks; copying a touched owner set is O(K) in its owners. No second spatial structure
is introduced. Other graph and surface snapshot fields still copy their stored contents, so
the complete snapshot has not yet achieved local cost.

The new dependency is justified by a populated-fixture diagnostic: at 100,000 distant buildings,
100,000 distant painted cells, 100,004 parcels, 392 roads and 600,024 agents, road-surface cloning
accounts for 2.780 ms of a 2.891 ms snapshot median. Graph cloning is 0.096 ms; terrain and water
are below 1 microsecond each in this flat fixture. The
[upstream structural-sharing contract](https://docs.rs/imbl/7.0.0/imbl/)
allows branch-local mutation without shifting a complete map copy onto the next write.
The lockfile selects 7.0.2 and its default thread-safe Arc storage. The project adds no unsafe
blocks and retains the existing compiler and spatial query APIs.

A regression retains a published index while inserting/removing owners and clearing its live
successor. Old memberships and ordering remain unchanged, empty live entries disappear, and
untouched owner sets remain pointer-identical. An isolated diagnostic separately measures clone,
edit-with-snapshot, release and local lookup. It confirms the tradeoff: structural sharing makes
cloning cheap but individual map lookups slower. Matched production query and edit measurements
must establish the practical effect; the map diagnostic is not whole-gameplay acceptance.

Fresh release verification passes **1,997 library tests**, with 79 manual fixtures ignored,
all three headless bridge suites and warning-free Rustdoc. One calibrated native heap repeat
passes all **56 phases** with exactly unchanged event counts, growth, peak and retention versus
the earlier road-footprint checkpoint. This covers native cell operations, not owner-index
construction or snapshot retention. The first trace aborted at a temporary-artifact storage
quota and is excluded; the complete retry is under `memory-retry/`. Byte-identical retained
binaries were consolidated into hard links to free space without changing their contents.

Three matched unprofiled native pairs use CPU 0 and one Rayon worker, reversing order in pair 2.
No build, test, profiling or renderer work runs concurrently. All local products remain identical
through 600,024 agents. Median trial statistics, in milliseconds:

| Background buildings / painted cells | One snapshot per edit, before → after | Road-worker p50, before → after | Readiness p95, before → after |
| --- | ---: | ---: | ---: |
| 0 | 0.007503 → 0.006392 | 34.848 → 34.908 | 0.165697 → 0.167271 |
| 1,000 | 0.058779 → 0.026606 | 34.744 → 34.717 | 0.166136 → 0.161876 |
| 10,000 | 0.388391 → 0.137000 | 34.834 → 34.662 | 0.169834 → 0.167504 |
| 100,000 | 3.556706 → 1.291348 | 34.717 → 35.293 | 0.175049 → 0.175801 |

The largest snapshot median falls **63.7%**; it still grows with the background and is reported
separately from worker work. Largest-background worker median increases **1.7%**. Maximum
readiness p95 is 0.181870 → **0.178001 ms**, inside 0.300 ms. Clear node-validation maximum p95
is 7.078142 → **7.147682 ms**, still above 5 ms. Its largest-background median is 6.641 → 6.652 ms.
These measurements establish a snapshot improvement, not full edit-path or node-latency acceptance.

Three additional matched production height-query pairs use the existing graded-yard fixture,
76 points, ten batches of 100,000 queries, CPU 0 and one Rayon worker. Median trial p50 is
**515.7 → 527.6 ns** (2.3% higher); median trial p90 is 518.4 → 529.3 ns. In the isolated map
diagnostic, cloning 100,004 entries falls from 7.040 ms to 42 ns, but a local edit with a retained
snapshot rises from 289 to 2,664 ns and local lookup from 6.99 to 22.08 ns. That diagnostic uses
101 samples after five warmups, alternates map order per sample, and measures release separately.
It is one run of the collection operations, not a substitute for the matched production fixtures.

Three clean renderer pairs retain the previous fixture and fixed budgets: one Rayon worker,
unrestricted CPU affinity, 60 Hz effective cap, Godot 4.7.2 Forward+ and RX 7900 XTX. Accepted
pairs are **1, 2 and 4**, with pair 2 reversed. Pair 3 is retained but excluded because diagnostic
binary hashing overlapped its candidate run; a fresh complete pair replaces it. Both builds pass
all gates in the accepted pairs. Maximum candidate trial p95 is **2.345 ms** for edit frames,
**0.270 ms** idle, **18.285 ms** to matching versions and **33.479 ms** through post-draw; GPU
p95 is 0.027 ms. Baseline maxima are 2.287 / 0.267 / 18.281 / 33.820 ms and 0.037 ms GPU.
Median trial total edit work remains 0.542–0.559 ms before and 0.544–0.562 ms after. Raw post-draw
maxima reach 83.275 / 83.174 ms; the 50 ms ceiling remains a p95 limit. All **4,848 measured
operations** retain identical local products, zero idle uploads, one edited chunk and unchanged
28,224-byte packed / 16,128-byte surface-buffer replacements. This is not dense or full-gameplay
renderer acceptance.

Eight fresh PNGs and their complete manifest are byte-identical to the queued-preparation
checkpoint. The reference process exits normally. Baseline renderer trial 4 is interrupted only
after complete results and its final pass line; the previously observed shutdown wait remains
unresolved. All other renderer processes exit normally. Each process is terminal before another
timed process starts, and candidate deployment is restored and verified afterward.

Evidence is under `/tmp/zone04-snapshot/`: `library-tests.log`, `bridge-results.json`, `rustdoc.log`,
`components-{before,after}.log`, `diagnostic-metadata.json`, `index_bench.rs`, `index-bench.jsonl`,
`height-{before,after}-{1,2,3}.log`, `height-metadata.json`, `native/{metadata.json,results.json,summary.json}`,
`memory-retry/{metadata.json,results.json,comparison.json,*.data.gz}`, `measurement-{metadata,extra}.json`,
`trial-selection.json`, `summary.json`, and `rendered/{manifest.json,comparison.json,*.png}`.
Native runners and both renderer runners record commands and source/binary identities. Candidate
extension SHA-256 is `77a35bc00e2d251f083f5ea6d01a52cd240b4ac8228ea30872e04d3df274071c`; baseline
is `4ce88ce15dd941508d4170e990ea66f5fbac21967697130fac42131cbbf7d90a`. Candidate test executable
SHA-256 is `2aafceef70eeaefd714729a85b71ba7bcc3f337bb6fcf74c364c1a274fe9ea14`; the baseline,
including the same snapshot diagnostic, is `82266a600a3680537fcd5cce40f71fea481f51f6d192beb2a97e4ab292aed146`.

Whole-edit locality, node-validation latency, remaining edit-path coverage and broader
memory/rendering acceptance remain open. The six owner indices no longer require full snapshot
copies, but the rest of the snapshot still does. `ZONE-04` remains in progress.

### Shared compiler inputs and reverse coverage

The next snapshot diagnostic separates the remaining surface fields. At 100,000 background
buildings / painted cells, cloning cached node inputs takes 366.263 µs median; the six reverse
owner-to-chunk maps total 84.876 µs. Complete surface cloning is 534.907 µs and graph cloning
54.454 µs. These individually warmed field measurements identify copying costs; their sum is
not a replacement for measuring the actual snapshot.

Cached node inputs and the six reverse coverage maps now use the existing `imbl` dependency
with immutable `Arc` records. Snapshots share map roots in O(1); replacing or removing a record
copies O(log N) map structure without cloning unrelated input polygons or chunk lists. Coverage
generation retains its existing local geometry work. Preview certificates and bounded undo
entries share the same immutable node inputs. Reverse coverage insertion no longer clones a
chunk vector just to return an unused result; removal of an absent record uses an empty slice
without allocating a placeholder. Hot compiled-geometry maps retain their existing storage.

A regression keeps a published surface alive through local road recompilation, compiler undo,
and clearing the live surface. It verifies unchanged published inputs and coverage, updated
local records, shared distant records and exact input restoration through undo. The existing
failed-compile and shrinking-coverage regressions retain their assertions with shared records.
No spatial index, dependency, save format or geometry predicate is added by this change.

Fresh release verification passes **1,998 library tests**, with 79 manual fixtures ignored.
The detailed snapshot diagnostic uses three warmups and 21 samples, CPU 0 and one Rayon worker,
with setup and clone release excluded from each component timer. At the largest background,
surface clone median falls 534.907 → 60.853 µs and the complete warmed snapshot falls
592.674 → 118.649 µs. Snapshot release falls 433.904 → 68.341 µs. Each changed map's warmed
clone takes 0.014–0.015 µs. This is a component diagnostic; matched once-per-edit timings below
determine the practical improvement. Remaining graph and surface fields still copy globally.

Three matched unprofiled native pairs use CPU 0 and one Rayon worker, reversing order in pair 2.
No build, tests, profiling or renderer runs concurrently with the timings. All 72 records retain
identical local products through 100,000 distant buildings, 100,000 distant painted cells,
100,004 parcels, 392 roads and 600,024 agents. Median trial statistics, in milliseconds:

| Background buildings / painted cells | One snapshot per edit, before → after | Road-worker p50, before → after | Readiness p95, before → after |
| --- | ---: | ---: | ---: |
| 0 | 0.005095 → 0.004304 | 34.815 → 35.048 | 0.156162 → 0.157500 |
| 1,000 | 0.026219 → 0.012784 | 34.852 → 34.564 | 0.159083 → 0.149647 |
| 10,000 | 0.137650 → 0.045317 | 34.954 → 34.713 | 0.165178 → 0.159469 |
| 100,000 | 1.251914 → 0.354420 | 34.952 → 34.622 | 0.156209 → 0.159931 |

The largest snapshot median falls **71.7%**. Worker medians range from 0.7% higher to 0.9% lower;
these small changes do not establish a worker-speed improvement. Maximum readiness p95 falls
0.229719 → **0.165840 ms**, inside 0.300 ms. Clear node-validation maximum p95 is
7.454202 → **6.994564 ms**, still above its 5 ms budget. Snapshot cost still follows the background;
neither whole-edit locality nor node-latency acceptance is complete.

Three further matched height-query pairs use the same graded-yard fixture, 76 points and ten
batches of 100,000 queries, CPU 0 and one Rayon worker. Median trial p50 is 524.2 → 519.7 ns;
median trial p90 is 525.2 → 521.7 ns. This finds no height-query regression in that fixture and
does not replace dense/full-gameplay query acceptance. Changed map construction and snapshot
retention have not received calibrated heap measurements in this checkpoint; prior cell-store
heap results do not cover them.

The rebuilt extension passes all three headless bridge suites and warning-free Rustdoc.
Eight GPU reference PNGs and their complete manifest are byte-identical to the preceding
checkpoint; the reference process exits normally. This verifies geometry and tool integration.
The preceding renderer latency and heap runs remain evidence for their original binaries,
not fresh performance measurements of this build.

Artifacts are under `/tmp/zone04-surface-snapshot/`: `library-tests.log`, `check-fixed.log`,
`build.log`, `rustdoc.log`, `bridge-results.json`, `components-{before,after}.log`,
`diagnostic-metadata.json`, `native/{measure.py,metadata.json,results.json,summary.txt}`,
`height.py`, `height-metadata.json`, `height-{before,after}-{1,2,3}.log`, `summary.json`,
`reference-metadata.json`, and `rendered/{manifest.json,comparison.json,*.png}`. Metadata records
commands, source identities, executable hashes, CPU affinity and worker settings. The native
runner verifies all measured source and executable identities after the final pair. Component
diagnostics run `populated_cell_snapshot_scaling`; native pairs run
`populated_cell_node_move_validation_scaling` and `populated_cell_road_plan_scaling`; the query
fixture is `graded_yard_height_query_benchmark`, all through the release library test executables
with `--exact --ignored --nocapture --test-threads=1`, `METRUM_DEBUG=0`, `RAYON_NUM_THREADS=1`
and `taskset -c 0`.

Candidate extension SHA-256 is
`4c369ba37de4d5e9fc4929ae9ea943765e81bd72d6526319dfa365974477ad2a`; baseline is
`77a35bc00e2d251f083f5ea6d01a52cd240b4ac8228ea30872e04d3df274071c`. Candidate test executable
SHA-256 is `94e382d9da7664cce88f5c854697d9963142a13b7a1e206288810550a7297037`; baseline,
including the same detailed clone diagnostic, is
`69bcdcaf0b58903e7837b12a45ce72ba0d4502bac02e8cac169be3fee760881f`.
Whole-edit locality, node-validation latency, remaining edit-path coverage and broader
memory/rendering acceptance remain open. `ZONE-04` stays in progress.

### Field and parcel edit invalidation

Two native regressions reproduced stale cell caches after field commits and farm removal.
Field clearance was authoritative for new placement, but an already generated chunk could
still expose empty cells inside a new field, or retain missing cells after the farm was removed.
Building-lot invalidation did not cover the field extending beyond that lot.

Successful field commits now invalidate both previous and accepted bounds through the existing
cell chunk and lot-work queue. Allocator removal publication reads the removed field polygon
before its agriculture record is deleted or remapped; demolition undo invalidates the restored
field. Invalidation visits O(K) affected 512 m chunks after O(V) polygon bounds extraction,
with no background cell, road or parcel scan. Existing generation performs the subsequent
local reservation checks. No dependency, index or save-format change is introduced.

A farm whose storage slot moves during deletion keeps the same field polygon. Its distant
cached grid remains unchanged during removal and undo; undo no longer allocates a bounds vector
or invalidates that unchanged field's vegetation. If that remapped farm's field is subsequently
resized, the older demolition undo is rejected before mutation instead of overwriting its newer
polygon. That check compares the captured field to the current owner in O(log F + V), for F
field records and V vertices of the moved field. It runs only on the undo command.

Coverage exercises initial commit, shrinking, rejection over newly painted cells, stale paint
preview rejection, farm removal/undo beyond its footprint, a distant remapped farm and stale
demolition undo after a later resize. A separate regression exercises authored-parcel attachment
repair: painted destination cells reject the move atomically; erasing that paint permits repair,
releases old cells and suppresses new cells; repair back restores the original footprint.
That parcel path already used the shared reservation and invalidation rules and needed no
production change in this checkpoint.

Fresh release verification passes **2,002 library tests**, with 80 manual fixtures ignored.
The rebuilt extension passes `zoning_cells_tool_test`, `zoning_road_tool_test`,
`road_junction_preview_test` and `field_edit_tool_test`. The field tool suite controls simulation
responses; the native Rust regressions above exercise actual geometry, reservations and undo.
Rustdoc is warning-free, and formatting and diff checks pass.

Three unprofiled release trials isolate field commit and cached-cell refresh. Each keeps one
1 km road and farm fixed, alternating 400 × 500 m and 40 × 500 m fields while retained distant
paint grows to 100,000 cells across 60 chunks. There are three warmup cycles and 21 measured
cycles, yielding 42 commit and 42 refresh samples per background. Polygon argument cloning,
fixture construction and result verification are outside the timers. CPU 0, one Rayon worker
and `METRUM_DEBUG=0` are fixed; no other agent build, test or profiling runs overlap the timings.
Median trial statistics:

| Distant painted cells | Commit p50 (µs) | Grid refresh p50 (ms) | Grid refresh p95 (ms) |
| ---: | ---: | ---: | ---: |
| 0 | 16.102 | 2.039 | 2.394 |
| 1,000 | 15.942 | 2.036 | 2.403 |
| 10,000 | 17.869 | 2.044 | 2.405 |
| 100,000 | 15.300 | 2.048 | 2.391 |

All twelve trial/background records retain identical local cells and unchanged distant paint,
chunk versions and completeness. Maximum trial commit p95 is 22.054 µs; refresh p95 is
2.436357 ms. These are candidate costs, not a before/after speedup: the preceding implementation
did not perform the required cache refresh. Background fields, roads, buildings and agents do
not grow in this fixture, so it does not certify complete field lifecycle or whole-city locality.
No new heap, renderer latency or reference-image measurements were run in this checkpoint;
earlier results remain evidence for their original binaries.

Artifacts are under `/tmp/zone04-field-reservations/`: `regressions-before.log`,
`release-tests.log`, `build.log`, `rustdoc.log`, `build-metadata.json`, `bridge-results.json`,
`measurement-metadata.json`, `scaling-{1,2,3}.log`, `scaling-results.json`,
`scaling-summary.json`, `after-tests` and `after.so`. Build commands are
`cargo test --release --lib`, `cargo build --release` and `cargo doc --no-deps`, from `rust/`.
Run `python3 /tmp/zone04-field-reservations/measure.py` to repeat the three cost trials; it calls
`nodes::sim::core::tests::fields::field_cell_cache_edit_scaling` through the retained release
test executable with `--exact --ignored --nocapture --test-threads=1`, `taskset -c 0`,
`RAYON_NUM_THREADS=1` and `METRUM_DEBUG=0`. Metadata verifies source and executable identities
before and after measurement. Extension SHA-256 is
`1cff42919bb64af6c3f22272a46915dbbe1b5dec565c2a90199f935a79ad5cfb`; test executable SHA-256 is
`9fa28cf6b2becf1f45ac3c8a56d33d9b0f38a9002dde3a164ff8c808f8188a50`.

Whole-edit locality, node-validation latency, remaining edit-path coverage and broader
memory/rendering acceptance remain open. `ZONE-04` stays in progress.

### Reservation checks before demolition undo

Two further native regressions failed on the preceding build: placing a manual parcel after
farm demolition, or expanding a different farm's field, still allowed the removed field to be
restored over the newer reservation. Those edits need not change building identities or add an
undo entry, so the existing structural and remapped-owner checks did not cover them.

The inverse now checks the removed building lot against indexed fields and, when it has no
parcel owner, against both zoning reservation types. The removed field polygon also checks
current field and zoning reservations. Existing cell-lot ownership checks remain in
place. No state is changed on rejection, and the same journal can restore the building and
field after the conflicting reservation is released. The tests cover zoned and profile-zero
manual parcels inside the former field and inside the farm lot alone, plus a later expansion
by a farm outside the deletion's swap pair. Existing valid demolition undo continues to pass.

The additional work is O(J + V + K) plus exact polygon overlap work on local candidates, for
captured journal records J, at most two footprint contours with V vertices, and queried local
index references K. It uses the existing field, parcel and cell indices without a city scan or
building-index rebuild. Temporary prepared footprints belong to the undo command; this is not
a per-agent or idle-tick path. No dependency, spatial index or save schema is added.

Fresh verification passes **2,004 release library tests**, with 81 manual fixtures ignored,
all four headless suites from the preceding checkpoint, warning-free Rustdoc, formatting and
diff checks. The release extension is deployed locally.

Three unprofiled release trials measure rejected undo commands while one farm and road remain
fixed. Isolated distant cell paint and indexed field footprints grow together from zero to
100,000 each. Local blockers are a field, a profile-zero authored parcel, or painted cells
without a derived lot. For each case, three warmup batches precede 21 measured batches of 64
calls. Setup, snapshot creation and state comparisons are excluded. CPU 0, one Rayon worker and
`METRUM_DEBUG=0` are fixed, without overlapping build/test/profiling work. Median of trial
medians, in microseconds per rejected command:

| Distant cells / field footprints, each | Field conflict | Parcel conflict | Paint conflict |
| ---: | ---: | ---: | ---: |
| 0 | 0.728 | 0.968 | 0.927 |
| 1,000 | 0.849 | 1.158 | 1.038 |
| 10,000 | 0.854 | 1.163 | 1.028 |
| 100,000 | 0.848 | 1.161 | 1.035 |

All 36 records preserve paint, dependency revisions, treasury, building/field authority and
the inverse journal. Each trial also restores successfully after releasing its final blocker.
The maximum p95 of batch means is 2.615 µs; this is not an individual-command tail percentile.
These are corrected-build costs, not a before/after speedup: the old behavior accepted an
invalid restoration. The fixture isolates reservation indices and does not grow running farms,
background building/agent state or authored parcels. It therefore does not establish complete
undo locality or a populated city's full restoration cost. Heap, GPU latency and reference
captures were not rerun; earlier measurements retain their original scope and build identity.

Artifacts are under `/tmp/zone04-restore-reservations/`: `regressions-before.log`,
`check-fixed.log`, `release-tests.log`, `build.log`, `rustdoc.log`, `build-metadata.json`,
`bridge-results.json`, `measurement-metadata.json`, `scaling-{1,2,3}.log`,
`scaling-results.json`, `scaling-summary.json` and retained binaries. Build commands remain
`cargo test --release --lib`, `cargo build --release` and `cargo doc --no-deps`, from `rust/`.
`python3 /tmp/zone04-restore-reservations/measure.py` repeats the three trials by running
`nodes::sim::core::tests::fields::undo_scaling::rejected_field_undo_reservation_scaling` through
`after-tests --exact --ignored --nocapture --test-threads=1`, with `taskset -c 0` and the worker
settings above. Source and executable identities are checked before and after the measurements.
Extension SHA-256 is `7f0d0d1602c218e670e34e98ab3b041c9be0a655110dbd0e9a9dcd96400a121a`;
test executable SHA-256 is `845971f2539525d24eb0b91345543b6fca7f190819c9b6580d44c85aff0165c0`.
Whole-edit locality, node-validation latency, remaining edit-path coverage and broader
memory/rendering acceptance remain open. `ZONE-04` stays in progress.

### Feature handoff — `ZONE-04`

The requested feature is implemented and handed off on **2026-09-28**. Road-generated cells,
compatible orthogonal alignment, non-overlapping curved/angled grids, preservation of existing
paint/buildings, all four selectors and separate Erase are connected to gameplay. Authored parcels
remain an alternative in the same city. Shared reservations, derived building lots, demand,
redevelopment, road/terrain/field edits, undo and mixed-workflow save/load are integrated.

In the zoning toolbar, choose **Cells**, choose a profile, then select **Cell**, **Marquee**,
**Fill** or **Brush**. **Erase** uses the selected shape; selecting a profile leaves Erase.
Fill follows connected cells with the seed's current profile on the same grid. Choose **Parcels**
to use the existing workflow. Switching workflows or settings cancels a pending gesture.

Final verification passes **2,004 release library tests** (81 manual fixtures ignored),
`zoning_cells_tool_test`, `zoning_road_tool_test`, `road_junction_preview_test` and
`field_edit_tool_test`, plus warning-free Rustdoc and formatting/diff checks. Eight fresh rendered
layouts pass, including the seven reference arrangements and a rotated block. The mixed-paint
capture was also inspected. The release extension is deployed at `godot/bin/libmetrum_rise.so`.

An experimental polygon-crossing index was removed before handoff: its isolated large-ring
gain did not establish a useful improvement in the populated node-edit check, and blocked moves
cost more. The final Rust/Cargo sources match `source-before.json` in
`/tmp/zone04-node-cost/`; executable and extension hashes are those in the preceding checkpoint.
Fresh final evidence is `final-release-tests.log`, `final/bridge-results.json`,
`final/rustdoc.log`, `rendered-before/{manifest.json,*.png}` and `reference-before-metadata.json`.
The `before` name identifies the retained implementation; `after` artifacts describe the
discarded experiment. The final suite ran the retained release executable with `METRUM_DEBUG=0`
and `RAYON_NUM_THREADS=1`; source hashes were checked after restoring the two experimental files.
Godot checks use the existing scripts and the deployed extension. Rendered checks required
desktop access because the sandbox could not connect to the display; they then exited normally.

### Performance measurement scope

The feature handoff does not certify the outstanding performance budgets:

- Clear road-node validation remains above its **5 ms** component target. The final retained
  build's locality check reports p95 **6.868 ms** at 100,000 distant buildings/cells and
  **7.062 ms** maximum across backgrounds. Local compiled products still match through
  600,024 agents; one final unprofiled check is not a new performance-certification campaign.
- Complete road-edit/removal locality still includes global snapshot storage outside the
  improved cell queries and shared surface maps. Snapshot cost is separate from local work.
- Allocation/retention acceptance for shared maps, queued chunk preparation and the renderer,
  plus dense local conflicts and large fills, is not established by this handoff. Existing
  heap measurements retain their original build and fixture scope.
- Broader external edit-path coverage and matched gameplay rendering acceptance remain open.
  The completed reference captures establish geometry, not whole-city frame-time budgets.

Historical `ZONE-04` progress notes above describe their checkpoints; this handoff is the
current status.

### Post-handoff paint/layout correction — 2026-09-28

Historical first correction: the whole-grid priority described below was subsequently rejected
in gameplay testing and removed in the frontage-priority follow-up. Its tests and timings apply
only to that earlier build.

Testing exposed a WYSIWYG failure at almost perpendicular roads: painting promoted a whole
retained strip ahead of the empty candidates, filling gaps and moving the visible grid boundary.
Generation now resolves canonical frames before depth rows and merges retained copies after
current-road candidates establish ownership. Empty overlaps therefore show the winning lattice
before painting; paint/erase does not promote an unchanged strip. Historical geometry and
occupied cells still retain their road-edit protection. No geometry tolerance or road alignment
rule changes. Candidate sorting remains O(C log C), with the same bounded spatial conflict
queries and no additional index or allocation pass.

The new regression first reproduced the paint-induced layout change on the preceding code.
It now checks complete empty corner coverage, identical cell addresses after painting each
frame and erasing, and disjoint cells at three small angular offsets. All 69 cell-store tests
pass, including partial invalidation and straight/curved road-edit preservation.
The complete release library suite passes 2,005 tests, with 81 manual fixtures ignored.
The rebuilt extension passes `zoning_cells_tool_test` and `zoning_road_tool_test` headlessly
with isolated user data and is deployed at `godot/bin/libmetrum_rise.so`. Running games must
restart to load it; the reported gameplay corner still needs the user's visual retest.

Matched unprofiled release measurements use the i9-12900K, Rust 1.98.1, one Rayon worker and
`METRUM_DEBUG=0`; setup is excluded. Three interleaved CPU-0-pinned trials of
`benchmark_cell_zoning_locality --ignored --nocapture --test-threads=1` give median generation
times in microseconds:

| Distant roads / cells | Before | After |
| --- | ---: | ---: |
| 0 / 0 | 86.903 | 85.339 |
| 1,000 / 16,384 | 89.499 | 89.822 |
| 10,000 / 131,072 | 90.382 | 90.374 |

Local products remain 144 cells at every background. Three interleaved trials of
`benchmark_cell_curve_frontage_locality` with the same flags, without CPU pinning, measure
3.372 → 3.345, 3.406 → 3.378 and 3.368 → 3.444 ms at 0/128/1,024 distant curves.
The fixed local curve retains 18 occupied lots; its resolved coverage increases from 232 to
252 cells. Each build preserves identical local products as background occupancy increases
to 5,138 lots. These measurements establish local generation cost, not whole-city rendering.

Evidence is in `rust/target/zone04-wysiwyg/`: `before.sha256` identifies the retained baseline
executable built from `a9958afd76c5b5dcfa272ac688eb47c590d96c19` plus the failing regression;
`after.sha256` records the corrected executable and changed Rust sources. `pinned-{before,after}-*.log`
and `curve-{before,after}-*.log` hold acceptance timings; earlier unpinned straight-grid trials
are retained separately. `cell-tests.log`, `full-tests.log` and `build.log` record validation.
The two named bridge logs and `deployed.sha256` record the extension checks and deployment.

### Frontage-priority follow-up — 2026-09-28

Gameplay testing rejected whole-grid dominance because one lattice suppressed the neighboring
road's frontage. Candidate ownership again compares depth row before canonical frame identity.
Compatible grids still share cells, while incompatible grids retain their road alignment and
may leave gaps. Current-road candidates still establish ownership before retained duplicates,
preserving the paint/erase stability correction. Existing paint and building claims remain fixed.
No manual controls, save-format changes or alignment changes are introduced. Sorting remains
O(C log C) with unchanged local conflict queries and allocation structure.

The corner regression now requires front-row cells with the correct supplying road on both
sides of the overlap, as well as unchanged cell addresses after paint/erase at three angular
offsets. Follow-up verification artifacts are in `rust/target/zone04-frontage/`.
All 69 cell-store tests pass. Three matched, interleaved, unprofiled release trials use the
existing `benchmark_cell_zoning_locality` and `benchmark_cell_curve_frontage_locality` fixtures
with `--ignored --nocapture --test-threads=1`, CPU 0, `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`,
Rust 1.98.1 and the same i9-12900K. Setup remains outside the timers. Median trial times:

| Workload / distant background | Whole-grid priority | Restored depth priority |
| --- | ---: | ---: |
| Straight / 0 roads, 0 cells | 85.753 µs | 85.842 µs |
| Straight / 1,000 roads, 16,384 cells | 88.966 µs | 89.301 µs |
| Straight / 10,000 roads, 131,072 cells | 90.539 µs | 90.271 µs |
| Curve / 0 curves | 3.274 ms | 3.225 ms |
| Curve / 128 curves | 3.280 ms | 3.241 ms |
| Curve / 1,024 curves | 3.275 ms | 3.242 ms |

Both workloads retain identical local products as background size increases. Straight coverage
remains 144 cells. The curve fixture retains 18 occupied local lots; split/restored coverage
changes from 252/252 to 236/232 cells under the restored rule. The largest background retains
5,138 occupied lots. These are local generation measurements, not gameplay frame-time acceptance.
`before.sha256` identifies the preceding whole-grid-priority executable; `after.sha256` records
the corrected executable and changed sources. The named benchmark logs contain the results.
Fresh full verification passes 2,005 release library tests (81 manual fixtures ignored) and
both `zoning_cells_tool_test` and `zoning_road_tool_test` against the rebuilt extension, using
isolated Godot user data. The full suite ran the hashed release test executable directly;
`full-tests.log`, `build.log` and the named bridge logs record the checks. `deployed.sha256`
verifies the atomic deployment to `godot/bin/libmetrum_rise.so`; restart the game to load it.

### Unsupported rear-cell removal — 2026-09-28

Competing grids could leave empty cells behind a missing cell, with no rectangular connection
to road frontage. Generation now filters conflict survivors against up to four uninterrupted
same-frame columns of at most six cells. A column needs complete eligible frontage coverage;
split-road intervals combine in the same order as lot validation. Another road on a compatible
grid may supply a valid alternative column. Failed candidates are not reconsidered, and no
whole-grid dominance or flood fill is introduced.

The existing local priority table also holds survivor membership and four six-bit masks of
recorded supplier depths. Column checks skip unrelated directions, allocate nothing and run
with Rayon. For C local candidates and F frontage records, support adds
O(D(C + F)) expected work, with fixed D = 6, after existing conflict resolution. Generation
evaluates five extra rows plus their conflict ring. External site and parcel-removal
invalidation includes the same seven-cell halo, so rear cells update across chunk boundaries.
Saved paint is retained when support disappears. Claimed cells retain supplier metadata for
building grace and road-split repair; the lot lifecycle still owns claim release.

Regressions cover removal behind an excluded middle row, retained paint and restoration after
the exclusion is removed, rectangular frontage coverage at competing roads, and cold narrow
requests matching full-area generation. Existing paint/erase, partial-invalidation, split-road
and occupied-lot regressions remain part of acceptance. Evidence is recorded in
`rust/target/zone04-orphans/`.

All 71 cell-store tests pass. Three matched, interleaved, unprofiled release trials use the
existing `benchmark_cell_zoning_locality` and `benchmark_cell_curve_frontage_locality` fixtures
with `--ignored --nocapture --test-threads=1`, CPU 0, `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`,
Rust 1.98.1 and the i9-12900K. Fixture setup is excluded. Median trial times:

| Workload / distant background | Before | After |
| --- | ---: | ---: |
| Straight / 0 roads, 0 cells | 88.052 µs | 113.169 µs |
| Straight / 1,000 roads, 16,384 cells | 90.942 µs | 117.963 µs |
| Straight / 10,000 roads, 131,072 cells | 93.173 µs | 120.527 µs |
| Curve / 0 curves | 3.409 ms | 3.406 ms |
| Curve / 128 curves | 3.422 ms | 3.404 ms |
| Curve / 1,024 curves | 3.403 ms | 3.418 ms |

The new support predicate adds 25–27 µs (29–30%) to the small 144-cell straight fixture;
the occupied curve refresh is effectively unchanged. This is additional local generation
work, not a speedup. Both fixtures retain identical local products as background grows.
The curve keeps 18 occupied local lots; its split/restored coverage changes from 236/232
to 222/222 cells. Maximum background occupancy is 5,138 lots. These fixtures establish local
cost and locality, not whole-city rendering acceptance.

`before.sha256` identifies the baseline executable at `fcda7f72f6e4e9663fcbe6a9afae352fdcd28abd`;
`after.sha256` records the final test executable and changed Rust sources. The named benchmark
logs hold acceptance measurements. `initial-*` records the discarded all-direction search,
whose straight-grid overhead motivated the supplier-depth masks.
Fresh full verification passes 2,007 release library tests (81 manual fixtures ignored),
including occupied no-build road-split attachment repair. The hashed test executable ran
directly with `RAYON_NUM_THREADS=1` and `METRUM_DEBUG=0`. The rebuilt extension passes
`zoning_cells_tool_test` and `zoning_road_tool_test` headlessly with isolated user data.
`cell-tests.log`, `full-tests.log`, `build.log` and the named bridge logs record validation;
`deployed.sha256` verifies atomic deployment to `godot/bin/libmetrum_rise.so`. Restart the
game for visual retesting of the reported corner.

### Straight-road junction continuity — 2026-09-28

Adding a branch must retain the continuing road's uninterrupted backside grid. Previously,
phase selection compared both split children against new endpoint/junction anchors, which
could replace the parent's phase and leave a seam. Same-width subdivisions now inherit the
appropriate directed parent phase unless a perpendicular corner constrains that same curb.
Opposite-side anchors cannot move the backside grid. Same-side corner alignment remains
available; longer restored/merged roads use the normal phase selection so rejected edits do
not retain a staged junction's phase.

Partial frontage spans use shared endpoint coordinates in the canonical frame, with the same
source-precision bound as the coverage proof at cell endpoints. The straight terminal check
uses the remaining road length even when rounding leaves a tiny final group; an earlier group
cannot offer a whole extra column beyond the road. Curve group handling is unchanged.

The added regression checks unchanged backside cell identities across perpendicular/angled
branches, three road rotations, both edge directions and origin/translated coordinates. It
also bounds the original road's column count. Existing block-layout, no-build, occupied-lot,
save and rollback tests remain acceptance requirements. Work retains the existing local
O((K + A) log K + X) alignment bound and bounded endpoint coverage queries; no new index,
citywide traversal or per-agent/tick allocation is introduced.

Fresh verification passes all 2,008 release library tests (81 manual fixtures ignored),
including native orthogonal block layouts and rejected-edit rollback, plus both
`zoning_cells_tool_test` and `zoning_road_tool_test` headlessly. The release extension was
rebuilt and atomically deployed. Logs and executable/source/library hashes are under
`rust/target/zone04-junction/`; `regression.log` records the original failing junction test,
`full-tests.log` the final full run, and `deployed.sha256` the installed library identity.

Three interleaved matched, unprofiled release trials compare baseline
`e1e83dd3d2fb282c5692b9caf99165aee6c5a3b3` (`before-tests`, `before.sha256`) with the final test
executable (`after.sha256`). Commands use `RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0
<executable> <benchmark> --ignored --nocapture --test-threads=1`; benchmarks are
`benchmark_cell_zoning_locality` and `benchmark_cell_split_frontage_locality`. Rust 1.98.1,
i9-12900K; fixture setup excluded, no competing builds or test suites. Median trial times:

| Workload / distant background | Before | After |
| --- | ---: | ---: |
| Straight / 0 roads, 0 cells | 107.790 µs | 109.422 µs |
| Straight / 1,000 roads, 16,384 cells | 110.617 µs | 111.736 µs |
| Straight / 10,000 roads, 131,072 cells | 111.782 µs | 112.593 µs |
| Split / 0 roads/lots | 1,085.162 µs | 1,099.203 µs |
| Split / 1,024 roads/lots, 6,144 cells | 1,093.613 µs | 1,106.446 µs |
| Split / 10,000 roads/lots, 60,000 cells | 1,093.401 µs | 1,110.908 µs |
| Occupied split/restore / 0 roads/lots | 1,085.161 µs | 1,100.435 µs |
| Occupied split/restore / 1,024 roads/lots, 6,144 cells | 1,089.798 µs | 1,106.750 µs |
| Occupied split/restore / 10,000 roads/lots, 60,000 cells | 1,093.231 µs | 1,107.968 µs |

Products match before/after and remain constant as background grows. Straight generation
adds 0.8–1.6 µs and split refresh adds 13–18 µs (roughly 1–2%); cost remains local. These
fixtures measure generation and lot attachment refresh, not whole-frame rendering. Restart
the game and add a junction to a straight road for visual acceptance; stored alignments in
already-authored saves are retained rather than globally reset.


### Unified zoning-grid road snap — 2026-09-28

Added the Tool mode / Snapping panel and replaced the old Shift implementation with
one checkbox-controlled curb-aware snap path. The Shift shortcut is removed; road connections
remain active. The cursor reads an immutable persistent alignment map in the existing road-tool
snapshot. No simulation lock, new spatial index, or city-wide grid copy is added to pointer work.

Fresh verification on this build: **2,012 release Rust tests passed, 82 ignored**, plus
`network_tool_chunk_renderer_test`, `zoning_cells_tool_test`, and `zoning_road_tool_test`.
The rectangle regression checks all 144 interior cells at three rotations, three road widths,
and two cell sizes, including first-road snapping for the cardinal fixtures. Other new checks
cover differing reference/selected widths, branch attachment, immutable snapshots, and the
actual checkbox/Spline bridge behavior and independence from Shift. The options panel was rendered under Xvfb;
`rust/target/zoning-snap/ui-preview.png` records the layout. Release build and atomic deployment
completed; restart the game to load the updated extension.

Checkbox-only follow-up: removed the road tool's Shift key query and shortcut tooltip. A fresh
`network_tool_chunk_renderer_test` run passes, including Shift press/release leaving the checkbox
setting in control (`rust/target/zoning-snap/checkbox-only-test.log`). This GDScript-only change
leaves the Rust build and measured algorithms above unchanged; Rust tests and benchmarks were
not rerun for shortcut removal.

Matched unprofiled release measurements use Rust 1.98.1, i9-12900K, CPU 0 affinity,
`RAYON_NUM_THREADS=1`, and `METRUM_DEBUG=0`, without competing builds/tests. The baseline is
commit `5f6f7d47d51d18b8314b688a7659db668464d55b`; its test executable matches the previous
junction-fix artifact hash. Setup and background population are outside repeated query timings.
Commands (substitute the baseline or final executable for `$exe`):

```sh
RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 "$exe" benchmark_cell_split_frontage_locality --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 "$exe" benchmark_zoning_snap_locality --ignored --nocapture --test-threads=1
RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 "$exe" populated_cell_road_plan_scaling --ignored --nocapture --test-threads=1
```

Three interleaved baseline/final split trials and three new cursor-query trials give these
median-of-trial medians (microseconds):

| Workload | Background roads | Before | After |
|---|---:|---:|---:|
| New zoning cursor query | 0 | — | 0.059 |
| New zoning cursor query | 1,000 | — | 0.068 |
| New zoning cursor query | 10,000 | — | 0.074 |
| Split frontage refresh | 0 | 1089.886 | 1088.562 |
| Split frontage refresh | 1,024 | 1096.835 | 1098.014 |
| Split frontage refresh | 10,000 | 1100.533 | 1103.794 |
| Occupied split/restore | 0 | 1092.459 | 1084.610 |
| Occupied split/restore | 1,024 | 1098.712 | 1087.085 |
| Occupied split/restore | 10,000 | 1095.924 | 1091.123 |

The cursor checksum stays `(266240, 532480)` at every background size. Existing split/restore
products remain identical, with matching paint and lot assertions. The cursor figures measure
the added grid query only, not raycasting, rendering, or the complete Godot frame.

One matched populated-city run per build (100 measured samples per background size) also
kept identical local plan/terrain products. This fixture increases distant buildings, parcels,
painted cells, agents and roads while keeping the edited neighborhood fixed:

| Remote buildings / painted cells | Agents | Remote roads | Plan p50 before → after (ms) | Worker p50 before → after (ms) | One-time snapshot before → after (ms) |
|---:|---:|---:|---:|---:|---:|
| 0 | 24 | 0 | 33.940 → 34.525 | 34.600 → 35.183 | 0.004 → 0.004 |
| 1,000 | 6,024 | 4 | 33.654 → 34.168 | 34.511 → 35.080 | 0.015 → 0.012 |
| 10,000 | 60,024 | 40 | 34.075 → 34.309 | 35.026 → 35.276 | 0.057 → 0.050 |
| 100,000 | 600,024 | 391 | 34.488 → 34.851 | 35.434 → 35.674 | 0.357 → 0.399 |

The largest fixture has 100,004 parcels and 600,024 agents. Repeated planning remains local;
its small timing differences do not indicate a material regression. The separately reported
one-time snapshot retains existing background-dependent costs outside this change.

Artifacts: `rust/target/zoning-snap/` contains `final-tests.log`, `build.log`, the three
`*test.log` bridge reports, `split-{before,after}-{1,2,3}.log`, `cursor-{1,2,3}.log`,
`populated-{before,after}.log`, and `ui-preview.{log,png}`. `before.sha256`, `after.sha256`, and
`deployed.sha256` identify the baseline/final executables, changed Rust sources and matching
built/deployed libraries. The earlier `full-tests.log` predates the first-road length refinement;
`final-tests.log` is the acceptance run for the deployed build.


### Free-angle snapping and left-side options — 2026-09-28

Moved the road options panel to the left of road types. Zoning snap now captures an axis only
within 5 degrees and half a cell of lateral distance. Other angles follow the cursor exactly;
the distance cap prevents large sideways jumps on long strokes. The same rule applies to a
first road on open terrain. Checkbox-only control and width-aware spacing remain in place.
The change adds O(1) arithmetic to the existing local query and allocates no buffers.

Fresh release verification: **2,013 Rust tests passed, 82 ignored**, plus
`network_tool_chunk_renderer_test`. New coverage checks capture and release around all four
directions on cardinal and rotated grids, free angles from 6 to 84 degrees between axes,
long-stroke lateral limits, free drawing through the native cursor bridge, and the panel's
left-side ordering. Existing full-rectangle regressions still pass. The actual panel was
rendered and inspected under Xvfb. Release build and atomic library deployment completed.

Three interleaved, unprofiled before/after cursor benchmark trials used the previous deployed
checkbox-only build as baseline, Rust 1.98.1, CPU 0 on i9-12900K, `RAYON_NUM_THREADS=1`, and
`METRUM_DEBUG=0`; no other builds or test suites were running. Command:

```sh
RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 "$exe" benchmark_zoning_snap_locality --ignored --nocapture --test-threads=1
```

Median of trial medians, microseconds; fixture setup is excluded:

| Background roads | Before | After |
|---:|---:|---:|
| 0 | 0.063 | 0.063 |
| 1,000 | 0.069 | 0.073 |
| 10,000 | 0.074 | 0.077 |

All cursor checksums remain `(266240, 532480)` across background sizes. These figures cover
only the grid query, not the whole Godot frame. Artifacts are in
`rust/target/zoning-snap-angle/`: `tests.log`, `build.log`, `bridge.log`,
`ui-preview.{log,png}`, `cursor-{before,after}-{1,2,3}.log`, and baseline/final/deployed SHA-256
manifests. Earlier measurements above belong to their recorded builds.

### Straight-road endpoint continuation — 2026-09-28

Continuing a same-width straight road now inherits both existing curb phases when the new
segment meets the old segment end-to-end. Previously only subdivisions inherited the phase;
an extension starting between cell boundaries created a separate lattice and left rejected
partial columns at the join. Existing continuous-frontage coverage can now join the shared
cell across both suppliers. True bends and width changes still require their own geometry.
Longer overlapping restored/merged sources retain the existing phase recomputation contract,
so rollback cannot preserve a temporary junction's offset.

The change uses the existing local adjacency and collinearity proof with O(1) endpoint
interval comparisons; alignment remains O((K + A) log K + K log R + X), with no added
allocation or citywide query. A regression reproduces the original gap after extending a
113 m road, then checks both sides and every row across rotations and edge directions.
Native commit coverage also exercises compiled-road exclusions at the join.

Fresh verification passes all 2,016 release library tests (82 ignored), including both new
regressions and existing save/rollback/layout checks, plus `zoning_cells_tool_test` and
`zoning_road_tool_test` headlessly. The release extension is rebuilt and atomically deployed.
Logs and executable/source/deployed-library SHA-256 identities are under
`rust/target/zoning-continuation/`; `reproduction.log` records the original missing column.

Matched unprofiled release runs use the pre-fix executable in `before.sha256` and final
executable in `after.sha256`, Rust 1.98.1 on i9-12900K, with no competing builds/tests:
`RAYON_NUM_THREADS=1 METRUM_DEBUG=0 taskset -c 0 <executable> <fixture> --ignored --nocapture --test-threads=1`.
The existing `benchmark_cell_split_frontage_locality` measures split refresh at
1,176 → 1,210 / 1,200 → 1,205 / 1,210 → 1,208 µs with 0 / 1,024 / 10,000 distant roads
and lots (up to 60,000 reserved cells); occupied split/restore measures
1,174 → 1,198 / 1,219 → 1,203 / 1,232 → 1,200 µs. Products match across builds and sizes.

The `populated_cell_road_plan_scaling` fixture holds affected geometry fixed while growing
background buildings/painted cells to 100,000, roads to 391, parcels to 100,004 and agents to
600,024. Across 100 samples per size, worker p50 before → after is
39.416 → 38.099 / 38.466 → 38.093 / 39.066 → 37.997 / 38.641 → 39.411 ms at
0 / 1,000 / 10,000 / 100,000 background buildings. All local products match. Largest-case
compile p50 is 37.625 → 38.383 ms; separately measured one-time snapshot setup is
0.403 → 0.391 ms. Cost remains local; these single matched trials establish no speedup.
Raw results are `<fixture>-{before,after}.log` in the artifact directory above.

Restart the game and redraw the continuation for visual acceptance. Existing saved grid
choices remain retained; this change does not globally rephase already-authored roads.

### Road-commit cell overlay latency — 2026-09-30

After a road commit, the cells on both sides appeared visibly after the road. The new
[`zoning_road_commit_latency.gd`](../godot/tests/zoning_road_commit_latency.gd) fixture measured
the gap: cells reached the overlay **9–15 frames (≈170–300 ms)** after the road snapshot. Two
causes stacked:

- Every visible chunk's version included the global heightmap source/visual generations. Road
  earthwork resets visual terrain over whole road-surface chunks, so every commit bumped them and
  all visible chunks (16–24 in the fixture) re-exported at two uploads per frame.
- Invalidated chunks regenerated only on request: the overlay polled, the export queued one
  `PrepareCellChunk` command (one job in flight) and returned busy, and the chunk uploaded on a
  later frame. Each dirty chunk cost at least one round trip.

**Local height revisions.** `TerrainSystem` keeps a revision per world-zero-aligned 512 m region.
Every sample write records its exact changed rectangle, expanded by a 96 m draping margin plus one
terrain cell, into the regions it reaches. A centre-owned parcel reaches at most 72 m past its
chunk and a cell under 8 m. Covered writes are the grid-rect patch marker (`set_height`, brushes,
slope, region reset, visual-overlay restore), `set_visual_heights_at_grid_unmarked` and the dense
replacements. Whole-grid replacements and mark-all record a global epoch instead.
`SparseChunkGrid::copy_rect_from` now returns the bounds of cells whose value changed. It skips
unchanged partial chunks without copying them and still re-shares whole-chunk payloads. Resets of
identical samples record nothing. A query is O(1), and a write costs O(regions touched).

Chunk state polling now returns four values per chunk: cell revision, generated flag, parcel
revision and local height revision (previously five, with two global epochs).

**Eager preparation.** `CellStore` lists warm chunks whose generated state a local edit cleared.
Cold chunks and whole-world invalidations are never listed. The overlay reports visibility
changes via `set_zoning_cell_overlay_visible`, a queued command like the camera AABB. While the
overlay is shown, the simulation tick regenerates the listed chunks inside its existing lock,
before building the snapshot, so an edit's snapshot publishes with its cells ready. A hidden
overlay drains the list without work. Invalidations from building growth therefore cost nothing
while zoning is not displayed. Work is O(edited chunks) using the existing bounded dirty-region
generation; it adds no thread, index or saved state.

Trade-off: the road snapshot now waits for this preparation. With `METRUM_DEBUG_PERF=1`, the
fixture's commit ticks spent **0.5–12.6 ms (median ≈7 ms)** in `cell_prepare_ms`, which grows as
crossing roads accumulate in the dirty envelope. The lazy path performed the same generation
later, in separate lock-holding commands. The road becomes visible 1–3 frames after submit on
both builds; per-run medians differ by one frame in both directions (baseline run A: 1 frame,
candidate running run B: 3), so no road delay is resolved at 24 commits per run. Publishing the
road first would leave cells one frame behind again.

Matched runs: 8 roads per world (inside a chunk, across one boundary, across both), fresh world
per repetition, 3 repetitions per run, paused and running, 60 FPS cap, 4 Rayon workers, isolated
Godot profile. Headless has two runs per variant, alternating builds; windowed has one. Frames
count from the frame the road snapshot is observable to the frame after the last cell upload
(0 = same frame). Median run p50 / worst run p95 / maximum:

| Mode | Speed | Variant | Cell lag frames | Cell seen after road, ms | Changed chunks | Upload frames |
| --- | --- | --- | --- | --- | --- | --- |
| headless | paused | baseline | 11 / 13 / 15 | 200.0 / 233.5 / 266.7 | 16 / 16 / 16 | 9 / 10 / 10 |
| headless | paused | height revisions only | 3 / 6 / 7 | 66.7 / 116.7 / 133.4 | 2 / 4 / 4 | 2 / 5 / 6 |
| headless | paused | both changes | 0 / 1 / 7 | 16.7 / 34.7 / 133.5 | 2 / 4 / 4 | 1 / 2 / 4 |
| headless | running | baseline | 11 / 15 / 15 | 200.1 / 266.7 / 266.8 | 16 / 16 / 16 | 9 / 11 / 11 |
| headless | running | both changes | 0 / 1 / 2 | 16.8 / 33.5 / 49.9 | 2 / 4 / 4 | 1 / 2 / 2 |
| windowed | paused | baseline | 15 / 17 / 17 | 266.8 / 300.1 / 300.1 | 24 / 24 / 24 | 14 / 14 / 14 |
| windowed | paused | both changes | 0 / 1 / 2 | 16.7 / 33.4 / 50.1 | 2 / 4 / 4 | 1 / 2 / 3 |
| windowed | running | baseline | 15 / 16 / 17 | 266.6 / 283.5 / 300.0 | 24 / 24 / 24 | 14 / 14 / 14 |
| windowed | running | both changes | 0 / 1 / 1 | 16.7 / 33.4 / 33.4 | 2 / 4 / 4 | 1 / 2 / 2 |

"Cell seen" includes the one-frame observation offset, so 16.7 ms is the floor. Height-only
running and windowed rows (3 / 6 / 6–7 frames) are in `compare.txt`. The remaining
one-frame cases are commits that dirty four chunks, which drain at the unchanged two-upload cap.
Each changed chunk now uploads exactly once. Before, the lazy path could upload the same chunk
twice for one edit (two partial preparation steps).

Fresh verification: all **2,033** release library tests pass (85 ignored). New regressions cover:

- changed-bound reporting from `copy_rect_from`;
- per-chunk height revisions: interior writes, margin spill, unchanged resets, visual writes and
  epochs;
- hidden-versus-shown eager preparation after native road commits.

Headless `zoning_cells_tool_test`, `zoning_road_tool_test`, `field_edit_tool_test`,
`road_junction_preview_test` and `network_tool_chunk_renderer_test` pass.
`zoning_reference_test` fails `03_orthogonal_t` "uninterrupted backside" identically on the
HEAD baseline (the same 78 errors and per-layout cell counts). The failure predates this change
and remains open.

Paint edits also benefit, since a paint commit's warm chunks regenerate on the next tick.
The existing [`zoning_overlay_benchmark.gd`](../godot/tests/zoning_overlay_benchmark.gd) ran twice
per build: windowed Forward+, one Rayon worker, order reversed for the second pair. Median edit
settling stays at 2 frames and main-thread edit-frame p95 at 2.1–2.3 ms on both builds; idle
p95 stays ≤ 0.35 ms. p95 settling is 3 frames in 7 of 8 candidate cases and 4 of 8 baseline
cases. The baseline misses the 50 ms completion/presentation p95 budget in 4 of 8 cases, the
candidate in 1 of 8. The budget sits exactly at three 60 Hz frames, so single runs flip on one
frame. Results are in `overlay-compare.txt`. Edit replacements, packed bytes and resident
geometry are unchanged. Both builds sometimes hang in engine shutdown after printing results
(candidate run A, baseline run B); the processes were stopped. This matches the intermittent
shutdown hang recorded above and remains open.

Not measured: lock contention in a populated city (no loadable populated fixture) and
preparation cost in dense 512 m chunks. Cost is local to the edited chunks, but a dense downtown
chunk can hold the road snapshot longer than this fixture's 12.6 ms maximum.

Builds: baseline is HEAD `398f7bab` with extension SHA-256
`7a09021db5f8f2e54d521b6625dc0b3ae360c2c1ee4b2168e9aa6192bb31156e`. Height revisions only:
`93a5b1eebbee03f4427784ffec7489b3db0d8fce14cc41d4140678b70b612263`. Both changes:
`23cb570e059b1035ac719809d0223ba24678c4f4605ba605c0d7e53d053f6552`. The deployed build differs
only by rustfmt line moves: `1cc1b854accd2524b7f3d935288870438d2dc722c4856033bee6b4e6275672e0`,
rechecked with one headless run (`final-headless-a`: lag 0 / 1 / 4 paused, 0 / 1 / 2 running).
Machine: i9-12900K,
Godot 4.7.2. Isolated projects, logs, per-run JSON, `run.sh` and `summarize.py` / `compare.txt`
are under `rust/target/zone-latency/`. Reproduce a run with
`rust/target/zone-latency/artifacts/run.sh <baseline|height|candidate> <label>`; set
`GODOT_MODE=--windowed` for the windowed runs.
