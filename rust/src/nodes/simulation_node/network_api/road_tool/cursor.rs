// SPDX-License-Identifier: GPL-2.0-only

//! Cursor road-tool Godot API methods.

use super::super::super::*;

#[godot_api(secondary)]
impl SimulationNode {
    /// Returns the closest network point (node/edge) within range.
    ///
    /// Uses `try_lock` for the same reason as `intersect_terrain` — called every
    /// frame while the road tool is active; must not stall on the sim mutex.
    /// Returns `null` when contended; GDScript handles null from this call already.
    #[func]
    pub fn get_closest_network_point(&self, world_pos: Vector3, max_dist: f32) -> Variant {
        match self.try_lock_core() {
            Some(core) => match core.get_closest_network_point_internal(world_pos, max_dist) {
                Some(p) => p.to_variant(),
                None => Variant::nil(),
            },
            None => Variant::nil(),
        }
    }

    /// Resolves the road-tool cursor position in one non-blocking Rust query.
    ///
    /// This combines visible-surface picking, angle snapping, network snapping, optional
    /// ghost-guide snapping, map-border snapping, and self-snapping so the Godot editor loop
    /// does not perform several bridge calls for every mouse frame.
    #[func]
    pub fn get_road_tool_cursor_pos(
        &self,
        ray_origin: Vector3,
        ray_dir: Vector3,
        altitude_offset_m: f32,
        active: bool,
        current_state: i32,
        start_pos: Vector3,
        control_pos: Vector3,
        shift_pressed: bool,
        start_tangent_angle: f32,
        ghost_enabled: bool,
        border_snap_dist_m: f32,
        sticky_network_snap_active: bool,
        sticky_network_snap_pos: Vector3,
        sticky_network_snap_release_dist_m: f32,
    ) -> Variant {
        let query = self.road_tool_query_snapshot.read().unwrap();
        let (world_w, world_h) = query.terrain.world_size();
        let half_w = world_w * 0.5;
        let half_h = world_h * 0.5;
        let border_snap_dist = border_snap_dist_m.min(half_w.min(half_h));

        let road_hit = query.road_surface.raycast_road_visible_surface(
            &query.region_graph,
            &query.terrain,
            ray_origin,
            ray_dir,
        );
        let terrain_hit = query.terrain.raycast_visual_terrain(ray_origin, ray_dir);
        let mut pos = match Self::road_tool_closest_cursor_hit(ray_origin, road_hit, terrain_hit) {
            Some((hit, true)) => hit,
            Some((mut hit, false)) => {
                hit.y += altitude_offset_m;
                hit
            }
            None => {
                if ray_dir.y >= -0.001 {
                    return Variant::nil();
                }
                let t_plane = -ray_origin.y / ray_dir.y;
                let mut hit = ray_origin + ray_dir * t_plane;
                hit.x = hit.x.clamp(-half_w, half_w);
                hit.z = hit.z.clamp(-half_h, half_h);
                hit = Self::road_tool_snap_to_border(hit, half_w, half_h);
                hit.y =
                    Self::road_tool_cursor_height_at_xz(&query, hit.x, hit.z, altitude_offset_m);
                return hit.to_variant();
            }
        };

        if active && shift_pressed {
            let ref_pos = if current_state == 1 {
                start_pos
            } else {
                control_pos
            };
            let dir = pos - ref_pos;
            let length = dir.length();
            if length > 0.1 {
                let snap_rad = std::f32::consts::PI / 12.0;
                let relative =
                    ((dir.z.atan2(dir.x) - start_tangent_angle) / snap_rad).round() * snap_rad;
                let angle = start_tangent_angle + relative;
                let snapped_length = ((length / 10.0).round() * 10.0).max(10.0);
                let snapped_height = pos.y;
                pos = ref_pos + Vector3::new(angle.cos(), 0.0, angle.sin()) * snapped_length;
                pos.y = snapped_height;
            }
        }

        let snap_to_existing_roads = !shift_pressed;

        if snap_to_existing_roads
            && Self::road_tool_should_keep_sticky_network_snap(
                &query.region_graph,
                pos,
                sticky_network_snap_active,
                sticky_network_snap_pos,
                sticky_network_snap_release_dist_m,
            )
        {
            let mut sticky_pos = sticky_network_snap_pos;
            sticky_pos.y = Self::road_tool_cursor_height_at_xz(
                &query,
                sticky_pos.x,
                sticky_pos.z,
                altitude_offset_m,
            );
            return sticky_pos.to_variant();
        }

        if snap_to_existing_roads {
            if let Some(mut snapped_pos) =
                crate::simulation::network::interaction::get_closest_point_xz(
                    &query.region_graph,
                    pos,
                    5.0,
                )
            {
                snapped_pos.y = Self::road_tool_cursor_height_at_xz(
                    &query,
                    snapped_pos.x,
                    snapped_pos.z,
                    altitude_offset_m,
                );
                return snapped_pos.to_variant();
            }
        }

        if ghost_enabled && snap_to_existing_roads {
            use crate::nodes::sim::bridge::network::get_road_ghost_snap_from_parts;
            if let Some(ghost_snap) = get_road_ghost_snap_from_parts(
                &query.region_graph,
                &query.road_surface,
                &query.terrain,
                &query.ghost_snap_index,
                pos,
                10.0,
                altitude_offset_m,
            ) {
                return ghost_snap.to_variant();
            }
        }

        if Self::road_tool_is_near_border(pos, half_w, half_h, border_snap_dist) {
            pos = Self::road_tool_snap_to_border(pos, half_w, half_h);
            pos.y = Self::road_tool_cursor_height_at_xz(&query, pos.x, pos.z, altitude_offset_m);
            return pos.to_variant();
        }

        if active {
            if pos.distance_to(start_pos) < 2.5 {
                return start_pos.to_variant();
            }
            if current_state == 2 && pos.distance_to(control_pos) < 2.5 {
                return control_pos.to_variant();
            }
        }

        pos.to_variant()
    }

