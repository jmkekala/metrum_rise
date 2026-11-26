// SPDX-License-Identifier: GPL-3.0-or-later

//! The bundled-entry table in a scratch profile: every row, convergence after the caller's
//! steps, the receipt's format, the legacy table and the committed manifest's freshness.

use super::*;
use std::path::PathBuf;

const OLD_KUOPIO: &str = "worlds/old_kuopio.sqlite";

// A scratch folder with a fake bundle (`bundle/`), a profile (`profile/`) and a Trash.
struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("metrum-bundled-{}", files::token()));
        fs::create_dir_all(root.join("profile")).unwrap();
        Self(root)
    }
    fn write(&self, relative: &str, data: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
    fn read(&self, relative: &str) -> String {
        fs::read_to_string(self.0.join(relative)).unwrap()
    }
    fn exists(&self, relative: &str) -> bool {
        present(&self.0.join(relative))
    }
    fn profile(&self) -> PathBuf {
        self.0.join("profile")
    }
    // Bundle v1: a pack with a file in a subfolder and the files the bootstrap skips, two worlds.
    fn bundle_v1(&self) {
        self.write("bundle/mods/pack/pack.toml", "pack v1");
        self.write("bundle/mods/pack/assets/a/model.glb", "model v1");
        self.write("bundle/mods/pack/assets/a/model.glb.import", "sidecar");
        self.write("bundle/mods/pack/.hidden/x", "hidden");
        self.write("bundle/mods/.keep", "keep");
        self.write("bundle/worlds/city.sqlite", "city v1");
        self.write("bundle/worlds/blank.sqlite", "blank v1");
        self.write("bundle/worlds/city.sqlite.import", "sidecar");
    }
    fn manifest(&self) -> String {
        manifest_of(&self.0.join("bundle")).unwrap()
    }
    // Plan against the fake bundle with a test legacy table; returns the plan and the number of
    // installed entries hashed.
    fn plan_counted(&self, legacy: &[(&str, &[&str])]) -> (Plan, usize) {
        let mut hashed = 0;
        let mut digest = |path: &Path, pack: bool| {
            hashed += 1;
            installed_digest(path, pack)
        };
        let plan = plan_with(&self.manifest(), legacy, &self.profile(), &mut digest).unwrap();
        (plan, hashed)
    }
    fn plan(&self) -> Plan {
        let (plan, _) = self.plan_counted(&[]);
        assert!(plan.problems.is_empty(), "{:?}", plan.problems);
        plan
    }
    // What `UserDataBootstrap` does: each step's bundle side (the Trash, the copy into
    // staging), then `commit`; once all were tried, one `record` of those that succeeded.
    fn apply(&self, plan: &Plan) {
        let profile = self.profile();
        for action in &plan.actions {
            self.bundle_side(action);
            commit(&profile, action).unwrap();
        }
        record(&profile, &plan.actions).unwrap();
    }
    fn bundle_side(&self, action: &Action) {
        let installed = self.profile().join(&action.path);
        if action.operation == Operation::TrashAndReplace {
            fs::create_dir_all(self.0.join("trash")).unwrap();
            let name = installed.file_name().unwrap();
            fs::rename(&installed, self.0.join("trash").join(name)).unwrap();
        }
        if matches!(
            action.operation,
            Operation::Seed | Operation::Replace | Operation::TrashAndReplace
        ) {
            let staging = self.profile().join(action.staging());
            copy(&self.0.join("bundle").join(&action.path), &staging);
        }
    }
    // Install the bundle into the profile as a fresh start would.
    fn install(&self) {
        let plan = self.plan();
        self.apply(&plan);
    }
    fn receipt(&self) -> BTreeMap<String, Record> {
        read_receipt(&self.profile()).unwrap().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn copy(source: &Path, target: &Path) {
    if source.is_dir() {
        fs::create_dir_all(target).unwrap();
        for entry in fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            if seeds(&entry.file_name().to_string_lossy()) {
                copy(&entry.path(), &target.join(entry.file_name()));
            }
        }
    } else {
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::copy(source, target).unwrap();
    }
}

fn ops(plan: &Plan) -> Vec<(&str, Operation)> {
    plan.actions
        .iter()
        .map(|action| (action.path.as_str(), action.operation))
        .collect()
}

fn sha(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

// The manifest of a bootstrap folder: a line per seeded entry of `mods/` (folders) and
// `worlds/` (files), sorted by path. A file in `mods/` or a folder in `worlds/` is refused: the
// caller tells packs from worlds by the folder.
fn manifest_of(bootstrap: &Path) -> Result<String, String> {
    let mut lines = Vec::new();
    for (folder, pack) in [("mods", true), ("worlds", false)] {
        for entry in fs::read_dir(bootstrap.join(folder)).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !seeds(&name) {
                continue;
            }
            let path = format!("{folder}/{name}");
            let digest = installed_digest(&entry.path(), pack)?
                .ok_or_else(|| format!("{path}: a pack must be a folder, a world a file"))?;
            lines.push(format!("{digest}  {path}\n"));
        }
    }
    lines.sort_by(|a, b| a[66..].cmp(&b[66..]));
    Ok(lines.concat())
}

// Seeded files under `folder` that are Git LFS pointer stubs rather than their content.
fn lfs_stubs(folder: &Path) -> Vec<String> {
    let mut found = Vec::new();
    collect_files(folder, "", &mut found).unwrap();
    found.retain(|relative| {
        let mut head = [0u8; 24];
        let mut file = fs::File::open(folder.join(relative)).unwrap();
        let count = std::io::Read::read(&mut file, &mut head).unwrap();
        head[..count].starts_with(b"version https://git-lfs")
    });
    found
}

#[test]
fn a_fresh_profile_seeds_every_bundled_entry_and_then_plans_nothing() {
    let fx = Fixture::new();
    fx.bundle_v1();
    let plan = fx.plan();
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::Seed),
            ("worlds/blank.sqlite", Operation::Seed),
            ("worlds/city.sqlite", Operation::Seed),
        ]
    );
    fx.apply(&plan);
    assert_eq!(fx.read("profile/worlds/city.sqlite"), "city v1");
    assert_eq!(fx.read("profile/mods/pack/assets/a/model.glb"), "model v1");
    assert!(!fx.exists("profile/mods/pack/assets/a/model.glb.import"));
    assert!(!fx.exists("profile/mods/pack/.hidden"));
    assert!(!fx.exists("profile/mods/.bundled-pack"));
    let receipt = fx.receipt();
    assert_eq!(receipt.len(), 3);
    assert!(receipt.values().all(|record| record.owner == Owner::Game));
    assert!(fx.plan().actions.is_empty());
}

