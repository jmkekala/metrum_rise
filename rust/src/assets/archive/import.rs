// SPDX-License-Identifier: GPL-2.0-only

//! Share-archive import (`TOOLS-10`): verify a `.metrum.zip`, stage it inside the mods folder
//! and rename it into place.
//!
//! Every rule export enforces is checked again here, so an archive that export would never
//! produce is refused. Staging is O(entries + bytes): every entry is decompressed and hashed
//! once, sizes are enforced on the decompressed bytes, and the staged pack then goes through
//! the same [`inventory`] export uses, so the installed file set always equals [`asset_files`].
//!
//! [`asset_files`]: super::asset_files

use super::{CHECKSUMS, hash_copy, inventory, parse_checksums};
use crate::assets::{PackManifest, authoring::files};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use zip::{CompressionMethod, ZipArchive};

/// Most entries one archive may hold.
const MAX_ENTRIES: usize = 10_000;
/// Largest decompressed size of a single entry.
const MAX_ENTRY_BYTES: u64 = 256 << 20;
/// Largest decompressed size of the whole archive.
const MAX_TOTAL_BYTES: u64 = 1 << 30;
/// Highest decompressed/compressed ratio allowed for entries over [`RATIO_FLOOR`].
const MAX_RATIO: u64 = 200;
const RATIO_FLOOR: u64 = 1 << 20;
/// Staging folders live inside the mods folder so the final rename stays on one filesystem.
/// The leading dot keeps the pack scanner and the pack list away from them.
const STAGING_PREFIX: &str = ".import-";
/// A staging folder from another process is only swept once it is this old, so a second
/// game instance sharing the profile keeps the pack its open dialog is reviewing.
const STALE_STAGING: std::time::Duration = std::time::Duration::from_secs(60 * 60);

/// How an archive's pack relates to the copy already installed under the same `pack_id`.
pub(crate) enum Installed {
    /// No pack with this `pack_id` is installed.
    Absent,
    /// The installed pack holds exactly the archive's files; nothing would change.
    Identical,
    /// A different copy is installed; `version` is its `pack.toml` version, if readable.
    Different { version: Option<String> },
}

/// A verified archive. Unless the pack is already installed identically, it is extracted
/// into `staging` and waits for [`commit`] or [`discard`].
pub(crate) struct Staged {
    pub(crate) staging: Option<PathBuf>,
    pub(crate) pack: PackManifest,
    pub(crate) assets: usize,
    pub(crate) files: usize,
    pub(crate) bytes: u64,
    pub(crate) installed: Installed,
}

/// Normalise an expected SHA-256 typed by the player: 64 hex digits, alone or as the first
/// field of a `sha256sum` line, in any case and with surrounding whitespace.
pub(crate) fn expected_sha256(text: &str) -> Option<String> {
    let digest = text.split_whitespace().next()?;
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| digest.to_ascii_lowercase())
}

/// Stream a file through SHA-256 and return it as lowercase hex.
pub(crate) fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    hash_copy(&mut file, &mut vec![0; 1 << 16], |_| Ok(())).map_err(|e| e.to_string())
}

// Removes a staging folder unless it was handed to the caller.
struct Staging(Option<PathBuf>);
impl Drop for Staging {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

/// Verify `archive` against the expected SHA-256 and every share-archive rule, then extract
/// it into a fresh staging folder in `mods`. `bundled` lists `pack_id`s shipped with the
/// game, which an import may never replace. Nothing outside the staging folder is written,
/// and a refusal removes the staging folder.
pub(crate) fn stage(
    archive: &Path,
    expected: &str,
    mods: &Path,
    bundled: &[String],
) -> Result<Staged, String> {
    let expected = expected_sha256(expected).ok_or("Enter the 64-character SHA-256")?;
    let mut file = File::open(archive).map_err(|e| e.to_string())?;
    let mut buffer = vec![0; 1 << 16];
    // Hash the open handle that is then unzipped, so the verified bytes are the ones read.
    let actual = hash_copy(&mut file, &mut buffer, |_| Ok(())).map_err(|e| e.to_string())?;
    if actual != expected {
        return Err(format!(
            "The archive's SHA-256 is {actual}, not the expected value. It was not imported."
        ));
    }
    let declared = declared_entries(&mut file)?;
    file.rewind().map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|e| format!("Not a readable zip: {e}"))?;
    // The reader keeps one entry per name, so a mismatch means duplicate paths.
    if declared != zip.len() {
        return Err("The archive contains duplicate paths".into());
    }
    if zip.len() > MAX_ENTRIES {
        return Err(format!("The archive has more than {MAX_ENTRIES} entries"));
    }
    let names = entry_names(&mut zip)?;
    let id = &names[0].0;
    if bundled.iter().any(|pack| pack == id) {
        return Err(format!(
            "'{id}' is a pack bundled with the game and cannot be replaced by an import"
        ));
    }
    if !mods.is_dir() || mods.is_symlink() {
        return Err("The mods folder is missing or is a symbolic link".into());
    }
    sweep(mods);
    let staging = Staging(Some(
        mods.join(format!("{STAGING_PREFIX}{}", files::token())),
    ));
    let root = staging.0.as_ref().unwrap().join(id);
    fs::create_dir(staging.0.as_ref().unwrap()).map_err(|e| e.to_string())?;
    let sums = extract(&mut zip, &names, &root, &mut buffer)?;
    verify_checksums(&root, &sums)?;
    let contents = inventory(&root).map_err(|e| format!("Invalid pack: {e}"))?;
    if contents.pack.pack_id != *id {
        return Err(format!(
            "The archive folder '{id}' does not match pack_id '{}'",
            contents.pack.pack_id
        ));
    }
    // Inventory leaves out unreferenced files; an archive may carry none besides its checksums.
    if let Some(extra) = contents.excluded.iter().find(|path| *path != CHECKSUMS) {
        return Err(format!(
            "{extra} is not referenced by the pack, so export would never include it"
        ));
    }
    let installed = installed(mods, id, &sums)?;
    let mut staging = staging;
    Ok(Staged {
        staging: match installed {
            Installed::Identical => None,
            _ => staging.0.take(),
        },
        pack: contents.pack,
        assets: contents.assets,
        files: sums.len(),
        bytes: contents.bytes,
        installed,
    })
}

