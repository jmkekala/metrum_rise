//! Per-asset manifest (`asset.toml`).
//!
//! Every imported asset ships with one `asset.toml` describing its class, dimensions,
//! LODs, anchors, and class-specific gameplay metadata. Mesh and texture files are
//! referenced by relative path; binary data never lives in the manifest.
//!
//! The active asset class is identified by which of the optional class tables is present
//! (`[building]`, `[prop]`, `[vehicle]`, `[character]`). Exactly one must be populated.

use super::{ManifestError, is_valid_asset_id};
use serde::Deserialize;

// ── Shared sub-types ──────────────────────���────────────────────��──────────────

/// One LOD tier entry inside an asset manifest.
///
/// LOD tiers must be listed in ascending distance order (nearest first = LOD0).
#[derive(Debug, Clone, Deserialize)]
pub struct LodEntry {
    /// Path to the `.glb` file for this tier, relative to the asset folder.
    pub file: String,
    /// Minimum camera distance (inclusive) at which this tier is active, in metres.
    pub distance_min_m: f32,
    /// Maximum camera distance (exclusive) at which this tier is active, in metres.
    /// `None` means this tier remains active until the asset is culled.
    pub distance_max_m: Option<f32>,
}

/// Attachment point or interaction hotspot on an asset.
///
/// All asset classes use the same `[[anchors]]` array-of-tables shape.
#[derive(Debug, Clone, Deserialize)]
pub struct Anchor {
    /// Role of this anchor in the simulation and editor.
    #[serde(rename = "type")]
    pub anchor_type: AnchorType,
    /// Short identifier for this anchor within the asset (e.g. `"main"`, `"rear"`).
    pub name: String,
    /// Local-space position `[x, y, z]` relative to the asset's placement origin.
    pub position: [f32; 3],
    /// Local-space forward direction unit vector `[x, y, z]`.
    pub forward: [f32; 3],
}

/// Recognised anchor roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorType {
    /// Main pedestrian entrance / road-facing access point on a building.
    Entrance,
    /// Rear access, loading bay, or service door.
    Service,
    /// Socket for attaching a prop (sign, bench, light, planter) to this asset.
    PropSocket,
    /// Wheel position marker on a vehicle.
    Wheel,
    /// Light emitter position marker on a vehicle.
    Light,
}

// ── Zone / land-use ───────────────────────────────────────────────────────────

/// Land-use category for a zoned building asset.
///
/// Maps onto [`crate::simulation::grid::zoning::ZoneType`] during registry step.
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
pub struct BuildingData {
    /// How this building enters the world.
    #[serde(default = "default_placement_mode")]
    pub placement_mode: PlacementMode,
    /// Land-use category this building satisfies when placed through painted zoning.
    pub zone_type: Option<ZoneClass>,
    /// Density tier for painted-zoning legality.
    pub density: Option<String>,
    /// Footprint width in zoning grid cells (along the road).
    pub lot_width_cells: u16,
    /// Footprint depth in zoning grid cells (away from the road).
    pub lot_depth_cells: u16,
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
    /// Editor preview scale override. `1.0` if absent (no override).
    pub preview_scale: Option<f32>,
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

// ── Prop ────────────────────────────────────────────────────���─────────────────

/// Placement grid behaviour for a prop asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SnapMode {
    /// No snapping — placed at exact cursor position.
    Free,
    /// Snaps to the environment grid.
    Grid,
    /// Snaps to road edge or pavement edge.
    Edge,
    /// Snaps to an arbitrary mesh surface.
    Surface,
}

/// Terrain interaction behaviour for a prop asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerrainBehavior {
    /// Asset sits flat on a levelled ground patch.
    FlatGround,
    /// Asset conforms its base to the terrain slope.
    ConformToSurface,
    /// Asset hangs downward from a surface (e.g. stalactites, overhead signs).
    HangFromSurface,
}

