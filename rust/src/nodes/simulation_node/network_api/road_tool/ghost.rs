// SPDX-License-Identifier: GPL-2.0-only

//! Ghost road-tool Godot API methods.

use super::super::super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns ghost guide data for the road-tool overlay.
    #[func]
    pub fn get_road_ghost_guides(&self) -> PackedFloat32Array {
        use crate::nodes::sim::bridge::network::get_road_ghost_guides;
        match self.try_lock_core() {
            Some(core) => get_road_ghost_guides(&core),
            None => PackedFloat32Array::new(),
        }
    }

    /// Returns fully-resolved ghost guide line vertices and colors.
    #[func]
    pub fn get_road_ghost_line_data(&self) -> VarDictionary {
        use crate::nodes::sim::bridge::network::get_road_ghost_line_data;
        match self.try_lock_core() {
            Some(mut core) => get_road_ghost_line_data(&mut core),
            None => VarDictionary::new(),
        }
    }

    /// Returns the closest ghost-guide snap point within range, or null.
    #[func]
    pub fn get_road_ghost_snap(
        &self,
        world_pos: Vector3,
        max_dist_m: f32,
        altitude_offset_m: f32,
    ) -> Variant {
        use crate::nodes::sim::bridge::network::get_road_ghost_snap_from_parts;
        let query = self.road_tool_query_snapshot.read().unwrap();
        match get_road_ghost_snap_from_parts(
            &query.region_graph,
            &query.road_surface,
            &query.terrain,
            &query.ghost_snap_index,
            world_pos,
            max_dist_m,
            altitude_offset_m,
        ) {
            Some(point) => point.to_variant(),
            None => Variant::nil(),
        }
    }

    /// Returns the full physical geometry of every non-deleted road edge.
    #[func]
    pub fn get_road_edge_polylines(&self) -> PackedFloat32Array {
        use crate::nodes::sim::bridge::network::get_road_edge_polylines;
        match self.try_lock_core() {
            Some(core) => get_road_edge_polylines(&core),
            None => PackedFloat32Array::new(),
        }
    }

    /// Returns the tangent direction of the road whose endpoint is nearest to `pos`
    /// within `max_dist` metres.
    ///
    /// Returns the road tangent direction closest to `pos` within `max_dist` metres.
    ///
    /// Walks the physical geometry of every non-deleted edge and finds the closest point
    /// on the polyline (not just endpoints), returning the segment tangent at that point.
    /// This ensures mid-road snaps give the correct perpendicular direction.
    ///
    /// The returned `Vector2` is `(world_x, world_z)` of the unit tangent (either
    /// direction along the edge — callers only need the axis, not the sign).
    /// Returns `Vector2(0, 1)` (world +Z) if no road is within range.
    ///
    /// Non-blocking: returns `Vector2(0, 1)` if the SimCore mutex is contended.
    /// Returns the road tangent direction closest to `pos` within `max_dist` metres.
    #[func]
    pub fn get_road_tangent_at(&self, pos: Vector3, max_dist: f32) -> Vector2 {
        use crate::nodes::sim::bridge::network::get_road_tangent_at;
        match self.try_lock_core() {
            Some(core) => get_road_tangent_at(&core, pos, max_dist),
            None => Vector2::new(0.0, 1.0),
        }
    }
}
