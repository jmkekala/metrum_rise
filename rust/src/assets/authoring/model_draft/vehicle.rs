// SPDX-License-Identifier: GPL-3.0-or-later

//! Vehicle drafts (`TOOLS-41`, docs/vehicles.md "Facts sidecar" and docs/asset_editor.md "Draft
//! command"): the version 2 `kind: "vehicle"` facts sidecar, read as strictly as version 1, the
//! vehicle role, and the `[vehicle]` asset a draft publishes. No lot, door or parking rule
//! applies. Runs once per draft command: O(file size) to read, O(listed file bytes) to hash.

use super::report::{self, Line};
use super::sidecar::{Bounds, FileRef, SCHEMA, Source};
use crate::assets::authoring::files::safe_relative;
use crate::assets::{AssetManifest, BodyClass, VehicleLamp};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The sidecar version a vehicle export carries.
pub(crate) const VERSION: u64 = 2;
/// The tier bands of a 4.5 m car (docs/vehicles.md "Tiers"); a model's are these scaled by its
/// length over [`REFERENCE_LENGTH_M`], so a tier switches at the same size on screen.
const TIER_STARTS_M: [f32; 4] = [0.0, 30.0, 100.0, 400.0];
const REFERENCE_LENGTH_M: f32 = 4.5;

/// A version 2 vehicle sidecar. `lamps` reuse the pack schema's lens table, whose keys are the
/// sidecar's. Every key is read so that the reader stays strict; the draft copies only some.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VehicleFacts {
    pub(crate) schema: String,
    pub(crate) version: u64,
    pub(crate) kind: String,
    pub(crate) name: String,
    pub(crate) path: String,
    pub(crate) source: Source,
    pub(crate) license: String,
    pub(crate) bounds: Bounds,
    pub(crate) body: Body,
    pub(crate) axles: Vec<Axle>,
    pub(crate) wheels: Vec<Wheel>,
    pub(crate) attributes: Attributes,
    pub(crate) lamps: Vec<VehicleLamp>,
    pub(crate) glass: Vec<Glass>,
    pub(crate) cargo: Option<Cargo>,
    /// Reserved for trailers; must be `null`.
    pub(crate) coupling: Option<Value>,
    pub(crate) interior: Option<Interior>,
    pub(crate) lods: Vec<Tier>,
    pub(crate) textures: Textures,
    pub(crate) variants: Vec<Variant>,
    pub(crate) default_variant: String,
    pub(crate) preview: FileRef,
}

// Every key is read so that the reader stays strict; the draft copies only some.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Body {
    pub(crate) class: BodyClass,
    pub(crate) length_m: f32,
    pub(crate) width_m: f32,
    pub(crate) width_mirrors_m: f32,
    pub(crate) height_m: f32,
    pub(crate) wheelbase_m: f32,
    pub(crate) front_overhang_m: f32,
    pub(crate) rear_overhang_m: f32,
    pub(crate) ground_clearance_m: f32,
}

// Every key is read so that the reader stays strict; the draft copies only some.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Axle {
    pub(crate) z: f32,
    pub(crate) track_m: f32,
    pub(crate) steer: bool,
    pub(crate) twin: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Wheel {
    pub(crate) id: String,
    pub(crate) part: u8,
    pub(crate) axle: usize,
    pub(crate) side: crate::assets::Side,
    pub(crate) centre: [f32; 3],
    pub(crate) radius_m: f32,
    pub(crate) width_m: f32,
    pub(crate) steer: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Attributes {
    pub(crate) set: String,
    pub(crate) r: String,
    pub(crate) g: String,
    pub(crate) b: String,
    pub(crate) version: u64,
}

// Every key is read so that the reader stays strict; the draft copies only some.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Glass {
    pub(crate) group: String,
    pub(crate) id: u8,
    pub(crate) area_m2: f32,
}

// Every key is read so that the reader stays strict; the draft copies only some.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Cargo {
    pub(crate) min: [f32; 3],
    pub(crate) max: [f32; 3],
    pub(crate) door: String,
}

