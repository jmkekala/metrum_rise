use crate::config::ZONING_DEPTH;
use super::{ZoneType, ZoningSystem};

/// Per-edge zoning grid holding zone type and occupancy for both road sides.
///
/// Storage layout: flat `Vec` of length `cells_long × ZONING_DEPTH`, indexed as
/// `col * ZONING_DEPTH + row`. Left and right sides are separate Vecs.
#[derive(Clone, Debug, Default)]
pub struct EdgeZoning {
    /// Zone type for each cell on the left side (relative to edge travel direction).
    pub left_side: Vec<ZoneType>,
    /// Zone type for each cell on the right side.
    pub right_side: Vec<ZoneType>,
    /// `true` if a building's footprint covers this left-side cell.
    pub left_occupied: Vec<bool>,
    /// `true` if a building's footprint covers this right-side cell.
    pub right_occupied: Vec<bool>,
    /// Cached obstruction result for left-side cells. Stale after road edits —
    /// not yet used as a read-through cache (see `docs/project.md`).
    pub left_blocked: Vec<bool>,
    /// Cached obstruction result for right-side cells.
    pub right_blocked: Vec<bool>,
    /// Block depth per column on the left side. 0 = individual cells; N = block zone of depth N.
    /// Length equals `cells_long`.
    pub left_block_depth: Vec<u8>,
    /// Block depth per column on the right side. 0 = individual cells; N = block zone of depth N.
    /// Length equals `cells_long`.
    pub right_block_depth: Vec<u8>,
    /// Block placement ID per column on the left side. 0 = not a block zone; non-zero = unique ID
    /// for the specific block placement this column belongs to. Prevents adjacent blocks from
    /// merging into one quad in the renderer even when they share the same zone type and depth.
    /// Length equals `cells_long`.
    pub left_block_id: Vec<u16>,
    /// Block placement ID per column on the right side.
    /// Length equals `cells_long`.
    pub right_block_id: Vec<u16>,
    /// Number of columns in this grid (= `floor(edge_length / GRID_CELL_SIZE)`).
    pub cells_long: usize,
}

impl ZoningSystem {
    /// Returns the number of cells along the road for a specific edge's zoning grid.
    pub fn get_edge_grid_width(&self, edge_idx: usize) -> usize {
        self.edge_grids.get(&edge_idx).map(|g| g.cells_long).unwrap_or(0)
    }

    /// Splits an existing edge's zoning grid into two at the specified column index `split_x`.
    pub fn split_edge_grid(&mut self, old_idx: usize, new_idx: usize, split_x: usize) {
        if let Some(old_grid) = self.edge_grids.get(&old_idx).cloned() {
            let cells_long = old_grid.cells_long;
            let actual_split_x = split_x.min(cells_long);
            let part2_cells = cells_long.saturating_sub(actual_split_x);

            let mut new_grid = EdgeZoning {
                left_side: vec![ZoneType::None; part2_cells * ZONING_DEPTH],
                right_side: vec![ZoneType::None; part2_cells * ZONING_DEPTH],
                left_occupied: vec![false; part2_cells * ZONING_DEPTH],
                right_occupied: vec![false; part2_cells * ZONING_DEPTH],
                left_blocked: vec![false; part2_cells * ZONING_DEPTH],
                right_blocked: vec![false; part2_cells * ZONING_DEPTH],
                left_block_depth: vec![0u8; part2_cells],
                right_block_depth: vec![0u8; part2_cells],
                left_block_id: vec![0u16; part2_cells],
                right_block_id: vec![0u16; part2_cells],
                cells_long: part2_cells,
            };

            for x in 0..part2_cells {
                let old_x = actual_split_x + x;
                if old_x < old_grid.left_block_depth.len() {
                    new_grid.left_block_depth[x] = old_grid.left_block_depth[old_x];
                    new_grid.right_block_depth[x] = old_grid.right_block_depth[old_x];
                    new_grid.left_block_id[x] = old_grid.left_block_id[old_x];
                    new_grid.right_block_id[x] = old_grid.right_block_id[old_x];
                }
                for y in 0..ZONING_DEPTH {
                    if old_x * ZONING_DEPTH + y < old_grid.left_side.len() {
                        let old_i = old_x * ZONING_DEPTH + y;
                        let new_i = x * ZONING_DEPTH + y;
                        new_grid.left_side[new_i] = old_grid.left_side[old_i];
                        new_grid.right_side[new_i] = old_grid.right_side[old_i];
                        new_grid.left_occupied[new_i] = old_grid.left_occupied[old_i];
                        new_grid.right_occupied[new_i] = old_grid.right_occupied[old_i];
                        new_grid.left_blocked[new_i] = old_grid.left_blocked[old_i];
                        new_grid.right_blocked[new_i] = old_grid.right_blocked[old_i];
                    }
                }
            }
            self.edge_grids.insert(new_idx, new_grid);

            if let Some(g) = self.edge_grids.get_mut(&old_idx) {
                g.left_side.truncate(actual_split_x * ZONING_DEPTH);
                g.right_side.truncate(actual_split_x * ZONING_DEPTH);
                g.left_occupied.truncate(actual_split_x * ZONING_DEPTH);
                g.right_occupied.truncate(actual_split_x * ZONING_DEPTH);
                g.left_blocked.truncate(actual_split_x * ZONING_DEPTH);
                g.right_blocked.truncate(actual_split_x * ZONING_DEPTH);
                g.left_block_depth.truncate(actual_split_x);
                g.right_block_depth.truncate(actual_split_x);
                g.left_block_id.truncate(actual_split_x);
                g.right_block_id.truncate(actual_split_x);
                g.cells_long = actual_split_x;
            }
        }
    }

