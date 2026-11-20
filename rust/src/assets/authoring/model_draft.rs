// SPDX-License-Identifier: GPL-3.0-or-later

//! The model draft pipeline's engine-independent half (`TOOLS-33`, docs/asset_editor.md
//! "Model Draft Pipeline"): read a facts sidecar and role inputs, derive the editor document
//! a draft publishes, and report instead when the asset already exists. Publication itself
//! is the editor's (`AssetAuthoringFiles`); nothing here writes into an asset folder.
//! Runs once per draft command, never in a simulation or rendering path.

pub(crate) mod acceptance;
pub(crate) mod report;
pub(crate) mod rules;
pub(crate) mod sidecar;

#[cfg(test)]
pub(crate) mod tests;

use crate::assets::PackManifest;
use crate::assets::authoring::{colours, files};
use serde_json::{Value, json};
use sidecar::{Facts, Role};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A read and checked sidecar with its role inputs, as the draft command received them.
pub(crate) struct Inputs {
    pub(crate) facts: Facts,
    /// The sidecar and role as read, copied verbatim into the ledger.
    pub(crate) sidecar: Value,
    pub(crate) role: Role,
    pub(crate) role_value: Value,
    /// The sidecar's folder, which every file path in it is relative to.
    pub(crate) export: PathBuf,
}

/// Read the preconditions that need no pack: sidecar schema and version, every listed hash,
/// a clean source, and well-formed role inputs. Each failure is a refusal.
pub(crate) fn read_inputs(sidecar_path: &Path, role: Value) -> Result<Inputs, String> {
    let text = std::fs::read_to_string(sidecar_path)
        .map_err(|e| format!("cannot read sidecar {}: {e}", sidecar_path.display()))?;
    let sidecar: Value =
        serde_json::from_str(&text).map_err(|e| format!("sidecar is not JSON: {e}"))?;
    let facts = Facts::from_value(sidecar.clone())?;
    if facts.source.dirty {
        return Err(format!(
            "the export of {} was built from a dirty models tree; commit and rebuild it",
            facts.name
        ));
    }
    let export = sidecar_path
        .parent()
        .ok_or("the sidecar has no folder")?
        .to_owned();
    facts.verify_files(&export)?;
    Ok(Inputs {
        facts,
        sidecar,
        role: Role::parse(role.clone())?,
        role_value: role,
        export,
    })
}

/// A draft derived for one pack, not yet published.
pub(crate) struct Derived {
    pub(crate) pack: PathBuf,
    /// The editor document (`params`, `sources`, `colour_sources`, `thumbnail_source`) that
    /// `AssetAuthoringFiles.publish_document` takes.
    pub(crate) document: Value,
    /// The derived values and the free space around the building, for the report.
    pub(crate) summary: Value,
    /// R8's source and destination: the models preview, the thumbnail to write.
    pub(crate) preview: PathBuf,
    pub(crate) thumbnail: PathBuf,
}