    pub(in crate::nodes::simulation_node) fn road_tool_should_keep_sticky_network_snap(
        graph: &crate::simulation::network::graph::RegionGraph,
        cursor_pos: Vector3,
        sticky_active: bool,
        sticky_pos: Vector3,
        release_dist_m: f32,
    ) -> bool {
        if !sticky_active || release_dist_m <= 0.0 {
            return false;
        }
        let dx = cursor_pos.x - sticky_pos.x;
        let dz = cursor_pos.z - sticky_pos.z;
        if dx * dx + dz * dz > release_dist_m * release_dist_m {
            return false;
        }
        crate::simulation::network::interaction::get_closest_point_xz(graph, sticky_pos, 0.25)
            .is_some_and(|network_pos| {
                let dx = network_pos.x - sticky_pos.x;
                let dz = network_pos.z - sticky_pos.z;
                dx * dx + dz * dz <= 0.25 * 0.25
            })
    }

    pub(in crate::nodes::simulation_node) fn road_tool_closest_cursor_hit(
        ray_origin: Vector3,
        road_hit: Option<Vector3>,
        terrain_hit: Option<Vector3>,
    ) -> Option<(Vector3, bool)> {
        match (road_hit, terrain_hit) {
            (Some(road), Some(terrain)) => {
                let road_dist_sq = (road - ray_origin).length_squared();
                let terrain_dist_sq = (terrain - ray_origin).length_squared();
                if road_dist_sq <= terrain_dist_sq {
                    Some((road, true))
                } else {
                    Some((terrain, false))
                }
            }
            (Some(road), None) => Some((road, true)),
            (None, Some(terrain)) => Some((terrain, false)),
            (None, None) => None,
        }
    }

    fn road_tool_cursor_height_at_xz(
        query: &RoadToolQuerySnapshot,
        x: f32,
        z: f32,
        altitude_offset_m: f32,
    ) -> f32 {
        query
            .road_surface
            .sample_visible_surface_height(&query.region_graph, &query.terrain, x, z)
            .unwrap_or_else(|| {
                query.terrain.sample_visual_height_world(x, z) * crate::config::HEIGHT_SCALE
                    + altitude_offset_m
            })
    }
}
