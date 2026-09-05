// SPDX-License-Identifier: GPL-2.0-only

//! Household supply, employment, business, utility, and housing tests.

use super::*;
use crate::assets::AssetManifest;
use crate::assets::asset::{Anchor, AnchorType, BuildingData, MeshPart, PlacementMode, ZoneClass};
use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::agents::{
    AGE_CHILD, AGE_ELDER, AgentSystem, TRANSIT_ACCESS_INGRESS, TRANSIT_IN_BUILDING,
    VEHICLE_FREIGHT_DELIVERY,
};
use crate::simulation::economy::definitions::{
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::economy::households::metrics::{
    active_worker_capacity_for_profile, building_operation_factors,
    demand_sink_cash_cost_per_resident_excluding_resource, household_is_housed,
    household_supply_resource_runtime_id, household_supply_unit_price,
    refresh_commercial_activity_floor, scaled_output_buffer_capacity_units_for_building,
    scaled_output_units_per_day_for_building,
};
use crate::simulation::economy::logistics::{
    CarrierClass, Shipment, ShipmentEndpoint, ShipmentStatus, ShipmentSystem,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::{Edge, RegionGraph};
use crate::simulation::network::types::{EdgeClass, NodeType, TransitFlags, TransitType};
use crate::simulation::pathing::cch::CchGraph;
use crate::simulation::zoning::ZoneType;
use godot::prelude::{Vector2, Vector3};

mod business;
mod commercial;
mod employment;
mod housing;
mod replenishment;
mod service_visits;
mod support;
