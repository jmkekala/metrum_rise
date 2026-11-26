// SPDX-License-Identifier: GPL-3.0-or-later

//! Bundled packs and worlds after a game update (`TOOLS-17`): which installed copies of the
//! game's bundled entries a start replaces, and the install receipt that tells the game's
//! untouched copies from the player's.
//!
//! An entry is a top-level bundled item: a world file `worlds/<name>` or a pack folder
//! `mods/<name>`, as the profile holds it. It is wholly the game's or wholly the player's; never
//! merged file by file. Three inputs decide:
//! - [`MANIFEST`]: `godot/bootstrap/manifest.sha256`, compiled in, one `sha256sum`-form line
//!   per bundled entry. A world's digest is its file's SHA-256; a pack's is [`folder_digest`].
//!   A Rust test recomputes it from `godot/bootstrap` and fails when it is stale.
//! - The receipt [`RECEIPT`] in the profile: per entry, the bundled digest last reconciled
//!   (`revision`) and whose the installed copy is (`owner`, `game` or `player`).
//! - The installed entries themselves, hashed only when the receipt cannot answer.
//!
//! Rust decides and never reads `res://` (an export's bundle is inside the `.pck`); the caller
//! does the file work. Call sequence (wired in round T17-2, `UserDataBootstrap.run`):
//! 1. `plan(MANIFEST, profile)` returns the [`Action`]s in order, plus problems to log.
//! 2. For each action GDScript does its [`Operation`]'s file step, copying from
//!    `res://bootstrap/<path>` into the staging sibling [`Action::staging`] and renaming it to
//!    `user://<path>`, so an interrupted copy is never taken for an installed entry.
//! 3. After each step that succeeded, `record(profile, &action)` writes the receipt atomically. A
//!    failed step is not recorded, so the next start plans it again.
//!
//! Cost: a start with nothing to do reads the receipt and stats two paths per manifest entry,
//! O(entries), and reads no installed file's content. After an update, each entry whose
//! revision changed and whose owner is the game is hashed once, O(bytes of changed entries)
//! plus O(F log F) path ordering per pack. A player's entry is hashed once, when it becomes
//! theirs, and never again.

use super::archive::{self, hex, sha256};
use super::authoring::files;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

/// The bundle manifest: `<sha256 hex>  <worlds|mods>/<name>` per bundled entry, sorted by path.
pub(crate) const MANIFEST: &str = include_str!("../../../godot/bootstrap/manifest.sha256");
/// The install receipt's file name in the profile root (`user://bundled.toml`).
pub(crate) const RECEIPT: &str = "bundled.toml";
/// Prefix of an entry's staging sibling. The leading dot keeps the pack scanner away from it.
pub(crate) const STAGING_PREFIX: &str = ".bundled-";

// The receipt a profile from before the receipt is taken to hold: per name, every world the
// bootstrap seeded under it, by SHA-256 (verified from git). An installed copy with one of these
// digests is the game's untouched copy and is replaced, or removed when no longer bundled. Only
// worlds: a pre-receipt pack that differs from the bundle is trashed (decision D2 (a)).
const LEGACY: &[(&str, &[&str])] = &[
    // The 18,432 m Kuopio with its two narrow rivers (commit 2febfcd0b), never released.
    (
        "worlds/kuopio_18432m_8m.sqlite",
        &["f13cd1350fee43009550ecf0a6f4a81c8715597996e5145d1f6314d91c9e3160"],
    ),
    // The 18,000 m, 10 m Kuopio with hand-made lakes, replaced by kuopio_18432m_8m.sqlite.
    (
        "worlds/kuopio_324km2_10m.sqlite",
        &["07ae5ff39f9c905eccfec41b15ac4aa69de70fba50e96b34a19aeee991cabdbd"],
    ),
    // The 500 m, 10 m blank world; the same name now ships 512 m, 8 m.
    (
        "worlds/test.sqlite",
        &["feb256447173c9aab87b6295c10492f92c008b4ec4d3c250b3fe3f09af070e81"],
    ),
];

