//! Shared fixtures for terrain-clip tests.

use super::*;

pub(super) fn terrain_clip_source_edge_for_node_test(
    start: Vector3,
    end: Vector3,
    node_id: u32,
) -> RoadSurfaceTerrainClipSourceEdge {
    terrain_clip_source_edge_for_node_kind_test(
        start,
        end,
        node_id,
        RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
    )
}
pub(super) fn terrain_clip_source_edge_for_node_kind_test(
    start: Vector3,
    end: Vector3,
    node_id: u32,
    edge_kind: RoadSurfaceTerrainClipEdgeKind,
) -> RoadSurfaceTerrainClipSourceEdge {
    RoadSurfaceTerrainClipSourceEdge {
        start,
        end,
        kind: edge_kind,
        source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind: RoadSurfaceVisualNodePieceKind::Terminal,
            owner_kind: RoadSurfaceBandKind::Sidewalk,
            owner_index: 0,
            boundary_source: None,
        },
    }
}
pub(super) fn terrain_clip_loop_for_node_test(
    points: &[Vector3],
    node_id: u32,
) -> RoadSurfaceTerrainClipLoop {
    terrain_clip_loop_for_node_kind_test(
        points,
        node_id,
        RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
    )
}
pub(super) fn terrain_clip_loop_for_node_kind_test(
    points: &[Vector3],
    node_id: u32,
    edge_kind: RoadSurfaceTerrainClipEdgeKind,
) -> RoadSurfaceTerrainClipLoop {
    RoadSurfaceTerrainClipLoop {
        source_edges: points
            .iter()
            .zip(points.iter().cycle().skip(1))
            .take(points.len())
            .map(|(&start, &end)| {
                terrain_clip_source_edge_for_node_kind_test(start, end, node_id, edge_kind)
            })
            .collect(),
        points_world: points.to_vec(),
    }
}
