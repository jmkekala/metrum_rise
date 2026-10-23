// SPDX-License-Identifier: GPL-2.0-only

//! Sampled span cross-sections read by section-based renderers (markings, bridges, furniture).

use super::{BandKind, RoadVec2};

/// One ordered lateral band inside a span cross-section.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadBand {
    /// Band classification.
    pub kind: BandKind,
    /// Lateral start offset from the section centerline, in metres; negative is left.
    pub lateral_start_m: f32,
    /// Lateral end offset from the section centerline, in metres.
    pub lateral_end_m: f32,
    /// World height at `lateral_start_m`, in metres.
    pub height_start_m: f32,
    /// World height at `lateral_end_m`, in metres.
    pub height_end_m: f32,
}

/// One sampled cross-section along an edge's span piece.
#[derive(Clone, Debug, PartialEq)]
pub struct RoadSection {
    /// Owning edge id.
    pub edge_idx: usize,
    /// Longitudinal distance from the edge start, in metres.
    pub s_m: f32,
    /// Section centre in world XZ metres.
    pub center_xz: RoadVec2,
    /// Centerline height in world metres.
    pub center_height_m: f32,
    /// Unit tangent in XZ, along the edge's forward direction.
    pub tangent_xz: RoadVec2,
    /// Unit lateral axis in XZ.
    pub lateral_xz: RoadVec2,
    /// Lateral bands, ordered left to right in the edge's forward frame.
    pub bands: Vec<RoadBand>,
}
