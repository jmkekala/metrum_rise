# Zoning System — Implementation Reference

This document describes the zoning system **as implemented** after the world-space zoning rewrite and overlay/tooling pass.
Update it when the implementation changes; update [`project.md`](project.md) and [`roadmap.md`](roadmap.md) when tracked status changes.

---

## 1. What Was Replaced

The legacy `EdgeZoning` system stored zone type and occupancy per-road-edge in road-local cell
coordinates. It caused 90–111 ms stalls at ~30 road edges because `flush_zoning_updates` /
`recalculate_obstructions` ran a Voronoi ownership pass over every dirty edge on each daily tick.
The following are **fully deleted**:

- `ZoningSystem::edge_grids: HashMap<usize, EdgeZoning>`
- `split_edge_grid` / `merge_edge_grids` in `topology.rs`
- `recalculate_obstructions`, `flush_zoning_updates`, `zoning_dirty_edges`
- `update_edge_grid_size` call in `add_road`
- `zoning/grid.rs`, `zoning/block.rs`, `zoning/obstruction.rs`

---

## 2. Data Structures

### `ZoningSystem` (`simulation/grid/zoning/mod.rs`)

```rust
pub struct ZoningSystem {
    pub profiles:         Arc<ZoningProfileRegistry>,
    pub grid:             DataGrid<u16>,        // 2000×2000 runtime profile ids, 0 = unpainted
    pub occupied:         DataGrid<bool>,       // 2000×2000, building footprints
    pub distance_to_road: DataGrid<u8>,         // 2000×2000, metres to nearest road (clamped 255)
    pub no_build_mask:    DataGrid<bool>,       // 2000×2000, no-build frontage suppression
    pub config:           WorldConfig,
}
```

Cell size: **10 m**. A 20 km × 20 km map → 2000 × 2000 cells.

Coordinate conversion (world → cell):
```
cx = ((x / zone_cell_m) + hw).round() as usize   // hw = (width - 1) / 2
cy = ((z / zone_cell_m) + hh).round() as usize   // hh = (height - 1) / 2
```
Out-of-bounds returns `None`; callers skip the slot.

### Allocator-owned placement state

Allocator-specific placement state such as `EdgeOccupancy` now belongs to
[`building_allocator.md`](building_allocator.md). This zoning document only owns the world grid,
occupied grid, distance-to-road grid, no-build mask, and the player-facing zoning/tool contract.

### Derived broad zone families

The authoritative painted world stores runtime `ZoneProfile` ids, not `ZoneType` bytes. Broad
`ZoneType` values are now derived helpers used only where a larger family bucket is still useful.

| Value | Name | Notes |
|---|---|---|
| 0 | None | Unpainted / no zoning |
| 1 | Residential | Shipped baseline family |
| 2 | Commercial | Shipped baseline family |
| 3 | Industrial | Shipped baseline family |
| 4 | Office | Deferred extension; not in the shipped Phase 1 profile set |
| 5 | Mixed | Deferred extension; not in the shipped Phase 1 profile set |

---

## 3. Public Rust API

All exposed via `#[func]` in `nodes/simulation_node.rs` and backed by
`nodes/sim/editing.rs` where mutation is needed:

| Method | Purpose |
|---|---|
| `get_zone_profiles() -> Array[Dictionary]` | Return the validated zoning-profile registry for UI and tools |
| `capture_zoning_patch(grid_x, grid_y, width_cells, height_cells) -> PackedByteArray` | Capture one little-endian `u16` profile-id patch for undo |
| `apply_zoning_patch(grid_x, grid_y, width_cells, height_cells, target_profile_runtime_id, write_mask)` | Apply one masked paint patch |
| `restore_zoning_patch(grid_x, grid_y, width_cells, height_cells, profile_ids_le_u16)` | Restore one full patch for undo |
| `get_zone_profile_texture_data_rg8() -> PackedByteArray` | Full profile-id grid packed as `RG8` for overlay upload |
| `get_zone_profile_style_lut_rgba8() -> PackedByteArray` | One-row RGBA8 style LUT for the overlay shader |
| `get_occupied_texture_data() -> PackedByteArray` | Full occupied grid as flat u8 |
| `get_distance_texture_data() -> PackedByteArray` | Full distance grid as flat u8 |
| `get_no_build_mask_texture_data() -> PackedByteArray` | Full no-build mask as flat u8 |
| `get_zone_grid_size() -> Vector2i` | Return zoning-grid dimensions for Godot tools |
| `get_no_building_spawn_edge_indices() -> PackedInt32Array` | Indices of flagged edges (overlay) |
| `set_no_building_spawn(edge_idx, enabled)` | Toggle no-build flag |
| `get_no_building_spawn(edge_idx) -> bool` | Read no-build flag |

`ZoningSystem::update_edge_indices` is a no-op kept for call-site compatibility with
`compact_edges` — the world grid has no edge-keyed data to remap.

---

## 4. Allocator Interaction

The full roadside placement and removal pipeline is owned by [`building_allocator.md`](building_allocator.md).
From the zoning system's perspective, the allocator currently consumes these services:

- `get_zone_profile_runtime_id_world(...)` to read the painted runtime profile at footprint cells
- `profiles.asset_is_legal(...)` to test that the painted profile accepts the asset's
  `zone_type + density + tags`
- `distance_to_road_world(...)` to reject road-overlap cases and resolve scan-order ownership near
  competing roads
- `is_rect_occupied(...)` to reject overlapping building footprints
- `mark_occupied_rect(..., true|false)` to keep the occupied grid synchronized with building
  placement and removal

The important zoning-side rule is:

- zoning provides legality and occupancy data
- allocator owns candidate-site discovery, frontage attachment, and final placement or eviction

---

## 5. No-Build Edge Flag (`Edge::no_building_spawn`)

- Default: `false`.
- **Auto-set**: `speed_limit ≥ 80 km/h` → `true` in `create_edge_internal`.
- **Player toggle**: "No buildings" checkbox in the road properties panel (`main_ui.gd` / `select_tool.gd`).
  Toggling calls `set_no_building_spawn(edge_idx, bool)` and sets `allocator.dirty = true` — existing
  buildings facing the edge are evicted on the next allocator tick.
- **Overlay**: when the zone tool is active, `zoning_overlay.gd` draws orange `ImmediateMesh` lines
  along all flagged edges via `get_no_building_spawn_edge_indices()`.
- **Persistence**: `no_building_spawn INTEGER NOT NULL DEFAULT 0` column in `network_edges` SQLite
  table. Forward-compat `ALTER TABLE ADD COLUMN` migration applied on load of old saves.
- **Split propagation**: `split_edge` in `topology.rs` copies the flag to both child edges.

---

## 6. Road Selection and Properties Panel

`select_tool.gd` — click to select one edge; drag along connected edges to extend the selection.

- **Connectivity rule**: drag only adds an edge if it shares at least one node with the current selection (`_connected_nodes` dictionary).
- **Highlight**: `ImmediateMesh` ribbon built from `get_edge_geometry_3d` + `get_edge_width`.
  Material: `CULL_DISABLED`, `no_depth_test = true`, `render_priority = 100`, emission shimmer
  tweened between `Color(1,1,0)` and `Color(1,1,1)` every 0.5 s.
- **Properties panel**: shown by `main_ui.show_road_properties_multi(edge_indices)`. On open,
  reads current class (`get_edge_class`) and no-build state (`get_no_building_spawn`) from Rust and
  reflects them in the UI (depressed class button, checked checkbox). Multi-selection shows count
  and shared state if all edges agree.
- **Click isolation**: panel uses `_unhandled_input` in `select_tool.gd`; the Control node in
  `main_ui.gd` consumes mouse events before they reach the tool, preventing click-through.

---

## 7. Zone Rendering (`zoning_overlay.gd` + `zoning_overlay.gdshader`)

A single full-map `MeshInstance3D` quad at `Y = 0.005` (below road asphalt at `Y = 0.01`).
Five uploaded resources drive the zoning overlay material:
- profile ids packed as `RG8`
- style LUT packed as `RGBA8`
- distance-to-road as `R8`
- occupied as `R8`
- no-build mask as `R8`

### Shader behaviour

| Input | Effect |
|---|---|
| Profile-id `RG8` texel | Reconstruct runtime `u16` profile id |
| Style LUT | Resolve the player-facing profile colour |
| Distance ≤ 3 m (road carriageway) | Alpha → 0 (road asphalt masking) |
| Distance 3–40 m | Full zone alpha |
| Distance 40–100 m | Linear taper to 0.4× |
| Distance > 100 m | 0.2× (ghost — zoned but no road access) |
| Occupied byte = 1 | Alpha × 0.4 (building suppression) |
| Tool active | Alpha 0.38, procedural 10 m cell grid visible |
| Tool passive | Alpha 0.20, cell grid hidden |

