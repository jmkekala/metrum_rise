//! Edge input conditioning, preview compilation, and sampled cross-section generation.

use crate::simulation::network::types::EdgeClass;

mod handoff;
mod input;
mod polyline;
mod preview;
mod profile;
mod sections;

pub use preview::PreviewRoadSurfaceResult;
use profile::EdgeProfilePlaneBlend;

#[cfg(test)]
pub(super) const CURB_STEP_HEIGHT_M: f32 = profile::CURB_STEP_HEIGHT_M;

pub(crate) fn edge_class_sort_key(edge_class: EdgeClass) -> u8 {
    match edge_class {
        EdgeClass::Standard => 0,
        EdgeClass::Bridge => 1,
        EdgeClass::Tunnel => 2,
    }
}