/// Whose an installed entry is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Owner {
    /// The game's untouched copy: replaced when the bundle changes.
    Game,
    /// The player changed it (or had it before the game bundled the name): never replaced and
    /// never hashed again; deleting it brings the game's version back.
    Player,
}

/// One receipt entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Record {
    /// The bundled digest last reconciled: for a game entry, also its installed digest.
    pub(crate) revision: String,
    /// Whose the installed copy is.
    pub(crate) owner: Owner,
}

#[derive(Default, Serialize, Deserialize)]
struct ReceiptFile {
    #[serde(default)]
    entries: BTreeMap<String, Record>,
}

/// What the caller does for one entry. Paths are relative to `res://bootstrap/` (the bundled
/// copy) and to the profile root (the installed copy and its staging sibling).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operation {
    /// Remove the staging sibling an interrupted copy left. The receipt does not change.
    RemoveStaging,
    /// The entry is absent: copy the bundled one in. Records the game's revision.
    Seed,
    /// The installed copy is the game's untouched older one: copy the bundled one into staging,
    /// delete the installed copy, rename staging into place. Records the game's revision.
    Replace,
    /// A pack from before the receipt that differs from the bundle (decision D2 (a)): as
    /// [`Operation::Replace`], but the installed folder goes to the system Trash.
    TrashAndReplace,
    /// The game's untouched copy of a world it no longer bundles: delete it. Drops the record.
    Remove,
    /// The installed copy already equals the bundle: nothing on disk. Records the game's revision.
    Adopt,
    /// The player's copy stays and stops updating: nothing on disk, one debug-channel log line
    /// (decision D3 (a)). Records the new revision as the player's.
    KeepPlayers,
    /// The entry is no longer bundled and stays (a pack, a changed world) or is gone: nothing
    /// on disk. Drops the record.
    Forget,
}

impl Operation {
    const ALL: [Self; 8] = [
        Self::RemoveStaging,
        Self::Seed,
        Self::Replace,
        Self::TrashAndReplace,
        Self::Remove,
        Self::Adopt,
        Self::KeepPlayers,
        Self::Forget,
    ];

    /// The name the bridge passes to GDScript and back.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::RemoveStaging => "remove_staging",
            Self::Seed => "seed",
            Self::Replace => "replace",
            Self::TrashAndReplace => "trash_and_replace",
            Self::Remove => "remove",
            Self::Adopt => "adopt",
            Self::KeepPlayers => "keep_players",
            Self::Forget => "forget",
        }
    }

    /// The operation of a [`Operation::name`].
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|op| op.name() == name)
    }

    // The receipt change once the step is done: Some(Some(owner)) sets the record,
    // Some(None) drops it, None leaves the receipt alone.
    fn change(self) -> Option<Option<Owner>> {
        match self {
            Self::RemoveStaging => None,
            Self::Seed | Self::Replace | Self::TrashAndReplace | Self::Adopt => {
                Some(Some(Owner::Game))
            }
            Self::KeepPlayers => Some(Some(Owner::Player)),
            Self::Remove | Self::Forget => Some(None),
        }
    }

    // Order of execution: leftover staging first, then disk changes, then receipt-only steps,
    // so a crash part way leaves as few entries recorded ahead of their files as possible.
    fn phase(self) -> u8 {
        match self {
            Self::RemoveStaging => 0,
            Self::Seed | Self::Replace | Self::TrashAndReplace | Self::Remove => 1,
            Self::Adopt | Self::KeepPlayers | Self::Forget => 2,
        }
    }
}

/// One planned step for one entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Action {
    /// The entry, `worlds/<file>` or `mods/<folder>`.
    pub(crate) path: String,
    /// What to do.
    pub(crate) operation: Operation,
    /// The manifest digest the receipt records after the step; empty when the entry is not
    /// bundled ([`Operation::Remove`], [`Operation::Forget`] of a dropped entry).
    pub(crate) revision: String,
}

impl Action {
    /// True for a pack folder, false for a world file.
    pub(crate) fn is_pack(&self) -> bool {
        self.path.starts_with("mods/")
    }

