// SPDX-License-Identifier: GPL-3.0-or-later

//! The facts sidecar (`<name>.model.json`) a `metrum_models` build writes beside its export,
//! and the role inputs of one wanted asset (docs/asset_editor.md "Facts sidecar", `TOOLS-33`).
//! Both readers are strict: unknown keys, another schema or version, and malformed values are
//! refusals. Reading is O(file size); hashing is O(listed file bytes), once per draft.

use crate::assets::authoring::files::safe_relative;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

pub(crate) const SCHEMA: &str = "metrum-model-facts";
pub(crate) const VERSION: u64 = 1;

/// A door's facing is a unit vector; the reader accepts this much rounding in the file.
const UNIT_TOLERANCE: f64 = 0.01;
/// `bounds.min.y` is the ground; the reader accepts this much rounding in the file.
const GROUND_TOLERANCE_M: f64 = 0.001;

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Facts {
    pub(crate) schema: String,
    pub(crate) version: u64,
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) source: Source,
    pub(crate) license: String,
    pub(crate) bounds: Bounds,
    pub(crate) footprint: Vec<[f64; 2]>,
    /// Lowest first; empty for a model with no occupied storey (a residential draft refuses it).
    pub(crate) storeys: Vec<Storey>,
    pub(crate) dwellings: u32,
    /// Empty for a model without a door; at most one is `main`, an entrance (a zoned draft
    /// refuses a model without one).
    pub(crate) doors: Vec<Door>,
    pub(crate) lods: Vec<LodFile>,
    pub(crate) textures: Textures,
    pub(crate) variants: Vec<Variant>,
    pub(crate) default_variant: String,
    pub(crate) preview: FileRef,
    pub(crate) hero: bool,
    /// Every lane a vehicle drives into or through the model (`PARK-21`). Optional in schema
    /// version 1: absent (`None`), the model says nothing about lanes and a draft or report
    /// leaves the parts' passages as authored; present, empty included, it is the whole list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) passages: Option<Vec<SidecarPassage>>,
}

