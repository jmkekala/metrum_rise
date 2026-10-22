// SPDX-License-Identifier: GPL-2.0-only

//! Building placement and lifecycle management.
//!
//! [`BuildingAllocator::maintain`] validates parcel/road attachments and rebuilds derived indices
//! and pathing after mutations. Daily maintenance advances rezoning grace; immediate edits do not.
//!
//! Household admission and building growth execute separately from demand-owned hourly plans.

mod admission_order;
mod entrance;
mod geometry;
mod index;
mod lifecycle;
mod placement;
mod production_changes;
mod site;
pub(crate) mod yard;

#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::support::indexed_test_building;

pub(crate) use admission_order::ChunkBounds;
pub(crate) use entrance::{
    BuildingEntranceKey, DemandSiteEntranceKey, FreightEntranceAccess, building_local_xz_basis,
};
pub(crate) use index::SavedVacancyOrder;
#[cfg(test)]
pub(crate) use lifecycle::DemandBuildingActionExecution;
pub(crate) use lifecycle::{BuildingRemovalKind, BuildingRemovalRequest};
pub(crate) use placement::BuildingSiteEnvironment;
pub(crate) use placement::ExplicitServicePlacementPreview;
pub(crate) use placement::SpawnProfileAssetCandidates;
pub(crate) use placement::construction_duration_hours;
pub(crate) use site::BuildingSiteGradingRequest;
pub(crate) use site::{BuildingSiteSurfaceClient, SitePavingPartition};

use crate::assets::{AssetRegistry, ZoneClass};
use crate::debug_log;
use crate::simulation::agriculture::FieldClearanceIndex;
use crate::simulation::core::handles::{Handle, HandleTable};
use crate::simulation::core::scheduler::SLOTS_PER_HOUR;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntime, ResourceRuntimeId, RuntimeEconomyCatalog, load_runtime_economy_catalog,
};
use crate::simulation::economy::households::physical_worker_capacity_for_profile;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::VehicleFrontageAccess;
use crate::simulation::work_area::profile_kind_uses_explicit_work_area;
use crate::simulation::zoning::{ZoneType, ZoningSystem};
use godot::prelude::Vector2;
use std::collections::{HashMap, VecDeque};

/// Shipped baseline private-use families supported by the live zoning-driven runtime.
pub(crate) const BASELINE_PRIVATE_ZONES: [ZoneType; 3] = [
    ZoneType::Residential,
    ZoneType::Commercial,
    ZoneType::Industrial,
];

/// Returns the compact baseline bucket index for one shipped private-use family.
pub(crate) fn baseline_private_zone_slot(zone: ZoneType) -> Option<usize> {
    match zone {
        ZoneType::Residential => Some(0),
        ZoneType::Commercial => Some(1),
        ZoneType::Industrial => Some(2),
        ZoneType::None | ZoneType::Office | ZoneType::Mixed => None,
    }
}

/// Final allocator-side reason a selected demand spawn could not be committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DemandSpawnPlacementRejection {
    /// The selected asset no longer resolves to valid placement parameters.
    AssetUnavailable,
    /// The selected parcel no longer exists in the zoning system.
    ParcelUnavailable,
    /// The parcel exists, but its current geometry or occupancy no longer accepts the asset.
    SlotUnavailable,
    /// A driveway anchor could not resolve an adjacent road surface height.
    DrivewayRoadSurfaceMissing,
    /// Multiple driveway anchors required incompatible flat-site heights.
    DrivewayHeightConflict,
    /// The asset has driveway anchors, but none touch the claimed road edge.
    DrivewayConnectionMissing,
    /// The frontage fallback could not resolve an adjacent road surface height.
    FrontageRoadSurfaceMissing,
    /// The selected flat-site height conflicts with an already placed neighboring site.
    NeighborSiteHeightConflict,
    /// The flat support footprint cannot tie into surrounding terrain or roads within slope limits.
    SiteSupportTieInInvalid,
}

/// Final allocator-side reason an explicit service placement could not be committed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ExplicitServicePlacementRejection {
    /// The selected asset is not present in the loaded registry.
    AssetUnavailable,
    /// The selected asset is not an explicit service-building asset.
    NotServiceBuilding,
    /// The selected asset is not an explicit resource-extractor building.
    NotIndustryBuilding,
    /// The selected utility asset references an unsupported or missing runtime profile.
    UtilityProfileUnavailable,
    /// The selected extractor asset references an unsupported or mismatched runtime profile.
    ExtractorProfileUnavailable,
    /// The selected field asset references an unsupported or mismatched runtime profile.
    FieldProfileUnavailable,
    /// No road frontage near the requested point can accept the building footprint.
    RoadFrontageUnavailable,
    /// A driveway anchor could not resolve an adjacent road surface height.
    DrivewayRoadSurfaceMissing,
    /// Multiple driveway anchors required incompatible flat-site heights.
    DrivewayHeightConflict,
    /// The asset has driveway anchors, but none touch the claimed road edge.
    DrivewayConnectionMissing,
    /// The frontage fallback could not resolve an adjacent road surface height.
    FrontageRoadSurfaceMissing,
    /// The selected flat-site height conflicts with an already placed neighboring site.
    NeighborSiteHeightConflict,
    /// The flat support footprint cannot tie into surrounding terrain or roads within slope limits.
    SiteSupportTieInInvalid,
    /// The selected footprint overlaps an existing building site.
    SiteOverlap,
    /// The selected footprint overlaps an existing road corridor.
    RoadOverlap,
    /// The selected footprint overlaps a committed agricultural field.
    FieldOverlap,
}

/// The applicant seeds of buildings placed outside the demand queue (`SIM-02-I`): this bit
/// and the city counter. Queued spawn ids, the seeds of demand-placed buildings, stay below it.
pub(crate) const CITY_APPLICANT_SEEDS: u64 = 1 << 62;

