//! Asset export helpers: validate form data and write `pack.toml` / `asset.toml` to disk.
//!
//! GDScript sends a JSON string describing the form state. Rust validates it, generates
//! well-formed TOML, round-trips it through [`AssetManifest::from_str`] for final
//! validation, and writes the output files. Pack TOML is only written when the file does
//! not already exist, so re-exporting individual assets never overwrites pack metadata.

use crate::assets::asset::PlacementMode;
use crate::assets::{AssetManifest, CURRENT_SCHEMA_VERSION, PackManifest};
use crate::debug_log;
use crate::simulation::grid::zoning::load_builtin_profile_registry;
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
    /// Maximum number of residents this building can house.
    #[serde(default)]
    pub residents_capacity: Option<u32>,
    /// Maximum number of workers this building can employ.
    #[serde(default)]
    pub worker_capacity: Option<u32>,
    /// Service class tag for civic buildings (e.g. `"fire_station"`).
    #[serde(default)]
    pub service_class: Option<String>,
    /// Reference to an authored economy profile selected from the current economy catalog.
    #[serde(default)]
    pub economy_profile: Option<String>,
    /// Uniform scale applied in the asset preview viewport.
    #[serde(default)]
    pub preview_scale: Option<f32>,
    /// Pivot offset applied when placing the asset in world space.
    #[serde(default)]
    pub pivot_offset: Option<[f32; 3]>,

    /// LOD entries ordered from highest to lowest detail.
    #[serde(default)]
    pub lods: Vec<LodParams>,
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
            if let Some(r) = p.residents_capacity {
                if r > 0 {
                    out.push_str(&format!("residents_capacity = {r}\n"));
                }
            }
            if let Some(w) = p.worker_capacity {
                if w > 0 {
                    out.push_str(&format!("worker_capacity = {w}\n"));
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
            if let Some(ps) = p.preview_scale {
                if (ps - 1.0).abs() > 0.001 {
                    out.push_str(&format!("preview_scale = {ps}\n"));
                }
            }
            if let Some([px, py, pz]) = p.pivot_offset {
                if px.abs() > 1e-4 || py.abs() > 1e-4 || pz.abs() > 1e-4 {
                    out.push_str(&format!("pivot_offset = [{px}, {py}, {pz}]\n"));
                }
            }
        }
        other => {
            // Future: prop, vehicle. Return an error-shaped string that the caller detects.
            out.push_str(&format!("# unsupported asset_class: {other}\n"));
        }
    }

    for lod in &p.lods {
        out.push_str("\n[[lods]]\n");
        out.push_str(&format!("file = \"{}\"\n", lod.file));
        out.push_str(&format!("distance_min_m = {}\n", lod.distance_min_m));
        if let Some(max) = lod.distance_max_m {
            out.push_str(&format!("distance_max_m = {max}\n"));
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
            "unsupported zoned building legality '{} + {}' for baseline shipped zoning profiles",
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

fn validate_building_export_contract(params: &ExportParams) -> Result<(), String> {
    let placement_mode = parse_placement_mode(&params.placement_mode)?;
    if params.lot_width_cells == 0 || params.lot_depth_cells == 0 {
        return Err("lot_width_cells and lot_depth_cells must be > 0".to_owned());
    }
    if params.min_zone_width_cells == Some(0) || params.min_zone_depth_cells == Some(0) {
        return Err("min_zone_width_cells and min_zone_depth_cells must be > 0".to_owned());
    }

    match placement_mode {
        PlacementMode::ZonedPrivate => {
            let zone_type = non_empty_optional_string(&params.zone_type)
                .ok_or_else(|| "zoned_private buildings require zone_type".to_owned())?;
            let _density = non_empty_optional_string(&params.density)
                .ok_or_else(|| "zoned_private buildings require density".to_owned())?;
            validate_against_builtin_zoning(params)?;
            match zone_type {
                "residential" => {
                    if params.residents_capacity.unwrap_or(0) == 0 {
                        return Err(
                            "residential zoned_private buildings require residents_capacity"
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
        "lods": m.lods.iter().map(|l| serde_json::json!({
            "file": l.file,
            "distance_min_m": l.distance_min_m,
            "distance_max_m": l.distance_max_m,
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
        obj["residents_capacity"] = serde_json::json!(b.residents_capacity);
        obj["worker_capacity"] = serde_json::json!(b.worker_capacity);
        obj["service_class"] = serde_json::json!(b.service_class.as_deref().unwrap_or("none"));
        obj["economy_profile"] = serde_json::json!(b.economy_profile);
        obj["preview_scale"] = serde_json::json!(b.preview_scale.unwrap_or(1.0));
    }
    if let Some(po) = m.pivot_offset {
        obj["pivot_offset"] = serde_json::json!(po);
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
            "residents_capacity": 6,
            "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}],
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
            "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}]
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
            "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}]
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
            "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}],
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
}
