// SPDX-License-Identifier: GPL-2.0-only

//! Asset-local filesystem operations: dependency planning and transactional publication.
//! Work is O(files + bytes copied), plus ordered-map insertion O(F log F); no city scans.
//! File writes are deliberately sequential to keep publication/rollback ordering explicit.

use serde_json::Value;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);
pub(crate) const MAX_DRAFT_BYTES: u64 = 64 * 1024 * 1024;
type Files = BTreeMap<PathBuf, PathBuf>;

pub(crate) fn create_pack(mods: &Path, id: &str, name: &str, author: &str) -> Result<(), String> {
    let manifest = crate::assets::pack::manifest_toml(id, name, "0.1.0", author, "CC0");
    crate::assets::PackManifest::from_str(&manifest).map_err(|e| e.to_string())?;
    let directory = mods.join(id);
    if directory.is_symlink() {
        return Err("Pack directory cannot be a symbolic link".into());
    }
    fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let path = directory.join("pack.toml");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    if let Err(error) = file
        .write_all(manifest.as_bytes())
        .and_then(|_| file.sync_all())
    {
        let _ = fs::remove_file(path);
        return Err(error.to_string());
    }
    Ok(())
}

fn token() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    )
}

pub(crate) fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains([':', '\\'])
        && !Path::new(path).is_absolute()
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
}

pub(crate) fn read_gltf(path: &Path) -> Result<Value, String> {
    let mut file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let mut json = String::new();
    if path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("glb"))
    {
        let mut header = [0u8; 20];
        file.read_exact(&mut header).map_err(|e| e.to_string())?;
        let word = |index| u32::from_le_bytes(header[index..index + 4].try_into().unwrap());
        let total = file.metadata().map_err(|e| e.to_string())?.len();
        if word(0) != 0x46546c67
            || word(4) != 2
            || u64::from(word(8)) != total
            || total < 20
            || u64::from(word(12)) > total - 20
            || word(16) != 0x4e4f534a
        {
            return Err(format!("Invalid GLB header: {}", path.display()));
        }
        file.take(u64::from(word(12)))
            .read_to_string(&mut json)
            .map_err(|e| e.to_string())?;
    } else {
        file.read_to_string(&mut json).map_err(|e| e.to_string())?;
    }
    serde_json::from_str::<Value>(&json)
        .map_err(|e| e.to_string())
        .and_then(|value| {
            if value.is_object() {
                Ok(value)
            } else {
                Err("glTF must be a JSON object".into())
            }
        })
}

fn decode_uri(uri: &str) -> Result<String, String> {
    let mut bytes = Vec::with_capacity(uri.len());
    let mut input = uri.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let hex = |b: u8| (b as char).to_digit(16).map(|n| n as u8);
            let high = input
                .next()
                .and_then(hex)
                .ok_or("Invalid dependency URI escape")?;
            let low = input
                .next()
                .and_then(hex)
                .ok_or("Invalid dependency URI escape")?;
            bytes.push(high * 16 + low);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).map_err(|e| e.to_string())
}

fn same_contents(a: &Path, b: &Path) -> Result<bool, String> {
    let mut a = File::open(a).map_err(|e| e.to_string())?;
    let mut b = File::open(b).map_err(|e| e.to_string())?;
    if a.metadata().map_err(|e| e.to_string())?.len()
        != b.metadata().map_err(|e| e.to_string())?.len()
    {
        return Ok(false);
    }
    let mut left = [0; 8192];
    let mut right = [0; 8192];
    loop {
        let count = a.read(&mut left).map_err(|e| e.to_string())?;
        if count == 0 {
            return Ok(true);
        }
        b.read_exact(&mut right[..count])
            .map_err(|e| e.to_string())?;
        if left[..count] != right[..count] {
            return Ok(false);
        }
    }
}

