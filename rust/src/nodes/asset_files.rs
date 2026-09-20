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
    /// Resolve a user asset directory without permitting links or escaping the asset root.
    #[func]
    pub fn asset_location(mods: GString, pack: GString, asset: GString) -> VarDictionary {
        match files::asset_directory(&native_path(mods), &pack.to_string(), &asset.to_string()) {
            Ok(path) => vdict! { "path": path.to_string_lossy().as_ref() },
            Err(error) => vdict! { "error": error },
        }
    }

    /// Check trash eligibility without changing the filesystem; repeat immediately before trash.
    #[func]
    pub fn inspect_trash(
        mods: GString,
        pack: GString,
        asset: GString,
        protected: PackedStringArray,
    ) -> VarDictionary {
        let protected = protected
            .as_slice()
            .iter()
            .map(|p| native_path(p.clone()))
            .collect::<Vec<_>>();
        match files::trash_target(
            &native_path(mods),
            &pack.to_string(),
            &asset.to_string(),
            &protected,
        ) {
            Ok(path) => vdict! { "path": path.to_string_lossy().as_ref() },
            Err(error) => vdict! { "error": error },
        }
    }

    /// Prepare independent source files and an unpublished copy, preserving unknown metadata.
    #[func]
    pub fn copy_for_editing(
        mods: GString,
        manifest: VarDictionary,
        destination: VarDictionary,
        id: GString,
        name: GString,
        workspace: GString,
    ) -> VarDictionary {
        let result = (|| -> Result<VarDictionary, String> {
            let text = |data: &VarDictionary, key: &str| {
                data.get(key)
                    .and_then(|v| v.try_to::<GString>().ok())
                    .unwrap_or_default()
                    .to_string()
            };
            if name.to_string().trim().is_empty() {
                return Err("Enter a name for the copy".into());
            }
            let source = files::asset_directory(
                &native_path(mods.clone()),
                &text(&manifest, "pack_id"),
                &text(&manifest, "asset_id"),
            )?;
            let parts: VarArray = manifest
                .get("mesh_parts")
                .and_then(|v| v.try_to().ok())
                .unwrap_or_default();
            let mut relative_sources = Vec::new();
            let mut models = Vec::new();
            for part in parts.iter_shared() {
                let part: VarDictionary = part.try_to().map_err(|e| e.to_string())?;
                let lods: VarArray = part
                    .get("lods")
                    .and_then(|v| v.try_to().ok())
                    .unwrap_or_default();
                let mut paths = Vec::new();
                for lod in lods.iter_shared() {
                    let lod: VarDictionary = lod.try_to().map_err(|e| e.to_string())?;
                    let file = text(&lod, "file");
                    models.push((file.clone(), source.join(&file)));
                    paths.push(file);
                }
                relative_sources.push(paths);
            }
            let thumbnail = text(&manifest, "thumbnail");
            let extras = if thumbnail.is_empty() {
                vec![]
            } else {
                vec![(thumbnail.clone(), source.join(&thumbnail))]
            };
            files::plan(&models, &extras)?;
            let root = files::working_copy(
                &native_path(mods),
                &text(&manifest, "pack_id"),
                &text(&manifest, "asset_id"),
                &text(&destination, "pack_id"),
                &id.to_string(),
                &native_path(workspace),
            )?;
            let mut params = manifest.duplicate_deep();
            let mut supporting_sources = VarDictionary::new();
            for (relative, path) in files::attribution_files(&root)? {
                supporting_sources.set(relative, path.to_string_lossy().as_ref());
            }
            params.set("pack_id", text(&destination, "pack_id"));
            params.set("pack_name", text(&destination, "display_name"));
            params.set("pack_author", text(&destination, "author"));
            params.set("asset_id", id);
            params.set("display_name", name);
            let sources = relative_sources
                .into_iter()
                .map(|paths| {
                    paths
                        .into_iter()
                        .map(|p| root.join(p).to_string_lossy().as_ref().to_variant())
                        .collect::<VarArray>()
                        .to_variant()
                })
                .collect::<VarArray>();
            Ok(
                vdict! { "params": params, "sources": sources, "origin": VarDictionary::new(), "supporting_sources": supporting_sources, "thumbnail_source": if thumbnail.is_empty() { String::new() } else { root.join(thumbnail).to_string_lossy().into_owned() } },
            )
        })();
        match result {
            Ok(document) => vdict! { "document": document },
            Err(error) => vdict! { "error": error },
        }
    }

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
            let mut extras: Vec<_> = params
                .thumbnail
                .iter()
                .map(|file| {
                    (
                        file.clone(),
                        PathBuf::from(state["thumbnail_source"].as_str().unwrap_or("")),
                    )
                })
                .collect();
            if let Some(sources) = state.get("supporting_sources") {
                let sources: std::collections::BTreeMap<String, String> =
                    serde_json::from_value(sources.clone()).map_err(|e| e.to_string())?;
                for (relative, path) in sources {
                    if !relative.starts_with("attribution/") || !files::safe_relative(&relative) {
                        return Err("Supporting credits must stay within attribution/".into());
                    }
                    extras.push((relative, PathBuf::from(path)));
                }
            }
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
