// SPDX-License-Identifier: GPL-3.0-or-later

//! The acceptance check of one drafted asset (docs/asset_editor.md "Acceptance check"):
//! existing validation reused, plus the assertions the pipeline adds. Read-only. The engine
//! supplies what only Godot can do (mesh bounds import, image decoding) as callbacks, so the
//! check runs without the engine in tests. O(pack files + asset tiers), once per draft.

use super::report::{ENTRANCE_TOLERANCE_M, door_offset};
use super::rules::LOT_FIT_TOLERANCE_M;
use super::sidecar::read_ledger;
use crate::assets::asset::geometry::fits_lot;
use crate::assets::asset::validation::ZONE_CELL_M;
use crate::assets::authoring::{colours, files, lod_chain_error};
use crate::assets::{AnchorType, AssetManifest, AssetRegistry, scan_pack_dir};
use serde::Serialize;
use std::path::Path;

/// A zoned building drafts four tiers (docs/asset_editor.md "Buildings").
const ZONED_TIERS: usize = 4;
/// Triangle ceilings per tier, ordinary and hero/landmark (docs/asset_editor.md "Buildings").
const TRIANGLE_CEILINGS: [u64; 4] = [12_000, 4_000, 1_500, 500];
const HERO_TRIANGLE_CEILINGS: [u64; 4] = [20_000, 6_000, 2_500, 700];
/// Pedestrian door sanity bounds, metres: clear height, clear width, threshold height.
const DOOR_HEIGHT_M: (f64, f64) = (2.0, 2.6);
const DOOR_WIDTH_M: (f64, f64) = (0.7, 6.0);
const DOOR_THRESHOLD_M: (f64, f64) = (0.0, 1.5);

/// What the engine does for the check: import a manifest's `LOD0` bounds into its parts
/// (`import_building_bounds`), and decode one image file.
pub(crate) struct Engine<'a> {
    pub(crate) bounds: &'a dyn Fn(&mut AssetManifest, &Path) -> Result<(), String>,
    pub(crate) decodes: &'a dyn Fn(&Path) -> bool,
}

/// Failures block a `--write`; warnings are reported only.
#[derive(Debug, Default, Serialize)]
pub(crate) struct Outcome {
    pub(crate) failures: Vec<String>,
    pub(crate) warnings: Vec<String>,
}

