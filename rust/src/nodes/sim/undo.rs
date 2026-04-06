//! Undo/Redo system for simulation state.

use crate::nodes::sim::core::{SimCore, SimulationSnapshot};

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
                Some(self.heightmap.data.clone())
            } else {
                None
            },
            water: if inc_water {
                Some(self.watermap.depth.clone())
            } else {
                None
            },
            trans_graph: if inc_trans_graph {
                Some(self.region_graph.clone())
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
            let mut sync_trans_graph = false;

            if let Some(t_data) = state.terrain {
                self.heightmap.data = t_data;
                sync_trans_graph = true;
            }
            if let Some(w_data) = state.water {
                self.watermap.depth = w_data;
            }
            if let Some(tr_graph) = state.trans_graph {
                self.region_graph = tr_graph;
                sync_trans_graph = true;
            }
            if let Some(z_sys) = state.zoning {
                self.zoning = z_sys;
            }

            if sync_trans_graph {
                // Rebuild lane topology from the restored graph so crosswalk geometry
                // and junction connections match the reverted road network.
                self.transit_network.lane_system.rebuild(&mut self.region_graph);
                self.transit_network.cch_graph =
                    crate::simulation::pathing::cch::CchGraph::build(&self.region_graph);
            }
            return true;
        }
        false
    }
}
