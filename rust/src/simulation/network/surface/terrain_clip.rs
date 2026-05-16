//! Source-preserving terrain-clip boundary export for owned road pieces.

use super::{
    NODE_OVERLAY_MIN_AREA_M2, NODE_OVERLAY_NUMERIC_DUST_WIDTH_M, NodeOverlayContour,
    NodeOverlayPoint, NodeOverlayShape, RoadSurfaceBandKind, RoadSurfaceSystem,
    RoadSurfaceVisualPolygon, WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2,
    earthwork::RoadSurfaceEarthworkFaceSource,
    keys::{SurfaceSegmentParameter, SurfaceXzKey, SurfaceXzSegmentKey},
};
use godot::prelude::Vector3;
use std::collections::{BTreeMap, BTreeSet};

use super::backend::ROAD_OVERLAY_COORDINATE_SCALE;

const NODE_OVERLAY_SCALE: f64 = ROAD_OVERLAY_COORDINATE_SCALE;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum RoadSurfaceTerrainClipEdgeKind {
    SidewalkOuter,
    ShoulderOuter,
    FootprintBoundary,
    SpanHandoff,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipSourceEdge {
    pub(crate) start: Vector3,
    pub(crate) end: Vector3,
    pub(crate) kind: RoadSurfaceTerrainClipEdgeKind,
    pub(crate) source: RoadSurfaceEarthworkFaceSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipLoop {
    pub(crate) points_world: Vec<Vector3>,
    pub(crate) source_edges: Vec<RoadSurfaceTerrainClipSourceEdge>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RoadSurfaceTerrainClipExport {
    pub(crate) loops: Vec<RoadSurfaceTerrainClipLoop>,
    pub(crate) polygons: Vec<RoadSurfaceVisualPolygon>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RoadSurfaceTerrainClipExportError {
    OverlayUnionFailed {
        source_loop_count: usize,
    },
    MissingOuterBoundaryOwner {
        shape_index: usize,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        context: String,
    },
    MissingOutputBoundaryOwner {
        shape_index: usize,
        start: Vector3,
        end: Vector3,
    },
    UnclosedOutputBoundary {
        shape_index: usize,
        start: Vector3,
        end: Vector3,
    },
}

impl RoadSurfaceTerrainClipExportError {
    pub(crate) fn debug_label(&self) -> &'static str {
        match self {
            Self::OverlayUnionFailed { .. } => "terrain_clip_overlay_union_failed",
            Self::MissingOuterBoundaryOwner { .. } => "terrain_clip_missing_outer_boundary_owner",
            Self::MissingOutputBoundaryOwner { .. } => "terrain_clip_missing_output_boundary_owner",
            Self::UnclosedOutputBoundary { .. } => "terrain_clip_unclosed_output_boundary",
        }
    }
}

#[derive(Clone, Copy)]
struct TerrainClipSourceEdge {
    start: Vector3,
    end: Vector3,
    kind: RoadSurfaceTerrainClipEdgeKind,
    source: RoadSurfaceEarthworkFaceSource,
    source_index: usize,
    edge_index: usize,
}

#[derive(Clone, Copy)]
struct TerrainClipSegmentHeights {
    start_y: f32,
    end_y: f32,
}

#[derive(Clone, Copy)]
struct TerrainClipEndpointSample {
    kind: RoadSurfaceTerrainClipEdgeKind,
    source_index: usize,
    edge_index: usize,
    y: f32,
}

#[derive(Clone, Copy)]
struct TerrainClipSourceInterval {
    start_t: f64,
    end_t: f64,
    start_y: f32,
    end_y: f32,
}

#[derive(Clone, Debug, PartialEq)]
enum TerrainClipSegmentPointRecovery {
    Degenerate,
    Covered(Vec<Vector3>),
    Partial,
    Missing,
}

type OverlaySegmentParameter = SurfaceSegmentParameter;

pub(crate) fn terrain_clip_edge_kind_for_band(
    kind: RoadSurfaceBandKind,
) -> RoadSurfaceTerrainClipEdgeKind {
    match kind {
        RoadSurfaceBandKind::Sidewalk => RoadSurfaceTerrainClipEdgeKind::SidewalkOuter,
        RoadSurfaceBandKind::CurbOrShoulder => RoadSurfaceTerrainClipEdgeKind::ShoulderOuter,
        _ => RoadSurfaceTerrainClipEdgeKind::FootprintBoundary,
    }
}

impl RoadSurfaceSystem {
    fn overlay_contours_from_terrain_clip_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Vec<NodeOverlayContour> {
        let mut contours = Vec::new();
        for boundary_loop in boundary_loops {
            let contour = Self::overlay_contour_from_world_points(&boundary_loop.points_world);
            if Self::overlay_contour_area(&contour).abs() > NODE_OVERLAY_MIN_AREA_M2 {
                contours.push(contour);
            }
        }
        contours
    }

    fn overlay_contour_from_world_points(points_world: &[Vector3]) -> NodeOverlayContour {
        let mut contour = Vec::with_capacity(points_world.len());
        for point in points_world {
            let overlay_point = Self::overlay_point_from_world_point(*point);
            if contour
                .last()
                .is_none_or(|last: &NodeOverlayPoint| *last != overlay_point)
            {
                contour.push(overlay_point);
            }
        }
        if contour.len() >= 2 && contour.first() == contour.last() {
            contour.pop();
        }
        contour
    }

    fn overlay_point_from_world_point(point: Vector3) -> NodeOverlayPoint {
        [
            (f64::from(point.x) * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
            (f64::from(point.z) * NODE_OVERLAY_SCALE).round() / NODE_OVERLAY_SCALE,
        ]
    }

    pub(super) fn union_terrain_clip_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<Vec<RoadSurfaceVisualPolygon>, RoadSurfaceTerrainClipExportError> {
        Self::union_terrain_clip_boundary_loops_with_sources(boundary_loops).map(|loops| {
            loops
                .into_iter()
                .filter_map(|boundary_loop| {
                    Self::make_boundary_loop_polygon(boundary_loop.points_world)
                })
                .collect()
        })
    }

    pub(super) fn union_terrain_clip_boundary_export(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<RoadSurfaceTerrainClipExport, RoadSurfaceTerrainClipExportError> {
        let loops = Self::union_terrain_clip_boundary_loops_with_sources(boundary_loops)?;
        let mut polygons = loops
            .iter()
            .filter_map(|boundary_loop| {
                Self::make_boundary_loop_polygon(boundary_loop.points_world.clone())
            })
            .collect::<Vec<_>>();
        Self::sort_visual_polygons(&mut polygons);
        Ok(RoadSurfaceTerrainClipExport { loops, polygons })
    }

    pub(super) fn union_terrain_clip_boundary_loops_with_sources(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Result<Vec<RoadSurfaceTerrainClipLoop>, RoadSurfaceTerrainClipExportError> {
        if boundary_loops.is_empty() {
            return Ok(Vec::new());
        }

        let contours = Self::overlay_contours_from_terrain_clip_boundary_loops(boundary_loops);
        let Some(mut shapes) = Self::overlay_union_contours(&contours) else {
            return Err(RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: boundary_loops.len(),
            });
        };
        Self::sort_overlay_shapes(&mut shapes);
        let source_edges = Self::terrain_clip_source_edges_from_boundary_loops(boundary_loops);
        Self::terrain_clip_boundary_loops_from_overlay_shapes_with_source_edges(
            &shapes,
            &source_edges,
        )
    }

    fn terrain_clip_boundary_loops_from_overlay_shapes_with_source_edges(
        shapes: &[NodeOverlayShape],
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<Vec<RoadSurfaceTerrainClipLoop>, RoadSurfaceTerrainClipExportError> {
        let mut loops = Vec::new();
        for (shape_index, shape) in shapes.iter().enumerate() {
            let boundary_loop =
                Self::terrain_clip_boundary_loop_from_overlay_shape_with_source_edges(
                    shape,
                    shape_index,
                    source_edges,
                )?;
            loops.push(boundary_loop);
        }
        Self::sort_terrain_clip_loops(&mut loops);
        Ok(loops)
    }

    fn terrain_clip_boundary_loop_from_overlay_shape_with_source_edges(
        shape: &NodeOverlayShape,
        shape_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<RoadSurfaceTerrainClipLoop, RoadSurfaceTerrainClipExportError> {
        let outer_contour =
            shape
                .first()
                .ok_or_else(|| RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                    source_loop_count: 0,
                })?;
        let contour = Self::compact_overlay_contour_by_key(outer_contour);
        if contour.len() < 3 {
            return Err(RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: 0,
            });
        }

        let mut output_edges = Vec::new();
        for index in 0..contour.len() {
            let start = contour[index];
            let end = contour[(index + 1) % contour.len()];
            let segment_points = match Self::terrain_clip_segment_points_from_source_edges(
                start,
                end,
                source_edges,
            ) {
                TerrainClipSegmentPointRecovery::Degenerate => continue,
                TerrainClipSegmentPointRecovery::Covered(points) => points,
                TerrainClipSegmentPointRecovery::Partial => {
                    let context = format!(
                        "partial_coverage {}",
                        Self::terrain_clip_missing_source_context_label(start, end, source_edges)
                    );
                    crate::debug_log!(
                        "road",
                        "terrain_clip_missing_outer_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                        shape_index,
                        start[0],
                        start[1],
                        end[0],
                        end[1],
                        context
                    );
                    return Err(
                        RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner {
                            shape_index,
                            start,
                            end,
                            context,
                        },
                    );
                }
                TerrainClipSegmentPointRecovery::Missing => {
                    if let Some(points) = Self::terrain_clip_dust_connector_points_from_source_edges(
                        &contour,
                        index,
                        source_edges,
                    ) {
                        points
                    } else if let Some(points) =
                        Self::terrain_clip_source_chain_points_from_source_edges(
                            start,
                            end,
                            source_edges,
                        )
                    {
                        points
                    } else {
                        let context = Self::terrain_clip_missing_source_context_label(
                            start,
                            end,
                            source_edges,
                        );
                        crate::debug_log!(
                            "road",
                            "terrain_clip_missing_outer_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3}) {}",
                            shape_index,
                            start[0],
                            start[1],
                            end[0],
                            end[1],
                            context
                        );
                        return Err(
                            RoadSurfaceTerrainClipExportError::MissingOuterBoundaryOwner {
                                shape_index,
                                start,
                                end,
                                context,
                            },
                        );
                    }
                }
            };
            if let Err((missing_start, missing_end)) =
                Self::append_terrain_clip_sourced_segment_points(
                    &mut output_edges,
                    segment_points,
                    source_edges,
                )
            {
                crate::debug_log!(
                    "road",
                    "terrain_clip_missing_output_boundary_owner shape={} start=({:.3},{:.3}) end=({:.3},{:.3})",
                    shape_index,
                    missing_start.x,
                    missing_start.z,
                    missing_end.x,
                    missing_end.z
                );
                return Err(
                    RoadSurfaceTerrainClipExportError::MissingOutputBoundaryOwner {
                        shape_index,
                        start: missing_start,
                        end: missing_end,
                    },
                );
            }
        }

        Self::close_terrain_clip_source_edges(&mut output_edges);
        if output_edges.len() < 3 {
            return Err(RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: 0,
            });
        }
        let first_start = output_edges.first().map(|edge| edge.start).ok_or_else(|| {
            RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: 0,
            }
        })?;
        let last_end = output_edges.last().map(|edge| edge.end).ok_or_else(|| {
            RoadSurfaceTerrainClipExportError::OverlayUnionFailed {
                source_loop_count: 0,
            }
        })?;
        if !Self::world_points_same_for_boundary(first_start, last_end) {
            crate::debug_log!(
                "road",
                "terrain_clip_unclosed_output_boundary shape={} start=({:.3},{:.3}) end=({:.3},{:.3})",
                shape_index,
                first_start.x,
                first_start.z,
                last_end.x,
                last_end.z
            );
            return Err(RoadSurfaceTerrainClipExportError::UnclosedOutputBoundary {
                shape_index,
                start: first_start,
                end: last_end,
            });
        }
        let points_world = output_edges.iter().map(|edge| edge.start).collect();
        Ok(RoadSurfaceTerrainClipLoop {
            points_world,
            source_edges: output_edges,
        })
    }

    fn compact_overlay_contour_by_key(contour: &NodeOverlayContour) -> NodeOverlayContour {
        let mut compact = Vec::with_capacity(contour.len());
        for &point in contour {
            if compact
                .last()
                .is_none_or(|last| !overlay_points_same_for_boundary(*last, point))
            {
                compact.push(point);
            }
        }
        while compact.len() >= 2
            && overlay_points_same_for_boundary(*compact.first().unwrap(), *compact.last().unwrap())
        {
            compact.pop();
        }
        remove_repeated_overlay_point_spurs(&mut compact);
        compact
    }

    fn terrain_clip_source_edges_from_boundary_loops(
        boundary_loops: &[RoadSurfaceTerrainClipLoop],
    ) -> Vec<TerrainClipSourceEdge> {
        let mut edges = Vec::new();
        for (source_index, boundary_loop) in boundary_loops.iter().enumerate() {
            for (edge_index, source_edge) in boundary_loop.source_edges.iter().copied().enumerate()
            {
                let start = Self::terrain_clip_canonical_loop_point(
                    source_edge.start,
                    &boundary_loop.points_world,
                );
                let end = Self::terrain_clip_canonical_loop_point(
                    source_edge.end,
                    &boundary_loop.points_world,
                );
                if Self::terrain_clip_world_key(start) == Self::terrain_clip_world_key(end) {
                    continue;
                }
                edges.push(TerrainClipSourceEdge {
                    start,
                    end,
                    kind: source_edge.kind,
                    source: source_edge.source,
                    source_index,
                    edge_index,
                });
            }
        }
        // Keep source endpoints on one canonical coordinate only when they are already the same
        // solved-height seam endpoint after boundary-loop representation cleanup.
        Self::canonicalize_terrain_clip_source_endpoint_groups(&mut edges);

        edges.sort_by_key(|edge| {
            let start_key = Self::terrain_clip_world_key(edge.start);
            let end_key = Self::terrain_clip_world_key(edge.end);
            let edge_key = SurfaceXzSegmentKey::new(start_key, end_key);
            (
                edge_key.start(),
                edge_key.end(),
                terrain_clip_edge_kind_priority(edge.kind),
                Self::overlay_height_key(edge.start.y),
                Self::overlay_height_key(edge.end.y),
                edge.source_index,
                edge.edge_index,
            )
        });
        edges
    }

    fn canonicalize_terrain_clip_source_endpoint_groups(edges: &mut [TerrainClipSourceEdge]) {
        let mut groups: Vec<Vec<Vector3>> = Vec::new();
        for point in edges.iter().flat_map(|edge| [edge.start, edge.end]) {
            if let Some(group) = groups.iter_mut().find(|group| {
                group.iter().any(|candidate| {
                    Self::terrain_clip_source_points_share_canonical_endpoint(*candidate, point)
                })
            }) {
                group.push(point);
            } else {
                groups.push(vec![point]);
            }
        }

        let mut replacements = BTreeMap::new();
        for group in groups {
            if group.len() < 2 {
                continue;
            }
            let mut point_counts = BTreeMap::<(i64, i64, i64), (usize, Vector3)>::new();
            for point in group {
                let key = Self::terrain_clip_source_point_group_key(point);
                let entry = point_counts.entry(key).or_insert((0, point));
                entry.0 += 1;
            }
            let mut counted_points = point_counts.into_iter().collect::<Vec<_>>();
            counted_points.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(&b.0)));
            let Some((_, (_, replacement))) = counted_points.first().copied() else {
                continue;
            };
            for (key, _) in counted_points {
                replacements.insert(key, replacement);
            }
        }

        for edge in edges {
            if let Some(point) =
                replacements.get(&Self::terrain_clip_source_point_group_key(edge.start))
            {
                edge.start = *point;
            }
            if let Some(point) =
                replacements.get(&Self::terrain_clip_source_point_group_key(edge.end))
            {
                edge.end = *point;
            }
        }
    }

    fn terrain_clip_source_points_share_canonical_endpoint(a: Vector3, b: Vector3) -> bool {
        let dx = a.x - b.x;
        let dz = a.z - b.z;
        dx * dx + dz * dz <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
            && Self::overlay_height_key(a.y) == Self::overlay_height_key(b.y)
    }

    fn terrain_clip_source_point_group_key(point: Vector3) -> (i64, i64, i64) {
        let key = Self::terrain_clip_world_key(point);
        (key.x_key(), key.z_key(), Self::overlay_height_key(point.y))
    }

    fn terrain_clip_canonical_loop_point(point: Vector3, loop_points: &[Vector3]) -> Vector3 {
        loop_points
            .iter()
            .copied()
            .find(|candidate| {
                let dx = candidate.x - point.x;
                let dz = candidate.z - point.z;
                dx * dx + dz * dz <= WORLD_POINT_DEDUP_DISTANCE_SQUARED_M2
                    && Self::overlay_height_key(candidate.y) == Self::overlay_height_key(point.y)
            })
            .unwrap_or(point)
    }

    fn terrain_clip_segment_heights_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSegmentHeights> {
        let TerrainClipSegmentPointRecovery::Covered(points) =
            Self::terrain_clip_segment_points_from_source_edges(start, end, source_edges)
        else {
            return None;
        };
        Some(TerrainClipSegmentHeights {
            start_y: points.first()?.y,
            end_y: points.last()?.y,
        })
    }

    fn terrain_clip_segment_points_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipSegmentPointRecovery {
        if Self::terrain_clip_overlay_key(start) == Self::terrain_clip_overlay_key(end) {
            return TerrainClipSegmentPointRecovery::Degenerate;
        }

        let mut samples = Vec::new();
        for &source_edge in source_edges {
            if let Some(interval) =
                Self::terrain_clip_source_interval_on_segment(start, end, source_edge)
            {
                samples.push(interval);
            }
        }
        Self::terrain_clip_points_from_interval_coverage(start, end, samples)
    }

    fn terrain_clip_source_chain_points_from_source_edges(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<Vec<Vector3>> {
        if Self::terrain_clip_overlay_key(start) == Self::terrain_clip_overlay_key(end) {
            return None;
        }

        let start_key = Self::terrain_clip_overlay_key(start);
        let end_key = Self::terrain_clip_overlay_key(end);
        let source_indices = source_edges
            .iter()
            .map(|edge| edge.source_index)
            .collect::<BTreeSet<_>>();
        let mut best: Option<(usize, usize, Vec<Vector3>)> = None;
        for source_index in source_indices.iter().copied() {
            let mut source_chain_edges = source_edges
                .iter()
                .copied()
                .filter(|edge| edge.source_index == source_index)
                .collect::<Vec<_>>();
            source_chain_edges.sort_by_key(|edge| edge.edge_index);
            if source_chain_edges.len() < 2 {
                continue;
            }

            let start_positions =
                Self::terrain_clip_source_loop_positions_at_key(start_key, &source_chain_edges);
            let end_positions =
                Self::terrain_clip_source_loop_positions_at_key(end_key, &source_chain_edges);
            for start_position in start_positions {
                for end_position in end_positions.iter().copied() {
                    if start_position == end_position {
                        continue;
                    }
                    let Some(path_keys) = Self::terrain_clip_ordered_source_loop_key_path(
                        &source_chain_edges,
                        start_position,
                        end_position,
                    ) else {
                        continue;
                    };
                    let source_edge_count = path_keys.len().saturating_sub(1);
                    let mut points = path_keys
                        .into_iter()
                        .filter_map(|key| {
                            Self::terrain_clip_source_point_for_vertex_key(key, source_edges)
                        })
                        .collect::<Vec<_>>();
                    Self::raise_terrain_clip_points_to_highest_source_heights(
                        &mut points,
                        source_edges,
                    );
                    Self::dedup_terrain_clip_segment_points(&mut points);
                    if points.len() < 2 {
                        continue;
                    }
                    if best.as_ref().is_none_or(
                        |(best_source_index, best_edge_count, best_points)| {
                            (source_index, source_edge_count, points.len())
                                < (*best_source_index, *best_edge_count, best_points.len())
                        },
                    ) {
                        best = Some((source_index, source_edge_count, points));
                    }
                }
            }
        }

        best.map(|(_, _, points)| points)
    }

    fn terrain_clip_source_loop_positions_at_key(
        key: SurfaceXzKey,
        source_edges: &[TerrainClipSourceEdge],
    ) -> BTreeSet<usize> {
        let mut positions = BTreeSet::new();
        if source_edges.is_empty() {
            return positions;
        }
        for (position, source_edge) in source_edges.iter().copied().enumerate() {
            let start_key = Self::terrain_clip_world_key(source_edge.start);
            if start_key == key {
                positions.insert(position);
            }
            let end_key = Self::terrain_clip_world_key(source_edge.end);
            if end_key == key {
                positions.insert((position + 1) % source_edges.len());
            }
        }
        positions
    }

    fn terrain_clip_ordered_source_loop_key_path(
        source_edges: &[TerrainClipSourceEdge],
        start_position: usize,
        end_position: usize,
    ) -> Option<Vec<SurfaceXzKey>> {
        if source_edges.is_empty() || start_position >= source_edges.len() {
            return None;
        }
        let mut path = vec![Self::terrain_clip_source_loop_vertex_key(
            source_edges,
            start_position,
        )?];
        let mut cursor = start_position;
        for _ in 0..source_edges.len() {
            if cursor == end_position {
                return (path.len() >= 2).then_some(path);
            }
            cursor = (cursor + 1) % source_edges.len();
            path.push(Self::terrain_clip_source_loop_vertex_key(
                source_edges,
                cursor,
            )?);
        }
        None
    }

    fn terrain_clip_source_loop_vertex_key(
        source_edges: &[TerrainClipSourceEdge],
        position: usize,
    ) -> Option<SurfaceXzKey> {
        let source_edge = source_edges.get(position % source_edges.len())?;
        Some(Self::terrain_clip_world_key(source_edge.start))
    }

    fn terrain_clip_source_point_for_vertex_key(
        key: SurfaceXzKey,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<Vector3> {
        source_edges
            .iter()
            .flat_map(|edge| [edge.start, edge.end])
            .filter(|point| Self::terrain_clip_world_key(*point) == key)
            .max_by(|a, b| a.y.total_cmp(&b.y))
    }

    fn raise_terrain_clip_points_to_highest_source_heights(
        points: &mut [Vector3],
        source_edges: &[TerrainClipSourceEdge],
    ) {
        for point in points {
            let overlay_point = [f64::from(point.x), f64::from(point.z)];
            if let Some(height) = Self::terrain_clip_overlay_point_height_from_source_edges(
                overlay_point,
                source_edges,
            ) {
                point.y = point.y.max(height);
            }
        }
    }

    fn terrain_clip_points_from_interval_coverage<I>(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        intervals: I,
    ) -> TerrainClipSegmentPointRecovery
    where
        I: IntoIterator<Item = TerrainClipSourceInterval>,
    {
        let intervals = intervals.into_iter().collect::<Vec<_>>();
        if intervals.is_empty() {
            return TerrainClipSegmentPointRecovery::Missing;
        }

        let mut breakpoints = Self::terrain_clip_interval_breakpoints(&intervals);
        Self::append_terrain_clip_height_crossings(&intervals, &mut breakpoints);
        breakpoints.sort_by(|a, b| a.total_cmp(b));
        breakpoints.dedup_by(|a, b| *a == *b);

        let mut heights = vec![None; breakpoints.len()];
        let mut covered_any = false;
        for index in 0..breakpoints.len().saturating_sub(1) {
            let start_t = breakpoints[index];
            let end_t = breakpoints[index + 1];
            if end_t <= start_t {
                continue;
            }

            let covering = intervals
                .iter()
                .copied()
                .filter(|interval| Self::terrain_clip_interval_covers(*interval, start_t, end_t))
                .collect::<Vec<_>>();
            if covering.is_empty() {
                return TerrainClipSegmentPointRecovery::Partial;
            }
            covered_any = true;

            let Some(start_y) = Self::terrain_clip_highest_source_height_at_t(&covering, start_t)
            else {
                return TerrainClipSegmentPointRecovery::Partial;
            };
            let Some(end_y) = Self::terrain_clip_highest_source_height_at_t(&covering, end_t)
            else {
                return TerrainClipSegmentPointRecovery::Partial;
            };
            Self::merge_terrain_clip_height(&mut heights[index], start_y);
            Self::merge_terrain_clip_height(&mut heights[index + 1], end_y);
        }
        if !covered_any {
            return TerrainClipSegmentPointRecovery::Missing;
        }

        let mut points = Vec::new();
        for (t, height) in breakpoints.into_iter().zip(heights) {
            let Some(height) = height else {
                continue;
            };
            let point = interpolate_overlay_point(start, end, t);
            points.push(Vector3::new(point[0] as f32, height, point[1] as f32));
        }
        Self::dedup_terrain_clip_segment_points(&mut points);
        if points.len() >= 2 {
            TerrainClipSegmentPointRecovery::Covered(points)
        } else {
            TerrainClipSegmentPointRecovery::Degenerate
        }
    }

    fn terrain_clip_interval_breakpoints(intervals: &[TerrainClipSourceInterval]) -> Vec<f64> {
        let mut breakpoints = Vec::with_capacity(intervals.len() * 2 + 2);
        breakpoints.push(0.0);
        breakpoints.push(1.0);
        for interval in intervals {
            breakpoints.push(interval.start_t.clamp(0.0, 1.0));
            breakpoints.push(interval.end_t.clamp(0.0, 1.0));
        }
        breakpoints
    }

    fn append_terrain_clip_height_crossings(
        intervals: &[TerrainClipSourceInterval],
        breakpoints: &mut Vec<f64>,
    ) {
        for first_index in 0..intervals.len() {
            for second_index in first_index + 1..intervals.len() {
                let first = intervals[first_index];
                let second = intervals[second_index];
                let start_t = first.start_t.max(second.start_t).max(0.0);
                let end_t = first.end_t.min(second.end_t).min(1.0);
                if end_t <= start_t {
                    continue;
                }
                let start_delta =
                    interval_height_at(first, start_t) - interval_height_at(second, start_t);
                let end_delta =
                    interval_height_at(first, end_t) - interval_height_at(second, end_t);
                if Self::overlay_heights_equal(start_delta, 0.0) {
                    breakpoints.push(start_t);
                }
                if Self::overlay_heights_equal(end_delta, 0.0) {
                    breakpoints.push(end_t);
                }
                if start_delta.signum() == end_delta.signum() {
                    continue;
                }
                let denominator = f64::from(start_delta - end_delta);
                if denominator == 0.0 {
                    continue;
                }
                let crossing_t = start_t + (end_t - start_t) * f64::from(start_delta) / denominator;
                if crossing_t > start_t && crossing_t < end_t {
                    breakpoints.push(crossing_t);
                }
            }
        }
    }

    fn terrain_clip_interval_covers(
        interval: TerrainClipSourceInterval,
        start_t: f64,
        end_t: f64,
    ) -> bool {
        interval.start_t <= start_t && interval.end_t >= end_t
    }

    fn terrain_clip_highest_source_height_at_t(
        intervals: &[TerrainClipSourceInterval],
        t: f64,
    ) -> Option<f32> {
        intervals
            .iter()
            .copied()
            .map(|interval| interval_height_at(interval, t))
            .max_by(|a, b| a.total_cmp(b))
    }

    fn merge_terrain_clip_height(height: &mut Option<f32>, candidate: f32) {
        *height = Some(height.map_or(candidate, |current| current.max(candidate)));
    }

    fn dedup_terrain_clip_segment_points(points: &mut Vec<Vector3>) {
        let mut deduped = Vec::with_capacity(points.len());
        for &point in points.iter() {
            if let Some(last) = deduped.last_mut() {
                if Self::world_points_same_for_boundary(*last, point) {
                    if point.y > last.y {
                        last.y = point.y;
                    }
                    continue;
                }
            }
            deduped.push(point);
        }
        *points = deduped;
    }

    fn terrain_clip_dust_connector_heights_from_source_edges(
        contour: &NodeOverlayContour,
        segment_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSegmentHeights> {
        let len = contour.len();
        if len < 3 {
            return None;
        }

        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        if !Self::terrain_clip_connector_is_numeric_dust(contour, segment_index) {
            return None;
        }

        let previous = contour[(segment_index + len - 1) % len];
        let next = contour[(segment_index + 2) % len];
        if let (Some(previous_heights), Some(next_heights)) = (
            Self::terrain_clip_segment_heights_from_source_edges(previous, start, source_edges),
            Self::terrain_clip_segment_heights_from_source_edges(end, next, source_edges),
        ) {
            return Some(TerrainClipSegmentHeights {
                start_y: previous_heights.end_y,
                end_y: next_heights.start_y,
            });
        }

        let heights =
            Self::terrain_clip_contour_vertex_heights_from_source_edges(contour, source_edges)?;
        Some(TerrainClipSegmentHeights {
            start_y: heights[segment_index],
            end_y: heights[(segment_index + 1) % len],
        })
    }

    fn terrain_clip_contour_vertex_heights_from_source_edges(
        contour: &NodeOverlayContour,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<Vec<f32>> {
        let len = contour.len();
        if len < 3 {
            return None;
        }

        let mut heights = contour
            .iter()
            .copied()
            .map(|point| {
                Self::terrain_clip_overlay_point_height_from_source_edges(point, source_edges)
            })
            .collect::<Vec<_>>();
        let anchor = heights.iter().position(Option::is_some)?;
        let mut offset = 1usize;
        while offset < len {
            let index = (anchor + offset) % len;
            if heights[index].is_some() {
                offset += 1;
                continue;
            }

            let run_start_offset = offset;
            while offset < len && heights[(anchor + offset) % len].is_none() {
                offset += 1;
            }
            let prev_index = (anchor + run_start_offset - 1) % len;
            let next_index = (anchor + offset) % len;
            Self::interpolate_terrain_clip_dust_run_heights(
                contour,
                &mut heights,
                prev_index,
                next_index,
            )?;
        }

        heights.into_iter().collect()
    }

    fn terrain_clip_overlay_point_height_from_source_edges(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<f32> {
        let mut height = None;
        for &source_edge in source_edges {
            let source_start = [
                f64::from(source_edge.start.x),
                f64::from(source_edge.start.z),
            ];
            let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
            let Some(t) = Self::overlay_segment_parameter(point, source_start, source_end) else {
                continue;
            };
            Self::merge_terrain_clip_height(
                &mut height,
                interpolate_height_f64(source_edge.start.y, source_edge.end.y, t),
            );
        }
        height
    }

    fn interpolate_terrain_clip_dust_run_heights(
        contour: &NodeOverlayContour,
        heights: &mut [Option<f32>],
        prev_index: usize,
        next_index: usize,
    ) -> Option<()> {
        let start_y = heights[prev_index]?;
        let end_y = heights[next_index]?;
        let mut total_length_m = 0.0f64;
        let mut edge_index = prev_index;
        while edge_index != next_index {
            if !Self::terrain_clip_connector_is_numeric_dust(contour, edge_index) {
                return None;
            }
            let next = (edge_index + 1) % contour.len();
            total_length_m += overlay_segment_length_m(contour[edge_index], contour[next]);
            edge_index = next;
        }

        let mut distance_m = 0.0f64;
        edge_index = prev_index;
        while edge_index != next_index {
            let next = (edge_index + 1) % contour.len();
            distance_m += overlay_segment_length_m(contour[edge_index], contour[next]);
            if heights[next].is_none() {
                let t = if total_length_m > 0.0 {
                    distance_m / total_length_m
                } else {
                    0.0
                };
                heights[next] = Some(interpolate_height_f64(start_y, end_y, t));
            }
            edge_index = next;
        }

        Some(())
    }

    fn terrain_clip_dust_connector_points_from_source_edges(
        contour: &NodeOverlayContour,
        segment_index: usize,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<Vec<Vector3>> {
        let len = contour.len();
        let heights = Self::terrain_clip_dust_connector_heights_from_source_edges(
            contour,
            segment_index,
            source_edges,
        )?;
        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        Some(vec![
            Vector3::new(start[0] as f32, heights.start_y, start[1] as f32),
            Vector3::new(end[0] as f32, heights.end_y, end[1] as f32),
        ])
    }

    fn append_terrain_clip_sourced_segment_points(
        out: &mut Vec<RoadSurfaceTerrainClipSourceEdge>,
        mut points: Vec<Vector3>,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<(), (Vector3, Vector3)> {
        Self::dedup_terrain_clip_segment_points(&mut points);
        for segment in points.windows(2) {
            let start = segment[0];
            let end = segment[1];
            if Self::world_points_same_for_boundary(start, end) {
                continue;
            }
            let start_overlay = [f64::from(start.x), f64::from(start.z)];
            let end_overlay = [f64::from(end.x), f64::from(end.z)];
            let Some(source) = Self::terrain_clip_output_source_for_segment(
                start_overlay,
                end_overlay,
                source_edges,
            )
            .or_else(|| {
                Self::terrain_clip_output_source_for_endpoint_segment(
                    start_overlay,
                    end_overlay,
                    source_edges,
                )
            })
            .or_else(|| {
                Self::terrain_clip_output_dust_connector_source(
                    start_overlay,
                    end_overlay,
                    source_edges,
                )
            }) else {
                return Err((start, end));
            };
            Self::append_terrain_clip_source_edge(
                out,
                RoadSurfaceTerrainClipSourceEdge {
                    start,
                    end,
                    kind: source.kind,
                    source: source.source,
                },
            );
        }
        Ok(())
    }

    fn append_terrain_clip_source_edge(
        out: &mut Vec<RoadSurfaceTerrainClipSourceEdge>,
        mut edge: RoadSurfaceTerrainClipSourceEdge,
    ) {
        if Self::world_points_same_for_boundary(edge.start, edge.end) {
            return;
        }
        if let Some(last) = out.last_mut() {
            if Self::world_points_same_for_boundary(last.end, edge.start) {
                let shared = if edge.start.y > last.end.y {
                    edge.start
                } else {
                    last.end
                };
                last.end = shared;
                edge.start = shared;
            }
        }
        out.push(edge);
    }

    fn close_terrain_clip_source_edges(edges: &mut [RoadSurfaceTerrainClipSourceEdge]) {
        if edges.len() < 2 {
            return;
        }
        let first_start = edges[0].start;
        let last_index = edges.len() - 1;
        let last_end = edges[last_index].end;
        if Self::world_points_same_for_boundary(first_start, last_end) {
            let shared = if last_end.y > first_start.y {
                last_end
            } else {
                first_start
            };
            edges[0].start = shared;
            edges[last_index].end = shared;
        }
    }

    fn terrain_clip_output_source_for_segment(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSourceEdge> {
        let mut best = None;
        for &source_edge in source_edges {
            let interval = Self::terrain_clip_source_interval_on_segment(start, end, source_edge)?;
            if !Self::terrain_clip_interval_covers(interval, 0.0, 1.0) {
                continue;
            }
            let height = interval_height_at(interval, 0.5);
            if best.is_none_or(|(best_height, best_edge): (f32, TerrainClipSourceEdge)| {
                height > best_height
                    || (Self::overlay_heights_equal(height, best_height)
                        && Self::terrain_clip_source_edge_ordering(source_edge, best_edge).is_lt())
            }) {
                best = Some((height, source_edge));
            }
        }
        best.map(|(_, source_edge)| source_edge)
    }

    fn terrain_clip_output_source_for_endpoint_segment(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSourceEdge> {
        let start_key = Self::terrain_clip_overlay_key(start);
        let end_key = Self::terrain_clip_overlay_key(end);
        let mut candidates = source_edges
            .iter()
            .copied()
            .filter(|source_edge| {
                let source_start_key = Self::terrain_clip_world_key(source_edge.start);
                let source_end_key = Self::terrain_clip_world_key(source_edge.end);
                (source_start_key == start_key && source_end_key == end_key)
                    || (source_start_key == end_key && source_end_key == start_key)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| Self::terrain_clip_source_edge_ordering(*a, *b));
        candidates.into_iter().next()
    }

    fn terrain_clip_output_dust_connector_source(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Option<TerrainClipSourceEdge> {
        let mut endpoint_edges =
            Self::terrain_clip_source_edges_at_overlay_point(start, source_edges);
        endpoint_edges.extend(Self::terrain_clip_source_edges_at_overlay_point(
            end,
            source_edges,
        ));
        endpoint_edges.sort_by(|a, b| Self::terrain_clip_source_edge_ordering(*a, *b));
        endpoint_edges.dedup_by(|a, b| Self::terrain_clip_source_edge_ordering(*a, *b).is_eq());
        endpoint_edges.into_iter().next()
    }

    fn terrain_clip_source_edges_at_overlay_point(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipSourceEdge> {
        source_edges
            .iter()
            .copied()
            .filter(|source_edge| {
                let source_start = [
                    f64::from(source_edge.start.x),
                    f64::from(source_edge.start.z),
                ];
                let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
                Self::overlay_segment_parameter(point, source_start, source_end).is_some()
            })
            .collect()
    }

    fn terrain_clip_source_edge_ordering(
        a: TerrainClipSourceEdge,
        b: TerrainClipSourceEdge,
    ) -> std::cmp::Ordering {
        terrain_clip_edge_kind_priority(a.kind)
            .cmp(&terrain_clip_edge_kind_priority(b.kind))
            .then(a.source_index.cmp(&b.source_index))
            .then(a.edge_index.cmp(&b.edge_index))
    }

    fn world_points_same_for_boundary(a: Vector3, b: Vector3) -> bool {
        Self::terrain_clip_world_key(a) == Self::terrain_clip_world_key(b)
    }

    fn terrain_clip_connector_is_numeric_dust(
        contour: &NodeOverlayContour,
        segment_index: usize,
    ) -> bool {
        let len = contour.len();
        if len < 4 {
            return false;
        }

        let start = contour[segment_index];
        let end = contour[(segment_index + 1) % len];
        let connector_length_squared_m2 =
            (start[0] - end[0]) * (start[0] - end[0]) + (start[1] - end[1]) * (start[1] - end[1]);
        if connector_length_squared_m2
            <= f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
                * f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M)
        {
            return true;
        }
        let budget_m2 = f64::from(Self::overlay_numeric_area_budget_m2(
            Self::overlay_contour_perimeter_m(contour),
            contour.len(),
        ));
        if connector_length_squared_m2 > budget_m2 {
            return false;
        }

        let area_m2 = overlay_contour_area_local(contour).abs();
        let remove_start_delta = contour_area_delta_after_removing_vertex(contour, segment_index)
            .map(|area| (area - area_m2).abs());
        let remove_end_delta =
            contour_area_delta_after_removing_vertex(contour, (segment_index + 1) % len)
                .map(|area| (area - area_m2).abs());
        remove_start_delta
            .into_iter()
            .chain(remove_end_delta)
            .any(|delta| delta <= budget_m2)
    }

    fn terrain_clip_endpoint_samples(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipEndpointSample> {
        let point_key = Self::terrain_clip_overlay_key(point);
        let mut samples = Vec::new();
        for &source_edge in source_edges {
            if Self::terrain_clip_world_key(source_edge.start) == point_key {
                samples.push(TerrainClipEndpointSample {
                    kind: source_edge.kind,
                    source_index: source_edge.source_index,
                    edge_index: source_edge.edge_index,
                    y: source_edge.start.y,
                });
            }
            if Self::terrain_clip_world_key(source_edge.end) == point_key {
                samples.push(TerrainClipEndpointSample {
                    kind: source_edge.kind,
                    source_index: source_edge.source_index,
                    edge_index: source_edge.edge_index,
                    y: source_edge.end.y,
                });
            }
        }
        samples
    }

    fn terrain_clip_missing_source_context_label(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> String {
        format!(
            "start_sources={} end_sources={}",
            Self::terrain_clip_endpoint_context_label(start, source_edges),
            Self::terrain_clip_endpoint_context_label(end, source_edges)
        )
    }

    fn terrain_clip_endpoint_context_label(
        point: NodeOverlayPoint,
        source_edges: &[TerrainClipSourceEdge],
    ) -> String {
        let mut samples = Self::terrain_clip_endpoint_samples(point, source_edges)
            .into_iter()
            .map(|sample| {
                format!(
                    "{:?}:{}:{}@{:.3}",
                    sample.kind, sample.source_index, sample.edge_index, sample.y
                )
            })
            .collect::<Vec<_>>();
        samples.sort();
        samples.dedup();
        if samples.is_empty() {
            "none".to_string()
        } else {
            samples.into_iter().take(6).collect::<Vec<_>>().join("|")
        }
    }

    fn terrain_clip_source_interval_on_segment(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
        source_edge: TerrainClipSourceEdge,
    ) -> Option<TerrainClipSourceInterval> {
        let source_start = [
            f64::from(source_edge.start.x),
            f64::from(source_edge.start.z),
        ];
        let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
        let source_start_t = Self::overlay_line_parameter(source_start, start, end)?;
        let source_end_t = Self::overlay_line_parameter(source_end, start, end)?;
        let mut overlap_start_t = source_start_t.min(source_end_t).max(0.0);
        let mut overlap_end_t = source_start_t.max(source_end_t).min(1.0);
        let endpoint_dust_t = Self::overlay_endpoint_dust_parameter(start, end)?;
        if overlap_start_t <= endpoint_dust_t {
            overlap_start_t = 0.0;
        }
        if overlap_end_t >= 1.0 - endpoint_dust_t {
            overlap_end_t = 1.0;
        }
        if overlap_end_t <= overlap_start_t {
            return None;
        }

        let overlap_start = interpolate_overlay_point(start, end, overlap_start_t);
        let overlap_end = interpolate_overlay_point(start, end, overlap_end_t);
        let edge_overlap_start_t =
            Self::overlay_segment_parameter(overlap_start, source_start, source_end)?;
        let edge_overlap_end_t =
            Self::overlay_segment_parameter(overlap_end, source_start, source_end)?;
        Some(TerrainClipSourceInterval {
            start_t: overlap_start_t,
            end_t: overlap_end_t,
            start_y: interpolate_height_f64(
                source_edge.start.y,
                source_edge.end.y,
                edge_overlap_start_t,
            ),
            end_y: interpolate_height_f64(
                source_edge.start.y,
                source_edge.end.y,
                edge_overlap_end_t,
            ),
        })
    }

    fn overlay_line_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        Self::overlay_segment_parameter_unbounded(point, start, end)
            .map(OverlaySegmentParameter::as_f64)
            .or_else(|| Self::overlay_numeric_dust_line_parameter(point, start, end))
    }

    fn overlay_segment_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        if let Some(parameter) = Self::overlay_segment_parameter_unbounded(point, start, end) {
            return Self::overlay_segment_parameter_with_endpoint_dust(
                parameter.as_f64(),
                start,
                end,
            );
        }
        let parameter = Self::overlay_numeric_dust_line_parameter(point, start, end)?;
        Self::overlay_segment_parameter_with_endpoint_dust(parameter, start, end)
    }

    fn overlay_segment_parameter_unbounded(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<OverlaySegmentParameter> {
        let start_key = Self::terrain_clip_overlay_key(start);
        let end_key = Self::terrain_clip_overlay_key(end);
        let point_key = Self::terrain_clip_overlay_key(point);
        point_key.exact_line_parameter(start_key, end_key)
    }

    fn overlay_numeric_dust_line_parameter(
        point: NodeOverlayPoint,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        // `i_overlay` may emit split vertices microscopically off the source edge's integer line.
        // The source edge still owns the height; this only recovers its interval parameter.
        let dx = end[0] - start[0];
        let dz = end[1] - start[1];
        let length_squared = dx * dx + dz * dz;
        if length_squared == 0.0 {
            return None;
        }
        let point_dx = point[0] - start[0];
        let point_dz = point[1] - start[1];
        let length = length_squared.sqrt();
        let cross = point_dx * dz - point_dz * dx;
        if cross.abs() > f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) * length {
            return None;
        }
        Some((point_dx * dx + point_dz * dz) / length_squared)
    }

    fn overlay_segment_parameter_with_endpoint_dust(
        parameter: f64,
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        let endpoint_dust = Self::overlay_endpoint_dust_parameter(start, end)?;
        (parameter >= -endpoint_dust && parameter <= 1.0 + endpoint_dust)
            .then_some(parameter.clamp(0.0, 1.0))
    }

    fn overlay_endpoint_dust_parameter(
        start: NodeOverlayPoint,
        end: NodeOverlayPoint,
    ) -> Option<f64> {
        let dx = end[0] - start[0];
        let dz = end[1] - start[1];
        let length = (dx * dx + dz * dz).sqrt();
        if length == 0.0 {
            return None;
        }
        Some(f64::from(NODE_OVERLAY_NUMERIC_DUST_WIDTH_M) / length)
    }

    fn overlay_height_key(height_m: f32) -> i64 {
        (f64::from(height_m) * NODE_OVERLAY_SCALE).round() as i64
    }

    fn overlay_heights_equal(a: f32, b: f32) -> bool {
        Self::overlay_height_key(a) == Self::overlay_height_key(b)
    }

    fn terrain_clip_overlay_key(point: NodeOverlayPoint) -> SurfaceXzKey {
        SurfaceXzKey::from_overlay_point(point)
    }

    fn terrain_clip_world_key(point: Vector3) -> SurfaceXzKey {
        SurfaceXzKey::from_godot_world_xz(point)
    }
}

fn interpolate_height_f64(start_y: f32, end_y: f32, t: f64) -> f32 {
    (f64::from(start_y) + f64::from(end_y - start_y) * t) as f32
}

fn overlay_segment_length_m(start: NodeOverlayPoint, end: NodeOverlayPoint) -> f64 {
    let dx = end[0] - start[0];
    let dz = end[1] - start[1];
    (dx * dx + dz * dz).sqrt()
}

fn overlay_points_same_for_boundary(a: NodeOverlayPoint, b: NodeOverlayPoint) -> bool {
    SurfaceXzKey::from_overlay_point(a) == SurfaceXzKey::from_overlay_point(b)
}

fn remove_repeated_overlay_point_spurs(points: &mut NodeOverlayContour) {
    while points.len() >= 3 {
        let Some((first, second)) = first_repeated_overlay_point_pair(points) else {
            break;
        };
        let cycle = points[first..second].to_vec();
        let mut remainder = Vec::with_capacity(points.len() - (second - first));
        remainder.extend_from_slice(&points[..=first]);
        remainder.extend_from_slice(&points[second + 1..]);

        let cycle_area = overlay_contour_area_local(&cycle).abs();
        let remainder_area = overlay_contour_area_local(&remainder).abs();
        if remainder.len() >= 3 && remainder_area >= cycle_area {
            *points = remainder;
        } else if cycle.len() >= 3 {
            *points = cycle;
        } else {
            break;
        }
    }
}

fn first_repeated_overlay_point_pair(points: &NodeOverlayContour) -> Option<(usize, usize)> {
    for first in 0..points.len() {
        for second in first + 2..points.len() {
            if first == 0 && second + 1 == points.len() {
                continue;
            }
            if overlay_points_same_for_boundary(points[first], points[second]) {
                return Some((first, second));
            }
        }
    }
    None
}

fn overlay_contour_area_local(contour: &NodeOverlayContour) -> f64 {
    if contour.len() < 3 {
        return 0.0;
    }
    let mut signed_area = 0.0;
    for index in 0..contour.len() {
        let current = contour[index];
        let next = contour[(index + 1) % contour.len()];
        signed_area += current[0] * next[1] - next[0] * current[1];
    }
    signed_area * 0.5
}

fn contour_area_delta_after_removing_vertex(
    contour: &NodeOverlayContour,
    index: usize,
) -> Option<f64> {
    if contour.len() <= 3 || index >= contour.len() {
        return None;
    }
    let mut reduced = Vec::with_capacity(contour.len() - 1);
    reduced.extend_from_slice(&contour[..index]);
    reduced.extend_from_slice(&contour[index + 1..]);
    Some(overlay_contour_area_local(&reduced).abs())
}

fn interpolate_overlay_point(
    start: NodeOverlayPoint,
    end: NodeOverlayPoint,
    t: f64,
) -> NodeOverlayPoint {
    [
        start[0] + (end[0] - start[0]) * t,
        start[1] + (end[1] - start[1]) * t,
    ]
}

fn interval_height_at(interval: TerrainClipSourceInterval, t: f64) -> f32 {
    let span = interval.end_t - interval.start_t;
    if span == 0.0 {
        return interval.start_y;
    }
    interpolate_height_f64(
        interval.start_y,
        interval.end_y,
        (t - interval.start_t) / span,
    )
}

fn terrain_clip_edge_kind_priority(kind: RoadSurfaceTerrainClipEdgeKind) -> u8 {
    match kind {
        RoadSurfaceTerrainClipEdgeKind::SidewalkOuter => 0,
        RoadSurfaceTerrainClipEdgeKind::ShoulderOuter => 1,
        RoadSurfaceTerrainClipEdgeKind::FootprintBoundary => 2,
        RoadSurfaceTerrainClipEdgeKind::SpanHandoff => 3,
    }
}
