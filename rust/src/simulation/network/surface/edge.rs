// SPDX-License-Identifier: GPL-2.0-only

//! Edge input conditioning, preview compilation, and sampled cross-section generation.

use crate::simulation::network::types::EdgeClass;

mod handoff;
mod input;
mod polyline;
mod preview;
mod preview_visual;
mod profile;
mod sections;

use handoff::EdgeMouthPolicy;
/// Standard vertical step from carriageway/asphalt to raised curb or sidewalk top.
pub(crate) const CURB_STEP_HEIGHT_M: f32 = profile::CURB_STEP_HEIGHT_M;

#[cfg(test)]
pub(crate) use input::PreparedRoadInput;
pub(crate) use input::RoadExtensionReprofile;
pub use preview::{PreviewRoadSurfaceResult, RoadPreviewValidation};
pub(crate) use preview_visual::RoadPreviewVisualMesh;
use profile::EdgeProfilePlaneBlend;

pub(crate) fn edge_class_sort_key(edge_class: EdgeClass) -> u8 {
    match edge_class {
        EdgeClass::Standard => 0,
        EdgeClass::Bridge => 1,
        EdgeClass::Tunnel => 2,
    }
}
