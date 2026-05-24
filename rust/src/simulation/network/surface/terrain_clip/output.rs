//! Terrain-clip output edge sourcing.

use super::super::{
    NodeFootprintBoundarySegmentSource, NodeFootprintBoundaryVertexSource, RoadSurfaceSystem,
    earthwork::RoadSurfaceEarthworkFaceSource, keys::SurfaceHeightMmKey,
};
use super::heights::interval_height_at;
use super::model::*;
use godot::prelude::Vector3;

enum TerrainClipOutputSourceSelection {
    Missing,
    Ambiguous(String),
    Source(TerrainClipSourceEdge),
}

impl RoadSurfaceSystem {
    pub(super) fn append_terrain_clip_sourced_segment_points(
        out: &mut Vec<RoadSurfaceTerrainClipSourceEdge>,
        mut points: Vec<Vector3>,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<(), TerrainClipOutputSourceError> {
        Self::dedup_terrain_clip_top_envelope_points(&mut points);
        for segment in points.windows(2) {
            let start = segment[0];
            let end = segment[1];
            if Self::world_points_same_for_boundary(start, end) {
                continue;
            }
            let source = Self::terrain_clip_output_source_for_points(start, end, source_edges)?;
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

    fn terrain_clip_output_source_for_points(
        start: Vector3,
        end: Vector3,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Result<TerrainClipSourceEdge, TerrainClipOutputSourceError> {
        if let Some(source) = Self::terrain_clip_output_source_result(
            Self::terrain_clip_output_source_for_segment(start, end, source_edges),
            start,
            end,
        )? {
            return Ok(source);
        }
        if let Some(source) = Self::terrain_clip_output_source_result(
            Self::terrain_clip_output_source_for_endpoint_segment(start, end, source_edges),
            start,
            end,
        )? {
            return Ok(source);
        }
        if let Some(source) = Self::terrain_clip_output_source_result(
            Self::terrain_clip_output_dust_connector_source(start, end, source_edges),
            start,
            end,
        )? {
            return Ok(source);
        }
        Err(TerrainClipOutputSourceError::Missing { start, end })
    }

    fn terrain_clip_output_source_result(
        selection: TerrainClipOutputSourceSelection,
        start: Vector3,
        end: Vector3,
    ) -> Result<Option<TerrainClipSourceEdge>, TerrainClipOutputSourceError> {
        match selection {
            TerrainClipOutputSourceSelection::Missing => Ok(None),
            TerrainClipOutputSourceSelection::Source(source) => Ok(Some(source)),
            TerrainClipOutputSourceSelection::Ambiguous(context) => {
                Err(TerrainClipOutputSourceError::Ambiguous {
                    start,
                    end,
                    context,
                })
            }
        }
    }

    pub(super) fn close_terrain_clip_source_edges(edges: &mut [RoadSurfaceTerrainClipSourceEdge]) {
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

    fn append_terrain_clip_source_edge(
        out: &mut Vec<RoadSurfaceTerrainClipSourceEdge>,
        mut edge: RoadSurfaceTerrainClipSourceEdge,
    ) {
        if Self::world_points_same_for_boundary(edge.start, edge.end) {
            return;
        }
        if let Some(last) = out.last_mut()
            && Self::world_points_same_for_boundary(last.end, edge.start)
        {
            let shared = if edge.start.y > last.end.y {
                edge.start
            } else {
                last.end
            };
            last.end = shared;
            edge.start = shared;
        }
        out.push(edge);
    }

    fn terrain_clip_output_source_for_segment(
        start: Vector3,
        end: Vector3,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipOutputSourceSelection {
        let start_overlay = [f64::from(start.x), f64::from(start.z)];
        let end_overlay = [f64::from(end.x), f64::from(end.z)];
        let mut candidates = Vec::new();
        for &source_edge in source_edges {
            let Some(interval) = Self::terrain_clip_source_interval_on_segment(
                start_overlay,
                end_overlay,
                source_edge,
            ) else {
                continue;
            };
            if !Self::terrain_clip_interval_covers(interval, 0.0, 1.0) {
                continue;
            }
            if Self::overlay_heights_equal(interval_height_at(interval, 0.0), start.y)
                && Self::overlay_heights_equal(interval_height_at(interval, 1.0), end.y)
            {
                candidates.push(source_edge);
            }
        }
        Self::unique_terrain_clip_output_source(candidates, "covered_segment", Some((start, end)))
    }

    fn terrain_clip_output_source_for_endpoint_segment(
        start: Vector3,
        end: Vector3,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipOutputSourceSelection {
        let start_key = Self::terrain_clip_world_key(start);
        let end_key = Self::terrain_clip_world_key(end);
        let candidates = source_edges
            .iter()
            .copied()
            .filter(|source_edge| {
                let source_start_key = Self::terrain_clip_world_key(source_edge.start);
                let source_end_key = Self::terrain_clip_world_key(source_edge.end);
                if source_start_key == start_key && source_end_key == end_key {
                    Self::overlay_heights_equal(source_edge.start.y, start.y)
                        && Self::overlay_heights_equal(source_edge.end.y, end.y)
                } else if source_start_key == end_key && source_end_key == start_key {
                    Self::overlay_heights_equal(source_edge.start.y, end.y)
                        && Self::overlay_heights_equal(source_edge.end.y, start.y)
                } else {
                    false
                }
            })
            .collect::<Vec<_>>();
        Self::unique_terrain_clip_output_source(candidates, "endpoint_segment", Some((start, end)))
    }

    fn terrain_clip_output_dust_connector_source(
        start: Vector3,
        end: Vector3,
        source_edges: &[TerrainClipSourceEdge],
    ) -> TerrainClipOutputSourceSelection {
        let mut candidates = Self::terrain_clip_source_edges_at_world_xz_point(start, source_edges);
        candidates.extend(Self::terrain_clip_source_edges_at_world_xz_point(
            end,
            source_edges,
        ));
        if candidates.is_empty() {
            return TerrainClipOutputSourceSelection::Missing;
        }
        let first = candidates[0];
        if !candidates
            .iter()
            .copied()
            .all(|candidate| terrain_clip_source_edges_same_provenance(candidate, first))
        {
            return TerrainClipOutputSourceSelection::Ambiguous(
                "dust_connector_endpoint_sources_disagree".to_string(),
            );
        }
        Self::unique_terrain_clip_output_source(candidates, "dust_connector", None)
    }

    fn terrain_clip_source_edges_at_world_xz_point(
        point: Vector3,
        source_edges: &[TerrainClipSourceEdge],
    ) -> Vec<TerrainClipSourceEdge> {
        let overlay_point = [f64::from(point.x), f64::from(point.z)];
        source_edges
            .iter()
            .copied()
            .filter(|&source_edge| {
                let source_start = [
                    f64::from(source_edge.start.x),
                    f64::from(source_edge.start.z),
                ];
                let source_end = [f64::from(source_edge.end.x), f64::from(source_edge.end.z)];
                let Some(_t) =
                    Self::overlay_segment_parameter(overlay_point, source_start, source_end)
                else {
                    return false;
                };
                true
            })
            .collect()
    }

    fn unique_terrain_clip_output_source(
        mut candidates: Vec<TerrainClipSourceEdge>,
        context: &'static str,
        canonical_segment: Option<(Vector3, Vector3)>,
    ) -> TerrainClipOutputSourceSelection {
        if candidates.is_empty() {
            return TerrainClipOutputSourceSelection::Missing;
        }
        candidates.sort_by(|a, b| terrain_clip_source_edge_ordering(*a, *b));
        let visible_candidates = candidates
            .iter()
            .copied()
            .filter(|candidate| candidate.kind != RoadSurfaceTerrainClipEdgeKind::SpanHandoff)
            .collect::<Vec<_>>();
        let provenance_candidates = if visible_candidates.is_empty() {
            candidates.as_slice()
        } else {
            visible_candidates.as_slice()
        };
        let first = provenance_candidates[0];
        if !provenance_candidates
            .iter()
            .copied()
            .all(|candidate| terrain_clip_source_edges_same_provenance(candidate, first))
        {
            if let Some((start, end)) = canonical_segment
                && let Some(source) = Self::canonical_same_owner_node_boundary_output_source(
                    provenance_candidates,
                    start,
                    end,
                )
            {
                return TerrainClipOutputSourceSelection::Source(source);
            }
            let sources = provenance_candidates
                .iter()
                .take(6)
                .map(|candidate| {
                    format!(
                        "{:?}:{}:{}:{:?}",
                        candidate.kind,
                        candidate.source_index,
                        candidate.edge_index,
                        candidate.source
                    )
                })
                .collect::<Vec<_>>()
                .join("|");
            return TerrainClipOutputSourceSelection::Ambiguous(format!(
                "{context}_sources_disagree candidates={sources}"
            ));
        }
        TerrainClipOutputSourceSelection::Source(first)
    }

    fn canonical_same_owner_node_boundary_output_source(
        candidates: &[TerrainClipSourceEdge],
        start: Vector3,
        end: Vector3,
    ) -> Option<TerrainClipSourceEdge> {
        let first = *candidates.first()?;
        let RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
            node_id,
            kind,
            owner_kind,
            owner_index,
            ..
        } = first.source
        else {
            return None;
        };
        if !candidates.iter().copied().all(|candidate| {
            candidate.kind == first.kind
                && matches!(
                    candidate.source,
                    RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                        node_id: candidate_node_id,
                        kind: candidate_kind,
                        owner_kind: candidate_owner_kind,
                        owner_index: candidate_owner_index,
                        ..
                    } if candidate_node_id == node_id
                        && candidate_kind == kind
                        && candidate_owner_kind == owner_kind
                        && candidate_owner_index == owner_index
                )
        }) {
            return None;
        }

        Some(TerrainClipSourceEdge {
            start,
            end,
            kind: first.kind,
            source: RoadSurfaceEarthworkFaceSource::NodeFootprintBoundary {
                node_id,
                kind,
                owner_kind,
                owner_index,
                boundary_source: Some(NodeFootprintBoundarySegmentSource {
                    start: Self::canonical_output_boundary_vertex_source(start),
                    end: Self::canonical_output_boundary_vertex_source(end),
                }),
            },
            source_index: first.source_index,
            edge_index: first.edge_index,
        })
    }

    fn canonical_output_boundary_vertex_source(
        point: Vector3,
    ) -> NodeFootprintBoundaryVertexSource {
        let key = Self::terrain_clip_world_key(point);
        NodeFootprintBoundaryVertexSource::CanonicalBoundaryPoint {
            x_key: key.x_key(),
            z_key: key.z_key(),
            y_mm: SurfaceHeightMmKey::from_m_f32(point.y).as_i64(),
        }
    }
}

pub(super) enum TerrainClipOutputSourceError {
    Missing {
        start: Vector3,
        end: Vector3,
    },
    Ambiguous {
        start: Vector3,
        end: Vector3,
        context: String,
    },
}
