//! Runtime zoning-profile value types.

use crate::simulation::zoning::ZoneType;

/// Density band for a zoning profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ZoneDensity {
    /// Low-density development.
    Low,
    /// Medium-density development.
    Medium,
    /// High-density development.
    High,
}

impl ZoneDensity {
    /// Parses one authored density string.
    pub fn from_str_name(value: &str) -> Option<Self> {
        match value.trim() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }

    /// Returns the canonical snake-case density key.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// One validated runtime zoning profile.
#[derive(Clone, Debug)]
pub struct ZoneProfileRuntime {
    /// Runtime profile id assigned to parcels. `0` is reserved for free/unzoned parcels.
    pub runtime_id: u16,
    /// Stable authored TOML id.
    pub id: String,
    /// Player-facing display name.
    pub display_name: String,
    /// Deterministic UI ordering key inside one top-level category.
    pub ui_order: u32,
    /// Broad zone family derived from the profile.
    pub zone_type: ZoneType,
    /// Density band for this profile.
    pub density: ZoneDensity,
    /// Secondary required asset tags for legality filtering.
    pub required_asset_tags: Vec<String>,
    /// Matching demand-side growth profile id.
    pub growth_profile_id: String,
    /// Parsed RGB colour used by the UI and overlay LUT.
    pub ui_color_rgb: [u8; 3],
    /// Stable UI icon key.
    pub ui_icon: String,
    /// Player-facing tooltip/description text.
    pub ui_description: String,
}