#[cfg(unix)]
#[test]
fn an_unchanged_profile_plans_nothing_and_reads_no_installed_file() {
    use std::os::unix::fs::PermissionsExt;
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    // Unreadable copies: hashing either would report a problem.
    let locked = [
        fx.profile().join("mods/pack"),
        fx.profile().join("worlds/city.sqlite"),
        fx.profile().join("worlds/blank.sqlite"),
    ];
    for path in &locked {
        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
    }
    let real = plan(&fx.manifest(), &fx.profile()).unwrap();
    let (counted, hashed) = fx.plan_counted(LEGACY);
    for path in &locked {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(
        real.actions.is_empty() && real.problems.is_empty(),
        "{real:?}"
    );
    assert!(counted.actions.is_empty());
    assert_eq!(hashed, 0);
}

#[test]
fn untouched_copies_are_replaced_when_the_bundle_changes() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    // v2: a pack file changed, one added, one removed; one world changed.
    fx.write("bundle/mods/pack/pack.toml", "pack v2");
    fx.write("bundle/mods/pack/assets/b/new.glb", "new");
    fs::remove_file(fx.0.join("bundle/mods/pack/assets/a/model.glb")).unwrap();
    fx.write("bundle/worlds/city.sqlite", "city v2");
    let (plan, hashed) = fx.plan_counted(&[]);
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::Replace),
            ("worlds/city.sqlite", Operation::Replace),
        ]
    );
    assert_eq!(hashed, 2, "only the two changed entries are hashed");
    fx.apply(&plan);
    assert_eq!(fx.read("profile/mods/pack/pack.toml"), "pack v2");
    assert_eq!(fx.read("profile/mods/pack/assets/b/new.glb"), "new");
    assert!(!fx.exists("profile/mods/pack/assets/a/model.glb"));
    assert_eq!(fx.read("profile/worlds/city.sqlite"), "city v2");
    assert!(fx.plan().actions.is_empty());
}

#[test]
fn an_edited_copy_becomes_the_players_and_stays_through_two_revisions() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fx.write("profile/mods/pack/assets/a/model.glb", "the player's model");
    fx.write("profile/worlds/city.sqlite", "the player's city");
    // An edit with no bundle change is not noticed: nothing hashed.
    let (plan, hashed) = fx.plan_counted(&[]);
    assert!(plan.actions.is_empty());
    assert_eq!(hashed, 0);
    // v2 hashes each changed entry once and keeps the edits.
    fx.write("bundle/mods/pack/pack.toml", "pack v2");
    fx.write("bundle/worlds/city.sqlite", "city v2");
    let (plan, hashed) = fx.plan_counted(&[]);
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::KeepPlayers),
            ("worlds/city.sqlite", Operation::KeepPlayers),
        ]
    );
    assert_eq!(hashed, 2);
    fx.apply(&plan);
    let receipt = fx.receipt();
    assert_eq!(receipt["mods/pack"].owner, Owner::Player);
    assert_eq!(receipt["worlds/city.sqlite"].owner, Owner::Player);
    // v3 records the new revision without hashing; v3 again plans nothing.
    fx.write("bundle/mods/pack/pack.toml", "pack v3");
    fx.write("bundle/worlds/city.sqlite", "city v3");
    let (plan, hashed) = fx.plan_counted(&[]);
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::KeepPlayers),
            ("worlds/city.sqlite", Operation::KeepPlayers),
        ]
    );
    assert_eq!(hashed, 0);
    fx.apply(&plan);
    assert!(fx.plan().actions.is_empty());
    assert_eq!(
        fx.read("profile/mods/pack/assets/a/model.glb"),
        "the player's model"
    );
    assert_eq!(fx.read("profile/worlds/city.sqlite"), "the player's city");
    assert_eq!(fx.receipt()["mods/pack"].revision, fx.manifest()[..64]);
}

