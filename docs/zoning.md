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