fn add_file(files: &mut Files, relative: &str, source: &Path) -> Result<(), String> {
    if !safe_relative(relative) || matches!(relative, "asset.toml" | "pack.toml") {
        return Err(format!("Unsafe or reserved asset filename: {relative}"));
    }
    if !source.is_file() {
        return Err(format!("Missing mesh dependency: {}", source.display()));
    }
    let destination = PathBuf::from(relative);
    if let Some(previous) = files.get(&destination)
        && previous != source
        && !same_contents(previous, source)?
    {
        return Err(format!(
            "Different source files would overwrite '{relative}'. Rename/repack them first."
        ));
    }
    files.insert(destination, source.to_owned());
    Ok(())
}

fn collect_textures(files: &mut Files, source: &Path, relative: &Path) -> Result<(), String> {
    if !source.exists() {
        return Ok(());
    }
    if source.is_symlink() {
        return Err(format!(
            "Texture directory cannot be a symbolic link: {}",
            source.display()
        ));
    }
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let destination = relative.join(entry.file_name());
        if entry.path().is_dir() {
            collect_textures(files, &entry.path(), &destination)?;
        } else {
            add_file(files, &destination.to_string_lossy(), &entry.path())?;
        }
    }
    Ok(())
}

pub(crate) fn plan(
    models: &[(String, PathBuf)],
    extras: &[(String, PathBuf)],
) -> Result<Files, String> {
    let mut files = Files::new();
    for (relative, source) in models {
        add_file(&mut files, relative, source)?;
        let source_dir = source.parent().ok_or("Model has no parent directory")?;
        let destination_dir = Path::new(relative).parent().unwrap_or(Path::new(""));
        if source
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("fbx"))
        {
            for folder in ["Textures", "textures"] {
                collect_textures(
                    &mut files,
                    &source_dir.join(folder),
                    &destination_dir.join(folder),
                )?;
            }
        } else {
            let gltf = read_gltf(source)?;
            for collection in ["images", "buffers"] {
                let Some(entries) = gltf.get(collection) else {
                    continue;
                };
                for entry in entries
                    .as_array()
                    .ok_or("glTF dependency collection must be an array")?
                {
                    let Some(uri) = entry.get("uri") else {
                        continue;
                    };
                    let uri = uri.as_str().ok_or("glTF dependency URI must be a string")?;
                    if uri.is_empty() || uri.starts_with("data:") {
                        continue;
                    }
                    let decoded = decode_uri(uri)?;
                    if !safe_relative(&decoded) {
                        return Err(format!(
                            "Dependency must stay inside the asset folder: {decoded}"
                        ));
                    }
                    add_file(
                        &mut files,
                        &destination_dir.join(&decoded).to_string_lossy(),
                        &source_dir.join(&decoded),
                    )?;
                }
            }
        }
    }
    for (relative, source) in extras {
        add_file(&mut files, relative, source)?;
    }
    Ok(files)
}

fn copy_directory(source: &Path, target: &Path) -> Result<(), String> {
    if source.is_symlink() {
        return Err(format!(
            "Cannot publish through a symbolic link: {}",
            source.display()
        ));
    }
    fs::create_dir_all(target).map_err(|e| e.to_string())?;
    for entry in fs::read_dir(source).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!(
                "Cannot preserve symbolic link: {}",
                entry.path().display()
            ));
        }
        if kind.is_dir() {
            copy_directory(&entry.path(), &target.join(entry.file_name()))?;
        } else if kind.is_file() {
            fs::copy(entry.path(), target.join(entry.file_name())).map_err(|e| e.to_string())?;
        } else {
            return Err("Asset contains a non-regular file".into());
        }
    }
    Ok(())
}

struct Stage {
    path: PathBuf,
    preserve: bool,
}
impl Drop for Stage {
    fn drop(&mut self) {
        if !self.preserve
            && let Err(error) = fs::remove_dir_all(&self.path)
        {
            crate::debug_log!(
                "asset-editor",
                "Could not remove staging directory {}: {error}",
                self.path.display()
            );
        }
    }
}

