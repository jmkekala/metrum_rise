// SPDX-License-Identifier: GPL-2.0-only

//! Demand action contracts consumed by building placement and lifecycle code.

use super::types::DemandUse;
use crate::simulation::buildings::allocator::Building;
use crate::simulation::zoning::ZoneType;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    pub(crate) density: String,
}

/// Spawn candidates grouped by the three demand-owned private zone families.
#[derive(Clone, Debug, Default)]
pub(crate) struct DemandSpawnCandidatesByUse {
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
