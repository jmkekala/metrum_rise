//! Edge input conditioning, preview compilation, and sampled cross-section generation.

mod handoff;
mod input;
mod polyline;
mod preview;
mod profile;
mod sections;

pub use preview::PreviewRoadSurfaceResult;

pub(super) const VISUAL_MIN_SPAN_LENGTH_M: f32 = handoff::VISUAL_MIN_SPAN_LENGTH_M;
#[cfg(test)]
pub(super) const CURB_STEP_HEIGHT_M: f32 = profile::CURB_STEP_HEIGHT_M;
