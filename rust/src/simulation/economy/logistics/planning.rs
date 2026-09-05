// SPDX-License-Identifier: GPL-2.0-only

//! Shared hourly freight planning context.

use std::sync::Arc;

use crate::simulation::buildings::allocator::BuildingAllocator;
use crate::simulation::economy::accessibility::{ModeComponentIndex, max_speed_for_modes};
use crate::simulation::economy::definitions::{
    RuntimeEconomyCatalog, RuntimeEconomyTuning, load_runtime_economy_catalog,
    load_runtime_economy_tuning,
};
use crate::simulation::network::graph::RegionGraph;
use crate::simulation::network::types::TransitFlags;

use super::data::ShipmentSystem;
use super::reservations::ReservationViews;
use super::route_cache::FreightRouteCache;
use super::routing::connected_border_nodes;
use super::supplier_index::SupplierCandidateIndex;

/// Immutable indices and mutable reservation state shared by one hourly freight pass.
pub(super) struct FreightPlanningContext {
    /// Cached authored runtime catalog used by all freight planners this hour.
    pub(super) catalog: Arc<RuntimeEconomyCatalog>,
    /// Cached authored runtime tuning used by all freight planners this hour.
    pub(super) tuning: Arc<RuntimeEconomyTuning>,
    /// Runtime resource count captured from the catalog.
    pub(super) resource_count: usize,
    /// Reservation view seeded from existing shipments and updated by new plans.
    pub(super) reservations: ReservationViews,
    /// Reachable `OWA` border nodes in deterministic graph order.
    pub(super) border_nodes: Vec<u32>,
    /// Freight-car connected-component labels for building entrances.
    pub(super) freight_components: ModeComponentIndex,
    /// Fastest car edge speed for lower-bound route scan pruning.
    pub(super) max_freight_speed: f32,
    /// Resource-keyed index of local suppliers reachable by freight components.
    pub(super) supplier_index: SupplierCandidateIndex,
    /// Revision-scoped exact freight route cache borrowed from the shipment system.
    pub(super) route_cache: FreightRouteCache,
}

impl FreightPlanningContext {
    /// Builds the shared context once for an hourly input/export planning pass.
    pub(super) fn build(
        shipments: &mut ShipmentSystem,
        allocator: &BuildingAllocator,
        graph: &RegionGraph,
    ) -> Self {
        let tuning = load_runtime_economy_tuning()
            .unwrap_or_else(|err| panic!("could not load built-in economy runtime tuning: {err}"));
        let catalog = load_runtime_economy_catalog()
            .unwrap_or_else(|err| panic!("could not load built-in runtime economy catalog: {err}"));
        let resource_count = catalog.resource_count();
        let reservations = shipments.build_reservation_views(resource_count);
        let border_nodes = connected_border_nodes(graph);
        let freight_components = ModeComponentIndex::build(graph, TransitFlags::CAR);
        let max_freight_speed = max_speed_for_modes(graph, TransitFlags::CAR).max(1.0);
        let supplier_index =
            SupplierCandidateIndex::build(allocator, graph, &catalog, &freight_components);
        let route_cache = std::mem::take(&mut shipments.freight_route_cache);

        Self {
            catalog,
            tuning,
            resource_count,
            reservations,
            border_nodes,
            freight_components,
            max_freight_speed,
            supplier_index,
            route_cache,
        }
    }

    /// Restores the route cache after hourly freight planning has finished.
    pub(super) fn finish(self, shipments: &mut ShipmentSystem) {
        shipments.freight_route_cache = self.route_cache;
    }
}
