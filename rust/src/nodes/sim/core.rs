//! Background simulation thread, `SimCore` state bundle, and `RenderSnapshot`.
//!
//! `SimCore` owns all simulation state. The background thread continuously ticks
//! it at ~60 Hz, writes a `RenderSnapshot` after every tick, and never touches
//! Godot objects. The Godot main thread reads only from the snapshot for rendering
//! and locks the `Arc<Mutex<SimCore>>` briefly for mutations (road edits, etc.).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use crate::config::HEIGHT_SCALE;
use crate::debug_log;
use crate::nodes::sim::render::lane_pose::sample_lane_pose;
use crate::nodes::sim::render::water::{CachedWaterPatchMesh, WaterPatchMeshCacheKey};
use crate::nodes::sim::road_tool::RoadGhostSnapIndex;
use godot::prelude::{Vector2, Vector3, godot_error};

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::core::config::WorldConfig;
use crate::simulation::core::time::TimeSystem;
use crate::simulation::economy::agents::{
    AGE_ADULT, AGE_CHILD, AGE_ELDER, AgentSystem, MODE_CAR, TRANSIT_IN_BUILDING,
    age_group_can_work, transit_is_visible,
};
use crate::simulation::economy::definitions::load_runtime_economy_catalog;
use crate::simulation::economy::demand::{
    DemandBuildingActionPlan, DemandSpawnAction, DemandSystem,
};
use crate::simulation::economy::fiscal::FiscalRevenue;
use crate::simulation::economy::households::HouseholdSystem;
use crate::simulation::economy::logistics::ShipmentSystem;
use crate::simulation::grid::desirability::DesirabilitySystem;
use crate::simulation::grid::noise::NoiseSystem;
use crate::simulation::grid::pollution::PollutionSystem;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::lanes::{Lane, LaneType};
use crate::simulation::network::render::NetworkMeshData;
use crate::simulation::network::surface::{
    CURB_STEP_HEIGHT_M, RoadPreviewValidation, RoadSurfaceCompileReason, RoadSurfaceSystem,
};
use crate::simulation::terrain::cdt::{
    TerrainCdtError, TerrainCdtInput, TerrainCdtMesh, TerrainCdtPatch,
};
use crate::simulation::terrain::{
    TerrainPatchSnapshot, TerrainSystem, terrain_cdt_local_sample_margin_m,
};
use crate::simulation::water::WaterSystem;
use crate::simulation::world_definition::{AuthoredLakeFill, AuthoredOpenWaterFill};
use crate::simulation::zoning::{ZoneType, ZoningSystem};

const MINUTES_PER_DAY_U64: u64 = 24 * 60;
const DEMAND_SPAWN_ACTIONS_PER_MINUTE: usize = 1;

/// Demand-planned private building spawn delayed to a later authored minute.
#[derive(Clone, Debug)]
pub(crate) struct PendingDemandSpawnAction {
    /// Absolute authored minute when this spawn may be released.
    pub(crate) due_minute: u64,
    /// Demand bucket that selected this spawn.
    pub(crate) zone_type: ZoneType,
    /// Final parcel and asset placement request.
    pub(crate) action: DemandSpawnAction,
    /// Operational day when demand originally planned the spawn.
    pub(crate) planned_day_index: u32,
    /// Minute of day when demand originally planned the spawn.
    pub(crate) planned_minute_of_day: u16,
}

fn access_phase_target(core: &SimCore, agent_idx: usize, egress: bool) -> Option<Vector3> {
    let building_id = if egress {
        core.agents.current_building[agent_idx]
    } else {
        core.agents.target_building[agent_idx]
    };
    let entrance = core.allocator.entrances.get(building_id)?;
    if egress {
        if core.agents.transit_mode[agent_idx] == MODE_CAR {
            let lane_id = core.agents.planned_attach_lane_id[agent_idx] as usize;
            let lane_d = core.agents.planned_attach_lane_d[agent_idx];
            let lane = core.transit_network.lane_system.lanes.get(lane_id)?;
            let lane_pos = BuildingAllocator::sample_pos_on_lane(lane, lane_d);
            Some(Vector3::new(lane_pos.x, 0.0, lane_pos.y))
        } else {
            Some(Vector3::new(entrance.curb_pos.x, 0.0, entrance.curb_pos.y))
        }
    } else {
        Some(Vector3::new(entrance.door_pos.x, 0.0, entrance.door_pos.y))
    }
}

fn pedestrian_lane_surface_height(lane: &Lane, lane_y: f32) -> f32 {
    if lane.lane_type == LaneType::Foot
        && lane.edge_id != usize::MAX
        && lane.lane_idx.unsigned_abs() == 100
    {
        lane_y + CURB_STEP_HEIGHT_M
    } else {
        lane_y
    }
}

fn pedestrian_needs_access_surface(transit: u8) -> bool {
    use crate::simulation::economy::agents::{TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS};

    transit == TRANSIT_ACCESS_EGRESS || transit == TRANSIT_ACCESS_INGRESS
}

fn pedestrian_access_surface_height(core: &SimCore, world_x: f32, world_z: f32) -> f32 {
    core.transit_network
        .road_surface
        .sample_visible_surface_height(&core.region_graph, &core.heightmap, world_x, world_z)
        .or_else(|| {
            core.allocator
                .sample_building_site_height(Vector2::new(world_x, world_z))
        })
        .unwrap_or_else(|| {
            core.heightmap.sample_visual_height_world(world_x, world_z) * HEIGHT_SCALE
        })
}

fn absolute_operational_minute(day_index: u32, minute_of_day: u16) -> u64 {
    u64::from(day_index.saturating_sub(1)) * MINUTES_PER_DAY_U64 + u64::from(minute_of_day)
}

fn demand_plan_has_non_spawn_actions(plan: &DemandBuildingActionPlan) -> bool {
    [&plan.residential, &plan.commercial, &plan.industrial]
        .iter()
        .any(|use_plan| {
            !use_plan.despawns.is_empty()
                || !use_plan.downgrades.is_empty()
                || !use_plan.upgrades.is_empty()
        })
}

fn demand_plan_without_spawns(plan: &DemandBuildingActionPlan) -> DemandBuildingActionPlan {
    let mut immediate_plan = plan.clone();
    immediate_plan.residential.spawns.clear();
    immediate_plan.commercial.spawns.clear();
    immediate_plan.industrial.spawns.clear();
    immediate_plan
}

#[cfg(test)]
mod tests {
    use super::{
        CURB_STEP_HEIGHT_M, CityTreasury, SimCore, absolute_operational_minute,
        demand_plan_has_non_spawn_actions, demand_plan_without_spawns,
        pedestrian_lane_surface_height, pedestrian_needs_access_surface,
    };
    use crate::assets::AssetManifest;
    use crate::assets::asset::{
        Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass,
    };
    use crate::simulation::buildings::allocator::BuildingAllocator;
    use crate::simulation::core::config::WorldConfig;
    use crate::simulation::core::time::TimeSystem;
    use crate::simulation::economy::agents::{
        AgentSystem, TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING,
        TRANSIT_NETWORK,
    };
    use crate::simulation::economy::demand::{
        DemandBuildingActionKey, DemandBuildingActionPlan, DemandSpawnAction, DemandSystem,
    };
    use crate::simulation::economy::households::HouseholdSystem;
    use crate::simulation::economy::logistics::ShipmentSystem;
    use crate::simulation::grid::desirability::DesirabilitySystem;
    use crate::simulation::grid::noise::NoiseSystem;
    use crate::simulation::grid::pollution::PollutionSystem;
    use crate::simulation::network::lanes::{Lane, LaneType};
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitFlags, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::network::{TransitNetwork, graph::Edge, graph::RegionGraph};
    use crate::simulation::terrain::TerrainSystem;
    use crate::simulation::water::WaterSystem;
    use crate::simulation::zoning::{ZoneType, ZoningSystem};
    use godot::prelude::Vector3;
    use std::collections::{HashMap, VecDeque};

    fn test_core() -> SimCore {
        let config = WorldConfig::default();
        SimCore {
            time: TimeSystem::new(),
            heightmap: TerrainSystem::from_world_config(&config),
            watermap: WaterSystem::from_world_config(&config),
            region_graph: RegionGraph::new(),
            transit_network: TransitNetwork::new_with_world_terrain_chunk_span(
                config.terrain_chunk_m,
            ),
            zoning: ZoningSystem::new(&config),
            pollution: PollutionSystem::new(&config),
            noise: NoiseSystem::new(&config),
            desirability: DesirabilitySystem::new(&config),
            demand: DemandSystem::new(),
            pending_demand_spawns: VecDeque::new(),
            allocator: BuildingAllocator::new(),
            agents: AgentSystem::new(),
            households: HouseholdSystem::new(),
            logistics: ShipmentSystem::new(),
            config,
            treasury: CityTreasury::new(0.0),
            service_policy: Default::default(),
            budget_history: VecDeque::new(),
            budget_last_lifetime_build_cost: 0.0,
            debug_household_admissions_since_daily: 0,
            undo_stack: VecDeque::new(),
            world_lake_fills: Vec::new(),
            world_open_water_fills: Vec::new(),
            world_lake_fill_preview: None,
            authored_water_patch_fill_debug_cache: HashMap::new(),
            terrain_stroke_active: false,
            terrain_stroke_has_changes: false,
            terrain_dirty: false,
            water_dirty: false,
            network_dirty: false,
            benchmark_mode: true,
            last_tick_duration: 0.0,
            last_agent_tick_us: 0,
            last_road_timing: String::new(),
            last_surface_debug_edges: Vec::new(),
            refined_terrain_patch_cache: HashMap::new(),
            water_patch_mesh_cache: HashMap::new(),
            road_locked_terrain_patch_keys: Vec::new(),
            cached_road_mesh_data: None,
            road_tool_surface_generation: 1,
            camera_aabb: (0.0, 0.0, 0.0, 0.0),
        }
    }

    fn add_test_border_road(core: &mut SimCore) {
        let border = core
            .region_graph
            .add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Border);
        let junction = core
            .region_graph
            .add_node(Vector3::new(180.0, 0.0, 0.0), NodeType::Junction);
        core.region_graph.add_edge(Edge {
            start_node: border,
            end_node: junction,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 13.89,
            base_cost: 180.0,
            physical_length: 180.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(180.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(180.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        core.transit_network
            .lane_system
            .rebuild(&mut core.region_graph);
    }

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        asset_id: &str,
        zone_type: ZoneType,
    ) -> String {
        let (zone_class, household_capacity, worker_capacity, economy_profile) = match zone_type {
            ZoneType::Residential => (ZoneClass::Residential, Some(6), None, None),
            ZoneType::Commercial => (
                ZoneClass::Commercial,
                None,
                Some(4),
                Some("grocery_basic".to_owned()),
            ),
            ZoneType::Industrial => (
                ZoneClass::Industrial,
                None,
                Some(4),
                Some("food_processor_basic".to_owned()),
            ),
            _ => panic!("test asset requires a baseline private-use zone"),
        };
        let manifest = AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![],
            mesh_parts: vec![MeshPart::single_lod0("main", "lod0.glb")],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
                width_m: None,
                length_m: None,
                vehicle_class: None,
            }],
            site_surfaces: vec![],
            building: Some(BuildingData {
                flat_size_m2: household_capacity.map(|_| 80.0),
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone_class),
                density: Some("low".to_owned()),
                lot_width_cells: 2,
                lot_depth_cells: 2,
                frontage_forward: None,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                household_capacity,
                worker_capacity,
                service_class: None,
                economy_profile,
            }),
            prop: None,
            vehicle: None,
            character: None,
        };
        allocator.registry.register("test", manifest, String::new());
        format!("test:{asset_id}")
    }

    fn place_test_parcel_run(core: &mut SimCore, zone_type: ZoneType, start_x: f32, end_x: f32) {
        let profile = core
            .zoning
            .profiles
            .default_runtime_id_for_zone_type(zone_type)
            .expect("test zoning profile");
        core.zoning
            .place_parcel_run_at(
                start_x,
                -20.0,
                end_x,
                -20.0,
                profile,
                20.0,
                30.0,
                0.0,
                &core.region_graph,
            )
            .expect("test parcel run");
    }

    #[test]
    fn absolute_operational_minute_is_day_stable() {
        assert_eq!(absolute_operational_minute(1, 0), 0);
        assert_eq!(absolute_operational_minute(1, 1439), 1439);
        assert_eq!(absolute_operational_minute(2, 0), 1440);
        assert_eq!(absolute_operational_minute(3, 60), 2940);
    }

    #[test]
    fn immediate_demand_plan_strips_spawns_only() {
        let mut plan = DemandBuildingActionPlan::default();
        plan.residential.spawns.push(DemandSpawnAction {
            parcel_id: 7,
            asset_id: "building.residential.test".to_owned(),
        });
        plan.residential.despawns.push(DemandBuildingActionKey {
            parcel_id: 8,
            edge_idx: 1,
            side: 1,
            cell_x: 2,
            width_cells: 3,
            depth_cells: 4,
            level: 1,
            asset_id: "building.residential.old".to_owned(),
        });

        let immediate = demand_plan_without_spawns(&plan);

        assert!(immediate.residential.spawns.is_empty());
        assert_eq!(immediate.residential.despawns.len(), 1);
        assert!(demand_plan_has_non_spawn_actions(&immediate));
    }

    #[test]
    fn max_demand_cheat_runtime_queues_and_executes_rci_spawns() {
        let mut core = test_core();
        add_test_border_road(&mut core);
        register_test_asset(&mut core.allocator, "residential", ZoneType::Residential);
        register_test_asset(&mut core.allocator, "commercial", ZoneType::Commercial);
        register_test_asset(&mut core.allocator, "industrial", ZoneType::Industrial);
        place_test_parcel_run(&mut core, ZoneType::Residential, 10.0, 50.0);
        place_test_parcel_run(&mut core, ZoneType::Commercial, 60.0, 100.0);
        place_test_parcel_run(&mut core, ZoneType::Industrial, 110.0, 150.0);

        core.apply_money_and_max_demand_cheat(1_000_000.0);
        core.execute_hourly_demand_pass(1, 0, &[]);

        assert!(
            core.pending_demand_spawns
                .iter()
                .any(|pending| pending.zone_type == ZoneType::Residential)
        );
        assert!(
            core.pending_demand_spawns
                .iter()
                .any(|pending| pending.zone_type == ZoneType::Commercial)
        );
        assert!(
            core.pending_demand_spawns
                .iter()
                .any(|pending| pending.zone_type == ZoneType::Industrial)
        );

        let queued_spawn_count = core.pending_demand_spawns.len();
        let mut executed_spawn_count = 0_usize;
        for minute_offset in 1..=queued_spawn_count {
            executed_spawn_count +=
                core.execute_pending_demand_spawns_for_minute(1, minute_offset as u16);
        }

        assert_eq!(executed_spawn_count, queued_spawn_count);
        assert!(core.pending_demand_spawns.is_empty());
        assert!(
            core.allocator
                .buildings
                .iter()
                .any(|building| building.zone_type == ZoneType::Residential)
        );
        assert!(
            core.allocator
                .buildings
                .iter()
                .any(|building| building.zone_type == ZoneType::Commercial)
        );
        assert!(
            core.allocator
                .buildings
                .iter()
                .any(|building| building.zone_type == ZoneType::Industrial)
        );
    }

    #[test]
    fn pedestrian_lane_surface_height_matches_lane_semantics() {
        let sidewalk = Lane {
            edge_id: 7,
            lane_idx: 100,
            lane_type: LaneType::Foot,
            ..Lane::default()
        };
        assert_eq!(
            pedestrian_lane_surface_height(&sidewalk, 4.0),
            4.0 + CURB_STEP_HEIGHT_M
        );

        let crosswalk = Lane {
            edge_id: usize::MAX,
            is_crosswalk: true,
            lane_type: LaneType::Foot,
            ..Lane::default()
        };
        assert_eq!(pedestrian_lane_surface_height(&crosswalk, 4.0), 4.0);

        let footpath = Lane {
            edge_id: 7,
            lane_idx: 0,
            lane_type: LaneType::Foot,
            ..Lane::default()
        };
        assert_eq!(pedestrian_lane_surface_height(&footpath, 4.0), 4.0);
    }

    #[test]
    fn pedestrian_access_surface_is_limited_to_door_transitions() {
        assert!(pedestrian_needs_access_surface(TRANSIT_ACCESS_EGRESS));
        assert!(pedestrian_needs_access_surface(TRANSIT_ACCESS_INGRESS));
        assert!(!pedestrian_needs_access_surface(TRANSIT_NETWORK));
        assert!(!pedestrian_needs_access_surface(TRANSIT_IN_BUILDING));
    }
}