    /// The staging sibling copies go through: `mods/.bundled-<name>` for `mods/<name>`.
    pub(crate) fn staging(&self) -> String {
        let (folder, name) = self.path.split_once('/').unwrap_or(("", &self.path));
        format!("{folder}/{STAGING_PREFIX}{name}")
    }
}

/// The steps one start takes, and what it could not decide (an unreadable receipt or installed
/// copy), for the caller to log. An entry with a problem gets no step and is planned again at
/// the next start.
#[derive(Debug, Default)]
pub(crate) struct Plan {
    /// Steps in execution order.
    pub(crate) actions: Vec<Action>,
    /// One message per problem.
    pub(crate) problems: Vec<String>,
}

/// Plan the start-up reconciliation of the profile at `user_dir` against `manifest` (the
/// [`MANIFEST`] text in the game). Writes nothing. Errors only on a malformed manifest.
pub(crate) fn plan(manifest: &str, user_dir: &Path) -> Result<Plan, String> {
    plan_with(manifest, LEGACY, user_dir, &mut installed_digest)
}

// The digest of an installed entry: Ok(None) when it is not of the entry's kind (a symbolic
// link, a file where a pack folder belongs), which compares unequal to every digest.
type Digester<'a> = dyn FnMut(&Path, bool) -> Result<Option<String>, String> + 'a;

fn plan_with(
    manifest: &str,
    legacy: &[(&str, &[&str])],
    user_dir: &Path,
    digest: &mut Digester,
) -> Result<Plan, String> {
    let manifest = parse_manifest(manifest)?;
    let mut planner = Planner {
        user_dir,
        legacy: None,
        digest,
        plan: Plan::default(),
    };
    let receipt = match read_receipt(user_dir) {
        Ok(Some(entries)) => entries,
        Ok(None) => {
            planner.legacy = Some(legacy);
            BTreeMap::new()
        }
        // An unreadable receipt is taken as an empty one, not as a profile from before the
        // receipt: every installed entry is hashed and a differing one kept, never trashed.
        Err(problem) => {
            planner.plan.problems.push(problem);
            BTreeMap::new()
        }
    };
    for (&path, &bundled) in &manifest {
        planner.bundled(path, bundled, receipt.get(path));
    }
    for (path, record) in &receipt {
        if !manifest.contains_key(path.as_str()) {
            planner.dropped(path, record);
        }
    }
    if let Some(legacy) = planner.legacy {
        for &(path, digests) in legacy {
            if !manifest.contains_key(path) {
                planner.dropped_legacy(path, digests);
            }
        }
    }
    let mut plan = planner.plan;
    plan.actions.sort_by_key(|action| action.operation.phase());
    Ok(plan)
}

struct Planner<'a, 'd> {
    user_dir: &'a Path,
    // The legacy table when the profile has no receipt.
    legacy: Option<&'a [(&'a str, &'a [&'a str])]>,
    digest: &'a mut Digester<'d>,
    plan: Plan,
}

impl Planner<'_, '_> {
    fn push(&mut self, path: &str, operation: Operation, revision: &str) {
        self.plan.actions.push(Action {
            path: path.to_owned(),
            operation,
            revision: revision.to_owned(),
        });
    }

    // Hash the installed entry; a failure becomes a problem and the entry gets no step.
    fn installed(&mut self, path: &str) -> Option<Option<String>> {
        (self.digest)(&self.user_dir.join(path), path.starts_with("mods/"))
            .map_err(|e| self.plan.problems.push(format!("{path}: {e}")))
            .ok()
    }

