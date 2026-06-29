//! Per-asset manifest (`asset.toml`).
//!
//! Every imported asset ships with one `asset.toml` describing its class, dimensions,
//! meshes, anchors, and class-specific gameplay metadata. Mesh and texture files are
//! referenced by relative path; binary data never lives in the manifest.
//!
//! The active asset class is identified by which of the optional class tables is present
//! (`[building]`, `[prop]`, `[vehicle]`, `[character]`). Exactly one must be populated.

use super::{ManifestError, is_valid_asset_id};
use serde::Deserialize;

const ZONE_CELL_M: f32 = 10.0;
const ANCHOR_FORWARD_UNIT_EPS: f32 = 0.02;
const ANCHOR_LOT_EPS_M: f32 = 0.001;

// ── Shared sub-types ──────────────────────���────────────────────��──────────────

/// One LOD tier entry inside an asset manifest.
///
/// LOD tiers must be listed in ascending distance order (nearest first = LOD0).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LodEntry {
    /// Path to the `.glb` file for this tier, relative to the asset folder.
    pub file: String,
    /// Minimum camera distance (inclusive) at which this tier is active, in metres.
    pub distance_min_m: f32,
    /// Maximum camera distance (exclusive) at which this tier is active, in metres.
    /// `None` means this tier remains active until the asset is culled.
    pub distance_max_m: Option<f32>,
}

/// One renderable building mesh part.
///
/// A building may be authored from multiple mesh files. Each part has its own local
/// transform relative to the building placement origin and owns the LOD list for that
/// part. Runtime rendering still batches by asset part through MultiMesh instances.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshPart {
    /// Short editor label for this mesh part.
    pub name: String,
    /// Local-space position `[x, y, z]` relative to the building placement origin.
    #[serde(default = "default_vec3_zero")]
    pub position: [f32; 3],
    /// Local-space Euler rotation in degrees. Building rendering currently supports Y only.
    #[serde(default = "default_vec3_zero")]
    pub rotation_degrees: [f32; 3],
    /// Uniform scale for this part.
    #[serde(default = "default_mesh_part_scale")]
    pub scale: f32,
    /// Optional mesh pivot correction applied after this part's local scale/rotation.
    #[serde(default)]
    pub pivot_offset: Option<[f32; 3]>,
    /// LOD tiers for this specific mesh part, nearest-first.
    #[serde(default)]
    pub lods: Vec<LodEntry>,
}

impl MeshPart {
    /// Creates a default-position building mesh part with one LOD0 mesh file.
    pub fn single_lod0(name: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            position: [0.0, 0.0, 0.0],
            rotation_degrees: [0.0, 0.0, 0.0],
            scale: 1.0,
            pivot_offset: None,
            lods: vec![LodEntry {
                file: file.into(),
                distance_min_m: 0.0,
                distance_max_m: None,
            }],
        }
    }
}

fn default_vec3_zero() -> [f32; 3] {
    [0.0, 0.0, 0.0]
}

fn default_mesh_part_scale() -> f32 {
    1.0
}

/// Attachment point or interaction hotspot on an asset.
///
/// All asset classes use the same `[[anchors]]` array-of-tables shape.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    /// Role of this anchor in the simulation and editor.
    #[serde(rename = "type")]
    pub anchor_type: AnchorType,
    /// Optional short identifier for this anchor within the asset (e.g. `"main"`, `"rear"`).
    #[serde(default)]
    pub name: String,
    /// Local-space position `[x, y, z]` relative to the asset's placement origin.
    pub position: [f32; 3],
    /// Local-space forward direction unit vector `[x, y, z]`.
    pub forward: [f32; 3],
    /// Optional usable/access width in metres for entrances, driveways, parking, or loading bays.
    #[serde(default)]
    pub width_m: Option<f32>,
    /// Optional usable/access length in metres for parking or loading bays.
    #[serde(default)]
    pub length_m: Option<f32>,
    /// Optional vehicle class accepted by this anchor, such as `"car"` or `"freight"`.
    #[serde(default)]
    pub vehicle_class: Option<String>,
}

/// Recognised anchor roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnchorType {
    /// Main pedestrian entrance / road-facing access point on a building.
    Entrance,
    /// Vehicle connector from the lot interior toward the road-facing edge.
    Driveway,
    /// Vehicle parking position inside a building lot.
    Parking,
    /// Freight or service vehicle stop position inside a building lot.
    LoadingBay,
    /// Wheel position marker on a vehicle.
    Wheel,
    /// Light emitter position marker on a vehicle.
    Light,
}