// Options > Mods > Restore: the player's copy goes to the Trash, the game's version is
// installed through the start-up's own step and the entry is the game's again.
#[test]
fn restore_trashes_the_players_copy_and_installs_the_games_version() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fx.write("profile/mods/pack/assets/a/model.glb", "the player's model");
    fx.write("bundle/mods/pack/pack.toml", "pack v2");
    let plan = fx.plan();
    fx.apply(&plan);
    assert_eq!(fx.receipt()["mods/pack"].owner, Owner::Player);
    let manifest = fx.manifest();
    assert_eq!(restorable(&manifest, &fx.profile()), ["pack"]);

    let action = restore(&manifest, &fx.profile(), "pack").unwrap();
    assert_eq!(action.operation, Operation::TrashAndReplace);
    fx.bundle_side(&action);
    commit(&fx.profile(), &action).unwrap();
    record(&fx.profile(), std::slice::from_ref(&action)).unwrap();

    assert_eq!(
        fx.read("trash/pack/assets/a/model.glb"),
        "the player's model"
    );
    assert_eq!(
        folder_digest(&fx.profile().join("mods/pack")).unwrap(),
        parse_manifest(&manifest).unwrap()["mods/pack"]
    );
    assert!(!fx.exists("profile/mods/.bundled-pack"));
    let receipt = fx.receipt();
    assert_eq!(receipt["mods/pack"].owner, Owner::Game);
    assert_eq!(receipt["mods/pack"].revision, action.revision);
    assert_eq!(receipt["worlds/city.sqlite"].owner, Owner::Game);
    assert!(restorable(&manifest, &fx.profile()).is_empty());
    // The next start has nothing to do, and the bundle's next change replaces the copy.
    assert!(fx.plan().actions.is_empty());
    fx.write("bundle/mods/pack/pack.toml", "pack v3");
    assert_eq!(ops(&fx.plan()), [("mods/pack", Operation::Replace)]);
}

// Without the Trash the caller stops before staging: nothing changes. A commit without the
// Trash step refuses to touch the player's copy.
#[test]
fn a_restore_whose_trash_fails_leaves_everything_as_it_was() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fx.write("profile/mods/pack/assets/a/model.glb", "the player's model");
    fx.write("bundle/mods/pack/pack.toml", "pack v2");
    let plan = fx.plan();
    fx.apply(&plan);
    let receipt = fx.read("profile/bundled.toml");
    let manifest = fx.manifest();
    let action = restore(&manifest, &fx.profile(), "pack").unwrap();
    // The caller's Trash failed: no staging, no commit, no record.
    assert_eq!(fx.read("profile/bundled.toml"), receipt);
    assert_eq!(restorable(&manifest, &fx.profile()), ["pack"]);
    // Staged but not trashed: the commit refuses and the copy stays.
    copy(
        &fx.0.join("bundle/mods/pack"),
        &fx.profile().join(action.staging()),
    );
    let error = commit(&fx.profile(), &action).unwrap_err();
    assert!(error.contains("nothing was replaced"), "{error}");
    assert_eq!(
        fx.read("profile/mods/pack/assets/a/model.glb"),
        "the player's model"
    );
    assert_eq!(fx.read("profile/bundled.toml"), receipt);
    // That leftover staging copy refuses a restore until a start cleans it.
    let error = restore(&manifest, &fx.profile(), "pack").unwrap_err();
    assert!(error.contains("unfinished"), "{error}");
    assert!(restorable(&manifest, &fx.profile()).is_empty());
}

#[test]
fn restore_refuses_what_is_not_the_players_bundled_pack() {
    let fx = Fixture::new();
    fx.bundle_v1();
    let manifest = fx.manifest();
    // No receipt yet.
    fx.write("profile/mods/pack/pack.toml", "the player's");
    assert!(restore(&manifest, &fx.profile(), "pack").is_err());
    fs::remove_dir_all(fx.profile().join("mods/pack")).unwrap();
    fx.install();
    for (name, refusal) in [
        ("pack", "the game's copy"),
        ("other", "not a pack the game bundles"),
        ("../worlds/city.sqlite", "not a pack the game bundles"),
        ("city.sqlite", "not a pack the game bundles"),
    ] {
        let error = restore(&manifest, &fx.profile(), name).unwrap_err();
        assert!(error.contains(refusal), "{name}: {error}");
    }
    // A player's world is no pack: never offered.
    fx.write("profile/worlds/city.sqlite", "the player's city");
    fx.write("bundle/worlds/city.sqlite", "city v2");
    let plan = fx.plan();
    fx.apply(&plan);
    assert_eq!(fx.receipt()["worlds/city.sqlite"].owner, Owner::Player);
    assert!(restorable(&fx.manifest(), &fx.profile()).is_empty());
    // A player's pack that is a file, not a folder.
    fs::remove_dir_all(fx.profile().join("mods/pack")).unwrap();
    fx.write("profile/mods/pack", "a file");
    fx.write("bundle/mods/pack/pack.toml", "pack v2");
    let plan = fx.plan();
    fx.apply(&plan);
    assert_eq!(fx.receipt()["mods/pack"].owner, Owner::Player);
    let error = restore(&fx.manifest(), &fx.profile(), "pack").unwrap_err();
    assert!(error.contains("not an installed pack folder"), "{error}");
}

