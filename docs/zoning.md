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
    pub grid:             DataGrid<ZoneType>,  // 2000×2000, 1 byte/cell = 4 MB
    pub occupied:         DataGrid<bool>,       // 2000×2000, building footprints
    pub distance_to_road: DataGrid<u8>,         // 2000×2000, metres to nearest road (clamped 255)
    pub config:           MapConfig,
}
```

Cell size: **10 m**. A 20 km × 20 km map → 2000 × 2000 cells.

Coordinate conversion (world → cell):
```
cx = ((x / zone_cell_m) + hw).round() as usize   // hw = (width - 1) / 2
cy = ((z / zone_cell_m) + hh).round() as usize   // hh = (height - 1) / 2
```
Out-of-bounds returns `None`; callers skip the slot.

### `EdgeOccupancy` (`simulation/buildings/allocator.rs`)

```rust
pub struct EdgeOccupancy {
    pub cells_long: usize,
    pub left:  Vec<bool>,
    pub right: Vec<bool>,
}
```

`BuildingAllocator::edge_occupancy: HashMap<usize, EdgeOccupancy>` — fast O(1) same-road column
check before the rotated-rect world-grid test. Keys are remapped in `update_edge_indices` when
`compact_edges` runs.

### Zone types

| Value | Name | Colour |
|---|---|---|
| 0 | None | — |
| 1 | Residential | Green |
| 2 | Commercial | Blue |
| 3 | Industrial | Yellow-orange |
| 4 | Office | Purple |
| 5 | Mixed | Teal |

---

## 3. Public Rust API

All exposed via `#[func]` in `nodes/simulation_node.rs` and backed by `nodes/sim/editing.rs`:

| Method | Purpose |
|---|---|
| `set_zone_rect(x_min, z_min, x_max, z_max, zone_type: u8)` | Paint a world-space rectangle |
| `set_zone_rect_raw(x_min, z_min, x_max, z_max, bytes: PackedByteArray)` | Restore sub-rect (undo) |
| `get_zone_subrect(x_min, z_min, x_max, z_max) -> PackedByteArray` | Capture sub-rect (undo) |
| `get_zone_texture_data() -> PackedByteArray` | Full zone grid as flat u8 (R8 texture upload) |
| `get_occupied_texture_data() -> PackedByteArray` | Full occupied grid as flat u8 |
| `get_distance_texture_data() -> PackedByteArray` | Full distance grid as flat u8 |
| `get_no_building_spawn_edge_indices() -> PackedInt32Array` | Indices of flagged edges (overlay) |
| `set_no_building_spawn(edge_idx, enabled)` | Toggle no-build flag |
| `get_no_building_spawn(edge_idx) -> bool` | Read no-build flag |

`ZoningSystem::update_edge_indices` is a no-op kept for call-site compatibility with
`compact_edges` — the world grid has no edge-keyed data to remap.

---

## 4. Building Placement

Executed per candidate slot along each non-deleted, non-flagged road edge:

1. **Edge-occupancy pre-check** — `edge_occupancy[edge_idx].left[cell_x]` (or `.right`). O(1).
   Skips if the column on this side is already taken.
2. **Ownership check** — `distance_to_road_world(frontage_center) < 5`. O(1). If a closer road
   surface exists, this slot belongs to that road's scan; skip to avoid buildings facing the wrong road.
3. **Zone lookup** — `zoning.get_zone_world(center_x, center_z)`. O(1). Skips if `ZoneType::None`.
4. **Road-overlap rejection** — `zoning.distance_to_road_world(center_x, center_z) < half_depth`.
   Rejects buildings whose body would overlap a road carriageway.
4. **Desirability gate** — `desirability grid ≥ 20`. Skips low-quality locations.
5. **Desirability gate** — `desirability grid ≥ 20`. Skips low-quality locations.
6. **Rotated-rect occupancy check** — `zoning.is_rect_occupied(center_x, center_z, tangent, width, depth)`.
   Catches cross-road overlap and neighbouring buildings. O(footprint area).
7. **Place** — push `Building`, call `zoning.mark_occupied_rect(…, true)`, set `edge_occupancy` slot.

### Removal

A building is removed when any of these is true:
- Its `edge_idx` is deleted (`edge.deleted`)
- Its edge now has `no_building_spawn = true`
- `distance_to_road_world(center_x, center_y) < half_depth` (a new road was placed through it)
- `get_zone_world(center_x, center_y) != b.zone_type` (zone was erased or changed)

On removal:
- `zoning.mark_occupied_rect(…, false)` — clears the occupied footprint
- `edge_occupancy[edge_idx].left/right[cell_x] = false` — clears the column slot
- Swap-remove from `buildings` vec — the *moved* building's zone is dirtied so its flow field
  rebuilds with correct indices (see swap-remove bug fix, 2026-04-06)

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
Three `R8` textures (zone type, distance-to-road, occupied) are uploaded to a `ShaderMaterial`.

### Shader behaviour

| Input | Effect |
|---|---|
| Zone type byte | Colour LUT lookup |
| Distance ≤ 3 m (road carriageway) | Alpha → 0 (road asphalt masking) |
| Distance 3–40 m | Full zone alpha |
| Distance 40–100 m | Linear taper to 0.4× |
| Distance > 100 m | 0.2× (ghost — zoned but no road access) |
| Occupied byte = 1 | Alpha × 0.4 (building suppression) |
| Tool active | Alpha 0.38, procedural 10 m cell grid visible |
| Tool passive | Alpha 0.20, cell grid hidden |

### Texture update cadence

- **Zone type texture**: full re-upload after each paint operation (`set_zone_rect` → `get_zone_texture_data`).
- **Distance-to-road texture**: re-uploaded from `network_renderer.gd` after every road placement or removal.
- **Occupied texture**: re-uploaded every 30 frames from `buildings.gd`.

### No-build overlay

Orange `ImmediateMesh` lines are drawn along flagged edges when the zone tool is active.
Rebuilt by `_rebuild_no_build_overlay()` called from `set_tool_active()` and `full_refresh()`.

---

## 8. Save / Load

- Zone grid serialised as a single flat `BLOB` in `zoning_world_grid` SQLite table (width, height, data).
- `edge_occupancy` is **not** saved directly — it is rebuilt on load by replaying each building's
  frontage attachment.
- `occupied` grid is rebuilt on load by calling `mark_occupied_rect` for each loaded building.
- `distance_to_road` is rebuilt on load by `update_distance_to_road(graph)`.
- **Migration**: if `zoning_world_grid` table is absent but `zoning_grids` (old edge-local table) is
  present, load returns an empty grid. The player must repaint zones; old edge-local data cannot be
  faithfully migrated.

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
There is exactly one production removal path: the eviction loop in `allocator.rs::tick`. It always
calls `mark_occupied_rect(…, false)` and clears the `edge_occupancy` slot before the `swap_remove`.
The swap-remove remap dirties the moved building's zone so flow fields rebuild with correct indices
and the footprint is never double-cleared. `allocator.clear()` is always preceded by
`zoning.clear()` (in `network::clear`), so no occupied cells are left stranded on full reset.

### 4. Corner buildings
Not implemented (v0.01 scope). The world-grid model supports them naturally when added:
mark `edge_occupancy` on both incident edges and call `mark_occupied_rect` once for the combined
footprint. Requires `corner: bool` and `secondary_edge_idx: usize` on `Building`.
