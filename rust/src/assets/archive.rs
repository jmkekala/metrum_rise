// SPDX-License-Identifier: GPL-2.0-only

//! Share archives (`TOOLS-09`): a pack's referenced file set and its deterministic zip.
//! Import (`TOOLS-10`) lives in [`import`] and re-checks every rule export enforces;
//! [`verify`] checks an installed pack against the checksums import kept (`TOOLS-11`).
//!
//! [`asset_files`] is the single definition of what an installed asset consists of.
//! Publication checks its staged asset against it and export packages exactly it, so the
//! two cannot disagree. Export is O(files + bytes) with O(F log F) path ordering; it never
//! modifies the pack except for an explicitly requested version bump.

use super::authoring::files::{self, Files};
use super::pack::{self, PackSettings};
use super::{AssetManifest, PackManifest};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

mod import;
mod verify;
pub(crate) use import::{Installed, commit, discard, expected_sha256, sha256, stage, sweep};
pub(crate) use verify::verify;

const CHECKSUMS: &str = "checksums.sha256";

/// Every file an asset's manifest references, keyed by its path inside the asset folder.
/// `asset.toml` itself is not included.
pub(crate) fn asset_files(manifest: &AssetManifest, directory: &Path) -> Result<Files, String> {
    let models: Vec<_> = manifest
        .lods
        .iter()
        .chain(manifest.mesh_parts.iter().flat_map(|part| &part.lods))
        .map(|lod| (lod.file.clone(), directory.join(&lod.file)))
        .collect();
    let mut references: Vec<&str> = manifest.thumbnail.as_deref().into_iter().collect();
    if let Some(appearance) = manifest
        .building
        .as_ref()
        .and_then(|b| b.appearance.as_ref())
    {
        let overrides = appearance
            .schemes
            .iter()
            .flat_map(|scheme| &scheme.overrides);
        references.extend(overrides.flat_map(|entry| entry.textures()));
    }
    if let Some(vehicle) = &manifest.vehicle {
        references.extend(
            vehicle
                .color_variants
                .iter()
                .map(|v| v.albedo_file.as_str()),
        );
    }
    if let Some(character) = &manifest.character {
        references.extend(
            character
                .skin_variants
                .iter()
                .map(|v| v.albedo_file.as_str()),
        );
    }
    let mut extras: Vec<_> = references
        .into_iter()
        .map(|path| (path.to_owned(), directory.join(path)))
        .collect();
    // Credits copied from other packs travel with the asset; see `files::working_copy`.
    if directory.join("attribution").is_dir() {
        extras.extend(files::attribution_files(directory)?);
    }
    files::plan(&models, &extras)
}

/// What a pack's share archive would contain. Paths are relative to the pack root and use `/`.
pub(crate) struct Inventory {
    pub(crate) pack: PackManifest,
    pub(crate) files: BTreeMap<String, PathBuf>,
    pub(crate) excluded: Vec<String>,
    pub(crate) assets: usize,
    pub(crate) bytes: u64,
}