/// Class-specific data for a prop or environment detail asset.
#[derive(Debug, Clone, Deserialize)]
pub struct PropData {
    /// Logical category used by the content browser (e.g. `"street_furniture"`, `"foliage"`).
    pub category: String,
    /// Axis-aligned bounding box `[width_m, height_m, depth_m]` used for placement guides.
    pub bounding_size_m: [f32; 3],
    /// How the prop snaps to the world when placed.
    pub snap_mode: SnapMode,
    /// How the prop interacts with terrain geometry.
    pub terrain_behavior: TerrainBehavior,
}

// ── Vehicle ──────────────────────────────────���────────────────────────────────

/// Broad gameplay category for a vehicle asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleClass {
    /// Ordinary civilian traffic.
    Civil,
    /// Police / law-enforcement.
    Police,
    /// Fire / emergency.
    Fire,
    /// Ambulance / medical.
    Ambulance,
    /// Maintenance, utility, or delivery.
    Utility,
    /// Public transit bus.
    Bus,
}

/// Physical form factor of a vehicle asset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VehicleFamily {
    /// Passenger car (two-box or three-box body).
    Sedan,
    /// Sport-utility vehicle or crossover.
    Suv,
    /// Van or minivan.
    Van,
    /// Truck, lorry, or heavy goods vehicle.
    Truck,
}

/// One colour or livery variant for a vehicle asset.
#[derive(Debug, Clone, Deserialize)]
pub struct ColorVariant {
    /// Short display name for this variant (e.g. `"red"`, `"police_livery"`).
    pub name: String,
    /// Path to the albedo texture for this variant, relative to the asset folder.
    pub albedo_file: String,
}

/// Class-specific data for a vehicle asset.
#[derive(Debug, Clone, Deserialize)]
pub struct VehicleData {
    /// Gameplay category (determines spawning rules and icon).
    pub vehicle_class: VehicleClass,
    /// Physical form factor (determines collision and lane-fit checks).
    pub vehicle_family: VehicleFamily,
    /// Vehicle length in metres (bumper to bumper).
    pub length_m: f32,
    /// Vehicle width in metres (mirror to mirror).
    pub width_m: f32,
    /// Vehicle height in metres (ground to roof).
    pub height_m: f32,
    /// Available colour or livery variants. At least one must be provided.
    #[serde(default)]
    pub color_variants: Vec<ColorVariant>,
}

// ── Character ─────────────────────────────────────────────────────────────────

/// Character archetype family, determining which VAT data pool is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchetypeFamily {
    /// Adult male proportions.
    AdultMale,
    /// Adult female proportions.
    AdultFemale,
    /// Child proportions (separate rest mesh and animation bakes).
    Child,
}

/// One skin or clothing texture variant for a character.
#[derive(Debug, Clone, Deserialize)]
pub struct SkinVariant {
    /// Short display name (e.g. `"default"`, `"summer"`).
    pub name: String,
    /// Path to the albedo texture for this variant, relative to the asset folder.
    pub albedo_file: String,
}

/// Class-specific data for a character source asset.
///
/// Runtime packs ship baked VAT outputs only. Source clip references
/// are editor-only and are not included in exported runtime packs.
#[derive(Debug, Clone, Deserialize)]
pub struct CharacterData {
    /// Archetype family this character belongs to.
    pub archetype_family: ArchetypeFamily,
    /// Informal age group label (e.g. `"adult"`, `"elderly"`). Optional.
    pub age_group: Option<String>,
    /// Informal body type label (e.g. `"average"`, `"athletic"`). Optional.
    pub body_type: Option<String>,
    /// Available skin or clothing variants. At least one is expected.
    #[serde(default)]
    pub skin_variants: Vec<SkinVariant>,
}

// ── Top-level AssetManifest ───────────────────────────────────────────────────

/// The resolved asset class after validating that exactly one class table is present.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssetClass {
    /// Building.
    Building,
    /// Prop or environment detail.
    Prop,
    /// Traffic vehicle.
    Vehicle,
    /// Pedestrian character source asset.
    Character,
}

