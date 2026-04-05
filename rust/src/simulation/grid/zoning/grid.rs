use super::{ZoneType, ZoningSystem};

/// Per-edge zoning grid holding zone type and occupancy for both road sides.
///
/// Storage layout: flat `Vec` of length `cells_long × depth`, indexed as
/// `col * depth + row` where `depth` is `left_depth` or `right_depth` for the
/// respective side. Sides are independent — their depths grow separately as
/// zoning is painted deeper from the road.
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
    /// Cached obstruction result for left-side cells. Stale after road edits.
    pub left_blocked: Vec<bool>,
    /// Cached obstruction result for right-side cells.
    pub right_blocked: Vec<bool>,
    /// Block depth per column on the left side. 0 = individual cells; N = block zone of depth N.
    /// Length equals `cells_long`.
    pub left_block_depth: Vec<u16>,
    /// Block depth per column on the right side. 0 = individual cells; N = block zone of depth N.
    /// Length equals `cells_long`.
    pub right_block_depth: Vec<u16>,
    /// Block placement ID per column on the left side. 0 = not a block zone; non-zero = unique ID
    /// for the specific block placement this column belongs to.
    /// Length equals `cells_long`.
    pub left_block_id: Vec<u16>,
    /// Block placement ID per column on the right side.
    /// Length equals `cells_long`.
    pub right_block_id: Vec<u16>,
    /// Number of columns in this grid (= `floor(edge_length / GRID_CELL_SIZE)`).
    pub cells_long: usize,
    /// Active depth on the left side. Cell arrays for the left side have `cells_long * left_depth` entries.
    pub left_depth: usize,
    /// Active depth on the right side. Cell arrays for the right side have `cells_long * right_depth` entries.
    pub right_depth: usize,
}

/// Reformats a flat `x * old_depth + y` layout into a `x * new_depth + y` layout.
/// New cells are filled with `fill`. Excess old cells (when shrinking) are dropped.
fn reformat<T: Copy>(
    old: &[T],
    fill: T,
    cells_long: usize,
    old_depth: usize,
    new_depth: usize,
) -> Vec<T> {
    let mut out = vec![fill; cells_long * new_depth];
    let copy_rows = old_depth.min(new_depth);
    for x in 0..cells_long {
        for y in 0..copy_rows {
            let old_i = x * old_depth + y;
            let new_i = x * new_depth + y;
            if old_i < old.len() {
                out[new_i] = old[old_i];
            }
        }
    }
    out
}

impl EdgeZoning {
    /// Returns the active depth for the requested side (`side > 0` = left, else right).
    #[inline]
    pub fn side_depth(&self, side: i8) -> usize {
        if side > 0 { self.left_depth } else { self.right_depth }
    }

    /// Grows the left side to `new_depth` if it is currently smaller.
    /// Existing data is preserved; new cells default to `None` / `false`.
    pub fn grow_left_depth(&mut self, new_depth: usize) {
        if new_depth <= self.left_depth { return; }
        let cl = self.cells_long;
        let old = self.left_depth;
        self.left_side     = reformat(&self.left_side,     ZoneType::None, cl, old, new_depth);
        self.left_occupied = reformat(&self.left_occupied, false,          cl, old, new_depth);
        self.left_blocked  = reformat(&self.left_blocked,  false,          cl, old, new_depth);
        self.left_depth = new_depth;
    }

    /// Grows the right side to `new_depth` if it is currently smaller.
    pub fn grow_right_depth(&mut self, new_depth: usize) {
        if new_depth <= self.right_depth { return; }
        let cl = self.cells_long;
        let old = self.right_depth;
        self.right_side     = reformat(&self.right_side,     ZoneType::None, cl, old, new_depth);
        self.right_occupied = reformat(&self.right_occupied, false,          cl, old, new_depth);
        self.right_blocked  = reformat(&self.right_blocked,  false,          cl, old, new_depth);
        self.right_depth = new_depth;
    }
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
            let ld = old_grid.left_depth;
            let rd = old_grid.right_depth;