/// Material for an authored building yard surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteSurfaceMaterial {
    /// Dark paved surface, commonly used for authored vehicle aprons and parking pads.
    Asphalt,
    /// Light hard surface, commonly used for authored walkways and service pads.
    Concrete,
}

/// One authored polygon ground-treatment surface inside a building lot.
///
/// Runtime treats this as a material/layout region on the flat building support footprint.
/// Gameplay must not render it as a loose terrain overlay.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteSurface {
    /// Visual material used by the asset-editor preview and live site client.
    pub material: SiteSurfaceMaterial,
    /// Optional editor label for this surface.
    #[serde(default)]
    pub name: String,
    /// Local-space vertical offset in metres relative to the building placement origin.
    #[serde(default)]
    pub y_m: f32,
    /// Local-space `[x, z]` polygon vertices, in winding order, relative to the asset origin.
    pub vertices: Vec<[f32; 2]>,
}

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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
pub struct ColorVariant {
    /// Short display name for this variant (e.g. `"red"`, `"police_livery"`).
    pub name: String,
    /// Path to the albedo texture for this variant, relative to the asset folder.
    pub albedo_file: String,
}

/// Class-specific data for a vehicle asset.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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
    /// LOD tier list for non-building assets.
    ///
    /// Building assets use [`MeshPart::lods`] instead so multi-structure assets can
    /// share one building contract without top-level mesh metadata.
    /// Listed nearest-first (LOD0 is the highest-detail tier).
    #[serde(default)]
    pub lods: Vec<LodEntry>,
    /// Building-only mesh parts. Empty for props, vehicles, and characters.
    #[serde(default)]
    pub mesh_parts: Vec<MeshPart>,
    /// Anchor points for entrance, service, prop sockets, wheels, and lights.
    #[serde(default)]
    pub anchors: Vec<Anchor>,
    /// Building-only authored visual yard surfaces.
    #[serde(default)]
    pub site_surfaces: Vec<SiteSurface>,

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

        for anchor in &self.anchors {
            validate_anchor_common(&self.asset_id, anchor)?;
        }

        if let Some(b) = &self.building {
            if !self.lods.is_empty() {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{}': building assets use [[mesh_parts]] with [[mesh_parts.lods]]; top-level [[lods]] is not valid",
                    self.asset_id
                )));
            }
            validate_building_mesh_parts(&self.asset_id, &self.mesh_parts)?;
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
                validate_building_anchor_position(&self.asset_id, b, anchor)?;
                match anchor.anchor_type {
                    AnchorType::Entrance if anchor.name == "main" => {
                        main_entrance_count += 1;
                    }
                    AnchorType::Entrance => {
                        return Err(ManifestError::Validation(format!(
                            "asset_id '{}': additional entrance anchor '{}' is not allowed on building assets; use type = \"driveway\", \"parking\", or \"loading_bay\" for site access points",
                            self.asset_id, anchor.name
                        )));
                    }
                    AnchorType::Driveway => {
                        validate_positive_anchor_field(
                            &self.asset_id,
                            anchor,
                            "width_m",
                            anchor.width_m,
                        )?;
                        validate_building_site_anchor_footprint(&self.asset_id, b, anchor)?;
                    }
                    AnchorType::Parking | AnchorType::LoadingBay => {
                        validate_positive_anchor_field(
                            &self.asset_id,
                            anchor,
                            "width_m",
                            anchor.width_m,
                        )?;
                        validate_positive_anchor_field(
                            &self.asset_id,
                            anchor,
                            "length_m",
                            anchor.length_m,
                        )?;
                        validate_building_site_anchor_footprint(&self.asset_id, b, anchor)?;
                    }
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
            for surface in &self.site_surfaces {
                validate_building_site_surface(&self.asset_id, b, surface)?;
            }
        } else if !self.mesh_parts.is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': [[mesh_parts]] is only valid for building assets",
                self.asset_id
            )));
        } else if !self.site_surfaces.is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': [[site_surfaces]] is only valid for building assets",
                self.asset_id
            )));
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

fn validate_positive_anchor_field(
    asset_id: &str,
    anchor: &Anchor,
    field_name: &str,
    value: Option<f32>,
) -> Result<(), ManifestError> {
    let Some(value) = value else {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' with type = \"{:?}\" requires positive {}",
            asset_id, anchor.name, anchor.anchor_type, field_name
        )));
    };
    if !value.is_finite() || value <= 0.0 {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' has invalid {} {}; expected a positive finite value",
            asset_id, anchor.name, field_name, value
        )));
    }
    Ok(())
}

