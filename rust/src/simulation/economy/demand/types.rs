//! Shared demand value types and constants.

use crate::simulation::zoning::ZoneType;

pub(super) const EPSILON: f32 = 0.0001;
pub(super) const DEMAND_HOURLY_CADENCE_FRACTION: f32 = 1.0 / 24.0;
pub(super) const RESIDENTIAL_SPAWN_VACANT_SLOT_RESERVE_RATIO: f32 = 0.10;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum DemandUse {
    Residential,
    Commercial,
    Industrial,
}

impl DemandUse {
    pub(super) fn zone_type(self) -> ZoneType {
        match self {
            Self::Residential => ZoneType::Residential,
            Self::Commercial => ZoneType::Commercial,
            Self::Industrial => ZoneType::Industrial,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum DemandChannel {
    ResidentialGrowth,
    CommercialGrowth,
    IndustrialGrowth,
}

impl DemandChannel {
    pub(super) fn from_str_name(value: &str) -> Option<Self> {
        match value.trim() {
            "ResidentialGrowth" => Some(Self::ResidentialGrowth),
            "CommercialGrowth" => Some(Self::CommercialGrowth),
            "IndustrialGrowth" => Some(Self::IndustrialGrowth),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct GrowthProfileRuntime {
    pub(super) demand_channel: DemandChannel,
    pub(super) spawn_threshold: f32,
    pub(super) despawn_threshold: f32,
    pub(super) upgrade_threshold: f32,
    pub(super) downgrade_threshold: f32,
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct UseTuningF32 {
    pub(crate) residential: f32,
    pub(crate) commercial: f32,
    pub(crate) industrial: f32,
}

impl UseTuningF32 {
    pub(super) fn get(&self, use_kind: DemandUse) -> f32 {
        match use_kind {
            DemandUse::Residential => self.residential,
            DemandUse::Commercial => self.commercial,
            DemandUse::Industrial => self.industrial,
        }
    }

    pub(super) fn get_mut(&mut self, use_kind: DemandUse) -> &mut f32 {
        match use_kind {
            DemandUse::Residential => &mut self.residential,
            DemandUse::Commercial => &mut self.commercial,
            DemandUse::Industrial => &mut self.industrial,
        }
    }

    pub(crate) fn as_array(self) -> [f32; 3] {
        [self.residential, self.commercial, self.industrial]
    }
}
