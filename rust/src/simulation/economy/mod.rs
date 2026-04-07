//! Economic simulation foundations: explicit households, agent movement, and
//! temporary city-growth demand counters.
//!
//! [`agents::AgentSystem`] owns movement and transit state in Structure-of-Arrays
//! form. [`households::HouseholdSystem`] owns the first-pass building-centric
//! household economy loop with explicit household budgets, replenishment, and
//! work/home planning. [`logistics::ShipmentSystem`] owns batched building-level
//! freight reservations and delayed deliveries between suppliers, stores, and
//! `OWA` border terminals.
//!
//! [`demand::DemandSystem`] remains temporarily in place for zoning-driven building
//! growth until the full economy-authored construction loop replaces it.

pub mod agents;
pub mod demand;
pub mod households;
pub mod logistics;
