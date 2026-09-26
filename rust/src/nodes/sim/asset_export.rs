// SPDX-License-Identifier: GPL-2.0-only

//! Shared authoring preflight and manifest serialization for staged asset publication.
//!
//! JSON metadata is validated and round-tripped through [`AssetManifest`]. Filesystem
//! publication belongs exclusively to `assets::authoring::files`; this module only reads packs.

use crate::assets::asset::{AnchorType, PlacementMode, SiteSurfaceMaterial};
use crate::assets::pack::toml_string;
use crate::assets::{AssetManifest, PackManifest};
use crate::debug_log;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, load_runtime_economy_catalog,
};
use crate::simulation::zoning::load_builtin_profile_registry;
use serde::{Deserialize, Deserializer};
use std::path::Path;

// ── Input structs (JSON from GDScript) ───────────────────────────────────────

/// LOD entry sent from the building importer form.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LodParams {
    /// Path to the `.glb` file for this LOD level, relative to the pack directory.
    pub file: String,
    /// Minimum camera distance (m) at which this LOD is used.
    pub distance_min_m: f32,
    /// Maximum camera distance (m) at which this LOD is used; `None` means no upper limit.
    #[serde(default)]
    pub distance_max_m: Option<f32>,
}

/// Anchor point entry sent from the building importer form.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnchorParams {
    /// Semantic type of this anchor (e.g. `"entrance"`, `"driveway"`).
    pub anchor_type: String,
    /// Optional identifier for this anchor within the asset (e.g. `"main"`).
    #[serde(default)]
    pub name: String,
    /// World-space position of the anchor relative to the asset origin.
    pub position: [f32; 3],
    /// Forward direction vector of the anchor in asset-local space.
    pub forward: [f32; 3],
    /// Optional usable/access width in metres.
    #[serde(default)]
    pub width_m: Option<f32>,
    /// Optional usable/access length in metres.
    #[serde(default)]
    pub length_m: Option<f32>,
    /// Optional vehicle class accepted by this anchor.
    #[serde(default)]
    pub vehicle_class: Option<String>,
}

/// Visual yard surface entry sent from the building importer form.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteSurfaceParams {
    /// Surface material key such as `"asphalt"` or `"concrete"`.
    pub material: String,
    /// Optional editor label for this surface.
    #[serde(default)]
    pub name: String,
    /// Local vertical offset relative to the building placement origin.
    #[serde(default)]
    pub y_m: f32,
    /// Local `[x, z]` polygon vertices in winding order.
    pub vertices: Vec<[f32; 2]>,
}

/// One renderable mesh part sent from the building importer form.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshPartParams {
    /// Editor label for this mesh part.
    pub name: String,
    /// Local position relative to the building placement origin.
    #[serde(default)]
    pub position: [f32; 3],
    /// Local Euler rotation in degrees. Building runtime supports Y rotation.
    #[serde(default)]
    pub rotation_degrees: [f32; 3],
    /// Uniform part scale.
    #[serde(default = "default_part_scale")]
    pub scale: f32,
    /// Optional pivot correction for this part.
    #[serde(default)]
    pub pivot_offset: Option<[f32; 3]>,
    /// LOD entries ordered from highest to lowest detail.
    #[serde(default)]
    pub lods: Vec<LodParams>,
}

/// Flat JSON payload sent by the building importer form.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportParams {
    /// Saved asset-wide HDR window emission strength, independent of preview multipliers.
    #[serde(default = "crate::assets::asset::BuildingData::default_window_brightness")]
    pub window_brightness: f32,
    /// Coordinated building appearance metadata; omitted for source-only materials.
    #[serde(default)]
    pub appearance: Option<crate::assets::asset::BuildingAppearance>,
    /// Pack identifier string (e.g. `"kenney"`).
    pub pack_id: String,
    /// Human-readable pack name shown in the pack manager.
    pub pack_name: String,
    /// Author credit shown in the pack manager.
    pub pack_author: String,
    /// Semver version string for the pack (default `"0.1.0"`).
    #[serde(default = "default_version")]
    pub pack_version: String,
    /// SPDX licence identifier for the pack (default `"CC0"`).
    #[serde(default = "default_license")]
    pub pack_license: String,

    /// Asset class tag (`"building"` for Step 5; extended in Step 6).
    pub asset_class: String,
    /// Asset identifier string (e.g. `"building.residential.house_a"`).
    pub asset_id: String,
    /// Human-readable name shown in the asset browser.
    pub display_name: String,
    /// Existing or generated thumbnail relative to the asset directory.
    #[serde(default)]
    pub thumbnail: Option<String>,
    /// Optional grouping set within the pack (e.g. `"suburban"`).
    #[serde(default)]
    pub asset_set: Option<String>,
    /// Free-form search tags.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Zone type the building belongs to (e.g. `"residential"`).
    #[serde(default)]
    pub zone_type: Option<String>,
    /// Density hint (e.g. `"low"`, `"medium"`, `"high"`).
    #[serde(default)]
    pub density: Option<String>,
    /// How this building enters the world.
    #[serde(default = "default_placement_mode")]
    pub placement_mode: String,
    /// Footprint width in 10 m zone cells along the road frontage.
    #[serde(default, deserialize_with = "whole_number")]
    pub lot_width_cells: u16,
    /// Footprint depth in 10 m zone cells away from the road.
    #[serde(default, deserialize_with = "whole_number")]
    pub lot_depth_cells: u16,
    /// Asset-local direction of the road-facing frontage.
    #[serde(default = "default_frontage_forward")]
    pub frontage_forward: [f32; 3],
    /// Minimum accepted zoned width for this building.
    #[serde(default, deserialize_with = "optional_whole_number")]
    pub min_zone_width_cells: Option<u16>,
    /// Minimum accepted zoned depth for this building.
    #[serde(default, deserialize_with = "optional_whole_number")]
    pub min_zone_depth_cells: Option<u16>,
    /// Development level (1 = lowest density / newest; higher = denser / upgraded).
    #[serde(default = "default_level", deserialize_with = "whole_number")]
    pub level: u8,
    /// Maximum number of households this building can house.
    #[serde(default, deserialize_with = "optional_whole_number")]
    pub household_capacity: Option<u32>,
    /// Direct worker capacity used only when no economy profile is selected.
    #[serde(default, deserialize_with = "optional_whole_number")]
    pub worker_capacity: Option<u32>,
    /// Target floor area per household in square meters.
    #[serde(default)]
    pub flat_size_m2: Option<f32>,
    /// Service class tag for civic buildings (e.g. `"fire_station"`).
    #[serde(default)]
    pub service_class: Option<String>,
    /// Reference to an authored economy profile selected from the current economy catalog.
    #[serde(default)]
    pub economy_profile: Option<String>,
    /// Optional resource id extracted by this explicit industry building.
    #[serde(default)]
    pub extractor_resource: Option<String>,
    /// Optional extraction area mode. Version one supports `"player_polygon"`.
    #[serde(default)]
    pub extractor_area_mode: Option<String>,
    /// Optional resource id grown by this explicit agricultural building.
    #[serde(default)]
    pub field_resource: Option<String>,
    /// Optional field area mode. Version one supports `"player_polygon"`.
    #[serde(default)]
    pub field_area_mode: Option<String>,

    /// Building mesh parts. Each part owns its own LOD entries.
    #[serde(default)]
    pub mesh_parts: Vec<MeshPartParams>,
    /// Named anchor points (frontage, entrances, etc.).
    #[serde(default)]
    pub anchors: Vec<AnchorParams>,
    /// Authored visual yard surfaces.
    #[serde(default)]
    pub site_surfaces: Vec<SiteSurfaceParams>,
}

