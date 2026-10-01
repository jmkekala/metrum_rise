// SPDX-License-Identifier: GPL-2.0-only

//! File-service bridge for asset authoring. Rust owns dependency planning, staging and drafts.
//! Godot supplies Variant serialization, user:// resolution and texture decoding validation.

use crate::assets::PackManifest;
use crate::assets::archive;
use crate::assets::authoring::{colours, files};
use crate::assets::pack::{PackSettings, bump_version, compare_versions};
use crate::nodes::sim::asset_export::{ExportParams, validated_tomls};
use godot::builtin::vdict;
use godot::classes::{Image, Json, ProjectSettings};
use godot::prelude::*;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Stateless authoring file services; no widgets, meshes or live simulation state.
#[derive(GodotClass)]
#[class(init, base = RefCounted)]
pub struct AssetAuthoringFiles;

#[godot_api]
impl AssetAuthoringFiles {
    /// Validate source material mappings and decode all distinct scheme textures without writes.
    #[func]
    pub fn colour_document_error(document: GString) -> GString {
        let result = serde_json::from_str::<Value>(&document.to_string())
            .map_err(|error| error.to_string())
            .and_then(|state| colour_dependencies(&state).map(|_| ()));
        result.err().unwrap_or_default().as_str().into()
    }
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
            let colour_params: Value =
                serde_json::from_str(&Json::stringify(&params.to_variant()).to_string())
                    .map_err(|error| error.to_string())?;
            let colour_sources = colours::exported_sources(&colour_params, &root)?;
            let colour_sources =
                Json::parse_string(&GString::from(colour_sources.to_string().as_str()));
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
                vdict! { "params": params, "sources": sources, "colour_sources": colour_sources, "origin": VarDictionary::new(), "supporting_sources": supporting_sources, "thumbnail_source": if thumbnail.is_empty() { String::new() } else { root.join(thumbnail).to_string_lossy().into_owned() } },
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

    /// Current editable `pack.toml` metadata of an installed pack.
    #[func]
    pub fn pack_settings(mods: GString, pack: GString) -> VarDictionary {
        let result = files::pack_directory(&native_path(mods), &pack.to_string())
            .and_then(|root| {
                std::fs::read_to_string(root.join("pack.toml")).map_err(|e| e.to_string())
            })
            .and_then(|text| PackManifest::from_str(&text).map_err(|e| e.to_string()));
        match result {
            Ok(manifest) => pack_dictionary(&manifest),
            Err(error) => vdict! { "error": error },
        }
    }

    /// Validate and atomically rewrite pack metadata; `pack_id` cannot change.
    #[func]
    pub fn update_pack(mods: GString, pack: GString, settings: VarDictionary) -> GString {
        let text = |key: &str| {
            settings
                .get(key)
                .and_then(|v| v.try_to::<GString>().ok())
                .unwrap_or_default()
                .to_string()
        };
        let values = [
            "display_name",
            "version",
            "author",
            "license",
            "description",
        ]
        .map(text);
        let [display_name, version, author, license, description] = &values;
        let settings = PackSettings {
            display_name,
            version,
            author,
            license,
            description,
        };
        files::update_pack(&native_path(mods), &pack.to_string(), &settings)
            .err()
            .unwrap_or_default()
            .as_str()
            .into()
    }

    /// Next semantic version for `patch`, `minor` or `major`; empty when it has none.
    #[func]
    pub fn bumped_version(version: GString, part: GString) -> GString {
        bump_version(&version.to_string(), &part.to_string())
            .unwrap_or_default()
            .as_str()
            .into()
    }

    /// Validate a pack and summarise its share archive without writing. Takes native paths
    /// and touches no engine singleton, so it may run on a worker thread.
    #[func]
    pub fn inspect_pack(mods: GString, pack: GString) -> VarDictionary {
        let result = files::pack_directory(Path::new(&mods.to_string()), &pack.to_string())
            .and_then(|root| archive::inventory(&root));
        match result {
            Ok(contents) => {
                let mut summary = pack_dictionary(&contents.pack);
                summary.set("assets", contents.assets as i64);
                // checksums.sha256 is generated into the archive as one more file.
                summary.set("files", contents.files.len() as i64 + 1);
                summary.set("bytes", contents.bytes as i64);
                let excluded: PackedStringArray =
                    contents.excluded.iter().map(GString::from).collect();
                summary.set("excluded", excluded);
                summary
            }
            Err(error) => vdict! { "error": error },
        }
    }

    /// Write `<pack_id>-<version>.metrum.zip` and its `.sha256` sidecar into `destination`,
    /// after an optional `patch`/`minor`/`major` bump (empty for none). Native paths only;
    /// safe on a worker thread.
    #[func]
    pub fn export_pack(
        mods: GString,
        pack: GString,
        destination: GString,
        bump: GString,
    ) -> VarDictionary {
        let bump = bump.to_string();
        match archive::export(
            Path::new(&mods.to_string()),
            &pack.to_string(),
            Path::new(&destination.to_string()),
            Some(bump.as_str()).filter(|part| !part.is_empty()),
        ) {
            Ok(exported) => vdict! {
                "path": exported.path.to_string_lossy().as_ref(),
                "sha256": exported.sha256,
                "version": exported.version,
            },
            Err(error) => vdict! { "error": error },
        }
    }

    /// SHA-256 of a file as lowercase hex. Native path; safe on a worker thread.
    #[func]
    pub fn file_sha256(path: GString) -> VarDictionary {
        match archive::sha256(Path::new(&path.to_string())) {
            Ok(sha256) => vdict! { "sha256": sha256 },
            Err(error) => vdict! { "error": error },
        }
    }