fn validate_anchor_common(asset_id: &str, anchor: &Anchor) -> Result<(), ManifestError> {
    validate_finite_vec3(asset_id, anchor, "position", anchor.position)?;
    validate_anchor_forward(asset_id, anchor)?;
    if let Some(vehicle_class) = anchor.vehicle_class.as_deref() {
        validate_anchor_vehicle_class(asset_id, anchor, vehicle_class)?;
    }
    Ok(())
}

fn validate_finite_vec3(
    asset_id: &str,
    anchor: &Anchor,
    field_name: &str,
    value: [f32; 3],
) -> Result<(), ManifestError> {
    if value.iter().any(|component| !component.is_finite()) {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' has invalid {} {:?}; expected finite values",
            asset_id, anchor.name, field_name, value
        )));
    }
    Ok(())
}

fn validate_anchor_forward(asset_id: &str, anchor: &Anchor) -> Result<(), ManifestError> {
    validate_finite_vec3(asset_id, anchor, "forward", anchor.forward)?;
    let [x, y, z] = anchor.forward;
    let length = (x * x + y * y + z * z).sqrt();
    if length <= ANCHOR_FORWARD_UNIT_EPS || (length - 1.0).abs() > ANCHOR_FORWARD_UNIT_EPS {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' forward {:?} must be a non-zero unit vector",
            asset_id, anchor.name, anchor.forward
        )));
    }
    Ok(())
}

fn validate_anchor_vehicle_class(
    asset_id: &str,
    anchor: &Anchor,
    vehicle_class: &str,
) -> Result<(), ManifestError> {
    match vehicle_class.trim() {
        "car" | "freight" | "service" => Ok(()),
        other => Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' has invalid vehicle_class '{}'; expected car, freight, or service",
            asset_id, anchor.name, other
        ))),
    }
}

fn validate_building_anchor_position(
    asset_id: &str,
    building: &BuildingData,
    anchor: &Anchor,
) -> Result<(), ManifestError> {
    let half_width = f32::from(building.lot_width_cells) * ZONE_CELL_M * 0.5;
    let half_depth = f32::from(building.lot_depth_cells) * ZONE_CELL_M * 0.5;
    let [x, _, z] = anchor.position;
    if x.abs() > half_width + ANCHOR_LOT_EPS_M || z.abs() > half_depth + ANCHOR_LOT_EPS_M {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' position [{}, {}, {}] is outside the building lot bounds +/-{}m x +/-{}m",
            asset_id,
            anchor.name,
            anchor.position[0],
            anchor.position[1],
            anchor.position[2],
            half_width,
            half_depth
        )));
    }
    Ok(())
}

fn validate_building_site_anchor_footprint(
    asset_id: &str,
    building: &BuildingData,
    anchor: &Anchor,
) -> Result<(), ManifestError> {
    let (anchor_width, anchor_length) = match anchor.anchor_type {
        AnchorType::Driveway => {
            let width = anchor.width_m.unwrap_or(0.0);
            (width, (width * 1.4).max(1.5))
        }
        AnchorType::Parking | AnchorType::LoadingBay => (
            anchor.width_m.unwrap_or(0.0),
            anchor.length_m.unwrap_or(0.0),
        ),
        _ => return Ok(()),
    };
    let [forward_x, _, forward_z] = anchor.forward;
    let horizontal_len = (forward_x * forward_x + forward_z * forward_z).sqrt();
    if horizontal_len <= ANCHOR_FORWARD_UNIT_EPS {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': anchor '{}' with type = \"{:?}\" must have a horizontal forward direction",
            asset_id, anchor.name, anchor.anchor_type
        )));
    }
    let fwd_x = forward_x / horizontal_len;
    let fwd_z = forward_z / horizontal_len;
    let side_x = -fwd_z;
    let side_z = fwd_x;
    let half_anchor_width = anchor_width * 0.5;
    let half_lot_width = f32::from(building.lot_width_cells) * ZONE_CELL_M * 0.5;
    let half_lot_depth = f32::from(building.lot_depth_cells) * ZONE_CELL_M * 0.5;
    let offsets = [
        (-side_x * half_anchor_width, -side_z * half_anchor_width),
        (side_x * half_anchor_width, side_z * half_anchor_width),
        (
            side_x * half_anchor_width + fwd_x * anchor_length,
            side_z * half_anchor_width + fwd_z * anchor_length,
        ),
        (
            -side_x * half_anchor_width + fwd_x * anchor_length,
            -side_z * half_anchor_width + fwd_z * anchor_length,
        ),
    ];
    for (offset_x, offset_z) in offsets {
        let x = anchor.position[0] + offset_x;
        let z = anchor.position[2] + offset_z;
        if x.abs() > half_lot_width + ANCHOR_LOT_EPS_M
            || z.abs() > half_lot_depth + ANCHOR_LOT_EPS_M
        {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': anchor '{}' site footprint for type = \"{:?}\" crosses the building lot bounds +/-{}m x +/-{}m",
                asset_id, anchor.name, anchor.anchor_type, half_lot_width, half_lot_depth
            )));
        }
    }
    Ok(())
}