fn whole_number<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<u64> + TryFrom<i64>,
{
    crate::simulation::economy::definitions::deserialize_unsigned_from_number(
        deserializer,
        std::any::type_name::<T>(),
    )
}

fn optional_whole_number<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: TryFrom<u64> + TryFrom<i64>,
{
    Option::<serde_json::Value>::deserialize(deserializer)?
        .map(|value| whole_number(value).map_err(serde::de::Error::custom))
        .transpose()
}

fn default_version() -> String {
    "0.1.0".to_owned()
}
fn default_license() -> String {
    "CC0".to_owned()
}
fn default_level() -> u8 {
    1
}

fn default_frontage_forward() -> [f32; 3] {
    [0.0, 0.0, 1.0]
}

fn default_part_scale() -> f32 {
    1.0
}

fn default_placement_mode() -> String {
    "zoned_private".to_owned()
}

// ── TOML generation ───────────────────────────────────────────────────────────

fn build_pack_toml(p: &ExportParams) -> String {
    crate::assets::pack::manifest_toml(
        &p.pack_id,
        &p.pack_name,
        &p.pack_version,
        &p.pack_author,
        &p.pack_license,
    )
}

fn build_asset_toml(p: &ExportParams) -> Result<String, String> {
    let mut out = String::new();
    out.push_str(&format!("asset_id = {}\n", toml_string(&p.asset_id)));
    out.push_str(&format!(
        "display_name = {}\n",
        toml_string(&p.display_name)
    ));
    if let Some(thumbnail) = &p.thumbnail {
        out.push_str(&format!("thumbnail = {}\n", toml_string(thumbnail)));
    }

    if let Some(set) = &p.asset_set {
        if !set.is_empty() {
            out.push_str(&format!("asset_set = {}\n", toml_string(set)));
        }
    }

    if !p.tags.is_empty() {
        let tag_list = p
            .tags
            .iter()
            .map(|t| toml_string(t))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("tags = [{tag_list}]\n"));
    }

    out.push('\n');

    match p.asset_class.as_str() {
        "building" => {
            out.push_str("[building]\n");
            out.push_str(&format!("window_brightness = {}\n", p.window_brightness));
            let placement_mode = p.placement_mode.trim();
            let zone = p.zone_type.as_deref().unwrap_or("residential");
            out.push_str(&format!("placement_mode = \"{placement_mode}\"\n"));
            if placement_mode == "zoned_private" {
                out.push_str(&format!("zone_type = {}\n", toml_string(zone)));
                let density = p.density.as_deref().unwrap_or("low");
                out.push_str(&format!("density = {}\n", toml_string(density)));
            }
            out.push_str(&format!("lot_width_cells = {}\n", p.lot_width_cells));
            out.push_str(&format!("lot_depth_cells = {}\n", p.lot_depth_cells));
            let [fx, fy, fz] = p.frontage_forward;
            out.push_str(&format!("frontage_forward = [{fx}, {fy}, {fz}]\n"));
            if let Some(min_width) = p.min_zone_width_cells {
                out.push_str(&format!("min_zone_width_cells = {min_width}\n"));
            }
            if let Some(min_depth) = p.min_zone_depth_cells {
                out.push_str(&format!("min_zone_depth_cells = {min_depth}\n"));
            }
            out.push_str(&format!("level = {}\n", p.level));
            // Export authored values, not effective runtime defaults. Hidden/dormant
            // fields may only be cleared by an explicit, confirmed editor command.
            if let Some(h) = p.household_capacity {
                out.push_str(&format!("household_capacity = {h}\n"));
            }
            if let Some(w) = p.worker_capacity {
                out.push_str(&format!("worker_capacity = {w}\n"));
            }
            if let Some(f) = p.flat_size_m2 {
                out.push_str(&format!("flat_size_m2 = {f}\n"));
            }
            if let Some(sc) = &p.service_class {
                if !sc.is_empty() && sc != "none" {
                    out.push_str(&format!("service_class = {}\n", toml_string(sc)));
                }
            }
            if let Some(ep) = &p.economy_profile {
                if !ep.is_empty() {
                    out.push_str(&format!("economy_profile = {}\n", toml_string(ep)));
                }
            }
            if let Some(resource) = non_empty_optional_string(&p.extractor_resource) {
                let area_mode =
                    non_empty_optional_string(&p.extractor_area_mode).unwrap_or("player_polygon");
                out.push_str("\n[building.extractor]\n");
                out.push_str(&format!("resource = {}\n", toml_string(resource)));
                out.push_str(&format!("area_mode = {}\n", toml_string(area_mode)));
            }
            if let Some(resource) = non_empty_optional_string(&p.field_resource) {
                let area_mode =
                    non_empty_optional_string(&p.field_area_mode).unwrap_or("player_polygon");
                out.push_str("\n[building.field]\n");
                out.push_str(&format!("resource = {}\n", toml_string(resource)));
                out.push_str(&format!("area_mode = {}\n", toml_string(area_mode)));
            }
        }
        other => {
            // Future: prop, vehicle. Return an error-shaped string that the caller detects.
            out.push_str(&format!("# unsupported asset_class: {other}\n"));
        }
    }

    for part in &p.mesh_parts {
        out.push_str("\n[[mesh_parts]]\n");
        out.push_str(&format!("name = {}\n", toml_string(&part.name)));
        let [x, y, z] = part.position;
        out.push_str(&format!("position = [{x}, {y}, {z}]\n"));
        let [rx, ry, rz] = part.rotation_degrees;
        out.push_str(&format!("rotation_degrees = [{rx}, {ry}, {rz}]\n"));
        out.push_str(&format!("scale = {}\n", part.scale));
        if let Some([px, py, pz]) = part.pivot_offset {
            out.push_str(&format!("pivot_offset = [{px}, {py}, {pz}]\n"));
        }
        for lod in &part.lods {
            out.push_str("\n[[mesh_parts.lods]]\n");
            out.push_str(&format!("file = {}\n", toml_string(&lod.file)));
            out.push_str(&format!("distance_min_m = {}\n", lod.distance_min_m));
            if let Some(max) = lod.distance_max_m {
                out.push_str(&format!("distance_max_m = {max}\n"));
            }
        }
    }

    for anchor in &p.anchors {
        out.push_str("\n[[anchors]]\n");
        out.push_str(&format!("type = {}\n", toml_string(&anchor.anchor_type)));
        if !anchor.name.is_empty() {
            out.push_str(&format!("name = {}\n", toml_string(&anchor.name)));
        }
        let [x, y, z] = anchor.position;
        out.push_str(&format!("position = [{x}, {y}, {z}]\n"));
        let [fx, fy, fz] = anchor.forward;
        out.push_str(&format!("forward = [{fx}, {fy}, {fz}]\n"));
        if let Some(width) = anchor.width_m {
            out.push_str(&format!("width_m = {width}\n"));
        }
        if let Some(length) = anchor.length_m {
            out.push_str(&format!("length_m = {length}\n"));
        }
        if let Some(vehicle_class) = &anchor.vehicle_class {
            if !vehicle_class.is_empty() {
                out.push_str(&format!("vehicle_class = {}\n", toml_string(vehicle_class)));
            }
        }
    }

    for surface in &p.site_surfaces {
        out.push_str("\n[[site_surfaces]]\n");
        out.push_str(&format!("material = {}\n", toml_string(&surface.material)));
        if !surface.name.is_empty() {
            out.push_str(&format!("name = {}\n", toml_string(&surface.name)));
        }
        out.push_str(&format!("y_m = {}\n", surface.y_m));
        out.push_str("vertices = [");
        for (index, [x, z]) in surface.vertices.iter().copied().enumerate() {
            if index > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!("[{x}, {z}]"));
        }
        out.push_str("]\n");
    }

    if let Some(appearance) = &p.appearance {
        // Serialize the nested contract through serde, retaining proper TOML escaping.
        let table = std::collections::BTreeMap::from([(
            "building",
            std::collections::BTreeMap::from([("appearance", appearance)]),
        )]);
        let encoded = toml::to_string(&table).map_err(|error| error.to_string())?;
        out.push('\n');
        out.push_str(&encoded);
    }
    Ok(out)
}

