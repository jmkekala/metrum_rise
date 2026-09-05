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
mod reservations;
mod resource;
mod route_cache;
mod routing;
mod supplier_index;
mod tick;
mod timing;

#[cfg(test)]
mod tests;

pub(crate) use self::data::ShipmentBuildingUndo;
pub use self::data::{
    CarrierClass, FreightRequestFailure, FreightRequestKey, Shipment, ShipmentEndpoint,
    ShipmentStatus, ShipmentSystem,
};
pub(crate) use self::routing::has_connected_border_node;
