//! Demand action contracts consumed by building placement and lifecycle code.

use super::types::DemandUse;
use crate::simulation::buildings::allocator::Building;

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

pub(super) fn demand_building_action_key(building: &Building) -> DemandBuildingActionKey {
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