fn validate_against_builtin_zoning(params: &ExportParams) -> Result<(), String> {
    let registry = load_builtin_profile_registry()?;
    let zone_type = params.zone_type.as_deref().unwrap_or("residential");
    let density = params.density.as_deref().unwrap_or("low");
    let matches_any_profile = registry.profiles().iter().any(|profile| {
        profile.zone_type.as_str() == zone_type && profile.density.as_str() == density
    });
    if !matches_any_profile {
        return Err(format!(
            "unsupported zoned building legality '{} + {}' for the baseline shipped zoning profiles; office and mixed remain future extensions",
            zone_type, density
        ));
    }
    Ok(())
}

fn anchor_type_key(anchor_type: AnchorType) -> &'static str {
    match anchor_type {
        AnchorType::Entrance => "entrance",
        AnchorType::Driveway => "driveway",
        AnchorType::Parking => "parking",
        AnchorType::LoadingBay => "loading_bay",
        AnchorType::Wheel => "wheel",
        AnchorType::Light => "light",
    }
}

fn site_surface_material_key(material: SiteSurfaceMaterial) -> &'static str {
    match material {
        SiteSurfaceMaterial::Asphalt => "asphalt",
        SiteSurfaceMaterial::Concrete => "concrete",
    }
}

fn parse_placement_mode(value: &str) -> Result<PlacementMode, String> {
    match value.trim() {
        "zoned_private" => Ok(PlacementMode::ZonedPrivate),
        "explicit" => Ok(PlacementMode::Explicit),
        _ => Err(format!("unsupported placement_mode '{}'", value.trim())),
    }
}

fn non_empty_optional_string(value: &Option<String>) -> Option<&str> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn non_none_service_class(value: &Option<String>) -> Option<&str> {
    non_empty_optional_string(value).filter(|value| *value != "none")
}

fn validate_service_class(service_class: &str) -> Result<(), String> {
    match service_class.trim() {
        "police" | "fire" | "healthcare" | "education" | "power" | "water" | "waste"
        | "transit" | "parks" | "government" => Ok(()),
        other => Err(format!(
            "unsupported service_class '{}'; expected one of police, fire, healthcare, education, power, water, waste, transit, parks, government",
            other
        )),
    }
}

fn expected_utility_service_for_class(service_class: &str) -> Option<&'static str> {
    match service_class.trim() {
        "power" => Some("power"),
        "water" => Some("water"),
        "waste" => Some("sewage"),
        _ => None,
    }
}

fn is_utility_service_class(service_class: &str) -> bool {
    expected_utility_service_for_class(service_class).is_some()
}

fn validate_utility_profile_matches_service(
    economy_profile: &str,
    service_class: &str,
) -> Result<(), String> {
    let Some(expected_service) = expected_utility_service_for_class(service_class) else {
        return Ok(());
    };
    let catalog = load_runtime_economy_catalog()
        .map_err(|err| format!("could not load economy catalog for utility validation: {err}"))?;
    let profile = catalog
        .all_profiles()
        .iter()
        .find(|profile| profile.id == economy_profile)
        .ok_or_else(|| {
            format!(
                "utility economy_profile '{economy_profile}' is missing from the runtime catalog"
            )
        })?;
    if !matches!(
        profile.kind,
        EconomyProfileRuntimeKind::UtilityProducer | EconomyProfileRuntimeKind::UtilityProcessor
    ) {
        return Err(format!(
            "utility economy_profile '{}' must be a utility producer or processor",
            economy_profile
        ));
    }
    if profile.utility_service.as_deref() != Some(expected_service) {
        return Err(format!(
            "utility economy_profile '{}' does not provide the '{}' service",
            economy_profile, expected_service
        ));
    }
    Ok(())
}

