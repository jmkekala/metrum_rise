//! SQLite schema and enum mapping for persistent storage.

use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::types::{EdgeClass, NodeType, TransitType};
use super::SaveLoadError;

/// Current save format version.
pub const SAVE_VERSION: i64 = 11;
/// Sentinel for missing integer references in SQLite.
pub const NONE_REF: i64 = -1;

/// The SQL schema string.
pub const SCHEMA: &str = r#"
CREATE TABLE save_meta(
    version INTEGER NOT NULL,
    saved_at_unix INTEGER NOT NULL,
    game_build TEXT NOT NULL
);
CREATE TABLE map_config(
    width_m REAL NOT NULL,
    height_m REAL NOT NULL,
    env_cell_m REAL NOT NULL,
    zone_cell_m REAL NOT NULL
);
CREATE TABLE time_state(
    time_elapsed REAL NOT NULL,
    speed_multiplier REAL NOT NULL,
    current_day INTEGER NOT NULL,
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
    depth_blob_f32_le BLOB NOT NULL,
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
    industrial REAL NOT NULL
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
    no_building_spawn INTEGER NOT NULL DEFAULT 0
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
CREATE TABLE IF NOT EXISTS zoning_grids(
    edge_id INTEGER PRIMARY KEY,
    cells_long INTEGER NOT NULL,
    left_depth INTEGER NOT NULL,
    right_depth INTEGER NOT NULL,
    left_zone_blob_u8 BLOB NOT NULL,
    right_zone_blob_u8 BLOB NOT NULL
);
CREATE TABLE zoning_world_grid(
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    data BLOB NOT NULL
);
CREATE TABLE buildings(
    building_id INTEGER PRIMARY KEY,
    edge_id INTEGER NOT NULL,
    frontage_t REAL NOT NULL,
    side INTEGER NOT NULL,
    cell_x INTEGER NOT NULL,
    cell_y INTEGER NOT NULL,
    zone_type INTEGER NOT NULL,
    occupancy INTEGER NOT NULL,
    worker_count INTEGER NOT NULL,
    stock REAL NOT NULL,
    revenue REAL NOT NULL,
    operating_budget REAL NOT NULL,
    utility_service_available INTEGER NOT NULL,
    shipment_cooldown_days INTEGER NOT NULL,
    width INTEGER NOT NULL,
    depth INTEGER NOT NULL,
    asset_id TEXT NOT NULL,
    level INTEGER NOT NULL,
    broken INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE founding_state(
    bootstrap_consumed INTEGER NOT NULL
);
CREATE TABLE households(
    household_id INTEGER PRIMARY KEY,
    home_building INTEGER NOT NULL,
    budget REAL NOT NULL,
    stock REAL NOT NULL,
    member_count INTEGER NOT NULL,
    consumption_rate REAL NOT NULL,
    stock_days REAL NOT NULL,
    replenishment_state INTEGER NOT NULL,
    cooldown_days INTEGER NOT NULL,
    reserved_store_building_id INTEGER NOT NULL,
    reserved_amount REAL NOT NULL,
    reserved_total_cost REAL NOT NULL,
    pickup_eta_days INTEGER NOT NULL
);
CREATE TABLE shipments(
    shipment_id INTEGER PRIMARY KEY,
    resource_type INTEGER NOT NULL,
    amount REAL NOT NULL,
    source_kind INTEGER NOT NULL,
    source_building_id INTEGER NOT NULL,
    source_border_node INTEGER NOT NULL,
    destination_building_id INTEGER NOT NULL,
    carrier_class INTEGER NOT NULL,
    status INTEGER NOT NULL,
    total_cost REAL NOT NULL,
    eta_days INTEGER NOT NULL
);
CREATE TABLE agents(
    agent_id INTEGER PRIMARY KEY,
    home_building INTEGER NOT NULL,
    household_id INTEGER NOT NULL,
    work_building INTEGER NOT NULL,
    current_building INTEGER NOT NULL,
    target_building INTEGER NOT NULL,
    current_node INTEGER NOT NULL,
    target_node INTEGER NOT NULL,
    current_edge INTEGER NOT NULL,
    current_lane_id INTEGER NOT NULL,
    lane_distance REAL NOT NULL,
    pos_x REAL NOT NULL,
    pos_y REAL NOT NULL,
    is_visible INTEGER NOT NULL,
    activity INTEGER NOT NULL,
    transit INTEGER NOT NULL,
    transit_mode INTEGER NOT NULL,
    pedestrian_side INTEGER NOT NULL,
    happiness REAL NOT NULL,
    money REAL NOT NULL,
    journey_start_time REAL NOT NULL,
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

pub fn zone_type_to_i64(value: ZoneType) -> i64 {
    i64::from(value as u8)
}

pub fn zone_type_from_i64(value: i64) -> Result<ZoneType, SaveLoadError> {
    match value {
        0 => Ok(ZoneType::None),
        1 => Ok(ZoneType::Residential),
        2 => Ok(ZoneType::Commercial),
        3 => Ok(ZoneType::Industrial),
        4 => Ok(ZoneType::Office),
        5 => Ok(ZoneType::Mixed),
        _ => Err(SaveLoadError::custom(format!(
            "unknown ZoneType value {}",
            value
        ))),
    }
}
