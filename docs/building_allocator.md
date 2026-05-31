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
- a few still-transitional growth behaviors

This document owns the allocator-side contracts. It does not own:

- zoning profile design or paint-tool behavior in [`zoning.md`](zoning.md)
- demand formulas or growth-profile tuning in [`demand.md`](demand.md)
- exact entrance or trip movement behavior in [`entrance_and_exit.md`](entrance_and_exit.md)
- asset-authoring UI behavior in [`asset_editor.md`](asset_editor.md)
- building economy behavior in [`economy.md`](economy.md)

## Current Tick Order

`BuildingAllocator::tick()` currently runs in this order:

1. `cleanup_stale_buildings()`
2. `network.rebuild_pathing_if_dirty()`
3. `rebuild_entrance_cache()` when the cache is dirty or length-mismatched
4. `rebuild_zone_index()` when indices are dirty
5. clear `dirty`

Important ownership note:

- ordinary household admission no longer runs inside allocator tick; the live runtime now computes
  `households_to_admit_today` in the demand layer after daily economy settlement, then calls
  `execute_demand_household_admission(...)` as a separate consumption step
- private building spawn, despawn, upgrade, and downgrade also no longer depend on allocator-local
  heuristics; the live runtime now consumes demand-owned daily building-action plans through
  `execute_demand_building_actions(...)` after the daily demand pass
- fresh-map startup no longer uses an allocator-owned founding placement exception; the live runtime
  now relies on the organic pioneer-demand bootstrap described in [`demand.md`](demand.md)
- the deterministic midnight application order is now:
  `execute_demand_household_removal(...)`, then `execute_demand_building_actions(...)`, then
  `execute_demand_household_admission(...)`; any post-admission workplace assignment happens
  outside allocator ownership

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

## Build-Site Model

In the live runtime, a build site is one frontage-attached roadside candidate footprint. It is not a
free-floating parcel.

Baseline build-site identity:

- `edge_idx`: attached road edge
- `side`: one side of that edge
- `cell_x`: leading frontage column on that edge side
- `width_cells` and `depth_cells`: candidate footprint size

Placement parameters currently come from the building asset manifest:

- `zone_type`
- `lot_width_cells`
- `lot_depth_cells`
- `level`

Deterministic discovery order in the current allocator:

- ascending `edge_idx`
- side order `[1, -1]`
- ascending `cell_x`

That scan order currently decides the first valid founding-placement result and also serves as the
natural fallback ordering for any later deterministic tie-break.

### Legality checks

`resolve_slot()` currently validates a candidate parcel in this order:

1. parent edge exists, is not deleted, and allows building spawn
2. the parcel is not already occupied
3. the parcel has a non-zero runtime `ZoneProfile`
4. that `ZoneProfile` accepts the candidate asset's baseline `zone_type`, `density`, and authored
   tag filters
5. the asset footprint fits inside the parcel frontage and depth

If all checks pass, the allocator commits placement by:

1. claiming the parcel in `ZoningSystem`
2. pushing the new `Building`
3. setting `dirty`, `dirty_index`, `entrances_dirty`, and the building's `dirty_zones` entry

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
- `recompute_derived_transforms()` rebuilds `center_x`, `center_y`, and `facing_dir` from saved
  attachment data plus live road geometry

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

## Transitional Growth Responsibilities

The allocator still owns some lifecycle behavior that should eventually move elsewhere:

- invalid-placement cleanup that removes buildings when zoning or road attachment becomes illegal

The allocator now only executes already-decided household admission through
`execute_demand_household_admission(...)`; it no longer owns the immigration-pressure formula or
the daily admission count itself.

Target direction:

- [`demand.md`](demand.md) should own immigration, emigration, spawn, despawn, upgrade, and
  downgrade pressure
- allocator should execute already-decided legal placement or removal, not invent the city's growth
  policy locally

## Known Limitations And Follow-Up

The current allocator foundation is usable and directionally correct for a road-frontage city
builder, but several parts still need cleanup or hardening before the allocator should be treated as
fully mature.

Current follow-up limitations:

- The live legality and ownership checks are still somewhat weaker than the intended long-term
  contract. In particular, some spec wording is cleaner than the exact current implementation and
  should be reconciled before the allocator becomes the long-term growth execution layer.
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
- [`demand.md`](demand.md): growth pressure, site scoring, and future spawn-despawn-upgrade
  decisions
- [`entrance_and_exit.md`](entrance_and_exit.md): exact meaning and runtime use of the derived
  entrance cache
- [`economy.md`](economy.md): post-placement building economy behavior