fn validate_extractor_profile_matches_resource(
    economy_profile: &str,
    resource_id: &str,
) -> Result<(), String> {
    let resource_id = resource_id.trim();
    if resource_id.is_empty() {
        return Err("extractor_resource must not be empty".to_owned());
    }
    let catalog = load_runtime_economy_catalog()
        .map_err(|err| format!("could not load economy catalog for extractor validation: {err}"))?;
    let profile = catalog
        .all_profiles()
        .iter()
        .find(|profile| profile.id == economy_profile)
        .ok_or_else(|| {
            format!(
                "extractor economy_profile '{economy_profile}' is missing from the runtime catalog"
            )
        })?;
    if profile.kind != EconomyProfileRuntimeKind::Extractor {
        return Err(format!(
            "extractor economy_profile '{}' must have kind = \"extractor\"",
            economy_profile
        ));
    }
    let Some(resource_runtime_id) = catalog.resource_runtime_id_for_id(resource_id) else {
        return Err(format!(
            "extractor resource '{}' is missing from the runtime catalog",
            resource_id
        ));
    };
    if profile.output_port(resource_runtime_id).is_none() {
        return Err(format!(
            "extractor economy_profile '{}' must output resource '{}'",
            economy_profile, resource_id
        ));
    }
    if profile.worker_capacity == 0 {
        return Err(format!(
            "extractor economy_profile '{}' must have worker_capacity > 0",
            economy_profile
        ));
    }
    Ok(())
}

fn validate_field_profile_matches_resource(
    economy_profile: &str,
    resource_id: &str,
) -> Result<(), String> {
    let resource_id = resource_id.trim();
    if resource_id.is_empty() {
        return Err("field_resource must not be empty".to_owned());
    }
    let catalog = load_runtime_economy_catalog()
        .map_err(|err| format!("could not load economy catalog for field validation: {err}"))?;
    let profile = catalog
        .all_profiles()
        .iter()
        .find(|profile| profile.id == economy_profile)
        .ok_or_else(|| {
            format!("field economy_profile '{economy_profile}' is missing from the runtime catalog")
        })?;
    if profile.kind != EconomyProfileRuntimeKind::FieldProducer {
        return Err(format!(
            "field economy_profile '{}' must have kind = \"field_producer\"",
            economy_profile
        ));
    }
    let Some(resource_runtime_id) = catalog.resource_runtime_id_for_id(resource_id) else {
        return Err(format!(
            "field resource '{}' is missing from the runtime catalog",
            resource_id
        ));
    };
    if profile.output_port(resource_runtime_id).is_none() {
        return Err(format!(
            "field economy_profile '{}' must output resource '{}'",
            economy_profile, resource_id
        ));
    }
    if profile.worker_capacity == 0 {
        return Err(format!(
            "field economy_profile '{}' must have worker_capacity > 0",
            economy_profile
        ));
    }
    Ok(())
}

fn validate_building_export_contract(params: &ExportParams) -> Result<(), String> {
    let placement_mode = parse_placement_mode(&params.placement_mode)?;
    let service_class = non_none_service_class(&params.service_class);
    let economy_profile = non_empty_optional_string(&params.economy_profile);
    let extractor_resource = non_empty_optional_string(&params.extractor_resource);
    let extractor_area_mode =
        non_empty_optional_string(&params.extractor_area_mode).unwrap_or("player_polygon");
    let field_resource = non_empty_optional_string(&params.field_resource);
    let field_area_mode =
        non_empty_optional_string(&params.field_area_mode).unwrap_or("player_polygon");
    for (resource, mode, name) in [
        (extractor_resource, &params.extractor_area_mode, "extractor"),
        (field_resource, &params.field_area_mode, "field"),
    ] {
        if resource.is_none() && non_empty_optional_string(mode).is_some() {
            return Err(format!("{name}_area_mode requires {name}_resource"));
        }
    }
    if let Some(service_class) = service_class {
        validate_service_class(service_class)?;
    }
    if extractor_resource.is_some() && extractor_area_mode != "player_polygon" {
        return Err(
            "extractor_area_mode must be \"player_polygon\" for extractor buildings".to_owned(),
        );
    }
    if field_resource.is_some() && field_area_mode != "player_polygon" {
        return Err("field_area_mode must be \"player_polygon\" for field buildings".to_owned());
    }
    if extractor_resource.is_some() && field_resource.is_some() {
        return Err("buildings cannot export both extractor and field metadata".to_owned());
    }
    if params.lot_width_cells == 0 || params.lot_depth_cells == 0 {
        return Err("lot_width_cells and lot_depth_cells must be > 0".to_owned());
    }
    if params.min_zone_width_cells == Some(0) || params.min_zone_depth_cells == Some(0) {
        return Err("min_zone_width_cells and min_zone_depth_cells must be > 0".to_owned());
    }
    if params.mesh_parts.is_empty() {
        return Err("building exports require at least one mesh part".to_owned());
    }

    match placement_mode {
        PlacementMode::ZonedPrivate => {
            let zone_type = non_empty_optional_string(&params.zone_type)
                .ok_or_else(|| "zoned_private buildings require zone_type".to_owned())?;
            let _density = non_empty_optional_string(&params.density)
                .ok_or_else(|| "zoned_private buildings require density".to_owned())?;
            if service_class.is_some() {
                return Err(
                    "zoned_private buildings must not export service_class; use explicit placement for service or utility assets"
                        .to_owned(),
                );
            }
            if extractor_resource.is_some() || field_resource.is_some() {
                return Err(
                    "zoned_private buildings must not export extractor or field metadata; use explicit placement for industry assets"
                        .to_owned(),
                );
            }
            validate_against_builtin_zoning(params)?;
            match zone_type {
                "residential" => {
                    if economy_profile.is_some() {
                        return Err("residential buildings must not export economy_profile".into());
                    }
                    if params.household_capacity.unwrap_or(0) == 0 {
                        return Err(
                            "residential zoned_private buildings require household_capacity"
                                .to_owned(),
                        );
                    }
                    if params.worker_capacity.unwrap_or(0) > 0 {
                        return Err(
                            "residential zoned_private buildings must not use worker_capacity"
                                .to_owned(),
                        );
                    }
                }
                "commercial" | "industrial" => {
                    if params.worker_capacity.unwrap_or(0) == 0 && economy_profile.is_none() {
                        return Err(
                            "commercial and industrial zoned_private buildings require worker_capacity or economy_profile"
                                .to_owned(),
                        );
                    }
                    if let Some(id) = economy_profile {
                        let catalog = load_runtime_economy_catalog()?;
                        let data = serde_json::json!({
                            "asset_class": "building", "placement_mode": "zoned_private", "zone_type": zone_type,
                        });
                        if !catalog.profile_for_id(id).is_some_and(|profile| {
                            crate::assets::authoring::profile_matches(&data, profile, &catalog)
                        }) {
                            return Err(format!(
                                "economy_profile '{id}' is not an executable {zone_type} profile"
                            ));
                        }
                    }
                }
                other => {
                    return Err(format!(
                        "unsupported zoned_private zone_type '{}' for the baseline contract",
                        other
                    ));
                }
            }
        }
        PlacementMode::Explicit => {
            if non_empty_optional_string(&params.zone_type).is_some()
                || non_empty_optional_string(&params.density).is_some()
            {
                return Err("explicit buildings must not export zone_type or density".to_owned());
            }
            if economy_profile.is_some()
                && extractor_resource.is_none()
                && field_resource.is_none()
                && !service_class.is_some_and(is_utility_service_class)
            {
                return Err(
                    "economy_profile requires an extractor, farm or utility building contract"
                        .into(),
                );
            }
            if let Some(service_class) = service_class {
                if is_utility_service_class(service_class) {
                    let Some(economy_profile) = economy_profile else {
                        return Err(
                            "explicit utility service buildings require economy_profile".to_owned()
                        );
                    };
                    validate_utility_profile_matches_service(economy_profile, service_class)?;
                }
            }
            if let Some(resource_id) = extractor_resource {
                if service_class.is_some() {
                    return Err(
                        "extractor buildings must not export service_class; use the Industry toolbar"
                            .to_owned(),
                    );
                }
                let Some(economy_profile) = economy_profile else {
                    return Err("extractor buildings require economy_profile".to_owned());
                };
                validate_extractor_profile_matches_resource(economy_profile, resource_id)?;
            }
            if let Some(resource_id) = field_resource {
                if service_class.is_some() {
                    return Err(
                        "field buildings must not export service_class; use the Industry toolbar"
                            .to_owned(),
                    );
                }
                let Some(economy_profile) = economy_profile else {
                    return Err("field buildings require economy_profile".to_owned());
                };
                validate_field_profile_matches_resource(economy_profile, resource_id)?;
            }
        }
    }

    Ok(())
}