/// Currency cost per meter of new road laid, deducted from the city treasury at placement.
pub(crate) const ROAD_BUILD_COST_PER_METER: f64 = 100.0;
/// Starter build cost per service-building lot cell, deducted from the city treasury at placement.
pub(crate) const SERVICE_BUILD_COST_PER_LOT_CELL: f64 = 2_500.0;
/// Currency upkeep per meter of road per day, settled from the city treasury each day.
pub(crate) const ROAD_UPKEEP_PER_METER_PER_DAY: f64 = 0.1;
/// Fine render step used for terrain patches whose topology is clipped by visible road surfaces.
pub(crate) const ROAD_LOCKED_TERRAIN_RENDER_STEP_M: f32 = 2.0;
const PEDESTRIAN_SURFACE_CLEARANCE_M: f32 = 0.02;
/// Stable service policy id for the city electricity service.
pub(crate) const SERVICE_POLICY_ELECTRICITY: &str = "electricity";
/// Number of completed daily budget entries kept for UI trend graphs.
pub(crate) const ECONOMY_HISTORY_DAYS: usize = 180;

/// Player-controlled city service funding policies.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CityServicePolicy {
    /// Electricity service funding ratio in `0.0..=1.0`.
    pub(crate) electricity_funding: f32,
}

impl Default for CityServicePolicy {
    fn default() -> Self {
        Self {
            electricity_funding: 1.0,
        }
    }
}

/// Completed daily accounting buckets shown by the Economy Overview.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct DailyBudgetLedgerEntry {
    /// Operational day index this completed entry represents.
    pub(crate) day_index: u32,
    /// Total city income recorded for the day.
    pub(crate) income: f64,
    /// Total city expenses recorded for the day.
    pub(crate) expenses: f64,
    /// Net cashflow for the day.
    pub(crate) net: f64,
    /// Treasury balance after the daily ledger closed.
    pub(crate) treasury: f64,
    /// Combined tax income bucket.
    pub(crate) tax_income: f64,
    /// Local utility and service revenue collected by city-owned providers.
    pub(crate) utility_service_revenue: f64,
    /// Unemployment benefit expense paid from the treasury.
    pub(crate) benefits: f64,
    /// City-owned service payroll expense.
    pub(crate) city_wages: f64,
    /// Treasury-funded service input and fuel purchase expense.
    pub(crate) fuel_input_purchases: f64,
    /// City-paid OWA fallback expense.
    pub(crate) imports_owa: f64,
    /// Construction, road upkeep, and internal service operating costs.
    pub(crate) construction_service_costs: f64,
    /// Electricity units produced during the day.
    pub(crate) power_produced: f64,
    /// Electricity units consumed from local producers during the day.
    pub(crate) power_consumed: f64,
    /// Electricity demand not served by local producers during the day.
    pub(crate) power_unmet: f64,
    /// Local electricity coverage ratio in `0.0..=1.0`.
    pub(crate) power_coverage: f64,
    /// Coal inventory currently held by city power providers.
    pub(crate) coal_inventory: f64,
    /// Estimated coal units bought for city power providers during the day.
    pub(crate) coal_bought: f64,
    /// Estimated coal units consumed by city power providers during the day.
    pub(crate) coal_consumed: f64,
    /// City fuel/input cost attributable to electricity providers.
    pub(crate) electricity_fuel_cost: f64,
    /// City payroll cost attributable to electricity providers.
    pub(crate) electricity_wage_cost: f64,
    /// Local electricity revenue collected from consumers.
    pub(crate) electricity_revenue: f64,
    /// Local electricity service balance after fuel and payroll.
    pub(crate) electricity_net: f64,
}
/// City-level fiscal ledger, separate from household budgets and building budgets.
///
/// The balance may go negative: deficits are an explicit fiscal state rather than
/// a blocked operation. Future debt/credit systems may add consequences later.
pub struct CityTreasury {
    /// Current balance in currency units. May be negative.
    pub balance: f64,
    /// Running total of all infrastructure build costs since game start.
    pub lifetime_build_cost: f64,
    /// Running total of all collected tax revenue since game start.
    pub lifetime_tax_revenue: f64,
    /// Road upkeep deducted in the most recent daily settlement.
    pub last_daily_upkeep: f64,
    /// Income tax collected in the most recently finalized fiscal day.
    pub last_daily_income_tax: f64,
    /// Household VAT collected in the most recently finalized fiscal day.
    pub last_daily_household_vat: f64,
    /// Business purchase tax collected in the most recently finalized fiscal day.
    pub last_daily_business_purchase_tax: f64,
    /// Business profit tax collected in the most recently finalized fiscal day.
    pub last_daily_business_profit_tax: f64,
    /// Property tax collected in the most recently finalized fiscal day.
    pub last_daily_property_tax: f64,
    /// Income tax collected since the last daily fiscal finalization.
    pub pending_income_tax: f64,
    /// Household VAT collected since the last daily fiscal finalization.
    pub pending_household_vat: f64,
    /// Business purchase tax collected since the last daily fiscal finalization.
    pub pending_business_purchase_tax: f64,
    /// Business profit tax collected since the last daily fiscal finalization.
    pub pending_business_profit_tax: f64,
    /// Property tax collected since the last daily fiscal finalization.
    pub pending_property_tax: f64,
}

impl CityTreasury {
    /// Initialises the treasury with the given startup balance.
    pub(crate) fn new(startup_balance: f64) -> Self {
        Self {
            balance: startup_balance,
            lifetime_build_cost: 0.0,
            lifetime_tax_revenue: 0.0,
            last_daily_upkeep: 0.0,
            last_daily_income_tax: 0.0,
            last_daily_household_vat: 0.0,
            last_daily_business_purchase_tax: 0.0,
            last_daily_business_profit_tax: 0.0,
            last_daily_property_tax: 0.0,
            pending_income_tax: 0.0,
            pending_household_vat: 0.0,
            pending_business_purchase_tax: 0.0,
            pending_business_profit_tax: 0.0,
            pending_property_tax: 0.0,
        }
    }

    /// Deducts an infrastructure build cost from the treasury. Balance may go negative.
    pub(crate) fn deduct_build_cost(&mut self, amount: f64) {
        self.balance -= amount;
        self.lifetime_build_cost += amount;
    }

    /// Settles one day's infrastructure upkeep cost. Balance may go negative.
    pub(crate) fn settle_daily_upkeep(&mut self, amount: f64) {
        self.balance -= amount;
        self.last_daily_upkeep = amount;
    }

    /// Records wage income tax withheld from household income.
    pub(crate) fn collect_income_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::Income);
    }

    /// Records VAT collected from household shopping purchases.
    pub(crate) fn collect_household_vat(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::HouseholdVat);
    }

    /// Records tax collected from business input purchases.
    pub(crate) fn collect_business_purchase_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::BusinessPurchase);
    }

    /// Records tax collected from positive daily business profit.
    pub(crate) fn collect_business_profit_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::BusinessProfit);
    }

    /// Records one-time property tax from new private construction.
    pub(crate) fn collect_property_tax(&mut self, amount: f64) {
        self.record_tax(amount, TaxBucket::Property);
    }

    /// Rolls the current pending fiscal window into daily reporting buckets.
    pub(crate) fn finalize_daily_tax_window(&mut self) {
        self.last_daily_income_tax = self.pending_income_tax;
        self.last_daily_household_vat = self.pending_household_vat;
        self.last_daily_business_purchase_tax = self.pending_business_purchase_tax;
        self.last_daily_business_profit_tax = self.pending_business_profit_tax;
        self.last_daily_property_tax = self.pending_property_tax;
        self.pending_income_tax = 0.0;
        self.pending_household_vat = 0.0;
        self.pending_business_purchase_tax = 0.0;
        self.pending_business_profit_tax = 0.0;
        self.pending_property_tax = 0.0;
    }

    fn record_tax(&mut self, amount: f64, bucket: TaxBucket) {
        if amount <= 0.0 {
            return;
        }
        self.balance += amount;
        self.lifetime_tax_revenue += amount;
        match bucket {
            TaxBucket::Income => self.pending_income_tax += amount,
            TaxBucket::HouseholdVat => self.pending_household_vat += amount,
            TaxBucket::BusinessPurchase => self.pending_business_purchase_tax += amount,
            TaxBucket::BusinessProfit => self.pending_business_profit_tax += amount,
            TaxBucket::Property => self.pending_property_tax += amount,
        }
    }
}

#[derive(Clone, Copy)]
enum TaxBucket {
    Income,
    HouseholdVat,
    BusinessPurchase,
    BusinessProfit,
    Property,
}

/// Validation state for one transient world-editor lake-fill preview.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldLakeFillPreviewStatus {
    /// The preview covers a closed basin and can be committed.
    Ready,
    /// The chosen surface is at or below the seed terrain height.
    SurfaceBelowSeedTerrain,
    /// The chosen surface spills out of the basin and reaches the world edge.
    EscapesWorldEdge,
    /// The chosen open-water surface does not connect to the world edge.
    DoesNotReachWorldEdge,
}

/// Preview feature kind for world-editor surface fills.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorldWaterFillKind {
    /// Closed inland basin fill.
    Lake,
    /// Edge-connected open-water fill.
    OpenWater,
}

/// Debug summary for one authored water fill that contributes to a render patch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct AuthoredWaterPatchFillDebug {
    /// Authored fill kind that produced the patch contribution.
    pub(crate) kind: WorldWaterFillKind,
    /// Committed fill index in its authored list, or `-1` for a transient preview.
    pub(crate) fill_index: i32,
    /// Whether this contribution came from the active transient preview.
    pub(crate) preview: bool,
    /// Snapped seed X coordinate in world metres.
    pub(crate) world_x: f32,
    /// Snapped seed Z coordinate in world metres.
    pub(crate) world_z: f32,
    /// Authored flat water surface elevation in metres.
    pub(crate) surface_elevation_m: f32,
    /// Number of cells in the complete fill body.
    pub(crate) filled_cells: usize,
    /// Whether the complete fill body touches the world edge.
    pub(crate) touches_world_edge: bool,
    /// Number of non-zero water samples contributed inside the requested patch.
    pub(crate) patch_nonzero_samples: usize,
    /// Maximum contributed water depth inside the requested patch.
    pub(crate) patch_max_depth_m: f32,
    /// Sum of contributed water depths inside the requested patch.
    pub(crate) patch_sum_depth_m: f32,
}

/// Transient lake-fill preview state owned by the world editor runtime.
///
/// This state is never serialized into `WorldDefinition`. It exists only so the
/// editor can show live water feedback while the author adjusts the target
/// surface elevation before confirming the lake fill.
#[derive(Clone, Copy, Debug)]
pub(crate) struct WorldLakeFillPreview {
    /// Preview feature kind.
    pub kind: WorldWaterFillKind,
    /// Snapped seed X coordinate in world metres.
    pub seed_world_x: f32,
    /// Snapped seed Z coordinate in world metres.
    pub seed_world_z: f32,
    /// Seed terrain height in rendered world metres.
    pub seed_height_m: f32,
    /// Preview surface elevation in rendered world metres.
    pub surface_elevation_m: f32,
    /// Preview validation outcome.
    pub status: WorldLakeFillPreviewStatus,
    /// Number of filled terrain cells in the preview flood.
    pub filled_cells: usize,
}

impl WorldLakeFillPreview {
    /// Returns `true` when the preview is valid and may be committed.
    pub(crate) fn is_valid(self) -> bool {
        self.status == WorldLakeFillPreviewStatus::Ready
    }
}

/// Full water runtime snapshot for undo history.
pub(crate) struct WaterRuntimeSnapshot {
    /// Flat authored or loaded baseline water depth above terrain.
    pub baseline_depth: Vec<f32>,
}

/// Derived road render state that matches an undo graph snapshot.
pub(crate) struct NetworkRenderRuntimeSnapshot {
    /// Compiled road-surface cache for the snapped road graph.
    pub(crate) road_surface: RoadSurfaceSystem,
    /// Refined terrain patches prepared for road-locked render patches.
    pub(crate) refined_terrain_patch_cache:
        HashMap<RefinedTerrainPatchCacheKey, CachedRefinedTerrainPatch>,
    /// Render patches that were road-locked when the snapshot was captured.
    pub(crate) road_locked_terrain_patch_keys: Vec<(usize, usize)>,
}

/// Building and economy runtime state that must move together for entity deletion undo.
pub(crate) struct SimulationRuntimeSnapshot {
    /// Building allocator, indices, derived site data, and occupancy-facing metadata.
    pub(crate) allocator: BuildingAllocator,
    /// Live agent SoA state after lifecycle eviction/remapping.
    pub(crate) agents: AgentSystem,
    /// Household records that reference building indices.
    pub(crate) households: HouseholdSystem,
    /// Freight reservations and shipment state that reference buildings.
    pub(crate) logistics: ShipmentSystem,
    /// Delayed demand spawns that can later mutate allocator and zoning state.
    pub(crate) pending_demand_spawns: VecDeque<PendingDemandSpawnAction>,
}

