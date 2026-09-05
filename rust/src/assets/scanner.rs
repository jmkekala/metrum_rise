// SPDX-License-Identifier: GPL-2.0-only

//! Pack directory scanner.
//!
//! Reads a directory of content packs from the native filesystem and produces
//! validated [`PackManifest`] + [`AssetManifest`] pairs for use with [`AssetRegistry`].
//! Only `std::fs` I/O is used — no Godot APIs — so this module is fully testable
//! without the engine.
//!
//! **Expected layout:**
//! ```text
//! <mods_dir>/
//!   <pack_name>/
//!     pack.toml          ← required
//!     buildings/
//!       residential/
//!         lowrise_corner/
//!           asset.toml   ← per-asset manifest
//!           lods/
//!             lod0.glb
//!     ...
//! ```
//!
//! Each immediate subdirectory of `mods_dir` is treated as one content pack.
//! `asset.toml` files are discovered recursively within the pack directory.
//! Broken packs (missing `pack.toml`, invalid TOML, validation failure) are skipped
//! with a diagnostic entry in [`ScanResult::warnings`].

use crate::assets::{AssetManifest, PackManifest};
use std::path::Path;

/// A successfully parsed content pack: one `pack.toml` and all its `asset.toml` files.
#[derive(Debug)]
pub struct ScannedPack {
    /// Parsed and validated pack manifest.
    pub pack: PackManifest,
    /// Successfully parsed assets paired with their native asset directory path.
    /// The directory is the folder containing `asset.toml` — used to resolve relative LOD paths.
    pub assets: Vec<(AssetManifest, String)>,
}

/// Output of [`scan_pack_dir`].
#[derive(Debug, Default)]
pub struct ScanResult {
    /// Successfully scanned packs in stable directory-entry order.
    pub packs: Vec<ScannedPack>,
    /// Human-readable diagnostic messages for every skipped pack or asset.
    pub warnings: Vec<String>,
}

/// Scans `dir` for content packs and returns the results.
///
/// Each immediate subdirectory of `dir` is treated as one pack. If `dir` does not exist
/// or cannot be read, an empty `ScanResult` with one warning is returned — callers must
/// not treat a missing mods directory as a fatal error.
pub fn scan_pack_dir(dir: &Path) -> ScanResult {
    let mut result = ScanResult::default();

    let read_dir = match std::fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) => {
            result.warnings.push(format!(
                "cannot read mods directory '{}': {e}",
                dir.display()
            ));
            return result;
        }
    };

    let mut pack_dirs: Vec<std::path::PathBuf> = read_dir
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    pack_dirs.sort(); // deterministic order

    for pack_dir in pack_dirs {
        let pack_toml_path = pack_dir.join("pack.toml");
        let pack_toml = match std::fs::read_to_string(&pack_toml_path) {
            Ok(s) => s,
            Err(e) => {
                result.warnings.push(format!(
                    "skipping '{}': cannot read pack.toml: {e}",
                    pack_dir.display()
                ));
                continue;
            }
        };

        let pack_manifest = match PackManifest::from_str(&pack_toml) {
            Ok(m) => m,
            Err(e) => {
                result.warnings.push(format!(
                    "skipping '{}': invalid pack.toml: {e}",
                    pack_dir.display()
                ));
                continue;
            }
        };

        let mut assets = Vec::new();
        collect_assets(&pack_dir, &mut assets, &mut result.warnings);

        result.packs.push(ScannedPack {
            pack: pack_manifest,
            assets,
        });
    }

    result
}

