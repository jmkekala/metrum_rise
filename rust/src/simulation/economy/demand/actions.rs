// SPDX-License-Identifier: GPL-2.0-only

//! Demand action contracts consumed by building placement and lifecycle code.

use super::types::DemandUse;
use crate::simulation::buildings::allocator::Building;
use crate::simulation::core::scheduler::SLOTS_PER_MINUTE;
use crate::simulation::zoning::ZoneType;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DemandBuildingActionKey {
    pub(crate) parcel_id: u64,
    pub(crate) edge_idx: usize,
    pub(crate) side: i8,
    pub(crate) cell_x: usize,
    pub(crate) width_cells: u16,
    pub(crate) depth_cells: u16,
    pub(crate) level: u8,
    pub(crate) asset_id: String,
}

impl DemandBuildingActionKey {
    /// Revalidates a selected action against the parcel's current building without cloning its key.
    pub(crate) fn matches_building(&self, building: &Building) -> bool {
        self.parcel_id == building.parcel_id
            && self.edge_idx == building.edge_idx
            && self.side == building.side
            && self.cell_x == building.cell_x
            && self.width_cells == building.width_cells
            && self.depth_cells == building.depth_cells
            && self.level == building.level
            && self.asset_id == building.asset_id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DemandLevelChangeAction {
    pub(crate) building: DemandBuildingActionKey,
    pub(crate) target_asset_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DemandSpawnAction {
    pub(crate) parcel_id: u64,
    pub(crate) asset_id: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DemandUseActionPlan {
    pub(crate) despawns: Vec<DemandBuildingActionKey>,
    pub(crate) downgrades: Vec<DemandLevelChangeAction>,
    pub(crate) upgrades: Vec<DemandLevelChangeAction>,
    pub(crate) spawns: Vec<DemandSpawnAction>,
}

/// Slots between a planning slot's setup and its plan (`SIM-02-K`): the setup counts the live
/// workforce and scans the lots and existing buildings at the first slot of the slot's minute,
/// the plan runs this many slots later in the same minute, so each half holds about half the
/// pass and at 32× (about 8.5 slots a frame) the two halves mostly fall in frames of their own.
pub(crate) const PLANNING_SETUP_LEAD: u64 = SLOTS_PER_MINUTE / 2;

/// The hour's demand planning slots, in order (`SIM-02-I`), one a game minute (`SIM-02-K`):
/// residential, commercial and industrial plan their zone in the hour's minutes 0, 1 and 2;
/// minute 3 plans the downgrades and the hour's admission, then the hour's plan is executed.
/// The discriminant is the slot of the hour the plan runs in, [`PLANNING_SETUP_LEAD`] slots
/// after the minute's first slot, which holds its setup ([`Self::setup_slot`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u64)]
pub(crate) enum PlanningSlot {
    /// Minute 0, after the operational hour and the census published at the hour's first slot:
    /// homes.
    Residential = PLANNING_SETUP_LEAD,
    /// Minute 1: shops and services.
    Commercial = SLOTS_PER_MINUTE + PLANNING_SETUP_LEAD,
    /// Minute 2: industry.
    Industrial = 2 * SLOTS_PER_MINUTE + PLANNING_SETUP_LEAD,
    /// Minute 3: downgrades, admission and the hour's actions.
    Actions = 3 * SLOTS_PER_MINUTE + PLANNING_SETUP_LEAD,
}

impl PlanningSlot {
    /// The slot of the hour of this planning slot's setup: its minute's first slot
    /// (`SIM-02-K`).
    pub(crate) const fn setup_slot(self) -> u64 {
        self as u64 - PLANNING_SETUP_LEAD
    }

    /// Every planning slot, in order (test drivers of the four slots).
    #[cfg(test)]
    pub(crate) const ALL: [Self; 4] = [
        Self::Residential,
        Self::Commercial,
        Self::Industrial,
        Self::Actions,
    ];

    /// The zone a zone slot plans; none for the actions slot.
    pub(super) fn zone(self) -> Option<DemandUse> {
        match self {
            Self::Residential => Some(DemandUse::Residential),
            Self::Commercial => Some(DemandUse::Commercial),
            Self::Industrial => Some(DemandUse::Industrial),
            Self::Actions => None,
        }
    }

    /// The zone type a zone slot plans and queues.
    pub(crate) fn zone_type(self) -> Option<ZoneType> {
        self.zone().map(DemandUse::zone_type)
    }

    /// The test-only work counter of this slot's skips.
    pub(super) fn skip_work(self) -> crate::debug::work::Work {
        use crate::debug::work::Work;
        match self {
            Self::Residential => Work::ResidentialPlanningSkips,
            Self::Commercial => Work::CommercialPlanningSkips,
            Self::Industrial => Work::IndustrialPlanningSkips,
            Self::Actions => Work::ActionsPlanningSkips,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct DemandBuildingActionPlan {
    pub(crate) residential: DemandUseActionPlan,
    pub(crate) commercial: DemandUseActionPlan,
    pub(crate) industrial: DemandUseActionPlan,
}

impl DemandBuildingActionPlan {
    pub(super) fn use_plan_mut(&mut self, use_kind: DemandUse) -> &mut DemandUseActionPlan {
        match use_kind {
            DemandUse::Residential => &mut self.residential,
            DemandUse::Commercial => &mut self.commercial,
            DemandUse::Industrial => &mut self.industrial,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DemandSpawnCandidate {
    pub(crate) action: DemandSpawnAction,
}

/// Spawn candidates grouped by the three demand-owned private zone families.
#[derive(Clone, Debug, Default)]
pub(crate) struct DemandSpawnCandidatesByUse {
    /// Site-discovery evidence, ordered residential/commercial/industrial.
    pub(crate) availability: [SpawnSiteAvailability; 3],
    /// Legal residential build sites in allocator build-site order.
    pub(crate) residential: Vec<DemandSpawnCandidate>,
    /// Legal commercial build sites in allocator build-site order.
    pub(crate) commercial: Vec<DemandSpawnCandidate>,
    /// Legal industrial build sites in allocator build-site order.
    pub(crate) industrial: Vec<DemandSpawnCandidate>,
}

impl DemandSpawnCandidatesByUse {
    /// Appends one candidate to the matching private-zone bucket.
    pub(crate) fn push_zone_type(&mut self, zone_type: ZoneType, candidate: DemandSpawnCandidate) {
        match zone_type {
            ZoneType::Residential => self.residential.push(candidate),
            ZoneType::Commercial => self.commercial.push(candidate),
            ZoneType::Industrial => self.industrial.push(candidate),
            _ => {}
        }
    }

    /// Removes and returns candidates for one private zone family.
    pub(crate) fn take_zone_type(&mut self, zone_type: ZoneType) -> Vec<DemandSpawnCandidate> {
        match zone_type {
            ZoneType::Residential => std::mem::take(&mut self.residential),
            ZoneType::Commercial => std::mem::take(&mut self.commercial),
            ZoneType::Industrial => std::mem::take(&mut self.industrial),
            _ => Vec::new(),
        }
    }
}

pub(crate) fn demand_building_action_key(building: &Building) -> DemandBuildingActionKey {
    DemandBuildingActionKey {
        parcel_id: building.parcel_id,
        edge_idx: building.edge_idx,
        side: building.side,
        cell_x: building.cell_x,
        width_cells: building.width_cells,
        depth_cells: building.depth_cells,
        level: building.level,
        asset_id: building.asset_id.clone(),
    }
}

/// Maps private zone families to the fixed R/C/I reporting order.
pub(crate) fn private_zone_index(zone: ZoneType) -> Option<usize> {
    match zone {
        ZoneType::Residential => Some(0),
        ZoneType::Commercial => Some(1),
        ZoneType::Industrial => Some(2),
        _ => None,
    }
}

/// Placement evidence collected with the existing parallel parcel discovery pass.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SpawnSiteAvailability {
    /// At least one authored initial asset is legal for this use's zoning profiles.
    pub(crate) has_assets: bool,
    /// Unoccupied parcels with a known private zoning profile, before site checks.
    pub(crate) free_lots: usize,
    /// Free parcels with no eligible initial asset for their particular profile.
    pub(crate) missing_asset_lots: usize,
    /// Free parcels whose roads, size, access or terrain prevent a legal placement.
    pub(crate) unusable_lots: usize,
}

impl SpawnSiteAvailability {
    /// Combines independent discovery partitions without allocating.
    pub(crate) fn merge(&mut self, other: Self) {
        self.has_assets |= other.has_assets;
        self.free_lots += other.free_lots;
        self.missing_asset_lots += other.missing_asset_lots;
        self.unusable_lots += other.unusable_lots;
    }
}