/// A placed building occupying one authored parcel or an explicit non-zoned site.
pub struct Building {
    /// World-space X centre of the building footprint (metres, ground-plane X axis).
    pub center_x: f32,
    /// World-space Z centre of the building footprint (metres, ground-plane Z axis).
    pub center_y: f32,
    /// Fixed support-surface height captured when the building was placed.
    ///
    /// Rendered building parts use this instead of re-sampling terrain after placement.
    pub support_height_m: f32,
    /// Width of the footprint in zoning cells.
    pub width_cells: u16,
    /// Depth of the footprint in zoning cells.
    pub depth_cells: u16,
    /// Authoritative runtime zoning-profile id captured when this building was placed.
    pub zone_profile_runtime_id: u16,
    /// Stable authored parcel id claimed by this private zoned building; `0` means no parcel.
    pub parcel_id: u64,
    /// Seed of the applicant stream offered this building's household places (`SIM-02-I`):
    /// the id of the queued spawn that placed it, or the city counter's next value
    /// ([`CITY_APPLICANT_SEEDS`]) for a building placed outside the demand queue. A stable
    /// identity, so planning, admission and arrivals draw the same applicants across remaps,
    /// removals and loads. Saved.
    pub applicant_seed: u64,
    /// Redevelopment counter of the claimed parcel when this building was placed.
    ///
    /// Captured rather than read live so the render path needs no zoning lookup, and so the
    /// colour cannot shift under a standing building when its parcel is later cleared.
    pub build_generation: u32,
    /// Cached broad baseline family derived from [`Self::zone_profile_runtime_id`].
    ///
    /// Kept as a hot-path cache for broad R/C/I grouping and economy lookups. Legality comes
    /// from the parcel's authoritative zoning-profile id.
    pub zone_type: ZoneType,
    /// Unit vector pointing from the building frontage toward the road.
    pub facing_dir: Vector2,
    /// T-coordinate (0.0 to 1.0) along [`Self::edge_idx`] for this building's frontage.
    pub frontage_t: f32,
    /// Distance in metres from the road centreline to the building's frontage curb.
    pub side_offset: f32,
    /// How far the road's outer edge stands off the lot's front line at its left end, centre
    /// and right end (`geometry::lot_front_gaps`): zero on a straight road, up to a few metres
    /// where a rigid lot fronts a curve. Paving that reaches the lot line runs on across this
    /// verge to the road. Derived from the road, never saved.
    pub front_gap_m: [f32; 3],
    /// True once this building has entered the permanent deserted state.
    ///
    /// A one-way latch: set by the daily bankruptcy check when budget was negative at end of the
    /// previous day and is still negative at the start of the current day. Never cleared.
    pub is_deserted: bool,
    /// True when this building's budget ended the previous daily settlement below zero.
    ///
    /// Checked at the start of the next daily settlement to declare bankruptcy if still negative.
    pub budget_distress: bool,
    /// Index into [`RegionGraph::edges`] for the road segment this building fronts.
    pub edge_idx: usize,
    /// Road side: `1` = left, `-1` = right.
    pub side: i8,
    /// Column index (along the road) of the building's leading cell.
    pub cell_x: usize,
    /// Depth offset of the building's leading cell (0 = frontage row).
    pub cell_y: u16,
    /// Assigned households, including a reserved household arrival, for every housing provider.
    ///
    /// For residential buildings and farmhouses, this counts family slots,
    /// which must be <= `household_capacity`. Total residents (agents) are tracked
    /// by the AgentSystem referencing these households.
    pub occupancy: u32,
    /// Total workers currently assigned to this building.
    pub worker_count: u32,
    /// Per-building service funding override in `0.0..=1.0`; negative means inherit city policy.
    pub service_funding_override: f32,
    /// Qualified asset ID identifying the model for this building.
    pub asset_id: String,
    /// Current growth tier.
    pub level: u8,
    /// Authored duration of the current construction task in operational hours.
    ///
    /// `0` means this building was placed complete or has already finished construction.
    pub construction_total_hours: u16,
    /// Absolute slot at which construction completes and the building becomes live economy
    /// capacity (`SIM-02-D`); `0` once complete.
    pub construction_complete_slot: u64,
    /// If true, the asset was missing from the registry during load.
    pub broken: bool,
    /// Compact runtime economy-profile id resolved from the asset's authored profile reference.
    pub economy_profile_runtime_id: u16,
    /// True when the asset references an unresolved or unsupported economy profile.
    pub economy_broken: bool,
    /// Current typed on-site inventory by runtime resource id.
    ///
    /// Slot `resource_runtime_id - 1` stores the amount for that resource.
    pub resource_inventory: Vec<f32>,
    /// Lifetime gross revenue collected by this building.
    pub revenue: f32,
    /// Current operating budget available for wages and utility costs.
    ///
    /// May go negative after utility payment; see `budget_distress` and the bankruptcy spec.
    pub operating_budget: f32,
    /// Operating-budget baseline captured after the most recent daily profit-tax settlement.
    pub profit_tax_budget_baseline: f32,
    /// Completed previous-day operating-budget delta captured before the daily baseline reset.
    pub last_day_profit: f32,
    /// First absolute slot at which this building's hourly phase may request freight again; a
    /// passive deadline compared at the phase, never decremented (`SIM-02-J`).
    pub shipment_cooldown_until_slot: u64,
    /// Currency value of input shipments received from OWA during the current day.
    ///
    /// Reset once per day after the demand snapshot is taken. Read by the demand system to
    /// compute the fraction of business and utility inputs sourced from OWA.
    pub daily_owa_input_value: f32,
    /// Currency value of input shipments received from local industrial during the current day.
    ///
    /// Reset once per day after the demand snapshot is taken.
    pub daily_local_input_value: f32,
    /// Net city-funded input purchases committed during the current day, after refunds.
    ///
    /// Reset once per day after the demand snapshot is taken. This is separate from
    /// received-input counters because treasury-backed service purchases are paid at shipment
    /// reservation time, before the cargo may arrive.
    pub daily_city_funded_input_cost: f32,
    /// Net sales revenue collected from household shopping during the current day.
    ///
    /// Rolled into [`Self::recent_household_sales_value`] at the daily economy reset.
    pub daily_household_sales_value: f32,
    /// Aggregate utility power service units produced during the current day.
    ///
    /// Daily utility settlement uses this to route electricity payments without re-reading
    /// end-of-day fuel inventory after the plant has already consumed it.
    pub daily_power_service_units: f32,
    /// This day's electricity delivered to workplaces and residences, in kWh.
    pub daily_electricity: crate::simulation::economy::electricity::BuildingElectricityUsage,
    /// Last completed day's consumption, retained for building inspection.
    pub recent_electricity: crate::simulation::economy::electricity::BuildingElectricityUsage,
    /// Realized production fraction for this hour, rebuilt by the owning production pass.
    pub hourly_economy_activity: f32,
    /// Aggregate utility power service units consumed from this building during the current day.
    ///
    /// Daily settlement derives this from citywide power demand and the plant's share of total
    /// produced power.
    pub daily_power_served_units: f32,
    /// Aggregate utility power service units produced during the last completed day.
    ///
    /// Building summaries and inspectors read this after the daily economy reset has cleared the
    /// live accumulator for the next day.
    pub recent_power_service_units: f32,
    /// Aggregate utility power service units consumed from this building last completed day.
    ///
    /// The inspector uses this with [`Self::recent_power_service_units`] to show used vs unused
    /// production.
    pub recent_power_served_units: f32,
    /// Most recently completed day's household sales revenue.
    ///
    /// Commercial staffing and input targets use this as a cheap demand signal instead of
    /// assuming every shop should immediately operate at full authored capacity.
    pub recent_household_sales_value: f32,
    /// Workers paid since the building's last daily phase; at most its funded jobs are paid in
    /// one 24-hour window (`SIM-02-G`).
    pub paid_workers_in_window: u32,
    /// Funded live jobs from the latest hourly census, which wages at each worker's daily phase
    /// read. Copied and saved so a reload does not depend on a rebuilt census (`SIM-02-G`).
    pub funded_job_capacity: u32,
    /// Wages left unpaid since the building's last daily phase because it had no cash even
    /// after selling output: a private business that cannot meet its payroll is insolvent, as
    /// it was when the midnight payroll ran before the bills (`SIM-02-G`).
    pub unpaid_cash_wages_in_window: u32,
    /// Runtime-only activity cap of a store, service store or explicit work area: its funded
    /// share of full output from the latest job-funding census (`DEM-02`).
    ///
    /// The building's hourly phase sets it from [`Self::funded_activity`] before production,
    /// and it holds until the next phase, so it is saved (`SIM-02-F`, `SIM-02-I`).
    pub commercial_activity_floor_scale: f32,
    /// Funded share of full output in the latest hourly census, copied like
    /// [`Self::funded_job_capacity`] and saved, so the activity floor set at each hourly phase
    /// never reads a census rebuilt on load (`SIM-02-I`).
    pub funded_activity: f32,
    /// Throughput a service store recorded at its last hourly phase; zero for any other
    /// building. At its next phase the store draws its share of the service-sales pool by the
    /// capacity this gives under [`Self::service_sales_profile_runtime_id`] (`SIM-02-I`).
    pub service_sales_capacity: f32,
    /// Economy profile [`Self::service_sales_capacity`] was recorded under.
    pub service_sales_profile_runtime_id: u16,
    /// Cached scale from an explicit player-drawn production area to authored full-area capacity.
    ///
    /// `1.0` means one hectare of output and profile-defined worker density. Explicit farms and
    /// extractors start at `0.0` until their nearby field or extraction polygon is committed.
    /// A depleted extractor returns to `0.0`; its committed polygon remains in the extraction system.
    pub work_area_scale: f32,
    /// True when the current painted zoning profile is incompatible and the building is waiting
    /// for the rezoning grace timer to expire.
    pub pending_redevelopment: bool,
    /// Remaining deterministic daily grace before incompatible rezoning forces removal.
    pub rezone_grace_days_remaining: u8,
}

