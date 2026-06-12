//! Public parcel zoning edit limits and first-slice defaults.

/// First-slice authored parcel frontage in metres.
pub const DEFAULT_PARCEL_FRONTAGE_M: f32 = 20.0;
/// First-slice authored parcel depth in metres.
pub const DEFAULT_PARCEL_DEPTH_M: f32 = 20.0;
/// Smallest player-authored parcel frontage accepted by the Rust placement path.
pub const MIN_PARCEL_FRONTAGE_M: f32 = 5.0;
/// Largest player-authored parcel frontage accepted by the Rust placement path.
pub const MAX_PARCEL_FRONTAGE_M: f32 = 80.0;
/// Smallest player-authored parcel depth accepted by the Rust placement path.
pub const MIN_PARCEL_DEPTH_M: f32 = 5.0;
/// Largest player-authored parcel depth accepted by the Rust placement path.
pub const MAX_PARCEL_DEPTH_M: f32 = 120.0;
/// Smallest spacing between generated drag-run parcels.
pub const MIN_PARCEL_GAP_M: f32 = 0.0;
/// Largest spacing between generated drag-run parcels.
pub const MAX_PARCEL_GAP_M: f32 = 20.0;
