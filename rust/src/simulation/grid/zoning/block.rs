use crate::config::ZONING_DEPTH;
use super::{ZoningSystem, ZoneType};

impl ZoningSystem {
    /// Sets a range of cells to a specific zone type with a given depth.
    pub fn set_zone_range(
        &mut self,
        edge_idx: usize,
        side: i8,
        start_t: f32,
        end_t: f32,
        depth: usize,
        zone_type: ZoneType,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) {
        let (cells_long, edge_physical_l, edge_geom) = {
            if let Some(grid) = self.edge_grids.get(&edge_idx) {
                let edge = graph.edge(edge_idx);
                (grid.cells_long, edge.physical_length, &edge.physical_geometry)
            } else {
                return;
            }
        };

        if edge_physical_l < 0.1 || edge_geom.is_empty() { return; }

        let start_x = (start_t * cells_long as f32).floor() as usize;
        let end_x = (end_t * cells_long as f32).ceil() as usize;
        
        let (start, end) = {
            let sx = start_x.min(cells_long);
            let ex = end_x.min(cells_long);
            if sx <= ex { (sx, ex) } else { (ex, sx) }
        };

        let depth = depth.min(ZONING_DEPTH);
        
        let edge = graph.edge(edge_idx);
        let mut min_pos = godot::prelude::Vector3::new(f32::MAX, 0.0, f32::MAX);
        let mut max_pos = godot::prelude::Vector3::new(f32::MIN, 0.0, f32::MIN);
        for p in &edge.physical_geometry {
            min_pos.x = min_pos.x.min(p.x);
            min_pos.z = min_pos.z.min(p.z);
            max_pos.x = max_pos.x.max(p.x);
            max_pos.z = max_pos.z.max(p.z);
        }
        let padding = 120.0;
        let nearby_edges = graph.get_edges_near_aabb(
            godot::prelude::Vector3::new(min_pos.x - padding, 0.0, min_pos.z - padding),
            godot::prelude::Vector3::new(max_pos.x + padding, 0.0, max_pos.z + padding),
        );

        let mut writes = Vec::new();
        for x in start..end {
            for y in 0..depth {
                let allowed = zone_type == ZoneType::None || !self.is_cell_obstructed(edge_idx, side, x, y, graph, Some(&nearby_edges));
                if allowed {
                    writes.push((x, y));
                }
            }
        }

        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let cells = if side > 0 { &mut grid.left_side } else { &mut grid.right_side };
            for (x, y) in writes {
                let idx = x * ZONING_DEPTH + y;
                if idx < cells.len() {
                    cells[idx] = zone_type;
                }
            }
        }
    }

    /// Sets a zone range as a solid block, storing the block depth per column so the renderer
    /// can merge adjacent cells into a single quad instead of showing individual cell borders.
    pub fn set_block_zone_range(
        &mut self,
        edge_idx: usize,
        side: i8,
        start_t: f32,
        end_t: f32,
        depth: usize,
        zone_type: ZoneType,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) {
        self.set_zone_range(edge_idx, side, start_t, end_t, depth, zone_type, graph);
        let bid = if zone_type != ZoneType::None {
            let id = self.next_block_id;
            self.next_block_id = self.next_block_id.wrapping_add(1).max(1);
            id
        } else {
            0
        };
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let cells_long = grid.cells_long;
            let start_x = (start_t * cells_long as f32).floor() as usize;
            let end_x = (end_t * cells_long as f32).ceil() as usize;
            let (start, end) = {
                let sx = start_x.min(cells_long);
                let ex = end_x.min(cells_long);
                if sx <= ex { (sx, ex) } else { (ex, sx) }
            };
            let block_depth = depth.min(ZONING_DEPTH) as u8;
            let (bd, bi) = if side > 0 {
                (&mut grid.left_block_depth, &mut grid.left_block_id)
            } else {
                (&mut grid.right_block_depth, &mut grid.right_block_id)
            };
            for x in start..end {
                if x < bd.len() {
                    bd[x] = if bid != 0 { block_depth } else { 0 };
                    bi[x] = bid;
                }
            }
        }
    }
}
