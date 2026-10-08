// SPDX-License-Identifier: GPL-2.0-only

//! Batched building-level freight reservations and delayed deliveries.
//!
//! The first shipment slice keeps freight explicit without introducing per-order
//! micro-deliveries. Commercial buildings open bounded restock requests,
//! industrial suppliers reserve stock for them, and `OWA` border terminals act
//! as the external fallback for ordinary imported goods.

mod data;
mod exports;
mod inbound;
mod local_supplier;
mod owa_import;
mod planning;
mod progress;
mod quantization;
mod recurring_supply;
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
pub(crate) use self::recurring_supply::RecurringSupply;
pub(crate) use self::resource::{building_outputs_can_export_to_owa, freight_goods_multiplier};
pub(crate) use self::timing::freight_price_phase;
pub(crate) use self::routing::{collect_connected_border_nodes, has_connected_border_node};