            let mut new_grid = EdgeZoning {
                left_side:      vec![ZoneType::None; part2_cells * ld],
                right_side:     vec![ZoneType::None; part2_cells * rd],
                left_occupied:  vec![false; part2_cells * ld],
                right_occupied: vec![false; part2_cells * rd],
                left_blocked:   vec![false; part2_cells * ld],
                right_blocked:  vec![false; part2_cells * rd],
                left_block_depth:  vec![0u16; part2_cells],
                right_block_depth: vec![0u16; part2_cells],
                left_block_id:  vec![0u16; part2_cells],
                right_block_id: vec![0u16; part2_cells],
                cells_long: part2_cells,
                left_depth: ld,
                right_depth: rd,
            };

            for x in 0..part2_cells {
                let old_x = actual_split_x + x;
                if old_x < old_grid.left_block_depth.len() {
                    new_grid.left_block_depth[x]  = old_grid.left_block_depth[old_x];
                    new_grid.right_block_depth[x] = old_grid.right_block_depth[old_x];
                    new_grid.left_block_id[x]     = old_grid.left_block_id[old_x];
                    new_grid.right_block_id[x]    = old_grid.right_block_id[old_x];
                }
                for y in 0..ld {
                    let old_i = old_x * ld + y;
                    let new_i = x * ld + y;
                    if old_i < old_grid.left_side.len() {
                        new_grid.left_side[new_i]     = old_grid.left_side[old_i];
                        new_grid.left_occupied[new_i] = old_grid.left_occupied[old_i];
                        new_grid.left_blocked[new_i]  = old_grid.left_blocked[old_i];
                    }
                }
                for y in 0..rd {
                    let old_i = old_x * rd + y;
                    let new_i = x * rd + y;
                    if old_i < old_grid.right_side.len() {
                        new_grid.right_side[new_i]     = old_grid.right_side[old_i];
                        new_grid.right_occupied[new_i] = old_grid.right_occupied[old_i];
                        new_grid.right_blocked[new_i]  = old_grid.right_blocked[old_i];
                    }
                }
            }
            self.edge_grids.insert(new_idx, new_grid);