### Texture update cadence

- **Profile texture**: full re-upload after each paint operation
  (`apply_zoning_patch` / `restore_zoning_patch` → `get_zone_profile_texture_data_rg8`).
- **Style LUT**: uploaded when the overlay initializes or the profile registry changes.
- **Distance-to-road texture**: re-uploaded from `network_renderer.gd` after every road placement or removal.
- **Occupied texture**: re-uploaded every 30 frames from `buildings.gd`.
- **No-build mask texture**: re-uploaded with the distance texture and after no-build flag toggles.

### No-build overlay

Orange `ImmediateMesh` lines are drawn along flagged edges when the zone tool is active.
Rebuilt by `_rebuild_no_build_overlay()` called from `set_tool_active()` and `full_refresh()`.

---

## 8. Save / Load

- Zone grid serialised as a single flat `BLOB` in `zoning_world_grid` SQLite table (width,
  height, data), with one little-endian `u16` runtime profile id per cell.
- allocator-owned `edge_occupancy` is **not** saved directly — see
  [`building_allocator.md`](building_allocator.md) for the rebuild contract.
- placed buildings now save the authoritative `zone_profile_runtime_id`; the old broad
  `zone_type` field is rebuilt as a derived cache from the loaded profile id
- `occupied` grid is rebuilt on load by replaying placed building footprints through
  `mark_occupied_rect`.
- `distance_to_road` is rebuilt on load by `update_distance_to_road(graph)`.
- old edge-local `zoning_grids` save compatibility was intentionally removed during the profile-grid
  rewrite; only the world-grid `zoning_world_grid` format is supported now

---

## 9. Known Limitations

### 1. Scan-order ownership ambiguity — resolved
After computing `frontage_center`, the allocator reads `distance_to_road_world` at that point.
Expected distance to this edge's own surface is `SIDEWALK_WIDTH + zone_cell_m/2 = 6.5 m`;
threshold is 5 m (1.5 m grid-quantization tolerance). If a closer road surface exists, the slot
is skipped — the nearer edge's scan will claim it. O(1) per candidate, no extra data structure.

### 2. No-build flag zone tint — resolved
`ZoningSystem::no_build_mask: DataGrid<bool>` marks cells within 32 m of any `no_building_spawn`
road surface. The shader multiplies zone alpha by `(1 - no_build)` — zone tint disappears in the
building-depth strip alongside no-build roads. Mask recomputed by `update_no_build_mask(graph)`,
called automatically at the end of `update_distance_to_road` (road changes) and from
`set_no_building_spawn_internal` (player toggle). Uploaded as a 4th R8 texture (`no_build_tex`).

### 3. Occupied grid sync risk — resolved
The live synchronization contract is now documented in [`building_allocator.md`](building_allocator.md).
Zoning relies on allocator cleanup to clear occupied footprints before building removal and on full
reset paths to clear allocator and zoning state together.

### 4. Corner buildings
Not implemented (v0.01 scope). The live zoning system does not yet support dedicated corner-site
placement or dual-edge occupancy.

---

## 10. Proposed Future Zoning Schema (Not Yet Implemented)

Sections 1-9 describe the live zoning implementation. This section is a forward-looking proposal for a more game-friendly zoning model that can support demand-driven household growth, private building growth, and later building level-up without hardcoding every future subtype into the core UI or runtime.

The intent is to purge the old category-only zoning model as much as practical and replace it with the new profile-based model. Old-save compatibility is not a goal for this redesign.

### Design Goals

- keep the player-facing zoning UI simple and readable
- keep the internal representation flexible enough to add more subcategories later
- let zoning define what is legally allowed while demand decides whether growth actually happens
- allow multiple building families inside one broad subcategory such as medium-density housing
- keep the baseline `v1` profile set focused on residential, commercial, and industrial zoning
- allow later private-use families such as office or mixed only as explicit extensions

### Terminology Conventions

This section uses the following terms consistently:

- `top-level category`: the broad player-facing family such as `Residential` or `Industrial`
- `zoning subcategory`: the exact player-facing choice under that family, such as `Low Density Housing`
- `ZoneProfileId`: the authoritative painted zoning choice stored on the map
- `ZoneType`: the broad family derived from the referenced `ZoneProfile`
- `density`: the legal density band of the painted zoning choice
- `building level`: how far one concrete building has upgraded or downgraded within that legal band
- `build site`: one frontage-attached candidate roadside footprint anchored to one side of one road edge; in baseline `v1` it is identified by the road edge, side, leading frontage column, and footprint dimensions, with world-space center and facing derived from that attachment. This is a gameplay term, not a cadastral parcel system

### Player-Facing Model

The proposed player-facing zoning UI has two layers:

- top-level category
- zoning subcategory

Player-facing interpretation:

- the top-level category is a UI grouping such as `Residential` or `Industrial`
- the zoning subcategory is the actual zoning choice the player paints onto the map
- internally, one painted zoning subcategory maps to one authored `ZoneProfileId`

Top-level categories:

- `Residential`
- `Commercial`
- `Industrial`

The player first chooses a top-level category, then one of that category's zoning subcategories.

Examples of an initial v1 set:

| Top-level category | Initial zoning subcategories |
|---|---|
| `Residential` | `Low Density Housing`, `Medium Density Housing`, `High Density Housing` |
| `Commercial` | `Low Density Commercial`, `Medium Density Commercial`, `High Density Commercial` |
| `Industrial` | `Low Density Industrial`, `Medium Density Industrial`, `High Density Industrial` |

Important rule for the initial set:

- zoning subcategories stay broad
- `row housing` is not a separate initial zoning option
- a zoning subcategory such as `Medium Density Housing` may contain several valid building families

That means the player paints broad intent, while the simulation still has room for visual and economic variety inside the allowed band.

Concrete example:

- the player opens `Residential`
- the player chooses `Low Density Housing`
- the game paints the profile id for that exact choice, for example `res_low_housing`
- that profile says this is a `Residential` zone, with `Low` density, using the allowed low-density residential asset pool
- the cell does not need to store a second separate top-level residential value if the profile already says it is residential

### Internal Representation

The internal model should stay data-driven. The UI labels above should map to explicit zone profiles rather than to hardcoded special cases.

Recommended shape:

```text
ZoneType
  = Residential | Commercial | Industrial

ZoneDensity
  = Low | Medium | High

ZoneProfileId
  = stable authored id such as res_medium_housing

ZoneProfile
  - id
  - display_name
  - ui_order
  - zone_type
  - density
  - required_asset_tags
  - growth_profile_id
```

Interpretation:

- `id` is the stable authored profile identifier
- `zone_type` is the derived top-level category
- `density` is the broad legal density band of the painted zoning choice
- `ui_order` determines deterministic subcategory ordering inside one top-level category; ties sort by `id`
- `required_asset_tags` defines any extra asset-tag filters that must be present after the base zone-type and density match
- `growth_profile_id` points at the demand-owned growth evaluation profile described in [`demand.md`](demand.md); in baseline `v1`, this is a small closed shipped set with one default profile per shipped baseline `zone_type + density`

Authoritative runtime rule:

- the painted world grid should ultimately store `ZoneProfileId`, not both `ZoneProfileId` and `ZoneType`
- `ZoneType` should be derived from the referenced `ZoneProfile` whenever broad category logic needs it
- the current category-only painted grid is transitional and should be removed rather than preserved indefinitely

Concrete interpretation of the authoritative rule:

- if the player paints `Low Density Housing`, the cell stores the corresponding `ZoneProfileId`
- the runtime reads that profile and derives `ZoneType::Residential` from it
- any broad residential logic should read the profile's `zone_type`, not a second conflicting painted field
- this avoids drift between "what exact zone did the player paint?" and "what broad family does it belong to?"
- the legal `density` comes from that same profile, while any later building `level` still belongs to the spawned building instance rather than to the painted map cell

This keeps the model flexible:

- the initial implementation can ship only a small profile list
- later additions can add new subcategories such as row housing without redesigning the whole system
- the same mechanism works for later private-use families too if the design later adds them

### ZoneProfile Data And Loading

`ZoneProfile`s should be data-authored in shipped TOML files bundled with the game, following the
same source-of-truth pattern used by the economy data in [`economy.md`](economy.md).

Canonical `v1` data shape:

```text
zoning/
  profiles.toml
  profiles.index.bin   # optional derived cache
```

Canonical source-of-truth rules:

- `zoning/profiles.toml` is the authoritative authored data
- any compiled cache or index file is optional derived data only
- the base-game profile set ships with the game and does not live in the user directory
- a dedicated zoning-profile editor is not required for the first implementation
- hand-authored TOML is acceptable while the base profile set remains small and stable
- a future developer-facing editor may reuse existing tooling later, but exported TOML remains the authoritative data format

Recommended `profiles.toml` shape:

```toml
[[profiles]]
id = "res_low_housing"
display_name = "Low Density Housing"
ui_order = 10
zone_type = "residential"
density = "low"
required_asset_tags = []
growth_profile_id = "residential_low_default"

[profiles.ui]
color = "#2DBE60"
icon = "housing_low"
description = "Small detached and semi-detached housing."
```

Deterministic validation rules:

- every `id` must be globally unique
- every `display_name` must be non-empty
- every `ui_order` must be an integer `>= 0`
- every `zone_type` must decode to a known `ZoneType`
- every `density` must decode to a known `ZoneDensity`
- every `profiles.ui.color` must be a valid `#RRGGBB` hex colour
- every `profiles.ui.icon` must be a non-empty stable UI key
- every `profiles.ui.description` must be non-empty
- every `growth_profile_id` must resolve to a valid demand-owned `GrowthProfile`
- in baseline `v1`, every shipped `growth_profile_id` must be the one default demand profile that
  matches the profile's `(zone_type, density)`
- `required_asset_tags` must be stored as a deduplicated set or deduplicated during load

Deterministic runtime loading rules:

1. Read the shipped `zoning/profiles.toml` file during startup.
2. Validate the full file before creating the live zoning-profile registry.
3. If any shipped base-game profile is invalid, fail validation explicitly rather than silently dropping or rewriting profiles.
4. Build the runtime registry keyed by stable `ZoneProfile.id`.
5. Group profiles for the UI by `zone_type`.
6. Within each top-level category, sort profiles by `(ui_order, id)`.
7. Reserve compiled runtime profile id `0` for `unpainted / none`.
8. Assign compiled runtime profile ids `1..N` in the deterministic global order:
   top-level category order `Residential, Commercial, Industrial`, then
   `(ui_order, id)` inside each category.
9. Use that same compiled-id assignment for the painted grid, save/load blobs, undo payloads,
   and overlay uploads.
10. Use the `(ui_order, id)` sorted order whenever the player-facing zoning UI presents
    subcategories.

### Asset Legality Contract

The future zoning model should use the current building asset schema as the baseline legality
contract instead of inventing a second unrelated compatibility system.

Baseline asset-side legality inputs from [`asset_editor.md`](asset_editor.md):

- `asset_class`
- `building.zone_type`
- `building.density`
- shared asset `tags`
- `building.level`
- `upgrade_family` (currently stored as `asset_set` in the implemented asset format)
- `building.lot_width_cells` and `building.lot_depth_cells`

Deterministic baseline legality rule:

An asset is legal for a `ZoneProfile` if and only if all of the following are true:

1. `asset_class == "building"`
2. `building.zone_type == ZoneProfile.zone_type`
3. `building.density == ZoneProfile.density`
4. every tag in `ZoneProfile.required_asset_tags` is present in the asset's shared `tags`

Interpretation:

- `zone_type + density` are the hard zoning-legality keys
- `required_asset_tags` are secondary filters that narrow an already legal pool
- site-specific filters such as later corner detection are evaluated after this profile-legality step rather than by inventing new base zoning types

Not part of baseline profile legality:

- `building.level` is a growth-tier field, not a zoning-legality field
- `upgrade_family` is an upgrade-family field, not a zoning-legality field
- `lot_width_cells` and `lot_depth_cells` are site-fit requirements, not zone-profile legality keys
- capacities, `economy_profile`, and anchors affect runtime behavior after placement, not whether the profile may spawn that asset at all

Mixed-use note:

- baseline `v1` does not ship mixed-use profiles
- if mixed-use returns later, it should come back as an explicit extension with its own legality,
  demand, and asset-authoring rules written down at the same time

### Baseline Build-Site Definition

In the baseline replacement model, a `build site` is not a free-floating parcel. It is one
roadside building candidate attached to exactly one road edge and one side of that edge.

Deterministic `v1` build-site identity:

- `edge_idx`: the attached road edge
- `side`: which side of that edge the site occupies
- `leading_cell_x`: the first frontage column claimed on that edge side
- `width_cells` and `depth_cells`: the candidate building footprint dimensions in zoning cells
- `zone_profile_id`: the painted zoning profile that the entire candidate footprint must match

Derived runtime geometry:

- `frontage_t`: frontage position along the attached edge
- `center_2d`: world-space footprint center
- `facing_dir`: outward building-facing direction derived from the edge tangent and side

Deterministic `v1` legality rules:

- the parent edge must be buildable: not deleted, not `no_building_spawn`, and not degenerate
- the footprint must fit along the edge's available frontage columns
- every covered zoning cell in the candidate footprint must resolve to the same painted
  `zone_profile_id`
- the candidate footprint must not overlap another occupied building footprint
- the candidate footprint must not overlap road-owned space or fail the current "too close to the
  road" rejection
- baseline `v1` build sites attach to one edge only; corner or other multi-edge sites are later
  extensions

Deterministic discovery and fallback order:

- baseline legal-site discovery should enumerate candidate sites in ascending `edge_idx`, then side
  order `[1, -1]`, then ascending `leading_cell_x`, matching the current frontage allocator's scan
  order
- demand-owned growth evaluation may later score those legal sites however it wants
- if multiple legal sites remain exactly tied after demand scoring, the fallback tie-break should be
  that same discovery order

### Build-Site Ownership

`Build site` is a cross-subsystem runtime concept. It should not be owned by zoning alone.

Recommended ownership split:

- zoning owns the painted zoning profile on the map and the occupancy helpers used to reject
  overlapping building footprints
- the road/network subsystem owns the authoritative road-edge geometry, edge existence, and
  `no_building_spawn` policy that define whether roadside attachment is even possible
- asset data owns the footprint dimensions and any extra site-fit requirements such as tags
- the building allocator owns build-site discovery, frontage attachment, geometric fit checks, and
  final placement or removal
- demand owns the scoring and choice between already-legal build sites; it should not redefine the
  underlying geometry contract

Practical interpretation:

- zoning should answer "is this footprint legally zoned and already occupied?"
- roads should answer "can a building attach to this edge and where is the roadside geometry?"
- the allocator should answer "does this concrete frontage-attached site fit without overlapping
  roads or other buildings?"
- demand should answer "which legal site should change first?"

### Deterministic Asset Selection

After demand has chosen a legal build site, asset selection must stay deterministic.

Deterministic `v1` fresh-spawn selection pipeline:

1. Start from the assets that are already legal for the active `ZoneProfile`.
2. Keep only assets whose footprint and any site-specific tag filters fit the chosen build site.
3. Keep only `level = 1` assets for ordinary fresh spawn.
4. Group the remaining assets into building families:
   - if `upgrade_family` is present, that is the family key
   - if `upgrade_family` is absent, the asset behaves as a singleton family keyed by its qualified
     asset id
5. Build a deterministic family-preference order for the current roadside strip:
   - the strip key is `(zone_profile_id, edge_idx, side)`
   - sort family keys by a stable hash of `(strip key, family key)`
   - this family order must not depend on registry insertion order, file order, or runtime RNG
6. Try families in that deterministic order until one family has at least one candidate that fits
   the chosen build site.
7. Inside the chosen family, sort candidates by qualified asset id.
8. Pick one candidate from that sorted family-local list by a stable site hash of
   `(zone_profile_id, edge_idx, side, leading_cell_x, qualified_asset_id)`.

Interpretation:

- family choice is stable for one roadside strip, so most buildings along that strip prefer the same
  family and the zoning stays visually tight by default
- site-local hash inside the chosen family provides bounded deterministic variety between similar
  variants
- if the preferred family does not fit an awkward leftover site, the next family in the same stable
  family order acts as the deterministic fallback or infill family
- no asset-authored `selection_priority` field is needed in baseline `v1`

Deterministic upgrade and downgrade rule:

- upgrades and downgrades stay inside the current building's `upgrade_family`
- ordinary upgrade moves from `level = N` to `level = N + 1`
- ordinary downgrade moves from `level = N` to `level = N - 1`
- if a building has no `upgrade_family`, it has no family-based upgrade or downgrade path
- valid content should keep `level` unique within one family, so no extra tie-break is needed for a
  correct family-level transition