    // An entry the manifest lists, with digest `bundled`. The table:
    //   absent                                  -> Seed
    //   recorded at `bundled`                   -> nothing, no hashing
    //   recorded, owner player                  -> KeepPlayers (new revision), no hashing
    //   recorded, owner game: installed equals
    //     the bundle                            -> Adopt
    //     the record                            -> Replace
    //     neither                               -> KeepPlayers
    //   hashed, not of the entry's kind (a link) -> KeepPlayers
    //   unrecorded: installed equals the bundle -> Adopt
    //     no receipt, a legacy world digest     -> Replace
    //     no receipt, a pack                    -> TrashAndReplace (D2 (a))
    //     otherwise (a world, or a receipt)     -> KeepPlayers
    fn bundled(&mut self, path: &str, bundled: &str, record: Option<&Record>) {
        let staging = Action {
            path: path.to_owned(),
            operation: Operation::RemoveStaging,
            revision: String::new(),
        };
        if present(&self.user_dir.join(staging.staging())) {
            self.plan.actions.push(staging);
        }
        if !present(&self.user_dir.join(path)) {
            return self.push(path, Operation::Seed, bundled);
        }
        let operation = match record {
            Some(record) if record.revision == bundled => return,
            Some(record) if record.owner == Owner::Player => Operation::KeepPlayers,
            _ => {
                let Some(installed) = self.installed(path) else {
                    return;
                };
                // Not of the entry's kind (a symbolic link): the player's, whatever else holds.
                let Some(installed) = installed else {
                    return self.push(path, Operation::KeepPlayers, bundled);
                };
                if installed == bundled {
                    Operation::Adopt
                } else if let Some(record) = record {
                    if installed == record.revision {
                        Operation::Replace
                    } else {
                        Operation::KeepPlayers
                    }
                } else if let Some(legacy) = self.legacy {
                    if path.starts_with("mods/") {
                        Operation::TrashAndReplace
                    } else if shipped(legacy, path, &installed) {
                        Operation::Replace
                    } else {
                        Operation::KeepPlayers
                    }
                } else {
                    Operation::KeepPlayers
                }
            }
        };
        self.push(path, operation, bundled);
    }

    // A receipt entry the manifest no longer lists: an untouched game world is removed; a pack
    // (D4 (a)), a changed or player's world, a missing entry or a malformed key is forgotten.
    fn dropped(&mut self, path: &str, record: &Record) {
        let removable = entry_path(path)
            && path.starts_with("worlds/")
            && record.owner == Owner::Game
            && present(&self.user_dir.join(path));
        let operation = if !removable {
            Operation::Forget
        } else {
            match self.installed(path) {
                None => return,
                Some(Some(installed)) if installed == record.revision => Operation::Remove,
                Some(_) => Operation::Forget,
            }
        };
        self.push(path, operation, "");
    }

    // A legacy world no longer bundled, in a profile without a receipt: removed when untouched.
    fn dropped_legacy(&mut self, path: &str, digests: &[&str]) {
        if !path.starts_with("worlds/") || !present(&self.user_dir.join(path)) {
            return;
        }
        if let Some(Some(installed)) = self.installed(path)
            && digests.contains(&installed.as_str())
        {
            self.push(path, Operation::Remove, "");
        }
    }
}

fn shipped(legacy: &[(&str, &[&str])], path: &str, digest: &str) -> bool {
    legacy
        .iter()
        .any(|&(name, digests)| name == path && digests.contains(&digest))
}

