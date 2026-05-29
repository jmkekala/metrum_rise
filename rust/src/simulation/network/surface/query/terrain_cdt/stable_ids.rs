//! Stable terrain CDT footprint and loop IDs.

use super::*;

impl RoadSurfaceSystem {
    pub(super) fn terrain_cdt_stable_footprint_group_ids_for_terrain_clip_export(
        export: &RoadSurfaceTerrainClipExport,
    ) -> BTreeMap<usize, u64> {
        let mut shape_indices = export
            .loop_topologies
            .iter()
            .map(|topology| topology.shape_index)
            .collect::<Vec<_>>();
        shape_indices.sort_unstable();
        shape_indices.dedup();

        shape_indices
            .into_iter()
            .map(|shape_index| {
                (
                    shape_index,
                    Self::terrain_cdt_stable_footprint_group_id_for_terrain_clip_shape(
                        export,
                        shape_index,
                    ),
                )
            })
            .collect()
    }

    fn terrain_cdt_stable_footprint_group_id_for_terrain_clip_shape(
        export: &RoadSurfaceTerrainClipExport,
        shape_index: usize,
    ) -> u64 {
        let mut contours = export
            .loops
            .iter()
            .zip(export.loop_topologies.iter().copied())
            .filter(|(_, topology)| topology.shape_index == shape_index)
            .collect::<Vec<_>>();
        contours.sort_by_key(|(_, topology)| topology.contour_index);

        let mut hasher = TerrainClipStableHasher::new();
        hasher.write_str("terrain_clip_union_shape_v1");
        hasher.write_usize(contours.len());
        for (boundary_loop, topology) in contours {
            hasher.write_usize(topology.contour_index);
            hasher.write_usize(match topology.role {
                RoadSurfaceTerrainClipContourRole::Outer => 0,
                RoadSurfaceTerrainClipContourRole::Hole => 1,
            });
            hasher.write_usize(boundary_loop.points_world.len());
            for point in &boundary_loop.points_world {
                let key = SurfaceXzKey::from_world_xz(*point);
                hasher.write_i64(key.x_key());
                hasher.write_i64(key.z_key());
                hasher.write_i64(SurfaceHeightMmKey::from_m_f64(point.y).as_i64());
            }
            hasher.write_usize(boundary_loop.source_edges.len());
            for edge in &boundary_loop.source_edges {
                hasher.write_u64(Self::terrain_cdt_stable_piece_id_for_source(edge.source));
            }
        }
        hasher.finish()
    }

    pub(super) fn terrain_cdt_stable_piece_id_for_terrain_clip_loop(
        boundary_loop: &RoadSurfaceTerrainClipLoop,
        loop_index: usize,
    ) -> u64 {
        if boundary_loop.points_world.is_empty() {
            return terrain_cdt_usize_to_u64(loop_index);
        }

        let mut hasher = TerrainClipStableHasher::new();
        hasher.write_str("terrain_clip_union_loop_v1");
        hasher.write_usize(boundary_loop.points_world.len());
        for point in &boundary_loop.points_world {
            let key = SurfaceXzKey::from_world_xz(*point);
            hasher.write_i64(key.x_key());
            hasher.write_i64(key.z_key());
            hasher.write_i64(SurfaceHeightMmKey::from_m_f64(point.y).as_i64());
        }
        hasher.write_usize(boundary_loop.source_edges.len());
        for edge in &boundary_loop.source_edges {
            let start_key = SurfaceXzKey::from_world_xz(edge.start);
            let end_key = SurfaceXzKey::from_world_xz(edge.end);
            hasher.write_i64(start_key.x_key());
            hasher.write_i64(start_key.z_key());
            hasher.write_i64(SurfaceHeightMmKey::from_m_f64(edge.start.y).as_i64());
            hasher.write_i64(end_key.x_key());
            hasher.write_i64(end_key.z_key());
            hasher.write_i64(SurfaceHeightMmKey::from_m_f64(edge.end.y).as_i64());
            hasher.write_u64(Self::terrain_cdt_stable_piece_id_for_source(edge.source));
        }
        hasher.finish()
    }

    pub(super) fn terrain_cdt_stable_piece_id_for_source(
        source: RoadSurfaceEarthworkFaceSource,
    ) -> u64 {
        match source {
            RoadSurfaceEarthworkFaceSource::SpanSupportBoundary { edge_idx, .. } => {
                terrain_cdt_usize_to_u64(edge_idx)
            }
            RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary { node_id, .. } => {
                (1_u64 << 63) | u64::from(node_id)
            }
            RoadSurfaceEarthworkFaceSource::NodeSameMaterialBoundaryHandoff { node_id, .. } => {
                (1_u64 << 63) | u64::from(node_id)
            }
        }
    }
}

struct TerrainClipStableHasher {
    state: u64,
}

impl TerrainClipStableHasher {
    fn new() -> Self {
        Self {
            state: 0xcbf29ce484222325,
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.state ^= u64::from(byte);
            self.state = self.state.wrapping_mul(0x100000001b3);
        }
        self.state ^= 0xff;
        self.state = self.state.wrapping_mul(0x100000001b3);
    }

    fn write_i64(&mut self, value: i64) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_u64(&mut self, value: u64) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_usize(&mut self, value: usize) {
        self.write_u64(terrain_cdt_usize_to_u64(value));
    }

    fn write_str(&mut self, value: &str) {
        self.write_bytes(value.as_bytes());
    }

    fn finish(self) -> u64 {
        self.state
    }
}

pub(super) fn terrain_cdt_usize_to_u32(value: usize) -> u32 {
    u32::try_from(value).expect("terrain CDT export index must fit u32")
}

pub(super) fn terrain_cdt_usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).expect("terrain CDT export index must fit u64")
}