/// A snapshot of simulation state for undo history.
pub(crate) struct SimulationSnapshot {
    /// Terrain heightmap data.
    pub(crate) terrain: Option<Vec<f32>>,
    /// Water runtime state.
    pub(crate) water: Option<WaterRuntimeSnapshot>,
    /// Road network graph state.
    pub(crate) trans_graph: Option<crate::simulation::network::graph::RegionGraph>,
    /// Derived road render state matching `trans_graph`.
    pub(crate) network_render: Option<NetworkRenderRuntimeSnapshot>,
    /// Zoning system state.
    pub(crate) zoning: Option<ZoningSystem>,
    /// Building/economy runtime state.
    pub(crate) runtime: Option<SimulationRuntimeSnapshot>,
}

/// Cache key for one production refined terrain patch mesh.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct RefinedTerrainPatchCacheKey {
    /// Terrain render-patch X index.
    pub(crate) patch_x: usize,
    /// Terrain render-patch Z index.
    pub(crate) patch_z: usize,
    /// Refined render step quantized to millimetres.
    pub(crate) render_step_mm: u32,
}

/// Complete input needed to build a refined road-clipped terrain patch off the Godot frame.
pub(crate) struct RefinedTerrainPatchBuildInput {
    /// Cache key for the produced patch.
    pub(crate) key: RefinedTerrainPatchCacheKey,
    /// Base visual terrain patch snapshot.
    pub(crate) patch: TerrainPatchSnapshot,
    /// Local CDT windows assembled from source terrain samples and road footprint loops.
    pub(crate) windows: Vec<RefinedTerrainCdtWindowBuildInput>,
    /// Number of source road-boundary records found by the clip query.
    pub(crate) road_clip_source_count: usize,
    /// Terrain-clip setup error, if the road-boundary query failed before CDT input was built.
    pub(crate) clip_error_label: Option<&'static str>,
}

/// Cache key for one local CDT window inside a refined render patch.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct RefinedTerrainCdtWindowKey {
    /// Window minimum X in quantized millimetres.
    pub(crate) min_x_mm: i64,
    /// Window minimum Z in quantized millimetres.
    pub(crate) min_z_mm: i64,
    /// Window maximum X in quantized millimetres.
    pub(crate) max_x_mm: i64,
    /// Window maximum Z in quantized millimetres.
    pub(crate) max_z_mm: i64,
    /// Stable fingerprint of road loops and terrain samples in this window.
    pub(crate) fingerprint: u64,
}

/// Build input for one local CDT window inside a refined render patch.
pub(crate) struct RefinedTerrainCdtWindowBuildInput {
    /// Window cache key.
    pub(crate) key: RefinedTerrainCdtWindowKey,
    /// CDT input for this local window.
    pub(crate) cdt_input: TerrainCdtInput,
    /// Previous compiled window when the fingerprint did not change.
    pub(crate) previous: Option<CachedRefinedTerrainCdtWindow>,
}

/// Cached local CDT window built away from the Godot frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedRefinedTerrainCdtWindow {
    /// Window cache key.
    pub(crate) key: RefinedTerrainCdtWindowKey,
    /// Number of road loops supplied to the CDT builder.
    pub(crate) input_road_loops: usize,
    /// Number of source terrain samples supplied to the CDT builder.
    pub(crate) input_source_samples: usize,
    /// Local CDT window used inside the base render patch.
    pub(crate) cdt_patch: TerrainCdtPatch,
    /// CDT result for this window.
    pub(crate) mesh_result: Result<TerrainCdtMesh, TerrainCdtError>,
    /// Time spent in CDT construction for this window.
    pub(crate) cdt_ms: f64,
    /// True when this window was reused from the previous patch cache.
    pub(crate) reused: bool,
}

/// Cached production refined terrain patch built away from the Godot frame.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CachedRefinedTerrainPatch {
    /// Cache key for this patch.
    pub(crate) key: RefinedTerrainPatchCacheKey,
    /// Base visual terrain patch snapshot.
    pub(crate) patch: TerrainPatchSnapshot,
    /// Number of road loops supplied to the CDT builder.
    pub(crate) input_road_loops: usize,
    /// Number of source terrain samples supplied to the CDT builder.
    pub(crate) input_source_samples: usize,
    /// Local CDT windows composed into this render patch.
    pub(crate) windows: Vec<CachedRefinedTerrainCdtWindow>,
    /// Number of source road-boundary records found by the clip query.
    pub(crate) road_clip_source_count: usize,
    /// Terrain-clip setup error, if the road-boundary query failed before CDT input was built.
    pub(crate) clip_error_label: Option<&'static str>,
    /// Time spent in CDT construction for this patch's rebuilt windows.
    pub(crate) cdt_ms: f64,
    /// Number of windows reused from the previous cache entry.
    pub(crate) reused_windows: usize,
}

/// All simulation state — owned exclusively by the background sim thread when running.
///
/// The main thread accesses this via `Arc<Mutex<SimCore>>`. The lock is held for at
/// most one tick duration (~7 ms at 100 k agents) per mutation.
pub struct SimCore {
    /// Simulation clock and day counter.
    pub time: TimeSystem,
    /// Terrain heightmap.
    pub heightmap: TerrainSystem,
    /// Shallow-water simulation.
    pub watermap: WaterSystem,
    /// Road topology graph.
    pub region_graph: crate::simulation::network::graph::RegionGraph,
    /// Lane system, CCH pathfinder, and road mutation helpers.
    pub transit_network: TransitNetwork,
    /// Road-aligned parcel zoning.
    pub zoning: ZoningSystem,
    /// Pollution diffusion grid.
    pub pollution: PollutionSystem,
    /// Traffic noise grid.
    pub noise: NoiseSystem,
    /// Composite desirability grid.
    pub desirability: DesirabilitySystem,
    /// Global R/C/I demand counters.
    pub demand: DemandSystem,
    /// Demand building spawns scheduled across authored minutes to avoid hourly bursts.
    pub(crate) pending_demand_spawns: VecDeque<PendingDemandSpawnAction>,
    /// Building placement and vacancy index.
    pub allocator: BuildingAllocator,
    /// Agent FSM in Structure-of-Arrays layout.
    pub agents: AgentSystem,
    /// Explicit household runtime records and first-pass daily economy logic.
    pub households: HouseholdSystem,
    /// Active building-level freight reservations and delayed deliveries.
    pub logistics: ShipmentSystem,
    /// World configuration (extent, chunk metadata, cell sizes).
    pub config: WorldConfig,
    /// City-level fiscal ledger tracking infrastructure build cost and daily upkeep.
    pub treasury: CityTreasury,
    /// Player-controlled live service funding policies.
    pub(crate) service_policy: CityServicePolicy,
    /// Completed daily budget ledger entries for overview windows and trend graphs.
    pub(crate) budget_history: VecDeque<DailyBudgetLedgerEntry>,
    /// Lifetime build cost observed when the most recent budget ledger entry was recorded.
    pub(crate) budget_last_lifetime_build_cost: f64,
    /// Runtime-only economy debug counter reset after each daily diagnostic line.
    pub(crate) debug_household_admissions_since_daily: u32,
    /// Undo history stack — kept in SimCore so all mutations are co-located.
    pub(crate) undo_stack: VecDeque<SimulationSnapshot>,
    /// Authored-world lake fill records when editing or playing from a `WorldDefinition`.
    pub(crate) world_lake_fills: Vec<AuthoredLakeFill>,
    /// Authored-world edge-connected open-water fills when editing or playing from a `WorldDefinition`.
    pub(crate) world_open_water_fills: Vec<AuthoredOpenWaterFill>,
    /// Transient world-editor lake-fill preview. Never saved into `WorldDefinition`.
    pub(crate) world_lake_fill_preview: Option<WorldLakeFillPreview>,
    /// Cached authored-water fill debug summaries keyed by water render patch.
    pub(crate) authored_water_patch_fill_debug_cache:
        HashMap<(usize, usize), Vec<AuthoredWaterPatchFillDebug>>,
    /// True while the world editor is accumulating one terrain brush stroke.
    pub(crate) terrain_stroke_active: bool,
    /// True once the active terrain brush stroke has applied at least one terrain mutation.
    pub(crate) terrain_stroke_has_changes: bool,
    /// Set by terrain mutations; cleared by the Godot render layer.
    pub terrain_dirty: bool,
    /// Set by water mutations; cleared by the Godot render layer.
    pub water_dirty: bool,
    /// Set by any network mutation (road, rail); cleared by `clear_network_dirty()` after
    /// `NetworkRenderer` finishes rebuilding the visual mesh. Stays `true` until GDScript
    /// explicitly clears it — same pattern as `terrain_dirty` and `water_dirty`.
    pub network_dirty: bool,
    /// True when running in benchmark mode (skips undo stack on road placement).
    pub benchmark_mode: bool,
    /// Duration of the last daily economy tick in milliseconds.
    pub last_tick_duration: f64,
    /// Duration of the last agent movement tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Per-phase timing breakdown from the last road placement, for profiling.
    pub last_road_timing: String,
    /// Edge ids touched by the most recent committed network edit and queued for one focused
    /// road-surface debug dump after the next terrain/mesh rebuild.
    pub(crate) last_surface_debug_edges: Vec<usize>,
    /// Production refined terrain patches precomputed by the sim thread for Godot upload.
    pub(crate) refined_terrain_patch_cache:
        HashMap<RefinedTerrainPatchCacheKey, CachedRefinedTerrainPatch>,
    /// Production water patch meshes built in Rust and reused across camera-driven LOD changes.
    pub(crate) water_patch_mesh_cache: HashMap<WaterPatchMeshCacheKey, CachedWaterPatchMesh>,
    /// Sorted terrain render patches that must use road-locked refined terrain meshes.
    pub(crate) road_locked_terrain_patch_keys: Vec<(usize, usize)>,
    /// Latest full road mesh generated by the sim thread after a network edit.
    pub(crate) cached_road_mesh_data: Option<NetworkMeshData>,
    /// Monotonic stamp for road-tool terrain/surface snapshots.
    pub(crate) road_tool_surface_generation: u64,
    /// World-space AABB for frustum culling: (x_min, x_max, z_min, z_max).
    /// Agents outside this rect are excluded from `RenderSnapshot` transforms.
    /// Updated each frame via `SimCommand::SetCameraAabb`. Defaults to "show all".
    pub camera_aabb: (f32, f32, f32, f32),
}

#[derive(Clone, Copy, Debug, Default)]
struct DailyCityFlowDiagnostics {
    active_households: u32,
    housed_households: u32,
    unhoused_households: u32,
    zero_budget_households: u32,
    stock_empty_households: u32,
    stock_low_households: u32,
    total_household_slots: u32,
    vacant_household_slots: u32,
    resident_agents: u32,
    child_agents: u32,
    adult_agents: u32,
    elder_agents: u32,
    pending_household_carriers: u32,
    employed_agents: u32,
    unemployed_agents: u32,
    commercial_job_capacity: u32,
    commercial_filled_jobs: u32,
    industrial_job_capacity: u32,
    industrial_filled_jobs: u32,
}

#[derive(Clone, Debug)]
pub(crate) struct RoadPreviewSnapshot {
    pub(crate) request_id: u64,
    pub(crate) surface_generation: u64,
    pub(crate) prepared_points: Vec<godot::prelude::Vector3>,
    pub(crate) surface_vertices: Vec<godot::prelude::Vector3>,
    pub(crate) validation: RoadPreviewValidation,
    pub(crate) is_valid: bool,
}

#[derive(Clone)]
pub(crate) struct RoadPreviewWorkerContext {
    terrain: TerrainSystem,
    region_graph: RegionGraph,
    road_surface: RoadSurfaceSystem,
    surface_chunk_span_m: f32,
    surface_generation: u64,
}

impl RoadPreviewWorkerContext {
    pub(crate) fn from_core(core: &SimCore) -> Self {
        Self {
            terrain: core.heightmap.clone(),
            region_graph: core.region_graph.clone(),
            road_surface: core.transit_network.road_surface.clone(),
            surface_chunk_span_m: core.transit_network.road_surface.chunk_span_m(),
            surface_generation: core.road_tool_surface_generation,
        }
    }
}

pub(crate) struct RoadPreviewRequest {
    pub(crate) request_id: u64,
    pub(crate) points: Vec<godot::prelude::Vector3>,
    pub(crate) fwd_lanes: i32,
    pub(crate) bkw_lanes: i32,
}

#[derive(Clone)]
pub(crate) struct RoadToolQuerySnapshot {
    pub(crate) terrain: TerrainSystem,
    pub(crate) region_graph: RegionGraph,
    pub(crate) road_surface: RoadSurfaceSystem,
    pub(crate) ghost_snap_index: RoadGhostSnapIndex,
    pub(crate) surface_generation: u64,
}

impl RoadToolQuerySnapshot {
    pub(crate) fn from_core(core: &SimCore) -> Self {
        Self {
            terrain: core.heightmap.clone(),
            region_graph: core.region_graph.clone(),
            road_surface: core.transit_network.road_surface.clone(),
            ghost_snap_index: RoadGhostSnapIndex::from_graph(&core.region_graph),
            surface_generation: core.road_tool_surface_generation,
        }
    }
}

impl SimCore {
    pub(crate) fn bump_road_tool_surface_generation(&mut self) {
        self.road_tool_surface_generation =
            self.road_tool_surface_generation.wrapping_add(1).max(1);
    }
}

