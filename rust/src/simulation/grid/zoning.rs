//! Zoning grid system — land-use cells aligned to road edges.
//!
//! Each road edge that has zoning enabled gets an [`EdgeZoning`] grid. The grid is
//! `cells_long` columns wide (one column per `GRID_CELL_SIZE` metres of road length)
//! and [`ZONING_DEPTH`] rows deep (perpendicular to the road). The grid exists on both
//! the left and right sides of the road independently.
//!
//! Cell indices: `(col, row)` where `col` runs along the road and `row` runs away from it.
//! `row = 0` is the first row adjacent to the sidewalk.
//!
//! # Obstruction check
//!
//! [`ZoningSystem::is_cell_obstructed`] tests 5 sample points per cell using asphalt
//! collision and Voronoi ownership against nearby edges. This is O(K × L) per cell
//! where K = nearby edges and L = polyline segments. Results should be cached in
//! `left_blocked`/`right_blocked` cache is populated by `recalculate_obstructions()` on road
//! edits and read by `BuildingAllocator::tick()` via `is_blocked()` for O(1) placement checks.

use crate::config::{CLEARANCE_THRESHOLD, ZONING_DEPTH};
use crate::simulation::core::config::MapConfig;
use crate::simulation::network::types::EdgeClass;
use godot::prelude::*;
use rayon::prelude::*;
use std::collections::HashMap;

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
    /// Number of columns in this grid (= `floor(edge_length / GRID_CELL_SIZE)`).
    pub cells_long: usize,
}

/// Manages all [`EdgeZoning`] grids across the entire road network.
#[derive(Clone)]
pub struct ZoningSystem {
    /// Zoning grids keyed by edge index in [`RegionGraph::edges`].
    /// Only edges with `zoning_left || zoning_right` have entries here.
    pub edge_grids: HashMap<usize, EdgeZoning>,
    /// Global map configuration.
    pub config: MapConfig,
}

impl ZoningSystem {
    /// Creates a new, empty zoning system.
    pub fn new(config: &MapConfig) -> Self {
        Self {
            edge_grids: HashMap::new(),
            config: config.clone(),
        }
    }

    /// Clears all zoning data from the system.
    pub fn clear(&mut self) {
        self.edge_grids.clear();
    }

    /// Remaps the keys of edge_grids from [Old ID] to [New ID].
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

    /// Returns the number of cells along the road for a specific edge's zoning grid.
    pub fn get_edge_grid_width(&self, edge_idx: usize) -> usize {
        self.edge_grids.get(&edge_idx).map(|g| g.cells_long).unwrap_or(0)
    }

    /// Splits an existing edge's zoning grid into two at the specified column index `split_x`.
    pub fn split_edge_grid(&mut self, old_idx: usize, new_idx: usize, split_x: usize) {
        if let Some(old_grid) = self.edge_grids.get(&old_idx).cloned() {
            let cells_long = old_grid.cells_long;
            // Clamp split_x to ensure we don't try to truncate more than we have
            let actual_split_x = split_x.min(cells_long);
            let part2_cells = cells_long.saturating_sub(actual_split_x);

            let mut new_grid = EdgeZoning {
                left_side: vec![ZoneType::None; part2_cells * ZONING_DEPTH],
                right_side: vec![ZoneType::None; part2_cells * ZONING_DEPTH],
                left_occupied: vec![false; part2_cells * ZONING_DEPTH],
                right_occupied: vec![false; part2_cells * ZONING_DEPTH],
                left_blocked: vec![false; part2_cells * ZONING_DEPTH],
                right_blocked: vec![false; part2_cells * ZONING_DEPTH],
                cells_long: part2_cells,
            };

            // Copy data to new grid
            for x in 0..part2_cells {
                for y in 0..ZONING_DEPTH {
                    let old_x = actual_split_x + x;
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

            // Truncate old grid
            if let Some(g) = self.edge_grids.get_mut(&old_idx) {
                g.left_side.truncate(actual_split_x * ZONING_DEPTH);
                g.right_side.truncate(actual_split_x * ZONING_DEPTH);
                g.left_occupied.truncate(actual_split_x * ZONING_DEPTH);
                g.right_occupied.truncate(actual_split_x * ZONING_DEPTH);
                g.left_blocked.truncate(actual_split_x * ZONING_DEPTH);
                g.right_blocked.truncate(actual_split_x * ZONING_DEPTH);
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
            first_grid
                .left_side
                .extend_from_slice(&second_grid.left_side);
            first_grid
                .right_side
                .extend_from_slice(&second_grid.right_side);
            first_grid
                .left_occupied
                .extend_from_slice(&second_grid.left_occupied);
            first_grid
                .right_occupied
                .extend_from_slice(&second_grid.right_occupied);
            first_grid
                .left_blocked
                .extend_from_slice(&second_grid.left_blocked);
            first_grid
                .right_blocked
                .extend_from_slice(&second_grid.right_blocked);
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
                cells_long,
            });

        if entry.cells_long != cells_long {
            entry
                .left_side
                .resize(cells_long * ZONING_DEPTH, ZoneType::None);
            entry
                .right_side
                .resize(cells_long * ZONING_DEPTH, ZoneType::None);
            entry.left_occupied.resize(cells_long * ZONING_DEPTH, false);
            entry
                .right_occupied
                .resize(cells_long * ZONING_DEPTH, false);
            entry.left_blocked.resize(cells_long * ZONING_DEPTH, false);
            entry.right_blocked.resize(cells_long * ZONING_DEPTH, false);
            entry.cells_long = cells_long;
        }
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
        // "FIRST ZONED WINS": If we are setting a zone (not clearing), 
        // skip if the cell is already obstructed by physical road or an existing zone.
        if zone_type != ZoneType::None && self.is_cell_obstructed(edge_idx, side, x, y, graph, None) {
             return;
        }

        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let cells = if side > 0 {
                &mut grid.left_side
            } else {
                &mut grid.right_side
            };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                cells[idx] = zone_type;
            }
        }
    }

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
        // Collect obstruction results BEFORE mutably borrowing the grid to avoid borrow checker errors.
        let (cells_long, edge_physical_l, edge_geom) = {
            if let Some(grid) = self.edge_grids.get(&edge_idx) {
                let edge = &graph.edges[edge_idx];
                (grid.cells_long, edge.physical_length, &edge.physical_geometry)
            } else {
                return;
            }
        };