/// Recursively walks `dir` and parses every `asset.toml` found.
/// Successfully parsed manifests are appended to `out`; failures add to `warnings`.
fn collect_assets(dir: &Path, out: &mut Vec<(AssetManifest, String)>, warnings: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut paths: Vec<std::path::PathBuf> =
        entries.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        if path.is_dir() {
            collect_assets(&path, out, warnings);
        } else if path.file_name().and_then(|n| n.to_str()) == Some("asset.toml") {
            let asset_dir = path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            match std::fs::read_to_string(&path) {
                Ok(s) => match s.parse::<AssetManifest>() {
                    Ok(m) => out.push((m, asset_dir)),
                    Err(e) => warnings.push(format!("skipping '{}': {e}", path.display())),
                },
                Err(e) => warnings.push(format!("cannot read '{}': {e}", path.display())),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn write(path: &PathBuf, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    const VALID_PACK_TOML: &str = r#"
pack_id = "test-pack"
schema_version = 1
display_name = "Test Pack"
version = "1.0.0"
author = "Test"
license = "MIT"
"#;

    const BUILDING_TOML: &str = r#"
asset_id = "building.residential.house"
display_name = "House"

[[anchors]]
type = "entrance"
name = "main"
position = [0.0, 0.0, 0.5]
forward = [0.0, 0.0, 1.0]

[building]
placement_mode = "zoned_private"
zone_type = "residential"
density = "low"
lot_width_cells = 2
lot_depth_cells = 2
household_capacity = 6

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "lod0.glb"
distance_min_m = 0.0
"#;

    #[test]
    fn scan_nonexistent_dir_returns_warning() {
        let result = scan_pack_dir(Path::new("/tmp/metrum_test_nonexistent_dir_xyz"));
        assert!(result.packs.is_empty());
        assert_eq!(result.warnings.len(), 1);
    }

    #[test]
    fn scan_empty_dir_returns_no_packs() {
        let dir = std::env::temp_dir().join("metrum_scan_empty");
        fs::create_dir_all(&dir).unwrap();
        let result = scan_pack_dir(&dir);
        assert!(result.packs.is_empty());
        assert!(result.warnings.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn scan_valid_pack_with_one_asset() {
        let base = std::env::temp_dir().join("metrum_scan_valid");
        let pack = base.join("test-pack");

        write(&pack.join("pack.toml"), VALID_PACK_TOML);
        write(
            &pack.join("buildings/residential/house/asset.toml"),
            BUILDING_TOML,
        );

        let result = scan_pack_dir(&base);
        assert_eq!(
            result.warnings.len(),
            0,
            "unexpected warnings: {:?}",
            result.warnings
        );
        assert_eq!(result.packs.len(), 1);
        assert_eq!(result.packs[0].pack.pack_id, "test-pack");
        assert_eq!(result.packs[0].assets.len(), 1);
        assert_eq!(
            result.packs[0].assets[0].0.asset_id,
            "building.residential.house"
        );

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn scan_skips_pack_with_invalid_pack_toml() {
        let base = std::env::temp_dir().join("metrum_scan_bad_pack");
        let pack = base.join("bad-pack");

        write(&pack.join("pack.toml"), "this is not valid toml = [[[");
        write(&pack.join("buildings/house/asset.toml"), BUILDING_TOML);

        let result = scan_pack_dir(&base);
        assert!(result.packs.is_empty());
        assert_eq!(result.warnings.len(), 1);
        assert!(result.warnings[0].contains("bad-pack"));

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn scan_skips_broken_asset_but_keeps_pack() {
        let base = std::env::temp_dir().join("metrum_scan_bad_asset");
        let pack = base.join("test-pack");

        write(&pack.join("pack.toml"), VALID_PACK_TOML);
        write(&pack.join("buildings/house/asset.toml"), BUILDING_TOML);
        write(&pack.join("buildings/broken/asset.toml"), "not = valid [[[");

        let result = scan_pack_dir(&base);
        assert_eq!(result.packs.len(), 1, "pack should survive a broken asset");
        assert_eq!(result.packs[0].assets.len(), 1, "only the valid asset");
        assert_eq!(result.warnings.len(), 1, "one warning for the broken asset");

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn scan_multiple_packs_in_deterministic_order() {
        let base = std::env::temp_dir().join("metrum_scan_multi");

        for name in ["pack-b", "pack-a", "pack-c"] {
            let pack = base.join(name);
            let toml = VALID_PACK_TOML.replace("test-pack", name);
            write(&pack.join("pack.toml"), &toml);
        }

        let result = scan_pack_dir(&base);
        assert_eq!(result.warnings.len(), 0);
        assert_eq!(result.packs.len(), 3);
        // Directories are scanned in sorted order.
        assert_eq!(result.packs[0].pack.pack_id, "pack-a");
        assert_eq!(result.packs[1].pack.pack_id, "pack-b");
        assert_eq!(result.packs[2].pack.pack_id, "pack-c");

        fs::remove_dir_all(&base).unwrap();
    }

    /// Verifies the full pipeline for a pack laid out exactly like the kenney export:
    ///   `<mods>/<pack_id>/pack.toml`
    ///   `<mods>/<pack_id>/assets/<asset_id>/asset.toml`
    ///   `<mods>/<pack_id>/assets/<asset_id>/<lod_file>`
    ///
    /// Checks: scan finds the pack, assets register into the correct zone index,
    /// `lot_size` round-trips, and `asset_dir` resolves to the building mesh file.
    #[test]
    fn scan_and_register_kenney_style_pack() {
        use crate::assets::asset::ZoneClass;
        use crate::assets::registry::AssetRegistry;

        let base = std::env::temp_dir().join("metrum_scan_kenney_style");
        let pack_dir = base.join("kenney");

        let pack_toml = r#"
pack_id = "kenney"
schema_version = 1
display_name = "Kenney Assets"
version = "0.1.0"
author = "https://kenney.itch.io/"
license = "CC0"
"#;
        let house_toml = r#"
asset_id = "building.residential.house_a"
display_name = "House A"

[building]
zone_type = "residential"
lot_width_cells = 1
lot_depth_cells = 1
level = 1
household_capacity = 5
density = "low"

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "building-type-a.glb"
distance_min_m = 0
distance_max_m = 150

[[anchors]]
type = "entrance"
name = "main"
position = [0, 0, 5]
forward = [0, 0, -1]
"#;
        let shop_toml = r#"
asset_id = "building.commercial.shop_a"
display_name = "Shop A"

[building]
zone_type = "commercial"
lot_width_cells = 2
lot_depth_cells = 1
level = 1
worker_capacity = 8
density = "low"

[[mesh_parts]]
name = "main"

[[mesh_parts.lods]]
file = "shop-a.glb"
distance_min_m = 0
distance_max_m = 150

[[anchors]]
type = "entrance"
name = "main"
position = [0, 0, 5]
forward = [0, 0, -1]
"#;
        let house_dir = pack_dir.join("assets").join("building.residential.house_a");
        let shop_dir = pack_dir.join("assets").join("building.commercial.shop_a");

        write(&pack_dir.join("pack.toml"), pack_toml);
        write(&house_dir.join("asset.toml"), house_toml);
        write(&house_dir.join("building-type-a.glb"), "placeholder");
        write(&shop_dir.join("asset.toml"), shop_toml);
        write(&shop_dir.join("shop-a.glb"), "placeholder");

        // ── scan ────────────────────────────────────────────────────────────
        let result = scan_pack_dir(&base);
        assert!(
            result.warnings.is_empty(),
            "unexpected warnings: {:?}",
            result.warnings
        );
        assert_eq!(result.packs.len(), 1);
        let pack = &result.packs[0];
        assert_eq!(pack.pack.pack_id, "kenney");
        assert_eq!(pack.assets.len(), 2);

        // ── register ────────────────────────────────────────────────────────
        let mut registry = AssetRegistry::new();
        for (asset, asset_dir) in pack.assets.clone() {
            registry.register(&pack.pack.pack_id, asset, asset_dir);
        }

        // Zone indices populated correctly.
        let res = registry.buildings_for_zone(ZoneClass::Residential);
        assert_eq!(res, &["kenney:building.residential.house_a"]);
        let com = registry.buildings_for_zone(ZoneClass::Commercial);
        assert_eq!(com, &["kenney:building.commercial.shop_a"]);

        // Lot sizes round-trip.
        assert_eq!(
            registry.lot_size("kenney:building.residential.house_a"),
            (1, 1)
        );
        assert_eq!(
            registry.lot_size("kenney:building.commercial.shop_a"),
            (2, 1)
        );

        // asset_dir + part LOD file resolves to the file we created on disk.
        let entry = registry.get("kenney:building.residential.house_a").unwrap();
        let lod_path =
            std::path::Path::new(&entry.asset_dir).join(&entry.manifest.mesh_parts[0].lods[0].file);
        assert!(lod_path.exists(), "LOD file not found at {:?}", lod_path);

        // density field round-trips.
        let density = registry
            .get("kenney:building.residential.house_a")
            .and_then(|e| e.manifest.building.as_ref())
            .and_then(|b| b.density_key());
        assert_eq!(density, Some("low"));

        fs::remove_dir_all(&base).unwrap();
    }
}