// Every key is read so that the reader stays strict; the draft copies only some.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Interior {
    pub(crate) file: String,
    pub(crate) triangles: u64,
    pub(crate) sha256: String,
}

// Every key is read so that the reader stays strict; the draft copies only some.
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Tier {
    pub(crate) file: String,
    pub(crate) triangles: u64,
    pub(crate) sha256: String,
    pub(crate) wheel_parts: bool,
    pub(crate) lamp_ids: bool,
    pub(crate) glass_ids: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Textures {
    pub(crate) orm: FileRef,
    pub(crate) emission: FileRef,
    pub(crate) normal: Option<FileRef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Variant {
    pub(crate) name: String,
    pub(crate) albedo: FileRef,
    pub(crate) paint: bool,
}

/// What a vehicle draft needs that the sidecar cannot say.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct VehicleRole {
    pub(crate) pack_id: String,
    pub(crate) asset_id: String,
    /// Absent or empty: `default_display_name` of the asset id.
    #[serde(default)]
    pub(crate) display_name: String,
    /// `[vehicle] vehicle_class` (`civil`, `utility`, ...).
    pub(crate) vehicle_class: String,
    /// `[vehicle] vehicle_family` (`estate`, `truck`, ...).
    pub(crate) vehicle_family: String,
    pub(crate) seats: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) spawn_weight: Option<f32>,
    /// sRGB colour and weight; absent takes the body class's default palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) paints: Option<Vec<RolePaint>>,
}

/// One palette entry of a role, as `[[vehicle.paints]]` takes it.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RolePaint {
    pub(crate) colour: [f32; 3],
    pub(crate) weight: f32,
}

/// Whether a parsed sidecar is a vehicle's (`kind: "vehicle"`); a building's has no kind.
pub(crate) fn is_vehicle(sidecar: &Value) -> bool {
    sidecar.get("kind").is_some()
}

impl VehicleFacts {
    /// Check a parsed sidecar: schema, version and kind before the fields.
    pub(crate) fn from_value(value: Value) -> Result<Self, String> {
        if value["schema"] != SCHEMA {
            return Err(format!("sidecar schema must be \"{SCHEMA}\""));
        }
        if value["version"].as_u64() != Some(VERSION) || value["kind"] != "vehicle" {
            return Err(format!(
                "sidecar version {} kind {} is not supported; this reader reads version {VERSION} \
                 kind \"vehicle\"",
                value["version"], value["kind"]
            ));
        }
        let facts: Self = serde_json::from_value(value).map_err(|e| format!("sidecar: {e}"))?;
        facts.check()?;
        Ok(facts)
    }

    /// Every hashed file as `(path relative to the export folder, sha256)`: tiers, maps, variant
    /// albedos, interior, then the preview (which lives beside the export folder).
    pub(crate) fn hashed_files(&self) -> Vec<(&str, &str)> {
        let mut files: Vec<_> = self
            .lods
            .iter()
            .map(|lod| (lod.file.as_str(), lod.sha256.as_str()))
            .collect();
        let maps = [Some(&self.textures.orm), Some(&self.textures.emission), self.textures.normal.as_ref()];
        files.extend(maps.into_iter().flatten().map(|f| (f.file.as_str(), f.sha256.as_str())));
        files.extend(
            self.variants
                .iter()
                .map(|v| (v.albedo.file.as_str(), v.albedo.sha256.as_str())),
        );
        files.extend(self.interior.iter().map(|i| (i.file.as_str(), i.sha256.as_str())));
        files.push((self.preview.file.as_str(), self.preview.sha256.as_str()));
        files
    }

