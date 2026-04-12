//! Batched building-level freight reservations and delayed deliveries.
//!
//! The first shipment slice keeps freight explicit without introducing per-order
//! micro-deliveries. Commercial buildings open bounded restock requests,
//! industrial suppliers reserve stock for them, and `OWA` border terminals act
//! as the external fallback for ordinary imported goods.

use std::collections::HashMap;

use crate::simulation::buildings::allocator::{Building, BuildingAllocator};
use crate::simulation::economy::definitions::{
    FreightTimingProfile, RuntimeEconomyCatalog, RuntimeEconomyTuning, StarterResourceKind,
    load_runtime_economy_catalog, load_runtime_economy_tuning,
};
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::NodeType;

/// Starter upstream resource produced by industry and consumed by stores.
pub const RESOURCE_STAPLE_FOOD: u8 = 0;
/// First-pass shipped household-facing resource used by the starter economy chain.
pub const RESOURCE_HOUSEHOLD_SUPPLIES: u8 = 1;
/// Legacy starter industrial-input resource kept as extension space for later profiles.
pub const RESOURCE_INDUSTRIAL_INPUTS: u8 = 2;
/// The shipment originates from a local supplier building.
pub const SHIPMENT_SOURCE_LOCAL: u8 = 0;
/// The shipment originates from an `OWA` border terminal.
pub const SHIPMENT_SOURCE_OWA: u8 = 1;
/// The assigned carrier is a local or border truck.
pub const CARRIER_TRUCK: u8 = 0;
/// Shipment is active and still travelling.
pub const SHIPMENT_IN_TRANSIT: u8 = 0;
/// Shipment arrived successfully.
pub const SHIPMENT_FULFILLED: u8 = 1;
/// Shipment failed and its reservations were released.
pub const SHIPMENT_FAILED: u8 = 2;

const SUPPLIER_SEARCH_MAX_RING: i32 = 3;
const SUPPLIER_SEARCH_CANDIDATES: usize = 8;
const BORDER_ACTIVE_JOBS_PER_NODE: usize = 4;
const OPERATIONAL_HOUR_SECONDS: f32 = 60.0 * 60.0;

/// One reserved freight job moving stock between buildings or from `OWA`.
#[derive(Clone, Debug)]
pub struct Shipment {
    /// Resource type carried by this shipment.
    pub resource_type: u8,
    /// Reserved amount in resource units.
    pub amount: f32,
    /// Whether the source is local or `OWA`.
    pub source_kind: u8,
    /// Source building index for local shipments; `usize::MAX` for `OWA`.
    pub source_building_id: usize,
    /// Border node used by `OWA` shipments; `u32::MAX` for local freight.
    pub source_border_node: u32,
    /// Destination building receiving the shipment.
    pub destination_building_id: usize,
    /// Carrier class used by the shipment.
    pub carrier_class: u8,
    /// Current shipment state.
    pub status: u8,
    /// Reserved payment held by the destination until completion or failure.
    pub total_cost: f32,
    /// Remaining operational-hour steps before the shipment arrives.
    pub eta_hours: u16,
}

/// Runtime collection of active freight jobs.
#[derive(Clone, Debug, Default)]
pub struct ShipmentSystem {
    /// All active shipment jobs.
    pub shipments: Vec<Shipment>,
}

impl ShipmentSystem {
    /// Creates an empty shipment system.
    pub fn new() -> Self {
        Self {
            shipments: Vec::new(),
        }
    }

    /// Clears all active shipments.
    pub fn clear(&mut self) {
        self.shipments.clear();
    }

    /// Remaps building references after a building swap-remove.
    pub fn remap_building_indices(&mut self, mapping: &HashMap<usize, usize>) {
        for shipment in &mut self.shipments {
            if let Some(&new_id) = mapping.get(&shipment.destination_building_id) {
                shipment.destination_building_id = new_id;
            }
            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL {
                if let Some(&new_id) = mapping.get(&shipment.source_building_id) {
                    shipment.source_building_id = new_id;
                }
            }
        }
    }

    /// Cancels any shipment touching the removed building before swap-remove happens.
    pub fn invalidate_building(
        &mut self,
        removed_building: usize,
        allocator: &mut BuildingAllocator,
    ) {
        self.shipments.retain(|shipment| {
            let touches_removed = shipment.destination_building_id == removed_building
                || (shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                    && shipment.source_building_id == removed_building);

            if !touches_removed {
                return true;
            }

            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                && shipment.source_building_id == removed_building
                && shipment.destination_building_id < allocator.buildings.len()
                && shipment.destination_building_id != removed_building
            {
                allocator.buildings[shipment.destination_building_id].operating_budget +=
                    shipment.total_cost;
            }

            false
        });
    }

