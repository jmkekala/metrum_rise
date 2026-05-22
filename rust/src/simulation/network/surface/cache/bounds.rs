//! Chunk bounds and piece AABB helpers.

use super::*;

impl RoadSurfaceSystem {
    pub(in crate::simulation::network::surface) fn visual_node_piece_bounds(
        &self,
        piece: &RoadSurfaceVisualNodePiece,
        kind: ChunkCacheKind,
    ) -> Option<(Vector3, Vector3)> {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        let mut saw_point = false;

        match kind {
            ChunkCacheKind::Surface => {
                for point in piece
                    .outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
            }
            ChunkCacheKind::Earthwork => {
                for point in piece
                    .earthwork_outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
                if !saw_point {
                    for point in piece
                        .earthwork_surface_polygons
                        .iter()
                        .flat_map(|polygon| polygon.points_world.iter())
                    {
                        min_x = min_x.min(point.x);
                        max_x = max_x.max(point.x);
                        min_z = min_z.min(point.z);
                        max_z = max_z.max(point.z);
                        saw_point = true;
                    }
                }
            }
        }

        saw_point.then_some((
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        ))
    }

    pub(in crate::simulation::network::surface) fn visual_span_piece_bounds(
        &self,
        piece: &RoadSurfaceVisualSpanPiece,
        kind: ChunkCacheKind,
    ) -> Option<(Vector3, Vector3)> {
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_z = f32::MAX;
        let mut max_z = f32::MIN;
        let mut saw_point = false;

        match kind {
            ChunkCacheKind::Surface => {
                for point in piece
                    .outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
            }
            ChunkCacheKind::Earthwork => {
                for point in piece
                    .earthwork_outer_boundary_loops
                    .iter()
                    .flat_map(|polygon| polygon.points_world.iter())
                {
                    min_x = min_x.min(point.x);
                    max_x = max_x.max(point.x);
                    min_z = min_z.min(point.z);
                    max_z = max_z.max(point.z);
                    saw_point = true;
                }
                if !saw_point {
                    for point in piece
                        .earthwork_surface_polygons
                        .iter()
                        .flat_map(|polygon| polygon.points_world.iter())
                    {
                        min_x = min_x.min(point.x);
                        max_x = max_x.max(point.x);
                        min_z = min_z.min(point.z);
                        max_z = max_z.max(point.z);
                        saw_point = true;
                    }
                }
            }
        }

        saw_point.then_some((
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
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
        world_x: f32,
        world_z: f32,
    ) -> SurfaceChunkKey {
        (
            (world_x / self.chunk_span_m).floor() as i32,
            (world_z / self.chunk_span_m).floor() as i32,
        )
    }

    pub(in crate::simulation::network::surface) fn chunk_bounds(
        &self,
        chunk: SurfaceChunkKey,
    ) -> (Vector3, Vector3) {
        let min_x = chunk.0 as f32 * self.chunk_span_m;
        let min_z = chunk.1 as f32 * self.chunk_span_m;
        let max_x = min_x + self.chunk_span_m;
        let max_z = min_z + self.chunk_span_m;
        (
            Vector3::new(min_x, 0.0, min_z),
            Vector3::new(max_x, 0.0, max_z),
        )
    }

    pub(in crate::simulation::network::surface) fn bounds_to_chunk_keys(
        &self,
        min: Vector3,
        max: Vector3,
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
}
