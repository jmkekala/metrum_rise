// SPDX-License-Identifier: GPL-2.0-only

//! Inspection-only parking counters for the debug console. Nothing here feeds a decision.

use super::ParkingSystem;
use crate::simulation::core::handles::Handle;

/// Why a resident departure from a building drove or walked. Counted in the ordered parking
/// phase since the last load; not saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DepartureOutcome {
    Drove,
    NoCar,
    Child,
    CarStored,
    CarInUse,
    NoParkingNearDestination,
    DestinationParkingFull,
    NoDriveRoute,
    WalkingFaster,
    ReservationLost,
}

impl DepartureOutcome {
    /// Every outcome, in console print order.
    pub(crate) const ALL: [Self; 10] = [
        Self::Drove,
        Self::NoCar,
        Self::Child,
        Self::CarStored,
        Self::CarInUse,
        Self::NoParkingNearDestination,
        Self::DestinationParkingFull,
        Self::NoDriveRoute,
        Self::WalkingFaster,
        Self::ReservationLost,
    ];

    /// Plain-language label shown in the debug console.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Drove => "drove",
            Self::NoCar => "walked: household has no car",
            Self::Child => "walked: child",
            Self::CarStored => "walked: car stored off-map (no free spot near home)",
            Self::CarInUse => "walked: car away with another household member",
            Self::NoParkingNearDestination => "walked: no parking within 400 m walk of destination",
            Self::DestinationParkingFull => "walked: all parking near destination full",
            Self::NoDriveRoute => "walked: no drive route between the two spots",
            Self::WalkingFaster => "walked: walking was quicker",
            Self::ReservationLost => "walked: destination spot taken at departure",
        }
    }
}

/// Departure outcome counts since load.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DepartureStats {
    counts: [u64; DepartureOutcome::ALL.len()],
}

impl DepartureStats {
    /// O(1).
    pub(crate) fn record(&mut self, outcome: DepartureOutcome) {
        self.counts[outcome as usize] += 1;
    }

    /// O(1).
    pub(crate) fn count(&self, outcome: DepartureOutcome) -> u64 {
        self.counts[outcome as usize]
    }
}

/// Car custody breakdown for inspection.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct VehicleBreakdown {
    pub parked: usize,
    pub moving: usize,
    pub borrowed: usize,
    pub reserved: usize,
}

impl ParkingSystem {
    /// Classifies why a car trip was not offered. O(destination candidate list), called only
    /// for departures that already failed their quote.
    pub(crate) fn walk_reason(
        &self,
        car: Handle<super::Vehicle>,
        adult: bool,
        destination: u64,
    ) -> DepartureOutcome {
        if car.is_none() {
            return DepartureOutcome::NoCar;
        }
        if !adult {
            return DepartureOutcome::Child;
        }
        let Some(v) = self.vehicle_handles.resolve(car) else {
            return DepartureOutcome::NoCar;
        };
        if self.vehicles.place[v].is_none() && !self.vehicles.moving[v] {
            return DepartureOutcome::CarStored;
        }
        if self.vehicles.moving[v]
            || !self.vehicles.borrower[v].is_none()
            || !self.vehicles.reserved[v].is_none()
        {
            return DepartureOutcome::CarInUse;
        }
        if self.economy_ends(destination).is_none() {
            return DepartureOutcome::NoParkingNearDestination;
        }
        if self.first_free_candidate(destination).is_none() {
            return DepartureOutcome::DestinationParkingFull;
        }
        DepartureOutcome::NoDriveRoute
    }

    /// O(vehicles), console inspection only.
    pub(crate) fn vehicle_breakdown(&self) -> VehicleBreakdown {
        let mut breakdown = VehicleBreakdown::default();
        for v in 0..self.vehicles.len() {
            breakdown.parked +=
                usize::from(!self.vehicles.place[v].is_none() && !self.vehicles.moving[v]);
            breakdown.moving += usize::from(self.vehicles.moving[v]);
            breakdown.borrowed += usize::from(!self.vehicles.borrower[v].is_none());
            breakdown.reserved += usize::from(!self.vehicles.reserved[v].is_none());
        }
        breakdown
    }
}
