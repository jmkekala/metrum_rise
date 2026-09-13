// SPDX-License-Identifier: GPL-2.0-only

//! Chunk helpers for parcel-local broad-phase checks.

use crate::simulation::network::graph::RegionGraph;
use godot::prelude::Vector2;

pub(crate) fn chunk_key(point: Vector2) -> (i32, i32) {
    (
        (point.x / RegionGraph::CHUNK_SIZE).floor() as i32,
        (point.y / RegionGraph::CHUNK_SIZE).floor() as i32,
    )
}

/// Visits intersected chunks in X-then-Z order without allocating a temporary list.
pub(crate) fn chunks_for_aabb(
    min: Vector2,
    max: Vector2,
) -> impl Iterator<Item = (i32, i32)> + Clone {
    let min_chunk = chunk_key(min);
    let max_chunk = chunk_key(max);
    (min_chunk.0..=max_chunk.0)
        .flat_map(move |cx| (min_chunk.1..=max_chunk.1).map(move |cz| (cx, cz)))
}
