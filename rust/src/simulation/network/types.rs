//! Common types and enumerations for the transportation network.

/// The mode of transport for an edge or lane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransitType {
    /// Standard road for vehicles and pedestrians.
    #[default]
    Road,
    /// Rail-based transport.
    Rail,
    /// Water-based transport.
    Ship,
    /// Air-based transport.
    Air,
    /// Dedicated pedestrian-only paths.
    Foot,
}

/// Bit-flags used to filter edges and lanes by allowed transport modes.
pub struct TransitFlags;
impl TransitFlags {
    /// No transport allowed.
    pub const NONE: u8 = 0;
    /// Pedestrians allowed.
    pub const FOOT: u8 = 1 << 0;
    /// Road vehicles (cars, buses, trucks) allowed.
    pub const CAR: u8 = 1 << 1;
    /// Trains and trams allowed.
    pub const RAIL: u8 = 1 << 2;
    /// Ships allowed.
    pub const SHIP: u8 = 1 << 3;
    /// Aircraft allowed.
    pub const AIR: u8 = 1 << 4;
}

/// The functional role of a network node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NodeType {
    /// Standard road intersection or endpoint.
    #[default]
    Junction,
    /// Public transport stop or station.
    Station,
    /// Port or harbor facility.
    Harbor,
    /// Airport facility.
    Airport,
    /// Multi-modal transfer point.
    Transfer,
    /// A road endpoint that has been designated as an external connection to the region.
    ///
    /// Agents (immigrants) spawn at Border nodes and leave through them. A Border node only
    /// functions as a spawn point if it has at least one non-deleted incident edge (i.e. the
    /// road is actually connected to the city network).
    Border,
}

/// Architectural classification of a road edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EdgeClass {
    /// Standard surface-level road.
    #[default]
    Standard,
    /// Elevated road with structural components (deck, walls, pillars).
    Bridge,
    /// Underground or covered road.
    Tunnel,
}
