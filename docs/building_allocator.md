# Building Allocator — Implementation Reference

This document describes the current `BuildingAllocator` runtime, its authoritative responsibilities,
and its ownership boundaries with zoning, demand, entrance logic, and the asset pipeline.

Update this file when allocator behavior changes. Update [`project.md`](project.md) when the live
status or ownership map changes materially.

## Scope

`BuildingAllocator` is the live integration layer between:

- roadside building placement and removal
- road-edge attachment and frontage occupancy
- zoning and occupancy fit checks
- allocator-owned search indices and vacancy tracking
- derived building entrance-cache rebuilds
- execution of demand-owned growth decisions

This document owns the allocator-side contracts. It does not own:

- zoning profile design or paint-tool behavior in [`zoning.md`](zoning.md)
- demand formulas or growth-profile tuning in [`demand.md`](demand.md)
- exact entrance or trip movement behavior in [`entrance_and_exit.md`](entrance_and_exit.md)
- asset-authoring UI behavior in [`asset_editor.md`](asset_editor.md)
- building economy behavior in [`economy.md`](economy.md)
- terrain clipping, fixed-site support surfaces, or engineered-ground ownership in
  [`earthworks.md`](earthworks.md)

## Maintenance order and elapsed time

`BuildingAllocator::maintain(elapsed_days, ...)` currently runs in this order:

1. `cleanup_stale_buildings()`
2. `network.rebuild_pathing_if_dirty()`
3. `rebuild_entrance_cache()` when the cache is dirty or length-mismatched
4. `rebuild_building_site_clients()` when the site-client count differs from the building count
5. `rebuild_zone_index()` when indices are dirty
6. clear `dirty`

Daily settlement passes one elapsed day. Immediate no-building road edits pass zero: they remove
invalid attachments immediately without consuming unrelated buildings' rezoning grace. The first
incompatible-zone detection starts three full grace days; subsequent daily calls consume one each.
Restoring compatible zoning clears the pending state even when no day has elapsed. Missing assets,
parcels, deleted roads and roads that disallow buildings remain immediate-removal conditions.

Important ownership note:

- ordinary household admission runs after the hourly demand pass, outside allocator maintenance;
  the runtime executes the demand-owned count through
  `execute_demand_household_admission_with_preference(...)`
- private building spawn, despawn, upgrade, and downgrade also no longer depend on allocator-local
  heuristics; the live runtime consumes demand-owned hourly plans. Spawns are distributed into
  minute-queued actions; non-spawn actions execute immediately after the hourly pass.
- fresh-map startup no longer uses an allocator-owned founding placement exception; the live runtime
  now relies on the organic pioneer-demand bootstrap described in [`demand.md`](demand.md)
- midnight settlement/removals precede the hour-zero demand pass. Each hourly pass admits
  households, queues spawns, then executes non-spawn building actions and publishes pending site
  changes. Workplace assignment remains outside allocator ownership.

## Core Data Model

### `Building`

The authoritative placement-side fields on each building are:

- `zone_profile_runtime_id`
- `zone_type` (derived broad-family cache)
- `edge_idx`
- `side`
- `cell_x`
- `cell_y`
- `width_cells`
- `depth_cells`
- `center_x`, `center_y`
- `support_height_m`
- `facing_dir`
- `frontage_t`
- `asset_id`
- `level`

These fields define where the building is attached, how large its footprint is, and which asset and
growth tier it represents. `zone_profile_runtime_id` is the authoritative zoning-side legality key
that now saves and loads with each building. `zone_type` is intentionally retained as a derived
broad-family cache because allocator indices, vacancy tracking, and several hot-path consumers only
need the baseline residential/commercial/industrial family. Later systems such as entrance
planning, economy, and rendering consume these fields rather than inventing their own separate
placement truth.

`support_height_m` is the fixed placement support plane consumed by building rendering, entrance
derivation, and the engineered-ground building-site client that owns the required flat support
footprint.

`facing_dir` is the road-facing frontage direction: the direction the building's authored front
points in world space. It is the negative of the zoning parcel `normal`, because parcel `normal`
points from the road into the parcel while a building frontage faces back toward the road.

### `EdgeOccupancy`

`edge_occupancy: HashMap<usize, EdgeOccupancy>` tracks claimed frontage columns per road edge side.

- `cells_long`: number of frontage columns currently represented for that edge
- `left`: claimed leading columns for buildings on the left side
- `right`: claimed leading columns for buildings on the right side

This is a fast O(1) pre-check that rejects same-edge frontage conflicts before the more expensive
world-grid footprint tests run.

