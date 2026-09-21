// SPDX-License-Identifier: GPL-2.0-only

//! Read-only colour discovery and exact material resolution, outside rendering hot paths.

use super::files;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub(crate) fn exported_sources(
    params: &serde_json::Value,
    root: &Path,
) -> Result<serde_json::Value, String> {
    use crate::assets::asset::BuildingAppearance;
    let mut paths = serde_json::Map::new();
    if !params["appearance"].is_null() {
        let appearance: BuildingAppearance =
            serde_json::from_value(params["appearance"].clone()).map_err(|e| e.to_string())?;
        for scheme in appearance.schemes {
            for entry in scheme.overrides {
                for relative in entry.textures() {
                    if !files::safe_relative(relative) {
                        return Err(format!("Unsafe colour scheme texture path: {relative}"));
                    }
                    paths.insert(relative.to_owned(), serde_json::json!(root.join(relative)));
                }
            }
        }
    }
    Ok(paths.into())
}

// Resolve declarations to concrete preview part/LOD paths. This runs on scheme/document
// changes only; GDScript applies the returned textures without making binding decisions.
pub(crate) fn preview_plan(
    state: &serde_json::Value,
    selected: &str,
) -> Result<serde_json::Value, String> {
    use crate::assets::asset::{BuildingAppearance, MeshPart};
    use serde_json::json;
    if state["params"]["appearance"].is_null() || selected.is_empty() {
        return Ok(json!([]));
    }
    dependencies(state)?;
    let appearance: BuildingAppearance =
        serde_json::from_value(state["params"]["appearance"].clone()).map_err(|e| e.to_string())?;
    let parts: Vec<MeshPart> =
        serde_json::from_value(state["params"]["mesh_parts"].clone()).map_err(|e| e.to_string())?;
    let scheme = appearance
        .schemes
        .iter()
        .find(|s| s.id == selected)
        .ok_or("Selected colour scheme is missing")?;
    let mut plan = Vec::new();
    for entry in &scheme.overrides {
        let part = parts
            .iter()
            .position(|p| p.name == entry.part)
            .ok_or("Missing colour scheme part")?;
        let encoded = serde_json::to_value(entry).map_err(|e| e.to_string())?;
        let mut textures = serde_json::Map::new();
        for channel in ["albedo", "orm", "normal", "emission"] {
            if let Some(relative) = encoded[channel].as_str() {
                textures.insert(channel.into(), state["colour_sources"][relative].clone());
            }
        }
        for (lod, material) in entry.materials.iter().enumerate() {
            plan.push(json!({"part":part, "path":state["sources"][part][lod], "material":material, "textures":textures}));
        }
    }
    Ok(json!(plan))
}

