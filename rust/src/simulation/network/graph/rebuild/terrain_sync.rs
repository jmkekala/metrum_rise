// SPDX-License-Identifier: GPL-2.0-only

//! Terrain-height synchronization for standard road source geometry.

use super::super::data::RegionGraph;
use crate::simulation::network::types::EdgeClass;
use crate::simulation::terrain::TerrainSystem;

const TERRAIN_SYNC_SMOOTH_ITERATIONS: usize = 50;
const TERRAIN_SYNC_SMOOTH_LAMBDA: f32 = 0.5;
const TERRAIN_SYNC_SMOOTH_MU: f32 = -0.53;

impl RegionGraph {
    /// Synchronizes all road nodes and intermediate geometries to the terrain heightmap.
    ///
    /// Applies Laplacian smoothing to road grades to ensure smooth vertical transitions.
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
        }
        self.rebuild_intersection_clips();
    }
}
