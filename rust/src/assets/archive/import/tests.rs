// SPDX-License-Identifier: GPL-2.0-only

//! Import regressions: export round trip, identical reinstall, and refusals that leave the
//! mods folder untouched.

use super::super::hex;
use super::super::tests::Fixture;
use super::*;
use sha2::{Digest, Sha256};
use zip::ZipWriter;
use zip::write::SimpleFileOptions;

const PACK_TOML: &str = "pack_id = \"p\"\nschema_version = 1\ndisplay_name = \"P\"\n\
                         version = \"1.0.0\"\nauthor = \"A\"\nlicense = \"CC0\"\n";

// Archive entries as (zip path, bytes).
type Entries<'a> = &'a [(&'a str, &'a [u8])];

fn mods(fixture: &Fixture) -> PathBuf {
    let path = fixture.0.join("install");
    fs::create_dir_all(&path).unwrap();
    path
}

fn listing(path: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

// Write a zip of `entries` and return its path and SHA-256. With `sums`, a correct
// `checksums.sha256` for the entries under the first entry's folder is added there.
fn craft(fixture: &Fixture, entries: Entries, sums: bool) -> (PathBuf, String) {
    let path = fixture.0.join("crafted.zip");
    let mut zip = ZipWriter::new(File::create(&path).unwrap());
    let options = SimpleFileOptions::DEFAULT.compression_method(CompressionMethod::Deflated);
    let root = format!("{}/", entries[0].0.split('/').next().unwrap());
    let mut listed: Vec<_> = entries
        .iter()
        .filter_map(|(name, data)| Some((name.strip_prefix(&root)?, digest(data))))
        .collect();
    listed.sort();
    let text: String = listed
        .iter()
        .map(|(name, sum)| format!("{sum}  {name}\n"))
        .collect();
    for (name, data) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(data).unwrap();
    }
    if sums {
        zip.start_file(format!("{root}checksums.sha256"), options)
            .unwrap();
        zip.write_all(text.as_bytes()).unwrap();
    }
    zip.finish().unwrap();
    let sum = sha256(&path).unwrap();
    (path, sum)
}

#[test]
fn expected_hash_accepts_digits_or_a_sidecar_line() {
    let hash = "AB".repeat(32);
    assert_eq!(expected_sha256(&hash), Some("ab".repeat(32)));
    assert_eq!(
        expected_sha256(&format!("  {hash}  pack-1.0.0.metrum.zip\n")),
        Some("ab".repeat(32))
    );
    assert_eq!(expected_sha256(&hash[1..]), None);
    assert_eq!(expected_sha256(&"zz".repeat(32)), None);
    assert_eq!(expected_sha256(""), None);
}

#[test]
fn exported_archive_round_trips_and_identical_reinstall_changes_nothing() {
    let fixture = Fixture::new();
    let exported = fixture.export("out", None).unwrap();
    let mods = mods(&fixture);
    let staged = stage(&exported.path, &exported.sha256, &mods, &[]).unwrap();
    assert!(matches!(staged.installed, Installed::Absent));
    assert_eq!(
        (staged.pack.pack_id.as_str(), staged.assets),
        ("test-pack", 1)
    );
    assert_eq!(staged.files, 8);
    let id = commit(&mods, &staged.staging.unwrap()).unwrap();
    assert_eq!(id, "test-pack");
    assert_eq!(listing(&mods), ["test-pack"]);
    let installed = mods.join("test-pack");
    assert_eq!(
        fs::read(installed.join("assets/prop.bench/mesh.bin")).unwrap(),
        b"mesh"
    );
    assert!(!installed.join("assets/prop.bench/unused.png").exists());
    // The installed pack exports to the same bytes it was imported from.
    let source = inventory(&fixture.pack()).unwrap();
    assert_eq!(
        inventory(&installed)
            .unwrap()
            .files
            .keys()
            .collect::<Vec<_>>(),
        source.files.keys().collect::<Vec<_>>()
    );

    // Files an archive never carries do not make the installed copy different.
    fs::write(installed.join("assets/prop.bench/source.blend"), "draft").unwrap();
    let again = stage(&exported.path, &exported.sha256, &mods, &[]).unwrap();
    assert!(matches!(again.installed, Installed::Identical));
    assert!(again.staging.is_none());
    assert_eq!(listing(&mods), ["test-pack"]);

    // A changed referenced file or a newer version is offered as a replacement.
    fs::write(installed.join("assets/prop.bench/tex.png"), "edited").unwrap();
    let changed = stage(&exported.path, &exported.sha256, &mods, &[]).unwrap();
    assert!(
        matches!(&changed.installed, Installed::Different { version } if version.as_deref() == Some("1.0.0"))
    );
    let staging = changed.staging.unwrap();
    assert!(
        commit(&mods, &staging)
            .err()
            .unwrap()
            .contains("still exists")
    );
    assert_eq!(listing(&mods), ["test-pack"]);
    let newer = fixture.export("next", Some("minor")).unwrap();
    let update = stage(&newer.path, &newer.sha256, &mods, &[]).unwrap();
    assert_eq!(update.pack.version, "1.1.0");
    discard(&mods, &update.staging.unwrap()).unwrap();
    assert_eq!(listing(&mods), ["test-pack"]);
    assert!(discard(&mods, &installed).is_err());
}