        if edge_physical_l < 0.1 || edge_geom.is_empty() { return; }

        let start_x = (start_t * cells_long as f32).floor() as usize;
        let end_x = (end_t * cells_long as f32).ceil() as usize;
        
        // Final clamped range
        let (start, end) = {
            let sx = start_x.min(cells_long);
            let ex = end_x.min(cells_long);
            if sx <= ex { (sx, ex) } else { (ex, sx) }
        };

        let depth = depth.min(ZONING_DEPTH);
        
        // Cache nearby edges for the range
        let edge = &graph.edges[edge_idx];
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

        // Pre-calculate which cells are NOT obstructed
        // We only care if we are zoned; if we are clearing (None), everything is "not obstructed"
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

    /// Returns the zone type of a specific cell.
    pub fn get_cell(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> ZoneType {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            let cells = if side > 0 {
                &grid.left_side
            } else {
                &grid.right_side
            };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                return cells[idx];
            }
        }
        ZoneType::None
    }

    /// Returns `true` if a building's footprint covers the specified cell.
    pub fn is_occupied(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> bool {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            let cells = if side > 0 {
                &grid.left_occupied
            } else {
                &grid.right_occupied
            };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                return cells[idx];
            }
        }
        true // Assume occupied if out of bounds or grid missing
    }

    /// Sets the occupancy state of a specific cell.
    pub fn set_occupied(&mut self, edge_idx: usize, side: i8, x: usize, y: usize, occupied: bool) {
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let cells = if side > 0 {
                &mut grid.left_occupied
            } else {
                &mut grid.right_occupied
            };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                cells[idx] = occupied;
            }
        }
    }

    /// Sets the obstruction state of a specific cell in the cache.
    pub fn set_blocked(&mut self, edge_idx: usize, side: i8, x: usize, y: usize, blocked: bool) {
        if let Some(grid) = self.edge_grids.get_mut(&edge_idx) {
            let cells = if side > 0 {
                &mut grid.left_blocked
            } else {
                &mut grid.right_blocked
            };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                cells[idx] = blocked;
            }
        }
    }

    /// Returns the cached obstruction state of a specific cell.
    pub fn is_blocked(&self, edge_idx: usize, side: i8, x: usize, y: usize) -> bool {
        if let Some(grid) = self.edge_grids.get(&edge_idx) {
            let cells = if side > 0 {
                &grid.left_blocked
            } else {
                &grid.right_blocked
            };
            if x < grid.cells_long && x * ZONING_DEPTH + y < cells.len() {
                let idx = x * ZONING_DEPTH + y;
                return cells[idx];
            }
        }
        false
    }

    /// Explicitly updates the obstruction cache for an edge.
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

        // Batch fetch nearby edges for the entire road segment
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

        // Parallelize the per-cell checks
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

    /// Returns the world-space center position of a specific zoning cell.
    pub fn get_cell_center(
        &self,
        edge_idx: usize,
        side: i8,
        x: usize,
        y: usize,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> Vector2 {
        if edge_idx >= graph.edges.len() {
            return Vector2::new(0.0, 0.0);
        }
        let edge = &graph.edges[edge_idx];
        let geom = &edge.physical_geometry;
        if geom.len() < 2 {
            return Vector2::new(0.0, 0.0);
        }

        let total_l = edge.physical_length;
        if total_l < 0.1 {
            return Vector2::new(0.0, 0.0);
        }

        let t = (x as f32 + 0.5) * self.config.zone_cell_m / total_l;
        if t > 1.0 {
            return Vector2::new(0.0, 0.0);
        }

        // Find position and tangent at T
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

        // Left normal is (y, -x) for Godot Y-down (Side 1).
        let normal = Vector2::new(tangent.y, -tangent.x) * side as f32;
        let depth = (y as f32 + 0.5) * self.config.zone_cell_m;
        let half_width = graph.edges[edge_idx].width * 0.5;

        pos + normal * (half_width + crate::config::SIDEWALK_WIDTH + depth)
    }
    /// Tests whether a specific cell is obstructed by asphalt, other road footprints, or competitor zoning.
    pub fn is_cell_obstructed(
        &self,
        edge_idx: usize,
        side: i8,
        x: usize,
        y: usize,
        graph: &crate::simulation::network::graph::RegionGraph,
        nearby_edges_cache: Option<&[usize]>,
    ) -> bool {
        let center = self.get_cell_center(edge_idx, side, x, y, graph);
        if center.x == 0.0 && center.y == 0.0 {
            return true;
        }

        let size = self.config.zone_cell_m;
        let edge = &graph.edges[edge_idx];

        // We check ONLY the center point for obstruction.
        // This is more permissive and allows zoning to go much closer to junctions.
        let check_pts_with_t = vec![(center, (x as f32 + 0.5) * size / edge.physical_length)];

        for (pt, t_us) in check_pts_with_t {
            let my_y = edge.get_y_at_t(t_us);
            let mut closest_competitor_has_zone = false;
            let mut asphalt_collision = false;

            let pt_3d = godot::prelude::Vector3::new(pt.x, 0.0, pt.y);

            let nearby_edges_vec;
            let nearby_edges = if let Some(cache) = nearby_edges_cache {
                cache
            } else {
                nearby_edges_vec = graph.get_edges_near_point(pt_3d, 120.0);
                &nearby_edges_vec
            };

            for &i in nearby_edges {
                let e = &graph.edges[i];
                if e.deleted {
                    continue;
                }

                // Skip own road segment unless it's a sharp curve that overlaps itself
                let is_self = i == edge_idx;

                // Sister Edge Detection (Road continuation at 2-way junctions)
                let is_sister = i != edge_idx
                    && (e.start_node == edge.start_node
                        || e.start_node == edge.end_node
                        || e.end_node == edge.start_node
                        || e.end_node == edge.end_node)
                    && e.width == edge.width
                    && e.primary_type == edge.primary_type;

                if is_sister {
                    let shared_node =
                        if e.start_node == edge.start_node || e.start_node == edge.end_node {
                            e.start_node
                        } else {
                            e.end_node
                        };
                    let conn_count = graph.adjacency[shared_node as usize].len();

                    if conn_count <= 2 {
                        let t1 = if edge.start_node == shared_node {
                            (edge.physical_geometry[1] - edge.physical_geometry[0]).normalized()
                        } else {
                            (edge.physical_geometry[edge.physical_geometry.len() - 2]
                                - edge.physical_geometry[edge.physical_geometry.len() - 1])
                                .normalized()
                        };
                        let t2 = if e.start_node == shared_node {
                            (e.physical_geometry[1] - e.physical_geometry[0]).normalized()
                        } else {
                            (e.physical_geometry[e.physical_geometry.len() - 2]
                                - e.physical_geometry[e.physical_geometry.len() - 1])
                                .normalized()
                        };
                        if t1.dot(t2) < -0.85 {
                            continue;
                        }
                    }
                }

                let pts = &e.physical_geometry;
                if pts.len() < 2 {
                    continue;
                }

                let mut cur_l_other = 0.0;
                for j in 0..pts.len() - 1 {
                    if is_self {
                        continue; // A road cannot obstruct its own zoning.
                    }

                    let p1 = pts[j];
                    let p2 = pts[j + 1];
                    let p1_2d = Vector2::new(p1.x, p1.z);
                    let p2_2d = Vector2::new(p2.x, p2.z);
                    let seg_vec = p2_2d - p1_2d;
                    let l_seg = seg_vec.length();
                    if l_seg == 0.0 {
                        continue;
                    }
                    let l2 = l_seg * l_seg;

                    let t_raw = ((pt.x - p1_2d.x) * seg_vec.x + (pt.y - p1_2d.y) * seg_vec.y) / l2;
                    let is_past_end = t_raw < -1e-4 || t_raw > 1.0 + 1e-4;
                    let t_proj = t_raw.clamp(0.0, 1.0);

                    let proj = p1_2d + seg_vec * t_proj;
                    let d_sq = pt.distance_squared_to(proj);
                    let tangent_other = seg_vec.normalized();
                    // Right-side normal is (-y, x) in Godot's Y-down system.
                    let normal_right = Vector2::new(-tangent_other.y, tangent_other.x);
                    let to_pt_other = pt - proj;

                    // --- VERTICAL CLEARANCE CHECK ---
                    // If the other road is a bridge or tunnel at a different level, ignore it.
                    let other_y = p1.y + (p2.y - p1.y) * t_proj;
                    if (other_y - my_y).abs() > CLEARANCE_THRESHOLD {
                        cur_l_other += l_seg;
                        continue;
                    }

                    // A. Road Footprint Collision: Overlapping other road asphalt or sidewalk
                    let mut hw_other = (e.width * 0.5) + crate::config::SIDEWALK_WIDTH;

                    // Relax the check if these roads meet at a junction.
                    // This allows zoning to 'bite' into the neighbor's sidewalk area at corners,
                    // which is necessary to fill the gap.
                    let is_neighbor = i != edge_idx
                        && (e.start_node == edge.start_node
                            || e.start_node == edge.end_node
                            || e.end_node == edge.start_node
                            || e.end_node == edge.end_node);

                    if is_neighbor {
                        hw_other = e.width * 0.5; // Only block actual asphalt, allow sidewalk overlap
                    }

                    if !is_past_end && d_sq < (hw_other + 0.1).powi(2) {
                        asphalt_collision = true;
                        break; 
                    }

                    // B. Zoning Claim Check
                    if e.class == crate::simulation::network::types::EdgeClass::Standard {
                        let d_other = f32::sqrt(d_sq);
                        // If dot > 0, point is on the Right side (side -1).
                        let side_other = if to_pt_other.dot(normal_right) > 0.0 { -1 } else { 1 };
                        let curb_dist_other = e.width * 0.5 + crate::config::SIDEWALK_WIDTH;
                        let row_other = ((d_other - curb_dist_other) / size).floor() as usize;
                        // Use global length along edge for column calculation
                        let col_other = ((cur_l_other + t_proj * l_seg) / size).floor() as usize;

                        // "FIRST ZONED WINS": If this competitor has a zone AT THIS POINT, it wins.
                        // We ONLY check this if we are actually within the road segment's longitudinal bounds.
                        // Footprint collision (asphalt) still happens past end to protect intersections.
                        if !is_past_end && row_other < ZONING_DEPTH && self.get_cell(i, side_other, col_other, row_other) != ZoneType::None {
                            closest_competitor_has_zone = true;
                        }
                    }
                    cur_l_other += l_seg;
                }
                if asphalt_collision {
                    break;
                }
            }

            if asphalt_collision {
                return true;
            }

            // 2. Competitor Check: Only block if the other road ALREADY has committed zones
            // spanning this point. This implements a pure "First Zoned Wins" policy without
            // any neutral Voronoi split.
            if closest_competitor_has_zone {
                 return true;
            }
            
            // 3. SPLAY CHECK (Curvature self-overlap)
            // y=0 cells on very tight curves must be blocked to prevent self-overlap.
            if y == 0 {
                let t_eps = 5.0 / edge.physical_length.max(10.0);
                let t_low = (t_us - t_eps).max(0.0);
                let t_high = (t_us + t_eps).min(1.0);
                let tan_low = edge.get_tangent_at_t(t_low);
                let tan_high = edge.get_tangent_at_t(t_high);
                let angle_diff = tan_low.angle_to(tan_high).abs();
                let dist = (t_high - t_low) * edge.physical_length;

                if dist > 0.001 {
                    let curvature = angle_diff / dist;
                    let radius = 1.0 / curvature.max(0.0001);
                    let hw = edge.width * 0.5 + crate::config::SIDEWALK_WIDTH;

                    let splay_ratio = (radius + hw) / radius.max(0.1);
                    if splay_ratio > 1.15 {
                        return true;
                    }
                }
            }
        }
        false
    }


    // Helper for rendering all painted cells
    /// Packs and returns all painted and non-blocked zoning cells for Godot-side rendering.
    pub fn get_render_data(
        &self,
        graph: &crate::simulation::network::graph::RegionGraph,
    ) -> PackedFloat32Array {
        let mut data = Vec::new();
        for (&edge_idx, grid) in &self.edge_grids {
            if edge_idx >= graph.edges.len() || graph.edges[edge_idx].deleted {
                continue;
            }
            for side in [1, -1] {
                let _side_sign = side as f32;
                let cells = if side > 0 {
                    &grid.left_side
                } else {
                    &grid.right_side
                };
                let _blocked = if side > 0 {
                    &grid.left_blocked
                } else {
                    &grid.right_blocked
                };
                // Safety: Ensure we don't try to access beyond the actual vector length
                let actual_cells_long = (cells.iter().len() / ZONING_DEPTH).min(grid.cells_long);
                for x in 0..actual_cells_long {
                    for y in 0..ZONING_DEPTH {
                        let idx = x * ZONING_DEPTH + y;
                        if idx >= cells.len() {
                            continue;
                        } // Extra safety
                        let z_type = cells[idx];
                        if z_type == ZoneType::None {
                            continue;
                        }

                        // PERFORMANCE: Use the pre-computed obstruction cache!
                        if _blocked.get(idx).cloned().unwrap_or(true) {
                            continue;
                        }

                        // Calculate world position of this cell
                        // This is a simplified version; real implementation needs tangent/normal integration
                        // but let's just pass the raw data for now and let the visual side handle some positioning?
                        // Actually, better to pass cell centers.

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::core::config::MapConfig;

    /// Build a ZoningSystem with one edge grid of the given length (zone_cell_m = 10.0).
    fn make_zoning(edge_idx: usize, length: f32) -> ZoningSystem {
        let cfg = MapConfig::default(); // zone_cell_m = 10.0
        let mut z = ZoningSystem::new(&cfg);
        z.update_edge_grid_size(edge_idx, length);
        z
    }

    // ── set_zone_range ──────────────────────────────────────────────────────────

    #[test]
    fn zone_range_full_zones_all_columns() {
        // 100 m / 10 m = 10 columns
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.set_zone_range(0, 1, 0.0, 1.0, 4, ZoneType::Residential, &graph);
        for x in 0..10 {
            for y in 0..4 {
                assert_eq!(
                    z.get_cell(0, 1, x, y),
                    ZoneType::Residential,
                    "col={x} row={y}"
                );
            }
            // Depth clamped: row 4 should remain None
            assert_eq!(
                z.get_cell(0, 1, x, 4),
                ZoneType::None,
                "col={x} row=4 should be empty"
            );
        }
    }

    #[test]
    fn zone_range_partial_zones_correct_columns() {
        // t=0.3..0.7 on 10 cols → floor(3.0)=3 .. ceil(7.0)=7 → cols 3,4,5,6
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.set_zone_range(0, 1, 0.3, 0.7, ZONING_DEPTH, ZoneType::Commercial, &graph);
        for x in 0..3 {
            assert_eq!(
                z.get_cell(0, 1, x, 0),
                ZoneType::None,
                "col {x} should be empty"
            );
        }
        for x in 3..7 {
            assert_eq!(
                z.get_cell(0, 1, x, 0),
                ZoneType::Commercial,
                "col {x} should be zoned"
            );
        }
        for x in 7..10 {
            assert_eq!(
                z.get_cell(0, 1, x, 0),
                ZoneType::None,
                "col {x} should be empty"
            );
        }
    }

    #[test]
    fn zone_range_sides_are_independent() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.set_zone_range(0, 1, 0.0, 1.0, 1, ZoneType::Residential, &graph);
        z.set_zone_range(0, -1, 0.0, 1.0, 1, ZoneType::Commercial, &graph);
        assert_eq!(z.get_cell(0, 1, 0, 0), ZoneType::Residential);
        assert_eq!(z.get_cell(0, -1, 0, 0), ZoneType::Commercial);
    }

    #[test]
    fn zone_range_reversed_t_still_zones_range() {
        // The function must swap start/end when end_t < start_t (B6 regression guard)
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.set_zone_range(0, 1, 0.7, 0.3, 1, ZoneType::Industrial, &graph);
        for x in 3..7 {
            assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Industrial, "col {x}");
        }
        for x in [0, 1, 2, 7, 8, 9] {
            assert_eq!(
                z.get_cell(0, 1, x, 0),
                ZoneType::None,
                "col {x} should be empty"
            );
        }
    }

    #[test]
    fn zone_range_clear_with_none() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.set_zone_range(0, 1, 0.0, 1.0, 1, ZoneType::Residential, &graph);
        z.set_zone_range(0, 1, 0.0, 0.5, 1, ZoneType::None, &graph);
        for x in 0..5 {
            assert_eq!(
                z.get_cell(0, 1, x, 0),
                ZoneType::None,
                "col {x} should be cleared"
            );
        }
        for x in 5..10 {
            assert_eq!(
                z.get_cell(0, 1, x, 0),
                ZoneType::Residential,
                "col {x} should remain"
            );
        }
    }

    // ── boundary conditions ─────────────────────────────────────────────────────

    #[test]
    fn get_cell_out_of_bounds_returns_none() {
        let z = make_zoning(0, 100.0);
        assert_eq!(
            z.get_cell(0, 1, 10, 0),
            ZoneType::None,
            "x beyond cells_long"
        );
        assert_eq!(z.get_cell(99, 1, 0, 0), ZoneType::None, "missing edge grid");
    }

    #[test]
    fn is_occupied_out_of_bounds_returns_true() {
        // Conservative: missing/OOB cell treated as occupied to block building placement
        let z = make_zoning(0, 100.0);
        assert!(z.is_occupied(0, 1, 100, 0), "x beyond cells_long");
        assert!(z.is_occupied(99, 1, 0, 0), "missing edge grid");
    }

    // ── split_edge_grid ─────────────────────────────────────────────────────────

    #[test]
    fn split_produces_correct_column_counts() {
        // 100 m → 10 cols. Split at 4 → old=4, new=6
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.split_edge_grid(0, 1, 4);
        assert_eq!(z.edge_grids[&0].cells_long, 4);
        assert_eq!(z.edge_grids[&1].cells_long, 6);
    }

    #[test]
    fn split_assigns_zone_data_to_correct_half() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        // Cols 0..4 Residential, cols 4..10 Commercial
        z.set_zone_range(0, 1, 0.0, 0.4, 1, ZoneType::Residential, &graph);
        z.set_zone_range(0, 1, 0.4, 1.0, 1, ZoneType::Commercial, &graph);
        z.split_edge_grid(0, 1, 4);

        for x in 0..4 {
            assert_eq!(z.get_cell(0, 1, x, 0), ZoneType::Residential, "old col {x}");
        }
        // New edge re-indexes from 0; originally cols 4..10
        for x in 0..6 {
            assert_eq!(z.get_cell(1, 1, x, 0), ZoneType::Commercial, "new col {x}");
        }
    }

    #[test]
    fn split_at_zero_produces_empty_old_full_new() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.split_edge_grid(0, 1, 0);
        assert_eq!(z.edge_grids[&0].cells_long, 0);
        assert_eq!(z.edge_grids[&1].cells_long, 10);
    }

    #[test]
    fn split_at_end_produces_full_old_empty_new() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.split_edge_grid(0, 1, 10);
        assert_eq!(z.edge_grids[&0].cells_long, 10);
        assert_eq!(z.edge_grids[&1].cells_long, 0);
    }

    #[test]
    fn split_copies_occupancy_to_new_half() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        z.set_occupied(0, 1, 7, 0, true);
        z.split_edge_grid(0, 1, 4);
        // Col 7 maps to col 3 in the new edge (7 - 4 = 3)
        assert!(
            z.is_occupied(1, 1, 3, 0),
            "occupied flag should transfer to new edge"
        );
        assert!(
            !z.is_occupied(0, 1, 3, 0),
            "old edge col 3 should not be marked occupied"
        );
    }

    // ── merge_edge_grids ────────────────────────────────────────────────────────

    #[test]
    fn merge_combines_column_counts_and_removes_second() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(40.0, 0.0, 0.0)], 7.0); // 4 cols
        z.update_edge_grid_size(1, 60.0); // 6 cols
        z.merge_edge_grids(0, 1);
        assert_eq!(z.edge_grids[&0].cells_long, 10);
        assert!(
            !z.edge_grids.contains_key(&1),
            "second grid should be removed"
        );
    }

    #[test]
    fn merge_preserves_zone_data_order() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::ZERO, godot::prelude::Vector3::new(40.0, 0.0, 0.0)], 7.0);
        z.update_edge_grid_size(1, 60.0);
        
        // Setup data manually to bypass graph/obstruction checks in a pure-data test
        if let Some(g0) = z.edge_grids.get_mut(&0) {
            for i in 0..g0.left_side.len() { g0.left_side[i] = ZoneType::Residential; }
        }
        if let Some(g1) = z.edge_grids.get_mut(&1) {
            for i in 0..g1.left_side.len() { g1.left_side[i] = ZoneType::Commercial; }
        }

        z.merge_edge_grids(0, 1);

        for x in 0..4 {
            assert_eq!(
                z.get_cell(0, 1, x, 0),
                ZoneType::Residential,
                "merged col {x}"
            );
        }
        for x in 4..10 {
            assert_eq!(
                z.get_cell(0, 1, x, 0),
                ZoneType::Commercial,
                "merged col {x}"
            );
        }
    }

    // ── splay regression tests ──────────────────────────────────────────────────
    //
    // Verify that `is_cell_obstructed` applies the splay check only to y=0 cells.
    // Before the fix the check ran for every y, which caused the entire 3×3 building
    // footprint to be rejected on any road with moderate curvature (y=2 always exceeds
    // the 1.15 threshold on curves even when y=0 is fine).

    fn make_edge_graph(
        pts: Vec<godot::prelude::Vector3>,
        width: f32,
    ) -> (crate::simulation::network::graph::RegionGraph, ZoningSystem) {
        use crate::simulation::network::graph::{Edge, RegionGraph};
        use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};

        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(*pts.first().unwrap(), NodeType::Junction);
        let n1 = graph.add_node(*pts.last().unwrap(), NodeType::Junction);

        let mut length = 0.0f32;
        for i in 0..pts.len() - 1 {
            length += (pts[i + 1] - pts[i]).length();
        }

        graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
            width,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 50.0,
            base_cost: 0.0,
            physical_length: length,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: pts.clone(),
            physical_geometry: pts,
            class: EdgeClass::Standard,
            deleted: false,
        });
        graph.rebuild_adjacency_list();

        let mut zoning = ZoningSystem::new(&MapConfig::default());
        zoning.update_edge_grid_size(0, length);
        (graph, zoning)
    }

    /// 9 points along a 90° arc of the given radius. Center at (radius, 0, 0),
    /// sweeping from (0,0,0) to (radius,0,radius). Arc length ≈ radius * π/2.
    fn arc_pts(radius: f32) -> Vec<godot::prelude::Vector3> {
        (0..=8)
            .map(|k| {
                let theta = std::f32::consts::PI - (k as f32 / 8.0) * std::f32::consts::FRAC_PI_2;
                godot::prelude::Vector3::new(
                    radius + radius * theta.cos(),
                    0.0,
                    radius * theta.sin(),
                )
            })
            .collect()
    }

    #[test]
    fn splay_y0_not_blocked_on_straight_road() {
        use godot::prelude::Vector3;
        let pts = vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)];
        let (graph, zoning) = make_edge_graph(pts, 7.0);
        // Straight road: inner/outer widths are equal, ratio = 1.0 — must not be blocked.
        assert!(!zoning.is_cell_obstructed(0, -1, 1, 0, &graph, Some(&[])));
    }

    #[test]
    fn splay_y0_blocked_on_tight_curve() {
        // 20 m radius → splay ratio ≈ 1.4 at y=0 outer side. Must block.
        let (graph, zoning) = make_edge_graph(arc_pts(20.0), 7.0);
        assert!(
            zoning.is_cell_obstructed(0, -1, 1, 0, &graph, Some(&[])),
            "y=0 on tight curve must be splay-blocked"
        );
    }

    #[test]
    fn splay_y1_not_blocked_on_tight_curve() {
        // Regression guard: after the fix splay is only checked for y=0, so y=1
        // on the same curve must pass (no asphalt/Voronoi competitors present).
        let (graph, zoning) = make_edge_graph(arc_pts(20.0), 7.0);
        assert!(
            !zoning.is_cell_obstructed(0, -1, 1, 1, &graph, Some(&[])),
            "y=1 on tight curve must NOT be splay-blocked (regression guard)"
        );
    }

    #[test]
    fn splay_y2_not_blocked_on_tight_curve() {
        let (graph, zoning) = make_edge_graph(arc_pts(20.0), 7.0);
        assert!(
            !zoning.is_cell_obstructed(0, -1, 1, 2, &graph, Some(&[])),
            "y=2 on tight curve must NOT be splay-blocked"
        );
    }

    // ── junction tests ─────────────────────────────────────────────────────────

    fn make_t_junction_graph(
        angle_rad: f32,
        width: f32,
    ) -> (crate::simulation::network::graph::RegionGraph, ZoningSystem) {
        use crate::simulation::network::graph::{Edge, RegionGraph};
        use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
        use godot::prelude::Vector3;

        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n_left = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Junction);
        let n_right = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        
        let dir = Vector3::new(angle_rad.cos(), 0.0, angle_rad.sin());
        let n_arm = graph.add_node(dir * 100.0, NodeType::Junction);

        // Edge 0: horizontal (-100 to 100). We split it at the center node later?
        // Actually simpler to just add two edges from center.
        let e0_pts = vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)];
        graph.add_edge(Edge {
            start_node: n_left, end_node: n_right, width, class: EdgeClass::Standard, 
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
            physical_length: 200.0, geometry: e0_pts.clone(), physical_geometry: e0_pts,
            fwd_lanes: 1, bkw_lanes: 1, deleted: false,
            ..Default::default()
        });

        // Edge 1: the angling arm
        let e1_pts = vec![Vector3::new(0.0, 0.0, 0.0), dir * 100.0];
        graph.add_edge(Edge {
            start_node: center, end_node: n_arm, width, class: EdgeClass::Standard,
            primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
            physical_length: 100.0, geometry: e1_pts.clone(), physical_geometry: e1_pts,
            fwd_lanes: 1, bkw_lanes: 1, deleted: false,
            ..Default::default()
        });

        graph.rebuild_adjacency_list();
        let mut zoning = ZoningSystem::new(&MapConfig::default());
        zoning.update_edge_grid_size(0, 200.0);
        zoning.update_edge_grid_size(1, 100.0);
        (graph, zoning)
    }

    fn make_star_graph(angles: &[f32], length: f32, width: f32) -> (crate::simulation::network::graph::RegionGraph, ZoningSystem) {
        use crate::simulation::network::graph::{Edge, RegionGraph};
        use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
        use godot::prelude::Vector3;

        let mut graph = RegionGraph::new();
        let center = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);

        for (i, &angle) in angles.iter().enumerate() {
            let dir = Vector3::new(angle.cos(), 0.0, angle.sin());
            let n_arm = graph.add_node(dir * length, NodeType::Junction);
            let pts = vec![Vector3::new(0.0, 0.0, 0.0), dir * length];
            let edge_idx = graph.add_edge(Edge {
                start_node: center, end_node: n_arm, width, class: EdgeClass::Standard,
                primary_type: TransitType::Road, allowed_types: TransitFlags::CAR,
                physical_length: length, geometry: pts.clone(), physical_geometry: pts,
                fwd_lanes: 1, bkw_lanes: 1, deleted: false,
                ..Default::default()
            });
            graph.add_to_spatial_index(edge_idx);
        }

        graph.rebuild_adjacency_list();
        let mut zoning = ZoningSystem::new(&MapConfig::default());
        for i in 0..angles.len() {
            zoning.update_edge_grid_size(i, length);
        }
        (graph, zoning)
    }

    #[test]
    fn junction_zoning_90_deg_non_destructive() {
        // T-junction: Horizontal road (Edge 0) and arm at 90 deg (Edge 1)
        let (graph, mut zoning) = make_t_junction_graph(std::f32::consts::FRAC_PI_2, 7.0);

        // 1. Zone Edge 0 fully on the side facing the arm.
        // Side -1 is 'Right' which in Y-down CCW (1,0)->(0,-1) is DOWN.
        zoning.set_zone_range(0, -1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
        zoning.set_zone_range(1, 1, 0.0, 1.0, 3, ZoneType::Commercial, &graph);

        let initial_count = zoning.edge_grids[&0].right_side.iter().filter(|&&z| z != ZoneType::None).count();
        assert!(initial_count > 0);

        // 2. Zone Edge 1 (the newcomer)
        // It's a 100m arm starting at (0,0) and going to (0,100).
        // Its sidewalk overlaps Edge 0's zones.
        zoning.set_zone_range(1, 1, 0.0, 1.0, 3, ZoneType::Industrial, &graph);
        zoning.set_zone_range(1, -1, 0.0, 1.0, 3, ZoneType::Industrial, &graph);

        // ASSERT: Edge 0's zones must be UNCHANGED (First Zoned Wins)
        let final_count = zoning.edge_grids[&0].right_side.iter().filter(|&&z| z != ZoneType::None).count();
        assert_eq!(initial_count, final_count, "Existing zones on Edge 0 were corrupted by new road zoning");

        // ASSERT: Edge 1 must have SOME zones, but they should be blocked/skipped near the center
        let e1_count = zoning.edge_grids[&1].left_side.iter().filter(|&&z| z != ZoneType::None).count();
        assert!(e1_count > 0);
        // Specifically, the first cell at t=0 should be blocked because it overlaps Edge 0's asphalt
        assert_eq!(zoning.get_cell(1, 1, 0, 0), ZoneType::None, "New road zoned on top of asphalt/existing zones");
    }

    #[test]
    fn junction_zoning_45_deg_non_destructive() {
        // T-junction at 45 degrees
        let (graph, mut zoning) = make_t_junction_graph(std::f32::consts::FRAC_PI_4, 7.0);

        // Main road side -1 (Right/Bottom). Arm side 1 (Left/Right-ish).
        // They overlap in the inner corner.
        zoning.set_zone_range(0, -1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
        let initial_count = zoning.edge_grids[&0].right_side.iter().filter(|&&z| z != ZoneType::None).count();

        // Newcomer zones arm
        zoning.set_zone_range(1, 1, 0.0, 1.0, 3, ZoneType::Commercial, &graph);

        assert_eq!(
            initial_count, 
            zoning.edge_grids[&0].right_side.iter().filter(|&&z| z != ZoneType::None).count(),
            "Existing zones on Edge 0 were corrupted at 45 deg"
        );
        
        // Ensure new road correctly respect the existing Residential
        // A cell at t=0 on Edge 1 (right on the junction) must be None.
        assert_eq!(zoning.get_cell(1, 1, 0, 0), ZoneType::None);
    }
    #[test]
    fn junction_zoning_multi_segment_non_destructive() {
        // T-junction where the horizontal road (Edge 0) has 2 segments.
        // Bent at (50, 0, 10). Junction at (50, 0, 10).
        use godot::prelude::Vector3;
        use crate::simulation::network::graph::RegionGraph;
        use crate::simulation::network::types::{EdgeClass, NodeType, TransitType, TransitFlags};

        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n_mid = graph.add_node(Vector3::new(50.0, 0.0, 10.0), NodeType::Junction);
        let n_end = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let n_arm = graph.add_node(Vector3::new(50.0, 0.0, 50.0), NodeType::Junction);

        // Edge 0: (0,0)->(50,10)->(100,0). Total len approx 101.98m
        let e0_idx = graph.add_edge(crate::simulation::network::graph::data::Edge {
            start_node: n0, end_node: n_end, physical_length: 101.98,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 10.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(50.0, 0.0, 10.0), Vector3::new(100.0, 0.0, 0.0)],
            width: 7.0, primary_type: TransitType::Road, allowed_types: TransitFlags::CAR, class: EdgeClass::Standard, ..Default::default()
        });
        // Edge 1: (50,10)->(50,50)
        let e1_idx = graph.add_edge(crate::simulation::network::graph::data::Edge {
            start_node: n_mid, end_node: n_arm, physical_length: 40.0,
            geometry: vec![Vector3::new(50.0, 0.0, 10.0), Vector3::new(50.0, 0.0, 50.0)],
            physical_geometry: vec![Vector3::new(50.0, 0.0, 10.0), Vector3::new(50.0, 0.0, 50.0)],
            width: 7.0, primary_type: TransitType::Road, allowed_types: TransitFlags::CAR, class: EdgeClass::Standard, ..Default::default()
        });
        graph.rebuild_adjacency_list();
        graph.add_to_spatial_index(e0_idx);
        graph.add_to_spatial_index(e1_idx);

        let mut zoning = ZoningSystem::new(&crate::simulation::core::config::MapConfig::default());
        zoning.update_edge_grid_size(0, 101.98);
        zoning.update_edge_grid_size(1, 40.0);

        // Zone the bent horizontal road first on both sides to ensure we hit the arm
        zoning.set_zone_range(0, 1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
        zoning.set_zone_range(0, -1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
        let count_left = zoning.edge_grids[&0].left_side.iter().filter(|&&z| z != ZoneType::None).count();
        let count_right = zoning.edge_grids[&0].right_side.iter().filter(|&&z| z != ZoneType::None).count();

        // Zone the arm vertical road second — use west side (side=-1) which is not claimed by edge 0
        zoning.set_zone_range(1, -1, 0.0, 1.0, 3, ZoneType::Commercial, &graph);

        // Verify Edge 0 zones were NOT cleared/overwritten
        assert_eq!(
            count_left,
            zoning.edge_grids[&0].left_side.iter().filter(|&&z| z != ZoneType::None).count(),
            "Manual overwrite occurred on LEFT side of multi-segment edge"
        );
        assert_eq!(
            count_right,
            zoning.edge_grids[&0].right_side.iter().filter(|&&z| z != ZoneType::None).count(),
            "Manual overwrite occurred on RIGHT side of multi-segment edge"
        );

        // Verify Edge 1 correctly zoned its west (free) side at the far end of the arm,
        // beyond edge 0's 3-row zone depth (~35m reach from the junction).
        assert_eq!(zoning.get_cell(1, -1, 3, 1), ZoneType::Commercial, "Arm far cell should be zoned Commercial");

        // The cells near the junction may be blocked by edge 0's territory; verify they are not corrupted with Residential
        for x in 0..4 {
            let cell = zoning.get_cell(1, -1, x, 0);
            assert!(cell == ZoneType::None || cell == ZoneType::Commercial, "Cell x={x} was corrupted with neighbor zone data");
        }
    }

    // ── split → merge round-trip ────────────────────────────────────────────────

    #[test]
    fn split_then_merge_round_trips_all_data() {
        let (graph, mut z) = make_edge_graph(vec![godot::prelude::Vector3::new(0.0, 0.0, 0.0), godot::prelude::Vector3::new(100.0, 0.0, 0.0)], 7.0);
        // Paint first half Residential, second half Commercial, 3 rows deep
        for x in 0..5 {
            for y in 0..3 {
                z.set_cell(0, 1, x, y, ZoneType::Residential, &graph);
            }
        }
        for x in 5..10 {
            for y in 0..3 {
                z.set_cell(0, 1, x, y, ZoneType::Commercial, &graph);
            }
        }

        z.split_edge_grid(0, 1, 5);
        z.merge_edge_grids(0, 1);

        assert_eq!(z.edge_grids[&0].cells_long, 10);
        for x in 0..5 {
            for y in 0..3 {
                assert_eq!(
                    z.get_cell(0, 1, x, y),
                    ZoneType::Residential,
                    "col={x} row={y}"
                );
            }
        }
        for x in 5..10 {
            for y in 0..3 {
                assert_eq!(
                    z.get_cell(0, 1, x, y),
                    ZoneType::Commercial,
                    "col={x} row={y}"
                );
            }
        }
    }
    #[test]
    fn junction_zoning_3way() {
        let angles = [0.0, 2.0944, 4.1888]; // 120 deg apart
        verify_junction_nongrowth(&angles, "3-way");
    }

    #[test]
    fn junction_zoning_4way() {
        let angles = [0.0, 1.5708, 3.14159, 4.71239]; // 90 deg apart
        verify_junction_nongrowth(&angles, "4-way");
    }

    #[test]
    fn junction_zoning_8way() {
        let mut angles = Vec::new();
        for i in 0..8 {
            angles.push((i as f32) * 0.7854); // 45 deg apart
        }
        verify_junction_nongrowth(&angles, "8-way");
    }

    #[test]
    fn junction_zoning_y_junction() {
        let angles = [1.5708, 2.87979, 3.40339]; // Up, then 75 deg off (fork)
        verify_junction_nongrowth(&angles, "Y-Fork");
    }

    fn verify_junction_nongrowth(angles: &[f32], name: &str) {
        let (graph, mut zoning) = make_star_graph(angles, 100.0, 7.0);

        for i in 0..angles.len() {
            // Record counts of ALL existing roads
            let mut before_counts = Vec::new();
            for j in 0..angles.len() {
                let l = zoning.edge_grids[&j].left_side.iter().filter(|&&z| z != ZoneType::None).count();
                let r = zoning.edge_grids[&j].right_side.iter().filter(|&&z| z != ZoneType::None).count();
                before_counts.push((l, r));
            }

            // Zone new arm on BOTH sides
            zoning.set_zone_range(i, 1, 0.0, 1.0, 3, ZoneType::Residential, &graph);
            zoning.set_zone_range(i, -1, 0.0, 1.0, 3, ZoneType::Residential, &graph);

            // Verify that all PREVIOUSLY zoned roads have identical counts
            for j in 0..angles.len() {
                if i == j { 
                    // i was just zoned, so it should have some zoning
                    let l = zoning.edge_grids[&j].left_side.iter().filter(|&&z| z != ZoneType::None).count();
                    let r = zoning.edge_grids[&j].right_side.iter().filter(|&&z| z != ZoneType::None).count();
                    assert!(l > 0, "Arm {} of {} failed to zone any left cells", i, name);
                    assert!(r > 0, "Arm {} of {} failed to zone any right cells", i, name);
                    continue; 
                }
                let l = zoning.edge_grids[&j].left_side.iter().filter(|&&z| z != ZoneType::None).count();
                let r = zoning.edge_grids[&j].right_side.iter().filter(|&&z| z != ZoneType::None).count();
                assert_eq!(l, before_counts[j].0, "Arm {} of {} corrupted sibling {}'s left zoning", i, name, j);
                assert_eq!(r, before_counts[j].1, "Arm {} of {} corrupted sibling {}'s right zoning", i, name, j);
            }
        }
    }
}