/// Validate a pack and derive its referenced file set without writing anything.
pub(crate) fn inventory(root: &Path) -> Result<Inventory, String> {
    let mut present = Vec::new();
    walk(root, "", &mut present)?;
    present.sort_unstable();
    let pack_toml = fs::read_to_string(root.join("pack.toml")).map_err(|e| e.to_string())?;
    let pack = PackManifest::from_str(&pack_toml).map_err(|e| format!("pack.toml: {e}"))?;
    let mut referenced = BTreeMap::from([("pack.toml".to_owned(), root.join("pack.toml"))]);
    let mut assets = 0;
    for path in present
        .iter()
        .filter(|p| p.rsplit('/').next() == Some("asset.toml"))
    {
        // The scanner loads every asset.toml in the pack; one outside this layout would be
        // installed but never exported, so it refuses the export instead.
        let ["assets", id, "asset.toml"] = path.split('/').collect::<Vec<_>>()[..] else {
            return Err(format!(
                "{path}: assets must live at assets/<asset_id>/asset.toml"
            ));
        };
        let directory = root.join("assets").join(id);
        let manifest = fs::read_to_string(directory.join("asset.toml"))
            .map_err(|e| e.to_string())
            .and_then(|text| AssetManifest::from_str(&text).map_err(|e| e.to_string()))
            .map_err(|e| format!("{path}: {e}"))?;
        if manifest.asset_id != id {
            return Err(format!(
                "{path}: folder name must match asset_id '{}'",
                manifest.asset_id
            ));
        }
        for (relative, source) in
            asset_files(&manifest, &directory).map_err(|e| format!("assets/{id}: {e}"))?
        {
            referenced.insert(
                format!("assets/{id}/{}", relative.to_string_lossy()),
                source,
            );
        }
        referenced.insert(path.clone(), directory.join("asset.toml"));
        assets += 1;
    }
    let mut folded = BTreeSet::new();
    for path in referenced.keys() {
        if !folded.insert(path.to_lowercase()) {
            return Err(format!(
                "Another file differs from {path} only by letter case"
            ));
        }
    }
    let mut bytes = 0;
    for source in referenced.values() {
        bytes += fs::metadata(source).map_err(|e| e.to_string())?.len();
    }
    Ok(Inventory {
        excluded: present
            .into_iter()
            .filter(|path| !referenced.contains_key(path))
            .collect(),
        pack,
        files: referenced,
        assets,
        bytes,
    })
}

// Regular files only, sorted by the caller's map. Any link could make the runtime scanner
// load something the archive omits, so links anywhere in the pack refuse the export.
fn walk(root: &Path, relative: &str, out: &mut Vec<String>) -> Result<(), String> {
    for entry in fs::read_dir(root.join(relative)).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|name| format!("File name is not UTF-8: {}", name.to_string_lossy()))?;
        let path = if relative.is_empty() {
            name
        } else {
            format!("{relative}/{name}")
        };
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            return Err(format!("Packs cannot contain symbolic links: {path}"));
        }
        if kind.is_dir() {
            walk(root, &path, out)?;
        } else if kind.is_file() {
            out.push(path);
        }
    }
    Ok(())
}

/// A written archive and its SHA-256, which the sidecar also records.
pub(crate) struct Exported {
    pub(crate) path: PathBuf,
    pub(crate) sha256: String,
    pub(crate) version: String,
}

/// Export an installed pack as `<pack_id>-<version>.metrum.zip` plus its `.sha256` sidecar.
/// `bump` (`patch`, `minor` or `major`) rewrites `pack.toml` first, after validation passes.
pub(crate) fn export(
    mods: &Path,
    pack: &str,
    destination: &Path,
    bump: Option<&str>,
) -> Result<Exported, String> {
    let root = files::pack_directory(mods, pack)?;
    let mut contents = inventory(&root)?;
    if !destination.is_dir() {
        return Err("Choose an existing destination folder".into());
    }
    if let Some(part) = bump {
        let version = pack::bump_version(&contents.pack.version, part)?;
        let settings = PackSettings {
            version: &version,
            ..PackSettings::of(&contents.pack)
        };
        files::update_pack(mods, pack, &settings)?;
        contents = inventory(&root)?;
    }
    write(&contents, destination)
}

