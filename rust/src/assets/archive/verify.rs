// SPDX-License-Identifier: GPL-2.0-only

//! Installed-pack verification (`TOOLS-11`): compare a pack folder with the
//! `checksums.sha256` its import kept.
//!
//! Read-only. Cost is O(listed files + bytes); files are hashed in parallel with Rayon, and the
//! pack's export set comes from the same [`inventory`] export and import use.

use super::{CHECKSUMS, inventory, parse_checksums, sha256};
use crate::assets::authoring::files;
use rayon::prelude::*;
use std::fs;
use std::path::Path;

/// Differences between an installed pack and its checksums; all empty means intact.
pub(crate) struct Verification {
    pub(crate) files: usize,
    pub(crate) changed: Vec<String>,
    pub(crate) missing: Vec<String>,
    /// Files the pack would now export that the checksums do not list, e.g. a new asset.
    pub(crate) extra: Vec<String>,
    /// Why the pack no longer validates as an exportable pack, if it does not.
    pub(crate) invalid: Option<String>,
}

/// Check `mods/<pack>` against its `checksums.sha256`. Packs without one (made locally rather
/// than imported) cannot be verified.
pub(crate) fn verify(mods: &Path, pack: &str) -> Result<Verification, String> {
    let root = files::pack_directory(mods, pack)?;
    let listing = root.join(CHECKSUMS);
    if listing.is_symlink() || !listing.is_file() {
        return Err(format!(
            "{pack} has no {CHECKSUMS}; only imported packs can be verified"
        ));
    }
    let text = fs::read_to_string(&listing).map_err(|e| format!("{CHECKSUMS}: {e}"))?;
    let listed = parse_checksums(&text)?;
    // (path, Some(changed)) for present files, (path, None) for missing ones.
    let states = listed
        .par_iter()
        .map(|(path, digest)| {
            let file = root.join(path);
            if file.is_symlink() || !file.is_file() {
                return Ok((*path, None));
            }
            Ok((*path, Some(sha256(&file)? != *digest)))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let pick = |wanted: Option<bool>| -> Vec<String> {
        states
            .iter()
            .filter(|(_, state)| *state == wanted)
            .map(|(path, _)| path.to_string())
            .collect()
    };
    let (extra, invalid) = match inventory(&root) {
        Ok(contents) => (
            contents
                .files
                .into_keys()
                .filter(|path| !listed.contains_key(path.as_str()))
                .collect(),
            None,
        ),
        Err(error) => (Vec::new(), Some(error)),
    };
    Ok(Verification {
        files: listed.len(),
        changed: pick(Some(true)),
        missing: pick(None),
        extra,
        invalid,
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests::Fixture;
    use super::super::{commit, stage};
    use super::*;

    #[test]
    fn imported_pack_verifies_and_reports_each_kind_of_difference() {
        let fixture = Fixture::new();
        let exported = fixture.export("out", None).unwrap();
        let mods = fixture.0.join("install");
        fs::create_dir_all(&mods).unwrap();
        let staged = stage(&exported.path, &exported.sha256, &mods, &[]).unwrap();
        commit(&mods, &staged.staging.unwrap()).unwrap();
        let intact = verify(&mods, "test-pack").unwrap();
        assert_eq!(intact.files, 7);
        assert!(intact.changed.is_empty() && intact.missing.is_empty());
        assert!(intact.extra.is_empty() && intact.invalid.is_none());

        let installed = mods.join("test-pack/assets/prop.bench");
        fs::write(installed.join("tex.png"), "edited").unwrap();
        fs::remove_file(installed.join("thumb.png")).unwrap();
        // Unreferenced files are not part of the pack, so they are not reported.
        fs::write(installed.join("notes.txt"), "draft").unwrap();
        let report = verify(&mods, "test-pack").unwrap();
        assert_eq!(report.changed, ["assets/prop.bench/tex.png"]);
        assert_eq!(report.missing, ["assets/prop.bench/thumb.png"]);
        // A missing referenced file also makes the pack fail validation.
        assert!(report.invalid.unwrap().contains("thumb.png"));

        fs::write(installed.join("thumb.png"), "thumb").unwrap();
        let other = mods.join("test-pack/assets/prop.other");
        fs::create_dir_all(&other).unwrap();
        let manifest = fs::read_to_string(installed.join("asset.toml"))
            .unwrap()
            .replace("prop.bench", "prop.other")
            .replace("thumbnail = \"thumb.png\"\n", "");
        fs::write(other.join("asset.toml"), manifest).unwrap();
        fs::copy(installed.join("model.gltf"), other.join("model.gltf")).unwrap();
        fs::copy(installed.join("mesh.bin"), other.join("mesh.bin")).unwrap();
        fs::copy(installed.join("tex.png"), other.join("tex.png")).unwrap();
        let report = verify(&mods, "test-pack").unwrap();
        assert_eq!(report.missing, Vec::<String>::new());
        assert_eq!(report.extra.len(), 4, "{:?}", report.extra);
        assert!(
            report
                .extra
                .iter()
                .all(|p| p.starts_with("assets/prop.other/"))
        );

        // The source pack was made locally: no checksums, nothing to verify against.
        fs::remove_file(fixture.pack().join(CHECKSUMS)).unwrap();
        let error = verify(&fixture.0.join("mods"), "test-pack").err().unwrap();
        assert!(error.contains("only imported packs"), "{error}");
        assert!(verify(&mods, "absent").is_err());
    }
}