impl Outcome {
    pub(crate) fn passed(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Check asset `asset_id` of pack `pack_id` in the packs folder `mods`.
pub(crate) fn check(mods: &Path, pack_id: &str, asset_id: &str, engine: &Engine) -> Outcome {
    let mut outcome = Outcome::default();
    let pack = mods.join(pack_id);
    let scan = scan_pack_dir(mods);
    let folder = pack.join("assets").join(asset_id);
    let folder_text = folder.to_string_lossy();
    for warning in &scan.warnings {
        if warning.contains(asset_id) || warning.contains(folder_text.as_ref()) {
            outcome.failures.push(format!("pack scan: {warning}"));
        }
    }
    match crate::assets::archive::inventory(&pack) {
        Ok(inventory) => {
            let prefix = format!("assets/{asset_id}/");
            for path in inventory.excluded.iter().filter(|p| p.starts_with(&prefix)) {
                outcome
                    .failures
                    .push(format!("pack export would leave out {path}"));
            }
        }
        Err(error) => outcome.failures.push(format!("pack export: {error}")),
    }
    let mut registry = AssetRegistry::new();
    let mut found = None;
    for scanned in scan.packs {
        for (manifest, directory) in scanned.assets {
            if scanned.pack.pack_id == pack_id && manifest.asset_id == asset_id {
                found = Some((manifest.clone(), directory.clone()));
            }
            registry.register(&scanned.pack.pack_id, manifest, directory);
        }
    }
    let Some((mut manifest, directory)) = found else {
        outcome
            .failures
            .push(format!("the scan registers no asset {pack_id}:{asset_id}"));
        return outcome;
    };
    let directory = Path::new(&directory);
    check_lods(&manifest, &mut outcome);
    match (engine.bounds)(&mut manifest, directory) {
        Ok(()) => check_fit(&manifest, &mut outcome),
        Err(error) => outcome.failures.push(format!("bounds import: {error}")),
    }
    check_colours(&manifest, directory, engine, &mut outcome);
    check_stock(&manifest, pack_id, &registry, &mut outcome);
    check_ledger(&manifest, &pack, &mut outcome);
    if manifest
        .building
        .as_ref()
        .is_some_and(|building| building.worker_capacity.is_some_and(|w| w > 0))
        && !manifest
            .anchors
            .iter()
            .any(|anchor| anchor.anchor_type == AnchorType::Parking)
    {
        outcome
            .warnings
            .push("a workplace without a parking anchor: add parking in the editor (Q3)".into());
    }
    outcome
}

fn check_lods(manifest: &AssetManifest, outcome: &mut Outcome) {
    for part in &manifest.mesh_parts {
        if let Err(error) = lod_chain_error(&part.lods) {
            outcome
                .failures
                .push(format!("part '{}': {error}", part.name));
        }
        if part.lods.len() != ZONED_TIERS {
            outcome.failures.push(format!(
                "part '{}' has {} LOD tiers; a zoned building has {ZONED_TIERS}",
                part.name,
                part.lods.len()
            ));
        }
    }
}

fn check_fit(manifest: &AssetManifest, outcome: &mut Outcome) {
    let Some(building) = &manifest.building else {
        return;
    };
    let half = [building.lot_width_cells, building.lot_depth_cells]
        .map(|cells| f32::from(cells) * ZONE_CELL_M * 0.5);
    for part in &manifest.mesh_parts {
        let Some(bounds) = part.imported_bounds else {
            continue;
        };
        if !fits_lot(part.plan_bounds(bounds), half, LOT_FIT_TOLERANCE_M as f32) {
            outcome
                .failures
                .push(format!("part '{}' crosses the lot bounds", part.name));
        }
    }
}

// Every scheme texture decodes and every scheme names a material each tier has.
fn check_colours(
    manifest: &AssetManifest,
    directory: &Path,
    engine: &Engine,
    outcome: &mut Outcome,
) {
    let Some(appearance) = manifest
        .building
        .as_ref()
        .and_then(|building| building.appearance.as_ref())
    else {
        return;
    };
    if let Err(error) = appearance.validate(&manifest.mesh_parts) {
        outcome.failures.push(format!("colour schemes: {error}"));
        return;
    }
    for scheme in &appearance.schemes {
        for entry in &scheme.overrides {
            let Some(part) = manifest.mesh_parts.iter().find(|p| p.name == entry.part) else {
                continue;
            };
            for (tier, (lod, name)) in part.lods.iter().zip(&entry.materials).enumerate() {
                let result = colours::materials(&directory.join(&lod.file))
                    .and_then(|materials| colours::resolve(name, &materials).map(|_| ()));
                if let Err(error) = result {
                    outcome
                        .failures
                        .push(format!("scheme '{}' LOD{tier}: {error}", scheme.id));
                }
            }
            for texture in entry.textures() {
                if !(engine.decodes)(&directory.join(texture)) {
                    outcome
                        .failures
                        .push(format!("scheme '{}': {texture} does not decode", scheme.id));
                }
            }
        }
    }
}

// A level-1 asset is a spawn candidate of its zoning profile (the allocator's own filter, run
// on a registry of this asset alone); a higher level has a level below it in `registry`.
fn check_stock(
    manifest: &AssetManifest,
    pack_id: &str,
    registry: &AssetRegistry,
    outcome: &mut Outcome,
) {
    let Some(building) = &manifest.building else {
        return;
    };
    if building.level == 1 {
        let profiles = crate::simulation::zoning::load_builtin_profile_registry();
        let catalog = crate::simulation::economy::definitions::load_runtime_economy_catalog();
        let (profiles, catalog) = match (profiles, catalog) {
            (Ok(profiles), Ok(catalog)) => (profiles, catalog),
            (Err(error), _) | (_, Err(error)) => {
                outcome.failures.push(format!("spawn check: {error}"));
                return;
            }
        };
        let mut allocator = crate::simulation::buildings::allocator::BuildingAllocator::new();
        allocator
            .registry
            .register(pack_id, manifest.clone(), String::new());
        if allocator
            .stocked_zone_profiles(&profiles, &catalog)
            .is_empty()
        {
            outcome.failures.push(
                "the asset is no spawn candidate of any zoning profile (a workplace needs a runtime economy_profile)"
                    .into(),
            );
        }
    } else if registry
        .prev_level(&manifest.qualified_id(pack_id), 0)
        .is_none()
    {
        outcome.failures.push(format!(
            "level {} has no level {} asset of the same zone, density and lot cells (or asset_set)",
            building.level,
            building.level - 1
        ));
    }
}

// Door sanity, the entrance on the main door, and triangle ceilings, from the ledger sidecar.
fn check_ledger(manifest: &AssetManifest, pack: &Path, outcome: &mut Outcome) {
    let facts = match read_ledger(pack, &manifest.asset_id) {
        Ok(Some(facts)) => facts,
        Ok(None) => {
            outcome
                .failures
                .push("the asset has no ledger entry in model_sources/".into());
            return;
        }
        Err(error) => {
            outcome.failures.push(error);
            return;
        }
    };
    // Pedestrian doors only: vehicle, balcony and store doors have other sizes and heights.
    for door in facts.doors.iter().filter(|door| door.kind.pedestrian()) {
        for (name, value, (low, high)) in [
            ("height_m", door.height_m, DOOR_HEIGHT_M),
            ("width_m", door.width_m, DOOR_WIDTH_M),
            ("threshold y", door.position[1], DOOR_THRESHOLD_M),
        ] {
            if !(low..=high).contains(&value) {
                outcome.failures.push(format!(
                    "door '{}' {name} {value} is outside {low}..{high} m",
                    door.id
                ));
            }
        }
    }
    let door = facts.main_door();
    let part = manifest.mesh_parts.iter().find(|part| {
        part.lods
            .first()
            .is_some_and(|lod| lod.file == facts.lods[0].file)
    });
    let entrance = manifest
        .anchors
        .iter()
        .find(|a| a.anchor_type == AnchorType::Entrance && a.name == "main");
    match (part, entrance) {
        (Some(part), Some(anchor)) => {
            let (distance, _) = door_offset(part, door, anchor.position, anchor.forward);
            if distance > ENTRANCE_TOLERANCE_M {
                outcome.failures.push(format!(
                    "entrance/main stands {distance:.2} m from the main door in plan"
                ));
            }
        }
        _ => outcome
            .failures
            .push("no mesh part shows the ledger's LOD0 file, or no entrance/main".into()),
    }
    let ceilings = if facts.hero {
        HERO_TRIANGLE_CEILINGS
    } else {
        TRIANGLE_CEILINGS
    };
    if let Some(part) = part {
        let directory = pack.join("assets").join(&manifest.asset_id);
        for (tier, (lod, ceiling)) in part.lods.iter().zip(ceilings).enumerate() {
            match glb_triangles(&directory.join(&lod.file)) {
                Ok(count) if count > ceiling => outcome.warnings.push(format!(
                    "LOD{tier} has {count} triangles, above its ceiling {ceiling}"
                )),
                Ok(_) => {}
                Err(error) => outcome.warnings.push(format!("LOD{tier}: {error}")),
            }
        }
    }
}

/// Triangles one `.glb` draws: each node's mesh primitives, from the index count (the vertex
/// count without indices) of triangle-list primitives. Reads the JSON chunk only.
pub(crate) fn glb_triangles(path: &Path) -> Result<u64, String> {
    let gltf = files::read_gltf(path)?;
    let count = |accessor: &serde_json::Value| {
        accessor
            .as_u64()
            .and_then(|index| gltf["accessors"][index as usize]["count"].as_u64())
            .ok_or("a primitive names no accessor with a count")
    };
    let mut triangles = 0;
    for node in gltf["nodes"].as_array().into_iter().flatten() {
        let Some(mesh) = node["mesh"].as_u64() else {
            continue;
        };
        for primitive in gltf["meshes"][mesh as usize]["primitives"]
            .as_array()
            .into_iter()
            .flatten()
        {
            // Mode 4 (triangles) is the glTF default; other modes draw no triangle list.
            if primitive["mode"].as_u64().unwrap_or(4) != 4 {
                continue;
            }
            let indices = if primitive["indices"].is_null() {
                count(&primitive["attributes"]["POSITION"])?
            } else {
                count(&primitive["indices"])?
            };
            triangles += indices / 3;
        }
    }
    Ok(triangles)
}
