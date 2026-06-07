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
mod progress;
mod reservations;
mod resource;
mod routing;
mod tick;
mod timing;

#[cfg(test)]
mod tests;

pub use self::data::{
    CARRIER_TRUCK, SHIPMENT_DEST_OWA, SHIPMENT_FAILED, SHIPMENT_FULFILLED, SHIPMENT_IN_TRANSIT,
    SHIPMENT_SOURCE_LOCAL, SHIPMENT_SOURCE_OWA, Shipment, ShipmentSystem,
};