    /// Advances freight deliveries and opens new bounded restock jobs on one operational hour.
    pub fn hourly_tick(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
    ) {
        self.progress_shipments(allocator);
        self.decrement_building_cooldowns(allocator);
        self.create_profile_input_shipments(allocator, transit_network, graph, minute_of_day);
        self.shipments
            .retain(|shipment| shipment.status == SHIPMENT_IN_TRANSIT);
    }

    fn progress_shipments(&mut self, allocator: &mut BuildingAllocator) {
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let retry_cooldown_hours = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"))
            .operational_clock
            .shipment_retry_cooldown_hours;
        for shipment in &mut self.shipments {
            if shipment.status != SHIPMENT_IN_TRANSIT {
                continue;
            }

            if shipment.eta_hours > 0 {
                shipment.eta_hours -= 1;
            }
            if shipment.eta_hours > 0 {
                continue;
            }

            let dest_idx = shipment.destination_building_id;
            if dest_idx >= allocator.buildings.len() {
                shipment.status = SHIPMENT_FAILED;
                continue;
            }

            match shipment.source_kind {
                SHIPMENT_SOURCE_LOCAL => {
                    let src_idx = shipment.source_building_id;
                    if src_idx >= allocator.buildings.len()
                        || allocator.buildings[src_idx].broken
                        || allocator.buildings[src_idx].economy_broken
                        || allocator.buildings[src_idx].stock < shipment.amount
                        || !building_accepts_input_resource(
                            &catalog,
                            &allocator.buildings[dest_idx],
                            shipment.resource_type,
                        )
                    {
                        allocator.buildings[dest_idx].operating_budget += shipment.total_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_hours =
                            retry_cooldown_hours;
                        shipment.status = SHIPMENT_FAILED;
                        continue;
                    }

                    allocator.buildings[src_idx].stock -= shipment.amount;
                    allocator.buildings[src_idx].revenue += shipment.total_cost;
                    allocator.buildings[src_idx].operating_budget += shipment.total_cost;
                    allocator.buildings[dest_idx].input_stock += shipment.amount;
                    shipment.status = SHIPMENT_FULFILLED;
                }
                SHIPMENT_SOURCE_OWA => {
                    if !building_accepts_input_resource(
                        &catalog,
                        &allocator.buildings[dest_idx],
                        shipment.resource_type,
                    ) {
                        allocator.buildings[dest_idx].operating_budget += shipment.total_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_hours =
                            retry_cooldown_hours;
                        shipment.status = SHIPMENT_FAILED;
                        continue;
                    }
                    allocator.buildings[dest_idx].input_stock += shipment.amount;
                    shipment.status = SHIPMENT_FULFILLED;
                }
                _ => {
                    allocator.buildings[dest_idx].operating_budget += shipment.total_cost;
                    allocator.buildings[dest_idx].shipment_cooldown_hours = retry_cooldown_hours;
                    shipment.status = SHIPMENT_FAILED;
                }
            }
        }
    }

    fn decrement_building_cooldowns(&self, allocator: &mut BuildingAllocator) {
        for building in &mut allocator.buildings {
            if building.shipment_cooldown_hours > 0 {
                building.shipment_cooldown_hours -= 1;
            }
        }
    }

