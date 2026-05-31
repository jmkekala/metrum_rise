//! Core parcel identity, placement geometry, and placement error types.

use godot::prelude::Vector2;

/// Stable parcel identifier persisted in saves and referenced by buildings.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParcelId(u64);

impl ParcelId {
    /// Reserved non-parcel value used by legacy or explicit non-zoned buildings.
    pub const NONE: Self = Self(0);

    /// Creates a parcel id from its persisted integer representation.
    pub fn from_raw(value: u64) -> Self {
        Self(value)
    }

    /// Returns the persisted integer representation of this parcel id.
    pub fn raw(self) -> u64 {
        self.0
    }

    /// Returns true when this is the reserved non-parcel value.
    pub fn is_none(self) -> bool {
        self.0 == 0
    }
}

/// User-authored road-aligned lot used as zoning and building-spawn authority.
#[derive(Clone, Debug)]
pub struct ZoningParcel {
    id: ParcelId,
    edge_idx: usize,
    side: i8,
    frontage_center_t: f32,
    frontage_m: f32,
    depth_m: f32,
    zone_profile_runtime_id: u16,
    occupied_building: Option<usize>,
    front_center: Vector2,
    center: Vector2,
    tangent: Vector2,
    normal: Vector2,
    corners: [Vector2; 4],
    aabb_min: Vector2,
    aabb_max: Vector2,
}

impl ZoningParcel {
    /// Returns the stable parcel id.
    pub fn id(&self) -> ParcelId {
        self.id
    }

    /// Returns the road edge this parcel fronts.
    pub fn edge_idx(&self) -> usize {
        self.edge_idx
    }

    /// Returns the side of the road this parcel occupies: `1` or `-1`.
    pub fn side(&self) -> i8 {
        self.side
    }

    /// Returns the center `t` coordinate of the parcel frontage on its road edge.
    pub fn frontage_center_t(&self) -> f32 {
        self.frontage_center_t
    }

    /// Returns the frontage width in metres.
    pub fn frontage_m(&self) -> f32 {
        self.frontage_m
    }

    /// Returns the parcel depth in metres.
    pub fn depth_m(&self) -> f32 {
        self.depth_m
    }

    /// Returns the assigned zoning-profile runtime id, or `0` for free/unzoned parcels.
    pub fn zone_profile_runtime_id(&self) -> u16 {
        self.zone_profile_runtime_id
    }

    /// Returns the occupied building index when a private zoned building claims this parcel.
    pub fn occupied_building(&self) -> Option<usize> {
        self.occupied_building
    }

    /// Returns true when no building currently claims this parcel.
    pub fn is_available(&self) -> bool {
        self.occupied_building.is_none()
    }

    /// Returns the world-space center of the parcel frontage line.
    pub fn front_center(&self) -> Vector2 {
        self.front_center
    }

    /// Returns the world-space center of the parcel rectangle.
    pub fn center(&self) -> Vector2 {
        self.center
    }

    /// Returns the unit vector along the road frontage.
    pub fn tangent(&self) -> Vector2 {
        self.tangent
    }

    /// Returns the unit vector pointing from the road into the parcel.
    pub fn normal(&self) -> Vector2 {
        self.normal
    }

    /// Returns parcel corners in front-left, front-right, rear-right, rear-left order.
    pub fn corners(&self) -> [Vector2; 4] {
        self.corners
    }

    /// Returns the minimum XZ corner of the parcel AABB.
    pub fn aabb_min(&self) -> Vector2 {
        self.aabb_min
    }

    /// Returns the maximum XZ corner of the parcel AABB.
    pub fn aabb_max(&self) -> Vector2 {
        self.aabb_max
    }

    pub(crate) fn set_zone_profile_runtime_id(&mut self, runtime_id: u16) {
        self.zone_profile_runtime_id = runtime_id;
    }

    pub(crate) fn set_occupied_building(&mut self, building_idx: Option<usize>) {
        self.occupied_building = building_idx;
    }

    pub(crate) fn set_edge_idx(&mut self, edge_idx: usize) {
        self.edge_idx = edge_idx;
    }

    pub(super) fn new(
        id: ParcelId,
        geometry: ParcelGeometry,
        zone_profile_runtime_id: u16,
    ) -> Self {
        Self {
            id,
            edge_idx: geometry.edge_idx,
            side: geometry.side,
            frontage_center_t: geometry.frontage_center_t,
            frontage_m: geometry.frontage_m,
            depth_m: geometry.depth_m,
            zone_profile_runtime_id,
            occupied_building: None,
            front_center: geometry.front_center,
            center: geometry.center,
            tangent: geometry.tangent,
            normal: geometry.normal,
            corners: geometry.corners,
            aabb_min: geometry.aabb_min,
            aabb_max: geometry.aabb_max,
        }
    }
}

/// Geometry for a parcel that has been projected onto a road edge.
#[derive(Clone, Copy, Debug)]
pub struct ParcelGeometry {
    /// Road edge index that owns the parcel frontage.
    pub edge_idx: usize,
    /// Road side: `1` or `-1`.
    pub side: i8,
    /// Center `t` coordinate along the road edge.
    pub frontage_center_t: f32,
    /// Parcel frontage width in metres.
    pub frontage_m: f32,
    /// Parcel depth in metres.
    pub depth_m: f32,
    /// World-space center of the parcel frontage line.
    pub front_center: Vector2,
    /// World-space center of the parcel rectangle.
    pub center: Vector2,
    /// Unit vector along the road frontage.
    pub tangent: Vector2,
    /// Unit vector pointing from the road into the parcel.
    pub normal: Vector2,
    /// Corners in front-left, front-right, rear-right, rear-left order.
    pub corners: [Vector2; 4],
    /// Minimum XZ corner of the parcel AABB.
    pub aabb_min: Vector2,
    /// Maximum XZ corner of the parcel AABB.
    pub aabb_max: Vector2,
}

/// Reason a parcel placement request was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParcelPlacementError {
    /// The selected profile id was not present in the zoning-profile registry.
    UnknownProfile,
    /// No buildable road edge was close enough to attach the requested parcel.
    NoRoadAttachment,
    /// The projected frontage would extend beyond the road edge ends.
    FrontageOutOfBounds,
    /// The requested parcel dimensions are outside the supported first-slice edit range.
    InvalidDimensions,
    /// The requested drag-run gap is outside the supported first-slice edit range.
    InvalidGap,
    /// The requested rectangle overlaps an existing parcel.
    OverlapsExistingParcel,
    /// The requested rectangle overlaps another road-owned corridor.
    OverlapsRoad,
    /// One or more parcel corners would sit outside the authored world extent.
    OutsideWorld,
}
