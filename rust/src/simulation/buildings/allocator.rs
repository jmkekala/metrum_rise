//! Building placement and lifecycle management.
//!
//! [`BuildingAllocator::tick`] runs once per simulation tick. It:
//! 1. Removes buildings whose zoning cell has been changed or whose road edge was deleted.
//! 2. Scans zoned, unoccupied cells with sufficient demand and spawns new buildings.
//! 3. Spawns immigrant agents up to the current residential capacity.

use crate::assets::{AssetRegistry, ZoneClass};
use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::NodeType;
use godot::prelude::Vector2;
use std::collections::HashMap;

/// A placed building occupying a variable-footprint area on a zoning grid.
#[derive(Clone)]
pub struct Building {
    /// World-space X centre of the building footprint (metres, ground-plane X axis).
    pub center_x: f32,
    /// World-space Z centre of the building footprint (metres, ground-plane Z axis; legacy `*_y` field name).
    pub center_y: f32,
    /// Width of the footprint in zoning grid cells.
    pub width_cells: u16,
    /// Depth of the footprint in zoning grid cells.
    pub depth_cells: u16,
    /// Zone category this building was spawned into.
    pub zone_type: ZoneType,
    /// Unit vector pointing from the road toward the building (outward normal from the road edge).
    pub facing_dir: Vector2,
    /// T-coordinate (0.0 to 1.0) along the road edge [`edge_idx`] for this building's frontage.
    pub frontage_t: f32,
    /// Signed side of the road: `+1.0` = left, `-1.0` = right (relative to edge direction).
    pub side_offset: f32,
    /// Ticks since this building lost its zoning. Currently unused.
    pub abandoned_timer: u32,
    /// Index into [`RegionGraph::edges`] for the road segment this building fronts.
    pub edge_idx: usize,
    /// Road side: `1` = left, `-1` = right.
    pub side: i8,
    /// Column index (along the road) of the building's leading cell.
    pub cell_x: usize,
    /// Depth offset of the building's leading cell (0 = frontage row).
    pub cell_y: u16,
    /// Total agents currently residing or working in this building.
    pub occupancy: u32,
    /// Qualified asset ID (`"pack_id:asset_id"`) identifying the model for this building.
    /// Changes when the building upgrades to a higher tier.
    pub asset_id: String,
    /// Current growth tier, derived from the manifest at placement and incremented on upgrade.
    pub level: u8,
}

/// Manages the full lifecycle of [`Building`]s: placement, removal, and immigrant spawning.
#[derive(Clone)]
pub struct BuildingAllocator {
    /// All currently placed buildings. Removal uses `swap_remove` — order is not preserved.
    pub buildings: Vec<Building>,
    /// Set to `true` when the building list changes in a tick, signalling renderers to refresh.
    pub dirty: bool,
    /// Per-edge frontage occupancy tracker. Keyed by edge index.
    /// Avoids redundant world-grid checks for the common same-road case.
    pub edge_occupancy: HashMap<usize, EdgeOccupancy>,
    /// Inverted index: `zone_index[ZoneType as usize]` contains building indices (Bug B16).
    pub zone_index: [Vec<usize>; 6],
    /// Inverted index: `vacancy_index[ZoneType as usize]` contains indices of buildings with occupancy < 6 (Bug B16a).
    pub vacancy_index: [Vec<usize>; 6],
    /// Tracks the position of each building in its respective `vacancy_index` list for O(1) removal.
    /// Indexed by building ID; `usize::MAX` if not in any vacancy list.
    pub vacancy_pos: Vec<usize>,
    /// Recalculates inverted indices if true.
    pub dirty_index: bool,
    /// Per-zone dirty flags set when buildings of a zone type are spawned or removed.
    /// Drained by `SimCore::simulate_tick_internal` to mark flow fields for rebuild.
    pub dirty_zones: [bool; 6],
    /// Registry of all loaded pack assets. Populated at startup by scanning the mods directory.
    pub registry: AssetRegistry,
}

/// Tracks which frontage columns along a road edge are claimed by placed buildings.
///
/// One entry per edge side (left/right). Indexed by column number along the road (cell_x).
#[derive(Clone)]
pub struct EdgeOccupancy {
    /// Number of columns along this road edge.
    pub cells_long: usize,
    /// True if a building has its frontage in this column on the left side.
    pub left: Vec<bool>,
    /// True if a building has its frontage in this column on the right side.
    pub right: Vec<bool>,
}

/// Returns the road node an agent departing from `building` should walk toward.
///
/// Picks the nearer endpoint: start_node when `frontage_t < 0.5`, end_node otherwise.
/// This replaces the old `frontage_node` stored field, computing it on demand from the
/// graph so that no split node needs to be inserted into the road graph.
pub(crate) fn building_depart_node(building: &Building, graph: &RegionGraph) -> u32 {
    let edge = graph.edge(building.edge_idx);
    if building.frontage_t < 0.5 {
        edge.start_node
    } else {
        edge.end_node
    }
}

/// Converts a simulation [`ZoneType`] to the corresponding [`ZoneClass`] used by the asset registry.
/// Returns `None` for `ZoneType::None`.
fn zone_type_to_zone_class(zt: ZoneType) -> Option<ZoneClass> {
    match zt {
        ZoneType::Residential => Some(ZoneClass::Residential),
        ZoneType::Commercial  => Some(ZoneClass::Commercial),
        ZoneType::Industrial  => Some(ZoneClass::Industrial),
        ZoneType::Office      => Some(ZoneClass::Office),
        ZoneType::Mixed       => Some(ZoneClass::Mixed),
        ZoneType::None        => None,
    }
}

