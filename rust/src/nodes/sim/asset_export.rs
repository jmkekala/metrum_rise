//! Asset export helpers: validate form data and write `pack.toml` / `asset.toml` to disk.
//!
//! GDScript sends a JSON string describing the form state. Rust validates it, generates
//! well-formed TOML, round-trips it through [`AssetManifest::from_str`] for final
//! validation, and writes the output files. Pack TOML is only written when the file does
//! not already exist, so re-exporting individual assets never overwrites pack metadata.

use crate::assets::asset::PlacementMode;
use crate::assets::{AssetManifest, CURRENT_SCHEMA_VERSION, PackManifest};
use crate::debug_log;
use crate::simulation::economy::definitions::{
    EconomyProfileRuntimeKind, load_runtime_economy_catalog,
};
use crate::simulation::zoning::load_builtin_profile_registry;
use serde::Deserialize;
use std::path::Path;

// ── Input structs (JSON from GDScript) ───────────────────────────────────────

/// LOD entry sent from the building importer form.
#[derive(Deserialize)]
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
pub struct AnchorParams {
    /// Semantic type of this anchor (e.g. `"frontage"`, `"entrance"`).
    pub anchor_type: String,
    /// Identifier for this anchor within the asset (e.g. `"main"`).
    pub name: String,
    /// World-space position of the anchor relative to the asset origin.
    pub position: [f32; 3],
    /// Forward direction vector of the anchor in asset-local space.
    pub forward: [f32; 3],
}

/// One renderable mesh part sent from the building importer form.
#[derive(Deserialize)]
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
pub struct ExportParams {
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
    #[serde(default)]
    pub lot_width_cells: u16,
    /// Footprint depth in 10 m zone cells away from the road.
    #[serde(default)]
    pub lot_depth_cells: u16,
    /// Minimum accepted zoned width for this building.
    #[serde(default)]
    pub min_zone_width_cells: Option<u16>,
    /// Minimum accepted zoned depth for this building.
    #[serde(default)]
    pub min_zone_depth_cells: Option<u16>,
    /// Development level (1 = lowest density / newest; higher = denser / upgraded).
    #[serde(default = "default_level")]
    pub level: u8,
    /// Maximum number of households this building can house.
    #[serde(default)]
    pub household_capacity: Option<u32>,
    /// Maximum number of workers this building can employ.
    #[serde(default)]
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

    /// Building mesh parts. Each part owns its own LOD entries.
    #[serde(default)]
    pub mesh_parts: Vec<MeshPartParams>,
    /// Named anchor points (frontage, entrances, etc.).
    #[serde(default)]
    pub anchors: Vec<AnchorParams>,
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

fn default_part_scale() -> f32 {
    1.0
}

fn default_placement_mode() -> String {
    "zoned_private".to_owned()
}

// ── TOML generation ───────────────────────────────────────────────────────────

fn build_pack_toml(p: &ExportParams) -> String {
    let desc_line = format!("description = \"\"\n");
    format!(
        "pack_id = \"{}\"\nschema_version = {}\ndisplay_name = \"{}\"\nversion = \"{}\"\nauthor = \"{}\"\nlicense = \"{}\"\n{}",
        p.pack_id,
        CURRENT_SCHEMA_VERSION,
        p.pack_name,
        p.pack_version,
        p.pack_author,
        p.pack_license,
        desc_line
    )
}

fn build_asset_toml(p: &ExportParams) -> String {
    let mut out = String::new();

    out.push_str(&format!("asset_id = \"{}\"\n", p.asset_id));
    out.push_str(&format!("display_name = \"{}\"\n", p.display_name));

    if let Some(set) = &p.asset_set {
        if !set.is_empty() {
            out.push_str(&format!("asset_set = \"{set}\"\n"));
        }
    }

    if !p.tags.is_empty() {
        let tag_list = p
            .tags
            .iter()
            .map(|t| format!("\"{t}\""))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("tags = [{tag_list}]\n"));
    }

    out.push('\n');