    /// Refuse unless every listed file hashes to its digest (the preview relative to the asset
    /// folder, the export's parent).
    pub(crate) fn verify_files(&self, export: &Path) -> Result<(), String> {
        let (preview, export_files) = self.hashed_files().split_last().map(|(p, e)| (*p, e.to_vec())).expect("preview");
        let folder = export.parent().unwrap_or(export);
        for (path, digest) in export_files
            .iter()
            .map(|&(file, digest)| (export.join(file), digest))
            .chain([(folder.join(preview.0), preview.1)])
        {
            if path.is_symlink() || !path.is_file() {
                return Err(format!("sidecar file is missing: {}", path.display()));
            }
            let actual = crate::assets::archive::sha256(&path)?;
            if actual != digest {
                return Err(format!(
                    "{}: SHA-256 {actual} differs from the sidecar's {digest}",
                    path.display()
                ));
            }
        }
        Ok(())
    }

    fn check(&self) -> Result<(), String> {
        for (name, value) in [
            ("name", &self.name),
            ("path", &self.path),
            ("license", &self.license),
            ("source.blender", &self.source.blender),
        ] {
            if value.trim().is_empty() {
                return Err(format!("sidecar {name} must not be empty"));
            }
        }
        if self.source.commit.len() != 40 || !self.source.commit.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("sidecar source.commit must be a 40-hex commit".into());
        }
        let a = &self.attributes;
        if (a.set.as_str(), a.r.as_str(), a.g.as_str(), a.b.as_str(), a.version)
            != ("COLOR_0", "part", "surface", "paint", 1)
        {
            return Err("sidecar attributes must be COLOR_0 with r part, g surface, b paint, version 1".into());
        }
        if self.coupling.is_some() {
            return Err("sidecar coupling is reserved for trailers and must be null".into());
        }
        if self.lods.len() != crate::assets::VEHICLE_TIERS
            || self.lods.iter().any(|lod| !lod.file.to_ascii_lowercase().ends_with(".glb"))
        {
            return Err("sidecar must list four .glb tiers, LOD0 to LOD3".into());
        }
        for (file, digest) in self.hashed_files() {
            if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return Err(format!("{file}: sha256 must be 64 lowercase hex digits"));
            }
        }
        let files = self.hashed_files();
        let (preview, export) = files.split_last().expect("the preview is always listed");
        if let Some((file, _)) = export.iter().find(|(file, _)| !safe_relative(file)) {
            return Err(format!("sidecar file must be inside the export folder: {file}"));
        }
        if !safe_relative(preview.0) {
            return Err(format!("sidecar preview must be inside the asset folder: {}", preview.0));
        }
        for (index, wheel) in self.wheels.iter().enumerate() {
            let axle = self.axles.get(wheel.axle).ok_or_else(|| {
                format!("sidecar wheel '{}' names axle {}, which is not listed", wheel.id, wheel.axle)
            })?;
            if usize::from(wheel.part) != index + 1 || wheel.steer != axle.steer {
                return Err(format!(
                    "sidecar wheel '{}' must be part {} and steer as its axle",
                    wheel.id,
                    index + 1
                ));
            }
        }
        if self.cargo.is_none() && self.body.class != BodyClass::Car {
            return Err("sidecar cargo must be stated for a van, truck or bus".into());
        }
        if !self.variants.iter().any(|v| v.name == self.default_variant) {
            return Err("sidecar default_variant names no variant".into());
        }
        Ok(())
    }
}

impl VehicleRole {
    /// Read and check a vehicle role, filling in the display name.
    pub(crate) fn parse(value: Value) -> Result<Self, String> {
        let mut role: Self = serde_json::from_value(value).map_err(|e| format!("role: {e}"))?;
        if !crate::assets::is_valid_pack_id(&role.pack_id) {
            return Err(format!("role pack_id '{}' is not a pack id", role.pack_id));
        }
        if !crate::assets::is_valid_asset_id(&role.asset_id) {
            return Err(format!("role asset_id '{}' is not an asset id", role.asset_id));
        }
        serde_json::from_value::<crate::assets::VehicleClass>(json!(role.vehicle_class))
            .map_err(|_| format!("role vehicle_class '{}' is not a vehicle class", role.vehicle_class))?;
        serde_json::from_value::<crate::assets::VehicleFamily>(json!(role.vehicle_family))
            .map_err(|_| format!("role vehicle_family '{}' is not a vehicle family", role.vehicle_family))?;
        if role.display_name.trim().is_empty() {
            role.display_name = super::sidecar::default_display_name(&role.asset_id);
        }
        Ok(role)
    }
}