#[test]
fn a_deleted_entry_is_seeded_again_even_when_it_was_the_players() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fx.write("profile/worlds/city.sqlite", "the player's city");
    fx.write("bundle/worlds/city.sqlite", "city v2");
    let plan = fx.plan();
    fx.apply(&plan);
    assert_eq!(fx.receipt()["worlds/city.sqlite"].owner, Owner::Player);
    fs::remove_file(fx.profile().join("worlds/city.sqlite")).unwrap();
    fs::remove_dir_all(fx.profile().join("mods/pack")).unwrap();
    let plan = fx.plan();
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::Seed),
            ("worlds/city.sqlite", Operation::Seed),
        ]
    );
    fx.apply(&plan);
    assert_eq!(fx.read("profile/worlds/city.sqlite"), "city v2");
    assert_eq!(fx.receipt()["worlds/city.sqlite"].owner, Owner::Game);
}

#[test]
fn an_update_already_installed_is_adopted() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    // A copy step that finished before its record (a crash), or a player who copied v2 in.
    fx.write("bundle/worlds/city.sqlite", "city v2");
    fx.write("profile/worlds/city.sqlite", "city v2");
    let plan = fx.plan();
    assert_eq!(ops(&plan), [("worlds/city.sqlite", Operation::Adopt)]);
    fx.apply(&plan);
    assert_eq!(fx.receipt()["worlds/city.sqlite"].owner, Owner::Game);
    assert!(fx.plan().actions.is_empty());
}

#[test]
fn a_world_no_longer_bundled_is_removed_only_when_untouched() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fx.write("profile/worlds/blank.sqlite", "the player's blank");
    fs::remove_file(fx.0.join("bundle/worlds/city.sqlite")).unwrap();
    fs::remove_file(fx.0.join("bundle/worlds/blank.sqlite")).unwrap();
    let plan = fx.plan();
    assert_eq!(
        ops(&plan),
        [
            ("worlds/city.sqlite", Operation::Remove),
            ("worlds/blank.sqlite", Operation::Forget),
        ]
    );
    fx.apply(&plan);
    assert!(!fx.exists("profile/worlds/city.sqlite"));
    assert_eq!(fx.read("profile/worlds/blank.sqlite"), "the player's blank");
    assert_eq!(fx.receipt().keys().collect::<Vec<_>>(), ["mods/pack"]);
    assert!(fx.plan().actions.is_empty());
}

#[test]
fn a_players_world_no_longer_bundled_is_forgotten_unhashed() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fx.write("profile/worlds/city.sqlite", "the player's city");
    fx.write("bundle/worlds/city.sqlite", "city v2");
    let plan = fx.plan();
    fx.apply(&plan);
    fs::remove_file(fx.0.join("bundle/worlds/city.sqlite")).unwrap();
    let (plan, hashed) = fx.plan_counted(&[]);
    assert_eq!(ops(&plan), [("worlds/city.sqlite", Operation::Forget)]);
    assert_eq!(hashed, 0);
}

#[test]
fn a_pack_no_longer_bundled_stays_installed() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fs::remove_dir_all(fx.0.join("bundle/mods/pack")).unwrap();
    let (plan, hashed) = fx.plan_counted(&[]);
    assert_eq!(ops(&plan), [("mods/pack", Operation::Forget)]);
    assert_eq!(hashed, 0);
    fx.apply(&plan);
    assert_eq!(fx.read("profile/mods/pack/pack.toml"), "pack v1");
    assert!(fx.plan().actions.is_empty());
}

#[test]
fn an_unrecorded_entry_beside_a_receipt_is_adopted_when_equal_and_kept_otherwise() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    // The bundle gains a pack and a world the profile already holds under those names.
    fx.write("bundle/mods/same/pack.toml", "same");
    fx.write("profile/mods/same/pack.toml", "same");
    fx.write("bundle/mods/named/pack.toml", "the game's");
    fx.write("profile/mods/named/pack.toml", "the player's own pack");
    fx.write("bundle/worlds/named.sqlite", "the game's");
    fx.write("profile/worlds/named.sqlite", "the player's own world");
    let plan = fx.plan();
    assert_eq!(
        ops(&plan),
        [
            ("mods/named", Operation::KeepPlayers),
            ("mods/same", Operation::Adopt),
            ("worlds/named.sqlite", Operation::KeepPlayers),
        ]
    );
    fx.apply(&plan);
    assert_eq!(
        fx.read("profile/mods/named/pack.toml"),
        "the player's own pack"
    );
    assert!(!fx.exists("trash"));
}

