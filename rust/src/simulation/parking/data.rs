// SPDX-License-Identifier: GPL-3.0-or-later

//! Dense parking rows, source identities, and bounded neighbourhood indices.

// `soa_derive`'s expansion writes `field: field` for every column of `Place` and `Vehicle`.
#![allow(clippy::redundant_field_names)]

use super::*;
use crate::assets::asset::validation::MAX_ROUTE_POINTS;
use soa_derive::StructOfArray;

/// The geometry owner of a place; building identities survive allocator swap removal.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub(crate) enum PlaceSource {
    /// Stable building handle and the anchor's manifest index, independent of dense row swaps.
    Building { building: u64, anchor: u32 },
    /// One parking lane of a road edge (`PARK-05`). Live edges are tombstoned, never renumbered;
    /// a save renumbers them past the deleted ones and maps this edge with them.
    Kerb { edge: u32, right: bool },
    /// A building's proven `loading_bay` anchor (`PARK-19`): a stop for freight, never parking
    /// supply. In no chunk list, so no search, catchment, wake or render pass sees it.
    Dock { building: u64, anchor: u32 },
}

/// One public bay. Membership offsets make chunk/source removal O(1).
#[derive(StructOfArray)]
#[soa_derive(Debug, Clone)]
pub(crate) struct Place {
    pub source: PlaceSource,
    pub active: bool,
    pub retired_for: Handle<Vehicle>,
    pub road_point: [f32; 2],
    pub identity: [u64; 2],
    pub position: [f32; 3],
    pub path: [[f32; 2]; 4],
    // A car place's long way in `ParkingSystem::car_ways` (`PARK-25`), whose first, second,
    // second to last and last points `path` holds; `ways::NO_WAY` when `path` is the whole way.
    // A column, so it moves with the row on swap removal.
    pub way: u32,
    pub access: crate::simulation::buildings::allocator::BuildingEntrance,
    pub heading: f32,
    pub width: f32,
    pub length: f32,
    pub occupant: Handle<Vehicle>,
    pub reservation: Handle<Vehicle>,
    // The freight carrier holding a dock (`PARK-20`); none on a bay, and on every dock until
    // carriers take docks.
    pub freight_holder: crate::simulation::economy::agents::AgentHandle,
    // A dock's is its slot in its chunk's dock list, which only the paint reads (`PARK-30`):
    // a dock is in no car chunk list.
    pub chunk_slot: usize,
    pub source_slot: usize,
    // Derived, never saved: this bay's slot in its chunk's `parked` list, or `NOT_PARKED`.
    pub parked_slot: u32,
    // Derived, never saved: the row-major pose of `ground_model` standing here on the ground as
    // it is now. A car's contacts belong to its model, so the pose is one model's.
    pub ground: [f32; 12],
    pub ground_model: u8,
}

/// `Place::ground_model` of a place with no valid pose: new, loaded, or its ground moved.
pub(super) const GROUND_UNSOLVED: u8 = u8::MAX;

/// `Place::parked_slot` of a bay on no parked list: empty, or its car is moving.
pub(super) const NOT_PARKED: u32 = u32::MAX;

/// A persistent car, independent of the resident movement FSM in PARK-01.
#[derive(StructOfArray)]
#[soa_derive(Debug, Clone)]
pub(crate) struct Vehicle {
    pub home: [f32; 2],
    pub home_building: u64,
    pub owner: u64,
    pub borrower: crate::simulation::economy::agents::AgentHandle,
    pub moving: bool,
    pub retired_places: [Handle<Place>; 3],
    pub place: Handle<Place>,
    pub reserved: Handle<Place>,
    pub model: u8,
    pub colour: u8,
}

#[derive(Default, Debug)]
pub(super) struct Chunk {
    pub places: Vec<Handle<Place>>,
    // Derived, never saved: the bays a parked car stands on, in no order. Its capacity follows
    // `places`, so a custody event never allocates.
    pub parked: Vec<Handle<Place>>,
    // The publication this chunk's last one replaced, whose buffers the next rebuild reuses.
    pub spare: Option<Arc<render::ParkedChunk>>,
    pub dirty: bool,
    pub release_pending: bool,
    pub wake_after: Option<Handle<Vehicle>>,
    pub version: u64,
}

/// A build-time geometry product, without live custody or occupancy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bay {
    pub identity: [u64; 2],
    pub position: [f32; 3],
    pub path: [[f32; 2]; 4],
    pub heading: f32,
    pub width: f32,
    pub length: f32,
}

/// One way of a dock's authored drive (`PARK-21`), or a car place's long way on its way to the
/// place rows (`PARK-25`): a polyline of at most `MAX_ROUTE_POINTS` world points `[x, z]`, kept
/// inline so building one needs no allocation. Empty: no such way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DockRoute {
    points: [[f32; 2]; MAX_ROUTE_POINTS],
    len: u8,
}

impl DockRoute {
    /// No way: a dock without an exit has no way out.
    pub(crate) const EMPTY: Self = Self {
        points: [[0.0; 2]; MAX_ROUTE_POINTS],
        len: 0,
    };

    /// The route through `points`, or `None` when they are more than `MAX_ROUTE_POINTS`.
    /// O(points).
    pub(crate) fn new(points: &[[f32; 2]]) -> Option<Self> {
        let mut route = Self::EMPTY;
        route.points.get_mut(..points.len())?.copy_from_slice(points);
        route.len = points.len() as u8;
        Some(route)
    }

    /// The route's points, in travel order. O(1).
    pub(crate) fn as_slice(&self) -> &[[f32; 2]] {
        &self.points[..usize::from(self.len)]
    }
}

/// A dock's authored drive (`PARK-21`), kept beside its row: the whole way in from the arrival
/// gate's road connection to the stop, and the way out from the stop to the exit gate's road
/// connection, or empty when the truck backs out along the way in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct DockWay {
    /// The arrival gate's road connection, its approach, the waypoints (or the aisle) and the
    /// stop.
    pub route_in: DockRoute,
    /// The stop, the waypoints, the exit gate's approach and its road connection; empty when
    /// the dock names no exit.
    pub route_out: DockRoute,
}

/// Exact quote supplied by a caller, or Euclidean walking time in the foundation query.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ParkingCost {
    pub walk_m: f32,
    pub walk_s: f32,
    pub drive_s: f32,
}

pub(super) fn chunk_at(point: [f32; 2]) -> (i32, i32) {
    let size = crate::simulation::network::graph::RegionGraph::CHUNK_SIZE;
    (
        (point[0] / size).floor() as i32,
        (point[1] / size).floor() as i32,
    )
}
