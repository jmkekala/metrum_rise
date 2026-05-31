//! Chunk helpers for parcel-local broad-phase checks.

use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;

pub(crate) fn chunk_key(point: Vector2) -> (i32, i32) {
    (
        (point.x / RegionGraph::CHUNK_SIZE).floor() as i32,
        (point.y / RegionGraph::CHUNK_SIZE).floor() as i32,
    )
}

pub(crate) fn chunks_for_aabb(min: Vector2, max: Vector2) -> Vec<(i32, i32)> {
    let min_chunk = chunk_key(min);
    let max_chunk = chunk_key(max);
    let mut chunks = Vec::new();
    for cx in min_chunk.0..=max_chunk.0 {
        for cz in min_chunk.1..=max_chunk.1 {
            chunks.push((cx, cz));
        }
    }
    chunks
}