- crossing demand-side upgrade or downgrade thresholds only makes the next family level eligible;
  the economy-side viability gate described in [`economy.md`](economy.md) must still pass before
  the level change actually happens

Future explicit higher-level direct spawn:

- baseline `v1` fresh spawn does not skip directly to higher levels
- a later demand-owned or scenario-owned rule may explicitly request direct spawn at `level > 1`
- if that extension is added, it must choose the target spawn level before asset selection begins,
  then run the same deterministic family-selection and site-variant rules at that requested level

### Rezoning And Redevelopment

This rezoning contract applies only to ordinary private zoned buildings that were spawned from a
painted `ZoneProfile`.

Exclusions:

- future landmarks do not require painted zoning and do not use this rezoning path
- future city-owned service or utility buildings are explicit player-placed assets and do not use
  this rezoning path

Deterministic incompatible-rezoning rule:

- if the player repaints an occupied build site to a `ZoneProfile` that is incompatible with the
  building's current asset, the building enters `pending_redevelopment`
- set `rezone_grace_days_remaining = 3`
- decrement that timer once per deterministic daily redevelopment pass

While `pending_redevelopment` is active:

- the building may continue operating temporarily
- the building keeps its current residents, workers, and economy state
- the building may not upgrade
- if the building is removed for any reason during that grace period, the replacement must obey the
  current painted `ZoneProfile`, not the old one

Recovery rule:

- if the player repaints the site back to a compatible `ZoneProfile` before the grace timer expires,
  clear `pending_redevelopment` and reset the timer

Expiry rule:

- when `rezone_grace_days_remaining` reaches zero, remove the building in the deterministic daily
  redevelopment pass
- once removed, the site becomes an empty legal build site for the current `ZoneProfile`
- any later replacement or respawn is governed by the current zone plus ordinary demand and
  allocator rules

Important boundary:

- this short redevelopment grace applies only to zoning incompatibility
- road deletion, road rebuild, or other frontage-attachment invalidation is not a zoning event and
  should be handled by allocator-side attachment rules rather than by the rezoning timer

### Growth Contract

The intended ownership split for future zoning growth is:

- zoning profile decides what is legally allowed at a build site
- demand decides whether enough pressure exists for spawn, despawn, upgrade, or downgrade
- demand-owned growth evaluation reads local site conditions to decide which legal build sites are attractive enough to change
- economy decides whether the resulting building is actually viable once it exists

Practical rules:

- zoning should define the legal ceiling, not one exact building result
- demand should not directly pick one exact asset id
- zoning should not own demand weighting or local modifier weighting directly
- asset selection should choose from the legal asset pool for that zone profile
- upgrades should stay inside the same zone profile unless the player rezones
- crossing from one density band to another should usually require rezoning rather than happening silently
- household relocation, eviction, or affordability failure does not by itself change zoning legality
  or building level; any later building replacement still has to obey the current zone profile

Example:

- `Medium Density Housing` may allow townhouses, walk-ups, and small apartment blocks
- a level 1 and a level 3 medium-density building may both be valid in the same painted area
- demand and local conditions decide whether a site stays modest, upgrades, downgrades, or is replaced

### Office And Mixed As Later Extensions

Baseline `v1` should stay focused on residential, commercial, and industrial zoning only.

If office or mixed-use zoning returns later, it should return as an explicit extension rather than
as a half-specified baseline family.

Requirements for that later extension:

- define the player-facing category and subcategory contract explicitly
- add matching zoning-profile data and legality rules
- add matching demand-side formulas and `GrowthProfile` data
- add any asset-authoring or upgrade-family rules needed for that family

### Future Extensibility

The schema should allow later additions without breaking the core model.

Recommended extensibility rule:

- adding a new `ZoneProfile` should be normal content expansion
- adding a new top-level `ZoneType` should be possible, but rare
- a new `ZoneType` is justified only when the area's gameplay and growth behavior is fundamentally different from existing families rather than just a subtype of one of them

Corner-building direction:

- do not introduce a dedicated corner zoning type for the base system
- zoning should continue to define only the broad legal use and density
- corner-ness should be treated as a later build-site condition detected by the allocator
- asset selection may then prefer or require corner-capable assets through existing asset tags such as `corner_capable`, `requires_corner`, or `inline_only`
- runtime placement would still need explicit dual-edge occupancy and attachment support
- the world-grid model can support that later by marking `edge_occupancy` on both incident edges and calling `mark_occupied_rect` once for the combined footprint
- a concrete runtime implementation would likely need `corner: bool`, `secondary_edge_idx: usize`, and either a derived `corner_angle_rad: f32` cache or equivalent corner-geometry metadata on `Building` so asset selection and orientation can distinguish different corner shapes

Examples of later additions:

- `Medium Density Row Housing` under `Residential`
- `Transit-Oriented Mixed Use` under `Mixed`
- `Campus Office` under `Office`
- `Logistics Industry` under `Industrial`
- `Waterfront Commercial` or other special commercial forms

Examples of possible future top-level categories, if the game later needs them:

- `Agricultural`
- `Special District`
- `Entertainment` or `Tourism`

Those later additions should be implemented as new `ZoneProfile` entries, not as brand-new zoning architecture.

Explicit exclusion:

- future city-owned service or utility buildings remain outside this painted-zoning path even if the
  game later adds more painted private-use categories

### Relationship To Demand

This proposed zoning model is designed to work with the demand ownership described in [`demand.md`](demand.md).

Recommended relationship:

- demand computes broad household and private-building growth pressure
- in baseline `v1`, demand uses the fixed city-level signal normalization and `DemandChannel`
  formulas defined in [`demand.md`](demand.md) rather than zoning-authored weighting rules
- in baseline `v1`, the shipped `GrowthProfile` set stays intentionally small and closed, with one
  default profile per `zone_type + density`; new zoning profiles should normally reuse one of those
  defaults rather than create new demand behavior
- demand-owned `GrowthProfile` data tunes cadence, thresholds, hysteresis, and action budgets for
  that fixed evaluator
- zoning profiles define which kinds of buildings may answer that pressure
- in baseline `v1`, demand chooses among already-legal build sites through the deterministic
  candidate ordering and daily action-budget rules described in [`demand.md`](demand.md)
- building upgrades and downgrades should read sustained demand rather than one-frame spikes;
  later local modifiers may influence that pressure only if demand adds them explicitly

Raw local signals such as pollution, crime, education, parks, transit access, utility stability, and other neighborhood conditions should remain owned by their own simulation systems. Zoning and demand should consume summaries of those signals rather than becoming the source of truth for them.

### Replacement Direction From The Current Runtime

The current live runtime stores only `ZoneType` in the world grid and uses that broad type for placement and cleanup.

The replacement direction is:

- keep the current broad zone families as `ZoneType`
- introduce stable `ZoneProfileId` values for the actual player-painted zoning choice
- teach building placement, growth, cleanup, and UI to read the active profile instead of only the broad `ZoneType`
- remove category-only compatibility shims once the new model is live
- allow old saves to break instead of carrying a long-term migration burden for the replaced zoning format

The important rule is to keep the player-facing model stable:

- broad categories remain familiar
- subcategories add control without exploding the top-level toolbar
- the simulation remains free to add more profiles later

### Zoning Tool

The current live zoning tool is legacy and should be fully replaced once the profile-based zoning
model is implemented. This is a top-layer replacement, not an incremental extension of the current
rectangle-only tool.

Current live behavior:

- Godot owns a hardcoded zoning toolbar that maps directly to the current `ZoneType` values
  `0..5`
- the paint interaction is rectangle-only: one mouse drag produces one axis-aligned world-space
  rectangle
- the Godot-to-Rust write payload is rectangle-specific: `(x_min, z_min, x_max, z_max, zone_type)`
- the preview is one colored rectangle derived from the current broad `ZoneType`
- the tool keeps its own rectangle undo ring by capturing one raw sub-rectangle before paint and
  restoring it with a second rectangle-specific raw write call
- Rust also currently pushes a full `ZoningSystem` snapshot for the same paint operation; that
  duplicated zoning-undo path is legacy
- the overlay already uses a full-grid upload model, but the zoning layer itself is currently one
  R8 `ZoneType` texture with hardcoded broad-family colours
- save/load already persists the whole painted world grid as one blob, and that whole-grid
  persistence model should remain

Deterministic replacement contract:

- the replacement tool is driven by the loaded `ZoneProfile` registry rather than by hardcoded
  `ZoneType` button lists
- the player-facing UI still groups choices by top-level `ZoneType`
- within one top-level category, the subcategory list comes from the validated runtime
  `ZoneProfile` registry sorted by `(ui_order, id)`
- the baseline replacement supports at least `rectangle` and `brush` paint modes
- later modes such as fill or stamp may be added, but they must reuse the same generic patch-based
  edit API
- committed paint edits operate on snapped zoning-grid cell coordinates, not on unsnapped
  world-space floats

Canonical replacement paint payload:

- `grid_x`, `grid_y`, `width_cells`, `height_cells`: bounding box of the edited patch in zoning
  grid coordinates
- `write_mask`: one deterministic mask entry per cell in that bounding box; `1` means "this cell is
  written by this edit", `0` means "leave the existing cell unchanged"
- `target_profile_runtime_id`: the compiled runtime `ZoneProfile` id to paint into every masked
  cell

Canonical replacement undo payload:

- `grid_x`, `grid_y`, `width_cells`, `height_cells`: same bounding box as the forward paint
- `previous_profile_runtime_ids`: one stored runtime profile id per cell in that same box before
  the edit was applied

Brush rasterization rule:

- baseline `v1` brush size is an integer `radius_cells`
- the brush center always snaps to one zoning-grid cell center before any rasterization happens
- one brush stamp paints a deterministic integer round mask
- for one stamp, the brush paints offset cell `(dx, dy)` if and only if
  `dx * dx + dy * dy <= radius_cells * radius_cells`
- a drag stroke paints the union of those brush stamps along the deterministic supercover grid line
  between the previous snapped brush center and the new snapped brush center
- one committed drag produces one merged patch, one `write_mask`, and one undo entry
- preview smoothing or interpolation may be added later for visuals, but preview-only rendering must
  not change the committed painted cell set
- later UI support for multiple brush sizes is allowed, but every supported size must still use this
  same integer `radius_cells` rasterization rule

Tool/API rules:

- Godot should no longer call rectangle-specific bridge methods such as `set_zone_rect`,
  `get_zone_subrect`, or `set_zone_rect_raw`
- Rust should instead expose the following zoning-tool bridge methods:
  - `get_zone_profiles() -> Array[Dictionary]`
  - `capture_zoning_patch(grid_x, grid_y, width_cells, height_cells) -> PackedByteArray`
  - `apply_zoning_patch(grid_x, grid_y, width_cells, height_cells, target_profile_runtime_id, write_mask) -> void`
  - `restore_zoning_patch(grid_x, grid_y, width_cells, height_cells, profile_ids_le_u16) -> void`
- rectangle mode and brush mode differ only in how they generate `write_mask`; they do not get
  separate simulation-side storage or save rules
- ordinary paint uses `target_profile_runtime_id`
- erase uses the reserved runtime id `0`, which means `unpainted / none`

Bridge parameter contract:

- `grid_x`, `grid_y`, `width_cells`, and `height_cells` are integer zoning-grid coordinates or
  extents on the Godot boundary
- `target_profile_runtime_id` is passed over the Godot boundary as an integer, but Rust validates
  it against the compiled `u16` runtime profile-id registry
- `write_mask` is a `PackedByteArray` of length exactly `width_cells * height_cells`
- `write_mask` uses deterministic row-major ordering: `x` increases first inside one row, then `y`
  advances to the next row
- every `write_mask` byte must be either `0` or `1`
- `capture_zoning_patch()` returns `width_cells * height_cells * 2` bytes containing little-endian
  `u16` profile ids in that same row-major order
- `restore_zoning_patch()` accepts that same packed little-endian `u16` row-major payload and
  restores the entire patch bounding box exactly

Registry query contract:

- `get_zone_profiles()` returns the already validated profile registry in the deterministic order
  used by the UI and runtime-id assignment
- each returned entry must include at least:
  `id`, `runtime_id`, `display_name`, `ui_order`, `zone_type`, `density`,
  `ui_color`, `ui_icon`, and `ui_description`
- Godot uses that registry for zoning buttons, tooltips, preview colour selection, and overlay
  presentation metadata; it must not rebuild those lists from hardcoded `ZoneType` assumptions

Runtime-id rule:

- authored `ZoneProfile.id` stays a stable string key in TOML and UI data
- the live zoning grid stores a compiled dense runtime profile id
- baseline `v1` should use `u16` runtime profile ids
- runtime profile id `0` is reserved for `unpainted / none`
- non-zero runtime profile ids are assigned deterministically from the validated profile registry
  using the category and `(ui_order, id)` order defined above

Undo rule:

- after replacement, zoning paint uses one authoritative patch-based zoning undo path
- zone paint must not push full-grid zoning snapshots into the generic `SimCore` undo stack
- the current duplicated setup of "tool-local rectangle undo plus Rust full-zoning snapshot undo"
  is legacy and should be removed

Overlay rule:

- the full-grid upload model remains
- the current `distance_to_road`, `occupied`, and `no_build_mask` textures remain valid
- the separate zoning overlay mesh remains; the replacement does not recolour the terrain material
  directly
- the current single-channel broad-`ZoneType` overlay texture is replaced by profile-aware overlay
  data
- the authoritative overlay source is the compiled runtime `ZoneProfile`-id grid
- Rust exposes that grid to Godot as `get_zone_profile_texture_data_rg8() -> PackedByteArray`
- `get_zone_profile_texture_data_rg8()` packs one `u16` runtime profile id per zoning cell into two
  bytes in deterministic row-major order:
  low byte first, high byte second
- Godot uploads that payload as an `RG8` image with one texel per zoning cell
- the shader reconstructs the `u16` runtime profile id from the two normalized channels using
  rounded byte reconstruction; it does not read hardcoded broad `ZoneType` ids anymore
- Rust also exposes `get_zone_profile_style_lut_rgba8() -> PackedByteArray`
- that LUT contains one `RGBA8` texel per runtime profile id in ascending runtime-id order
- LUT entry `0` is reserved for `unpainted / none`
- LUT entries `1..N` derive their RGB values from `profiles.ui.color` in `zoning/profiles.toml`
- overlay colours, icons, tooltips, and other presentation data come from the loaded
  `zoning/profiles.toml` registry rather than from hardcoded `ZoneType` colour tables
- baseline presentation data should include at least `profiles.ui.color`, `profiles.ui.icon`, and
  `profiles.ui.description`

Save/load rule:

- the save format continues to persist the whole painted zoning world grid as one blob
- after replacement, that blob stores compiled `u16` runtime `ZoneProfile` ids in deterministic
  row-major order using little-endian encoding
- save/load does not preserve or reconstruct rectangle paint operations; it only persists the final
  painted grid state

Legacy code after replacement:

- the hardcoded `ZoneType` zoning button list in the current Godot UI
- the rectangle-only Godot zoning tool and its rectangle preview assumptions
- rectangle-specific Rust bridge methods such as `set_zone_rect`, `get_zone_subrect`, and
  `set_zone_rect_raw`
- the current rectangle-shaped GDScript zoning undo payload
- zone paint calling `push_undo_state(..., inc_zoning = true)`
- any hardcoded "zoning id `1..5` means fixed colour X" assumptions in the current tool layer

Those legacy paths should be removed rather than preserved behind compatibility shims.

## 11. Remaining Follow-Up Limitations

Section 10 now covers the core future zoning rules closely enough to act as the main implementation
spec. The remaining items below are mostly tooling, bridge, and content-authoring follow-up rather
than unresolved core zoning behavior.

### Asset-editor follow-up

- The only zoning-side contract here is that the asset editor must consume the shipped
  zoning-profile registry from `zoning/profiles.toml` instead of owning hardcoded zoning-choice
  lists.
- The deterministic editor behavior and UX rules for that integration belong in
  [`asset_editor.md`](asset_editor.md), not here.

## 12. Proposed Implementation Plan

The zoning, demand, and economy specs are now deterministic enough that implementation can proceed
in one deliberate cross-doc order. The intended order is:

1. zoning plus asset-editor changes
2. demand-layer changes
3. economy-side integration and replacement work

That order keeps the legal world model and authoring surface stable first, then moves city-growth
ownership into demand, and only after that finishes the deeper economy-side signal and viability
work those demand outputs consume.

### Phase 1: Zoning And Asset-Editor Foundation

Completed in the current implementation:

- `zoning/profiles.toml` is now the shipped source of truth for the broad initial
  `low / medium / high` residential, commercial, and industrial profile set.
