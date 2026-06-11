//! SQLite schema and enum mapping for persistent storage.

use super::SaveLoadError;
use crate::simulation::network::types::{EdgeClass, NodeType, TransitType, VehicleFrontageAccess};

/// Current save format version.
pub const SAVE_VERSION: i64 = 43;
/// Sentinel for missing integer references in SQLite.
pub const NONE_REF: i64 = -1;

/// The SQL schema string.
pub const SCHEMA: &str = r#"
CREATE TABLE save_meta(
    version INTEGER NOT NULL,
    saved_at_unix INTEGER NOT NULL,
    game_build TEXT NOT NULL
);
CREATE TABLE world_config(
    width_m REAL NOT NULL,
    height_m REAL NOT NULL,
    terrain_cell_m REAL NOT NULL,
    terrain_chunk_m REAL NOT NULL,
    terrain_base_elevation_m REAL NOT NULL,
    env_cell_m REAL NOT NULL,
    zone_cell_m REAL NOT NULL
);
CREATE TABLE time_state(
    time_elapsed REAL NOT NULL,
    speed_multiplier REAL NOT NULL,
    day_index INTEGER NOT NULL,
    minute_of_day INTEGER NOT NULL,
    seconds_per_day REAL NOT NULL,
    agent_sim_time REAL NOT NULL
);
CREATE TABLE terrain_state(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    height_blob_f32_le BLOB NOT NULL
);
CREATE TABLE water_state(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    baseline_depth_blob_f32_le BLOB NOT NULL,
    dynamic_depth_blob_f32_le BLOB NOT NULL,
    velocity_blob_f32_le BLOB NOT NULL,
    flux_blob_f32x4_le BLOB NOT NULL
);
CREATE TABLE water_sources(
    grid_x INTEGER NOT NULL,
    grid_y INTEGER NOT NULL,
    rate_m_per_tick REAL NOT NULL
);
CREATE TABLE demand_state(
    residential REAL NOT NULL,
    commercial REAL NOT NULL,
    industrial REAL NOT NULL,
    households_to_admit_today INTEGER NOT NULL,
    households_to_remove_today INTEGER NOT NULL,
    admission_action_credit REAL NOT NULL,
    removal_action_credit REAL NOT NULL,
    persistent_exit_action_credit REAL NOT NULL,
    spawn_action_credit_residential REAL NOT NULL,
    spawn_action_credit_commercial REAL NOT NULL,
    spawn_action_credit_industrial REAL NOT NULL,
    upgrade_action_credit_residential REAL NOT NULL,
    upgrade_action_credit_commercial REAL NOT NULL,
    upgrade_action_credit_industrial REAL NOT NULL,
    downgrade_action_credit_residential REAL NOT NULL,
    downgrade_action_credit_commercial REAL NOT NULL,
    downgrade_action_credit_industrial REAL NOT NULL,
    despawn_action_credit_residential REAL NOT NULL,
    despawn_action_credit_commercial REAL NOT NULL,
    despawn_action_credit_industrial REAL NOT NULL,
    spawn_hysteresis_active_residential INTEGER NOT NULL,
    spawn_hysteresis_active_commercial INTEGER NOT NULL,
    spawn_hysteresis_active_industrial INTEGER NOT NULL,
    upgrade_hysteresis_active_residential INTEGER NOT NULL,
    upgrade_hysteresis_active_commercial INTEGER NOT NULL,
    upgrade_hysteresis_active_industrial INTEGER NOT NULL,
    downgrade_hysteresis_active_residential INTEGER NOT NULL,
    downgrade_hysteresis_active_commercial INTEGER NOT NULL,
    downgrade_hysteresis_active_industrial INTEGER NOT NULL,
    despawn_hysteresis_active_residential INTEGER NOT NULL,
    despawn_hysteresis_active_commercial INTEGER NOT NULL,
    despawn_hysteresis_active_industrial INTEGER NOT NULL,
    recent_household_failure_pressure REAL NOT NULL
);
CREATE TABLE pollution_state(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    grid_blob_f32_le BLOB NOT NULL
);
CREATE TABLE noise_state(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    grid_blob_f32_le BLOB NOT NULL
);
CREATE TABLE network_nodes(
    node_id INTEGER PRIMARY KEY,
    x REAL NOT NULL,
    y REAL NOT NULL,
    z REAL NOT NULL,
    node_type INTEGER NOT NULL
);
CREATE TABLE network_edges(
    edge_id INTEGER PRIMARY KEY,
    start_node INTEGER NOT NULL,
    end_node INTEGER NOT NULL,
    primary_type INTEGER NOT NULL,
    allowed_types INTEGER NOT NULL,
    class INTEGER NOT NULL,
    width REAL NOT NULL,
    fwd_lanes INTEGER NOT NULL,
    bkw_lanes INTEGER NOT NULL,
    speed_limit REAL NOT NULL,
    base_cost REAL NOT NULL,
    physical_length REAL NOT NULL,
    current_congestion REAL NOT NULL,
    start_clip REAL NOT NULL,
    end_clip REAL NOT NULL,
    no_building_spawn INTEGER NOT NULL DEFAULT 0,
    vehicle_frontage_access INTEGER NOT NULL DEFAULT 1
);
CREATE TABLE network_edge_geometry(
    edge_id INTEGER NOT NULL,
    point_index INTEGER NOT NULL,
    x REAL NOT NULL,
    y REAL NOT NULL,
    z REAL NOT NULL,
    physical INTEGER NOT NULL,
    PRIMARY KEY(edge_id, physical, point_index)
);
CREATE TABLE lane_connections(
    node_id INTEGER NOT NULL,
    from_edge INTEGER NOT NULL,
    from_lane INTEGER NOT NULL,
    to_edge INTEGER NOT NULL,
    to_lane INTEGER NOT NULL
);
CREATE TABLE zoning_parcels(
    parcel_id INTEGER PRIMARY KEY,
    edge_id INTEGER NOT NULL,
    side INTEGER NOT NULL,
    frontage_t REAL NOT NULL,
    frontage_m REAL NOT NULL,
    depth_m REAL NOT NULL,
    profile_runtime_id INTEGER NOT NULL
);
CREATE TABLE buildings(
    building_id INTEGER PRIMARY KEY,
    parcel_id INTEGER NOT NULL,
    edge_id INTEGER NOT NULL,
    frontage_t REAL NOT NULL,
    side INTEGER NOT NULL,
    cell_x INTEGER NOT NULL,
    cell_y INTEGER NOT NULL,
    profile_runtime_id INTEGER NOT NULL,
    occupancy INTEGER NOT NULL,
    worker_count INTEGER NOT NULL,
    revenue REAL NOT NULL,
    operating_budget REAL NOT NULL,
    profit_tax_budget_baseline REAL NOT NULL,
    shipment_cooldown_hours INTEGER NOT NULL,
    width INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    asset_id TEXT NOT NULL,
    level INTEGER NOT NULL,
    construction_total_hours INTEGER NOT NULL,
    construction_remaining_hours INTEGER NOT NULL,
    broken INTEGER NOT NULL DEFAULT 0,
    pending_redevelopment INTEGER NOT NULL DEFAULT 0,
    rezone_grace_days_remaining INTEGER NOT NULL DEFAULT 0,
    is_deserted INTEGER NOT NULL DEFAULT 0,
    budget_distress INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE building_inventories(
    building_id INTEGER NOT NULL,
    resource_runtime_id INTEGER NOT NULL,
    amount REAL NOT NULL,
    PRIMARY KEY(building_id, resource_runtime_id)
);
CREATE TABLE households(
    household_id INTEGER PRIMARY KEY,
    home_building INTEGER NOT NULL,
    budget REAL NOT NULL,
    stock REAL NOT NULL,
    member_count INTEGER NOT NULL,
    child_count INTEGER NOT NULL,
    adult_count INTEGER NOT NULL,
    elder_count INTEGER NOT NULL,
    consumption_rate REAL NOT NULL,
    stock_days REAL NOT NULL,
    replenishment_state INTEGER NOT NULL,
    cooldown_hours INTEGER NOT NULL,
    replenishment_failure_count INTEGER NOT NULL,
    reserved_store_building_id INTEGER NOT NULL,
    reserved_amount REAL NOT NULL,
    reserved_total_cost REAL NOT NULL,
    shopping_agent_id INTEGER NOT NULL,
    shopping_agent_schedule_seed INTEGER NOT NULL,
    shopping_timeout_hours_remaining INTEGER NOT NULL,
    replenishment_search_cursor INTEGER NOT NULL,
    stay_failure_days INTEGER NOT NULL,
    unhoused_days_elapsed INTEGER NOT NULL,
    replenishment_offset_hours INTEGER NOT NULL,
    unemployment_days_elapsed INTEGER NOT NULL
);
CREATE TABLE city_treasury(
    balance REAL NOT NULL,
    lifetime_build_cost REAL NOT NULL,
    lifetime_tax_revenue REAL NOT NULL,
    last_daily_upkeep REAL NOT NULL,
    last_daily_income_tax REAL NOT NULL,
    last_daily_household_vat REAL NOT NULL,
    last_daily_business_purchase_tax REAL NOT NULL,
    last_daily_business_profit_tax REAL NOT NULL,
    last_daily_property_tax REAL NOT NULL,
    pending_income_tax REAL NOT NULL,
    pending_household_vat REAL NOT NULL,
    pending_business_purchase_tax REAL NOT NULL,
    pending_business_profit_tax REAL NOT NULL,
    pending_property_tax REAL NOT NULL
);
CREATE TABLE shipments(
    shipment_id INTEGER PRIMARY KEY,
    resource_runtime_id INTEGER NOT NULL,
    amount REAL NOT NULL,
    source_endpoint_kind INTEGER NOT NULL,
    source_building_id INTEGER NOT NULL,
    source_border_node INTEGER NOT NULL,
    destination_endpoint_kind INTEGER NOT NULL,
    destination_building_id INTEGER NOT NULL,
    destination_border_node INTEGER NOT NULL,
    carrier_class INTEGER NOT NULL,
    status INTEGER NOT NULL,
    carrier_agent_id INTEGER NOT NULL,
    total_cost REAL NOT NULL,
    tax_cost REAL NOT NULL,
    eta_hours INTEGER NOT NULL,
    queued_hours INTEGER NOT NULL
);
CREATE TABLE freight_request_failures(
    destination_building_id INTEGER NOT NULL,
    resource_runtime_id INTEGER NOT NULL,
    failures INTEGER NOT NULL,
    terminal INTEGER NOT NULL,
    PRIMARY KEY(destination_building_id, resource_runtime_id)
);
CREATE TABLE agents(
    agent_id INTEGER PRIMARY KEY,
    home_building INTEGER NOT NULL,
    household_id INTEGER NOT NULL,
    age_group INTEGER NOT NULL,
    pending_household_size INTEGER NOT NULL DEFAULT 0,
    freight_shipment_id INTEGER NOT NULL,
    work_building INTEGER NOT NULL,
    current_building INTEGER NOT NULL,
    target_building INTEGER NOT NULL,
    freight_target_border_node INTEGER NOT NULL,
    current_node INTEGER NOT NULL,
    planned_attach_node INTEGER NOT NULL DEFAULT 4294967295,
    planned_detach_node INTEGER NOT NULL DEFAULT 4294967295,
    planned_attach_lane_id INTEGER NOT NULL DEFAULT 4294967295,
    planned_detach_lane_id INTEGER NOT NULL DEFAULT 4294967295,
    planned_attach_lane_d REAL NOT NULL DEFAULT 0.0,
    planned_detach_lane_d REAL NOT NULL DEFAULT 0.0,
    access_flags INTEGER NOT NULL DEFAULT 0,
    next_replan_time REAL NOT NULL DEFAULT 0.0,
    current_edge INTEGER NOT NULL,
    current_lane_id INTEGER NOT NULL,
    lane_distance REAL NOT NULL,
    pos_x REAL NOT NULL,
    pos_y REAL NOT NULL,
    activity INTEGER NOT NULL,
    transit INTEGER NOT NULL,
    transit_mode INTEGER NOT NULL,
    pedestrian_side INTEGER NOT NULL,
    happiness REAL NOT NULL,
    money REAL NOT NULL,
    journey_start_time REAL NOT NULL,
    schedule_seed INTEGER NOT NULL,
    cached_commute_minutes INTEGER NOT NULL,
    next_commute_refresh_time REAL NOT NULL,
    next_departure_day INTEGER NOT NULL,
    next_departure_minute INTEGER NOT NULL,
    next_departure_origin_building INTEGER NOT NULL,
    next_departure_target_building INTEGER NOT NULL,
    next_departure_activity INTEGER NOT NULL,
    cached_schedule_work_building INTEGER NOT NULL,
    cached_work_profile_index INTEGER NOT NULL,
    has_car INTEGER NOT NULL,
    vehicle_type INTEGER NOT NULL,
    current_path_index INTEGER NOT NULL
);
CREATE TABLE agent_path_nodes(
    agent_id INTEGER NOT NULL,
    step_index INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    PRIMARY KEY(agent_id, step_index)
);
CREATE TABLE agent_ped_steps(
    agent_id INTEGER NOT NULL,
    step_index INTEGER NOT NULL,
    edge_id INTEGER NOT NULL,
    forward INTEGER NOT NULL,
    side INTEGER NOT NULL,
    PRIMARY KEY(agent_id, step_index)
);
"#;

pub fn transit_type_to_i64(value: TransitType) -> i64 {
    match value {
        TransitType::Road => 0,
        TransitType::Rail => 1,
        TransitType::Ship => 2,
        TransitType::Air => 3,
        TransitType::Foot => 4,
    }
}

pub fn transit_type_from_i64(value: i64) -> Result<TransitType, SaveLoadError> {
    match value {
        0 => Ok(TransitType::Road),
        1 => Ok(TransitType::Rail),
        2 => Ok(TransitType::Ship),
        3 => Ok(TransitType::Air),
        4 => Ok(TransitType::Foot),
        _ => Err(SaveLoadError::custom(format!(
            "unknown TransitType value {}",
            value
        ))),
    }
}

pub fn node_type_to_i64(value: NodeType) -> i64 {
    match value {
        NodeType::Junction => 0,
        NodeType::Station => 1,
        NodeType::Harbor => 2,
        NodeType::Airport => 3,
        NodeType::Transfer => 4,
        NodeType::Border => 5,
    }
}

pub fn node_type_from_i64(value: i64) -> Result<NodeType, SaveLoadError> {
    match value {
        0 => Ok(NodeType::Junction),
        1 => Ok(NodeType::Station),
        2 => Ok(NodeType::Harbor),
        3 => Ok(NodeType::Airport),
        4 => Ok(NodeType::Transfer),
        5 => Ok(NodeType::Border),
        _ => Err(SaveLoadError::custom(format!(
            "unknown NodeType value {}",
            value
        ))),
    }
}

pub fn edge_class_to_i64(value: EdgeClass) -> i64 {
    match value {
        EdgeClass::Standard => 0,
        EdgeClass::Bridge => 1,
        EdgeClass::Tunnel => 2,
    }
}

pub fn edge_class_from_i64(value: i64) -> Result<EdgeClass, SaveLoadError> {
    match value {
        0 => Ok(EdgeClass::Standard),
        1 => Ok(EdgeClass::Bridge),
        2 => Ok(EdgeClass::Tunnel),
        _ => Err(SaveLoadError::custom(format!(
            "unknown EdgeClass value {}",
            value
        ))),
    }
}

pub fn vehicle_frontage_access_to_i64(value: VehicleFrontageAccess) -> i64 {
    match value {
        VehicleFrontageAccess::SameSideOnly => 0,
        VehicleFrontageAccess::BothSides => 1,
    }
}

pub fn vehicle_frontage_access_from_i64(
    value: i64,
) -> Result<VehicleFrontageAccess, SaveLoadError> {
    match value {
        0 => Ok(VehicleFrontageAccess::SameSideOnly),
        1 => Ok(VehicleFrontageAccess::BothSides),
        _ => Err(SaveLoadError::custom(format!(
            "unknown VehicleFrontageAccess value {}",
            value
        ))),
    }
}