    fn create_profile_input_shipments(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        minute_of_day: u16,
    ) {
        let (reserved_outbound, reserved_inbound, has_open_inbound, border_job_counts) =
            self.build_reservation_views();
        let border_nodes = connected_border_nodes(graph);
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));

        for dest_idx in 0..allocator.buildings.len() {
            let building = &allocator.buildings[dest_idx];
            if building.broken
                || building.economy_broken
                || building.edge_idx == usize::MAX
                || has_open_inbound.get(dest_idx).copied().unwrap_or(false)
                || building.shipment_cooldown_hours > 0
            {
                continue;
            }
            let Some(profile) = catalog.profile_by_runtime_id(building.economy_profile_runtime_id)
            else {
                continue;
            };
            let Some(input_port) = profile.input else {
                continue;
            };
            let Some(freight_profile) = freight_profile_for_building(&catalog, &tuning, building)
            else {
                continue;
            };

            let target_units = profile.inventory_target_units();
            if target_units <= 0.0 {
                continue;
            }
            let reorder_units = profile.inventory_reorder_units();
            let critical_units = profile.inventory_critical_units();
            let effective_input_stock =
                building.input_stock + reserved_inbound.get(dest_idx).copied().unwrap_or(0.0);
            if reorder_units > 0.0 && effective_input_stock >= reorder_units {
                continue;
            }
            if reorder_units <= 0.0 && effective_input_stock >= target_units {
                continue;
            }

            let allow_emergency = effective_input_stock <= critical_units;
            let desired_amount = (target_units - effective_input_stock).max(0.0);
            if desired_amount <= 0.0 {
                continue;
            }
            if desired_amount < profile.min_shipment_units && !allow_emergency {
                continue;
            }

            if self.try_local_supplier_for_resource(
                dest_idx,
                desired_amount,
                allow_emergency,
                profile.min_shipment_units,
                input_port.resource,
                allocator,
                transit_network,
                graph,
                &reserved_outbound,
                freight_profile,
                minute_of_day,
                &catalog,
            ) {
                continue;
            }

            let import_unit_price = catalog
                .unit_price_for_resource(input_port.resource)
                .unwrap_or(profile.unit_price_currency);
            if self.try_owa_fallback_for_resource(
                dest_idx,
                desired_amount,
                allow_emergency,
                profile.min_shipment_units,
                import_unit_price,
                resource_kind_to_shipment_type(input_port.resource),
                allocator,
                transit_network,
                graph,
                &border_nodes,
                &border_job_counts,
                freight_profile,
                minute_of_day,
            ) {
                continue;
            }

            allocator.buildings[dest_idx].shipment_cooldown_hours =
                tuning.operational_clock.shipment_retry_cooldown_hours;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn try_local_supplier_for_resource(
        &mut self,
        dest_idx: usize,
        desired_amount: f32,
        allow_emergency: bool,
        min_shipment_units: f32,
        resource: StarterResourceKind,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        reserved_outbound: &[f32],
        freight_profile: &FreightTimingProfile,
        minute_of_day: u16,
        catalog: &RuntimeEconomyCatalog,
    ) -> bool {
        if dest_idx >= allocator.entrances.len() {
            return false;
        }
        let destination = &allocator.buildings[dest_idx];
        let candidates = allocator.find_nearby_buildings_by_zones(
            destination.center_x,
            destination.center_y,
            &[ZoneType::Industrial, ZoneType::Commercial],
            SUPPLIER_SEARCH_MAX_RING,
            SUPPLIER_SEARCH_CANDIDATES,
        );

        for candidate_idx in candidates {
            if candidate_idx == dest_idx || candidate_idx >= allocator.buildings.len() {
                continue;
            }
            let supplier = &allocator.buildings[candidate_idx];
            if supplier.broken || supplier.economy_broken || !supplier.utility_service_available {
                continue;
            }
            let Some(supplier_profile) =
                catalog.profile_by_runtime_id(supplier.economy_profile_runtime_id)
            else {
                continue;
            };
            let Some(output_port) = supplier_profile.output else {
                continue;
            };
            if output_port.resource != resource {
                continue;
            }

            let reserved = reserved_outbound.get(candidate_idx).copied().unwrap_or(0.0);
            let available = (supplier.stock - reserved).max(0.0);
            if available <= 0.0 {
                continue;
            }

            let amount = available.min(desired_amount);
            if amount < min_shipment_units && !allow_emergency {
                continue;
            }

            let total_cost = amount
                * adjusted_unit_price(
                    supplier_profile.unit_price_currency,
                    freight_profile,
                    minute_of_day,
                );
            if allocator.buildings[dest_idx].operating_budget < total_cost {
                continue;
            }

            let Some(travel_seconds) = allocator.freight_car_eta_between_buildings(
                candidate_idx,
                dest_idx,
                transit_network,
                graph,
            ) else {
                continue;
            };

            allocator.buildings[dest_idx].operating_budget -= total_cost;
            self.shipments.push(Shipment {
                resource_type: resource_kind_to_shipment_type(resource),
                amount,
                source_kind: SHIPMENT_SOURCE_LOCAL,
                source_building_id: candidate_idx,
                source_border_node: u32::MAX,
                destination_building_id: dest_idx,
                carrier_class: CARRIER_TRUCK,
                status: SHIPMENT_IN_TRANSIT,
                total_cost,
                eta_hours: eta_hours_from_travel_seconds(adjusted_travel_seconds(
                    travel_seconds,
                    freight_profile,
                    minute_of_day,
                )),
            });
            return true;
        }

        false
    }

    #[allow(clippy::too_many_arguments)]
    fn try_owa_fallback_for_resource(
        &mut self,
        dest_idx: usize,
        desired_amount: f32,
        allow_emergency: bool,
        min_shipment_units: f32,
        unit_price: f32,
        resource_type: u8,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        border_nodes: &[u32],
        border_job_counts: &HashMap<u32, usize>,
        freight_profile: &FreightTimingProfile,
        minute_of_day: u16,
    ) -> bool {
        if border_nodes.is_empty() {
            return false;
        }

        let min_amount = if desired_amount < min_shipment_units && allow_emergency {
            desired_amount
        } else {
            min_shipment_units
        };
        let max_affordable_amount = allocator.buildings[dest_idx].operating_budget / unit_price;
        if max_affordable_amount < min_amount {
            return false;
        }
        let amount = desired_amount.max(min_amount).min(max_affordable_amount);
        let total_cost = amount * adjusted_unit_price(unit_price, freight_profile, minute_of_day);

        let mut best_border = u32::MAX;
        let mut best_cost = f32::MAX;
        for &border_node in border_nodes {
            if border_job_counts.get(&border_node).copied().unwrap_or(0)
                >= BORDER_ACTIVE_JOBS_PER_NODE
            {
                continue;
            }
            let Some(travel_seconds) = allocator.freight_car_eta_from_border_node(
                border_node,
                dest_idx,
                transit_network,
                graph,
            ) else {
                continue;
            };
            if travel_seconds < best_cost {
                best_cost = travel_seconds;
                best_border = border_node;
            }
        }

        if best_border == u32::MAX {
            return false;
        }

        allocator.buildings[dest_idx].operating_budget -= total_cost;
        self.shipments.push(Shipment {
            resource_type,
            amount,
            source_kind: SHIPMENT_SOURCE_OWA,
            source_building_id: usize::MAX,
            source_border_node: best_border,
            destination_building_id: dest_idx,
            carrier_class: CARRIER_TRUCK,
            status: SHIPMENT_IN_TRANSIT,
            total_cost,
            eta_hours: eta_hours_from_travel_seconds(adjusted_travel_seconds(
                best_cost,
                freight_profile,
                minute_of_day,
            )),
        });
        true
    }

    fn build_reservation_views(&self) -> (Vec<f32>, Vec<f32>, Vec<bool>, HashMap<u32, usize>) {
        let mut max_building = 0usize;
        for shipment in &self.shipments {
            max_building = max_building.max(shipment.destination_building_id);
            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL {
                max_building = max_building.max(shipment.source_building_id);
            }
        }

        let mut reserved_outbound = vec![0.0; max_building.saturating_add(1)];
        let mut reserved_inbound = vec![0.0; max_building.saturating_add(1)];
        let mut has_open_inbound = vec![false; max_building.saturating_add(1)];
        let mut border_job_counts = HashMap::new();

        for shipment in &self.shipments {
            if shipment.status != SHIPMENT_IN_TRANSIT {
                continue;
            }
            if shipment.destination_building_id < reserved_inbound.len() {
                reserved_inbound[shipment.destination_building_id] += shipment.amount;
                has_open_inbound[shipment.destination_building_id] = true;
            }
            if shipment.source_kind == SHIPMENT_SOURCE_LOCAL
                && shipment.source_building_id < reserved_outbound.len()
            {
                reserved_outbound[shipment.source_building_id] += shipment.amount;
            }
            if shipment.source_kind == SHIPMENT_SOURCE_OWA {
                *border_job_counts
                    .entry(shipment.source_border_node)
                    .or_insert(0) += 1;
            }
        }

        (
            reserved_outbound,
            reserved_inbound,
            has_open_inbound,
            border_job_counts,
        )
    }
}