/// Per-asset manifest deserialized from `asset.toml`.
///
/// Shared fields appear at the top level. Class-specific data lives in one of the
/// optional subsections (`building`, `prop`, `vehicle`, `character`). Exactly one
/// subsection must be populated; [`AssetManifest::validate`] enforces this.
#[derive(Debug, Clone, Deserialize)]
pub struct AssetManifest {
    /// Stable dot-separated identifier within the pack (e.g. `"building.residential.lowrise_corner"`).
    /// Combined with the pack's `pack_id` to form the globally unique `pack_id:asset_id`.
    pub asset_id: String,
    /// Human-readable name shown in the asset browser and content manager.
    pub display_name: String,
    /// Optional grouping label for related assets within a pack (e.g. `"lowrise_residential"`).
    pub asset_set: Option<String>,
    /// Search tags for the content browser.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Path to the thumbnail image relative to the asset folder. Generated by the editor.
    pub thumbnail: Option<String>,
    /// LOD tier list. Must include at least one entry (LOD0) for building and vehicle assets.
    /// Listed nearest-first (LOD0 is the highest-detail tier).
    #[serde(default)]
    pub lods: Vec<LodEntry>,
    /// Anchor points for entrance, service, prop sockets, wheels, and lights.
    #[serde(default)]
    pub anchors: Vec<Anchor>,

    /// XZ-centre + Y-ground offset in model units.
    /// Applied when positioning the mesh in a MultiMesh transform so the model is
    /// centred over its lot cell and grounded at Y=0, regardless of where the original
    /// model pivot was placed. Computed once by the asset editor on import/autofit.
    #[serde(default)]
    pub pivot_offset: Option<[f32; 3]>,

    // ── Class-specific subsections (exactly one must be Some after parsing) ──
    /// Populated when this is a building asset.
    pub building: Option<BuildingData>,
    /// Populated when this is a prop or environment detail asset.
    pub prop: Option<PropData>,
    /// Populated when this is a vehicle asset.
    pub vehicle: Option<VehicleData>,
    /// Populated when this is a character source asset.
    pub character: Option<CharacterData>,
}