// Complete authored texture dependency set. Source inventories are read once per part/LOD,
// not once per scheme. Image decoding is checked by the engine bridge before publication.
pub(crate) fn dependencies(state: &serde_json::Value) -> Result<Vec<(String, PathBuf)>, String> {
    use crate::assets::asset::{BuildingAppearance, MeshPart};
    use std::collections::BTreeMap;
    let value = &state["params"]["appearance"];
    if value.is_null() {
        return Ok(Vec::new());
    }
    let appearance: BuildingAppearance =
        serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
    let parts: Vec<MeshPart> =
        serde_json::from_value(state["params"]["mesh_parts"].clone()).map_err(|e| e.to_string())?;
    appearance.validate(&parts)?;
    let sources: Vec<Vec<String>> =
        serde_json::from_value(state["sources"].clone()).map_err(|e| e.to_string())?;
    let mut inventories = BTreeMap::new();
    let mut textures = BTreeMap::new();
    for scheme in &appearance.schemes {
        for entry in &scheme.overrides {
            let part = parts
                .iter()
                .position(|p| p.name == entry.part)
                .ok_or("Missing colour scheme part")?;
            for (lod, name) in entry.materials.iter().enumerate() {
                let key = (part, lod);
                if let std::collections::btree_map::Entry::Vacant(slot) = inventories.entry(key) {
                    let path = sources
                        .get(part)
                        .and_then(|paths| paths.get(lod))
                        .ok_or("Missing colour scheme LOD source")?;
                    slot.insert(materials(Path::new(path))?);
                }
                resolve(name, &inventories[&key]).map_err(|error| {
                    format!("{} / {} LOD{lod}: {error}", scheme.name, entry.part)
                })?;
            }
            for relative in entry.textures() {
                let path = state["colour_sources"][relative]
                    .as_str()
                    .ok_or_else(|| format!("Relink colour scheme texture: {relative}"))?;
                if !Path::new(path).is_file() {
                    return Err(format!("Missing colour scheme texture: {path}"));
                }
                textures.insert(relative.to_owned(), PathBuf::from(path));
            }
        }
    }
    Ok(textures.into_iter().collect())
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct MaterialSource {
    pub(crate) name: String,
    pub(crate) albedo: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct Candidate {
    pub(crate) id: String,
    pub(crate) path: PathBuf,
}

// Return source material definitions, not primitive slots: repeated use of one material
// on several primitives is valid, while two definitions with the same name are ambiguous.
pub(crate) fn materials(path: &Path) -> Result<Vec<MaterialSource>, String> {
    let gltf = files::read_gltf(path)?;
    let Some(materials) = gltf.get("materials").and_then(|value| value.as_array()) else {
        return Ok(Vec::new());
    };
    materials
        .iter()
        .map(|material| {
            let albedo = material["pbrMetallicRoughness"]["baseColorTexture"]["index"]
                .as_u64()
                .and_then(|index| gltf["textures"][index as usize]["source"].as_u64())
                .and_then(|index| gltf["images"][index as usize]["uri"].as_str());
            let albedo = match albedo {
                Some(uri) if !uri.starts_with("data:") => {
                    let relative = files::decode_uri(uri)?;
                    if !files::safe_relative(&relative) {
                        return Err(format!(
                            "Albedo URI must stay inside the model folder: {relative}"
                        ));
                    }
                    Some(path.parent().unwrap_or(Path::new(".")).join(relative))
                }
                _ => None,
            };
            Ok(MaterialSource {
                name: material["name"].as_str().unwrap_or_default().to_owned(),
                albedo,
            })
        })
        .collect()
}

pub(crate) fn resolve<'a>(
    name: &str,
    materials: &'a [MaterialSource],
) -> Result<&'a MaterialSource, String> {
    if name.trim().is_empty() {
        return Err("Choose a named source material".into());
    }
    let mut matches = materials.iter().filter(|material| material.name == name);
    let material = matches
        .next()
        .ok_or_else(|| format!("Missing source material: {name}"))?;
    if matches.next().is_some() {
        return Err(format!(
            "Ambiguous source material: {name}; give source materials unique names"
        ));
    }
    Ok(material)
}

// This is candidate discovery, never a binding heuristic: the caller must show and confirm
// the exact target/material mapping before adding any returned texture to the document.
// O(directory entries + candidates log candidates), once per source import or explicit review.
pub(crate) fn discover(albedo: &Path) -> Result<Vec<Candidate>, String> {
    let stem = albedo
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let Some((base, _)) = stem.rsplit_once("_albedo_") else {
        return Ok(Vec::new());
    };
    let prefix = format!("{base}_albedo_");
    let directory = albedo.parent().ok_or("Albedo has no source folder")?;
    let mut candidates = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_file()
            || path.extension() != albedo.extension()
        {
            continue;
        }
        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix(&prefix))
        else {
            continue;
        };
        if !id.is_empty()
            && id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
        {
            candidates.push(Candidate {
                id: id.into(),
                path,
            });
        }
    }
    candidates.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_scheme_material_resolution_rejects_missing_and_ambiguous_names() {
        let mut inventory = vec![MaterialSource {
            name: "walls".into(),
            albedo: None,
        }];
        assert!(resolve("walls", &inventory).is_ok());
        assert!(resolve("roof", &inventory).is_err());
        assert!(resolve("", &inventory).is_err());
        inventory.push(inventory[0].clone());
        assert!(resolve("walls", &inventory).is_err());
    }
}