/// Pre-computed rendering data written by the sim thread and read by the render thread.
///
/// Contains only pure Rust types so the struct is `Send + Sync` without unsafe.
/// The Godot main thread converts these `Vec<f32>` buffers to `PackedFloat32Array`
/// when the `#[func]` render getters are called.
pub struct RenderSnapshot {
    /// Per `pedestrian_type` → flat 12-float `Transform3D` buffer.
    pub pedestrian_transforms: HashMap<u8, Vec<f32>>,
    /// Per `(vehicle_type * 10 + color_variant)` → flat 12-float `Transform3D` buffer.
    pub car_transforms: HashMap<u8, Vec<f32>>,
    /// Per car transform bucket → render IDs matching `car_transforms` instance order.
    pub car_render_ids: HashMap<u8, Vec<i64>>,
    /// Mirrors `SimCore::terrain_dirty` at snapshot time.
    pub terrain_dirty: bool,
    /// Mirrors `SimCore::water_dirty` at snapshot time.
    pub water_dirty: bool,
    /// Mirrors `SimCore::network_dirty` at snapshot time; cleared the same frame.
    pub network_dirty: bool,
    /// Current simulation day.
    pub current_day: u32,
    /// Current minute since operational midnight.
    pub current_minute_of_day: u16,
    /// Duration of the last daily tick in milliseconds.
    pub last_tick_ms: f64,
    /// Duration of the last agent tick in microseconds.
    pub last_agent_tick_us: u64,
    /// Number of CCH pathfinding calls since the last daily tick reset.
    pub pathfind_count: u32,
    /// Total number of live agents.
    pub agent_count: i32,
    /// Current city treasury balance in currency units.
    pub treasury_balance: f64,
    /// Heightmap width in cells (for CSV logging on the main thread).
    pub heightmap_width: usize,
    /// Heightmap height in cells (for CSV logging on the main thread).
    pub heightmap_height: usize,
    /// Terrain world extent in metres, cached so Godot tools do not lock `SimCore` per frame.
    pub terrain_world_size: godot::prelude::Vector2,
    /// Revision of zoning overlay-visible parcel geometry and zoning profiles.
    pub zoning_overlay_revision: u64,
    /// Revision of zoning occupancy that affects overlay parcel coloring.
    pub zoning_overlay_occupancy_revision: u64,
    /// World-space positions of all live canonical network nodes.
    /// Pre-computed here so `get_network_nodes()` reads the snapshot (RwLock)
    /// instead of locking SimCore — avoids main-thread stalls during road placement.
    pub node_positions: Vec<godot::prelude::Vector3>,
}

impl Default for RenderSnapshot {
    fn default() -> Self {
        Self {
            pedestrian_transforms: HashMap::new(),
            car_transforms: HashMap::new(),
            car_render_ids: HashMap::new(),
            terrain_dirty: true,
            water_dirty: true,
            network_dirty: false,
            current_day: 1,
            current_minute_of_day: 0,
            last_tick_ms: 0.0,
            last_agent_tick_us: 0,
            pathfind_count: 0,
            agent_count: 0,
            treasury_balance: 0.0,
            heightmap_width: 0,
            terrain_world_size: godot::prelude::Vector2::ZERO,
            zoning_overlay_revision: 0,
            zoning_overlay_occupancy_revision: 0,
            node_positions: Vec::new(),
            heightmap_height: 0,
        }
    }
}

/// Commands sent from the Godot main thread to the sim background thread.
pub enum SimCommand {
    /// Update the simulation speed multiplier.
    SetSpeed(f32),
    /// Update the camera world-space AABB used for agent frustum culling.
    /// Values: (x_min, x_max, z_min, z_max) in world units, padded by ~200 m.
    SetCameraAabb(f32, f32, f32, f32),
    /// Place a new road segment.  Executed in the sim thread so the main thread
    /// never blocks on the expensive lane-rebuild and zoning-obstruction passes.
    AddRoad {
        /// World-space polyline points.
        points: Vec<godot::prelude::Vector3>,
        /// Forward lane count.
        fwd_lanes: i32,
        /// Backward lane count.
        bkw_lanes: i32,
    },
    /// Ask the background thread to exit cleanly.
    Quit,
}

pub(crate) fn run_road_preview_worker(
    context: Arc<RwLock<RoadPreviewWorkerContext>>,
    result: Arc<RwLock<Option<RoadPreviewSnapshot>>>,
    rx: std::sync::mpsc::Receiver<RoadPreviewRequest>,
) {
    while let Ok(mut request) = rx.recv() {
        while let Ok(next) = rx.try_recv() {
            request = next;
        }

        let road_debug = crate::debug::category_enabled("road");
        let total_start = road_debug.then(Instant::now);
        let point_count = request.points.len();
        let preview = {
            let context = context.read().unwrap();
            compile_road_preview_from_context(&context, request)
        };
        let prepared_count = preview.prepared_points.len();
        let surface_vertex_count = preview.surface_vertices.len();
        let validation = preview.validation.clone();
        let is_valid = preview.is_valid;
        *result.write().unwrap() = Some(preview);
        if road_debug {
            debug_log!(
                "road",
                "preview_surface_worker points={} prepared_points={} surface_vertices={} valid={} reason={} max_grade={:.3} allowed_grade={:.3} span=({:.3},{:.3}) run={:.3} dy={:.3} span_y=({:.3},{:.3}) span_terrain=({:.3},{:.3}) span_delta=({:.3},{:.3}) endpoint_snap=({},{}) endpoint_delta=({:.3},{:.3}) total_ms={:.3}",
                point_count,
                prepared_count,
                surface_vertex_count,
                is_valid,
                validation.invalid_reason,
                validation.max_grade,
                validation.allowed_grade,
                validation.offending_span_start_m,
                validation.offending_span_end_m,
                validation.offending_span_run_m,
                validation.offending_span_height_delta_m,
                validation.offending_span_start_height_m,
                validation.offending_span_end_height_m,
                validation.offending_span_start_terrain_height_m,
                validation.offending_span_end_terrain_height_m,
                validation.offending_span_start_support_delta_m,
                validation.offending_span_end_support_delta_m,
                validation.start_endpoint_snapped_node_id,
                validation.end_endpoint_snapped_node_id,
                validation.start_endpoint_support_delta_m,
                validation.end_endpoint_support_delta_m,
                total_start
                    .map(|start| start.elapsed().as_secs_f64() * 1000.0)
                    .unwrap_or(0.0)
            );
        }
    }
}

pub(crate) fn compile_road_preview_from_context(
    context: &RoadPreviewWorkerContext,
    request: RoadPreviewRequest,
) -> RoadPreviewSnapshot {
    let preview_surface = RoadSurfaceSystem::new(context.surface_chunk_span_m);
    let preview = preview_surface.compile_preview_surface_mesh_only_with_existing_surface(
        &request.points,
        request.fwd_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        request.bkw_lanes.clamp(0, i32::from(u8::MAX)) as u8,
        &context.terrain,
        &context.region_graph,
        &context.road_surface,
    );

    RoadPreviewSnapshot {
        request_id: request.request_id,
        surface_generation: context.surface_generation,
        prepared_points: preview.prepared_points,
        surface_vertices: preview.surface_vertices,
        validation: preview.validation,
        is_valid: preview.is_valid,
    }
}

impl SimCore {
    /// Applies the gameplay cheat grant and pins all demand channels to maximum pressure.
    pub(crate) fn apply_money_and_max_demand_cheat(&mut self, money_amount: f64) -> f64 {
        if money_amount.is_finite() && money_amount > 0.0 {
            self.treasury.balance += money_amount;
        }
        self.demand.enable_max_demand_cheat();
        self.treasury.balance
    }

    /// Applies a live service funding policy change from the UI.
    pub(crate) fn set_service_funding(&mut self, service_id: &str, funding: f32) -> bool {
        let funding = funding.clamp(0.0, 1.0);
        match service_id {
            SERVICE_POLICY_ELECTRICITY | "power" => {
                self.service_policy.electricity_funding = funding;
                self.apply_service_funding_staffing_policy();
                true
            }
            _ => false,
        }
    }

    /// Applies a live per-building service funding override from an inspector panel.
    pub(crate) fn set_building_service_funding_override_at(
        &mut self,
        world_x: f32,
        world_z: f32,
        service_id: &str,
        funding: f32,
    ) -> bool {
        if !matches!(service_id, SERVICE_POLICY_ELECTRICITY | "power") {
            return false;
        }
        let Some(building_idx) = self.nearest_building_idx_at(world_x, world_z, 30.0) else {
            return false;
        };
        if !self.building_provides_service(building_idx, "power") {
            return false;
        }
        self.allocator.buildings[building_idx].service_funding_override = funding.clamp(0.0, 1.0);
        self.apply_service_funding_staffing_policy();
        true
    }

    pub(crate) fn effective_electricity_funding_for_building(&self, building_idx: usize) -> f32 {
        let Some(building) = self.allocator.buildings.get(building_idx) else {
            return self.service_policy.electricity_funding;
        };
        if building.service_funding_override >= 0.0 {
            building.service_funding_override.clamp(0.0, 1.0)
        } else {
            self.service_policy.electricity_funding
        }
    }

    pub(crate) fn electricity_funding_by_building(&self) -> Vec<f32> {
        let mut funding = vec![1.0; self.allocator.buildings.len()];
        let Ok(catalog) = load_runtime_economy_catalog() else {
            return funding;
        };
        for (idx, building) in self.allocator.buildings.iter().enumerate() {
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if profile.utility_service.as_deref() == Some("power") {
                funding[idx] = self.effective_electricity_funding_for_building(idx);
            }
        }
        funding
    }

    fn apply_service_funding_staffing_policy(&mut self) {
        let funding = self.electricity_funding_by_building();
        self.households.enforce_service_funding_staffing(
            &mut self.agents,
            &mut self.allocator,
            &funding,
        );
    }

    fn nearest_building_idx_at(&self, world_x: f32, world_z: f32, radius_m: f32) -> Option<usize> {
        let mut best_idx = usize::MAX;
        let mut best_dist_sq = radius_m.max(0.0) * radius_m.max(0.0);
        for (idx, building) in self.allocator.buildings.iter().enumerate() {
            let dx = building.center_x - world_x;
            let dz = building.center_y - world_z;
            let dist_sq = dx * dx + dz * dz;
            if dist_sq < best_dist_sq {
                best_idx = idx;
                best_dist_sq = dist_sq;
            }
        }
        (best_idx != usize::MAX).then_some(best_idx)
    }

    fn building_provides_service(&self, building_idx: usize, utility_service: &str) -> bool {
        let Some(building) = self.allocator.buildings.get(building_idx) else {
            return false;
        };
        let Ok(catalog) = load_runtime_economy_catalog() else {
            return false;
        };
        catalog
            .profile_by_runtime_id(building.economy_profile_runtime_id)
            .is_some_and(|profile| profile.utility_service.as_deref() == Some(utility_service))
    }

    fn record_daily_budget_ledger(&mut self, day_index: u32) {
        let construction_delta =
            (self.treasury.lifetime_build_cost - self.budget_last_lifetime_build_cost).max(0.0);
        self.budget_last_lifetime_build_cost = self.treasury.lifetime_build_cost;

        let entry = self.build_budget_ledger_entry(day_index, construction_delta);
        self.budget_history.push_back(entry);
        while self.budget_history.len() > ECONOMY_HISTORY_DAYS {
            self.budget_history.pop_front();
        }

        debug_log!(
            "economy",
            "budget ledger: day={} income={:.1} expenses={:.1} net={:.1} treasury={:.1} power=produced:{:.1} consumed:{:.1} unmet:{:.1} funding={:.2}",
            entry.day_index,
            entry.income,
            entry.expenses,
            entry.net,
            entry.treasury,
            entry.power_produced,
            entry.power_consumed,
            entry.power_unmet,
            self.service_policy.electricity_funding,
        );
    }

    fn build_budget_ledger_entry(
        &self,
        day_index: u32,
        construction_delta: f64,
    ) -> DailyBudgetLedgerEntry {
        let tax_income = self.treasury.last_daily_income_tax
            + self.treasury.last_daily_household_vat
            + self.treasury.last_daily_business_purchase_tax
            + self.treasury.last_daily_business_profit_tax
            + self.treasury.last_daily_property_tax;
        let benefits = self
            .households
            .daily_ledgers()
            .iter()
            .map(|ledger| f64::from(ledger.unemployment_benefit_income.max(0.0)))
            .sum::<f64>();
        let city_wages = f64::from(self.households.last_city_service_wage_cost().max(0.0));
        let power = self.households.last_power_settlement();
        let utility_service_revenue = f64::from(
            power.household_local_revenue
                + power.private_local_revenue
                + power.city_service_local_cost,
        );
        let imports_owa = f64::from(power.city_service_owa_cost.max(0.0));
        let construction_service_costs = construction_delta
            + self.treasury.last_daily_upkeep.max(0.0)
            + f64::from(power.city_service_local_cost.max(0.0));
        let (coal_inventory, coal_bought, coal_consumed, electricity_fuel_cost) =
            self.electricity_provider_daily_fuel_summary();
        let electricity_wage_cost = city_wages;
        let power_consumed = f64::from(power.served_units.max(0.0));
        let power_unmet = f64::from((power.demand_units - power.served_units).max(0.0));
        let power_produced = f64::from(power.supply_units.max(0.0));
        let electricity_revenue = utility_service_revenue;
        let electricity_net = electricity_revenue - electricity_fuel_cost - electricity_wage_cost;

        let income = tax_income + utility_service_revenue;
        let expenses = benefits
            + city_wages
            + electricity_fuel_cost
            + imports_owa
            + construction_service_costs;
        let net = income - expenses;

        DailyBudgetLedgerEntry {
            day_index,
            income,
            expenses,
            net,
            treasury: self.treasury.balance,
            tax_income,
            utility_service_revenue,
            benefits,
            city_wages,
            fuel_input_purchases: electricity_fuel_cost,
            imports_owa,
            construction_service_costs,
            power_produced,
            power_consumed,
            power_unmet,
            power_coverage: f64::from(power.coverage.clamp(0.0, 1.0)),
            coal_inventory,
            coal_bought,
            coal_consumed,
            electricity_fuel_cost,
            electricity_wage_cost,
            electricity_revenue,
            electricity_net,
        }
    }

    fn electricity_provider_daily_fuel_summary(&self) -> (f64, f64, f64, f64) {
        let Ok(catalog) = load_runtime_economy_catalog() else {
            return (0.0, 0.0, 0.0, 0.0);
        };
        let coal_runtime_id = catalog.resource_runtime_id_for_id("coal");
        let mut coal_inventory = 0.0f64;
        let mut coal_consumed = 0.0f64;
        let mut fuel_cost = 0.0f64;

        for building in &self.allocator.buildings {
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            if profile.utility_service.as_deref() != Some("power") {
                continue;
            }
            fuel_cost += f64::from(building.daily_city_funded_input_cost.max(0.0));
            if let Some(coal_runtime_id) = coal_runtime_id {
                coal_inventory += f64::from(building.inventory_units(coal_runtime_id).max(0.0));
            }
            for input_port in &profile.inputs {
                if Some(input_port.resource_runtime_id) != coal_runtime_id {
                    continue;
                }
                if profile.base_rate_units_per_day <= f32::EPSILON {
                    continue;
                }
                let produced_ratio =
                    building.daily_power_service_units.max(0.0) / profile.base_rate_units_per_day;
                coal_consumed += f64::from(input_port.units_per_day.max(0.0) * produced_ratio);
            }
        }

        let coal_bought = coal_runtime_id
            .and_then(|resource| catalog.unit_price_for_resource(resource))
            .filter(|unit_price| *unit_price > f32::EPSILON)
            .map(|unit_price| fuel_cost / f64::from(unit_price))
            .unwrap_or(0.0);

        (coal_inventory, coal_bought, coal_consumed, fuel_cost)
    }