// ── Public helpers called from SimulationNode ─────────────────────────────────

/// Validate a draft's runtime metadata without writing files or requiring a live city.
/// Mesh dependency existence is checked separately by the editor's packaging service.
pub(crate) fn validate_asset_params_internal(params_json: &str) -> String {
    let params: ExportParams = match serde_json::from_str(params_json) {
        Ok(params) => params,
        Err(error) => return format!("JSON parse error: {error}"),
    };
    validated_tomls(&params).err().unwrap_or_default()
}

pub(crate) fn validated_tomls(params: &ExportParams) -> Result<(String, String), String> {
    if params.asset_class != "building" {
        return Err(format!(
            "unsupported asset_class '{}' (building authoring only)",
            params.asset_class
        ));
    }
    validate_building_export_contract(params)?;
    let asset_toml = build_asset_toml(params)?;
    asset_toml
        .parse::<AssetManifest>()
        .map_err(|error| format!("validation error: {error}"))?;
    let pack_toml = build_pack_toml(params);
    PackManifest::from_str(&pack_toml)
        .map_err(|error| format!("pack validation error: {error}"))?;
    Ok((asset_toml, pack_toml))
}

/// Returns a JSON object describing the manifest for an already-registered asset,
/// suitable for repopulating the importer form. Returns `""` if not found.
pub fn get_asset_manifest_json_internal(
    registry: &crate::assets::AssetRegistry,
    qualified_id: &str,
) -> String {
    let Some(entry) = registry.get(qualified_id) else {
        debug_log!(
            "asset-editor",
            "get_asset_manifest_json: '{}' not found",
            qualified_id
        );
        return String::new();
    };
    debug_log!(
        "asset-editor",
        "get_asset_manifest_json: loading '{}'",
        qualified_id
    );
    let m = &entry.manifest;

    let mut obj = serde_json::json!({
        "pack_id": entry.pack_id,
        "asset_id": m.asset_id,
        "display_name": m.display_name,
        "thumbnail": m.thumbnail,
        "asset_set": m.asset_set,
        "tags": m.tags,
        "asset_class": m.class().map(|c| format!("{c:?}").to_lowercase()).unwrap_or_default(),
        "mesh_parts": m.mesh_parts.iter().map(|part| serde_json::json!({
            "name": part.name,
            "position": part.position,
            "rotation_degrees": part.rotation_degrees,
            "scale": part.scale,
            "pivot_offset": part.pivot_offset,
            "lods": part.lods.iter().map(|l| serde_json::json!({
                "file": l.file,
                "distance_min_m": l.distance_min_m,
                "distance_max_m": l.distance_max_m,
            })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "anchors": m.anchors.iter().map(|a| serde_json::json!({
            "anchor_type": anchor_type_key(a.anchor_type),
            "name": a.name,
            "position": a.position,
            "forward": a.forward,
            "width_m": a.width_m,
            "length_m": a.length_m,
            "vehicle_class": a.vehicle_class.as_deref(),
        })).collect::<Vec<_>>(),
        "site_surfaces": m.site_surfaces.iter().map(|s| serde_json::json!({
            "material": site_surface_material_key(s.material),
            "name": s.name,
            "y_m": s.y_m,
            "vertices": s.vertices,
        })).collect::<Vec<_>>(),
    });

    if let Some(b) = &m.building {
        obj["window_brightness"] = serde_json::json!(b.window_brightness);
        obj["appearance"] = serde_json::json!(b.appearance);
        obj["placement_mode"] = serde_json::json!(match b.placement_mode {
            PlacementMode::ZonedPrivate => "zoned_private",
            PlacementMode::Explicit => "explicit",
        });
        obj["zone_type"] = match b.zone_type {
            Some(zone_type) => serde_json::json!(format!("{:?}", zone_type).to_lowercase()),
            None => serde_json::Value::Null,
        };
        obj["density"] = match b.density_key() {
            Some(density) => serde_json::json!(density),
            None => serde_json::Value::Null,
        };
        obj["lot_width_cells"] = serde_json::json!(b.lot_width_cells);
        obj["lot_depth_cells"] = serde_json::json!(b.lot_depth_cells);
        obj["frontage_forward"] = serde_json::json!(m.building_frontage_forward());
        obj["min_zone_width_cells"] = serde_json::json!(b.min_zone_width_cells);
        obj["min_zone_depth_cells"] = serde_json::json!(b.min_zone_depth_cells);
        obj["level"] = serde_json::json!(b.level);
        obj["household_capacity"] = serde_json::json!(b.household_capacity);
        obj["flat_size_m2"] = serde_json::json!(b.flat_size_m2);
        obj["worker_capacity"] = serde_json::json!(b.worker_capacity);
        obj["service_class"] = serde_json::json!(b.service_class.as_deref().unwrap_or("none"));
        obj["economy_profile"] = serde_json::json!(b.economy_profile);
        obj["extractor_resource"] = serde_json::json!(
            b.extractor
                .as_ref()
                .map(|extractor| extractor.resource.as_str())
        );
        obj["extractor_area_mode"] = serde_json::json!(
            b.extractor
                .as_ref()
                .map(|extractor| extractor.area_mode.as_str())
        );
        obj["field_resource"] =
            serde_json::json!(b.field.as_ref().map(|field| field.resource.as_str()));
        obj["field_area_mode"] =
            serde_json::json!(b.field.as_ref().map(|field| field.area_mode.as_str()));
    }

    serde_json::to_string(&obj).unwrap_or_default()
}

/// Reads `<pack_dir>/pack.toml` and returns a JSON object with pack metadata,
/// or `""` if the file is missing or fails to parse.
pub fn get_pack_manifest_json_internal(pack_dir: &str) -> String {
    let path = Path::new(pack_dir).join("pack.toml");
    let toml_str = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => {
            debug_log!(
                "asset-editor",
                "get_pack_manifest_json: no pack.toml at {}",
                path.display()
            );
            return String::new();
        }
    };
    let pack = match PackManifest::from_str(&toml_str) {
        Ok(p) => p,
        Err(e) => {
            debug_log!("asset-editor", "get_pack_manifest_json: parse error: {e}");
            return String::new();
        }
    };
    let obj = serde_json::json!({
        "pack_id":      pack.pack_id,
        "display_name": pack.display_name,
        "author":       pack.author,
        "version":      pack.version,
        "license":      pack.license,
    });
    serde_json::to_string(&obj).unwrap_or_default()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn one_mesh_part_json() -> serde_json::Value {
        serde_json::json!([{
            "name": "main",
            "position": [0.0, 0.0, 0.0],
            "rotation_degrees": [0.0, 0.0, 0.0],
            "scale": 1.0,
            "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}]
        }])
    }

    fn minimal_building_json(asset_id: &str) -> String {
        serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": asset_id,
            "display_name": "Test House",
            "placement_mode": "zoned_private",
            "zone_type": "residential",
            "density": "low",
            "lot_width_cells": 2,
            "lot_depth_cells": 2,
            "level": 1,
            "household_capacity": 6,
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 2.0],
                "forward": [0.0, 0.0, 1.0]
            }]
        })
        .to_string()
    }

    #[test]
    fn godot_whole_number_json_is_lossless_but_fractions_and_overflow_fail() {
        let mut data: serde_json::Value =
            serde_json::from_str(&minimal_building_json("building.residential.numeric")).unwrap();
        for key in [
            "lot_width_cells",
            "lot_depth_cells",
            "level",
            "household_capacity",
        ] {
            data[key] = serde_json::json!(data[key].as_f64().unwrap());
        }
        assert!(validate_asset_params_internal(&data.to_string()).is_empty());
        data["household_capacity"] = serde_json::json!(1.5);
        assert!(validate_asset_params_internal(&data.to_string()).contains("whole number"));
        data["household_capacity"] = serde_json::json!(4294967296.0);
        assert!(validate_asset_params_internal(&data.to_string()).contains("range"));
        data["household_capacity"] = serde_json::Value::Null;
        let params: ExportParams = serde_json::from_value(data).unwrap();
        assert_eq!(params.household_capacity, None);
    }

    #[test]
    fn window_brightness_defaults_validates_and_round_trips() {
        let mut data: serde_json::Value =
            serde_json::from_str(&minimal_building_json("building.residential.windows")).unwrap();
        let params: ExportParams = serde_json::from_value(data.clone()).unwrap();
        assert_eq!(params.window_brightness, 3.0);
        for brightness in [0.0, 2.75, 10.0] {
            data["window_brightness"] = serde_json::json!(brightness);
            let params: ExportParams = serde_json::from_value(data.clone()).unwrap();
            let (toml, _) = validated_tomls(&params).unwrap();
            let manifest = AssetManifest::from_str(&toml).unwrap();
            assert_eq!(
                manifest.building.as_ref().unwrap().window_brightness,
                brightness
            );
            let mut registry = crate::assets::registry::AssetRegistry::new();
            registry.register("test-pack", manifest, String::new());
            let loaded: serde_json::Value =
                serde_json::from_str(&get_asset_manifest_json_internal(
                    &registry,
                    "test-pack:building.residential.windows",
                ))
                .unwrap();
            assert_eq!(loaded["window_brightness"], brightness);
        }
        for brightness in [-0.1, 10.1] {
            data["window_brightness"] = serde_json::json!(brightness);
            assert!(
                validate_asset_params_internal(&data.to_string()).contains("window_brightness")
            );
        }
    }

    #[test]
    fn round_trip_preserves_thumbnail_small_pivot_and_precise_area() {
        let mut data: serde_json::Value =
            serde_json::from_str(&minimal_building_json("building.residential.precise")).unwrap();
        data["thumbnail"] = serde_json::json!("preview.png");
        data["appearance"] = serde_json::json!({
            "default_scheme": "red", "spawn": "random_scheme",
            "schemes": [{"id": "red", "name": "Red", "overrides": [{
                "part": "main", "materials": ["walls"], "albedo": "red.png"
            }]}]
        });
        data["flat_size_m2"] = serde_json::json!(60.125);
        data["min_zone_width_cells"] = serde_json::json!(2);
        data["mesh_parts"][0]["pivot_offset"] = serde_json::json!([0.00001, 0.0, 0.0]);
        let params: ExportParams = serde_json::from_value(data).unwrap();
        let (toml, _) = validated_tomls(&params).unwrap();
        let manifest = AssetManifest::from_str(&toml).unwrap();
        assert_eq!(
            manifest.building.as_ref().unwrap().appearance,
            params.appearance
        );
        assert_eq!(manifest.thumbnail.as_deref(), Some("preview.png"));
        assert_eq!(
            manifest.building.as_ref().unwrap().flat_size_m2,
            Some(60.125)
        );
        assert_eq!(
            manifest.building.as_ref().unwrap().min_zone_width_cells,
            Some(2)
        );
        assert_eq!(
            manifest.mesh_parts[0].pivot_offset,
            Some([0.00001, 0.0, 0.0])
        );
        let mut registry = crate::assets::registry::AssetRegistry::new();
        registry.register("test-pack", manifest, String::new());
        let loaded: serde_json::Value = serde_json::from_str(&get_asset_manifest_json_internal(
            &registry,
            "test-pack:building.residential.precise",
        ))
        .unwrap();
        assert_eq!(loaded["thumbnail"], "preview.png");
        assert_eq!(loaded["flat_size_m2"], 60.125);
    }

    #[test]
    fn preflight_uses_export_validation_without_writing() {
        let source = minimal_building_json("building.residential.preflight");
        assert!(validate_asset_params_internal(&source).is_empty());
        let mut invalid: serde_json::Value = serde_json::from_str(&source).unwrap();
        invalid["anchors"] = serde_json::json!([]);
        assert!(validate_asset_params_internal(&invalid.to_string()).contains("entrance"));
    }

    #[test]
    fn export_rejects_unknown_placement_and_orphaned_area_metadata() {
        let source: serde_json::Value =
            serde_json::from_str(&minimal_building_json("building.residential.strict")).unwrap();
        for mode in ["", "future_placement", "zoned_prvate"] {
            let mut data = source.clone();
            data["placement_mode"] = serde_json::json!(mode);
            assert!(validate_asset_params_internal(&data.to_string()).contains("placement_mode"));
        }
        for kind in ["extractor", "field"] {
            let mut data = source.clone();
            data[format!("{kind}_area_mode")] = serde_json::json!("player_polygon");
            assert!(
                validate_asset_params_internal(&data.to_string())
                    .contains(&format!("{kind}_area_mode requires {kind}_resource"))
            );
        }
    }

    #[test]
    fn export_preserves_dormant_anchor_dimensions() {
        let mut data: serde_json::Value =
            serde_json::from_str(&minimal_building_json("building.residential.dimensions"))
                .unwrap();
        data["anchors"][0]["width_m"] = serde_json::json!(0.0);
        data["anchors"][0]["length_m"] = serde_json::json!(-1.0);
        let params: ExportParams = serde_json::from_value(data).unwrap();
        let (toml, _) = validated_tomls(&params).unwrap();
        let manifest = AssetManifest::from_str(&toml).unwrap();
        assert_eq!(manifest.anchors[0].width_m, Some(0.0));
        assert_eq!(manifest.anchors[0].length_m, Some(-1.0));
    }

    #[test]
    fn publication_rejects_profiles_hidden_by_authoring_type() {
        let mut data: serde_json::Value =
            serde_json::from_str(&minimal_building_json("building.residential.profile")).unwrap();
        data["economy_profile"] = serde_json::json!("grocery_basic");
        assert!(validate_asset_params_internal(&data.to_string()).contains("economy_profile"));
        data["placement_mode"] = serde_json::json!("explicit");
        data["zone_type"] = serde_json::Value::Null;
        data["density"] = serde_json::Value::Null;
        for service in [None, Some("police")] {
            data["service_class"] = serde_json::json!(service);
            assert!(validate_asset_params_internal(&data.to_string()).contains("economy_profile"));
        }
    }

    #[test]
    fn commercial_export_rejects_extraction_profiles() {
        let mut data: serde_json::Value =
            serde_json::from_str(&minimal_building_json("building.commercial.shop")).unwrap();
        data["zone_type"] = serde_json::json!("commercial");
        data["household_capacity"] = serde_json::Value::Null;
        data["economy_profile"] = serde_json::json!("coal_mine_basic");
        assert!(
            validate_asset_params_internal(&data.to_string())
                .contains("not an executable commercial profile")
        );
        data["economy_profile"] = serde_json::json!("grocery_basic");
        assert!(validate_asset_params_internal(&data.to_string()).is_empty());
    }

    #[test]
    fn export_writes_frontage_forward_separately_from_entrance_forward() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.residential.frontage_split",
            "display_name": "Frontage Split",
            "placement_mode": "zoned_private",
            "zone_type": "residential",
            "density": "low",
            "lot_width_cells": 2,
            "lot_depth_cells": 2,
            "frontage_forward": [1.0, 0.0, 0.0],
            "level": 1,
            "household_capacity": 6,
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 2.0],
                "forward": [0.0, 0.0, -1.0]
            }]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(result.is_empty(), "expected success, got: {result}");

        let asset_toml = validated_tomls(&serde_json::from_str(&json).unwrap())
            .unwrap()
            .0;
        assert!(asset_toml.contains("frontage_forward = [1, 0, 0]"));
        assert!(asset_toml.contains("forward = [0, 0, -1]"));
    }

    #[test]
    fn export_escapes_toml_strings() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test \"Pack\"",
            "pack_author": "Line\nAuthor",
            "asset_class": "building",
            "asset_id": "building.residential.escaped_house",
            "display_name": "Quoted \"House\"",
            "tags": ["quoted \"tag\"", "line\nbreak"],
            "placement_mode": "zoned_private",
            "zone_type": "residential",
            "density": "low",
            "lot_width_cells": 2,
            "lot_depth_cells": 2,
            "level": 1,
            "household_capacity": 6,
            "mesh_parts": one_mesh_part_json(),
            "anchors": [
                {
                    "anchor_type": "entrance",
                    "name": "main",
                    "position": [0.0, 0.0, 2.0],
                    "forward": [0.0, 0.0, 1.0]
                },
                {
                    "anchor_type": "parking",
                    "name": "bay \"north\"\n",
                    "position": [0.0, 0.0, 0.0],
                    "forward": [0.0, 0.0, 1.0],
                    "width_m": 2.5,
                    "length_m": 5.0,
                    "vehicle_class": "car"
                }
            ]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(result.is_empty(), "expected success, got: {result}");

        let asset_toml = validated_tomls(&serde_json::from_str(&json).unwrap())
            .unwrap()
            .0;
        assert!(asset_toml.contains("name = \"bay \\\"north\\\"\\n\""));
        assert!(asset_toml.contains("vehicle_class = \"car\""));
        asset_toml
            .parse::<AssetManifest>()
            .expect("escaped TOML should round-trip");
    }

    #[test]
    fn export_rejects_unknown_json_fields() {
        let mut json: serde_json::Value = serde_json::from_str(&minimal_building_json(
            "building.residential.unknown_fields",
        ))
        .unwrap();
        json["legacy_field"] = serde_json::json!(true);

        let result = validate_asset_params_internal(&json.to_string());
        assert!(
            result.contains("unknown field"),
            "expected unknown-field parse error, got: {result}"
        );
    }

    #[test]
    fn export_rejects_invalid_asset_id() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "Bad.ID",
            "display_name": "Bad",
            "placement_mode": "zoned_private",
            "zone_type": "residential",
            "density": "low",
            "lot_width_cells": 2,
            "lot_depth_cells": 2,
            "mesh_parts": one_mesh_part_json()
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(!result.is_empty(), "expected validation error");
    }

    #[test]
    fn export_rejects_zero_lot_cells() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.residential.house",
            "display_name": "House",
            "placement_mode": "zoned_private",
            "zone_type": "residential",
            "density": "low",
            "lot_width_cells": 0,
            "lot_depth_cells": 2,
            "mesh_parts": one_mesh_part_json()
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(
            !result.is_empty(),
            "expected validation error for zero lot cells"
        );
    }

    #[test]
    fn export_writes_economy_profile_when_selected() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.commercial.grocery_test",
            "display_name": "Grocery Test",
            "placement_mode": "zoned_private",
            "zone_type": "commercial",
            "density": "low",
            "lot_width_cells": 2,
            "lot_depth_cells": 2,
            "household_capacity": 3,
            "flat_size_m2": 60.0,
            "worker_capacity": 8,
            "economy_profile": "grocery_basic",
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 0.5],
                "forward": [0.0, 0.0, 1.0]
            }]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(result.is_empty(), "expected success, got: {result}");

        let asset_toml = validated_tomls(&serde_json::from_str(&json).unwrap())
            .unwrap()
            .0;
        assert!(asset_toml.contains("economy_profile = \"grocery_basic\""));
        assert!(asset_toml.contains("worker_capacity = 8"));
        assert!(asset_toml.contains("household_capacity = 3"));
        assert!(asset_toml.contains("flat_size_m2 = 60"));
    }

    #[test]
    fn farm_export_preserves_authored_fields_while_runtime_houses_one_family() {
        for area in [None, Some(180.0)] {
            let mut json: serde_json::Value =
                serde_json::from_str(&minimal_building_json("building.farm")).unwrap();
            json["placement_mode"] = serde_json::json!("explicit");
            json["zone_type"] = serde_json::Value::Null;
            json["density"] = serde_json::Value::Null;
            json["economy_profile"] = serde_json::json!("grain_farm_basic");
            json["field_resource"] = serde_json::json!("grain");
            json["household_capacity"] = serde_json::json!(9);
            json["flat_size_m2"] = serde_json::json!(area);
            let params: ExportParams = serde_json::from_value(json).unwrap();
            validate_building_export_contract(&params).unwrap();
            let manifest = AssetManifest::from_str(&build_asset_toml(&params).unwrap()).unwrap();
            let building = manifest.building.as_ref().unwrap();
            assert_eq!(building.household_capacity, Some(9));
            assert_eq!(building.flat_size_m2, area);
            let mut registry = crate::assets::registry::AssetRegistry::new();
            registry.register("test-pack", manifest, String::new());
            let id = "test-pack:building.farm";
            let resolved_area =
                area.unwrap_or(crate::assets::asset::BuildingData::DEFAULT_FARMHOUSE_AREA_M2);
            assert_eq!(registry.household_capacity(id), 1);
            assert_eq!(registry.flat_size_m2(id), resolved_area);
            let imported: serde_json::Value =
                serde_json::from_str(&get_asset_manifest_json_internal(&registry, id)).unwrap();
            assert_eq!(imported["household_capacity"], 9);
            assert_eq!(imported["flat_size_m2"], serde_json::json!(area));
        }
    }

    #[test]
    fn export_accepts_profile_bound_zoned_employment_without_worker_capacity() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.industrial.profile_capacity",
            "display_name": "Profile Capacity",
            "placement_mode": "zoned_private",
            "zone_type": "industrial",
            "density": "low",
            "lot_width_cells": 2,
            "lot_depth_cells": 2,
            "economy_profile": "food_processor_basic",
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 0.5],
                "forward": [0.0, 0.0, 1.0]
            }]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(result.is_empty(), "expected success, got: {result}");

        let asset_toml = validated_tomls(&serde_json::from_str(&json).unwrap())
            .unwrap()
            .0;
        assert!(asset_toml.contains("economy_profile = \"food_processor_basic\""));
        assert!(!asset_toml.contains("worker_capacity"));
    }

    #[test]
    fn export_rejects_zoned_service_class() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.residential.invalid_service",
            "display_name": "Invalid Service",
            "placement_mode": "zoned_private",
            "zone_type": "residential",
            "density": "low",
            "lot_width_cells": 2,
            "lot_depth_cells": 2,
            "household_capacity": 2,
            "service_class": "fire",
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 0.5],
                "forward": [0.0, 0.0, 1.0]
            }]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(
            result.contains("zoned_private buildings must not export service_class"),
            "expected service-class validation error, got: {result}"
        );
    }

    #[test]
    fn export_rejects_utility_without_economy_profile() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.power.invalid",
            "display_name": "Invalid Power Plant",
            "placement_mode": "explicit",
            "lot_width_cells": 3,
            "lot_depth_cells": 4,
            "worker_capacity": 4,
            "service_class": "power",
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 0.5],
                "forward": [0.0, 0.0, 1.0]
            }]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(
            result.contains("explicit utility service buildings require economy_profile"),
            "expected utility profile validation error, got: {result}"
        );
    }

    #[test]
    fn export_rejects_invalid_service_class() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.service.invalid",
            "display_name": "Invalid Service",
            "placement_mode": "explicit",
            "lot_width_cells": 3,
            "lot_depth_cells": 4,
            "worker_capacity": 4,
            "service_class": "sewage",
            "economy_profile": "wastewater_treatment_basic",
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 0.5],
                "forward": [0.0, 0.0, 1.0]
            }]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(
            result.contains("unsupported service_class 'sewage'"),
            "expected service-class enum validation error, got: {result}"
        );
    }

    #[test]
    fn export_rejects_utility_profile_for_wrong_service() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.power.wrong_profile",
            "display_name": "Wrong Utility Profile",
            "placement_mode": "explicit",
            "lot_width_cells": 3,
            "lot_depth_cells": 4,
            "worker_capacity": 4,
            "service_class": "power",
            "economy_profile": "water_plant_basic",
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 0.5],
                "forward": [0.0, 0.0, 1.0]
            }]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(
            result.contains("does not provide the 'power' service"),
            "expected utility service mismatch error, got: {result}"
        );
    }

    #[test]
    fn export_writes_explicit_utility_profile() {
        let json = serde_json::json!({
            "pack_id": "test-pack",
            "pack_name": "Test Pack",
            "pack_author": "Tester",
            "asset_class": "building",
            "asset_id": "building.power.plant_test",
            "display_name": "Power Plant Test",
            "placement_mode": "explicit",
            "lot_width_cells": 3,
            "lot_depth_cells": 4,
            "worker_capacity": 4,
            "service_class": "power",
            "economy_profile": "power_plant_basic",
            "mesh_parts": one_mesh_part_json(),
            "anchors": [{
                "anchor_type": "entrance",
                "name": "main",
                "position": [0.0, 0.0, 0.5],
                "forward": [0.0, 0.0, 1.0]
            }]
        })
        .to_string();

        let result = validate_asset_params_internal(&json);
        assert!(result.is_empty(), "expected success, got: {result}");

        let asset_toml = validated_tomls(&serde_json::from_str(&json).unwrap())
            .unwrap()
            .0;
        assert!(asset_toml.contains("placement_mode = \"explicit\""));
        assert!(asset_toml.contains("service_class = \"power\""));
        assert!(asset_toml.contains("economy_profile = \"power_plant_basic\""));
    }
}