- Rust-side `ZoneProfile` loading, validation, deterministic sorting, and compiled runtime id
  assignment are implemented.
- Read-only profile-registry queries are exposed to Godot so the UI and tools no longer need
  hardcoded zoning choices.
- The asset editor now loads that shared registry, replaces hardcoded zoning lists, writes
  deterministic `zone_type` and `density` choices from the loaded data, and validates authored
  zoned-building legality against the shipped registry.
- The asset editor now also follows the newer conditional building contract closely enough for
  baseline implementation, including `placement_mode`, explicit-building authoring, `service_class`,
  and conditional zoning-field export.
- Old editor-side `office` and `mixed` zoning controls were removed from the asset editor and live
  zoning-related tools so the tooling surface matches the shipped baseline registry.
- The painted zoning grid, helper bridge, and save-load format now store compiled `ZoneProfile`
  ids instead of broad `ZoneType`.
- The live zoning tool and overlay were replaced with the profile-driven brush/rectangle tool,
  patch bridge, and registry-driven UI described earlier in this document.
- Allocator legality, stale-building cleanup, and rezoning compatibility now consume the
  profile-based zoning contract, including the deterministic rezoning grace period for
  incompatible repainting.
- Deterministic fresh-spawn asset selection now follows the baseline family-order and site-variant
  rules: strip-stable family preference by `(zone_profile_id, edge_idx, side)` and site-local
  variant choice by stable hash of `(zone_profile_id, edge_idx, side, leading_cell_x,
  qualified_asset_id)`.

Pending to finish Phase 1 fully:

- none currently tracked inside Phase 1. Remaining broad-`ZoneType` helpers are now test-only
  migration coverage rather than part of normal profile-driven runtime behavior.

Exit condition:

- shipped profiles load deterministically at startup
- the asset editor can read the registry, no longer owns hardcoded zoning-choice lists, and
  validates authored zoning-related building fields against the shipped data
- editor and tool zoning-choice UIs expose only the shipped baseline categories instead of the old
  hardcoded office or mixed options
- the authoritative painted zoning state, live tool, save-load path, and allocator legality checks
  all use the same profile-based zoning contract
- deterministic asset selection matches the zoning-spec family-order and site-variant rules rather
  than only broad profile legality filtering
- transitional broad-`ZoneType` helpers are no longer used by normal runtime behavior

### Phase 2: Demand-Layer Integration

Completed in the current implementation:

- The demand-owned `GrowthProfile` registry is now shipped and `growth_profile_id` from zoning
  resolves against it.
- The baseline demand signal normalization, `DemandChannel` formulas, pioneer-demand bootstrap rules, and
  household admission or removal formulas from [`demand.md`](demand.md) are now implemented.
- The daily demand pass now runs from the settled snapshot handoff described in [`demand.md`](demand.md)
  instead of as a separate allocator-owned pressure pass.
- `utility_service_stability` no longer uses a permanent placeholder constant; the live runtime now
  derives it from settled building-level utility outcomes as the current baseline approximation.
- Ordinary household admission is now driven by the demand-owned `households_to_admit_today`
  output instead of allocator-owned immigration pressure logic.
- Private building spawn, despawn, upgrade, and downgrade decisions now execute from
  demand-owned daily building-action budgets and deterministic action plans in the live runtime.
- Fresh-map startup no longer uses allocator-owned founding placement; the live runtime now relies
  on the organic pioneer-demand bootstrap described in [`demand.md`](demand.md) to begin growth
  without a hidden placement exception.
- Silent runtime fallback capacities were removed; resident and worker capacities now come from
  authored asset data or resolve to zero if the asset data is missing.
- [`building_allocator.md`](building_allocator.md) is now aligned with the live profile-based
  legality, footprint-wide compatibility, rezoning grace, and demand-owned admission handoff.

Pending to finish Phase 2 fully:

- none currently tracked

Deferred extension:

- direct `level > 1` spawn remains a later explicit extension after the baseline demand loop works.

Exit condition:

- zoning defines legal envelopes
- demand owns household admission or removal plus private-building change decisions
- allocator executes profile-legal placements and removals against those demand outputs

### Phase 3: Economy Integration And Replacement

Completed in the current implementation:

- The settled economy-side source values that baseline demand consumes now come from one
  post-settlement daily snapshot: housing capacity and vacancy, housed-resident presence,
  affordability, stock stability, utility-service stability, reachable jobs, unhoused tracking,
  and the daily settlement handoff.
- Household relocation, eviction, rehousing from `unhoused`, and deterministic household-removal
  target selection are now implemented from the rules described in [`economy.md`](economy.md).
- Demand-owned residential building changes now pass through economy-side occupancy and household
  affordability gates authored in `economy/profiles.toml`.
- Demand-owned commercial and industrial building changes now pass through economy-side staffing,
  operating-buffer, and utility-availability gates authored in `economy/profiles.toml`.
- Industrial building changes now also pass through starter input-coverage and output-headroom
  gates backed by the live industrial input buffer plus output stock buffer.
- The operational-clock and daily-settlement handoff now runs in one stable order so demand reads
  one post-settlement snapshot per day instead of partial hourly state.

Pending to finish Phase 3 fully:

- none currently tracked

Exit condition:

- economy produces the settled daily snapshot that demand expects
- economy consumes demand-owned household and building change outputs directly from the daily demand
  pass
- zoning legality, demand ownership, and economy viability all meet cleanly in one runtime path

### Phase 4: Legacy Cleanup And Validation

Completed in the live runtime:

- the old rectangle-only zoning tool path, old hardcoded zoning lists, and obsolete zone-type-only
  UI/bridge calls are gone from the live zoning tool and overlay flow
- the remaining broad-`ZoneType` compatibility helpers were deleted from the zoning runtime once
  tests and save-load no longer needed them
- save/load and building legality now rely on authoritative `ZoneProfile` runtime ids; the cached
  broad `zone_type` field remains only as a derived hot-path cache on buildings
- allocator-owned immigration and other growth-decision leftovers were removed in favor of the
  demand-owned daily household and building action path
- ordinary household admission no longer inherits transport-oriented `TRANSIT_IMMIGRATING`
  defaults; optional border-origin transport visualization is now a separate path
- deferred `Office` and `Mixed` baseline runtime tables were removed from allocator indices,
  flow-field dirtying, and other shipped baseline runtime paths
- tests and save paths now exercise the profile-driven zoning contract directly instead of relying
  on broad-zone fallback helpers

Intentionally retained after the Phase 4 cleanup:

- `Building.zone_type` was successfully demoted from a legacy quasi-authoritative zoning field into
  an explicit derived broad-family hot-path cache beside authoritative `zone_profile_runtime_id`.
  It remains in the runtime by design and is no longer treated as a second legality source or an
  unresolved legacy seam.
- `ZoneType::Office` and `ZoneType::Mixed` were intentionally retained only as extension
  vocabulary in the broad enum and authored asset/profile mapping helpers. They are no longer part
  of any shipped baseline runtime table and are not treated as unresolved baseline legacy.
- `TRANSIT_IMMIGRATING` remains only for optional border-origin transport visualization and
  explicitly-authored exceptional arrivals. Ordinary household admission no longer depends on it.

Pending to finish Phase 4 fully:

- none currently tracked

Exit condition:

- fulfilled in the live baseline runtime: the old zoning toolchain, broad-`ZoneType`
  compatibility helpers, allocator-owned growth decisions, and baseline `Office`/`Mixed` runtime
  leftovers are gone from normal zoning-driven behavior

### Phase 5: Later Extensions

- Add district-style or family-preference systems if tighter authored neighborhood identity is
  needed.
- Add corner and other multi-edge build-site support.
- If later scenarios need a special founding-placement rule, add it as explicit scenario-owned
  setup rather than reintroducing allocator-owned bootstrap logic.
- Reintroduce office or mixed-use zoning only if their legality, demand, and growth-profile rules
  are specified together as one coherent extension.
- Keep the broad `ZoneType` enum vocabulary only as extension space for authored future content;
  baseline runtime tables should stay residential/commercial/industrial unless a full extension
  ships.
- Add any extra zoning subcategories beyond the broad initial `low / medium / high` baseline.
- If later gameplay needs authored zone-to-zone transition permissions, reintroduce explicit
  `upgrade_targets` or `downgrade_targets`-style `ZoneProfile` transition fields only when a real
  runtime system uses them.
- Keep the optional border-origin `TRANSIT_IMMIGRATING` transport path only for exceptional or
  explicitly-authored arrivals; ordinary household admission should remain separate from it.
