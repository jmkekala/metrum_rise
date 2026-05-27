//! Raised-step render key test helpers.

use super::*;

#[derive(Clone, Copy, Debug)]
pub(in crate::simulation::network::surface::tests) struct TestTopBoundaryEdge {
    pub(in crate::simulation::network::surface::tests) kind: RoadSurfaceBandKind,
    pub(in crate::simulation::network::surface::tests) owner_index: usize,
    pub(in crate::simulation::network::surface::tests) start: RoadVec3,
    pub(in crate::simulation::network::surface::tests) end: RoadVec3,
    pub(in crate::simulation::network::surface::tests) key: TestRenderEdgeKey,
    pub(in crate::simulation::network::surface::tests) xz_key: TestRenderXzEdgeKey,
    pub(in crate::simulation::network::surface::tests) avg_y_m: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::tests) struct TestRenderVertexKey {
    pub(in crate::simulation::network::surface::tests) x_key: i64,
    pub(in crate::simulation::network::surface::tests) y_mm: i64,
    pub(in crate::simulation::network::surface::tests) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::tests) struct TestRenderEdgeKey {
    pub(in crate::simulation::network::surface::tests) start: TestRenderVertexKey,
    pub(in crate::simulation::network::surface::tests) end: TestRenderVertexKey,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::tests) struct TestRenderXzVertexKey {
    pub(in crate::simulation::network::surface::tests) x_key: i64,
    pub(in crate::simulation::network::surface::tests) z_key: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(in crate::simulation::network::surface::tests) struct TestRenderXzEdgeKey {
    pub(in crate::simulation::network::surface::tests) start: TestRenderXzVertexKey,
    pub(in crate::simulation::network::surface::tests) end: TestRenderXzVertexKey,
}

impl TestRenderVertexKey {
    pub(in crate::simulation::network::surface::tests) fn from_point(point: RoadVec3) -> Self {
        let (x_key, z_key) = test_xz_key(point);
        Self {
            x_key,
            y_mm: (point.y * 1000.0).round() as i64,
            z_key,
        }
    }

    pub(in crate::simulation::network::surface::tests) fn xz(self) -> TestRenderXzVertexKey {
        TestRenderXzVertexKey {
            x_key: self.x_key,
            z_key: self.z_key,
        }
    }
}

impl TestRenderXzVertexKey {
    pub(in crate::simulation::network::surface::tests) fn from_arrangement_key(
        key: super::arrangement::NodeArrangementKey,
    ) -> Self {
        Self {
            x_key: key.x_key(),
            z_key: key.z_key(),
        }
    }
}

impl TestRenderEdgeKey {
    pub(in crate::simulation::network::surface::tests) fn normalized(
        start: RoadVec3,
        end: RoadVec3,
    ) -> Option<Self> {
        let start = TestRenderVertexKey::from_point(start);
        let end = TestRenderVertexKey::from_point(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    pub(in crate::simulation::network::surface::tests) fn xz(self) -> TestRenderXzEdgeKey {
        let start = self.start.xz();
        let end = self.end.xz();
        if start <= end {
            TestRenderXzEdgeKey { start, end }
        } else {
            TestRenderXzEdgeKey {
                start: end,
                end: start,
            }
        }
    }
}

impl TestRenderXzEdgeKey {
    pub(in crate::simulation::network::surface::tests) fn normalized_from_arrangement_keys(
        start: super::arrangement::NodeArrangementKey,
        end: super::arrangement::NodeArrangementKey,
    ) -> Option<Self> {
        let start = TestRenderXzVertexKey::from_arrangement_key(start);
        let end = TestRenderXzVertexKey::from_arrangement_key(end);
        if start == end {
            return None;
        }
        Some(if start <= end {
            Self { start, end }
        } else {
            Self {
                start: end,
                end: start,
            }
        })
    }

    pub(in crate::simulation::network::surface::tests) fn contains(self, edge: Self) -> bool {
        test_render_xz_vertex_key_lies_on_segment(edge.start, self.start, self.end)
            && test_render_xz_vertex_key_lies_on_segment(edge.end, self.start, self.end)
    }
}

pub(in crate::simulation::network::surface::tests) fn test_render_xz_vertex_key_lies_on_segment(
    point: TestRenderXzVertexKey,
    start: TestRenderXzVertexKey,
    end: TestRenderXzVertexKey,
) -> bool {
    let dx = i128::from(end.x_key - start.x_key);
    let dz = i128::from(end.z_key - start.z_key);
    let px = i128::from(point.x_key - start.x_key);
    let pz = i128::from(point.z_key - start.z_key);
    dx * pz - dz * px == 0
        && point.x_key >= start.x_key.min(end.x_key)
        && point.x_key <= start.x_key.max(end.x_key)
        && point.z_key >= start.z_key.min(end.z_key)
        && point.z_key <= start.z_key.max(end.z_key)
}
