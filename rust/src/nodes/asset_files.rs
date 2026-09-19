// SPDX-License-Identifier: GPL-2.0-only

//! File-service bridge for asset authoring. Rust owns dependency planning, staging and drafts.
//! Godot is used only for native Variant serialization and user:// path resolution.

use crate::assets::authoring::files;
use crate::nodes::sim::asset_export::{ExportParams, validated_tomls};
use godot::builtin::vdict;
use godot::classes::{Json, ProjectSettings};
use godot::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Stateless authoring file services; no widgets, meshes or live simulation state.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct AssetAuthoringFiles;

#[godot_api]
impl AssetAuthoringFiles {
    /// Create a validated pack without overwriting an existing manifest.
    #[func]
    pub fn create_pack(mods: GString, id: GString, name: GString, author: GString) -> GString {
        files::create_pack(
            &native_path(mods),
            &id.to_string(),
            &name.to_string(),
            &author.to_string(),
        )
        .err()
        .unwrap_or_default()
        .as_str()
        .into()
    }

    /// Validate and publish a document's complete model/dependency set transactionally.
    #[func]
    pub fn publish_document(document: GString, output: GString) -> GString {
        let result = (|| {
            let state: Value =
                serde_json::from_str(&document.to_string()).map_err(|e| e.to_string())?;
            let params: ExportParams =
                serde_json::from_value(state["params"].clone()).map_err(|e| e.to_string())?;
            let (asset, pack) = validated_tomls(&params)?;
            let sources: Vec<Vec<String>> =
                serde_json::from_value(state["sources"].clone()).map_err(|e| e.to_string())?;
            if sources.len() != params.mesh_parts.len() {
                return Err("Mesh source count does not match the document".into());
            }
            let mut models = Vec::new();
            for (part, paths) in params.mesh_parts.iter().zip(sources) {
                if part.lods.len() != paths.len() {
                    return Err(format!(
                        "Source count does not match LODs for {}",
                        part.name
                    ));
                }
                for (lod, path) in part.lods.iter().zip(paths) {
                    models.push((lod.file.clone(), PathBuf::from(path)));
                }
            }
            let extras: Vec<_> = params
                .thumbnail
                .iter()
                .map(|file| {
                    (
                        file.clone(),
                        PathBuf::from(state["thumbnail_source"].as_str().unwrap_or("")),
                    )
                })
                .collect();
            let plan = files::plan(&models, &extras)?;
            files::publish(&native_path(output), &params.asset_id, &plan, &asset, &pack)
        })();
        result.err().unwrap_or_default().as_str().into()
    }

    /// Read only the JSON chunk of a GLB/glTF for authoring inspection.
    #[func]
    pub fn read_gltf_json(path: GString) -> GString {
        files::read_gltf(&native_path(path))
            .unwrap_or_default()
            .to_string()
            .as_str()
            .into()
    }

    /// Save incomplete metadata losslessly; native object encoding remains disabled.
    #[func]
    pub fn save_draft(path: GString, document: VarDictionary) -> GString {
        let native = Json::from_native_ex(&document.to_variant())
            .full_objects(false)
            .done();
        let envelope = vdict! { "format": "metrum-asset-draft", "version": 1, "document": native };
        let payload = Json::stringify(&envelope.to_variant()).to_string();
        files::save_draft(&native_path(path), &payload)
            .err()
            .unwrap_or_default()
            .as_str()
            .into()
    }

    /// Load a versioned, structurally safe draft without repairing its authored values.
    #[func]
    pub fn load_draft(path: GString) -> VarDictionary {
        match load_document(&native_path(path)) {
            Ok(document) => vdict! { "document": document },
            Err(error) => vdict! { "error": error },
        }
    }

    /// Delete only an explicitly selected asset directory after a successful move publication.
    #[func]
    pub fn remove_asset(mods: GString, pack: GString, asset: GString) -> GString {
        let pack = pack.to_string();
        let asset = asset.to_string();
        if !crate::assets::is_valid_pack_id(&pack) || !crate::assets::is_valid_asset_id(&asset) {
            return "Invalid source asset identity".into();
        }
        let pack_dir = native_path(mods).join(pack);
        let assets = pack_dir.join("assets");
        let target = assets.join(asset);
        if pack_dir.is_symlink() || assets.is_symlink() || target.is_symlink() {
            return "Cannot remove an asset through a symbolic link".into();
        }
        std::fs::remove_dir_all(target)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default()
            .as_str()
            .into()
    }
}

fn native_path(path: GString) -> PathBuf {
    PathBuf::from(
        ProjectSettings::singleton()
            .globalize_path(&path)
            .to_string(),
    )
}

fn load_document(path: &Path) -> Result<VarDictionary, String> {
    let payload = files::load_draft(path)?;
    let mut parser = Json::new_gd();
    if parser.parse(&GString::from(payload.as_str())) != godot::global::Error::OK {
        return Err(format!(
            "Invalid draft JSON: {}",
            parser.get_error_message()
        ));
    }
    let envelope = parser
        .get_data()
        .try_to::<VarDictionary>()
        .map_err(|_| "Draft must be an object")?;
    if envelope.get("format") != Some("metrum-asset-draft".to_variant())
        || envelope.get("version").and_then(|v| v.try_to::<f64>().ok()) != Some(1.0)
    {
        return Err("Unsupported draft format or version; no document was changed".into());
    }
    let encoded = envelope
        .get("document")
        .ok_or("Draft is missing its document")?;
    let document = Json::to_native_ex(&encoded)
        .allow_objects(false)
        .done()
        .try_to::<VarDictionary>()
        .map_err(|_| "Draft is missing its asset document")?;
    let state: Value = serde_json::from_str(&Json::stringify(&document.to_variant()).to_string())
        .map_err(|e| e.to_string())?;
    if !state["params"].is_object() {
        return Err("Draft is missing asset metadata".into());
    }
    if let Some(sources) = state.get("sources") {
        serde_json::from_value::<Vec<Vec<String>>>(sources.clone())
            .map_err(|_| "Draft sources must contain arrays of file paths")?;
    }
    for key in ["mesh_parts", "anchors", "site_surfaces"] {
        if let Some(entries) = state["params"].get(key) {
            if !entries
                .as_array()
                .is_some_and(|values| values.iter().all(Value::is_object))
            {
                return Err(format!("Draft {key} must be an array of objects"));
            }
        }
    }
    if let Some(parts) = state["params"]["mesh_parts"].as_array() {
        for part in parts {
            if let Some(lods) = part.get("lods")
                && !lods
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_object))
            {
                return Err("Draft mesh LODs must be an array of objects".into());
            }
        }
    }
    Ok(document)
}
