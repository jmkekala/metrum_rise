// SPDX-License-Identifier: GPL-2.0-only

//! Cursor road-tool Godot API methods.

use super::super::super::*;
use crate::nodes::sim::road_tool::pick::{CursorQuery, road_tool_cursor};
use crate::simulation::network::interaction::{
    NetworkSnapTarget, StartDirection, road_start_direction,
};

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

    /// Resolves the road-tool cursor position in one non-blocking Rust query.
    ///
    /// This combines visible-surface picking, zoning-grid snapping, network snapping,
    /// map-border snapping, and self-snapping so the Godot editor loop
    /// does not perform several bridge calls for every mouse frame.
    /// Returns position, snap node/edge ids (-1 when absent), the snapshot generation and whether
    /// the stroke was angle-snapped.
    /// Pass the previous result back as `previous_snap` to retain a generation-checked target.
    /// `angle_snap` is the reference direction (unit XZ). While the straight-road end is being
    /// placed (`current_state` 2), a stroke from `start_pos` within 4 degrees of a 45-degree
    /// multiple from the reference snaps onto it. A zero reference leaves the cursor free.
    /// Network targets win over angle snapping. The query is `pick::road_tool_cursor`; since
    /// `ROAD-112` round C only the curve modes' GDScript path calls it.
    #[func]
    #[allow(clippy::too_many_arguments)]
    pub fn get_road_tool_cursor_pos(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
        altitude_offset_m: f32,
        active: bool,
        current_state: i32,
        start_pos: Vector3,
        control_pos: Vector3,
        zoning_snap_enabled: bool,
        fwd_lanes: i32,
        bkw_lanes: i32,
        border_snap_dist_m: f32,
        previous_snap: VarDictionary,
        sticky_network_snap_release_dist_m: f32,
        angle_snap: Vector3,
    ) -> Variant {
        let query = self.road_tool.view.read().unwrap();
        let previous_id = |key| {
            previous_snap
                .get(key)
                .and_then(|value| value.try_to::<i64>().ok())
                .unwrap_or(-1)
        };
        let cursor = road_tool_cursor(
            &query,
            &CursorQuery {
                ray_origin,
                ray_dir,
                altitude_offset_m,
                active,
                current_state,
                start_pos,
                control_pos,
                zoning_snap_enabled,
                fwd_lanes,
                bkw_lanes,
                border_snap_dist_m,
                previous_snap: decode_target(
                    previous_id("snap_node"),
                    previous_id("snap_edge"),
                    previous_id("surface_generation"),
                ),
                sticky_release_dist_m: sticky_network_snap_release_dist_m,
                angle_snap,
            },
        );
        cursor.map_or_else(Variant::nil, |cursor| {
            let mut result =
                Self::road_tool_cursor_result(cursor.position, cursor.target, cursor.surface_generation);
            if cursor.angle_snapped {
                result.set("angle_snapped", true);
            }
            result.to_variant()
        })
    }

    /// How a road started at `pos` should leave it, from the snap target the cursor returned
    /// (`snap_node`/`snap_edge`, -1 when absent, checked against `surface_generation`): `kind` 1
    /// continues a dead end along `direction` (unit XZ), 2 runs along the road through the start
    /// (`direction` either sense), 0 has no direction (free ground or a junction). Reads only the
    /// touched edges.
    #[func]
    pub fn get_road_tool_start_direction(
        &self,
        pos: Vector3,
        snap_node: i64,
        snap_edge: i64,
        surface_generation: i64,
    ) -> VarDictionary {
        let query = self.road_tool.view.read().unwrap();
        let target = decode_target(snap_node, snap_edge, surface_generation)
            .filter(|&(_, generation)| generation == query.surface_generation)
            .map(|(target, _)| target);
        let (kind, direction) = match target
            .and_then(|target| road_start_direction(&query.region_graph, target, pos))
        {
            Some(StartDirection::Continue(direction)) => (1, direction),
            Some(StartDirection::Along(direction)) => (2, direction),
            None => (0, Vector3::ZERO),
        };
        let mut result = VarDictionary::new();
        result.set("kind", kind);
        result.set("direction", direction);
        result
    }

    fn road_tool_cursor_result(
        position: Vector3,
        target: Option<NetworkSnapTarget>,
        generation: u64,
    ) -> VarDictionary {
        let mut result = VarDictionary::new();
        result.set("position", position);
        result.set(
            "snap_edge",
            match target {
                Some(NetworkSnapTarget::Edge(id)) => id as i64,
                _ => -1,
            },
        );
        result.set(
            "snap_node",
            match target {
                Some(NetworkSnapTarget::Node(id)) => i64::from(id),
                _ => -1,
            },
        );
        result.set(
            "surface_generation",
            i64::try_from(generation).unwrap_or(-1),
        );
        result
    }
}

// A network target from the bridge's ids (-1 when absent; a node before an edge) and the view
// generation it was taken at; `None` without a target or a valid generation.
fn decode_target(
    snap_node: i64,
    snap_edge: i64,
    generation: i64,
) -> Option<(NetworkSnapTarget, u64)> {
    let generation = u64::try_from(generation).ok()?;
    let target = u32::try_from(snap_node)
        .ok()
        .map(NetworkSnapTarget::Node)
        .or_else(|| usize::try_from(snap_edge).ok().map(NetworkSnapTarget::Edge))?;
    Some((target, generation))
}
