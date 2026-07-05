//! Undo/Redo system for simulation state.

use crate::nodes::sim::core::{
    NetworkRenderRuntimeSnapshot, SimCore, SimulationSnapshot, WaterRuntimeSnapshot,
};

impl SimCore {
    /// Pushes a new state snapshot onto the undo stack.
    ///
    /// Parameters `inc_terrain`, `inc_water`, `inc_trans_graph`, `inc_zoning` controls
    /// which components are included in the snapshot.
    pub fn push_undo_state(
        &mut self,
        inc_terrain: bool,
        inc_water: bool,
        inc_trans_graph: bool,
        inc_zoning: bool,
    ) {
        if self.undo_stack.len() >= 30 {
            self.undo_stack.pop_front(); // Constant 30-size rolling window
        }
        self.undo_stack.push_back(SimulationSnapshot {
            terrain: if inc_terrain {
                Some(self.heightmap.clone_visual_dense())
            } else {
                None
            },
            water: if inc_water {
                Some(WaterRuntimeSnapshot {
                    baseline_depth: self.watermap.clone_baseline_depth_dense(),
                })
            } else {
                None
            },
            trans_graph: if inc_trans_graph {
                Some(self.region_graph.clone())
            } else {
                None
            },
            network_render: if inc_trans_graph {
                Some(NetworkRenderRuntimeSnapshot {
                    road_surface: self.transit_network.road_surface.clone(),
                    refined_terrain_patch_cache: self.refined_terrain_patch_cache.clone(),
                    road_locked_terrain_patch_keys: self.road_locked_terrain_patch_keys.clone(),
                })
            } else {
                None
            },
            zoning: if inc_zoning {
                Some(self.zoning.clone())
            } else {
                None
            },
        });
    }

    /// Pops the last state snapshot from the undo stack and restores simulation state.
    /// Returns true if an action was undone.
    pub fn undo_action_internal(&mut self) -> bool {
        if let Some(state) = self.undo_stack.pop_back() {
            let SimulationSnapshot {
                terrain,
                water,
                trans_graph,
                network_render,
                zoning,
            } = state;
            let mut sync_trans_graph = false;
            let old_road_locked_patch_keys = self.road_locked_terrain_patch_keys.clone();

            if let Some(t_data) = terrain {
                self.heightmap
                    .replace_visual_from_dense(&t_data)
                    .expect("undo terrain snapshot must match the live terrain dimensions");
                sync_trans_graph = true;
            }
            if let Some(w_data) = water {
                self.watermap
                    .replace_baseline_depth_from_dense(&w_data.baseline_depth)
                    .expect("undo baseline water snapshot must match the live water dimensions");
                self.water_patch_mesh_cache.clear();
                self.water_dirty = true;
            }
            if let Some(tr_graph) = trans_graph {
                self.region_graph = tr_graph;
                sync_trans_graph = true;
            }
            if let Some(z_sys) = zoning {
                self.zoning = z_sys;
            }

            if sync_trans_graph {
                // Rebuild lane topology from the restored graph so crosswalk geometry
                // and junction connections match the reverted road network.
                self.transit_network
                    .lane_system
                    .rebuild(&mut self.region_graph);
                self.transit_network
                    .rebuild_cch_and_check(&self.region_graph);
                self.transit_network.cch_dirty_chunks.clear();
                self.transit_network.flow_fields.mark_all_dirty();
                self.restore_network_render_state(network_render, old_road_locked_patch_keys);
            }
            return true;
        }
        false
    }

    fn restore_network_render_state(
        &mut self,
        snapshot: Option<NetworkRenderRuntimeSnapshot>,
        old_road_locked_patch_keys: Vec<(usize, usize)>,
    ) {
        let restored_road_locked_patch_keys = snapshot
            .as_ref()
            .map(|snapshot| snapshot.road_locked_terrain_patch_keys.clone())
            .unwrap_or_default();
        let mut dirty_patch_keys = old_road_locked_patch_keys;
        dirty_patch_keys.extend(restored_road_locked_patch_keys.iter().copied());
        dirty_patch_keys.sort_unstable();
        dirty_patch_keys.dedup();
        for (patch_x, patch_z) in dirty_patch_keys {
            if let Some(patch) = self.heightmap.visual_patch_snapshot(patch_x, patch_z) {
                self.heightmap.reset_visual_region_from_source_world(
                    patch.world_origin_x,
                    patch.world_origin_z,
                    patch.world_origin_x + patch.world_size_x,
                    patch.world_origin_z + patch.world_size_z,
                );
            } else {
                self.heightmap.mark_render_patch_dirty(patch_x, patch_z);
            }
        }

        if let Some(snapshot) = snapshot {
            self.transit_network.road_surface = snapshot.road_surface;
            self.refined_terrain_patch_cache = snapshot.refined_terrain_patch_cache;
            self.road_locked_terrain_patch_keys = snapshot.road_locked_terrain_patch_keys;
        } else {
            self.transit_network.road_surface.clear();
            self.refined_terrain_patch_cache.clear();
            self.road_locked_terrain_patch_keys.clear();
        }
        self.cached_road_mesh_data = None;
        self.terrain_dirty = true;
        self.network_dirty = true;
    }
}