- If later gameplay wants visible multi-mode household arrival and departure through road, rail,
  ship, air, or other external links, replace narrow `TRANSIT_IMMIGRATING` with shared
  `OutsideGateway` support plus `TRANSIT_EXTERNAL_ARRIVAL` and `TRANSIT_EXTERNAL_DEPARTURE` in
  [`entrance_and_exit.md`](entrance_and_exit.md), while keeping admission and removal counts owned
  by [`demand.md`](demand.md) and household-side state owned by [`economy.md`](economy.md).
- Replace the current building-loss displacement fallback with an explicit rehousing, unhoused,
  disaster, or removal contract instead of reusing ordinary entrance-travel states.
- Continue removing test and helper fixtures that rely on unresolved building manifests or
  under-specified asset capacities as the authored asset contract becomes stricter.

This phase is intentionally non-blocking for the first playable profile-based zoning replacement.

---

## 13. Road Ownership Gap and Nearest-Road Grid

### Problem

The allocator uses `distance_to_road` — distance to the **nearest road of any kind** — as an
implicit ownership proxy when deciding whether a building slot is valid. `resolve_slot` rejects
any slot whose building-centre distance to the nearest road is less than `half_depth`:

```rust
let road_dist = zoning.distance_to_road_world(center_2d.x, center_2d.y) as f32;
if road_dist < half_depth {
    return None;
}
```

This is incorrect because `distance_to_road` measures the globally nearest road, not the road
the slot belongs to. A slot may validly sit `half_depth` from its own road but be closer to a
neighbouring road, and the check rejects it anyway.

There is no `nearest_road_id` grid — no precomputed O(1) answer to "which road owns this cell?"
Any per-slot ownership resolution that reconstructs that answer is expensive and fragile.

Two cases where this breaks down for free-form road networks:

- **Parallel roads with a narrow gap (≤ 2 × building depth):** Both roads try to place buildings
  at `half_depth` offset from their own centrelines. Near the midpoint the building centre is
  within `half_depth` of the opposite road, so all deep slots are rejected by both sides. The gap
  stays empty even though one road should own those cells.
- **Angled junction (e.g. 110°):** The rectangular frontage strips from two roads overlap at the
  corner. Both roads scan the same world-space cells. The distance check is not directional and
  cannot determine which road owns a given cell.

### Design Invariant

For free-form roads (any angle, curves) **every zone cell must belong to exactly one road edge.**
Overlapping ownership claims are not acceptable.

### Fix: Multi-Source BFS Nearest-Road Grid

Add a new grid to `ZoningSystem`:

```rust
/// Edge index of the road whose surface is closest to each cell.
/// `u32::MAX` means no road within the configured ownership radius.
/// Derived runtime cache — not saved; recomputed from the road graph
/// alongside `distance_to_road` after every road edit.
pub nearest_road_id: DataGrid<u32>,
```

**Algorithm — localized BFS over the dirty region around the changed edge:**

On every road add, remove, or geometry change:

1. Compute the axis-aligned bounding box of the changed edge(s) and expand it by
   `max_zone_depth` on all sides. Clamp to grid bounds. This is the **dirty region**.
2. Clear `nearest_road_id` and `distance_to_road` to their default values inside the dirty
   region only.
3. Collect every road edge whose AABB intersects the dirty region. These are the **local seeds**.
4. Rasterize each local seed edge: mark every dirty-region cell whose centre falls within
   `edge_half_width` of that edge as a distance-zero seed, tagged with `edge_idx`.
5. Push all seeds into a bucket queue keyed by integer metre distance.
6. BFS outward from all seeds simultaneously, constrained to the dirty region, up to
   `max_zone_depth`.
7. Each cell receives the `(edge_idx, distance)` of the first wave that reaches it.
8. Tie-break: when two seeds reach a cell with equal distance, the lower `edge_idx` wins
   (deterministic).
9. Write results into both `nearest_road_id` and `distance_to_road`.

**Complexity:**

- O(dirty\_cells) per road edit, where `dirty_cells` is the area of the expanded edge AABB.
- For a typical 200 m road segment with 40 m max zone depth the dirty region is roughly
  280 × 85 ≈ 24 000 cells — orders of magnitude less than the full grid.
- Multiple simultaneous edits merge their dirty AABBs before the BFS runs.
- Replaces the current O(N\_edges × W × H) per-cell segment scan.
- On initial load, run once over the full grid (all edges are new). That single startup pass is
  O(W × H) and does not recur during normal play.

**Ownership check in `resolve_slot`:**

Replace the `distance_to_road < half_depth` rejection with an explicit ownership check over every
road-relative cell in the candidate footprint:

```rust
if zoning.nearest_road_id_world(cell_center.x, cell_center.y) != edge_idx as u32 {
    return None; // cell belongs to a different road
}
```

This is O(1) per cell. No Voronoi calculation at placement time.

**Effect on the two problem cases:**

| Case | Before | After |
|---|---|---|
| Parallel roads, 30 m gap | Deep slots rejected on both sides; gap stays empty | BFS assigns each road exactly 15 m; both sides fill their half cleanly |
| 110° junction | Both roads scan overlapping cells; ownership is undefined | Voronoi bisector partitions the junction; each cell belongs to exactly one road |

### What Does Not Change

- The world-space `DataGrid<u16>` zone painting grid is unchanged. Players still paint zones on
  the axis-aligned canvas.
- `distance_to_road` is still populated and still drives the shader road-proximity dimming.
- `EdgeOccupancy` per-edge frontage tracking is unchanged.
- Save / load format is unchanged — `nearest_road_id` is a derived runtime cache. It is
  recomputed on load from the road graph, exactly as `distance_to_road` is today.

### Visual Grid Note

The player-facing zone overlay renders an axis-aligned 10 m cell grid. Buildings are placed
road-relative and do not align to that grid. This is an intentional decoupling — the grid is a
zone-painting canvas, not a building-placement constraint. Whether to replace the overlay with a
road-relative visual is a separate UX decision outside the scope of this fix.

### Update Trigger

The existing `update_distance_to_road(graph)` call must be replaced by a combined
`update_road_ownership(graph)` that populates both `distance_to_road` and `nearest_road_id` in
one BFS pass. The call site and trigger (every road add, remove, or geometry change) are
unchanged.

### Disjoint Road Networks

The BFS is purely spatial and does not require topological connectivity. Two entirely separate
road networks on the same map are handled identically to two connected ones — each edge seeds
from its own surface and cells go to the nearest edge regardless of which network it belongs to.

- **Networks farther apart than `max_zone_depth`:** a dirty region from one network never
  reaches the other. Each network's `nearest_road_id` cells are updated independently and
  neither disturbs the other.
- **Networks within `max_zone_depth` of each other:** edges from both networks appear as local
  seeds inside the merged dirty region. The BFS assigns boundary cells to whichever edge is
  physically closer — correct Voronoi behavior, regardless of connectivity.

### `edge_idx` Stability On Road Removal

`nearest_road_id` stores raw `edge_idx` values. The road graph uses swap-remove semantics on
deletion: the removed edge's slot is filled by the last edge in the array, giving that edge a
new index. This invalidates any `nearest_road_id` cells that stored either the removed index or
the swapped-in index.

Required invariant: when a road removal triggers `compact_edges` or any index remap, the
ownership update must treat **both** the removed edge and the swapped-in edge as dirty. In
practice this means:

1. Before removal, record the AABB of the edge being removed.
2. After removal and remap, record the AABB of the edge that received the remapped index
   (the former last edge).
3. Union both AABBs, expand by `max_zone_depth`, and run one combined localized BFS over
   that merged dirty region.

This guarantees that cells previously owned by the removed edge get reassigned to their new
nearest road, and cells previously owned by the remapped edge have their stored index
corrected — without touching the rest of the grid.

---

## 14. `ZONE-01` Parcel Zoning Replacement Plan

This section owns the proposed replacement plan tracked as [`ZONE-01`](roadmap.md). It is not
implemented yet.

When `ZONE-01` is implemented, it supersedes the `nearest_road_id` grid work in Section 13 instead
of layering on top of it. Until the parcel model is implemented, Section 13 remains the documented
current-grid ownership gap.

### Goal

Replace the authoritative `DataGrid<u16>` painted zoning canvas with user-placed road-aligned
parcels.

The player-facing intent is:

- players place parcels beside roads
- a parcel may be created already zoned with the currently selected `ZoneProfile`
- a parcel may also be created free / unzoned, then painted later
- private zoned buildings may spawn only inside legal parcels
- Godot remains input and display only; Rust owns parcel geometry, legality, overlap checks,
  placement queries, save/load, and deterministic render payloads

