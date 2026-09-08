// SPDX-License-Identifier: GPL-2.0-only

//! Surface Debug road-tool Godot API methods.

use super::super::super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Exports physical road profiles with original terrain heights for offline regression audits.
    ///
    /// Each point is `[x, road_y, z, source_y]` in metres. This diagnostic is O(edge slots +
    /// profile points), uses Rayon across edges, and must only run outside interaction timings.
    #[func]
    pub fn get_road_terrain_benchmark_snapshot(&self) -> GString {
        use rayon::prelude::*;
        let snapshot = {
            let core = self.lock_core();
            let edges: Vec<_> = core
                .region_graph
                .edges()
                .par_iter()
                .enumerate()
                .filter(|(_, edge)| !edge.deleted)
                .map(|(id, edge)| {
                    let points: Vec<_> = edge
                        .physical_geometry
                        .iter()
                        .map(|point| {
                            [
                                point.x,
                                point.y,
                                point.z,
                                core.heightmap.sample_height_world(point.x, point.z)
                                    * crate::config::HEIGHT_SCALE,
                            ]
                        })
                        .collect();
                    serde_json::json!({
                        "edge_id": id, "start_node": edge.start_node, "end_node": edge.end_node,
                        "fwd_lanes": edge.fwd_lanes, "bkw_lanes": edge.bkw_lanes, "points": points,
                    })
                })
                .collect();
            serde_json::json!({"generation": core.road_tool_surface_generation, "edges": edges})
        };
        GString::from(&snapshot.to_string())
    }

    /// Returns compiled road-surface debug line data for editor visualization.
    ///
    /// Uses `try_lock` because this is only a debug/editor helper and should never stall the
    /// Godot main thread while the simulation mutex is busy.
    #[func]
    pub fn get_road_surface_debug_data(&self) -> Variant {
        match self.try_lock_core() {
            Some(mut core) => core.get_road_surface_debug_data_internal().to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns JSON describing final road-surface triangles under one world-space point.
    ///
    /// Uses `try_lock` because this is a debug/editor helper called from active tools.
    #[func]
    pub fn get_road_surface_probe_debug(&self, world_pos: Vector3) -> GString {
        match self.try_lock_core() {
            Some(mut core) => core.get_road_surface_probe_debug_internal(world_pos),
            None => GString::new(),
        }
    }

    /// Returns terrain patches whose raw heightmap payloads are forbidden by engineered ownership.
    #[func]
    pub fn get_engineered_terrain_patches(&self) -> PackedInt32Array {
        let snapshot = self.snapshot.read().unwrap();
        let mut packed = PackedInt32Array::new();
        for &(patch_x, patch_z) in snapshot.engineered_terrain_patch_keys.iter() {
            packed.push(i32::try_from(patch_x).unwrap_or(i32::MAX));
            packed.push(i32::try_from(patch_z).unwrap_or(i32::MAX));
        }
        packed
    }
}