            if let Some(g) = self.edge_grids.get_mut(&old_idx) {
                g.left_side.truncate(actual_split_x * ld);
                g.right_side.truncate(actual_split_x * rd);
                g.left_occupied.truncate(actual_split_x * ld);
                g.right_occupied.truncate(actual_split_x * rd);
                g.left_blocked.truncate(actual_split_x * ld);
                g.right_blocked.truncate(actual_split_x * rd);
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
            // Ensure matching depths before concatenation; grow the shallower side.
            let target_ld = first_grid.left_depth.max(second_grid.left_depth);
            let target_rd = first_grid.right_depth.max(second_grid.right_depth);
            first_grid.grow_left_depth(target_ld);
            first_grid.grow_right_depth(target_rd);

            // Re-format second grid to matching depths if it was shallower.
            let s_left_side     = reformat(&second_grid.left_side,     ZoneType::None, second_grid.cells_long, second_grid.left_depth,  target_ld);
            let s_left_occ      = reformat(&second_grid.left_occupied,  false,         second_grid.cells_long, second_grid.left_depth,  target_ld);
            let s_left_blk      = reformat(&second_grid.left_blocked,   false,         second_grid.cells_long, second_grid.left_depth,  target_ld);
            let s_right_side    = reformat(&second_grid.right_side,    ZoneType::None, second_grid.cells_long, second_grid.right_depth, target_rd);
            let s_right_occ     = reformat(&second_grid.right_occupied, false,         second_grid.cells_long, second_grid.right_depth, target_rd);
            let s_right_blk     = reformat(&second_grid.right_blocked,  false,         second_grid.cells_long, second_grid.right_depth, target_rd);

            first_grid.left_side.extend_from_slice(&s_left_side);
            first_grid.right_side.extend_from_slice(&s_right_side);
            first_grid.left_occupied.extend_from_slice(&s_left_occ);
            first_grid.right_occupied.extend_from_slice(&s_right_occ);
            first_grid.left_blocked.extend_from_slice(&s_left_blk);
            first_grid.right_blocked.extend_from_slice(&s_right_blk);
            first_grid.left_block_depth.extend_from_slice(&second_grid.left_block_depth);
            first_grid.right_block_depth.extend_from_slice(&second_grid.right_block_depth);
            first_grid.left_block_id.extend_from_slice(&second_grid.left_block_id);
            first_grid.right_block_id.extend_from_slice(&second_grid.right_block_id);
            first_grid.cells_long += second_grid.cells_long;
        }
    }

    /// Resizes or creates the zoning grid for an edge based on its physical length.
    /// New grids start with depth 0 on both sides; depth grows as zoning is painted.
    pub fn update_edge_grid_size(&mut self, edge_idx: usize, length: f32) {
        let cells_long = (length / self.config.zone_cell_m).floor() as usize;
        let entry = self.edge_grids.entry(edge_idx).or_insert_with(|| EdgeZoning {
            left_side:      vec![],
            right_side:     vec![],
            left_occupied:  vec![],
            right_occupied: vec![],
            left_blocked:   vec![],
            right_blocked:  vec![],
            left_block_depth:  vec![0u16; cells_long],
            right_block_depth: vec![0u16; cells_long],
            left_block_id:  vec![0u16; cells_long],
            right_block_id: vec![0u16; cells_long],
            cells_long,
            left_depth: 0,
            right_depth: 0,
        });

        if entry.cells_long != cells_long {
            let ld = entry.left_depth;
            let rd = entry.right_depth;
            // Cell arrays: x * depth + y layout — resize by adding/removing whole columns.
            entry.left_side.resize(cells_long * ld, ZoneType::None);
            entry.right_side.resize(cells_long * rd, ZoneType::None);
            entry.left_occupied.resize(cells_long * ld, false);
            entry.right_occupied.resize(cells_long * rd, false);
            entry.left_blocked.resize(cells_long * ld, false);
            entry.right_blocked.resize(cells_long * rd, false);
            // Per-column arrays.
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
            let depth = grid.side_depth(side);
            let cells = if side > 0 { &grid.left_side } else { &grid.right_side };
            if x < grid.cells_long && y < depth {
                return cells[x * depth + y];
            }
        }
        ZoneType::None
    }

    /// Sets the zone type of a specific cell.
    /// Grows the side depth to accommodate `y + 1` when painting a non-None zone type.
    pub fn set_cell(
        &mut self,
        edge_idx: usize,
        side: i8,
        x: usize,
        y: usize,
        zone_type: ZoneType,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) {
        if zone_type != ZoneType::None {
            // Grow depth so the new row is accessible before the obstruction check.
            if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
                if side > 0 { grid.grow_left_depth(y + 1); }
                else        { grid.grow_right_depth(y + 1); }
            }
            if self.is_cell_obstructed(edge_idx, side, x, y, graph, None) {
                return;
            }
        }
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let depth = grid.side_depth(side);
            let cells = if side > 0 { &mut grid.left_side } else { &mut grid.right_side };
            if x < grid.cells_long && y < depth {
                cells[x * depth + y] = zone_type;
            }
        }
    }

    /// Returns `true` if a building's footprint covers the specified cell.
    pub fn is_occupied(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> bool {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            let depth = grid.side_depth(side);
            let cells = if side > 0 { &grid.left_occupied } else { &grid.right_occupied };
            if x < grid.cells_long && y < depth {
                return cells[x * depth + y];
            }
        }
        true
    }

    /// Sets the occupancy state of a specific cell.
    /// Grows side depth to accommodate `y + 1` when marking a cell occupied.
    pub fn set_occupied(&mut self, edge_idx: usize, side: i8, x: usize, y: usize, occupied: bool) {
        if occupied {
            if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
                if side > 0 { grid.grow_left_depth(y + 1); }
                else        { grid.grow_right_depth(y + 1); }
            }
        }
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let depth = grid.side_depth(side);
            let cells = if side > 0 { &mut grid.left_occupied } else { &mut grid.right_occupied };
            if x < grid.cells_long && y < depth {
                cells[x * depth + y] = occupied;
            }
        }
    }

    /// Sets the obstruction state of a specific cell in the cache.
    pub fn set_blocked(&mut self, edge_idx: usize, side: i8, x: usize, y: usize, blocked: bool) {
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let depth = grid.side_depth(side);
            let cells = if side > 0 { &mut grid.left_blocked } else { &mut grid.right_blocked };
            if x < grid.cells_long && y < depth {
                cells[x * depth + y] = blocked;
            }
        }
    }

    /// Returns the cached obstruction state of a specific cell.
    pub fn is_blocked(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> bool {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            let depth = grid.side_depth(side);
            let cells = if side > 0 { &grid.left_blocked } else { &grid.right_blocked };
            if x < grid.cells_long && y < depth {
                return cells[x * depth + y];
            }
        }
        false
    }
}
