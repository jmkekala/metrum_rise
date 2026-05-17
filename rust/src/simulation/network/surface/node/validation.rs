//! Structured validation and diagnostics for canonical node surface compilation.

mod boundaries;
mod crossings;
mod mapping;
mod report;
mod solution;
#[cfg(test)]
mod tests;
mod triangles;

use super::arrangement::NodeArrangementKey;
use super::keys::{SurfaceHeightMmKey, SurfaceXzKey, SurfaceXzSegmentKey};
use super::triangulation::NodeTriangulatedRegion;
use parry2d::shape::Segment;

pub(crate) use report::NodeValidationReport;
#[cfg(test)]
pub(crate) use report::{NodeGeometryDiagnostic, NodeGeometryDiagnosticKind};

const VALIDATION_MIN_SEGMENT_LENGTH_M: f32 = 0.000001;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeValidationPointKey {
    x_key: i64,
    z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct NodeValidationEdgeKey {
    start: NodeValidationPointKey,
    end: NodeValidationPointKey,
}

#[derive(Clone, Copy)]
struct BoundarySegment {
    index: usize,
    edge: [usize; 2],
    key_edge: NodeValidationEdgeKey,
    segment: Segment,
}

fn edge_key_for_indices(
    region: &NodeTriangulatedRegion,
    edge: [usize; 2],
) -> NodeValidationEdgeKey {
    NodeValidationEdgeKey::new(
        point_key_from_world(region.vertices[edge[0]].point_world),
        point_key_from_world(region.vertices[edge[1]].point_world),
    )
}

fn point_key_from_world(point: super::backend::RoadVec3) -> NodeValidationPointKey {
    NodeValidationPointKey::from_surface_key(SurfaceXzKey::from_world_xz(point))
}

fn quantize_m(value: f64) -> i64 {
    SurfaceHeightMmKey::from_m_f64(value).as_i64()
}

fn validation_point_key_to_mm(value: i64) -> i64 {
    SurfaceXzKey::coordinate_key_to_mm(value)
}

impl NodeValidationPointKey {
    fn from_surface_key(key: SurfaceXzKey) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }

    fn from_arrangement_key(key: NodeArrangementKey) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }

    fn surface_key(self) -> SurfaceXzKey {
        SurfaceXzKey::from_raw_keys(self.x_key, self.z_key)
    }

    fn x_mm(self) -> i64 {
        validation_point_key_to_mm(self.x_key)
    }

    fn z_mm(self) -> i64 {
        validation_point_key_to_mm(self.z_key)
    }
}

impl NodeValidationEdgeKey {
    fn new(a: NodeValidationPointKey, b: NodeValidationPointKey) -> Self {
        let segment = SurfaceXzSegmentKey::new(a.surface_key(), b.surface_key());
        Self {
            start: NodeValidationPointKey::from_surface_key(segment.start()),
            end: NodeValidationPointKey::from_surface_key(segment.end()),
        }
    }

    fn endpoints(self) -> [NodeValidationPointKey; 2] {
        [self.start, self.end]
    }

    fn is_degenerate(self) -> bool {
        self.start == self.end
    }
}
