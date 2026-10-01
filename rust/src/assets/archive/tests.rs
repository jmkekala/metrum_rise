// SPDX-License-Identifier: GPL-2.0-only

//! Share-archive regressions: determinism, exact contents, refusals and rollback.

use super::*;
use std::io::Cursor;

pub(super) struct Fixture(pub(super) PathBuf);
impl Fixture {
    // mods/test-pack with one prop whose glTF loads an image and a buffer, plus credits,
    // a thumbnail and unreferenced files export must leave out.
    pub(super) fn new() -> Self {
        let fixture = Self(std::env::temp_dir().join(format!("metrum-archive-{}", files::token())));
        fixture.write(
            "pack.toml",
            "pack_id = \"test-pack\"\nschema_version = 1\ndisplay_name = \"Test\"\n\
             version = \"1.0.0\"\nauthor = \"A\"\nlicense = \"CC0\"\ntags = [\"kept\"]\n",
        );
        fixture.write(
            "assets/prop.bench/asset.toml",
            &manifest("prop.bench", "\"thumb.png\""),
        );
        fixture.write(
            "assets/prop.bench/model.gltf",
            r#"{"images":[{"uri":"tex.png"}],"buffers":[{"uri":"mesh.bin"}]}"#,
        );
        fixture.write("assets/prop.bench/tex.png", "png");
        fixture.write("assets/prop.bench/mesh.bin", "mesh");
        fixture.write("assets/prop.bench/thumb.png", "thumb");
        fixture.write("assets/prop.bench/attribution/other.toml", "credit");
        fixture.write("assets/prop.bench/unused.png", "stale");
        fixture.write("assets/prop.bench/source.blend", "draft");
        fixture.write("pack.index.bin", "cache");
        fixture.write("checksums.sha256", "stale");
        fs::create_dir_all(fixture.0.join("out")).unwrap();
        fixture
    }
    pub(super) fn pack(&self) -> PathBuf {
        self.0.join("mods/test-pack")
    }
    pub(super) fn write(&self, relative: &str, data: &str) {
        let path = self.pack().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, data).unwrap();
    }
    pub(super) fn export(&self, out: &str, bump: Option<&str>) -> Result<Exported, String> {
        let destination = self.0.join(out);
        fs::create_dir_all(&destination).unwrap();
        export(&self.0.join("mods"), "test-pack", &destination, bump)
    }
    fn out_files(&self) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(self.0.join("out"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mesh = self.pack().join("assets/prop.bench/mesh.bin");
            let _ = fs::set_permissions(mesh, fs::Permissions::from_mode(0o644));
        }
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn manifest(id: &str, thumbnail: &str) -> String {
    format!(
        "asset_id = \"{id}\"\ndisplay_name = \"Bench\"\nthumbnail = {thumbnail}\n[prop]\n\
         category = \"test\"\nbounding_size_m = [1.0, 1.0, 1.0]\nsnap_mode = \"free\"\n\
         terrain_behavior = \"flat_ground\"\n[[lods]]\nfile = \"model.gltf\"\ndistance_min_m = 0.0\n"
    )
}

fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[test]
fn export_is_deterministic_and_contains_exactly_the_referenced_files() {
    let fixture = Fixture::new();
    let before = inventory(&fixture.pack()).unwrap();
    assert_eq!(
        before.excluded,
        [
            "assets/prop.bench/source.blend",
            "assets/prop.bench/unused.png",
            "checksums.sha256",
            "pack.index.bin"
        ]
    );
    assert_eq!((before.assets, before.files.len()), (1, 7));
    let first = fixture.export("out", None).unwrap();
    let second = fixture.export("again", None).unwrap();
    let bytes = fs::read(&first.path).unwrap();
    assert_eq!(bytes, fs::read(&second.path).unwrap());
    assert_eq!(first.sha256, sha(&bytes));
    assert_eq!(
        first.path.file_name().unwrap(),
        "test-pack-1.0.0.metrum.zip"
    );
    assert_eq!(
        fixture.out_files(),
        [
            "test-pack-1.0.0.metrum.zip",
            "test-pack-1.0.0.metrum.zip.sha256"
        ]
    );
    assert_eq!(
        fs::read_to_string(fixture.0.join("out/test-pack-1.0.0.metrum.zip.sha256")).unwrap(),
        format!("{}  test-pack-1.0.0.metrum.zip\n", first.sha256)
    );
    // The source pack is read, never written: its stale checksum file stays as it was.
    assert_eq!(
        fs::read_to_string(fixture.pack().join("checksums.sha256")).unwrap(),
        "stale"
    );

    let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
    let mut entries = BTreeMap::new();
    let mut names = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).unwrap();
        let name = entry.name().to_owned();
        let stored = name.ends_with(".png");
        assert_eq!(
            entry.compression(),
            if stored {
                CompressionMethod::Stored
            } else {
                CompressionMethod::Deflated
            },
            "{name}"
        );
        assert_eq!(entry.last_modified(), Some(zip::DateTime::default()));
        assert_eq!(entry.unix_mode().map(|mode| mode & 0o777), Some(0o644));
        let mut data = Vec::new();
        entry.read_to_end(&mut data).unwrap();
        names.push(name.clone());
        entries.insert(name.strip_prefix("test-pack/").unwrap().to_owned(), data);
    }
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted);
    assert_eq!(
        entries.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "assets/prop.bench/asset.toml",
            "assets/prop.bench/attribution/other.toml",
            "assets/prop.bench/mesh.bin",
            "assets/prop.bench/model.gltf",
            "assets/prop.bench/tex.png",
            "assets/prop.bench/thumb.png",
            "checksums.sha256",
            "pack.toml"
        ]
    );
    let listed = String::from_utf8(entries.remove("checksums.sha256").unwrap()).unwrap();
    let expected: String = entries
        .iter()
        .map(|(path, data)| format!("{}  {path}\n", sha(data)))
        .collect();
    assert_eq!(listed, expected);
}