    /// Merges two adjacent edge zoning grids into one.
    pub fn merge_edge_grids(&mut self, first_idx: usize, second_idx: usize) {
        let second_grid = if let Some(g) = self.edge_grids.remove(&second_idx) {
            g
        } else {
            return;
        };

        if let Some(first_grid) = self.edge_grids.get_mut(&first_idx) {
            first_grid.left_side.extend_from_slice(&second_grid.left_side);
            first_grid.right_side.extend_from_slice(&second_grid.right_side);
            first_grid.left_occupied.extend_from_slice(&second_grid.left_occupied);
            first_grid.right_occupied.extend_from_slice(&second_grid.right_occupied);
            first_grid.left_blocked.extend_from_slice(&second_grid.left_blocked);
            first_grid.right_blocked.extend_from_slice(&second_grid.right_blocked);
            first_grid.left_block_depth.extend_from_slice(&second_grid.left_block_depth);
            first_grid.right_block_depth.extend_from_slice(&second_grid.right_block_depth);
            first_grid.left_block_id.extend_from_slice(&second_grid.left_block_id);
            first_grid.right_block_id.extend_from_slice(&second_grid.right_block_id);
            first_grid.cells_long += second_grid.cells_long;
        }
    }

    /// Resizes or creates the zoning grid for an edge based on its physical length.
    pub fn update_edge_grid_size(&mut self, edge_idx: usize, length: f32) {
        let cells_long = (length / self.config.zone_cell_m).floor() as usize;
        let entry = self
            .edge_grids
            .entry(edge_idx)
            .or_insert_with(|| EdgeZoning {
                left_side: vec![ZoneType::None; cells_long * ZONING_DEPTH],
                right_side: vec![ZoneType::None; cells_long * ZONING_DEPTH],
                left_occupied: vec![false; cells_long * ZONING_DEPTH],
                right_occupied: vec![false; cells_long * ZONING_DEPTH],
                left_blocked: vec![false; cells_long * ZONING_DEPTH],
                right_blocked: vec![false; cells_long * ZONING_DEPTH],
                left_block_depth: vec![0u8; cells_long],
                right_block_depth: vec![0u8; cells_long],
                left_block_id: vec![0u16; cells_long],
                right_block_id: vec![0u16; cells_long],
                cells_long,
            });

        if entry.cells_long != cells_long {
            entry.left_side.resize(cells_long * ZONING_DEPTH, ZoneType::None);
            entry.right_side.resize(cells_long * ZONING_DEPTH, ZoneType::None);
            entry.left_occupied.resize(cells_long * ZONING_DEPTH, false);
            entry.right_occupied.resize(cells_long * ZONING_DEPTH, false);
            entry.left_blocked.resize(cells_long * ZONING_DEPTH, false);
            entry.right_blocked.resize(cells_long * ZONING_DEPTH, false);
            entry.left_block_depth.resize(cells_long, 0);
            entry.right_block_depth.resize(cells_long, 0);
            entry.left_block_id.resize(cells_long, 0);
            entry.right_block_id.resize(cells_long, 0);
            entry.cells_long = cells_long;
        }
    }

    /// Returns the zone type of a specific cell.
    pub fn get_cell(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> ZoneType {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            let cells = if side > 0 { &grid.left_side } else { &grid.right_side };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                return cells[idx];
            }
        }
        ZoneType::None
    }

    /// Sets the zone type of a specific cell.
    pub fn set_cell(
        &mut self,
        edge_idx: usize,
        side: i8,
        x: usize,
        y: usize,
        zone_type: ZoneType,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) {
        if zone_type != ZoneType::None && self.is_cell_obstructed(edge_idx, side, x, y, graph, None) {
             return;
        }

        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let cells = if side > 0 { &mut grid.left_side } else { &mut grid.right_side };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                cells[idx] = zone_type;
            }
        }
    }

    /// Returns `true` if a building's footprint covers the specified cell.
    pub fn is_occupied(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> bool {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            let cells = if side > 0 { &grid.left_occupied } else { &grid.right_occupied };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                return cells[idx];
            }
        }
        true
    }

    /// Sets the occupancy state of a specific cell.
    pub fn set_occupied(&mut self, edge_idx: usize, side: i8, x: usize, y: usize, occupied: bool) {
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let cells = if side > 0 { &mut grid.left_occupied } else { &mut grid.right_occupied };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                cells[idx] = occupied;
            }
        }
    }

    /// Sets the obstruction state of a specific cell in the cache.
    pub fn set_blocked(&mut self, edge_idx: usize, side: i8, x: usize, y: usize, blocked: bool) {
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let cells = if side > 0 { &mut grid.left_blocked } else { &mut grid.right_blocked };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                cells[idx] = blocked;
            }
        }
    }

    /// Returns the cached obstruction state of a specific cell.
    pub fn is_blocked(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> bool {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            let cells = if side > 0 { &grid.left_blocked } else { &grid.right_blocked };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                return cells[idx];
            }
        }
        false
    }
}
