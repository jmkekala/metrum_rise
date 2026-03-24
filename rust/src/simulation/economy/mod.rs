//! Economic simulation: agent state machines, global demand tracking, and immigration.
//!
//! [`agents::AgentSystem`] is the core of this module — it owns all agent state in SoA layout
//! and drives the Home→Work→Shop activity cycle each tick.
//!
//! [`demand::DemandSystem`] tracks global R/C/I demand counters consumed by the building allocator.

pub mod demand;
pub mod agents;
pub mod agents_test;