// Removes a temporary file unless it was renamed into place.
struct Temporary(Option<PathBuf>);
impl Drop for Temporary {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn write(contents: &Inventory, destination: &Path) -> Result<Exported, String> {
    let id = &contents.pack.pack_id;
    let name = format!("{id}-{}.metrum.zip", contents.pack.version);
    let mut archive = Temporary(Some(
        destination.join(format!(".{name}.{}.tmp", files::token())),
    ));
    let mut sidecar = Temporary(Some(
        destination.join(format!(".{name}.sha256.{}.tmp", files::token())),
    ));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(archive.0.as_ref().unwrap())
        .map_err(|e| e.to_string())?;
    let mut zip = ZipWriter::new(file);
    // checksums.sha256 sorts between assets/ and pack.toml, so the files after it are read
    // into memory first; their listed hashes are then of exactly the bytes written.
    let (before, after): (Vec<_>, Vec<_>) = contents
        .files
        .iter()
        .partition(|(path, _)| path.as_str() < CHECKSUMS);
    let mut late = Vec::with_capacity(after.len());
    for (path, source) in after {
        late.push((path, fs::read(source).map_err(|e| format!("{path}: {e}"))?));
    }
    let mut sums = String::new();
    let mut buffer = vec![0; 1 << 16];
    for (path, source) in before {
        let mut input = File::open(source).map_err(|e| format!("{path}: {e}"))?;
        let length = input.metadata().map_err(|e| e.to_string())?.len();
        start(&mut zip, id, path, length)?;
        let digest = hash_copy(&mut input, &mut buffer, |chunk| zip.write_all(chunk))
            .map_err(|e| format!("{path}: {e}"))?;
        sums.push_str(&format!("{digest}  {path}\n"));
    }
    for (path, bytes) in &late {
        sums.push_str(&format!("{}  {path}\n", hex(&Sha256::digest(bytes))));
    }
    start(&mut zip, id, CHECKSUMS, sums.len() as u64)?;
    zip.write_all(sums.as_bytes()).map_err(|e| e.to_string())?;
    for (path, bytes) in &late {
        start(&mut zip, id, path, bytes.len() as u64)?;
        zip.write_all(bytes).map_err(|e| e.to_string())?;
    }
    let mut file = zip.finish().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    // The writer seeks back to patch headers, so the digest is taken from the final file.
    std::io::Seek::rewind(&mut file).map_err(|e| e.to_string())?;
    let sha256 = hash_copy(&mut file, &mut buffer, |_| Ok(())).map_err(|e| e.to_string())?;
    drop(file);
    let mut out = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(sidecar.0.as_ref().unwrap())
        .map_err(|e| e.to_string())?;
    out.write_all(format!("{sha256}  {name}\n").as_bytes())
        .and_then(|_| out.sync_all())
        .map_err(|e| e.to_string())?;
    let path = destination.join(&name);
    fs::rename(archive.0.as_ref().unwrap(), &path).map_err(|e| e.to_string())?;
    archive.0 = None;
    fs::rename(
        sidecar.0.as_ref().unwrap(),
        destination.join(format!("{name}.sha256")),
    )
    .map_err(|e| {
        format!("The archive was written (SHA-256 {sha256}) but its checksum file was not: {e}")
    })?;
    sidecar.0 = None;
    Ok(Exported {
        path,
        sha256,
        version: contents.pack.version.clone(),
    })
}

fn start(zip: &mut ZipWriter<File>, id: &str, path: &str, length: u64) -> Result<(), String> {
    let compressed = !path.rsplit_once('.').is_some_and(|(_, ext)| {
        ["png", "webp", "jpg", "jpeg"]
            .iter()
            .any(|known| ext.eq_ignore_ascii_case(known))
    });
    // DEFAULT fixes the timestamp at 1980-01-01 00:00 and adds no extra fields.
    let options = SimpleFileOptions::DEFAULT
        .compression_method(if compressed {
            CompressionMethod::Deflated
        } else {
            CompressionMethod::Stored
        })
        .unix_permissions(0o644)
        .large_file(length >= u64::from(u32::MAX));
    zip.start_file(format!("{id}/{path}"), options)
        .map_err(|e| e.to_string())
}

// Stream `input` to `sink` through one reused buffer and return its SHA-256 as hex.
fn hash_copy(
    input: &mut impl Read,
    buffer: &mut [u8],
    mut sink: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    loop {
        let count = input.read(buffer)?;
        if count == 0 {
            return Ok(hex(&hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
        sink(&buffer[..count])?;
    }
}

// Parse `checksums.sha256` as export writes it: `<sha256 hex>  <path>` lines, each path safe,
// relative to the pack root, listed once and sorted. Returns path -> digest.
fn parse_checksums(text: &str) -> Result<BTreeMap<&str, &str>, String> {
    let mut listed = BTreeMap::new();
    let mut previous = "";
    for line in text.lines() {
        let (digest, path) = line
            .split_once("  ")
            .filter(|(digest, path)| {
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
                    && files::safe_relative(path)
                    && *path != CHECKSUMS
            })
            .ok_or_else(|| format!("{CHECKSUMS}: malformed line '{line}'"))?;
        if path <= previous {
            return Err(format!(
                "{CHECKSUMS} must list each file once, sorted by path"
            ));
        }
        previous = path;
        listed.insert(path, digest);
    }
    Ok(listed)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests;