fn connected_border_nodes(graph: &RegionGraph) -> Vec<u32> {
    graph
        .nodes()
        .iter()
        .enumerate()
        .filter_map(|(idx, node)| {
            if node.node_type != NodeType::Border {
                return None;
            }
            let connected = graph
                .node_adjacency(idx as u32)
                .iter()
                .any(|&edge_idx| !graph.edge(edge_idx).deleted);
            if connected { Some(idx as u32) } else { None }
        })
        .collect()
}

fn freight_profile_prefers_minute(profile: &FreightTimingProfile, minute_of_day: u16) -> bool {
    profile
        .preferred_windows
        .iter()
        .any(|window| minute_of_day >= window.start_minute && minute_of_day < window.end_minute)
}

fn adjusted_travel_seconds(
    travel_seconds: f32,
    profile: &FreightTimingProfile,
    minute_of_day: u16,
) -> f32 {
    if freight_profile_prefers_minute(profile, minute_of_day) {
        travel_seconds
    } else {
        travel_seconds + f32::from(profile.outside_window_eta_penalty_minutes) * 60.0
    }
}

fn adjusted_unit_price(unit_price: f32, profile: &FreightTimingProfile, minute_of_day: u16) -> f32 {
    if freight_profile_prefers_minute(profile, minute_of_day) {
        unit_price
    } else {
        unit_price * profile.outside_window_cost_multiplier
    }
}