// An existing entry of any kind, a dangling symbolic link included.
fn present(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

fn installed_digest(path: &Path, pack: bool) -> Result<Option<String>, String> {
    let kind = fs::symlink_metadata(path)
        .map_err(|e| e.to_string())?
        .file_type();
    if pack && kind.is_dir() {
        folder_digest(path).map(Some)
    } else if !pack && kind.is_file() {
        sha256(path).map(Some)
    } else {
        Ok(None)
    }
}

/// Whether the bootstrap copies an entry of this name: not `.`-prefixed (staging, `.keep`) and
/// not a Godot `.import` sidecar. Mirrors `UserDataBootstrap._should_seed_entry`.
pub(crate) fn seeds(name: &str) -> bool {
    !name.starts_with('.') && !name.ends_with(".import")
}

/// A pack folder's digest: the SHA-256 of its `sha256sum` lines (`<hex>  <path>\n`, paths
/// relative with `/`, sorted bytewise) over every regular file the bootstrap copies
/// ([`seeds`] at every level; symbolic links are not followed). It equals
/// `find . -type f ... | LC_ALL=C sort | xargs sha256sum | sha256sum` run inside the folder
/// with the same exclusions. O(files + bytes) plus O(F log F) ordering.
pub(crate) fn folder_digest(root: &Path) -> Result<String, String> {
    let mut found = Vec::new();
    collect_files(root, "", &mut found)?;
    found.sort_unstable();
    let mut lines = String::new();
    for relative in &found {
        let digest = sha256(&root.join(relative))?;
        let _ = writeln!(lines, "{digest}  {relative}");
    }
    Ok(hex(&Sha256::digest(lines.as_bytes())))
}

fn collect_files(directory: &Path, prefix: &str, found: &mut Vec<String>) -> Result<(), String> {
    let entries = fs::read_dir(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !seeds(&name) {
            continue;
        }
        let relative = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        // `file_type` does not follow symbolic links, so a link is neither.
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_dir() {
            collect_files(&entry.path(), &relative, found)?;
        } else if kind.is_file() {
            found.push(relative);
        }
    }
    Ok(())
}

// `worlds/<name>` or `mods/<name>`: one safe name the bootstrap copies.
fn entry_path(path: &str) -> bool {
    files::safe_relative(path)
        && path.split_once('/').is_some_and(|(folder, name)| {
            matches!(folder, "worlds" | "mods") && !name.contains('/') && seeds(name)
        })
}

fn parse_manifest(text: &str) -> Result<BTreeMap<&str, &str>, String> {
    let entries = archive::parse_sha256_lines(text, "manifest.sha256")?;
    match entries.keys().find(|path| !entry_path(path)) {
        Some(path) => Err(format!(
            "manifest.sha256: '{path}' is not worlds/<name> or mods/<name>"
        )),
        None => Ok(entries),
    }
}

/// The receipt at `user_dir`: Ok(None) when the profile has none (a profile from before it, or
/// a fresh one), Err when it cannot be read or parsed.
pub(crate) fn read_receipt(user_dir: &Path) -> Result<Option<BTreeMap<String, Record>>, String> {
    let text = match fs::read_to_string(user_dir.join(RECEIPT)) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{RECEIPT}: {e}")),
    };
    toml::from_str::<ReceiptFile>(&text)
        .map(|file| Some(file.entries))
        .map_err(|e| format!("{RECEIPT}: {e}"))
}

/// Record a finished `action` in the receipt at `user_dir`, written atomically (readers see the
/// old or the new receipt). An unreadable receipt is replaced by one holding this entry. Refuses
/// to set a record for a path that is not `worlds/<name>` or `mods/<name>` or a revision that
/// is not a SHA-256.
/// O(receipt entries).
pub(crate) fn record(user_dir: &Path, action: &Action) -> Result<(), String> {
    let Some(owner) = action.operation.change() else {
        return Ok(());
    };
    let mut entries = read_receipt(user_dir).ok().flatten().unwrap_or_default();
    match owner {
        // Any key may be dropped, a malformed one included; only entries are set.
        Some(_) if !entry_path(&action.path) => {
            return Err(format!("'{}' is not a bundled entry", action.path));
        }
        Some(owner) => {
            let digest = action.revision.as_bytes();
            if digest.len() != 64
                || !digest
                    .iter()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            {
                return Err(format!("'{}' is not a SHA-256 revision", action.revision));
            }
            let record = Record {
                revision: action.revision.clone(),
                owner,
            };
            entries.insert(action.path.clone(), record);
        }
        None => {
            entries.remove(&action.path);
        }
    }
    let body = toml::to_string(&ReceiptFile { entries }).map_err(|e| e.to_string())?;
    let text = format!(
        "# The bundled packs and worlds this profile holds, as the game last reconciled them.\n\
         # Written by the game at start-up (TOOLS-17). Delete an entry's pack folder or world\n\
         # file to get the game's version back at the next start.\n\n{body}"
    );
    files::write_atomic(&user_dir.join(RECEIPT), text.as_bytes())
        .map_err(|e| format!("{RECEIPT} was not written: {e}"))
}

#[cfg(test)]
mod tests;