pub(crate) fn publish(
    output: &Path,
    asset_id: &str,
    files: &Files,
    asset_toml: &str,
    pack_toml: &str,
) -> Result<(), String> {
    if !crate::assets::is_valid_asset_id(asset_id) {
        return Err("Invalid asset ID".into());
    }
    let assets = output.join("assets");
    if output.is_symlink() || assets.is_symlink() {
        return Err("Pack directories cannot be symbolic links".into());
    }
    fs::create_dir_all(&assets).map_err(|e| e.to_string())?;
    let mut stage = Stage {
        path: output.join(format!(".stage-{}", token())),
        preserve: true,
    };
    fs::create_dir(&stage.path).map_err(|e| e.to_string())?;
    stage.preserve = false;
    let staged = stage.path.join("asset");
    let target = assets.join(asset_id);
    if target.exists() || target.is_symlink() {
        copy_directory(&target, &staged)?;
    } else {
        fs::create_dir(&staged).map_err(|e| e.to_string())?;
    }
    for (relative, source) in files {
        let destination = staged.join(relative);
        fs::create_dir_all(destination.parent().ok_or("Missing dependency parent")?)
            .map_err(|e| e.to_string())?;
        fs::copy(source, destination).map_err(|e| e.to_string())?;
    }
    fs::write(staged.join("asset.toml"), asset_toml).map_err(|e| e.to_string())?;
    let pack = output.join("pack.toml");
    let created_pack = match OpenOptions::new().write(true).create_new(true).open(&pack) {
        Ok(mut file) => {
            if let Err(error) = file
                .write_all(pack_toml.as_bytes())
                .and_then(|_| file.sync_all())
            {
                let _ = fs::remove_file(&pack);
                return Err(error.to_string());
            }
            true
        }
        Err(error)
            if error.kind() == std::io::ErrorKind::AlreadyExists
                && pack.is_file()
                && !pack.is_symlink() =>
        {
            false
        }
        Err(error) => return Err(format!("Cannot publish pack manifest: {error}")),
    };
    let backup = stage.path.join("previous");
    let had_target = target.exists();
    let result = (|| {
        if had_target {
            fs::rename(&target, &backup).map_err(|e| e.to_string())?;
        }
        if let Err(error) = fs::rename(&staged, &target) {
            if had_target && let Err(restore) = fs::rename(&backup, &target) {
                stage.preserve = true;
                return Err(format!(
                    "Publish failed ({error}); restore failed ({restore}). Previous asset preserved at {}",
                    backup.display()
                ));
            }
            return Err(error.to_string());
        }
        Ok(())
    })();
    if result.is_err() && created_pack {
        let _ = fs::remove_file(pack);
    }
    result
}

pub(crate) fn save_draft(path: &Path, payload: &str) -> Result<(), String> {
    if payload.len() as u64 > MAX_DRAFT_BYTES {
        return Err("Draft exceeds the 64 MiB metadata limit".into());
    }
    let parent = path.parent().ok_or("Choose a draft file")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(".draft-{}.tmp", token()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|e| e.to_string())?;
    let result = file
        .write_all(payload.as_bytes())
        .and_then(|_| file.sync_all())
        .and_then(|_| {
            drop(file);
            fs::rename(&temporary, path)
        });
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Draft was not saved; previous file unchanged: {e}"))
}