// Entry count from the end-of-central-directory record. The zip reader indexes entries by
// name, so it alone cannot reveal duplicates. Zip64 archives (count 0xFFFF) are refused:
// export only writes them for entries far beyond the import limits.
fn declared_entries(file: &mut File) -> Result<usize, String> {
    let length = file.seek(SeekFrom::End(0)).map_err(|e| e.to_string())?;
    let tail = length.min(22 + u64::from(u16::MAX));
    file.seek(SeekFrom::Start(length - tail))
        .map_err(|e| e.to_string())?;
    let mut bytes = Vec::with_capacity(tail as usize);
    file.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    let start = (0..bytes.len().saturating_sub(21))
        .rev()
        .find(|&at| bytes[at..at + 4] == [0x50, 0x4b, 0x05, 0x06])
        .ok_or("Not a readable zip: no end of central directory")?;
    match u16::from_le_bytes([bytes[start + 10], bytes[start + 11]]) {
        u16::MAX => Err(format!("The archive has more than {MAX_ENTRIES} entries")),
        count => Ok(usize::from(count)),
    }
}

// Validated `(pack_id, relative path)` per entry index, all sharing one root folder.
fn entry_names(zip: &mut ZipArchive<File>) -> Result<Vec<(String, String)>, String> {
    let mut names = Vec::with_capacity(zip.len());
    let mut folded = BTreeSet::new();
    for index in 0..zip.len() {
        let entry = zip.by_index_raw(index).map_err(|e| e.to_string())?;
        let name = std::str::from_utf8(entry.name_raw())
            .map_err(|_| "An entry name is not UTF-8")?
            .to_owned();
        if entry.is_dir() || entry.is_symlink() {
            return Err(format!("{name}: archives may hold regular files only"));
        }
        if !matches!(
            entry.compression(),
            CompressionMethod::Stored | CompressionMethod::Deflated
        ) {
            return Err(format!("{name}: unsupported compression"));
        }
        let Some((root, path)) = name.split_once('/') else {
            return Err(format!("{name}: entries must be inside the pack folder"));
        };
        if name.contains('\\')
            || !crate::assets::is_valid_pack_id(root)
            || !path.split('/').all(|part| !matches!(part, "" | "." | ".."))
        {
            return Err(format!("{name}: unsafe or invalid path"));
        }
        if names
            .first()
            .is_some_and(|(first, _): &(String, String)| first != root)
        {
            return Err("The archive must contain exactly one top-level folder".into());
        }
        if !folded.insert(path.to_lowercase()) {
            return Err(format!("{name}: another entry differs only by letter case"));
        }
        names.push((root.to_owned(), path.to_owned()));
    }
    if names.is_empty() {
        return Err("The archive is empty".into());
    }
    Ok(names)
}

// Decompress every entry into `root`, enforcing the limits on the bytes actually produced,
// and return each path's SHA-256.
fn extract(
    zip: &mut ZipArchive<File>,
    names: &[(String, String)],
    root: &Path,
    buffer: &mut [u8],
) -> Result<BTreeMap<String, String>, String> {
    let mut sums = BTreeMap::new();
    let mut total = 0;
    for (index, (_, path)) in names.iter().enumerate() {
        let entry = zip.by_index(index).map_err(|e| format!("{path}: {e}"))?;
        let compressed = entry.compressed_size().max(1);
        let target = root.join(path);
        fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut out = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| format!("{path}: {e}"))?;
        let mut written = 0;
        let mut limited = entry.take(MAX_ENTRY_BYTES + 1);
        let digest = hash_copy(&mut limited, buffer, |chunk| {
            written += chunk.len() as u64;
            if written > MAX_ENTRY_BYTES || total + written > MAX_TOTAL_BYTES {
                return Err(std::io::Error::other("exceeds the import size limits"));
            }
            out.write_all(chunk)
        })
        .map_err(|e| format!("{path}: {e}"))?;
        if written > RATIO_FLOOR && written / compressed > MAX_RATIO {
            return Err(format!(
                "{path}: compression ratio exceeds the import limits"
            ));
        }
        out.sync_all().map_err(|e| e.to_string())?;
        total += written;
        sums.insert(path.clone(), digest);
    }
    Ok(sums)
}