// Every field but the asset id and the inventory is `Copy`, so a new field is copied without
// listing it here; a new owned field fails to compile until it is cloned too.
impl Clone for Building {
    fn clone(&self) -> Self {
        Self {
            asset_id: self.asset_id.clone(),
            resource_inventory: self.resource_inventory.clone(),
            ..*self
        }
    }

    // Reuses this building's string and inventory storage, so copies into retained rows (the
    // census input copy, `SIM-02-I`) allocate nothing once grown.
    fn clone_from(&mut self, source: &Self) {
        let mut asset_id = std::mem::take(&mut self.asset_id);
        let mut resource_inventory = std::mem::take(&mut self.resource_inventory);
        asset_id.clone_from(&source.asset_id);
        resource_inventory.clone_from(&source.resource_inventory);
        *self = Self {
            asset_id,
            resource_inventory,
            ..*source
        };
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BuildingEntrance {
    pub edge_idx: usize,
    pub side: i8,
    pub vehicle_frontage_access: VehicleFrontageAccess,
    pub entrance_s_m: f32,
    pub door_pos: Vector2,
    pub curb_pos: Vector2,
    pub foot_lane_fwd: usize,
    pub foot_lane_bkw: usize,
    pub car_lane_fwd: usize,
    pub car_lane_bkw: usize,
    pub flags: u8,
}

impl Default for BuildingEntrance {
    fn default() -> Self {
        Self {
            edge_idx: usize::MAX,
            side: 0,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
            entrance_s_m: 0.0,
            door_pos: Vector2::ZERO,
            curb_pos: Vector2::ZERO,
            foot_lane_fwd: usize::MAX,
            foot_lane_bkw: usize::MAX,
            car_lane_fwd: usize::MAX,
            car_lane_bkw: usize::MAX,
            flags: 0,
        }
    }
}

/// Building rows a copy of the store must refresh: the rows named by building revision steps,
/// or every row after a step that could not name them. Unlike the bounded revision log, it is
/// kept until its one consumer drains it, so draining never depends on how much history a log
/// retained. Bounded by the store: more rows than buildings collapse into `all`.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RowChanges {
    /// Rows changed since the last drain, in recording order, possibly repeated.
    pub(crate) rows: Vec<usize>,
    /// Whether some change could not name its rows.
    pub(crate) all: bool,
}

impl RowChanges {
    // Adds `rows` of a store of `buildings` rows. O(rows), amortized allocation-free.
    fn record(&mut self, rows: impl IntoIterator<Item = usize>, buildings: usize) {
        if self.all {
            return;
        }
        self.rows.extend(rows);
        if self.rows.len() > buildings.saturating_add(2) {
            self.mark_all();
        }
    }

    fn mark_all(&mut self) {
        self.all = true;
        self.rows.clear();
    }
}

/// Treasury purchases of city-service inputs over one calendar day, posted when a purchase is
/// paid or refunded (`SIM-02-G`). O(1) per posting.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct CityInputDay {
    /// Net purchases by every city service.
    pub(crate) total: f64,
    /// The power plants' share.
    pub(crate) power: f64,
}

