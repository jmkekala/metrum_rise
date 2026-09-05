// SPDX-License-Identifier: GPL-2.0-only

//! Shared asset manifest model and prop schema.

use super::{BuildingData, CharacterData, VehicleData};
use crate::assets::ManifestError;
use serde::Deserialize;

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
    /// Vehicle connector pinned to the road-facing lot edge and pointing into the lot.
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
    /// Parses and validates one `asset.toml` manifest.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(source: &str) -> Result<Self, ManifestError> {
        source.parse()
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
}

impl std::str::FromStr for AssetManifest {
    type Err = ManifestError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let manifest: Self = toml::from_str(s)?;
        manifest.validate()?;
        Ok(manifest)
    }
}