    fn print_sim_console_summary(&self, day_index: u32, minute_of_day: u16) {
        let mut at_home = 0usize;
        let mut at_work = 0usize;
        let mut shopping = 0usize;
        let mut travelling = 0usize;
        let mut other = 0usize;

        for i in 0..self.agents.len() {
            if self.agents.transit[i] != TRANSIT_IN_BUILDING {
                travelling += 1;
                continue;
            }

            match self.agents.activity[i] {
                0 => at_home += 1,
                1 => at_work += 1,
                2 => shopping += 1,
                _ => other += 1,
            }
        }

        let household_count = self
            .households
            .households
            .iter()
            .filter(|household| household.member_count > 0)
            .count();
        let hours = minute_of_day / 60;
        let minutes = minute_of_day % 60;

        println!(
            "[SIM_DEBUG] Day {} {:02}:{:02} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} remove={} buildings={} households={} agents={} states=(home={}, work={}, shopping={}, travelling={}, other={}) actions=spawn({}/{}/{}) upgrade({}/{}/{}) downgrade({}/{}/{}) despawn({}/{}/{})",
            day_index,
            hours,
            minutes,
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.demand.households_to_remove_today,
            self.allocator.buildings.len(),
            household_count,
            self.agents.len(),
            at_home,
            at_work,
            shopping,
            travelling,
            other,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
        );
    }

    fn daily_city_flow_diagnostics(&self) -> DailyCityFlowDiagnostics {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;

        let mut diagnostics = DailyCityFlowDiagnostics::default();
        let catalog = load_runtime_economy_catalog().ok();

        for (building_idx, building) in self.allocator.buildings.iter().enumerate() {
            if matches!(building.zone_type, ZoneType::Residential) {
                let household_capacity = self.allocator.household_capacity(building_idx);
                diagnostics.total_household_slots = diagnostics
                    .total_household_slots
                    .saturating_add(household_capacity);
                diagnostics.vacant_household_slots =
                    diagnostics.vacant_household_slots.saturating_add(
                        household_capacity
                            .saturating_sub(building.occupancy.min(household_capacity)),
                    );
            }

            let worker_capacity = catalog
                .as_ref()
                .map(|catalog| {
                    self.allocator
                        .worker_capacity_with_catalog(building_idx, catalog.as_ref())
                })
                .unwrap_or_else(|| self.allocator.worker_capacity(building_idx));
            match building.zone_type {
                ZoneType::Commercial => {
                    diagnostics.commercial_job_capacity = diagnostics
                        .commercial_job_capacity
                        .saturating_add(worker_capacity);
                }
                ZoneType::Industrial => {
                    diagnostics.industrial_job_capacity = diagnostics
                        .industrial_job_capacity
                        .saturating_add(worker_capacity);
                }
                _ => {}
            }
        }

        for household in &self.households.households {
            if household.member_count == 0 {
                continue;
            }
            diagnostics.active_households = diagnostics.active_households.saturating_add(1);
            let live_home = self
                .allocator
                .buildings
                .get(household.home_building_id)
                .is_some_and(|building| {
                    !building.broken
                        && !building.economy_broken
                        && !building.is_deserted
                        && building.is_operational()
                });
            if live_home {
                diagnostics.housed_households = diagnostics.housed_households.saturating_add(1);
            } else {
                diagnostics.unhoused_households = diagnostics.unhoused_households.saturating_add(1);
            }
            if household.budget <= f32::EPSILON {
                diagnostics.zero_budget_households =
                    diagnostics.zero_budget_households.saturating_add(1);
            }
            if household.stock_days <= f32::EPSILON {
                diagnostics.stock_empty_households =
                    diagnostics.stock_empty_households.saturating_add(1);
            }
            if household.stock_days <= 1.0 {
                diagnostics.stock_low_households =
                    diagnostics.stock_low_households.saturating_add(1);
            }
        }

        for agent_idx in 0..self.agents.len() {
            if self.agents.pending_household_size[agent_idx] > 0 {
                diagnostics.pending_household_carriers =
                    diagnostics.pending_household_carriers.saturating_add(1);
                continue;
            }
            let household_id = self.agents.household_id[agent_idx];
            if household_id == usize::MAX || household_id >= self.households.households.len() {
                continue;
            }
            diagnostics.resident_agents = diagnostics.resident_agents.saturating_add(1);
            match self.agents.age_group[agent_idx] {
                AGE_CHILD => {
                    diagnostics.child_agents = diagnostics.child_agents.saturating_add(1);
                }
                AGE_ADULT => {
                    diagnostics.adult_agents = diagnostics.adult_agents.saturating_add(1);
                }
                AGE_ELDER => {
                    diagnostics.elder_agents = diagnostics.elder_agents.saturating_add(1);
                }
                _ => {}
            }

            if !age_group_can_work(self.agents.age_group[agent_idx]) {
                continue;
            }

            let work_building = self.agents.work_building[agent_idx];
            if work_building >= self.allocator.buildings.len() {
                diagnostics.unemployed_agents = diagnostics.unemployed_agents.saturating_add(1);
                continue;
            }
            let worker_capacity = catalog
                .as_ref()
                .map(|catalog| {
                    self.allocator
                        .worker_capacity_with_catalog(work_building, catalog.as_ref())
                })
                .unwrap_or_else(|| self.allocator.worker_capacity(work_building));
            if worker_capacity == 0 {
                diagnostics.unemployed_agents = diagnostics.unemployed_agents.saturating_add(1);
                continue;
            }

            diagnostics.employed_agents = diagnostics.employed_agents.saturating_add(1);
            match self.allocator.buildings[work_building].zone_type {
                ZoneType::Commercial => {
                    diagnostics.commercial_filled_jobs =
                        diagnostics.commercial_filled_jobs.saturating_add(1);
                }
                ZoneType::Industrial => {
                    diagnostics.industrial_filled_jobs =
                        diagnostics.industrial_filled_jobs.saturating_add(1);
                }
                _ => {}
            }
        }

        diagnostics
    }

    fn log_daily_city_flow_diagnostics(&self, day_index: u32, removed_households: u32) {
        if !crate::debug::category_enabled("economy") {
            return;
        }

        let diagnostics = self.daily_city_flow_diagnostics();
        let total_job_capacity = diagnostics
            .commercial_job_capacity
            .saturating_add(diagnostics.industrial_job_capacity);
        let filled_jobs = diagnostics
            .commercial_filled_jobs
            .saturating_add(diagnostics.industrial_filled_jobs);
        let open_jobs = total_job_capacity.saturating_sub(filled_jobs);
        let commercial_open_jobs = diagnostics
            .commercial_job_capacity
            .saturating_sub(diagnostics.commercial_filled_jobs);
        let industrial_open_jobs = diagnostics
            .industrial_job_capacity
            .saturating_sub(diagnostics.industrial_filled_jobs);
        let occupied_household_slots = diagnostics
            .total_household_slots
            .saturating_sub(diagnostics.vacant_household_slots);
        let net_households =
            self.debug_household_admissions_since_daily as i32 - removed_households as i32;

        debug_log!(
            "economy",
            "city flow diagnostics: day={} net_households={:+} admitted_since_daily={} \
             removed_today={} households={} housed={} unhoused={} zero_budget={} \
             stock_empty={} stock_low={} resident_agents={} pending_carriers={} \
             children={} adults={} elders={} employed={} unemployed={} jobs={}/{} open_jobs={} \
             commercial_jobs={}/{} commercial_open={} industrial_jobs={}/{} industrial_open={} \
             homes={}/{} vacant_homes={} treasury={:.0} taxes=(income={:.1} household_vat={:.1} \
             business_purchase={:.1} business_profit={:.1} property={:.1} lifetime={:.1})",
            day_index,
            net_households,
            self.debug_household_admissions_since_daily,
            removed_households,
            diagnostics.active_households,
            diagnostics.housed_households,
            diagnostics.unhoused_households,
            diagnostics.zero_budget_households,
            diagnostics.stock_empty_households,
            diagnostics.stock_low_households,
            diagnostics.resident_agents,
            diagnostics.pending_household_carriers,
            diagnostics.child_agents,
            diagnostics.adult_agents,
            diagnostics.elder_agents,
            diagnostics.employed_agents,
            diagnostics.unemployed_agents,
            filled_jobs,
            total_job_capacity,
            open_jobs,
            diagnostics.commercial_filled_jobs,
            diagnostics.commercial_job_capacity,
            commercial_open_jobs,
            diagnostics.industrial_filled_jobs,
            diagnostics.industrial_job_capacity,
            industrial_open_jobs,
            occupied_household_slots,
            diagnostics.total_household_slots,
            diagnostics.vacant_household_slots,
            self.treasury.balance,
            self.treasury.last_daily_income_tax,
            self.treasury.last_daily_household_vat,
            self.treasury.last_daily_business_purchase_tax,
            self.treasury.last_daily_business_profit_tax,
            self.treasury.last_daily_property_tax,
            self.treasury.lifetime_tax_revenue,
        );
    }

    fn print_daily_building_economy(&mut self, day_index: u32) {
        use crate::simulation::economy::definitions::load_runtime_economy_catalog;

        if !crate::debug::category_enabled("economy") {
            self.households.reset_daily_ledgers();
            return;
        }
        let Ok(catalog) = load_runtime_economy_catalog() else {
            self.households.reset_daily_ledgers();
            return;
        };

        for (idx, b) in self.allocator.buildings.iter().enumerate() {
            if b.zone_type == ZoneType::Residential {
                continue;
            }
            let zone_tag = match b.zone_type {
                ZoneType::Residential => "RES",
                ZoneType::Commercial => "COM",
                ZoneType::Industrial => "IND",
                _ => "OTHER",
            };
            let worker_cap = self
                .allocator
                .worker_capacity_with_catalog(idx, catalog.as_ref());
            let _resident_cap = self.allocator.household_capacity(idx);
            let profile_id = catalog
                .profile_by_runtime_id(b.economy_profile_runtime_id)
                .map(|p| p.id.as_str())
                .unwrap_or("none");

            // Build inventory snapshot string for all non-zero resources.
            let mut inv_parts = Vec::new();
            for (slot, &amount) in b.resource_inventory.iter().enumerate() {
                if amount <= 0.0 {
                    continue;
                }
                let rid = (slot + 1) as u16;
                let name = catalog.resource_id_for_runtime_id(rid).unwrap_or("?");
                // capacity from output port if available
                let cap =
                    if let Some(p) = catalog.profile_by_runtime_id(b.economy_profile_runtime_id) {
                        p.outputs
                            .iter()
                            .find(|o| o.resource_runtime_id == rid)
                            .map(|o| p.output_buffer_capacity_units_for(o))
                            .unwrap_or(0.0)
                    } else {
                        0.0
                    };
                if cap > 0.0 {
                    inv_parts.push(format!("{}={:.1}/{:.1}", name, amount, cap));
                } else {
                    inv_parts.push(format!("{}={:.1}", name, amount));
                }
            }
            let inv_str = if inv_parts.is_empty() {
                "none".to_owned()
            } else {
                inv_parts.join(" ")
            };

            // Daily I/O from profile (per-day throughput at full capacity).
            let mut io_parts = Vec::new();
            if let Some(p) = catalog.profile_by_runtime_id(b.economy_profile_runtime_id) {
                for port in &p.inputs {
                    let name = catalog
                        .resource_id_for_runtime_id(port.resource_runtime_id)
                        .unwrap_or("?");
                    io_parts.push(format!("-{:.1}{}/day", port.units_per_day, name));
                }
                for port in &p.outputs {
                    let name = catalog
                        .resource_id_for_runtime_id(port.resource_runtime_id)
                        .unwrap_or("?");
                    io_parts.push(format!("+{:.1}{}/day", port.units_per_day, name));
                }
                if p.utility_service.as_deref() == Some("power") {
                    io_parts.push(format!(
                        "power_out_today={:.1}",
                        b.recent_power_service_units
                    ));
                }
            }
            let io_str = if io_parts.is_empty() {
                "none".to_owned()
            } else {
                io_parts.join(" ")
            };

            println!(
                "[ECON] Day {:>4} idx={} {} asset={} profile={} workers={}/{} budget={:.1} revenue={:.1} distress={} broken={} io=[{}] inventory=[{}]",
                day_index,
                idx,
                zone_tag,
                b.asset_id,
                profile_id,
                b.worker_count,
                worker_cap,
                b.operating_budget,
                b.revenue,
                if b.budget_distress { "Y" } else { "N" },
                if b.broken || b.economy_broken {
                    "Y"
                } else {
                    "N"
                },
                io_str,
                inv_str,
            );
        }

        let mut households_at_budget_floor = 0u32;
        let mut households_below_1d_stock = 0u32;
        let mut households_below_2d_stock = 0u32;
        let mut households_below_3d_stock = 0u32;
        let mut total_wages_paid = 0.0f32;
        let mut total_household_shopping_spend = 0.0f32;
        let mut total_benefits_paid = 0.0f32;
        let mut total_utility_stock_cost = 0.0f32;
        let mut total_household_supply_use_cost = 0.0f32;
        let mut total_household_utility_cost = 0.0f32;

        for (idx, h) in self.households.households.iter().enumerate() {
            if h.member_count == 0 {
                continue;
            }
            let ledger = self
                .households
                .daily_ledgers()
                .get(idx)
                .copied()
                .unwrap_or_default();
            if h.budget <= f32::EPSILON {
                households_at_budget_floor += 1;
            }
            if h.stock_days < 1.0 {
                households_below_1d_stock += 1;
            }
            if h.stock_days < 2.0 {
                households_below_2d_stock += 1;
            }
            if h.stock_days < 3.0 {
                households_below_3d_stock += 1;
            }
            total_wages_paid += ledger.wage_income;
            total_household_shopping_spend += ledger.shopping_spend;
            total_benefits_paid += ledger.unemployment_benefit_income;
            total_utility_stock_cost += ledger.utility_stock_consumption_cost;
            total_household_supply_use_cost += ledger.household_supply_consumption_cost;
            let household_utility_cost = ledger.power_consumption_cost
                + ledger.water_consumption_cost
                + ledger.sewage_consumption_cost;
            total_household_utility_cost += household_utility_cost;
            let home_asset = self
                .allocator
                .buildings
                .get(h.home_building_id)
                .map(|b| b.asset_id.as_str())
                .unwrap_or("none");

            let state_str = match h.replenishment_state {
                0 => "STABLE",
                1 => "NEEDS",
                2 => "WAITING_SHOPPER",
                3 => "SHOPPING_TO_STORE",
                4 => "SHOPPING_RETURNING",
                5 => "FULFILLED",
                6 => "COOLDOWN",
                7 => "FAILED_TERMINAL",
                _ => "UNKNOWN",
            };

            let ub_str = if h.unemployment_days_elapsed > 0 {
                format!(" ub={}d", h.unemployment_days_elapsed)
            } else {
                String::new()
            };
            println!(
                "[ECON] Day {:>4} HH:{:<2} home_idx={:<2} asset={} residents={} children={} adults={} elders={} budget={:<5.1} stock={:<4.2}days state={}{} ledger=(before={:.1} wage={:.1} benefit={:.1} shopping={:.1} power={:.1} water={:.1} sewage={:.1} utility={:.1} stock_use={:.1} utility_stock={:.1} after={:.1} unemployed_adults={} shopper_trips={}/{})",
                day_index,
                idx,
                h.home_building_id,
                home_asset,
                h.member_count,
                h.child_count,
                h.adult_count,
                h.elder_count,
                h.budget,
                h.stock_days,
                state_str,
                ub_str,
                ledger.budget_before,
                ledger.wage_income,
                ledger.unemployment_benefit_income,
                ledger.shopping_spend,
                ledger.power_consumption_cost,
                ledger.water_consumption_cost,
                ledger.sewage_consumption_cost,
                household_utility_cost,
                ledger.household_supply_consumption_cost,
                ledger.utility_stock_consumption_cost,
                ledger.budget_after,
                ledger.unemployed_adults,
                ledger.shopper_trips_completed,
                ledger.shopper_trips_failed,
            );
        }
        println!(
            "[ECON] Day {:>4} household ledger summary: budget_floor={} stock_below_1d={} stock_below_2d={} stock_below_3d={} wages_paid={:.1} shopping_spend={:.1} benefits_paid={:.1} utility_cost={:.1} stock_use_cost={:.1} utility_stock_cost={:.1}",
            day_index,
            households_at_budget_floor,
            households_below_1d_stock,
            households_below_2d_stock,
            households_below_3d_stock,
            total_wages_paid,
            total_household_shopping_spend,
            total_benefits_paid,
            total_household_utility_cost,
            total_household_supply_use_cost,
            total_utility_stock_cost,
        );
        println!(
            "[ECON] Day {:>4} fiscal summary: income_tax={:.1} household_vat={:.1} business_purchase_tax={:.1} business_profit_tax={:.1} property_tax={:.1} tax_total={:.1} lifetime_tax={:.1} road_upkeep={:.1} treasury={:.1}",
            day_index,
            self.treasury.last_daily_income_tax,
            self.treasury.last_daily_household_vat,
            self.treasury.last_daily_business_purchase_tax,
            self.treasury.last_daily_business_profit_tax,
            self.treasury.last_daily_property_tax,
            self.treasury.last_daily_income_tax
                + self.treasury.last_daily_household_vat
                + self.treasury.last_daily_business_purchase_tax
                + self.treasury.last_daily_business_profit_tax
                + self.treasury.last_daily_property_tax,
            self.treasury.lifetime_tax_revenue,
            self.treasury.last_daily_upkeep,
            self.treasury.balance,
        );
        self.households.reset_daily_ledgers();
    }