    match p.asset_class.as_str() {
        "building" => {
            out.push_str("[building]\n");
            let placement_mode = normalized_placement_mode_key(&p.placement_mode);
            out.push_str(&format!("placement_mode = \"{placement_mode}\"\n"));
            if placement_mode == "zoned_private" {
                let zone = p.zone_type.as_deref().unwrap_or("residential");
                out.push_str(&format!("zone_type = \"{zone}\"\n"));
                let density = p.density.as_deref().unwrap_or("low");
                out.push_str(&format!("density = \"{density}\"\n"));
            }
            out.push_str(&format!("lot_width_cells = {}\n", p.lot_width_cells));
            out.push_str(&format!("lot_depth_cells = {}\n", p.lot_depth_cells));
            if let Some(min_width) = p.min_zone_width_cells {
                if min_width > 0 && min_width != p.lot_width_cells {
                    out.push_str(&format!("min_zone_width_cells = {min_width}\n"));
                }
            }
            if let Some(min_depth) = p.min_zone_depth_cells {
                if min_depth > 0 && min_depth != p.lot_depth_cells {
                    out.push_str(&format!("min_zone_depth_cells = {min_depth}\n"));
                }
            }
            out.push_str(&format!("level = {}\n", p.level));
            if let Some(h) = p.household_capacity {
                if h > 0 {
                    out.push_str(&format!("household_capacity = {h}\n"));
                }
            }
            if let Some(w) = p.worker_capacity {
                if w > 0 {
                    out.push_str(&format!("worker_capacity = {w}\n"));
                }
            }
            if let Some(f) = p.flat_size_m2 {
                if f > 0.0 {
                    out.push_str(&format!("flat_size_m2 = {f:.1}\n"));
                }
            }
            if let Some(sc) = &p.service_class {
                if !sc.is_empty() && sc != "none" {
                    out.push_str(&format!("service_class = \"{sc}\"\n"));
                }
            }
            if let Some(ep) = &p.economy_profile {
                if !ep.is_empty() {
                    out.push_str(&format!("economy_profile = \"{ep}\"\n"));
                }
            }
        }
        other => {
            // Future: prop, vehicle. Return an error-shaped string that the caller detects.
            out.push_str(&format!("# unsupported asset_class: {other}\n"));
        }
    }

    for part in &p.mesh_parts {
        out.push_str("\n[[mesh_parts]]\n");
        out.push_str(&format!("name = \"{}\"\n", part.name));
        let [x, y, z] = part.position;
        out.push_str(&format!("position = [{x}, {y}, {z}]\n"));
        let [rx, ry, rz] = part.rotation_degrees;
        out.push_str(&format!("rotation_degrees = [{rx}, {ry}, {rz}]\n"));
        out.push_str(&format!("scale = {}\n", part.scale));
        if let Some([px, py, pz]) = part.pivot_offset {
            if px.abs() > 1e-4 || py.abs() > 1e-4 || pz.abs() > 1e-4 {
                out.push_str(&format!("pivot_offset = [{px}, {py}, {pz}]\n"));
            }
        }
        for lod in &part.lods {
            out.push_str("\n[[mesh_parts.lods]]\n");
            out.push_str(&format!("file = \"{}\"\n", lod.file));
            out.push_str(&format!("distance_min_m = {}\n", lod.distance_min_m));
            if let Some(max) = lod.distance_max_m {
                out.push_str(&format!("distance_max_m = {max}\n"));
            }
        }
    }

    for anchor in &p.anchors {
        out.push_str("\n[[anchors]]\n");
        out.push_str(&format!("type = \"{}\"\n", anchor.anchor_type));
        out.push_str(&format!("name = \"{}\"\n", anchor.name));
        let [x, y, z] = anchor.position;
        out.push_str(&format!("position = [{x}, {y}, {z}]\n"));
        let [fx, fy, fz] = anchor.forward;
        out.push_str(&format!("forward = [{fx}, {fy}, {fz}]\n"));
    }

    out
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

fn normalized_placement_mode_key(value: &str) -> &'static str {
    match value.trim() {
        "explicit" => "explicit",
        _ => "zoned_private",
    }
}