fn validate_building_site_surface(
    asset_id: &str,
    building: &BuildingData,
    surface: &SiteSurface,
) -> Result<(), ManifestError> {
    if !surface.y_m.is_finite() {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': site surface '{}' has invalid y_m {}; expected a finite value",
            asset_id, surface.name, surface.y_m
        )));
    }
    if surface.vertices.len() < 3 {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': site surface '{}' must contain at least three vertices",
            asset_id, surface.name
        )));
    }
    let half_lot_width = f32::from(building.lot_width_cells) * ZONE_CELL_M * 0.5;
    let half_lot_depth = f32::from(building.lot_depth_cells) * ZONE_CELL_M * 0.5;

    for (vertex_index, [x, z]) in surface.vertices.iter().copied().enumerate() {
        if !x.is_finite() || !z.is_finite() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': site surface '{}' vertex {} has invalid coordinate [{}, {}]; expected finite values",
                asset_id, surface.name, vertex_index, x, z
            )));
        }
        if x.abs() > half_lot_width + ANCHOR_LOT_EPS_M
            || z.abs() > half_lot_depth + ANCHOR_LOT_EPS_M
        {
            return Err(ManifestError::Validation(format!(
                "asset_id '{}': site surface '{}' vertex {} crosses the building lot bounds +/-{}m x +/-{}m",
                asset_id, surface.name, vertex_index, half_lot_width, half_lot_depth
            )));
        }
    }

    if site_surface_polygon_signed_area(&surface.vertices).abs() <= 0.001 {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': site surface '{}' has zero or near-zero polygon area",
            asset_id, surface.name
        )));
    }

    if site_surface_polygon_self_intersects(&surface.vertices) {
        return Err(ManifestError::Validation(format!(
            "asset_id '{}': site surface '{}' polygon self-intersects",
            asset_id, surface.name
        )));
    }

    Ok(())
}

fn site_surface_polygon_signed_area(vertices: &[[f32; 2]]) -> f32 {
    let mut twice_area = 0.0;
    for i in 0..vertices.len() {
        let [ax, az] = vertices[i];
        let [bx, bz] = vertices[(i + 1) % vertices.len()];
        twice_area += ax * bz - bx * az;
    }
    twice_area * 0.5
}

fn site_surface_polygon_self_intersects(vertices: &[[f32; 2]]) -> bool {
    for a in 0..vertices.len() {
        let b = (a + 1) % vertices.len();
        for c in (a + 1)..vertices.len() {
            let d = (c + 1) % vertices.len();
            if a == c || a == d || b == c || b == d {
                continue;
            }
            if site_surface_segments_intersect(vertices[a], vertices[b], vertices[c], vertices[d]) {
                return true;
            }
        }
    }
    false
}

fn site_surface_segments_intersect(a: [f32; 2], b: [f32; 2], c: [f32; 2], d: [f32; 2]) -> bool {
    const EPS: f32 = 0.0001;
    let ab_c = site_surface_orientation(a, b, c);
    let ab_d = site_surface_orientation(a, b, d);
    let cd_a = site_surface_orientation(c, d, a);
    let cd_b = site_surface_orientation(c, d, b);

    if ab_c.abs() <= EPS && site_surface_point_on_segment(a, b, c) {
        return true;
    }
    if ab_d.abs() <= EPS && site_surface_point_on_segment(a, b, d) {
        return true;
    }
    if cd_a.abs() <= EPS && site_surface_point_on_segment(c, d, a) {
        return true;
    }
    if cd_b.abs() <= EPS && site_surface_point_on_segment(c, d, b) {
        return true;
    }

    (ab_c > EPS) != (ab_d > EPS) && (cd_a > EPS) != (cd_b > EPS)
}