/// A lane through the model, in the sidecar's frame: the fields of a mesh part's passage
/// (`[[mesh_parts.passages]]`), which the draft copies onto the drafted part unchanged, since a
/// part's local frame is the `.glb`'s.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SidecarPassage {
    /// Unique in the asset; the passage's `name`.
    pub(crate) id: String,
    /// Centre of the mouth on the outer wall plane at ground level, `[x, 0, z]`.
    pub(crate) position: [f64; 3],
    /// Unit direction into the model, `[x, 0, z]`: the opposite of the mouth's outward normal.
    pub(crate) forward: [f64; 3],
    pub(crate) width_m: f64,
    pub(crate) length_m: f64,
    pub(crate) clear_height_m: f64,
    pub(crate) through: bool,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Source {
    pub(crate) commit: String,
    pub(crate) dirty: bool,
    pub(crate) blender: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Bounds {
    pub(crate) min: [f64; 3],
    pub(crate) max: [f64; 3],
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Storey {
    pub(crate) floor_y_m: f64,
    pub(crate) area_m2: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Door {
    pub(crate) id: String,
    pub(crate) kind: DoorKind,
    pub(crate) position: [f64; 3],
    pub(crate) facing: [f64; 3],
    pub(crate) width_m: f64,
    pub(crate) height_m: f64,
    pub(crate) main: bool,
}

/// What a door leads to; the main door is an entrance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DoorKind {
    /// A pedestrian door from the ground into the occupied space.
    Entrance,
    /// A staff or back pedestrian door at ground level.
    Service,
    /// A loading or goods door.
    Vehicle,
    /// A door not reachable from the ground (upper-floor doors included).
    Balcony,
    /// The door of a shed or an unheated store.
    Store,
}

impl DoorKind {
    /// Pedestrian doors at ground level, which the acceptance check sizes.
    pub(crate) fn pedestrian(self) -> bool {
        matches!(self, Self::Entrance | Self::Service)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LodFile {
    pub(crate) file: String,
    pub(crate) triangles: u64,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct FileRef {
    pub(crate) file: String,
    pub(crate) sha256: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Textures {
    pub(crate) orm: FileRef,
    pub(crate) normal: FileRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) emission: Option<FileRef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Variant {
    pub(crate) id: String,
    pub(crate) albedo: String,
    pub(crate) sha256: String,
}

/// The role inputs of one wanted asset: what the sidecar cannot say (`TOOLS-33` "Draft command").
/// `pack_id` is required here: the launcher fills it in when `--mods` holds exactly one pack.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Role {
    pub(crate) pack_id: String,
    pub(crate) asset_id: String,
    /// Absent or empty: `default_display_name` of the asset id.
    #[serde(default)]
    pub(crate) display_name: String,
    pub(crate) zone_type: String,
    pub(crate) density: String,
    #[serde(default = "first_level")]
    pub(crate) level: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) economy_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) asset_set: Option<String>,
    /// The ordered lot `[width, depth]` in whole cells; derived by R1 when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) lot_cells: Option<[u16; 2]>,
}

impl Facts {
    /// Check a parsed sidecar. Schema and version are read before the fields, so a newer
    /// file is refused for its version, not for the keys it adds.
    pub(crate) fn from_value(value: serde_json::Value) -> Result<Self, String> {
        if value["schema"] != SCHEMA {
            return Err(format!("sidecar schema must be \"{SCHEMA}\""));
        }
        if value["version"].as_u64() != Some(VERSION) {
            return Err(format!(
                "sidecar version {} is not supported; this reader reads version {VERSION}",
                value["version"]
            ));
        }
        let facts: Self = serde_json::from_value(value).map_err(|e| format!("sidecar: {e}"))?;
        facts.check()?;
        Ok(facts)
    }

    /// The one door a visitor uses, when the builder tagged one: `check` allows at most one,
    /// of kind entrance. A model may have none (a chimney that is part of a placed asset).
    pub(crate) fn main_door(&self) -> Option<&Door> {
        self.doors.iter().find(|door| door.main)
    }

    /// The main door a zoned draft puts its entrance on (R4); a model without one is a refusal.
    pub(crate) fn main_entrance(&self) -> Result<&Door, String> {
        self.main_door().ok_or_else(|| {
            format!(
                "{} has no main entrance door: a zoned building needs one (a sidecar door with \
                 main true, of kind entrance)",
                self.name
            )
        })
    }

    /// Every hashed file as `(path relative to the sidecar's folder, sha256)`, in file order:
    /// tiers, shared maps, variant albedos, preview.
    pub(crate) fn hashed_files(&self) -> Vec<(&str, &str)> {
        let mut files: Vec<(&str, &str)> = self
            .lods
            .iter()
            .map(|lod| (lod.file.as_str(), lod.sha256.as_str()))
            .collect();
        let textures = [
            Some(&self.textures.orm),
            Some(&self.textures.normal),
            self.textures.emission.as_ref(),
        ];
        files.extend(
            textures
                .into_iter()
                .flatten()
                .map(|file| (file.file.as_str(), file.sha256.as_str())),
        );
        files.extend(
            self.variants
                .iter()
                .map(|variant| (variant.albedo.as_str(), variant.sha256.as_str())),
        );
        files.push((self.preview.file.as_str(), self.preview.sha256.as_str()));
        files
    }

    /// Refuse unless every listed file beside the sidecar hashes to its recorded digest.
    pub(crate) fn verify_files(&self, folder: &Path) -> Result<(), String> {
        for (file, digest) in self.hashed_files() {
            let path = folder.join(file);
            if path.is_symlink() || !path.is_file() {
                return Err(format!("sidecar file is missing: {}", path.display()));
            }
            let actual = crate::assets::archive::sha256(&path)?;
            if actual != digest {
                return Err(format!(
                    "{file}: SHA-256 {actual} differs from the sidecar's {digest}"
                ));
            }
        }
        Ok(())
    }

    fn check(&self) -> Result<(), String> {
        let text = |name: &str, value: &str| {
            if value.trim().is_empty() {
                Err(format!("sidecar {name} must not be empty"))
            } else {
                Ok(())
            }
        };
        text("name", &self.name)?;
        text("path", &self.path)?;
        text("license", &self.license)?;
        text("source.blender", &self.source.blender)?;
        if self.source.commit.len() != 40 || !self.source.commit.bytes().all(is_lower_hex) {
            return Err("sidecar source.commit must be a 40-hex commit".into());
        }
        let Bounds { min, max } = self.bounds;
        if !min.iter().chain(&max).all(|v| v.is_finite())
            || (0..3).any(|axis| max[axis] <= min[axis])
        {
            return Err("sidecar bounds must be finite with max above min on every axis".into());
        }
        if min[1].abs() > GROUND_TOLERANCE_M {
            return Err("sidecar bounds.min.y must be 0, the ground".into());
        }
        let footprint: Vec<[f32; 2]> = self
            .footprint
            .iter()
            .map(|&[x, z]| [x as f32, z as f32])
            .collect();
        if !crate::assets::asset::geometry::is_valid(&footprint) {
            return Err("sidecar footprint must be one simple polygon of 3 or more points".into());
        }
        if crate::assets::asset::geometry::signed_area(&footprint) <= 0.0 {
            return Err("sidecar footprint must wind with positive area in [x, z]".into());
        }
        for storey in &self.storeys {
            if !storey.floor_y_m.is_finite() || !storey.area_m2.is_finite() || storey.area_m2 < 0.0
            {
                return Err("sidecar storeys need finite heights and areas of 0 or more".into());
            }
        }
        if self
            .storeys
            .windows(2)
            .any(|pair| pair[1].floor_y_m <= pair[0].floor_y_m)
        {
            return Err("sidecar storeys must be listed lowest first".into());
        }
        self.check_doors()?;
        self.check_passages()?;
        if self.lods.is_empty() {
            return Err("sidecar lists no LOD file".into());
        }
        for (file, digest) in self.hashed_files() {
            if digest.len() != 64 || !digest.bytes().all(is_lower_hex) {
                return Err(format!("{file}: sha256 must be 64 lowercase hex digits"));
            }
        }
        if self
            .lods
            .iter()
            .any(|lod| !lod.file.to_ascii_lowercase().ends_with(".glb"))
        {
            return Err("sidecar LOD files must be .glb files".into());
        }
        // Only the preview, listed last, lives outside the export folder (`../preview.png`).
        let files = self.hashed_files();
        let (preview, export) = files.split_last().expect("the preview is always listed");
        for (file, _) in export {
            if !safe_relative(file) {
                return Err(format!(
                    "sidecar file must be inside the export folder: {file}"
                ));
            }
        }
        if !preview_path(preview.0) {
            return Err(format!(
                "sidecar preview path must stay relative: {}",
                preview.0
            ));
        }
        let mut ids = BTreeSet::new();
        for variant in &self.variants {
            if !is_scheme_id(&variant.id) || !ids.insert(variant.id.as_str()) {
                return Err(format!(
                    "sidecar variant id '{}' is invalid or repeated",
                    variant.id
                ));
            }
        }
        if !ids.contains(self.default_variant.as_str()) {
            return Err("sidecar default_variant names no variant".into());
        }
        Ok(())
    }

    fn check_doors(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for door in &self.doors {
            if door.id.trim().is_empty() || !ids.insert(door.id.as_str()) {
                return Err(format!(
                    "sidecar door id '{}' is empty or repeated",
                    door.id
                ));
            }
            let [fx, fy, fz] = door.facing;
            if !door
                .position
                .iter()
                .chain(&door.facing)
                .all(|v| v.is_finite())
                || fy != 0.0
                || ((fx * fx + fz * fz).sqrt() - 1.0).abs() > UNIT_TOLERANCE
            {
                return Err(format!(
                    "sidecar door '{}' needs a finite position and a horizontal unit facing",
                    door.id
                ));
            }
            if !(door.width_m > 0.0 && door.height_m > 0.0) {
                return Err(format!("sidecar door '{}' needs a positive size", door.id));
            }
        }
        // The builder tags the main door; the reader never picks one (Q7). A file may tag none,
        // `doors` empty included: a zoned draft refuses that, not the reader.
        let mut mains = self.doors.iter().filter(|door| door.main);
        match (mains.next(), mains.next()) {
            (Some(_), Some(_)) => Err("sidecar must mark at most one door as main".into()),
            (Some(door), None) if door.kind != DoorKind::Entrance => Err(format!(
                "sidecar main door '{}' must be of kind entrance",
                door.id
            )),
            _ => Ok(()),
        }
    }
}

impl Facts {
    // Passages: unique ids, a finite mouth on the ground, a horizontal unit forward and a
    // positive size. Whether one lies inside the model and runs along a lot axis is the drafted
    // asset's validation (`check_passages_in_bounds`, `lot_axis_forward`). O(passages log passages).
    fn check_passages(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for passage in self.passages.iter().flatten() {
            if passage.id.trim().is_empty() || !ids.insert(passage.id.as_str()) {
                return Err(format!(
                    "sidecar passage id '{}' is empty or repeated",
                    passage.id
                ));
            }
            let [fx, fy, fz] = passage.forward;
            if !passage
                .position
                .iter()
                .chain(&passage.forward)
                .all(|v| v.is_finite())
                || passage.position[1].abs() > GROUND_TOLERANCE_M
                || fy != 0.0
                || ((fx * fx + fz * fz).sqrt() - 1.0).abs() > UNIT_TOLERANCE
            {
                return Err(format!(
                    "sidecar passage '{}' needs a finite mouth on the ground and a horizontal unit forward",
                    passage.id
                ));
            }
            if ![passage.width_m, passage.length_m, passage.clear_height_m]
                .iter()
                .all(|v| v.is_finite() && *v > 0.0)
            {
                return Err(format!(
                    "sidecar passage '{}' needs a positive width, length and clear height",
                    passage.id
                ));
            }
        }
        Ok(())
    }

    /// The passages as the drafted part's `passages`, in sidecar order. O(passages).
    pub(crate) fn part_passages(&self) -> Vec<serde_json::Value> {
        self.passages
            .iter()
            .flatten()
            .map(|p| {
                serde_json::json!({
                    "name": p.id,
                    "position": p.position,
                    "forward": p.forward,
                    "width_m": p.width_m,
                    "length_m": p.length_m,
                    "clear_height_m": p.clear_height_m,
                    "through": p.through,
                })
            })
            .collect()
    }
}

impl Role {
    /// Read and check role inputs, filling in the defaults: `level` 1 and the display name.
    pub(crate) fn parse(value: serde_json::Value) -> Result<Self, String> {
        let mut role: Self = serde_json::from_value(value).map_err(|e| format!("role: {e}"))?;
        if !crate::assets::is_valid_pack_id(&role.pack_id) {
            return Err(format!(
                "role pack_id '{}' is not a pack id: {PACK_ID_RULE}",
                role.pack_id
            ));
        }
        if !crate::assets::is_valid_asset_id(&role.asset_id) {
            return Err(format!(
                "role asset_id '{}' is not an asset id: dot-separated segments of lowercase \
                 letters, digits and underscores",
                role.asset_id
            ));
        }
        if role.display_name.trim().is_empty() {
            role.display_name = default_display_name(&role.asset_id);
        }
        // Zoned buildings only in this version (Q12); explicit buildings wait.
        if !matches!(
            role.zone_type.as_str(),
            "residential" | "commercial" | "industrial"
        ) {
            return Err(format!(
                "role zone_type '{}' is not a zoned building type (residential, commercial, industrial)",
                role.zone_type
            ));
        }
        if role.level == 0 {
            return Err("role level must be 1 or more".into());
        }
        if role.zone_type == "residential" && role.economy_profile.is_some() {
            return Err("a residential role takes no economy_profile".into());
        }
        if role.zone_type != "residential"
            && role
                .economy_profile
                .as_deref()
                .is_none_or(|p| p.trim().is_empty())
        {
            return Err(format!(
                "a {} role needs an economy_profile: a workplace without one never spawns",
                role.zone_type
            ));
        }
        if role.lot_cells.is_some_and(|lot| lot.contains(&0)) {
            return Err("role lot_cells must be whole cells, at least 1 each".into());
        }
        Ok(role)
    }
}

/// What `is_valid_pack_id` accepts, for refusals; `tools/model_draft.py` says the same.
const PACK_ID_RULE: &str = "a pack id is the name of the pack's folder in --mods, made of \
                            lowercase letters, digits and hyphens, not starting or ending with a hyphen";

fn first_level() -> u8 {
    1
}

/// A role's display name when it gives none: the asset id's last `.` segment without a trailing
/// `_level_<digits>` (both levels of a house share their name), then `_` read as spaces and each
/// word capitalised (R7's scheme name rule): `building.residential.house_3` and
/// `building.residential.house_3_level_2` both give "House 3".
pub(crate) fn default_display_name(asset_id: &str) -> String {
    let segment = asset_id.rsplit('.').next().unwrap_or(asset_id);
    let name = segment
        .rsplit_once("_level_")
        .filter(|(base, level)| {
            !base.is_empty() && !level.is_empty() && level.bytes().all(|b| b.is_ascii_digit())
        })
        .map_or(segment, |(base, _)| base);
    super::rules::scheme_name(name)
}

/// The ledger entry beside a published draft, `<pack>/model_sources/<asset_id>.json`.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Ledger {
    pub(crate) sidecar: serde_json::Value,
    pub(crate) role: serde_json::Value,
}

pub(crate) fn ledger_path(pack: &Path, asset_id: &str) -> PathBuf {
    pack.join("model_sources").join(format!("{asset_id}.json"))
}

/// The ledger's sidecar, read by the same strict reader; `None` when the asset has no entry.
pub(crate) fn read_ledger(pack: &Path, asset_id: &str) -> Result<Option<Facts>, String> {
    let path = ledger_path(pack, asset_id);
    if !path.exists() && !path.is_symlink() {
        return Ok(None);
    }
    if path.is_symlink() || !path.is_file() {
        return Err(format!(
            "ledger entry is not a regular file: {}",
            path.display()
        ));
    }
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let ledger: Ledger =
        serde_json::from_str(&text).map_err(|e| format!("ledger {}: {e}", path.display()))?;
    Facts::from_value(ledger.sidecar)
        .map(Some)
        .map_err(|e| format!("ledger {}: {e}", path.display()))
}

/// Copy the sidecar as read and the role with its defaults filled in beside the pack's assets.
/// The scanner registers only `asset.toml` files, and an editor republish replaces only
/// `assets/<asset_id>/`.
pub(crate) fn write_ledger(
    pack: &Path,
    asset_id: &str,
    sidecar: &serde_json::Value,
    role: &serde_json::Value,
) -> Result<PathBuf, String> {
    let path = ledger_path(pack, asset_id);
    let folder = path.parent().ok_or("ledger has no folder")?;
    if folder.is_symlink() {
        return Err("model_sources cannot be a symbolic link".into());
    }
    std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
    let ledger = Ledger {
        sidecar: sidecar.clone(),
        role: role.clone(),
    };
    let text = serde_json::to_string_pretty(&ledger).map_err(|e| e.to_string())? + "\n";
    crate::assets::authoring::files::write_atomic(&path, text.as_bytes())
        .map_err(|e| format!("ledger was not written: {e}"))?;
    Ok(path)
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

/// A colour scheme id, as `BuildingAppearance::validate` accepts it.
fn is_scheme_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

// `../preview.png`: parent steps, then plain names; never absolute.
fn preview_path(file: &str) -> bool {
    let mut names = 0;
    for component in Path::new(file).components() {
        match component {
            Component::ParentDir if names == 0 => {}
            Component::Normal(_) => names += 1,
            _ => return false,
        }
    }
    names > 0 && !file.contains('\\')
}