fn eta_hours_from_travel_seconds(travel_seconds: f32) -> u16 {
    ((travel_seconds / OPERATIONAL_HOUR_SECONDS).ceil() as u16).max(1)
}

fn resource_kind_to_shipment_type(resource: StarterResourceKind) -> u8 {
    match resource {
        StarterResourceKind::StapleFood => RESOURCE_STAPLE_FOOD,
        StarterResourceKind::HouseholdSupplies => RESOURCE_HOUSEHOLD_SUPPLIES,
    }
}

fn shipment_type_to_resource_kind(resource_type: u8) -> Option<StarterResourceKind> {
    match resource_type {
        RESOURCE_STAPLE_FOOD => Some(StarterResourceKind::StapleFood),
        RESOURCE_HOUSEHOLD_SUPPLIES => Some(StarterResourceKind::HouseholdSupplies),
        _ => None,
    }
}

fn freight_profile_for_building<'a>(
    catalog: &RuntimeEconomyCatalog,
    tuning: &'a RuntimeEconomyTuning,
    building: &Building,
) -> Option<&'a FreightTimingProfile> {
    if let Some(profile_id) = catalog
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .and_then(|profile| profile.freight_timing_profile.as_deref())
        && let Some(profile) = tuning
            .operational_clock
            .freight_profiles
            .iter()
            .find(|profile| profile.id == profile_id)
    {
        return Some(profile);
    }
    tuning
        .operational_clock
        .freight_profile_for_zone_type(match building.zone_type {
            ZoneType::Commercial => "commercial",
            ZoneType::Industrial => "industrial",
            _ => return None,
        })
}