/// Manages the full lifecycle of [`Building`]s.
#[derive(Clone)]
pub struct BuildingAllocator {
    /// All currently placed buildings.
    pub buildings: Vec<Building>,
    /// Stable handles of the dense buildings; saved with the store.
    pub(crate) building_handles: HandleTable<Building>,
    /// Set to `true` when the building list changes, signalling renderers to refresh.
    pub dirty: bool,
    /// Per-edge frontage occupancy tracker.
    pub edge_occupancy: HashMap<usize, EdgeOccupancy>,
    /// Inverted index in residential, commercial, industrial order (`BASELINE_PRIVATE_ZONES`).
    pub zone_index: [Vec<usize>; 3],
    /// Inverted vacancy index for shipped residential/commercial/industrial buildings.
    pub vacancy_index: [Vec<usize>; 3],
    /// Position of each building in its respective `vacancy_index` list for O(1) removal.
    pub vacancy_pos: Vec<usize>,
    /// Residential vacancies ordered for household admission, rebuilt with `vacancy_index`.
    pub(crate) admission_order: admission_order::AdmissionOrder,
    /// Coarse 512 m chunk index of all building centers for bounded rendering/site/economy queries.
    pub building_chunks: HashMap<(i32, i32), Vec<usize>>,
    /// Inclusive occupied chunk bounds, maintained with the existing center index.
    pub(crate) building_chunk_bounds: Option<[i32; 4]>,
    /// Highest indexed support plane, for conservative directional-shadow caster queries.
    pub(crate) max_building_support_m: f32,
    /// Indexed buildings whose support equals [`Self::max_building_support_m`], so a kept
    /// index's removal lowers it only when it takes the last (`SIM-02-K`).
    pub(crate) max_building_support_count: usize,
    /// Derived full-field reservations rebuilt from AgricultureSystem on load and undo.
    pub(crate) field_clearance: FieldClearanceIndex,
    /// Ordered swap removals awaiting production-site owner remapping in SimCore.
    pub(crate) pending_production_site_removals: Vec<(usize, usize)>,
    /// Slot in progress on the simulation clock, set by SimCore; construction deadlines count
    /// from it.
    pub(crate) now_slot: u64,
    /// Next value of the city applicant-seed counter, without [`CITY_APPLICANT_SEEDS`]. Saved.
    pub(crate) next_applicant_seed: u64,
    /// Construction deadlines set since SimCore last scheduled their completion events, in
    /// placement order.
    pub(crate) pending_construction: Vec<(Handle<Building>, u64)>,
    /// Removals queued by the hour's actions slot (planned clearance) and by the buildings'
    /// daily phases (expired compatibility), oldest first, until the `building removals` queue
    /// serves them in the hour's removal window (`SIM-02-J`). Saved.
    pub(crate) removal_requests: std::collections::VecDeque<BuildingRemovalRequest>,
    /// The `building removals` queue's service this hour, logged at its last slot.
    pub(crate) removal_stats: crate::simulation::core::request_queue::QueueStats,
    /// Treasury purchases of city-service inputs during the calendar day so far (`SIM-02-G`).
    pub(crate) city_input_day: CityInputDay,
    /// Purchases of the last closed calendar day, read by its budget report.
    pub(crate) last_city_input_day: CityInputDay,
    /// Yard hedges to lay and remove, in the order the buildings were placed and removed.
    pub(crate) pending_yards: Vec<yard::YardEvent>,
    /// Recent in-place level changes, read by the renderer to animate the replacement.
    pub(crate) level_changes: LevelChangeLog,
    /// Maximum half-diagonal of placed lots in zoning cells, rebuilt with [`Self::building_chunks`].
    pub(crate) max_lot_radius_cells: f32,
    /// Indexed buildings whose lot radius equals [`Self::max_lot_radius_cells`] (`SIM-02-K`).
    pub(crate) max_lot_radius_count: usize,
    /// Explicit non-parcel sites in the existing building index; invalid when `dirty_index`.
    pub(crate) explicit_cell_site_count: usize,
    /// Maximum support-footprint distance from its indexed lot center, in world metres.
    pub(crate) max_site_radius_m: f32,
    /// Number of site records attaining the exact current maximum radius.
    pub(crate) max_site_radius_count: usize,
    /// Recalculates inverted indices if true.
    pub dirty_index: bool,
    /// Per-family dirty flags in residential, commercial, industrial order (`BASELINE_PRIVATE_ZONES`).
    pub dirty_zones: [bool; 3],
    /// True when the derived entrance cache must be rebuilt before use.
    pub(crate) entrances_dirty: bool,
    /// Revision bumped whenever building indices may have become stale for external systems.
    pub(crate) building_ref_revision: u64,
    // Building indices changed at each `building_ref_revision` step, for incremental tables.
    building_changes: crate::simulation::core::change_log::IndexChangeLog,
    /// The same steps for the census input copy (`SIM-02-I`), kept until the copy drains them;
    /// saved with the copy, unlike the bounded log above.
    pub(crate) census_copy_changes: RowChanges,
    production_revision: u64,
    production_changes: crate::simulation::core::change_log::IndexChangeLog,
    /// Appearance-only revision; does not invalidate routing or entrance references.
    pub(crate) building_visual_revision: u64,
    /// Revision bumped whenever derived building entrance/access data changes.
    pub(crate) entrance_ref_revision: u64,
    /// Derived building entrance/access cache keyed by building index.
    pub(crate) entrances: Vec<BuildingEntrance>,
    /// Derived flat building-site clients keyed by building index.
    pub(crate) building_sites: Vec<BuildingSiteClient>,
    /// Shared terrain-change outbox for placement, redevelopment, and removal in every mode:
    /// one box per site change, never merged, so changes far apart refresh only the terrain
    /// patches each touches (`SIM-02-K`).
    pub(crate) building_site_dirty_bounds: Vec<(f32, f32, f32, f32)>,
    /// Separate explicit-site exclusion edits; ordinary parcel buildings do not block cells.
    /// Keeping envelopes separate avoids treating untouched land between sites as edited.
    pub(crate) cell_site_dirty_bounds: Vec<(f32, f32, f32, f32)>,
    // Geometry-only feasibility, independent of demand and dropped on allocator snapshots.
    site_feasibility: placement::SiteFeasibilityCache,
    // Admission's border arrival sweeps, kept per routing revision; derived, never saved.
    border_arrival: lifecycle::BorderArrivalCosts,
    // The zone-index rebuild's reused storage.
    index_scratch: index::IndexScratch,
    // What a zone index kept across the removal queue's swap-removes still owes (`SIM-02-K`).
    kept_index: index::KeptIndex,
    /// Registry of all loaded pack assets.
    pub registry: AssetRegistry,
}

pub(crate) use site::{BuildingSiteClient, BuildingSiteTerrainSnapshot, NearbySites};

/// Derived runtime binding from an asset-side `economy_profile` reference.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct EconomyProfileBinding {
    /// Compact runtime profile id, or `0` when no runtime profile is bound.
    pub runtime_id: u16,
    /// True when the asset referenced an economy profile that the live runtime cannot execute.
    pub economy_broken: bool,
}

/// Bounded record of in-place level changes for presentation only.
///
/// Keys are `(parcel_id, build_generation)`, which a level change keeps. Readers remember the
/// running total they last saw; only the newest [`Self::CAPACITY`] keys are retained, so a
/// headless run without a reader stays bounded and a reader that falls far behind merely skips
/// animations. Never saved.
#[derive(Debug, Clone, Default)]
pub(crate) struct LevelChangeLog {
    keys: VecDeque<(u64, u32)>,
    total: u64,
}

impl LevelChangeLog {
    const CAPACITY: usize = 4096;

    pub(crate) fn record(&mut self, key: (u64, u32)) {
        if self.keys.len() == Self::CAPACITY {
            self.keys.pop_front();
        }
        self.keys.push_back(key);
        self.total += 1;
    }

    /// Returns the running total and the retained keys recorded after `seen`. O(new keys).
    pub(crate) fn since(&self, seen: u64) -> (u64, impl Iterator<Item = &(u64, u32)>) {
        let new = self.total.saturating_sub(seen).min(self.keys.len() as u64) as usize;
        (self.total, self.keys.iter().skip(self.keys.len() - new))
    }

    // The total stays monotonic so a reader never mistakes later changes for seen ones.
    fn clear(&mut self) {
        self.keys.clear();
    }
}

impl Building {
    /// Stable key selecting this instance's authored colour scheme.
    ///
    /// Keyed on the claimed parcel and its redevelopment generation, never on the allocator
    /// ordinal: [`BuildingAllocator`] swap-removes and remaps buildings, so an ordinal-keyed
    /// scheme would recolour surviving neighbours whenever one building was demolished. The
    /// generation is what makes a rebuilt plot draw a fresh colour instead of repeating the
    /// one the player demolished. Explicit sites hold no parcel, so they fall back to their
    /// placed centre, which is fixed at placement and round-trips through saves.
    pub fn appearance_key(&self) -> u64 {
        if self.parcel_id != 0 {
            return placement::stable_parcel_selection_hash(
                self.zone_profile_runtime_id,
                self.parcel_id,
                self.build_generation,
                "appearance",
            );
        }
        // Millimetre quantisation keeps the key independent of float formatting while
        // staying far finer than the smallest distance between two placed footprints.
        let x = (self.center_x * 1000.0).round() as i64 as u64;
        let z = (self.center_y * 1000.0).round() as i64 as u64;
        placement::stable_parcel_selection_hash(
            self.zone_profile_runtime_id,
            x ^ z.rotate_left(32),
            self.build_generation,
            "appearance.explicit",
        )
    }

    // Level changes keep the parcel, generation and zoning profile, so the key is stable
    // across upgrades and downgrades of the same standing building.
    fn level_change_key(&self) -> u64 {
        placement::stable_parcel_selection_hash(
            self.zone_profile_runtime_id,
            self.parcel_id,
            self.build_generation,
            "level_change",
        )
    }

