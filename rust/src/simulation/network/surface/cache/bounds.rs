//! Chunk bounds and piece AABB helpers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn visual_node_piece_bounds(
        &self,
        piece: &RoadSurfaceVisualNodePiece,
        kind: ChunkCacheKind,
    ) -> Option<(RoadVec3, RoadVec3)> {
        let mut min_x = f64::MAX;
        let mut max_x = f64::MIN;
        let mut min_z = f64::MAX;
        let mut max_z = f64::MIN;
        let mut saw_point = false;

        match kind {
            ChunkCacheKind::Surface => {
                for point in piece
                    .outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(f64::from(point.x));
                    max_x = max_x.max(f64::from(point.x));
                    min_z = min_z.min(f64::from(point.z));
                    max_z = max_z.max(f64::from(point.z));
                    saw_point = true;
                }
            }
            ChunkCacheKind::Earthwork => {
                for point in piece
                    .earthwork_outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(f64::from(point.x));
                    max_x = max_x.max(f64::from(point.x));
                    min_z = min_z.min(f64::from(point.z));
                    max_z = max_z.max(f64::from(point.z));
                    saw_point = true;
                }
                if !saw_point {
                    for point in piece
                        .earthwork_surface_polygons
                        .iter()
                        .flat_map(|polygon| polygon.points_world.iter())
                    {
                        min_x = min_x.min(f64::from(point.x));
                        max_x = max_x.max(f64::from(point.x));
                        min_z = min_z.min(f64::from(point.z));
                        max_z = max_z.max(f64::from(point.z));
                        saw_point = true;
                    }
                }
            }
        }

        saw_point.then_some((
            RoadVec3::new(min_x, 0.0, min_z),
            RoadVec3::new(max_x, 0.0, max_z),
        ))
    }

    pub(in crate::simulation::network::surface) fn visual_span_piece_bounds(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        kind: ChunkCacheKind,
    ) -> Option<(RoadVec3, RoadVec3)> {
        let mut min_x = f64::MAX;
        let mut max_x = f64::MIN;
        let mut min_z = f64::MAX;
        let mut max_z = f64::MIN;
        let mut saw_point = false;

        match kind {
            ChunkCacheKind::Surface => {
                for point in piece
                    .outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(f64::from(point.x));
                    max_x = max_x.max(f64::from(point.x));
                    min_z = min_z.min(f64::from(point.z));
                    max_z = max_z.max(f64::from(point.z));
                    saw_point = true;
                }
            }
            ChunkCacheKind::Earthwork => {
                for point in piece
                    .earthwork_outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(f64::from(point.x));
                    max_x = max_x.max(f64::from(point.x));
                    min_z = min_z.min(f64::from(point.z));
                    max_z = max_z.max(f64::from(point.z));
                    saw_point = true;
                }
                if !saw_point {
                    for point in piece
                        .earthwork_surface_polygons
                        .iter()
                        .flat_map(|polygon| polygon.points_world.iter())
                    {
                        min_x = min_x.min(f64::from(point.x));
                        max_x = max_x.max(f64::from(point.x));
                        min_z = min_z.min(f64::from(point.z));
                        max_z = max_z.max(f64::from(point.z));
                        saw_point = true;
                    }
                }
            }
        }

        saw_point.then_some((
            RoadVec3::new(min_x, 0.0, min_z),
            RoadVec3::new(max_x, 0.0, max_z),
        ))
    }

    pub(in crate::simulation::network::surface) fn sorted_chunk_keys(
        &self,
        chunks: &HashSet<SurfaceChunkKey>,
    ) -> Vec<SurfaceChunkKey> {
        let mut chunks: Vec<SurfaceChunkKey> = chunks.iter().copied().collect();
        chunks.sort_unstable();
        chunks
    }

    pub(in crate::simulation::network::surface) fn chunk_coords_for_world(
        &self,
        world_x: f64,
        world_z: f64,
    ) -> SurfaceChunkKey {
        (
            ((world_x - f64::from(self.chunk_origin_x_m)) / f64::from(self.chunk_span_m)).floor()
                as i32,
            ((world_z - f64::from(self.chunk_origin_z_m)) / f64::from(self.chunk_span_m)).floor()
                as i32,
        )
    }

    pub(in crate::simulation::network::surface) fn chunk_bounds(
        &self,
        chunk: SurfaceChunkKey,
    ) -> (RoadVec3, RoadVec3) {
        let chunk_span_m = f64::from(self.chunk_span_m);
        let min_x = f64::from(self.chunk_origin_x_m) + f64::from(chunk.0) * chunk_span_m;
        let min_z = f64::from(self.chunk_origin_z_m) + f64::from(chunk.1) * chunk_span_m;
        let max_x = min_x + chunk_span_m;
        let max_z = min_z + chunk_span_m;
        (
            RoadVec3::new(min_x, 0.0, min_z),
            RoadVec3::new(max_x, 0.0, max_z),
        )
    }

    pub(in crate::simulation::network::surface) fn bounds_to_chunk_keys(
        &self,
        min: RoadVec3,
        max: RoadVec3,
    ) -> Vec<SurfaceChunkKey> {
        let min_chunk = self.chunk_coords_for_world(min.x, min.z);
        let max_chunk = self.chunk_coords_for_world(max.x, max.z);
        let mut chunks = Vec::new();
        for cx in min_chunk.0..=max_chunk.0 {
            for cz in min_chunk.1..=max_chunk.1 {
                chunks.push((cx, cz));
            }
        }
        chunks
    }

    pub(in crate::simulation::network::surface) fn query_chunk_coords_for_world(
        world_x: f64,
        world_z: f64,
    ) -> SurfaceChunkKey {
        (
            (world_x / SURFACE_QUERY_CHUNK_SPAN_M).floor() as i32,
            (world_z / SURFACE_QUERY_CHUNK_SPAN_M).floor() as i32,
        )
    }

    /// Returns the fixed world-space bounds of one fine road-query chunk.
    pub fn query_chunk_world_bounds(chunk: SurfaceChunkKey) -> (RoadVec3, RoadVec3) {
        let min_x = f64::from(chunk.0) * SURFACE_QUERY_CHUNK_SPAN_M;
        let min_z = f64::from(chunk.1) * SURFACE_QUERY_CHUNK_SPAN_M;
        (
            RoadVec3::new(min_x, 0.0, min_z),
            RoadVec3::new(
                min_x + SURFACE_QUERY_CHUNK_SPAN_M,
                0.0,
                min_z + SURFACE_QUERY_CHUNK_SPAN_M,
            ),
        )
    }

    pub(super) fn bounds_to_query_chunk_keys(min: RoadVec3, max: RoadVec3) -> Vec<SurfaceChunkKey> {
        let min_chunk = Self::query_chunk_coords_for_world(min.x, min.z);
        let max_chunk = Self::query_chunk_coords_for_world(max.x, max.z);
        let mut chunks = Vec::new();
        for cx in min_chunk.0..=max_chunk.0 {
            for cz in min_chunk.1..=max_chunk.1 {
                chunks.push((cx, cz));
            }
        }
        chunks
    }
}