impl BuildingAllocator {
    /// Remaps all building edge indices after a road network compaction.
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        let old_len = self.buildings.len();
        for b in &mut self.buildings {
            if let Some(&new_id) = mapping.get(&b.edge_idx) {
                b.edge_idx = new_id;
            } else {
                b.edge_idx = usize::MAX;
            }
        }
        self.buildings.retain(|b| b.edge_idx != usize::MAX);
        if self.buildings.len() != old_len {
            self.dirty = true;
            self.dirty_index = true;
        }
        // Remap edge_occupancy keys.
        let mut new_occ = HashMap::new();
        for (old_idx, occ) in self.edge_occupancy.drain() {
            if let Some(&new_id) = mapping.get(&old_idx) {
                new_occ.insert(new_id, occ);
            }
        }
        self.edge_occupancy = new_occ;
    }

    /// Creates an empty allocator.
    pub fn new() -> Self {
        Self {
            buildings: Vec::new(),
            dirty: false,
            edge_occupancy: HashMap::new(),
            zone_index: [const { Vec::new() }; 6],
            vacancy_index: [const { Vec::new() }; 6],
            vacancy_pos: Vec::new(),
            dirty_index: true,
            dirty_zones: [false; 6],
            registry: AssetRegistry::new(),
        }
    }

    /// Removes all buildings and resets the dirty flag.
    pub fn clear(&mut self) {
        self.buildings.clear();
        self.edge_occupancy.clear();
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        self.dirty = false;
        self.dirty_index = false;
    }

    /// Advances the building lifecycle by one simulation tick.
    ///
    /// Removes stale buildings, grows new ones into high-demand zones, and spawns immigrants.
    /// Calls `network.rebuild_pathing()` once if any building was added or removed.
    pub fn tick(
        &mut self,
        _demand: &mut crate::simulation::economy::demand::DemandSystem,
        zoning: &mut ZoningSystem,
        desirability: &crate::simulation::grid::desirability::DesirabilitySystem,
        _noise: &crate::simulation::grid::noise::NoiseSystem,
        _agents: &mut crate::simulation::economy::agents::AgentSystem,
        network: &mut crate::simulation::network::TransitNetwork,
        graph: &mut RegionGraph,
        config: &crate::simulation::core::config::MapConfig,
    ) {
        let mut spawned_this_tick = 0;
        let max_spawns = 10;

        // 1. Cleanup: Remove buildings if their zone changed or their road edge was deleted.
        let zone_cell_m = zoning.config.zone_cell_m;
        let mut i = 0;
        while i < self.buildings.len() {
            let b = &self.buildings[i];
            let remove = {
                let edge_ok = b.edge_idx < graph.edge_count() && !graph.edge(b.edge_idx).deleted;
                if !edge_ok {
                    true
                } else if graph.edge(b.edge_idx).no_building_spawn {
                    // Edge was flagged no-build after this building was placed — evict it.
                    true
                } else {
                    // Check whether a road now runs through the building's body.
                    // Building center sits ~(SIDEWALK_WIDTH + half_depth) from its own road edge,
                    // so a value less than half_depth means a different road is inside the footprint.
                    let half_depth = b.depth_cells as f32 * zone_cell_m * 0.5;
                    let road_dist = zoning.distance_to_road_world(b.center_x, b.center_y) as f32;
                    if road_dist < half_depth {
                        true
                    } else {
                        // Check zone type at the building's frontage world position.
                        let current_zone = zoning.get_zone_world(b.center_x, b.center_y);
                        current_zone != b.zone_type
                    }
                }
            };

            if remove {
                let b_edge_idx = b.edge_idx;
                let b_side = b.side;
                let b_cell_x = b.cell_x;
                let b_center_x = b.center_x;
                let b_center_y = b.center_y;
                let b_facing = b.facing_dir;
                let b_width = b.width_cells;
                let b_depth = b.depth_cells;
                let b_zone = b.zone_type;
                self.dirty_zones[b_zone as usize] = true;

                // Clear world-grid occupancy for the entire rotated-rect footprint.
                // facing_dir points outward from road (= normal), tangent is perpendicular.
                let tangent = Vector2::new(-b_facing.y, b_facing.x);
                let width_m  = b_width as f32 * zone_cell_m;
                let depth_m  = b_depth as f32 * zone_cell_m;
                zoning.mark_occupied_rect(b_center_x, b_center_y, tangent, width_m, depth_m, false);

                // Clear per-edge frontage slot.
                if let Some(occ) = self.edge_occupancy.get_mut(&b_edge_idx) {
                    let slot = if b_side > 0 { &mut occ.left } else { &mut occ.right };
                    if b_cell_x < slot.len() { slot[b_cell_x] = false; }
                }

                let _ = (b_side, b_cell_x); // suppress unused warnings
                let last_idx = self.buildings.len() - 1;
                if i < last_idx {
                    // The building at last_idx will be moved into slot i by swap_remove.
                    // Its zone's flow field holds `nearest_building[v] = last_idx`, which
                    // will be out of bounds after the removal. Mark it dirty so the flow
                    // field is rebuilt with the corrected index on the next frame.
                    self.dirty_zones[self.buildings[last_idx].zone_type as usize] = true;
                    let mut mapping = std::collections::HashMap::new();
                    mapping.insert(last_idx, i);
                    _agents.remap_building_indices(&mapping);
                }

                self.buildings.swap_remove(i);
                self.dirty_index = true;
            } else {
                i += 1;
            }
        }

        // 2. Growth Logic: scan road frontage cells for unoccupied zoned land.
        'edge_loop: for edge_idx in 0..graph.edge_count() {
            if spawned_this_tick >= max_spawns {
                break;
            }
            let edge = graph.edge(edge_idx);
            if edge.deleted || edge.no_building_spawn || edge.physical_length < 0.1 || edge.physical_geometry.len() < 2 {
                continue;
            }
            let edge_len   = edge.physical_length;
            let edge_width = edge.width;
            let cells_long = (edge_len / zone_cell_m).floor() as usize;
            if cells_long == 0 { continue; }

            'side_loop: for side in [1i8, -1i8] {
                for i in 0..cells_long {
                    // Alternate scan direction for visual variety.
                    let x = if i % 2 == 0 { (i / 2).min(cells_long - 1) } else { (cells_long - 1).saturating_sub(i / 2) };
                    if spawned_this_tick >= max_spawns {
                        break 'edge_loop;
                    }

                    // Fast pre-check: is the frontage column already taken by a building on this edge?
                    if let Some(occ) = self.edge_occupancy.get(&edge_idx) {
                        let slot = if side > 0 { &occ.left } else { &occ.right };
                        if x < slot.len() && slot[x] { continue; }
                    }

                    // Compute world position of frontage cell centre (row 0).
                    let t_col = (x as f32 + 0.5) * zone_cell_m / edge_len;
                    let world_pos = self.get_pos_on_edge(graph, edge_idx, t_col);
                    let tangent   = self.get_tangent_on_edge(graph, edge_idx, t_col);
                    let normal    = Vector2::new(tangent.y, -tangent.x) * (side as f32);
                    let curb_dist = edge_width * 0.5 + crate::config::SIDEWALK_WIDTH;
                    let frontage_center = world_pos + normal * (curb_dist + zone_cell_m * 0.5);

                    // Ownership check: if a closer road surface exists, that road's scan will
                    // claim this slot. Skip to avoid buildings facing the wrong road.
                    // Expected distance from frontage_center to this edge's surface:
                    //   SIDEWALK_WIDTH + zone_cell_m * 0.5 = 1.5 + 5.0 = 6.5 m.
                    // Threshold is 5 m (1.5 m tolerance for grid quantization).
                    {
                        let road_dist = zoning.distance_to_road_world(frontage_center.x, frontage_center.y);
                        let expected: u8 = (crate::config::SIDEWALK_WIDTH + zone_cell_m * 0.5 - 1.5) as u8;
                        if road_dist < expected {
                            continue;
                        }
                    }

                    let z_type = zoning.get_zone_world(frontage_center.x, frontage_center.y);
                    if z_type == ZoneType::None {
                        continue;
                    }

                    let demand = match z_type {
                        ZoneType::Residential => _demand.residential,
                        ZoneType::Commercial  => _demand.commercial,
                        ZoneType::Industrial  => _demand.industrial,
                        ZoneType::Office      => _demand.commercial * 0.5,
                        ZoneType::Mixed       => (_demand.residential + _demand.commercial) * 0.5,
                        _ => 0.0,
                    };
                    if demand < 10.0 { continue; }

                    // Select a registered asset for this zone.
                    let zone_class = zone_type_to_zone_class(z_type);
                    let candidates = zone_class.map(|zc| self.registry.buildings_for_zone(zc)).unwrap_or(&[]);
                    if candidates.is_empty() { continue; }
                    let asset_id = candidates[(edge_idx ^ x) % candidates.len()].clone();
                    let (dw, dh) = self.registry.lot_size(&asset_id);

                    // Footprint must fit within remaining columns.
                    if x + dw > cells_long { continue; }

                    // Compute footprint centre in world space.
                    let t_center = (x as f32 + dw as f32 * 0.5) * zone_cell_m / edge_len;
                    let world_pos_on_edge = self.get_pos_on_edge(graph, edge_idx, t_center);
                    let tangent_c = self.get_tangent_on_edge(graph, edge_idx, t_center);
                    let normal_c  = Vector2::new(tangent_c.y, -tangent_c.x) * (side as f32);
                    let depth_offset = crate::config::SIDEWALK_WIDTH + (dh as f32 * 0.5) * zone_cell_m;
                    let center_2d = world_pos_on_edge + normal_c * (edge_width * 0.5 + depth_offset);

                    // Zone check: all footprint cells must share the same zone type.
                    let mut can_build = true;
                    'zone_check: for dx in 0..dw {
                        let t_dx = (x as f32 + dx as f32 + 0.5) * zone_cell_m / edge_len;
                        let wp = self.get_pos_on_edge(graph, edge_idx, t_dx);
                        let td = self.get_tangent_on_edge(graph, edge_idx, t_dx);
                        let nd = Vector2::new(td.y, -td.x) * (side as f32);
                        for dy in 0..dh {
                            let cell_center = wp + nd * (curb_dist + (dy as f32 + 0.5) * zone_cell_m);
                            if zoning.get_zone_world(cell_center.x, cell_center.y) != z_type {
                                can_build = false;
                                break 'zone_check;
                            }
                        }
                    }
                    if !can_build { continue; }

                    // Rotated-rect occupancy check on the world grid.
                    let width_m = dw as f32 * zone_cell_m;
                    let depth_m = dh as f32 * zone_cell_m;
                    if zoning.is_rect_occupied(center_2d.x, center_2d.y, tangent_c, width_m, depth_m) {
                        continue;
                    }

                    // Reject if the building centre is inside another road's carriageway.
                    // A legitimately placed building sits (SIDEWALK_WIDTH + half_depth) from its
                    // own road edge, so distance_to_road ≥ SIDEWALK_WIDTH there. Using half_depth
                    // as the threshold catches any candidate whose body overlaps a road surface.
                    {
                        let half_depth = depth_m * 0.5;
                        let road_dist = zoning.distance_to_road_world(center_2d.x, center_2d.y) as f32;
                        if road_dist < half_depth {
                            continue;
                        }
                    }

                    // B5: Desirability Gate.
                    {
                        let (gx_raw, gy_raw) = config.world_to_env_grid(
                            center_2d.x, center_2d.y,
                            desirability.grid.width, desirability.grid.height,
                        );
                        let gx = (gx_raw.round() as usize).min(desirability.grid.width.saturating_sub(1));
                        let gy = (gy_raw.round() as usize).min(desirability.grid.height.saturating_sub(1));
                        if *desirability.grid.get(gx, gy).unwrap_or(&50.0) < 20.0 {
                            continue;
                        }
                    }

                    // All checks passed — place the building.
                    let frontage_t_full = t_center;

                    // Mark world-grid cells as occupied.
                    zoning.mark_occupied_rect(center_2d.x, center_2d.y, tangent_c, width_m, depth_m, true);

                    // Mark per-edge frontage slot.
                    let occ = self.edge_occupancy.entry(edge_idx).or_insert_with(|| EdgeOccupancy {
                        cells_long,
                        left:  vec![false; cells_long],
                        right: vec![false; cells_long],
                    });
                    let slot = if side > 0 { &mut occ.left } else { &mut occ.right };
                    if x < slot.len() { slot[x] = true; }

                    let initial_level = self.registry.get(&asset_id)
                        .and_then(|e| e.manifest.building.as_ref())
                        .map(|b| b.level)
                        .unwrap_or(1);

                    self.buildings.push(Building {
                        zone_type: z_type,
                        facing_dir: normal_c,
                        frontage_t: frontage_t_full,
                        side_offset: edge_width * 0.5 + crate::config::SIDEWALK_WIDTH,
                        center_x: center_2d.x,
                        center_y: center_2d.y,
                        edge_idx,
                        side,
                        cell_x: x,
                        cell_y: 0,
                        width_cells: dw as u16,
                        depth_cells: dh as u16,
                        occupancy: 0,
                        asset_id,
                        level: initial_level,
                        abandoned_timer: 0,
                    });

                    spawned_this_tick += 1;
                    self.dirty_index = true;
                    self.dirty_zones[z_type as usize] = true;

                    match z_type {
                        ZoneType::Residential => _demand.residential -= 5.0,
                        ZoneType::Commercial  => _demand.commercial  -= 5.0,
                        ZoneType::Industrial  => _demand.industrial  -= 5.0,
                        _ => {}
                    }

                    break 'side_loop;
                }
            }
        }

        // 3. Upgrade pass: buildings whose demand + desirability conditions are met level up
        //    in-place when a higher-tier asset in the same family is registered.
        for b in &mut self.buildings {
            let Some(next_id) = self.registry.next_level(&b.asset_id) else { continue };
            // Upgrade when demand is high and the city grid says the area is desirable.
            let demand_ok = match b.zone_type {
                ZoneType::Residential => _demand.residential > 50.0,
                ZoneType::Commercial  => _demand.commercial  > 50.0,
                ZoneType::Industrial  => _demand.industrial  > 50.0,
                _ => false,
            };
            if !demand_ok { continue; }
            let (gx_raw, gy_raw) = config.world_to_env_grid(
                b.center_x, b.center_y,
                desirability.grid.width, desirability.grid.height,
            );
            let gx = (gx_raw.round() as usize).min(desirability.grid.width.saturating_sub(1));
            let gy = (gy_raw.round() as usize).min(desirability.grid.height.saturating_sub(1));
            if *desirability.grid.get(gx, gy).unwrap_or(&0.0) < 60.0 { continue; }

            b.asset_id = next_id.to_owned();
            b.level = self.registry.get(&b.asset_id)
                .and_then(|e| e.manifest.building.as_ref())
                .map(|bd| bd.level)
                .unwrap_or(b.level + 1);
            self.dirty = true;
            self.dirty_zones[b.zone_type as usize] = true;
        }

        network.rebuild_pathing_if_dirty(graph);

        if self.dirty_index {
            self.rebuild_zone_index();
        }

        // 3. Immigration Logic
        //
        // Agents only spawn if the player has built at least one road to the map boundary and
        // confirmed it as an external connection (NodeType::Border). A border node with no
        // connected edges (i.e. its road was later deleted) is skipped.
        let total_capacity: usize = self
            .buildings
            .iter()
            .filter(|b| b.zone_type == ZoneType::Residential || b.zone_type == ZoneType::Mixed)
            .fold(0, |acc, b| {
                let cap = self.registry.capacity(&b.asset_id).max(1) as usize;
                acc + cap.saturating_sub(b.occupancy as usize)
            });

        if _agents.len() < total_capacity {
            let demand_factor = (_demand.residential / 100.0).max(0.0).min(1.0);
            let gap = total_capacity - _agents.len();
            let num_to_spawn = ((gap as f32 * 0.2 * demand_factor) as usize).max(1).min(10);

            // Collect all connected Border nodes as valid spawn points.
            let border_nodes: Vec<u32> = graph
                .nodes()
                .iter()
                .enumerate()
                .filter_map(|(i, node)| {
                    if node.node_type != NodeType::Border {
                        return None;
                    }
                    // Only spawn from nodes that still have at least one live incident edge.
                    let connected = graph
                        .node_adjacency(i as u32)
                        .iter().any(|&e| !graph.edge(e).deleted);
                    if connected { Some(i as u32) } else { None }
                })
                .collect();

            if !border_nodes.is_empty() {
                let mut rng = rand::thread_rng();
                for _ in 0..num_to_spawn {
                    if let Some(home_idx) = _agents.find_available_home(self) {
                        let spawn_node =
                            border_nodes[rand::Rng::gen_range(&mut rng, 0..border_nodes.len())];
                        let mut spawn_pos = graph.node(spawn_node).pos;
                        
                        // Offset the spawn position to the correct lane center (RHT/LHT)
                        if let Some(&edge_idx) = graph.node_adjacency(spawn_node).get(0) {
                            let edge = graph.edge(edge_idx);
                            if edge.physical_geometry.len() >= 2 {
                                let dir = if edge.start_node == spawn_node {
                                    (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized()
                                } else {
                                    (edge.physical_geometry[edge.physical_geometry.len()-2] - edge.physical_geometry[edge.physical_geometry.len()-1]).normalized()
                                };
                                let side_mul = if crate::config::DRIVE_ON_LEFT { -1.0 } else { 1.0 };
                                let normal = godot::prelude::Vector3::new(-dir.z, 0.0, dir.x);
                                spawn_pos += normal * (crate::config::LANE_WIDTH * 0.5 * side_mul);
                            }
                        }

                        let home_bldg = &self.buildings[home_idx];
                        let home_node = building_depart_node(home_bldg, graph);

                        _agents.spawn_agent(
                            home_idx,
                            home_node,
                            0.0,
                            0.0,
                            spawn_node,
                            spawn_pos.x,
                            spawn_pos.z,
                        );
                    } else {
                        break; // No more homes available
                    }
                }
            }
            // No border nodes → immigration is blocked until the player builds an external connection.
        }

        self.dirty = false;
    }

    /// Returns the world-space (X, Z) position at fractional distance `t` along an edge.
    pub fn get_pos_on_edge(&self, graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        Self::sample_pos_on_edge(graph, edge_idx, t)
    }

    fn sample_pos_on_edge(graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        let edge = graph.edge(edge_idx);
        let geo = &edge.physical_geometry;
        if geo.is_empty() {
            return Vector2::ZERO;
        }

        let target_dist = t * edge.physical_length;
        let mut curr_dist = 0.0;

        for i in 0..geo.len() - 1 {
            let p1 = Vector2::new(geo[i].x, geo[i].z);
            let p2 = Vector2::new(geo[i + 1].x, geo[i + 1].z);
            let d = (p2 - p1).length();
            if curr_dist + d >= target_dist {
                let local_t = (target_dist - curr_dist) / d;
                return p1 + (p2 - p1) * local_t;
            }
            curr_dist += d;
        }
        Vector2::new(geo.last().unwrap().x, geo.last().unwrap().z)
    }

    /// Returns the tangent vector (X, Z) at fractional distance `t` along an edge.
    pub fn get_tangent_on_edge(&self, graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        Self::sample_tangent_on_edge(graph, edge_idx, t)
    }

    fn sample_tangent_on_edge(graph: &RegionGraph, edge_idx: usize, t: f32) -> Vector2 {
        let edge = graph.edge(edge_idx);
        let geo = &edge.physical_geometry;
        if geo.len() < 2 {
            return Vector2::new(1.0, 0.0);
        }

        let target_dist = t * edge.physical_length;
        let mut curr_dist = 0.0;
        for i in 0..geo.len() - 1 {
            let p1 = Vector2::new(geo[i].x, geo[i].z);
            let p2 = Vector2::new(geo[i + 1].x, geo[i + 1].z);
            let dist = p2 - p1;
            let d = dist.length();
            if curr_dist + d >= target_dist {
                return if d > 1e-6 {
                    dist.normalized()
                } else {
                    Vector2::new(1.0, 0.0)
                };
            }
            curr_dist += d;
        }
        let p_end = Vector2::new(geo.last().unwrap().x, geo.last().unwrap().z);
        let p_prev = Vector2::new(geo[geo.len() - 2].x, geo[geo.len() - 2].z);
        let dist = p_end - p_prev;
        if dist.length() > 1e-6 {
            dist.normalized()
        } else {
            Vector2::new(1.0, 0.0)
        }
    }

    /// Recomputes world-space building transforms from saved frontage attachment data.
    pub(crate) fn recompute_derived_transforms(
        &mut self,
        graph: &RegionGraph,
        zoning: &ZoningSystem,
    ) -> Result<(), String> {
        for building in &mut self.buildings {
            if building.edge_idx >= graph.edge_count() {
                return Err(format!(
                    "building edge {} out of bounds for {} edges",
                    building.edge_idx,
                    graph.edge_count()
                ));
            }

            let edge = graph.edge(building.edge_idx);
            if edge.physical_geometry.len() < 2 || edge.physical_length <= 1e-6 {
                return Err(format!(
                    "building edge {} has insufficient geometry for transform rebuild",
                    building.edge_idx
                ));
            }

            let zone_cell_m = zoning.config.zone_cell_m;
            let width_cells = building.width_cells as f32;
            let depth_cells = building.depth_cells as f32;
            let along_offset = width_cells * 0.5 * zone_cell_m;
            let depth_offset = crate::config::SIDEWALK_WIDTH
                + (building.cell_y as f32 + depth_cells * 0.5) * zone_cell_m;
            let edge_t =
                (building.cell_x as f32 * zone_cell_m / edge.physical_length).clamp(0.0, 1.0);

            let world_pos_on_edge = Self::sample_pos_on_edge(graph, building.edge_idx, edge_t);
            let tangent = Self::sample_tangent_on_edge(graph, building.edge_idx, edge_t);
            let normal = Vector2::new(tangent.y, -tangent.x) * building.side as f32;
            let center_2d = world_pos_on_edge
                + normal * (edge.width * 0.5 + depth_offset)
                + tangent * along_offset;

            building.center_x = center_2d.x;
            building.center_y = center_2d.y;
            building.facing_dir = normal;
            building.side_offset = building.side as f32;
        }

        self.dirty = true;
        Ok(())
    }

    /// Returns the occupant capacity for a building, from its registered manifest.
    /// Falls back to 6 if the asset is not registered (legacy / unknown assets).
    fn building_capacity(&self, building_idx: usize) -> u32 {
        let cap = self.registry.capacity(&self.buildings[building_idx].asset_id);
        if cap == 0 { 6 } else { cap }
    }

    /// Repopulates the internal zone and vacancy indices (Bug B16/B16a fix).
    pub fn rebuild_zone_index(&mut self) {
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        self.vacancy_pos.resize(self.buildings.len(), usize::MAX);

        for (idx, b) in self.buildings.iter().enumerate() {
            let zi = b.zone_type as usize;
            if zi < 6 {
                self.zone_index[zi].push(idx);
                if b.occupancy < self.building_capacity(idx) {
                    let v_idx = self.vacancy_index[zi].len();
                    self.vacancy_index[zi].push(idx);
                    self.vacancy_pos[idx] = v_idx;
                }
            }
        }
        self.dirty_index = false;
    }

    /// Increments occupancy for a building and updates vacancy index if it becomes full. O(1).
    pub fn claim_vacancy(&mut self, building_idx: usize) {
        if building_idx >= self.buildings.len() {
            return;
        }
        let cap = self.building_capacity(building_idx);
        let b = &mut self.buildings[building_idx];
        b.occupancy += 1;

        // If it was in the vacancy list and is now full, remove it
        if b.occupancy >= cap {
            let zi = b.zone_type as usize;
            let v_pos = self.vacancy_pos[building_idx];
            if v_pos != usize::MAX {
                let list = &mut self.vacancy_index[zi];
                let last_b_idx = *list.last().unwrap();
                list.swap_remove(v_pos);
                self.vacancy_pos[last_b_idx] = v_pos;
                self.vacancy_pos[building_idx] = usize::MAX;
            }
        }
    }

    /// Decrements occupancy for a building and updates vacancy index if it gained space. O(1).
    pub fn release_vacancy(&mut self, building_idx: usize) {
        if building_idx >= self.buildings.len() {
            return;
        }
        let cap = self.building_capacity(building_idx);
        let b = &mut self.buildings[building_idx];
        b.occupancy = b.occupancy.saturating_sub(1);

        // If it was full and now has space, add it back to vacancy index
        if b.occupancy + 1 == cap {
            let zi = b.zone_type as usize;
            if self.vacancy_pos[building_idx] == usize::MAX {
                let v_idx = self.vacancy_index[zi].len();
                self.vacancy_index[zi].push(building_idx);
                self.vacancy_pos[building_idx] = v_idx;
            }
        }
    }

    /// Pick a random building from a specific zone type. O(1).
    pub fn get_random_building_by_zone(
        &self,
        zone: ZoneType,
        rng: &mut impl rand::Rng,
    ) -> Option<usize> {
        let list = &self.zone_index[zone as usize];
        if list.is_empty() {
            return None;
        }
        Some(list[rng.gen_range(0..list.len())])
    }

    /// Returns `(depart_node, building_index)` pairs for all buildings of `zone`.
    ///
    /// Used by [`FlowFieldSystem::rebuild_dirty`] to seed the multi-source Dijkstra.
    pub fn get_sources_for_zone(&self, zone: ZoneType, graph: &RegionGraph) -> Vec<(u32, usize)> {
        self.zone_index[zone as usize]
            .iter()
            .map(|&idx| (building_depart_node(&self.buildings[idx], graph), idx))
            .collect()
    }

    /// Pick a random building from any of the specified zone types. O(1).
    pub fn get_random_building_by_zones(
        &self,
        zones: &[ZoneType],
        rng: &mut impl rand::Rng,
    ) -> Option<usize> {
        // We sum the counts and pick based on weighted probability of lengths
        let mut total = 0;
        for &zone in zones {
            total += self.zone_index[zone as usize].len();
        }
        if total == 0 {
            return None;
        }

        let mut pick = rng.gen_range(0..total);
        for &zone in zones {
            let list = &self.zone_index[zone as usize];
            if pick < list.len() {
                return Some(list[pick]);
            }
            pick -= list.len();
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::asset::{BuildingData, LodEntry, ZoneClass};
    use crate::assets::AssetManifest;
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::grid::zoning::ZoneType;
    use godot::prelude::Vector2;
    use rand::SeedableRng;

    /// Registers a minimal 1×1 building asset for the given zone type so placement tests pass.
    fn register_test_asset(allocator: &mut BuildingAllocator, pack_id: &str, asset_id: &str, zone: ZoneClass) {
        let manifest = AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![LodEntry { file: "lod0.glb".to_owned(), distance_min_m: 0.0, distance_max_m: None }],
            anchors: vec![],
            building: Some(BuildingData {
                zone_type: zone,
                density: "low".to_owned(),
                lot_width_cells: 1,
                lot_depth_cells: 1,
                level: 1,
                residents_capacity: Some(6),
                worker_capacity: None,
                service_class: None,
                preview_scale: None,
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        };
        allocator.registry.register(pack_id, manifest, String::new());
    }

    #[test]
    fn test_zone_index_consistency() {
        let mut allocator = BuildingAllocator::new();
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

        // 1. Add buildings
        for i in 0..10 {
            allocator.buildings.push(Building {
                center_x: i as f32,
                center_y: 0.0,
                width_cells: 3,
                depth_cells: 3,
                zone_type: if i % 2 == 0 {
                    ZoneType::Residential
                } else {
                    ZoneType::Commercial
                },
                facing_dir: Vector2::new(0.0, 1.0),
                frontage_t: 0.5,
                side_offset: 0.0,
                abandoned_timer: 0,
                edge_idx: 0,
                side: 1,
                cell_x: i,
                cell_y: 0,
                occupancy: 0,
                asset_id: String::new(), level: 1,
            });
        }
        allocator.dirty_index = true;
        allocator.rebuild_zone_index();

        assert_eq!(
            allocator.zone_index[ZoneType::Residential as usize].len(),
            5
        );
        assert_eq!(allocator.zone_index[ZoneType::Commercial as usize].len(), 5);

        // 2. Remove a building (Residential at index 0)
        allocator.buildings.swap_remove(0);
        allocator.dirty_index = true;
        allocator.rebuild_zone_index();

        assert_eq!(allocator.buildings.len(), 9);
        assert_eq!(
            allocator.zone_index[ZoneType::Residential as usize].len(),
            4
        );
        assert_eq!(allocator.zone_index[ZoneType::Commercial as usize].len(), 5);

        // 3. Random selection
        let pick = allocator.get_random_building_by_zone(ZoneType::Commercial, &mut rng);
        assert!(pick.is_some());
        assert_eq!(
            allocator.buildings[pick.unwrap()].zone_type,
            ZoneType::Commercial
        );
    }

    #[test]
    fn test_vacancy_index_consistency() {
        let mut allocator = BuildingAllocator::new();
        let _rng = rand::rngs::StdRng::seed_from_u64(42);

        // 1. Add 5 Residential buildings
        for i in 0..5 {
            allocator.buildings.push(Building {
                center_x: i as f32,
                center_y: 0.0,
                width_cells: 3,
                depth_cells: 3,
                zone_type: ZoneType::Residential,
                facing_dir: Vector2::new(0.0, 1.0),
                frontage_t: 0.5,
                side_offset: 0.0,
                abandoned_timer: 0,
                edge_idx: 0,
                side: 1,
                cell_x: i,
                cell_y: 0,
                occupancy: 0,
                asset_id: String::new(), level: 1,
            });
        }
        allocator.rebuild_zone_index();

        assert_eq!(
            allocator.vacancy_index[ZoneType::Residential as usize].len(),
            5
        );

        // 2. Fill one building to capacity
        allocator.claim_vacancy(0); // 1
        allocator.claim_vacancy(0); // 2
        allocator.claim_vacancy(0); // 3
        allocator.claim_vacancy(0); // 4
        allocator.claim_vacancy(0); // 5
        assert_eq!(
            allocator.vacancy_index[ZoneType::Residential as usize].len(),
            5
        );
        allocator.claim_vacancy(0); // 6 (Full)

        assert_eq!(
            allocator.vacancy_index[ZoneType::Residential as usize].len(),
            4
        );
        assert!(!allocator.vacancy_index[ZoneType::Residential as usize].contains(&0));

        // 3. Release one spot
        allocator.release_vacancy(0); // 5
        assert_eq!(
            allocator.vacancy_index[ZoneType::Residential as usize].len(),
            5
        );
        assert!(allocator.vacancy_index[ZoneType::Residential as usize].contains(&0));

        // 4. Test swap_remove integrity in BuildingAllocator::tick
        // We'll manually simulate the swap logic in tick:
        // Remove building at index 1, building 4 moves to index 1.
        let mut agents = AgentSystem::new();
        // Setup agents to represent occupancy
        for _ in 0..5 {
            agents.spawn_agent(usize::MAX, 0, 0.0, 0.0, 0, 0.0, 0.0);
        }

        let last_idx = allocator.buildings.len() - 1; // 4
        let i = 1;
        let mut mapping = std::collections::HashMap::new();
        mapping.insert(last_idx, i);
        agents.remap_building_indices(&mapping); // Not strictly needed for allocator test but good practice

        allocator.buildings.swap_remove(i);
        allocator.rebuild_zone_index(); // Rebuild after swap

        assert_eq!(allocator.buildings.len(), 4);
        assert_eq!(
            allocator.zone_index[ZoneType::Residential as usize].len(),
            4
        );
        assert_eq!(
            allocator.vacancy_index[ZoneType::Residential as usize].len(),
            4
        );
    }

    #[test]
    fn test_building_placement_demand_subtraction() {
        use crate::simulation::economy::demand::DemandSystem;
        use crate::simulation::grid::desirability::DesirabilitySystem;
        use crate::simulation::grid::noise::NoiseSystem;
        use crate::simulation::grid::zoning::ZoningSystem;
        use crate::simulation::network::TransitNetwork;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
        register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);
        let mut demand = DemandSystem::new();
        demand.residential = 100.0;

        let map_cfg = MapConfig::default();
        let mut zoning = ZoningSystem::new(&map_cfg);
        let mut desirability = DesirabilitySystem::new(&map_cfg);
        let (env_w, env_h) = map_cfg.get_env_grid_size();
        for x in 0..env_w {
            for y in 0..env_h {
                desirability.grid.set(x, y, 100.0);
            }
        } // High desirability

        let noise = NoiseSystem::new(&map_cfg);
        let mut agents = AgentSystem::new();
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();

        network.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        // Paint zone rect covering the road frontage area
        zoning.set_zone_rect(-50.0, -50.0, 150.0, 50.0, crate::simulation::grid::zoning::ZoneType::Residential);

        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut network,
            &mut graph,
            &map_cfg,
        );

        assert_eq!(allocator.buildings.len(), 1);
        assert_eq!(
            demand.residential, 95.0,
            "Residential demand should decrease by 5.0 after placement"
        );
    }

    #[test]
    fn test_building_placement_desirability_gate() {
        use crate::simulation::economy::demand::DemandSystem;
        use crate::simulation::grid::desirability::DesirabilitySystem;
        use crate::simulation::grid::noise::NoiseSystem;
        use crate::simulation::grid::zoning::ZoningSystem;
        use crate::simulation::network::TransitNetwork;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
        register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);
        let mut demand = DemandSystem::new();
        demand.residential = 14.0; // Enough for placement (>10.0)

        let map_cfg = MapConfig::default();
        let mut zoning = ZoningSystem::new(&map_cfg);
        let mut desirability = DesirabilitySystem::new(&map_cfg);
        let (env_w, env_h) = map_cfg.get_env_grid_size();
        for x in 0..env_w {
            for y in 0..env_h {
                desirability.grid.set(x, y, 10.0);
            }
        } // Below gate threshold (< 20.0)

        let noise = NoiseSystem::new(&map_cfg);
        let mut agents = AgentSystem::new();
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();

        network.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        zoning.set_zone_rect(-50.0, -50.0, 150.0, 50.0, crate::simulation::grid::zoning::ZoneType::Residential);

        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut network,
            &mut graph,
            &map_cfg,
        );

        assert_eq!(
            allocator.buildings.len(),
            0,
            "No building should spawn when desirability is below 20.0"
        );
    }

    #[test]
    fn test_building_removal_clears_zoning_occupancy() {
        use crate::simulation::economy::demand::DemandSystem;
        use crate::simulation::grid::desirability::DesirabilitySystem;
        use crate::simulation::grid::noise::NoiseSystem;
        use crate::simulation::grid::zoning::ZoningSystem;
        use crate::simulation::network::TransitNetwork;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
        register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);
        let mut demand = DemandSystem::new();
        let map_cfg = MapConfig::default();
        let mut zoning = ZoningSystem::new(&map_cfg);
        let desirability = DesirabilitySystem::new(&map_cfg);
        let noise = NoiseSystem::new(&map_cfg);
        let mut agents = AgentSystem::new();
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();

        network.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );

        allocator.buildings.push(Building {
            center_x: 5.0,
            center_y: 10.0,
            width_cells: 3,
            depth_cells: 3,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.05,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            asset_id: String::new(), level: 1,
        });
        // Mark the building footprint as occupied
        zoning.mark_occupied_rect(5.0, 10.0, godot::prelude::Vector2::new(0.0, 1.0), 30.0, 30.0, true);

        // Remove zoning: leave world grid empty (no zone painted) — building zone_type won't match
        // ZoneType::None, so removal will trigger because get_zone_world returns None.

        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut network,
            &mut graph,
            &map_cfg,
        );

        assert_eq!(
            allocator.buildings.len(),
            0,
            "Building should have been removed"
        );
        // After removal, the occupied rect at that location should be cleared.
        assert!(
            !zoning.is_rect_occupied(5.0, 10.0, godot::prelude::Vector2::new(0.0, 1.0), 5.0, 5.0),
            "Zoning occupancy should be cleared after building removal"
        );
    }

    #[test]
    fn test_immigration_claims_vacant_home() {
        use crate::simulation::core::config::MapConfig;

        use crate::simulation::economy::agents::AgentSystem;
        use crate::simulation::economy::demand::DemandSystem;
        use crate::simulation::grid::desirability::DesirabilitySystem;
        use crate::simulation::grid::noise::NoiseSystem;
        use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
        use crate::simulation::network::graph::RegionGraph;
        use crate::simulation::network::TransitNetwork;
        use godot::prelude::{Vector2, Vector3};

        let mut allocator = BuildingAllocator::new();
        register_test_asset(&mut allocator, "base", "b.res.house", ZoneClass::Residential);
        let mut demand = DemandSystem::new();
        let map_cfg = MapConfig::default();
        let mut zoning = ZoningSystem::new(&map_cfg);
        let desirability = DesirabilitySystem::new(&map_cfg);
        let noise = NoiseSystem::new(&map_cfg);
        let mut agents = AgentSystem::new();
        let mut network = TransitNetwork::new();
        let mut graph = RegionGraph::new();

        demand.residential = 100.0;

        network.add_road(
            &mut graph,
            vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            1,
            1,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        let edge_id = graph.edge_count() - 1;

        // Add a Border node
        graph.set_node_type(0, crate::simulation::network::types::NodeType::Border);

        // Paint zone so the frontage area is residential
        zoning.set_zone_rect(-50.0, -50.0, 150.0, 50.0, ZoneType::Residential);
        allocator.buildings.push(Building {
            center_x: 10.0,
            center_y: 10.0,
            width_cells: 3,
            depth_cells: 3,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.1,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: edge_id,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            asset_id: String::new(), level: 1,
        });
        allocator.rebuild_zone_index();

        // 1 immigrant is spawned for sure since demand is high and occupancy is 0/6
        allocator.tick(
            &mut demand,
            &mut zoning,
            &desirability,
            &noise,
            &mut agents,
            &mut network,
            &mut graph,
            &map_cfg,
        );

        assert_eq!(agents.len(), 1, "Agent should have immigrated");
        assert_eq!(
            agents.home_building[0], 0,
            "Immigrant should have claimed home index 0"
        );
        assert_eq!(
            agents.target_building[0], 0,
            "Immigrant target_building should be set to home"
        );
        assert_eq!(
            allocator.buildings[0].occupancy, 1,
            "Building occupancy should be 1"
        );
    }
}