/// Derive the draft document for the role's pack in `mods`, the thumbnail going to `work`.
pub(crate) fn derive(inputs: &Inputs, mods: &Path, work: &Path) -> Result<Derived, String> {
    let (facts, role) = (&inputs.facts, &inputs.role);
    let pack = files::pack_directory(mods, &role.pack_id)?;
    let manifest = std::fs::read_to_string(pack.join("pack.toml"))
        .map_err(|e| e.to_string())
        .and_then(|text| PackManifest::from_str(&text).map_err(|e| e.to_string()))
        .map_err(|e| format!("pack.toml of {}: {e}", role.pack_id))?;
    let identity = rules::PackIdentity {
        name: &manifest.display_name,
        author: &manifest.author,
        version: &manifest.version,
        license: &manifest.license,
    };
    let materials = tier_materials(facts, &inputs.export)?;
    let params = rules::params(facts, role, &identity, &materials)?;
    let thumbnail = work.join(rules::THUMBNAIL_FILE);
    let sources: Vec<String> = facts
        .lods
        .iter()
        .map(|lod| inputs.export.join(&lod.file).to_string_lossy().into_owned())
        .collect();
    let colour_sources: BTreeMap<String, String> = facts
        .variants
        .iter()
        .map(|variant| {
            let path = inputs.export.join(&variant.albedo);
            (
                rules::colour_path(&variant.albedo),
                path.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let part = &params["mesh_parts"][0];
    let position: [f64; 3] =
        serde_json::from_value(part["position"].clone()).map_err(|e| e.to_string())?;
    let lot = [
        params["lot_width_cells"].as_u64().unwrap_or(0) as u16,
        params["lot_depth_cells"].as_u64().unwrap_or(0) as u16,
    ];
    let summary = json!({
        "lot_cells": lot,
        "part_position": position,
        "entrance": params["anchors"][0],
        "household_capacity": params["household_capacity"],
        "flat_size_m2": params["flat_size_m2"],
        "worker_capacity": params["worker_capacity"],
        "economy_profile": params["economy_profile"],
        "schemes": facts.variants.len(),
        "free_space": rules::free_space(facts, lot, position),
    });
    let document = json!({
        "params": params,
        "sources": [sources],
        "colour_sources": colour_sources,
        "thumbnail_source": thumbnail.to_string_lossy(),
        "origin": {},
    });
    Ok(Derived {
        pack,
        document,
        summary,
        preview: inputs.export.join(&facts.preview.file),
        thumbnail,
    })
}

// R7: the one material of each tier, read from the tier's `.glb`; any other count stops it.
fn tier_materials(facts: &Facts, export: &Path) -> Result<Vec<String>, String> {
    facts
        .lods
        .iter()
        .map(|lod| {
            let materials = colours::materials(&export.join(&lod.file))?;
            match &materials[..] {
                [material] if !material.name.trim().is_empty() => Ok(material.name.clone()),
                _ => Err(format!(
                    "{} has {} materials; a draft needs exactly one named material per tier",
                    lod.file,
                    materials.len()
                )),
            }
        })
        .collect()
}

/// The no-overwrite rule: `None` when `assets/<asset_id>` is free, else the stale-geometry
/// report against what is there. Anything at that path (file, folder or link) counts.
pub(crate) fn existing(pack: &Path, inputs: &Inputs) -> Option<Vec<report::Line>> {
    let asset_id = &inputs.role.asset_id;
    let target = pack.join("assets").join(asset_id);
    if !target.exists() && !target.is_symlink() {
        return None;
    }
    let unreadable = |message: String| {
        vec![report::Line {
            check: "files",
            field: "asset_id",
            message,
        }]
    };
    let mods = pack.parent().unwrap_or(Path::new("."));
    let pack_id = &inputs.role.pack_id;
    let directory = match files::asset_directory(mods, pack_id, asset_id) {
        Ok(directory) => directory,
        Err(error) => return Some(unreadable(format!("assets/{asset_id} exists: {error}"))),
    };
    let manifest = match std::fs::read_to_string(directory.join("asset.toml"))
        .map_err(|e| e.to_string())
        .and_then(|text| crate::assets::AssetManifest::from_str(&text).map_err(|e| e.to_string()))
    {
        Ok(manifest) => manifest,
        Err(error) => return Some(unreadable(format!("assets/{asset_id}/asset.toml: {error}"))),
    };
    let published = match published_digests(&manifest, &directory) {
        Ok(published) => published,
        Err(error) => return Some(unreadable(format!("assets/{asset_id}: {error}"))),
    };
    let mut lines = Vec::new();
    let ledger = sidecar::read_ledger(pack, asset_id).unwrap_or_else(|error| {
        lines.push(report::Line {
            check: "facts",
            field: "asset_id",
            message: error,
        });
        None
    });
    lines.extend(report::stale_report(
        &inputs.facts,
        ledger.as_ref(),
        &manifest,
        &published,
    ));
    Some(lines)
}

// SHA-256 of every published tier and texture; the thumbnail and credits are no model file.
fn published_digests(
    manifest: &crate::assets::AssetManifest,
    directory: &Path,
) -> Result<BTreeMap<String, String>, String> {
    let mut digests = BTreeMap::new();
    for (relative, source) in crate::assets::archive::asset_files(manifest, directory)? {
        let relative = relative.to_string_lossy().replace('\\', "/");
        if manifest.thumbnail.as_deref() == Some(relative.as_str())
            || relative.starts_with("attribution/")
        {
            continue;
        }
        digests.insert(relative, crate::assets::archive::sha256(&source)?);
    }
    Ok(digests)
}