impl AssetManifest {
    /// Parses an `asset.toml` TOML string into an [`AssetManifest`].
    pub fn from_str(s: &str) -> Result<Self, ManifestError> {
        let manifest: Self = toml::from_str(s)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Returns the resolved [`AssetClass`], or `Err` if more than one class section
    /// is populated (use [`AssetManifest::validate`] to check the full manifest first).
    pub fn class(&self) -> Result<AssetClass, ManifestError> {
        match (
            self.building.is_some(),
            self.prop.is_some(),
            self.vehicle.is_some(),
            self.character.is_some(),
        ) {
            (true, false, false, false) => Ok(AssetClass::Building),
            (false, true, false, false) => Ok(AssetClass::Prop),
            (false, false, true, false) => Ok(AssetClass::Vehicle),
            (false, false, false, true) => Ok(AssetClass::Character),
            _ => Err(ManifestError::Validation(format!(
                "asset_id '{}': exactly one class section required \
                 ([building], [prop], [vehicle], or [character])",
                self.asset_id
            ))),
        }
    }

    /// Returns the fully-qualified asset ID `"pack_id:asset_id"`.
    pub fn qualified_id(&self, pack_id: &str) -> String {
        format!("{}:{}", pack_id, self.asset_id)
    }

    /// Validates structural and semantic constraints.
    ///
    /// Checks that:
    /// - `asset_id` is valid dot-separated kebab segments.
    /// - Exactly one class section is populated.
    /// - `display_name` is non-empty.
    /// - Building lot dimensions are non-zero when present.
    /// - Vehicle dimensions are positive when present.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if !is_valid_asset_id(&self.asset_id) {
            return Err(ManifestError::Validation(format!(
                "invalid asset_id '{}': must be dot-separated segments of lowercase \
                 letters, digits, and underscores",
                self.asset_id
            )));
        }
        if self.display_name.is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': display_name must not be empty",
                self.asset_id
            )));
        }

        // Enforce exactly one class section.
        let _ = self.class()?;

        if let Some(b) = &self.building {
            if b.lot_width_cells == 0 || b.lot_depth_cells == 0 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': lot_width_cells and lot_depth_cells must be > 0",
                    self.asset_id
                )));
            }
            if b.level == 0 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': level must be >= 1",
                    self.asset_id
                )));
            }
            if b.effective_min_zone_width_cells() == 0 || b.effective_min_zone_depth_cells() == 0 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': min_zone_width_cells and min_zone_depth_cells must be > 0",
                    self.asset_id
                )));
            }
            match b.placement_mode {
                PlacementMode::ZonedPrivate => {
                    let Some(zone_type) = b.zone_type else {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': zoned_private buildings require zone_type",
                            self.asset_id
                        )));
                    };
                    let Some(density) = b.density_key() else {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': zoned_private buildings require density",
                            self.asset_id
                        )));
                    };
                    if density.trim().is_empty() {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': density must not be empty",
                            self.asset_id
                        )));
                    }
                    match zone_type {
                        ZoneClass::Residential => {
                            if b.household_capacity.is_none() {
                                return Err(ManifestError::Validation(format!(
                                    "asset_id '{}': residential zoned_private buildings require household_capacity",
                                    self.asset_id
                                )));
                            }
                            if b.worker_capacity.is_some() {
                                return Err(ManifestError::Validation(format!(
                                    "asset_id '{}': residential zoned_private buildings must not use worker_capacity",
                                    self.asset_id
                                )));
                            }
                        }
                        ZoneClass::Commercial | ZoneClass::Industrial => {
                            if b.worker_capacity.is_none() && b.economy_profile.is_none() {
                                return Err(ManifestError::Validation(format!(
                                    "asset_id '{}': commercial and industrial zoned_private buildings require worker_capacity or economy_profile",
                                    self.asset_id
                                )));
                            }
                        }
                        ZoneClass::Office | ZoneClass::Mixed => {
                            return Err(ManifestError::Validation(format!(
                                "asset_id '{}': office and mixed are reserved future extensions outside the baseline shipped building contract",
                                self.asset_id
                            )));
                        }
                    }
                }
                PlacementMode::Explicit => {
                    if b.zone_type.is_some() || b.density.is_some() {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': explicit buildings must not declare zone_type or density",
                            self.asset_id
                        )));
                    }
                }
            }

            let mut main_entrance_count = 0usize;
            for anchor in &self.anchors {
                match anchor.anchor_type {
                    AnchorType::Entrance if anchor.name == "main" => {
                        main_entrance_count += 1;
                    }
                    AnchorType::Entrance => {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': additional entrance anchor '{}' is not allowed on building assets; use type = \"service\" for secondary access points",
                            self.asset_id, anchor.name
                        )));
                    }
                    AnchorType::Service | AnchorType::PropSocket => {}
                    AnchorType::Wheel => {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': anchor '{}' uses type = \"wheel\", which is not valid for building assets",
                            self.asset_id, anchor.name
                        )));
                    }
                    AnchorType::Light => {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': anchor '{}' uses type = \"light\", which is not valid for building assets",
                            self.asset_id, anchor.name
                        )));
                    }
                }
            }
            if main_entrance_count != 1 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': building assets require exactly one [[anchors]] entry with type = \"entrance\" and name = \"main\"",
                    self.asset_id
                )));
            }
        }

        if let Some(v) = &self.vehicle {
            if v.length_m <= 0.0 || v.width_m <= 0.0 || v.height_m <= 0.0 {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': vehicle dimensions must be positive",
                    self.asset_id
                )));
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Building ────────────────────────────────────────────────────────────

    const BUILDING_TOML: &str = r#"
asset_id = "building.residential.lowrise_corner"
display_name = "Low-rise Corner Building"
tags = ["residential", "corner"]
asset_set = "lowrise_residential"

[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 3
lot_depth_cells = 3
household_capacity = 12
service_class = "standard"
economy_profile = "residential_basic"

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 4.5]
forward = [0.0, 0.0, 1.0]

[[lods]]
file = "lod0.glb"
distance_min_m = 0.0
distance_max_m = 150.0