fn parse_placement_mode(value: &str) -> Result<PlacementMode, String> {
    match normalized_placement_mode_key(value) {
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

fn validate_building_export_contract(params: &ExportParams) -> Result<(), String> {
    let placement_mode = parse_placement_mode(&params.placement_mode)?;
    let service_class = non_none_service_class(&params.service_class);
    let economy_profile = non_empty_optional_string(&params.economy_profile);
    if let Some(service_class) = service_class {
        validate_service_class(service_class)?;
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
            validate_against_builtin_zoning(params)?;
            match zone_type {
                "residential" => {
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
                    if params.worker_capacity.unwrap_or(0) == 0 {
                        return Err(
                            "commercial and industrial zoned_private buildings require worker_capacity"
                                .to_owned(),
                        );
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
        }
    }

    Ok(())
}

// ── Public helpers called from SimulationNode ─────────────────────────────────

/// Validates the JSON export params, writes `pack.toml` (if absent) and
/// `assets/<asset_id>/asset.toml`, and returns an error string or `""` on success.
pub fn validate_and_export_asset_internal(params_json: &str, output_dir: &str) -> String {
    let params: ExportParams = match serde_json::from_str(params_json) {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("JSON parse error: {e}");
            debug_log!("asset-editor", "{msg}");
            return msg;
        }
    };

    debug_log!(
        "asset-editor",
        "export requested: class={} asset_id={} pack={}",
        params.asset_class,
        params.asset_id,
        params.pack_id
    );

    if params.asset_class != "building" {
        let msg = format!(
            "unsupported asset_class '{}' (Step 5 handles buildings only)",
            params.asset_class
        );
        debug_log!("asset-editor", "{msg}");
        return msg;
    }

    if let Err(err) = validate_building_export_contract(&params) {
        debug_log!("asset-editor", "{err}");
        return err;
    }

    // Build and round-trip validate the asset TOML.
    let asset_toml = build_asset_toml(&params);
    debug_log!("asset-editor", "generated asset.toml:\n{asset_toml}");
    if let Err(e) = AssetManifest::from_str(&asset_toml) {
        let msg = format!("validation error: {e}\n\nGenerated TOML:\n{asset_toml}");
        debug_log!("asset-editor", "validation failed: {e}");
        return msg;
    }

    // Build and validate the pack TOML.
    let pack_toml = build_pack_toml(&params);
    if let Err(e) = PackManifest::from_str(&pack_toml) {
        let msg = format!("pack validation error: {e}");
        debug_log!("asset-editor", "{msg}");
        return msg;
    }

    // Write files.
    let out = Path::new(output_dir);
    if let Err(e) = std::fs::create_dir_all(out) {
        let msg = format!("could not create output dir: {e}");
        debug_log!("asset-editor", "{msg}");
        return msg;
    }

    // pack.toml — only written when absent (re-export must not clobber pack metadata).
    let pack_path = out.join("pack.toml");
    if !pack_path.exists() {
        if let Err(e) = std::fs::write(&pack_path, &pack_toml) {
            let msg = format!("could not write pack.toml: {e}");
            debug_log!("asset-editor", "{msg}");
            return msg;
        }
        debug_log!("asset-editor", "wrote pack.toml → {}", pack_path.display());
    } else {
        debug_log!("asset-editor", "pack.toml already exists — skipping");
    }

    // assets/<asset_id>/asset.toml
    let asset_dir = out.join("assets").join(&params.asset_id);
    if let Err(e) = std::fs::create_dir_all(&asset_dir) {
        let msg = format!("could not create asset dir: {e}");
        debug_log!("asset-editor", "{msg}");
        return msg;
    }
    let asset_toml_path = asset_dir.join("asset.toml");
    if let Err(e) = std::fs::write(&asset_toml_path, &asset_toml) {
        let msg = format!("could not write asset.toml: {e}");
        debug_log!("asset-editor", "{msg}");
        return msg;
    }

    debug_log!("asset-editor", "export OK → {}", asset_toml_path.display());
    String::new() // success
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
            "anchor_type": format!("{:?}", a.anchor_type).to_lowercase(),
            "name": a.name,
            "position": a.position,
            "forward": a.forward,
        })).collect::<Vec<_>>(),
    });

    if let Some(b) = &m.building {
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
        obj["min_zone_width_cells"] = serde_json::json!(b.min_zone_width_cells);
        obj["min_zone_depth_cells"] = serde_json::json!(b.min_zone_depth_cells);
        obj["level"] = serde_json::json!(b.level);
        obj["household_capacity"] = serde_json::json!(b.household_capacity);
        obj["flat_size_m2"] = serde_json::json!(b.flat_size_m2);
        obj["worker_capacity"] = serde_json::json!(b.worker_capacity);
        obj["service_class"] = serde_json::json!(b.service_class.as_deref().unwrap_or("none"));
        obj["economy_profile"] = serde_json::json!(b.economy_profile);
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
    fn export_writes_files_to_temp_dir() {
        let dir = std::env::temp_dir().join("metrum_export_test");
        let _ = std::fs::remove_dir_all(&dir);

        let result = validate_and_export_asset_internal(
            &minimal_building_json("building.residential.house"),
            dir.to_str().unwrap(),
        );
        assert!(result.is_empty(), "expected success, got: {result}");

        assert!(dir.join("pack.toml").exists());
        assert!(
            dir.join("assets")
                .join("building.residential.house")
                .join("asset.toml")
                .exists()
        );
    }

    #[test]
    fn export_does_not_overwrite_existing_pack_toml() {
        let dir = std::env::temp_dir().join("metrum_export_no_overwrite");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("pack.toml"), "# sentinel\n").unwrap();

        let result = validate_and_export_asset_internal(
            &minimal_building_json("building.residential.house2"),
            dir.to_str().unwrap(),
        );
        assert!(result.is_empty(), "expected success, got: {result}");

        let content = std::fs::read_to_string(dir.join("pack.toml")).unwrap();
        assert_eq!(content, "# sentinel\n", "pack.toml must not be overwritten");
    }

    #[test]
    fn export_rejects_invalid_asset_id() {
        let dir = std::env::temp_dir().join("metrum_export_invalid");
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

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(!result.is_empty(), "expected validation error");
    }

    #[test]
    fn export_rejects_zero_lot_cells() {
        let dir = std::env::temp_dir().join("metrum_export_zero_lot");
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

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(
            !result.is_empty(),
            "expected validation error for zero lot cells"
        );
    }

    #[test]
    fn export_writes_economy_profile_when_selected() {
        let dir = std::env::temp_dir().join("metrum_export_economy_profile");
        let _ = std::fs::remove_dir_all(&dir);

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

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(result.is_empty(), "expected success, got: {result}");

        let asset_toml = std::fs::read_to_string(
            dir.join("assets")
                .join("building.commercial.grocery_test")
                .join("asset.toml"),
        )
        .unwrap();
        assert!(asset_toml.contains("economy_profile = \"grocery_basic\""));
    }

    #[test]
    fn export_rejects_zoned_service_class() {
        let dir = std::env::temp_dir().join("metrum_export_zoned_service_class");
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

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(
            result.contains("zoned_private buildings must not export service_class"),
            "expected service-class validation error, got: {result}"
        );
    }

    #[test]
    fn export_rejects_utility_without_economy_profile() {
        let dir = std::env::temp_dir().join("metrum_export_utility_without_profile");
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

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(
            result.contains("explicit utility service buildings require economy_profile"),
            "expected utility profile validation error, got: {result}"
        );
    }

    #[test]
    fn export_rejects_invalid_service_class() {
        let dir = std::env::temp_dir().join("metrum_export_invalid_service_class");
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

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(
            result.contains("unsupported service_class 'sewage'"),
            "expected service-class enum validation error, got: {result}"
        );
    }

    #[test]
    fn export_rejects_utility_profile_for_wrong_service() {
        let dir = std::env::temp_dir().join("metrum_export_wrong_utility_profile");
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

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(
            result.contains("does not provide the 'power' service"),
            "expected utility service mismatch error, got: {result}"
        );
    }

    #[test]
    fn export_writes_explicit_utility_profile() {
        let dir = std::env::temp_dir().join("metrum_export_explicit_utility");
        let _ = std::fs::remove_dir_all(&dir);
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

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(result.is_empty(), "expected success, got: {result}");

        let asset_toml = std::fs::read_to_string(
            dir.join("assets")
                .join("building.power.plant_test")
                .join("asset.toml"),
        )
        .unwrap();
        assert!(asset_toml.contains("placement_mode = \"explicit\""));
        assert!(asset_toml.contains("service_class = \"power\""));
        assert!(asset_toml.contains("economy_profile = \"power_plant_basic\""));
    }
}