### First-Slice Rules

Keep the first implementation deliberately narrow:

- default parcel size is `20 m` frontage by `30 m` depth
- one parcel attaches to exactly one road edge and one side
- one parcel is a road-aligned rectangle; its frontage edge follows the selected road side and its
  depth extends away from the road
- one parcel stores one `zone_profile_runtime_id`; `0` means free / unzoned
- one parcel may host at most one private zoned building
- parcels must not overlap each other
- parcels are private zoning land only; explicit city-owned service / utility buildings remain
  outside this painted-zoning path
- parcels are not engineered-ground clients in the first slice: they do not flatten terrain, own a
  support surface, generate cut / fill, or create terrain tie-in geometry

Deferred from the first slice:

- corner parcels
- irregular polygons
- merging and splitting
- category-specific default parcel sizes
- multi-building parcels
- automatic parcel subdivision
- arbitrary district or cadastral systems
- flat building pads, plot foundations, retaining faces, or other terrain-overriding parcel /
  building support surfaces; those must be tracked as separate earthworks work and extend
  [`earthworks.md`](earthworks.md)

Accepted first implementation decisions:

- old world-grid zoning saves do not require compatibility; the parcel rewrite may break old saves
  instead of migrating or reconstructing parcel state from the retired grid
- the first placement gesture is one click for one parcel
- curved-road parcels are straight rectangles built from the road tangent at the parcel center
- buildings sit at the road frontage inside the parcel; unused parcel depth remains rear space
- overlapping parcels are rejected outright, with no trimming, merging, min/max repair, or hidden
  geometry fixup
- selected `zone_profile_runtime_id = 0` creates a free / unzoned parcel
- selected `zone_profile_runtime_id != 0` creates a pre-zoned parcel

### Rust Data Model

The replacement should live under the zoning module near the code that owns it, for example:

```text
rust/src/simulation/grid/zoning/parcels.rs
```

Recommended first data shape:

```text
ParcelId
  - stable numeric id, not an array index

Parcel
  - id
  - edge_idx
  - side
  - frontage_center_t
  - frontage_m
  - depth_m
  - zone_profile_runtime_id
  - occupied_building: Option<usize>
  - corner keys / quantized boundary keys
  - world corners
  - aabb
```

Required identity rule:

- parcel boundary identity must use canonical quantized keys / stable IDs
- runtime array order must not become the authoritative identity
- if road edge indices are remapped by compaction, parcel edge references must be remapped through
  the same topology update path as building edge references

### Spatial Index

Parcel queries must be local.

Use a chunk-local parcel index aligned with existing world chunking instead of scanning every
parcel:

```text
parcel_chunks: HashMap<ChunkCoord, Vec<ParcelId>>
```

Required query bound:

- parcel lookup, overlap checks, and allocator candidate discovery are `O(local_parcels)` for the
  touched chunks
- no placement, cleanup, or demand pass may scan all parcels for every candidate site
- chunk membership is rebuilt only for parcels whose geometry changes

### Parcel Creation

Godot may send pointer input and selected profile id, but Rust must decide the final parcel.

Rust creation responsibilities:

1. find the hovered road edge and side
2. snap frontage to the road-owned geometry deterministically
3. build a `20 m x 30 m` road-aligned rectangle by default
4. compute canonical boundary keys and the parcel AABB
5. reject overlaps against local parcel chunks
6. reject invalid road attachment
7. assign a stable `ParcelId`
8. write the parcel into the parcel store and touched chunk index
9. return a parcel render / debug payload for Godot upload

Creating a free parcel uses `zone_profile_runtime_id = 0`. Creating a pre-zoned parcel uses the
currently selected non-zero runtime profile id.

### Parcel Editing

Minimum Rust-side bridge surface:

```text
preview_parcel_at(world_x, world_z, selected_profile_runtime_id)
create_parcel_at(world_x, world_z, selected_profile_runtime_id)
set_parcel_profile(parcel_id, zone_profile_runtime_id)
delete_parcel(parcel_id)
get_parcel_at_world(world_x, world_z)
get_parcel_render_payload()
```

Bridge rules:

- Godot must not compute parcel corners, parcel overlap, parcel-road ownership, or legality
- Godot uploads Rust-produced parcel render payloads
- preview may be visual-only, but the committed parcel must be generated by the same Rust rules as
  the accepted preview

### Allocator Contract

Building placement changes from "read every world-grid footprint cell" to "find one legal parcel".

A private zoned spawn candidate is legal only when:

1. the parcel is attached to a valid non-deleted road edge
2. the parcel is not occupied
3. the parcel has `zone_profile_runtime_id != 0`
4. the parcel profile accepts the candidate asset's `zone_type + density + tags`
5. the asset footprint fits inside the parcel:

```text
asset_width_m  = lot_width_cells * zone_cell_m
asset_depth_m  = lot_depth_cells * zone_cell_m
asset_width_m <= parcel.frontage_m
asset_depth_m <= parcel.depth_m
```

The allocator still owns final building placement, entrance-cache dirtiness, vacancy indices, and
building removal execution. Zoning owns parcel legality and parcel occupancy helpers.

### Demand Contract

Demand continues to decide whether private building changes should happen. It does not choose parcel
geometry or exact asset IDs.

Spawn candidates should be empty legal parcels sorted deterministically by:

```text
(edge_idx, side_order, frontage_center_t_key, parcel_id)
```

where `side_order` remains `[1, -1]`.

Fresh-spawn deterministic asset selection should replace the current grid-column key with parcel
identity:

```text
strip key: (zone_profile_runtime_id, edge_idx, side)
site key:  (zone_profile_runtime_id, parcel_id, qualified_asset_id)
```

### Building Lifecycle And Rezoning

Placed private zoned buildings should store `parcel_id` as their authoritative zoning attachment.

The building may retain derived runtime fields such as `edge_idx`, `side`, `frontage_t`, center,
and facing direction for rendering, entrances, pathing, and save/load convenience. Those fields must
not become a second zoning authority.

Rezoning rule:

- changing an occupied parcel to an incompatible profile starts the existing deterministic
  `pending_redevelopment` grace path
- changing it back to a compatible profile clears the redevelopment state
- when the grace expires, allocator removal frees the parcel for future demand-owned redevelopment

Road deletion or road-geometry invalidation remains allocator / attachment cleanup, not a rezoning
event.

### Save / Load

The replacement save format should persist parcels instead of the whole painted profile grid:

```text
zoning_parcels
  - parcel_id
  - edge_id
  - side
  - frontage_center_t
  - frontage_m
  - depth_m
  - zone_profile_runtime_id
```

Buildings should save their `parcel_id` for private zoned placement ownership.

Old world-grid zoning saves do not require compatibility for this redesign. The parcel save format
may break old saves instead of carrying a migration path from the retired painted profile grid.

### Overlay And Tooling

The zoning overlay becomes parcel-based:

- Rust emits parcel top faces, borders, selection state, and profile colors or profile ids for
  Godot upload
- Godot no longer draws an authoritative 10 m zoning cell grid
- any texture or mesh used for display is derived from the parcel store and is not a simulation
  authority
- profile registry, profile colors, icons, and category grouping remain valid

The first tool mode should support:

- create free parcel
- create pre-zoned parcel with selected profile
- select parcel
- set selected parcel profile
- delete selected parcel

### Legacy Paths To Remove

Once parcel zoning is live, remove or retire normal-runtime use of:

- `ZoningSystem.grid: DataGrid<u16>` as authoritative painted zoning
- profile-grid patch APIs
- grid-profile save/load blob
- grid-profile overlay texture as an authority source
- footprint-wide `get_zone_profile_runtime_id_world` placement checks
- grid-column keys in demand spawn candidate identity

Keep deliberately separate systems only if they still have a non-zoning purpose:

- profile registry and profile legality
- road-distance / no-build display data, if still useful for UI
- building occupancy indexing, if replaced by an equal or better parcel-owned occupancy path

### Minimum Validation

The first implementation should add focused tests for:

- creating a `20 m x 30 m` parcel on both sides of a straight road
- overlap rejection between adjacent or crossing parcels
- free parcels do not spawn private buildings
- pre-zoned parcels spawn compatible assets
- incompatible assets are rejected by profile legality
- occupied parcel rezoning enters and clears redevelopment grace deterministically
- parcel save/load preserves IDs, profile ids, and building ownership
- demand spawn candidate ordering is stable after unrelated parcel creation
- road edge compaction remaps parcel edge references deterministically