#[test]
fn a_profile_without_receipt_takes_the_legacy_table() {
    let fx = Fixture::new();
    fx.bundle_v1();
    // A profile seeded by copy-if-missing: untouched old worlds, a changed one, a stale pack.
    fx.write("profile/worlds/blank.sqlite", "blank v0");
    fx.write(
        "profile/worlds/city.sqlite",
        "city v0 the player saved over",
    );
    fx.write("profile/mods/pack/pack.toml", "pack v0");
    fx.write("profile/worlds/old_kuopio.sqlite", "old kuopio");
    let legacy_blank = sha("blank v0");
    let legacy_city = sha("city v0");
    let legacy_kuopio = sha("old kuopio");
    let legacy: &[(&str, &[&str])] = &[
        ("worlds/blank.sqlite", &[legacy_blank.as_str()]),
        ("worlds/city.sqlite", &[legacy_city.as_str()]),
        (OLD_KUOPIO, &[legacy_kuopio.as_str()]),
    ];
    let (plan, _) = fx.plan_counted(legacy);
    assert!(plan.problems.is_empty());
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::TrashAndReplace),
            ("worlds/blank.sqlite", Operation::Replace),
            (OLD_KUOPIO, Operation::Remove),
            // The open point: a changed world under a bundled name stays the player's.
            ("worlds/city.sqlite", Operation::KeepPlayers),
        ]
    );
    fx.apply(&plan);
    assert_eq!(fx.read("trash/pack/pack.toml"), "pack v0");
    assert_eq!(fx.read("profile/mods/pack/pack.toml"), "pack v1");
    assert_eq!(fx.read("profile/worlds/blank.sqlite"), "blank v1");
    assert!(!fx.exists("profile/worlds/old_kuopio.sqlite"));
    assert_eq!(
        fx.read("profile/worlds/city.sqlite"),
        "city v0 the player saved over"
    );
    // The receipt now exists: the legacy table and D2 no longer apply.
    fx.write("profile/worlds/old_kuopio.sqlite", "old kuopio");
    let (plan, hashed) = fx.plan_counted(legacy);
    assert!(plan.actions.is_empty());
    assert_eq!(hashed, 0);
}

#[test]
fn a_profile_without_receipt_keeps_changed_legacy_worlds_and_adopts_current_copies() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fs::remove_file(fx.profile().join(RECEIPT)).unwrap();
    fx.write(
        "profile/worlds/old_kuopio.sqlite",
        "old kuopio the player edited",
    );
    let legacy_kuopio = sha("old kuopio");
    let legacy: &[(&str, &[&str])] = &[(OLD_KUOPIO, &[legacy_kuopio.as_str()])];
    let (plan, _) = fx.plan_counted(legacy);
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::Adopt),
            ("worlds/blank.sqlite", Operation::Adopt),
            ("worlds/city.sqlite", Operation::Adopt),
        ]
    );
    fx.apply(&plan);
    assert!(fx.exists("profile/worlds/old_kuopio.sqlite"));
    assert!(!fx.exists("trash"));
}

#[test]
fn a_first_start_without_receipt_survives_a_crash_and_leaves_a_receipt_however_it_ends() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.write("bundle/mods/other/pack.toml", "other v1");
    fx.write("profile/mods/other/pack.toml", "other v0");
    fx.write("profile/mods/pack/pack.toml", "pack v0");
    let (plan, _) = fx.plan_counted(&[]);
    assert_eq!(
        &ops(&plan)[..2],
        [
            ("mods/other", Operation::TrashAndReplace),
            ("mods/pack", Operation::TrashAndReplace),
        ]
    );
    // A crash after the first step's file work, before the start's one record.
    fx.bundle_side(&plan.actions[0]);
    commit(&fx.profile(), &plan.actions[0]).unwrap();
    assert!(!fx.exists("profile/bundled.toml"));
    // Still a profile from before the receipt: the done pack is adopted, the other still trashed.
    let (plan, _) = fx.plan_counted(&[]);
    assert!(ops(&plan).contains(&("mods/pack", Operation::TrashAndReplace)));
    assert!(ops(&plan).contains(&("mods/other", Operation::Adopt)));
    // A start where every step failed still writes the receipt, so the legacy rules end: the
    // stale pack it could not trash is the player's from then on, never trashed later.
    record(&fx.profile(), &[]).unwrap();
    assert!(fx.receipt().is_empty());
    let (plan, _) = fx.plan_counted(&[]);
    assert!(ops(&plan).contains(&("mods/pack", Operation::KeepPlayers)));
    assert!(ops(&plan).contains(&("mods/other", Operation::Adopt)));
}

#[test]
fn commit_replaces_through_the_retired_sibling_and_refuses_what_it_must_not_touch() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fx.write("bundle/mods/pack/pack.toml", "pack v2");
    let plan = fx.plan();
    assert_eq!(ops(&plan), [("mods/pack", Operation::Replace)]);
    let replace = &plan.actions[0];
    // A crash between moving the old copy aside and renaming the staged one into place.
    fx.bundle_side(replace);
    fs::rename(
        fx.profile().join("mods/pack"),
        fx.profile().join(replace.retired()),
    )
    .unwrap();
    let plan = fx.plan();
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::RemoveStaging),
            ("mods/pack", Operation::Seed),
        ]
    );
    fx.apply(&plan);
    assert_eq!(fx.read("profile/mods/pack/pack.toml"), "pack v2");
    assert!(!fx.exists("profile/mods/.bundled-pack"));
    assert!(!fx.exists("profile/mods/.bundled-pack.replaced"));
    assert!(fx.plan().actions.is_empty());

    let profile = fx.profile();
    let step = |path: &str, operation| Action {
        path: path.into(),
        operation,
        revision: sha("x"),
    };
    // Nothing staged; an installed entry never overwritten by a seed; a pack never removed;
    // a path outside the bundle folders never touched.
    assert!(commit(&profile, &step("worlds/city.sqlite", Operation::Seed)).is_err());
    fx.write("profile/worlds/.bundled-city.sqlite", "staged");
    assert!(commit(&profile, &step("worlds/city.sqlite", Operation::Seed)).is_err());
    assert_eq!(fx.read("profile/worlds/city.sqlite"), "city v1");
    assert!(commit(&profile, &step("mods/pack", Operation::Remove)).is_err());
    assert!(fx.exists("profile/mods/pack/pack.toml"));
    fx.write("profile/saves/a.sqlite", "a save");
    assert!(commit(&profile, &step("saves/a.sqlite", Operation::Remove)).is_err());
    assert!(
        commit(
            &profile,
            &step("worlds/../saves/a.sqlite", Operation::Remove)
        )
        .is_err()
    );
    assert!(fx.exists("profile/saves/a.sqlite"));
    // Receipt-only steps touch nothing, whatever the path.
    assert!(commit(&profile, &step("saves/a.sqlite", Operation::Forget)).is_ok());
    // A staging entry that is a symbolic link loses the link, not its target.
    #[cfg(unix)]
    {
        fx.write("elsewhere/kept.txt", "kept");
        std::os::unix::fs::symlink(fx.0.join("elsewhere"), profile.join("mods/.bundled-pack"))
            .unwrap();
        commit(&profile, &step("mods/pack", Operation::RemoveStaging)).unwrap();
        assert!(!fx.exists("profile/mods/.bundled-pack"));
        assert_eq!(fx.read("elsewhere/kept.txt"), "kept");
    }
}

