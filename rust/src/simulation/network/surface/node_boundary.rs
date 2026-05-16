//! Node-owned boundary, vertical-face, and visual-piece DTOs.

use super::{
    RoadSurfaceBandKind, RoadSurfaceVisualNodePieceKind, RoadSurfaceVisualPolygon, arrangement,
    backend::{ROAD_OVERLAY_COORDINATE_SCALE, RoadVec2},
    earthwork::{RoadSurfaceEarthworkBoundarySegment, RoadSurfaceEarthworkRenderFace},
    node_grade::NodeGradeVertexAuthority,
    terrain_clip::RoadSurfaceTerrainClipLoop,
};
use godot::prelude::Vector3;

#[derive(Debug)]
pub(crate) enum NodeBoundaryExportError {
    EmptyOuterBoundary,
    MissingFootprintBoundaryHeight,
    DegenerateOuterBoundaryLoop,
    MissingEarthworkBoundarySource,
    MissingNodeTopSurfaceGradeAuthority,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RoadSurfaceVerticalFaceSource {
    CanonicalStep {
        explicit_vertical_step_index: usize,
        segment: arrangement::NodeExplicitVerticalStepSegment,
    },
    FinalOwnedBoundary {
        segment: arrangement::NodeExplicitVerticalStepSegment,
    },
}

impl RoadSurfaceVerticalFaceSource {
    pub(crate) fn explicit_vertical_step_index(self) -> Option<usize> {
        match self {
            Self::CanonicalStep {
                explicit_vertical_step_index,
                ..
            } => Some(explicit_vertical_step_index),
            Self::FinalOwnedBoundary { .. } => None,
        }
    }

    pub(crate) fn segment(self) -> arrangement::NodeExplicitVerticalStepSegment {
        match self {
            Self::CanonicalStep { segment, .. } | Self::FinalOwnedBoundary { segment } => segment,
        }
    }

