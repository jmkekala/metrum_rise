// SPDX-License-Identifier: GPL-2.0-only

//! Shared freight planning context of one slot of the freight request queue.
//!
//! The context is assembled from persistent state (`SIM-02-I`): the maintained reservation
//! views, the freight topology cached by road-graph revision and the supplier index the rolling
//! sweep finished. It borrows them for the requests one slot serves (`SIM-02-J`) and hands them
//! back in [`FreightPlanningContext::finish`].

use std::sync::Arc;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::accessibility::{ModeComponentIndex, max_speed_for_modes};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::network::TransitNetwork;
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;

use super::data::ShipmentSystem;
use super::reservations::ReservationViews;
use super::route_cache::FreightRouteCache;
use super::routing::collect_connected_border_nodes;
use super::supplier_index::SupplierCandidateIndex;

/// Road-graph revisions a [`FreightTopology`] part was derived from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TopologyKeys {
    // CCH build generation and node-role revision of the border list.
    borders: Option<(u32, u64)>,
    // CCH build generation of the freight components.
    components: Option<u32>,
    // CCH build generation and metric revision of the top freight speed.
    speed: Option<(u32, u64)>,
}

/// Freight-car topology derived from the road graph, each part rebuilt only when the revisions
/// it reads change. Every topology edit rebuilds the CCH, so its build generation covers edges
/// and their modes; a border role set without one bumps the graph's node-role revision, and a
/// speed edit the metric revision. O(1) when unchanged.
#[derive(Clone, Debug, Default)]
pub(super) struct FreightTopology {
    keys: TopologyKeys,
    border_nodes: Vec<u32>,
    components: ModeComponentIndex,
    max_speed: f32,
}

impl FreightTopology {
    /// Brings every part up to date with `network` and `graph`: O(nodes) for a changed border
    /// list, O(nodes + edges) for changed components, O(edges) for a changed speed metric.
    pub(super) fn refresh(&mut self, network: &TransitNetwork, graph: &RegionGraph) {
        let generation = network.cch_graph.build_generation;
        let borders = Some((generation, graph.node_role_revision()));
        if self.keys.borders != borders {
            collect_connected_border_nodes(graph, &mut self.border_nodes);
            self.keys.borders = borders;
        }
        if self.keys.components != Some(generation) {
            self.components = ModeComponentIndex::build(graph, TransitFlags::CAR);
            self.keys.components = Some(generation);
        }
        let speed = Some((generation, network.cch_graph.metric_revision));
        if self.keys.speed != speed {
            self.max_speed = max_speed_for_modes(graph, TransitFlags::CAR).max(1.0);
            self.keys.speed = speed;
        }
    }

    /// Generation the components were built for; `None` before the first refresh.
    pub(super) fn components_generation(&self) -> Option<u32> {
        self.keys.components
    }

    /// Freight-car component labels of the last refresh.
    pub(super) fn components(&self) -> &ModeComponentIndex {
        &self.components
    }
}

/// Immutable indices and mutable reservation state shared by the freight requests of one slot.
pub(super) struct FreightPlanningContext {
    /// Cached authored runtime catalog used by the slot's freight planners.
    pub(super) catalog: Arc<RuntimeEconomyCatalog>,
    /// Cached authored runtime tuning used by the slot's freight planners.
    pub(super) tuning: Arc<RuntimeEconomyTuning>,
    /// Maintained reservation views, updated by new plans.
    pub(super) reservations: ReservationViews,
    /// Reachable `OWA` border nodes in deterministic graph order.
    pub(super) border_nodes: Vec<u32>,
    /// Freight-car connected-component labels for building entrances.
    pub(super) freight_components: ModeComponentIndex,
    /// Fastest car edge speed for lower-bound route scan pruning.
    pub(super) max_freight_speed: f32,
    /// Resource-keyed index of local suppliers reachable by freight components, keyed by
    /// building handle.
    pub(super) supplier_index: SupplierCandidateIndex,
    /// Revision-scoped exact freight route cache borrowed from the shipment system.
    pub(super) route_cache: FreightRouteCache,
    // Revisions of the borrowed topology.
    topology_keys: TopologyKeys,
    // Logistics hour of the supplier index, and whether this context built it rather than
    // borrowing the rolling sweep's front.
    supplier_index_hour: u32,
    supplier_index_built: bool,
}

