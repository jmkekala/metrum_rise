//! Economic simulation foundations: explicit households, agent movement, and
//! live economy-pressure counters.
//!
//! [`agents::AgentSystem`] owns movement and transit state in Structure-of-Arrays
//! form. [`households::HouseholdSystem`] owns the first-pass building-centric
//! household economy loop with explicit household budgets, replenishment, and
//! work/home planning. [`logistics::ShipmentSystem`] owns batched building-level
//! freight reservations and delayed deliveries between suppliers, stores, and
//! `OWA` border terminals.
//!
//! [`demand::DemandSystem`] remains temporarily in place as a lightweight
//! pressure/telemetry layer and save-loaded runtime signal while the full
//! authored company-formation and construction loop is built. It no longer
//! auto-spawns private buildings from zoned land.

pub mod agents;
pub mod demand;
pub mod definitions;
pub mod households;
pub mod logistics;
