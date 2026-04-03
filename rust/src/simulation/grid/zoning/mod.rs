use crate::config::ZONING_DEPTH;
use crate::simulation::core::config::MapConfig;
use crate::simulation::network::types::EdgeClass;
use godot::prelude::*;
use rayon::prelude::*;
use std::collections::HashMap;

/// Core zoning grid storage and cell accessors.
pub mod grid;
/// Path-point sampling and obstruction detection.
pub mod obstruction;
/// Block zone painting and ID allocation.
pub mod block;

pub use grid::EdgeZoning;

/// Land-use category painted onto a zoning grid cell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum ZoneType {
    /// No zoning — cell is unbuildable and transparent in the UI.
    None = 0,
    /// Residential housing — agents live here, consumes residential demand.
    Residential = 1,
    /// Retail / services — agents shop and work here, consumes commercial demand.
    Commercial = 2,
    /// Manufacturing / logistics — agents work here, consumes industrial demand.
    Industrial = 3,
    /// Office employment — treated as commercial demand at 50% weight currently.
    Office = 4,
    /// Dual-use: serves as both residential and commercial, consumes both demands.
    Mixed = 5,
}

/// Manages all [`EdgeZoning`] grids across the entire road network.
#[derive(Clone)]
pub struct ZoningSystem {
    /// Zoning grids keyed by edge index in [`RegionGraph::edges`].
    pub edge_grids: HashMap<usize, EdgeZoning>,
    /// Global map configuration.
    pub config: MapConfig,
    /// Monotonically increasing counter for assigning unique block placement IDs.
    pub next_block_id: u16,
}

impl ZoningSystem {
    /// Creates a new, empty zoning system.
    pub fn new(config: &MapConfig) -> Self {
        Self {
            edge_grids: HashMap::new(),
            config: config.clone(),
            next_block_id: 1,
        }
    }

    /// Clears all zoning data from the system.
    pub fn clear(&mut self) {
        self.edge_grids.clear();
    }

    /// Remaps the keys of edge_grids from [Old ID] to [New ID]. Used during graph compaction.
    pub fn update_edge_indices(&mut self, mapping: &HashMap<usize, usize>) {
        let mut new_grids = HashMap::new();
        for (old_idx, grid) in self.edge_grids.drain() {
            if let Some(&new_id) = mapping.get(&old_idx) {
                new_grids.insert(new_id, grid);
            }
        }
        self.edge_grids = new_grids;
    }

    /// Explicitly updates the obstruction cache for an edge by performing 5-point sampling per cell.
    pub fn recalculate_obstructions(
        &mut self,
        edge_idx: usize,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) {
        let cells_long = if let Some(grid) = self.edge_grids.get(&edge_idx) {
            grid.cells_long
        } else {
            return;
        };

        let edge = &graph.edges[edge_idx];
        if edge.class != EdgeClass::Standard {
            if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
                grid.left_blocked.fill(true);
                grid.right_blocked.fill(true);
            }
            return;
        }

        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        for p in &edge.physical_geometry {
            min_x = min_x.min(p.x);
            max_x = max_x.max(p.x);
            min_z = min_z.min(p.z);
            max_z = max_z.max(p.z);
        }
        let padding = 120.0;
        let nearby_edges = graph.get_edges_near_aabb(
            godot::prelude::Vector3::new(min_x - padding, 0.0, min_z - padding),
            godot::prelude::Vector3::new(max_x + padding, 0.0, max_z + padding),
        );

        let results: Vec<(bool, bool)> = (0..cells_long * ZONING_DEPTH)
            .into_par_iter()
            .map(|idx| {
                let x = idx / ZONING_DEPTH;
                let y = idx % ZONING_DEPTH;
                let l = self.is_cell_obstructed(edge_idx, 1, x, y, graph, Some(&nearby_edges));
                let r = self.is_cell_obstructed(edge_idx, -1, x, y, graph, Some(&nearby_edges));
                (l, r)
            })
            .collect();

        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            for (i, (l_blocked, r_blocked)) in results.into_iter().enumerate() {
                grid.left_blocked[i] = l_blocked;
                grid.right_blocked[i] = r_blocked;
            }
        }
    }

    /// Packs and returns all painted and non-blocked zoning cells for Godot-side rendering.
    pub fn get_render_data(
        &self,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> PackedFloat32Array {
        let mut data = Vec::new();
        for (&edge_idx, grid) in &self.edge_grids {
            if edge_idx >= graph.edges.len() || graph.edges[edge_idx].deleted { continue; }
            for side in [1, -1] {
                let cells = if side > 0 { &grid.left_side } else { &grid.right_side };
                let blocked = if side > 0 { &grid.left_blocked } else { &grid.right_blocked };
                let actual_cells_long = (cells.iter().len() / ZONING_DEPTH).min(grid.cells_long);
                for x in 0..actual_cells_long {
                    for y in 0..ZONING_DEPTH {
                        let idx = x * ZONING_DEPTH + y;
                        if idx >= cells.len() { continue; }
                        let z_type = cells[idx];
                        if z_type == ZoneType::None { continue; }

                        if blocked.get(idx).cloned().unwrap_or(true) { continue; }

                        data.push(edge_idx as f32);
                        data.push(side as f32);
                        data.push(x as f32);
                        data.push(y as f32);
                        data.push(z_type as u8 as f32);
                    }
                }
            }
        }
        PackedFloat32Array::from_iter(data)
    }

    /// Returns the world-space center position of a specific zoning cell.
    pub fn get_cell_center(
        &self,
        edge_idx: usize,
        side: i8,
        x: usize,
        y: usize,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> Vector2 {
        if edge_idx >= graph.edges.len() { return Vector2::new(0.0, 0.0); }
        let edge = &graph.edges[edge_idx];
        let geom = &edge.physical_geometry;
        if geom.len() < 2 { return Vector2::new(0.0, 0.0); }

        let total_l = edge.physical_length;
        if total_l < 0.1 { return Vector2::new(0.0, 0.0); }

        let t = (x as f32 + 0.5) * self.config.zone_cell_m / total_l;
        if t > 1.0 { return Vector2::new(0.0, 0.0); }

        let mut curr_l = 0.0;
        let mut pos = Vector2::new(0.0, 0.0);
        let mut tangent = Vector2::new(1.0, 0.0);
        let target_l = t * total_l;

        for i in 0..geom.len() - 1 {
            let p1 = Vector2::new(geom[i].x, geom[i].z);
            let p2 = Vector2::new(geom[i + 1].x, geom[i + 1].z);
            let d = (p2 - p1).length();
            if curr_l + d >= target_l {
                let local_t = (target_l - curr_l) / d;
                pos = p1 + (p2 - p1) * local_t;
                tangent = (p2 - p1).normalized();
                break;
            }
            curr_l += d;
            if i == geom.len() - 2 {
                pos = p2;
                tangent = (p2 - p1).normalized();
            }
        }

        let normal = Vector2::new(tangent.y, -tangent.x) * side as f32;
        let depth = (y as f32 + 0.5) * self.config.zone_cell_m;
        let half_width = graph.edges[edge_idx].width * 0.5;

        pos + normal * (half_width + crate::config::SIDEWALK_WIDTH + depth)
    }
}

/// Unit tests for the zoning system.
#[cfg(test)]
pub mod tests;