/// A read and checked vehicle sidecar with its role.
#[derive(Debug)]
pub(crate) struct VehicleInputs {
    pub(crate) facts: VehicleFacts,
    /// The sidecar as read, copied verbatim into the ledger.
    pub(crate) sidecar: Value,
    pub(crate) role: VehicleRole,
    /// The sidecar's folder.
    pub(crate) export: PathBuf,
    pub(crate) source_commit: Option<String>,
}

/// Read a vehicle sidecar already parsed as `sidecar` and its role: a clean source, a well-formed
/// role and every listed hash. Each failure is a refusal.
pub(crate) fn read_inputs(sidecar_path: &Path, sidecar: Value, role: Value) -> Result<VehicleInputs, String> {
    let facts = VehicleFacts::from_value(sidecar.clone())?;
    if facts.source.dirty {
        return Err(format!(
            "the export of {} was built from a dirty models tree; commit and rebuild it",
            facts.name
        ));
    }
    let role = VehicleRole::parse(role)?;
    let export = sidecar_path.parent().ok_or("the sidecar has no folder")?.to_owned();
    facts.verify_files(&export)?;
    Ok(VehicleInputs {
        facts,
        sidecar,
        role,
        export,
        source_commit: None,
    })
}

/// A vehicle draft derived for one pack, not yet published.
pub(crate) struct Derived {
    pub(crate) pack: PathBuf,
    pub(crate) asset_toml: String,
    /// The tiers, as `files::plan` takes models (their textures come with them).
    pub(crate) models: Vec<(String, PathBuf)>,
    /// Maps and albedos (one the tiers do not reference included), and the thumbnail.
    pub(crate) extras: Vec<(String, PathBuf)>,
    pub(crate) summary: Value,
    pub(crate) preview: PathBuf,
    pub(crate) thumbnail: PathBuf,
}

