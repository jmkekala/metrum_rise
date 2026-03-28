#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitType {
    Road,
    Rail,
    Ship,
    Air,
    Foot,
}

#[allow(dead_code)]
pub struct TransitFlags;
#[allow(dead_code)]
impl TransitFlags {
    pub const NONE: u8 = 0;
    pub const FOOT: u8 = 1 << 0;
    pub const CAR: u8 = 1 << 1;
    pub const RAIL: u8 = 1 << 2;
    pub const SHIP: u8 = 1 << 3;
    pub const AIR: u8 = 1 << 4;
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    Junction,
    Station,
    Harbor,
    Airport,
    Transfer,
    /// A road endpoint that has been designated as an external connection to the region.
    ///
    /// Agents (immigrants) spawn at Border nodes and leave through them. A Border node only
    /// functions as a spawn point if it has at least one non-deleted incident edge (i.e. the
    /// road is actually connected to the city network).
    Border,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeClass {
    Standard,
    Bridge,
    Tunnel,
}
