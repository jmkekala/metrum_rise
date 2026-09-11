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

## Current Tick Order

`BuildingAllocator::tick()` currently runs in this order:

1. `cleanup_stale_buildings()`
2. `network.rebuild_pathing_if_dirty()`
3. `rebuild_entrance_cache()` when the cache is dirty or length-mismatched
4. `rebuild_zone_index()` when indices are dirty
5. clear `dirty`

Important ownership note:

- ordinary household admission runs after the hourly demand pass, not inside allocator tick;
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

If all checks pass, the allocator commits placement by:

1. claiming the parcel in `ZoningSystem`
2. pushing the new `Building`
3. setting `dirty`, `dirty_index`, `entrances_dirty`, and the building's `dirty_zones` entry

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
3. if `swap_remove` will move another building, remap moved building indices in dependent systems
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
- `update_edge_indices()` remaps `Building.edge_idx` and parcel road attachments after road compaction
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

Current vacancy rule:

- a building enters the residential vacancy index when `household_capacity(idx) > occupancy`
- `claim_vacancy()` and `release_vacancy()` update that index in O(1)
- `household_capacity(idx)` and `worker_capacity(idx)` now come only from the resolved authored asset manifest; the live runtime no longer invents fallback capacities when asset data is missing.
- Note: `worker_capacity(idx)` is authoritatively overridden by the building's bound economy profile if one is present.

This index is allocator-owned because household admission and home claiming still route through the
allocator today.

## Entrance Cache Boundary

The allocator owns when the entrance cache is rebuilt. The entrance doc owns what the cache means.

Current rebuild triggers include:

- building placement and removal
- load or restore paths that rebuild building transforms
- road/lane changes that invalidate or recreate live edge or lane topology
- explicit dirty-flag or length-mismatch detection during allocator tick

The derived entrance cache must never become a second authoritative source of placement truth.

## Growth Policy And Lifecycle Responsibilities

[`demand.md`](demand.md) owns immigration, emigration, spawn, despawn, upgrade and downgrade
pressure. The allocator executes those decisions and owns placement legality and indexed lifecycle
updates, including cleanup after zoning or road attachments become illegal. It does not choose
growth policy or calculate admission pressure. Execution revalidates the current geometry even
when demand already accepted a candidate.

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
