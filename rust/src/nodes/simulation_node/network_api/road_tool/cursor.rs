// SPDX-License-Identifier: GPL-2.0-only

//! Cursor road-tool Godot API methods: the closest network point. The road tool's own cursor is
//! its session's (`road_tool_pointer`, `ROAD-112` round D removed the curve path's query).

use super::super::super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns the closest network point (node/edge) within range, or `null` for none.
    ///
    /// A per-frame query like `intersect_terrain`: `null` too while the sim thread holds the
    /// core, unless `wait` (a click), which waits for it.
    #[func]
    pub fn get_closest_network_point(
        &self,
        world_pos: Vector3,
        max_dist: f32,
        #[opt(default = false)] wait: bool,
    ) -> Variant {
        self.lock_core_if(wait).map_or_else(Variant::nil, |core| {
            core.get_closest_network_point_internal(world_pos, max_dist)
                .map_or_else(Variant::nil, |p| p.to_variant())
        })
    }
}