    /// Returns true while the building exists only as a construction site.
    pub(crate) fn is_under_construction(&self) -> bool {
        self.construction_complete_slot != 0
    }

    /// Returns true when the building can participate in household, labor, and economy flows.
    pub(crate) fn is_operational(&self) -> bool {
        !self.is_under_construction()
    }

    /// Updates the cached explicit production-area scale used by economy hot paths.
    pub(crate) fn set_work_area_scale(&mut self, scale: f32) {
        self.work_area_scale = crate::simulation::work_area::sanitize_work_area_scale(scale);
    }

    /// Returns `0.0..=1.0` construction progress at the continuous slot clock `now_slot`.
    pub(crate) fn construction_progress(&self, now_slot: f64) -> f32 {
        if !self.is_under_construction() || self.construction_total_hours == 0 {
            return 1.0;
        }
        let duration = f64::from(self.construction_total_hours) * SLOTS_PER_HOUR as f64;
        let start = self.construction_complete_slot as f64 - duration;
        ((now_slot - start) / duration).clamp(0.0, 1.0) as f32
    }

    /// Puts the building under construction with `hours` left at slot 0; `0` completes it.
    #[cfg(test)]
    pub(crate) fn set_construction_hours_left(&mut self, hours: u16) {
        self.construction_complete_slot = u64::from(hours) * SLOTS_PER_HOUR;
    }

    /// Hour boundaries from the hour holding `now_slot` to the first one at or after
    /// completion: the hour by which the building is ready, as the workforce calendar counts
    /// it. At an hour boundary this is the time left rounded up; `0` once complete.
    pub(crate) fn construction_hours_left(&self, now_slot: u64) -> u16 {
        let ready_hour = self.construction_complete_slot.div_ceil(SLOTS_PER_HOUR);
        u16::try_from(ready_hour.saturating_sub(now_slot / SLOTS_PER_HOUR)).unwrap_or(u16::MAX)
    }

    /// Returns the current inventory amount for one runtime resource.
    pub(crate) fn inventory_units(&self, resource_runtime_id: ResourceRuntimeId) -> f32 {
        if resource_runtime_id == 0 {
            return 0.0;
        }
        self.resource_inventory
            .get(resource_runtime_id as usize - 1)
            .copied()
            .unwrap_or(0.0)
    }

    /// Sets the current inventory amount for one runtime resource.
    pub(crate) fn set_inventory_units(
        &mut self,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        if resource_runtime_id == 0 {
            return;
        }
        let slot = resource_runtime_id as usize - 1;
        if self.resource_inventory.len() <= slot {
            self.resource_inventory.resize(slot + 1, 0.0);
        }
        self.resource_inventory[slot] = amount.max(0.0);
    }

    /// Adds one amount to the current inventory for one runtime resource.
    pub(crate) fn add_inventory_units(
        &mut self,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        let current = self.inventory_units(resource_runtime_id);
        self.set_inventory_units(resource_runtime_id, current + amount);
    }

    /// Removes up to one amount from the current inventory for one runtime resource.
    pub(crate) fn remove_inventory_units(
        &mut self,
        resource_runtime_id: ResourceRuntimeId,
        amount: f32,
    ) {
        let current = self.inventory_units(resource_runtime_id);
        self.set_inventory_units(resource_runtime_id, (current - amount).max(0.0));
    }

    /// Drops any inventory slots not referenced by the resolved economy profile.
    pub(crate) fn retain_inventory_for_profile(
        &mut self,
        profile: Option<&EconomyProfileRuntime>,
        resource_count: usize,
    ) {
        let mut retained = vec![0.0; resource_count];
        if let Some(profile) = profile {
            for port in profile.inputs.iter().chain(profile.outputs.iter()) {
                let slot = port.resource_runtime_id as usize - 1;
                if slot < self.resource_inventory.len() && slot < retained.len() {
                    retained[slot] = self.resource_inventory[slot];
                }
            }
        }
        self.resource_inventory = retained;
    }
}

/// Resolves the live runtime economy-profile binding for one asset id.
pub(crate) fn resolve_building_economy_profile_binding(
    registry: &AssetRegistry,
    asset_id: &str,
) -> EconomyProfileBinding {
    let Some(profile_id) = registry.economy_profile(asset_id) else {
        return EconomyProfileBinding::default();
    };
    let catalog = match load_runtime_economy_catalog() {
        Ok(catalog) => catalog,
        Err(err) => {
            debug_log!(
                "economy",
                "asset_id={} economy profile '{}' could not be resolved because the runtime catalog failed to load: {}",
                asset_id,
                profile_id,
                err
            );
            return EconomyProfileBinding {
                runtime_id: 0,
                economy_broken: true,
            };
        }
    };
    resolve_building_economy_profile_binding_with_catalog(registry, catalog.as_ref(), asset_id)
}

/// Resolves an asset's economy profile using a caller-owned runtime catalog.
pub(crate) fn resolve_building_economy_profile_binding_with_catalog(
    registry: &AssetRegistry,
    catalog: &RuntimeEconomyCatalog,
    asset_id: &str,
) -> EconomyProfileBinding {
    let Some(profile_id) = registry.economy_profile(asset_id) else {
        return EconomyProfileBinding::default();
    };
    let Some(profile) = catalog.profile_for_id(profile_id) else {
        debug_log!(
            "economy",
            "asset_id={} references missing economy profile '{}'; building will run economy-broken",
            asset_id,
            profile_id
        );
        return EconomyProfileBinding {
            runtime_id: 0,
            economy_broken: true,
        };
    };
    if !profile.runtime_supported {
        debug_log!(
            "economy",
            "asset_id={} references unsupported runtime economy profile '{}'; building will run economy-broken",
            asset_id,
            profile_id
        );
        return EconomyProfileBinding {
            runtime_id: 0,
            economy_broken: true,
        };
    }
    EconomyProfileBinding {
        runtime_id: profile.runtime_id,
        economy_broken: false,
    }
}

/// Tracks which frontage columns along a road edge are claimed by placed buildings.
#[derive(Clone)]
pub struct EdgeOccupancy {
    /// Number of columns along this road edge.
    pub cells_long: usize,
    /// True if a building has its frontage in this column on the left side.
    pub left: Vec<bool>,
    /// True if a building has its frontage in this column on the right side.
    pub right: Vec<bool>,
}

/// Converts an asset-manifest [`ZoneClass`] to the matching simulation [`ZoneType`].
pub(crate) fn zone_class_to_zone_type(zone: ZoneClass) -> ZoneType {
    match zone {
        ZoneClass::Residential => ZoneType::Residential,
        ZoneClass::Commercial => ZoneType::Commercial,
        ZoneClass::Industrial => ZoneType::Industrial,
        ZoneClass::Office => ZoneType::Office,
        ZoneClass::Mixed => ZoneType::Mixed,
    }
}

/// Converts a simulation [`ZoneType`] back to the authored [`ZoneClass`] when one exists.
pub(crate) fn zone_type_to_zone_class(zone: ZoneType) -> Option<ZoneClass> {
    match zone {
        ZoneType::Residential => Some(ZoneClass::Residential),
        ZoneType::Commercial => Some(ZoneClass::Commercial),
        ZoneType::Industrial => Some(ZoneClass::Industrial),
        ZoneType::Office => Some(ZoneClass::Office),
        ZoneType::Mixed => Some(ZoneClass::Mixed),
        ZoneType::None => None,
    }
}

