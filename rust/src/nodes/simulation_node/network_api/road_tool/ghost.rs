// SPDX-License-Identifier: GPL-2.0-only

//! Road geometry and tangent Godot API methods.

use super::super::super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns the full physical geometry of every non-deleted road edge.
    #[func]
    pub fn get_road_edge_polylines(&self) -> PackedFloat32Array {
        use crate::nodes::sim::bridge::network::get_road_edge_polylines;
        match self.try_lock_core() {
            Some(core) => get_road_edge_polylines(&core),
            None => PackedFloat32Array::new(),
        }
    }
}