#[test]
fn record_writes_only_a_missing_unreadable_or_changed_receipt() {
    let fx = Fixture::new();
    let profile = fx.profile();
    record(&profile, &[]).unwrap();
    assert!(fx.receipt().is_empty());
    let revision = sha("x");
    let adopt = Action {
        path: "mods/a".into(),
        operation: Operation::Adopt,
        revision: revision.clone(),
    };
    let written =
        format!("# by hand\n[entries.\"mods/a\"]\nrevision = \"{revision}\"\nowner = \"game\"\n");
    fx.write("profile/bundled.toml", &written);
    // Nothing to do, or the same record again: not rewritten (a normal start writes nothing).
    record(&profile, &[]).unwrap();
    record(&profile, std::slice::from_ref(&adopt)).unwrap();
    assert_eq!(fx.read("profile/bundled.toml"), written);
    let keep = Action {
        operation: Operation::KeepPlayers,
        ..adopt
    };
    record(&profile, &[keep]).unwrap();
    assert_eq!(fx.receipt()["mods/a"].owner, Owner::Player);
    fx.write("profile/bundled.toml", "[entries\nnot toml");
    record(&profile, &[]).unwrap();
    assert!(fx.receipt().is_empty());
}

#[test]
fn an_unreadable_receipt_is_reported_and_never_trashes() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.write("profile/mods/pack/pack.toml", "pack v0");
    fx.write("profile/bundled.toml", "[entries\nnot toml");
    let (plan, _) = fx.plan_counted(&[]);
    assert_eq!(plan.problems.len(), 1);
    assert!(plan.problems[0].starts_with(RECEIPT));
    assert_eq!(
        ops(&plan),
        [
            ("worlds/blank.sqlite", Operation::Seed),
            ("worlds/city.sqlite", Operation::Seed),
            ("mods/pack", Operation::KeepPlayers),
        ]
    );
    // Recording replaces the unreadable receipt.
    fx.apply(&plan);
    assert_eq!(fx.receipt().len(), 3);
}

#[test]
fn a_leftover_staging_entry_is_removed_first() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    // An interrupted copy: the staging folder only, and a staging file beside a missing world.
    fx.write("profile/mods/.bundled-pack/pack.toml", "partial");
    fs::remove_file(fx.profile().join("worlds/city.sqlite")).unwrap();
    fx.write("profile/worlds/.bundled-city.sqlite", "partial");
    let plan = fx.plan();
    assert_eq!(
        ops(&plan),
        [
            ("mods/pack", Operation::RemoveStaging),
            ("worlds/city.sqlite", Operation::RemoveStaging),
            ("worlds/city.sqlite", Operation::Seed),
        ]
    );
    assert_eq!(plan.actions[0].staging(), "mods/.bundled-pack");
    fx.apply(&plan);
    assert!(!fx.exists("profile/mods/.bundled-pack"));
    assert_eq!(fx.read("profile/worlds/city.sqlite"), "city v1");
    assert!(fx.plan().actions.is_empty());
}

#[cfg(unix)]
#[test]
fn a_linked_entry_is_the_players() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.write("elsewhere/pack/pack.toml", "pack v1");
    fs::create_dir_all(fx.profile().join("mods")).unwrap();
    std::os::unix::fs::symlink(fx.0.join("elsewhere/pack"), fx.profile().join("mods/pack"))
        .unwrap();
    // Even without a receipt, where a differing real folder would go to the Trash.
    let (plan, _) = fx.plan_counted(&[]);
    assert!(plan.problems.is_empty());
    assert!(ops(&plan).contains(&("mods/pack", Operation::KeepPlayers)));
}

