//! Building placement and lifecycle management.
//!
//! [`BuildingAllocator::tick`] runs once per simulation tick. It:
//! 1. Removes buildings whose zoning cell has been changed or whose road edge was deleted.
//! 2. Scans zoned, unoccupied cells with sufficient demand and spawns new buildings.
//! 3. Spawns immigrant agents up to the current residential capacity.

use crate::simulation::grid::zoning::{ZoneType, ZoningSystem};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::NodeType;
use godot::prelude::Vector2;
use std::collections::HashMap;

/// Metadata for a building model's physical dimensions.
#[derive(Clone, Copy, Debug)]
pub struct ModelMetadata {
    /// Width in Godot units (before 10.0 scale).
    pub size_x: f32,
    /// Height in Godot units.
    pub size_y: f32,
    /// Depth in Godot units.
    pub size_z: f32,
}

/// A placed building occupying a 3 × 3 cell (30 m × 30 m) footprint on a zoning grid.
#[derive(Clone)]
pub struct Building {
    /// World-space X centre of the building footprint (metres, Godot's forward axis).
    pub center_x: f32,
    /// World-space Z centre of the building footprint (metres, Godot's forward axis).
    pub center_y: f32,
    /// Width of the footprint in zoning grid cells (1 cell = 10m).
    pub width_cells: u8,
    /// Depth of the footprint in zoning grid cells.
    pub depth_cells: u8,
    /// Zone category this building was spawned into.
    pub zone_type: ZoneType,
    /// Unit vector pointing from the road toward the building (outward normal from the road edge).
    pub facing_dir: Vector2,
    /// T-coordinate (0.0 to 1.0) along the road edge [`edge_idx`] for this building's frontage.
    pub frontage_t: f32,
    /// Road-graph node at this building's frontage point.
    pub frontage_node: u32,
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
    /// Vertical cell index (0 to ZONING_DEPTH-1).
    pub cell_y: u8,
    /// Total agents currently residing or working in this building.
    pub occupancy: u32,
    /// Randomly assigned variant ID for 3D model selection.
    pub variant: u8,
}

/// Manages the full lifecycle of [`Building`]s: placement, removal, and immigrant spawning.
#[derive(Clone)]
pub struct BuildingAllocator {
    /// All currently placed buildings. Removal uses `swap_remove` — order is not preserved.
    pub buildings: Vec<Building>,
    /// Set to `true` when the building list changes in a tick, signalling renderers to refresh.
    pub dirty: bool,
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
    /// Metadata for each (ZoneType, variant_id) pair.
    pub model_metadata: HashMap<(u8, u8), ModelMetadata>,
}

impl BuildingAllocator {
    /// Remaps all building edge indices after a road network compaction.
    pub fn update_edge_indices(&mut self, mapping: &std::collections::HashMap<usize, usize>) {
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
    }

    /// Creates an empty allocator.
    pub fn new() -> Self {
        Self {
            buildings: Vec::new(),
            dirty: false,
            zone_index: [const { Vec::new() }; 6],
            vacancy_index: [const { Vec::new() }; 6],
            vacancy_pos: Vec::new(),
            dirty_index: true,
            dirty_zones: [false; 6],
            model_metadata: HashMap::new(),
        }
    }

    /// Sets metadata for a specific building model.
    pub fn set_model_metadata(&mut self, zone_id: u8, variant: u8, metadata: ModelMetadata) {
        self.model_metadata.insert((zone_id, variant), metadata);
    }

    /// Gets metadata for a specific building model, or returns a default 1x1x1 size.
    pub fn get_model_metadata(&self, zone_id: u8, variant: u8) -> ModelMetadata {
        *self.model_metadata.get(&(zone_id, variant)).unwrap_or(&ModelMetadata {
            size_x: 1.0,
            size_y: 1.0,
            size_z: 1.0,
        })
    }