impl FreightPlanningContext {
    /// Assembles the context for one slot's freight requests. O(borders) for the route-cache
    /// preparation, unless the road graph changed, the reservation views must be rebuilt, or no
    /// sweep finished this hour's supplier index: a world that has none (its first hour, a
    /// changed topology, an hour stepped without the sweep) builds one here, O(handles ×
    /// outputs), and keeps it as the front for the rest of the hour.
    pub(super) fn build(
        shipments: &mut ShipmentSystem,
        allocator: &mut BuildingAllocator,
        network: &TransitNetwork,
        graph: &RegionGraph,
    ) -> Self {
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        allocator
            .building_handles
            .register_appended(allocator.buildings.len());
        shipments.freight_topology.refresh(network, graph);
        let resource_count = catalog.resource_count();
        let reservations = shipments.take_reservation_views(resource_count);
        let hour = shipments.hour_ticks;
        let (supplier_index, supplier_index_built) = match shipments
            .supplier_roll
            .take_front(hour, shipments.freight_topology.components_generation())
        {
            Some(index) => (index, false),
            None => (
                SupplierCandidateIndex::build(
                    allocator,
                    graph,
                    &catalog,
                    shipments.freight_topology.components(),
                    &reservations,
                    tuning.logistics.truck_payload_kg,
                ),
                true,
            ),
        };
        let FreightTopology {
            keys: topology_keys,
            border_nodes,
            components: freight_components,
            max_speed: max_freight_speed,
        } = std::mem::take(&mut shipments.freight_topology);
        let mut route_cache = std::mem::take(&mut shipments.freight_route_cache);
        route_cache.prepare_buildings(allocator.buildings.len());
        route_cache.prepare_border_costs(&border_nodes, allocator.buildings.len(), network, graph);

        Self {
            catalog,
            tuning,
            reservations,
            border_nodes,
            freight_components,
            max_freight_speed,
            supplier_index,
            route_cache,
            topology_keys,
            supplier_index_hour: hour,
            supplier_index_built,
        }
    }

    /// Hands the route cache, reservation views, topology and supplier index back after the
    /// slot's planning; an index built here becomes the front for the rest of the hour, so the
    /// hour's later slots read the same listing.
    pub(super) fn finish(self, shipments: &mut ShipmentSystem) {
        shipments.freight_route_cache = self.route_cache;
        shipments.reservations = Some(self.reservations);
        shipments.freight_topology = FreightTopology {
            keys: self.topology_keys,
            border_nodes: self.border_nodes,
            components: self.freight_components,
            max_speed: self.max_freight_speed,
        };
        if self.supplier_index_built {
            let components = shipments.freight_topology.components_generation();
            shipments.supplier_roll.install_front(
                self.supplier_index_hour,
                components,
                self.supplier_index,
            );
        } else {
            shipments
                .supplier_roll
                .restore_front(self.supplier_index_hour, self.supplier_index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::economy::accessibility::NO_COMPONENT;
    use crate::simulation::network::graph::Edge;
    use crate::simulation::network::types::{
        EdgeClass, NodeType, TransitType, VehicleFrontageAccess,
    };
    use crate::simulation::pathing::cch::CchGraph;
    use godot::prelude::Vector3;

    #[test]
    fn freight_topology_rebuilds_only_the_parts_a_graph_change_reaches() {
        let (mut graph, mut network, first_edge, _, border) =
            super::super::tests::simple_graph_with_border();
        let mut topology = FreightTopology::default();
        topology.refresh(&network, &graph);
        assert_eq!(topology.border_nodes, [border]);
        assert_eq!(topology.max_speed, 20.0);
        let generation = topology.components_generation();
        assert!(generation.is_some());

        // A border role set without a topology edit or CCH rebuild.
        let far_end = graph.node_count() as u32 - 1;
        graph.set_node_type(far_end, NodeType::Border);
        topology.refresh(&network, &graph);
        assert_eq!(topology.border_nodes, [border, far_end]);
        assert_eq!(topology.components_generation(), generation);

        // A speed edit customizes the metric.
        graph.edge_mut(first_edge).speed_limit = 30.0;
        network.cch_graph.customize(&graph);
        topology.refresh(&network, &graph);
        assert_eq!(topology.max_speed, 30.0);
        assert_eq!(topology.components_generation(), generation);

        // A new road reaches the components once the edit rebuilds the CCH.
        let points = [
            Vector3::new(0.0, 0.0, 500.0),
            Vector3::new(100.0, 0.0, 500.0),
        ];
        let start = graph.add_node(points[0], NodeType::Junction);
        let end = graph.add_node(points[1], NodeType::Junction);
        graph.add_edge(Edge {
            start_node: start,
            end_node: end,
            primary_type: TransitType::Road,
            allowed_types: TransitFlags::CAR,
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
            geometry: points.to_vec(),
            physical_geometry: points.to_vec(),
            deleted: false,
            no_building_spawn: false,
            vehicle_frontage_access: VehicleFrontageAccess::BothSides,
        });
        graph.rebuild_adjacency_list();
        topology.refresh(&network, &graph);
        assert_eq!(topology.components().node_component(start), NO_COMPONENT);
        network.cch_graph = CchGraph::build(&graph);
        topology.refresh(&network, &graph);
        assert_ne!(topology.components_generation(), generation);
        let new_road = topology.components().node_component(start);
        assert_ne!(new_road, NO_COMPONENT);
        assert_ne!(new_road, topology.components().node_component(border));
    }
}