fn site_surface_orientation(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn site_surface_point_on_segment(a: [f32; 2], b: [f32; 2], p: [f32; 2]) -> bool {
    const EPS: f32 = 0.0001;
    p[0] >= a[0].min(b[0]) - EPS
        && p[0] <= a[0].max(b[0]) + EPS
        && p[1] >= a[1].min(b[1]) - EPS
        && p[1] <= a[1].max(b[1]) + EPS
}

fn validate_building_mesh_parts(
    asset_id: &str,
    mesh_parts: &[MeshPart],
) -> Result<(), ManifestError> {
    if mesh_parts.is_empty() {
        return Err(ManifestError::Validation(format!(
            "asset_id '{asset_id}': building assets require at least one [[mesh_parts]] entry"
        )));
    }

    let mut names: Vec<&str> = Vec::with_capacity(mesh_parts.len());
    for (part_index, part) in mesh_parts.iter().enumerate() {
        let name = part.name.trim();
        if name.is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': mesh part {part_index} name must not be empty"
            )));
        }
        if names.contains(&name) {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': duplicate mesh part name '{name}'"
            )));
        }
        names.push(name);
        if part.scale <= 0.0 || !part.scale.is_finite() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': mesh part '{name}' scale must be finite and > 0"
            )));
        }
        if part.rotation_degrees[0].abs() > 1e-4 || part.rotation_degrees[2].abs() > 1e-4 {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': mesh part '{name}' only supports Y rotation in the building renderer"
            )));
        }
        validate_lods(asset_id, Some(name), &part.lods)?;
    }

    Ok(())
}

fn validate_lods(
    asset_id: &str,
    part_name: Option<&str>,
    lods: &[LodEntry],
) -> Result<(), ManifestError> {
    if lods.is_empty() {
        let owner = part_name
            .map(|name| format!("mesh part '{name}'"))
            .unwrap_or_else(|| "asset".to_owned());
        return Err(ManifestError::Validation(format!(
            "asset_id '{asset_id}': {owner} requires at least one LOD entry"
        )));
    }

    let mut previous_min = -f32::INFINITY;
    for (lod_index, lod) in lods.iter().enumerate() {
        if lod.file.trim().is_empty() {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': LOD {lod_index} file must not be empty"
            )));
        }
        if !lod.distance_min_m.is_finite() || lod.distance_min_m < 0.0 {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': LOD {lod_index} distance_min_m must be finite and >= 0"
            )));
        }
        if lod.distance_min_m < previous_min {
            return Err(ManifestError::Validation(format!(
                "asset_id '{asset_id}': LOD entries must be ordered by distance_min_m"
            )));
        }
        if let Some(max) = lod.distance_max_m {
            if !max.is_finite() || max <= lod.distance_min_m {
                return Err(ManifestError::Validation(format!(
                    "asset_id '{asset_id}': LOD {lod_index} distance_max_m must be finite and greater than distance_min_m"
                )));
            }
        }
        previous_min = lod.distance_min_m;
    }

    Ok(())
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

[[anchors]]
type = "parking"
position = [-2.5, 0.0, 1.0]
forward = [0.0, 0.0, 1.0]
width_m = 2.5
length_m = 5.0
vehicle_class = "car"

[[site_surfaces]]
material = "concrete"
name = "front_walk"
y_m = 0.01
vertices = [[-0.7, 1.0], [0.7, 1.0], [0.7, 7.0], [-0.7, 7.0]]

[[mesh_parts]]
name = "main"
position = [0.0, 0.0, 0.0]
rotation_degrees = [0.0, 0.0, 0.0]
scale = 1.0

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
distance_max_m = 150.0

[[mesh_parts.lods]]
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
        assert_eq!(m.lods.len(), 0);
        assert_eq!(m.mesh_parts.len(), 1);
        assert_eq!(m.mesh_parts[0].name, "main");
        assert_eq!(m.mesh_parts[0].lods.len(), 2);
        assert_eq!(m.anchors.len(), 2);
        assert_eq!(m.anchors[0].anchor_type, AnchorType::Entrance);
        assert_eq!(m.anchors[0].position, [0.0, 0.0, 4.5]);
        assert_eq!(m.anchors[1].anchor_type, AnchorType::Parking);
        assert_eq!(m.anchors[1].name, "");
        assert_eq!(m.anchors[1].width_m, Some(2.5));
        assert_eq!(m.anchors[1].length_m, Some(5.0));
        assert_eq!(m.anchors[1].vehicle_class.as_deref(), Some("car"));
        assert_eq!(m.site_surfaces.len(), 1);
        assert_eq!(m.site_surfaces[0].material, SiteSurfaceMaterial::Concrete);
        assert_eq!(m.site_surfaces[0].name, "front_walk");
        assert_eq!(m.site_surfaces[0].y_m, 0.01);
        assert_eq!(m.site_surfaces[0].vertices.len(), 4);
    }

    #[test]
    fn building_rejects_top_level_lods() {
        let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 2.0]