    /// Executes one coarse operational-hour economy step before the daily settlement boundary.
    pub fn simulate_operational_hour_internal(&mut self, day_index: u32, minute_of_day: u16) {
        let absolute_hour = day_index
            .saturating_sub(1)
            .saturating_mul(24)
            .saturating_add(u32::from(minute_of_day / 60));
        self.allocator.advance_construction_hour();
        let service_funding_by_building = self.electricity_funding_by_building();
        let fiscal_revenue = self.households.operational_hour_tick(
            &mut self.agents,
            &mut self.allocator,
            &mut self.logistics,
            &self.transit_network,
            &self.region_graph,
            absolute_hour,
            minute_of_day,
            &mut self.treasury.balance,
            &service_funding_by_building,
        );
        self.collect_fiscal_revenue(fiscal_revenue);
        if minute_of_day != 0 {
            self.execute_hourly_demand_pass(day_index, minute_of_day, &service_funding_by_building);
        }
    }

    fn enqueue_hourly_demand_spawns(&mut self, day_index: u32, minute_of_day: u16) -> usize {
        let now = absolute_operational_minute(day_index, minute_of_day);
        let mut next_due_minute = self
            .pending_demand_spawns
            .back()
            .map(|pending| pending.due_minute.saturating_add(1))
            .unwrap_or_else(|| now.saturating_add(1))
            .max(now.saturating_add(1));
        let first_due_minute = next_due_minute;
        let mut queued = 0_usize;

        for (zone_type, spawns) in [
            (
                ZoneType::Residential,
                &self.demand.building_actions.residential.spawns,
            ),
            (
                ZoneType::Commercial,
                &self.demand.building_actions.commercial.spawns,
            ),
            (
                ZoneType::Industrial,
                &self.demand.building_actions.industrial.spawns,
            ),
        ] {
            for action in spawns {
                self.pending_demand_spawns
                    .push_back(PendingDemandSpawnAction {
                        due_minute: next_due_minute,
                        zone_type,
                        action: action.clone(),
                        planned_day_index: day_index,
                        planned_minute_of_day: minute_of_day,
                    });
                queued += 1;
                if queued % DEMAND_SPAWN_ACTIONS_PER_MINUTE == 0 {
                    next_due_minute = next_due_minute.saturating_add(1);
                }
            }
        }

        if queued > 0 {
            debug_log!(
                "economy",
                "queued demand spawns: day={} minute={} queued={} pending_total={} first_due={} last_due={}",
                day_index,
                minute_of_day,
                queued,
                self.pending_demand_spawns.len(),
                first_due_minute,
                next_due_minute.saturating_sub(1),
            );
        }
        queued
    }

    fn execute_pending_demand_spawns_for_minute(
        &mut self,
        day_index: u32,
        minute_of_day: u16,
    ) -> usize {
        let now = absolute_operational_minute(day_index, minute_of_day);
        let mut executed_this_minute = 0_usize;
        while executed_this_minute < DEMAND_SPAWN_ACTIONS_PER_MINUTE {
            let Some(pending) = self.pending_demand_spawns.front() else {
                return executed_this_minute;
            };
            if pending.due_minute > now {
                return executed_this_minute;
            }
            let pending = self
                .pending_demand_spawns
                .pop_front()
                .expect("pending spawn front existed");
            self.execute_pending_demand_spawn(pending, day_index, minute_of_day);
            executed_this_minute += 1;
        }
        executed_this_minute
    }

