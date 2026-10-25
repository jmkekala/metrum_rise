// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-height synchronization for standard road source geometry.

use super::super::data::RegionGraph;
use crate::simulation::network::grade::{
    SourcePoint, construct_within_bounds, source_bounds, source_ground_m,
};
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;

const TERRAIN_SYNC_SMOOTH_ITERATIONS: usize = 50;
const TERRAIN_SYNC_SMOOTH_LAMBDA: f32 = 0.5;
const TERRAIN_SYNC_SMOOTH_MU: f32 = -0.53;

impl RegionGraph {
    /// Synchronizes all road nodes and intermediate geometries to the terrain heightmap.
    ///
    /// Applies Laplacian smoothing to road grades to ensure smooth vertical transitions, then
    /// builds each re-graded control within the road grade rule inside the source band, as the
    /// stroke solve does (`ROAD-68` open question 37): where the ground leaves no profile within
    /// 1:3, the control is built at the least grade it allows, steeper than the maximum. Refusing
    /// such a terrain edit is open (`ROAD-68-K15` handoff).
    pub fn sync_to_terrain(&mut self, terrain: &TerrainSystem) {
        // 0. Pre-calculate which nodes are snappable (Standard only)
        let mut node_snappable = vec![true; self.nodes.len()];
        for edge in &self.edges {
            if edge.deleted {
                continue;
            }
            if edge.class != EdgeClass::Standard {
                node_snappable[edge.start_node as usize] = false;
                node_snappable[edge.end_node as usize] = false;
            }
        }

        // 1. Sync Nodes Only if snappable
        for (i, node) in self.nodes.iter_mut().enumerate() {
            if !node_snappable[i] {
                continue;
            }
            node.pos.y =
                terrain.sample_height_world(node.pos.x, node.pos.z) * crate::config::HEIGHT_SCALE;
        }

        // 2. Re-interpolate edge geometry and smooth grades.
        let mut temporary_heights = Vec::new();
        let (mut runs, mut sources) = (Vec::new(), Vec::new());
        for edge in &mut self.edges {
            if edge.deleted {
                continue;
            }
            if edge.class != EdgeClass::Standard {
                continue;
            }

            let count = edge.geometry.len();
            if count < 2 {
                continue;
            }

            // Snap endpoints to nodes
            edge.geometry[0] = self.nodes[edge.start_node as usize].pos;
            edge.geometry[count - 1] = self.nodes[edge.end_node as usize].pos;

            for point in &mut edge.geometry[1..count - 1] {
                point.y =
                    terrain.sample_height_world(point.x, point.z) * crate::config::HEIGHT_SCALE;
            }

            if count > 2 {
                temporary_heights.resize(count, 0.0);
                for _ in 0..TERRAIN_SYNC_SMOOTH_ITERATIONS {
                    for (target, window) in temporary_heights[1..count - 1]
                        .iter_mut()
                        .zip(edge.geometry.windows(3))
                    {
                        let laplacian = 0.5 * (window[0].y + window[2].y) - window[1].y;
                        *target = window[1].y + TERRAIN_SYNC_SMOOTH_LAMBDA * laplacian;
                    }
                    for (point, &height) in edge.geometry[1..count - 1]
                        .iter_mut()
                        .zip(&temporary_heights[1..count - 1])
                    {
                        point.y = height;
                    }
                    for (target, window) in temporary_heights[1..count - 1]
                        .iter_mut()
                        .zip(edge.geometry.windows(3))
                    {
                        let laplacian = 0.5 * (window[0].y + window[2].y) - window[1].y;
                        *target = window[1].y + TERRAIN_SYNC_SMOOTH_MU * laplacian;
                    }
                    for (point, &height) in edge.geometry[1..count - 1]
                        .iter_mut()
                        .zip(&temporary_heights[1..count - 1])
                    {
                        point.y = height;
                    }
                }
            }
            // The grade rule, with the two node ends held where the sync put them.
            runs.clear();
            runs.extend(
                edge.geometry
                    .windows(2)
                    .map(|pair| Self::edge_profile_point_distance_m(pair[0], pair[1])),
            );
            sources.clear();
            sources.extend(edge.geometry.iter().enumerate().map(|(index, point)| {
                SourcePoint {
                    ground_m: source_ground_m(terrain, point.x, point.z),
                    held_m: (index == 0 || index + 1 == count).then_some(point.y),
                }
            }));
            let bounds = source_bounds(&sources, &runs);
            temporary_heights.clear();
            temporary_heights.extend(edge.geometry.iter().map(|point| point.y));
            construct_within_bounds(&mut temporary_heights, &bounds, &runs);
            for (point, &height) in edge.geometry.iter_mut().zip(&temporary_heights) {
                point.y = height;
            }
            // Terrain authoring changes both source and physical heights explicitly. Clip
            // rebuilds preserve physical profiles and no longer perform an implicit copy.
            edge.physical_geometry.clone_from(&edge.geometry);
        }
        self.rebuild_intersection_clips();
        // The copies above drop every junction's pins: each node's arms re-derive their profiles
        // from the re-graded control polylines, as an edit's solve does (`ROAD-68` invariant 6,
        // open question 11 (c)). O(nodes + edges), like the sync.
        let nodes = (0..self.nodes.len() as u32).collect();
        self.finalize_junction_endpoint_profiles(&nodes, Some(terrain));
    }
}
