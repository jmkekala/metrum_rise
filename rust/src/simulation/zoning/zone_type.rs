//! Broad land-use families derived from zoning profiles.

/// Land-use category assigned to a zoning profile.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Hash)]
#[repr(u8)]
pub enum ZoneType {
    /// No zoning or no private land-use family.
    #[default]
    None = 0,
    /// Residential housing: agents live here and consumes residential demand.
    Residential = 1,
    /// Retail and services: agents shop and work here, consuming commercial demand.
    Commercial = 2,
    /// Manufacturing and logistics: agents work here, consuming industrial demand.
    Industrial = 3,
    /// Office employment reserved for a later explicit extension.
    Office = 4,
    /// Mixed residential/commercial use reserved for a later explicit extension.
    Mixed = 5,
}

impl ZoneType {
    /// Converts a raw `u8` to a `ZoneType`. Unknown values map to `None`.
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Residential,
            2 => Self::Commercial,
            3 => Self::Industrial,
            4 => Self::Office,
            5 => Self::Mixed,
            _ => Self::None,
        }
    }

    /// Returns the canonical snake-case string key for this zone family.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Residential => "residential",
            Self::Commercial => "commercial",
            Self::Industrial => "industrial",
            Self::Office => "office",
            Self::Mixed => "mixed",
        }
    }
}
