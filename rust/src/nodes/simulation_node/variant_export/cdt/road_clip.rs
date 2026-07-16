//! Road-clip query and road-loop variant export helpers.

use super::super::super::*;

impl SimulationNode {
    pub(in crate::nodes::simulation_node) fn append_road_clip_loops_for_bounds(
        dict: &mut VarDictionary,
        core: &SimCore,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) {
        let road_clip_query =
            Self::road_clip_loop_query_for_bounds(core, min_x, min_z, max_x, max_z);
        Self::append_road_clip_query(dict, &road_clip_query);
    }

    pub(in crate::nodes::simulation_node) fn road_clip_loop_query_for_bounds(
        core: &SimCore,
        min_x: f32,
        min_z: f32,
        max_x: f32,
        max_z: f32,
    ) -> RoadClipLoopQuery {
        match core
            .transit_network
            .road_surface
            .terrain_cdt_road_loops_for_world_bounds(&core.region_graph, min_x, min_z, max_x, max_z)
        {
            Ok((mut cdt_road_loops, source_count)) => {
                let site_loops = core
                    .allocator
                    .terrain_cdt_site_loops_for_world_bounds(min_x, min_z, max_x, max_z);
                let site_source_count = site_loops.len();
                cdt_road_loops.extend(site_loops);
                RoadClipLoopQuery {
                    cdt_road_loops,
                    source_count: source_count + site_source_count,
                    clip_error_label: None,
                }
            }
            Err(err) => {
                let site_loops = core
                    .allocator
                    .terrain_cdt_site_loops_for_world_bounds(min_x, min_z, max_x, max_z);
                let source_count = site_loops.len();
                RoadClipLoopQuery {
                    cdt_road_loops: site_loops,
                    source_count,
                    clip_error_label: Some(err.debug_label()),
                }
            }
        }
    }

    pub(in crate::nodes::simulation_node) fn append_road_clip_query(
        dict: &mut VarDictionary,
        road_clip_query: &RoadClipLoopQuery,
    ) {
        Self::append_road_clip_status(dict, road_clip_query);
        dict.set(
            "road_clip_signature",
            Self::road_clip_query_signature(road_clip_query),
        );
        Self::append_road_clip_loops(dict, &road_clip_query.cdt_road_loops);
    }

    pub(in crate::nodes::simulation_node) fn append_road_clip_status(
        dict: &mut VarDictionary,
        road_clip_query: &RoadClipLoopQuery,
    ) {
        let (status, error, source_count) = Self::road_clip_status_values(road_clip_query);
        dict.set("road_clip_status", GString::from(status));
        dict.set("road_clip_error", GString::from(error));
        dict.set("road_clip_source_count", source_count);
    }

    pub(in crate::nodes::simulation_node) fn road_clip_status_values(
        road_clip_query: &RoadClipLoopQuery,
    ) -> (&'static str, &'static str, i64) {
        let (status, error) = if let Some(error_label) = road_clip_query.clip_error_label {
            ("failed", error_label)
        } else {
            ("ok", "none")
        };
        (
            status,
            error,
            i64::try_from(road_clip_query.source_count).unwrap_or(0),
        )
    }

    pub(in crate::nodes::simulation_node) fn append_road_clip_loops(
        dict: &mut VarDictionary,
        road_clip_loops: &[TerrainCdtRoadLoop],
    ) {
        let point_count: usize = road_clip_loops
            .iter()
            .map(|road_loop| road_loop.vertices.len())
            .sum();
        dict.set(
            "road_clip_loop_count",
            i64::try_from(road_clip_loops.len()).unwrap_or(0),
        );
        dict.set(
            "road_clip_point_count",
            i64::try_from(point_count).unwrap_or(0),
        );
        if road_clip_loops.is_empty() {
            return;
        }

        let mut group_ids = BTreeMap::<u64, i32>::new();
        let mut loop_counts = Vec::with_capacity(road_clip_loops.len());
        let mut loop_groups = Vec::with_capacity(road_clip_loops.len());
        let mut loop_roles = Vec::with_capacity(road_clip_loops.len());
        let mut loop_points = Vec::new();
        for road_loop in road_clip_loops {
            let next_group_id = i32::try_from(group_ids.len()).unwrap_or(i32::MAX);
            let group_id = *group_ids
                .entry(road_loop.footprint_group_id)
                .or_insert(next_group_id);
            loop_counts.push(i32::try_from(road_loop.vertices.len()).unwrap_or(0));
            loop_groups.push(group_id);
            loop_roles.push(if road_loop.is_hole { 1 } else { 0 });
            loop_points.extend(
                road_loop
                    .vertices
                    .iter()
                    .map(|vertex| Vector3::new(vertex.x as f32, vertex.height_m, vertex.z as f32)),
            );
        }
        dict.set(
            "road_clip_loop_counts",
            PackedInt32Array::from_iter(loop_counts),
        );
        dict.set(
            "road_clip_loop_groups",
            PackedInt32Array::from_iter(loop_groups),
        );
        dict.set(
            "road_clip_loop_roles",
            PackedInt32Array::from_iter(loop_roles),
        );
        dict.set(
            "road_clip_loop_points",
            PackedVector3Array::from_iter(loop_points),
        );
    }

    pub(in crate::nodes::simulation_node) fn road_clip_query_signature(
        road_clip_query: &RoadClipLoopQuery,
    ) -> i64 {
        let mut hash = 0xcbf29ce484222325_u64;
        Self::mix_road_clip_signature(&mut hash, road_clip_query.source_count as u64);
        if let Some(error_label) = road_clip_query.clip_error_label {
            for byte in error_label.as_bytes() {
                Self::mix_road_clip_signature(&mut hash, u64::from(*byte));
            }
        }
        for road_loop in &road_clip_query.cdt_road_loops {
            Self::mix_road_clip_signature(&mut hash, road_loop.footprint_group_id);
            Self::mix_road_clip_signature(&mut hash, u64::from(road_loop.is_hole));
            Self::mix_road_clip_signature(&mut hash, road_loop.vertices.len() as u64);
            for vertex in &road_loop.vertices {
                Self::mix_road_clip_signature(
                    &mut hash,
                    ((vertex.x * 1000.0).round() as i64) as u64,
                );
                Self::mix_road_clip_signature(
                    &mut hash,
                    ((vertex.z * 1000.0).round() as i64) as u64,
                );
                Self::mix_road_clip_signature(
                    &mut hash,
                    ((vertex.height_m * 1000.0).round() as i64) as u64,
                );
            }
        }
        i64::from_ne_bytes(hash.to_ne_bytes())
    }

    pub(in crate::nodes::simulation_node) fn mix_road_clip_signature(hash: &mut u64, value: u64) {
        *hash ^= value;
        *hash = hash.wrapping_mul(0x100000001b3);
    }
}