#[test]
fn hash_mismatch_and_bundled_packs_are_refused() {
    let fixture = Fixture::new();
    let exported = fixture.export("out", None).unwrap();
    let mods = mods(&fixture);
    let wrong = "0".repeat(64);
    let error = stage(&exported.path, &wrong, &mods, &[]).err().unwrap();
    assert!(error.contains(&exported.sha256), "{error}");
    assert!(stage(&exported.path, "not a hash", &mods, &[]).is_err());
    let bundled = ["test-pack".to_owned()];
    let error = stage(&exported.path, &exported.sha256, &mods, &bundled)
        .err()
        .unwrap();
    assert!(error.contains("bundled"), "{error}");
    assert!(listing(&mods).is_empty());
}

#[test]
fn malformed_archives_are_refused_without_changing_mods() {
    let fixture = Fixture::new();
    let mods = mods(&fixture);
    let pack = PACK_TOML.as_bytes();
    let bomb = vec![0; 2 << 20];
    let cases: [(Entries, bool, &str); 10] = [
        (&[("p/pack.toml", pack)], true, ""),
        (
            &[("p/pack.toml", pack), ("q/x", b"x")],
            true,
            "one top-level",
        ),
        (&[("p/pack.toml", pack), ("p/a/../b", b"x")], true, "unsafe"),
        (
            &[("p/pack.toml", pack), ("p\\x", b"x")],
            true,
            "inside the pack",
        ),
        (
            &[("p/pack.toml", pack), ("p/A", b"x"), ("p/a", b"y")],
            true,
            "letter case",
        ),
        (&[("p/pack.toml", pack)], false, "missing"),
        (
            &[("p/pack.toml", pack), ("p/notes.txt", b"x")],
            true,
            "not referenced",
        ),
        (
            &[("p/pack.toml", pack), ("p/big.bin", &bomb)],
            true,
            "ratio",
        ),
        (&[("q/pack.toml", pack)], true, "pack_id"),
        (&[("p/pack.toml", b"pack_id = \"p\"")], true, "Invalid pack"),
    ];
    for (entries, sums, expected) in cases {
        let (path, sum) = craft(&fixture, entries, sums);
        let result = stage(&path, &sum, &mods, &[]);
        if expected.is_empty() {
            // The control case: a pack with no assets is a valid archive.
            discard(&mods, &result.unwrap().staging.unwrap()).unwrap();
        } else {
            let error = result.err().unwrap_or_default();
            assert!(error.contains(expected), "{expected}: {error}");
        }
        assert!(listing(&mods).is_empty(), "{expected}");
    }

    // A checksum line that does not match the entry it names.
    let (path, _) = craft(&fixture, &[("p/pack.toml", pack)], false);
    let mut zip = ZipWriter::new_append(
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap(),
    )
    .unwrap();
    zip.start_file("p/checksums.sha256", SimpleFileOptions::DEFAULT)
        .unwrap();
    zip.write_all(format!("{}  pack.toml\n", "0".repeat(64)).as_bytes())
        .unwrap();
    zip.finish().unwrap();
    let error = stage(&path, &sha256(&path).unwrap(), &mods, &[])
        .err()
        .unwrap();
    assert!(error.contains("does not match"), "{error}");
    assert!(listing(&mods).is_empty());
}

#[test]
fn sweep_removes_only_stale_staging_from_other_processes() {
    let fixture = Fixture::new();
    let mods = mods(&fixture);
    let old = std::time::SystemTime::now() - STALE_STAGING * 2;
    let own = format!("{STAGING_PREFIX}{}-7", std::process::id());
    for name in [".import-1-0", ".import-2-0", &own, "kept-pack", ".other"] {
        fs::create_dir_all(mods.join(name).join("p")).unwrap();
        if name != ".import-2-0" {
            File::open(mods.join(name))
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
    }
    sweep(&mods);
    // .import-2-0 is recent, so another running instance may still be reviewing it.
    let mut expected = vec![".import-2-0", &own, ".other", "kept-pack"];
    expected.sort();
    assert_eq!(listing(&mods), expected);
}