#[test]
fn version_bump_rewrites_the_manifest_before_packaging() {
    let fixture = Fixture::new();
    let exported = fixture.export("out", Some("minor")).unwrap();
    assert_eq!(exported.version, "1.1.0");
    assert_eq!(
        exported.path.file_name().unwrap(),
        "test-pack-1.1.0.metrum.zip"
    );
    let manifest =
        PackManifest::from_str(&fs::read_to_string(fixture.pack().join("pack.toml")).unwrap())
            .unwrap();
    assert_eq!(manifest.version, "1.1.0");
    assert_eq!(manifest.tags, ["kept"]);
    assert!(fixture.export("again", Some("sideways")).is_err());
    assert!(
        fs::read_to_string(fixture.pack().join("pack.toml"))
            .unwrap()
            .contains("1.1.0")
    );
}

#[test]
fn invalid_packs_are_refused_without_writing() {
    let broken: [(&str, &str, &str); 6] = [
        (
            "assets/prop.bench/asset.toml",
            "asset_id = \"prop.bench\"",
            "asset.toml",
        ),
        ("assets/prop.bench/mesh.bin", "", "Missing"),
        (
            "assets/prop.bench/asset.toml",
            &manifest("prop.other", "\"thumb.png\""),
            "folder name",
        ),
        (
            "assets/prop.bench/nested/asset.toml",
            "",
            "assets/<asset_id>",
        ),
        ("pack.toml", "", "pack.toml"),
        (
            "assets/prop.bench/model.gltf",
            r#"{"images":[{"uri":"tex.png"},{"uri":"Tex.png"}]}"#,
            "letter case",
        ),
    ];
    for (relative, contents, expected) in broken {
        let fixture = Fixture::new();
        match relative {
            "assets/prop.bench/mesh.bin" => fs::remove_file(fixture.pack().join(relative)).unwrap(),
            "pack.toml" => {
                let text = fs::read_to_string(fixture.pack().join(relative)).unwrap();
                fixture.write(relative, &text.replace("1.0.0", "1.0"));
            }
            _ => fixture.write(relative, contents),
        }
        fixture.write("assets/prop.bench/Tex.png", "other");
        let error = fixture.export("out", Some("patch")).err().unwrap();
        assert!(error.contains(expected), "{relative}: {error}");
        assert!(fixture.out_files().is_empty(), "{relative}");
        if relative != "pack.toml" {
            assert!(
                fs::read_to_string(fixture.pack().join("pack.toml"))
                    .unwrap()
                    .contains("1.0.0")
            );
        }
    }
}

#[cfg(unix)]
#[test]
fn links_are_refused_and_failed_writes_leave_previous_archives() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    symlink(
        fixture.0.join("out"),
        fixture.pack().join("assets/prop.bench/linked"),
    )
    .unwrap();
    assert!(
        fixture
            .export("out", None)
            .err()
            .unwrap()
            .contains("symbolic link")
    );
    fs::remove_file(fixture.pack().join("assets/prop.bench/linked")).unwrap();

    let previous = fixture.0.join("out/test-pack-1.0.0.metrum.zip");
    fs::write(&previous, "previous").unwrap();
    // Inventory still sees the file; reading it fails half-way through writing the zip.
    fs::set_permissions(
        fixture.pack().join("assets/prop.bench/mesh.bin"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    assert!(fixture.export("out", None).is_err());
    assert_eq!(fixture.out_files(), ["test-pack-1.0.0.metrum.zip"]);
    assert_eq!(fs::read_to_string(previous).unwrap(), "previous");
}
