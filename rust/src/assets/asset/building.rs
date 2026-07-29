//! Building asset schema and frontage compatibility rules.

use super::{AnchorType, AssetManifest};
use serde::Deserialize;

const ANCHOR_FORWARD_UNIT_EPS: f32 = 0.02;
const DEFAULT_BUILDING_FRONTAGE_FORWARD: [f32; 3] = [0.0, 0.0, 1.0];

// ── Zone / land-use ───────────────────────────────────────────────────────────

/// Land-use category for a zoned building asset.
///
/// Maps onto [`crate::simulation::zoning::ZoneType`] during registry step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ZoneClass {
    /// Residential housing.
    Residential,
    /// Retail and services.
    Commercial,
    /// Manufacturing and logistics.
    Industrial,
    /// Office employment reserved for a later explicit extension.
    Office,
    /// Mixed residential/commercial use reserved for a later explicit extension.
    Mixed,
}

/// Placement contract for one building asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementMode {
    /// Ordinary private building that participates in painted zoning legality and growth.
    ZonedPrivate,
    /// Explicitly placed building that stays outside painted zoning legality.
    Explicit,
}

// ── Building ──────────────────────────────────────────────────────────────────

/// Class-specific data for a building asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildingData {
    /// How this building enters the world.
    #[serde(default = "default_placement_mode")]
    pub placement_mode: PlacementMode,
    /// Land-use category this building satisfies when placed through painted zoning.
    pub zone_type: Option<ZoneClass>,
    /// Density tier for painted-zoning legality.
    pub density: Option<String>,
    /// Footprint width in zoning cells (along the road).
    pub lot_width_cells: u16,
    /// Footprint depth in zoning cells (away from the road).
    pub lot_depth_cells: u16,
    /// Asset-local direction of the road-facing building frontage.
    ///
    /// Older manifests may omit this; in that case the main entrance anchor forward is used as
    /// the legacy frontage direction.
    #[serde(default)]
    pub frontage_forward: Option<[f32; 3]>,
    /// Minimum zoned width accepted for this building. Defaults to `lot_width_cells`.
    pub min_zone_width_cells: Option<u16>,
    /// Minimum zoned depth accepted for this building. Defaults to `lot_depth_cells`.
    pub min_zone_depth_cells: Option<u16>,
    /// Growth tier within the asset family identified by `asset_set`.
    /// `1` = base tier (default). Buildings without `asset_set` ignore this field.
    #[serde(default = "default_level")]
    pub level: u8,
    /// Maximum number of households this building can house. Required for residential zones.
    pub household_capacity: Option<u32>,
    /// Maximum number of workers this building can employ. Required for commercial/industrial zones.
    pub worker_capacity: Option<u32>,
    /// Target floor area per household in square meters.
    pub flat_size_m2: Option<f32>,
    /// Service tier label used by demand weighting (e.g. `"standard"`, `"premium"`).
    pub service_class: Option<String>,
    /// Reference to an authored economy profile defined in the exported economy catalog.
    pub economy_profile: Option<String>,
    /// Resource extraction contract for explicitly placed extractor buildings.
    pub extractor: Option<BuildingExtractorData>,
}

/// Authored extraction behavior for one explicit industry building.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildingExtractorData {
    /// Authored resource id this building extracts, such as `"coal"`.
    pub resource: String,
    /// Extraction area ownership mode. Version one supports `"player_polygon"`.
    pub area_mode: String,
}

fn default_placement_mode() -> PlacementMode {
    PlacementMode::ZonedPrivate
}

fn default_level() -> u8 {
    1
}

impl BuildingData {
    /// Returns `true` when this building participates in painted zoning.
    pub fn is_zoned_private(&self) -> bool {
        self.placement_mode == PlacementMode::ZonedPrivate
    }

    /// Returns the authored zoning density key when present.
    pub fn density_key(&self) -> Option<&str> {
        self.density.as_deref()
    }

    /// Returns the minimum zoned width for this building.
    pub fn effective_min_zone_width_cells(&self) -> u16 {
        self.min_zone_width_cells.unwrap_or(self.lot_width_cells)
    }

    /// Returns the minimum zoned depth for this building.
    pub fn effective_min_zone_depth_cells(&self) -> u16 {
        self.min_zone_depth_cells.unwrap_or(self.lot_depth_cells)
    }
}

impl AssetManifest {
    /// Returns the building frontage direction, with legacy driveway/entrance fallbacks.
    pub(crate) fn building_frontage_forward(&self) -> [f32; 3] {
        self.building
            .as_ref()
            .and_then(|building| building.frontage_forward)
            .or_else(|| self.legacy_driveway_frontage_forward())
            .or_else(|| {
                self.anchors
                    .iter()
                    .find(|anchor| {
                        anchor.anchor_type == AnchorType::Entrance && anchor.name == "main"
                    })
                    .map(|anchor| anchor.forward)
            })
            .unwrap_or(DEFAULT_BUILDING_FRONTAGE_FORWARD)
    }

    fn legacy_driveway_frontage_forward(&self) -> Option<[f32; 3]> {
        self.anchors
            .iter()
            .find(|anchor| anchor.anchor_type == AnchorType::Driveway)
            .and_then(|anchor| {
                let [x, _, z] = anchor.forward;
                let horizontal_len = (x * x + z * z).sqrt();
                (horizontal_len > ANCHOR_FORWARD_UNIT_EPS)
                    .then(|| [-x / horizontal_len, 0.0, -z / horizontal_len])
            })
    }
}