#[test]
fn receipt_keys_outside_the_bundle_folders_are_only_forgotten() {
    let fx = Fixture::new();
    fx.bundle_v1();
    fx.install();
    fx.write("profile/saves/a.sqlite", "a save");
    let digest = sha("a save");
    let mut text = fx.read("profile/bundled.toml");
    for key in [
        "saves/a.sqlite",
        "worlds/../saves/a.sqlite",
        "worlds/.hidden",
    ] {
        text += &format!("\n[entries.\"{key}\"]\nrevision = \"{digest}\"\nowner = \"game\"\n");
    }
    fx.write("profile/bundled.toml", &text);
    let (plan, hashed) = fx.plan_counted(&[]);
    assert!(
        plan.actions
            .iter()
            .all(|action| action.operation == Operation::Forget)
    );
    assert_eq!(plan.actions.len(), 3);
    assert_eq!(hashed, 0);
    fx.apply(&plan);
    assert!(fx.exists("profile/saves/a.sqlite"));
    assert_eq!(fx.receipt().len(), 3);
    // Setting a record for such a path is refused.
    let mut action = plan.actions[0].clone();
    action.operation = Operation::Adopt;
    action.revision = digest;
    assert!(record(&fx.profile(), &[action]).is_err());
}

#[test]
fn the_receipt_is_written_atomically_in_a_stable_form() {
    let fx = Fixture::new();
    let profile = fx.profile();
    let revision = sha("x");
    let action = |path: &str, operation| Action {
        path: path.into(),
        operation,
        revision: revision.clone(),
    };
    record(
        &profile,
        &[
            action("worlds/b.sqlite", Operation::KeepPlayers),
            action("mods/a", Operation::Seed),
        ],
    )
    .unwrap();
    let text = fx.read("profile/bundled.toml");
    assert!(text.ends_with(&format!(
        "[entries.\"mods/a\"]\nrevision = \"{revision}\"\nowner = \"game\"\n\n\
         [entries.\"worlds/b.sqlite\"]\nrevision = \"{revision}\"\nowner = \"player\"\n"
    )));
    // No temporary file is left beside it.
    let names: Vec<_> = fs::read_dir(&profile)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, [RECEIPT]);
    // The staging cleanup leaves the receipt alone; removal drops the record.
    record(&profile, &[action("mods/a", Operation::RemoveStaging)]).unwrap();
    assert_eq!(fx.receipt().len(), 2);
    record(&profile, &[action("mods/a", Operation::Remove)]).unwrap();
    assert_eq!(fx.receipt().keys().collect::<Vec<_>>(), ["worlds/b.sqlite"]);
    // A revision that is not a digest is refused.
    let mut bad = action("mods/a", Operation::Adopt);
    bad.revision = "v2".into();
    assert!(record(&profile, &[bad]).is_err());
    // Operation names round-trip for the bridge.
    for operation in Operation::ALL {
        assert_eq!(Operation::from_name(operation.name()), Some(operation));
    }
    assert_eq!(Operation::from_name("copy"), None);
}

#[test]
fn a_folder_digest_hashes_the_sha256sum_lines_of_its_seeded_files() {
    let fx = Fixture::new();
    fx.bundle_v1();
    let pack = fx.0.join("bundle/mods/pack");
    let lines = format!(
        "{}  assets/a/model.glb\n{}  pack.toml\n",
        sha("model v1"),
        sha("pack v1")
    );
    assert_eq!(folder_digest(&pack).unwrap(), sha(&lines));
    // Skipped files never change it.
    fx.write(
        "bundle/mods/pack/assets/a/model.glb.import",
        "other sidecar",
    );
    fx.write("bundle/mods/pack/.hidden/y", "more hidden");
    assert_eq!(folder_digest(&pack).unwrap(), sha(&lines));
}

#[test]
fn a_malformed_manifest_is_refused() {
    let fx = Fixture::new();
    let line = |path: &str| format!("{}  {path}\n", sha("x"));
    for text in [
        line("saves/a.sqlite"),
        line("worlds/a/b.sqlite"),
        line("mods/.bundled-pack"),
        line("worlds/b.sqlite") + &line("worlds/a.sqlite"),
        "not a line\n".into(),
    ] {
        assert!(plan(&text, &fx.profile()).is_err(), "{text}");
    }
}

#[test]
fn the_legacy_table_names_valid_entries_and_never_a_bundled_digest() {
    let manifest = parse_manifest(MANIFEST).unwrap();
    for &(path, digests) in LEGACY {
        assert!(entry_path(path) && path.starts_with("worlds/"), "{path}");
        assert!(
            !digests.contains(manifest.get(path).unwrap_or(&"")),
            "{path}"
        );
    }
}

// The bundled packs' versions as committed with the manifest, beside it in `godot/bootstrap/`:
// `<version>  mods/<name>` a line, sorted by path. Only these tests read it; the game never
// compares pack versions (`TOOLS-47`).
const PACK_VERSIONS: &str = "pack_versions.txt";

// `<version>  mods/<name>` for each pack of `manifest`, from the pack's `pack.toml` in
// `bootstrap`.
fn pack_versions_of(bootstrap: &Path, manifest: &str) -> Result<String, String> {
    let mut lines = String::new();
    for path in parse_manifest(manifest)?
        .keys()
        .filter(|path| path.starts_with("mods/"))
    {
        let text = fs::read_to_string(bootstrap.join(path).join("pack.toml"))
            .map_err(|e| format!("{path}/pack.toml: {e}"))?;
        let pack: crate::assets::PackManifest =
            text.parse().map_err(|e| format!("{path}/pack.toml: {e}"))?;
        let _ = writeln!(lines, "{}  {path}", pack.version);
    }
    Ok(lines)
}