pub(crate) fn load_draft(path: &Path) -> Result<String, String> {
    let file = File::open(path).map_err(|e| e.to_string())?;
    let mut payload = String::new();
    file.take(MAX_DRAFT_BYTES + 1)
        .read_to_string(&mut payload)
        .map_err(|e| e.to_string())?;
    if payload.len() as u64 > MAX_DRAFT_BYTES {
        return Err("Draft exceeds the 64 MiB metadata limit".into());
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("metrum-authoring-{}", token()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn write(&self, relative: &str, data: &str) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, data).unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn dependencies_collisions_and_escaped_paths() {
        let fixture = Fixture::new();
        let model = fixture.write("model.gltf", r#"{"images":[{"uri":"window%20light.png"}],"buffers":[{"uri":"mesh.bin"},{"uri":"data:embedded"}]}"#);
        fixture.write("window light.png", "texture");
        fixture.write("mesh.bin", "mesh");
        let models = vec![("nested/model.gltf".into(), model.clone())];
        let planned = plan(&models, &[]).unwrap();
        assert!(planned.contains_key(Path::new("nested/window light.png")));
        assert_eq!(planned.len(), 3);
        let same = fixture.write("same.gltf", &fs::read_to_string(&model).unwrap());
        let mut duplicates = models.clone();
        duplicates.push(("nested/model.gltf".into(), same));
        assert!(plan(&duplicates, &[]).is_ok());
        duplicates.push((
            "nested/model.gltf".into(),
            fixture.write("different.gltf", "{}"),
        ));
        assert!(plan(&duplicates, &[]).unwrap_err().contains("overwrite"));
        for unsafe_path in [
            "../escape",
            "/absolute",
            "a//b",
            "a/./b",
            "C:drive",
            r"a\b",
            "",
        ] {
            assert!(!safe_relative(unsafe_path));
        }
        assert!(plan(&models, &[("asset.toml".into(), model.clone())]).is_err());
        fs::write(&model, r#"{"images":[{"uri":"%2e%2e/escape.png"}]}"#).unwrap();
        assert!(plan(&models, &[]).unwrap_err().contains("inside"));
        fs::write(&model, r#"{"images":[{"uri":"missing.png"}]}"#).unwrap();
        assert!(plan(&models, &[]).unwrap_err().contains("Missing"));
    }

    #[test]
    fn publication_preserves_unmanaged_files_and_failed_copy_preserves_previous_asset() {
        let fixture = Fixture::new();
        let model = fixture.write("source/model.gltf", "{}");
        let output = fixture.0.join("pack");
        let files = plan(&[("model.gltf".into(), model.clone())], &[]).unwrap();
        publish(
            &output,
            "building.test",
            &files,
            "first manifest",
            "first pack",
        )
        .unwrap();
        let asset = output.join("assets/building.test");
        fs::write(asset.join("unmanaged.txt"), "keep").unwrap();
        publish(
            &output,
            "building.test",
            &files,
            "second manifest",
            "replace pack?",
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(output.join("pack.toml")).unwrap(),
            "first pack"
        );
        assert_eq!(
            fs::read_to_string(asset.join("unmanaged.txt")).unwrap(),
            "keep"
        );
        fs::remove_file(model).unwrap();
        assert!(publish(&output, "building.test", &files, "bad replacement", "").is_err());
        assert_eq!(
            fs::read_to_string(asset.join("asset.toml")).unwrap(),
            "second manifest"
        );
        assert_eq!(
            fs::read_to_string(asset.join("unmanaged.txt")).unwrap(),
            "keep"
        );
        assert!(!fs::read_dir(&output).unwrap().any(|e| {
            e.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".stage-")
        }));
    }

    #[test]
    fn drafts_replace_atomically_and_invalid_glb_headers_fail() {
        let fixture = Fixture::new();
        let path = fixture.0.join("asset.metrum-draft");
        save_draft(&path, "first").unwrap();
        save_draft(&path, "second").unwrap();
        assert_eq!(load_draft(&path).unwrap(), "second");
        assert!(read_gltf(&fixture.write("bad.glb", "short")).is_err());
        assert!(read_gltf(&fixture.write("bad.gltf", "[]")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn publication_and_texture_walk_never_follow_directory_links() {
        let fixture = Fixture::new();
        let model = fixture.write("source/model.fbx", "fbx fixture");
        std::os::unix::fs::symlink(&fixture.0, fixture.0.join("source/Textures")).unwrap();
        assert!(plan(&[("model.fbx".into(), model)], &[]).is_err());
        let output = fixture.0.join("pack");
        fs::create_dir_all(output.join("assets")).unwrap();
        let outside = fixture.write("outside/keep.txt", "unchanged");
        std::os::unix::fs::symlink(
            outside.parent().unwrap(),
            output.join("assets/building.test"),
        )
        .unwrap();
        assert!(publish(&output, "building.test", &Files::new(), "", "").is_err());
        assert_eq!(fs::read_to_string(outside).unwrap(), "unchanged");
    }
}
