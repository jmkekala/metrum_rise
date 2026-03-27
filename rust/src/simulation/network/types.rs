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
    /// Merge or diverge point: one mainline pair (roughly anti-parallel) plus
    /// one or more ramp edges. The mainline is not clipped here; only ramp
    /// edges are clipped and rendered with a taper strip.
    Merge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeClass {
    Standard,
    Bridge,
    Tunnel,
    /// Approach road joining a mainline at an acute angle (on-ramp / off-ramp).
    /// Clipped normally; rendered with a taper strip at the Merge node end.
    Ramp,
}