// Path to version, from `pack_versions.txt` text.
fn parse_pack_versions(text: &str) -> BTreeMap<&str, &str> {
    text.lines()
        .filter_map(|line| line.split_once("  "))
        .map(|(version, path)| (path, version))
        .collect()
}

// A bundled pack's `version` is raised whenever its content changes (`TOOLS-47`): refuses a pack
// whose digest differs from the committed manifest's while its version is not above the
// committed one. `pack.toml` is content too, so raising the version alone also changes the
// digest; a pack new to the bundle has nothing to compare with.
fn check_pack_versions(
    manifest: &str,
    versions: &str,
    committed_manifest: &str,
    committed_versions: &str,
) -> Result<(), String> {
    let committed = parse_manifest(committed_manifest)?;
    let (versions, committed_versions) = (
        parse_pack_versions(versions),
        parse_pack_versions(committed_versions),
    );
    for (path, digest) in parse_manifest(manifest)? {
        let (Some(old_digest), Some(old), Some(new)) = (
            committed.get(path),
            committed_versions.get(path),
            versions.get(path),
        ) else {
            continue;
        };
        if *old_digest != digest
            && crate::assets::pack::compare_versions(new, old) != Some(std::cmp::Ordering::Greater)
        {
            let name = &path["mods/".len()..];
            return Err(format!(
                "`{name}` changed: raise `version` in its `pack.toml` (committed {old}, found \
                 {new}) and regenerate"
            ));
        }
    }
    Ok(())
}

#[test]
fn a_changed_pack_needs_a_raised_version() {
    let committed_manifest = format!("{}  mods/a\n{}  mods/b\n", sha("a1"), sha("b1"));
    let committed_versions = "0.1.0  mods/a\n1.0.0  mods/b\n";
    let check = |a: &str, b: &str, a_version: &str| {
        check_pack_versions(
            &format!("{}  mods/a\n{}  mods/b\n", sha(a), sha(b)),
            &format!("{a_version}  mods/a\n1.0.0  mods/b\n"),
            &committed_manifest,
            committed_versions,
        )
    };
    assert_eq!(check("a1", "b1", "0.1.0"), Ok(()));
    assert_eq!(check("a2", "b1", "0.1.1"), Ok(()));
    let refused = Err(
        "`b` changed: raise `version` in its `pack.toml` (committed 1.0.0, found 1.0.0) and \
         regenerate"
            .to_owned(),
    );
    assert_eq!(check("a1", "b2", "0.1.0"), refused);
    // A lowered version is not a raise; a pack new to the bundle is not compared.
    assert!(check("a2", "b1", "0.0.9").is_err());
    let added = format!("{}  mods/a\n{}  mods/c\n", sha("a1"), sha("c1"));
    assert_eq!(
        check_pack_versions(
            &added,
            "0.1.0  mods/a\n0.1.0  mods/c\n",
            &committed_manifest,
            committed_versions
        ),
        Ok(())
    );
}

// The manifest and pack versions of `bootstrap` as the folder holds them, refused when it holds
// Git LFS pointer stubs or a pack changed without a raised version.
fn bootstrap_products(
    bootstrap: &Path,
    committed_versions: &str,
) -> Result<(String, String), String> {
    let stubs = lfs_stubs(bootstrap);
    if let Some(first) = stubs.first() {
        return Err(format!(
            "godot/bootstrap holds Git LFS pointer stubs, not the files ({first} first); \
             run `git lfs pull` or `git lfs checkout godot` and test again"
        ));
    }
    let manifest = manifest_of(bootstrap)?;
    let versions = pack_versions_of(bootstrap, &manifest)?;
    check_pack_versions(&manifest, &versions, MANIFEST, committed_versions)?;
    Ok((manifest, versions))
}

const REGENERATE: &str = "cargo test --lib assets::bundled::tests::write_bundled_manifest -- \
                          --ignored --exact";

#[test]
fn the_committed_manifest_matches_the_bootstrap_folder() {
    let bootstrap = Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot/bootstrap");
    let committed_versions = fs::read_to_string(bootstrap.join(PACK_VERSIONS)).unwrap();
    let (current, versions) = match bootstrap_products(&bootstrap, &committed_versions) {
        Ok(products) => products,
        Err(e) => panic!("{e}"),
    };
    assert!(
        current == MANIFEST && versions == committed_versions,
        "godot/bootstrap/manifest.sha256 or {PACK_VERSIONS} is stale: a bundled pack or world \
         changed. Regenerate them with `{REGENERATE}` and commit them.\nfrom the folder:\n\
         {current}{versions}committed:\n{MANIFEST}{committed_versions}"
    );
}

#[test]
#[ignore = "writes godot/bootstrap/manifest.sha256 and pack_versions.txt from the bootstrap folder"]
fn write_bundled_manifest() {
    let bootstrap = Path::new(env!("CARGO_MANIFEST_DIR")).join("../godot/bootstrap");
    let committed_versions = fs::read_to_string(bootstrap.join(PACK_VERSIONS)).unwrap_or_default();
    let (manifest, versions) = match bootstrap_products(&bootstrap, &committed_versions) {
        Ok(products) => products,
        Err(e) => panic!("nothing written: {e}"),
    };
    files::write_atomic(&bootstrap.join("manifest.sha256"), manifest.as_bytes()).unwrap();
    files::write_atomic(&bootstrap.join(PACK_VERSIONS), versions.as_bytes()).unwrap();
}
