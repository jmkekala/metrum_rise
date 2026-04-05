//! Asset export helpers: validate form data and write `pack.toml` / `asset.toml` to disk.
//!
//! GDScript sends a JSON string describing the form state. Rust validates it, generates
//! well-formed TOML, round-trips it through [`AssetManifest::from_str`] for final
//! validation, and writes the output files. Pack TOML is only written when the file does
//! not already exist, so re-exporting individual assets never overwrites pack metadata.

use std::path::Path;
use serde::Deserialize;
use crate::assets::{AssetManifest, PackManifest, CURRENT_SCHEMA_VERSION};
use crate::debug_log;

// ── Input structs (JSON from GDScript) ───────────────────────────────────────

#[derive(Deserialize)]
pub struct LodParams {
    pub file: String,
    pub distance_min_m: f32,
    #[serde(default)]
    pub distance_max_m: Option<f32>,
}

#[derive(Deserialize)]
pub struct AnchorParams {
    pub anchor_type: String,
    pub name: String,
    pub position: [f32; 3],
    pub forward: [f32; 3],
}

/// Flat JSON payload sent by the building importer form.
#[derive(Deserialize)]
pub struct ExportParams {
    // Pack
    pub pack_id: String,
    pub pack_name: String,
    pub pack_author: String,
    #[serde(default = "default_version")]
    pub pack_version: String,
    #[serde(default = "default_license")]
    pub pack_license: String,

    // Asset manifest
    pub asset_class: String,  // "building" only for Step 5; extended in Step 6
    pub asset_id: String,
    pub display_name: String,
    #[serde(default)]
    pub asset_set: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,

    // Building-specific
    #[serde(default)]
    pub zone_type: Option<String>,
    #[serde(default)]
    pub density: Option<String>,
    #[serde(default)]
    pub lot_width_cells: u16,
    #[serde(default)]
    pub lot_depth_cells: u16,
    #[serde(default = "default_level")]
    pub level: u8,
    #[serde(default)]
    pub residents_capacity: Option<u32>,
    #[serde(default)]
    pub worker_capacity: Option<u32>,
    #[serde(default)]
    pub service_class: Option<String>,
    #[serde(default)]
    pub preview_scale: Option<f32>,

    // LODs and anchors
    #[serde(default)]
    pub lods: Vec<LodParams>,
    #[serde(default)]
    pub anchors: Vec<AnchorParams>,
}

fn default_version() -> String { "0.1.0".to_owned() }
fn default_license() -> String { "CC0".to_owned() }
fn default_level() -> u8 { 1 }

// ── TOML generation ───────────────────────────────────────────────────────────

fn build_pack_toml(p: &ExportParams) -> String {
    let desc_line = format!("description = \"\"\n");
    format!(
        "pack_id = \"{}\"\nschema_version = {}\ndisplay_name = \"{}\"\nversion = \"{}\"\nauthor = \"{}\"\nlicense = \"{}\"\n{}",
        p.pack_id, CURRENT_SCHEMA_VERSION, p.pack_name, p.pack_version, p.pack_author, p.pack_license, desc_line
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
        let tag_list = p.tags.iter().map(|t| format!("\"{t}\"")).collect::<Vec<_>>().join(", ");
        out.push_str(&format!("tags = [{tag_list}]\n"));
    }

    out.push('\n');

    match p.asset_class.as_str() {
        "building" => {
            out.push_str("[building]\n");
            let zone = p.zone_type.as_deref().unwrap_or("residential");
            out.push_str(&format!("zone_type = \"{zone}\"\n"));
            let density = p.density.as_deref().unwrap_or("low");
            out.push_str(&format!("density = \"{density}\"\n"));
            out.push_str(&format!("lot_width_cells = {}\n", p.lot_width_cells));
            out.push_str(&format!("lot_depth_cells = {}\n", p.lot_depth_cells));
            out.push_str(&format!("level = {}\n", p.level));
            if let Some(r) = p.residents_capacity {
                if r > 0 { out.push_str(&format!("residents_capacity = {r}\n")); }
            }
            if let Some(w) = p.worker_capacity {
                if w > 0 { out.push_str(&format!("worker_capacity = {w}\n")); }
            }
            if let Some(sc) = &p.service_class {
                if !sc.is_empty() { out.push_str(&format!("service_class = \"{sc}\"\n")); }
            }
            if let Some(ps) = p.preview_scale {
                if (ps - 1.0).abs() > 0.001 { out.push_str(&format!("preview_scale = {ps}\n")); }
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

    debug_log!("asset-editor", "export requested: class={} asset_id={} pack={}", params.asset_class, params.asset_id, params.pack_id);

    if params.asset_class != "building" {
        let msg = format!("unsupported asset_class '{}' (Step 5 handles buildings only)", params.asset_class);
        debug_log!("asset-editor", "{msg}");
        return msg;
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
        debug_log!("asset-editor", "get_asset_manifest_json: '{}' not found", qualified_id);
        return String::new();
    };
    debug_log!("asset-editor", "get_asset_manifest_json: loading '{}'", qualified_id);
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
        obj["zone_type"] = serde_json::json!(format!("{:?}", b.zone_type).to_lowercase());
        obj["density"] = serde_json::json!(&b.density);
        obj["lot_width_cells"] = serde_json::json!(b.lot_width_cells);
        obj["lot_depth_cells"] = serde_json::json!(b.lot_depth_cells);
        obj["level"] = serde_json::json!(b.level);
        obj["residents_capacity"] = serde_json::json!(b.residents_capacity);
        obj["worker_capacity"] = serde_json::json!(b.worker_capacity);
        obj["service_class"] = serde_json::json!(b.service_class);
        obj["preview_scale"] = serde_json::json!(b.preview_scale.unwrap_or(1.0));
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
            debug_log!("asset-editor", "get_pack_manifest_json: no pack.toml at {}", path.display());
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
            "zone_type": "residential",
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
        }).to_string()
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
        assert!(dir.join("assets").join("building.residential.house").join("asset.toml").exists());
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
            "zone_type": "residential",
            "lot_width_cells": 2,
            "lot_depth_cells": 2,
            "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}]
        }).to_string();

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
            "zone_type": "residential",
            "lot_width_cells": 0,
            "lot_depth_cells": 2,
            "lods": [{"file": "lod0.glb", "distance_min_m": 0.0}]
        }).to_string();

        let result = validate_and_export_asset_internal(&json, dir.to_str().unwrap());
        assert!(!result.is_empty(), "expected validation error for zero lot cells");
    }
}