impl Default for BuildingAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl BuildingAllocator {
    fn record_production_site_removal(&mut self, removed: usize, last: usize) {
        if [removed, last].iter().any(|&idx| {
            self.registry
                .is_industry_area_asset(&self.buildings[idx].asset_id)
        }) {
            self.pending_production_site_removals.push((removed, last));
        }
    }

    /// Creates an empty allocator whose clock is at `now_slot`.
    pub(crate) fn new_at_slot(now_slot: u64) -> Self {
        Self {
            now_slot,
            ..Self::new()
        }
    }

    /// Creates an empty allocator.
    pub fn new() -> Self {
        Self {
            buildings: Vec::new(),
            building_handles: HandleTable::default(),
            dirty: false,
            edge_occupancy: HashMap::new(),
            zone_index: [const { Vec::new() }; 3],
            vacancy_index: [const { Vec::new() }; 3],
            vacancy_pos: Vec::new(),
            admission_order: Default::default(),
            building_chunks: HashMap::new(),
            building_chunk_bounds: None,
            max_building_support_m: f32::NEG_INFINITY,
            max_building_support_count: 0,
            field_clearance: FieldClearanceIndex::default(),
            pending_production_site_removals: Vec::new(),
            now_slot: 0,
            next_applicant_seed: 0,
            pending_construction: Vec::new(),
            removal_requests: std::collections::VecDeque::new(),
            removal_stats: Default::default(),
            city_input_day: CityInputDay::default(),
            last_city_input_day: CityInputDay::default(),
            pending_yards: Vec::new(),
            level_changes: LevelChangeLog::default(),
            max_lot_radius_cells: 0.0,
            max_lot_radius_count: 0,
            explicit_cell_site_count: 0,
            max_site_radius_m: 0.0,
            max_site_radius_count: 0,
            dirty_index: true,
            dirty_zones: [false; 3],
            entrances_dirty: false,
            building_ref_revision: 0,
            building_changes: Default::default(),
            census_copy_changes: RowChanges::default(),
            production_revision: 0,
            production_changes: Default::default(),
            building_visual_revision: 0,
            entrance_ref_revision: 0,
            entrances: Vec::new(),
            building_sites: Vec::new(),
            building_site_dirty_bounds: Vec::new(),
            cell_site_dirty_bounds: Vec::new(),
            site_feasibility: Default::default(),
            border_arrival: Default::default(),
            index_scratch: Default::default(),
            kept_index: Default::default(),
            registry: AssetRegistry::new(),
        }
    }

    /// Maintains building legality and derived caches after daily updates or immediate edits.
    ///
    /// Pass one elapsed day for daily settlement, or zero for an edit that does not advance time.
    /// Newly detected incompatible zoning starts its full grace period in either case.
    #[allow(clippy::too_many_arguments)]
    pub fn maintain(
        &mut self,
        elapsed_days: u8,
        zoning: &mut ZoningSystem,
        agents: &mut crate::simulation::economy::agents::AgentSystem,
        households: &mut crate::simulation::economy::households::HouseholdSystem,
        logistics: &mut crate::simulation::economy::logistics::ShipmentSystem,
        treasury_balance: &mut f64,
        network: &mut crate::simulation::network::TransitNetwork,
        graph: &mut RegionGraph,
    ) {
        // 1. Stale building cleanup.
        self.cleanup_stale_buildings(
            elapsed_days,
            zoning,
            agents,
            households,
            logistics,
            treasury_balance,
            graph,
            &network.lane_system,
        );

        self.repair_derived_caches(zoning, network, graph);
        self.dirty = false;
    }

    /// Repairs derived state after daily phases removed buildings (`SIM-02-G`): the building
    /// revision, then the caches [`Self::maintain`] repairs. O(buildings), only in a slot that
    /// removed one.
    pub(crate) fn finish_phase_removals(
        &mut self,
        zoning: &ZoningSystem,
        network: &mut crate::simulation::network::TransitNetwork,
        graph: &mut RegionGraph,
    ) {
        self.bump_building_ref_revision();
        self.entrances_dirty = true;
        self.repair_derived_caches(zoning, network, graph);
    }

    fn repair_derived_caches(
        &mut self,
        zoning: &ZoningSystem,
        network: &mut crate::simulation::network::TransitNetwork,
        graph: &mut RegionGraph,
    ) {
        network.rebuild_pathing_if_dirty(graph);

        if self.entrances_dirty || self.entrances.len() != self.buildings.len() {
            self.rebuild_entrance_cache(graph, &network.lane_system);
        }
        if self.building_sites.len() != self.buildings.len() {
            self.rebuild_building_site_clients(zoning.config.zone_cell_m);
        }

        if self.dirty_index {
            self.rebuild_zone_index();
        }
    }

    #[cfg(test)]
    pub(crate) fn execute_demand_household_admission(
        &mut self,
        households_to_admit_today: u32,
        agents: &mut crate::simulation::economy::agents::AgentSystem,
        transit_network: &crate::simulation::network::TransitNetwork,
        graph: &RegionGraph,
    ) -> u32 {
        self.execute_demand_household_admission_with_income(
            households_to_admit_today,
            0,
            agents,
            transit_network,
            graph,
            |_, _, _| true,
        )
    }

    /// Tries a bounded deterministic applicant batch, claiming income before reserving homes.
    pub(crate) fn execute_demand_household_admission_with_income(
        &mut self,
        households_to_admit_today: u32,
        next_household_id: usize,
        agents: &mut crate::simulation::economy::agents::AgentSystem,
        transit_network: &crate::simulation::network::TransitNetwork,
        graph: &RegionGraph,
        claim_income: impl FnMut(
            crate::simulation::economy::agents::HouseholdAgeComposition,
            u16,
            u8,
        ) -> bool,
    ) -> u32 {
        self.admit_households_from_demand(
            households_to_admit_today as usize,
            next_household_id,
            transit_network,
            graph,
            claim_income,
            |home, size, composition, border_node| {
                let border_pos = graph.node(border_node).pos;
                agents.spawn_household_arrival_carrier(
                    home,
                    size,
                    composition,
                    border_node,
                    border_pos.x,
                    border_pos.z,
                );
            },
        ) as u32
    }

