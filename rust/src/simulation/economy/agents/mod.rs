//! Agent simulation: data layout, activity states, and lifecycle management.

pub mod data;
pub mod decisions;
pub mod tick;
#[cfg(test)]
mod tests;

pub use data::{Agent, AgentSystem, AgentVec};

/// Agent is inside a building — not moving. Waiting for the next activity trigger.
pub const TRANSIT_IDLE: u8 = 0;
/// Agent is walking from building interior to the road edge (departure animation phase).
pub const TRANSIT_DEPARTING: u8 = 1;
/// Agent is actively moving along road edges toward `target_node`.
pub const TRANSIT_ON_ROAD: u8 = 2;
/// Agent has reached the road node nearest their destination and is walking the final distance to the building.
pub const TRANSIT_ARRIVING: u8 = 3;
/// Newly spawned agent travelling from a highway border node to their first home — has no current building.
pub const TRANSIT_IMMIGRATING: u8 = 4;
/// Agent is traversing a bezier curve through a road intersection (lane-change phase).
pub const TRANSIT_INTERSECTION: u8 = 5;

// Transit Modes
/// Agent is walking on foot (sidewalks/crosswalks).
pub const MODE_WALK: u8 = 0;
/// Agent is driving a private car (road edges).
pub const MODE_CAR: u8 = 1;
/// Agent is cycling (sidewalks or road edges).
pub const MODE_BIKE: u8 = 2;
/// Agent is a passenger on a bus.
pub const MODE_BUS_PASSENGER: u8 = 3;
/// Agent is a passenger on a train/metro.
pub const MODE_TRAIN_PASSENGER: u8 = 4;
/// Agent is a passenger in a taxi.
pub const MODE_TAXI_PASSENGER: u8 = 5;
/// Agent is a passenger on a ship/ferry.
pub const MODE_SHIP_PASSENGER: u8 = 6;

// Vehicle Types (Civilians)
/// Default civilian sedan.
pub const VEHICLE_SEDAN: u8 = 0;
/// Faster/Sportier civilian sedan.
pub const VEHICLE_SPORTS: u8 = 1;
/// Basic civilian SUV.
pub const VEHICLE_SUV: u8 = 2;
/// Premium civilian SUV.
pub const VEHICLE_LUXURY: u8 = 3;