// `checksums.sha256` must list every other extracted entry once, with its real hash.
fn verify_checksums(root: &Path, sums: &BTreeMap<String, String>) -> Result<(), String> {
    let text = fs::read_to_string(root.join(CHECKSUMS))
        .map_err(|_| format!("{CHECKSUMS} is missing or not UTF-8"))?;
    let listed = parse_checksums(&text)?;
    for (path, digest) in &listed {
        match sums.get(*path) {
            Some(actual) if actual == digest => {}
            Some(_) => return Err(format!("{path} does not match {CHECKSUMS}")),
            None => {
                return Err(format!(
                    "{CHECKSUMS} lists {path}, which is not in the archive"
                ));
            }
        }
    }
    if let Some(missing) = sums
        .keys()
        .find(|path| *path != CHECKSUMS && !listed.contains_key(path.as_str()))
    {
        return Err(format!("{missing} is not listed in {CHECKSUMS}"));
    }
    Ok(())
}

// Identical means: every archived file is installed with the same hash, and the installed
// pack's own export set has nothing more. Files export never carries are ignored.
fn installed(mods: &Path, id: &str, sums: &BTreeMap<String, String>) -> Result<Installed, String> {
    let directory = mods.join(id);
    if directory.is_symlink() {
        return Err(format!("mods/{id} is a symbolic link; remove it first"));
    }
    if !directory.exists() {
        return Ok(Installed::Absent);
    }
    let version = fs::read_to_string(directory.join("pack.toml"))
        .ok()
        .and_then(|text| PackManifest::from_str(&text).ok())
        .map(|pack| pack.version);
    let different = Installed::Different {
        version: version.clone(),
    };
    let Ok(current) = inventory(&directory) else {
        return Ok(different);
    };
    if current.files.keys().any(|path| !sums.contains_key(path)) {
        return Ok(different);
    }
    for (path, digest) in sums.iter().filter(|(path, _)| *path != CHECKSUMS) {
        let source = directory.join(path);
        if source.is_symlink() || !source.is_file() || sha256(&source)? != *digest {
            return Ok(different);
        }
    }
    Ok(Installed::Identical)
}

// Resolve a staging folder handed out by [`stage`]: a direct, real child of `mods`.
fn staging_folder(mods: &Path, staging: &Path) -> Result<PathBuf, String> {
    let name = staging
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| name.starts_with(STAGING_PREFIX))
        .ok_or("Not an import staging folder")?;
    let folder = mods.join(name);
    if staging != folder || folder.is_symlink() || !folder.is_dir() {
        return Err("Not an import staging folder".into());
    }
    Ok(folder)
}

/// Rename a staged pack into `mods/<pack_id>`. The caller has already moved any installed
/// copy to Trash; an existing folder is never overwritten.
pub(crate) fn commit(mods: &Path, staging: &Path) -> Result<String, String> {
    let folder = Staging(Some(staging_folder(mods, staging)?));
    let mut packs = fs::read_dir(folder.0.as_ref().unwrap())
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|e| e.file_name()));
    let (Some(Ok(id)), None) = (packs.next(), packs.next()) else {
        return Err("The staging folder does not hold one pack".into());
    };
    let id = id.into_string().map_err(|_| "Invalid staged pack folder")?;
    let target = mods.join(&id);
    if target.exists() || target.is_symlink() {
        return Err(format!(
            "mods/{id} still exists; the import was not installed"
        ));
    }
    fs::rename(folder.0.as_ref().unwrap().join(&id), &target).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Remove staging folders an earlier process left behind, e.g. after a crash. Names carry
/// the creating process id (`files::token`); folders of this process are never touched.
/// Best effort and O(entries in `mods`) plus the removed files.
pub(crate) fn sweep(mods: &Path) {
    let Ok(entries) = fs::read_dir(mods) else {
        return;
    };
    let own = format!("{STAGING_PREFIX}{}-", std::process::id());
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let stale = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age >= STALE_STAGING);
        let kind = entry.file_type().ok();
        if name.starts_with(STAGING_PREFIX)
            && !name.starts_with(&own)
            && stale
            && kind.is_some_and(|kind| kind.is_dir() && !kind.is_symlink())
        {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

/// Remove a staging folder from [`stage`] without installing it.
pub(crate) fn discard(mods: &Path, staging: &Path) -> Result<(), String> {
    fs::remove_dir_all(staging_folder(mods, staging)?).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests;
