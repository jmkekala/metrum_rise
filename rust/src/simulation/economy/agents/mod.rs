//! Agent simulation: data layout, activity states, and lifecycle management.

mod building_refs;
mod daily;
pub mod data;
mod lifecycle;
mod remap;
#[cfg(test)]
mod test_departure_side;
#[cfg(test)]
mod tests;
pub mod tick;

pub use data::{Agent, AgentSystem, AgentVec};

/// Agent is inside a building and hidden until the next trip trigger fires.
pub const TRANSIT_IN_BUILDING: u8 = 0;
/// Agent is traversing the short local segment from the building entry point to the network.
pub const TRANSIT_ACCESS_EGRESS: u8 = 1;
/// Agent is traversing the live lane/path network.
pub const TRANSIT_NETWORK: u8 = 2;
/// Agent is traversing the short local segment from the network into the destination building.
pub const TRANSIT_ACCESS_INGRESS: u8 = 3;
/// Border-spawn transport state used by household arrival carriers and exceptional/manual arrivals.
pub const TRANSIT_IMMIGRATING: u8 = 4;
/// Agent is traversing a bezier curve through a road intersection (lane-change phase).
pub const TRANSIT_INTERSECTION: u8 = 5;

/// Returns whether an agent in `transit` should be rendered in the live world.
pub(crate) fn transit_is_visible(transit: u8) -> bool {
    matches!(
        transit,
        TRANSIT_ACCESS_EGRESS
            | TRANSIT_NETWORK
            | TRANSIT_ACCESS_INGRESS
            | TRANSIT_IMMIGRATING
            | TRANSIT_INTERSECTION
    )
}

/// Trip-plan bit: the `planned_*` scalars contain a valid authoritative access/network plan.
pub const ACCESS_PLAN_VALID: u8 = 0x01;
/// Trip-plan bit: the node-path portion is zero-hop because attach and detach nodes match.
pub const ACCESS_ZERO_HOP_NODE_PATH: u8 = 0x02;
/// Trip-plan bit: the current path came from a validated flow-field fast path.
pub const ACCESS_PATH_FROM_FLOW_FIELD: u8 = 0x04;
/// Trip-plan bit: the trip originated from a border-node immigration spawn, not a building egress.
pub const ACCESS_IMMIGRATION_ORIGIN: u8 = 0x08;

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