    /// Normalised expected SHA-256 from 64 hex digits or a `sha256sum` line; empty if invalid.
    #[func]
    pub fn expected_sha256(text: GString) -> GString {
        archive::expected_sha256(&text.to_string())
            .unwrap_or_default()
            .as_str()
            .into()
    }

    /// Verify a share archive against `expected` and stage it inside `mods` (`TOOLS-10`).
    /// `installed` is `absent`, `identical` (nothing staged) or `different`, with `change`
    /// relating the versions; a staged pack must be passed to `commit_import` or
    /// `discard_import`. Native paths; worker-safe.
    #[func]
    pub fn stage_import(
        mods: GString,
        archive: GString,
        expected: GString,
        bundled: PackedStringArray,
    ) -> VarDictionary {
        let bundled: Vec<String> = bundled.as_slice().iter().map(GString::to_string).collect();
        let result = archive::stage(
            Path::new(&archive.to_string()),
            &expected.to_string(),
            Path::new(&mods.to_string()),
            &bundled,
        );
        let staged = match result {
            Ok(staged) => staged,
            Err(error) => return vdict! { "error": error },
        };
        let mut summary = pack_dictionary(&staged.pack);
        summary.set("assets", staged.assets as i64);
        summary.set("files", staged.files as i64);
        summary.set("bytes", staged.bytes as i64);
        let staging = staged.staging.unwrap_or_default();
        summary.set("staging", staging.to_string_lossy().as_ref());
        let (installed, version) = match staged.installed {
            archive::Installed::Absent => ("absent", None),
            archive::Installed::Identical => ("identical", None),
            archive::Installed::Different { version } => ("different", version),
        };
        // `update`, `same` or `downgrade` against the installed version; empty if unreadable.
        let change = version
            .as_deref()
            .and_then(|current| compare_versions(&staged.pack.version, current))
            .map_or("", |order| match order {
                std::cmp::Ordering::Greater => "update",
                std::cmp::Ordering::Equal => "same",
                std::cmp::Ordering::Less => "downgrade",
            });
        summary.set("installed", installed);
        summary.set("installed_version", version.unwrap_or_default());
        summary.set("change", change);
        summary
    }

    /// Rename a staged import into place; any installed copy must already be in Trash.
    #[func]
    pub fn commit_import(mods: GString, staging: GString) -> VarDictionary {
        match archive::commit(
            Path::new(&mods.to_string()),
            Path::new(&staging.to_string()),
        ) {
            Ok(pack_id) => vdict! { "pack_id": pack_id },
            Err(error) => vdict! { "error": error },
        }
    }

    /// Folder of an installed pack that may be moved to Trash; bundled ids are refused.
    #[func]
    pub fn inspect_pack_removal(
        mods: GString,
        pack: GString,
        bundled: PackedStringArray,
    ) -> VarDictionary {
        let bundled: Vec<String> = bundled.as_slice().iter().map(GString::to_string).collect();
        match files::removal_target(&native_path(mods), &pack.to_string(), &bundled) {
            Ok(path) => vdict! { "path": path.to_string_lossy().as_ref() },
            Err(error) => vdict! { "error": error },
        }
    }

    /// Compare an installed pack with the `checksums.sha256` its import kept (`TOOLS-11`).
    /// Native paths; safe on a worker thread.
    #[func]
    pub fn verify_installed_pack(mods: GString, pack: GString) -> VarDictionary {
        let list = |paths: Vec<String>| {
            paths
                .iter()
                .map(GString::from)
                .collect::<PackedStringArray>()
        };
        match archive::verify(Path::new(&mods.to_string()), &pack.to_string()) {
            Ok(report) => vdict! {
                "files": report.files as i64,
                "changed": list(report.changed),
                "missing": list(report.missing),
                "extra": list(report.extra),
                "invalid": report.invalid.unwrap_or_default(),
            },
            Err(error) => vdict! { "error": error },
        }
    }

    /// Remove import staging folders an earlier process left behind (e.g. after a crash).
    #[func]
    pub fn sweep_imports(mods: GString) {
        archive::sweep(&native_path(mods));
    }

    /// Remove a staged import without installing it.
    #[func]
    pub fn discard_import(mods: GString, staging: GString) -> GString {
        archive::discard(
            Path::new(&mods.to_string()),
            Path::new(&staging.to_string()),
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
            let mut state: Value =
                serde_json::from_str(&document.to_string()).map_err(|e| e.to_string())?;
            colours::canonical_names(&mut state)?;
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
            extras.extend(colour_dependencies(&state)?);
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

fn colour_dependencies(state: &Value) -> Result<Vec<(String, PathBuf)>, String> {
    let dependencies = colours::dependencies(state)?;
    for (_, path) in &dependencies {
        let image = Image::load_from_file(&GString::from(path.to_string_lossy().as_ref()));
        if image.is_none_or(|image| image.is_empty()) {
            return Err(format!(
                "Unreadable colour scheme texture: {}",
                path.display()
            ));
        }
    }
    Ok(dependencies)
}

fn pack_dictionary(manifest: &PackManifest) -> VarDictionary {
    vdict! {
        "pack_id": manifest.pack_id.as_str(),
        "display_name": manifest.display_name.as_str(),
        "version": manifest.version.as_str(),
        "author": manifest.author.as_str(),
        "license": manifest.license.as_str(),
        "description": manifest.description.as_deref().unwrap_or_default(),
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
