// SPDX-License-Identifier: GPL-2.0-only

//! Visible-surface height, band, ray, overlap and lane queries over published pieces.
//!
//! Every query maps XZ to pieces through the chunk index and then to the triangles that render,
//! in `O(1)` chunk lookup plus the triangles of one piece grid cell. `ROAD-68-A` publishes no
//! pieces, so each answers `None`, `false` or empty.

use super::{BandKind, RoadVec2, RoadVec3, RoadbedSystem};
use godot::prelude::Vector3;

/// Identity of one roadbed piece.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PieceId {
    /// The span piece of an edge.
    Span(usize),
    /// The junction, bend or terminal piece of a node.
    Node(u32),
}

/// Prepared owner-local surface lookup, reused for every point of one lane.
pub struct LaneSurfaceQuery<'a> {
    roadbed: &'a RoadbedSystem,
    carriageway_only: bool,
}

impl LaneSurfaceQuery<'_> {
    /// Visible height of the lane owner's surface at a world XZ point; carriageway-only for
    /// vehicle lanes.
    pub fn height_at(&self, x: f32, z: f32) -> Option<f32> {
        if self.carriageway_only {
            self.roadbed.carriageway_height_at(x, z)
        } else {
            self.roadbed.height_at(x, z)
        }
    }
}

impl RoadbedSystem {
    /// Height of the highest visible road triangle at a world XZ point.
    pub fn height_at(&self, _x: f32, _z: f32) -> Option<f32> {
        None
    }

    /// Height of the highest visible carriageway triangle at a world XZ point.
    pub fn carriageway_height_at(&self, _x: f32, _z: f32) -> Option<f32> {
        None
    }

    /// Band kind of the highest visible road triangle at a world XZ point.
    pub fn band_at(&self, _x: f32, _z: f32) -> Option<BandKind> {
        None
    }

    /// True when a visible triangle of an accepted band kind lies within `radius_m` of `center`.
    pub fn any_band_near_disc(
        &self,
        _center: RoadVec2,
        _radius_m: f64,
        _kinds: impl Fn(BandKind) -> bool,
    ) -> bool {
        false
    }

    /// First hit of a world-space ray against the visible road surface.
    pub fn raycast(&self, _origin: Vector3, _direction: Vector3) -> Option<Vector3> {
        None
    }

    /// Prepares the surface lookup for one lane owned by `edge_id` or, for a junction lane,
    /// `node_id`.
    pub fn lane_query(
        &self,
        _edge_id: usize,
        _node_id: usize,
        carriageway_only: bool,
    ) -> LaneSurfaceQuery<'_> {
        LaneSurfaceQuery {
            roadbed: self,
            carriageway_only,
        }
    }

    /// Visits visible top triangles, with their band kind, whose piece grid cells touch the XZ
    /// bounds `[min_x, min_z, max_x, max_z]`, until `visit` returns true; returns whether it did.
    pub fn any_top_triangle_in_bounds(
        &self,
        _bounds_xz: [f64; 4],
        _visit: impl FnMut([RoadVec3; 3], BandKind) -> bool,
    ) -> bool {
        false
    }

    /// Pieces whose bounds touch the XZ bounds `[min_x, min_z, max_x, max_z]`, each with the
    /// generation that built it, in ascending piece order.
    pub fn pieces_in_bounds(
        &self,
        _bounds_xz: [f64; 4],
    ) -> impl Iterator<Item = (PieceId, u64)> + '_ {
        std::iter::empty()
    }
}