forward = [0.0, 0.0, 1.0]

[[lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
        assert!(AssetManifest::from_str(toml).is_err());
    }

    #[test]
    fn building_rejects_unknown_top_level_manifest_field() {
        let toml = BUILDING_TOML.replace(
            "display_name = \"Low-rise Corner Building\"",
            "display_name = \"Low-rise Corner Building\"\nlegacy_field = true",
        );
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "expected unknown-field parse error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_unknown_anchor_field() {
        let toml = BUILDING_TOML.replace(
            "vehicle_class = \"car\"",
            "vehicle_class = \"car\"\npurpose = \"resident\"",
        );
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "expected unknown-field parse error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_unknown_site_surface_field() {
        let toml = BUILDING_TOML.replace("y_m = 0.01", "y_m = 0.01\nwidth_m = 1.4");
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "expected unknown-field parse error, got: {err}"
        );
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

    #[test]
    fn building_rejects_anchor_outside_lot() {
        let toml =
            BUILDING_TOML.replace("position = [-2.5, 0.0, 1.0]", "position = [16.0, 0.0, 1.0]");
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("outside the building lot bounds"),
            "expected outside-lot validation error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_site_anchor_footprint_outside_lot() {
        let toml = BUILDING_TOML.replace(
            "position = [-2.5, 0.0, 1.0]",
            "position = [-2.5, 0.0, 12.0]",
        );
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("site footprint"),
            "expected footprint validation error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_site_surface_vertex_outside_lot() {
        let toml = BUILDING_TOML.replace(
            "vertices = [[-0.7, 1.0], [0.7, 1.0], [0.7, 7.0], [-0.7, 7.0]]",
            "vertices = [[-0.7, 1.0], [16.0, 1.0], [0.7, 7.0], [-0.7, 7.0]]",
        );
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("crosses the building lot bounds"),
            "expected site-surface bounds validation error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_site_surface_self_intersection() {
        let toml = BUILDING_TOML.replace(
            "vertices = [[-0.7, 1.0], [0.7, 1.0], [0.7, 7.0], [-0.7, 7.0]]",
            "vertices = [[-2.0, 0.0], [2.0, 0.0], [-2.0, 2.0], [2.0, 2.0], [2.0, 4.0], [-2.0, 4.0]]",
        );
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("polygon self-intersects"),
            "expected site-surface self-intersection validation error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_driveway_footprint_outside_lot() {
        let toml = r#"
asset_id = "building.residential.bad"
display_name = "Bad"
[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
household_capacity = 1

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 9.0]
forward = [0.0, 0.0, 1.0]

[[anchors]]
type = "driveway"
position = [0.0, 0.0, 9.0]
forward = [0.0, 0.0, 1.0]
width_m = 3.0

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;
        let err = AssetManifest::from_str(toml).unwrap_err();
        assert!(
            err.to_string().contains("site footprint"),
            "expected driveway footprint validation error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_zero_anchor_forward() {
        let toml = BUILDING_TOML.replace("forward = [0.0, 0.0, 1.0]", "forward = [0.0, 0.0, 0.0]");
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("must be a non-zero unit vector"),
            "expected forward-vector validation error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_non_unit_anchor_forward() {
        let toml = BUILDING_TOML.replace("forward = [0.0, 0.0, 1.0]", "forward = [0.0, 0.0, 2.0]");
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("must be a non-zero unit vector"),
            "expected forward-vector validation error, got: {err}"
        );
    }

    #[test]
    fn building_rejects_invalid_anchor_vehicle_class() {
        let toml =
            BUILDING_TOML.replace("vehicle_class = \"car\"", "vehicle_class = \"hovercraft\"");
        let err = AssetManifest::from_str(&toml).unwrap_err();
        assert!(
            err.to_string().contains("invalid vehicle_class"),
            "expected vehicle-class validation error, got: {err}"
        );
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

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "water_tower.glb"
distance_min_m = 0.0
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