    /// Decides a bounded applicant batch as [`Self::execute_demand_household_admission_with_income`]
    /// does, claiming each accepted family's home, without launching carriers: each decision
    /// joins `admitted` as `(home, size, composition, border)` for the admission queue, which
    /// creates the carriers over the hour (`SIM-02-J`). Returns how many were admitted.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn decide_demand_household_admission(
        &mut self,
        households_to_admit_today: u32,
        next_household_id: usize,
        transit_network: &crate::simulation::network::TransitNetwork,
        graph: &RegionGraph,
        claim_income: impl FnMut(
            crate::simulation::economy::agents::HouseholdAgeComposition,
            u16,
            u8,
        ) -> bool,
        admitted: &mut Vec<(
            usize,
            u16,
            crate::simulation::economy::agents::HouseholdAgeComposition,
            u32,
        )>,
    ) -> u32 {
        self.admit_households_from_demand(
            households_to_admit_today as usize,
            next_household_id,
            transit_network,
            graph,
            claim_income,
            |home, size, composition, border_node| {
                admitted.push((home, size, composition, border_node));
            },
        ) as u32
    }

    /// Slot at which construction of `hours` operational hours started now completes; `0`
    /// (complete) for none.
    pub(crate) fn construction_complete_slot_after(&self, hours: u16) -> u64 {
        if hours == 0 {
            0
        } else {
            self.now_slot + u64::from(hours) * SLOTS_PER_HOUR
        }
    }

    /// Finishes construction at its deadline (`SIM-02-D`): the building becomes live economy
    /// capacity and joins clean indices in place. O(log B) plus one sorted insert; a stale
    /// index is left for its next reader to rebuild.
    pub(crate) fn complete_construction(&mut self, building_idx: usize) {
        let building = &mut self.buildings[building_idx];
        if !building.is_under_construction() {
            return;
        }
        building.construction_complete_slot = 0;
        building.construction_total_hours = 0;
        debug_log!(
            "economy",
            "building construction complete: asset={} zone={:?} level={}",
            building.asset_id,
            building.zone_type,
            building.level
        );
        if let Some(zone_idx) = baseline_private_zone_slot(building.zone_type) {
            self.dirty_zones[zone_idx] = true;
        }
        self.building_visual_revision = self.building_visual_revision.wrapping_add(1);
        self.record_production_change(building_idx);
        self.dirty = true;
        if !self.index_completed_building(building_idx) {
            self.mark_index_stale();
        }
    }

    /// Completes every construction site at once and rebuilds the indices, for fixtures that
    /// need a finished city. O(B).
    pub(crate) fn complete_all_construction(&mut self) {
        for building in &mut self.buildings {
            if building.is_under_construction() {
                building.construction_complete_slot = 0;
                building.construction_total_hours = 0;
                if let Some(zone_idx) = baseline_private_zone_slot(building.zone_type) {
                    self.dirty_zones[zone_idx] = true;
                }
            }
        }
        self.pending_construction.clear();
        self.building_visual_revision = self.building_visual_revision.wrapping_add(1);
        self.invalidate_production_changes();
        self.dirty = true;
        self.rebuild_zone_index();
    }

    /// Removes all buildings and resets the dirty flag.
    pub fn clear(&mut self) {
        let had_buildings = !self.buildings.is_empty();
        let had_entrances = !self.entrances.is_empty();
        let had_sites = !self.building_sites.is_empty();
        self.site_feasibility = placement::SiteFeasibilityCache::default();
        self.buildings.clear();
        self.building_handles.clear();
        self.field_clearance.clear();
        self.pending_production_site_removals.clear();
        self.pending_construction.clear();
        self.removal_requests.clear();
        self.pending_yards.clear();
        self.level_changes.clear();
        self.building_sites.clear();
        self.edge_occupancy.clear();
        for list in &mut self.zone_index {
            list.clear();
        }
        for list in &mut self.vacancy_index {
            list.clear();
        }
        self.vacancy_pos.clear();
        self.clear_admission_order();
        self.building_chunks.clear();
        self.building_chunk_bounds = None;
        self.max_building_support_m = f32::NEG_INFINITY;
        self.max_building_support_count = 0;
        self.max_lot_radius_cells = 0.0;
        self.max_lot_radius_count = 0;
        self.explicit_cell_site_count = 0;
        self.max_site_radius_m = 0.0;
        self.max_site_radius_count = 0;
        self.building_site_dirty_bounds.clear();
        self.cell_site_dirty_bounds.clear();
        self.dirty = false;
        self.dirty_index = false;
        self.kept_index = Default::default();
        self.entrances.clear();
        self.entrances_dirty = false;
        if had_buildings || had_sites {
            self.bump_building_ref_revision();
        }
        if had_entrances {
            self.bump_entrance_ref_revision();
        }
    }

    /// Returns the current building-reference revision observed by dependent systems.
    pub(crate) fn building_ref_revision(&self) -> u64 {
        self.building_ref_revision
    }

    /// Returns the current derived entrance-reference revision observed by dependent systems.
    pub(crate) fn entrance_ref_revision(&self) -> u64 {
        self.entrance_ref_revision
    }

    /// Advances the building revision for changes whose rows are unknown; incremental
    /// building-indexed tables rebuild in full.
    pub(crate) fn bump_building_ref_revision(&mut self) {
        self.invalidate_production_changes();
        self.building_ref_revision = self.building_ref_revision.wrapping_add(1);
        self.building_changes.clear(self.building_ref_revision);
        self.census_copy_changes.mark_all();
    }

    // Revision step whose only changed building rows are `indices`, including removed rows.
    fn record_building_changes(&mut self, indices: impl IntoIterator<Item = usize> + Clone) {
        self.invalidate_production_changes();
        self.building_ref_revision = self.building_ref_revision.wrapping_add(1);
        self.building_changes
            .record(self.building_ref_revision, indices.clone());
        self.census_copy_changes
            .record(indices, self.buildings.len());
    }

    /// Writes the building rows changed since `since` into `out`, sorted and deduplicated.
    /// Returns false when some change in between could not name its rows.
    pub(crate) fn building_changes_since(&self, since: u64, out: &mut Vec<usize>) -> bool {
        self.building_changes
            .changes_since(since, self.building_ref_revision, out)
    }

    /// Advances the derived entrance-reference revision.
    pub(crate) fn bump_entrance_ref_revision(&mut self) {
        self.entrance_ref_revision = self.entrance_ref_revision.wrapping_add(1);
    }

    /// Returns the asset this building would upgrade to, if any.
    ///
    /// Family members follow their `asset_set`; other zoned buildings pick one unfamilied asset
    /// by a stable per-site key, so planning and execution agree on the same target.
    pub(crate) fn upgrade_target(&self, building: &Building) -> Option<&str> {
        self.registry
            .next_level(&building.asset_id, building.level_change_key())
    }

    /// Returns the asset this building would downgrade to, if any. See [`Self::upgrade_target`].
    pub(crate) fn downgrade_target(&self, building: &Building) -> Option<&str> {
        self.registry
            .prev_level(&building.asset_id, building.level_change_key())
    }

    /// Returns usable household slots: one for farms, otherwise the asset's declared capacity.
    ///
    /// Unresolved assets or undeclared capacities count as zero.
    pub fn household_capacity(&self, building_idx: usize) -> u32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0;
        };
        if b.broken || b.economy_broken || b.is_deserted || b.is_under_construction() {
            return 0;
        }
        self.registry.household_capacity(&b.asset_id)
    }

    /// Rolls building `building_idx`'s daily accumulators at its daily phase (`SIM-02-G`): the
    /// finished 24-hour window becomes its `recent_*` values and the next window starts empty.
    /// O(1).
    pub(crate) fn roll_daily_accumulators(&mut self, building_idx: usize) {
        let building = &mut self.buildings[building_idx];
        building.daily_owa_input_value = 0.0;
        building.daily_local_input_value = 0.0;
        building.daily_city_funded_input_cost = 0.0;
        building.recent_household_sales_value = building.daily_household_sales_value.max(0.0);
        building.daily_household_sales_value = 0.0;
        building.recent_power_service_units = building.daily_power_service_units.max(0.0);
        building.daily_power_service_units = 0.0;
        building.recent_electricity = std::mem::take(&mut building.daily_electricity);
        building.recent_power_served_units = building.daily_power_served_units.max(0.0);
        building.daily_power_served_units = 0.0;
        building.paid_workers_in_window = 0;
        building.unpaid_cash_wages_in_window = 0;
    }

    /// Rolls every building's daily accumulators at once, as their daily phases would.
    #[cfg(test)]
    pub(crate) fn reset_daily_input_accumulators(&mut self) {
        for building_idx in 0..self.buildings.len() {
            self.roll_daily_accumulators(building_idx);
        }
    }

    /// Posts a city service's input purchase (negative for a refund) into the calendar day's
    /// totals, independent of the building's own window (`SIM-02-G`). O(1).
    pub(crate) fn record_city_input_purchase(&mut self, building_idx: usize, amount: f64) {
        let power = self.buildings.get(building_idx).is_some_and(|building| {
            load_runtime_economy_catalog().is_ok_and(|catalog| {
                catalog
                    .profile_by_runtime_id(building.economy_profile_runtime_id)
                    .is_some_and(|profile| profile.utility_service.as_deref() == Some("power"))
            })
        });
        self.city_input_day.total += amount;
        if power {
            self.city_input_day.power += amount;
        }
    }

    /// Ends the calendar day's city input purchases for its budget report. O(1).
    pub(crate) fn close_city_input_day(&mut self) {
        self.last_city_input_day = std::mem::take(&mut self.city_input_day);
    }

    /// Returns the target floor area per household for a building.
    pub fn flat_size_m2(&self, building_idx: usize) -> f32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0.0;
        };
        self.registry.flat_size_m2(&b.asset_id)
    }

    /// The applicant-stream seed of building `building_idx` ([`Building::applicant_seed`]),
    /// as the household applicant functions take it. O(1).
    pub(crate) fn applicant_seed(&self, building_idx: usize) -> usize {
        self.buildings[building_idx].applicant_seed as usize
    }

    /// Test fixtures that push buildings directly: seeds each building's applicants by its
    /// index, as distinct placements would give each home a stream of its own.
    #[cfg(test)]
    pub(crate) fn seed_applicants_by_index_for_test(&mut self) {
        for (index, building) in self.buildings.iter_mut().enumerate() {
            building.applicant_seed = index as u64;
        }
    }

    // The city counter's next applicant seed, for a building placed outside the demand queue.
    pub(super) fn next_city_applicant_seed(&mut self) -> u64 {
        let seed = CITY_APPLICANT_SEEDS | self.next_applicant_seed;
        self.next_applicant_seed += 1;
        seed
    }

    /// Returns the economy-profile worker capacity for an asset, failing safe on unresolved profiles.
    pub(crate) fn worker_capacity_for_asset_with_catalog(
        &self,
        asset_id: &str,
        catalog: &RuntimeEconomyCatalog,
    ) -> Option<u32> {
        if let Some(profile_id) = self.registry.economy_profile(asset_id) {
            if let Some(profile) = catalog.profile_for_id(profile_id)
                && profile.runtime_supported
            {
                return Some(profile.worker_capacity);
            }
            return None;
        }
        Some(self.registry.worker_capacity(asset_id))
    }

    /// Returns physical worker slots for a placed building, including its committed work area.
    pub(crate) fn worker_capacity_with_catalog(
        &self,
        building_idx: usize,
        catalog: &RuntimeEconomyCatalog,
    ) -> u32 {
        let Some(b) = self.buildings.get(building_idx) else {
            return 0;
        };
        if b.broken || b.economy_broken || b.is_deserted || b.is_under_construction() {
            return 0;
        }
        if let Some(profile) = catalog.profile_by_runtime_id(b.economy_profile_runtime_id)
            && profile_kind_uses_explicit_work_area(profile.kind)
        {
            return physical_worker_capacity_for_profile(b, profile);
        }
        self.worker_capacity_for_asset_with_catalog(&b.asset_id, catalog)
            .unwrap_or(0)
    }

    /// Returns whether the placed building is a city-funded explicit service building.
    pub(crate) fn is_city_service_building(&self, building: &Building) -> bool {
        self.registry.is_city_service_asset(&building.asset_id)
    }

    /// Returns a bounded nearby candidate list for the requested zones, sorted by distance.
    pub fn find_nearby_buildings_by_zones(
        &self,
        origin_x: f32,
        origin_y: f32,
        zones: &[ZoneType],
        max_chunk_radius: i32,
        candidate_limit: usize,
    ) -> Vec<usize> {
        let mut candidates = Vec::with_capacity(candidate_limit);
        self.fill_nearby_buildings_by_zones(
            origin_x,
            origin_y,
            zones,
            max_chunk_radius,
            candidate_limit,
            &mut candidates,
        );
        candidates
    }

    /// Fills a reusable nearby candidate buffer for the requested zones.
    pub fn fill_nearby_buildings_by_zones(
        &self,
        origin_x: f32,
        origin_y: f32,
        zones: &[ZoneType],
        max_chunk_radius: i32,
        candidate_limit: usize,
        candidates: &mut Vec<usize>,
    ) {
        self.fill_nearby_buildings(
            origin_x,
            origin_y,
            max_chunk_radius,
            candidate_limit,
            candidates,
            |_, building| zones.contains(&building.zone_type),
        );
    }

    /// Fills a reusable nearby candidate buffer using a caller-provided eligibility predicate.
    pub fn fill_nearby_buildings(
        &self,
        origin_x: f32,
        origin_y: f32,
        max_chunk_radius: i32,
        candidate_limit: usize,
        candidates: &mut Vec<usize>,
        mut eligible: impl FnMut(usize, &Building) -> bool,
    ) {
        candidates.clear();
        if candidate_limit == 0 {
            return;
        }
        let origin_chunk =
            RegionGraph::get_chunk_coords(godot::prelude::Vector3::new(origin_x, 0.0, origin_y));

        for ring in 0..=max_chunk_radius {
            for dx in -ring..=ring {
                for dz in -ring..=ring {
                    if ring > 0 && dx.abs() != ring && dz.abs() != ring {
                        continue;
                    }
                    let chunk_key = (origin_chunk.0 + dx, origin_chunk.1 + dz);
                    let Some(indices) = self.building_chunks.get(&chunk_key) else {
                        continue;
                    };
                    for &idx in indices {
                        if idx >= self.buildings.len() {
                            continue;
                        }
                        // Disconnected buildings remain renderable/pickable, but are
                        // not new economy candidates merely because the index covers them.
                        if self.buildings[idx].edge_idx != usize::MAX
                            && eligible(idx, &self.buildings[idx])
                        {
                            candidates.push(idx);
                        }
                    }
                }
            }
        }

        candidates.sort_unstable_by(|&a, &b| {
            let da = squared_distance(origin_x, origin_y, &self.buildings[a]);
            let db = squared_distance(origin_x, origin_y, &self.buildings[b]);
            da.total_cmp(&db).then_with(|| a.cmp(&b))
        });
        candidates.truncate(candidate_limit);
    }
}

fn squared_distance(origin_x: f32, origin_y: f32, building: &Building) -> f32 {
    let dx = building.center_x - origin_x;
    let dy = building.center_y - origin_y;
    dx * dx + dy * dy
}