fn building_accepts_input_resource(
    catalog: &RuntimeEconomyCatalog,
    building: &Building,
    resource_type: u8,
) -> bool {
    let Some(resource) = shipment_type_to_resource_kind(resource_type) else {
        return false;
    };
    catalog
        .profile_by_runtime_id(building.economy_profile_runtime_id)
        .and_then(|profile| profile.input)
        .is_some_and(|input| input.resource == resource)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{
        Anchor, AnchorType, BuildingData, LodEntry, PlacementMode, ZoneClass,
    };
    use crate::simulation::buildings::allocator::{
        Building, resolve_building_economy_profile_binding,
    };
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::{EdgeClass, TransitFlags, TransitType};
    use godot::prelude::{Vector2, Vector3};

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        pack_id: &str,
        asset_id: &str,
        zone: ZoneClass,
    ) -> String {
        let (residents_capacity, worker_capacity) = match zone {
            ZoneClass::Residential => (Some(6), None),
            ZoneClass::Commercial | ZoneClass::Industrial | ZoneClass::Office => (None, Some(4)),
            ZoneClass::Mixed => (Some(4), Some(2)),
        };
        let economy_profile = match zone {
            ZoneClass::Commercial => Some("grocery_basic".to_owned()),
            ZoneClass::Industrial => Some("food_processor_basic".to_owned()),
            _ => None,
        };
        let manifest = AssetManifest {
            asset_id: asset_id.to_owned(),
            display_name: "Test".to_owned(),
            asset_set: None,
            tags: vec![],
            thumbnail: None,
            lods: vec![LodEntry {
                file: "lod0.glb".to_owned(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
            anchors: vec![Anchor {
                anchor_type: AnchorType::Entrance,
                name: "main".to_owned(),
                position: [0.0, 0.0, 0.5],
                forward: [0.0, 0.0, 1.0],
            }],
            building: Some(BuildingData {
                placement_mode: PlacementMode::ZonedPrivate,
                zone_type: Some(zone),
                density: Some("low".to_owned()),
                lot_width_cells: 1,
                lot_depth_cells: 1,
                min_zone_width_cells: None,
                min_zone_depth_cells: None,
                level: 1,
                residents_capacity,
                worker_capacity,
                service_class: None,
                economy_profile,
                preview_scale: Some(1.0),
            }),
            prop: None,
            vehicle: None,
            character: None,
            pivot_offset: None,
        };
        allocator
            .registry
            .register(pack_id, manifest, String::new());
        format!("{pack_id}:{asset_id}")
    }

    fn make_building(
        allocator: &BuildingAllocator,
        center_x: f32,
        zone_type: ZoneType,
        edge_idx: usize,
        asset_id: &str,
        stock: f32,
        budget: f32,
        utility: bool,
    ) -> Building {
        let economy_binding =
            resolve_building_economy_profile_binding(&allocator.registry, asset_id);
        Building {
            center_x,
            center_y: 10.0,
            width_cells: 2,
            depth_cells: 2,
            zone_profile_runtime_id: 0,
            zone_type,
            facing_dir: Vector2::new(0.0, 1.0),
            frontage_t: 0.5,
            side_offset: 1.0,
            abandoned_timer: 0,
            edge_idx,
            side: 1,
            cell_x: 0,
            cell_y: 0,
            occupancy: 0,
            worker_count: 0,
            asset_id: asset_id.to_owned(),
            level: 1,
            broken: false,
            economy_profile_runtime_id: economy_binding.runtime_id,
            economy_broken: economy_binding.economy_broken,
            stock,
            input_stock: 0.0,
            revenue: 0.0,
            operating_budget: budget,
            utility_service_available: utility,
            shipment_cooldown_hours: 0,
            pending_redevelopment: false,
            rezone_grace_days_remaining: 0,
        }
    }

    fn simple_graph_with_border() -> (RegionGraph, TransitNetwork, usize, usize, u32) {
        let mut graph = RegionGraph::new();
        let n0 = graph.add_node(Vector3::new(-100.0, 0.0, 0.0), NodeType::Border);
        let n1 = graph.add_node(Vector3::new(0.0, 0.0, 0.0), NodeType::Junction);
        let n2 = graph.add_node(Vector3::new(100.0, 0.0, 0.0), NodeType::Junction);
        let e0 = graph.add_edge(Edge {
            start_node: n0,
            end_node: n1,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 20.0,
            base_cost: 5.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(-100.0, 0.0, 0.0), Vector3::new(0.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        });
        let e1 = graph.add_edge(Edge {
            start_node: n1,
            end_node: n2,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR | TransitFlags::FOOT,
            class: EdgeClass::Standard,
            width: 7.0,
            fwd_lanes: 1,
            bkw_lanes: 1,
            speed_limit: 20.0,
            base_cost: 5.0,
            physical_length: 100.0,
            current_congestion: 0.0,
            start_clip: 0.0,
            end_clip: 0.0,
            geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            physical_geometry: vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(100.0, 0.0, 0.0)],
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access:
                crate::simulation::network::types::VehicleFrontageAccess::BothSides,
        });
        graph.rebuild_adjacency_list();

        let mut network = TransitNetwork::new();
        network.lane_system.rebuild(&mut graph);
        network.cch_graph = crate::simulation::pathing::cch::CchGraph::build(&graph);
        (graph, network, e0, e1, n0)
    }

    #[test]
    fn local_supplier_creates_and_delivers_shipment() {
        let (graph, network, industrial_edge, commercial_edge, _) = simple_graph_with_border();
        let mut allocator = BuildingAllocator::new();
        let industrial_asset = register_test_asset(
            &mut allocator,
            "test",
            "logistics_industrial",
            ZoneClass::Industrial,
        );
        let commercial_asset = register_test_asset(
            &mut allocator,
            "test",
            "logistics_commercial",
            ZoneClass::Commercial,
        );
        let supplier = make_building(
            &allocator,
            -50.0,
            ZoneType::Industrial,
            industrial_edge,
            &industrial_asset,
            300.0,
            0.0,
            true,
        );
        let destination = make_building(
            &allocator,
            50.0,
            ZoneType::Commercial,
            commercial_edge,
            &commercial_asset,
            100.0,
            2_000.0,
            true,
        );
        allocator.buildings.push(supplier);
        allocator.buildings.push(destination);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.rebuild_zone_index();

        let mut shipments = ShipmentSystem::new();
        shipments.hourly_tick(&mut allocator, &network, &graph, 480);

        assert_eq!(shipments.shipments.len(), 1);
        assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_LOCAL);
        assert_eq!(allocator.buildings[1].stock, 100.0);

        shipments.hourly_tick(&mut allocator, &network, &graph, 480);

        assert!(allocator.buildings[1].input_stock > 0.0);
        assert!(allocator.buildings[0].stock < 300.0);
        assert!(allocator.buildings[0].revenue > 0.0);
        assert!(
            shipments
                .shipments
                .iter()
                .all(|shipment| shipment.resource_type == RESOURCE_STAPLE_FOOD)
        );
    }

    #[test]
    fn owa_border_fallback_creates_import_shipment() {
        let (graph, network, _industrial_edge, commercial_edge, border_node) =
            simple_graph_with_border();
        let mut allocator = BuildingAllocator::new();
        let commercial_asset = register_test_asset(
            &mut allocator,
            "test",
            "owa_commercial",
            ZoneClass::Commercial,
        );
        let destination = make_building(
            &allocator,
            50.0,
            ZoneType::Commercial,
            commercial_edge,
            &commercial_asset,
            50.0,
            5_000.0,
            true,
        );
        allocator.buildings.push(destination);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.rebuild_zone_index();

        let mut shipments = ShipmentSystem::new();
        shipments.hourly_tick(&mut allocator, &network, &graph, 480);

        assert_eq!(shipments.shipments.len(), 1);
        assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_OWA);
        assert_eq!(shipments.shipments[0].source_border_node, border_node);

        shipments.hourly_tick(&mut allocator, &network, &graph, 480);
        assert!(shipments.shipments.is_empty());
        assert!(allocator.buildings[0].input_stock > 0.0);
    }

    #[test]
    fn owa_border_fallback_scales_import_to_affordable_amount() {
        let (graph, network, _industrial_edge, commercial_edge, _border_node) =
            simple_graph_with_border();
        let mut allocator = BuildingAllocator::new();
        let commercial_asset = register_test_asset(
            &mut allocator,
            "test",
            "owa_affordable_commercial",
            ZoneClass::Commercial,
        );
        let destination = make_building(
            &allocator,
            50.0,
            ZoneType::Commercial,
            commercial_edge,
            &commercial_asset,
            50.0,
            500.0,
            true,
        );
        allocator.buildings.push(destination);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.rebuild_zone_index();

        let mut shipments = ShipmentSystem::new();
        shipments.hourly_tick(&mut allocator, &network, &graph, 480);

        assert_eq!(shipments.shipments.len(), 1);
        assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_OWA);
        let catalog = load_runtime_economy_catalog().expect("runtime economy catalog");
        let grocery = catalog
            .profile_for_id("grocery_basic")
            .expect("grocery starter profile");
        assert!(shipments.shipments[0].amount >= grocery.min_shipment_units);
        assert!(shipments.shipments[0].amount < grocery.inventory_target_units());
        assert!(allocator.buildings[0].operating_budget <= 0.001);
    }

    #[test]
    fn inputless_industrial_profile_does_not_request_input_imports() {
        let (graph, network, industrial_edge, _commercial_edge, _border_node) =
            simple_graph_with_border();
        let mut allocator = BuildingAllocator::new();
        let industrial_asset = register_test_asset(
            &mut allocator,
            "test",
            "owa_industrial_inputs",
            ZoneClass::Industrial,
        );
        let destination = make_building(
            &allocator,
            -50.0,
            ZoneType::Industrial,
            industrial_edge,
            &industrial_asset,
            0.0,
            5_000.0,
            true,
        );
        allocator.buildings.push(destination);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.rebuild_zone_index();

        let mut shipments = ShipmentSystem::new();
        shipments.hourly_tick(&mut allocator, &network, &graph, 480);

        assert!(shipments.shipments.is_empty());
        assert_eq!(allocator.buildings[0].input_stock, 0.0);
    }
}
