//! Batched building-level freight reservations and delayed deliveries.
//!
//! The first shipment slice keeps freight explicit without introducing per-order
//! micro-deliveries. Commercial buildings open bounded restock requests,
//! industrial suppliers reserve stock for them, and `OWA` border terminals act
//! as the external fallback for ordinary imported goods.

use std::collections::HashMap;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::grid::zoning::ZoneType;
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::{NodeType, TransitFlags};

/// First-pass shipped resource used by the starter economy chain.
pub const RESOURCE_HOUSEHOLD_SUPPLIES: u8 = 0;
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

const COMMERCIAL_STOCK_TARGET_UNITS: f32 = 600.0;
const COMMERCIAL_REORDER_UNITS: f32 = 400.0;
const COMMERCIAL_CRITICAL_UNITS: f32 = 100.0;
const COMMERCIAL_MIN_SHIPMENT_UNITS: f32 = 40.0;
const WHOLESALE_UNIT_PRICE: f32 = 5.0;
const OWA_IMPORT_ASK: f32 = 8.0;
const SUPPLIER_SEARCH_MAX_RING: i32 = 3;
const SUPPLIER_SEARCH_CANDIDATES: usize = 8;
const SHIPMENT_RETRY_COOLDOWN_DAYS: u8 = 1;
const BORDER_ACTIVE_JOBS_PER_NODE: usize = 4;
const OPERATIONAL_DAY_SECONDS: f32 = 24.0 * 60.0 * 60.0;

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
    /// Remaining daily economy steps before the shipment arrives.
    pub eta_days: u8,
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

    /// Advances freight deliveries and opens new bounded restock jobs.
    pub fn daily_tick(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        self.progress_shipments(allocator);
        self.decrement_building_cooldowns(allocator);
        self.create_commercial_restock_shipments(allocator, transit_network, graph);
        self.shipments
            .retain(|shipment| shipment.status == SHIPMENT_IN_TRANSIT);
    }

    fn progress_shipments(&mut self, allocator: &mut BuildingAllocator) {
        for shipment in &mut self.shipments {
            if shipment.status != SHIPMENT_IN_TRANSIT {
                continue;
            }

            if shipment.eta_days > 0 {
                shipment.eta_days -= 1;
            }
            if shipment.eta_days > 0 {
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
                        || allocator.buildings[src_idx].stock < shipment.amount
                    {
                        allocator.buildings[dest_idx].operating_budget += shipment.total_cost;
                        allocator.buildings[dest_idx].shipment_cooldown_days =
                            SHIPMENT_RETRY_COOLDOWN_DAYS;
                        shipment.status = SHIPMENT_FAILED;
                        continue;
                    }

                    allocator.buildings[src_idx].stock -= shipment.amount;
                    allocator.buildings[src_idx].revenue += shipment.total_cost;
                    allocator.buildings[src_idx].operating_budget += shipment.total_cost;
                    allocator.buildings[dest_idx].stock += shipment.amount;
                    shipment.status = SHIPMENT_FULFILLED;
                }
                SHIPMENT_SOURCE_OWA => {
                    allocator.buildings[dest_idx].stock += shipment.amount;
                    shipment.status = SHIPMENT_FULFILLED;
                }
                _ => {
                    allocator.buildings[dest_idx].operating_budget += shipment.total_cost;
                    allocator.buildings[dest_idx].shipment_cooldown_days =
                        SHIPMENT_RETRY_COOLDOWN_DAYS;
                    shipment.status = SHIPMENT_FAILED;
                }
            }
        }
    }

    fn decrement_building_cooldowns(&self, allocator: &mut BuildingAllocator) {
        for building in &mut allocator.buildings {
            if building.shipment_cooldown_days > 0 {
                building.shipment_cooldown_days -= 1;
            }
        }
    }

    fn create_commercial_restock_shipments(
        &mut self,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
    ) {
        let (reserved_outbound, reserved_inbound, has_open_inbound, border_job_counts) =
            self.build_reservation_views();

        let border_nodes = connected_border_nodes(graph);

        for dest_idx in 0..allocator.buildings.len() {
            let building = &allocator.buildings[dest_idx];
            if !matches!(building.zone_type, ZoneType::Commercial | ZoneType::Mixed)
                || building.broken
                || building.edge_idx == usize::MAX
                || has_open_inbound.get(dest_idx).copied().unwrap_or(false)
                || building.shipment_cooldown_days > 0
            {
                continue;
            }

            let effective_stock =
                building.stock + reserved_inbound.get(dest_idx).copied().unwrap_or(0.0);
            if effective_stock >= COMMERCIAL_REORDER_UNITS {
                continue;
            }

            let allow_emergency = effective_stock <= COMMERCIAL_CRITICAL_UNITS;
            let desired_amount = (COMMERCIAL_STOCK_TARGET_UNITS - effective_stock).max(0.0);
            if desired_amount <= 0.0 {
                continue;
            }
            if desired_amount < COMMERCIAL_MIN_SHIPMENT_UNITS && !allow_emergency {
                continue;
            }

            if self.try_local_supplier(
                dest_idx,
                desired_amount,
                allow_emergency,
                allocator,
                transit_network,
                graph,
                &reserved_outbound,
            ) {
                continue;
            }

            if self.try_owa_fallback(
                dest_idx,
                desired_amount,
                allow_emergency,
                allocator,
                transit_network,
                graph,
                &border_nodes,
                &border_job_counts,
            ) {
                continue;
            }

            allocator.buildings[dest_idx].shipment_cooldown_days = SHIPMENT_RETRY_COOLDOWN_DAYS;
        }
    }

    fn try_local_supplier(
        &mut self,
        dest_idx: usize,
        desired_amount: f32,
        allow_emergency: bool,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        reserved_outbound: &[f32],
    ) -> bool {
        if dest_idx >= allocator.entrances.len() {
            return false;
        }
        let destination = &allocator.buildings[dest_idx];
        let candidates = allocator.find_nearby_buildings_by_zones(
            destination.center_x,
            destination.center_y,
            &[ZoneType::Industrial],
            SUPPLIER_SEARCH_MAX_RING,
            SUPPLIER_SEARCH_CANDIDATES,
        );

        for candidate_idx in candidates {
            if candidate_idx == dest_idx || candidate_idx >= allocator.buildings.len() {
                continue;
            }
            let supplier = &allocator.buildings[candidate_idx];
            if supplier.broken || !supplier.utility_service_available {
                continue;
            }

            let reserved = reserved_outbound.get(candidate_idx).copied().unwrap_or(0.0);
            let available = (supplier.stock - reserved).max(0.0);
            if available <= 0.0 {
                continue;
            }

            let amount = available.min(desired_amount);
            if amount < COMMERCIAL_MIN_SHIPMENT_UNITS && !allow_emergency {
                continue;
            }

            let total_cost = amount * WHOLESALE_UNIT_PRICE;
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
                resource_type: RESOURCE_HOUSEHOLD_SUPPLIES,
                amount,
                source_kind: SHIPMENT_SOURCE_LOCAL,
                source_building_id: candidate_idx,
                source_border_node: u32::MAX,
                destination_building_id: dest_idx,
                carrier_class: CARRIER_TRUCK,
                status: SHIPMENT_IN_TRANSIT,
                total_cost,
                eta_days: eta_days_from_travel_seconds(travel_seconds),
            });
            return true;
        }

        false
    }

    fn try_owa_fallback(
        &mut self,
        dest_idx: usize,
        desired_amount: f32,
        allow_emergency: bool,
        allocator: &mut BuildingAllocator,
        transit_network: &TransitNetwork,
        graph: &RegionGraph,
        border_nodes: &[u32],
        border_job_counts: &HashMap<u32, usize>,
    ) -> bool {
        if border_nodes.is_empty() {
            return false;
        }

        let min_amount = if desired_amount < COMMERCIAL_MIN_SHIPMENT_UNITS && allow_emergency {
            desired_amount
        } else {
            COMMERCIAL_MIN_SHIPMENT_UNITS
        };
        let max_affordable_amount = allocator.buildings[dest_idx].operating_budget / OWA_IMPORT_ASK;
        if max_affordable_amount < min_amount {
            return false;
        }
        let amount = desired_amount.max(min_amount).min(max_affordable_amount);
        let total_cost = amount * OWA_IMPORT_ASK;

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
            resource_type: RESOURCE_HOUSEHOLD_SUPPLIES,
            amount,
            source_kind: SHIPMENT_SOURCE_OWA,
            source_building_id: usize::MAX,
            source_border_node: best_border,
            destination_building_id: dest_idx,
            carrier_class: CARRIER_TRUCK,
            status: SHIPMENT_IN_TRANSIT,
            total_cost,
            eta_days: eta_days_from_travel_seconds(best_cost),
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

fn eta_days_from_travel_seconds(travel_seconds: f32) -> u8 {
    ((travel_seconds / OPERATIONAL_DAY_SECONDS).ceil() as u8).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::AssetManifest;
    use crate::assets::asset::{Anchor, AnchorType, BuildingData, LodEntry, ZoneClass};
    use crate::simulation::buildings::allocator::Building;
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::{EdgeClass, TransitType};
    use godot::prelude::{Vector2, Vector3};

    fn register_test_asset(
        allocator: &mut BuildingAllocator,
        pack_id: &str,
        asset_id: &str,
        zone: ZoneClass,
    ) -> String {
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
                zone_type: zone,
                density: "low".to_owned(),
                lot_width_cells: 1,
                lot_depth_cells: 1,
                level: 1,
                residents_capacity: Some(6),
                worker_capacity: None,
                service_class: None,
                economy_profile: None,
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
        center_x: f32,
        zone_type: ZoneType,
        edge_idx: usize,
        stock: f32,
        budget: f32,
        utility: bool,
    ) -> Building {
        Building {
            center_x,
            center_y: 10.0,
            width_cells: 2,
            depth_cells: 2,
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
            asset_id: String::new(),
            level: 1,
            broken: false,
            stock,
            revenue: 0.0,
            operating_budget: budget,
            utility_service_available: utility,
            shipment_cooldown_days: 0,
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
        let mut supplier = make_building(
            -50.0,
            ZoneType::Industrial,
            industrial_edge,
            300.0,
            0.0,
            true,
        );
        supplier.asset_id = industrial_asset;
        let mut destination = make_building(
            50.0,
            ZoneType::Commercial,
            commercial_edge,
            100.0,
            2_000.0,
            true,
        );
        destination.asset_id = commercial_asset;
        allocator.buildings.push(supplier);
        allocator.buildings.push(destination);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.rebuild_zone_index();

        let mut shipments = ShipmentSystem::new();
        shipments.daily_tick(&mut allocator, &network, &graph);

        assert_eq!(shipments.shipments.len(), 1);
        assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_LOCAL);
        assert_eq!(allocator.buildings[1].stock, 100.0);

        shipments.daily_tick(&mut allocator, &network, &graph);

        assert!(shipments.shipments.is_empty());
        assert!(allocator.buildings[1].stock > 100.0);
        assert!(allocator.buildings[0].stock < 300.0);
        assert!(allocator.buildings[0].revenue > 0.0);
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
        let mut destination = make_building(
            50.0,
            ZoneType::Commercial,
            commercial_edge,
            50.0,
            5_000.0,
            true,
        );
        destination.asset_id = commercial_asset;
        allocator.buildings.push(destination);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.rebuild_zone_index();

        let mut shipments = ShipmentSystem::new();
        shipments.daily_tick(&mut allocator, &network, &graph);

        assert_eq!(shipments.shipments.len(), 1);
        assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_OWA);
        assert_eq!(shipments.shipments[0].source_border_node, border_node);

        shipments.daily_tick(&mut allocator, &network, &graph);
        assert!(shipments.shipments.is_empty());
        assert!(allocator.buildings[0].stock > 50.0);
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
        let mut destination = make_building(
            50.0,
            ZoneType::Commercial,
            commercial_edge,
            50.0,
            500.0,
            true,
        );
        destination.asset_id = commercial_asset;
        allocator.buildings.push(destination);
        allocator.rebuild_entrance_cache(&graph, &network.lane_system);
        allocator.rebuild_zone_index();

        let mut shipments = ShipmentSystem::new();
        shipments.daily_tick(&mut allocator, &network, &graph);

        assert_eq!(shipments.shipments.len(), 1);
        assert_eq!(shipments.shipments[0].source_kind, SHIPMENT_SOURCE_OWA);
        assert!(shipments.shipments[0].amount >= COMMERCIAL_MIN_SHIPMENT_UNITS);
        assert!(shipments.shipments[0].amount < COMMERCIAL_STOCK_TARGET_UNITS);
        assert!(allocator.buildings[0].operating_budget <= 0.001);
    }
}