[[lods]]
file = "lod1.glb"
distance_min_m = 150.0
distance_max_m = 600.0
"#;

    #[test]
    fn building_manifest_round_trip() {
        let m = AssetManifest::from_str(BUILDING_TOML).expect("parse failed");
        assert_eq!(m.asset_id, "building.residential.lowrise_corner");
        assert_eq!(m.display_name, "Low-rise Corner Building");
        assert_eq!(m.class().unwrap(), AssetClass::Building);
        let b = m.building.as_ref().unwrap();
        assert_eq!(b.zone_type, Some(ZoneClass::Residential));
        assert_eq!(b.placement_mode, PlacementMode::ZonedPrivate);
        assert_eq!(b.lot_width_cells, 3);
        assert_eq!(b.lot_depth_cells, 3);
        assert_eq!(b.household_capacity, Some(12));
        assert_eq!(b.economy_profile.as_deref(), Some("residential_basic"));
        assert_eq!(m.lods.len(), 2);
        assert_eq!(m.anchors.len(), 1);
        assert_eq!(m.anchors[0].anchor_type, AnchorType::Entrance);
        assert_eq!(m.anchors[0].position, [0.0, 0.0, 4.5]);
    }

    #[test]
    fn building_qualified_id() {
        let m = AssetManifest::from_str(BUILDING_TOML).unwrap();
        assert_eq!(
            m.qualified_id("kenney-city-pack"),
            "kenney-city-pack:building.residential.lowrise_corner"
        );
    }

    #[test]
    fn building_rejects_zero_lot_cells() {
        let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 0
lot_depth_cells = 3
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    #[test]
    fn building_rejects_missing_main_entrance() {
        let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    #[test]
    fn building_rejects_secondary_entrance_anchor() {
        let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 2.0]
forward = [0.0, 0.0, 1.0]

[[anchors]]
type = "entrance"
name = "rear"
position = [0.0, 0.0, -2.0]
forward = [0.0, 0.0, -1.0]
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    #[test]
    fn building_rejects_duplicate_main_entrance_anchor() {
        let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 2.0]
forward = [0.0, 0.0, 1.0]

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, -2.0]
forward = [0.0, 0.0, -1.0]
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    // ── Prop ────────────────────────────────────────────────────────────────

    const PROP_TOML: &str = r#"
asset_id = "prop.street.bench_01"
display_name = "Street Bench"
tags = ["street_furniture"]

[prop]
category = "street_furniture"
bounding_size_m = [1.5, 0.9, 0.6]
snap_mode = "edge"
terrain_behavior = "flat_ground"

[[lods]]
file = "bench.glb"
distance_min_m = 0.0
"#;

    #[test]
    fn prop_manifest_round_trip() {
        let m = AssetManifest::from_str(PROP_TOML).expect("parse failed");
        assert_eq!(m.class().unwrap(), AssetClass::Prop);
        let p = m.prop.as_ref().unwrap();
        assert_eq!(p.category, "street_furniture");
        assert_eq!(p.snap_mode, SnapMode::Edge);
        assert_eq!(p.terrain_behavior, TerrainBehavior::FlatGround);
        assert_eq!(p.bounding_size_m, [1.5, 0.9, 0.6]);
        assert_eq!(m.lods[0].distance_max_m, None);
    }

    // ── Vehicle ─────────────────────────────────────────────────────────────

    const VEHICLE_TOML: &str = r#"
asset_id = "vehicle.civil.sedan_compact"
display_name = "Compact Sedan"
tags = ["civil", "sedan"]

[vehicle]
vehicle_class = "civil"
vehicle_family = "sedan"
length_m = 4.5
width_m = 1.8
height_m = 1.5

[[vehicle.color_variants]]
name = "red"
albedo_file = "textures/sedan_red.png"

[[vehicle.color_variants]]
name = "blue"
albedo_file = "textures/sedan_blue.png"

[[anchors]]
type = "wheel"
name = "front_left"
position = [-0.85, 0.0, 1.5]
forward = [0.0, 0.0, 1.0]

[[lods]]
file = "lod0.glb"
distance_min_m = 0.0
distance_max_m = 40.0
"#;

    #[test]
    fn vehicle_manifest_round_trip() {
        let m = AssetManifest::from_str(VEHICLE_TOML).expect("parse failed");
        assert_eq!(m.class().unwrap(), AssetClass::Vehicle);
        let v = m.vehicle.as_ref().unwrap();
        assert_eq!(v.vehicle_class, VehicleClass::Civil);
        assert_eq!(v.vehicle_family, VehicleFamily::Sedan);
        assert_eq!(v.length_m, 4.5);
        assert_eq!(v.color_variants.len(), 2);
        assert_eq!(v.color_variants[0].name, "red");
        assert_eq!(m.anchors[0].anchor_type, AnchorType::Wheel);
    }

    #[test]
    fn vehicle_rejects_zero_dimensions() {
        let toml = r#"
asset_id = "vehicle.civil.bad"
display_name = "Bad"
[vehicle]
vehicle_class = "civil"
vehicle_family = "sedan"
length_m = 0.0
width_m = 1.8
height_m = 1.5
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    // ── Character ────────────────────────────────────────────────────────────

    const CHARACTER_TOML: &str = r#"
asset_id = "character.pedestrian.adult_male_01"
display_name = "Adult Male Pedestrian"
tags = ["pedestrian", "adult"]

[character]
archetype_family = "adult_male"
age_group = "adult"
body_type = "average"

[[character.skin_variants]]
name = "default"
albedo_file = "textures/skin_default.png"

[[character.skin_variants]]
name = "summer"
albedo_file = "textures/skin_summer.png"
"#;

    #[test]
    fn character_manifest_round_trip() {
        let m = AssetManifest::from_str(CHARACTER_TOML).expect("parse failed");
        assert_eq!(m.class().unwrap(), AssetClass::Character);
        let c = m.character.as_ref().unwrap();
        assert_eq!(c.archetype_family, ArchetypeFamily::AdultMale);
        assert_eq!(c.age_group.as_deref(), Some("adult"));
        assert_eq!(c.skin_variants.len(), 2);
        assert_eq!(c.skin_variants[1].name, "summer");
    }

    // ── Validation ────────────────────────────────────────────────────────────

    #[test]
    fn rejects_no_class_section() {
        let toml = r#"
asset_id = "building.residential.no_class"
display_name = "No Class"
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    #[test]
    fn rejects_two_class_sections() {
        let toml = r#"
asset_id = "building.residential.two_classes"
display_name = "Two Classes"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 3
lot_depth_cells = 3
[prop]
category = "street_furniture"
bounding_size_m = [1.0, 1.0, 1.0]
snap_mode = "free"
terrain_behavior = "flat_ground"
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    #[test]
    fn explicit_building_manifest_round_trip() {
        let toml = r#"
asset_id = "building.service.water_tower"
display_name = "Water Tower"

[building]
placement_mode = "explicit"
lot_width_cells = 4
lot_depth_cells = 4
service_class = "water"
worker_capacity = 6

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 2.0]
forward = [0.0, 0.0, 1.0]
"#;
        let manifest = AssetManifest::from_str(toml).expect("explicit parse failed");
        let building = manifest.building.as_ref().unwrap();
        assert_eq!(building.placement_mode, PlacementMode::Explicit);
        assert_eq!(building.zone_type, None);
        assert_eq!(building.density, None);
        assert_eq!(building.service_class.as_deref(), Some("water"));
    }

    #[test]
    fn explicit_building_rejects_zone_fields() {
        let toml = r#"
asset_id = "building.service.bad_explicit"
display_name = "Bad Explicit"
[building]
placement_mode = "explicit"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 1.0]
forward = [0.0, 0.0, 1.0]
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    #[test]
    fn rejects_invalid_asset_id() {
        let toml = r#"
asset_id = "Bad.Asset.ID"
display_name = "Bad"
[prop]
category = "test"
bounding_size_m = [1.0, 1.0, 1.0]
snap_mode = "free"
terrain_behavior = "flat_ground"
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }
}