    pub(crate) fn sort_key(
        self,
    ) -> (
        u8,
        arrangement::NodeExplicitVerticalStepSegment,
        Option<usize>,
    ) {
        match self {
            Self::CanonicalStep {
                explicit_vertical_step_index,
                segment,
            } => (0, segment, Some(explicit_vertical_step_index)),
            Self::FinalOwnedBoundary { segment } => (1, segment, None),
        }
    }
}

/// Explicit visual node piece compiled from the solved roadbed.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSurfaceVisualNodePiece {
    /// Owning node id.
    pub node_id: u32,
    /// Piece classification for rendering and debug.
    pub kind: RoadSurfaceVisualNodePieceKind,
    /// Outer piece-owned boundaries used for debug, surface chunk bounds, and terrain clipping.
    pub outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    /// Explicit asphalt-owned polygons for the node piece.
    pub road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit curb / shoulder-owned polygons for the node piece.
    pub curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    /// Explicit vertical faces at raised owner-pair material contacts.
    pub raised_step_face_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) raised_step_face_sources: Vec<RoadSurfaceVerticalFaceSource>,
    /// Explicit sidewalk-owned polygons for the node piece.
    pub sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
    pub(crate) node_grade_authorities: Vec<NodeGradeVertexAuthority>,
    pub(crate) node_top_surface_sources: Vec<NodeTopSurfacePolygonSource>,
    pub(crate) owned_regions: Vec<NodeOwnedRegion>,
    pub(crate) earthwork_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) render_earthwork_faces: Vec<RoadSurfaceEarthworkRenderFace>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeOwnedRegion {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) polygon: RoadSurfaceVisualPolygon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeTopSurfaceVertexSource {
    pub(crate) grade_authority_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeFootprintBoundaryDirectSource {
    pub(crate) top_surface_source_index: usize,
    pub(crate) grade_authority_index: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum NodeFootprintBoundaryVertexSource {
    Direct(NodeFootprintBoundaryDirectSource),
    BoundaryInterpolation {
        owning_segment_start: NodeFootprintBoundaryDirectSource,
        owning_segment_end: NodeFootprintBoundaryDirectSource,
        height_mm: i64,
    },
    SurfaceInterpolation {
        top_surface_source_index: usize,
        grade_authority_indices: [usize; 3],
        height_mm: i64,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) struct NodeFootprintBoundarySegmentSource {
    pub(crate) start: NodeFootprintBoundaryVertexSource,
    pub(crate) end: NodeFootprintBoundaryVertexSource,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeTopSurfacePolygonSource {
    pub(crate) kind: RoadSurfaceBandKind,
    pub(crate) owner_index: usize,
    pub(crate) height_field_id: arrangement::NodeBandHeightFieldId,
    pub(crate) vertex_sources: Vec<NodeTopSurfaceVertexSource>,
    pub(crate) triangle_sources: Vec<[NodeTopSurfaceVertexSource; 3]>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NodeSurfaceRegionResult {
    pub(crate) outer_boundary_loops: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) earthwork_boundary_segments: Vec<Vec<RoadSurfaceEarthworkBoundarySegment>>,
    pub(crate) terrain_clip_boundary_loops: Vec<RoadSurfaceTerrainClipLoop>,
    pub(crate) road_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) curb_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) raised_step_faces: Vec<(RoadSurfaceVisualPolygon, RoadSurfaceVerticalFaceSource)>,
    pub(crate) sidewalk_surface_polygons: Vec<RoadSurfaceVisualPolygon>,
    pub(crate) explicit_vertical_step_segments: Vec<arrangement::NodeExplicitVerticalStepSegment>,
    pub(crate) node_grade_authorities: Vec<NodeGradeVertexAuthority>,
    pub(crate) node_top_surface_sources: Vec<NodeTopSurfacePolygonSource>,
    pub(crate) owned_regions: Vec<NodeOwnedRegion>,
}

pub(super) fn interpolate_missing_footprint_boundary_heights(
    vertices: &mut [(arrangement::NodeArrangementKey, Option<i64>)],
) -> Result<(), NodeBoundaryExportError> {
    let Some(_first_missing_key) = vertices
        .iter()
        .find_map(|(key, height_mm)| height_mm.is_none().then_some(*key))
    else {
        return Ok(());
    };
    let Some(first_solved_index) = vertices
        .iter()
        .position(|(_, height_mm)| height_mm.is_some())
    else {
        return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
    };
    if vertices
        .iter()
        .filter(|(_, height_mm)| height_mm.is_some())
        .count()
        < 2
    {
        return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
    }

    let mut ordered_indices = Vec::with_capacity(vertices.len() + 1);
    ordered_indices.extend(first_solved_index..vertices.len());
    ordered_indices.extend(0..=first_solved_index);

    let mut start_pos = 0;
    while start_pos + 1 < ordered_indices.len() {
        let start_index = ordered_indices[start_pos];
        let Some(start_height_mm) = vertices[start_index].1 else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        };
        let Some(end_pos) = (start_pos + 1..ordered_indices.len())
            .find(|pos| vertices[ordered_indices[*pos]].1.is_some())
        else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        };
        if end_pos == start_pos + 1 {
            start_pos = end_pos;
            continue;
        }

        let end_index = ordered_indices[end_pos];
        let Some(end_height_mm) = vertices[end_index].1 else {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        };

        let mut cumulative_lengths = Vec::with_capacity(end_pos - start_pos + 1);
        cumulative_lengths.push(0.0);
        let mut total_length_m = 0.0;
        for pair_pos in start_pos..end_pos {
            total_length_m += arrangement_key_distance_m(
                vertices[ordered_indices[pair_pos]].0,
                vertices[ordered_indices[pair_pos + 1]].0,
            );
            cumulative_lengths.push(total_length_m);
        }
        if total_length_m <= f64::EPSILON {
            return Err(NodeBoundaryExportError::MissingFootprintBoundaryHeight);
        }

        for run_offset in 1..cumulative_lengths.len() - 1 {
            let index = ordered_indices[start_pos + run_offset];
            let t = cumulative_lengths[run_offset] / total_length_m;
            vertices[index].1 = Some(
                (start_height_mm as f64 + (end_height_mm - start_height_mm) as f64 * t).round()
                    as i64,
            );
        }
        start_pos = end_pos;
    }
    Ok(())
}

pub(super) fn remove_unsupported_numeric_boundary_vertices<F>(
    points: &mut Vec<Vector3>,
    mut should_keep_vertex: F,
) where
    F: FnMut(arrangement::NodeArrangementKey, [Vector3; 3]) -> bool,
{
    loop {
        if points.len() < 4 {
            return;
        }
        let mut removed = false;
        for index in 0..points.len() {
            let previous = if index == 0 {
                points.len() - 1
            } else {
                index - 1
            };
            let next = if index + 1 == points.len() {
                0
            } else {
                index + 1
            };
            let local_points = [points[previous], points[index], points[next]];
            let current_key = arrangement::NodeArrangementKey::from_point(RoadVec2::new(
                f64::from(points[index].x),
                f64::from(points[index].z),
            ));
            if should_keep_vertex(current_key, local_points) {
                continue;
            }
            points.remove(index);
            removed = true;
            break;
        }
        if !removed {
            return;
        }
    }
}

fn arrangement_key_distance_m(
    a: arrangement::NodeArrangementKey,
    b: arrangement::NodeArrangementKey,
) -> f64 {
    let dx = (a.x_key() - b.x_key()) as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    let dz = (a.z_key() - b.z_key()) as f64 / ROAD_OVERLAY_COORDINATE_SCALE;
    (dx * dx + dz * dz).sqrt()
}