### Allocator-owned indices and caches

- `zone_index`: building indices grouped by `ZoneType`
- `vacancy_index` and `vacancy_pos`: O(1) resident-vacancy tracking
- `building_chunks`: coarse 512 m spatial index of building centers
- `entrances`: derived per-building entrance/access cache

`entrances` is allocator-owned derived data, but its detailed semantics belong to
[`entrance_and_exit.md`](entrance_and_exit.md).

## Removal Lifecycle

Building deletion must enter through allocator lifecycle helpers, not through direct vector
mutation. Player bulldoze uses `remove_building_for_bulldoze(...)`, which wraps the existing
private removal path so parcel occupancy is cleared, agents are evicted, households and logistics
invalidate building references, swap-removed building IDs are remapped through dependent stores,
site terrain bounds are marked dirty, and entrance caches are rebuilt.

Production-site remaps are published after ordinary cleanup as well as player demolition.
Field reservations follow swap-removal immediately and are restored with their saved polygons
on demolition undo (`ECON-07`; see [`economy.md`](economy.md#field-placement-and-editing-econ-07)).
The bulldoze command retains the actual footprint center for revalidation: an offset imported
building footprint need not contain the logical lot center.

## Build-Site Model

In the live runtime, a build site is one frontage-attached roadside candidate footprint. It is not a
free-floating parcel.

Baseline build-site identity:

- `edge_idx`: attached road edge
- `side`: one side of that edge
- `parcel_id`: authored parcel identity (`0` for explicit sites)
- `width_cells` and `depth_cells`: candidate footprint size

Placement parameters currently come from the building asset manifest:

- `zone_type`
- `lot_width_cells`
- `lot_depth_cells`
- `level`

Deterministic discovery order in the current allocator:

- ascending `edge_idx`
- side order `[1, -1]`
- remaining ties by frontage column, asset dimensions, profile and parcel ID

Rayon collection sorts by this key before demand consumes candidates. There is no founding-placement
exception; compatible assets are ranked deterministically within each parcel.

### Legality checks

`resolve_slot()` currently validates a candidate parcel in this order:

1. parent edge exists, is not deleted, and allows building spawn
2. the parcel is not already occupied
3. the parcel has a non-zero runtime `ZoneProfile`
4. that `ZoneProfile` accepts the candidate asset's baseline `zone_type`, `density`, and authored
   tag filters
5. the asset footprint fits inside the parcel frontage and depth

Candidate discovery also checks the shared site-support solution for every compatible initial
asset before deterministic hash ranking (commercial output priorities remain primary). A rejected
asset cannot suppress a different constructible variant. Both positive and negative solutions are
memoized by exact asset/parcel pose. Commit re-runs legality and support validation against current
inputs before claiming anything; queued actions are not permission to use stale geometry.

Zoning reserves the entire explicit-building lot, including land outside the imported structure's
flat support footprint. This check shares the existing lot query used by placement neighbors and
converts its cached cell radius using the live world's cell size. Surface-height, terrain and
bulldoze queries retain their support-footprint bounds. Both use the same building chunk index.

Committing placement appends the building and derived site, marks `dirty`, `dirty_index`,
`entrances_dirty` and the relevant `dirty_zones` entry, then claims parcel occupancy in `ZoningSystem`.

The committed building center is still `front_center + parcel.normal * depth / 2`. Only the
frontage orientation flips: `Building::facing_dir = -parcel.normal`. Runtime render and entrance
transforms then align the asset's authored `main` entrance `forward` vector to `facing_dir`.

### Flat Building-Site Handoff

Live building sites are engineered-ground clients, and the allocator remains the placement
authority:

- zoning parcels remain legal intent only and do not alter terrain
- allocator placement chooses/validates one flat `support_height_m` for the building and yard pad
- structural mesh support, entrance landings and the authored yard interior require flat ground;
  mesh support uses cached imported LOD0 bounds and the renderer's part transform, not a fixed pad radius;
  part yaw/scale/pivot and frontage basis are shared with rendering, using Godot's positive-Y yaw;
  paving support reserves a 2 m lot-edge strip for graded road/neighbor/terrain tie-ins, using the
  existing support hull and terrain compiler rather than an independent flat overlay
- allocator registers the site client at construction start, not after the construction animation
  finishes
- terrain clipping, CDT stitching, material-region rendering, and chunk-local patch rebuilds remain
  owned by the earthworks / terrain path

Zoned, service, and explicit industry placement share `prepare_site_support()` and
`place_building_instance()`: the former resolves and validates the road-attached plane; the latter
installs the site, bumps the building revision, invalidates derived indices/entrances, and records
terrain bounds in the allocator's shared pending-site outbox. Demand accounting no longer carries
a separate terrain-dirtiness result. Placement, level changes and removal own their invalidation;
the simulation consumes it through `publish_pending_building_site_changes()`.
Live site bounds and detached terrain-snapshot bounds use the same footprint/material union.
Fresh asset selection has one feasibility/ranking loop; commercial resource priorities filter that
loop rather than maintaining a separate placement implementation. Level upgrades and downgrades
also share one execution path, retaining downgrade-before-upgrade ordering.

Roadside height selection uses the deterministic rule in [`earthworks.md`](earthworks.md):
driveway anchors first, parcel frontage midpoint as the no-driveway fallback. This sets the preferred
height; a deterministic interval solver chooses the closest feasible level pad satisfying driveway
and perimeter grades. All current placement
modes require a road attachment and reject a missing road surface; explicit placement is not
permission to silently substitute source-terrain height. A future non-road mode needs its own
explicit eligibility contract. Frontage projection and sampling both parameterize attachments by
3D physical arc length, even though nearest-point selection happens in XZ.

Zoning legality/occupancy, service/industry eligibility and charging, and private construction
timers remain mode-specific. The private-building rise animation still lowers the model during
construction; it never changes the authoritative site plane or terrain geometry.

Site feasibility is shared with non-mutating zoning previews. Cache epochs cover asset registration,
source/visual terrain, road publication and building references. After an epoch change, existing
terrain chunk snapshots, road chunk owner identities and indexed nearby site snapshots distinguish
local changes from unrelated city edits. Failed/pending road compilation cannot reuse a verdict.
Cloned allocators start with an empty cache. Removed/occupied/reattached/rezoned parcel entries are
pruned during the existing hourly collection; cursor-only poses have a separate 256-entry-per-asset
cap. New assets invalidate old decisions without requiring save edits.
Replacing an asset removes its previous zone, density and upgrade-family index memberships before
registering the new manifest, so re-evaluation cannot discover stale classifications.

Complexity: the existing hourly Rayon pass remains O(P × A) for P parcels and A compatible assets;
unchanged feasibility lookup is O(1) per resolved asset/pose. Cold solves and changed-epoch dependency
checks visit only the existing bounded terrain/road/building indices near that lot. For K grading
rays and R sampled rings, the interval solver has at most O(KR) interval endpoints, with a conservative
O(K²R² log(KR)) bound; K follows the asset perimeter at the fixed sample spacing, and R is at most six.
It does not scale with the city. Cache storage follows evaluated parcel/asset poses and their local
dependencies, plus bounded cursor poses. Index preparation is maintained outside the parallel pass.

The allocator must reject placement instead of repairing geometry when:

- the selected road/driveway connection cannot be resolved
- another driveway's road connection differs from the primary road connection by more than `0.35 m`
- any driveway-to-pad connection exceeds the shared grade budget
- a touching placed building site differs from the chosen support plane by more than `0.10 m`

Existing placed sites are fixed clients. A new placement must not average, move, or regrade them.
In-place upgrades and downgrades also validate the target asset's footprint, driveway connections,
neighbor clearances and terrain tie-ins through the shared solver. Their preferred height is the
existing support height, and any solution requiring it to change is rejected before mutation.
This adds a bounded site solve per attempted level-change action, not per agent or simulation tick.

## Removal And Synchronization

### Stale-building cleanup

`cleanup_stale_buildings()` removes a building when any of these becomes true:

- its `edge_idx` no longer exists or its edge is deleted
- its edge now has `no_building_spawn = true`
- its footprint has become too close to a road surface
- its full footprint no longer resolves to one compatible runtime `ZoneProfile` for the current
  asset after any rezoning grace has expired
- its `asset_id` no longer resolves to a valid building manifest required for legality checks

Removal order is important and currently deterministic:

1. clear the parcel claim in `ZoningSystem`
2. invalidate logistics references to the removed building
3. if `swap_remove` will move another building, remap moved building indices in dependent systems;
   parcel occupancy uses that building's known parcel ID through the existing stable-id map, and
   undo applies the inverse update after restoring the building records (`AUDIT-01-Z2`)
4. `swap_remove` from `buildings`
5. mark indices and entrance cache dirty

Important invariant:

- stale-building cleanup no longer falls back to broad `ZoneType` checks when a building's
  `asset_id` is missing or malformed
- explicit buildings are ignored by zoning-compatibility cleanup, but malformed or unresolved
  building manifests are treated as invalid runtime state and are removed instead of silently
  surviving on a weaker zoning check

### Save/load and topology rebuilds

- parcel occupancy is rebuilt from saved building parcel ids
- saving maps live road/node references into the snapshot without mutating live graph, building
  or parcel indices; retired in-place compaction/remap APIs are removed (`AUDIT-01-A15`)
- save/load uses `recompute_derived_transforms()` to rebuild `center_x`, `center_y`, and `facing_dir`
  from saved attachment data plus live road geometry
- live road splits and `repair_road_attachments_after_topology_edit()` instead preserve the placed
  world pose and `support_height_m`, changing attachment references rather than moving the site;
  entrance caches then rebuild against those repaired references
- `RoadEditPlan` captures nearby site footprints/support heights through the prepared building index
  and grades against final planned roads. Readiness never rebuilds that index; changed local sites
  invalidate the plan, and adoption checks the site set again after topology changes.
- Explicit sites (`parcel_id = 0`), including farms, use the saved `edge_idx`, `frontage_t`,
  `side`, and depth. `frontage_t` locates the frontage center along the physical road polyline;
  `cell_x = 0` is not a road station. Placement and load share the same position/tangent sampler,
  sidewalk setback, and half-depth offset. `support_height_m` remains the saved placement plane;
  site footprints and entrance caches rebuild from the restored transform.
- `BUILD-01` debugging: compare SQLite `buildings` attachment/support fields and
  `network_edge_geometry` physical points with the building inspector's `center_x` / `center_z`.
  `METRUM_DEBUG_BUILDINGS=1` adds footprint, facing, support-height, and nearby road/terrain
  samples to the building render frame's `site_mesh_data.debug_sites`. Its `center` is the
  support polygon centroid, which can differ from the building center. A second save/load must
  preserve building-part transforms, site surface vertices, and farm field polygons.
- The `iso.sqlite` native replay restores all three farms within 2 mm of independently sampled
  saved frontage positions. Their support heights, 43 field vertices, rendered building parts,
  and site surface meshes survive another save/load unchanged; all five surrounding terrain
  patch payloads compile successfully.

## Indices And Vacancy Rules

`rebuild_zone_index()` repopulates:

- `zone_index`
- `vacancy_index`
- `vacancy_pos`
- `building_chunks`

Inspector and service-funding picks share `nearest_building_idx_at()`. It searches building
centres strictly within 30 m, preferring the lowest building index on an exact distance tie.
Non-finite points return no hit. A clean query examines at most four existing 512 m chunks,
uses constant auxiliary storage and costs O(buildings in those chunks); it does not allocate or
sort a candidate list. Dirty indices are rebuilt once before querying using the existing
allocator maintenance path. Broken and unfinished placed buildings remain inspectable.
Household detail aggregation is a separate bridge path and remains under audit.

`AUDIT-01-B1` locality validation runs the ignored release test
`simulation::buildings::allocator::tests::indexing::benchmark_building_pick_locality`
with `RAYON_NUM_THREADS=24` and CPU affinity 0. Five fresh processes each use three warm-up and
11 measured samples of 2,000 picks, retaining 32 nearby buildings and adding 0 / 1,024 / 65,536
distant buildings. Setup/index construction is excluded; each workload checks the same chosen
building. Query medians are **76.752 / 74.401 / 75.689 ns**. Exact binary/source identity and raw
results are `/tmp/metrum-full-audit/building-pick-after-identity.json` and
`building-pick-locality*.{json,log}`. This establishes clean-query locality, not an old/new speedup.

Current vacancy rule:

- a building enters the residential vacancy index when `household_capacity(idx) > occupancy`
- `claim_vacancy()` and `release_vacancy()` update that index in O(1)
- `mark_building_deserted()` removes remaining vacancies immediately in O(1), including farms; admission and forced rehousing never wait for a later full index rebuild.
- Housing capacity comes from the resolved asset, with exactly one household slot for explicit field-producing farms. Unresolved, broken, deserted, and under-construction buildings offer no usable housing.
- Farms share the residential vacancy list without joining the residential zoning/growth index; household capacity does not grow with field area. Farmhouse area uses the authored `flat_size_m2`, defaulting to 120 m2 when unspecified.
- `worker_capacity_with_catalog(idx, catalog)` uses the bound economy profile and committed production area. Inspector, toolbar and city diagnostics reuse the live catalog; unresolved profiles offer zero jobs. Manifest capacity applies only to assets without an economy profile.

This index is allocator-owned because household admission and home claiming still route through the
allocator today.

## Entrance Cache Boundary

The allocator owns when the entrance cache is rebuilt. The entrance doc owns what the cache means.

Current rebuild triggers include:

- building placement and removal
- load or restore paths that rebuild building transforms
- road/lane changes that invalidate or recreate live edge or lane topology
- explicit dirty-flag or length-mismatch detection during allocator maintenance

The derived entrance cache must never become a second authoritative source of placement truth.

## Growth Policy And Lifecycle Responsibilities

[`demand.md`](demand.md) owns immigration, emigration, spawn, despawn, upgrade and downgrade
pressure. The allocator executes those decisions and owns placement legality and indexed lifecycle
updates, including cleanup after zoning or road attachments become illegal. It does not choose
growth policy or calculate admission pressure. Execution revalidates the current geometry even
when demand already accepted a candidate.

Selected despawn and level-change actions resolve the parcel's current occupant through the existing
stable-ID map, then compare all action-key fields against that building. This follows allocator
swap-removes without a temporary table or asset-string copies and rejects stale/replaced actions.
Lookup is expected O(1) in city size, with O(asset-ID length) comparison and constant auxiliary
storage per action. Placement initializes the profit-tax budget baseline once; demand execution
uses that same initialized record. Actual removal, placement and cache maintenance retain their
separate lifecycle costs.

`AUDIT-01-B2` measures the ignored release test
`simulation::buildings::allocator::tests::demand_actions::benchmark_demand_action_lookup`.
Five alternating before/after process pairs use CPU 0, `RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`,
three warm-up calls and 21 samples of four calls. One selected despawn becomes ineligible because
the home is occupied, matching the admission-before-action ordering. The plan is nonempty: the
live core skips empty and spawn-only immediate plans. Minimal indexed building/parcel records
supply increasing distant background; no full gameplay constructor, routing, agent simulation,
site rebuild or setup time is measured. Building/parcel checksums and revisions match in every pair.

| Total buildings and parcels | Before, ms per call | After, ms per call |
| --- | ---: | ---: |
| 1 | 0.000100750 | 0.000026500 |
| 1,024 | 0.064289500 | 0.000026750 |
| 65,536 | 5.776955250 | 0.000026500 |
| 262,144 | 31.211681750 | 0.000027750 |

These are medians of process medians for dispatch/revalidation, not whole-tick or demolition
speedups. The removed O(B) key construction allocated one hash table plus an asset-ID copy per
building. Artifacts are `/tmp/metrum-full-audit/demand-lookup-{before,after}-identity.json`,
`demand-lookup-matched-bench.json`, `demand-lookup-matched-summary.json` and per-pair logs.
Before/after executable SHA-256 values are
`120735fac831810ccdd5855fe61eef5837575050c6a89c454e0ad9d2e12eacbc` and
`82818134ba5af4d05188806130e0646fbeb0668a1e8dea78acb10518ec76295a`.
The existing combined action regression now removes the first storage slot before upgrading and
downgrading survivors, and rejects a stale asset key on an otherwise removable parcel.

### Site radius maintenance audit (`AUDIT-01-B3`)

The site query margin remains the exact largest site radius measured from its indexed lot centre.
Appending a site updates the maximum directly. Replacing a site examines its old/new radii and
reduces all sites only if a previous maximum shrinks. Removal shares one swap-order helper and
reduces only when the removed site matched the maximum. Tied maxima remain correct. Bulk rebuild
and undo still reconstruct the exact maximum; no new index or approximate upper bound is added.

Ordinary radius maintenance is O(V) for the changed site's V support/paving vertices. A shrinking
or removed maximum and a full rebuild remain O(total site vertices), now using Rayon with a minimum
split length of 4,096 sites. Radius calculation takes the maximum squared distance before one
square root. Mismatched site-store lengths go directly to the existing full rebuild, avoiding a
client that would immediately be discarded. The unused quad-bounds wrapper and a copied test
building/footprint are removed.

All 1,754 release tests pass (51 ignored). The new regression covers radius growth/shrinkage,
nonmaximum/maximum removal, the empty store and height queries across a chunk boundary. Five
alternating before/after pairs at each of one and eight workers preserve complete footprint/height
and radius-bit checksums. The ignored test is
`simulation::buildings::allocator::site::tests::benchmark_site_radius_maintenance`; each phase uses
three warmups and 21 batches of four calls. Minimal isolated records use the existing missing-asset
lot-footprint fallback, excluding gameplay setup, imported asset complexity and routing. One fixed
small site is rebuilt under a larger background maximum; a separate phase forces the full reduction.
Eight-worker medians, milliseconds per call:

| Sites | Local rebuild before | Local rebuild after | Full reduction before | Full reduction after |
| --- | ---: | ---: | ---: | ---: |
| 1 | 0.000041750 | 0.000042500 | 0.000012250 | 0.000016000 |
| 1,024 | 0.006327000 | 0.000045500 | 0.006309500 | 0.006272000 |
| 65,536 | 0.408843750 | 0.000045500 | 0.407894500 | 0.066663000 |
| 262,144 | 2.190566750 | 0.000055500 | 2.183913250 | 0.411176500 |

The single-record overhead is sub-nanosecond for rebuilding and about 4 ns for forced reduction.
With one worker, the 262,144-site local rebuild changes from 2.166914 to 0.000045750 ms, while forced
reduction is approximately unchanged at 2.188272250 versus 2.207823250 ms. The latter remains a full
scan. These measurements isolate site/radius maintenance and do not claim whole-demolition or
asset-placement speedups.

Three alternating eight-worker populated-road pairs also retain identical local products within
each build as the background grows to 100,000 buildings and 600,024 agents. Worker medians at
0 / 1,000 / 10,000 / 100,000 background buildings are **20.813 / 20.696 / 20.563 / 20.819 ms before**
and **20.958 / 20.770 / 20.815 / 20.925 ms after**. One-time snapshot costs, measured separately,
grow from 0.0069 to 3.4011 ms before and 0.0070 to 3.3856 ms after. Planning remains local; these
results establish no general road-planning speedup or exported cross-build payload-hash comparison.

Builds use `cargo test --offline --manifest-path rust/Cargo.toml --release --lib`, Rust 1.98.1
(48a229cea 2026-09-01). Affinity is CPU 0 for one worker and `0,2,4,6,8,10,12,14` for eight,
with `RAYON_NUM_THREADS` matching and `METRUM_DEBUG=0`. Before/after executable SHA-256 values are
`33a93821ce3c018a4c516237cbceb3a6d2b7087d52fefa63bbc11f7279e1041f` and
`87652b2ff206b2409017f57562c6daafc09337b26953d0154e09ca62f52b45ee`.
Source/binaries stayed fixed and no compilation, tests or archive compression competed with accepted
measurements. Runners, exact commands, source identities and results are in
`/tmp/metrum-full-audit/site-radius-*`. The initial capture hit the temporary quota; its incomplete
copy and unmatched probe were excluded. Verified compression recovered space before all matched
pairs restarted (`site-radius-capture-quota.log`, `binary_archives.json`).

## Rezoning clock correction (`AUDIT-01-B4`)

No-building road edits previously invoked the same implicit daily countdown as settlement. The
regression reproduced an unrelated building losing a grace day after one edit. Maintenance now
receives actual elapsed days, preserving immediate removal, recovery and swap-remapping hooks.
No saved fields, compatibility switches or extra city passes are introduced. The compatibility
scan remains O(B); removal and dependent-cache costs retain their existing ownership.

All 1,753 release tests pass (52 ignored). One duplicate expiry test was removed after its exact
three-day assertions were incorporated into the existing occupancy/removal test. Recovery and
expiry share a minimal two-building fixture; obsolete forced demand, immigration and copied
building initialization are removed. The road-edit regression checks immediate deletion, the
surviving parcel's remapped owner, repeated edits and the following real daily decrement.

Five alternating, unprofiled release pairs measure
`simulation::buildings::allocator::tests::lifecycle::benchmark_rezone_maintenance` on CPU 0 with
`RAYON_NUM_THREADS=1`, `METRUM_DEBUG=0`. Each process uses three warmups and 21 samples of four daily
compatibility updates. Half the buildings have incompatible zoning; countdowns start at 255 so
all 87 updates run without expiry. Setup, removal, cache rebuilds and immediate edits are excluded.
All pairs retain identical parcel/position/countdown checksums and occupancy/reference revisions.

| Buildings | Before median ms | After median ms |
| ---: | ---: | ---: |
| 1 | 0.00002775 | 0.00002775 |
| 1,024 | 0.025191 | 0.025731 |
| 65,536 | 2.851052 | 2.854124 |
| 262,144 | 15.8464665 | 15.940285 |

These medians of process medians show essentially unchanged daily scan cost; they establish no
road-edit locality or whole-maintenance speedup. Builds use Rust 1.98.1 and
`cargo test --offline --manifest-path rust/Cargo.toml --release --lib`. Before/after executable
SHA-256 values are `7c139ec8bd65608a81e73684535721497177ee4f786563580b4d04eda51a7bfe` and
`51c4d9a6cc124ce4e40b62e792b44c17a1dea0ef5529f5002cd892023f19172a`.
Exact commands, fixed source identities, the failing baseline regression and matched outputs are
retained under `/tmp/metrum-full-audit/rezone-clock-*`. Compilation and engine checks ran separately
from timing comparisons.

## Explicit lot-query correction (`AUDIT-01-B5`)

The zoning reservation check used a support-footprint search radius before testing full lot
geometry. A validated power-plant asset with a compact imported structure in a 200 m lot reproduced
the failure: a parcel overlapping the remote part of its lot was rejected before index preparation
but accepted afterward. The strengthened regression covers both signs of the 512 m boundary and
retains legal edge contact. It uses normal site derivation and index preparation with isolated road
and building fixtures, replacing unrelated gameplay placement setup.

The check now shares the existing full-lot candidate lookup with placement neighbors. That helper
takes query bounds, live cell size and the optional replaced-building index directly; no new index
or radius cache is added. The indexed search visits the intersected chunks and their candidates,
with existing candidate sorting; it does not scan unrelated buildings. Support-surface queries
retain their smaller radius and exact geometry. All 1,753 release tests pass (52 ignored).

Five alternating, unprofiled release pairs run the existing
`nodes::sim::core::tests::road_plan_scaling::populated_zoning_feasibility_scaling` workload. Four fixed
local residential sites remain while background grows to 100,000 buildings, 100,004 total parcels,
391 remote roads and 600,024 agents. Each case performs 300 warm queries; every result remains legal
and the total local solve count stays one across all background sizes and builds.

| Background buildings | Before warm median µs | After warm median µs |
| ---: | ---: | ---: |
| 0 | 0.683 | 0.671 |
| 1,000 | 0.702 | 0.682 |
| 10,000 | 0.689 | 0.682 |
| 100,000 | 0.686 | 0.674 |

These medians of process medians establish retained locality for cached zoning feasibility. The
separately measured first/revalidation call is 0.05649/0.05487 ms before/after at zero background and
0.01968/0.02009 ms at 100,000. This workload does not time explicit-lot rejection or establish a
general placement speedup; the boundary regression establishes the corrected rejection behavior.

Builds use Rust 1.98.1 and `cargo test --offline --manifest-path rust/Cargo.toml --release --lib`.
Runs use CPU affinity `0,2,4,6,8,10,12,14`, `RAYON_NUM_THREADS=8`, `METRUM_DEBUG=0`, without competing
compilation/tests. Before/after executable SHA-256 values are
`e01d8824fbf3aa3df8c0151bb0e97647c333f3d567a39995096c0a85892b68f9` and
`4a530c6a5d508e409a9a91bd0c797dc4add738b2ef05ccf0d2c87d45c1c2f76b`.
Exact commands, four-file diff, source identities and results are in
`/tmp/metrum-full-audit/lot-query-*`.

## Parcel-selection hash cleanup (`AUDIT-01-B6`)

Family and variant selection now call one `stable_parcel_selection_hash` implementation. The
existing hasher still writes the little-endian profile ID, little-endian parcel ID and UTF-8 key,
with `0xff` after each field. The family/hash/string/variant ordering is unchanged. The byte-folding
implementation and ranking tuple are checked against the preceding source; no loops, allocations
or data layouts change, and no new timing claim is made for this cleanup.

Five fixed vectors cover empty keys, full-width IDs, UTF-8 and embedded nulls. They pass against
both old functions before consolidation and against the shared function afterward. The startup
family and within-family variant tests retain their separate assertions while sharing setup.
Their copied hash algorithm, obsolete strip naming and profile-lookup helper with ignored allocator
and graph arguments are removed. Missing spawned parcels now fail the fixture explicitly.

All 1,754 release tests pass (52 ignored). The baseline vector run, complete final suite, three-file
diff and source-comparison evidence are retained under `/tmp/metrum-full-audit/parcel-hash-*`.

## Road dependency grid correction (`AUDIT-01-B7`)

Site feasibility cached nearby road records using configurable render-chunk coordinates to read
the fixed 32 m query index. With offset 512 m render chunks, the regression captured no local roads.
The cache could therefore reuse a preview/demand verdict after a local road update when its other
dependencies matched. Committed placement already performs uncached site validation.

Dependency capture now calls the existing fine-query coordinate converter. No new grid, radius or
cache is introduced. Collection remains local: O(C + K log K), for queried chunks C and candidate
record visits K. The regression covers positive/negative road positions, both render layouts,
local span and junction-record replacement, and preservation after remote span replacement.
All 1,755 release tests pass (52 ignored); the new regression fails against the preceding code.

Five alternating release pairs run
`nodes::sim::core::tests::road_plan_scaling::populated_zoning_feasibility_scaling`, with four fixed
local residential sites and 0/1,000/10,000/100,000 background buildings. At the largest size there
are 100,004 parcels, 391 remote roads and 600,024 agents. Each case performs 300 warm queries;
verdicts remain legal and the local solve count remains one at every size in both builds.

| Background buildings | Before warm median µs | After warm median µs |
| ---: | ---: | ---: |
| 0 | 0.682 | 0.677 |
| 1,000 | 0.704 | 0.698 |
| 10,000 | 0.680 | 0.674 |
| 100,000 | 0.678 | 0.681 |

The first/revalidation median is 0.049995/0.050701 ms before/after with no background and
0.020004/0.025032 ms with 100,000 background buildings. Correct local record checks add a few
microseconds to revalidation; warm queries retain locality. These measurements cover cached
feasibility, not complete placement or arbitrary road-edit latency.

Builds use Rust 1.98.1 and `cargo test --offline --manifest-path rust/Cargo.toml --release --lib`.
Matched runs use `RAYON_NUM_THREADS=8`, `METRUM_DEBUG=0`, CPU affinity
`0,2,4,6,8,10,12,14`, and no competing compilation/tests. Executable SHA-256 values are
`69080b7e81319429aec298641bef2a4c60f5ab0e43544bd68ef5f0d6c697f3e2` before and
`c29c1f793a1914aba3c6abb10029c13d20d567f0c939f80292cdce2935e7c2e6` after.
Exact commands, source identities, two-file diff and logs are in
`/tmp/metrum-full-audit/site-dependencies-*`.

## Known Limitations And Follow-Up

The current allocator foundation is usable and directionally correct for a road-frontage city
builder, but several parts still need cleanup or hardening before the allocator should be treated as
fully mature.

Current follow-up limitations:

- Building and terrain frame publication are not one atomic renderer transaction. Geometry
  regressions do not establish that every transient construction/publication artifact is fixed;
  visual hardening remains tracked under `EARTH-02`.
- `edge_occupancy` is currently only a fast leading-column pre-check. Final overlap safety still
  depends on the occupied-footprint test rather than on full frontage-span reservation.
- The runtime legality path is now profile-based and footprint-wide. The remaining broad
  `zone_type` field on `Building` is an intentional derived hot-path cache, not a second
  authoritative legality source.
- Stale-building cleanup now uses footprint-wide profile compatibility plus deterministic rezoning
  grace, but some other attachment-invalidity paths still collapse directly into removal rather than
  through a richer reattachment or redevelopment flow.
- Temporary road deletion and rebuild currently collapse into ordinary attachment invalidation. This
  is intentionally deferred to later allocator hardening work: frontage attachment should eventually
  get its own short deterministic reattachment grace so buildings are not demolished unnecessarily
  during intentional road rebuilds.
- The current full frontage scan order is acceptable for the baseline demand-owned spawn batches,
  but it should not become the permanent large-city private-development allocator once
  demand-driven growth scales further.

Recommended interpretation:

- keep the frontage-attached allocator model
- treat these items as hardening and ownership cleanup work, not as a reason to replace the whole
  allocator concept

## Ownership Boundaries

Recommended ownership split:

- [`zoning.md`](zoning.md): parcel legality, parcel geometry, and parcel occupancy helpers
- road/network systems: authoritative road-edge geometry, edge existence, and
  `no_building_spawn` policy
- [`asset_editor.md`](asset_editor.md): asset-authored footprint dimensions, baseline `zone_type`,
  `density`, anchors, and tags
- `BuildingAllocator`: build-site discovery, frontage attachment, fit checks, placement, removal,
  indices, and derived entrance-cache rebuild ownership
- [`demand.md`](demand.md): growth pressure, site scoring, and spawn-despawn-upgrade
  decisions
- [`entrance_and_exit.md`](entrance_and_exit.md): exact meaning and runtime use of the derived
  entrance cache
- [`economy.md`](economy.md): post-placement building economy behavior
