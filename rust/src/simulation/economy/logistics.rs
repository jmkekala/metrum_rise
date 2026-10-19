// SPDX-License-Identifier: GPL-2.0-only

//! Batched building-level freight reservations and delayed deliveries.
//!
//! The first shipment slice keeps freight explicit without introducing per-order
//! micro-deliveries. Commercial buildings open bounded restock requests,
//! industrial suppliers reserve stock for them, and `OWA` border terminals act
//! as the external fallback for ordinary imported goods. Buildings request freight at their
//! hourly phases and the `freight requests` queue plans it (`SIM-02-J`, `requests.rs`).

mod data;
mod exports;
mod inbound;
mod local_supplier;
mod owa_import;
mod planning;
mod progress;
mod quantization;
mod recurring_supply;
mod requests;
mod reservations;
mod resource;
mod route_cache;
mod routing;
mod supplier_index;
mod tick;
mod timing;
mod working_capital;

#[cfg(test)]
mod tests;

pub(crate) use self::data::ShipmentBuildingUndo;
pub use self::data::{
    CarrierClass, FreightRequestFailure, FreightRequestKey, Shipment, ShipmentEndpoint,
    ShipmentStatus, ShipmentSystem,
};
pub(crate) use self::quantization::{export_dispatch_target_kg, quantize_requested_amount};
pub(crate) use self::recurring_supply::{ProjectedSupplySource, RecurringSupply};
pub(crate) use self::requests::{FreightRequest, FreightRequestKind};
pub(crate) use self::resource::{
    building_outputs_can_export_to_owa, building_outputs_can_supply_local_inputs,
    freight_goods_multiplier, imported_input_unit_price,
};
pub(crate) use self::routing::collect_connected_border_nodes;
pub(crate) use self::supplier_index::{SavedSupplierRoll, SavedSupplierSweep, SupplierRoll};
pub(crate) use self::timing::freight_price_phase;