    fn execute_pending_demand_spawn(
        &mut self,
        pending: PendingDemandSpawnAction,
        day_index: u32,
        minute_of_day: u16,
    ) {
        let total_start = Instant::now();
        let compile_start = Instant::now();
        self.transit_network.road_surface.compile_dirty_with_reason(
            &self.region_graph,
            &self.heightmap,
            RoadSurfaceCompileReason::SimCommit,
        );
        let compile_ms = compile_start.elapsed().as_secs_f64() * 1000.0;
        let execute_start = Instant::now();
        let execution = self.allocator.execute_single_demand_spawn_action(
            pending.zone_type,
            &pending.action,
            &mut self.zoning,
            &self.region_graph,
            &self.transit_network.lane_system,
            &self.transit_network.road_surface,
            &self.heightmap,
            self.demand.runtime_catalog(),
            self.demand.runtime_tuning(),
        );
        let execute_ms = execute_start.elapsed().as_secs_f64() * 1000.0;
        self.treasury
            .collect_property_tax(execution.property_tax_paid as f64);
        if let Some(bounds) = execution.site_dirty_bounds {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        let total_ms = total_start.elapsed().as_secs_f64() * 1000.0;

        let use_execution = match pending.zone_type {
            ZoneType::Residential => execution.residential,
            ZoneType::Commercial => execution.commercial,
            ZoneType::Industrial => execution.industrial,
            _ => return,
        };
        debug_log!(
            "economy",
            "queued demand spawn execution: planned_day={} planned_minute={} executed_day={} executed_minute={} zone={:?} attempted={} placed={} failed={} pending_remaining={} compile_ms={:.3} execute_ms={:.3} total_ms={:.3}",
            pending.planned_day_index,
            pending.planned_minute_of_day,
            day_index,
            minute_of_day,
            pending.zone_type,
            use_execution.spawn_attempted,
            use_execution.spawn_executed,
            use_execution.spawn_rejections.total(),
            self.pending_demand_spawns.len(),
            compile_ms,
            execute_ms,
            total_ms,
        );
    }

    /// Executes one full economy / daily tick (called once per in-game day).
    pub fn simulate_tick_internal(&mut self, day_index: u32) {
        let tick_start = Instant::now();

        debug_log!(
            "economy",
            "daily tick start: buildings={} households={} agents={}",
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|h| h.member_count > 0)
                .count(),
            self.agents.len(),
        );
        self.allocator.tick(
            &mut self.zoning,
            &mut self.agents,
            &mut self.households,
            &mut self.logistics,
            &mut self.transit_network,
            &mut self.region_graph,
        );
        if let Some(bounds) = self.allocator.take_pending_site_dirty_bounds() {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        // Drain building dirty-zone flags → mark matching flow fields for rebuild.
        {
            use crate::simulation::buildings::allocator::BASELINE_PRIVATE_ZONES;
            for (zone_idx, zone) in BASELINE_PRIVATE_ZONES.iter().enumerate() {
                if self.allocator.dirty_zones[zone_idx] {
                    self.allocator.dirty_zones[zone_idx] = false;
                    self.transit_network.flow_fields.mark_zone_dirty(*zone);
                }
            }
        }

        self.pollution.tick(&self.allocator, &self.config);
        self.noise
            .tick(&self.allocator, &self.region_graph, &self.config);
        self.desirability
            .tick(&self.zoning, &self.pollution, &self.noise);
        let service_funding_by_building = self.electricity_funding_by_building();
        let fiscal_revenue = self.households.daily_settlement_tick(
            &mut self.agents,
            &mut self.allocator,
            &self.logistics,
            &self.transit_network,
            &self.region_graph,
            &mut self.treasury.balance,
            &service_funding_by_building,
        );
        self.collect_fiscal_revenue(fiscal_revenue);
        // City treasury: settle daily road upkeep on the fiscal cadence.
        let road_length_m: f64 = self
            .region_graph
            .edges()
            .iter()
            .filter(|e| !e.deleted)
            .map(|e| e.physical_length as f64)
            .sum();
        self.treasury
            .settle_daily_upkeep(road_length_m * ROAD_UPKEEP_PER_METER_PER_DAY);
        self.demand.run_daily_pass_with_service_funding(
            &self.allocator,
            &self.households,
            &self.region_graph,
            &self.zoning,
            self.treasury.balance,
            &service_funding_by_building,
        );
        let removed_households = self.households.execute_demand_household_removal(
            self.demand.households_to_remove_today,
            &mut self.agents,
            &mut self.allocator,
            &mut self.logistics,
        );
        self.demand
            .record_household_removal_execution(removed_households);
        self.demand
            .log_daily_household_action_diagnostics(day_index);
        self.execute_hourly_demand_pass(day_index, 0, &service_funding_by_building);
        // Minute 0 is the deterministic closing boundary: operational-hour work,
        // daily settlement, and midnight demand all post before the daily tax
        // buckets roll into the report.
        self.treasury.finalize_daily_tax_window();
        self.record_daily_budget_ledger(day_index);
        self.log_daily_city_flow_diagnostics(day_index, removed_households);
        self.debug_household_admissions_since_daily = 0;
        // Reset OWA/local input accumulators after the daily and midnight demand snapshots have
        // been taken.
        self.allocator.reset_daily_input_accumulators();
        debug_log!(
            "economy",
            "daily tick end: buildings={} households={} agents={} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} remove={} planned_spawns=({}/{}/{}) upgrades=({}/{}/{}) downgrades=({}/{}/{}) despawns=({}/{}/{}) treasury={:.0}",
            self.allocator.buildings.len(),
            self.households
                .households
                .iter()
                .filter(|h| h.member_count > 0)
                .count(),
            self.agents.len(),
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.demand.households_to_remove_today,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
            self.treasury.balance,
        );
        self.agents.daily_update(&self.pollution, &self.config);
        self.agents
            .pathfind_count
            .store(0, std::sync::atomic::Ordering::Relaxed);

        self.last_tick_duration = tick_start.elapsed().as_secs_f64() * 1000.0;
    }

    fn execute_hourly_demand_pass(
        &mut self,
        day_index: u32,
        minute_of_day: u16,
        service_funding_by_building: &[f32],
    ) {
        self.demand.run_hourly_pass_with_service_funding(
            &self.allocator,
            &self.households,
            &self.region_graph,
            &self.zoning,
            self.treasury.balance,
            service_funding_by_building,
        );
        let launched_households = self.allocator.execute_demand_household_admission(
            self.demand.households_to_admit_today,
            &mut self.agents,
            &self.transit_network,
            &self.region_graph,
        );
        self.debug_household_admissions_since_daily = self
            .debug_household_admissions_since_daily
            .saturating_add(launched_households);
        self.demand
            .record_household_admission_execution(launched_households);
        let queued_spawn_count = self.enqueue_hourly_demand_spawns(day_index, minute_of_day);
        let immediate_building_plan = demand_plan_without_spawns(&self.demand.building_actions);
        let building_action_execution =
            if demand_plan_has_non_spawn_actions(&immediate_building_plan) {
                self.allocator.execute_demand_building_actions(
                    &immediate_building_plan,
                    &mut self.zoning,
                    &mut self.agents,
                    &mut self.households,
                    &mut self.logistics,
                    &self.region_graph,
                    &self.transit_network.lane_system,
                    &self.transit_network.road_surface,
                    &self.heightmap,
                    self.demand.runtime_catalog(),
                    self.demand.runtime_tuning(),
                )
            } else {
                Default::default()
            };
        self.treasury
            .collect_property_tax(building_action_execution.property_tax_paid as f64);
        if let Some(bounds) = building_action_execution.site_dirty_bounds {
            self.mark_building_site_terrain_dirty_bounds(bounds);
        }
        self.demand
            .log_hourly_household_action_diagnostics(day_index, minute_of_day);
        self.demand
            .log_hourly_building_action_diagnostics(day_index, minute_of_day);
        for (use_label, execution) in [
            ("Residential", &building_action_execution.residential),
            ("Commercial", &building_action_execution.commercial),
            ("Industrial", &building_action_execution.industrial),
        ] {
            let rejections = execution.spawn_rejections;
            if execution.spawn_attempted == 0 && rejections.total() == 0 {
                continue;
            }
            debug_log!(
                "economy",
                "building action execution: day={} minute={} use={} \
                 spawn_attempted={} spawn_placed={} spawn_failed={} \
                 spawn_failed_geometry={} fail_asset={} fail_parcel={} fail_slot={} \
                 fail_driveway_surface={} fail_driveway_height={} fail_driveway_connection={} \
                 fail_frontage_surface={} fail_neighbor_height={} fail_site_tie_in={}",
                day_index,
                minute_of_day,
                use_label,
                execution.spawn_attempted,
                execution.spawn_executed,
                rejections.total(),
                rejections.geometry_total(),
                rejections.asset_unavailable,
                rejections.parcel_unavailable,
                rejections.slot_unavailable,
                rejections.driveway_road_surface_missing,
                rejections.driveway_height_conflict,
                rejections.driveway_connection_missing,
                rejections.frontage_road_surface_missing,
                rejections.neighbor_site_height_conflict,
                rejections.site_support_tie_in_invalid,
            );
        }
        debug_log!(
            "economy",
            "hourly demand: day={} minute={} demand=(R {:+.0}%, C {:+.0}%, I {:+.0}%) admit={} planned_spawns=({}/{}/{}) queued_spawns={} pending_spawns={} placed_spawns=({}/{}/{}) upgrades=({}/{}/{}) downgrades=({}/{}/{}) despawns=({}/{}/{})",
            day_index,
            minute_of_day,
            self.demand.net_residential_pressure() * 100.0,
            self.demand.net_commercial_pressure() * 100.0,
            self.demand.net_industrial_pressure() * 100.0,
            self.demand.households_to_admit_today,
            self.demand.building_actions.residential.spawns.len(),
            self.demand.building_actions.commercial.spawns.len(),
            self.demand.building_actions.industrial.spawns.len(),
            queued_spawn_count,
            self.pending_demand_spawns.len(),
            building_action_execution.residential.spawn_executed,
            building_action_execution.commercial.spawn_executed,
            building_action_execution.industrial.spawn_executed,
            self.demand.building_actions.residential.upgrades.len(),
            self.demand.building_actions.commercial.upgrades.len(),
            self.demand.building_actions.industrial.upgrades.len(),
            self.demand.building_actions.residential.downgrades.len(),
            self.demand.building_actions.commercial.downgrades.len(),
            self.demand.building_actions.industrial.downgrades.len(),
            self.demand.building_actions.residential.despawns.len(),
            self.demand.building_actions.commercial.despawns.len(),
            self.demand.building_actions.industrial.despawns.len(),
        );
    }

    pub(crate) fn mark_building_site_terrain_dirty_bounds(&mut self, bounds: (f32, f32, f32, f32)) {
        let margin_m =
            terrain_cdt_local_sample_margin_m(&self.heightmap, ROAD_LOCKED_TERRAIN_RENDER_STEP_M);
        self.allocator.mark_building_site_terrain_bounds_dirty(
            &mut self.heightmap,
            bounds,
            margin_m,
        );
        let (min_x, min_z, max_x, max_z) = bounds;
        let dirty_patch_keys = self.heightmap.render_patch_keys_for_world_bounds(
            min_x - margin_m,
            min_z - margin_m,
            max_x + margin_m,
            max_z + margin_m,
        );
        self.refined_terrain_patch_cache
            .retain(|key, _| !dirty_patch_keys.contains(&(key.patch_x, key.patch_z)));
        self.terrain_dirty = true;
    }

    fn collect_fiscal_revenue(&mut self, revenue: FiscalRevenue) {
        self.treasury.collect_income_tax(revenue.income_tax as f64);
        self.treasury
            .collect_household_vat(revenue.household_vat as f64);
        self.treasury
            .collect_business_purchase_tax(revenue.business_purchase_tax as f64);
        self.treasury
            .collect_business_profit_tax(revenue.business_profit_tax as f64);
        self.treasury
            .collect_property_tax(revenue.property_tax as f64);
    }

    /// Called once per in-game day by the tick loop to emit per-building economy lines.
    pub fn print_daily_building_economy_for_day(&mut self, day_index: u32) {
        self.print_daily_building_economy(day_index);
    }

    /// Pre-computes all per-frame rendering data into a `RenderSnapshot`.
    ///
    /// Called from the background thread at the end of every movement tick.
    /// Uses only pure Rust types so the resulting snapshot is `Send`.
    pub fn build_snapshot(&self) -> RenderSnapshot {
        use crate::simulation::economy::agents::{TRANSIT_ACCESS_EGRESS, TRANSIT_ACCESS_INGRESS};

        let mut pedestrian_transforms: HashMap<u8, Vec<f32>> = HashMap::new();
        let mut car_transforms: HashMap<u8, Vec<f32>> = HashMap::new();
        let mut car_render_ids: HashMap<u8, Vec<i64>> = HashMap::new();

        let (aabb_x_min, aabb_x_max, aabb_z_min, aabb_z_max) = self.camera_aabb;
        let cull = aabb_x_min < aabb_x_max; // false when default "show all"

        for i in 0..self.agents.len() {
            if !transit_is_visible(self.agents.transit[i]) {
                continue;
            }

            let mut world_x = self.agents.pos_x[i];
            let mut world_z = self.agents.pos_y[i];
            let mut lane_pose = None;
            let mut pedestrian_lane_surface_y = None;
            let lane_id = self.agents.current_lane_id[i];
            if lane_id != usize::MAX && lane_id < self.transit_network.lane_system.lanes.len() {
                let lane = &self.transit_network.lane_system.lanes[lane_id];
                lane_pose = sample_lane_pose(lane, self.agents.lane_distance[i]);
                if let Some((pos, _)) = lane_pose {
                    world_x = pos.x;
                    world_z = pos.z;
                    pedestrian_lane_surface_y = Some(pedestrian_lane_surface_height(lane, pos.y));
                }
            }

            if cull
                && (world_x < aabb_x_min
                    || world_x > aabb_x_max
                    || world_z < aabb_z_min
                    || world_z > aabb_z_max)
            {
                continue;
            }
            if self.agents.transit_mode[i] != MODE_CAR {
                // Pedestrian / walker — use variant MMI and oriented basis.
                let p_type = self.agents.pedestrian_type[i];
                let walk_cycle = self.agents.walk_phase[i];
                let buffer = pedestrian_transforms.entry(p_type).or_default();

                let mut basis_x = Vector3::RIGHT;
                let mut basis_y = Vector3::UP;
                let mut basis_z = Vector3::BACK;
                let world_y = pedestrian_lane_surface_y.unwrap_or_else(|| {
                    if pedestrian_needs_access_surface(self.agents.transit[i]) {
                        // Door-to-curb walkers are off-lane; keep this point query allocation-free.
                        pedestrian_access_surface_height(self, world_x, world_z)
                    } else {
                        self.heightmap.sample_visual_height_world(world_x, world_z) * HEIGHT_SCALE
                    }
                }) + PEDESTRIAN_SURFACE_CLEARANCE_M;

                if let Some((_, tangent)) = lane_pose {
                    // GLTF export converts Blender -Y (character facing) to +Z, so the
                    // model faces +Z in Godot. basis_z = fwd aligns +Z with travel dir.
                    basis_z = tangent;
                    let right = Vector3::UP.cross(basis_z);
                    if right.length_squared() > 1e-6 {
                        basis_x = right.normalized();
                        basis_y = basis_z.cross(basis_x).normalized();
                    }
                } else {
                    let transit = self.agents.transit[i];
                    if transit == TRANSIT_ACCESS_EGRESS {
                        if let Some(target) = access_phase_target(self, i, true) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    } else if transit == TRANSIT_ACCESS_INGRESS {
                        if let Some(target) = access_phase_target(self, i, false) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    }
                }

                buffer.push(basis_x.x);
                buffer.push(basis_y.x);
                buffer.push(basis_z.x);
                buffer.push(world_x);
                buffer.push(basis_x.y);
                buffer.push(basis_y.y);
                buffer.push(basis_z.y);
                buffer.push(world_y);
                buffer.push(basis_x.z);
                buffer.push(basis_y.z);
                buffer.push(basis_z.z);
                buffer.push(world_z);

                // Add walk_phase in CUSTOM_DATA0.x (requires MultiMesh use_custom_data = true)
                buffer.push(walk_cycle);
                buffer.push(0.0);
                buffer.push(0.0);
                buffer.push(0.0);
            } else {
                // Car — oriented along lane geometry.
                let v_type = self.agents.vehicle_type[i];
                let render_id = self.agents.render_id[i];
                let variant_id = (render_id % 5) as u8;
                let model_key = (v_type * 10) + variant_id;
                let buffer = car_transforms.entry(model_key).or_default();
                car_render_ids
                    .entry(model_key)
                    .or_default()
                    .push(render_id.min(i64::MAX as u64) as i64);

                let mut basis_x = Vector3::RIGHT;
                let mut basis_y = Vector3::UP;
                let mut basis_z = Vector3::BACK;
                let terrain_y = self.heightmap.sample_height_world(world_x, world_z) * HEIGHT_SCALE;
                let mut world_y = terrain_y + 0.02;

                if let Some((pos, tangent)) = lane_pose {
                    world_y = pos.y + 0.02;
                    basis_z = -tangent;
                    let right = Vector3::UP.cross(basis_z);
                    if right.length_squared() > 1e-6 {
                        basis_x = right.normalized();
                        basis_y = basis_z.cross(basis_x).normalized();
                    }
                } else {
                    let transit = self.agents.transit[i];
                    if transit == TRANSIT_ACCESS_EGRESS {
                        if let Some(target) = access_phase_target(self, i, true) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = -dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    } else if transit == TRANSIT_ACCESS_INGRESS {
                        if let Some(target) = access_phase_target(self, i, false) {
                            let dir = Vector3::new(target.x - world_x, 0.0, target.z - world_z);
                            if dir.length_squared() > 1e-6 {
                                basis_z = -dir.normalized();
                                let right = Vector3::UP.cross(basis_z);
                                if right.length_squared() > 1e-6 {
                                    basis_x = right.normalized();
                                    basis_y = basis_z.cross(basis_x).normalized();
                                }
                            }
                        }
                    }
                }

                buffer.push(basis_x.x);
                buffer.push(basis_y.x);
                buffer.push(basis_z.x);
                buffer.push(world_x);
                buffer.push(basis_x.y);
                buffer.push(basis_y.y);
                buffer.push(basis_z.y);
                buffer.push(world_y);
                buffer.push(basis_x.z);
                buffer.push(basis_y.z);
                buffer.push(basis_z.z);
                buffer.push(world_z);
            }
        }

        let node_positions: Vec<godot::prelude::Vector3> = self
            .region_graph
            .nodes()
            .iter()
            .enumerate()
            .filter(|(i, _)| {
                let node_id = *i as u32;
                self.region_graph.get_valid_node(node_id) == node_id
                    && self.region_graph.node_has_live_incident_edge(node_id)
            })
            .map(|(_, n)| n.pos)
            .collect();

        let (terrain_world_w, terrain_world_h) = self.heightmap.world_size();

        RenderSnapshot {
            pedestrian_transforms,
            car_transforms,
            car_render_ids,
            terrain_dirty: self.terrain_dirty,
            water_dirty: self.water_dirty,
            network_dirty: self.network_dirty,
            node_positions,
            current_day: self.time.day_index,
            current_minute_of_day: self.time.minute_of_day,
            last_tick_ms: self.last_tick_duration,
            last_agent_tick_us: self.last_agent_tick_us,
            pathfind_count: self
                .agents
                .pathfind_count
                .load(std::sync::atomic::Ordering::Relaxed),
            agent_count: self.agents.len() as i32,
            treasury_balance: self.treasury.balance,
            heightmap_width: self.heightmap.width,
            heightmap_height: self.heightmap.height,
            terrain_world_size: godot::prelude::Vector2::new(terrain_world_w, terrain_world_h),
            zoning_overlay_revision: self.zoning.overlay_revision(),
            zoning_overlay_occupancy_revision: self.zoning.overlay_occupancy_revision(),
        }
    }
}

/// Background simulation thread loop.
///
/// Runs at ~60 Hz, decoupled from Godot's render frame. The `core` mutex is held
/// for the duration of each movement tick; main-thread `#[func]` calls block for
/// at most one tick duration while the lock is held (~7 ms at 100 k agents).
/// After the tick the snapshot is written while the lock is *not* held, so render
/// reads are completely non-blocking.
pub(crate) fn run_sim_thread(
    core: Arc<Mutex<SimCore>>,
    snapshot: Arc<RwLock<RenderSnapshot>>,
    road_preview_context: Arc<RwLock<RoadPreviewWorkerContext>>,
    road_query_snapshot: Arc<RwLock<RoadToolQuerySnapshot>>,
    cmd_rx: std::sync::mpsc::Receiver<SimCommand>,
) {
    const TARGET_DT: f64 = 1.0 / 60.0;
    let target = Duration::from_micros(16_667); // ~60 Hz

    loop {
        let frame_start = Instant::now();

        // Drain all pending commands — non-blocking.
        let command_start = Instant::now();
        let mut commands_processed = 0_usize;
        let mut set_speed_commands = 0_usize;
        let mut camera_aabb_commands = 0_usize;
        let mut add_road_commands = 0_usize;
        let mut pending_speed = None;
        let mut pending_camera_aabb = None;
        let mut should_quit = false;
        loop {
            match cmd_rx.try_recv() {
                Ok(SimCommand::Quit) => {
                    commands_processed += 1;
                    should_quit = true;
                    break;
                }
                Ok(SimCommand::SetSpeed(s)) => {
                    commands_processed += 1;
                    set_speed_commands += 1;
                    pending_speed = Some(s);
                }
                Ok(SimCommand::SetCameraAabb(x0, x1, z0, z1)) => {
                    commands_processed += 1;
                    camera_aabb_commands += 1;
                    pending_camera_aabb = Some((x0, x1, z0, z1));
                }
                Ok(SimCommand::AddRoad {
                    points,
                    fwd_lanes,
                    bkw_lanes,
                }) => {
                    commands_processed += 1;
                    add_road_commands += 1;
                    let road_total = Instant::now();
                    let lock_wait_start = Instant::now();
                    let (
                        preview_context,
                        query_snapshot,
                        road_lock_wait_ms,
                        add_internal_ms,
                        finalize_ms,
                        surface_ms,
                        mesh_ms,
                        snapshot_ms,
                        collect_refined_ms,
                        invalidated_refined_cache_entries,
                    ) = {
                        let mut c = core.lock().unwrap();
                        let road_lock_wait_ms = lock_wait_start.elapsed().as_secs_f64() * 1000.0;
                        // Bulk-load defers per-edge rebuilds until finalization.
                        let add_internal_start = Instant::now();
                        c.transit_network.bulk_load = true;
                        c.add_road_internal(points, fwd_lanes, bkw_lanes);
                        let add_internal_ms = add_internal_start.elapsed().as_secs_f64() * 1000.0;
                        let finalize_start = Instant::now();
                        {
                            let c = &mut *c;
                            c.transit_network.bulk_load = false;

                            // Take dirty edges first so we can derive the affected nodes
                            // for the incremental clips pass.
                            let dirty = std::mem::take(&mut c.transit_network.bulk_dirty_edges);
                            let dirty_count = dirty.len();

                            // Collect nodes touched by the new/split edges.
                            let mut affected_nodes = std::collections::HashSet::new();
                            for &e_id in &dirty {
                                if e_id < c.region_graph.edge_count()
                                    && !c.region_graph.edge(e_id).deleted
                                {
                                    let e = c.region_graph.edge(e_id);
                                    affected_nodes
                                        .insert(c.region_graph.get_valid_node(e.start_node));
                                    affected_nodes
                                        .insert(c.region_graph.get_valid_node(e.end_node));
                                }
                            }
                            if crate::debug::category_enabled("road")
                                && std::env::var("METRUM_DEBUG_ROAD_GEOMETRY_DUMP")
                                    .map(|value| !value.is_empty() && value != "0")
                                    .unwrap_or(false)
                            {
                                c.last_surface_debug_edges.extend(dirty.iter().copied());
                                c.last_surface_debug_edges.sort_unstable();
                                c.last_surface_debug_edges.dedup();
                            }

                            let t_clips = Instant::now();
                            c.region_graph
                                .rebuild_intersection_clips_for_nodes(&affected_nodes);
                            let dt_clips_us = t_clips.elapsed().as_micros();

                            let t_inv = Instant::now();
                            // Invalidate agents BEFORE lane rebuild so old lane IDs are still valid.
                            c.agents.invalidate_lane_ids_for_edges(
                                &dirty,
                                &c.transit_network.lane_system,
                            );
                            let dt_inv_us = t_inv.elapsed().as_micros();

                            let t_lanes = Instant::now();
                            c.transit_network
                                .lane_system
                                .rebuild_edges_incremental(&mut c.region_graph, &dirty);
                            let dt_lanes_us = t_lanes.elapsed().as_micros();
                            c.rebuild_building_entrances_internal();

                            // Rebuild CCH and run the connectivity check. This is the only
                            // place the CCH is actually rebuilt for road placements — the
                            // sim-tick path is gated on speed > 0.0 and would miss paused edits.
                            c.transit_network.rebuild_cch_and_check(&c.region_graph);
                            c.transit_network.cch_dirty_chunks.clear();

                            // Zone flush is deferred to the next simulate_tick_internal call
                            // so it does not block road placement. zoning_dirty_edges accumulates.

                            let total_us = road_total.elapsed().as_micros();
                            let msg = format!(
                                "TOTAL={}µs  {}  clips={}µs  lanes={}µs({}e)  invalidate={}µs",
                                total_us,
                                c.last_road_timing,
                                dt_clips_us,
                                dt_lanes_us,
                                dirty_count,
                                dt_inv_us
                            );
                            debug_log!("road", "{}", msg);
                            c.last_road_timing = msg;
                        }
                        let finalize_ms = finalize_start.elapsed().as_secs_f64() * 1000.0;
                        let surface_start = Instant::now();
                        c.rebuild_network_surface_terrain_internal_with_entrance_rebuild(false);
                        let surface_ms = surface_start.elapsed().as_secs_f64() * 1000.0;
                        let mesh_start = Instant::now();
                        c.precompute_road_mesh_data();
                        let mesh_ms = mesh_start.elapsed().as_secs_f64() * 1000.0;
                        c.bump_road_tool_surface_generation();
                        let snapshot_start = Instant::now();
                        let preview_context = RoadPreviewWorkerContext::from_core(&c);
                        let query_snapshot = RoadToolQuerySnapshot::from_core(&c);
                        let snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
                        let collect_refined_start = Instant::now();
                        let invalidated_refined_cache_entries = c
                            .refresh_road_locked_terrain_patch_state(
                                ROAD_LOCKED_TERRAIN_RENDER_STEP_M,
                            );
                        c.network_dirty = true;
                        let collect_refined_ms =
                            collect_refined_start.elapsed().as_secs_f64() * 1000.0;
                        (
                            preview_context,
                            query_snapshot,
                            road_lock_wait_ms,
                            add_internal_ms,
                            finalize_ms,
                            surface_ms,
                            mesh_ms,
                            snapshot_ms,
                            collect_refined_ms,
                            invalidated_refined_cache_entries,
                        )
                    };
                    let refined_input_count = 0usize;
                    let refined_window_count = 0usize;
                    let refined_reused_windows = 0usize;
                    *road_preview_context.write().unwrap() = preview_context;
                    *road_query_snapshot.write().unwrap() = query_snapshot;
                    if crate::debug::is_perf_enabled() {
                        println!(
                            "[DEBUG:perf] add_road_command total_ms={:.3} lock_wait_ms={:.3} add_internal_ms={:.3} finalize_ms={:.3} surface_ms={:.3} mesh_ms={:.3} snapshot_ms={:.3} collect_refined_ms={:.3} refined_build_ms={:.3} refined_cdt_sum_ms={:.3} refined_inputs={} refined_entries={} refined_windows={} refined_reused_windows={} refined_cache_invalidated={} insert_lock_wait_ms={:.3} insert_ms={:.3} refined_prebuild=skipped",
                            road_total.elapsed().as_secs_f64() * 1000.0,
                            road_lock_wait_ms,
                            add_internal_ms,
                            finalize_ms,
                            surface_ms,
                            mesh_ms,
                            snapshot_ms,
                            collect_refined_ms,
                            0.0,
                            0.0,
                            refined_input_count,
                            0,
                            refined_window_count,
                            refined_reused_windows,
                            invalidated_refined_cache_entries,
                            0.0,
                            0.0
                        );
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    should_quit = true;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
            }
        }
        let command_ms = command_start.elapsed().as_secs_f64() * 1000.0;
        if should_quit {
            return;
        }

        let perf_enabled = crate::debug::is_perf_enabled();
        let lock_wait_ms: f64;
        let mut pathing_ms = 0.0;
        let mut agent_ms = 0.0;
        let mut minute_ms = 0.0;
        let mut pending_spawn_ms = 0.0;
        let mut hourly_ms = 0.0;
        let mut daily_ms = 0.0;
        let snapshot_ms: f64;
        let lock_held_ms: f64;
        let mut elapsed_minutes = 0_u16;
        let mut pending_spawns_executed = 0_usize;
        let mut hourly_ticks = 0_usize;
        let mut daily_ticks = 0_usize;
        let agent_count: i32;
        let pathfind_count: u32;

        // Tick and build snapshot inside one lock acquisition.
        let new_snapshot = {
            // Recover from a poisoned mutex (caused by a previous tick panic) rather
            // than propagating a PoisonError cascade to every main-thread call.
            let lock_wait_start = Instant::now();
            let mut core = match core.lock() {
                Ok(g) => g,
                Err(e) => {
                    godot_error!("[sim] mutex was poisoned by a previous tick panic — recovering");
                    e.into_inner()
                }
            };
            lock_wait_ms = lock_wait_start.elapsed().as_secs_f64() * 1000.0;
            let lock_held_start = Instant::now();
            if let Some(speed) = pending_speed {
                core.time.speed_multiplier = speed;
            }
            if let Some(camera_aabb) = pending_camera_aabb {
                core.camera_aabb = camera_aabb;
            }
            let speed = core.time.speed_multiplier;

            if speed > 0.0 {
                // Rebuild CCH if dirty, then rebuild any dirty flow fields.
                let pathing_start = Instant::now();
                let c = &mut *core;
                c.transit_network
                    .rebuild_pathing_if_dirty(&mut c.region_graph);
                {
                    let alloc = &c.allocator;
                    let graph = &c.region_graph;
                    c.transit_network
                        .flow_fields
                        .rebuild_dirty(graph, |zone, mode_flags| {
                            alloc.get_sources_for_zone(zone, graph, mode_flags)
                        });
                }
                pathing_ms = pathing_start.elapsed().as_secs_f64() * 1000.0;

                let dt = (TARGET_DT * speed as f64) as f32;
                let t_agent = Instant::now();

                // Wrap the tick in catch_unwind so that a panic inside the agent loop
                // does NOT poison the mutex.  The MutexGuard stays alive in the outer
                // frame, so the lock is still held across the catch boundary.
                let tick_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let c = &mut *core;
                    c.agents.tick(
                        &c.allocator,
                        &mut c.transit_network,
                        &mut c.region_graph,
                        dt,
                        c.time.day_index,
                        c.time.minute_of_day,
                    );
                }));
                if let Err(e) = tick_result {
                    let msg = e
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("(non-string payload)");
                    godot_error!("[sim] tick panicked — skipping frame: {}", msg);
                }

                core.last_agent_tick_us = t_agent.elapsed().as_micros() as u64;
                agent_ms = core.last_agent_tick_us as f64 / 1000.0;

                let minute_start = Instant::now();
                let time_advance = core.time.process_delta(TARGET_DT);
                elapsed_minutes = time_advance.elapsed_minutes;
                if time_advance.has_elapsed_minutes() {
                    for (step_day_index, step_minute_of_day) in time_advance.iter_elapsed_minutes()
                    {
                        let pending_spawn_start = Instant::now();
                        let pending_spawn_result =
                            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                core.execute_pending_demand_spawns_for_minute(
                                    step_day_index,
                                    step_minute_of_day,
                                )
                            }));
                        match pending_spawn_result {
                            Ok(executed) => {
                                pending_spawns_executed += executed;
                            }
                            Err(e) => {
                                let msg = e
                                    .downcast_ref::<&str>()
                                    .copied()
                                    .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                                    .unwrap_or("(non-string payload)");
                                godot_error!(
                                    "[sim] demand spawn tick panicked — skipping minute: {}",
                                    msg
                                );
                            }
                        }
                        pending_spawn_ms += pending_spawn_start.elapsed().as_secs_f64() * 1000.0;
                        if step_minute_of_day % 60 == 0 {
                            let hourly_start = Instant::now();
                            let hourly_result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    core.simulate_operational_hour_internal(
                                        step_day_index,
                                        step_minute_of_day,
                                    )
                                }));
                            if let Err(e) = hourly_result {
                                let msg = e
                                    .downcast_ref::<&str>()
                                    .copied()
                                    .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                                    .unwrap_or("(non-string payload)");
                                godot_error!(
                                    "[sim] operational hour tick panicked — skipping hour: {}",
                                    msg
                                );
                            }
                            hourly_ms += hourly_start.elapsed().as_secs_f64() * 1000.0;
                            hourly_ticks += 1;
                            if step_minute_of_day != 0 && crate::debug::is_sim_enabled() {
                                core.print_sim_console_summary(step_day_index, step_minute_of_day);
                            }
                        }
                        if step_minute_of_day == 0 {
                            let daily_start = Instant::now();
                            let daily_result =
                                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                                    core.simulate_tick_internal(step_day_index)
                                }));
                            if let Err(e) = daily_result {
                                let msg = e
                                    .downcast_ref::<&str>()
                                    .copied()
                                    .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                                    .unwrap_or("(non-string payload)");
                                godot_error!("[sim] daily tick panicked — skipping day: {}", msg);
                            }
                            daily_ms += daily_start.elapsed().as_secs_f64() * 1000.0;
                            daily_ticks += 1;
                            if crate::debug::is_sim_enabled() {
                                core.print_sim_console_summary(step_day_index, step_minute_of_day);
                            }
                            core.print_daily_building_economy_for_day(step_day_index);
                        }
                    }
                }
                minute_ms = minute_start.elapsed().as_secs_f64() * 1000.0;
            }

            // build_snapshot only reads state; wrap anyway so a panic here does
            // not poison the mutex and kill the render thread.
            let snapshot_start = Instant::now();
            let snap_result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.build_snapshot()));
            let snapshot = match snap_result {
                Ok(s) => s,
                Err(e) => {
                    let msg = e
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| e.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("(non-string payload)");
                    godot_error!("[sim] build_snapshot panicked — using default: {}", msg);
                    RenderSnapshot::default()
                }
            };
            snapshot_ms = snapshot_start.elapsed().as_secs_f64() * 1000.0;
            lock_held_ms = lock_held_start.elapsed().as_secs_f64() * 1000.0;
            agent_count = snapshot.agent_count;
            pathfind_count = snapshot.pathfind_count;
            snapshot
        };

        // Write snapshot — outside the sim lock so render reads are non-blocking.
        let snapshot_write_start = Instant::now();
        *snapshot.write().unwrap() = new_snapshot;
        let snapshot_write_ms = snapshot_write_start.elapsed().as_secs_f64() * 1000.0;
        let active_ms = frame_start.elapsed().as_secs_f64() * 1000.0;
        let unaccounted_ms =
            (active_ms - command_ms - lock_wait_ms - lock_held_ms - snapshot_write_ms).max(0.0);
        if perf_enabled && (active_ms >= 8.0 || command_ms >= 8.0 || elapsed_minutes > 0) {
            println!(
                "[DEBUG:perf] sim_frame active_ms={:.3} command_ms={:.3} lock_wait_ms={:.3} lock_held_ms={:.3} pathing_ms={:.3} agent_ms={:.3} minute_ms={:.3} pending_spawn_ms={:.3} hourly_ms={:.3} daily_ms={:.3} snapshot_ms={:.3} snapshot_write_ms={:.3} unaccounted_ms={:.3} elapsed_minutes={} pending_spawns={} hourly_ticks={} daily_ticks={} agents={} pathfinds={} commands={} set_speed_cmds={} camera_aabb_cmds={} add_road_cmds={}",
                active_ms,
                command_ms,
                lock_wait_ms,
                lock_held_ms,
                pathing_ms,
                agent_ms,
                minute_ms,
                pending_spawn_ms,
                hourly_ms,
                daily_ms,
                snapshot_ms,
                snapshot_write_ms,
                unaccounted_ms,
                elapsed_minutes,
                pending_spawns_executed,
                hourly_ticks,
                daily_ticks,
                agent_count,
                pathfind_count,
                commands_processed,
                set_speed_commands,
                camera_aabb_commands,
                add_road_commands,
            );
        }

        // Sleep to maintain ~60 Hz.
        let elapsed = frame_start.elapsed();
        if elapsed < target {
            std::thread::sleep(target - elapsed);
        }
    }
}