    /// Removes all buildings and resets the dirty flag.
    pub fn clear(&mut self) {
        self.buildings.clear();
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

        // 1. Cleanup: Remove buildings if their cells are no longer zoned correctly OR if the edge they're on is gone.
        let mut i = 0;
        while i < self.buildings.len() {
            let b = &self.buildings[i];
            let remove = if let Some(_) = zoning.edge_grids.get(&b.edge_idx) {
                let current_type = zoning.get_cell(b.edge_idx, b.side, b.cell_x, b.cell_y.into());
                current_type != b.zone_type || graph.edges[b.edge_idx].deleted
            } else {
                true // Edge gone
            };

            if remove {
                let b_edge_idx = b.edge_idx;
                let b_side = b.side;
                let b_cell_x = b.cell_x;
                let b_cell_y = b.cell_y;
                let b_width = b.width_cells;
                let b_depth = b.depth_cells;
                let b_frontage_node = b.frontage_node;
                let b_zone = b.zone_type;
                self.dirty_zones[b_zone as usize] = true;
                // `b` is no longer borrowed past this point; safe to pass `self` mutably below.

                // Merge the two half-edges back into one. If frontage_node has degree > 2
                // (not a pure Option-C split node), remove_frontage is a safe no-op.
                network.remove_frontage(graph, b_frontage_node, zoning, self, _agents);

                // Clear occupancy for the entire footprint
                let w_cells = b_width as usize;
                let d_cells = b_depth as usize;
                for dx in 0..w_cells {
                    for dy in 0..d_cells {
                        zoning.set_occupied(
                            b_edge_idx,
                            b_side,
                            b_cell_x + dx,
                            (b_cell_y as usize) + dy,
                            false,
                        );
                    }
                }
                let last_idx = self.buildings.len() - 1;
                if i < last_idx {
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

        // 2. Growth Logic: Find areas with un-occupied zones and high demand
        let edges_to_check: Vec<usize> = zoning.edge_grids.keys().cloned().collect();

        for edge_idx in edges_to_check {
            if spawned_this_tick >= max_spawns {
                break;
            }
            let (edge_len, edge_width) = if let Some(edge) = graph.edges.get(edge_idx) {
                if edge.deleted || edge.physical_geometry.len() < 2 {
                    (0.0, 0.0)
                } else {
                    (edge.physical_length, edge.width)
                }
            } else {
                (0.0, 0.0)
            };

            if edge_len < 0.1 {
                continue;
            }

            'side_loop: for side in [1, -1] {
                let cells_long = if let Some(g) = zoning.edge_grids.get(&edge_idx) {
                    g.cells_long
                } else {
                    0
                };

                for i in 0..cells_long {
                    let x = if i % 2 == 0 { (i / 2).min(cells_long - 1) } else { (cells_long - 1).saturating_sub(i / 2) };
                    if spawned_this_tick >= max_spawns {
                        break 'side_loop;
                    }

                    let z_type = zoning.get_cell(edge_idx, side, x, 0);
                    if z_type == ZoneType::None {
                        continue;
                    }

                    let demand = match z_type {
                        ZoneType::Residential => _demand.residential,
                        ZoneType::Commercial => _demand.commercial,
                        ZoneType::Industrial => _demand.industrial,
                        ZoneType::Office => _demand.commercial * 0.5,
                        ZoneType::Mixed => (_demand.residential + _demand.commercial) * 0.5,
                        _ => 0.0,
                    };

                    if demand < 10.0 {
                        continue;
                    }

                    // Determine deterministic variant and its footprint size
                    let variant = (edge_idx ^ x) as u8 % 64;
                    let meta = self.get_model_metadata(z_type as u8, variant);
                    let dw = meta.size_x.ceil().max(1.0) as usize;
                    let dh = meta.size_z.ceil().max(1.0) as usize;

                    // Ensure footprint fits in zoning grid
                    if x + dw > cells_long || dh > crate::config::ZONING_DEPTH as usize {
                        continue;
                    }

                    let mut can_build = true;
                    // B4: Grid Occupancy Check
                    // We check if all cells in the blueprint are zoned correctly and not occupied.
                    for dx in 0..dw {
                        for dy in 0..dh {
                            if zoning.get_cell(edge_idx, side, x + dx, dy) != z_type
                                || zoning.is_occupied(edge_idx, side, x + dx, dy)
                                || zoning.is_blocked(edge_idx, side, x + dx, dy)
                            {
                                can_build = false;
                                break;
                            }
                        }
                        if !can_build {
                            break;
                        }
                    }

                    // B5: Desirability Gate
                    if can_build {
                        let t_center = (x as f32 + (dw as f32 * 0.5)) * zoning.config.zone_cell_m / edge_len;
                        let world_pos = self.get_pos_on_edge(&graph, edge_idx, t_center);
                        let tangent = self.get_tangent_on_edge(&graph, edge_idx, t_center);
                        let normal =
                            godot::prelude::Vector2::new(tangent.y, -tangent.x) * (side as f32);
                        let depth_offset =
                            crate::config::SIDEWALK_WIDTH + ((dh as f32 * 0.5) * zoning.config.zone_cell_m);
                        let center_2d = world_pos + normal * (edge_width * 0.5 + depth_offset);

                        // Map world to grid coordinates
                        let (gx_raw, gy_raw) = config.world_to_env_grid(
                            center_2d.x,
                            center_2d.y,
                            desirability.grid.width,
                            desirability.grid.height,
                        );
                        let gx =
                            (gx_raw.round() as usize).min(desirability.grid.width.saturating_sub(1));
                        let gy = (gy_raw.round() as usize)
                            .min(desirability.grid.height.saturating_sub(1));

                        let val = *desirability.grid.get(gx, gy).unwrap_or(&50.0);
                        if val < 20.0 {
                            can_build = false;
                        }
                    }

                    if can_build {
                        // centre of the footprint
                        let frontage_t_offset = dw as f32 * 0.5;
                        let t_center = (x as f32 + frontage_t_offset) * zoning.config.zone_cell_m / edge_len;
                        let world_pos_on_edge = self.get_pos_on_edge(&graph, edge_idx, t_center);
                        let tangent = self.get_tangent_on_edge(&graph, edge_idx, t_center);
                        let normal =
                            godot::prelude::Vector2::new(tangent.y, -tangent.x) * (side as f32);

                        let depth_offset =
                            crate::config::SIDEWALK_WIDTH + ((dh as f32 * 0.5) * zoning.config.zone_cell_m);
                        let center_2d =
                            world_pos_on_edge + normal * (edge_width * 0.5 + depth_offset);

                        // Compute frontage_t on the original full edge for the split position.
                        let frontage_t_full = t_center;
                        // Mark footprint occupied on full edge BEFORE split (to allow split_edge_grid to copy flags correctly)
                        for adx in 0..dw {
                            for ady in 0..dh {
                                zoning.set_occupied(edge_idx, side, x + adx, ady, true);
                            }
                        }

                        // Push building with dummy node (it will be updated by split_for_frontage immediately)
                        self.buildings.push(Building {
                            zone_type: z_type,
                            facing_dir: normal,
                            frontage_t: frontage_t_full,
                            frontage_node: 0, 
                            side_offset: edge_width * 0.5 + crate::config::SIDEWALK_WIDTH,
                            center_x: center_2d.x,
                            center_y: center_2d.y,
                            edge_idx,
                            side: side as i8,
                            cell_x: x,
                            cell_y: 0,
                            width_cells: dw as u8,
                            depth_cells: dh as u8,
                            occupancy: 0,
                            variant: variant as u8,
                            abandoned_timer: 0,
                        });

                        // This call now includes the new building in its migration loop
                        network.split_for_frontage(graph, edge_idx, frontage_t_full, zoning, self);
                        
                        spawned_this_tick += 1;
                        self.dirty_index = true;
                        self.dirty_zones[z_type as usize] = true;

                        // Subtract from demand
                        match z_type {
                            ZoneType::Residential => _demand.residential -= 5.0,
                            ZoneType::Commercial => _demand.commercial -= 5.0,
                            ZoneType::Industrial => _demand.industrial -= 5.0,
                            _ => {}
                        }

                        // edge_idx geometry is now modified (split_for_frontage changed it
                        // in-place to the first half). Break both the x and side loops so
                        // subsequent iterations do not use stale edge_len or cells_long.
                        break 'side_loop;
                    }
                }
            }
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
            .fold(0, |acc, b| acc + (6 - b.occupancy as usize));

        if _agents.len() < total_capacity {
            let demand_factor = (_demand.residential / 100.0).max(0.0).min(1.0);
            let gap = total_capacity - _agents.len();
            let num_to_spawn = ((gap as f32 * 0.2 * demand_factor) as usize).max(1).min(10);

            // Collect all connected Border nodes as valid spawn points.
            let border_nodes: Vec<u32> = graph
                .nodes
                .iter()
                .enumerate()
                .filter_map(|(i, node)| {
                    if node.node_type != NodeType::Border {
                        return None;
                    }
                    // Only spawn from nodes that still have at least one live incident edge.
                    let connected = graph
                        .adjacency
                        .get(i)
                        .map_or(false, |adj| adj.iter().any(|&e| !graph.edges[e].deleted));
                    if connected { Some(i as u32) } else { None }
                })
                .collect();

            if !border_nodes.is_empty() {
                let mut rng = rand::thread_rng();
                for _ in 0..num_to_spawn {
                    if let Some(home_idx) = _agents.find_available_home(self) {
                        let spawn_node =
                            border_nodes[rand::Rng::gen_range(&mut rng, 0..border_nodes.len())];
                        let mut spawn_pos = graph.nodes[spawn_node as usize].pos;
                        
                        // Offset the spawn position to the correct lane center (RHT/LHT)
                        if let Some(edge_idx) = graph.adjacency[spawn_node as usize].get(0) {
                            let edge = &graph.edges[*edge_idx];
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
                        let home_node = home_bldg.frontage_node;

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
        let edge = &graph.edges[edge_idx];
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
        let edge = &graph.edges[edge_idx];
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
            if building.edge_idx >= graph.edges.len() {
                return Err(format!(
                    "building edge {} out of bounds for {} edges",
                    building.edge_idx,
                    graph.edges.len()
                ));
            }

            let edge = &graph.edges[building.edge_idx];
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
                if b.occupancy < 6 {
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
        let b = &mut self.buildings[building_idx];
        b.occupancy += 1;

        // If it was in the vacancy list and is now full, remove it
        if b.occupancy == 6 {
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
        let b = &mut self.buildings[building_idx];
        b.occupancy = b.occupancy.saturating_sub(1);

        // If it was full and now has space, add it back to vacancy index
        if b.occupancy == 5 {
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

    /// Returns `(frontage_node, building_index)` pairs for all buildings of `zone`.
    ///
    /// Used by [`FlowFieldSystem::rebuild_dirty`] to seed the multi-source Dijkstra.
    pub fn get_sources_for_zone(&self, zone: ZoneType) -> Vec<(u32, usize)> {
        self.zone_index[zone as usize]
            .iter()
            .map(|&idx| (self.buildings[idx].frontage_node, idx))
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
    use crate::simulation::core::config::MapConfig;
    use crate::simulation::economy::agents::AgentSystem;
    use crate::simulation::grid::zoning::ZoneType;
    use godot::prelude::Vector2;
    use rand::SeedableRng;

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
                frontage_node: 0,
                side_offset: 0.0,
                abandoned_timer: 0,
                edge_idx: 0,
                side: 1,
                cell_x: i,
                cell_y: 0,
                occupancy: 0,
                variant: 0,
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
        let mut rng = rand::rngs::StdRng::seed_from_u64(42);

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
                frontage_node: 0,
                side_offset: 0.0,
                abandoned_timer: 0,
                edge_idx: 0,
                side: 1,
                cell_x: i,
                cell_y: 0,
                occupancy: 0,
                variant: 0,
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
        use crate::simulation::grid::pollution::PollutionSystem;
        use crate::simulation::grid::zoning::ZoningSystem;
        use crate::simulation::network::TransitNetwork;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
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
            true,
            false,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        for x in 0..3 {
            for y in 0..3 {
                zoning.set_cell(0, 1, x, y, ZoneType::Residential);
            }
        }
        zoning.recalculate_obstructions(0, &graph);

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
        use crate::simulation::grid::pollution::PollutionSystem;
        use crate::simulation::grid::zoning::ZoningSystem;
        use crate::simulation::network::TransitNetwork;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
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
            true,
            false,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        for x in 0..3 {
            for y in 0..3 {
                zoning.set_cell(0, 1, x, y, ZoneType::Residential);
            }
        }
        zoning.recalculate_obstructions(0, &graph);

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
        use crate::simulation::grid::pollution::PollutionSystem;
        use crate::simulation::grid::zoning::ZoningSystem;
        use crate::simulation::network::TransitNetwork;
        use godot::prelude::Vector3;

        let mut allocator = BuildingAllocator::new();
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
            true,
            false,
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
            frontage_node: 0,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: 0,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            variant: 0,
        });
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_occupied(0, 1, dx, dy, true);
            }
        }

        // Remove zoning trigger
        for dx in 0..3 {
            for dy in 0..3 {
                zoning.set_cell(0, 1, dx, dy, ZoneType::None);
            }
        }

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
        assert!(
            !zoning.is_occupied(0, 1, 0, 0),
            "Zoning cell occupancy should be cleared after building removal"
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
            true,
            false,
            crate::simulation::network::types::EdgeClass::Standard,
            &mut zoning,
            &mut allocator,
        );
        let edge_id = graph.edges.len() - 1;

        // Add a Border node
        graph.nodes[0].node_type = crate::simulation::network::types::NodeType::Border;

        // Force a vacant residential building
        zoning.set_cell(edge_id, 1, 0, 0, ZoneType::Residential);
        allocator.buildings.push(Building {
            center_x: 10.0,
            center_y: 10.0,
            width_cells: 3,
            depth_cells: 3,
            zone_type: ZoneType::Residential,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.1,
            frontage_node: 0,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx: edge_id,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            variant: 0,
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
