// SPDX-License-Identifier: GPL-2.0-only

//! Dense parking rows, source identities, and bounded neighbourhood indices.

// `soa_derive`'s expansion writes `field: field` for every column of `Place` and `Vehicle`.
#![allow(clippy::redundant_field_names)]

use super::*;
use soa_derive::StructOfArray;

/// The geometry owner of a place; building identities survive allocator swap removal.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub(crate) enum PlaceSource {
    /// Stable building handle and the anchor's manifest index, independent of dense row swaps.
    Building { building: u64, anchor: u32 },
    /// One parking lane of a road edge (`PARK-05`); edges are tombstoned, never renumbered.
    Kerb { edge: u32, right: bool },
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
    pub access: crate::simulation::buildings::allocator::BuildingEntrance,
    pub heading: f32,
    pub width: f32,
    pub length: f32,
    pub occupant: Handle<Vehicle>,
    pub reservation: Handle<Vehicle>,
    pub chunk_slot: usize,
    pub source_slot: usize,
}

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