/// Derive the `[vehicle]` asset for the role's pack in `mods`, the thumbnail going to `work`.
pub(crate) fn derive(inputs: &VehicleInputs, mods: &Path, work: &Path) -> Result<Derived, String> {
    let (facts, role) = (&inputs.facts, &inputs.role);
    let pack = crate::assets::authoring::files::pack_directory(mods, &role.pack_id)?;
    let thumbnail = work.join(super::rules::THUMBNAIL_FILE);
    let asset_toml = asset_toml(facts, role);
    AssetManifest::from_str(&asset_toml)
        .map_err(|e| format!("the drafted asset fails the pack-load checks: {e}"))?;
    let models = facts
        .lods
        .iter()
        .map(|lod| (lod.file.clone(), inputs.export.join(&lod.file)))
        .collect();
    let mut extras: Vec<_> = facts
        .hashed_files()
        .split_last()
        .map(|(_, files)| files.to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter(|(file, _)| !file.to_ascii_lowercase().ends_with(".glb"))
        .map(|(file, _)| (file.to_owned(), inputs.export.join(file)))
        .collect();
    extras.push((super::rules::THUMBNAIL_FILE.to_owned(), thumbnail.clone()));
    let summary = json!({
        "vehicle": {
            "body_class": format!("{:?}", facts.body.class).to_lowercase(),
            "size_m": [facts.body.length_m, facts.body.width_m, facts.body.height_m],
            "tiers": facts.lods.iter().map(|lod| lod.triangles).collect::<Vec<_>>(),
            "tier_starts_m": tier_starts(facts.body.length_m),
            "wheels": facts.wheels.len(),
            "lenses": facts.lamps.len(),
            "variants": facts.variants.iter().map(|v| (v.name.clone(), v.paint)).collect::<Vec<_>>(),
        },
        "sidecar_commit": facts.source.commit,
        "source_commit": inputs.source_commit,
    });
    Ok(Derived {
        pack,
        asset_toml,
        models,
        extras,
        summary,
        preview: inputs.export.parent().unwrap_or(&inputs.export).join(&facts.preview.file),
        thumbnail,
    })
}

/// The tier switch distances of a model `length_m` long, whole metres.
pub(crate) fn tier_starts(length_m: f32) -> [f32; 4] {
    TIER_STARTS_M.map(|start| (start * length_m / REFERENCE_LENGTH_M).round())
}

// `asset.toml` of the draft: the role's values, then the sidecar's facts in its own order.
fn asset_toml(facts: &VehicleFacts, role: &VehicleRole) -> String {
    let text = |value: &str| toml::Value::String(value.to_owned()).to_string();
    let vec = |v: [f32; 3]| format!("[{}, {}, {}]", v[0], v[1], v[2]);
    let mut out = format!(
        "asset_id = {}\ndisplay_name = {}\nthumbnail = {}\ntags = [\"vehicle\", {}]\n",
        text(&role.asset_id),
        text(&role.display_name),
        text(super::rules::THUMBNAIL_FILE),
        text(&format!("{:?}", facts.body.class).to_lowercase()),
    );
    let starts = tier_starts(facts.body.length_m);
    for (index, lod) in facts.lods.iter().enumerate() {
        out += &format!("\n[[lods]]\nfile = {}\ndistance_min_m = {:.1}\n", text(&lod.file), starts[index]);
        if let Some(end) = starts.get(index + 1) {
            out += &format!("distance_max_m = {end:.1}\n");
        }
    }
    let body = &facts.body;
    out += &format!(
        "\n[vehicle]\nvehicle_class = {}\nvehicle_family = {}\nbody_class = {}\nlength_m = {}\n\
         width_m = {}\nheight_m = {}\nseats = {}\norm = {}\nemission = {}\n",
        text(&role.vehicle_class),
        text(&role.vehicle_family),
        text(&format!("{:?}", body.class).to_lowercase()),
        body.length_m,
        body.width_m,
        body.height_m,
        role.seats,
        text(&facts.textures.orm.file),
        text(&facts.textures.emission.file),
    );
    if let Some(weight) = role.spawn_weight {
        out += &format!("spawn_weight = {weight}\n");
    }
    if let Some(normal) = &facts.textures.normal {
        out += &format!("normal = {}\n", text(&normal.file));
    }
    // The default variant first: it is the one a car without a variant pick shows.
    let mut variants: Vec<_> = facts.variants.iter().collect();
    variants.sort_by_key(|v| v.name != facts.default_variant);
    for variant in variants {
        out += &format!(
            "\n[[vehicle.variants]]\nname = {}\nalbedo = {}\npaint = {}\n",
            text(&variant.name),
            text(&variant.albedo.file),
            variant.paint
        );
    }
    for paint in role.paints.iter().flatten() {
        out += &format!("\n[[vehicle.paints]]\ncolour = {}\nweight = {}\n", vec(paint.colour), paint.weight);
    }
    for wheel in &facts.wheels {
        out += &format!(
            "\n[[vehicle.wheels]]\npart = {}\nside = {}\ncentre = {}\nradius_m = {}\nwidth_m = {}\n\
             steer = {}\ntwin = {}\n",
            wheel.part,
            text(&snake(&wheel.side)),
            vec(wheel.centre),
            wheel.radius_m,
            wheel.width_m,
            wheel.steer,
            facts.axles[wheel.axle].twin
        );
    }
    for lamp in &facts.lamps {
        out += &format!(
            "\n[[vehicle.lamps]]\ngroup = {}\nlamp = {}\nside = {}\nposition = {}\ncolour = {}\n\
             centre = {}\nfacing = {}\nwidth_m = {}\nheight_m = {}\n",
            text(&snake(&lamp.group)),
            lamp.lamp,
            text(&snake(&lamp.side)),
            text(&snake(&lamp.position)),
            text(&snake(&lamp.colour)),
            vec(lamp.centre),
            vec(lamp.facing),
            lamp.width_m,
            lamp.height_m
        );
    }
    out
}

// An enum's TOML word: its Debug name in snake case (`TailBrake` → `tail_brake`).
fn snake(value: &impl std::fmt::Debug) -> String {
    let mut out = String::new();
    for (index, c) in format!("{value:?}").chars().enumerate() {
        if c.is_ascii_uppercase() {
            if index > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// The no-overwrite rule for a vehicle: `None` when `assets/<asset_id>` is free, else the stale
/// report: role values the manifest no longer carries, and every published tier or map whose
/// digest differs from the new sidecar's.
pub(crate) fn existing(pack: &Path, inputs: &VehicleInputs) -> Option<Vec<Line>> {
    let directory = pack.join("assets").join(&inputs.role.asset_id);
    if !directory.exists() && !directory.is_symlink() {
        return None;
    }
    let manifest = match std::fs::read_to_string(directory.join("asset.toml"))
        .map_err(|e| e.to_string())
        .and_then(|text| AssetManifest::from_str(&text).map_err(|e| e.to_string()))
    {
        Ok(manifest) => manifest,
        Err(error) => {
            return Some(vec![report::line(
                "files",
                "asset_id",
                format!("assets/{}/asset.toml: {error}", inputs.role.asset_id),
            )]);
        }
    };
    let mut lines = Vec::new();
    let drafted = asset_toml(&inputs.facts, &inputs.role);
    if let Ok(drafted) = AssetManifest::from_str(&drafted)
        && let (Some(old), Some(new)) = (&manifest.vehicle, &drafted.vehicle)
    {
        let old_words = format!("{:?}", (old.vehicle_class, old.vehicle_family, old.seats, old.body_class));
        let new_words = format!("{:?}", (new.vehicle_class, new.vehicle_family, new.seats, new.body_class));
        if old_words != new_words {
            lines.push(report::line(
                "role",
                "vehicle",
                format!("the asset has {old_words}; the role and sidecar give {new_words}"),
            ));
        }
    }
    let published: BTreeMap<String, String> = inputs
        .facts
        .hashed_files()
        .split_last()
        .map(|(_, files)| files.to_vec())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(file, digest)| {
            let path = directory.join(file);
            let actual = crate::assets::archive::sha256(&path).unwrap_or_default();
            (actual != digest).then(|| (file.to_owned(), actual))
        })
        .collect();
    for (file, actual) in published {
        lines.push(report::line(
            "files",
            "geometry",
            if actual.is_empty() {
                format!("{file} is not in the asset; redraft the model")
            } else {
                format!("{file} differs from the sidecar's: the model changed since the draft")
            },
        ));
    }
    Some(lines)
}

/// The vehicle acceptance check: the asset as the scanner loads it, with no warning naming it.
pub(crate) fn check(mods: &Path, pack_id: &str, asset_id: &str) -> super::acceptance::Outcome {
    let mut outcome = super::acceptance::Outcome::default();
    let scan = crate::assets::scan_pack_dir(mods);
    let manifest = mods.join(pack_id).join("assets").join(asset_id).join("asset.toml");
    let name = format!("'{}'", manifest.display());
    outcome.failures.extend(scan.warnings.into_iter().filter(|warning| warning.contains(&name)));
    let loaded = scan
        .packs
        .iter()
        .filter(|pack| pack.pack.pack_id == pack_id)
        .flat_map(|pack| &pack.assets)
        .any(|(asset, _)| asset.asset_id == asset_id && asset.vehicle.is_some());
    if !loaded && outcome.failures.is_empty() {
        outcome
            .failures
            .push(format!("{pack_id}:{asset_id} is not loaded as a vehicle by the pack scanner"));
    }
    outcome
}

/// The role as the ledger records it.
pub(crate) fn ledger_role(role: &VehicleRole) -> Value {
    json!(role)
}

#[cfg(test)]
pub(crate) mod tests;
